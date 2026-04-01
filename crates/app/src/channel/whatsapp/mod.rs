mod webhook;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{Router, routing::get};
use serde_json::{Value, json};

use super::dispatch::{
    ChannelCommandContext, ChannelResolvedRuntimeAccount, build_whatsapp_command_context,
    run_channel_serve_command_with_stop,
};
use super::serve_runtime::{ChannelServeCommandSpec, ChannelServeStopHandle};
use super::{ChannelOutboundTargetKind, WHATSAPP_COMMAND_FAMILY_DESCRIPTOR};
use super::{
    http::{ChannelOutboundHttpPolicy, build_outbound_http_client, validate_outbound_http_target},
    runtime_state::ChannelOperationRuntimeTracker,
};
use crate::config::{ChannelDefaultAccountSelectionSource, LoongClawConfig};
use crate::{CliResult, KernelContext, config::ResolvedWhatsappChannelConfig};
use webhook::{WhatsappWebhookState, whatsapp_verify_handler, whatsapp_webhook_handler};

pub(super) async fn run_whatsapp_send(
    resolved: &ResolvedWhatsappChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<()> {
    if target_kind != ChannelOutboundTargetKind::Address {
        return Err(format!(
            "whatsapp send requires address target kind, got {}",
            target_kind.as_str()
        ));
    }

    let access_token = resolved.access_token().ok_or_else(|| {
        "whatsapp access token missing (set whatsapp.access_token or env)".to_owned()
    })?;
    let phone_number_id = resolved.phone_number_id().ok_or_else(|| {
        "whatsapp phone_number_id missing (set whatsapp.phone_number_id or env)".to_owned()
    })?;
    let recipient = target_id.trim();
    if recipient.is_empty() {
        return Err("whatsapp outbound target id is empty".to_owned());
    }

    let api_base_url = resolved.resolved_api_base_url();
    let request_url = format!(
        "{}/{}/messages",
        api_base_url.trim_end_matches('/'),
        phone_number_id.trim()
    );
    let request_url =
        validate_outbound_http_target("whatsapp api_base_url", request_url.as_str(), policy)?;
    let request_body = json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": recipient,
        "type": "text",
        "text": {
            "preview_url": false,
            "body": text,
        },
    });

    let client = build_outbound_http_client("whatsapp send", policy)?;
    let request = client
        .post(request_url)
        .bearer_auth(access_token)
        .json(&request_body);
    let response = request
        .send()
        .await
        .map_err(|error| format!("whatsapp send failed: {error}"))?;
    let payload = read_whatsapp_json_response(response).await?;

    let message_id = payload
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| messages.first())
        .and_then(|message| message.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if message_id.is_none() {
        return Err(format!(
            "whatsapp send did not return a message id: {payload}"
        ));
    }

    Ok(())
}

#[allow(clippy::print_stdout)] // CLI startup banner
pub(super) async fn run_whatsapp_channel(
    config: &LoongClawConfig,
    resolved: &ResolvedWhatsappChannelConfig,
    resolved_path: &Path,
    selected_by_default: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    bind_override: Option<&str>,
    path_override: Option<&str>,
    kernel_ctx: KernelContext,
    runtime: Arc<ChannelOperationRuntimeTracker>,
    stop: ChannelServeStopHandle,
) -> CliResult<()> {
    let bind = bind_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| resolved.resolved_webhook_bind());
    if bind.is_empty() {
        return Err("whatsapp webhook bind address is empty".to_owned());
    }

    let path_raw = path_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| resolved.resolved_webhook_path());
    let path = if path_raw.starts_with('/') {
        path_raw
    } else {
        format!("/{path_raw}")
    };

    let state = WhatsappWebhookState::new(
        config.clone(),
        resolved_path.to_path_buf(),
        resolved,
        kernel_ctx,
        runtime,
    )?;
    let app = Router::new()
        .route(
            path.as_str(),
            get(whatsapp_verify_handler).post(whatsapp_webhook_handler),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind.as_str())
        .await
        .map_err(|error| format!("bind whatsapp webhook listener failed: {error}"))?;

    println!(
        "whatsapp channel started (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, bind={}, path={})",
        resolved_path.display(),
        resolved.configured_account_id,
        resolved.account.label,
        selected_by_default,
        default_account_source.as_str(),
        bind,
        path
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            stop.wait().await;
        })
        .await
        .map_err(|error| format!("whatsapp webhook server stopped: {error}"))
}

async fn read_whatsapp_json_response(response: reqwest::Response) -> CliResult<Value> {
    let status = response.status();
    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("decode whatsapp send response failed: {error}"))?;

    if status.is_success() {
        return Ok(payload);
    }

    let detail = payload
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| payload.to_string());
    Err(format!(
        "whatsapp send failed with status {}: {detail}",
        status.as_u16()
    ))
}

impl ChannelResolvedRuntimeAccount for ResolvedWhatsappChannelConfig {
    fn runtime_account_id(&self) -> &str {
        self.account.id.as_str()
    }

    fn runtime_account_label(&self) -> &str {
        self.account.label.as_str()
    }
}

pub(super) async fn run_whatsapp_channel_with_context(
    context: ChannelCommandContext<ResolvedWhatsappChannelConfig>,
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
            family: WHATSAPP_COMMAND_FAMILY_DESCRIPTOR,
        },
        validate_whatsapp_security_config,
        stop,
        initialize_runtime_environment,
        move |context, kernel_ctx, runtime, stop| {
            Box::pin(async move {
                let route = context.route.clone();
                let resolved_path = context.resolved_path.clone();
                let resolved = context.resolved.clone();
                let config = context.config.clone();
                run_whatsapp_channel(
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

pub(super) async fn run_whatsapp_channel_with_stop(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    let context = build_whatsapp_command_context(resolved_path, config, account_id)?;
    run_whatsapp_channel_with_context(context, None, None, stop, initialize_runtime_environment)
        .await
}

fn validate_whatsapp_security_config(config: &ResolvedWhatsappChannelConfig) -> CliResult<()> {
    let has_verify_token = config
        .verify_token()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_verify_token {
        return Err(
            "whatsapp verify_token is required for webhook verification (set whatsapp.verify_token or env)"
                .to_owned(),
        );
    }
    let has_app_secret = config
        .app_secret()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_app_secret {
        return Err(
            "whatsapp app_secret is required for payload signature verification (set whatsapp.app_secret or env)"
                .to_owned(),
        );
    }
    Ok(())
}
