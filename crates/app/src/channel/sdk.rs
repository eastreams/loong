use crate::{
    CliResult,
    config::{ConfigValidationIssue, LoongClawConfig},
};

use super::registry::{
    ChannelRuntimeCommandDescriptor, FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    MATRIX_CATALOG_COMMAND_FAMILY_DESCRIPTOR, TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    WECOM_CATALOG_COMMAND_FAMILY_DESCRIPTOR, resolve_channel_selection_order,
};

#[cfg(feature = "channel-feishu")]
use super::registry::FEISHU_RUNTIME_COMMAND_DESCRIPTOR;

#[cfg(feature = "channel-matrix")]
use super::registry::MATRIX_RUNTIME_COMMAND_DESCRIPTOR;

#[cfg(feature = "channel-telegram")]
use super::registry::TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR;

#[cfg(feature = "channel-wecom")]
use super::registry::WECOM_RUNTIME_COMMAND_DESCRIPTOR;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelRuntimeKind {
    Interactive,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub surface_label: &'static str,
    pub runtime_kind: ChannelRuntimeKind,
    pub serve_subcommand: Option<&'static str>,
}

type ChannelEnabledFn = fn(&LoongClawConfig) -> bool;
type ChannelValidationFn = fn(&LoongClawConfig) -> Vec<ConfigValidationIssue>;
type BackgroundSurfaceEnabledFn = fn(&LoongClawConfig, Option<&str>) -> CliResult<bool>;

#[derive(Clone, Copy)]
pub(crate) struct ChannelIntegrationDescriptor {
    pub descriptor: &'static ChannelDescriptor,
    pub background_runtime: Option<ChannelRuntimeCommandDescriptor>,
    pub is_enabled: ChannelEnabledFn,
    pub collect_validation_issues: ChannelValidationFn,
    pub background_surface_is_enabled: Option<BackgroundSurfaceEnabledFn>,
}

const CLI_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "cli",
    label: "cli",
    surface_label: "cli channel",
    runtime_kind: ChannelRuntimeKind::Interactive,
    serve_subcommand: None,
};

const TELEGRAM_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "telegram",
    label: "telegram",
    surface_label: "telegram channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: Some(TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve.command),
};

const FEISHU_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "feishu",
    label: "feishu",
    surface_label: "feishu channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: Some(FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve.command),
};

const MATRIX_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "matrix",
    label: "matrix",
    surface_label: "matrix channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: Some(MATRIX_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve.command),
};

const WECOM_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "wecom",
    label: "wecom",
    surface_label: "wecom channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: Some(WECOM_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve.command),
};

const DISCORD_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "discord",
    label: "discord",
    surface_label: "discord channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const SLACK_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "slack",
    label: "slack",
    surface_label: "slack channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const LINE_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "line",
    label: "line",
    surface_label: "line channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const DINGTALK_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "dingtalk",
    label: "dingtalk",
    surface_label: "dingtalk channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const WHATSAPP_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "whatsapp",
    label: "whatsapp",
    surface_label: "whatsapp channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const EMAIL_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "email",
    label: "email",
    surface_label: "email channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const WEBHOOK_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "webhook",
    label: "webhook",
    surface_label: "webhook channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const GOOGLE_CHAT_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "google-chat",
    label: "google-chat",
    surface_label: "google chat channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const SIGNAL_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "signal",
    label: "signal",
    surface_label: "signal channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const TWITCH_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "twitch",
    label: "twitch",
    surface_label: "twitch channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const TEAMS_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "teams",
    label: "teams",
    surface_label: "teams channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const TLON_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "tlon",
    label: "tlon",
    surface_label: "tlon channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const MATTERMOST_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "mattermost",
    label: "mattermost",
    surface_label: "mattermost channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const NEXTCLOUD_TALK_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "nextcloud-talk",
    label: "nextcloud-talk",
    surface_label: "nextcloud talk channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const SYNOLOGY_CHAT_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "synology-chat",
    label: "synology-chat",
    surface_label: "synology chat channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const IRC_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "irc",
    label: "irc",
    surface_label: "irc channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const IMESSAGE_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "imessage",
    label: "imessage",
    surface_label: "imessage channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const NOSTR_CHANNEL_DESCRIPTOR: ChannelDescriptor = ChannelDescriptor {
    id: "nostr",
    label: "nostr",
    surface_label: "nostr channel",
    runtime_kind: ChannelRuntimeKind::Service,
    serve_subcommand: None,
};

