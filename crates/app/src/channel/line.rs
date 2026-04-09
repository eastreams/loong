use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::post};
use hmac::{KeyInit, Mac};
use serde_json::json;

use crate::{
    CliResult, KernelContext, config::ChannelDefaultAccountSelectionSource,
    config::LoongClawConfig, config::ResolvedLineChannelConfig,
};

use super::dispatch::{
    ChannelCommandContext, ChannelServeCommandSpec, process_inbound_with_provider,
    run_channel_serve_command_with_stop,
};
use super::{
    ChannelDelivery, ChannelInboundMessage, ChannelOutboundTarget, ChannelOutboundTargetKind,
    ChannelPlatform, ChannelSession, ChannelTurnFeedbackPolicy, LINE_COMMAND_FAMILY_DESCRIPTOR,
    http::{
        ChannelOutboundHttpPolicy, build_outbound_http_client, validate_outbound_http_base_url,
        validate_outbound_http_target,
    },
    runtime::serve::ChannelServeStopHandle,
    runtime::state::ChannelOperationRuntimeTracker,
};

type LineHmacSha256 = hmac::Hmac<sha2::Sha256>;

const LINE_SIGNATURE_HEADER: &str = "x-line-signature";

#[derive(Debug)]
enum LineServeError {
    Validation(String),
    Internal(String),
}

impl LineServeError {
    fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    fn message(&self) -> &str {
        match self {
            Self::Validation(message) => message,
            Self::Internal(message) => message,
        }
    }
}

#[derive(Clone)]
struct LineServeState {
    config: LoongClawConfig,
    resolved_path: PathBuf,
    resolved: ResolvedLineChannelConfig,
    channel_secret: String,
    kernel_ctx: Arc<KernelContext>,
    runtime: Arc<ChannelOperationRuntimeTracker>,
}

impl LineServeState {
    fn new(
        config: &LoongClawConfig,
        resolved_path: &Path,
        resolved: &ResolvedLineChannelConfig,
        kernel_ctx: KernelContext,
        runtime: Arc<ChannelOperationRuntimeTracker>,
    ) -> CliResult<Self> {
        let channel_secret = resolved.channel_secret().ok_or_else(|| {
            "line channel_secret missing (set line.channel_secret or env)".to_owned()
        })?;

        Ok(Self {
            config: config.clone(),
            resolved_path: resolved_path.to_path_buf(),
            resolved: resolved.clone(),
            channel_secret,
            kernel_ctx: Arc::new(kernel_ctx),
            runtime,
        })
    }
}

pub(super) async fn run_line_send(
    resolved: &ResolvedLineChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<()> {
    if target_kind != ChannelOutboundTargetKind::Address {
        return Err(format!(
            "line send requires address target kind, got {}",
            target_kind.as_str()
        ));
    }

    let channel_access_token = resolved.channel_access_token().ok_or_else(|| {
        "line channel_access_token missing (set line.channel_access_token or env)".to_owned()
    })?;
    let recipient = target_id.trim();
    if recipient.is_empty() {
        return Err("line outbound target id is empty".to_owned());
    }

    let raw_api_base_url = resolved.resolved_api_base_url();
    let api_base_url =
        validate_outbound_http_base_url("line api_base_url", raw_api_base_url.as_str(), policy)?;
    let trimmed_api_base_url = api_base_url.as_str().trim_end_matches('/');
    let request_url = format!("{trimmed_api_base_url}/message/push");
    let request_url =
        validate_outbound_http_target("line api_base_url", request_url.as_str(), policy)?;
    let request_body = json!({
        "to": recipient,
        "messages": [
            {
                "type": "text",
                "text": text,
            }
        ],
    });

    let client = build_outbound_http_client("line send", policy)?;
    let request = client
        .post(request_url)
        .bearer_auth(channel_access_token)
        .json(&request_body);
    let response = request
        .send()
        .await
        .map_err(|error| format!("line send failed: {error}"))?;

    ensure_line_success(response).await
}

#[allow(clippy::print_stdout)] // CLI startup banner
pub(super) async fn run_line_channel(
    config: &LoongClawConfig,
    resolved: &ResolvedLineChannelConfig,
    resolved_path: &Path,
    selected_by_default: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    bind_override: Option<&str>,
    path_override: Option<&str>,
    kernel_ctx: KernelContext,
    runtime: Arc<ChannelOperationRuntimeTracker>,
    stop: ChannelServeStopHandle,
) -> CliResult<()> {
    let bind = resolve_line_bind(bind_override)?;
    let path = resolve_line_path(path_override);
    let state = LineServeState::new(config, resolved_path, resolved, kernel_ctx, runtime)?;
    let router = Router::new()
        .route(path.as_str(), post(line_webhook_handler))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(bind.as_str())
        .await
        .map_err(|error| format!("bind line webhook listener failed: {error}"))?;

    println!(
        "line channel started (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, bind={}, path={})",
        resolved_path.display(),
        resolved.configured_account_id,
        resolved.account.label,
        selected_by_default,
        default_account_source.as_str(),
        bind,
        path
    );

    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            stop.wait().await;
        })
        .await
        .map_err(|error| format!("line webhook server stopped: {error}"))
}

