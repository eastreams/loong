#[cfg(feature = "channel-telegram")]
use std::time::Duration;
use std::{path::PathBuf, sync::Arc};

use tokio::sync::Notify;
#[cfg(feature = "channel-telegram")]
use tokio::time::sleep;

use crate::CliResult;
#[cfg(any(
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
    feature = "channel-nextcloud-talk",
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
use crate::KernelContext;
#[cfg(any(
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
    feature = "channel-nextcloud-talk",
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
use crate::acp::{AcpConversationTurnOptions, AcpTurnProvenance};
use crate::config::LoongClawConfig;
#[cfg(any(
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
    feature = "channel-nextcloud-talk",
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

#[cfg(any(
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
    feature = "channel-nextcloud-talk",
    feature = "channel-signal",
    feature = "channel-slack",
    feature = "channel-irc",
    feature = "channel-synology-chat",
    feature = "channel-twitch",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
    feature = "channel-teams",
    feature = "channel-imessage",
    feature = "channel-nostr",
))]
#[cfg(feature = "channel-dingtalk")]
use crate::config::ResolvedDingtalkChannelConfig;
#[cfg(feature = "channel-discord")]
use crate::config::ResolvedDiscordChannelConfig;
#[cfg(feature = "channel-email")]
use crate::config::ResolvedEmailChannelConfig;
#[cfg(feature = "channel-feishu")]
use crate::config::ResolvedFeishuChannelConfig;
#[cfg(feature = "channel-google-chat")]
use crate::config::ResolvedGoogleChatChannelConfig;
#[cfg(feature = "channel-imessage")]
use crate::config::ResolvedImessageChannelConfig;
#[cfg(feature = "channel-irc")]
use crate::config::ResolvedIrcChannelConfig;
#[cfg(feature = "channel-line")]
use crate::config::ResolvedLineChannelConfig;
#[cfg(feature = "channel-matrix")]
use crate::config::ResolvedMatrixChannelConfig;
#[cfg(feature = "channel-mattermost")]
use crate::config::ResolvedMattermostChannelConfig;
#[cfg(feature = "channel-nextcloud-talk")]
use crate::config::ResolvedNextcloudTalkChannelConfig;
#[cfg(feature = "channel-nostr")]
use crate::config::ResolvedNostrChannelConfig;
#[cfg(feature = "channel-slack")]
use crate::config::ResolvedSlackChannelConfig;
#[cfg(feature = "channel-synology-chat")]
use crate::config::ResolvedSynologyChatChannelConfig;
#[cfg(feature = "channel-teams")]
use crate::config::ResolvedTeamsChannelConfig;
#[cfg(feature = "channel-telegram")]
use crate::config::ResolvedTelegramChannelConfig;
#[cfg(feature = "channel-webhook")]
use crate::config::ResolvedWebhookChannelConfig;
#[cfg(feature = "channel-wecom")]
use crate::config::ResolvedWecomChannelConfig;
#[cfg(feature = "channel-whatsapp")]
use crate::config::ResolvedWhatsappChannelConfig;

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
))]
use crate::conversation::{
    ConversationIngressChannel, ConversationIngressContext, ConversationIngressDelivery,
    ConversationIngressDeliveryResource, ConversationIngressFeishuCallbackContext,
    ConversationIngressPrivateContext, ConversationRuntime, ConversationRuntimeBinding,
    DefaultConversationRuntime,
};
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
))]
use crate::conversation::{ConversationTurnCoordinator, ProviderErrorMode};

pub(super) use super::commands::{
    ChannelCommandContext, ChannelResolvedRuntimeAccount, ChannelSendCommandSpec,
    run_channel_send_command, run_channel_serve_command_with_stop,
};
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
use super::registry::{
    CHANNEL_OPERATION_SERVE_ID, FEISHU_COMMAND_FAMILY_DESCRIPTOR, MATRIX_COMMAND_FAMILY_DESCRIPTOR,
    WECOM_COMMAND_FAMILY_DESCRIPTOR,
};
use super::runtime_state;
use super::runtime::serve::{
    ChannelServeCommandSpec, ChannelServeRuntimeSpec, ChannelServeStopHandle,
    with_channel_serve_runtime_with_stop,
};
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
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
))]
use super::runtime::turn_feedback::ChannelTurnFeedbackCapture;
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
))]
use super::runtime::turn_feedback::ChannelTurnFeedbackPolicy;
#[cfg(feature = "channel-webhook")]
use super::webhook;
#[cfg(feature = "channel-wecom")]
use super::wecom;
#[cfg(feature = "channel-whatsapp")]
use super::whatsapp;