const CLI_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &CLI_CHANNEL_DESCRIPTOR,
    background_runtime: None,
    is_enabled: cli_channel_is_enabled,
    collect_validation_issues: collect_cli_channel_validation_issues,
    background_surface_is_enabled: None,
};

#[cfg(feature = "channel-telegram")]
const TELEGRAM_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> =
    Some(TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR);

#[cfg(not(feature = "channel-telegram"))]
const TELEGRAM_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> = None;

const TELEGRAM_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &TELEGRAM_CHANNEL_DESCRIPTOR,
    background_runtime: TELEGRAM_BACKGROUND_RUNTIME,
    is_enabled: telegram_channel_is_enabled,
    collect_validation_issues: collect_telegram_channel_validation_issues,
    background_surface_is_enabled: Some(telegram_background_surface_is_enabled),
};

#[cfg(feature = "channel-feishu")]
const FEISHU_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> =
    Some(FEISHU_RUNTIME_COMMAND_DESCRIPTOR);

#[cfg(not(feature = "channel-feishu"))]
const FEISHU_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> = None;

const FEISHU_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &FEISHU_CHANNEL_DESCRIPTOR,
    background_runtime: FEISHU_BACKGROUND_RUNTIME,
    is_enabled: feishu_channel_is_enabled,
    collect_validation_issues: collect_feishu_channel_validation_issues,
    background_surface_is_enabled: Some(feishu_background_surface_is_enabled),
};

#[cfg(feature = "channel-matrix")]
const MATRIX_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> =
    Some(MATRIX_RUNTIME_COMMAND_DESCRIPTOR);

#[cfg(not(feature = "channel-matrix"))]
const MATRIX_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> = None;

const MATRIX_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &MATRIX_CHANNEL_DESCRIPTOR,
    background_runtime: MATRIX_BACKGROUND_RUNTIME,
    is_enabled: matrix_channel_is_enabled,
    collect_validation_issues: collect_matrix_channel_validation_issues,
    background_surface_is_enabled: Some(matrix_background_surface_is_enabled),
};

#[cfg(feature = "channel-wecom")]
const WECOM_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> =
    Some(WECOM_RUNTIME_COMMAND_DESCRIPTOR);

#[cfg(not(feature = "channel-wecom"))]
const WECOM_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> = None;

const WECOM_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &WECOM_CHANNEL_DESCRIPTOR,
    background_runtime: WECOM_BACKGROUND_RUNTIME,
    is_enabled: wecom_channel_is_enabled,
    collect_validation_issues: collect_wecom_channel_validation_issues,
    background_surface_is_enabled: Some(wecom_background_surface_is_enabled),
};

const DISCORD_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &DISCORD_CHANNEL_DESCRIPTOR,
    background_runtime: None,
    is_enabled: discord_channel_is_enabled,
    collect_validation_issues: collect_discord_channel_validation_issues,
    background_surface_is_enabled: None,
};

const SLACK_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &SLACK_CHANNEL_DESCRIPTOR,
    background_runtime: None,
    is_enabled: slack_channel_is_enabled,
    collect_validation_issues: collect_slack_channel_validation_issues,
    background_surface_is_enabled: None,
};

const LINE_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &LINE_CHANNEL_DESCRIPTOR,
    background_runtime: None,
    is_enabled: line_channel_is_enabled,
    collect_validation_issues: collect_line_channel_validation_issues,
    background_surface_is_enabled: None,
};

const DINGTALK_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &DINGTALK_CHANNEL_DESCRIPTOR,
    background_runtime: None,
    is_enabled: dingtalk_channel_is_enabled,
    collect_validation_issues: collect_dingtalk_channel_validation_issues,
    background_surface_is_enabled: None,
};

const WHATSAPP_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &WHATSAPP_CHANNEL_DESCRIPTOR,
    background_runtime: None,
    is_enabled: whatsapp_channel_is_enabled,
    collect_validation_issues: collect_whatsapp_channel_validation_issues,
    background_surface_is_enabled: None,
};

