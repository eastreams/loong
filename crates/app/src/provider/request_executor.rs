use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::Stream;
use serde_json::Value;
use tokio::time::sleep;

use crate::config::ProviderAuthScheme;
use crate::config::ProviderConfig;
use crate::conversation::turn_engine::{ProviderTurn, ToolIntent};

use super::{
    auth_profile_runtime::ProviderAuthProfile,
    contracts::{
        CompletionPayloadMode, ProviderApiError, ProviderCapabilityContract,
        ProviderRuntimeContract, adapt_payload_mode_for_error, parse_provider_api_error,
    },
    failover::{
        ModelRequestError, ProviderFailoverReason, ProviderFailoverStage, build_model_request_error,
    },
    policy,
    request_planner::{
        ModelRequestStatusPlan, classify_model_status_failure_reason_with_capability,
        plan_model_request_status_with_capability, plan_transport_error_retry,
    },
    transport,
    transport_trait::{ProviderTransport, TransportEventStream, TransportStream},
};

pub(super) struct ModelRequestRuntime<'a> {
    pub(super) provider: &'a ProviderConfig,
    pub(super) model: &'a str,
    pub(super) runtime_contract: ProviderRuntimeContract,
    pub(super) capability: ProviderCapabilityContract,
    pub(super) auto_model_mode: bool,
    pub(super) auth_profile: &'a ProviderAuthProfile,
    pub(super) request_auth_scheme: ProviderAuthScheme,
    pub(super) endpoint: &'a str,
    pub(super) headers: &'a reqwest::header::HeaderMap,
    pub(super) request_policy: &'a policy::ProviderRequestPolicy,
    pub(super) transport: &'a dyn ProviderTransport,
    pub(super) auth_context: &'a transport::RequestAuthContext,
}

pub(super) struct StreamingModelRequestRuntime<'a> {
    pub(super) provider: &'a ProviderConfig,
    pub(super) model: &'a str,
    pub(super) runtime_contract: ProviderRuntimeContract,
    pub(super) capability: ProviderCapabilityContract,
    pub(super) auto_model_mode: bool,
    pub(super) auth_profile: &'a ProviderAuthProfile,
    pub(super) request_auth_scheme: ProviderAuthScheme,
    pub(super) endpoint: &'a str,
    pub(super) headers: &'a reqwest::header::HeaderMap,
    pub(super) request_policy: &'a policy::ProviderRequestPolicy,
    pub(super) transport: &'a dyn ProviderTransport,
    pub(super) auth_context: &'a transport::RequestAuthContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelStatusOutcome {
    Retry { delay_ms: u64, next_backoff_ms: u64 },
    TryNextModel,
    Fail { reason: ProviderFailoverReason },
}

fn plan_model_status_outcome(
    status_code: u16,
    response_headers: &reqwest::header::HeaderMap,
    api_error: &ProviderApiError,
    attempt: usize,
    request_policy: &policy::ProviderRequestPolicy,
    backoff_ms: u64,
    auto_model_mode: bool,
    runtime_contract: ProviderRuntimeContract,
    capability: ProviderCapabilityContract,
) -> ModelStatusOutcome {
    match plan_model_request_status_with_capability(
        status_code,
        response_headers,
        api_error,
        attempt,
        request_policy,
        backoff_ms,
        auto_model_mode,
        runtime_contract,
        capability,
    ) {
        ModelRequestStatusPlan::Retry {
            delay_ms,
            next_backoff_ms,
        } => ModelStatusOutcome::Retry {
            delay_ms,
            next_backoff_ms,
        },
        ModelRequestStatusPlan::TryNextModel => ModelStatusOutcome::TryNextModel,
        ModelRequestStatusPlan::Fail => ModelStatusOutcome::Fail {
            reason: classify_model_status_failure_reason_with_capability(
                status_code,
                api_error,
                runtime_contract,
                capability,
            ),
        },
    }
}

fn render_status_failure_message(
    provider: &ProviderConfig,
    reason: ProviderFailoverReason,
    status_code: u16,
    model: &str,
    attempt: usize,
    max_attempts: usize,
    response_body: &Value,
) -> String {
    let support_facts = provider.support_facts();
    let auth_support = support_facts.auth;
    let region_endpoint_support = support_facts.region_endpoint;
    let mut message = format!(
        "provider returned status {status_code} for model `{model}` on attempt {attempt}/{max_attempts}: {response_body}"
    );
    if matches!(reason, ProviderFailoverReason::AuthRejected) {
        if let Some(hint) = auth_support.guidance_hint {
            message.push(' ');
            message.push_str(hint.as_str());
        }
        if let Some(hint) = region_endpoint_support.request_failure_hint {
            message.push(' ');
            message.push_str(hint.as_str());
        }
    }
    message
}

