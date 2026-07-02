use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::body::{Body, to_bytes};
use axum::http::Request;
use serde_json::json;
use tower::ServiceExt;

use crate::mvp::config::{
    LoongConfig, ProviderConfig, ProviderKind, ProviderProfileConfig, ProviderWireApi,
};

use super::build_openai_compat_test_router_no_backend;

const OPENAI_COMPAT_TEST_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;
static OPENAI_COMPAT_TEST_LOCK: Mutex<()> = Mutex::new(());

fn run_openai_compat_test_on_large_stack<F, Fut>(thread_name: &str, operation: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let _test_lock = OPENAI_COMPAT_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut env = crate::test_support::ScopedEnv::new();
    env.remove("LOONG_SQLITE_PATH");
    env.remove("LOONG_MEMORY_BACKEND");
    env.remove("LOONG_MEMORY_PROFILE");
    env.remove("LOONG_MEMORY_FAIL_OPEN");
    env.remove("LOONG_MEMORY_INGEST_MODE");
    env.remove("LOONG_MEMORY_SUMMARY_MAX_CHARS");
    env.remove("LOONG_SLIDING_WINDOW");
    env.remove("LOONG_MEMORY_PROFILE_NOTE");
    let join_handle = std::thread::Builder::new()
        .name(thread_name.to_owned())
        .stack_size(OPENAI_COMPAT_TEST_STACK_SIZE_BYTES)
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build openai compat test runtime");
            runtime.block_on(operation());
        })
        .expect("spawn openai compat large-stack test thread");
    match join_handle.join() {
        Ok(()) => {}
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn openai_compat_test_config() -> LoongConfig {
    let mut config = LoongConfig {
        providers: BTreeMap::from([
            (
                "openai-main".to_owned(),
                ProviderProfileConfig {
                    default_for_kind: true,
                    provider: ProviderConfig {
                        kind: ProviderKind::Openai,
                        model: "gpt-5".to_owned(),
                        wire_api: ProviderWireApi::ChatCompletions,
                        ..ProviderConfig::default()
                    },
                },
            ),
            (
                "anthropic-main".to_owned(),
                ProviderProfileConfig {
                    default_for_kind: false,
                    provider: ProviderConfig {
                        kind: ProviderKind::Anthropic,
                        model: "claude-sonnet-4-5".to_owned(),
                        ..ProviderConfig::default()
                    },
                },
            ),
        ]),
        active_provider: Some("openai-main".to_owned()),
        ..LoongConfig::default()
    };
    let sqlite_path = next_openai_compat_test_sqlite_path("test-config");
    config.memory.sqlite_path = sqlite_path.display().to_string();
    config
}

fn openai_compat_provider_config(base_url: String) -> LoongConfig {
    let mut config = LoongConfig {
        providers: BTreeMap::from([
            (
                "openai-main".to_owned(),
                ProviderProfileConfig {
                    default_for_kind: true,
                    provider: ProviderConfig {
                        kind: ProviderKind::Openai,
                        model: "gpt-5".to_owned(),
                        base_url: base_url.clone(),
                        api_key: Some(loong_contracts::SecretRef::Inline("test-key".to_owned())),
                        api_key_env: None,
                        oauth_access_token: None,
                        oauth_access_token_env: None,
                        wire_api: ProviderWireApi::ChatCompletions,
                        ..ProviderConfig::default()
                    },
                },
            ),
            (
                "anthropic-main".to_owned(),
                ProviderProfileConfig {
                    default_for_kind: false,
                    provider: ProviderConfig {
                        kind: ProviderKind::Anthropic,
                        model: "claude-sonnet-4-5".to_owned(),
                        base_url,
                        api_key: Some(loong_contracts::SecretRef::Inline("test-key".to_owned())),
                        api_key_env: None,
                        oauth_access_token: None,
                        oauth_access_token_env: None,
                        ..ProviderConfig::default()
                    },
                },
            ),
        ]),
        active_provider: Some("openai-main".to_owned()),
        ..LoongConfig::default()
    };
    let sqlite_path = next_openai_compat_test_sqlite_path("provider-config");
    config.memory.sqlite_path = sqlite_path.display().to_string();
    config
}

fn next_openai_compat_test_sqlite_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(super::next_openai_compat_test_artifact_name(label))
}