const EMAIL_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &EMAIL_CHANNEL_DESCRIPTOR,
    background_runtime: None,
    is_enabled: email_channel_is_enabled,
    collect_validation_issues: collect_email_channel_validation_issues,
    background_surface_is_enabled: None,
};

const WEBHOOK_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &WEBHOOK_CHANNEL_DESCRIPTOR,
    background_runtime: None,
    is_enabled: webhook_channel_is_enabled,
    collect_validation_issues: collect_webhook_channel_validation_issues,
    background_surface_is_enabled: None,
};

const GOOGLE_CHAT_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor =
    ChannelIntegrationDescriptor {
        descriptor: &GOOGLE_CHAT_CHANNEL_DESCRIPTOR,
        background_runtime: None,
        is_enabled: google_chat_channel_is_enabled,
        collect_validation_issues: collect_google_chat_channel_validation_issues,
        background_surface_is_enabled: None,
    };

const SIGNAL_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &SIGNAL_CHANNEL_DESCRIPTOR,
    background_runtime: None,
    is_enabled: signal_channel_is_enabled,
    collect_validation_issues: collect_signal_channel_validation_issues,
    background_surface_is_enabled: None,
};

const TWITCH_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &TWITCH_CHANNEL_DESCRIPTOR,
    background_runtime: None,
    is_enabled: twitch_channel_is_enabled,
    collect_validation_issues: collect_twitch_channel_validation_issues,
    background_surface_is_enabled: None,
};

const TEAMS_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &TEAMS_CHANNEL_DESCRIPTOR,
    background_runtime: None,
    is_enabled: teams_channel_is_enabled,
    collect_validation_issues: collect_teams_channel_validation_issues,
    background_surface_is_enabled: None,
};

const MATTERMOST_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &MATTERMOST_CHANNEL_DESCRIPTOR,
    background_runtime: None,
    is_enabled: mattermost_channel_is_enabled,
    collect_validation_issues: collect_mattermost_channel_validation_issues,
    background_surface_is_enabled: None,
};

const NEXTCLOUD_TALK_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor =
    ChannelIntegrationDescriptor {
        descriptor: &NEXTCLOUD_TALK_CHANNEL_DESCRIPTOR,
        background_runtime: None,
        is_enabled: nextcloud_talk_channel_is_enabled,
        collect_validation_issues: collect_nextcloud_talk_channel_validation_issues,
        background_surface_is_enabled: None,
    };

const SYNOLOGY_CHAT_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor =
    ChannelIntegrationDescriptor {
        descriptor: &SYNOLOGY_CHAT_CHANNEL_DESCRIPTOR,
        background_runtime: None,
        is_enabled: synology_chat_channel_is_enabled,
        collect_validation_issues: collect_synology_chat_channel_validation_issues,
        background_surface_is_enabled: None,
    };

const IRC_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &IRC_CHANNEL_DESCRIPTOR,
    background_runtime: None,
    is_enabled: irc_channel_is_enabled,
    collect_validation_issues: collect_irc_channel_validation_issues,
    background_surface_is_enabled: None,
};

const IMESSAGE_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &IMESSAGE_CHANNEL_DESCRIPTOR,
    background_runtime: None,
    is_enabled: imessage_channel_is_enabled,
    collect_validation_issues: collect_imessage_channel_validation_issues,
    background_surface_is_enabled: None,
};

const TLON_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &TLON_CHANNEL_DESCRIPTOR,
    background_runtime: None,
    is_enabled: tlon_channel_is_enabled,
    collect_validation_issues: collect_tlon_channel_validation_issues,
    background_surface_is_enabled: None,
};

const NOSTR_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    descriptor: &NOSTR_CHANNEL_DESCRIPTOR,
    background_runtime: None,
    is_enabled: nostr_channel_is_enabled,
    collect_validation_issues: collect_nostr_channel_validation_issues,
    background_surface_is_enabled: None,
};

