use std::convert::Infallible;

use axum::{
    Router,
    http::StatusCode,
    response::sse::{Event, Sse},
    routing::post,
};
use contracts::{
    provider::{Request, StreamItem},
    transcript::{Role, TranscriptItem},
};
use futures_util::stream;
use loong_provider_openai::{OpenAiConfig, OpenAiProvider};
use provider::{Provider, StreamError};
use serde_json::json;
use tokio::sync::mpsc;

fn request(model: &str) -> Request {
    Request {
        model: model.to_string(),
        messages: vec![TranscriptItem::Message {
            role: Role::User,
            text: "hi".to_string(),
        }],
        tools: Vec::new(),
    }
}

async fn spawn_server(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn text_event(delta: &str) -> Event {
    Event::default().data(json!({ "choices": [{ "delta": { "content": delta } }] }).to_string())
}

fn tool_call_event(
    index: usize,
    id: Option<&str>,
    name: Option<&str>,
    arguments: Option<&str>,
) -> Event {
    let mut tool_call = json!({ "index": index });
    if let Some(id) = id {
        tool_call["id"] = json!(id);
    }
    if let Some(name) = name {
        tool_call["function"] = json!({ "name": name });
    }
    if let Some(arguments) = arguments {
        tool_call["function"]["arguments"] = json!(arguments);
    }
    Event::default()
        .data(json!({ "choices": [{ "delta": { "tool_calls": [tool_call] } }] }).to_string())
}

async fn sse_text() -> Sse<impl stream::Stream<Item = Result<Event, Infallible>>> {
    Sse::new(stream::iter(vec![
        Ok(text_event("Hel")),
        Ok(text_event("lo")),
        Ok(Event::default().data("[DONE]")),
    ]))
}

async fn sse_tool_calls() -> Sse<impl stream::Stream<Item = Result<Event, Infallible>>> {
    Sse::new(stream::iter(vec![
        Ok(tool_call_event(0, Some("call_1"), Some("echo"), Some(""))),
        Ok(tool_call_event(0, None, None, Some("{\"x\":"))),
        Ok(tool_call_event(0, None, None, Some("1}"))),
        Ok(Event::default().data("[DONE]")),
    ]))
}

async fn sse_truncated() -> Sse<impl stream::Stream<Item = Result<Event, Infallible>>> {
    Sse::new(stream::iter(vec![Ok(text_event("partial"))]))
}

async fn unauthorized() -> (StatusCode, &'static str) {
    (StatusCode::UNAUTHORIZED, "bad key")
}

#[tokio::test]
async fn streams_text_deltas() {
    let app = Router::new().route("/chat/completions", post(sse_text));
    let base_url = spawn_server(app).await;

    let provider = OpenAiProvider::new(OpenAiConfig::new(base_url, "test-key"));
    let (mut tx, mut rx) = mpsc::channel(16);

    let result = provider.stream(request("gpt-test"), &mut tx).await;
    drop(tx);

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(
        rx.recv().await,
        Some(StreamItem::Text {
            delta: "Hel".to_string()
        })
    );
    assert_eq!(
        rx.recv().await,
        Some(StreamItem::Text {
            delta: "lo".to_string()
        })
    );
    assert_eq!(rx.recv().await, None);
}

#[tokio::test]
async fn reassembles_tool_call_deltas() {
    let app = Router::new().route("/chat/completions", post(sse_tool_calls));
    let base_url = spawn_server(app).await;

    let provider = OpenAiProvider::new(OpenAiConfig::new(base_url, "test-key"));
    let (mut tx, mut rx) = mpsc::channel(16);

    let result = provider.stream(request("gpt-test"), &mut tx).await;
    drop(tx);

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(
        rx.recv().await,
        Some(StreamItem::ToolCall {
            id: "call_1".to_string(),
            name: "echo".to_string(),
            arguments: "{\"x\":1}".to_string(),
        })
    );
    assert_eq!(rx.recv().await, None);
}

#[tokio::test]
async fn rejects_request_without_consuming_it_on_http_error() {
    let app = Router::new().route("/chat/completions", post(unauthorized));
    let base_url = spawn_server(app).await;

    let provider = OpenAiProvider::new(OpenAiConfig::new(base_url, "test-key"));
    let (mut tx, _rx) = mpsc::channel(16);

    let result = provider.stream(request("gpt-test"), &mut tx).await;

    match result {
        Err(StreamError::Rejected { req, .. }) => assert_eq!(req.model, "gpt-test"),
        other => panic!("expected rejected, got {other:?}"),
    }
}

#[tokio::test]
async fn truncated_stream_disconnects() {
    let app = Router::new().route("/chat/completions", post(sse_truncated));
    let base_url = spawn_server(app).await;

    let provider = OpenAiProvider::new(OpenAiConfig::new(base_url, "test-key"));
    let (mut tx, mut rx) = mpsc::channel(16);

    let result = provider.stream(request("gpt-test"), &mut tx).await;
    drop(tx);

    assert!(matches!(result, Err(StreamError::Disconnected { .. })));
    assert_eq!(
        rx.recv().await,
        Some(StreamItem::Text {
            delta: "partial".to_string()
        })
    );
    assert_eq!(rx.recv().await, None);
}
