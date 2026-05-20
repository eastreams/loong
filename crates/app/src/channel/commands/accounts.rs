use std::path::PathBuf;

use crate::{
    CliResult,
    config::{
        ChannelResolvedAccountRoute, FeishuChannelServeMode, LoongConfig,
        ResolvedDingtalkChannelConfig, ResolvedDiscordChannelConfig, ResolvedEmailChannelConfig,
        ResolvedFeishuChannelConfig, ResolvedGoogleChatChannelConfig,
        ResolvedImessageChannelConfig, ResolvedIrcChannelConfig, ResolvedLineChannelConfig,
        ResolvedMatrixChannelConfig, ResolvedMattermostChannelConfig,
        ResolvedNextcloudTalkChannelConfig, ResolvedNostrChannelConfig, ResolvedQqbotChannelConfig,
        ResolvedSlackChannelConfig, ResolvedSynologyChatChannelConfig, ResolvedTeamsChannelConfig,
        ResolvedTelegramChannelConfig, ResolvedWebhookChannelConfig, ResolvedWecomChannelConfig,
        ResolvedWhatsappChannelConfig,
    },
};

use super::context::ChannelCommandContext;
use crate::channel::{access_policy::ChannelInboundAccessPolicy, matrix};

fn finalize_command_context<R>(
    resolved_path: PathBuf,
    config: LoongConfig,
    enabled: bool,
    resolved: R,
    route: ChannelResolvedAccountRoute,
    disabled_message: impl FnOnce(&R) -> String,
) -> CliResult<ChannelCommandContext<R>> {
    if !enabled {
        return Err(disabled_message(&resolved));
    }

    Ok(ChannelCommandContext {
        resolved_path,
        config,
        resolved,
        route,
    })
}