use super::runtime_state::ChannelOperationRuntime;
use super::types::{
    ChannelAdapter, ChannelDeliveryFeishuCallback, ChannelDeliveryResource, ChannelInboundMessage,
    ChannelOutboundTargetKind, ChannelPlatform, ChannelSendReceipt, ChannelSession,
    FeishuChannelSendRequest,
};
#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp",
))]
use super::types::{
    ChannelResolvedAcpTurnHints, KnownChannelSessionSendTarget,
    parse_known_channel_session_send_target, process_channel_batch,
};

#[cfg(any(
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

#[cfg(feature = "channel-discord")]
fn load_discord_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedDiscordChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_discord_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-discord")]
fn build_discord_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedDiscordChannelConfig>> {
    let resolved = config.discord.resolve_account(account_id)?;
    let route = config
        .discord
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "discord account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-dingtalk")]
fn load_dingtalk_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedDingtalkChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_dingtalk_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-dingtalk")]
fn build_dingtalk_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedDingtalkChannelConfig>> {
    let resolved = config.dingtalk.resolve_account(account_id)?;
    let route = config
        .dingtalk
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "dingtalk account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-telegram")]
fn load_telegram_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedTelegramChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_telegram_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-telegram")]
pub(super) fn build_telegram_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedTelegramChannelConfig>> {
    let resolved = config.telegram.resolve_account(account_id)?;
    let route = config
        .telegram
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "telegram account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-feishu")]
fn load_feishu_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedFeishuChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_feishu_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-feishu")]
pub(super) fn build_feishu_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedFeishuChannelConfig>> {
    let resolved = crate::channel::feishu::api::resolve_requested_feishu_account(
        &config.feishu,
        account_id,
        "rerun with `--account <configured_account_id>` using one of those configured accounts",
    )?;
    let route = config
        .feishu
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "feishu account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-matrix")]
fn load_matrix_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedMatrixChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_matrix_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-matrix")]
fn build_matrix_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedMatrixChannelConfig>> {
    let resolved = config.matrix.resolve_account(account_id)?;
    let route = config
        .matrix
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "matrix account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-wecom")]
fn load_wecom_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedWecomChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_wecom_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-wecom")]
fn build_wecom_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedWecomChannelConfig>> {
    let resolved = config.wecom.resolve_account(account_id)?;
    let route = config
        .wecom
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "wecom account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-slack")]
fn load_slack_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedSlackChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_slack_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-slack")]
fn build_slack_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedSlackChannelConfig>> {
    let resolved = config.slack.resolve_account(account_id)?;
    let route = config
        .slack
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "slack account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-line")]
fn load_line_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedLineChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_line_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-line")]
fn build_line_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedLineChannelConfig>> {
    let resolved = config.line.resolve_account(account_id)?;
    let route = config
        .line
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "line account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-whatsapp")]
fn load_whatsapp_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedWhatsappChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_whatsapp_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-whatsapp")]
pub(super) fn build_whatsapp_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedWhatsappChannelConfig>> {
    let resolved = config.whatsapp.resolve_account(account_id)?;
    let route = config
        .whatsapp
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "whatsapp account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-email")]
fn load_email_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedEmailChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_email_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-email")]
fn build_email_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedEmailChannelConfig>> {
    let resolved = config.email.resolve_account(account_id)?;
    let route = config
        .email
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "email account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-webhook")]
fn load_webhook_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedWebhookChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_webhook_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-webhook")]
fn build_webhook_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedWebhookChannelConfig>> {
    let resolved = config.webhook.resolve_account(account_id)?;
    let route = config
        .webhook
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "webhook account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-google-chat")]
fn load_google_chat_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedGoogleChatChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_google_chat_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-google-chat")]
fn build_google_chat_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedGoogleChatChannelConfig>> {
    let resolved = config.google_chat.resolve_account(account_id)?;
    let route = config
        .google_chat
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "google_chat account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-teams")]
fn load_teams_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedTeamsChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_teams_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-teams")]
fn build_teams_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedTeamsChannelConfig>> {
    let resolved = config.teams.resolve_account(account_id)?;
    let route = config
        .teams
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "teams account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-mattermost")]
fn load_mattermost_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedMattermostChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_mattermost_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-mattermost")]
fn build_mattermost_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedMattermostChannelConfig>> {
    let resolved = config.mattermost.resolve_account(account_id)?;
    let route = config
        .mattermost
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "mattermost account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-nextcloud-talk")]
fn load_nextcloud_talk_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedNextcloudTalkChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_nextcloud_talk_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-nextcloud-talk")]
fn build_nextcloud_talk_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedNextcloudTalkChannelConfig>> {
    let resolved = config.nextcloud_talk.resolve_account(account_id)?;
    let route = config
        .nextcloud_talk
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "nextcloud_talk account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-synology-chat")]
fn load_synology_chat_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedSynologyChatChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_synology_chat_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-synology-chat")]
fn build_synology_chat_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedSynologyChatChannelConfig>> {
    let resolved = config.synology_chat.resolve_account(account_id)?;
    let route = config
        .synology_chat
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "synology_chat account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-irc")]
fn load_irc_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedIrcChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_irc_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-irc")]
fn build_irc_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedIrcChannelConfig>> {
    let resolved = config.irc.resolve_account(account_id)?;
    let route = config
        .irc
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "irc account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-imessage")]
fn load_imessage_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedImessageChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_imessage_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-imessage")]
fn build_imessage_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedImessageChannelConfig>> {
    let resolved = config.imessage.resolve_account(account_id)?;
    let route = config
        .imessage
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "imessage account `{}` is disabled by configuration",
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

#[cfg(feature = "channel-nostr")]
fn load_nostr_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedNostrChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_nostr_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-nostr")]
fn build_nostr_command_context(
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedNostrChannelConfig>> {
    let resolved = config.nostr.resolve_account(account_id)?;
    let route = config
        .nostr
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    if !resolved.enabled {
        return Err(format!(
            "nostr account `{}` is disabled by configuration",
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
    config: LoongClawConfig,
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
    config: LoongClawConfig,
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
    config: LoongClawConfig,
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
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
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
    runtime_state::load_channel_operation_runtime_for_account_from_dir(
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
    config: LoongClawConfig,
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
    config: LoongClawConfig,
    account_id: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    let context = build_wecom_command_context(resolved_path, config, account_id)?;
    run_wecom_channel_with_context(context, stop, initialize_runtime_environment).await
}

pub async fn run_background_channel_with_stop(
    channel_id: &str,
    resolved_path: PathBuf,
    config: LoongClawConfig,
    account_id: Option<&str>,
    stop: ChannelServeStopHandle,
    initialize_runtime_environment: bool,
) -> CliResult<()> {
    match channel_id {
        "telegram" => {
            #[cfg(feature = "channel-telegram")]
            {
                return run_telegram_channel_with_stop(
                    resolved_path,
                    config,
                    false,
                    account_id,
                    stop,
                    initialize_runtime_environment,
                )
                .await;
            }
            #[cfg(not(feature = "channel-telegram"))]
            {
                let _ = (
                    resolved_path,
                    config,
                    account_id,
                    stop,
                    initialize_runtime_environment,
                );
                return Err(
                    "telegram channel is disabled (enable feature `channel-telegram`)".to_owned(),
                );
            }
        }
        "feishu" => {
            #[cfg(feature = "channel-feishu")]
            {
                return run_feishu_channel_with_stop(
                    resolved_path,
                    config,
                    account_id,
                    None,
                    None,
                    stop,
                    initialize_runtime_environment,
                )
                .await;
            }
            #[cfg(not(feature = "channel-feishu"))]
            {
                let _ = (
                    resolved_path,
                    config,
                    account_id,
                    stop,
                    initialize_runtime_environment,
                );
                return Err(
                    "feishu channel is disabled (enable feature `channel-feishu`)".to_owned(),
                );
            }
        }
        "matrix" => {
            #[cfg(feature = "channel-matrix")]
            {
                return run_matrix_channel_with_stop(
                    resolved_path,
                    config,
                    false,
                    account_id,
                    stop,
                    initialize_runtime_environment,
                )
                .await;
            }
            #[cfg(not(feature = "channel-matrix"))]
            {
                let _ = (
                    resolved_path,
                    config,
                    account_id,
                    stop,
                    initialize_runtime_environment,
                );
                return Err(
                    "matrix channel is disabled (enable feature `channel-matrix`)".to_owned(),
                );
            }
        }
        "wecom" => {
            #[cfg(feature = "channel-wecom")]
            {
                return run_wecom_channel_with_stop(
                    resolved_path,
                    config,
                    account_id,
                    stop,
                    initialize_runtime_environment,
                )
                .await;
            }
            #[cfg(not(feature = "channel-wecom"))]
            {
                let _ = (
                    resolved_path,
                    config,
                    account_id,
                    stop,
                    initialize_runtime_environment,
                );
                return Err("wecom channel is disabled (enable feature `channel-wecom`)".to_owned());
            }
        }
        "whatsapp" => {
            #[cfg(feature = "channel-whatsapp")]
            {
                return run_whatsapp_channel_with_stop(
                    resolved_path,
                    config,
                    account_id,
                    stop,
                    initialize_runtime_environment,
                )
                .await;
            }
            #[cfg(not(feature = "channel-whatsapp"))]
            {
                let _ = (
                    resolved_path,
                    config,
                    account_id,
                    stop,
                    initialize_runtime_environment,
                );
                return Err(
                    "whatsapp channel is disabled (enable feature `channel-whatsapp`)".to_owned(),
                );
            }
        }
        _ => Err(format!("unsupported background channel `{channel_id}`")),
    }
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
))]
pub(crate) async fn send_text_to_known_session(
    config: &LoongClawConfig,
    session_id: &str,
    text: &str,
) -> CliResult<ChannelSendReceipt> {
    match parse_known_channel_session_send_target(config, session_id)? {
        KnownChannelSessionSendTarget::Telegram {
            account_id,
            chat_id,
            thread_id,
        } => {
            #[cfg(not(feature = "channel-telegram"))]
            {
                let _ = (config, account_id, chat_id, thread_id, text);
                Err("telegram channel is disabled (enable feature `channel-telegram`)".to_owned())
            }

            #[cfg(feature = "channel-telegram")]
            {
                let resolved = config
                    .telegram
                    .resolve_account_for_session_account_id(account_id.as_deref())?;
                if !resolved.enabled {
                    return Err(
                        "sessions_send_channel_disabled: telegram channel is disabled by config"
                            .to_owned(),
                    );
                }
                let allowed_chat_id = chat_id.parse::<i64>().map_err(|error| {
                    format!("sessions_send_invalid_telegram_target: `{chat_id}`: {error}")
                })?;
                if !resolved.allowed_chat_ids.contains(&allowed_chat_id) {
                    return Err(format!(
                        "sessions_send_target_not_allowed: telegram target `{allowed_chat_id}` is not present in telegram.allowed_chat_ids"
                    ));
                }
                let token = resolved.bot_token().ok_or_else(|| {
                    "telegram bot token missing (set telegram.bot_token or env)".to_owned()
                })?;
                let target = match thread_id {
                    Some(thread_id) => format!("{chat_id}:topic:{thread_id}"),
                    None => chat_id,
                };
                telegram::run_telegram_send(
                    &resolved,
                    token,
                    ChannelOutboundTargetKind::Conversation,
                    target.as_str(),
                    text,
                )
                .await?;
                Ok(ChannelSendReceipt {
                    channel: "telegram",
                    target,
                })
            }
        }
        KnownChannelSessionSendTarget::Feishu {
            account_id,
            conversation_id,
            reply_message_id,
        } => {
            #[cfg(not(feature = "channel-feishu"))]
            {
                let _ = (config, account_id, conversation_id, reply_message_id, text);
                Err("feishu channel is disabled (enable feature `channel-feishu`)".to_owned())
            }

            #[cfg(feature = "channel-feishu")]
            {
                let resolved = config
                    .feishu
                    .resolve_account_for_session_account_id(account_id.as_deref())?;
                if !resolved.enabled {
                    return Err(
                        "sessions_send_channel_disabled: feishu channel is disabled by config"
                            .to_owned(),
                    );
                }
                if !crate::channel::feishu::feishu_allowlist_allows_chat(
                    &resolved.allowed_chat_ids,
                    &conversation_id,
                ) {
                    return Err(format!(
                        "sessions_send_target_not_allowed: feishu target `{conversation_id}` is not present in feishu.allowed_chat_ids"
                    ));
                }
                let (target_kind, target) = match reply_message_id {
                    Some(message_id) => (ChannelOutboundTargetKind::MessageReply, message_id),
                    None => (ChannelOutboundTargetKind::ReceiveId, conversation_id),
                };
                let request = match target_kind {
                    ChannelOutboundTargetKind::MessageReply => FeishuChannelSendRequest {
                        receive_id: target.clone(),
                        text: Some(text.to_owned()),
                        ..FeishuChannelSendRequest::default()
                    },
                    ChannelOutboundTargetKind::ReceiveId
                    | ChannelOutboundTargetKind::Conversation
                    | ChannelOutboundTargetKind::Address => FeishuChannelSendRequest {
                        receive_id: target.clone(),
                        receive_id_type: Some("chat_id".to_owned()),
                        text: Some(text.to_owned()),
                        ..FeishuChannelSendRequest::default()
                    },
                    ChannelOutboundTargetKind::Endpoint => {
                        return Err(
                            "sessions_send_invalid_target_kind: feishu session sends do not support endpoint targets"
                                .to_owned(),
                        );
                    }
                };
                feishu::run_feishu_send(&resolved, &request).await?;
                Ok(ChannelSendReceipt {
                    channel: "feishu",
                    target,
                })
            }
        }
        KnownChannelSessionSendTarget::Matrix {
            account_id,
            room_id,
        } => {
            #[cfg(not(feature = "channel-matrix"))]
            {
                let _ = (config, account_id, room_id, text);
                Err("matrix channel is disabled (enable feature `channel-matrix`)".to_owned())
            }

            #[cfg(feature = "channel-matrix")]
            {
                let resolved = config
                    .matrix
                    .resolve_account_for_session_account_id(account_id.as_deref())?;
                if !resolved.enabled {
                    return Err(
                        "sessions_send_channel_disabled: matrix channel is disabled by config"
                            .to_owned(),
                    );
                }
                if !resolved
                    .allowed_room_ids
                    .iter()
                    .any(|allowed| allowed.trim() == room_id)
                {
                    return Err(format!(
                        "sessions_send_target_not_allowed: matrix target `{room_id}` is not present in matrix.allowed_room_ids"
                    ));
                }
                let token = resolved.access_token().ok_or_else(|| {
                    "matrix access token missing (set matrix.access_token or env)".to_owned()
                })?;
                matrix::run_matrix_send(
                    &resolved,
                    token,
                    ChannelOutboundTargetKind::Conversation,
                    room_id.as_str(),
                    text,
                )
                .await?;
                Ok(ChannelSendReceipt {
                    channel: "matrix",
                    target: room_id,
                })
            }
        }
        KnownChannelSessionSendTarget::Wecom {
            account_id,
            conversation_id,
            chat_type,
        } => {
            #[cfg(not(feature = "channel-wecom"))]
            {
                let _ = (config, account_id, conversation_id, chat_type, text);
                Err("wecom channel is disabled (enable feature `channel-wecom`)".to_owned())
            }

            #[cfg(feature = "channel-wecom")]
            {
                let resolved = config
                    .wecom
                    .resolve_account_for_session_account_id(account_id.as_deref())?;
                if !resolved.enabled {
                    return Err(
                        "sessions_send_channel_disabled: wecom channel is disabled by config"
                            .to_owned(),
                    );
                }
                let is_allowed = resolved
                    .allowed_conversation_ids
                    .iter()
                    .any(|allowed| allowed.trim() == conversation_id);
                if !is_allowed {
                    return Err(format!(
                        "sessions_send_target_not_allowed: wecom target `{conversation_id}` is not present in wecom.allowed_conversation_ids"
                    ));
                }
                wecom::send_wecom_text(&resolved, conversation_id.as_str(), chat_type, text)
                    .await?;
                Ok(ChannelSendReceipt {
                    channel: "wecom",
                    target: conversation_id,
                })
            }
        }
    }
}

#[cfg(not(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom"
)))]
pub(crate) async fn send_text_to_known_session(
    _config: &crate::config::LoongClawConfig,
    session_id: &str,
    _text: &str,
) -> CliResult<ChannelSendReceipt> {
    Err(format!("sessions_send_channel_unsupported: `{session_id}`"))
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
pub(super) async fn process_inbound_with_runtime_and_feedback<R: ConversationRuntime + ?Sized>(
    config: &LoongClawConfig,
    runtime: &R,
    message: &ChannelInboundMessage,
    binding: ConversationRuntimeBinding<'_>,
    feedback_policy: ChannelTurnFeedbackPolicy,
) -> CliResult<String> {
    let address = message.session.conversation_address();
    let acp_turn_hints = resolve_channel_acp_turn_hints(config, &message.session)?;
    let acp_options = AcpConversationTurnOptions::automatic()
        .with_additional_bootstrap_mcp_servers(&acp_turn_hints.bootstrap_mcp_servers)
        .with_working_directory(acp_turn_hints.working_directory.as_deref())
        .with_provenance(channel_message_acp_turn_provenance(message));
    let ingress = channel_message_ingress_context(message);
    let feedback_capture = ChannelTurnFeedbackCapture::new(feedback_policy);
    let observer = feedback_capture.observer_handle();
    let reply = ConversationTurnCoordinator::new()
        .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
            config,
            &address,
            &message.text,
            ProviderErrorMode::Propagate,
            runtime,
            &acp_options,
            binding,
            ingress.as_ref(),
            observer,
        )
        .await?;
    Ok(feedback_capture.render_reply(reply))
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
pub(crate) async fn process_inbound_with_provider(
    config: &LoongClawConfig,
    resolved_path: Option<&std::path::Path>,
    message: &ChannelInboundMessage,
    kernel_ctx: &KernelContext,
    feedback_policy: ChannelTurnFeedbackPolicy,
) -> CliResult<String> {
    let started_at = std::time::Instant::now();
    let result = match reload_channel_turn_config(config, resolved_path) {
        Ok(turn_config) => match DefaultConversationRuntime::from_config_or_env(&turn_config) {
            Ok(runtime) => {
                process_inbound_with_runtime_and_feedback(
                    &turn_config,
                    &runtime,
                    message,
                    ConversationRuntimeBinding::kernel(kernel_ctx),
                    feedback_policy,
                )
                .await
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    let duration_ms = started_at.elapsed().as_millis();
    match &result {
        Ok(reply) => {
            tracing::debug!(
                target: "loongclaw.channel",
                platform = %message.session.platform.as_str(),
                conversation_id = %message.session.conversation_id,
                configured_account_id = ?message.session.configured_account_id.as_deref(),
                account_id = ?message.session.account_id.as_deref(),
                source_message_id = ?message.delivery.source_message_id.as_deref(),
                ack_cursor = ?message.delivery.ack_cursor.as_deref(),
                text_len = message.text.chars().count(),
                reply_len = reply.chars().count(),
                duration_ms,
                "channel inbound processed"
            );
        }
        Err(error) => {
            tracing::warn!(
                target: "loongclaw.channel",
                platform = %message.session.platform.as_str(),
                conversation_id = %message.session.conversation_id,
                configured_account_id = ?message.session.configured_account_id.as_deref(),
                account_id = ?message.session.account_id.as_deref(),
                source_message_id = ?message.delivery.source_message_id.as_deref(),
                ack_cursor = ?message.delivery.ack_cursor.as_deref(),
                text_len = message.text.chars().count(),
                duration_ms,
                error = %crate::observability::summarize_error(error),
                "channel inbound failed"
            );
        }
    }
    result
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
pub(super) fn reload_channel_turn_config(
    config: &LoongClawConfig,
    resolved_path: Option<&std::path::Path>,
) -> CliResult<LoongClawConfig> {
    match resolved_path {
        Some(path) => config.reload_provider_runtime_state_from_path(path),
        None => Ok(config.clone()),
    }
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
fn resolve_channel_acp_turn_hints(
    config: &LoongClawConfig,
    session: &ChannelSession,
) -> CliResult<ChannelResolvedAcpTurnHints> {
    match session.platform {
        ChannelPlatform::Telegram => {
            let resolved = config
                .telegram
                .resolve_account_for_session_account_id(session.account_id.as_deref())?;
            let acp = resolved.acp;
            let working_directory = acp.resolved_working_directory();
            Ok(ChannelResolvedAcpTurnHints {
                bootstrap_mcp_servers: acp.bootstrap_mcp_servers,
                working_directory,
            })
        }
        ChannelPlatform::Feishu => {
            let resolved = config
                .feishu
                .resolve_account_for_session_account_id(session.account_id.as_deref())?;
            let acp = resolved.acp;
            let working_directory = acp.resolved_working_directory();
            Ok(ChannelResolvedAcpTurnHints {
                bootstrap_mcp_servers: acp.bootstrap_mcp_servers,
                working_directory,
            })
        }
        ChannelPlatform::Matrix => {
            let resolved = config
                .matrix
                .resolve_account_for_session_account_id(session.account_id.as_deref())?;
            let acp = resolved.acp;
            let working_directory = acp.resolved_working_directory();
            Ok(ChannelResolvedAcpTurnHints {
                bootstrap_mcp_servers: acp.bootstrap_mcp_servers,
                working_directory,
            })
        }
        ChannelPlatform::Wecom => {
            let resolved = config
                .wecom
                .resolve_account_for_session_account_id(session.account_id.as_deref())?;
            let acp = resolved.acp;
            let working_directory = acp.resolved_working_directory();
            Ok(ChannelResolvedAcpTurnHints {
                bootstrap_mcp_servers: acp.bootstrap_mcp_servers,
                working_directory,
            })
        }
        ChannelPlatform::WhatsApp => Ok(ChannelResolvedAcpTurnHints::default()),
        ChannelPlatform::Irc => Ok(ChannelResolvedAcpTurnHints::default()),
    }
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
fn channel_message_acp_turn_provenance(message: &ChannelInboundMessage) -> AcpTurnProvenance<'_> {
    AcpTurnProvenance {
        trace_id: None,
        source_message_id: message.delivery.source_message_id.as_deref(),
        ack_cursor: message.delivery.ack_cursor.as_deref(),
    }
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
pub(super) fn channel_message_ingress_context(
    message: &ChannelInboundMessage,
) -> Option<ConversationIngressContext> {
    let participant_id = trimmed_non_empty(message.session.participant_id.as_deref());
    let thread_id = trimmed_non_empty(message.session.thread_id.as_deref());
    let resources = message
        .delivery
        .resources
        .iter()
        .filter_map(normalized_channel_delivery_resource)
        .collect::<Vec<_>>();
    let delivery = ConversationIngressDelivery {
        source_message_id: trimmed_non_empty(message.delivery.source_message_id.as_deref()),
        sender_identity_key: trimmed_non_empty(message.delivery.sender_principal_key.as_deref()),
        thread_root_id: trimmed_non_empty(message.delivery.thread_root_id.as_deref()),
        parent_message_id: trimmed_non_empty(message.delivery.parent_message_id.as_deref()),
        resources,
    };
    let has_contextual_hints = participant_id.is_some()
        || thread_id.is_some()
        || delivery != ConversationIngressDelivery::default();
    if !has_contextual_hints {
        return None;
    }

    let conversation_id = message.session.conversation_id.trim();
    if conversation_id.is_empty() {
        return None;
    }

    Some(ConversationIngressContext {
        channel: ConversationIngressChannel {
            platform: message.session.platform.as_str().to_owned(),
            configured_account_id: trimmed_non_empty(
                message.session.configured_account_id.as_deref(),
            ),
            account_id: trimmed_non_empty(message.session.account_id.as_deref()),
            conversation_id: conversation_id.to_owned(),
            participant_id,
            thread_id,
        },
        delivery,
        private: ConversationIngressPrivateContext {
            feishu_callback: normalized_feishu_callback_context(
                message.delivery.feishu_callback.as_ref(),
            ),
        },
    })
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
fn trimmed_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
fn normalized_channel_delivery_resource(
    resource: &ChannelDeliveryResource,
) -> Option<ConversationIngressDeliveryResource> {
    let resource_type = resource.resource_type.trim();
    let file_key = resource.file_key.trim();
    if resource_type.is_empty() || file_key.is_empty() {
        return None;
    }

    Some(ConversationIngressDeliveryResource {
        resource_type: resource_type.to_owned(),
        file_key: file_key.to_owned(),
        file_name: trimmed_non_empty(resource.file_name.as_deref()),
    })
}

#[cfg(any(
    feature = "channel-telegram",
    feature = "channel-feishu",
    feature = "channel-matrix",
    feature = "channel-wecom",
    feature = "channel-whatsapp"
))]
fn normalized_feishu_callback_context(
    callback: Option<&ChannelDeliveryFeishuCallback>,
) -> Option<ConversationIngressFeishuCallbackContext> {
    let callback = callback?;
    let normalized = ConversationIngressFeishuCallbackContext {
        callback_token: trimmed_non_empty(callback.callback_token.as_deref()),
        open_message_id: trimmed_non_empty(callback.open_message_id.as_deref()),
        open_chat_id: trimmed_non_empty(callback.open_chat_id.as_deref()),
        operator_open_id: trimmed_non_empty(callback.operator_open_id.as_deref()),
        deferred_context_id: trimmed_non_empty(callback.deferred_context_id.as_deref()),
    };
    if normalized.callback_token.is_none()
        && normalized.open_message_id.is_none()
        && normalized.open_chat_id.is_none()
        && normalized.operator_open_id.is_none()
        && normalized.deferred_context_id.is_none()
    {
        return None;
    }
    Some(normalized)
}

#[cfg(feature = "channel-telegram")]
pub(super) fn validate_telegram_security_config(
    config: &ResolvedTelegramChannelConfig,
) -> CliResult<()> {
    if config.allowed_chat_ids.is_empty() {
        return Err(
            "telegram.allowed_chat_ids is empty; configure at least one trusted chat id".to_owned(),
        );
    }
    Ok(())
}

#[cfg(feature = "channel-feishu")]
pub(super) fn validate_feishu_security_config(
    config: &ResolvedFeishuChannelConfig,
) -> CliResult<()> {
    let has_allowlist = config
        .allowed_chat_ids
        .iter()
        .any(|value| !value.trim().is_empty());
    if !has_allowlist {
        return Err(
            "feishu.allowed_chat_ids is empty; configure at least one trusted chat id".to_owned(),
        );
    }

    if config.mode != crate::config::FeishuChannelServeMode::Webhook {
        return Ok(());
    }

    let has_verification_token = config
        .verification_token()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_verification_token {
        return Err(
            "feishu.verification_token is missing; configure token or verification_token_env"
                .to_owned(),
        );
    }

    let has_encrypt_key = config
        .encrypt_key()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_encrypt_key {
        return Err("feishu.encrypt_key is missing; configure key or encrypt_key_env".to_owned());
    }

    Ok(())
}

#[cfg(feature = "channel-matrix")]
pub(super) fn validate_matrix_security_config(
    config: &ResolvedMatrixChannelConfig,
) -> CliResult<()> {
    let has_allowlist = config
        .allowed_room_ids
        .iter()
        .any(|value| !value.trim().is_empty());
    if !has_allowlist {
        return Err(
            "matrix.allowed_room_ids is empty; configure at least one trusted room id".to_owned(),
        );
    }

    let base_url = config.resolved_base_url().unwrap_or_default();
    matrix::build_matrix_client_url(base_url.as_str())?;

    let has_access_token = config
        .access_token()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_access_token {
        return Err(
            "matrix.access_token is missing; configure access_token or access_token_env".to_owned(),
        );
    }

    let has_user_id = config
        .user_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if config.ignore_self_messages && !has_user_id {
        return Err(
            "matrix.user_id is missing; configure user_id when ignore_self_messages is enabled"
                .to_owned(),
        );
    }

    Ok(())
}

#[cfg(feature = "channel-wecom")]
pub(super) fn validate_wecom_security_config(config: &ResolvedWecomChannelConfig) -> CliResult<()> {
    let has_allowlist = config
        .allowed_conversation_ids
        .iter()
        .any(|value| !value.trim().is_empty());
    if !has_allowlist {
        return Err(
            "wecom.allowed_conversation_ids is empty; configure at least one trusted conversation id"
                .to_owned(),
        );
    }

    let websocket_url = config.resolved_websocket_url();
    let parsed_url = reqwest::Url::parse(websocket_url.as_str())
        .map_err(|error| format!("invalid wecom.websocket_url: {error}"))?;
    let scheme = parsed_url.scheme();
    if scheme != "ws" && scheme != "wss" {
        return Err("wecom.websocket_url must use ws or wss".to_owned());
    }

    let has_bot_id = config
        .bot_id()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_bot_id {
        return Err("wecom.bot_id is missing; configure bot_id or bot_id_env".to_owned());
    }

    let has_secret = config
        .secret()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    if !has_secret {
        return Err("wecom.secret is missing; configure secret or secret_env".to_owned());
    }

    Ok(())
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
