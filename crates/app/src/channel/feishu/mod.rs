use std::path::Path;
use std::sync::Arc;

use axum::{Router, routing::post};

use crate::CliResult;
use crate::KernelContext;
use crate::channel::{
    ChannelAdapter, ChannelOutboundTarget, runtime_state::ChannelOperationRuntimeTracker,
};
use crate::config::{
    ChannelDefaultAccountSelectionSource, LoongClawConfig, ResolvedFeishuChannelConfig,
};

mod adapter;
mod payload;
mod webhook;

use adapter::FeishuAdapter;
use payload::normalize_webhook_path;
use webhook::{FeishuWebhookState, feishu_webhook_handler};

pub(super) async fn run_feishu_send(
    config: &ResolvedFeishuChannelConfig,
    receive_id: &str,
    text: &str,
    as_card: bool,
) -> CliResult<()> {
    let mut adapter = FeishuAdapter::new(config)?;
    adapter.refresh_tenant_token().await?;
    let target = ChannelOutboundTarget::feishu_receive_id(receive_id);

    if as_card {
        adapter.send_card(&target, text).await
    } else {
        adapter.send_text(&target, text).await
    }
}

#[allow(clippy::print_stdout)] // CLI startup banner
pub(super) async fn run_feishu_channel(
    config: &LoongClawConfig,
    resolved: &ResolvedFeishuChannelConfig,
    resolved_path: &Path,
    selected_by_default: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    bind_override: Option<&str>,
    path_override: Option<&str>,
    kernel_ctx: KernelContext,
    runtime: Arc<ChannelOperationRuntimeTracker>,
) -> CliResult<()> {
    let mut adapter = FeishuAdapter::new(resolved)?;
    adapter.refresh_tenant_token().await?;

    let bind = bind_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| resolved.webhook_bind.trim().to_owned());
    if bind.is_empty() {
        return Err("feishu webhook bind address is empty".to_owned());
    }

    let path = normalize_webhook_path(
        path_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(resolved.webhook_path.as_str()),
    );

    let state = FeishuWebhookState::new(config.clone(), resolved, adapter, kernel_ctx, runtime);
    let app = Router::new()
        .route(path.as_str(), post(feishu_webhook_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind.as_str())
        .await
        .map_err(|error| format!("bind feishu webhook listener failed: {error}"))?;

    println!(
        "feishu channel started (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, bind={}, path={})",
        resolved_path.display(),
        resolved.configured_account_id,
        resolved.account.label,
        selected_by_default,
        default_account_source.as_str(),
        bind,
        path
    );

    axum::serve(listener, app)
        .await
        .map_err(|error| format!("feishu webhook server stopped: {error}"))
}
