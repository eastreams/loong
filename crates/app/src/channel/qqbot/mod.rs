mod message_manager;
mod token_manager;
mod websocket_manager;

use std::path::PathBuf;

use crate::CliResult;
use crate::KernelContext;
use crate::channel::commands::ChannelCommandContext;
use crate::channel::runtime::serve::ChannelServeStopHandle;
use crate::config::{
    ChannelDefaultAccountSelectionSource, LoongClawConfig, ResolvedQqbotChannelConfig,
};
use crate::context::DEFAULT_TOKEN_TTL_S;
use tracing;

use self::message_manager::QqbotMsgManager;
use self::token_manager::QqbotTokenManager;
use self::websocket_manager::QqbotWebsocketManager;

/// One-time send command: `loong qqbot-send`.
pub(super) async fn run_qqbot_send(
    resolved: &ResolvedQqbotChannelConfig,
    target_id: &str,
    text: &str,
    policy: crate::channel::http::ChannelOutboundHttpPolicy,
) -> CliResult<()> {
    let app_id = resolved
        .app_id()
        .ok_or("qqbot app_id missing (set qqbot.app_id or QQBOT_APP_ID env)")?;
    let client_secret = resolved.client_secret().ok_or(
        "qqbot client_secret missing (set qqbot.client_secret or QQBOT_CLIENT_SECRET env)",
    )?;

    let http_client = build_http_client(policy)?;
    let mut token_mgr = QqbotTokenManager::new(app_id, client_secret, http_client.clone(), policy);
    let token = token_mgr
        .get_valid_access_token()
        .await
        .map_err(|e| format!("qqbot send token unavailable: {e}"))?;

    self::websocket_manager::send_qqbot_message(&http_client, target_id, text, &token, policy)
        .await?;

    Ok(())
}

/// Long-running serve command: `loong qqbot-serve`.
pub(super) async fn run_qqbot_channel(
    config: &LoongClawConfig,
    resolved: &ResolvedQqbotChannelConfig,
    resolved_path: &std::path::Path,
    selected_by_default: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    kernel_ctx: KernelContext,
    stop: ChannelServeStopHandle,
) -> CliResult<()> {
    let _ = selected_by_default;
    let _ = default_account_source;

    tracing::info!(
        account_id = %resolved.account.id,
        config_path = %resolved_path.display(),
        "qqbot channel starting"
    );

    let app_id = resolved.app_id().ok_or("qqbot app_id missing")?;
    let client_secret = resolved
        .client_secret()
        .ok_or("qqbot client_secret missing")?;

    let policy = crate::channel::http::outbound_http_policy_from_config(config);
    let http_client = build_http_client(policy)?;

    let token_mgr = QqbotTokenManager::new(app_id, client_secret, http_client.clone(), policy);

    let account_id = resolved.account.id.clone();
    let (outbound_tx, outbound_rx) = tokio::sync::mpsc::channel(10);

    let msg_manager = QqbotMsgManager::new(
        config.clone(),
        resolved_path.to_path_buf(),
        resolved.clone(),
        kernel_ctx,
        account_id.clone(),
        outbound_tx,
    );

    let mut ws_manager = QqbotWebsocketManager::new(
        resolved.clone(),
        token_mgr,
        http_client,
        account_id,
        msg_manager,
        outbound_rx,
        policy,
    );

    tokio::select! {
        result = ws_manager.run_session() => result,
        _ = stop.wait() => {
            tracing::info!(
                account_id = %resolved.account.id,
                "qqbot channel shutting down"
            );
            Ok(())
        }
    }
}

pub(super) async fn run_qqbot_channel_with_context(
    context: ChannelCommandContext<ResolvedQqbotChannelConfig>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    if initialize_runtime_environment {
        crate::runtime_env::initialize_runtime_environment(
            &context.config,
            Some(context.resolved_path.as_path()),
        );
    }

    let kernel_ctx = crate::context::bootstrap_kernel_context_with_config(
        "channel-qqbot",
        DEFAULT_TOKEN_TTL_S,
        &context.config,
    )?;

    run_qqbot_channel(
        &context.config,
        &context.resolved,
        &context.resolved_path,
        context.route.selected_by_default(),
        context.route.default_account_source,
        kernel_ctx,
        stop,
    )
    .await
}

pub(super) async fn run_qqbot_channel_with_stop(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    let context = build_qqbot_command_context(resolved_path, config, account_id)?;
    run_qqbot_channel_with_context(context, stop, initialize_runtime_environment).await
}

fn build_qqbot_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedQqbotChannelConfig>> {
    let resolved = config.qqbot.resolve_account(account_id)?;
    let route = config
        .qqbot
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "qqbot account `{}` is disabled by configuration",
            resolved.configured_account_id
        ));
    }

    Ok(ChannelCommandContext {
        resolved_path,
        config,
        resolved,
        route,
    })
}

fn build_http_client(
    policy: crate::channel::http::ChannelOutboundHttpPolicy,
) -> CliResult<reqwest::Client> {
    crate::channel::http::build_outbound_http_client("qqbot", policy)
}
