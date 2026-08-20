//! OpenAI-compatible provider implementation.
//!
//! This crate adapts [`contracts::provider::Request`] to an OpenAI-compatible
//! chat completion endpoint and emits [`contracts::provider::StreamItem`]s.

use async_trait::async_trait;
use contracts::{
    provider::{Request, StreamItem},
    tool::ToolSpec,
    transcript::{Role, TranscriptItem},
};
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use provider::{Provider, StreamError};
use serde::Deserialize;
use serde_json::{Value, json};

/// Configuration for one OpenAI-compatible endpoint.
#[derive(Clone)]
pub struct OpenAiConfig {
    /// Base URL, for example `https://api.openai.com/v1`.
    pub base_url: String,
    /// Bearer token sent as `Authorization: Bearer ...`.
    pub api_key: String,
    /// Model name sent with every request.
    pub model: String,
}

impl OpenAiConfig {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }
}

/// OpenAI-compatible chat completion provider.
pub struct OpenAiProvider {
    client: reqwest::Client,
    config: OpenAiConfig,
}

impl OpenAiProvider {
    pub fn new(config: OpenAiConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }

    pub fn with_client(config: OpenAiConfig, client: reqwest::Client) -> Self {
        Self { client, config }
    }

    fn chat_completions_url(&self) -> String {
        format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        )
    }
}

#[async_trait]
impl<Out> Provider<Request, StreamItem, Out> for OpenAiProvider
where
    Out: loac::Writer<StreamItem> + Send,
{
    async fn stream(&self, req: Request, out: &mut Out) -> Result<(), StreamError<Request>> {
        let body = build_body(&req, &self.config.model);
        let response = match self
            .client
            .post(self.chat_completions_url())
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                return Err(StreamError::rejected(
                    format!("failed to reach upstream: {err}"),
                    req,
                ));
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let reason = match response.text().await {
                Ok(text) => format!("upstream returned {status}: {text}"),
                Err(err) => format!("upstream returned {status}: {err}"),
            };
            return Err(StreamError::Rejected { reason, req });
        }

        let mut events = response.bytes_stream().eventsource();
        let mut tool_calls: Vec<ToolCallBuilder> = Vec::new();
        let mut done = false;

        while let Some(event) = events.next().await {
            let event = match event {
                Ok(event) => event,
                Err(err) => {
                    return Err(StreamError::disconnected(format!(
                        "upstream SSE stream failed: {err}"
                    )));
                }
            };

            let data = event.data.trim();
            if data == "[DONE]" {
                done = true;
                break;
            }
            if data.is_empty() {
                continue;
            }

            let chunk: OpenAiChunk = match serde_json::from_str(data) {
                Ok(chunk) => chunk,
                Err(err) => {
                    return Err(StreamError::disconnected(format!(
                        "invalid SSE payload: {err}"
                    )));
                }
            };

            let Some(delta) = chunk
                .choices
                .iter()
                .find_map(|choice| choice.delta.as_ref())
            else {
                continue;
            };

            if let Some(content) = delta.content.as_deref()
                && !content.is_empty()
            {
                out.write(StreamItem::Text {
                    delta: content.to_owned(),
                })
                .await
                .map_err(|_| StreamError::disconnected("writer closed mid-stream"))?;
            }

            if let Some(tool_deltas) = delta.tool_calls.as_deref() {
                apply_tool_deltas(&mut tool_calls, tool_deltas);
            }
        }

        if !done {
            return Err(StreamError::disconnected(
                "upstream stream ended before [DONE]",
            ));
        }

        for builder in tool_calls {
            let (Some(id), Some(name)) = (builder.id, builder.name) else {
                continue;
            };
            out.write(StreamItem::ToolCall {
                id,
                name,
                arguments: builder.arguments,
            })
            .await
            .map_err(|_| StreamError::disconnected("writer closed mid-stream"))?;
        }

        Ok(())
    }
}

fn build_body(req: &Request, model: &str) -> Value {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(transcript_item_to_message)
        .collect();

    let tools: Vec<Value> = req.tools.iter().map(tool_spec_to_function).collect();

    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
    });

    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }

    body
}

fn transcript_item_to_message(item: &TranscriptItem) -> Value {
    match item {
        TranscriptItem::Message { role, text } => json!({
            "role": role_name(role),
            "content": text,
        }),
        TranscriptItem::ToolCall {
            call_id,
            name,
            arguments,
        } => json!({
            "role": "assistant",
            "tool_calls": [{
                "id": call_id.to_string(),
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": arguments,
                },
            }],
        }),
        TranscriptItem::ToolResult { call_id, output } => json!({
            "role": "tool",
            "tool_call_id": call_id.to_string(),
            "content": output,
        }),
    }
}

fn role_name(role: &Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

fn tool_spec_to_function(tool: &ToolSpec) -> Value {
    let parameters = serde_json::to_value(&tool.input_schema).unwrap_or_else(|_| json!({}));
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": parameters,
        },
    })
}

#[derive(Default)]
struct ToolCallBuilder {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

fn apply_tool_deltas(builders: &mut Vec<ToolCallBuilder>, deltas: &[OpenAiToolCallDelta]) {
    for delta in deltas {
        if builders.len() <= delta.index {
            builders.resize_with(delta.index + 1, ToolCallBuilder::default);
        }
        let builder = &mut builders[delta.index];
        if let Some(id) = delta.id.as_deref()
            && builder.id.is_none()
        {
            builder.id = Some(id.to_owned());
        }
        if let Some(function) = delta.function.as_ref() {
            if let Some(name) = function.name.as_deref()
                && builder.name.is_none()
            {
                builder.name = Some(name.to_owned());
            }
            if let Some(arguments) = function.arguments.as_deref() {
                builder.arguments.push_str(arguments);
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiChunk {
    choices: Vec<OpenAiChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    delta: Option<OpenAiDelta>,
}

#[derive(Debug, Deserialize)]
struct OpenAiDelta {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiToolCallDelta {
    index: usize,
    id: Option<String>,
    function: Option<OpenAiFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct OpenAiFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}
