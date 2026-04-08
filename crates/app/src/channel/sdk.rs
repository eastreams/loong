use std::sync::OnceLock;

use crate::{
    CliResult,
    config::{ConfigValidationIssue, LoongClawConfig},
};

use super::registry::{
    ChannelRuntimeCommandDescriptor, resolve_channel_catalog_entry,
    resolve_channel_command_family_descriptor, resolve_channel_selection_order,
};

#[cfg(feature = "channel-feishu")]
use super::registry::FEISHU_RUNTIME_COMMAND_DESCRIPTOR;

#[cfg(feature = "channel-matrix")]
use super::registry::MATRIX_RUNTIME_COMMAND_DESCRIPTOR;

#[cfg(feature = "channel-telegram")]
use super::registry::TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR;

#[cfg(feature = "channel-wecom")]
use super::registry::WECOM_RUNTIME_COMMAND_DESCRIPTOR;

#[cfg(feature = "channel-whatsapp")]
use super::registry::WHATSAPP_RUNTIME_COMMAND_DESCRIPTOR;

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
    pub channel_id: &'static str,
    pub background_runtime: Option<ChannelRuntimeCommandDescriptor>,
    pub is_enabled: ChannelEnabledFn,
    pub collect_validation_issues: ChannelValidationFn,
    pub background_surface_is_enabled: Option<BackgroundSurfaceEnabledFn>,
}

static CHANNEL_DESCRIPTORS: OnceLock<Vec<ChannelDescriptor>> = OnceLock::new();

const CLI_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "cli",
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
    channel_id: "telegram",
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
    channel_id: "feishu",
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
    channel_id: "matrix",
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
    channel_id: "wecom",
    background_runtime: WECOM_BACKGROUND_RUNTIME,
    is_enabled: wecom_channel_is_enabled,
    collect_validation_issues: collect_wecom_channel_validation_issues,
    background_surface_is_enabled: Some(wecom_background_surface_is_enabled),
};

const WEIXIN_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "weixin",
    background_runtime: None,
    is_enabled: weixin_channel_is_enabled,
    collect_validation_issues: collect_weixin_channel_validation_issues,
    background_surface_is_enabled: None,
};

const QQBOT_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "qqbot",
    background_runtime: None,
    is_enabled: qqbot_channel_is_enabled,
    collect_validation_issues: collect_qqbot_channel_validation_issues,
    background_surface_is_enabled: None,
};

const ONEBOT_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "onebot",
    background_runtime: None,
    is_enabled: onebot_channel_is_enabled,
    collect_validation_issues: collect_onebot_channel_validation_issues,
    background_surface_is_enabled: None,
};

const DISCORD_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "discord",
    background_runtime: None,
    is_enabled: discord_channel_is_enabled,
    collect_validation_issues: collect_discord_channel_validation_issues,
    background_surface_is_enabled: None,
};

const SLACK_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "slack",
    background_runtime: None,
    is_enabled: slack_channel_is_enabled,
    collect_validation_issues: collect_slack_channel_validation_issues,
    background_surface_is_enabled: None,
};

const LINE_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "line",
    background_runtime: None,
    is_enabled: line_channel_is_enabled,
    collect_validation_issues: collect_line_channel_validation_issues,
    background_surface_is_enabled: None,
};

const DINGTALK_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "dingtalk",
    background_runtime: None,
    is_enabled: dingtalk_channel_is_enabled,
    collect_validation_issues: collect_dingtalk_channel_validation_issues,
    background_surface_is_enabled: None,
};

#[cfg(feature = "channel-whatsapp")]
const WHATSAPP_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> =
    Some(WHATSAPP_RUNTIME_COMMAND_DESCRIPTOR);

#[cfg(not(feature = "channel-whatsapp"))]
const WHATSAPP_BACKGROUND_RUNTIME: Option<ChannelRuntimeCommandDescriptor> = None;

#[cfg(feature = "channel-whatsapp")]
const WHATSAPP_BACKGROUND_SURFACE_IS_ENABLED: Option<BackgroundSurfaceEnabledFn> =
    Some(whatsapp_background_surface_is_enabled);

