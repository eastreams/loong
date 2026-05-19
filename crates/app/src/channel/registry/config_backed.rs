use std::path::Path;

use crate::channel::http;
use crate::config::{
    ChannelDefaultAccountSelectionSource, LoongConfig, ResolvedDingtalkChannelConfig,
    ResolvedDiscordChannelConfig, ResolvedEmailChannelConfig, ResolvedGoogleChatChannelConfig,
    ResolvedImessageChannelConfig, ResolvedIrcChannelConfig, ResolvedMattermostChannelConfig,
    ResolvedNextcloudTalkChannelConfig, ResolvedSignalChannelConfig, ResolvedSlackChannelConfig,
    ResolvedSynologyChatChannelConfig, ResolvedTeamsChannelConfig, ResolvedWebhookChannelConfig,
    parse_email_smtp_endpoint, parse_irc_server_endpoint,
};

use super::status_support::{
    build_invalid_dingtalk_snapshot, build_invalid_discord_snapshot, build_invalid_email_snapshot,
    build_invalid_google_chat_snapshot, build_invalid_imessage_snapshot,
    build_invalid_irc_snapshot, build_invalid_mattermost_snapshot,
    build_invalid_nextcloud_talk_snapshot, build_invalid_signal_snapshot,
    build_invalid_slack_snapshot, build_invalid_synology_chat_snapshot,
    build_invalid_teams_snapshot, build_invalid_webhook_snapshot,
};
use super::*;

pub(super) fn build_dingtalk_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-dingtalk");
    let http_policy = http::outbound_http_policy_from_config(config);
    let default_selection = config.dingtalk.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .dingtalk
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .dingtalk
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_dingtalk_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                ),
                Err(error) => build_invalid_dingtalk_snapshot(
                    descriptor,
                    compiled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                ),
            }
        })
        .collect()
}

pub(super) fn build_discord_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-discord");
    let http_policy = http::outbound_http_policy_from_config(config);
    let default_selection = config.discord.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .discord
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .discord
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_discord_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                ),
                Err(error) => build_invalid_discord_snapshot(
                    descriptor,
                    compiled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                ),
            }
        })
        .collect()
}

pub(super) fn build_slack_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-slack");
    let http_policy = http::outbound_http_policy_from_config(config);
    let default_selection = config.slack.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .slack
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .slack
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_slack_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                ),
                Err(error) => build_invalid_slack_snapshot(
                    descriptor,
                    compiled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                ),
            }
        })
        .collect()
}

fn summarize_email_status_endpoint(raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if !trimmed.contains("://") {
        return Some(trimmed.to_owned());
    }

    let parsed_url = reqwest::Url::parse(trimmed).ok()?;
    let scheme = parsed_url.scheme();
    let host = parsed_url.host_str()?.trim();
    let port = parsed_url.port();

    let mut summary = format!("{scheme}://{host}");
    if let Some(port) = port {
        let port_text = port.to_string();
        summary.push(':');
        summary.push_str(port_text.as_str());
    }

    Some(summary)
}

pub(super) fn build_email_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-email");
    let default_selection = config.email.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .email
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .email
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_email_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                ),
                Err(error) => build_invalid_email_snapshot(
                    descriptor,
                    compiled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                ),
            }
        })
        .collect()
}

pub(super) fn build_webhook_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-webhook");
    let http_policy = http::outbound_http_policy_from_config(config);
    let default_selection = config.webhook.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .webhook
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .webhook
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_webhook_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                    runtime_dir,
                    now_ms,
                ),
                Err(error) => build_invalid_webhook_snapshot(
                    descriptor,
                    compiled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                    runtime_dir,
                    now_ms,
                ),
            }
        })
        .collect()
}

pub(super) fn build_google_chat_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-google-chat");
    let http_policy = http::outbound_http_policy_from_config(config);
    let default_selection = config.google_chat.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .google_chat
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .google_chat
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_google_chat_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                ),
                Err(error) => build_invalid_google_chat_snapshot(
                    descriptor,
                    compiled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                ),
            }
        })
        .collect()
}

