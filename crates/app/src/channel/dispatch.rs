#[cfg(feature = "channel-telegram")]
use std::time::Duration;
use std::{path::PathBuf, sync::Arc};

use tokio::sync::Notify;
#[cfg(feature = "channel-telegram")]
use tokio::time::sleep;

use crate::CliResult;
#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-dingtalk",
    feature = "channel-email",
    feature = "channel-feishu",
    feature = "channel-google-chat",
    feature = "channel-webhook",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-mattermost",
    feature = "channel-qqbot",
    feature = "channel-signal",
    feature = "channel-twitch",
    feature = "channel-slack",
    feature = "channel-synology-chat",
    feature = "channel-irc",
    feature = "channel-teams",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-imessage",
))]
#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-qqbot",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook",
))]
#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-dingtalk",
    feature = "channel-email",
    feature = "channel-feishu",
    feature = "channel-google-chat",
    feature = "channel-webhook",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-mattermost",
    feature = "channel-qqbot",
    feature = "channel-signal",
    feature = "channel-twitch",
    feature = "channel-slack",
    feature = "channel-synology-chat",
    feature = "channel-irc",
    feature = "channel-teams",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-imessage",
))]
use crate::config::LoongConfig;
#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-discord",
    feature = "channel-dingtalk",
    feature = "channel-email",
    feature = "channel-feishu",
    feature = "channel-google-chat",
    feature = "channel-webhook",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-mattermost",
    feature = "channel-qqbot",
    feature = "channel-signal",
    feature = "channel-twitch",
    feature = "channel-slack",
    feature = "channel-synology-chat",
    feature = "channel-irc",
    feature = "channel-teams",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-imessage",
))]
use crate::context::{DEFAULT_TOKEN_TTL_S, bootstrap_kernel_context_with_config};

#[cfg(feature = "channel-feishu")]
use crate::config::ResolvedFeishuChannelConfig;
#[cfg(feature = "channel-matrix")]
use crate::config::ResolvedMatrixChannelConfig;
#[cfg(feature = "channel-telegram")]
use crate::config::ResolvedTelegramChannelConfig;
#[cfg(feature = "channel-wecom")]
use crate::config::ResolvedWecomChannelConfig;

#[cfg(feature = "channel-feishu")]
use super::commands::accounts::{
    build_feishu_command_context, load_feishu_command_context, validate_feishu_security_config,
};
#[cfg(feature = "channel-line")]
use super::commands::accounts::{build_line_command_context, load_line_command_context};
#[cfg(feature = "channel-matrix")]
use super::commands::accounts::{
    build_matrix_command_context, load_matrix_command_context, validate_matrix_security_config,
};
#[cfg(feature = "channel-telegram")]
use super::commands::accounts::{
    build_telegram_command_context, load_telegram_command_context,
    validate_telegram_security_config,
};
#[cfg(feature = "channel-wecom")]
use super::commands::accounts::{
    build_wecom_command_context, load_wecom_command_context, validate_wecom_security_config,
};
#[cfg(feature = "channel-dingtalk")]
use super::commands::accounts::load_dingtalk_command_context;
#[cfg(feature = "channel-discord")]
use super::commands::accounts::load_discord_command_context;
#[cfg(feature = "channel-email")]
use super::commands::accounts::load_email_command_context;
#[cfg(feature = "channel-google-chat")]
use super::commands::accounts::load_google_chat_command_context;
#[cfg(feature = "channel-imessage")]
use super::commands::accounts::load_imessage_command_context;
#[cfg(feature = "channel-irc")]
use super::commands::accounts::load_irc_command_context;
#[cfg(feature = "channel-mattermost")]
use super::commands::accounts::load_mattermost_command_context;
#[cfg(feature = "channel-nextcloud-talk")]
use super::commands::accounts::load_nextcloud_talk_command_context;
#[cfg(feature = "channel-nostr")]
use super::commands::accounts::load_nostr_command_context;
#[cfg(feature = "channel-qqbot")]
use super::commands::accounts::load_qqbot_command_context;
#[cfg(feature = "channel-slack")]
use super::commands::accounts::load_slack_command_context;
#[cfg(feature = "channel-synology-chat")]
use super::commands::accounts::load_synology_chat_command_context;
#[cfg(feature = "channel-teams")]
use super::commands::accounts::load_teams_command_context;
#[cfg(feature = "channel-webhook")]
use super::commands::accounts::load_webhook_command_context;
#[cfg(feature = "channel-whatsapp")]
use super::commands::accounts::load_whatsapp_command_context;
pub(super) use super::commands::{
    ChannelCommandContext, ChannelSendCommandSpec, run_channel_send_command,
};
#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-qqbot",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
pub(super) use super::commands::{ChannelServeCommandSpec, run_channel_serve_command_with_stop};
#[cfg(feature = "channel-dingtalk")]
use super::dingtalk;
#[cfg(feature = "channel-discord")]
use super::discord;
#[cfg(feature = "channel-email")]
use super::email;
#[cfg(feature = "feishu-integration")]
use super::feishu;
#[cfg(feature = "channel-google-chat")]
use super::google_chat;
#[cfg(feature = "channel-imessage")]
use super::imessage;
#[cfg(feature = "channel-irc")]
use super::irc;
#[cfg(feature = "channel-line")]
use super::line;
#[cfg(feature = "channel-matrix")]
use super::matrix;
#[cfg(feature = "channel-mattermost")]
use super::mattermost;
#[cfg(feature = "channel-nextcloud-talk")]
use super::nextcloud_talk;
#[cfg(feature = "channel-nostr")]
use super::nostr;
#[cfg(feature = "channel-qqbot")]
use super::qqbot;
use super::registry::{
    CHANNEL_OPERATION_SERVE_ID, FEISHU_COMMAND_FAMILY_DESCRIPTOR, MATRIX_COMMAND_FAMILY_DESCRIPTOR,
    WECOM_COMMAND_FAMILY_DESCRIPTOR,
};
use super::runtime::serve::{
    ChannelServeRuntimeSpec, ChannelServeStopHandle, with_channel_serve_runtime_with_stop,
};
use super::runtime::state;
#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-nextcloud-talk",
    feature = "channel-qqbot",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook",
))]
#[cfg(feature = "channel-signal")]
use super::signal;
#[cfg(feature = "channel-signal")]
use super::signal_command;
#[cfg(feature = "channel-slack")]
use super::slack;
#[cfg(feature = "channel-synology-chat")]
use super::synology_chat;
#[cfg(feature = "channel-teams")]
use super::teams;
#[cfg(feature = "channel-telegram")]
use super::telegram;
#[cfg(feature = "channel-webhook")]
use super::webhook;
#[cfg(feature = "channel-wecom")]
use super::wecom;
#[cfg(feature = "channel-whatsapp")]
use super::whatsapp;

