use std::path::Path;

use axum::{routing::post, Router};

use crate::channel::ChannelAdapter;
use crate::config::LoongClawConfig;
use crate::CliResult;
use crate::KernelContext;

mod adapter;
mod payload;
mod webhook;

use adapter::FeishuAdapter;
use payload::normalize_webhook_path;
use webhook::{feishu_webhook_handler, FeishuWebhookState};

pub(super) async fn run_feishu_send(
    config: &LoongClawConfig,
    receive_id: &str,
    text: &str,
    as_card: bool,
) -> CliResult<()> {
    let mut adapter = FeishuAdapter::new(config)?;
    adapter.refresh_tenant_token().await?;

    if as_card {
        adapter.send_card(receive_id, text).await
    } else {
        adapter.send_text(receive_id, text).await
    }
}

pub(super) async fn run_feishu_channel(
    config: &LoongClawConfig,
    resolved_path: &Path,
    bind_override: Option<&str>,
    path_override: Option<&str>,
    kernel_ctx: KernelContext,
) -> CliResult<()> {
    let mut adapter = FeishuAdapter::new(config)?;
    adapter.refresh_tenant_token().await?;

    let bind = bind_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| config.feishu.webhook_bind.trim().to_owned());
    if bind.is_empty() {
        return Err("feishu webhook bind address is empty".to_owned());
    }

    let path = normalize_webhook_path(
        path_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(config.feishu.webhook_path.as_str()),
    );

    let state = FeishuWebhookState::new(config.clone(), adapter, kernel_ctx);
    let app = Router::new()
        .route(path.as_str(), post(feishu_webhook_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind.as_str())
        .await
        .map_err(|error| format!("bind feishu webhook listener failed: {error}"))?;

    println!(
        "feishu channel started (config={}, bind={}, path={})",
        resolved_path.display(),
        bind,
        path
    );

    axum::serve(listener, app)
        .await
        .map_err(|error| format!("feishu webhook server stopped: {error}"))
}