fn openai_compat_unsupported_stream_config() -> LoongConfig {
    LoongConfig {
        providers: BTreeMap::from([(
            "bedrock-main".to_owned(),
            ProviderProfileConfig {
                default_for_kind: true,
                provider: ProviderConfig {
                    kind: ProviderKind::Bedrock,
                    model: "anthropic.claude-3-7-sonnet-20250219-v1:0".to_owned(),
                    ..ProviderConfig::default()
                },
            },
        )]),
        active_provider: Some("bedrock-main".to_owned()),
        ..LoongConfig::default()
    }
}

fn openai_compat_duplicate_model_config(base_url: String) -> LoongConfig {
    LoongConfig {
        providers: BTreeMap::from([
            (
                "openai-main".to_owned(),
                ProviderProfileConfig {
                    default_for_kind: true,
                    provider: ProviderConfig {
                        kind: ProviderKind::Openai,
                        model: "gpt-5".to_owned(),
                        base_url: base_url.clone(),
                        api_key: Some(loong_contracts::SecretRef::Inline("primary-key".to_owned())),
                        api_key_env: None,
                        oauth_access_token: None,
                        oauth_access_token_env: None,
                        wire_api: ProviderWireApi::ChatCompletions,
                        ..ProviderConfig::default()
                    },
                },
            ),
            (
                "openai-backup".to_owned(),
                ProviderProfileConfig {
                    default_for_kind: false,
                    provider: ProviderConfig {
                        kind: ProviderKind::Openai,
                        model: "gpt-5".to_owned(),
                        base_url,
                        api_key: Some(loong_contracts::SecretRef::Inline("backup-key".to_owned())),
                        api_key_env: None,
                        oauth_access_token: None,
                        oauth_access_token_env: None,
                        wire_api: ProviderWireApi::ChatCompletions,
                        ..ProviderConfig::default()
                    },
                },
            ),
        ]),
        active_provider: Some("openai-main".to_owned()),
        ..LoongConfig::default()
    }
}

fn spawn_openai_compat_provider_server(
    status_line: &'static str,
    body: &'static str,
) -> (String, std::thread::JoinHandle<Vec<String>>) {
    spawn_openai_compat_provider_server_with_content_type(status_line, "application/json", body)
}

fn spawn_openai_compat_provider_server_with_content_type(
    status_line: &'static str,
    content_type: &'static str,
    body: &'static str,
) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind provider listener");
    let addr = listener.local_addr().expect("local addr");
    let server = std::thread::spawn(move || {
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut idle_deadline = None;
        let mut requests = Vec::new();
        loop {
            if Instant::now() >= deadline {
                break;
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_openai_compat_provider_request(&mut stream, deadline);
                    requests.push(request);
                    idle_deadline = Some(Instant::now() + Duration::from_secs(2));
                    let response = format!(
                        "{status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("write response");
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if let Some(idle_deadline) = idle_deadline
                        && Instant::now() >= idle_deadline
                    {
                        break;
                    }
                    std::thread::yield_now();
                }
                Err(error) => panic!("accept provider request: {error}"),
            }
        }
        if requests.is_empty() {
            panic!("timed out waiting for provider request");
        }
        requests
    });
    (format!("http://{addr}"), server)
}

