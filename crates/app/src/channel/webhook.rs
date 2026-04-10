use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::post};
use hmac::{KeyInit, Mac};
use reqwest::header::{CONTENT_TYPE, HeaderName, HeaderValue};
use serde_json::{Map, Value};

use crate::{
    CliResult, KernelContext,
    config::ChannelDefaultAccountSelectionSource,
    config::{ResolvedWebhookChannelConfig, WebhookPayloadFormat},
};

use super::{
    ChannelDelivery, ChannelInboundMessage, ChannelOutboundTarget, ChannelOutboundTargetKind,
    ChannelPlatform, ChannelSession, ChannelTurnFeedbackPolicy, WEBHOOK_COMMAND_FAMILY_DESCRIPTOR,
    core::webhook_auth::build_webhook_auth_header_from_parts,
    dispatch::{
        ChannelCommandContext, ChannelServeCommandSpec, process_inbound_with_provider,
        run_channel_serve_command_with_stop,
    },
    http::{
        ChannelOutboundHttpPolicy, build_outbound_http_client, response_body_detail,
        validate_outbound_http_target,
    },
    runtime::serve::ChannelServeStopHandle,
    runtime::state::ChannelOperationRuntimeTracker,
};
use crate::config::LoongClawConfig;

const WEBHOOK_JSON_CONTENT_TYPE: &str = "application/json";
const WEBHOOK_TEXT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";
const WEBHOOK_SIGNATURE_HEADER: &str = "x-loongclaw-signature-256";
const WEBHOOK_SIGNATURE_PREFIX: &str = "sha256=";

#[derive(Debug)]
enum WebhookServeError {
    Validation(String),
    Internal(String),
}

type WebhookHmacSha256 = hmac::Hmac<sha2::Sha256>;

struct WebhookRequestBody {
    content_type: &'static str,
    body: Vec<u8>,
}

#[derive(Clone)]
struct WebhookServeState {
    config: LoongClawConfig,
    resolved_path: PathBuf,
    resolved: ResolvedWebhookChannelConfig,
    expected_auth_header: Option<(HeaderName, HeaderValue)>,
    signing_secret: String,
    kernel_ctx: Arc<KernelContext>,
    runtime: Arc<ChannelOperationRuntimeTracker>,
}

enum WebhookServeError {
    Validation(String),
    Internal(String),
}

impl WebhookServeError {
    fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
}

impl WebhookServeState {
    fn new(
        config: &LoongClawConfig,
        resolved_path: &Path,
        resolved: &ResolvedWebhookChannelConfig,
        kernel_ctx: KernelContext,
        runtime: Arc<ChannelOperationRuntimeTracker>,
    ) -> CliResult<Self> {
        let expected_auth_header = build_webhook_auth_header(resolved)?;
        let signing_secret = resolved.signing_secret().ok_or_else(|| {
            "webhook signing_secret is required for serve runtime (set webhook.signing_secret or env)"
                .to_owned()
        })?;

        Ok(Self {
            config: config.clone(),
            resolved_path: resolved_path.to_path_buf(),
            resolved: resolved.clone(),
            expected_auth_header,
            signing_secret,
            kernel_ctx: Arc::new(kernel_ctx),
            runtime,
        })
    }
}