pub(super) async fn run_line_channel_with_context(
    context: ChannelCommandContext<ResolvedLineChannelConfig>,
    bind_override: Option<&str>,
    path_override: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    let bind_override = bind_override.map(str::to_owned);
    let path_override = path_override.map(str::to_owned);

    run_channel_serve_command_with_stop(
        context,
        ChannelServeCommandSpec {
            family: LINE_COMMAND_FAMILY_DESCRIPTOR,
        },
        validate_line_security_config,
        stop,
        initialize_runtime_environment,
        move |context, kernel_ctx, runtime, stop| {
            Box::pin(async move {
                let route = context.route.clone();
                let resolved_path = context.resolved_path.clone();
                let resolved = context.resolved.clone();
                let config = context.config.clone();

                run_line_channel(
                    &config,
                    &resolved,
                    &resolved_path,
                    route.selected_by_default(),
                    route.default_account_source,
                    bind_override.as_deref(),
                    path_override.as_deref(),
                    kernel_ctx,
                    runtime,
                    stop,
                )
                .await
            })
        },
    )
    .await
}

async fn line_webhook_handler(
    State(state): State<LineServeState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let signature_result = verify_line_signature(&state, &headers, body.as_ref());
    if let Err(error) = signature_result {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response();
    }

    let process_result = process_line_webhook(&state, body).await;
    match process_result {
        Ok(()) => StatusCode::OK.into_response(),
        Err(LineServeError::Validation(error)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
        Err(LineServeError::Internal(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

fn resolve_line_bind(bind_override: Option<&str>) -> CliResult<String> {
    let bind = bind_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "line serve requires --bind because line config does not define a local listener address"
                .to_owned()
        })?;

    Ok(bind.to_owned())
}

fn resolve_line_path(path_override: Option<&str>) -> String {
    let explicit_path = path_override
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match explicit_path {
        Some(explicit_path) if explicit_path.starts_with('/') => explicit_path.to_owned(),
        Some(explicit_path) => format!("/{explicit_path}"),
        None => "/".to_owned(),
    }
}

fn validate_line_security_config(config: &ResolvedLineChannelConfig) -> CliResult<()> {
    let has_channel_access_token = config
        .channel_access_token()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_channel_access_token {
        return Err(
            "line channel_access_token is required for webhook replies (set line.channel_access_token or env)"
                .to_owned(),
        );
    }

    let has_channel_secret = config
        .channel_secret()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_channel_secret {
        return Err(
            "line channel_secret is required for webhook signature verification (set line.channel_secret or env)"
                .to_owned(),
        );
    }

    Ok(())
}

fn verify_line_signature(
    state: &LineServeState,
    headers: &HeaderMap,
    body: &[u8],
) -> CliResult<()> {
    let provided_signature = headers
        .get(LINE_SIGNATURE_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "missing X-Line-Signature header".to_owned())?;
    let expected_signature = build_line_signature(&state.channel_secret, body)?;
    let valid_length = expected_signature.len() == provided_signature.len();
    let valid_bytes =
        crate::crypto::timing_safe_eq(expected_signature.as_bytes(), provided_signature.as_bytes());
    if !valid_length || !valid_bytes {
        return Err("invalid X-Line-Signature header".to_owned());
    }

    Ok(())
}

fn build_line_signature(channel_secret: &str, body: &[u8]) -> CliResult<String> {
    let mut mac = LineHmacSha256::new_from_slice(channel_secret.as_bytes())
        .map_err(|error| format!("build line webhook signature failed: {error}"))?;
    mac.update(body);
    let signature = mac.finalize().into_bytes();
    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        signature,
    ))
}

async fn process_line_webhook(state: &LineServeState, body: Bytes) -> Result<(), LineServeError> {
    let payload: serde_json::Value = serde_json::from_slice(body.as_ref()).map_err(|error| {
        LineServeError::validation(format!("parse line webhook payload failed: {error}"))
    })?;
    let events = payload
        .get("events")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .ok_or_else(|| LineServeError::validation("line webhook payload missing events array"))?;

    let mut processed_event_count = 0usize;
    let mut first_error: Option<LineServeError> = None;

    for (index, event) in events.iter().enumerate() {
        let event_type = event
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let event_id = event
            .get("webhookEventId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let event_result = process_line_event(state, event).await;
        match event_result {
            Ok(()) => {
                processed_event_count += 1;
            }
            Err(error) => {
                tracing::warn!(
                    event_index = index,
                    event_type,
                    event_id,
                    error = %error.message(),
                    "line webhook event processing failed"
                );
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }

    if processed_event_count > 0 {
        return Ok(());
    }

    if let Some(error) = first_error {
        return Err(error);
    }

    Ok(())
}

async fn process_line_event(
    state: &LineServeState,
    event: &serde_json::Value,
) -> Result<(), LineServeError> {
    let event_type = event
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if event_type != "message" {
        return Ok(());
    }

    let message = event
        .get("message")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| LineServeError::validation("line webhook message event missing message object"))?;
    let message_type = message
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if message_type != "text" {
        return Ok(());
    }

    let reply_token = event
        .get("replyToken")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| LineServeError::validation("line webhook event missing replyToken"))?;
    let inbound_text = message
        .get("text")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| LineServeError::validation("line webhook text message missing text"))?;
    let source = event
        .get("source")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| LineServeError::validation("line webhook event missing source object"))?;
    let conversation_id = resolve_line_conversation_id(source)
        .ok_or_else(|| LineServeError::validation("line webhook source missing address"))?;
    let participant_id = source
        .get("userId")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let source_message_id = message
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    state
        .runtime
        .mark_run_start()
        .await
        .map_err(|error| LineServeError::internal(format!("line runtime start failed: {error}")))?;

    let process_result = async {
        let session = ChannelSession::with_account(
            ChannelPlatform::Line,
            state.resolved.account.id.as_str(),
            conversation_id.as_str(),
        )
        .with_configured_account_id(state.resolved.configured_account_id.as_str());
        let session = match participant_id.as_deref() {
            Some(participant_id) => session.with_participant_id(participant_id),
            None => session,
        };
        let reply_target = ChannelOutboundTarget::new(
            ChannelPlatform::Line,
            ChannelOutboundTargetKind::Address,
            reply_token,
        );
        let channel_message = ChannelInboundMessage {
            session,
            reply_target,
            text: inbound_text.to_owned(),
            delivery: ChannelDelivery {
                ack_cursor: None,
                source_message_id,
                sender_principal_key: participant_id,
                thread_root_id: None,
                parent_message_id: None,
                resources: Vec::new(),
                feishu_callback: None,
            },
        };
        let reply = process_inbound_with_provider(
            &state.config,
            Some(state.resolved_path.as_path()),
            &channel_message,
            state.kernel_ctx.as_ref(),
            ChannelTurnFeedbackPolicy::final_trace_significant(),
        )
        .await
        .map_err(LineServeError::internal)?;

        send_line_reply(state, reply_token, reply.as_str())
            .await
            .map_err(LineServeError::internal)
    }
    .await;

    if let Err(error) = state.runtime.mark_run_end().await {
        tracing::warn!(error = %error, "line runtime end failed");
    }

    process_result
}