const CHANNEL_INTEGRATIONS: &[ChannelIntegrationDescriptor] = &[
    CLI_CHANNEL_INTEGRATION,
    TELEGRAM_CHANNEL_INTEGRATION,
    FEISHU_CHANNEL_INTEGRATION,
    MATRIX_CHANNEL_INTEGRATION,
    WECOM_CHANNEL_INTEGRATION,
    DISCORD_CHANNEL_INTEGRATION,
    SLACK_CHANNEL_INTEGRATION,
    LINE_CHANNEL_INTEGRATION,
    DINGTALK_CHANNEL_INTEGRATION,
    WHATSAPP_CHANNEL_INTEGRATION,
    EMAIL_CHANNEL_INTEGRATION,
    WEBHOOK_CHANNEL_INTEGRATION,
    GOOGLE_CHAT_CHANNEL_INTEGRATION,
    SIGNAL_CHANNEL_INTEGRATION,
    TWITCH_CHANNEL_INTEGRATION,
    TEAMS_CHANNEL_INTEGRATION,
    TLON_CHANNEL_INTEGRATION,
    MATTERMOST_CHANNEL_INTEGRATION,
    NEXTCLOUD_TALK_CHANNEL_INTEGRATION,
    SYNOLOGY_CHAT_CHANNEL_INTEGRATION,
    IRC_CHANNEL_INTEGRATION,
    IMESSAGE_CHANNEL_INTEGRATION,
    NOSTR_CHANNEL_INTEGRATION,
];

pub(crate) fn channel_descriptor(id: &str) -> Option<&'static ChannelDescriptor> {
    let integration = find_channel_integration(id)?;
    Some(integration.descriptor)
}

fn ordered_channel_integrations() -> Vec<&'static ChannelIntegrationDescriptor> {
    let mut integrations = CHANNEL_INTEGRATIONS.iter().collect::<Vec<_>>();
    integrations.sort_by_key(|integration| channel_integration_order_key(integration));
    integrations
}

fn channel_integration_order_key(
    integration: &ChannelIntegrationDescriptor,
) -> (u8, u16, &'static str) {
    let runtime_group = match integration.descriptor.runtime_kind {
        ChannelRuntimeKind::Interactive => 0_u8,
        ChannelRuntimeKind::Service => 1_u8,
    };
    let selection_order =
        resolve_channel_selection_order(integration.descriptor.id).unwrap_or(u16::MAX);
    (runtime_group, selection_order, integration.descriptor.id)
}

pub(crate) fn service_channel_descriptors() -> Vec<&'static ChannelDescriptor> {
    ordered_channel_integrations()
        .into_iter()
        .map(|integration| integration.descriptor)
        .filter(|descriptor| descriptor.runtime_kind == ChannelRuntimeKind::Service)
        .collect()
}

pub(crate) fn enabled_channel_ids(
    config: &LoongClawConfig,
    runtime_kind: Option<ChannelRuntimeKind>,
) -> Vec<String> {
    ordered_channel_integrations()
        .into_iter()
        .filter(|integration| {
            let enabled = (integration.is_enabled)(config);
            let matches_runtime_kind =
                runtime_kind.is_none_or(|kind| integration.descriptor.runtime_kind == kind);
            enabled && matches_runtime_kind
        })
        .map(|integration| integration.descriptor.id.to_owned())
        .collect()
}

pub(crate) fn collect_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    ordered_channel_integrations()
        .into_iter()
        .flat_map(|integration| (integration.collect_validation_issues)(config))
        .collect()
}

pub fn background_channel_runtime_descriptors() -> Vec<ChannelRuntimeCommandDescriptor> {
    ordered_channel_integrations()
        .into_iter()
        .filter_map(|integration| integration.background_runtime)
        .collect()
}

pub fn is_background_channel_surface_enabled(
    channel_id: &str,
    config: &LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<bool> {
    let integration = find_channel_integration(channel_id)
        .ok_or_else(|| format!("unsupported background channel `{channel_id}`"))?;
    let surface_is_enabled = integration
        .background_surface_is_enabled
        .ok_or_else(|| format!("unsupported background channel `{channel_id}`"))?;
    surface_is_enabled(config, account_id)
}

fn find_channel_integration(id: &str) -> Option<&'static ChannelIntegrationDescriptor> {
    let exact_integration = CHANNEL_INTEGRATIONS
        .iter()
        .find(|integration| integration.descriptor.id == id);

    if let Some(integration) = exact_integration {
        return Some(integration);
    }

    let normalized_id = super::registry::normalize_channel_catalog_id(id)?;

    CHANNEL_INTEGRATIONS
        .iter()
        .find(|integration| integration.descriptor.id == normalized_id)
}