pub(super) fn build_signal_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-signal");
    let http_policy = http::outbound_http_policy_from_config(config);
    let default_selection = config.signal.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .signal
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .signal
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_signal_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                ),
                Err(error) => build_invalid_signal_snapshot(
                    descriptor,
                    compiled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                ),
            }
        })
        .collect()
}

pub(super) fn build_teams_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-teams");
    let http_policy = http::outbound_http_policy_from_config(config);
    let default_selection = config.teams.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .teams
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .teams
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_teams_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                ),
                Err(error) => build_invalid_teams_snapshot(
                    descriptor,
                    compiled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                ),
            }
        })
        .collect()
}

pub(super) fn build_mattermost_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-mattermost");
    let http_policy = http::outbound_http_policy_from_config(config);
    let default_selection = config.mattermost.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .mattermost
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .mattermost
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_mattermost_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                ),
                Err(error) => build_invalid_mattermost_snapshot(
                    descriptor,
                    compiled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                ),
            }
        })
        .collect()
}

pub(super) fn build_nextcloud_talk_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-nextcloud-talk");
    let http_policy = http::outbound_http_policy_from_config(config);
    let default_selection = config.nextcloud_talk.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .nextcloud_talk
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .nextcloud_talk
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_nextcloud_talk_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                ),
                Err(error) => build_invalid_nextcloud_talk_snapshot(
                    descriptor,
                    compiled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                ),
            }
        })
        .collect()
}

pub(super) fn build_synology_chat_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-synology-chat");
    let http_policy = http::outbound_http_policy_from_config(config);
    let default_selection = config.synology_chat.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .synology_chat
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .synology_chat
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_synology_chat_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                ),
                Err(error) => build_invalid_synology_chat_snapshot(
                    descriptor,
                    compiled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                ),
            }
        })
        .collect()
}

pub(super) fn build_imessage_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-imessage");
    let http_policy = http::outbound_http_policy_from_config(config);
    let default_selection = config.imessage.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .imessage
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .imessage
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_imessage_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                ),
                Err(error) => build_invalid_imessage_snapshot(
                    descriptor,
                    compiled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                ),
            }
        })
        .collect()
}

pub(super) fn build_irc_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-irc");
    let default_selection = config.irc.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .irc
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .irc
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_irc_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                ),
                Err(error) => build_invalid_irc_snapshot(
                    descriptor,
                    compiled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                ),
            }
        })
        .collect()
}