#[cfg(not(feature = "channel-whatsapp"))]
const WHATSAPP_BACKGROUND_SURFACE_IS_ENABLED: Option<BackgroundSurfaceEnabledFn> = None;

const WHATSAPP_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "whatsapp",
    background_runtime: WHATSAPP_BACKGROUND_RUNTIME,
    is_enabled: whatsapp_channel_is_enabled,
    collect_validation_issues: collect_whatsapp_channel_validation_issues,
    background_surface_is_enabled: WHATSAPP_BACKGROUND_SURFACE_IS_ENABLED,
};

const EMAIL_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "email",
    background_runtime: None,
    is_enabled: email_channel_is_enabled,
    collect_validation_issues: collect_email_channel_validation_issues,
    background_surface_is_enabled: None,
};

const WEBHOOK_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "webhook",
    background_runtime: None,
    is_enabled: webhook_channel_is_enabled,
    collect_validation_issues: collect_webhook_channel_validation_issues,
    background_surface_is_enabled: None,
};

const GOOGLE_CHAT_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor =
    ChannelIntegrationDescriptor {
        channel_id: "google-chat",
        background_runtime: None,
        is_enabled: google_chat_channel_is_enabled,
        collect_validation_issues: collect_google_chat_channel_validation_issues,
        background_surface_is_enabled: None,
    };

const SIGNAL_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "signal",
    background_runtime: None,
    is_enabled: signal_channel_is_enabled,
    collect_validation_issues: collect_signal_channel_validation_issues,
    background_surface_is_enabled: None,
};

const TWITCH_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "twitch",
    background_runtime: None,
    is_enabled: twitch_channel_is_enabled,
    collect_validation_issues: collect_twitch_channel_validation_issues,
    background_surface_is_enabled: None,
};

const TEAMS_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "teams",
    background_runtime: None,
    is_enabled: teams_channel_is_enabled,
    collect_validation_issues: collect_teams_channel_validation_issues,
    background_surface_is_enabled: None,
};

const MATTERMOST_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "mattermost",
    background_runtime: None,
    is_enabled: mattermost_channel_is_enabled,
    collect_validation_issues: collect_mattermost_channel_validation_issues,
    background_surface_is_enabled: None,
};

const NEXTCLOUD_TALK_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor =
    ChannelIntegrationDescriptor {
        channel_id: "nextcloud-talk",
        background_runtime: None,
        is_enabled: nextcloud_talk_channel_is_enabled,
        collect_validation_issues: collect_nextcloud_talk_channel_validation_issues,
        background_surface_is_enabled: None,
    };

const SYNOLOGY_CHAT_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor =
    ChannelIntegrationDescriptor {
        channel_id: "synology-chat",
        background_runtime: None,
        is_enabled: synology_chat_channel_is_enabled,
        collect_validation_issues: collect_synology_chat_channel_validation_issues,
        background_surface_is_enabled: None,
    };

const IRC_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "irc",
    background_runtime: None,
    is_enabled: irc_channel_is_enabled,
    collect_validation_issues: collect_irc_channel_validation_issues,
    background_surface_is_enabled: None,
};

const IMESSAGE_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "imessage",
    background_runtime: None,
    is_enabled: imessage_channel_is_enabled,
    collect_validation_issues: collect_imessage_channel_validation_issues,
    background_surface_is_enabled: None,
};

const TLON_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "tlon",
    background_runtime: None,
    is_enabled: tlon_channel_is_enabled,
    collect_validation_issues: collect_tlon_channel_validation_issues,
    background_surface_is_enabled: None,
};

const NOSTR_CHANNEL_INTEGRATION: ChannelIntegrationDescriptor = ChannelIntegrationDescriptor {
    channel_id: "nostr",
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
    WEIXIN_CHANNEL_INTEGRATION,
    QQBOT_CHANNEL_INTEGRATION,
    ONEBOT_CHANNEL_INTEGRATION,
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

fn channel_descriptors() -> &'static [ChannelDescriptor] {
    let descriptors = CHANNEL_DESCRIPTORS.get_or_init(build_channel_descriptors);
    descriptors.as_slice()
}