pub(super) async fn execute_model_request<T, BuildBody, ParseSuccess, PreStatusError>(
    runtime: ModelRequestRuntime<'_>,
    mut build_body: BuildBody,
    mut parse_success: ParseSuccess,
    missing_shape_fragment: &'static str,
    mut pre_status_error: PreStatusError,
) -> Result<T, ModelRequestError>
where
    BuildBody: FnMut(CompletionPayloadMode) -> Value,
    ParseSuccess: FnMut(&Value) -> Option<T>,
    PreStatusError: FnMut(&ProviderApiError) -> bool,
{
    let mut attempt = 0usize;
    let mut backoff_ms = runtime.request_policy.initial_backoff_ms;
    let mut payload_mode =
        CompletionPayloadMode::default_for_contract(runtime.provider, runtime.runtime_contract);
    let mut tried_payload_modes = vec![payload_mode];

    loop {
        attempt += 1;
        let body = build_body(payload_mode);
        let request_endpoint =
            transport::resolve_request_endpoint(runtime.provider, runtime.endpoint, runtime.model);
        let request_endpoint =
            transport::resolve_request_url(runtime.provider, request_endpoint.as_str(), runtime.auth_context)
                .map_err(|error| {
                    build_model_request_error(
                        format!(
                            "provider request setup failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                            model = runtime.model,
                            max_attempts = runtime.request_policy.max_attempts
                        ),
                        false,
                        ProviderFailoverReason::TransportFailure,
                        ProviderFailoverStage::TransportFailure,
                        runtime.model,
                        attempt,
                        runtime.request_policy.max_attempts,
                        None,
                        None,
                    )
                })?;
        let body_bytes = transport::encode_json_request_body(&body).map_err(|error| {
            build_model_request_error(
                format!(
                    "provider request setup failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                    model = runtime.model,
                    max_attempts = runtime.request_policy.max_attempts
                ),
                false,
                ProviderFailoverReason::TransportFailure,
                ProviderFailoverStage::TransportFailure,
                runtime.model,
                attempt,
                runtime.request_policy.max_attempts,
                None,
                None,
            )
        })?;
        let mut headers = runtime.headers.clone();
        transport::apply_json_request_defaults(&mut headers).map_err(|error| {
            build_model_request_error(
                format!(
                    "provider request setup failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                    model = runtime.model,
                    max_attempts = runtime.request_policy.max_attempts
                ),
                false,
                ProviderFailoverReason::TransportFailure,
                ProviderFailoverStage::TransportFailure,
                runtime.model,
                attempt,
                runtime.request_policy.max_attempts,
                None,
                None,
            )
        })?;
        let request = transport::build_transport_request(
            reqwest::Method::POST,
            request_endpoint.clone(),
            headers,
            body_bytes,
            Some(runtime.auth_profile),
            runtime.request_auth_scheme,
        )
        .map_err(|error| {
            build_model_request_error(
                format!(
                    "provider request setup failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                    model = runtime.model,
                    max_attempts = runtime.request_policy.max_attempts
                ),
                false,
                ProviderFailoverReason::TransportFailure,
                ProviderFailoverStage::TransportFailure,
                runtime.model,
                attempt,
                runtime.request_policy.max_attempts,
                None,
                None,
            )
        })?;

        match runtime.transport.execute(request).await {
            Ok(response) => {
                let status = response.status;
                let response_headers = response.headers;
                let response_body = response.body;

                if status.is_success() {
                    let parsed = parse_success(&response_body).ok_or_else(|| {
                        build_model_request_error(
                            format!(
                                "provider response missing {missing_shape_fragment} for model `{model}` on attempt {attempt}/{max_attempts}: {response_body}",
                                model = runtime.model,
                                max_attempts = runtime.request_policy.max_attempts
                            ),
                            false,
                            ProviderFailoverReason::ResponseShapeInvalid,
                            ProviderFailoverStage::ResponseShapeInvalid,
                            runtime.model,
                            attempt,
                            runtime.request_policy.max_attempts,
                            None,
                            None,
                        )
                    })?;
                    return Ok(parsed);
                }

                let api_error = parse_provider_api_error(&response_body);
                if pre_status_error(&api_error) {
                    continue;
                }
                if let Some(next_mode) = adapt_payload_mode_for_error(
                    payload_mode,
                    runtime.provider,
                    runtime.runtime_contract,
                    &api_error,
                ) && !tried_payload_modes.contains(&next_mode)
                {
                    payload_mode = next_mode;
                    tried_payload_modes.push(next_mode);
                    continue;
                }

                let status_code = status.as_u16();
                match plan_model_status_outcome(
                    status_code,
                    &response_headers,
                    &api_error,
                    attempt,
                    runtime.request_policy,
                    backoff_ms,
                    runtime.auto_model_mode,
                    runtime.runtime_contract,
                    runtime.capability,
                ) {
                    ModelStatusOutcome::Retry {
                        delay_ms,
                        next_backoff_ms,
                    } => {
                        sleep(Duration::from_millis(delay_ms)).await;
                        backoff_ms = next_backoff_ms;
                        continue;
                    }
                    ModelStatusOutcome::TryNextModel => {
                        return Err(build_model_request_error(
                            format!(
                                "model `{}` rejected by provider endpoint; trying next candidate. status {status_code}: {response_body}",
                                runtime.model
                            ),
                            true,
                            ProviderFailoverReason::ModelMismatch,
                            ProviderFailoverStage::ModelCandidateRejected,
                            runtime.model,
                            attempt,
                            runtime.request_policy.max_attempts,
                            Some(status_code),
                            Some(api_error.clone()),
                        ));
                    }
                    ModelStatusOutcome::Fail { reason } => {
                        return Err(build_model_request_error(
                            render_status_failure_message(
                                runtime.provider,
                                reason,
                                status_code,
                                runtime.model,
                                attempt,
                                runtime.request_policy.max_attempts,
                                &response_body,
                            ),
                            false,
                            reason,
                            ProviderFailoverStage::StatusFailure,
                            runtime.model,
                            attempt,
                            runtime.request_policy.max_attempts,
                            Some(status_code),
                            Some(api_error.clone()),
                        ));
                    }
                }
            }
            Err(error) => {
                if let Some((retry_delay_ms, next_backoff_ms)) =
                    plan_transport_error_retry(attempt, runtime.request_policy, &error, backoff_ms)
                {
                    sleep(Duration::from_millis(retry_delay_ms)).await;
                    backoff_ms = next_backoff_ms;
                    continue;
                }
                let error_message = error.to_string();
                let mut message = match error.reason() {
                    ProviderFailoverReason::ResponseDecodeFailure => format!(
                        "provider response decode failed for model `{}` on attempt {attempt}/{max_attempts}: {error_message}",
                        runtime.model,
                        max_attempts = runtime.request_policy.max_attempts
                    ),
                    ProviderFailoverReason::ResponseShapeInvalid => format!(
                        "provider response shape invalid for model `{}` on attempt {attempt}/{max_attempts}: {error_message}",
                        runtime.model,
                        max_attempts = runtime.request_policy.max_attempts
                    ),
                    ProviderFailoverReason::ModelMismatch
                    | ProviderFailoverReason::RateLimited
                    | ProviderFailoverReason::ProviderOverloaded
                    | ProviderFailoverReason::AuthRejected
                    | ProviderFailoverReason::PayloadIncompatible
                    | ProviderFailoverReason::TransportFailure
                    | ProviderFailoverReason::RequestRejected => format!(
                        "provider request failed for model `{}` on attempt {attempt}/{max_attempts}: {error_message}",
                        runtime.model,
                        max_attempts = runtime.request_policy.max_attempts
                    ),
                };
                if error.reason() == ProviderFailoverReason::TransportFailure
                    && let Some(route_hint) = transport::render_transport_route_hint(
                        request_endpoint.as_str(),
                        error_message.as_str(),
                        error.is_timeout(),
                        error.is_connect(),
                    )
                {
                    message.push(' ');
                    message.push_str(route_hint.as_str());
                }
                return Err(build_model_request_error(
                    message,
                    false,
                    error.reason(),
                    error.stage(),
                    runtime.model,
                    attempt,
                    runtime.request_policy.max_attempts,
                    None,
                    None,
                ));
            }
        }
    }
}