fn cli_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.cli.enabled
}

fn telegram_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.telegram.enabled
}

fn feishu_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.feishu.enabled
}

fn matrix_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.matrix.enabled
}

fn wecom_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.wecom.enabled
}

fn discord_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.discord.enabled
}

fn slack_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.slack.enabled
}

fn line_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.line.enabled
}

fn dingtalk_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.dingtalk.enabled
}

fn whatsapp_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.whatsapp.enabled
}

fn email_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.email.enabled
}

fn webhook_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.webhook.enabled
}

fn google_chat_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.google_chat.enabled
}

fn signal_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.signal.enabled
}

fn twitch_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.twitch.enabled
}

fn teams_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.teams.enabled
}

fn tlon_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.tlon.enabled
}

fn mattermost_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.mattermost.enabled
}

fn nextcloud_talk_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.nextcloud_talk.enabled
}

fn synology_chat_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.synology_chat.enabled
}

fn irc_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.irc.enabled
}

fn imessage_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.imessage.enabled
}

fn nostr_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.nostr.enabled
}

fn collect_cli_channel_validation_issues(_config: &LoongClawConfig) -> Vec<ConfigValidationIssue> {
    Vec::new()
}

fn collect_telegram_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.telegram.validate()
}

fn collect_feishu_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.feishu.validate()
}

fn collect_matrix_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.matrix.validate()
}

fn collect_wecom_channel_validation_issues(config: &LoongClawConfig) -> Vec<ConfigValidationIssue> {
    config.wecom.validate()
}

fn collect_discord_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.discord.validate()
}

fn collect_slack_channel_validation_issues(config: &LoongClawConfig) -> Vec<ConfigValidationIssue> {
    config.slack.validate()
}

fn collect_line_channel_validation_issues(config: &LoongClawConfig) -> Vec<ConfigValidationIssue> {
    config.line.validate()
}

fn collect_dingtalk_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.dingtalk.validate()
}

fn collect_whatsapp_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.whatsapp.validate()
}

fn collect_email_channel_validation_issues(config: &LoongClawConfig) -> Vec<ConfigValidationIssue> {
    config.email.validate()
}

fn collect_webhook_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.webhook.validate()
}

fn collect_google_chat_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.google_chat.validate()
}

fn collect_signal_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.signal.validate()
}

fn collect_twitch_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.twitch.validate()
}

fn collect_teams_channel_validation_issues(config: &LoongClawConfig) -> Vec<ConfigValidationIssue> {
    config.teams.validate()
}

fn collect_tlon_channel_validation_issues(config: &LoongClawConfig) -> Vec<ConfigValidationIssue> {
    config.tlon.validate()
}

fn collect_mattermost_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.mattermost.validate()
}

fn collect_nextcloud_talk_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.nextcloud_talk.validate()
}

fn collect_synology_chat_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.synology_chat.validate()
}

fn collect_irc_channel_validation_issues(config: &LoongClawConfig) -> Vec<ConfigValidationIssue> {
    config.irc.validate()
}

fn collect_imessage_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.imessage.validate()
}

fn collect_nostr_channel_validation_issues(config: &LoongClawConfig) -> Vec<ConfigValidationIssue> {
    config.nostr.validate()
}