fn resolve_line_conversation_id(
    source: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let group_id = source
        .get("groupId")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if group_id.is_some() {
        return group_id;
    }

    let room_id = source
        .get("roomId")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if room_id.is_some() {
        return room_id;
    }

    source
        .get("userId")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

async fn send_line_reply(state: &LineServeState, reply_token: &str, text: &str) -> CliResult<()> {
    let channel_access_token = state.resolved.channel_access_token().ok_or_else(|| {
        "line channel_access_token missing (set line.channel_access_token or env)".to_owned()
    })?;
    let api_base_url = state.resolved.resolved_api_base_url();
    let trimmed_api_base_url = api_base_url.trim_end_matches('/');
    let request_url = format!("{trimmed_api_base_url}/message/reply");
    let request_url = validate_outbound_http_target(
        "line api_base_url",
        request_url.as_str(),
        super::http::outbound_http_policy_from_config(&state.config),
    )?;
    let request_body = json!({
        "replyToken": reply_token,
        "messages": [
            {
                "type": "text",
                "text": text,
            }
        ],
    });
    let client = build_outbound_http_client(
        "line reply",
        super::http::outbound_http_policy_from_config(&state.config),
    )?;
    let response = client
        .post(request_url)
        .bearer_auth(channel_access_token)
        .json(&request_body)
        .send()
        .await
        .map_err(|error| format!("line reply failed: {error}"))?;

    ensure_line_success(response).await
}

async fn ensure_line_success(response: reqwest::Response) -> CliResult<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let body = response
        .text()
        .await
        .map_err(|error| format!("read line error response failed: {error}"))?;
    let trimmed_body = body.trim();
    let detail = if trimmed_body.is_empty() {
        "empty response body".to_owned()
    } else {
        trimmed_body.to_owned()
    };

    Err(format!(
        "line send failed with status {}: {detail}",
        status.as_u16()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::runtime::state::start_channel_operation_runtime_tracker_for_test;
    use crate::context::{DEFAULT_TOKEN_TTL_S, bootstrap_test_kernel_context};
    use axum::body::to_bytes;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_resolved_line_config() -> ResolvedLineChannelConfig {
        let config: crate::config::LineChannelConfig = serde_json::from_value(serde_json::json!({
            "enabled": true,
            "account_id": "line-main",
            "channel_access_token": "line-access-token",
            "channel_secret": "line-channel-secret"
        }))
        .expect("deserialize line config");

        config
            .resolve_account(None)
            .expect("resolve line config for tests")
    }

    fn temp_line_test_dir(label: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("loongclaw-line-test-{label}-{timestamp}"))
    }

    async fn build_test_serve_state() -> LineServeState {
        let runtime_dir = temp_line_test_dir("state");
        let runtime = start_channel_operation_runtime_tracker_for_test(
            runtime_dir.as_path(),
            ChannelPlatform::Line,
            "serve",
            "line-test",
            "line:test",
            std::process::id(),
        )
        .await
        .expect("start line runtime tracker");
        let kernel_ctx = bootstrap_test_kernel_context("line-serve-test", DEFAULT_TOKEN_TTL_S)
            .expect("bootstrap line kernel context");
        let resolved = test_resolved_line_config();

        LineServeState::new(
            &LoongClawConfig::default(),
            Path::new("/tmp/loongclaw.toml"),
            &resolved,
            kernel_ctx,
            runtime.into(),
        )
        .expect("build line serve state")
    }

    #[tokio::test]
    async fn line_webhook_handler_rejects_missing_signature() {
        let state = build_test_serve_state().await;
        let response =
            line_webhook_handler(State(state), HeaderMap::new(), Bytes::from_static(br#"{}"#))
                .await
                .into_response();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read line response body");
        let payload: serde_json::Value =
            serde_json::from_slice(&body).expect("parse line error payload");

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            payload,
            serde_json::json!({ "error": "missing X-Line-Signature header" })
        );
    }

    #[tokio::test]
    async fn line_webhook_handler_rejects_invalid_signature() {
        let state = build_test_serve_state().await;
        let mut headers = HeaderMap::new();
        headers.insert(
            LINE_SIGNATURE_HEADER,
            "bad-signature".parse().expect("header"),
        );
        let response = line_webhook_handler(State(state), headers, Bytes::from_static(br#"{}"#))
            .await
            .into_response();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read line response body");
        let payload: serde_json::Value =
            serde_json::from_slice(&body).expect("parse line error payload");

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            payload,
            serde_json::json!({ "error": "invalid X-Line-Signature header" })
        );
    }
}