pub(super) async fn execute_streaming_model_request<T, BuildBody, ParseStreamItem>(
    runtime: StreamingModelRequestRuntime<'_>,
    mut build_body: BuildBody,
    parse_stream_item: ParseStreamItem,
) -> Result<impl Stream<Item = Result<T, ModelRequestError>>, ModelRequestError>
where
    T: Unpin,
    BuildBody: FnMut(CompletionPayloadMode) -> Value,
    ParseStreamItem: FnMut(Value) -> Option<T> + Unpin,
{
    let mut attempt = 0usize;
    let mut backoff_ms = runtime.request_policy.initial_backoff_ms;
    let mut payload_mode =
        CompletionPayloadMode::default_for_contract(runtime.provider, runtime.runtime_contract);
    let mut tried_payload_modes = vec![payload_mode];

    loop {
        attempt += 1;
        let body = build_body(payload_mode);
        let request_endpoint =
            transport::resolve_request_endpoint(runtime.provider, runtime.endpoint, runtime.model);
        let request_endpoint = transport::resolve_request_url(
            runtime.provider,
            request_endpoint.as_str(),
            runtime.auth_context,
        )
        .map_err(|error| {
            build_model_request_error(
                format!(
                    "provider request setup failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                    model = runtime.model,
                    max_attempts = runtime.request_policy.max_attempts
                ),
                false,
                ProviderFailoverReason::TransportFailure,
                ProviderFailoverStage::TransportFailure,
                runtime.model,
                attempt,
                runtime.request_policy.max_attempts,
                None,
                None,
            )
        })?;
        let body_bytes = transport::encode_json_request_body(&body).map_err(|error| {
            build_model_request_error(
                format!(
                    "provider request setup failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                    model = runtime.model,
                    max_attempts = runtime.request_policy.max_attempts
                ),
                false,
                ProviderFailoverReason::TransportFailure,
                ProviderFailoverStage::TransportFailure,
                runtime.model,
                attempt,
                runtime.request_policy.max_attempts,
                None,
                None,
            )
        })?;
        let mut headers = runtime.headers.clone();
        transport::apply_json_request_defaults(&mut headers).map_err(|error| {
            build_model_request_error(
                format!(
                    "provider request setup failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                    model = runtime.model,
                    max_attempts = runtime.request_policy.max_attempts
                ),
                false,
                ProviderFailoverReason::TransportFailure,
                ProviderFailoverStage::TransportFailure,
                runtime.model,
                attempt,
                runtime.request_policy.max_attempts,
                None,
                None,
            )
        })?;
        let request = transport::build_transport_request(
            reqwest::Method::POST,
            request_endpoint.clone(),
            headers,
            body_bytes,
            Some(runtime.auth_profile),
            runtime.request_auth_scheme,
        )
        .map_err(|error| {
            build_model_request_error(
                format!(
                    "provider request setup failed for model `{model}` on attempt {attempt}/{max_attempts}: {error}",
                    model = runtime.model,
                    max_attempts = runtime.request_policy.max_attempts
                ),
                false,
                ProviderFailoverReason::TransportFailure,
                ProviderFailoverStage::TransportFailure,
                runtime.model,
                attempt,
                runtime.request_policy.max_attempts,
                None,
                None,
            )
        })?;

        match runtime.transport.stream(request).await {
            Ok(TransportStream::Events { events }) => {
                let stream = ParsedTransportStream::new(events, parse_stream_item);
                return Ok(stream);
            }
            Ok(TransportStream::Response(response)) => {
                let status = response.status;
                let response_headers = response.headers;
                let response_body = response.body;

                let api_error = parse_provider_api_error(&response_body);
                if let Some(next_mode) = adapt_payload_mode_for_error(
                    payload_mode,
                    runtime.provider,
                    runtime.runtime_contract,
                    &api_error,
                ) && !tried_payload_modes.contains(&next_mode)
                {
                    payload_mode = next_mode;
                    tried_payload_modes.push(next_mode);
                    continue;
                }

                let status_code = status.as_u16();
                match plan_model_status_outcome(
                    status_code,
                    &response_headers,
                    &api_error,
                    attempt,
                    runtime.request_policy,
                    backoff_ms,
                    runtime.auto_model_mode,
                    runtime.runtime_contract,
                    runtime.capability,
                ) {
                    ModelStatusOutcome::Retry {
                        delay_ms,
                        next_backoff_ms,
                    } => {
                        sleep(Duration::from_millis(delay_ms)).await;
                        backoff_ms = next_backoff_ms;
                        continue;
                    }
                    ModelStatusOutcome::TryNextModel => {
                        return Err(build_model_request_error(
                            format!(
                                "model `{}` rejected by provider endpoint; trying next candidate. status {status_code}: {response_body}",
                                runtime.model
                            ),
                            true,
                            ProviderFailoverReason::ModelMismatch,
                            ProviderFailoverStage::ModelCandidateRejected,
                            runtime.model,
                            attempt,
                            runtime.request_policy.max_attempts,
                            Some(status_code),
                            Some(api_error.clone()),
                        ));
                    }
                    ModelStatusOutcome::Fail { reason } => {
                        return Err(build_model_request_error(
                            render_status_failure_message(
                                runtime.provider,
                                reason,
                                status_code,
                                runtime.model,
                                attempt,
                                runtime.request_policy.max_attempts,
                                &response_body,
                            ),
                            false,
                            reason,
                            ProviderFailoverStage::StatusFailure,
                            runtime.model,
                            attempt,
                            runtime.request_policy.max_attempts,
                            Some(status_code),
                            Some(api_error.clone()),
                        ));
                    }
                }
            }
            Err(error) => {
                if let Some((retry_delay_ms, next_backoff_ms)) =
                    plan_transport_error_retry(attempt, runtime.request_policy, &error, backoff_ms)
                {
                    sleep(Duration::from_millis(retry_delay_ms)).await;
                    backoff_ms = next_backoff_ms;
                    continue;
                }
                let error_message = error.to_string();
                let message = match error.reason() {
                    ProviderFailoverReason::ResponseDecodeFailure => format!(
                        "provider response decode failed for model `{}` on attempt {attempt}/{max_attempts}: {error_message}",
                        runtime.model,
                        max_attempts = runtime.request_policy.max_attempts
                    ),
                    ProviderFailoverReason::ResponseShapeInvalid => format!(
                        "provider response shape invalid for model `{}` on attempt {attempt}/{max_attempts}: {error_message}",
                        runtime.model,
                        max_attempts = runtime.request_policy.max_attempts
                    ),
                    ProviderFailoverReason::ModelMismatch
                    | ProviderFailoverReason::RateLimited
                    | ProviderFailoverReason::ProviderOverloaded
                    | ProviderFailoverReason::AuthRejected
                    | ProviderFailoverReason::PayloadIncompatible
                    | ProviderFailoverReason::TransportFailure
                    | ProviderFailoverReason::RequestRejected => {
                        let mut message = format!(
                            "provider request failed for model `{}` on attempt {attempt}/{max_attempts}: {error_message}",
                            runtime.model,
                            max_attempts = runtime.request_policy.max_attempts
                        );
                        if let Some(route_hint) = transport::render_transport_route_hint(
                            request_endpoint.as_str(),
                            error_message.as_str(),
                            error.is_timeout(),
                            error.is_connect(),
                        ) {
                            message.push(' ');
                            message.push_str(route_hint.as_str());
                        }
                        message
                    }
                };
                return Err(build_model_request_error(
                    message,
                    false,
                    error.reason(),
                    error.stage(),
                    runtime.model,
                    attempt,
                    runtime.request_policy.max_attempts,
                    None,
                    None,
                ));
            }
        }
    }
}