fn telegram_background_surface_is_enabled(
    config: &LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<bool> {
    if !config.telegram.enabled {
        return Ok(false);
    }
    let resolved = config.telegram.resolve_account(account_id)?;
    Ok(resolved.enabled)
}

#[cfg(feature = "feishu-integration")]
fn feishu_background_surface_is_enabled(
    config: &LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<bool> {
    if !config.feishu.enabled {
        return Ok(false);
    }

    let resolved = crate::feishu::resolve_requested_feishu_account(
        &config.feishu,
        account_id,
        "rerun with `--channel-account <CHANNEL=ACCOUNT>` using one of those configured accounts",
    )?;
    Ok(resolved.enabled)
}

#[cfg(not(feature = "feishu-integration"))]
fn feishu_background_surface_is_enabled(
    config: &LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<bool> {
    if !config.feishu.enabled {
        return Ok(false);
    }

    let resolved = config.feishu.resolve_account(account_id)?;
    Ok(resolved.enabled)
}

fn matrix_background_surface_is_enabled(
    config: &LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<bool> {
    if !config.matrix.enabled {
        return Ok(false);
    }
    let resolved = config.matrix.resolve_account(account_id)?;
    Ok(resolved.enabled)
}

fn wecom_background_surface_is_enabled(
    config: &LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<bool> {
    if !config.wecom.enabled {
        return Ok(false);
    }
    let resolved = config.wecom.resolve_account(account_id)?;
    Ok(resolved.enabled)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use loongclaw_contracts::SecretRef;

    use super::*;

    fn expected_service_channel_ids() -> Vec<&'static str> {
        let mut channel_ids = Vec::new();
        let catalog = super::super::registry::list_channel_catalog();

        for catalog_entry in catalog {
            let Some(descriptor) = channel_descriptor(catalog_entry.id) else {
                continue;
            };
            if descriptor.runtime_kind != ChannelRuntimeKind::Service {
                continue;
            }
            channel_ids.push(descriptor.id);
        }

        channel_ids
    }

    fn expected_background_channel_ids() -> Vec<&'static str> {
        let mut channel_ids = Vec::new();
        let catalog = super::super::registry::list_channel_catalog();

        for catalog_entry in catalog {
            let runtime_descriptor =
                super::super::registry::resolve_channel_runtime_command_descriptor(
                    catalog_entry.id,
                );
            if runtime_descriptor.is_none() {
                continue;
            }
            let Some(descriptor) = channel_descriptor(catalog_entry.id) else {
                continue;
            };
            if descriptor.runtime_kind != ChannelRuntimeKind::Service {
                continue;
            }
            channel_ids.push(descriptor.id);
        }

        channel_ids
    }

    #[test]
    fn service_channel_descriptors_follow_registry_selection_order() {
        let descriptors = service_channel_descriptors();
        let ids = descriptors
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();
        let expected_ids = expected_service_channel_ids();
        assert_eq!(ids, expected_ids);
    }

    #[test]
    fn background_channel_runtime_descriptors_follow_registry_selection_order() {
        let descriptors = background_channel_runtime_descriptors();
        let ids = descriptors
            .into_iter()
            .map(|descriptor| descriptor.channel_id)
            .collect::<Vec<_>>();
        let expected_ids = expected_background_channel_ids();

        assert_eq!(ids, expected_ids);
    }

    #[test]
    fn unsupported_background_channels_are_rejected() {
        let config = LoongClawConfig::default();
        let error = is_background_channel_surface_enabled("cli", &config, None)
            .expect_err("cli should not be a background channel");

        assert_eq!(error, "unsupported background channel `cli`");
    }

    #[test]
    fn background_channel_surface_enablement_normalizes_aliases() {
        let config = LoongClawConfig::default();
        let enabled = is_background_channel_surface_enabled(" LARK ", &config, None)
            .expect("feishu alias should normalize through the channel registry");

        assert!(!enabled);
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn feishu_background_surface_enablement_accepts_runtime_account_aliases() {
        let shared_account_id = "feishu_shared".to_owned();
        let work_account = crate::config::FeishuAccountConfig {
            account_id: Some(shared_account_id.clone()),
            app_id: Some(SecretRef::Inline("cli_work".to_owned())),
            app_secret: Some(SecretRef::Inline("app-secret-work".to_owned())),
            ..crate::config::FeishuAccountConfig::default()
        };
        let accounts = BTreeMap::from([("work".to_owned(), work_account)]);
        let feishu = crate::config::FeishuChannelConfig {
            enabled: true,
            accounts,
            ..crate::config::FeishuChannelConfig::default()
        };
        let config = LoongClawConfig {
            feishu,
            ..LoongClawConfig::default()
        };

        let enabled = is_background_channel_surface_enabled(
            "feishu",
            &config,
            Some(shared_account_id.as_str()),
        )
        .expect("resolve unique feishu runtime-account alias");

        assert!(enabled);
    }
}