#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-qqbot",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
pub use super::inbound_turn::{
    process_inbound_with_provider, process_inbound_with_provider_and_error_mode_and_retry_progress,
};
use super::runtime::state::ChannelOperationRuntime;
#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
))]
use super::types::{ChannelAdapter, process_channel_batch};
use super::types::{ChannelOutboundTargetKind, ChannelPlatform, FeishuChannelSendRequest};

#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-dingtalk",
    feature = "channel-webhook",
    feature = "channel-google-chat",
    feature = "channel-teams"
))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointBackedSendTargetSource {
    CliTarget,
    ConfiguredEndpoint,
}

#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-dingtalk",
    feature = "channel-webhook",
    feature = "channel-google-chat",
    feature = "channel-teams"
))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct EndpointBackedSendTarget {
    endpoint_url: String,
    source: EndpointBackedSendTargetSource,
}

#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-dingtalk",
    feature = "channel-webhook",
    feature = "channel-google-chat",
    feature = "channel-teams"
))]
fn resolve_endpoint_backed_send_target(
    channel_id: &str,
    cli_target: Option<&str>,
    configured_endpoint_url: Option<String>,
    config_field_path: &str,
) -> CliResult<EndpointBackedSendTarget> {
    let cli_target = cli_target
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(endpoint_url) = cli_target {
        return Ok(EndpointBackedSendTarget {
            endpoint_url,
            source: EndpointBackedSendTargetSource::CliTarget,
        });
    }

    let configured_endpoint_url = configured_endpoint_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(endpoint_url) = configured_endpoint_url {
        return Ok(EndpointBackedSendTarget {
            endpoint_url,
            source: EndpointBackedSendTargetSource::ConfiguredEndpoint,
        });
    }

    Err(format!(
        "{channel_id} send requires `--target` or a configured endpoint in `{config_field_path}`"
    ))
}

#[cfg(feature = "channel-telegram")]
#[allow(clippy::print_stdout)] // CLI startup banner
async fn run_telegram_channel_with_context(
    context: ChannelCommandContext<ResolvedTelegramChannelConfig>,
    once: bool,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    validate_telegram_security_config(&context.resolved)?;
    if initialize_runtime_environment {
        crate::runtime_env::initialize_runtime_environment(
            &context.config,
            Some(context.resolved_path.as_path()),
        );
    }
    let kernel_ctx = bootstrap_kernel_context_with_config(
        "channel-telegram",
        DEFAULT_TOKEN_TTL_S,
        &context.config,
    )?;
    let token = context
        .resolved
        .bot_token()
        .ok_or_else(|| "telegram bot token missing (set telegram.bot_token or env)".to_owned())?;
    let route = context.route.clone();
    let resolved_path = context.resolved_path.clone();
    let resolved = context.resolved.clone();
    let batch_config = context.config.clone();
    let batch_kernel_ctx = Arc::new(crate::KernelContext {
        kernel: kernel_ctx.kernel.clone(),
        token: kernel_ctx.token.clone(),
    });
    let runtime_account_id = resolved.account.id.clone();
    let runtime_account_label = resolved.account.label.clone();

    with_channel_serve_runtime_with_stop(
        ChannelServeRuntimeSpec {
            platform: ChannelPlatform::Telegram,
            operation_id: CHANNEL_OPERATION_SERVE_ID,
            account_id: runtime_account_id.as_str(),
            account_label: runtime_account_label.as_str(),
        },
        stop,
        move |runtime, stop| async move {
            let mut adapter = telegram::TelegramAdapter::new(&resolved, token);
            context.emit_route_notice("telegram");

            println!(
                "{} channel started (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, timeout={}s)",
                adapter.name(),
                resolved_path.display(),
                resolved.configured_account_id,
                resolved.account.label,
                route.selected_by_default(),
                route.default_account_source.as_str(),
                resolved.polling_timeout_s
            );

            loop {
                let batch = tokio::select! {
                    _ = stop.wait() => break,
                    batch = adapter.receive_batch() => batch?,
                };
                let config = batch_config.clone();
                let kernel_ctx = batch_kernel_ctx.clone();
                let had_messages = process_channel_batch(
                    &mut adapter,
                    batch,
                    Some(runtime.as_ref()),
                    |message, turn_feedback_policy| {
                        let config = config.clone();
                        let kernel_ctx = kernel_ctx.clone();
                        let resolved_path = resolved_path.clone();
                        Box::pin(async move {
                            process_inbound_with_provider(
                                &config,
                                Some(resolved_path.as_path()),
                                &message,
                                kernel_ctx.as_ref(),
                                turn_feedback_policy,
                            )
                            .await
                        })
                    },
                )
                .await?;
                if !had_messages && once {
                    break;
                }
                if once {
                    break;
                }
                tokio::select! {
                    _ = stop.wait() => break,
                    _ = sleep(Duration::from_millis(250)) => {}
                }
            }
            Ok(())
        },
    )
    .await
}