#[cfg(feature = "channel-discord")]
pub(in crate::channel) fn load_discord_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedDiscordChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_discord_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-discord")]
pub(in crate::channel) fn build_discord_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedDiscordChannelConfig>> {
    let resolved = config.discord.resolve_account(account_id)?;
    let route = config
        .discord
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "discord account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-dingtalk")]
pub(in crate::channel) fn load_dingtalk_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedDingtalkChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_dingtalk_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-dingtalk")]
pub(in crate::channel) fn build_dingtalk_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedDingtalkChannelConfig>> {
    let resolved = config.dingtalk.resolve_account(account_id)?;
    let route = config
        .dingtalk
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "dingtalk account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-telegram")]
pub(in crate::channel) fn load_telegram_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedTelegramChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_telegram_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-telegram")]
pub(in crate::channel) fn build_telegram_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedTelegramChannelConfig>> {
    let resolved = config.telegram.resolve_account(account_id)?;
    let route = config
        .telegram
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "telegram account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-feishu")]
pub(in crate::channel) fn load_feishu_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedFeishuChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_feishu_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-feishu")]
pub(in crate::channel) fn build_feishu_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
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
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "feishu account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-matrix")]
pub(in crate::channel) fn load_matrix_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedMatrixChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_matrix_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-matrix")]
pub(in crate::channel) fn build_matrix_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedMatrixChannelConfig>> {
    let resolved = config.matrix.resolve_account(account_id)?;
    let route = config
        .matrix
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "matrix account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-wecom")]
pub(in crate::channel) fn load_wecom_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedWecomChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_wecom_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-wecom")]
pub(in crate::channel) fn build_wecom_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedWecomChannelConfig>> {
    let resolved = config.wecom.resolve_account(account_id)?;
    let route = config
        .wecom
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "wecom account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-slack")]
pub(in crate::channel) fn load_slack_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedSlackChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_slack_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-slack")]
pub(in crate::channel) fn build_slack_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedSlackChannelConfig>> {
    let resolved = config.slack.resolve_account(account_id)?;
    let route = config
        .slack
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "slack account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-line")]
pub(in crate::channel) fn load_line_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedLineChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_line_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-line")]
pub(in crate::channel) fn build_line_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedLineChannelConfig>> {
    let resolved = config.line.resolve_account(account_id)?;
    let route = config
        .line
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "line account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-whatsapp")]
pub(in crate::channel) fn load_whatsapp_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedWhatsappChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_whatsapp_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-whatsapp")]
pub(in crate::channel) fn build_whatsapp_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedWhatsappChannelConfig>> {
    let resolved = config.whatsapp.resolve_account(account_id)?;
    let route = config
        .whatsapp
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "whatsapp account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-qqbot")]
pub(in crate::channel) fn load_qqbot_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedQqbotChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_qqbot_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-qqbot")]
pub(in crate::channel) fn build_qqbot_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedQqbotChannelConfig>> {
    let resolved = config.qqbot.resolve_account(account_id)?;
    let route = config
        .qqbot
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "qqbot account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-email")]
pub(in crate::channel) fn load_email_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedEmailChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_email_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-email")]
pub(in crate::channel) fn build_email_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedEmailChannelConfig>> {
    let resolved = config.email.resolve_account(account_id)?;
    let route = config
        .email
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "email account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-webhook")]
pub(in crate::channel) fn load_webhook_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedWebhookChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_webhook_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-webhook")]
pub(in crate::channel) fn build_webhook_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedWebhookChannelConfig>> {
    let resolved = config.webhook.resolve_account(account_id)?;
    let route = config
        .webhook
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "webhook account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-google-chat")]
pub(in crate::channel) fn load_google_chat_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedGoogleChatChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_google_chat_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-google-chat")]
pub(in crate::channel) fn build_google_chat_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedGoogleChatChannelConfig>> {
    let resolved = config.google_chat.resolve_account(account_id)?;
    let route = config
        .google_chat
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "google_chat account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-teams")]
pub(in crate::channel) fn load_teams_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedTeamsChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_teams_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-teams")]
pub(in crate::channel) fn build_teams_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedTeamsChannelConfig>> {
    let resolved = config.teams.resolve_account(account_id)?;
    let route = config
        .teams
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "teams account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-mattermost")]
pub(in crate::channel) fn load_mattermost_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedMattermostChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_mattermost_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-mattermost")]
pub(in crate::channel) fn build_mattermost_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedMattermostChannelConfig>> {
    let resolved = config.mattermost.resolve_account(account_id)?;
    let route = config
        .mattermost
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "mattermost account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-nextcloud-talk")]
pub(in crate::channel) fn load_nextcloud_talk_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedNextcloudTalkChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_nextcloud_talk_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-nextcloud-talk")]
pub(in crate::channel) fn build_nextcloud_talk_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedNextcloudTalkChannelConfig>> {
    let resolved = config.nextcloud_talk.resolve_account(account_id)?;
    let route = config
        .nextcloud_talk
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "nextcloud_talk account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-synology-chat")]
pub(in crate::channel) fn load_synology_chat_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedSynologyChatChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_synology_chat_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-synology-chat")]
pub(in crate::channel) fn build_synology_chat_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedSynologyChatChannelConfig>> {
    let resolved = config.synology_chat.resolve_account(account_id)?;
    let route = config
        .synology_chat
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "synology_chat account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-irc")]
pub(in crate::channel) fn load_irc_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedIrcChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_irc_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-irc")]
pub(in crate::channel) fn build_irc_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedIrcChannelConfig>> {
    let resolved = config.irc.resolve_account(account_id)?;
    let route = config
        .irc
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "irc account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-imessage")]
pub(in crate::channel) fn load_imessage_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedImessageChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_imessage_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-imessage")]
pub(in crate::channel) fn build_imessage_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedImessageChannelConfig>> {
    let resolved = config.imessage.resolve_account(account_id)?;
    let route = config
        .imessage
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "imessage account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-nostr")]
pub(in crate::channel) fn load_nostr_command_context(
    config_path: Option<&str>,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedNostrChannelConfig>> {
    let (resolved_path, config) = crate::config::load(config_path)?;
    build_nostr_command_context(resolved_path, config, account_id)
}

#[cfg(feature = "channel-nostr")]
pub(in crate::channel) fn build_nostr_command_context(
    resolved_path: PathBuf,
    config: LoongConfig,
    account_id: Option<&str>,
) -> CliResult<ChannelCommandContext<ResolvedNostrChannelConfig>> {
    let resolved = config.nostr.resolve_account(account_id)?;
    let route = config
        .nostr
        .resolved_account_route(account_id, resolved.configured_account_id.as_str());
    finalize_command_context(
        resolved_path,
        config,
        resolved.enabled,
        resolved,
        route,
        |resolved| {
            format!(
                "nostr account `{}` is disabled by configuration",
                resolved.configured_account_id
            )
        },
    )
}

#[cfg(feature = "channel-telegram")]
pub(in crate::channel) fn validate_telegram_security_config(
    config: &ResolvedTelegramChannelConfig,
) -> CliResult<()> {
    let access_policy = ChannelInboundAccessPolicy::from_i64_lists(
        config.allowed_chat_ids.as_slice(),
        config.allowed_sender_ids.as_slice(),
    );
    if !access_policy.has_conversation_restrictions() {
        return Err(
            "telegram.allowed_chat_ids is empty; configure at least one trusted chat id".to_owned(),
        );
    }
    Ok(())
}

#[cfg(feature = "channel-feishu")]
pub(in crate::channel) fn validate_feishu_security_config(
    config: &ResolvedFeishuChannelConfig,
) -> CliResult<()> {
    let access_policy = ChannelInboundAccessPolicy::from_string_lists(
        config.allowed_chat_ids.as_slice(),
        config.allowed_sender_ids.as_slice(),
        true,
    );
    if !access_policy.has_conversation_restrictions() {
        return Err(
            "feishu.allowed_chat_ids is empty; configure at least one trusted chat id".to_owned(),
        );
    }

    if config.mode != FeishuChannelServeMode::Webhook {
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
pub(in crate::channel) fn validate_matrix_security_config(
    config: &ResolvedMatrixChannelConfig,
) -> CliResult<()> {
    let access_policy = ChannelInboundAccessPolicy::from_string_lists(
        config.allowed_room_ids.as_slice(),
        config.allowed_sender_ids.as_slice(),
        false,
    );
    if !access_policy.has_conversation_restrictions() {
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
    if config.require_mention && !has_user_id {
        return Err(
            "matrix.user_id is missing; configure user_id when require_mention is enabled"
                .to_owned(),
        );
    }

    Ok(())
}

#[cfg(feature = "channel-wecom")]
pub(in crate::channel) fn validate_wecom_security_config(
    config: &ResolvedWecomChannelConfig,
) -> CliResult<()> {
    let access_policy = ChannelInboundAccessPolicy::from_string_lists(
        config.allowed_conversation_ids.as_slice(),
        config.allowed_sender_ids.as_slice(),
        false,
    );
    if !access_policy.has_conversation_restrictions() {
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
