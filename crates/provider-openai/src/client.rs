//! OpenAI-compatible chat completion client.

use async_trait::async_trait;
use contracts::provider::{Request, StreamItem};
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use provider::{Provider, StreamError};

use crate::request::build_body;
use crate::sse::{OpenAiChunk, ToolCallBuilder, apply_tool_deltas};

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
#[derive(Clone)]
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