fn build_dingtalk_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedDingtalkChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: http::ChannelOutboundHttpPolicy,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();

    let webhook_url = resolved.webhook_url();
    if webhook_url.is_none() {
        send_issues.push("webhook_url is missing".to_owned());
    }
    let validated_webhook_url = webhook_url
        .as_deref()
        .and_then(|url| validate_http_url("webhook_url", url, http_policy, &mut send_issues));

    let send_operation = if !compiled {
        unsupported_operation(
            DINGTALK_SEND_OPERATION,
            "binary built without feature `channel-dingtalk`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            DINGTALK_SEND_OPERATION,
            "disabled by dingtalk account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(DINGTALK_SEND_OPERATION, send_issues)
    } else {
        ready_operation(DINGTALK_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            DINGTALK_SERVE_OPERATION,
            "binary built without feature `channel-dingtalk`".to_owned(),
        )
    } else {
        unsupported_operation(
            DINGTALK_SERVE_OPERATION,
            "dingtalk custom robot surface is outbound-only".to_owned(),
        )
    };

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
    ];
    if resolved.secret().is_some() {
        notes.push("signed_webhook=true".to_owned());
    }
    if is_default_account {
        notes.push("default_account=true".to_owned());
    }
    notes.push(format!(
        "default_account_source={}",
        default_account_source.as_str()
    ));

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id: resolved.configured_account_id.clone(),
        configured_account_label: resolved.configured_account_label.clone(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: resolved.enabled,
        api_base_url: validated_webhook_url
            .as_ref()
            .and(webhook_url.as_deref())
            .and_then(http::redact_endpoint_status_url),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_discord_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedDiscordChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: http::ChannelOutboundHttpPolicy,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.bot_token().is_none() {
        send_issues.push("bot_token is missing".to_owned());
    }

    let resolved_api_base_url = resolved.resolved_api_base_url();
    let api_base_url = validate_http_base_url(
        "api_base_url",
        resolved_api_base_url.as_str(),
        http_policy,
        &mut send_issues,
    );

    let send_operation = if !compiled {
        unsupported_operation(
            DISCORD_SEND_OPERATION,
            "binary built without feature `channel-discord`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            DISCORD_SEND_OPERATION,
            "disabled by discord account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(DISCORD_SEND_OPERATION, send_issues)
    } else {
        ready_operation(DISCORD_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            DISCORD_SERVE_OPERATION,
            "binary built without feature `channel-discord`".to_owned(),
        )
    } else {
        unsupported_operation(
            DISCORD_SERVE_OPERATION,
            "discord serve runtime is not implemented yet".to_owned(),
        )
    };

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
    ];
    if is_default_account {
        notes.push("default_account=true".to_owned());
    }
    notes.push(format!(
        "default_account_source={}",
        default_account_source.as_str()
    ));
    let mut reserved_runtime_fields = Vec::new();
    if resolved.application_id().is_some() {
        reserved_runtime_fields.push("application_id".to_owned());
        notes.push("reserved_runtime_field=application_id".to_owned());
    }
    if !resolved.allowed_guild_ids.is_empty() {
        reserved_runtime_fields.push(format!(
            "allowed_guild_ids:{}",
            resolved.allowed_guild_ids.len()
        ));
        notes.push(format!(
            "reserved_runtime_field=allowed_guild_ids:{}",
            resolved.allowed_guild_ids.len()
        ));
    }

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id: resolved.configured_account_id.clone(),
        configured_account_label: resolved.configured_account_label.clone(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: resolved.enabled,
        api_base_url: api_base_url
            .as_ref()
            .and_then(|_| http::redact_endpoint_status_url(resolved_api_base_url.as_str())),
        notes,
        reserved_runtime_fields,
        operations: vec![send_operation, serve_operation],
    }
}