fn read_openai_compat_provider_request(
    stream: &mut std::net::TcpStream,
    deadline: Instant,
) -> String {
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .expect("set read timeout");
    let mut buffer = Vec::new();
    let mut temp = [0u8; 4096];
    let mut header_end = None;
    let mut expected_total_len = None;
    loop {
        if Instant::now() >= deadline {
            break;
        }
        match stream.read(&mut temp) {
            Ok(0) => break,
            Ok(read) => {
                buffer.extend_from_slice(&temp[..read]);
                if header_end.is_none()
                    && let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    let end = index + 4;
                    header_end = Some(end);
                    let headers = String::from_utf8_lossy(&buffer[..end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let lower = line.to_ascii_lowercase();
                            lower
                                .strip_prefix("content-length: ")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    expected_total_len = Some(end + content_length);
                }
                if let Some(total_len) = expected_total_len
                    && buffer.len() >= total_len
                {
                    break;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                std::thread::yield_now();
            }
            Err(error) => panic!("read provider request: {error}"),
        }
    }
    String::from_utf8(buffer).expect("utf8 request")
}

#[tokio::test]
async fn gateway_openai_models_rejects_missing_auth() {
    let app =
        build_openai_compat_test_router_no_backend(openai_compat_test_config(), "tok".to_owned());
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/models")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn gateway_openai_chat_completion_rejects_missing_auth() {
    let app =
        build_openai_compat_test_router_no_backend(openai_compat_test_config(), "tok".to_owned());
    let body = serde_json::json!({
        "model": "gpt-5",
        "messages": [{"role": "user", "content": "hello"}]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn gateway_openai_models_lists_configured_provider_profiles() {
    let app =
        build_openai_compat_test_router_no_backend(openai_compat_test_config(), "tok".to_owned());
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/models")
                .header("authorization", "Bearer tok")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    if status != axum::http::StatusCode::OK {
        panic!("status={status} body={}", String::from_utf8_lossy(&body));
    }
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let ids = payload["data"]
        .as_array()
        .expect("models array")
        .iter()
        .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();

    assert!(ids.contains(&"gpt-5"));
    assert!(ids.contains(&"claude-sonnet-4-5"));
}

#[tokio::test]
async fn gateway_openai_models_exposes_default_configured_model_value() {
    let app = build_openai_compat_test_router_no_backend(LoongConfig::default(), "tok".to_owned());
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/models")
                .header("authorization", "Bearer tok")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "body={}",
        String::from_utf8_lossy(&body)
    );
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let ids = payload["data"]
        .as_array()
        .expect("models array")
        .iter()
        .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();

    assert!(ids.contains(&"auto"), "ids={ids:?}");
}

#[test]
fn gateway_openai_models_disambiguate_duplicate_model_ids_by_profile() {
    run_openai_compat_test_on_large_stack("openai-compat-duplicate-models", || async move {
        gateway_openai_models_disambiguate_duplicate_model_ids_by_profile_impl().await;
    });
}

async fn gateway_openai_models_disambiguate_duplicate_model_ids_by_profile_impl() {
    let (base_url, server) = spawn_openai_compat_provider_server(
        "HTTP/1.1 200 OK",
        r#"{"choices":[{"message":{"role":"assistant","content":"provider says hi"}}]}"#,
    );
    let app = build_openai_compat_test_router_no_backend(
        openai_compat_duplicate_model_config(base_url),
        "tok".to_owned(),
    );
    let models_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/v1/models")
                .header("authorization", "Bearer tok")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(models_response.status(), axum::http::StatusCode::OK);
    let models_body = to_bytes(models_response.into_body(), usize::MAX)
        .await
        .expect("body");
    let models_payload: serde_json::Value = serde_json::from_slice(&models_body).expect("json");
    let ids = models_payload["data"]
        .as_array()
        .expect("models array")
        .iter()
        .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();

    assert!(ids.contains(&"openai-main:gpt-5"), "ids={ids:?}");
    assert!(ids.contains(&"openai-backup:gpt-5"), "ids={ids:?}");
    assert!(!ids.contains(&"gpt-5"), "ids={ids:?}");

    let completion_body = serde_json::json!({
        "model": "openai-backup:gpt-5",
        "messages": [{"role": "user", "content": "hello"}]
    });
    let completion_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&completion_body).expect("encode body"),
                ))
                .expect("request"),
        )
        .await
        .expect("response");

    let completion_status = completion_response.status();
    let completion_body = to_bytes(completion_response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(
        completion_status,
        axum::http::StatusCode::OK,
        "body={}",
        String::from_utf8_lossy(&completion_body)
    );
    let requests = server.join().expect("join provider server");
    assert_eq!(requests.len(), 1);
    let normalized_request = requests[0].to_ascii_lowercase();
    assert!(normalized_request.contains("authorization: bearer backup-key"));
}

#[tokio::test]
async fn gateway_openai_chat_completion_rejects_tools_fields() {
    let app =
        build_openai_compat_test_router_no_backend(openai_compat_test_config(), "tok".to_owned());
    let body = serde_json::json!({
        "model": "gpt-5",
        "messages": [{"role": "user", "content": "hello"}],
        "tools": [{"type": "function", "function": {"name": "echo"}}]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn gateway_openai_chat_completion_rejects_tool_choice_field() {
    let app =
        build_openai_compat_test_router_no_backend(openai_compat_test_config(), "tok".to_owned());
    let body = serde_json::json!({
        "model": "gpt-5",
        "messages": [{"role": "user", "content": "hello"}],
        "tool_choice": "auto"
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(payload["error"]["param"], "tool_choice");
}

#[tokio::test]
async fn gateway_openai_chat_completion_rejects_unknown_model() {
    let app =
        build_openai_compat_test_router_no_backend(openai_compat_test_config(), "tok".to_owned());
    let body = serde_json::json!({
        "model": "missing-model",
        "messages": [{"role": "user", "content": "hello"}]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn gateway_openai_chat_completion_rejects_messages_not_ending_with_user() {
    let app =
        build_openai_compat_test_router_no_backend(openai_compat_test_config(), "tok".to_owned());
    let body = serde_json::json!({
        "model": "gpt-5",
        "messages": [
            {"role": "user", "content": "hello"},
            {"role": "assistant", "content": "prior answer"}
        ]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(payload["error"]["param"], "messages");
}

#[test]
fn gateway_openai_chat_completion_returns_non_streaming_response() {
    run_openai_compat_test_on_large_stack("openai-compat-non-stream", || async move {
        gateway_openai_chat_completion_returns_non_streaming_response_impl().await;
    });
}

async fn gateway_openai_chat_completion_returns_non_streaming_response_impl() {
    let (base_url, server) = spawn_openai_compat_provider_server(
        "HTTP/1.1 200 OK",
        r#"{"choices":[{"message":{"role":"assistant","content":"provider says hi"}}]}"#,
    );
    let mut config = openai_compat_provider_config(base_url);
    config.provider.retry_max_attempts = 1;
    if let Some(profile) = config.providers.get_mut("openai-main") {
        profile.provider.retry_max_attempts = 1;
    }
    let app = build_openai_compat_test_router_no_backend(config, "tok".to_owned());
    let body = serde_json::json!({
        "model": "gpt-5",
        "messages": [
            {"role": "system", "content": "system prompt"},
            {"role": "assistant", "content": "prior answer"},
            {"role": "user", "content": "hello"}
        ]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "body={}",
        String::from_utf8_lossy(&body)
    );
    let requests = server.join().expect("join provider server");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("POST /v1/chat/completions "));
    assert!(
        requests[0].contains("system prompt"),
        "request={}",
        requests[0]
    );
    assert!(requests[0].contains("hello"), "request={}", requests[0]);
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");

    assert_eq!(payload["object"], "chat.completion");
    assert_eq!(payload["model"], "gpt-5");
    assert_eq!(payload["choices"][0]["message"]["role"], "assistant");
    assert_eq!(
        payload["choices"][0]["message"]["content"],
        "provider says hi"
    );
}

#[test]
fn gateway_openai_chat_completion_surfaces_provider_usage_in_non_streaming_response() {
    run_openai_compat_test_on_large_stack("openai-compat-usage", || async move {
        gateway_openai_chat_completion_surfaces_provider_usage_in_non_streaming_response_impl()
            .await;
    });
}

async fn gateway_openai_chat_completion_surfaces_provider_usage_in_non_streaming_response_impl() {
    let (base_url, _server) = spawn_openai_compat_provider_server(
        "HTTP/1.1 200 OK",
        r#"{"choices":[{"message":{"role":"assistant","content":"provider says hi"}}],"usage":{"prompt_tokens":11,"completion_tokens":7,"total_tokens":18}}"#,
    );
    let app = build_openai_compat_test_router_no_backend(
        openai_compat_provider_config(base_url),
        "tok".to_owned(),
    );
    let body = serde_json::json!({
        "model": "gpt-5",
        "messages": [{"role": "user", "content": "hello"}]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        payload["usage"],
        json!({
            "prompt_tokens": 11,
            "completion_tokens": 7,
            "total_tokens": 18
        })
    );
}

#[test]
fn gateway_openai_chat_completion_preserves_provider_rate_limit_status() {
    run_openai_compat_test_on_large_stack("openai-compat-rate-limit-status", || async move {
        gateway_openai_chat_completion_preserves_provider_rate_limit_status_impl().await;
    });
}

async fn gateway_openai_chat_completion_preserves_provider_rate_limit_status_impl() {
    let (base_url, _server) = spawn_openai_compat_provider_server(
        "HTTP/1.1 429 Too Many Requests",
        r#"{"error":{"message":"rate limit exceeded"}}"#,
    );
    let app = build_openai_compat_test_router_no_backend(
        openai_compat_provider_config(base_url),
        "tok".to_owned(),
    );
    let body = serde_json::json!({
        "model": "gpt-5",
        "messages": [{"role": "user", "content": "hello"}]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(
        status,
        axum::http::StatusCode::TOO_MANY_REQUESTS,
        "payload={payload}"
    );
    assert!(
        payload["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("rate limit"),
        "payload={payload}"
    );
}

#[test]
fn gateway_openai_chat_completion_persists_turn_history_through_gateway_runtime() {
    run_openai_compat_test_on_large_stack("openai-compat-history", || async move {
        gateway_openai_chat_completion_persists_turn_history_through_gateway_runtime_impl().await;
    });
}

async fn gateway_openai_chat_completion_persists_turn_history_through_gateway_runtime_impl() {
    let (base_url, server) = spawn_openai_compat_provider_server(
        "HTTP/1.1 200 OK",
        r#"{"choices":[{"message":{"role":"assistant","content":"provider says hi"}}]}"#,
    );
    let sqlite_path = next_openai_compat_test_sqlite_path("history");
    let mut config = openai_compat_provider_config(base_url);
    config.memory.sqlite_path = sqlite_path.display().to_string();
    let memory_config =
        crate::mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config_without_env_overrides(
            &config.memory,
        );
    let app = build_openai_compat_test_router_no_backend(config, "tok".to_owned());
    let body = serde_json::json!({
        "model": "gpt-5",
        "messages": [
            {"role": "system", "content": "system prompt"},
            {"role": "assistant", "content": "prior answer"},
            {"role": "user", "content": "hello"}
        ]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    if status != axum::http::StatusCode::OK {
        panic!("status={status} body={}", String::from_utf8_lossy(&body));
    }
    let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let session_id = payload["id"].as_str().expect("response id");
    let turns =
        crate::mvp::memory::window_direct(session_id, 8, &memory_config).expect("session turns");

    assert!(
        turns
            .iter()
            .any(|turn| turn.role == "system" && turn.content == "system prompt"),
        "turns={turns:?}"
    );
    assert!(
        turns
            .iter()
            .any(|turn| turn.role == "assistant" && turn.content == "prior answer"),
        "turns={turns:?}"
    );
    assert!(
        turns
            .iter()
            .any(|turn| turn.role == "assistant" && turn.content == "provider says hi"),
        "turns={turns:?}"
    );

    let _ = std::fs::remove_file(&sqlite_path);
    server.join().expect("join provider server");
}

#[test]
fn gateway_openai_chat_completion_streaming_persists_turn_history_through_gateway_runtime() {
    run_openai_compat_test_on_large_stack("openai-compat-stream-history", || async move {
        gateway_openai_chat_completion_streaming_persists_turn_history_through_gateway_runtime_impl()
            .await;
    });
}

async fn gateway_openai_chat_completion_streaming_persists_turn_history_through_gateway_runtime_impl()
 {
    let (base_url, _server) = spawn_openai_compat_provider_server_with_content_type(
        "HTTP/1.1 200 OK",
        "text/event-stream",
        concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hello \"},\"index\":0,\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"world\"},\"index\":0,\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"index\":0,\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        ),
    );
    let sqlite_path = next_openai_compat_test_sqlite_path("stream-history");
    let mut config = openai_compat_provider_config(base_url);
    config.memory.sqlite_path = sqlite_path.display().to_string();
    let memory_config =
        crate::mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config_without_env_overrides(
            &config.memory,
        );
    let app = build_openai_compat_test_router_no_backend(config, "tok".to_owned());
    let body = serde_json::json!({
        "model": "gpt-5",
        "stream": true,
        "messages": [
            {"role": "assistant", "content": "prior answer"},
            {"role": "user", "content": "hello"}
        ]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");
    let payloads = body_text
        .split("\n\n")
        .filter_map(|frame| frame.strip_prefix("data: "))
        .filter(|payload| *payload != "[DONE]")
        .filter_map(|payload| serde_json::from_str::<serde_json::Value>(payload).ok())
        .collect::<Vec<_>>();
    let session_id = payloads
        .iter()
        .find_map(|payload| payload["id"].as_str())
        .expect("stream chunk id");
    let turns =
        crate::mvp::memory::window_direct(session_id, 8, &memory_config).expect("session turns");

    assert!(
        turns
            .iter()
            .any(|turn| turn.role == "user" && turn.content == "hello"),
        "turns={turns:?}"
    );
    assert!(
        turns
            .iter()
            .any(|turn| turn.role == "assistant" && turn.content.starts_with("hello world")),
        "turns={turns:?}"
    );

    let _ = std::fs::remove_file(&sqlite_path);
}

#[test]
fn gateway_openai_chat_completion_passes_tuning_fields_to_provider() {
    run_openai_compat_test_on_large_stack("openai-compat-tuning", || async move {
        gateway_openai_chat_completion_passes_tuning_fields_to_provider_impl().await;
    });
}

async fn gateway_openai_chat_completion_passes_tuning_fields_to_provider_impl() {
    let (base_url, server) = spawn_openai_compat_provider_server(
        "HTTP/1.1 200 OK",
        r#"{"choices":[{"message":{"role":"assistant","content":"provider says hi"}}]}"#,
    );
    let app = build_openai_compat_test_router_no_backend(
        openai_compat_provider_config(base_url),
        "tok".to_owned(),
    );
    let body = serde_json::json!({
        "model": "gpt-5",
        "temperature": 0.2,
        "max_tokens": 77,
        "stop": ["END", "HALT"],
        "messages": [{"role": "user", "content": "hello"}]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let requests = server.join().expect("join provider server");
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].contains("\"temperature\":0.2"),
        "request={}",
        requests[0]
    );
    assert!(
        requests[0].contains("\"max_tokens\":77")
            || requests[0].contains("\"max_completion_tokens\":77"),
        "request={}",
        requests[0]
    );
    assert!(
        requests[0].contains("\"stop\":[\"END\",\"HALT\"]"),
        "request={}",
        requests[0]
    );
}

#[tokio::test]
async fn gateway_openai_chat_completion_rejects_invalid_stop_shape() {
    let app =
        build_openai_compat_test_router_no_backend(openai_compat_test_config(), "tok".to_owned());
    let body = serde_json::json!({
        "model": "gpt-5",
        "stop": {"bad": true},
        "messages": [{"role": "user", "content": "hello"}]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[test]
fn gateway_openai_chat_completion_returns_streaming_sse_response() {
    run_openai_compat_test_on_large_stack("openai-compat-stream-anthropic", || async move {
        gateway_openai_chat_completion_returns_streaming_sse_response_impl().await;
    });
}

async fn gateway_openai_chat_completion_returns_streaming_sse_response_impl() {
    let (base_url, _server) = spawn_openai_compat_provider_server_with_content_type(
        "HTTP/1.1 200 OK",
        "text/event-stream",
        concat!(
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hello \"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"world\"}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        ),
    );
    let app = build_openai_compat_test_router_no_backend(
        openai_compat_provider_config(base_url),
        "tok".to_owned(),
    );
    let body = serde_json::json!({
        "model": "claude-sonnet-4-5",
        "stream": true,
        "messages": [{"role": "user", "content": "hello"}]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(content_type.contains("text/event-stream"));
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");
    let contents = collect_stream_contents(body_text.as_str());

    assert!(body_text.contains("chat.completion.chunk"));
    assert_eq!(contents.join(""), "hello world");
    assert!(body_text.contains("data: [DONE]"));
}

#[test]
fn gateway_openai_chat_completion_returns_openai_model_streaming_sse_response() {
    run_openai_compat_test_on_large_stack("openai-compat-stream-openai", || async move {
        gateway_openai_chat_completion_returns_openai_model_streaming_sse_response_impl().await;
    });
}

async fn gateway_openai_chat_completion_returns_openai_model_streaming_sse_response_impl() {
    let (base_url, _server) = spawn_openai_compat_provider_server_with_content_type(
        "HTTP/1.1 200 OK",
        "text/event-stream",
        concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hello \"},\"index\":0,\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"world\"},\"index\":0,\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"index\":0,\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        ),
    );
    let app = build_openai_compat_test_router_no_backend(
        openai_compat_provider_config(base_url),
        "tok".to_owned(),
    );
    let body = serde_json::json!({
        "model": "gpt-5",
        "stream": true,
        "messages": [{"role": "user", "content": "hello"}]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(content_type.contains("text/event-stream"));
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");
    let contents = collect_stream_contents(body_text.as_str());

    assert_eq!(contents.join(""), "hello world");
    assert!(body_text.contains("data: [DONE]"));
}

#[tokio::test]
async fn gateway_openai_chat_completion_streaming_rejects_truly_unsupported_models() {
    let app = build_openai_compat_test_router_no_backend(
        openai_compat_unsupported_stream_config(),
        "tok".to_owned(),
    );
    let body = serde_json::json!({
        "model": "anthropic.claude-3-7-sonnet-20250219-v1:0",
        "stream": true,
        "messages": [{"role": "user", "content": "hello"}]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::NOT_IMPLEMENTED);
}

#[test]
fn gateway_openai_chat_completion_streaming_failure_emits_error_chunk() {
    run_openai_compat_test_on_large_stack("openai-compat-stream-error", || async move {
        gateway_openai_chat_completion_streaming_failure_emits_error_chunk_impl().await;
    });
}

async fn gateway_openai_chat_completion_streaming_failure_emits_error_chunk_impl() {
    let (base_url, _server) = spawn_openai_compat_provider_server_with_content_type(
        "HTTP/1.1 500 Internal Server Error",
        "application/json",
        r#"{"error":{"message":"stream backend failed"}}"#,
    );
    let app = build_openai_compat_test_router_no_backend(
        openai_compat_provider_config(base_url),
        "tok".to_owned(),
    );
    let body = serde_json::json!({
        "model": "claude-sonnet-4-5",
        "stream": true,
        "messages": [{"role": "user", "content": "hello"}]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(content_type.contains("text/event-stream"));
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");

    assert!(body_text.contains("\"error\""));
    assert!(body_text.contains("data: [DONE]"));
}

#[test]
fn gateway_openai_chat_completion_streaming_uses_provider_events_when_available() {
    run_openai_compat_test_on_large_stack("openai-compat-stream-provider-events", || async move {
        gateway_openai_chat_completion_streaming_uses_provider_events_when_available_impl().await;
    });
}

async fn gateway_openai_chat_completion_streaming_uses_provider_events_when_available_impl() {
    let (base_url, _server) = spawn_openai_compat_provider_server_with_content_type(
        "HTTP/1.1 200 OK",
        "text/event-stream",
        concat!(
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"partial \"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        ),
    );
    let app = build_openai_compat_test_router_no_backend(
        openai_compat_provider_config(base_url),
        "tok".to_owned(),
    );
    let body = serde_json::json!({
        "model": "claude-sonnet-4-5",
        "stream": true,
        "messages": [{"role": "user", "content": "hello"}]
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", "Bearer tok")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).expect("encode body")))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body_text = String::from_utf8(body.to_vec()).expect("utf8 body");
    let contents = collect_stream_contents(body_text.as_str());

    assert_eq!(contents, vec!["partial ".to_owned(), "hello".to_owned()]);
}

fn collect_stream_contents(body_text: &str) -> Vec<String> {
    body_text
        .split("\n\n")
        .filter_map(|frame| frame.strip_prefix("data: "))
        .filter(|payload| *payload != "[DONE]")
        .filter_map(|payload| serde_json::from_str::<serde_json::Value>(payload).ok())
        .filter_map(|payload| {
            payload
                .get("choices")
                .and_then(serde_json::Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("delta"))
                .and_then(|delta| delta.get("content"))
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .collect()
}