struct ParsedTransportStream<T, ParseStreamItem> {
    event_stream: TransportEventStream,
    parse_stream_item: ParseStreamItem,
    pending: VecDeque<Result<T, ModelRequestError>>,
}

impl<T, ParseStreamItem> ParsedTransportStream<T, ParseStreamItem>
where
    ParseStreamItem: FnMut(Value) -> Option<T>,
{
    fn new(event_stream: TransportEventStream, parse_stream_item: ParseStreamItem) -> Self {
        Self {
            event_stream,
            parse_stream_item,
            pending: VecDeque::new(),
        }
    }
}

impl<T, ParseStreamItem> Stream for ParsedTransportStream<T, ParseStreamItem>
where
    T: Unpin,
    ParseStreamItem: FnMut(Value) -> Option<T> + Unpin,
{
    type Item = Result<T, ModelRequestError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(item) = this.pending.pop_front() {
                return Poll::Ready(Some(item));
            }
            match this.event_stream.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(data))) => {
                    let parse_fn = &mut this.parse_stream_item;
                    if let Some(item) = parse_fn(data) {
                        this.pending.push_back(Ok(item));
                    }
                }
                Poll::Ready(Some(Err(error))) => {
                    let message = if error.reason() == ProviderFailoverReason::TransportFailure {
                        format!("streaming response error: {error}")
                    } else {
                        error.to_string()
                    };
                    return Poll::Ready(Some(Err(build_model_request_error(
                        message,
                        false,
                        error.reason(),
                        error.stage(),
                        "",
                        1,
                        1,
                        None,
                        None,
                    ))));
                }
                Poll::Ready(None) => {
                    return Poll::Ready(None);
                }
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
    }
}