fn build_slack_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedSlackChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: http::ChannelOutboundHttpPolicy,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.bot_token().is_none() {
        send_issues.push("bot_token is missing".to_owned());
    }

    let resolved_api_base_url = resolved.resolved_api_base_url();
    let api_base_url = validate_http_base_url(
        "api_base_url",
        resolved_api_base_url.as_str(),
        http_policy,
        &mut send_issues,
    );

    let send_operation = if !compiled {
        unsupported_operation(
            SLACK_SEND_OPERATION,
            "binary built without feature `channel-slack`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            SLACK_SEND_OPERATION,
            "disabled by slack account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(SLACK_SEND_OPERATION, send_issues)
    } else {
        ready_operation(SLACK_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            SLACK_SERVE_OPERATION,
            "binary built without feature `channel-slack`".to_owned(),
        )
    } else {
        unsupported_operation(
            SLACK_SERVE_OPERATION,
            "slack serve runtime is not implemented yet".to_owned(),
        )
    };

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
    ];
    if is_default_account {
        notes.push("default_account=true".to_owned());
    }
    notes.push(format!(
        "default_account_source={}",
        default_account_source.as_str()
    ));

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id: resolved.configured_account_id.clone(),
        configured_account_label: resolved.configured_account_label.clone(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: resolved.enabled,
        api_base_url: api_base_url
            .as_ref()
            .and_then(|_| http::redact_endpoint_status_url(resolved_api_base_url.as_str())),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_email_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedEmailChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();

    let smtp_host = resolved.smtp_host();
    if smtp_host.is_none() {
        send_issues.push("smtp_host is missing".to_owned());
    }

    let smtp_endpoint = smtp_host
        .as_deref()
        .map(parse_email_smtp_endpoint)
        .transpose();
    if let Err(error) = &smtp_endpoint {
        send_issues.push(format!("smtp_host is invalid: {error}"));
    }

    let smtp_username = resolved.smtp_username();
    if smtp_username.is_none() {
        send_issues.push("smtp_username is missing".to_owned());
    }

    let smtp_password = resolved.smtp_password();
    if smtp_password.is_none() {
        send_issues.push("smtp_password is missing".to_owned());
    }

    let from_address = resolved.from_address();
    if from_address.is_none() {
        send_issues.push("from_address is missing".to_owned());
    }

    let parsed_from_address = from_address
        .as_deref()
        .map(str::parse::<lettre::message::Mailbox>)
        .transpose();
    if let Err(error) = parsed_from_address {
        send_issues.push(format!("from_address is invalid: {error}"));
    }

    let send_operation = if !compiled {
        unsupported_operation(
            EMAIL_SEND_OPERATION,
            "binary built without feature `channel-email`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            EMAIL_SEND_OPERATION,
            "disabled by email account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(EMAIL_SEND_OPERATION, send_issues)
    } else {
        ready_operation(EMAIL_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            EMAIL_SERVE_OPERATION,
            "binary built without feature `channel-email`".to_owned(),
        )
    } else {
        unsupported_operation(
            EMAIL_SERVE_OPERATION,
            "email IMAP reply-loop serve runtime is not implemented yet".to_owned(),
        )
    };

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
    ];
    if let Some(from_address) = &from_address {
        notes.push(format!("from_address={from_address}"));
    }
    if is_default_account {
        notes.push("default_account=true".to_owned());
    }
    notes.push(format!(
        "default_account_source={}",
        default_account_source.as_str()
    ));

    let api_base_url = summarize_email_status_endpoint(smtp_host.as_deref());

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id: resolved.configured_account_id.clone(),
        configured_account_label: resolved.configured_account_label.clone(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: resolved.enabled,
        api_base_url,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_webhook_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedWebhookChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: http::ChannelOutboundHttpPolicy,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();

    let endpoint_url = resolved.endpoint_url();
    if endpoint_url.is_none() {
        send_issues.push("endpoint_url is missing".to_owned());
    }
    let validated_endpoint_url = endpoint_url
        .as_deref()
        .and_then(|url| validate_http_url("endpoint_url", url, http_policy, &mut send_issues));

    let auth_token = resolved.auth_token();
    let auth_validation = build_webhook_auth_header_from_parts(
        auth_token.as_deref(),
        resolved.auth_header_name.as_str(),
        resolved.auth_token_prefix.as_str(),
    );
    let auth_error = auth_validation.err();
    if let Some(error) = auth_error.as_ref() {
        send_issues.push(error.clone());
    }

    let payload_text_field = resolved.payload_text_field.trim();
    if resolved.payload_format == WebhookPayloadFormat::JsonText && payload_text_field.is_empty() {
        send_issues.push("payload_text_field is empty for json_text payload_format".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            WEBHOOK_SEND_OPERATION,
            "binary built without feature `channel-webhook`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            WEBHOOK_SEND_OPERATION,
            "disabled by webhook account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(WEBHOOK_SEND_OPERATION, send_issues)
    } else {
        ready_operation(WEBHOOK_SEND_OPERATION)
    };
    let send_operation = attach_runtime(
        ChannelPlatform::Webhook,
        WEBHOOK_SEND_OPERATION,
        send_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );

    let mut serve_issues = Vec::new();
    if resolved.signing_secret().is_none() {
        serve_issues.push("signing_secret is missing".to_owned());
    }

    let serve_operation = if !compiled {
        unsupported_operation(
            WEBHOOK_SERVE_OPERATION,
            "binary built without feature `channel-webhook`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            WEBHOOK_SERVE_OPERATION,
            "disabled by webhook account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(WEBHOOK_SERVE_OPERATION, serve_issues)
    } else {
        ready_operation(WEBHOOK_SERVE_OPERATION)
    };
    let serve_operation = attach_runtime(
        ChannelPlatform::Webhook,
        WEBHOOK_SERVE_OPERATION,
        serve_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
        format!("payload_format={}", resolved.payload_format.as_str()),
    ];
    if resolved.payload_format == WebhookPayloadFormat::JsonText {
        notes.push(format!("payload_text_field={payload_text_field}"));
    }
    if auth_token.is_some() {
        notes.push("auth_token_configured=true".to_owned());
    }
    let public_base_url = resolved
        .public_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if public_base_url.is_some() {
        notes.push("public_base_url_configured=true".to_owned());
    }
    if resolved.signing_secret().is_some() {
        notes.push("signing_secret_configured=true".to_owned());
    }
    if is_default_account {
        notes.push("default_account=true".to_owned());
    }
    notes.push(format!(
        "default_account_source={}",
        default_account_source.as_str()
    ));

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id: resolved.configured_account_id.clone(),
        configured_account_label: resolved.configured_account_label.clone(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: resolved.enabled,
        api_base_url: validated_endpoint_url
            .as_ref()
            .and(endpoint_url.as_deref())
            .and_then(http::redact_generic_webhook_status_url),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_google_chat_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedGoogleChatChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: http::ChannelOutboundHttpPolicy,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();

    let webhook_url = resolved.webhook_url();
    if webhook_url.is_none() {
        send_issues.push("webhook_url is missing".to_owned());
    }
    let validated_webhook_url = webhook_url
        .as_deref()
        .and_then(|url| validate_http_url("webhook_url", url, http_policy, &mut send_issues));

    let send_operation = if !compiled {
        unsupported_operation(
            GOOGLE_CHAT_SEND_OPERATION,
            "binary built without feature `channel-google-chat`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            GOOGLE_CHAT_SEND_OPERATION,
            "disabled by google_chat account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(GOOGLE_CHAT_SEND_OPERATION, send_issues)
    } else {
        ready_operation(GOOGLE_CHAT_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            GOOGLE_CHAT_SERVE_OPERATION,
            "binary built without feature `channel-google-chat`".to_owned(),
        )
    } else {
        unsupported_operation(
            GOOGLE_CHAT_SERVE_OPERATION,
            "google chat incoming webhook surface is outbound-only".to_owned(),
        )
    };

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
    ];
    if is_default_account {
        notes.push("default_account=true".to_owned());
    }
    notes.push(format!(
        "default_account_source={}",
        default_account_source.as_str()
    ));

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id: resolved.configured_account_id.clone(),
        configured_account_label: resolved.configured_account_label.clone(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: resolved.enabled,
        api_base_url: validated_webhook_url
            .as_ref()
            .and(webhook_url.as_deref())
            .and_then(http::redact_endpoint_status_url),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_mattermost_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedMattermostChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: http::ChannelOutboundHttpPolicy,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();

    let server_url = resolved.server_url();
    if server_url.is_none() {
        send_issues.push("server_url is missing".to_owned());
    }
    let validated_server_url = server_url
        .as_deref()
        .and_then(|url| validate_http_base_url("server_url", url, http_policy, &mut send_issues));
    if resolved.bot_token().is_none() {
        send_issues.push("bot_token is missing".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            MATTERMOST_SEND_OPERATION,
            "binary built without feature `channel-mattermost`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            MATTERMOST_SEND_OPERATION,
            "disabled by mattermost account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(MATTERMOST_SEND_OPERATION, send_issues)
    } else {
        ready_operation(MATTERMOST_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            MATTERMOST_SERVE_OPERATION,
            "binary built without feature `channel-mattermost`".to_owned(),
        )
    } else {
        unsupported_operation(
            MATTERMOST_SERVE_OPERATION,
            "mattermost serve runtime is not implemented yet".to_owned(),
        )
    };

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
    ];
    if is_default_account {
        notes.push("default_account=true".to_owned());
    }
    notes.push(format!(
        "default_account_source={}",
        default_account_source.as_str()
    ));

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id: resolved.configured_account_id.clone(),
        configured_account_label: resolved.configured_account_label.clone(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: resolved.enabled,
        api_base_url: validated_server_url
            .as_ref()
            .and(server_url.as_deref())
            .and_then(http::redact_endpoint_status_url),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_nextcloud_talk_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedNextcloudTalkChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: http::ChannelOutboundHttpPolicy,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();

    let server_url = resolved.server_url();
    if server_url.is_none() {
        send_issues.push("server_url is missing".to_owned());
    }
    let validated_server_url = server_url
        .as_deref()
        .and_then(|url| validate_http_base_url("server_url", url, http_policy, &mut send_issues));
    if resolved.shared_secret().is_none() {
        send_issues.push("shared_secret is missing".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            NEXTCLOUD_TALK_SEND_OPERATION,
            "binary built without feature `channel-nextcloud-talk`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            NEXTCLOUD_TALK_SEND_OPERATION,
            "disabled by nextcloud_talk account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(NEXTCLOUD_TALK_SEND_OPERATION, send_issues)
    } else {
        ready_operation(NEXTCLOUD_TALK_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            NEXTCLOUD_TALK_SERVE_OPERATION,
            "binary built without feature `channel-nextcloud-talk`".to_owned(),
        )
    } else {
        unsupported_operation(
            NEXTCLOUD_TALK_SERVE_OPERATION,
            "nextcloud talk bot callback serve is not implemented yet".to_owned(),
        )
    };

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
    ];
    if is_default_account {
        notes.push("default_account=true".to_owned());
    }
    notes.push(format!(
        "default_account_source={}",
        default_account_source.as_str()
    ));

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id: resolved.configured_account_id.clone(),
        configured_account_label: resolved.configured_account_label.clone(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: resolved.enabled,
        api_base_url: validated_server_url
            .as_ref()
            .and(server_url.as_deref())
            .and_then(http::redact_endpoint_status_url),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_synology_chat_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedSynologyChatChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: http::ChannelOutboundHttpPolicy,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();

    let incoming_url = resolved.incoming_url();
    if incoming_url.is_none() {
        send_issues.push("incoming_url is missing".to_owned());
    }
    let validated_incoming_url = incoming_url
        .as_deref()
        .and_then(|url| validate_http_url("incoming_url", url, http_policy, &mut send_issues));

    let send_operation = if !compiled {
        unsupported_operation(
            SYNOLOGY_CHAT_SEND_OPERATION,
            "binary built without feature `channel-synology-chat`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            SYNOLOGY_CHAT_SEND_OPERATION,
            "disabled by synology_chat account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(SYNOLOGY_CHAT_SEND_OPERATION, send_issues)
    } else {
        ready_operation(SYNOLOGY_CHAT_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            SYNOLOGY_CHAT_SERVE_OPERATION,
            "binary built without feature `channel-synology-chat`".to_owned(),
        )
    } else {
        unsupported_operation(
            SYNOLOGY_CHAT_SERVE_OPERATION,
            "synology chat outgoing webhook serve is not implemented yet".to_owned(),
        )
    };

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
    ];
    if !resolved.allowed_user_ids.is_empty() {
        let user_ids = resolved
            .allowed_user_ids
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>();
        notes.push(format!("allowed_user_ids={}", user_ids.join(",")));
    }
    if resolved.token().is_some() {
        notes.push("outgoing_webhook_token_configured=true".to_owned());
    }
    if is_default_account {
        notes.push("default_account=true".to_owned());
    }
    notes.push(format!(
        "default_account_source={}",
        default_account_source.as_str()
    ));

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id: resolved.configured_account_id.clone(),
        configured_account_label: resolved.configured_account_label.clone(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: resolved.enabled,
        api_base_url: validated_incoming_url
            .as_ref()
            .and(incoming_url.as_deref())
            .and_then(http::redact_endpoint_status_url),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_signal_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedSignalChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: http::ChannelOutboundHttpPolicy,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.signal_account().is_none() {
        send_issues.push("account is missing".to_owned());
    }

    let service_url = resolved.service_url();
    if service_url.is_none() {
        send_issues.push("service_url is missing".to_owned());
    }
    let validated_service_url = service_url
        .as_deref()
        .and_then(|url| validate_http_base_url("service_url", url, http_policy, &mut send_issues));

    let send_operation = if !compiled {
        unsupported_operation(
            SIGNAL_SEND_OPERATION,
            "binary built without feature `channel-signal`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            SIGNAL_SEND_OPERATION,
            "disabled by signal account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(SIGNAL_SEND_OPERATION, send_issues)
    } else {
        ready_operation(SIGNAL_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            SIGNAL_SERVE_OPERATION,
            "binary built without feature `channel-signal`".to_owned(),
        )
    } else {
        unsupported_operation(
            SIGNAL_SERVE_OPERATION,
            "signal serve runtime is not implemented yet".to_owned(),
        )
    };

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
    ];
    if let Some(signal_account) = resolved.signal_account() {
        notes.push(format!("signal_account={signal_account}"));
    }
    if is_default_account {
        notes.push("default_account=true".to_owned());
    }
    notes.push(format!(
        "default_account_source={}",
        default_account_source.as_str()
    ));

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id: resolved.configured_account_id.clone(),
        configured_account_label: resolved.configured_account_label.clone(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: resolved.enabled,
        api_base_url: validated_service_url
            .as_ref()
            .and(service_url.as_deref())
            .and_then(http::redact_endpoint_status_url),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_teams_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedTeamsChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: http::ChannelOutboundHttpPolicy,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();

    let webhook_url = resolved.webhook_url();
    if webhook_url.is_none() {
        send_issues.push("webhook_url is missing".to_owned());
    }
    let validated_webhook_url = webhook_url
        .as_deref()
        .and_then(|url| validate_http_url("webhook_url", url, http_policy, &mut send_issues));

    let send_operation = if !compiled {
        unsupported_operation(
            TEAMS_SEND_OPERATION,
            "binary built without feature `channel-teams`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            TEAMS_SEND_OPERATION,
            "disabled by teams account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(TEAMS_SEND_OPERATION, send_issues)
    } else {
        ready_operation(TEAMS_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            TEAMS_SERVE_OPERATION,
            "binary built without feature `channel-teams`".to_owned(),
        )
    } else {
        unsupported_operation(
            TEAMS_SERVE_OPERATION,
            "microsoft teams incoming webhook surface is outbound-only today".to_owned(),
        )
    };

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
    ];
    let serve_credentials_ready = resolved.app_id().is_some()
        && resolved.app_password().is_some()
        && resolved.tenant_id().is_some();
    if serve_credentials_ready {
        notes.push("future_serve_credentials_configured=true".to_owned());
    }
    if !resolved.allowed_conversation_ids.is_empty() {
        let allowed_conversation_ids = resolved.allowed_conversation_ids.join(",");
        notes.push(format!(
            "allowed_conversation_ids={allowed_conversation_ids}"
        ));
    }
    if is_default_account {
        notes.push("default_account=true".to_owned());
    }
    notes.push(format!(
        "default_account_source={}",
        default_account_source.as_str()
    ));

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id: resolved.configured_account_id.clone(),
        configured_account_label: resolved.configured_account_label.clone(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: resolved.enabled,
        api_base_url: validated_webhook_url
            .as_ref()
            .and(webhook_url.as_deref())
            .and_then(http::redact_generic_webhook_status_url),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_imessage_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedImessageChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: http::ChannelOutboundHttpPolicy,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();

    let bridge_url = resolved.bridge_url();
    if bridge_url.is_none() {
        send_issues.push("bridge_url is missing".to_owned());
    }
    let validated_bridge_url = bridge_url
        .as_deref()
        .and_then(|url| validate_http_base_url("bridge_url", url, http_policy, &mut send_issues));
    if resolved.bridge_token().is_none() {
        send_issues.push("bridge_token is missing".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            IMESSAGE_SEND_OPERATION,
            "binary built without feature `channel-imessage`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            IMESSAGE_SEND_OPERATION,
            "disabled by imessage account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(IMESSAGE_SEND_OPERATION, send_issues)
    } else {
        ready_operation(IMESSAGE_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            IMESSAGE_SERVE_OPERATION,
            "binary built without feature `channel-imessage`".to_owned(),
        )
    } else {
        unsupported_operation(
            IMESSAGE_SERVE_OPERATION,
            "imessage bridge sync runtime is not implemented yet".to_owned(),
        )
    };

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
    ];
    if !resolved.allowed_chat_ids.is_empty() {
        let allowed_chat_ids = resolved.allowed_chat_ids.join(",");
        notes.push(format!("allowed_chat_ids={allowed_chat_ids}"));
    }
    if is_default_account {
        notes.push("default_account=true".to_owned());
    }
    notes.push(format!(
        "default_account_source={}",
        default_account_source.as_str()
    ));

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id: resolved.configured_account_id.clone(),
        configured_account_label: resolved.configured_account_label.clone(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: resolved.enabled,
        api_base_url: validated_bridge_url
            .as_ref()
            .and(bridge_url.as_deref())
            .and_then(http::redact_endpoint_status_url),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_irc_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedIrcChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();

    let server = resolved.server();
    if server.is_none() {
        send_issues.push("server is missing".to_owned());
    }
    if let Some(server) = server.as_deref() {
        let parse_result = parse_irc_server_endpoint(server);
        if let Err(error) = parse_result {
            send_issues.push(format!("server is invalid: {error}"));
        }
    }

    if resolved.nickname().is_none() {
        send_issues.push("nickname is missing".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            IRC_SEND_OPERATION,
            "binary built without feature `channel-irc`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            IRC_SEND_OPERATION,
            "disabled by irc account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(IRC_SEND_OPERATION, send_issues)
    } else {
        ready_operation(IRC_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            IRC_SERVE_OPERATION,
            "binary built without feature `channel-irc`".to_owned(),
        )
    } else {
        unsupported_operation(
            IRC_SERVE_OPERATION,
            "irc relay-loop serve is not implemented yet".to_owned(),
        )
    };

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
    ];
    if let Some(nickname) = resolved.nickname() {
        notes.push(format!("nickname={nickname}"));
    }
    if let Some(username) = resolved.username() {
        notes.push(format!("username={username}"));
    }
    if !resolved.channel_names.is_empty() {
        let channel_names = resolved.channel_names.join(",");
        notes.push(format!("channel_names={channel_names}"));
    }
    if resolved.password().is_some() {
        notes.push("password_configured=true".to_owned());
    }
    if let Some(server) = server.as_deref() {
        let endpoint = parse_irc_server_endpoint(server);
        if let Ok(endpoint) = endpoint {
            let transport = match endpoint.transport {
                crate::config::IrcServerTransport::Plain => "irc",
                crate::config::IrcServerTransport::Tls => "ircs",
            };
            notes.push(format!("server_host={}", endpoint.host));
            notes.push(format!("server_port={}", endpoint.port));
            notes.push(format!("server_transport={transport}"));
        }
    }
    if is_default_account {
        notes.push("default_account=true".to_owned());
    }
    notes.push(format!(
        "default_account_source={}",
        default_account_source.as_str()
    ));

    ChannelStatusSnapshot {
        id: descriptor.id,
        configured_account_id: resolved.configured_account_id.clone(),
        configured_account_label: resolved.configured_account_label.clone(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: resolved.enabled,
        api_base_url: summarize_irc_status_endpoint(server.as_deref()),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn summarize_irc_status_endpoint(server: Option<&str>) -> Option<String> {
    let server = server?;
    let endpoint = parse_irc_server_endpoint(server).ok()?;
    let scheme = match endpoint.transport {
        crate::config::IrcServerTransport::Plain => "irc",
        crate::config::IrcServerTransport::Tls => "ircs",
    };
    let host = endpoint.host.as_str();
    let normalized_host = host.trim_start_matches('[');
    let normalized_host = normalized_host.trim_end_matches(']');
    let display_host = if normalized_host.contains(':') {
        format!("[{normalized_host}]")
    } else {
        normalized_host.to_owned()
    };
    Some(format!("{scheme}://{display_host}:{}", endpoint.port))
}