#[cfg(feature = "channel-telegram")]
pub async fn run_telegram_channel_with_stop(
    resolved_path: PathBuf,
    config: LoongConfig,
    once: bool,
    account_id: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    let context = build_telegram_command_context(resolved_path, config, account_id)?;
    run_telegram_channel_with_context(context, once, stop, initialize_runtime_environment).await
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_discord_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-discord") {
        return Err("discord channel is disabled (enable feature `channel-discord`)".to_owned());
    }
    #[cfg(not(feature = "channel-discord"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("discord channel is disabled (enable feature `channel-discord`)".to_owned());
    }

    #[cfg(feature = "channel-discord")]
    {
        let context = load_discord_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "discord",
            },
            |context| {
                Box::pin(async move {
                    discord::run_discord_send(
                        &context.resolved,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                        context.outbound_http_policy(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "discord message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_signal_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-signal") {
        return Err("signal channel is disabled (enable feature `channel-signal`)".to_owned());
    }
    #[cfg(not(feature = "channel-signal"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("signal channel is disabled (enable feature `channel-signal`)".to_owned());
    }

    #[cfg(feature = "channel-signal")]
    {
        let context = signal_command::load_signal_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "signal",
            },
            |context| {
                Box::pin(async move {
                    signal::run_signal_send(
                        &context.resolved,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                        context.outbound_http_policy(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "signal message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_nostr_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: Option<&str>,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-nostr") {
        return Err("nostr channel is disabled (enable feature `channel-nostr`)".to_owned());
    }

    #[cfg(not(feature = "channel-nostr"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("nostr channel is disabled (enable feature `channel-nostr`)".to_owned());
    }

    #[cfg(feature = "channel-nostr")]
    {
        let context = load_nostr_command_context(config_path, account_id)?;
        let target = target.map(str::to_owned);
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "nostr",
            },
            |context| {
                Box::pin(async move {
                    nostr::run_nostr_send(
                        &context.resolved,
                        target_kind,
                        target.as_deref(),
                        text.as_str(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "nostr event published (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_slack_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-slack") {
        return Err("slack channel is disabled (enable feature `channel-slack`)".to_owned());
    }
    #[cfg(not(feature = "channel-slack"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("slack channel is disabled (enable feature `channel-slack`)".to_owned());
    }

    #[cfg(feature = "channel-slack")]
    {
        let context = load_slack_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "slack",
            },
            |context| {
                Box::pin(async move {
                    slack::run_slack_send(
                        &context.resolved,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                        context.outbound_http_policy(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "slack message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_line_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-line") {
        return Err("line channel is disabled (enable feature `channel-line`)".to_owned());
    }
    #[cfg(not(feature = "channel-line"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("line channel is disabled (enable feature `channel-line`)".to_owned());
    }

    #[cfg(feature = "channel-line")]
    {
        let context = load_line_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec { channel_id: "line" },
            |context| {
                Box::pin(async move {
                    line::run_line_send(
                        &context.resolved,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                        context.outbound_http_policy(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "line message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI startup banner
pub async fn run_line_channel(
    config_path: Option<&str>,
    account_id: Option<&str>,
    bind_override: Option<&str>,
    path_override: Option<&str>,
) -> CliResult<()> {
    if !cfg!(feature = "channel-line") {
        return Err("line channel is disabled (enable feature `channel-line`)".to_owned());
    }
    #[cfg(not(feature = "channel-line"))]
    {
        let _ = (config_path, account_id, bind_override, path_override);
        return Err("line channel is disabled (enable feature `channel-line`)".to_owned());
    }

    #[cfg(feature = "channel-line")]
    {
        let context = load_line_command_context(config_path, account_id)?;
        line::run_line_channel_with_context(
            context,
            bind_override,
            path_override,
            ChannelServeStopHandle::new(),
            true,
        )
        .await
    }
}

#[cfg(feature = "channel-line")]
pub async fn run_line_channel_with_stop(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
    bind_override: Option<&str>,
    path_override: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    let context = build_line_command_context(resolved_path, config, account_id)?;
    line::run_line_channel_with_context(
        context,
        bind_override,
        path_override,
        stop,
        initialize_runtime_environment,
    )
    .await
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_dingtalk_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: Option<&str>,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-dingtalk") {
        return Err("dingtalk channel is disabled (enable feature `channel-dingtalk`)".to_owned());
    }
    #[cfg(not(feature = "channel-dingtalk"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("dingtalk channel is disabled (enable feature `channel-dingtalk`)".to_owned());
    }

    #[cfg(feature = "channel-dingtalk")]
    {
        let context = load_dingtalk_command_context(config_path, account_id)?;
        let send_target = resolve_endpoint_backed_send_target(
            "dingtalk",
            target,
            context.resolved.webhook_url(),
            "dingtalk.webhook_url",
        )?;
        let endpoint_url = send_target.endpoint_url;
        let target_source = match send_target.source {
            EndpointBackedSendTargetSource::CliTarget => "cli_target",
            EndpointBackedSendTargetSource::ConfiguredEndpoint => "configured_endpoint",
        };
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "dingtalk",
            },
            |context| {
                Box::pin(async move {
                    dingtalk::run_dingtalk_send(
                        &context.resolved,
                        target_kind,
                        endpoint_url.as_str(),
                        text.as_str(),
                        context.outbound_http_policy(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "dingtalk message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={}, target_source={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind,
                    target_source
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_whatsapp_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-whatsapp") {
        return Err("whatsapp channel is disabled (enable feature `channel-whatsapp`)".to_owned());
    }

    #[cfg(not(feature = "channel-whatsapp"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("whatsapp channel is disabled (enable feature `channel-whatsapp`)".to_owned());
    }

    #[cfg(feature = "channel-whatsapp")]
    {
        let context = load_whatsapp_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "whatsapp",
            },
            |context| {
                Box::pin(async move {
                    whatsapp::run_whatsapp_send(
                        &context.resolved,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                        context.outbound_http_policy(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "whatsapp message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI startup banner
pub async fn run_whatsapp_channel(
    config_path: Option<&str>,
    account_id: Option<&str>,
    bind_override: Option<&str>,
    path_override: Option<&str>,
) -> CliResult<()> {
    if !cfg!(feature = "channel-whatsapp") {
        return Err("whatsapp channel is disabled (enable feature `channel-whatsapp`)".to_owned());
    }

    #[cfg(not(feature = "channel-whatsapp"))]
    {
        let _ = (config_path, account_id, bind_override, path_override);
        return Err("whatsapp channel is disabled (enable feature `channel-whatsapp`)".to_owned());
    }

    #[cfg(feature = "channel-whatsapp")]
    {
        let context = load_whatsapp_command_context(config_path, account_id)?;
        whatsapp::run_whatsapp_channel_with_context(
            context,
            bind_override,
            path_override,
            ChannelServeStopHandle::new(),
            true,
        )
        .await
    }
}

#[cfg(feature = "channel-whatsapp")]
pub async fn run_whatsapp_channel_with_stop(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    whatsapp::run_whatsapp_channel_with_stop(
        resolved_path,
        config,
        account_id,
        stop,
        initialize_runtime_environment,
    )
    .await
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_email_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-email") {
        return Err("email channel is disabled (enable feature `channel-email`)".to_owned());
    }

    #[cfg(not(feature = "channel-email"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("email channel is disabled (enable feature `channel-email`)".to_owned());
    }

    #[cfg(feature = "channel-email")]
    {
        let context = load_email_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec { channel_id: "email" },
            |context| {
                Box::pin(async move {
                    email::run_email_send(
                        &context.resolved,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "email message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_webhook_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: Option<&str>,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-webhook") {
        return Err("webhook channel is disabled (enable feature `channel-webhook`)".to_owned());
    }

    #[cfg(not(feature = "channel-webhook"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("webhook channel is disabled (enable feature `channel-webhook`)".to_owned());
    }

    #[cfg(feature = "channel-webhook")]
    {
        let context = load_webhook_command_context(config_path, account_id)?;
        let send_target = resolve_endpoint_backed_send_target(
            "webhook",
            target,
            context.resolved.endpoint_url(),
            "webhook.endpoint_url",
        )?;
        let endpoint_url = send_target.endpoint_url;
        let target_source = match send_target.source {
            EndpointBackedSendTargetSource::CliTarget => "cli_target",
            EndpointBackedSendTargetSource::ConfiguredEndpoint => "configured_endpoint",
        };
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "webhook",
            },
            |context| {
                Box::pin(async move {
                    webhook::run_webhook_send(
                        &context.resolved,
                        target_kind,
                        endpoint_url.as_str(),
                        text.as_str(),
                        context.outbound_http_policy(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "webhook message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={}, target_source={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind,
                    target_source
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI startup banner
pub async fn run_webhook_channel(
    config_path: Option<&str>,
    account_id: Option<&str>,
    bind_override: Option<&str>,
    path_override: Option<&str>,
) -> CliResult<()> {
    if !cfg!(feature = "channel-webhook") {
        return Err("webhook channel is disabled (enable feature `channel-webhook`)".to_owned());
    }

    #[cfg(not(feature = "channel-webhook"))]
    {
        let _ = (config_path, account_id, bind_override, path_override);
        return Err("webhook channel is disabled (enable feature `channel-webhook`)".to_owned());
    }

    #[cfg(feature = "channel-webhook")]
    {
        let context = load_webhook_command_context(config_path, account_id)?;
        webhook::run_webhook_channel_with_context(
            context,
            bind_override,
            path_override,
            ChannelServeStopHandle::new(),
            true,
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI startup banner
pub async fn run_qqbot_channel(
    config_path: Option<&str>,
    account_id: Option<&str>,
    _bind_override: Option<&str>,
    _path_override: Option<&str>,
) -> CliResult<()> {
    if !cfg!(feature = "channel-qqbot") {
        return Err("qqbot channel is disabled (enable feature `channel-qqbot`)".to_owned());
    }

    #[cfg(not(feature = "channel-qqbot"))]
    {
        let _ = (config_path, account_id, _bind_override, _path_override);
        return Err("qqbot channel is disabled (enable feature `channel-qqbot`)".to_owned());
    }

    #[cfg(feature = "channel-qqbot")]
    {
        let context = load_qqbot_command_context(config_path, account_id)?;
        Box::pin(qqbot::run_qqbot_channel_with_context(
            context,
            ChannelServeStopHandle::new(),
            true,
        ))
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_google_chat_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: Option<&str>,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-google-chat") {
        return Err(
            "google chat channel is disabled (enable feature `channel-google-chat`)".to_owned(),
        );
    }

    #[cfg(not(feature = "channel-google-chat"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err(
            "google chat channel is disabled (enable feature `channel-google-chat`)".to_owned(),
        );
    }

    #[cfg(feature = "channel-google-chat")]
    {
        let context = load_google_chat_command_context(config_path, account_id)?;
        let send_target = resolve_endpoint_backed_send_target(
            "google-chat",
            target,
            context.resolved.webhook_url(),
            "google_chat.webhook_url",
        )?;
        let endpoint_url = send_target.endpoint_url;
        let target_source = match send_target.source {
            EndpointBackedSendTargetSource::CliTarget => "cli_target",
            EndpointBackedSendTargetSource::ConfiguredEndpoint => "configured_endpoint",
        };
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "google-chat",
            },
            |context| {
                Box::pin(async move {
                    google_chat::run_google_chat_send(
                        &context.resolved,
                        target_kind,
                        endpoint_url.as_str(),
                        text.as_str(),
                        context.outbound_http_policy(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "google chat message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={}, target_source={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind,
                    target_source
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_teams_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: Option<&str>,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-teams") {
        return Err("teams channel is disabled (enable feature `channel-teams`)".to_owned());
    }

    #[cfg(not(feature = "channel-teams"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("teams channel is disabled (enable feature `channel-teams`)".to_owned());
    }

    #[cfg(feature = "channel-teams")]
    {
        let context = load_teams_command_context(config_path, account_id)?;
        let send_target = resolve_endpoint_backed_send_target(
            "teams",
            target,
            context.resolved.webhook_url(),
            "teams.webhook_url",
        )?;
        let endpoint_url = send_target.endpoint_url;
        let target_source = match send_target.source {
            EndpointBackedSendTargetSource::CliTarget => "cli_target",
            EndpointBackedSendTargetSource::ConfiguredEndpoint => "configured_endpoint",
        };
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "teams",
            },
            |context| {
                Box::pin(async move {
                    teams::run_teams_send(
                        &context.resolved,
                        target_kind,
                        endpoint_url.as_str(),
                        text.as_str(),
                        context.outbound_http_policy(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "teams message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={}, target_source={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind,
                    target_source
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_mattermost_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-mattermost") {
        return Err(
            "mattermost channel is disabled (enable feature `channel-mattermost`)".to_owned(),
        );
    }

    #[cfg(not(feature = "channel-mattermost"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err(
            "mattermost channel is disabled (enable feature `channel-mattermost`)".to_owned(),
        );
    }

    #[cfg(feature = "channel-mattermost")]
    {
        let context = load_mattermost_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "mattermost",
            },
            |context| {
                Box::pin(async move {
                    mattermost::run_mattermost_send(
                        &context.resolved,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                        context.outbound_http_policy(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "mattermost message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_nextcloud_talk_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-nextcloud-talk") {
        return Err(
            "nextcloud talk channel is disabled (enable feature `channel-nextcloud-talk`)"
                .to_owned(),
        );
    }

    #[cfg(not(feature = "channel-nextcloud-talk"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err(
            "nextcloud talk channel is disabled (enable feature `channel-nextcloud-talk`)"
                .to_owned(),
        );
    }

    #[cfg(feature = "channel-nextcloud-talk")]
    {
        let context = load_nextcloud_talk_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "nextcloud-talk",
            },
            |context| {
                Box::pin(async move {
                    nextcloud_talk::run_nextcloud_talk_send(
                        &context.resolved,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                        context.outbound_http_policy(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "nextcloud talk message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_synology_chat_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: Option<&str>,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-synology-chat") {
        return Err(
            "synology chat channel is disabled (enable feature `channel-synology-chat`)".to_owned(),
        );
    }

    #[cfg(not(feature = "channel-synology-chat"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err(
            "synology chat channel is disabled (enable feature `channel-synology-chat`)".to_owned(),
        );
    }

    #[cfg(feature = "channel-synology-chat")]
    {
        let context = load_synology_chat_command_context(config_path, account_id)?;
        let target = target
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let target_selected = target.is_some();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "synology-chat",
            },
            |context| {
                Box::pin(async move {
                    synology_chat::run_synology_chat_send(
                        &context.resolved,
                        target_kind,
                        target.as_deref(),
                        text.as_str(),
                        context.outbound_http_policy(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "synology chat message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={}, target_selected={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind,
                    target_selected
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_irc_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-irc") {
        return Err("irc channel is disabled (enable feature `channel-irc`)".to_owned());
    }

    #[cfg(not(feature = "channel-irc"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("irc channel is disabled (enable feature `channel-irc`)".to_owned());
    }

    #[cfg(feature = "channel-irc")]
    {
        let context = load_irc_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec { channel_id: "irc" },
            |context| {
                Box::pin(async move {
                    irc::run_irc_send(
                        &context.resolved,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "irc message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_imessage_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-imessage") {
        return Err("imessage channel is disabled (enable feature `channel-imessage`)".to_owned());
    }

    #[cfg(not(feature = "channel-imessage"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("imessage channel is disabled (enable feature `channel-imessage`)".to_owned());
    }

    #[cfg(feature = "channel-imessage")]
    {
        let context = load_imessage_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "imessage",
            },
            |context| {
                Box::pin(async move {
                    imessage::run_imessage_send(
                        &context.resolved,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                        context.outbound_http_policy(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "imessage message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI startup banner
pub async fn run_telegram_channel(
    config_path: Option<&str>,
    once: bool,
    account_id: Option<&str>,
) -> CliResult<()> {
    if !cfg!(feature = "channel-telegram") {
        return Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned());
    }

    #[cfg(not(feature = "channel-telegram"))]
    {
        let _ = (config_path, once, account_id);
        return Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned());
    }

    #[cfg(feature = "channel-telegram")]
    {
        let context = load_telegram_command_context(config_path, account_id)?;
        run_telegram_channel_with_context(context, once, ChannelServeStopHandle::new(), true).await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_telegram_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-telegram") {
        return Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned());
    }

    #[cfg(not(feature = "channel-telegram"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned());
    }

    #[cfg(feature = "channel-telegram")]
    {
        let context = load_telegram_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "telegram",
            },
            |context| {
                Box::pin(async move {
                    let token = context.resolved.bot_token().ok_or_else(|| {
                        "telegram bot token missing (set telegram.bot_token or env)".to_owned()
                    })?;
                    telegram::run_telegram_send(
                        &context.resolved,
                        token,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "telegram message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_feishu_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    request: &FeishuChannelSendRequest,
) -> CliResult<()> {
    if !cfg!(feature = "channel-feishu") {
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(not(feature = "channel-feishu"))]
    {
        let _ = (config_path, account_id, request);
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(feature = "channel-feishu")]
    {
        let context = load_feishu_command_context(config_path, account_id)?;
        let request = request.clone();
        let success_receive_id_type = request
            .receive_id_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "feishu",
            },
            |context| {
                Box::pin(async move { feishu::run_feishu_send(&context.resolved, &request).await })
            },
            |context| {
                format!(
                    "feishu message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, receive_id_type={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    success_receive_id_type
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or(context.resolved.receive_id_type.as_str())
                )
            },
        )
        .await
    }
}

pub async fn run_feishu_channel(
    config_path: Option<&str>,
    account_id: Option<&str>,
    bind_override: Option<&str>,
    path_override: Option<&str>,
) -> CliResult<()> {
    if !cfg!(feature = "channel-feishu") {
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(not(feature = "channel-feishu"))]
    {
        let _ = (config_path, account_id, bind_override, path_override);
        return Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned());
    }

    #[cfg(feature = "channel-feishu")]
    {
        let context = load_feishu_command_context(config_path, account_id)?;
        run_feishu_channel_with_context(
            context,
            bind_override,
            path_override,
            ChannelServeStopHandle::new(),
            true,
        )
        .await
    }
}

#[cfg(feature = "channel-feishu")]
async fn run_feishu_channel_with_context(
    context: ChannelCommandContext<ResolvedFeishuChannelConfig>,
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
            family: FEISHU_COMMAND_FAMILY_DESCRIPTOR,
        },
        validate_feishu_security_config,
        stop,
        initialize_runtime_environment,
        move |context, kernel_ctx, runtime, stop| {
            Box::pin(async move {
                let route = context.route.clone();
                let resolved_path = context.resolved_path.clone();
                let resolved = context.resolved.clone();
                let config = context.config.clone();
                feishu::run_feishu_channel(
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

#[cfg(feature = "channel-feishu")]
pub async fn run_feishu_channel_with_stop(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
    bind_override: Option<&str>,
    path_override: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    let context = build_feishu_command_context(resolved_path, config, account_id)?;
    run_feishu_channel_with_context(
        context,
        bind_override,
        path_override,
        stop,
        initialize_runtime_environment,
    )
    .await
}

#[doc(hidden)]
#[cfg(any(
    feature = "channel-plugin-bridge",
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-line",
    feature = "channel-matrix",
    feature = "channel-nextcloud-talk",
    feature = "channel-qqbot",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-webhook"
))]
pub async fn run_channel_serve_runtime_probe_for_test(
    platform: ChannelPlatform,
    account_id: &str,
    account_label: &str,
    stop: ChannelServeStopHandle,
    entered: Arc<Notify>,
) -> CliResult<()> {
    with_channel_serve_runtime_with_stop(
        ChannelServeRuntimeSpec {
            platform,
            operation_id: CHANNEL_OPERATION_SERVE_ID,
            account_id,
            account_label,
        },
        stop,
        move |_runtime, stop| async move {
            entered.notify_one();
            stop.wait().await;
            Ok(())
        },
    )
    .await
}

#[doc(hidden)]
pub fn load_channel_operation_runtime_for_account_from_dir_for_test(
    runtime_dir: &std::path::Path,
    platform: ChannelPlatform,
    operation_id: &str,
    account_id: &str,
    now_ms: u64,
) -> Option<ChannelOperationRuntime> {
    state::load_channel_operation_runtime_for_account_from_dir(
        runtime_dir,
        platform,
        operation_id,
        account_id,
        now_ms,
    )
}

#[allow(clippy::print_stdout)] // CLI output
pub async fn run_matrix_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-matrix") {
        return Err("matrix channel is disabled (enable feature `channel-matrix`)".to_owned());
    }

    #[cfg(not(feature = "channel-matrix"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("matrix channel is disabled (enable feature `channel-matrix`)".to_owned());
    }

    #[cfg(feature = "channel-matrix")]
    {
        let context = load_matrix_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "matrix",
            },
            |context| {
                Box::pin(async move {
                    let token = context.resolved.access_token().ok_or_else(|| {
                        "matrix access token missing (set matrix.access_token or env)".to_owned()
                    })?;
                    matrix::run_matrix_send(
                        &context.resolved,
                        token,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "matrix message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)] // CLI startup banner
pub async fn run_matrix_channel(
    config_path: Option<&str>,
    once: bool,
    account_id: Option<&str>,
) -> CliResult<()> {
    if !cfg!(feature = "channel-matrix") {
        return Err("matrix channel is disabled (enable feature `channel-matrix`)".to_owned());
    }

    #[cfg(not(feature = "channel-matrix"))]
    {
        let _ = (config_path, once, account_id);
        return Err("matrix channel is disabled (enable feature `channel-matrix`)".to_owned());
    }

    #[cfg(feature = "channel-matrix")]
    {
        let context = load_matrix_command_context(config_path, account_id)?;
        run_matrix_channel_with_context(context, once, ChannelServeStopHandle::new(), true).await
    }
}

#[cfg(feature = "channel-matrix")]
#[allow(clippy::print_stdout)]
async fn run_matrix_channel_with_context(
    context: ChannelCommandContext<ResolvedMatrixChannelConfig>,
    once: bool,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    run_channel_serve_command_with_stop(
        context,
        ChannelServeCommandSpec {
            family: MATRIX_COMMAND_FAMILY_DESCRIPTOR,
        },
        validate_matrix_security_config,
        stop,
        initialize_runtime_environment,
        move |context, kernel_ctx, runtime, stop| {
            Box::pin(async move {
                let route = context.route.clone();
                let resolved_path = context.resolved_path.clone();
                let resolved = context.resolved.clone();
                let config = context.config.clone();
                let batch_kernel_ctx = Arc::new(crate::KernelContext {
                    kernel: kernel_ctx.kernel.clone(),
                    token: kernel_ctx.token.clone(),
                });
                let token = resolved.access_token().ok_or_else(|| {
                    "matrix access token missing (set matrix.access_token or env)".to_owned()
                })?;
                let mut adapter = matrix::MatrixAdapter::new(&resolved, token);

                println!(
                    "{} channel started (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, timeout={}s)",
                    adapter.name(),
                    resolved_path.display(),
                    resolved.configured_account_id,
                    resolved.account.label,
                    route.selected_by_default(),
                    route.default_account_source.as_str(),
                    resolved.sync_timeout_s
                );

                loop {
                    let batch = tokio::select! {
                        _ = stop.wait() => break,
                        batch = adapter.receive_batch() => batch?,
                    };
                    let had_messages = process_channel_batch(
                        &mut adapter,
                        batch,
                        Some(runtime.as_ref()),
                        |message, turn_feedback_policy| {
                            let config = config.clone();
                            let kernel_ctx = batch_kernel_ctx.clone();
                            let resolved_path = resolved_path.clone();
                            Box::pin(async move {
                                process_inbound_with_provider(
                                    &config,
                                    Some(resolved_path.as_path()),
                                    &message,
                                    kernel_ctx.as_ref(),
                                    turn_feedback_policy,
                                )
                                .await
                            })
                        },
                    )
                    .await?;
                    if !had_messages && once {
                        break;
                    }
                    if once {
                        break;
                    }
                }
                Ok(())
            })
        },
    )
    .await
}

#[cfg(feature = "channel-matrix")]
pub async fn run_matrix_channel_with_stop(
    resolved_path: PathBuf,
    config: LoongConfig,
    once: bool,
    account_id: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    let context = build_matrix_command_context(resolved_path, config, account_id)?;
    run_matrix_channel_with_context(context, once, stop, initialize_runtime_environment).await
}

#[allow(clippy::print_stdout)]
pub async fn run_wecom_send(
    config_path: Option<&str>,
    account_id: Option<&str>,
    target: &str,
    target_kind: ChannelOutboundTargetKind,
    text: &str,
) -> CliResult<()> {
    if !cfg!(feature = "channel-wecom") {
        return Err("wecom channel is disabled (enable feature `channel-wecom`)".to_owned());
    }

    #[cfg(not(feature = "channel-wecom"))]
    {
        let _ = (config_path, account_id, target, target_kind, text);
        return Err("wecom channel is disabled (enable feature `channel-wecom`)".to_owned());
    }

    #[cfg(feature = "channel-wecom")]
    {
        let context = load_wecom_command_context(config_path, account_id)?;
        let target = target.to_owned();
        let text = text.to_owned();
        run_channel_send_command(
            context,
            ChannelSendCommandSpec {
                channel_id: "wecom",
            },
            |context| {
                Box::pin(async move {
                    wecom::run_wecom_send(
                        &context.resolved,
                        target_kind,
                        target.as_str(),
                        text.as_str(),
                    )
                    .await
                })
            },
            |context| {
                format!(
                    "wecom message sent (config={}, configured_account={}, account={}, selected_by_default={}, default_source={}, target_kind={})",
                    context.resolved_path.display(),
                    context.resolved.configured_account_id,
                    context.resolved.account.label,
                    context.route.selected_by_default(),
                    context.route.default_account_source.as_str(),
                    target_kind
                )
            },
        )
        .await
    }
}

#[allow(clippy::print_stdout)]
pub async fn run_wecom_channel(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<()> {
    if !cfg!(feature = "channel-wecom") {
        return Err("wecom channel is disabled (enable feature `channel-wecom`)".to_owned());
    }

    #[cfg(not(feature = "channel-wecom"))]
    {
        let _ = (config_path, account_id);
        return Err("wecom channel is disabled (enable feature `channel-wecom`)".to_owned());
    }

    #[cfg(feature = "channel-wecom")]
    {
        let context = load_wecom_command_context(config_path, account_id)?;
        run_wecom_channel_with_context(context, ChannelServeStopHandle::new(), true).await
    }
}

#[cfg(feature = "channel-wecom")]
async fn run_wecom_channel_with_context(
    context: ChannelCommandContext<ResolvedWecomChannelConfig>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    run_channel_serve_command_with_stop(
        context,
        ChannelServeCommandSpec {
            family: WECOM_COMMAND_FAMILY_DESCRIPTOR,
        },
        validate_wecom_security_config,
        stop,
        initialize_runtime_environment,
        move |context, kernel_ctx, runtime, stop| {
            Box::pin(async move {
                let route = context.route.clone();
                let resolved_path = context.resolved_path.clone();
                let resolved = context.resolved.clone();
                let config = context.config.clone();
                wecom::run_wecom_channel(
                    &config,
                    &resolved,
                    &resolved_path,
                    route.selected_by_default(),
                    route.default_account_source,
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

#[cfg(feature = "channel-wecom")]
pub async fn run_wecom_channel_with_stop(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    let context = build_wecom_command_context(resolved_path, config, account_id)?;
    run_wecom_channel_with_context(context, stop, initialize_runtime_environment).await
}

#[cfg(all(test, feature = "channel-feishu"))]
mod tests {
    #[test]
    fn wildcard_allows_send_to_any_chat_id() {
        let allowed_chat_ids: Vec<String> = vec!["*".to_owned(), "oc_other".to_owned()];

        let result =
            crate::channel::feishu::feishu_allowlist_allows_chat(&allowed_chat_ids, "oc_random");

        assert!(result, "wildcard '*' should allow any chat_id");
    }

    #[test]
    fn exact_match_allows_send_without_wildcard() {
        let allowed_chat_ids: Vec<String> = vec!["oc_demo".to_owned()];

        let result =
            crate::channel::feishu::feishu_allowlist_allows_chat(&allowed_chat_ids, "oc_demo");

        assert!(result, "exact match should still work");
    }

    #[test]
    fn non_matched_chat_rejected_without_wildcard() {
        let allowed_chat_ids: Vec<String> = vec!["oc_demo".to_owned()];

        let result =
            crate::channel::feishu::feishu_allowlist_allows_chat(&allowed_chat_ids, "oc_other");

        assert!(
            !result,
            "non-matched chat_id should be rejected without wildcard"
        );
    }
}