fn build_channel_descriptors() -> Vec<ChannelDescriptor> {
    let mut descriptors = Vec::with_capacity(CHANNEL_INTEGRATIONS.len());

    for integration in CHANNEL_INTEGRATIONS {
        let channel_id = integration.channel_id;
        let background_runtime = integration.background_runtime;
        let descriptor = build_channel_descriptor(channel_id, background_runtime);
        descriptors.push(descriptor);
    }

    descriptors
}

fn build_channel_descriptor(
    channel_id: &'static str,
    background_runtime: Option<ChannelRuntimeCommandDescriptor>,
) -> ChannelDescriptor {
    let label = channel_display_label(channel_id);
    let surface_label_text = channel_surface_label_text(channel_id);
    let surface_label = leak_channel_string(surface_label_text);
    let runtime_kind = channel_runtime_kind(channel_id);
    let serve_subcommand = channel_serve_subcommand(channel_id, background_runtime);

    ChannelDescriptor {
        id: channel_id,
        label,
        surface_label,
        runtime_kind,
        serve_subcommand,
    }
}

fn channel_display_label(channel_id: &'static str) -> &'static str {
    if channel_id == "cli" {
        return "cli";
    }

    let catalog_entry = resolve_channel_catalog_entry(channel_id);
    debug_assert!(
        catalog_entry.is_some(),
        "missing catalog metadata for integrated channel `{channel_id}`"
    );
    let Some(catalog_entry) = catalog_entry else {
        return channel_id;
    };
    catalog_entry.label
}

fn channel_surface_label_text(channel_id: &str) -> String {
    if channel_id == "qqbot" {
        return "qq bot channel".to_owned();
    }

    let normalized = channel_id.replace('-', " ");
    let surface_label = format!("{normalized} channel");
    surface_label
}

fn leak_channel_string(value: String) -> &'static str {
    let boxed = value.into_boxed_str();
    Box::leak(boxed)
}

fn channel_runtime_kind(channel_id: &str) -> ChannelRuntimeKind {
    if channel_id == "cli" {
        return ChannelRuntimeKind::Interactive;
    }

    ChannelRuntimeKind::Service
}

fn channel_serve_subcommand(
    channel_id: &str,
    background_runtime: Option<ChannelRuntimeCommandDescriptor>,
) -> Option<&'static str> {
    background_runtime?;

    let family_descriptor = resolve_channel_command_family_descriptor(channel_id);
    debug_assert!(
        family_descriptor.is_some(),
        "missing command-family metadata for `{channel_id}`"
    );
    let family_descriptor = family_descriptor?;
    let serve_operation = family_descriptor.serve();
    let serve_command = serve_operation.command;
    Some(serve_command)
}

pub(crate) fn channel_descriptor(id: &str) -> Option<&'static ChannelDescriptor> {
    let integration = find_channel_integration(id)?;
    let descriptor_id = integration.channel_id;
    let descriptors = channel_descriptors();
    let descriptor = descriptors
        .iter()
        .find(|descriptor| descriptor.id == descriptor_id)?;
    Some(descriptor)
}

fn ordered_channel_integrations() -> Vec<&'static ChannelIntegrationDescriptor> {
    let mut integrations = CHANNEL_INTEGRATIONS.iter().collect::<Vec<_>>();
    integrations.sort_by_key(|integration| channel_integration_order_key(integration));
    integrations
}

fn channel_integration_order_key(
    integration: &ChannelIntegrationDescriptor,
) -> (u8, u16, &'static str) {
    let channel_id = integration.channel_id;
    let runtime_kind = channel_runtime_kind(channel_id);
    let runtime_group = match runtime_kind {
        ChannelRuntimeKind::Interactive => 0_u8,
        ChannelRuntimeKind::Service => 1_u8,
    };
    let selection_order = if channel_id == "cli" {
        u16::MAX
    } else {
        let selection_order = resolve_channel_selection_order(channel_id);
        debug_assert!(
            selection_order.is_some(),
            "missing selection-order metadata for `{channel_id}`"
        );
        selection_order.unwrap_or(u16::MAX)
    };
    (runtime_group, selection_order, channel_id)
}