#[allow(clippy::result_large_err)]
pub(super) async fn execute_streaming_turn_request<PreStatusError>(
    runtime: StreamingModelRequestRuntime<'_>,
    build_body: impl FnMut(CompletionPayloadMode) -> Value + Unpin,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    _messages: &[Value],
    on_token: StreamingTokenCallback,
    mut pre_status_error: PreStatusError,
) -> Result<ProviderTurn, ModelRequestError>
where
    PreStatusError: FnMut(&ProviderApiError) -> bool,
{
    let model_name = runtime.model.to_owned();
    let stream = match execute_streaming_model_request(runtime, build_body, |data: Value| {
        let event_type = data.get("type").and_then(|v| v.as_str())?;
        if event_type == "content_block_start" {
            let content_block = data.get("content_block")?;
            if content_block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                let name = content_block
                    .get("name")
                    .and_then(|v| v.as_str())?
                    .to_owned();
                let id = content_block.get("id").and_then(|v| v.as_str())?.to_owned();
                let index = data.get("index").and_then(|v| v.as_u64())? as usize;
                return Some(StreamingEvent::ToolCallStart { index, name, id });
            }
        }
        if event_type == "content_block_delta" {
            let delta = data.get("delta")?;
            let delta_type = delta.get("type").and_then(|v| v.as_str())?;
            if delta_type == "text_delta" {
                let text = delta.get("text").and_then(|v| v.as_str())?;
                return Some(StreamingEvent::Text(text.to_owned()));
            }
            if delta_type == "input_json_delta" {
                let partial = delta.get("partial_json").and_then(|v| v.as_str())?;
                let index = data.get("index").and_then(|v| v.as_u64())? as usize;
                return Some(StreamingEvent::ToolInputPartial {
                    index,
                    partial_json: partial.to_owned(),
                });
            }
        }
        if event_type == "message_stop" {
            return Some(StreamingEvent::Done);
        }
        if event_type == "message_start" || event_type == "message_delta" {
            return Some(StreamingEvent::Meta(data));
        }
        if event_type == "error" {
            let message = data
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown streaming error")
                .to_owned();
            return Some(StreamingEvent::StreamError(message));
        }
        None
    })
    .await
    {
        Ok(stream) => stream,
        Err(error) => {
            return Err(error);
        }
    };

    let mut accumulator = StreamingAccumulator::default();
    futures_util::pin_mut!(stream);
    while let Some(item) = futures_util::StreamExt::next(&mut stream).await {
        match item {
            Ok(StreamingEvent::Text(text)) => {
                if let Some(ref callback) = on_token {
                    callback(StreamingCallbackData::Text { text: text.clone() });
                }
                accumulator.text.push_str(&text);
            }
            Ok(StreamingEvent::ToolCallStart { index, name, id }) => {
                if let Some(ref callback) = on_token {
                    callback(StreamingCallbackData::ToolCallStart {
                        index,
                        name: name.clone(),
                        id: id.clone(),
                    });
                }
                accumulator.tool_calls.insert(
                    index,
                    ToolCallInfo {
                        name,
                        id,
                        input: String::new(),
                    },
                );
            }
            Ok(StreamingEvent::ToolInputPartial {
                index,
                partial_json,
            }) => {
                if let Some(ref callback) = on_token {
                    callback(StreamingCallbackData::ToolCallInput {
                        index,
                        partial_json: partial_json.clone(),
                    });
                }
                if let Some(tool_call) = accumulator.tool_calls.get_mut(&index) {
                    tool_call.input.push_str(&partial_json);
                }
            }
            Ok(StreamingEvent::Meta(data)) => {
                // Merge message_start and message_delta metadata for raw_meta
                if let Some(obj) = data.as_object() {
                    if !accumulator.meta.is_object() {
                        accumulator.meta = serde_json::json!({});
                    }
                    if let Some(meta) = accumulator.meta.as_object_mut() {
                        for (k, v) in obj {
                            meta.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            Ok(StreamingEvent::StreamError(message)) => {
                accumulator.error = Some(build_model_request_error(
                    format!("Anthropic streaming error: {message}"),
                    false,
                    ProviderFailoverReason::ResponseShapeInvalid,
                    ProviderFailoverStage::ResponseDecode,
                    &model_name,
                    1,
                    1,
                    None,
                    None,
                ));
            }
            Ok(StreamingEvent::Done) => {
                accumulator.done = true;
            }
            Err(e) => {
                if let Some(api_error) = &e.api_error
                    && pre_status_error(api_error)
                {
                    return Err(e);
                }
                accumulator.error = Some(e);
            }
        }
        if accumulator.done || accumulator.error.is_some() {
            break;
        }
    }

    if let Some(error) = accumulator.error {
        return Err(error);
    }

    if !accumulator.done {
        return Err(build_model_request_error(
            "streaming response ended without message_stop event".to_owned(),
            false,
            ProviderFailoverReason::ResponseShapeInvalid,
            ProviderFailoverStage::ResponseDecode,
            &model_name,
            1,
            1,
            None,
            None,
        ));
    }

    let tool_intents = accumulator
        .tool_calls
        .values()
        .map(|tool_call| {
            let args_json = serde_json::from_str(&tool_call.input).map_err(|e| {
                build_model_request_error(
                    format!("failed to parse tool call input: {}", e),
                    false,
                    ProviderFailoverReason::ResponseShapeInvalid,
                    ProviderFailoverStage::ResponseDecode,
                    &model_name,
                    1,
                    1,
                    None,
                    None,
                )
            })?;
            Ok(ToolIntent {
                tool_name: tool_call.name.clone(),
                args_json,
                source: "provider_tool_call".to_owned(),
                session_id: session_id.unwrap_or("").to_owned(),
                turn_id: turn_id.unwrap_or("").to_owned(),
                tool_call_id: tool_call.id.clone(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ProviderTurn {
        assistant_text: accumulator.text,
        tool_intents,
        raw_meta: accumulator.meta,
    })
}

#[derive(Clone)]
pub(crate) struct ToolCallInfo {
    pub name: String,
    pub id: String,
    pub input: String,
}

#[derive(Default)]
pub(crate) struct StreamingAccumulator {
    text: String,
    tool_calls: std::collections::BTreeMap<usize, ToolCallInfo>,
    meta: Value,
    done: bool,
    error: Option<ModelRequestError>,
}

pub(crate) enum StreamingEvent {
    Text(String),
    ToolCallStart {
        index: usize,
        name: String,
        id: String,
    },
    ToolInputPartial {
        index: usize,
        partial_json: String,
    },
    Meta(Value),
    StreamError(String),
    Done,
}

#[derive(Clone)]
pub enum StreamingCallbackData {
    Text {
        text: String,
    },
    ToolCallStart {
        index: usize,
        name: String,
        id: String,
    },
    ToolCallInput {
        index: usize,
        partial_json: String,
    },
}

pub type StreamingTokenCallback = Option<Arc<dyn Fn(StreamingCallbackData) + Send + Sync>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::contracts::provider_runtime_contract;
    use serde_json::json;

    struct ModelStatusCase {
        status_code: u16,
        attempt: usize,
        auto_model_mode: bool,
        api_error: ProviderApiError,
        expected: ModelStatusOutcome,
    }

    #[test]
    fn plan_model_status_outcome_matrix_is_stable() {
        let provider = ProviderConfig::default();
        let request_policy = policy::ProviderRequestPolicy::from_config(&provider);
        let runtime_contract = provider_runtime_contract(&provider);
        let headers = reqwest::header::HeaderMap::new();
        let backoff_ms = request_policy.initial_backoff_ms;

        let cases = vec![
            ModelStatusCase {
                status_code: 429,
                attempt: 1,
                auto_model_mode: true,
                api_error: ProviderApiError::default(),
                expected: ModelStatusOutcome::Retry {
                    delay_ms: backoff_ms,
                    next_backoff_ms: policy::next_backoff_ms(
                        backoff_ms,
                        request_policy.max_backoff_ms,
                    ),
                },
            },
            ModelStatusCase {
                status_code: 404,
                attempt: 1,
                auto_model_mode: true,
                api_error: ProviderApiError {
                    code: Some("model_not_found".to_owned()),
                    ..ProviderApiError::default()
                },
                expected: ModelStatusOutcome::TryNextModel,
            },
            ModelStatusCase {
                status_code: 404,
                attempt: 1,
                auto_model_mode: false,
                api_error: ProviderApiError {
                    code: Some("model_not_found".to_owned()),
                    ..ProviderApiError::default()
                },
                expected: ModelStatusOutcome::Fail {
                    reason: ProviderFailoverReason::ModelMismatch,
                },
            },
            ModelStatusCase {
                status_code: 503,
                attempt: request_policy.max_attempts,
                auto_model_mode: true,
                api_error: ProviderApiError::default(),
                expected: ModelStatusOutcome::Fail {
                    reason: ProviderFailoverReason::ProviderOverloaded,
                },
            },
            ModelStatusCase {
                status_code: 400,
                attempt: 1,
                auto_model_mode: true,
                api_error: ProviderApiError {
                    message: Some("unsupported parameter: max_completion_tokens".to_owned()),
                    ..ProviderApiError::default()
                },
                expected: ModelStatusOutcome::Fail {
                    reason: ProviderFailoverReason::PayloadIncompatible,
                },
            },
        ];

        for case in cases {
            let observed = plan_model_status_outcome(
                case.status_code,
                &headers,
                &case.api_error,
                case.attempt,
                &request_policy,
                backoff_ms,
                case.auto_model_mode,
                runtime_contract,
                runtime_contract.capability,
            );
            assert_eq!(
                observed, case.expected,
                "unexpected status outcome for status={}, attempt={}, auto_mode={}, error={:?}",
                case.status_code, case.attempt, case.auto_model_mode, case.api_error
            );
        }
    }

    #[test]
    fn render_status_failure_message_includes_auth_guidance_for_auth_rejection() {
        let provider = ProviderConfig {
            kind: crate::config::ProviderKind::ByteplusCoding,
            ..ProviderConfig::default()
        };

        let message = render_status_failure_message(
            &provider,
            ProviderFailoverReason::AuthRejected,
            401,
            "doubao-seed-1-6-thinking-250715",
            1,
            3,
            &json!({
                "error": {
                    "message": "invalid api key"
                }
            }),
        );

        assert!(message.contains("BytePlus"));
        assert!(message.contains("BYTEPLUS_API_KEY"));
        assert!(message.contains("Authorization: Bearer <BYTEPLUS_API_KEY>"));
    }

    #[test]
    fn render_status_failure_message_includes_region_hint_for_auth_rejection() {
        let provider = ProviderConfig {
            kind: crate::config::ProviderKind::Minimax,
            ..ProviderConfig::default()
        };

        let message = render_status_failure_message(
            &provider,
            ProviderFailoverReason::AuthRejected,
            401,
            "MiniMax-M2.7",
            1,
            3,
            &json!({
                "error": {
                    "message": "invalid api key"
                }
            }),
        );

        assert!(message.contains("provider.base_url"));
        assert!(message.contains("https://api.minimax.io"));
        assert!(message.contains("https://api.minimaxi.com"));
    }

    #[test]
    fn render_status_failure_message_ignores_models_endpoint_override_for_request_auth_hint() {
        let mut provider = ProviderConfig {
            kind: crate::config::ProviderKind::Zai,
            ..ProviderConfig::default()
        };
        provider.set_models_endpoint(Some("https://open.bigmodel.cn/v1/models".to_owned()));

        let message = render_status_failure_message(
            &provider,
            ProviderFailoverReason::AuthRejected,
            401,
            "glm-4.5",
            1,
            3,
            &json!({
                "error": {
                    "message": "invalid api key"
                }
            }),
        );

        assert!(message.contains("provider.base_url"));
        assert!(!message.contains("provider.models_endpoint"));
        assert!(message.contains("https://api.z.ai"));
        assert!(message.contains("https://open.bigmodel.cn"));
    }

    #[test]
    fn sse_stream_event_assembles_anthropic_delta_correctly() {
        use crate::provider::sse::SseStreamEvent;
        let event_type = Some("content_block_delta".to_owned());
        let data_lines = vec!["{\"type\":\"text_delta\",\"text\":\"Hello\"}".to_owned()];
        let event = SseStreamEvent::from_sse_lines(event_type, &data_lines);

        match event {
            Ok(Some(SseStreamEvent::Message { data, event_type })) => {
                assert_eq!(event_type.as_deref(), Some("content_block_delta"));
                let data: &serde_json::Value = &data;
                assert_eq!(
                    data.get("type")
                        .and_then(|v: &serde_json::Value| v.as_str()),
                    Some("text_delta")
                );
                assert_eq!(
                    data.get("text")
                        .and_then(|v: &serde_json::Value| v.as_str()),
                    Some("Hello")
                );
            }
            other => panic!("expected SseStreamEvent::Message, got {:?}", other),
        }
    }

    #[test]
    fn streaming_accumulator_accumulates_text_deltas() {
        let mut accumulator = StreamingAccumulator::default();

        accumulator.text.push_str("Hello");
        assert_eq!(accumulator.text, "Hello");
        assert!(!accumulator.done);
        assert!(accumulator.error.is_none());

        accumulator.text.push_str(" World");
        assert_eq!(accumulator.text, "Hello World");

        accumulator.done = true;
        assert!(accumulator.done);
    }

    #[test]
    fn streaming_accumulator_accumulates_tool_input_partials() {
        let mut accumulator = StreamingAccumulator::default();

        accumulator.tool_calls.insert(
            0,
            ToolCallInfo {
                name: "get_weather".to_owned(),
                id: "call_123".to_owned(),
                input: "{\"location".to_owned(),
            },
        );
        accumulator.tool_calls.insert(
            1,
            ToolCallInfo {
                name: "other_tool".to_owned(),
                id: "call_456".to_owned(),
                input: "{\"arg".to_owned(),
            },
        );

        assert_eq!(accumulator.tool_calls.len(), 2);
        assert_eq!(
            accumulator.tool_calls.get(&0).map(|t| &t.name),
            Some(&"get_weather".to_owned())
        );
        assert_eq!(
            accumulator.tool_calls.get(&1).map(|t| &t.input),
            Some(&"{\"arg".to_owned())
        );
    }

    #[test]
    fn streaming_event_parsing_text_delta() {
        let data = json!({"type": "content_block_delta", "delta": {"type": "text_delta", "text": "Hello"}});

        let event_type = data.get("type").and_then(|v| v.as_str());
        let delta = data.get("delta");
        let delta_type = delta.and_then(|d| d.get("type")).and_then(|v| v.as_str());
        let text = delta.and_then(|d| d.get("text")).and_then(|v| v.as_str());

        assert_eq!(event_type, Some("content_block_delta"));
        assert_eq!(delta_type, Some("text_delta"));
        assert_eq!(text, Some("Hello"));
    }

    #[test]
    fn streaming_event_parsing_input_json_delta() {
        let data = json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "input_json_delta", "partial_json": "{\"location\":\"NYC\"}"}
        });

        let event_type = data.get("type").and_then(|v| v.as_str());
        let delta = data.get("delta");
        let delta_type = delta.and_then(|d| d.get("type")).and_then(|v| v.as_str());
        let partial_json = delta
            .and_then(|d| d.get("partial_json"))
            .and_then(|v| v.as_str());
        let index = data
            .get("index")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        assert_eq!(event_type, Some("content_block_delta"));
        assert_eq!(delta_type, Some("input_json_delta"));
        assert_eq!(partial_json, Some("{\"location\":\"NYC\"}"));
        assert_eq!(index, Some(0));
    }

    #[test]
    fn streaming_event_parsing_message_delta_stop() {
        let data = json!({"type": "message_delta", "delta": {"type": "message_stop"}});

        let event_type = data.get("type").and_then(|v| v.as_str());
        let delta = data.get("delta");
        let delta_type = delta.and_then(|d| d.get("type")).and_then(|v| v.as_str());

        assert_eq!(event_type, Some("message_delta"));
        assert_eq!(delta_type, Some("message_stop"));
    }

    #[test]
    fn streaming_event_to_token_event_conversion() {
        use crate::acp::StreamingTokenEvent;
        use crate::acp::TokenDelta;

        let text_event = StreamingTokenEvent {
            event_type: "content_block_delta".to_owned(),
            delta: TokenDelta {
                text: Some("Hello".to_owned()),
                tool_call: None,
            },
            index: None,
        };

        let json = serde_json::to_string(&text_event).expect("should serialize");
        assert!(json.contains("Hello"));
        assert!(json.contains("content_block_delta"));
    }

    #[test]
    fn streaming_token_event_serialize_for_cli() {
        use crate::acp::StreamingTokenEvent;
        use crate::acp::TokenDelta;
        use crate::acp::ToolCallDelta;

        let tool_event = StreamingTokenEvent {
            event_type: "content_block_delta".to_owned(),
            delta: TokenDelta {
                text: None,
                tool_call: Some(ToolCallDelta {
                    name: Some("get_weather".to_owned()),
                    args: Some("{\"location\":\"NYC\"}".to_owned()),
                    id: Some("call_123".to_owned()),
                }),
            },
            index: Some(0),
        };

        let json = serde_json::to_string(&tool_event).expect("should serialize");
        assert!(json.contains("get_weather"));
        assert!(json.contains("NYC"));
    }

    #[test]
    fn streaming_accumulator_with_callback() {
        let mut accumulator = StreamingAccumulator::default();

        accumulator.text.push_str("Hello");
        accumulator.text.push_str(" World");

        let final_text = accumulator.text.clone();
        assert_eq!(final_text, "Hello World");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execute_model_request_uses_mock_transport_without_http() {
        use crate::provider::mock_transport::MockTransport;
        use crate::provider::transport_trait::TransportResponse;

        let provider = ProviderConfig::default();
        let runtime_contract = provider_runtime_contract(&provider);
        let request_policy = policy::ProviderRequestPolicy::from_config(&provider);
        let headers = reqwest::header::HeaderMap::new();
        let auth_context = transport::RequestAuthContext::default();
        let auth_profiles =
            crate::provider::auth_profile_runtime::resolve_provider_auth_profiles(&provider);
        let auth_profile = auth_profiles.first().expect("auth profile");
        let transport = MockTransport::with_execute_responses([Ok(TransportResponse {
            status: reqwest::StatusCode::OK,
            headers: reqwest::header::HeaderMap::new(),
            body: json!({
                "choices": [{
                    "message": {
                        "content": "mocked completion"
                    }
                }]
            }),
        })]);
        let runtime = ModelRequestRuntime {
            provider: &provider,
            model: "gpt-5.4",
            runtime_contract,
            capability: runtime_contract.capability,
            auto_model_mode: false,
            auth_profile,
            request_auth_scheme: crate::config::ProviderAuthScheme::Bearer,
            endpoint: "https://api.openai.com/v1/chat/completions",
            headers: &headers,
            request_policy: &request_policy,
            transport: &transport,
            auth_context: &auth_context,
        };

        let result = execute_model_request(
            runtime,
            |_| {
                json!({
                    "messages": [{
                        "role": "user",
                        "content": "ping"
                    }]
                })
            },
            crate::provider::shape::extract_message_content,
            "choices[0].message.content",
            |_| false,
        )
        .await
        .expect("mock completion should succeed");

        assert_eq!(result, "mocked completion");
        let recorded_requests = transport.requests();
        assert_eq!(recorded_requests.len(), 1);
        assert_eq!(
            recorded_requests[0].url,
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(recorded_requests[0].method, reqwest::Method::POST);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execute_streaming_model_request_uses_mock_transport_events() {
        use crate::provider::mock_transport::MockTransport;
        use futures_util::StreamExt;

        let provider = ProviderConfig::default();
        let runtime_contract = provider_runtime_contract(&provider);
        let request_policy = policy::ProviderRequestPolicy::from_config(&provider);
        let headers = reqwest::header::HeaderMap::new();
        let auth_context = transport::RequestAuthContext::default();
        let auth_profiles =
            crate::provider::auth_profile_runtime::resolve_provider_auth_profiles(&provider);
        let auth_profile = auth_profiles.first().expect("auth profile");
        let transport = MockTransport::with_stream_events([Ok(vec![
            Ok(json!({
                "type": "content_block_delta",
                "delta": {
                    "type": "text_delta",
                    "text": "streamed hello"
                }
            })),
            Ok(json!({
                "type": "message_delta",
                "delta": {
                    "type": "message_stop"
                }
            })),
        ])]);
        let runtime = StreamingModelRequestRuntime {
            provider: &provider,
            model: "gpt-5.4",
            runtime_contract,
            capability: runtime_contract.capability,
            auto_model_mode: false,
            auth_profile,
            request_auth_scheme: crate::config::ProviderAuthScheme::Bearer,
            endpoint: "https://api.openai.com/v1/chat/completions",
            headers: &headers,
            request_policy: &request_policy,
            transport: &transport,
            auth_context: &auth_context,
        };

        let mut stream = execute_streaming_model_request(
            runtime,
            |_| json!({}),
            |data: Value| {
                data.get("delta")
                    .and_then(|delta| delta.get("text"))
                    .and_then(|text| text.as_str())
                    .map(str::to_owned)
            },
        )
        .await
        .expect("mock stream should succeed");

        let first = stream.next().await.expect("first streamed item");
        assert_eq!(first.expect("text event"), "streamed hello");
        assert!(stream.next().await.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execute_model_request_preserves_response_decode_failure_reason() {
        use crate::provider::mock_transport::MockTransport;

        let provider = ProviderConfig::default();
        let runtime_contract = provider_runtime_contract(&provider);
        let request_policy = policy::ProviderRequestPolicy::from_config(&provider);
        let headers = reqwest::header::HeaderMap::new();
        let auth_context = transport::RequestAuthContext::default();
        let auth_profiles =
            crate::provider::auth_profile_runtime::resolve_provider_auth_profiles(&provider);
        let auth_profile = auth_profiles.first().expect("auth profile");
        let transport = MockTransport::with_execute_responses([Err(
            crate::provider::transport_trait::TransportError::response_decode(
                "read response body failed",
            ),
        )]);
        let runtime = ModelRequestRuntime {
            provider: &provider,
            model: "gpt-5.4",
            runtime_contract,
            capability: runtime_contract.capability,
            auto_model_mode: false,
            auth_profile,
            request_auth_scheme: crate::config::ProviderAuthScheme::Bearer,
            endpoint: "https://api.openai.com/v1/chat/completions",
            headers: &headers,
            request_policy: &request_policy,
            transport: &transport,
            auth_context: &auth_context,
        };

        let error = execute_model_request(
            runtime,
            |_| json!({}),
            |_| Some("ok".to_owned()),
            "choices[0].message.content",
            |_| false,
        )
        .await
        .expect_err("decode failures should surface as model request errors");

        assert_eq!(error.reason, ProviderFailoverReason::ResponseDecodeFailure);
        assert_eq!(error.snapshot.stage, ProviderFailoverStage::ResponseDecode);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn streaming_transport_errors_preserve_response_shape_invalid_reason() {
        use crate::provider::mock_transport::MockTransport;
        use futures_util::StreamExt;

        let provider = ProviderConfig::default();
        let runtime_contract = provider_runtime_contract(&provider);
        let request_policy = policy::ProviderRequestPolicy::from_config(&provider);
        let headers = reqwest::header::HeaderMap::new();
        let auth_context = transport::RequestAuthContext::default();
        let auth_profiles =
            crate::provider::auth_profile_runtime::resolve_provider_auth_profiles(&provider);
        let auth_profile = auth_profiles.first().expect("auth profile");
        let transport = MockTransport::with_stream_events([Ok(vec![Err(
            crate::provider::transport_trait::TransportError::response_shape_invalid(
                "invalid UTF-8 in SSE stream",
            ),
        )])]);
        let runtime = StreamingModelRequestRuntime {
            provider: &provider,
            model: "gpt-5.4",
            runtime_contract,
            capability: runtime_contract.capability,
            auto_model_mode: false,
            auth_profile,
            request_auth_scheme: crate::config::ProviderAuthScheme::Bearer,
            endpoint: "https://api.openai.com/v1/chat/completions",
            headers: &headers,
            request_policy: &request_policy,
            transport: &transport,
            auth_context: &auth_context,
        };

        let mut stream =
            execute_streaming_model_request(runtime, |_| json!({}), |_| Some("ok".to_owned()))
                .await
                .expect("stream should be created");

        let error = stream
            .next()
            .await
            .expect("stream item")
            .expect_err("malformed SSE should fail");

        assert_eq!(error.reason, ProviderFailoverReason::ResponseShapeInvalid);
        assert_eq!(error.snapshot.stage, ProviderFailoverStage::ResponseDecode);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execute_streaming_model_request_handles_response_branch_without_http() {
        use crate::provider::mock_transport::MockTransport;

        let provider = ProviderConfig::default();
        let runtime_contract = provider_runtime_contract(&provider);
        let request_policy = policy::ProviderRequestPolicy::from_config(&provider);
        let headers = reqwest::header::HeaderMap::new();
        let auth_context = transport::RequestAuthContext::default();
        let auth_profiles =
            crate::provider::auth_profile_runtime::resolve_provider_auth_profiles(&provider);
        let auth_profile = auth_profiles.first().expect("auth profile");
        let transport = MockTransport::with_stream_response(
            crate::provider::transport_trait::TransportResponse {
                status: reqwest::StatusCode::BAD_REQUEST,
                headers: reqwest::header::HeaderMap::new(),
                body: json!({
                    "error": {
                        "message": "bad request"
                    }
                }),
            },
        );
        let runtime = StreamingModelRequestRuntime {
            provider: &provider,
            model: "gpt-5.4",
            runtime_contract,
            capability: runtime_contract.capability,
            auto_model_mode: false,
            auth_profile,
            request_auth_scheme: crate::config::ProviderAuthScheme::Bearer,
            endpoint: "https://api.openai.com/v1/chat/completions",
            headers: &headers,
            request_policy: &request_policy,
            transport: &transport,
            auth_context: &auth_context,
        };

        let error =
            execute_streaming_model_request(runtime, |_| json!({}), |_| Some("ok".to_owned()))
                .await
                .err()
                .expect("non-event transport response should fail");

        assert_eq!(error.snapshot.stage, ProviderFailoverStage::StatusFailure);
        assert_eq!(error.snapshot.status_code, Some(400));
    }
}
