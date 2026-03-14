use std::path::Path;
use std::sync::Arc;

use axum::{Router, routing::post};

use crate::CliResult;
use crate::KernelContext;
use crate::channel::{
    ChannelAdapter, ChannelOutboundTarget, ChannelOutboundTargetKind,
    runtime_state::ChannelOperationRuntimeTracker,
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
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
    as_card: bool,
) -> CliResult<()> {
    let mut adapter = FeishuAdapter::new(config)?;
    adapter.refresh_tenant_token().await?;
    let target = build_feishu_send_target(target_kind, target_id)?;

    if as_card {
        adapter.send_card(&target, text).await
    } else {
        adapter.send_text(&target, text).await
    }
}

fn build_feishu_send_target(
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
) -> CliResult<ChannelOutboundTarget> {
    let target_id = target_id.trim();
    if target_id.is_empty() {
        return Err("feishu outbound target id is empty".to_owned());
    }

    let target = match target_kind {
        ChannelOutboundTargetKind::MessageReply => {
            ChannelOutboundTarget::feishu_message_reply(target_id)
        }
        ChannelOutboundTargetKind::ReceiveId => ChannelOutboundTarget::feishu_receive_id(target_id),
        ChannelOutboundTargetKind::Conversation => {
            return Err(
                "feishu send does not support conversation targets; use receive_id or message_reply"
                    .to_owned(),
            );
        }
    };
    Ok(target)
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

    let state = FeishuWebhookState::new(config.clone(), resolved, adapter, kernel_ctx, runtime)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_feishu_send_target_supports_receive_id_and_reply_kinds() {
        let receive_id_target =
            build_feishu_send_target(ChannelOutboundTargetKind::ReceiveId, " ou_123 ")
                .expect("receive id target");
        let reply_target =
            build_feishu_send_target(ChannelOutboundTargetKind::MessageReply, " om_123 ")
                .expect("reply target");

        assert_eq!(receive_id_target.kind, ChannelOutboundTargetKind::ReceiveId);
        assert_eq!(receive_id_target.id, "ou_123");
        assert_eq!(reply_target.kind, ChannelOutboundTargetKind::MessageReply);
        assert_eq!(reply_target.id, "om_123");
    }

    #[test]
    fn build_feishu_send_target_rejects_conversation_kind() {
        assert_eq!(
            build_feishu_send_target(ChannelOutboundTargetKind::Conversation, "oc_123")
                .expect_err("conversation targets should be rejected"),
            "feishu send does not support conversation targets; use receive_id or message_reply"
        );
    }
}