pub(crate) fn service_channel_descriptors() -> Vec<&'static ChannelDescriptor> {
    ordered_channel_integrations()
        .into_iter()
        .filter_map(|integration| channel_descriptor(integration.channel_id))
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
            let maybe_descriptor = channel_descriptor(integration.channel_id);
            let Some(descriptor) = maybe_descriptor else {
                return false;
            };
            let enabled = (integration.is_enabled)(config);
            let matches_runtime_kind =
                runtime_kind.is_none_or(|kind| descriptor.runtime_kind == kind);
            enabled && matches_runtime_kind
        })
        .map(|integration| integration.channel_id.to_owned())
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
        .find(|integration| integration.channel_id == id);

    if let Some(integration) = exact_integration {
        return Some(integration);
    }

    let normalized_id = super::registry::normalize_channel_catalog_id(id)?;

    CHANNEL_INTEGRATIONS
        .iter()
        .find(|integration| integration.channel_id == normalized_id)
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

fn weixin_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.weixin.enabled
}

fn qqbot_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.qqbot.enabled
}

fn onebot_channel_is_enabled(config: &LoongClawConfig) -> bool {
    config.onebot.enabled
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

fn collect_weixin_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.weixin.validate()
}

fn collect_qqbot_channel_validation_issues(config: &LoongClawConfig) -> Vec<ConfigValidationIssue> {
    config.qqbot.validate()
}

fn collect_onebot_channel_validation_issues(
    config: &LoongClawConfig,
) -> Vec<ConfigValidationIssue> {
    config.onebot.validate()
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

    let resolved = crate::channel::feishu::api::resolve_requested_feishu_account(
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

fn whatsapp_background_surface_is_enabled(
    config: &LoongClawConfig,
    account_id: Option<&str>,
) -> CliResult<bool> {
    if !config.whatsapp.enabled {
        return Ok(false);
    }
    let resolved = config.whatsapp.resolve_account(account_id)?;
    Ok(resolved.enabled)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use loongclaw_contracts::SecretRef;

    use super::*;

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

        assert_eq!(
            ids,
            vec![
                "telegram",
                "feishu",
                "matrix",
                "wecom",
                "weixin",
                "qqbot",
                "onebot",
                "discord",
                "slack",
                "line",
                "dingtalk",
                "whatsapp",
                "email",
                "webhook",
                "google-chat",
                "signal",
                "twitch",
                "teams",
                "mattermost",
                "nextcloud-talk",
                "synology-chat",
                "irc",
                "imessage",
                "nostr",
                "tlon",
            ]
        );
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

    #[test]
    fn channel_descriptor_lookup_normalizes_plugin_backed_aliases() {
        let weixin = channel_descriptor("wechat").expect("wechat alias should resolve");
        assert_eq!(weixin.id, "weixin");
        assert_eq!(weixin.label, "Weixin");
        assert_eq!(weixin.surface_label, "weixin channel");
        assert_eq!(weixin.serve_subcommand, None);

        let qqbot = channel_descriptor("qq").expect("qq alias should resolve");
        assert_eq!(qqbot.id, "qqbot");
        assert_eq!(qqbot.label, "QQ Bot");
        assert_eq!(qqbot.surface_label, "qq bot channel");
        assert_eq!(qqbot.serve_subcommand, None);

        let onebot = channel_descriptor("onebot-v11").expect("onebot alias should resolve");
        assert_eq!(onebot.id, "onebot");
        assert_eq!(onebot.label, "OneBot");
        assert_eq!(onebot.surface_label, "onebot channel");
        assert_eq!(onebot.serve_subcommand, None);
    }

    #[test]
    fn channel_descriptors_reuse_curated_registry_labels() {
        let feishu = channel_descriptor("feishu").expect("feishu descriptor");
        assert_eq!(feishu.label, "Feishu/Lark");
        assert_eq!(feishu.surface_label, "feishu channel");

        let google_chat = channel_descriptor("google-chat").expect("google chat descriptor");
        assert_eq!(google_chat.label, "Google Chat");
        assert_eq!(google_chat.surface_label, "google chat channel");
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