pub(super) async fn run_webhook_send(
    resolved: &ResolvedWebhookChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    endpoint_url: &str,
    text: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<()> {
    ensure_webhook_target_kind(target_kind)?;

    let request_url = parse_webhook_endpoint_url(endpoint_url, policy)?;
    let request_body = build_webhook_request_body(resolved, text)?;
    let auth_header = build_webhook_auth_header(resolved)?;

    let client = build_outbound_http_client("webhook send", policy)?;
    let mut request = client
        .post(request_url)
        .header(CONTENT_TYPE, request_body.content_type)
        .body(request_body.body);

    if let Some((header_name, header_value)) = auth_header {
        request = request.header(header_name, header_value);
    }

    let response = request
        .send()
        .await
        .map_err(|error| format!("webhook send failed: {error}"))?;

    ensure_webhook_success(response).await
}

fn ensure_webhook_target_kind(target_kind: ChannelOutboundTargetKind) -> CliResult<()> {
    if target_kind == ChannelOutboundTargetKind::Endpoint {
        return Ok(());
    }

    Err(format!(
        "webhook send requires endpoint target kind, got {}",
        target_kind.as_str()
    ))
}

fn parse_webhook_endpoint_url(
    endpoint_url: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<reqwest::Url> {
    validate_outbound_http_target("webhook outbound target endpoint", endpoint_url, policy)
}

fn build_webhook_request_body(
    resolved: &ResolvedWebhookChannelConfig,
    text: &str,
) -> CliResult<WebhookRequestBody> {
    match resolved.payload_format {
        WebhookPayloadFormat::JsonText => build_webhook_json_request_body(resolved, text),
        WebhookPayloadFormat::PlainText => build_webhook_plain_text_request_body(text),
    }
}

fn build_webhook_json_request_body(
    resolved: &ResolvedWebhookChannelConfig,
    text: &str,
) -> CliResult<WebhookRequestBody> {
    let request_json = build_webhook_json_payload(resolved, text)?;
    let request_bytes = serde_json::to_vec(&request_json)
        .map_err(|error| format!("serialize webhook json payload failed: {error}"))?;

    Ok(WebhookRequestBody {
        content_type: WEBHOOK_JSON_CONTENT_TYPE,
        body: request_bytes,
    })
}

fn build_webhook_plain_text_request_body(text: &str) -> CliResult<WebhookRequestBody> {
    let request_text = text.to_owned();
    let request_bytes = request_text.into_bytes();

    Ok(WebhookRequestBody {
        content_type: WEBHOOK_TEXT_CONTENT_TYPE,
        body: request_bytes,
    })
}

fn build_webhook_json_payload(
    resolved: &ResolvedWebhookChannelConfig,
    text: &str,
) -> CliResult<Value> {
    let field_name = resolved.payload_text_field.trim();
    if field_name.is_empty() {
        return Err("webhook payload_text_field is empty for json_text payload format".to_owned());
    }

    let mut payload = Map::new();
    let text_value = Value::String(text.to_owned());
    payload.insert(field_name.to_owned(), text_value);

    Ok(Value::Object(payload))
}

fn build_webhook_auth_header(
    resolved: &ResolvedWebhookChannelConfig,
) -> CliResult<Option<(HeaderName, HeaderValue)>> {
    let auth_token = resolved.auth_token();
    let auth_token = auth_token.as_deref();
    let auth_header_name = resolved.auth_header_name.as_str();
    let auth_token_prefix = resolved.auth_token_prefix.as_str();

    build_webhook_auth_header_from_parts(auth_token, auth_header_name, auth_token_prefix)
}

async fn ensure_webhook_success(response: reqwest::Response) -> CliResult<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let body = response
        .text()
        .await
        .map_err(|error| format!("read webhook error response failed: {error}"))?;
    let detail = response_body_detail(body.as_str());

    Err(format!(
        "webhook send failed with status {}: {detail}",
        status.as_u16()
    ))
}

#[allow(clippy::print_stdout)] // CLI startup banner
pub(super) async fn run_webhook_channel(
    config: &LoongClawConfig,
    resolved: &ResolvedWebhookChannelConfig,
    resolved_path: &Path,
    selected_by_default: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    bind_override: Option<&str>,
    path_override: Option<&str>,
    kernel_ctx: KernelContext,
    runtime: Arc<ChannelOperationRuntimeTracker>,
    stop: ChannelServeStopHandle,
) -> CliResult<()> {
    let bind = resolve_webhook_bind(bind_override)?;
    let path = resolve_webhook_path(resolved, path_override)?;
    let state = WebhookServeState::new(config, resolved_path, resolved, kernel_ctx, runtime)?;
    let router = Router::new()
        .route(path.as_str(), post(webhook_serve_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind.as_str())
        .await
        .map_err(|error| format!("bind webhook listener failed: {error}"))?;

    println!(
        "webhook channel started (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, bind={}, path={})",
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
        .map_err(|error| format!("webhook server stopped: {error}"))
}

pub(super) async fn run_webhook_channel_with_context(
    context: ChannelCommandContext<ResolvedWebhookChannelConfig>,
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
            family: WEBHOOK_COMMAND_FAMILY_DESCRIPTOR,
        },
        validate_webhook_security_config,
        stop,
        initialize_runtime_environment,
        move |context, kernel_ctx, runtime, stop| {
            Box::pin(async move {
                let route = context.route.clone();
                let resolved_path = context.resolved_path.clone();
                let resolved = context.resolved.clone();
                let config = context.config.clone();

                run_webhook_channel(
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

async fn webhook_serve_handler(
    State(state): State<WebhookServeState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let verification = verify_webhook_request(&state, &headers, body.as_ref());
    if let Err(error) = verification {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response();
    }

    let process_result = process_webhook_request(&state, body).await;
    match process_result {
        Ok(response) => response,
        Err(WebhookServeError::Validation(error)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
        Err(WebhookServeError::Internal(error)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

fn resolve_webhook_bind(bind_override: Option<&str>) -> CliResult<String> {
    let bind = bind_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "webhook serve requires --bind because webhook config does not define a local listener address"
                .to_owned()
        })?;

    Ok(bind.to_owned())
}

fn resolve_webhook_path(
    resolved: &ResolvedWebhookChannelConfig,
    path_override: Option<&str>,
) -> CliResult<String> {
    let explicit_path = path_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(explicit_path) = explicit_path {
        return Ok(normalize_webhook_path(explicit_path.as_str()));
    }

    let public_base_url = resolved
        .public_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(public_base_url) = public_base_url {
        let public_url = reqwest::Url::parse(public_base_url)
            .map_err(|error| format!("webhook public_base_url is invalid: {error}"))?;
        let public_path = public_url.path();
        if !public_path.trim().is_empty() && public_path != "/" {
            return Ok(normalize_webhook_path(public_path));
        }
    }

    Ok("/".to_owned())
}

fn normalize_webhook_path(raw_path: &str) -> String {
    let trimmed_path = raw_path.trim();
    if trimmed_path.starts_with('/') {
        return trimmed_path.to_owned();
    }

    format!("/{trimmed_path}")
}

fn validate_webhook_security_config(config: &ResolvedWebhookChannelConfig) -> CliResult<()> {
    let has_signing_secret = config
        .signing_secret()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_signing_secret {
        return Err(
            "webhook signing_secret is required for serve runtime (set webhook.signing_secret or env)"
                .to_owned(),
        );
    }

    let auth_header = build_webhook_auth_header(config)?;
    let _ = auth_header;
    Ok(())
}

fn verify_webhook_request(
    state: &WebhookServeState,
    headers: &HeaderMap,
    body: &[u8],
) -> CliResult<()> {
    verify_webhook_auth_header(state, headers)?;
    verify_webhook_signature(state, headers, body)?;
    Ok(())
}

fn verify_webhook_auth_header(state: &WebhookServeState, headers: &HeaderMap) -> CliResult<()> {
    let Some((expected_name, expected_value)) = state.expected_auth_header.as_ref() else {
        return Ok(());
    };
    let incoming_value = headers
        .get(expected_name)
        .ok_or_else(|| "missing auth header".to_owned())?;
    if incoming_value.as_bytes() != expected_value.as_bytes() {
        return Err("invalid auth header".to_owned());
    }

    Ok(())
}

fn verify_webhook_signature(
    state: &WebhookServeState,
    headers: &HeaderMap,
    body: &[u8],
) -> CliResult<()> {
    let provided_signature = headers
        .get(WEBHOOK_SIGNATURE_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "missing signature header".to_owned())?;
    let provided_hex = provided_signature
        .strip_prefix(WEBHOOK_SIGNATURE_PREFIX)
        .ok_or_else(|| format!("signature must start with `{WEBHOOK_SIGNATURE_PREFIX}`"))?;
    let expected_hex = build_webhook_signature_hex(state.signing_secret.as_str(), body)?;
    let valid_length = expected_hex.len() == provided_hex.len();
    let valid_bytes =
        crate::crypto::timing_safe_eq(expected_hex.as_bytes(), provided_hex.as_bytes());
    if !valid_length || !valid_bytes {
        return Err("invalid signature".to_owned());
    }

    Ok(())
}

fn build_webhook_signature_hex(signing_secret: &str, body: &[u8]) -> CliResult<String> {
    let mut mac = WebhookHmacSha256::new_from_slice(signing_secret.as_bytes())
        .map_err(|error| format!("build webhook signature failed: {error}"))?;
    mac.update(body);
    let signature = mac.finalize().into_bytes();
    Ok(hex::encode(signature))
}

async fn process_webhook_request(
    state: &WebhookServeState,
    body: Bytes,
) -> Result<Response, WebhookServeError> {
    let inbound_message = build_webhook_inbound_message(state, body.as_ref())
        .map_err(WebhookServeError::validation)?;

    state.runtime.mark_run_start().await.map_err(|error| {
        WebhookServeError::internal(format!("webhook runtime start failed: {error}"))
    })?;

    let process_result = async {
        let reply = process_inbound_with_provider(
            &state.config,
            Some(state.resolved_path.as_path()),
            &inbound_message,
            state.kernel_ctx.as_ref(),
            ChannelTurnFeedbackPolicy::final_trace_significant(),
        )
        .await
        .map_err(WebhookServeError::internal)?;

        build_webhook_inline_response(state, reply.as_str()).map_err(WebhookServeError::internal)
    }
    .await;

    if let Err(error) = state.runtime.mark_run_end().await {
        tracing::warn!(error = %error, "webhook runtime end failed");
    }

    process_result
}

fn build_webhook_inbound_message(
    state: &WebhookServeState,
    body: &[u8],
) -> CliResult<ChannelInboundMessage> {
    let parsed_body = parse_webhook_inbound_payload(&state.resolved, body)?;
    let conversation_id = parsed_body
        .conversation_id
        .unwrap_or_else(|| state.resolved.account.id.clone());
    let session = ChannelSession::with_account(
        ChannelPlatform::Webhook,
        state.resolved.account.id.as_str(),
        conversation_id.as_str(),
    )
    .with_configured_account_id(state.resolved.configured_account_id.as_str());
    let session = match parsed_body.participant_id.as_deref() {
        Some(participant_id) => session.with_participant_id(participant_id),
        None => session,
    };
    let session = match parsed_body.thread_id.as_deref() {
        Some(thread_id) => session.with_thread_id(thread_id),
        None => session,
    };
    let reply_target = ChannelOutboundTarget::new(
        ChannelPlatform::Webhook,
        ChannelOutboundTargetKind::Endpoint,
        state.resolved.account.id.as_str(),
    );
    let sender_principal_key = parsed_body.participant_id.clone();
    let source_message_id = parsed_body.source_message_id.clone();

    Ok(ChannelInboundMessage {
        session,
        reply_target,
        text: parsed_body.text,
        delivery: ChannelDelivery {
            ack_cursor: None,
            source_message_id,
            sender_principal_key,
            thread_root_id: None,
            parent_message_id: None,
            resources: Vec::new(),
            feishu_callback: None,
        },
    })
}

struct ParsedWebhookInboundPayload {
    text: String,
    conversation_id: Option<String>,
    participant_id: Option<String>,
    thread_id: Option<String>,
    source_message_id: Option<String>,
}

fn parse_webhook_inbound_payload(
    resolved: &ResolvedWebhookChannelConfig,
    body: &[u8],
) -> CliResult<ParsedWebhookInboundPayload> {
    match resolved.payload_format {
        WebhookPayloadFormat::PlainText => parse_plain_text_webhook_payload(body),
        WebhookPayloadFormat::JsonText => parse_json_text_webhook_payload(resolved, body),
    }
}

fn parse_plain_text_webhook_payload(body: &[u8]) -> CliResult<ParsedWebhookInboundPayload> {
    let text = std::str::from_utf8(body)
        .map_err(|error| format!("webhook body is not valid utf-8: {error}"))?;
    let trimmed_text = text.trim();
    if trimmed_text.is_empty() {
        return Err("webhook text payload is empty".to_owned());
    }

    Ok(ParsedWebhookInboundPayload {
        text: trimmed_text.to_owned(),
        conversation_id: None,
        participant_id: None,
        thread_id: None,
        source_message_id: None,
    })
}

fn parse_json_text_webhook_payload(
    resolved: &ResolvedWebhookChannelConfig,
    body: &[u8],
) -> CliResult<ParsedWebhookInboundPayload> {
    let payload: Value = serde_json::from_slice(body)
        .map_err(|error| format!("parse webhook json payload failed: {error}"))?;
    let payload_object = payload
        .as_object()
        .ok_or_else(|| "webhook json payload must be an object".to_owned())?;
    let text_key = resolved.payload_text_field.trim();
    if text_key.is_empty() {
        return Err("webhook payload_text_field is empty for json_text payload format".to_owned());
    }
    let text = payload_object
        .get(text_key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("webhook json payload missing non-empty `{text_key}` field"))?;
    let conversation_id = optional_webhook_metadata_field(payload_object, "conversation_id");
    let participant_id = optional_webhook_metadata_field(payload_object, "participant_id");
    let thread_id = optional_webhook_metadata_field(payload_object, "thread_id");
    let source_message_id = optional_webhook_metadata_field(payload_object, "source_message_id");

    Ok(ParsedWebhookInboundPayload {
        text: text.to_owned(),
        conversation_id,
        participant_id,
        thread_id,
        source_message_id,
    })
}

fn optional_webhook_metadata_field(
    payload_object: &Map<String, Value>,
    field_name: &str,
) -> Option<String> {
    payload_object
        .get(field_name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn build_webhook_inline_response(state: &WebhookServeState, reply: &str) -> CliResult<Response> {
    match state.resolved.payload_format {
        WebhookPayloadFormat::PlainText => Ok((
            StatusCode::OK,
            [(CONTENT_TYPE, WEBHOOK_TEXT_CONTENT_TYPE)],
            reply.to_owned(),
        )
            .into_response()),
        WebhookPayloadFormat::JsonText => {
            let response_json = build_webhook_json_payload(&state.resolved, reply)?;
            Ok((StatusCode::OK, axum::Json(response_json)).into_response())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::runtime::state::start_channel_operation_runtime_tracker_for_test;
    use crate::config::WebhookChannelConfig;
    use crate::context::{DEFAULT_TOKEN_TTL_S, bootstrap_test_kernel_context};
    use axum::body::to_bytes;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_resolved_webhook_config(
        payload_format: WebhookPayloadFormat,
    ) -> ResolvedWebhookChannelConfig {
        let payload_format_raw = payload_format.as_str();
        let config: WebhookChannelConfig = serde_json::from_value(serde_json::json!({
            "enabled": true,
            "account_id": "Webhook Ops",
            "payload_format": payload_format_raw,
            "payload_text_field": "message"
        }))
        .expect("deserialize webhook config");

        config
            .resolve_account(None)
            .expect("resolve webhook config for tests")
    }

    fn temp_webhook_test_dir(label: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("loongclaw-webhook-test-{label}-{timestamp}"))
    }

    async fn build_test_serve_state(
        signing_secret: Option<&str>,
        auth_token: Option<&str>,
    ) -> WebhookServeState {
        let runtime_dir = temp_webhook_test_dir("state");
        let runtime = start_channel_operation_runtime_tracker_for_test(
            runtime_dir.as_path(),
            ChannelPlatform::Webhook,
            "serve",
            "webhook-test",
            "webhook:test",
            std::process::id(),
        )
        .await
        .expect("start runtime tracker");
        let kernel_ctx = bootstrap_test_kernel_context("webhook-serve-test", DEFAULT_TOKEN_TTL_S)
            .expect("bootstrap webhook kernel context");
        let mut resolved = test_resolved_webhook_config(WebhookPayloadFormat::JsonText);
        resolved.signing_secret = signing_secret.map(|value| {
            serde_json::from_value(serde_json::json!(value))
                .expect("deserialize signing secret for test state")
        });
        if let Some(auth_token) = auth_token {
            resolved.auth_token = Some(
                serde_json::from_value(serde_json::json!(auth_token))
                    .expect("deserialize auth token for test state"),
            );
        }

        WebhookServeState::new(
            &LoongClawConfig::default(),
            std::path::Path::new("/tmp/loongclaw.toml"),
            &resolved,
            kernel_ctx,
            runtime.into(),
        )
        .expect("build webhook serve state")
    }

    #[test]
    fn build_webhook_json_payload_uses_custom_text_field() {
        let resolved = test_resolved_webhook_config(WebhookPayloadFormat::JsonText);

        let payload = build_webhook_json_payload(&resolved, "hello webhook")
            .expect("build webhook json payload");

        assert_eq!(payload["message"].as_str(), Some("hello webhook"));
    }

    #[test]
    fn build_webhook_plain_text_request_body_returns_raw_text() {
        let request_body = build_webhook_plain_text_request_body("hello webhook")
            .expect("build webhook plain text request body");

        assert_eq!(request_body.content_type, WEBHOOK_TEXT_CONTENT_TYPE);
        assert_eq!(request_body.body, b"hello webhook".to_vec());
    }

    #[test]
    fn build_webhook_auth_header_rejects_invalid_header_name() {
        let mut resolved = test_resolved_webhook_config(WebhookPayloadFormat::JsonText);
        let auth_token =
            serde_json::from_value(serde_json::json!("token-123")).expect("deserialize auth token");
        resolved.auth_token = Some(auth_token);
        resolved.auth_header_name = "bad header".to_owned();

        let error =
            build_webhook_auth_header(&resolved).expect_err("invalid header name should fail");

        assert!(
            error.contains("webhook auth_header_name is invalid"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn build_webhook_auth_header_rejects_invalid_header_value() {
        let mut resolved = test_resolved_webhook_config(WebhookPayloadFormat::JsonText);
        let auth_token =
            serde_json::from_value(serde_json::json!("token-123")).expect("deserialize auth token");
        resolved.auth_token = Some(auth_token);
        resolved.auth_token_prefix = "Bearer\n".to_owned();

        let error =
            build_webhook_auth_header(&resolved).expect_err("invalid header value should fail");

        assert!(
            error.contains("webhook auth header value is invalid"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn build_webhook_json_payload_rejects_empty_text_field() {
        let mut resolved = test_resolved_webhook_config(WebhookPayloadFormat::JsonText);
        resolved.payload_text_field = "   ".to_owned();

        let error =
            build_webhook_json_payload(&resolved, "hello").expect_err("empty field should fail");

        assert!(
            error.contains("webhook payload_text_field is empty"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn resolve_webhook_path_uses_public_base_url_path_when_override_missing() {
        let mut resolved = test_resolved_webhook_config(WebhookPayloadFormat::JsonText);
        resolved.public_base_url = Some("https://hooks.example.test/inbound/webhook".to_owned());

        let path =
            resolve_webhook_path(&resolved, None).expect("resolve webhook path from public url");

        assert_eq!(path, "/inbound/webhook");
    }

    #[tokio::test]
    async fn webhook_serve_handler_rejects_missing_signature() {
        let state = build_test_serve_state(Some("signing-secret"), None).await;
        let response = webhook_serve_handler(
            State(state),
            HeaderMap::new(),
            Bytes::from_static(br#"{"message":"hello"}"#),
        )
        .await
        .into_response();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read webhook response body");
        let payload: Value = serde_json::from_slice(&body).expect("parse webhook error body");

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            payload,
            serde_json::json!({ "error": "missing signature header" })
        );
    }

    #[tokio::test]
    async fn webhook_serve_handler_rejects_invalid_auth_header() {
        let state = build_test_serve_state(Some("signing-secret"), Some("token-123")).await;
        let body = Bytes::from_static(br#"{"message":"hello"}"#);
        let signature = build_webhook_signature_hex("signing-secret", body.as_ref())
            .expect("build webhook test signature");
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer wrong-token"),
        );
        headers.insert(
            WEBHOOK_SIGNATURE_HEADER,
            HeaderValue::from_str(format!("{WEBHOOK_SIGNATURE_PREFIX}{signature}").as_str())
                .expect("signature header value"),
        );

        let response = webhook_serve_handler(State(state), headers, body)
            .await
            .into_response();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read webhook response body");
        let payload: Value = serde_json::from_slice(&body).expect("parse webhook error body");

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            payload,
            serde_json::json!({ "error": "invalid auth header" })
        );
    }

    #[tokio::test]
    async fn webhook_serve_handler_rejects_invalid_payloads_as_bad_requests() {
        let state = build_test_serve_state(Some("signing-secret"), None).await;
        let body = Bytes::from_static(br#"not-json"#);
        let signature = build_webhook_signature_hex("signing-secret", body.as_ref())
            .expect("build webhook test signature");
        let mut headers = HeaderMap::new();
        headers.insert(
            WEBHOOK_SIGNATURE_HEADER,
            HeaderValue::from_str(format!("{WEBHOOK_SIGNATURE_PREFIX}{signature}").as_str())
                .expect("signature header value"),
        );

        let response = webhook_serve_handler(State(state), headers, body)
            .await
            .into_response();
        let status = response.status();

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
