use std::path::Path;

use serde::Serialize;

mod descriptors;
mod planned;
mod tlon;
mod twitch;

use crate::config::{
    ChannelDefaultAccountSelectionSource, DINGTALK_SECRET_ENV, DINGTALK_WEBHOOK_URL_ENV,
    DISCORD_BOT_TOKEN_ENV, FEISHU_APP_ID_ENV, FEISHU_APP_SECRET_ENV, FEISHU_ENCRYPT_KEY_ENV,
    FEISHU_VERIFICATION_TOKEN_ENV, FeishuChannelServeMode, GOOGLE_CHAT_WEBHOOK_URL_ENV,
    IMESSAGE_BRIDGE_TOKEN_ENV, IMESSAGE_BRIDGE_URL_ENV, IRC_NICKNAME_ENV, IRC_SERVER_ENV,
    LINE_CHANNEL_ACCESS_TOKEN_ENV, LINE_CHANNEL_SECRET_ENV, LoongClawConfig,
    MATRIX_ACCESS_TOKEN_ENV, MATTERMOST_BOT_TOKEN_ENV, MATTERMOST_SERVER_URL_ENV,
    NEXTCLOUD_TALK_SERVER_URL_ENV, NEXTCLOUD_TALK_SHARED_SECRET_ENV, NOSTR_PRIVATE_KEY_ENV,
    NOSTR_RELAY_URLS_ENV, ResolvedDingtalkChannelConfig, ResolvedDiscordChannelConfig,
    ResolvedEmailChannelConfig, ResolvedFeishuChannelConfig, ResolvedGoogleChatChannelConfig,
    ResolvedImessageChannelConfig, ResolvedIrcChannelConfig, ResolvedLineChannelConfig,
    ResolvedMatrixChannelConfig, ResolvedMattermostChannelConfig,
    ResolvedNextcloudTalkChannelConfig, ResolvedNostrChannelConfig, ResolvedSignalChannelConfig,
    ResolvedSlackChannelConfig, ResolvedSynologyChatChannelConfig, ResolvedTeamsChannelConfig,
    ResolvedTelegramChannelConfig, ResolvedTlonChannelConfig, ResolvedTwitchChannelConfig,
    ResolvedWebhookChannelConfig, ResolvedWecomChannelConfig, ResolvedWhatsappChannelConfig,
    SIGNAL_ACCOUNT_ENV, SIGNAL_SERVICE_URL_ENV, SLACK_BOT_TOKEN_ENV,
    SYNOLOGY_CHAT_INCOMING_URL_ENV, SYNOLOGY_CHAT_TOKEN_ENV, TEAMS_APP_ID_ENV,
    TEAMS_APP_PASSWORD_ENV, TEAMS_TENANT_ID_ENV, TEAMS_WEBHOOK_URL_ENV, TELEGRAM_BOT_TOKEN_ENV,
    TWITCH_ACCESS_TOKEN_ENV, WEBHOOK_ENDPOINT_URL_ENV, WEBHOOK_SIGNING_SECRET_ENV,
    WECOM_BOT_ID_ENV, WECOM_SECRET_ENV, WHATSAPP_ACCESS_TOKEN_ENV, WHATSAPP_APP_SECRET_ENV,
    WHATSAPP_PHONE_NUMBER_ID_ENV, WHATSAPP_VERIFY_TOKEN_ENV, WebhookPayloadFormat,
    parse_email_smtp_endpoint, parse_irc_server_endpoint,
};

use self::descriptors::CHANNEL_REGISTRY;
pub use self::tlon::TLON_CATALOG_COMMAND_FAMILY_DESCRIPTOR;
pub use self::twitch::TWITCH_CATALOG_COMMAND_FAMILY_DESCRIPTOR;
use super::{
    ChannelCatalogTargetKind, ChannelOperationRuntime, ChannelPlatform,
    access_policy::{ChannelInboundAccessPolicy, ChannelInboundAccessPolicySummary},
    core::webhook_auth::build_webhook_auth_header_from_parts,
    runtime::state,
};

#[path = "registry_bridge.rs"]
mod bridge;
#[path = "registry_nostr_impl.rs"]
mod nostr_impl;

#[path = "registry_plugin_bridge.rs"]
mod plugin_bridge;

#[path = "registry_surface.rs"]
mod surface_support;

#[cfg(test)]
#[path = "registry_plugin_bridge_tests.rs"]
mod plugin_bridge_tests;

pub use super::catalog::{
    CHANNEL_OPERATION_SEND_ID, CHANNEL_OPERATION_SERVE_ID, ChannelCapability,
    ChannelCatalogCommandFamilyDescriptor, ChannelCatalogImplementationStatus,
    ChannelCatalogOperation, ChannelCatalogOperationAvailability,
    ChannelCatalogOperationRequirement, ChannelCommandFamilyDescriptor, ChannelDoctorCheckSpec,
    ChannelDoctorCheckTrigger, ChannelDoctorOperationSpec, ChannelOnboardingDescriptor,
    ChannelOnboardingStrategy, ChannelOperationDescriptor, ChannelRuntimeCommandDescriptor,
    FEISHU_RUNTIME_COMMAND_DESCRIPTOR, LINE_RUNTIME_COMMAND_DESCRIPTOR,
    MATRIX_RUNTIME_COMMAND_DESCRIPTOR, TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR,
    WEBHOOK_RUNTIME_COMMAND_DESCRIPTOR, WECOM_RUNTIME_COMMAND_DESCRIPTOR,
    WHATSAPP_RUNTIME_COMMAND_DESCRIPTOR, catalog_only_channel_entries, list_channel_catalog,
    normalize_channel_catalog_id, normalize_channel_platform,
    resolve_channel_catalog_command_family_descriptor, resolve_channel_catalog_entry,
    resolve_channel_catalog_operation, resolve_channel_command_family_descriptor,
    resolve_channel_doctor_operation_spec, resolve_channel_onboarding_descriptor,
    resolve_channel_operation_descriptor, resolve_channel_runtime_command_descriptor,
};
pub(crate) use super::catalog::{
    catalog_only_channel_entries_from, resolve_channel_selection_order,
};
pub use nostr_impl::NOSTR_CATALOG_COMMAND_FAMILY_DESCRIPTOR;
pub use plugin_bridge::validate_plugin_channel_bridge_manifest;
pub use plugin_bridge::{
    ChannelDiscoveredPluginBridge, ChannelDiscoveredPluginBridgeStatus,
    ChannelPluginBridgeContract, ChannelPluginBridgeDiscovery,
    ChannelPluginBridgeDiscoveryAmbiguityStatus, ChannelPluginBridgeDiscoveryStatus,
    ChannelPluginBridgeManifestStatus, ChannelPluginBridgeManifestValidation,
    ChannelPluginBridgeSelectionStatus, ChannelPluginBridgeStableTarget,
};
use plugin_bridge::{
    channel_surface_plugin_bridge_discovery_by_id, plugin_bridge_contract_from_descriptor,
};
use surface_support::build_channel_surfaces;

const DISCORD_APPLICATION_ID_ENV: &str = "DISCORD_APPLICATION_ID";
const SLACK_APP_TOKEN_ENV: &str = "SLACK_APP_TOKEN";
const SLACK_SIGNING_SECRET_ENV: &str = "SLACK_SIGNING_SECRET";
const EMAIL_SMTP_USERNAME_ENV: &str = "EMAIL_SMTP_USERNAME";
const EMAIL_SMTP_PASSWORD_ENV: &str = "EMAIL_SMTP_PASSWORD";
const EMAIL_IMAP_USERNAME_ENV: &str = "EMAIL_IMAP_USERNAME";
const EMAIL_IMAP_PASSWORD_ENV: &str = "EMAIL_IMAP_PASSWORD";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelCatalogEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub selection_order: u16,
    pub selection_label: &'static str,
    pub blurb: &'static str,
    pub implementation_status: ChannelCatalogImplementationStatus,
    pub capabilities: Vec<ChannelCapability>,
    pub aliases: Vec<&'static str>,
    pub transport: &'static str,
    pub onboarding: ChannelOnboardingDescriptor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_bridge_contract: Option<ChannelPluginBridgeContract>,
    pub supported_target_kinds: Vec<ChannelCatalogTargetKind>,
    pub operations: Vec<ChannelCatalogOperation>,
}

impl ChannelCatalogEntry {
    pub fn operation(&self, id: &str) -> Option<&ChannelCatalogOperation> {
        self.operations.iter().find(|operation| operation.id == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelOperationHealth {
    Ready,
    Disabled,
    Unsupported,
    Misconfigured,
}

impl ChannelOperationHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Disabled => "disabled",
            Self::Unsupported => "unsupported",
            Self::Misconfigured => "misconfigured",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelOperationStatus {
    pub id: &'static str,
    pub label: &'static str,
    pub command: &'static str,
    pub health: ChannelOperationHealth,
    pub detail: String,
    pub issues: Vec<String>,
    pub runtime: Option<ChannelOperationRuntime>,
}

impl ChannelOperationStatus {
    pub fn is_ready(&self) -> bool {
        self.health == ChannelOperationHealth::Ready
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelStatusSnapshot {
    pub id: &'static str,
    pub configured_account_id: String,
    pub configured_account_label: String,
    pub is_default_account: bool,
    pub default_account_source: ChannelDefaultAccountSelectionSource,
    pub label: &'static str,
    pub aliases: Vec<&'static str>,
    pub transport: &'static str,
    pub compiled: bool,
    pub enabled: bool,
    pub api_base_url: Option<String>,
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reserved_runtime_fields: Vec<String>,
    pub operations: Vec<ChannelOperationStatus>,
}

impl ChannelStatusSnapshot {
    pub fn operation(&self, id: &str) -> Option<&ChannelOperationStatus> {
        self.operations.iter().find(|operation| operation.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelInventory {
    pub channels: Vec<ChannelStatusSnapshot>,
    pub catalog_only_channels: Vec<ChannelCatalogEntry>,
    pub channel_catalog: Vec<ChannelCatalogEntry>,
    pub channel_surfaces: Vec<ChannelSurface>,
    pub channel_access_policies: Vec<ChannelConfiguredAccountAccessPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelConfiguredAccountAccessPolicy {
    pub channel_id: &'static str,
    pub configured_account_id: String,
    pub conversation_config_key: &'static str,
    pub sender_config_key: &'static str,
    #[serde(flatten)]
    pub summary: ChannelInboundAccessPolicySummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelSurface {
    pub catalog: ChannelCatalogEntry,
    pub configured_accounts: Vec<ChannelStatusSnapshot>,
    pub default_configured_account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_bridge_discovery: Option<ChannelPluginBridgeDiscovery>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChannelRuntimeDescriptor {
    pub(super) family: ChannelCommandFamilyDescriptor,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChannelRegistryOperationDescriptor {
    pub(super) operation: ChannelCatalogOperation,
    pub(super) doctor_checks: &'static [ChannelDoctorCheckSpec],
}

pub(crate) type ChannelSnapshotBuilder =
    fn(&ChannelRegistryDescriptor, &LoongClawConfig, &Path, u64) -> Vec<ChannelStatusSnapshot>;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChannelRegistryDescriptor {
    pub(super) id: &'static str,
    pub(super) runtime: Option<ChannelRuntimeDescriptor>,
    snapshot_builder: Option<ChannelSnapshotBuilder>,
    pub(super) selection_order: u16,
    selection_label: &'static str,
    blurb: &'static str,
    implementation_status: ChannelCatalogImplementationStatus,
    capabilities: &'static [ChannelCapability],
    label: &'static str,
    aliases: &'static [&'static str],
    transport: &'static str,
    pub(super) onboarding: ChannelOnboardingDescriptor,
    pub(super) operations: &'static [ChannelRegistryOperationDescriptor],
}

const TELEGRAM_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "direct send",
    command: "telegram-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: TELEGRAM_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const TELEGRAM_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "reply loop",
    command: "telegram-serve",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: true,
    requirements: TELEGRAM_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

pub const TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "telegram",
        default_send_target_kind: ChannelCatalogTargetKind::Conversation,
        send: TELEGRAM_SEND_OPERATION,
        serve: TELEGRAM_SERVE_OPERATION,
    };

pub const TELEGRAM_COMMAND_FAMILY_DESCRIPTOR: ChannelCommandFamilyDescriptor =
    ChannelCommandFamilyDescriptor {
        runtime: TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR,
        catalog: TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    };

const TELEGRAM_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["telegram.enabled", "telegram.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const TELEGRAM_BOT_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "bot_token",
        label: "bot token",
        config_paths: &[
            "telegram.bot_token",
            "telegram.accounts.<account>.bot_token",
        ],
        env_pointer_paths: &[
            "telegram.bot_token_env",
            "telegram.accounts.<account>.bot_token_env",
        ],
        default_env_var: Some(TELEGRAM_BOT_TOKEN_ENV),
    };
const TELEGRAM_ALLOWED_CHAT_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_chat_ids",
        label: "allowed chat ids",
        config_paths: &[
            "telegram.allowed_chat_ids",
            "telegram.accounts.<account>.allowed_chat_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const TELEGRAM_ALLOWED_SENDER_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_sender_ids",
        label: "allowed sender ids",
        config_paths: &[
            "telegram.allowed_sender_ids",
            "telegram.accounts.<account>.allowed_sender_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const TELEGRAM_REQUIRE_MENTION_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "require_mention",
        label: "require explicit bot mention outside private chats",
        config_paths: &[
            "telegram.require_mention",
            "telegram.accounts.<account>.require_mention",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const TELEGRAM_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] =
    &[TELEGRAM_ENABLED_REQUIREMENT, TELEGRAM_BOT_TOKEN_REQUIREMENT];
const TELEGRAM_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    TELEGRAM_ENABLED_REQUIREMENT,
    TELEGRAM_BOT_TOKEN_REQUIREMENT,
    TELEGRAM_ALLOWED_CHAT_IDS_REQUIREMENT,
    TELEGRAM_ALLOWED_SENDER_IDS_REQUIREMENT,
    TELEGRAM_REQUIRE_MENTION_REQUIREMENT,
];

const TELEGRAM_SERVE_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[
    ChannelDoctorCheckSpec {
        name: "telegram channel",
        trigger: ChannelDoctorCheckTrigger::OperationHealth,
    },
    ChannelDoctorCheckSpec {
        name: "telegram channel runtime",
        trigger: ChannelDoctorCheckTrigger::ReadyRuntime,
    },
];
const TELEGRAM_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: TELEGRAM_SERVE_DOCTOR_CHECKS,
    },
];
const TELEGRAM_CAPABILITIES: &[ChannelCapability] = &[
    ChannelCapability::RuntimeBacked,
    ChannelCapability::MultiAccount,
    ChannelCapability::Send,
    ChannelCapability::Serve,
    ChannelCapability::RuntimeTracking,
];
const TELEGRAM_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure telegram bot credentials, allowed chat ids, and optional mention gating in loongclaw.toml under telegram or telegram.accounts.<account>",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};

const FEISHU_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "direct send",
    command: "feishu-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: FEISHU_SEND_REQUIREMENTS,
    default_target_kind: Some(ChannelCatalogTargetKind::ReceiveId),
    supported_target_kinds: &[
        ChannelCatalogTargetKind::ReceiveId,
        ChannelCatalogTargetKind::MessageReply,
    ],
};

const FEISHU_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "inbound reply service",
    command: "feishu-serve",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: true,
    requirements: FEISHU_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::MessageReply],
};

pub const FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "feishu",
        default_send_target_kind: ChannelCatalogTargetKind::ReceiveId,
        send: FEISHU_SEND_OPERATION,
        serve: FEISHU_SERVE_OPERATION,
    };

pub const FEISHU_COMMAND_FAMILY_DESCRIPTOR: ChannelCommandFamilyDescriptor =
    ChannelCommandFamilyDescriptor {
        runtime: FEISHU_RUNTIME_COMMAND_DESCRIPTOR,
        catalog: FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    };

const FEISHU_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["feishu.enabled", "feishu.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const FEISHU_APP_ID_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "app_id",
        label: "app id",
        config_paths: &["feishu.app_id", "feishu.accounts.<account>.app_id"],
        env_pointer_paths: &["feishu.app_id_env", "feishu.accounts.<account>.app_id_env"],
        default_env_var: Some(FEISHU_APP_ID_ENV),
    };
const FEISHU_APP_SECRET_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "app_secret",
        label: "app secret",
        config_paths: &["feishu.app_secret", "feishu.accounts.<account>.app_secret"],
        env_pointer_paths: &[
            "feishu.app_secret_env",
            "feishu.accounts.<account>.app_secret_env",
        ],
        default_env_var: Some(FEISHU_APP_SECRET_ENV),
    };
const FEISHU_ALLOWED_CHAT_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_chat_ids",
        label: "allowed chat ids",
        config_paths: &[
            "feishu.allowed_chat_ids",
            "feishu.accounts.<account>.allowed_chat_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const FEISHU_ALLOWED_SENDER_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_sender_ids",
        label: "allowed sender ids",
        config_paths: &[
            "feishu.allowed_sender_ids",
            "feishu.accounts.<account>.allowed_sender_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const FEISHU_MODE_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "mode",
        label: "serve mode",
        config_paths: &["feishu.mode", "feishu.accounts.<account>.mode"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const FEISHU_VERIFICATION_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "verification_token",
        label: "verification token (webhook mode only)",
        config_paths: &[
            "feishu.verification_token",
            "feishu.accounts.<account>.verification_token",
        ],
        env_pointer_paths: &[
            "feishu.verification_token_env",
            "feishu.accounts.<account>.verification_token_env",
        ],
        default_env_var: Some(FEISHU_VERIFICATION_TOKEN_ENV),
    };
const FEISHU_ENCRYPT_KEY_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "encrypt_key",
        label: "encrypt key (webhook mode only)",
        config_paths: &[
            "feishu.encrypt_key",
            "feishu.accounts.<account>.encrypt_key",
        ],
        env_pointer_paths: &[
            "feishu.encrypt_key_env",
            "feishu.accounts.<account>.encrypt_key_env",
        ],
        default_env_var: Some(FEISHU_ENCRYPT_KEY_ENV),
    };
const FEISHU_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    FEISHU_ENABLED_REQUIREMENT,
    FEISHU_APP_ID_REQUIREMENT,
    FEISHU_APP_SECRET_REQUIREMENT,
];
const FEISHU_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    FEISHU_ENABLED_REQUIREMENT,
    FEISHU_APP_ID_REQUIREMENT,
    FEISHU_APP_SECRET_REQUIREMENT,
    FEISHU_MODE_REQUIREMENT,
    FEISHU_ALLOWED_CHAT_IDS_REQUIREMENT,
    FEISHU_ALLOWED_SENDER_IDS_REQUIREMENT,
    FEISHU_VERIFICATION_TOKEN_REQUIREMENT,
    FEISHU_ENCRYPT_KEY_REQUIREMENT,
];

const FEISHU_SEND_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[ChannelDoctorCheckSpec {
    name: "feishu channel",
    trigger: ChannelDoctorCheckTrigger::OperationHealth,
}];
const FEISHU_SERVE_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[
    ChannelDoctorCheckSpec {
        name: "feishu inbound transport",
        trigger: ChannelDoctorCheckTrigger::OperationHealth,
    },
    ChannelDoctorCheckSpec {
        name: "feishu serve runtime",
        trigger: ChannelDoctorCheckTrigger::ReadyRuntime,
    },
];
const FEISHU_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: FEISHU_SEND_DOCTOR_CHECKS,
    },
    ChannelRegistryOperationDescriptor {
        operation: FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: FEISHU_SERVE_DOCTOR_CHECKS,
    },
];
const FEISHU_CAPABILITIES: &[ChannelCapability] = &[
    ChannelCapability::RuntimeBacked,
    ChannelCapability::MultiAccount,
    ChannelCapability::Send,
    ChannelCapability::Serve,
    ChannelCapability::RuntimeTracking,
];
const FEISHU_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure feishu or lark app credentials, allowed chat ids, and either webhook secrets or mode = \"websocket\" in loongclaw.toml under feishu or feishu.accounts.<account>",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};

const MATRIX_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "direct send",
    command: "matrix-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: MATRIX_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const MATRIX_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "sync reply loop",
    command: "matrix-serve",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: true,
    requirements: MATRIX_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

pub const MATRIX_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "matrix",
        default_send_target_kind: ChannelCatalogTargetKind::Conversation,
        send: MATRIX_SEND_OPERATION,
        serve: MATRIX_SERVE_OPERATION,
    };

pub const MATRIX_COMMAND_FAMILY_DESCRIPTOR: ChannelCommandFamilyDescriptor =
    ChannelCommandFamilyDescriptor {
        runtime: MATRIX_RUNTIME_COMMAND_DESCRIPTOR,
        catalog: MATRIX_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    };

const WECOM_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "proactive send",
    command: "wecom-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: WECOM_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const WECOM_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "long connection reply loop",
    command: "wecom-serve",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: true,
    requirements: WECOM_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

pub const WECOM_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "wecom",
        default_send_target_kind: ChannelCatalogTargetKind::Conversation,
        send: WECOM_SEND_OPERATION,
        serve: WECOM_SERVE_OPERATION,
    };

pub const WECOM_COMMAND_FAMILY_DESCRIPTOR: ChannelCommandFamilyDescriptor =
    ChannelCommandFamilyDescriptor {
        runtime: WECOM_RUNTIME_COMMAND_DESCRIPTOR,
        catalog: WECOM_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    };

const MATRIX_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["matrix.enabled", "matrix.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const MATRIX_ACCESS_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "access_token",
        label: "access token",
        config_paths: &[
            "matrix.access_token",
            "matrix.accounts.<account>.access_token",
        ],
        env_pointer_paths: &[
            "matrix.access_token_env",
            "matrix.accounts.<account>.access_token_env",
        ],
        default_env_var: Some(MATRIX_ACCESS_TOKEN_ENV),
    };
const MATRIX_BASE_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "base_url",
        label: "homeserver base url",
        config_paths: &["matrix.base_url", "matrix.accounts.<account>.base_url"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const MATRIX_ALLOWED_ROOM_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_room_ids",
        label: "allowed room ids",
        config_paths: &[
            "matrix.allowed_room_ids",
            "matrix.accounts.<account>.allowed_room_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const MATRIX_ALLOWED_SENDER_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_sender_ids",
        label: "allowed sender ids",
        config_paths: &[
            "matrix.allowed_sender_ids",
            "matrix.accounts.<account>.allowed_sender_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const MATRIX_REQUIRE_MENTION_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "require_mention",
        label: "require explicit mention in synced rooms",
        config_paths: &[
            "matrix.require_mention",
            "matrix.accounts.<account>.require_mention",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const MATRIX_USER_ID_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "user_id",
        label: "user id when ignore_self_messages is enabled",
        config_paths: &["matrix.user_id", "matrix.accounts.<account>.user_id"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const MATRIX_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    MATRIX_ENABLED_REQUIREMENT,
    MATRIX_ACCESS_TOKEN_REQUIREMENT,
    MATRIX_BASE_URL_REQUIREMENT,
];
const MATRIX_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    MATRIX_ENABLED_REQUIREMENT,
    MATRIX_ACCESS_TOKEN_REQUIREMENT,
    MATRIX_BASE_URL_REQUIREMENT,
    MATRIX_ALLOWED_ROOM_IDS_REQUIREMENT,
    MATRIX_ALLOWED_SENDER_IDS_REQUIREMENT,
    MATRIX_REQUIRE_MENTION_REQUIREMENT,
    MATRIX_USER_ID_REQUIREMENT,
];

const MATRIX_SEND_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[ChannelDoctorCheckSpec {
    name: "matrix channel",
    trigger: ChannelDoctorCheckTrigger::OperationHealth,
}];
const MATRIX_SERVE_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[
    ChannelDoctorCheckSpec {
        name: "matrix room sync",
        trigger: ChannelDoctorCheckTrigger::OperationHealth,
    },
    ChannelDoctorCheckSpec {
        name: "matrix channel runtime",
        trigger: ChannelDoctorCheckTrigger::ReadyRuntime,
    },
];
const MATRIX_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: MATRIX_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: MATRIX_SEND_DOCTOR_CHECKS,
    },
    ChannelRegistryOperationDescriptor {
        operation: MATRIX_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: MATRIX_SERVE_DOCTOR_CHECKS,
    },
];
const MATRIX_CAPABILITIES: &[ChannelCapability] = &[
    ChannelCapability::RuntimeBacked,
    ChannelCapability::MultiAccount,
    ChannelCapability::Send,
    ChannelCapability::Serve,
    ChannelCapability::RuntimeTracking,
];
const MATRIX_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure matrix access tokens, homeserver base url, allowed room ids, and optional mention gating in loongclaw.toml under matrix or matrix.accounts.<account>",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};

const PLUGIN_BACKED_CHANNEL_CAPABILITIES: &[ChannelCapability] = &[
    ChannelCapability::PluginBacked,
    ChannelCapability::MultiAccount,
    ChannelCapability::Send,
    ChannelCapability::Serve,
    ChannelCapability::RuntimeTracking,
];

const PLUGIN_BRIDGE_REQUIRED_SETUP_SURFACE: &str = "channel";
const PLUGIN_BRIDGE_RUNTIME_OWNER: &str = "external_plugin";
const PLUGIN_BRIDGE_RECOMMENDED_METADATA_KEYS: &[&str] = &[
    "bridge_kind",
    "adapter_family",
    "entrypoint",
    "transport_family",
    "target_contract",
    "account_scope",
];

const CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES: &[ChannelCapability] =
    &[ChannelCapability::MultiAccount, ChannelCapability::Send];

const DISCORD_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["discord.enabled", "discord.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const DISCORD_BOT_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "bot_token",
        label: "bot token",
        config_paths: &["discord.bot_token", "discord.accounts.<account>.bot_token"],
        env_pointer_paths: &[
            "discord.bot_token_env",
            "discord.accounts.<account>.bot_token_env",
        ],
        default_env_var: Some(DISCORD_BOT_TOKEN_ENV),
    };
const DISCORD_APPLICATION_ID_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "application_id",
        label: "application id",
        config_paths: &[
            "discord.application_id",
            "discord.accounts.<account>.application_id",
        ],
        env_pointer_paths: &[
            "discord.application_id_env",
            "discord.accounts.<account>.application_id_env",
        ],
        default_env_var: Some(DISCORD_APPLICATION_ID_ENV),
    };
const DISCORD_ALLOWED_GUILD_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_guild_ids",
        label: "allowed guild ids",
        config_paths: &[
            "discord.allowed_guild_ids",
            "discord.accounts.<account>.allowed_guild_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const DISCORD_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] =
    &[DISCORD_ENABLED_REQUIREMENT, DISCORD_BOT_TOKEN_REQUIREMENT];
const DISCORD_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    DISCORD_ENABLED_REQUIREMENT,
    DISCORD_BOT_TOKEN_REQUIREMENT,
    DISCORD_APPLICATION_ID_REQUIREMENT,
    DISCORD_ALLOWED_GUILD_IDS_REQUIREMENT,
];
const DISCORD_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "direct send",
    command: "discord-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: DISCORD_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const DISCORD_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "gateway reply loop",
    command: "discord-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: DISCORD_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

pub const DISCORD_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "discord",
        default_send_target_kind: ChannelCatalogTargetKind::Conversation,
        send: DISCORD_SEND_OPERATION,
        serve: DISCORD_SERVE_OPERATION,
    };

const DISCORD_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: DISCORD_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: DISCORD_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[],
    },
];
const DISCORD_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure discord bot credentials in loongclaw.toml under discord or discord.accounts.<account>; outbound direct send is shipped, while gateway-based serve support remains planned",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};

const SLACK_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["slack.enabled", "slack.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const SLACK_BOT_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "bot_token",
        label: "bot token",
        config_paths: &["slack.bot_token", "slack.accounts.<account>.bot_token"],
        env_pointer_paths: &[
            "slack.bot_token_env",
            "slack.accounts.<account>.bot_token_env",
        ],
        default_env_var: Some(SLACK_BOT_TOKEN_ENV),
    };
const SLACK_APP_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "app_token",
        label: "socket mode app token",
        config_paths: &["slack.app_token", "slack.accounts.<account>.app_token"],
        env_pointer_paths: &[
            "slack.app_token_env",
            "slack.accounts.<account>.app_token_env",
        ],
        default_env_var: Some(SLACK_APP_TOKEN_ENV),
    };
const SLACK_SIGNING_SECRET_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "signing_secret",
        label: "signing secret",
        config_paths: &[
            "slack.signing_secret",
            "slack.accounts.<account>.signing_secret",
        ],
        env_pointer_paths: &[
            "slack.signing_secret_env",
            "slack.accounts.<account>.signing_secret_env",
        ],
        default_env_var: Some(SLACK_SIGNING_SECRET_ENV),
    };
const SLACK_ALLOWED_CHANNEL_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_channel_ids",
        label: "allowed channel ids",
        config_paths: &[
            "slack.allowed_channel_ids",
            "slack.accounts.<account>.allowed_channel_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const SLACK_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] =
    &[SLACK_ENABLED_REQUIREMENT, SLACK_BOT_TOKEN_REQUIREMENT];
const SLACK_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    SLACK_ENABLED_REQUIREMENT,
    SLACK_BOT_TOKEN_REQUIREMENT,
    SLACK_APP_TOKEN_REQUIREMENT,
    SLACK_SIGNING_SECRET_REQUIREMENT,
    SLACK_ALLOWED_CHANNEL_IDS_REQUIREMENT,
];
const SLACK_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "direct send",
    command: "slack-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: SLACK_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const SLACK_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "events reply loop",
    command: "slack-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: SLACK_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

pub const SLACK_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "slack",
        default_send_target_kind: ChannelCatalogTargetKind::Conversation,
        send: SLACK_SEND_OPERATION,
        serve: SLACK_SERVE_OPERATION,
    };

const SLACK_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: SLACK_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: SLACK_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[],
    },
];
const SLACK_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure slack bot credentials in loongclaw.toml under slack or slack.accounts.<account>; outbound direct send is shipped, while Events API or Socket Mode serve support remains planned",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};

const LINE_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["line.enabled", "line.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const LINE_CHANNEL_ACCESS_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "channel_access_token",
        label: "channel access token",
        config_paths: &[
            "line.channel_access_token",
            "line.accounts.<account>.channel_access_token",
        ],
        env_pointer_paths: &[
            "line.channel_access_token_env",
            "line.accounts.<account>.channel_access_token_env",
        ],
        default_env_var: Some(LINE_CHANNEL_ACCESS_TOKEN_ENV),
    };
const LINE_CHANNEL_SECRET_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "channel_secret",
        label: "channel secret",
        config_paths: &[
            "line.channel_secret",
            "line.accounts.<account>.channel_secret",
        ],
        env_pointer_paths: &[
            "line.channel_secret_env",
            "line.accounts.<account>.channel_secret_env",
        ],
        default_env_var: Some(LINE_CHANNEL_SECRET_ENV),
    };
const LINE_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    LINE_ENABLED_REQUIREMENT,
    LINE_CHANNEL_ACCESS_TOKEN_REQUIREMENT,
];
const LINE_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    LINE_ENABLED_REQUIREMENT,
    LINE_CHANNEL_ACCESS_TOKEN_REQUIREMENT,
    LINE_CHANNEL_SECRET_REQUIREMENT,
];
const LINE_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "push send",
    command: "line-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: LINE_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const LINE_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "webhook reply loop",
    command: "line-serve",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: true,
    requirements: LINE_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
pub const LINE_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "line",
        default_send_target_kind: ChannelCatalogTargetKind::Address,
        send: LINE_SEND_OPERATION,
        serve: LINE_SERVE_OPERATION,
    };

pub const LINE_COMMAND_FAMILY_DESCRIPTOR: ChannelCommandFamilyDescriptor =
    ChannelCommandFamilyDescriptor {
        runtime: LINE_RUNTIME_COMMAND_DESCRIPTOR,
        catalog: LINE_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    };

const LINE_SERVE_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[
    ChannelDoctorCheckSpec {
        name: "line serve health",
        trigger: ChannelDoctorCheckTrigger::OperationHealth,
    },
    ChannelDoctorCheckSpec {
        name: "line serve runtime",
        trigger: ChannelDoctorCheckTrigger::ReadyRuntime,
    },
];

const LINE_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: LINE_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: LINE_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: LINE_SERVE_DOCTOR_CHECKS,
    },
];
const LINE_CAPABILITIES: &[ChannelCapability] = &[
    ChannelCapability::RuntimeBacked,
    ChannelCapability::MultiAccount,
    ChannelCapability::Send,
    ChannelCapability::Serve,
    ChannelCapability::RuntimeTracking,
];
const LINE_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure LINE Messaging API credentials in loongclaw.toml under line or line.accounts.<account>; outbound push send and inbound webhook serve are shipped, and line-serve requires --bind plus an optional --path override at runtime",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};

const WECOM_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["wecom.enabled", "wecom.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const WECOM_BOT_ID_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "bot_id",
        label: "aibot bot id",
        config_paths: &["wecom.bot_id", "wecom.accounts.<account>.bot_id"],
        env_pointer_paths: &["wecom.bot_id_env", "wecom.accounts.<account>.bot_id_env"],
        default_env_var: Some(WECOM_BOT_ID_ENV),
    };
const WECOM_SECRET_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "secret",
        label: "aibot secret",
        config_paths: &["wecom.secret", "wecom.accounts.<account>.secret"],
        env_pointer_paths: &["wecom.secret_env", "wecom.accounts.<account>.secret_env"],
        default_env_var: Some(WECOM_SECRET_ENV),
    };
const WECOM_ALLOWED_CONVERSATION_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_conversation_ids",
        label: "allowed conversation ids",
        config_paths: &[
            "wecom.allowed_conversation_ids",
            "wecom.accounts.<account>.allowed_conversation_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const WECOM_ALLOWED_SENDER_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_sender_ids",
        label: "allowed sender ids",
        config_paths: &[
            "wecom.allowed_sender_ids",
            "wecom.accounts.<account>.allowed_sender_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const WECOM_WEBSOCKET_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "websocket_url",
        label: "websocket url override",
        config_paths: &[
            "wecom.websocket_url",
            "wecom.accounts.<account>.websocket_url",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const WECOM_PING_INTERVAL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "ping_interval_s",
        label: "ping interval seconds",
        config_paths: &[
            "wecom.ping_interval_s",
            "wecom.accounts.<account>.ping_interval_s",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const WECOM_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    WECOM_ENABLED_REQUIREMENT,
    WECOM_BOT_ID_REQUIREMENT,
    WECOM_SECRET_REQUIREMENT,
    WECOM_WEBSOCKET_URL_REQUIREMENT,
];
const WECOM_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    WECOM_ENABLED_REQUIREMENT,
    WECOM_BOT_ID_REQUIREMENT,
    WECOM_SECRET_REQUIREMENT,
    WECOM_ALLOWED_CONVERSATION_IDS_REQUIREMENT,
    WECOM_ALLOWED_SENDER_IDS_REQUIREMENT,
    WECOM_WEBSOCKET_URL_REQUIREMENT,
    WECOM_PING_INTERVAL_REQUIREMENT,
];
const WECOM_SEND_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[ChannelDoctorCheckSpec {
    name: "wecom channel",
    trigger: ChannelDoctorCheckTrigger::OperationHealth,
}];
const WECOM_SERVE_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[
    ChannelDoctorCheckSpec {
        name: "wecom aibot long connection",
        trigger: ChannelDoctorCheckTrigger::OperationHealth,
    },
    ChannelDoctorCheckSpec {
        name: "wecom serve runtime",
        trigger: ChannelDoctorCheckTrigger::ReadyRuntime,
    },
];
const WECOM_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: WECOM_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: WECOM_SEND_DOCTOR_CHECKS,
    },
    ChannelRegistryOperationDescriptor {
        operation: WECOM_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: WECOM_SERVE_DOCTOR_CHECKS,
    },
];
const WECOM_CAPABILITIES: &[ChannelCapability] = &[
    ChannelCapability::RuntimeBacked,
    ChannelCapability::MultiAccount,
    ChannelCapability::Send,
    ChannelCapability::Serve,
    ChannelCapability::RuntimeTracking,
];
const WECOM_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure wecom aibot long connection credentials, allowed conversation ids, and optional websocket overrides in loongclaw.toml under wecom or wecom.accounts.<account>; do not configure webhook callback mode for this surface",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};

const DINGTALK_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["dingtalk.enabled", "dingtalk.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const DINGTALK_WEBHOOK_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "webhook_url",
        label: "custom robot webhook url",
        config_paths: &[
            "dingtalk.webhook_url",
            "dingtalk.accounts.<account>.webhook_url",
        ],
        env_pointer_paths: &[
            "dingtalk.webhook_url_env",
            "dingtalk.accounts.<account>.webhook_url_env",
        ],
        default_env_var: Some(DINGTALK_WEBHOOK_URL_ENV),
    };
const DINGTALK_SECRET_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "secret",
        label: "custom robot sign secret",
        config_paths: &["dingtalk.secret", "dingtalk.accounts.<account>.secret"],
        env_pointer_paths: &[
            "dingtalk.secret_env",
            "dingtalk.accounts.<account>.secret_env",
        ],
        default_env_var: Some(DINGTALK_SECRET_ENV),
    };
const DINGTALK_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    DINGTALK_ENABLED_REQUIREMENT,
    DINGTALK_WEBHOOK_URL_REQUIREMENT,
];
const DINGTALK_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    DINGTALK_ENABLED_REQUIREMENT,
    DINGTALK_WEBHOOK_URL_REQUIREMENT,
    DINGTALK_SECRET_REQUIREMENT,
];
const DINGTALK_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "custom robot send",
    command: "dingtalk-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: DINGTALK_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Endpoint],
};
const DINGTALK_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "outgoing callback service",
    command: "dingtalk-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: DINGTALK_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Endpoint],
};
pub const DINGTALK_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "dingtalk",
        default_send_target_kind: ChannelCatalogTargetKind::Endpoint,
        send: DINGTALK_SEND_OPERATION,
        serve: DINGTALK_SERVE_OPERATION,
    };
const DINGTALK_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: DINGTALK_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: DINGTALK_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[],
    },
];
const DINGTALK_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure DingTalk custom robot webhook credentials in loongclaw.toml under dingtalk or dingtalk.accounts.<account>; outbound webhook send is shipped, while inbound outgoing-callback serve support remains planned",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};

const WHATSAPP_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["whatsapp.enabled", "whatsapp.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const WHATSAPP_ACCESS_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "access_token",
        label: "cloud api access token",
        config_paths: &[
            "whatsapp.access_token",
            "whatsapp.accounts.<account>.access_token",
        ],
        env_pointer_paths: &[
            "whatsapp.access_token_env",
            "whatsapp.accounts.<account>.access_token_env",
        ],
        default_env_var: Some(WHATSAPP_ACCESS_TOKEN_ENV),
    };
const WHATSAPP_PHONE_NUMBER_ID_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "phone_number_id",
        label: "phone number id",
        config_paths: &[
            "whatsapp.phone_number_id",
            "whatsapp.accounts.<account>.phone_number_id",
        ],
        env_pointer_paths: &[
            "whatsapp.phone_number_id_env",
            "whatsapp.accounts.<account>.phone_number_id_env",
        ],
        default_env_var: Some(WHATSAPP_PHONE_NUMBER_ID_ENV),
    };
const WHATSAPP_VERIFY_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "verify_token",
        label: "webhook verify token",
        config_paths: &[
            "whatsapp.verify_token",
            "whatsapp.accounts.<account>.verify_token",
        ],
        env_pointer_paths: &[
            "whatsapp.verify_token_env",
            "whatsapp.accounts.<account>.verify_token_env",
        ],
        default_env_var: Some(WHATSAPP_VERIFY_TOKEN_ENV),
    };
const WHATSAPP_APP_SECRET_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "app_secret",
        label: "meta app secret",
        config_paths: &[
            "whatsapp.app_secret",
            "whatsapp.accounts.<account>.app_secret",
        ],
        env_pointer_paths: &[
            "whatsapp.app_secret_env",
            "whatsapp.accounts.<account>.app_secret_env",
        ],
        default_env_var: Some(WHATSAPP_APP_SECRET_ENV),
    };
const WHATSAPP_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    WHATSAPP_ENABLED_REQUIREMENT,
    WHATSAPP_ACCESS_TOKEN_REQUIREMENT,
    WHATSAPP_PHONE_NUMBER_ID_REQUIREMENT,
];
const WHATSAPP_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    WHATSAPP_ENABLED_REQUIREMENT,
    WHATSAPP_ACCESS_TOKEN_REQUIREMENT,
    WHATSAPP_PHONE_NUMBER_ID_REQUIREMENT,
    WHATSAPP_VERIFY_TOKEN_REQUIREMENT,
    WHATSAPP_APP_SECRET_REQUIREMENT,
];
const WHATSAPP_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "business send",
    command: "whatsapp-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: WHATSAPP_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const WHATSAPP_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "cloud webhook service",
    command: "whatsapp-serve",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: true,
    requirements: WHATSAPP_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
pub const WHATSAPP_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "whatsapp",
        default_send_target_kind: ChannelCatalogTargetKind::Address,
        send: WHATSAPP_SEND_OPERATION,
        serve: WHATSAPP_SERVE_OPERATION,
    };

pub const WHATSAPP_COMMAND_FAMILY_DESCRIPTOR: ChannelCommandFamilyDescriptor =
    ChannelCommandFamilyDescriptor {
        runtime: WHATSAPP_RUNTIME_COMMAND_DESCRIPTOR,
        catalog: WHATSAPP_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    };

const WHATSAPP_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: WHATSAPP_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: WHATSAPP_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[
            ChannelDoctorCheckSpec {
                name: "whatsapp serve health",
                trigger: ChannelDoctorCheckTrigger::OperationHealth,
            },
            ChannelDoctorCheckSpec {
                name: "whatsapp serve runtime",
                trigger: ChannelDoctorCheckTrigger::ReadyRuntime,
            },
        ],
    },
];
const WHATSAPP_CAPABILITIES: &[ChannelCapability] = &[
    ChannelCapability::RuntimeBacked,
    ChannelCapability::MultiAccount,
    ChannelCapability::Send,
    ChannelCapability::Serve,
    ChannelCapability::RuntimeTracking,
];
const WHATSAPP_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure whatsapp cloud api credentials (access_token, phone_number_id, verify_token, app_secret) in loongclaw.toml under whatsapp or whatsapp.accounts.<account>; both outbound business send and inbound webhook serve are shipped",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};

const EMAIL_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["email.enabled", "email.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const EMAIL_SMTP_HOST_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "smtp_host",
        label: "smtp host",
        config_paths: &["email.smtp_host", "email.accounts.<account>.smtp_host"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const EMAIL_SMTP_USERNAME_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "smtp_username",
        label: "smtp username",
        config_paths: &[
            "email.smtp_username",
            "email.accounts.<account>.smtp_username",
        ],
        env_pointer_paths: &[
            "email.smtp_username_env",
            "email.accounts.<account>.smtp_username_env",
        ],
        default_env_var: Some(EMAIL_SMTP_USERNAME_ENV),
    };
const EMAIL_SMTP_PASSWORD_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "smtp_password",
        label: "smtp password",
        config_paths: &[
            "email.smtp_password",
            "email.accounts.<account>.smtp_password",
        ],
        env_pointer_paths: &[
            "email.smtp_password_env",
            "email.accounts.<account>.smtp_password_env",
        ],
        default_env_var: Some(EMAIL_SMTP_PASSWORD_ENV),
    };
const EMAIL_FROM_ADDRESS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "from_address",
        label: "from address",
        config_paths: &[
            "email.from_address",
            "email.accounts.<account>.from_address",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const EMAIL_IMAP_HOST_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "imap_host",
        label: "imap host",
        config_paths: &["email.imap_host", "email.accounts.<account>.imap_host"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const EMAIL_IMAP_USERNAME_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "imap_username",
        label: "imap username",
        config_paths: &[
            "email.imap_username",
            "email.accounts.<account>.imap_username",
        ],
        env_pointer_paths: &[
            "email.imap_username_env",
            "email.accounts.<account>.imap_username_env",
        ],
        default_env_var: Some(EMAIL_IMAP_USERNAME_ENV),
    };
const EMAIL_IMAP_PASSWORD_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "imap_password",
        label: "imap password",
        config_paths: &[
            "email.imap_password",
            "email.accounts.<account>.imap_password",
        ],
        env_pointer_paths: &[
            "email.imap_password_env",
            "email.accounts.<account>.imap_password_env",
        ],
        default_env_var: Some(EMAIL_IMAP_PASSWORD_ENV),
    };
const EMAIL_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    EMAIL_ENABLED_REQUIREMENT,
    EMAIL_SMTP_HOST_REQUIREMENT,
    EMAIL_SMTP_USERNAME_REQUIREMENT,
    EMAIL_SMTP_PASSWORD_REQUIREMENT,
    EMAIL_FROM_ADDRESS_REQUIREMENT,
];
const EMAIL_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    EMAIL_ENABLED_REQUIREMENT,
    EMAIL_IMAP_HOST_REQUIREMENT,
    EMAIL_IMAP_USERNAME_REQUIREMENT,
    EMAIL_IMAP_PASSWORD_REQUIREMENT,
];
const EMAIL_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "smtp send",
    command: "email-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: EMAIL_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const EMAIL_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "imap reply loop",
    command: "email-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: EMAIL_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const EMAIL_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: EMAIL_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: EMAIL_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[],
    },
];
pub const EMAIL_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "email",
        default_send_target_kind: ChannelCatalogTargetKind::Address,
        send: EMAIL_SEND_OPERATION,
        serve: EMAIL_SERVE_OPERATION,
    };
const EMAIL_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure smtp relay settings under email or email.accounts.<account>; outbound smtp send is shipped, while imap-backed reply-loop serve support remains planned",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};

const WEBHOOK_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["webhook.enabled", "webhook.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const WEBHOOK_ENDPOINT_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "endpoint_url",
        label: "endpoint url",
        config_paths: &[
            "webhook.endpoint_url",
            "webhook.accounts.<account>.endpoint_url",
        ],
        env_pointer_paths: &[
            "webhook.endpoint_url_env",
            "webhook.accounts.<account>.endpoint_url_env",
        ],
        default_env_var: Some(WEBHOOK_ENDPOINT_URL_ENV),
    };
const WEBHOOK_SIGNING_SECRET_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "signing_secret",
        label: "signing secret",
        config_paths: &[
            "webhook.signing_secret",
            "webhook.accounts.<account>.signing_secret",
        ],
        env_pointer_paths: &[
            "webhook.signing_secret_env",
            "webhook.accounts.<account>.signing_secret_env",
        ],
        default_env_var: Some(WEBHOOK_SIGNING_SECRET_ENV),
    };
const WEBHOOK_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    WEBHOOK_ENABLED_REQUIREMENT,
    WEBHOOK_ENDPOINT_URL_REQUIREMENT,
];
const WEBHOOK_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    WEBHOOK_ENABLED_REQUIREMENT,
    WEBHOOK_SIGNING_SECRET_REQUIREMENT,
];
const WEBHOOK_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "http post send",
    command: "webhook-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: WEBHOOK_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Endpoint],
};
const WEBHOOK_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "inbound webhook service",
    command: "webhook-serve",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: true,
    requirements: WEBHOOK_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Endpoint],
};

pub const WEBHOOK_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "webhook",
        default_send_target_kind: ChannelCatalogTargetKind::Endpoint,
        send: WEBHOOK_SEND_OPERATION,
        serve: WEBHOOK_SERVE_OPERATION,
    };

pub const WEBHOOK_COMMAND_FAMILY_DESCRIPTOR: ChannelCommandFamilyDescriptor =
    ChannelCommandFamilyDescriptor {
        runtime: WEBHOOK_RUNTIME_COMMAND_DESCRIPTOR,
        catalog: WEBHOOK_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    };

const WEBHOOK_SERVE_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[
    ChannelDoctorCheckSpec {
        name: "webhook serve health",
        trigger: ChannelDoctorCheckTrigger::OperationHealth,
    },
    ChannelDoctorCheckSpec {
        name: "webhook serve runtime",
        trigger: ChannelDoctorCheckTrigger::ReadyRuntime,
    },
];

const WEBHOOK_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: WEBHOOK_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: WEBHOOK_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: WEBHOOK_SERVE_DOCTOR_CHECKS,
    },
];
const WEBHOOK_CAPABILITIES: &[ChannelCapability] = &[
    ChannelCapability::RuntimeBacked,
    ChannelCapability::MultiAccount,
    ChannelCapability::Send,
    ChannelCapability::Serve,
    ChannelCapability::RuntimeTracking,
];
const WEBHOOK_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure generic webhook delivery in loongclaw.toml under webhook or webhook.accounts.<account>; outbound endpoint send and inbound signed webhook serve are shipped, and webhook-serve requires --bind plus an optional --path override at runtime",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};

const GOOGLE_CHAT_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &[
            "google_chat.enabled",
            "google_chat.accounts.<account>.enabled",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const GOOGLE_CHAT_WEBHOOK_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "webhook_url",
        label: "incoming webhook url",
        config_paths: &[
            "google_chat.webhook_url",
            "google_chat.accounts.<account>.webhook_url",
        ],
        env_pointer_paths: &[
            "google_chat.webhook_url_env",
            "google_chat.accounts.<account>.webhook_url_env",
        ],
        default_env_var: Some(GOOGLE_CHAT_WEBHOOK_URL_ENV),
    };
const GOOGLE_CHAT_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    GOOGLE_CHAT_ENABLED_REQUIREMENT,
    GOOGLE_CHAT_WEBHOOK_URL_REQUIREMENT,
];
const GOOGLE_CHAT_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    GOOGLE_CHAT_ENABLED_REQUIREMENT,
    GOOGLE_CHAT_WEBHOOK_URL_REQUIREMENT,
];
const GOOGLE_CHAT_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "incoming webhook send",
    command: "google-chat-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: GOOGLE_CHAT_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Endpoint],
};
const GOOGLE_CHAT_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "interactive event service",
    command: "google-chat-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: GOOGLE_CHAT_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Endpoint],
};
pub const GOOGLE_CHAT_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "google-chat",
        default_send_target_kind: ChannelCatalogTargetKind::Endpoint,
        send: GOOGLE_CHAT_SEND_OPERATION,
        serve: GOOGLE_CHAT_SERVE_OPERATION,
    };
const GOOGLE_CHAT_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: GOOGLE_CHAT_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: GOOGLE_CHAT_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[],
    },
];
const GOOGLE_CHAT_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor =
    ChannelOnboardingDescriptor {
        strategy: ChannelOnboardingStrategy::ManualConfig,
        setup_hint: "configure Google Chat incoming webhook credentials in loongclaw.toml under google_chat or google_chat.accounts.<account>; outbound webhook send is shipped, while interactive event serve support remains planned",
        status_command: "loong doctor",
        repair_command: Some("loong doctor --fix"),
    };

const SIGNAL_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["signal.enabled", "signal.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const SIGNAL_SERVICE_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "service_url",
        label: "service url",
        config_paths: &[
            "signal.service_url",
            "signal.accounts.<account>.service_url",
        ],
        env_pointer_paths: &[
            "signal.service_url_env",
            "signal.accounts.<account>.service_url_env",
        ],
        default_env_var: Some(SIGNAL_SERVICE_URL_ENV),
    };
const SIGNAL_ACCOUNT_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "account",
        label: "account identifier",
        config_paths: &["signal.account", "signal.accounts.<account>.account"],
        env_pointer_paths: &[
            "signal.account_env",
            "signal.accounts.<account>.account_env",
        ],
        default_env_var: Some(SIGNAL_ACCOUNT_ENV),
    };
const SIGNAL_ALLOWED_SENDER_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_sender_ids",
        label: "allowed sender ids",
        config_paths: &[
            "signal.allowed_sender_ids",
            "signal.accounts.<account>.allowed_sender_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const SIGNAL_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    SIGNAL_ENABLED_REQUIREMENT,
    SIGNAL_SERVICE_URL_REQUIREMENT,
    SIGNAL_ACCOUNT_REQUIREMENT,
];
const SIGNAL_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    SIGNAL_ENABLED_REQUIREMENT,
    SIGNAL_SERVICE_URL_REQUIREMENT,
    SIGNAL_ACCOUNT_REQUIREMENT,
    SIGNAL_ALLOWED_SENDER_IDS_REQUIREMENT,
];
const SIGNAL_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "direct message send",
    command: "signal-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: SIGNAL_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const SIGNAL_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "linked-device listener",
    command: "signal-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: SIGNAL_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
pub const SIGNAL_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "signal",
        default_send_target_kind: ChannelCatalogTargetKind::Address,
        send: SIGNAL_SEND_OPERATION,
        serve: SIGNAL_SERVE_OPERATION,
    };
const SIGNAL_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: SIGNAL_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: SIGNAL_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[],
    },
];
const SIGNAL_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure signal bridge connection details in loongclaw.toml under signal or signal.accounts.<account>; outbound direct send is shipped, while inbound listener support remains planned",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};

const TEAMS_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["teams.enabled", "teams.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const TEAMS_WEBHOOK_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "webhook_url",
        label: "incoming webhook url",
        config_paths: &["teams.webhook_url", "teams.accounts.<account>.webhook_url"],
        env_pointer_paths: &[
            "teams.webhook_url_env",
            "teams.accounts.<account>.webhook_url_env",
        ],
        default_env_var: Some(TEAMS_WEBHOOK_URL_ENV),
    };
const TEAMS_APP_ID_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "app_id",
        label: "app id",
        config_paths: &["teams.app_id", "teams.accounts.<account>.app_id"],
        env_pointer_paths: &["teams.app_id_env", "teams.accounts.<account>.app_id_env"],
        default_env_var: Some(TEAMS_APP_ID_ENV),
    };
const TEAMS_APP_PASSWORD_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "app_password",
        label: "app password",
        config_paths: &[
            "teams.app_password",
            "teams.accounts.<account>.app_password",
        ],
        env_pointer_paths: &[
            "teams.app_password_env",
            "teams.accounts.<account>.app_password_env",
        ],
        default_env_var: Some(TEAMS_APP_PASSWORD_ENV),
    };
const TEAMS_TENANT_ID_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "tenant_id",
        label: "tenant id",
        config_paths: &["teams.tenant_id", "teams.accounts.<account>.tenant_id"],
        env_pointer_paths: &[
            "teams.tenant_id_env",
            "teams.accounts.<account>.tenant_id_env",
        ],
        default_env_var: Some(TEAMS_TENANT_ID_ENV),
    };
const TEAMS_ALLOWED_CONVERSATION_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_conversation_ids",
        label: "allowed conversation ids",
        config_paths: &[
            "teams.allowed_conversation_ids",
            "teams.accounts.<account>.allowed_conversation_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const TEAMS_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] =
    &[TEAMS_ENABLED_REQUIREMENT, TEAMS_WEBHOOK_URL_REQUIREMENT];
const TEAMS_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    TEAMS_ENABLED_REQUIREMENT,
    TEAMS_APP_ID_REQUIREMENT,
    TEAMS_APP_PASSWORD_REQUIREMENT,
    TEAMS_TENANT_ID_REQUIREMENT,
    TEAMS_ALLOWED_CONVERSATION_IDS_REQUIREMENT,
];
const TEAMS_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "incoming webhook send",
    command: "teams-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: TEAMS_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Endpoint],
};
const TEAMS_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "bot event service",
    command: "teams-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: TEAMS_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
pub const TEAMS_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "teams",
        default_send_target_kind: ChannelCatalogTargetKind::Endpoint,
        send: TEAMS_SEND_OPERATION,
        serve: TEAMS_SERVE_OPERATION,
    };
const TEAMS_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: TEAMS_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: TEAMS_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[],
    },
];
const TEAMS_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure Microsoft Teams webhook delivery in loongclaw.toml under teams or teams.accounts.<account>; outbound incoming-webhook send is shipped, while bot-framework serve support remains planned",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};

const MATTERMOST_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &[
            "mattermost.enabled",
            "mattermost.accounts.<account>.enabled",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const MATTERMOST_SERVER_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "server_url",
        label: "server url",
        config_paths: &[
            "mattermost.server_url",
            "mattermost.accounts.<account>.server_url",
        ],
        env_pointer_paths: &[
            "mattermost.server_url_env",
            "mattermost.accounts.<account>.server_url_env",
        ],
        default_env_var: Some(MATTERMOST_SERVER_URL_ENV),
    };
const MATTERMOST_BOT_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "bot_token",
        label: "bot token",
        config_paths: &[
            "mattermost.bot_token",
            "mattermost.accounts.<account>.bot_token",
        ],
        env_pointer_paths: &[
            "mattermost.bot_token_env",
            "mattermost.accounts.<account>.bot_token_env",
        ],
        default_env_var: Some(MATTERMOST_BOT_TOKEN_ENV),
    };
const MATTERMOST_ALLOWED_CHANNEL_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_channel_ids",
        label: "allowed channel ids",
        config_paths: &[
            "mattermost.allowed_channel_ids",
            "mattermost.accounts.<account>.allowed_channel_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const MATTERMOST_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    MATTERMOST_ENABLED_REQUIREMENT,
    MATTERMOST_SERVER_URL_REQUIREMENT,
    MATTERMOST_BOT_TOKEN_REQUIREMENT,
];
const MATTERMOST_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    MATTERMOST_ENABLED_REQUIREMENT,
    MATTERMOST_SERVER_URL_REQUIREMENT,
    MATTERMOST_BOT_TOKEN_REQUIREMENT,
    MATTERMOST_ALLOWED_CHANNEL_IDS_REQUIREMENT,
];
const MATTERMOST_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "channel send",
    command: "mattermost-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: MATTERMOST_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const MATTERMOST_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "event websocket service",
    command: "mattermost-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: MATTERMOST_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
pub const MATTERMOST_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "mattermost",
        default_send_target_kind: ChannelCatalogTargetKind::Conversation,
        send: MATTERMOST_SEND_OPERATION,
        serve: MATTERMOST_SERVE_OPERATION,
    };
const MATTERMOST_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: MATTERMOST_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: MATTERMOST_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[],
    },
];
const MATTERMOST_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure Mattermost server and bot credentials in loongclaw.toml under mattermost or mattermost.accounts.<account>; outbound post send is shipped, while inbound websocket serve support remains planned",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};

const NEXTCLOUD_TALK_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &[
            "nextcloud_talk.enabled",
            "nextcloud_talk.accounts.<account>.enabled",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const NEXTCLOUD_TALK_SERVER_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "server_url",
        label: "server url",
        config_paths: &[
            "nextcloud_talk.server_url",
            "nextcloud_talk.accounts.<account>.server_url",
        ],
        env_pointer_paths: &[
            "nextcloud_talk.server_url_env",
            "nextcloud_talk.accounts.<account>.server_url_env",
        ],
        default_env_var: Some(NEXTCLOUD_TALK_SERVER_URL_ENV),
    };
const NEXTCLOUD_TALK_SHARED_SECRET_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "shared_secret",
        label: "bot shared secret",
        config_paths: &[
            "nextcloud_talk.shared_secret",
            "nextcloud_talk.accounts.<account>.shared_secret",
        ],
        env_pointer_paths: &[
            "nextcloud_talk.shared_secret_env",
            "nextcloud_talk.accounts.<account>.shared_secret_env",
        ],
        default_env_var: Some(NEXTCLOUD_TALK_SHARED_SECRET_ENV),
    };
const NEXTCLOUD_TALK_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    NEXTCLOUD_TALK_ENABLED_REQUIREMENT,
    NEXTCLOUD_TALK_SERVER_URL_REQUIREMENT,
    NEXTCLOUD_TALK_SHARED_SECRET_REQUIREMENT,
];
const NEXTCLOUD_TALK_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    NEXTCLOUD_TALK_ENABLED_REQUIREMENT,
    NEXTCLOUD_TALK_SERVER_URL_REQUIREMENT,
    NEXTCLOUD_TALK_SHARED_SECRET_REQUIREMENT,
];
const NEXTCLOUD_TALK_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "room send",
    command: "nextcloud-talk-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: NEXTCLOUD_TALK_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const NEXTCLOUD_TALK_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "talk room service",
    command: "nextcloud-talk-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: NEXTCLOUD_TALK_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
pub const NEXTCLOUD_TALK_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "nextcloud-talk",
        default_send_target_kind: ChannelCatalogTargetKind::Conversation,
        send: NEXTCLOUD_TALK_SEND_OPERATION,
        serve: NEXTCLOUD_TALK_SERVE_OPERATION,
    };
const NEXTCLOUD_TALK_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: NEXTCLOUD_TALK_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: NEXTCLOUD_TALK_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[],
    },
];
const NEXTCLOUD_TALK_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor =
    ChannelOnboardingDescriptor {
        strategy: ChannelOnboardingStrategy::ManualConfig,
        setup_hint: "configure Nextcloud Talk bot credentials in loongclaw.toml under nextcloud_talk or nextcloud_talk.accounts.<account>; outbound room send is shipped, while inbound bot callback serve support remains planned",
        status_command: "loong doctor",
        repair_command: Some("loong doctor --fix"),
    };

const SYNOLOGY_CHAT_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &[
            "synology_chat.enabled",
            "synology_chat.accounts.<account>.enabled",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const SYNOLOGY_CHAT_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "token",
        label: "outgoing webhook token",
        config_paths: &[
            "synology_chat.token",
            "synology_chat.accounts.<account>.token",
        ],
        env_pointer_paths: &[
            "synology_chat.token_env",
            "synology_chat.accounts.<account>.token_env",
        ],
        default_env_var: Some(SYNOLOGY_CHAT_TOKEN_ENV),
    };
const SYNOLOGY_CHAT_INCOMING_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "incoming_url",
        label: "incoming webhook url",
        config_paths: &[
            "synology_chat.incoming_url",
            "synology_chat.accounts.<account>.incoming_url",
        ],
        env_pointer_paths: &[
            "synology_chat.incoming_url_env",
            "synology_chat.accounts.<account>.incoming_url_env",
        ],
        default_env_var: Some(SYNOLOGY_CHAT_INCOMING_URL_ENV),
    };
const SYNOLOGY_CHAT_ALLOWED_USER_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_user_ids",
        label: "allowed user ids",
        config_paths: &[
            "synology_chat.allowed_user_ids",
            "synology_chat.accounts.<account>.allowed_user_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const SYNOLOGY_CHAT_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    SYNOLOGY_CHAT_ENABLED_REQUIREMENT,
    SYNOLOGY_CHAT_INCOMING_URL_REQUIREMENT,
];
const SYNOLOGY_CHAT_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    SYNOLOGY_CHAT_ENABLED_REQUIREMENT,
    SYNOLOGY_CHAT_TOKEN_REQUIREMENT,
    SYNOLOGY_CHAT_INCOMING_URL_REQUIREMENT,
    SYNOLOGY_CHAT_ALLOWED_USER_IDS_REQUIREMENT,
];
const SYNOLOGY_CHAT_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "chat send",
    command: "synology-chat-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: SYNOLOGY_CHAT_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const SYNOLOGY_CHAT_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "webhook service",
    command: "synology-chat-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: SYNOLOGY_CHAT_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
pub const SYNOLOGY_CHAT_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "synology-chat",
        default_send_target_kind: ChannelCatalogTargetKind::Address,
        send: SYNOLOGY_CHAT_SEND_OPERATION,
        serve: SYNOLOGY_CHAT_SERVE_OPERATION,
    };
const SYNOLOGY_CHAT_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: SYNOLOGY_CHAT_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: SYNOLOGY_CHAT_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[],
    },
];
const SYNOLOGY_CHAT_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor =
    ChannelOnboardingDescriptor {
        strategy: ChannelOnboardingStrategy::ManualConfig,
        setup_hint: "configure Synology Chat incoming webhook credentials in loongclaw.toml under synology_chat or synology_chat.accounts.<account>; outbound incoming-webhook send is shipped, while inbound outgoing-webhook serve support remains planned",
        status_command: "loong doctor",
        repair_command: Some("loong doctor --fix"),
    };

const IRC_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["irc.enabled", "irc.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const IRC_SERVER_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "server",
        label: "server",
        config_paths: &["irc.server", "irc.accounts.<account>.server"],
        env_pointer_paths: &["irc.server_env", "irc.accounts.<account>.server_env"],
        default_env_var: Some(IRC_SERVER_ENV),
    };
const IRC_NICKNAME_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "nickname",
        label: "nickname",
        config_paths: &["irc.nickname", "irc.accounts.<account>.nickname"],
        env_pointer_paths: &["irc.nickname_env", "irc.accounts.<account>.nickname_env"],
        default_env_var: Some(IRC_NICKNAME_ENV),
    };
const IRC_CHANNEL_NAMES_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "channel_names",
        label: "channel names",
        config_paths: &["irc.channel_names", "irc.accounts.<account>.channel_names"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const IRC_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    IRC_ENABLED_REQUIREMENT,
    IRC_SERVER_REQUIREMENT,
    IRC_NICKNAME_REQUIREMENT,
];
const IRC_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    IRC_ENABLED_REQUIREMENT,
    IRC_SERVER_REQUIREMENT,
    IRC_NICKNAME_REQUIREMENT,
    IRC_CHANNEL_NAMES_REQUIREMENT,
];
const IRC_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "message send",
    command: "irc-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: IRC_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const IRC_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "relay loop",
    command: "irc-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: IRC_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
pub const IRC_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "irc",
        default_send_target_kind: ChannelCatalogTargetKind::Conversation,
        send: IRC_SEND_OPERATION,
        serve: IRC_SERVE_OPERATION,
    };
const IRC_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: IRC_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: IRC_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[],
    },
];
const IRC_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure IRC connection details in loongclaw.toml under irc or irc.accounts.<account>; outbound send is shipped, while long-lived relay-loop serve support remains planned",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};

const IMESSAGE_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["imessage.enabled", "imessage.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const IMESSAGE_BRIDGE_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "bridge_url",
        label: "bridge url",
        config_paths: &[
            "imessage.bridge_url",
            "imessage.accounts.<account>.bridge_url",
        ],
        env_pointer_paths: &[
            "imessage.bridge_url_env",
            "imessage.accounts.<account>.bridge_url_env",
        ],
        default_env_var: Some(IMESSAGE_BRIDGE_URL_ENV),
    };
const IMESSAGE_BRIDGE_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "bridge_token",
        label: "bridge token",
        config_paths: &[
            "imessage.bridge_token",
            "imessage.accounts.<account>.bridge_token",
        ],
        env_pointer_paths: &[
            "imessage.bridge_token_env",
            "imessage.accounts.<account>.bridge_token_env",
        ],
        default_env_var: Some(IMESSAGE_BRIDGE_TOKEN_ENV),
    };
const IMESSAGE_ALLOWED_CHAT_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_chat_ids",
        label: "allowed chat ids",
        config_paths: &[
            "imessage.allowed_chat_ids",
            "imessage.accounts.<account>.allowed_chat_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const IMESSAGE_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    IMESSAGE_ENABLED_REQUIREMENT,
    IMESSAGE_BRIDGE_URL_REQUIREMENT,
    IMESSAGE_BRIDGE_TOKEN_REQUIREMENT,
];
const IMESSAGE_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    IMESSAGE_ENABLED_REQUIREMENT,
    IMESSAGE_BRIDGE_URL_REQUIREMENT,
    IMESSAGE_BRIDGE_TOKEN_REQUIREMENT,
    IMESSAGE_ALLOWED_CHAT_IDS_REQUIREMENT,
];
const IMESSAGE_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "chat send",
    command: "imessage-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: IMESSAGE_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const IMESSAGE_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "bridge sync service",
    command: "imessage-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: IMESSAGE_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
pub const IMESSAGE_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "imessage",
        default_send_target_kind: ChannelCatalogTargetKind::Conversation,
        send: IMESSAGE_SEND_OPERATION,
        serve: IMESSAGE_SERVE_OPERATION,
    };
const IMESSAGE_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: IMESSAGE_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: IMESSAGE_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[],
    },
];
const IMESSAGE_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure BlueBubbles bridge credentials in loongclaw.toml under imessage or imessage.accounts.<account>; outbound chat send is shipped, while inbound bridge sync serve support remains planned",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};
pub(super) fn find_channel_registry_descriptor(
    raw: &str,
) -> Option<&'static ChannelRegistryDescriptor> {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    CHANNEL_REGISTRY.iter().find(|descriptor| {
        descriptor.id == normalized
            || descriptor
                .aliases
                .iter()
                .copied()
                .any(|alias| alias == normalized)
    })
}

pub(super) fn sorted_channel_registry_descriptors() -> Vec<&'static ChannelRegistryDescriptor> {
    let mut descriptors = CHANNEL_REGISTRY.iter().collect::<Vec<_>>();
    descriptors.sort_by_key(|descriptor| (descriptor.selection_order, descriptor.id));
    descriptors
}

pub(super) fn channel_catalog_entry_from_descriptor(
    descriptor: &ChannelRegistryDescriptor,
) -> ChannelCatalogEntry {
    let mut supported_target_kinds = Vec::new();
    for operation in descriptor.operations {
        for kind in operation.operation.supported_target_kinds {
            if !supported_target_kinds.contains(kind) {
                supported_target_kinds.push(*kind);
            }
        }
    }

    let plugin_bridge_contract = plugin_bridge_contract_from_descriptor(descriptor);

    ChannelCatalogEntry {
        id: descriptor.id,
        label: descriptor.label,
        selection_order: descriptor.selection_order,
        selection_label: descriptor.selection_label,
        blurb: descriptor.blurb,
        implementation_status: descriptor.implementation_status,
        capabilities: descriptor.capabilities.to_vec(),
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        onboarding: descriptor.onboarding,
        plugin_bridge_contract,
        supported_target_kinds,
        operations: descriptor
            .operations
            .iter()
            .map(|descriptor| descriptor.operation)
            .collect(),
    }
}

pub fn channel_inventory(config: &LoongClawConfig) -> ChannelInventory {
    channel_inventory_with_now(
        config,
        state::default_channel_runtime_state_dir().as_path(),
        now_ms(),
    )
}

pub fn channel_status_snapshots(config: &LoongClawConfig) -> Vec<ChannelStatusSnapshot> {
    channel_status_snapshots_with_now(
        config,
        state::default_channel_runtime_state_dir().as_path(),
        now_ms(),
    )
}

fn channel_inventory_with_now(
    config: &LoongClawConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelInventory {
    let channel_catalog = list_channel_catalog();
    let channels = channel_status_snapshots_with_now(config, runtime_dir, now_ms);
    let catalog_only_channels = catalog_only_channel_entries_from(&channel_catalog, &channels);
    let plugin_bridge_discovery_by_id =
        channel_surface_plugin_bridge_discovery_by_id(config, &channel_catalog);
    let channel_surfaces =
        build_channel_surfaces(&channel_catalog, &channels, &plugin_bridge_discovery_by_id);
    let channel_access_policies = build_channel_access_policies(config);
    ChannelInventory {
        channels,
        catalog_only_channels,
        channel_catalog,
        channel_surfaces,
        channel_access_policies,
    }
}

fn channel_status_snapshots_with_now(
    config: &LoongClawConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let mut snapshots = Vec::new();
    for descriptor in sorted_channel_registry_descriptors() {
        let Some(snapshot_builder) = descriptor.snapshot_builder else {
            continue;
        };
        let built_snapshots = snapshot_builder(descriptor, config, runtime_dir, now_ms);
        snapshots.extend(built_snapshots);
    }
    snapshots
}

fn build_channel_access_policies(
    config: &LoongClawConfig,
) -> Vec<ChannelConfiguredAccountAccessPolicy> {
    let mut policies = Vec::new();
    extend_telegram_channel_access_policies(&mut policies, config);
    extend_feishu_channel_access_policies(&mut policies, config);
    extend_matrix_channel_access_policies(&mut policies, config);
    extend_wecom_channel_access_policies(&mut policies, config);
    policies
}

fn extend_telegram_channel_access_policies(
    policies: &mut Vec<ChannelConfiguredAccountAccessPolicy>,
    config: &LoongClawConfig,
) {
    for configured_account_id in config.telegram.configured_account_ids() {
        let resolved = config
            .telegram
            .resolve_account(Some(configured_account_id.as_str()));
        let Ok(resolved) = resolved else {
            continue;
        };
        let access_policy = ChannelInboundAccessPolicy::from_i64_lists(
            resolved.allowed_chat_ids.as_slice(),
            resolved.allowed_sender_ids.as_slice(),
        );
        let mut summary = access_policy.summary();
        summary.mention_required = resolved.require_mention;
        policies.push(ChannelConfiguredAccountAccessPolicy {
            channel_id: "telegram",
            configured_account_id: resolved.configured_account_id,
            conversation_config_key: "allowed_chat_ids",
            sender_config_key: "allowed_sender_ids",
            summary,
        });
    }
}

fn extend_feishu_channel_access_policies(
    policies: &mut Vec<ChannelConfiguredAccountAccessPolicy>,
    config: &LoongClawConfig,
) {
    for configured_account_id in config.feishu.configured_account_ids() {
        let resolved = config
            .feishu
            .resolve_account(Some(configured_account_id.as_str()));
        let Ok(resolved) = resolved else {
            continue;
        };
        let access_policy = ChannelInboundAccessPolicy::from_string_lists(
            resolved.allowed_chat_ids.as_slice(),
            resolved.allowed_sender_ids.as_slice(),
            true,
        );
        let summary = access_policy.summary();
        policies.push(ChannelConfiguredAccountAccessPolicy {
            channel_id: "feishu",
            configured_account_id: resolved.configured_account_id,
            conversation_config_key: "allowed_chat_ids",
            sender_config_key: "allowed_sender_ids",
            summary,
        });
    }
}

fn extend_matrix_channel_access_policies(
    policies: &mut Vec<ChannelConfiguredAccountAccessPolicy>,
    config: &LoongClawConfig,
) {
    for configured_account_id in config.matrix.configured_account_ids() {
        let resolved = config
            .matrix
            .resolve_account(Some(configured_account_id.as_str()));
        let Ok(resolved) = resolved else {
            continue;
        };
        let access_policy = ChannelInboundAccessPolicy::from_string_lists(
            resolved.allowed_room_ids.as_slice(),
            resolved.allowed_sender_ids.as_slice(),
            false,
        );
        let mut summary = access_policy.summary();
        summary.mention_required = resolved.require_mention;
        policies.push(ChannelConfiguredAccountAccessPolicy {
            channel_id: "matrix",
            configured_account_id: resolved.configured_account_id,
            conversation_config_key: "allowed_room_ids",
            sender_config_key: "allowed_sender_ids",
            summary,
        });
    }
}

fn extend_wecom_channel_access_policies(
    policies: &mut Vec<ChannelConfiguredAccountAccessPolicy>,
    config: &LoongClawConfig,
) {
    for configured_account_id in config.wecom.configured_account_ids() {
        let resolved = config
            .wecom
            .resolve_account(Some(configured_account_id.as_str()));
        let Ok(resolved) = resolved else {
            continue;
        };
        let access_policy = ChannelInboundAccessPolicy::from_string_lists(
            resolved.allowed_conversation_ids.as_slice(),
            resolved.allowed_sender_ids.as_slice(),
            false,
        );
        let summary = access_policy.summary();
        policies.push(ChannelConfiguredAccountAccessPolicy {
            channel_id: "wecom",
            configured_account_id: resolved.configured_account_id,
            conversation_config_key: "allowed_conversation_ids",
            sender_config_key: "allowed_sender_ids",
            summary,
        });
    }
}

fn validate_http_url(
    field: &str,
    value: &str,
    policy: super::http::ChannelOutboundHttpPolicy,
    issues: &mut Vec<String>,
) -> Option<reqwest::Url> {
    let validation = super::http::validate_outbound_http_target(field, value, policy);
    match validation {
        Ok(url) => Some(url),
        Err(error) => {
            issues.push(error);
            None
        }
    }
}

fn validate_http_base_url(
    field: &str,
    value: &str,
    policy: super::http::ChannelOutboundHttpPolicy,
    issues: &mut Vec<String>,
) -> Option<reqwest::Url> {
    let validation = super::http::validate_outbound_http_base_url(field, value, policy);
    match validation {
        Ok(url) => Some(url),
        Err(error) => {
            issues.push(error);
            None
        }
    }
}

fn validate_websocket_url(field: &str, value: &str, issues: &mut Vec<String>) {
    let parsed_url = reqwest::Url::parse(value);
    let url = match parsed_url {
        Ok(url) => url,
        Err(error) => {
            let issue = format!("{field} is invalid: {error}");
            issues.push(issue);
            return;
        }
    };

    let scheme = url.scheme();
    let is_ws = scheme == "ws";
    let is_wss = scheme == "wss";
    if is_ws || is_wss {
        return;
    }

    let issue = format!("{field} must use ws or wss, got {scheme}");
    issues.push(issue);
}

#[cfg(test)]
fn runtime_backed_channel_registry_descriptors() -> Vec<&'static ChannelRegistryDescriptor> {
    sorted_channel_registry_descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.runtime.is_some())
        .collect()
}

fn build_telegram_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-telegram");
    let default_selection = config.telegram.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .telegram
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .telegram
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_telegram_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    runtime_dir,
                    now_ms,
                ),
                Err(error) => build_invalid_telegram_snapshot(
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

fn build_telegram_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedTelegramChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.bot_token().is_none() {
        send_issues.push("bot token is missing (telegram.bot_token or env)".to_owned());
    }

    let access_policy = ChannelInboundAccessPolicy::from_i64_lists(
        resolved.allowed_chat_ids.as_slice(),
        resolved.allowed_sender_ids.as_slice(),
    );
    let mut serve_issues = send_issues.clone();
    let has_allowlist = access_policy.has_conversation_restrictions();
    if !has_allowlist {
        serve_issues.push("allowed_chat_ids is empty".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            TELEGRAM_SEND_OPERATION,
            "binary built without feature `channel-telegram`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            TELEGRAM_SEND_OPERATION,
            "disabled by telegram account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(TELEGRAM_SEND_OPERATION, send_issues)
    } else {
        ready_operation(TELEGRAM_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            TELEGRAM_SERVE_OPERATION,
            "binary built without feature `channel-telegram`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            TELEGRAM_SERVE_OPERATION,
            "disabled by telegram account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(TELEGRAM_SERVE_OPERATION, serve_issues)
    } else {
        ready_operation(TELEGRAM_SERVE_OPERATION)
    };
    let send_operation = attach_runtime(
        ChannelPlatform::Telegram,
        TELEGRAM_SEND_OPERATION,
        send_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );
    let serve_operation = attach_runtime(
        ChannelPlatform::Telegram,
        TELEGRAM_SERVE_OPERATION,
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
        format!("polling_timeout_s={}", resolved.polling_timeout_s),
    ];
    if !resolved.allowed_chat_ids.is_empty() {
        let allowed_chat_ids = resolved
            .allowed_chat_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        notes.push(format!("allowed_chat_ids={allowed_chat_ids}"));
    }
    if !resolved.allowed_sender_ids.is_empty() {
        let allowed_sender_ids = resolved
            .allowed_sender_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        notes.push(format!("allowed_sender_ids={allowed_sender_ids}"));
    }
    notes.push(format!("require_mention={}", resolved.require_mention));
    if !resolved.acp.bootstrap_mcp_servers.is_empty() {
        notes.push(format!(
            "acp_bootstrap_mcp_servers={}",
            resolved.acp.bootstrap_mcp_servers.join(",")
        ));
    }
    if let Some(working_directory) = resolved.acp.resolved_working_directory() {
        notes.push(format!(
            "acp_working_directory={}",
            working_directory.display()
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
        api_base_url: Some(resolved.base_url),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_feishu_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-feishu");
    let default_selection = config.feishu.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .feishu
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .feishu
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_feishu_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    runtime_dir,
                    now_ms,
                ),
                Err(error) => build_invalid_feishu_snapshot(
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

fn build_matrix_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-matrix");
    let default_selection = config.matrix.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .matrix
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .matrix
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_matrix_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    runtime_dir,
                    now_ms,
                ),
                Err(error) => build_invalid_matrix_snapshot(
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

fn build_wecom_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-wecom");
    let default_selection = config.wecom.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .wecom
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .wecom
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_wecom_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    runtime_dir,
                    now_ms,
                ),
                Err(error) => build_invalid_wecom_snapshot(
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

fn build_discord_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-discord");
    let http_policy = super::http::outbound_http_policy_from_config(config);
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

fn build_slack_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-slack");
    let http_policy = super::http::outbound_http_policy_from_config(config);
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

fn build_line_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-line");
    let http_policy = super::http::outbound_http_policy_from_config(config);
    let default_selection = config.line.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .line
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .line
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_line_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                    runtime_dir,
                    now_ms,
                ),
                Err(error) => build_invalid_line_snapshot(
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

fn build_dingtalk_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-dingtalk");
    let http_policy = super::http::outbound_http_policy_from_config(config);
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

fn build_whatsapp_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-whatsapp");
    let http_policy = super::http::outbound_http_policy_from_config(config);
    let default_selection = config.whatsapp.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .whatsapp
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .whatsapp
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_whatsapp_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                    runtime_dir,
                    now_ms,
                ),
                Err(error) => build_invalid_whatsapp_snapshot(
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

fn build_email_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
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

fn build_webhook_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-webhook");
    let http_policy = super::http::outbound_http_policy_from_config(config);
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

fn build_google_chat_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-google-chat");
    let http_policy = super::http::outbound_http_policy_from_config(config);
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

fn build_signal_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-signal");
    let http_policy = super::http::outbound_http_policy_from_config(config);
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

fn build_teams_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-teams");
    let http_policy = super::http::outbound_http_policy_from_config(config);
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

fn build_mattermost_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-mattermost");
    let http_policy = super::http::outbound_http_policy_from_config(config);
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

fn build_nextcloud_talk_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-nextcloud-talk");
    let http_policy = super::http::outbound_http_policy_from_config(config);
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

fn build_synology_chat_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-synology-chat");
    let http_policy = super::http::outbound_http_policy_from_config(config);
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

fn build_irc_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
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

fn build_imessage_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-imessage");
    let http_policy = super::http::outbound_http_policy_from_config(config);
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

fn build_dingtalk_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedDingtalkChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: super::http::ChannelOutboundHttpPolicy,
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
            .and_then(super::http::redact_endpoint_status_url),
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
    http_policy: super::http::ChannelOutboundHttpPolicy,
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
            .and_then(|_| super::http::redact_endpoint_status_url(resolved_api_base_url.as_str())),
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
    http_policy: super::http::ChannelOutboundHttpPolicy,
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
            .and_then(|_| super::http::redact_endpoint_status_url(resolved_api_base_url.as_str())),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_line_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedLineChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: super::http::ChannelOutboundHttpPolicy,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.channel_access_token().is_none() {
        send_issues.push("channel_access_token is missing".to_owned());
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
            LINE_SEND_OPERATION,
            "binary built without feature `channel-line`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            LINE_SEND_OPERATION,
            "disabled by line account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(LINE_SEND_OPERATION, send_issues.clone())
    } else {
        ready_operation(LINE_SEND_OPERATION)
    };
    let send_operation = attach_runtime(
        ChannelPlatform::Line,
        LINE_SEND_OPERATION,
        send_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );

    let mut serve_issues = send_issues.clone();
    if resolved.channel_secret().is_none() {
        serve_issues.push("channel_secret is missing".to_owned());
    }

    let serve_operation = if !compiled {
        unsupported_operation(
            LINE_SERVE_OPERATION,
            "binary built without feature `channel-line`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            LINE_SERVE_OPERATION,
            "disabled by line account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(LINE_SERVE_OPERATION, serve_issues)
    } else {
        ready_operation(LINE_SERVE_OPERATION)
    };
    let serve_operation = attach_runtime(
        ChannelPlatform::Line,
        LINE_SERVE_OPERATION,
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
            .and_then(|_| super::http::redact_endpoint_status_url(resolved_api_base_url.as_str())),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_whatsapp_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedWhatsappChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: super::http::ChannelOutboundHttpPolicy,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.access_token().is_none() {
        send_issues.push("access_token is missing".to_owned());
    }
    if resolved.phone_number_id().is_none() {
        send_issues.push("phone_number_id is missing".to_owned());
    }

    let resolved_api_base_url = resolved.resolved_api_base_url();
    let api_base_url = validate_http_base_url(
        "api_base_url",
        resolved_api_base_url.as_str(),
        http_policy,
        &mut send_issues,
    );

    let mut serve_issues = send_issues.clone();
    if resolved.verify_token().is_none() {
        serve_issues.push("verify_token is missing".to_owned());
    }
    if resolved.app_secret().is_none() {
        serve_issues.push("app_secret is missing".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            WHATSAPP_SEND_OPERATION,
            "binary built without feature `channel-whatsapp`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            WHATSAPP_SEND_OPERATION,
            "disabled by whatsapp account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(WHATSAPP_SEND_OPERATION, send_issues)
    } else {
        ready_operation(WHATSAPP_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            WHATSAPP_SERVE_OPERATION,
            "binary built without feature `channel-whatsapp`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            WHATSAPP_SERVE_OPERATION,
            "disabled by whatsapp account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(WHATSAPP_SERVE_OPERATION, serve_issues)
    } else {
        ready_operation(WHATSAPP_SERVE_OPERATION)
    };
    let send_operation = attach_runtime(
        ChannelPlatform::WhatsApp,
        WHATSAPP_SEND_OPERATION,
        send_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );
    let serve_operation = attach_runtime(
        ChannelPlatform::WhatsApp,
        WHATSAPP_SERVE_OPERATION,
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
    ];
    if let Some(phone_number_id) = resolved.phone_number_id() {
        notes.push(format!("phone_number_id={phone_number_id}"));
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
        api_base_url: api_base_url
            .as_ref()
            .and_then(|_| super::http::redact_endpoint_status_url(resolved_api_base_url.as_str())),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
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
    http_policy: super::http::ChannelOutboundHttpPolicy,
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
            .and_then(super::http::redact_generic_webhook_status_url),
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
    http_policy: super::http::ChannelOutboundHttpPolicy,
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
            .and_then(super::http::redact_endpoint_status_url),
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
    http_policy: super::http::ChannelOutboundHttpPolicy,
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
            .and_then(super::http::redact_endpoint_status_url),
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
    http_policy: super::http::ChannelOutboundHttpPolicy,
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
            .and_then(super::http::redact_endpoint_status_url),
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
    http_policy: super::http::ChannelOutboundHttpPolicy,
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
            .and_then(super::http::redact_endpoint_status_url),
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
    http_policy: super::http::ChannelOutboundHttpPolicy,
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
            .and_then(super::http::redact_endpoint_status_url),
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
    http_policy: super::http::ChannelOutboundHttpPolicy,
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
            .and_then(super::http::redact_generic_webhook_status_url),
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
    http_policy: super::http::ChannelOutboundHttpPolicy,
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
            .and_then(super::http::redact_endpoint_status_url),
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

fn build_feishu_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedFeishuChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.app_id().is_none() {
        send_issues.push("app_id is missing".to_owned());
    }
    if resolved.app_secret().is_none() {
        send_issues.push("app_secret is missing".to_owned());
    }

    let access_policy = ChannelInboundAccessPolicy::from_string_lists(
        resolved.allowed_chat_ids.as_slice(),
        resolved.allowed_sender_ids.as_slice(),
        true,
    );
    let mut serve_issues = send_issues.clone();
    let has_allowlist = access_policy.has_conversation_restrictions();
    if !has_allowlist {
        serve_issues.push("allowed_chat_ids is empty".to_owned());
    }
    if resolved.mode == FeishuChannelServeMode::Webhook {
        if resolved.verification_token().is_none() {
            serve_issues.push("verification_token is missing".to_owned());
        }
        if resolved.encrypt_key().is_none() {
            serve_issues.push("encrypt_key is missing".to_owned());
        }
    }

    let send_operation = if !compiled {
        unsupported_operation(
            FEISHU_SEND_OPERATION,
            "binary built without feature `channel-feishu`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            FEISHU_SEND_OPERATION,
            "disabled by feishu account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(FEISHU_SEND_OPERATION, send_issues)
    } else {
        ready_operation(FEISHU_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            FEISHU_SERVE_OPERATION,
            "binary built without feature `channel-feishu`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            FEISHU_SERVE_OPERATION,
            "disabled by feishu account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(FEISHU_SERVE_OPERATION, serve_issues)
    } else {
        ready_operation(FEISHU_SERVE_OPERATION)
    };
    let send_operation = attach_runtime(
        ChannelPlatform::Feishu,
        FEISHU_SEND_OPERATION,
        send_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );
    let serve_operation = attach_runtime(
        ChannelPlatform::Feishu,
        FEISHU_SERVE_OPERATION,
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
        format!("mode={}", resolved.mode.as_str()),
        format!("receive_id_type={}", resolved.receive_id_type),
    ];
    if let Some(allowed_chat_ids) = access_policy.string_conversations() {
        notes.push(format!("allowed_chat_ids={}", allowed_chat_ids.join(",")));
    }
    if let Some(allowed_sender_ids) = access_policy.string_senders() {
        notes.push(format!(
            "allowed_sender_ids={}",
            allowed_sender_ids.join(",")
        ));
    }
    if resolved.mode == FeishuChannelServeMode::Webhook {
        notes.push(format!("webhook_bind={}", resolved.webhook_bind));
        notes.push(format!("webhook_path={}", resolved.webhook_path));
    }
    if !resolved.acp.bootstrap_mcp_servers.is_empty() {
        notes.push(format!(
            "acp_bootstrap_mcp_servers={}",
            resolved.acp.bootstrap_mcp_servers.join(",")
        ));
    }
    if let Some(working_directory) = resolved.acp.resolved_working_directory() {
        notes.push(format!(
            "acp_working_directory={}",
            working_directory.display()
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
        api_base_url: Some(resolved.resolved_base_url()),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_matrix_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedMatrixChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.access_token().is_none() {
        send_issues.push("access_token is missing".to_owned());
    }
    if resolved.resolved_base_url().is_none() {
        send_issues.push("base_url is missing".to_owned());
    }

    let access_policy = ChannelInboundAccessPolicy::from_string_lists(
        resolved.allowed_room_ids.as_slice(),
        resolved.allowed_sender_ids.as_slice(),
        false,
    );
    let mut serve_issues = send_issues.clone();
    let has_allowlist = access_policy.has_conversation_restrictions();
    if !has_allowlist {
        serve_issues.push("allowed_room_ids is empty".to_owned());
    }
    let has_user_id = resolved
        .user_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if resolved.ignore_self_messages && !has_user_id {
        serve_issues.push("user_id is missing while ignore_self_messages is enabled".to_owned());
    }
    if resolved.require_mention && !has_user_id {
        serve_issues.push("user_id is missing while require_mention is enabled".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            MATRIX_SEND_OPERATION,
            "binary built without feature `channel-matrix`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            MATRIX_SEND_OPERATION,
            "disabled by matrix account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(MATRIX_SEND_OPERATION, send_issues)
    } else {
        ready_operation(MATRIX_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            MATRIX_SERVE_OPERATION,
            "binary built without feature `channel-matrix`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            MATRIX_SERVE_OPERATION,
            "disabled by matrix account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(MATRIX_SERVE_OPERATION, serve_issues)
    } else {
        ready_operation(MATRIX_SERVE_OPERATION)
    };
    let send_operation = attach_runtime(
        ChannelPlatform::Matrix,
        MATRIX_SEND_OPERATION,
        send_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );
    let serve_operation = attach_runtime(
        ChannelPlatform::Matrix,
        MATRIX_SERVE_OPERATION,
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
        format!("sync_timeout_s={}", resolved.sync_timeout_s),
        format!("ignore_self_messages={}", resolved.ignore_self_messages),
    ];
    if let Some(allowed_room_ids) = access_policy.string_conversations() {
        notes.push(format!("allowed_room_ids={}", allowed_room_ids.join(",")));
    }
    if let Some(allowed_sender_ids) = access_policy.string_senders() {
        notes.push(format!(
            "allowed_sender_ids={}",
            allowed_sender_ids.join(",")
        ));
    }
    notes.push(format!("require_mention={}", resolved.require_mention));
    if let Some(user_id) = resolved.user_id.as_deref() {
        notes.push(format!("user_id={user_id}"));
    }
    if !resolved.acp.bootstrap_mcp_servers.is_empty() {
        notes.push(format!(
            "acp_bootstrap_mcp_servers={}",
            resolved.acp.bootstrap_mcp_servers.join(",")
        ));
    }
    if let Some(working_directory) = resolved.acp.resolved_working_directory() {
        notes.push(format!(
            "acp_working_directory={}",
            working_directory.display()
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
        api_base_url: resolved.resolved_base_url(),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_wecom_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedWecomChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.bot_id().is_none() {
        send_issues.push("bot_id is missing".to_owned());
    }
    if resolved.secret().is_none() {
        send_issues.push("secret is missing".to_owned());
    }

    let websocket_url = resolved.resolved_websocket_url();
    validate_websocket_url(
        "wecom.websocket_url",
        websocket_url.as_str(),
        &mut send_issues,
    );

    let access_policy = ChannelInboundAccessPolicy::from_string_lists(
        resolved.allowed_conversation_ids.as_slice(),
        resolved.allowed_sender_ids.as_slice(),
        false,
    );
    let mut serve_issues = send_issues.clone();
    let has_allowlist = access_policy.has_conversation_restrictions();
    if !has_allowlist {
        serve_issues.push("allowed_conversation_ids is empty".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            WECOM_SEND_OPERATION,
            "binary built without feature `channel-wecom`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            WECOM_SEND_OPERATION,
            "disabled by wecom account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(WECOM_SEND_OPERATION, send_issues)
    } else {
        ready_operation(WECOM_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            WECOM_SERVE_OPERATION,
            "binary built without feature `channel-wecom`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            WECOM_SERVE_OPERATION,
            "disabled by wecom account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(WECOM_SERVE_OPERATION, serve_issues)
    } else {
        ready_operation(WECOM_SERVE_OPERATION)
    };
    let send_operation = attach_runtime(
        ChannelPlatform::Wecom,
        WECOM_SEND_OPERATION,
        send_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );
    let serve_operation = attach_runtime(
        ChannelPlatform::Wecom,
        WECOM_SERVE_OPERATION,
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
        format!("websocket_url={websocket_url}"),
        format!("ping_interval_s={}", resolved.ping_interval_s),
        format!("reconnect_interval_s={}", resolved.reconnect_interval_s),
    ];
    if let Some(allowed_conversation_ids) = access_policy.string_conversations() {
        notes.push(format!(
            "allowed_conversation_ids={}",
            allowed_conversation_ids.join(",")
        ));
    }
    if let Some(allowed_sender_ids) = access_policy.string_senders() {
        notes.push(format!(
            "allowed_sender_ids={}",
            allowed_sender_ids.join(",")
        ));
    }
    if !resolved.acp.bootstrap_mcp_servers.is_empty() {
        notes.push(format!(
            "acp_bootstrap_mcp_servers={}",
            resolved.acp.bootstrap_mcp_servers.join(",")
        ));
    }
    if let Some(working_directory) = resolved.acp.resolved_working_directory() {
        notes.push(format!(
            "acp_working_directory={}",
            working_directory.display()
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
        api_base_url: Some(websocket_url),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_telegram_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            TELEGRAM_SEND_OPERATION,
            "binary built without feature `channel-telegram`".to_owned(),
        )
    } else {
        misconfigured_operation(TELEGRAM_SEND_OPERATION, vec![error.clone()])
    };
    let serve_operation = if !compiled {
        unsupported_operation(
            TELEGRAM_SERVE_OPERATION,
            "binary built without feature `channel-telegram`".to_owned(),
        )
    } else {
        misconfigured_operation(TELEGRAM_SERVE_OPERATION, vec![error.clone()])
    };

    let mut notes = vec![
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_feishu_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            FEISHU_SEND_OPERATION,
            "binary built without feature `channel-feishu`".to_owned(),
        )
    } else {
        misconfigured_operation(FEISHU_SEND_OPERATION, vec![error.clone()])
    };
    let serve_operation = if !compiled {
        unsupported_operation(
            FEISHU_SERVE_OPERATION,
            "binary built without feature `channel-feishu`".to_owned(),
        )
    } else {
        misconfigured_operation(FEISHU_SERVE_OPERATION, vec![error.clone()])
    };

    let mut notes = vec![
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_matrix_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            MATRIX_SEND_OPERATION,
            "binary built without feature `channel-matrix`".to_owned(),
        )
    } else {
        misconfigured_operation(MATRIX_SEND_OPERATION, vec![error.clone()])
    };
    let serve_operation = if !compiled {
        unsupported_operation(
            MATRIX_SERVE_OPERATION,
            "binary built without feature `channel-matrix`".to_owned(),
        )
    } else {
        misconfigured_operation(MATRIX_SERVE_OPERATION, vec![error.clone()])
    };

    let mut notes = vec![
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_wecom_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            WECOM_SEND_OPERATION,
            "binary built without feature `channel-wecom`".to_owned(),
        )
    } else {
        misconfigured_operation(WECOM_SEND_OPERATION, vec![error.clone()])
    };
    let serve_operation = if !compiled {
        unsupported_operation(
            WECOM_SERVE_OPERATION,
            "binary built without feature `channel-wecom`".to_owned(),
        )
    } else {
        misconfigured_operation(WECOM_SERVE_OPERATION, vec![error.clone()])
    };

    let mut notes = vec![
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_discord_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            DISCORD_SEND_OPERATION,
            "binary built without feature `channel-discord`".to_owned(),
        )
    } else {
        misconfigured_operation(DISCORD_SEND_OPERATION, vec![error.clone()])
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
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_slack_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            SLACK_SEND_OPERATION,
            "binary built without feature `channel-slack`".to_owned(),
        )
    } else {
        misconfigured_operation(SLACK_SEND_OPERATION, vec![error.clone()])
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
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_line_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            LINE_SEND_OPERATION,
            "binary built without feature `channel-line`".to_owned(),
        )
    } else {
        misconfigured_operation(LINE_SEND_OPERATION, vec![error.clone()])
    };
    let send_operation = attach_runtime(
        ChannelPlatform::Line,
        LINE_SEND_OPERATION,
        send_operation,
        configured_account_id,
        configured_account_id,
        runtime_dir,
        now_ms,
    );
    let serve_operation = if !compiled {
        unsupported_operation(
            LINE_SERVE_OPERATION,
            "binary built without feature `channel-line`".to_owned(),
        )
    } else {
        misconfigured_operation(LINE_SERVE_OPERATION, vec![error.clone()])
    };
    let serve_operation = attach_runtime(
        ChannelPlatform::Line,
        LINE_SERVE_OPERATION,
        serve_operation,
        configured_account_id,
        configured_account_id,
        runtime_dir,
        now_ms,
    );

    let mut notes = vec![
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_dingtalk_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            DINGTALK_SEND_OPERATION,
            "binary built without feature `channel-dingtalk`".to_owned(),
        )
    } else {
        misconfigured_operation(DINGTALK_SEND_OPERATION, vec![error.clone()])
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
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_whatsapp_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            WHATSAPP_SEND_OPERATION,
            "binary built without feature `channel-whatsapp`".to_owned(),
        )
    } else {
        misconfigured_operation(WHATSAPP_SEND_OPERATION, vec![error.clone()])
    };
    let serve_operation = if !compiled {
        unsupported_operation(
            WHATSAPP_SERVE_OPERATION,
            "binary built without feature `channel-whatsapp`".to_owned(),
        )
    } else {
        misconfigured_operation(WHATSAPP_SERVE_OPERATION, vec![error.clone()])
    };

    let mut notes = vec![
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_email_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            EMAIL_SEND_OPERATION,
            "binary built without feature `channel-email`".to_owned(),
        )
    } else {
        misconfigured_operation(EMAIL_SEND_OPERATION, vec![error.clone()])
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
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_webhook_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            WEBHOOK_SEND_OPERATION,
            "binary built without feature `channel-webhook`".to_owned(),
        )
    } else {
        misconfigured_operation(WEBHOOK_SEND_OPERATION, vec![error.clone()])
    };
    let send_operation = attach_runtime(
        ChannelPlatform::Webhook,
        WEBHOOK_SEND_OPERATION,
        send_operation,
        configured_account_id,
        configured_account_id,
        runtime_dir,
        now_ms,
    );
    let serve_operation = if !compiled {
        unsupported_operation(
            WEBHOOK_SERVE_OPERATION,
            "binary built without feature `channel-webhook`".to_owned(),
        )
    } else {
        misconfigured_operation(WEBHOOK_SERVE_OPERATION, vec![error.clone()])
    };
    let serve_operation = attach_runtime(
        ChannelPlatform::Webhook,
        WEBHOOK_SERVE_OPERATION,
        serve_operation,
        configured_account_id,
        configured_account_id,
        runtime_dir,
        now_ms,
    );

    let mut notes = vec![
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_google_chat_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            GOOGLE_CHAT_SEND_OPERATION,
            "binary built without feature `channel-google-chat`".to_owned(),
        )
    } else {
        misconfigured_operation(GOOGLE_CHAT_SEND_OPERATION, vec![error.clone()])
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
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_signal_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            SIGNAL_SEND_OPERATION,
            "binary built without feature `channel-signal`".to_owned(),
        )
    } else {
        misconfigured_operation(SIGNAL_SEND_OPERATION, vec![error.clone()])
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
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_irc_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            IRC_SEND_OPERATION,
            "binary built without feature `channel-irc`".to_owned(),
        )
    } else {
        misconfigured_operation(IRC_SEND_OPERATION, vec![error.clone()])
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
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_teams_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            TEAMS_SEND_OPERATION,
            "binary built without feature `channel-teams`".to_owned(),
        )
    } else {
        misconfigured_operation(TEAMS_SEND_OPERATION, vec![error.clone()])
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
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_imessage_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            IMESSAGE_SEND_OPERATION,
            "binary built without feature `channel-imessage`".to_owned(),
        )
    } else {
        misconfigured_operation(IMESSAGE_SEND_OPERATION, vec![error.clone()])
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
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_mattermost_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            MATTERMOST_SEND_OPERATION,
            "binary built without feature `channel-mattermost`".to_owned(),
        )
    } else {
        misconfigured_operation(MATTERMOST_SEND_OPERATION, vec![error.clone()])
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
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_nextcloud_talk_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            NEXTCLOUD_TALK_SEND_OPERATION,
            "binary built without feature `channel-nextcloud-talk`".to_owned(),
        )
    } else {
        misconfigured_operation(NEXTCLOUD_TALK_SEND_OPERATION, vec![error.clone()])
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
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_synology_chat_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            SYNOLOGY_CHAT_SEND_OPERATION,
            "binary built without feature `channel-synology-chat`".to_owned(),
        )
    } else {
        misconfigured_operation(SYNOLOGY_CHAT_SEND_OPERATION, vec![error.clone()])
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
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
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
        configured_account_id: configured_account_id.to_owned(),
        configured_account_label: configured_account_id.to_owned(),
        is_default_account,
        default_account_source,
        label: descriptor.label,
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        compiled,
        enabled: false,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn ready_operation(operation: ChannelCatalogOperation) -> ChannelOperationStatus {
    ChannelOperationStatus {
        id: operation.id,
        label: operation.label,
        command: operation.command,
        health: ChannelOperationHealth::Ready,
        detail: "ready".to_owned(),
        issues: Vec::new(),
        runtime: None,
    }
}

fn disabled_operation(
    operation: ChannelCatalogOperation,
    detail: String,
) -> ChannelOperationStatus {
    ChannelOperationStatus {
        id: operation.id,
        label: operation.label,
        command: operation.command,
        health: ChannelOperationHealth::Disabled,
        detail,
        issues: Vec::new(),
        runtime: None,
    }
}

fn unsupported_operation(
    operation: ChannelCatalogOperation,
    detail: String,
) -> ChannelOperationStatus {
    ChannelOperationStatus {
        id: operation.id,
        label: operation.label,
        command: operation.command,
        health: ChannelOperationHealth::Unsupported,
        detail: detail.clone(),
        issues: vec![detail],
        runtime: None,
    }
}

fn misconfigured_operation(
    operation: ChannelCatalogOperation,
    issues: Vec<String>,
) -> ChannelOperationStatus {
    ChannelOperationStatus {
        id: operation.id,
        label: operation.label,
        command: operation.command,
        health: ChannelOperationHealth::Misconfigured,
        detail: issues.join("; "),
        issues,
        runtime: None,
    }
}

fn attach_runtime(
    platform: ChannelPlatform,
    operation: ChannelCatalogOperation,
    mut status: ChannelOperationStatus,
    account_id: &str,
    account_label: &str,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelOperationStatus {
    if operation.tracks_runtime {
        status.runtime = state::load_channel_operation_runtime_for_account_from_dir(
            runtime_dir,
            platform,
            operation.id,
            account_id,
            now_ms,
        )
        .map(|mut runtime| {
            if runtime.account_id.is_none() {
                runtime.account_id = Some(account_id.to_owned());
            }
            if runtime.account_label.is_none() {
                runtime.account_label = Some(account_label.to_owned());
            }
            runtime
        })
        .or(Some(ChannelOperationRuntime {
            running: false,
            stale: false,
            busy: false,
            active_runs: 0,
            last_run_activity_at: None,
            last_heartbeat_at: None,
            pid: None,
            account_id: Some(account_id.to_owned()),
            account_label: Some(account_label.to_owned()),
            instance_count: 0,
            running_instances: 0,
            stale_instances: 0,
        }));
    }
    status
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

mod tlon_support;

#[cfg(test)]
mod hotspot_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_channel_platform_maps_lark_alias_to_feishu() {
        assert_eq!(
            normalize_channel_platform("lark"),
            Some(ChannelPlatform::Feishu)
        );
        assert_eq!(
            normalize_channel_platform(" TELEGRAM "),
            Some(ChannelPlatform::Telegram)
        );
        assert_eq!(normalize_channel_platform("discord"), None);
    }

    #[test]
    fn resolve_channel_selection_order_uses_registry_metadata() {
        assert_eq!(resolve_channel_selection_order("telegram"), Some(10));
        assert_eq!(resolve_channel_selection_order("discord-bot"), Some(40));
        assert_eq!(resolve_channel_selection_order(" DISCORD-BOT "), Some(40));
        assert_eq!(resolve_channel_selection_order("unknown"), None);
    }

    #[test]
    fn normalize_channel_catalog_id_maps_runtime_and_stub_aliases() {
        assert_eq!(normalize_channel_catalog_id("lark"), Some("feishu"));
        assert_eq!(normalize_channel_catalog_id(" TELEGRAM "), Some("telegram"));
        assert_eq!(normalize_channel_catalog_id("discord-bot"), Some("discord"));
        assert_eq!(normalize_channel_catalog_id("slack"), Some("slack"));
        assert_eq!(normalize_channel_catalog_id("gchat"), Some("google-chat"));
        assert_eq!(normalize_channel_catalog_id("wechat"), Some("weixin"));
        assert_eq!(normalize_channel_catalog_id("wx"), Some("weixin"));
        assert_eq!(normalize_channel_catalog_id("qq"), Some("qqbot"));
        assert_eq!(normalize_channel_catalog_id("onebot-v11"), Some("onebot"));
        assert_eq!(
            normalize_channel_catalog_id("synochat"),
            Some("synology-chat")
        );
        assert_eq!(
            normalize_channel_catalog_id("bluebubbles"),
            Some("imessage")
        );
        assert_eq!(normalize_channel_catalog_id("urbit"), Some("tlon"));
        assert_eq!(normalize_channel_catalog_id("web-ui"), Some("webchat"));
        assert_eq!(normalize_channel_catalog_id("unknown"), None);
    }

    #[test]
    fn runtime_backed_channel_registry_descriptors_only_include_runtime_backed_surfaces() {
        let runtime_backed = runtime_backed_channel_registry_descriptors();

        assert_eq!(
            runtime_backed
                .iter()
                .map(|descriptor| descriptor.id)
                .collect::<Vec<_>>(),
            vec![
                "telegram", "feishu", "matrix", "wecom", "line", "whatsapp", "webhook"
            ]
        );
        assert!(
            runtime_backed
                .iter()
                .all(|descriptor| descriptor.runtime.is_some())
        );
    }

    #[test]
    fn resolve_channel_runtime_command_descriptor_returns_runtime_surface_metadata() {
        let telegram = resolve_channel_runtime_command_descriptor("telegram")
            .expect("telegram runtime command descriptor");
        let lark =
            resolve_channel_runtime_command_descriptor("lark").expect("lark runtime descriptor");
        let line =
            resolve_channel_runtime_command_descriptor("line").expect("line runtime descriptor");
        let wecom =
            resolve_channel_runtime_command_descriptor("wecom").expect("wecom runtime descriptor");
        let webhook = resolve_channel_runtime_command_descriptor("webhook")
            .expect("webhook runtime descriptor");

        assert_eq!(telegram.channel_id, "telegram");
        assert_eq!(telegram.platform, ChannelPlatform::Telegram);
        assert_eq!(telegram.serve_bootstrap_agent_id, "channel-telegram");

        assert_eq!(lark.channel_id, "feishu");
        assert_eq!(lark.platform, ChannelPlatform::Feishu);
        assert_eq!(lark.serve_bootstrap_agent_id, "channel-feishu");

        assert_eq!(line.channel_id, "line");
        assert_eq!(line.platform, ChannelPlatform::Line);
        assert_eq!(line.serve_bootstrap_agent_id, "channel-line");

        assert_eq!(wecom.channel_id, "wecom");
        assert_eq!(wecom.platform, ChannelPlatform::Wecom);
        assert_eq!(wecom.serve_bootstrap_agent_id, "channel-wecom");

        assert_eq!(webhook.channel_id, "webhook");
        assert_eq!(webhook.platform, ChannelPlatform::Webhook);
        assert_eq!(webhook.serve_bootstrap_agent_id, "channel-webhook");
    }

    #[test]
    fn resolve_channel_runtime_command_descriptor_skips_stub_surfaces() {
        assert_eq!(resolve_channel_runtime_command_descriptor("discord"), None);
        assert_eq!(
            resolve_channel_runtime_command_descriptor("slack-bot"),
            None
        );
    }

    #[test]
    fn resolve_channel_catalog_command_family_descriptor_includes_matrix_runtime_channel() {
        let matrix = resolve_channel_catalog_command_family_descriptor("matrix")
            .expect("matrix catalog command family");

        assert_eq!(matrix.channel_id, "matrix");
        assert_eq!(matrix.send.id, CHANNEL_OPERATION_SEND_ID);
        assert_eq!(matrix.send.command, "matrix-send");
        assert_eq!(matrix.serve.id, CHANNEL_OPERATION_SERVE_ID);
        assert_eq!(matrix.serve.command, "matrix-serve");
        assert_eq!(
            matrix.default_send_target_kind,
            ChannelCatalogTargetKind::Conversation
        );
    }

    #[test]
    fn resolve_channel_catalog_command_family_descriptor_includes_wecom_runtime_channel() {
        let wecom = resolve_channel_catalog_command_family_descriptor("wecom")
            .expect("wecom catalog command family");

        assert_eq!(wecom.channel_id, "wecom");
        assert_eq!(wecom.send.id, CHANNEL_OPERATION_SEND_ID);
        assert_eq!(wecom.send.command, "wecom-send");
        assert_eq!(wecom.serve.id, CHANNEL_OPERATION_SERVE_ID);
        assert_eq!(wecom.serve.command, "wecom-serve");
        assert_eq!(
            wecom.default_send_target_kind,
            ChannelCatalogTargetKind::Conversation
        );
    }

    #[test]
    fn resolve_channel_catalog_command_family_descriptor_rejects_unknown_channels() {
        assert_eq!(
            resolve_channel_catalog_command_family_descriptor("unknown-channel"),
            None
        );
    }

    #[test]
    fn resolve_channel_command_family_descriptor_returns_runtime_send_and_serve_metadata() {
        let telegram = resolve_channel_command_family_descriptor("telegram")
            .expect("telegram command family descriptor");
        let lark =
            resolve_channel_command_family_descriptor("lark").expect("lark family descriptor");
        let telegram_catalog = resolve_channel_catalog_command_family_descriptor("telegram")
            .expect("telegram catalog family");
        let lark_catalog =
            resolve_channel_catalog_command_family_descriptor("lark").expect("lark catalog family");

        assert_eq!(telegram.runtime.channel_id, "telegram");
        assert_eq!(telegram.runtime.platform, ChannelPlatform::Telegram);
        assert_eq!(telegram.catalog, telegram_catalog);
        assert_eq!(telegram.catalog.send.id, CHANNEL_OPERATION_SEND_ID);
        assert_eq!(telegram.catalog.send.command, "telegram-send");
        assert_eq!(telegram.catalog.serve.id, CHANNEL_OPERATION_SERVE_ID);
        assert_eq!(telegram.catalog.serve.command, "telegram-serve");
        assert_eq!(
            telegram.catalog.send.default_target_kind(),
            Some(telegram.catalog.default_send_target_kind)
        );

        assert_eq!(lark.runtime.channel_id, "feishu");
        assert_eq!(lark.runtime.platform, ChannelPlatform::Feishu);
        assert_eq!(lark.catalog, lark_catalog);
        assert_eq!(lark.catalog.send.command, "feishu-send");
        assert_eq!(lark.catalog.serve.command, "feishu-serve");
        assert_eq!(
            lark.catalog.send.default_target_kind(),
            Some(lark.catalog.default_send_target_kind)
        );
    }

    #[test]
    fn resolve_channel_command_family_descriptor_skips_stub_surfaces() {
        assert_eq!(resolve_channel_command_family_descriptor("discord"), None);
        assert_eq!(resolve_channel_command_family_descriptor("slack-bot"), None);
    }

    #[test]
    fn resolve_channel_operation_descriptor_combines_catalog_and_doctor_metadata() {
        let lark_serve = resolve_channel_operation_descriptor("lark", CHANNEL_OPERATION_SERVE_ID)
            .expect("lark serve descriptor");
        assert_eq!(lark_serve.operation.command, "feishu-serve");
        assert_eq!(
            lark_serve
                .doctor
                .expect("lark serve doctor metadata")
                .checks
                .iter()
                .map(|check| check.name)
                .collect::<Vec<_>>(),
            vec!["feishu inbound transport", "feishu serve runtime"]
        );

        let discord_send =
            resolve_channel_operation_descriptor("discord-bot", CHANNEL_OPERATION_SEND_ID)
                .expect("discord send descriptor");
        assert_eq!(discord_send.operation.command, "discord-send");
        assert_eq!(discord_send.doctor, None);

        assert_eq!(
            resolve_channel_operation_descriptor("telegram", "unknown"),
            None
        );
    }

    #[test]
    fn resolve_channel_catalog_entry_returns_config_backed_metadata_for_alias_lookup() {
        let discord = resolve_channel_catalog_entry("discord-bot").expect("discord entry");
        let encoded = serde_json::to_value(&discord).expect("serialize discord entry");

        assert_eq!(discord.id, "discord");
        assert_eq!(discord.selection_order, 40);
        assert_eq!(discord.selection_label, "community server bot");
        assert!(discord.blurb.contains("outbound message surface"));
        assert_eq!(
            discord.implementation_status,
            ChannelCatalogImplementationStatus::ConfigBacked
        );
        assert_eq!(discord.transport, "discord_http_api");
        assert_eq!(discord.operations[0].command, "discord-send");
        assert_eq!(discord.operations[1].command, "discord-serve");
        assert_eq!(
            encoded
                .get("operations")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("availability"))
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                }),
            Some(vec!["implemented", "stub"])
        );
        assert_eq!(
            encoded
                .get("onboarding")
                .and_then(|onboarding| onboarding.get("strategy"))
                .and_then(serde_json::Value::as_str),
            Some("manual_config")
        );
        assert!(
            encoded.get("plugin_bridge_contract").is_none(),
            "config-backed catalog entries should omit plugin bridge contract when absent: {encoded}"
        );
    }

    #[test]
    fn resolve_channel_catalog_entry_exposes_onboarding_contracts() {
        let telegram = resolve_channel_catalog_entry("telegram").expect("telegram entry");
        let lark = resolve_channel_catalog_entry("lark").expect("lark entry");
        let discord = resolve_channel_catalog_entry("discord").expect("discord entry");
        let weixin = resolve_channel_catalog_entry("wechat").expect("weixin entry");
        let qqbot = resolve_channel_catalog_entry("qq").expect("qqbot entry");
        let onebot = resolve_channel_catalog_entry("onebot-v11").expect("onebot entry");

        assert_eq!(
            telegram.onboarding.strategy,
            ChannelOnboardingStrategy::ManualConfig
        );
        assert_eq!(telegram.onboarding.status_command, "loong doctor");
        assert_eq!(
            telegram.onboarding.repair_command,
            Some("loong doctor --fix")
        );
        assert!(telegram.onboarding.setup_hint.contains("loongclaw.toml"));

        assert_eq!(
            lark.onboarding.strategy,
            ChannelOnboardingStrategy::ManualConfig
        );
        assert_eq!(lark.onboarding.status_command, "loong doctor");

        assert_eq!(
            discord.onboarding.strategy,
            ChannelOnboardingStrategy::ManualConfig
        );
        assert_eq!(
            discord.onboarding.repair_command,
            Some("loong doctor --fix")
        );
        assert!(
            discord
                .onboarding
                .setup_hint
                .contains("outbound direct send is shipped")
        );

        assert_eq!(weixin.onboarding.strategy.as_str(), "plugin_bridge");
        assert_eq!(weixin.onboarding.status_command, "loong doctor");
        assert_eq!(weixin.onboarding.repair_command, None);
        assert!(weixin.onboarding.setup_hint.contains("ClawBot"));

        assert_eq!(qqbot.onboarding.strategy.as_str(), "plugin_bridge");
        assert_eq!(qqbot.onboarding.status_command, "loong doctor");
        assert_eq!(qqbot.onboarding.repair_command, None);
        assert!(qqbot.onboarding.setup_hint.contains("QQ Bot"));

        assert_eq!(onebot.onboarding.strategy.as_str(), "plugin_bridge");
        assert_eq!(onebot.onboarding.status_command, "loong doctor");
        assert_eq!(onebot.onboarding.repair_command, None);
        assert!(onebot.onboarding.setup_hint.contains("OneBot"));
    }

    #[test]
    fn resolve_channel_doctor_operation_spec_uses_registry_metadata() {
        let telegram =
            resolve_channel_doctor_operation_spec("telegram", "serve").expect("telegram spec");
        assert_eq!(
            telegram
                .checks
                .iter()
                .map(|check| (check.name, check.trigger))
                .collect::<Vec<_>>(),
            vec![
                (
                    "telegram channel",
                    ChannelDoctorCheckTrigger::OperationHealth,
                ),
                (
                    "telegram channel runtime",
                    ChannelDoctorCheckTrigger::ReadyRuntime,
                ),
            ]
        );

        let feishu_send =
            resolve_channel_doctor_operation_spec("feishu", "send").expect("feishu send spec");
        assert_eq!(
            feishu_send
                .checks
                .iter()
                .map(|check| (check.name, check.trigger))
                .collect::<Vec<_>>(),
            vec![("feishu channel", ChannelDoctorCheckTrigger::OperationHealth)]
        );

        let lark_serve =
            resolve_channel_doctor_operation_spec("lark", "serve").expect("lark serve spec");
        assert_eq!(
            lark_serve
                .checks
                .iter()
                .map(|check| (check.name, check.trigger))
                .collect::<Vec<_>>(),
            vec![
                (
                    "feishu inbound transport",
                    ChannelDoctorCheckTrigger::OperationHealth,
                ),
                (
                    "feishu serve runtime",
                    ChannelDoctorCheckTrigger::ReadyRuntime,
                ),
            ]
        );

        assert_eq!(
            resolve_channel_doctor_operation_spec("discord", "serve"),
            None
        );
        assert_eq!(
            resolve_channel_doctor_operation_spec("telegram", "send"),
            None
        );

        let weixin_send =
            resolve_channel_doctor_operation_spec("weixin", "send").expect("weixin send spec");
        let weixin_send_checks = weixin_send
            .checks
            .iter()
            .map(|check| (check.name, check.trigger))
            .collect::<Vec<_>>();
        assert_eq!(
            weixin_send_checks,
            vec![(
                "weixin bridge send contract",
                ChannelDoctorCheckTrigger::PluginBridgeHealth,
            )]
        );

        let qqbot_serve =
            resolve_channel_doctor_operation_spec("qqbot", "serve").expect("qqbot serve spec");
        let qqbot_serve_checks = qqbot_serve
            .checks
            .iter()
            .map(|check| (check.name, check.trigger))
            .collect::<Vec<_>>();
        assert_eq!(
            qqbot_serve_checks,
            vec![(
                "qqbot bridge serve contract",
                ChannelDoctorCheckTrigger::PluginBridgeHealth,
            )]
        );

        let onebot_serve =
            resolve_channel_doctor_operation_spec("onebot", "serve").expect("onebot serve spec");
        let onebot_serve_checks = onebot_serve
            .checks
            .iter()
            .map(|check| (check.name, check.trigger))
            .collect::<Vec<_>>();
        assert_eq!(
            onebot_serve_checks,
            vec![(
                "onebot bridge serve contract",
                ChannelDoctorCheckTrigger::PluginBridgeHealth,
            )]
        );
    }

    #[test]
    fn channel_catalog_keeps_lark_alias_under_feishu_surface() {
        let catalog = list_channel_catalog();
        let feishu = catalog
            .iter()
            .find(|entry| entry.id == "feishu")
            .expect("feishu catalog entry");
        let encoded = serde_json::to_value(feishu).expect("serialize feishu entry");

        assert_eq!(
            feishu.implementation_status,
            ChannelCatalogImplementationStatus::RuntimeBacked
        );
        assert_eq!(feishu.aliases, vec!["lark"]);
        assert_eq!(feishu.operations.len(), 2);
        assert_eq!(feishu.operations[0].command, "feishu-send");
        assert_eq!(feishu.operations[1].command, "feishu-serve");
        assert_eq!(
            encoded
                .get("operations")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("availability"))
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                }),
            Some(vec!["implemented", "implemented"])
        );
    }

    #[test]
    fn channel_catalog_includes_discord_and_slack_config_backed_surfaces() {
        let catalog = list_channel_catalog();
        let telegram = catalog
            .iter()
            .find(|entry| entry.id == "telegram")
            .expect("telegram catalog entry");
        let matrix = catalog
            .iter()
            .find(|entry| entry.id == "matrix")
            .expect("matrix catalog entry");
        let discord = catalog
            .iter()
            .find(|entry| entry.id == "discord")
            .expect("discord catalog entry");
        let slack = catalog
            .iter()
            .find(|entry| entry.id == "slack")
            .expect("slack catalog entry");
        let telegram_json = serde_json::to_value(telegram).expect("serialize telegram entry");
        let discord_json = serde_json::to_value(discord).expect("serialize discord entry");
        let slack_json = serde_json::to_value(slack).expect("serialize slack entry");

        assert_eq!(telegram.operations.len(), 2);
        assert_eq!(telegram.operations[0].command, "telegram-send");
        assert_eq!(telegram.operations[1].command, "telegram-serve");
        assert_eq!(
            matrix.implementation_status,
            ChannelCatalogImplementationStatus::RuntimeBacked
        );
        assert_eq!(matrix.transport, "matrix_client_server_sync");
        assert!(matrix.aliases.is_empty());
        assert_eq!(matrix.operations.len(), 2);
        assert_eq!(matrix.operations[0].command, "matrix-send");
        assert_eq!(matrix.operations[1].command, "matrix-serve");
        assert_eq!(
            discord.implementation_status,
            ChannelCatalogImplementationStatus::ConfigBacked
        );
        assert_eq!(discord.transport, "discord_http_api");
        assert_eq!(discord.aliases, vec!["discord-bot"]);
        assert_eq!(discord.selection_order, 40);
        assert_eq!(discord.selection_label, "community server bot");
        assert_eq!(discord.operations.len(), 2);
        assert_eq!(discord.operations[0].command, "discord-send");
        assert_eq!(discord.operations[1].command, "discord-serve");

        assert_eq!(
            slack.implementation_status,
            ChannelCatalogImplementationStatus::ConfigBacked
        );
        assert_eq!(slack.transport, "slack_web_api");
        assert_eq!(slack.aliases, vec!["slack-bot"]);
        assert_eq!(slack.operations.len(), 2);
        assert_eq!(slack.operations[0].command, "slack-send");
        assert_eq!(slack.operations[1].command, "slack-serve");
        assert_eq!(
            telegram_json
                .get("capabilities")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                }),
            Some(vec![
                "runtime_backed",
                "multi_account",
                "send",
                "serve",
                "runtime_tracking",
            ])
        );
        assert_eq!(
            serde_json::to_value(matrix)
                .expect("serialize matrix entry")
                .get("capabilities")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                }),
            Some(vec![
                "runtime_backed",
                "multi_account",
                "send",
                "serve",
                "runtime_tracking",
            ])
        );
        assert_eq!(
            discord_json
                .get("capabilities")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                }),
            Some(vec!["multi_account", "send"])
        );
        assert_eq!(
            slack_json
                .get("capabilities")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                }),
            Some(vec!["multi_account", "send"])
        );
    }

    #[test]
    fn channel_catalog_operations_expose_requirement_metadata() {
        let catalog = list_channel_catalog();
        let telegram = catalog
            .iter()
            .find(|entry| entry.id == "telegram")
            .expect("telegram catalog entry");
        let feishu = catalog
            .iter()
            .find(|entry| entry.id == "feishu")
            .expect("feishu catalog entry");
        let discord = catalog
            .iter()
            .find(|entry| entry.id == "discord")
            .expect("discord catalog entry");
        let line = catalog
            .iter()
            .find(|entry| entry.id == "line")
            .expect("line catalog entry");
        let google_chat = catalog
            .iter()
            .find(|entry| entry.id == "google-chat")
            .expect("google chat catalog entry");
        let twitch = catalog
            .iter()
            .find(|entry| entry.id == "twitch")
            .expect("twitch catalog entry");
        let teams = catalog
            .iter()
            .find(|entry| entry.id == "teams")
            .expect("teams catalog entry");
        let mattermost = catalog
            .iter()
            .find(|entry| entry.id == "mattermost")
            .expect("mattermost catalog entry");
        let nextcloud_talk = catalog
            .iter()
            .find(|entry| entry.id == "nextcloud-talk")
            .expect("nextcloud talk catalog entry");
        let synology_chat = catalog
            .iter()
            .find(|entry| entry.id == "synology-chat")
            .expect("synology chat catalog entry");
        let imessage = catalog
            .iter()
            .find(|entry| entry.id == "imessage")
            .expect("imessage catalog entry");
        let matrix = catalog
            .iter()
            .find(|entry| entry.id == "matrix")
            .expect("matrix catalog entry");

        assert_eq!(
            telegram.operations[0]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "bot_token"]
        );
        assert_eq!(
            telegram.operations[1]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec![
                "enabled",
                "bot_token",
                "allowed_chat_ids",
                "allowed_sender_ids",
                "require_mention"
            ]
        );
        assert_eq!(
            telegram.operations[0].requirements[1].default_env_var,
            Some("TELEGRAM_BOT_TOKEN")
        );
        assert_eq!(
            telegram.operations[0].requirements[1].env_pointer_paths,
            &[
                "telegram.bot_token_env",
                "telegram.accounts.<account>.bot_token_env",
            ]
        );

        assert_eq!(
            feishu.operations[0]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "app_id", "app_secret"]
        );
        assert_eq!(
            feishu.operations[1]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec![
                "enabled",
                "app_id",
                "app_secret",
                "mode",
                "allowed_chat_ids",
                "allowed_sender_ids",
                "verification_token",
                "encrypt_key",
            ]
        );
        assert_eq!(
            feishu.operations[1].requirements[6].default_env_var,
            Some("FEISHU_VERIFICATION_TOKEN")
        );
        assert_eq!(
            feishu.operations[1].requirements[7].default_env_var,
            Some("FEISHU_ENCRYPT_KEY")
        );
        assert_eq!(
            matrix.operations[1]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec![
                "enabled",
                "access_token",
                "base_url",
                "allowed_room_ids",
                "allowed_sender_ids",
                "require_mention",
                "user_id",
            ]
        );

        assert_eq!(
            discord.operations[0]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "bot_token"]
        );
        assert_eq!(
            discord.operations[1]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec![
                "enabled",
                "bot_token",
                "application_id",
                "allowed_guild_ids"
            ]
        );
        assert_eq!(
            discord.operations[1].requirements[2].default_env_var,
            Some("DISCORD_APPLICATION_ID")
        );

        assert_eq!(
            line.operations[0]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "channel_access_token"]
        );
        assert_eq!(
            line.operations[1]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "channel_access_token", "channel_secret"]
        );
        assert_eq!(
            line.operations[0].requirements[1].default_env_var,
            Some("LINE_CHANNEL_ACCESS_TOKEN")
        );
        assert_eq!(
            line.operations[1].requirements[2].default_env_var,
            Some("LINE_CHANNEL_SECRET")
        );

        let dingtalk = catalog
            .iter()
            .find(|entry| entry.id == "dingtalk")
            .expect("dingtalk catalog entry");

        assert_eq!(
            dingtalk.operations[0]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "webhook_url"]
        );
        assert_eq!(
            dingtalk.operations[1]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "webhook_url", "secret"]
        );
        assert_eq!(
            dingtalk.operations[0].requirements[1].default_env_var,
            Some("DINGTALK_WEBHOOK_URL")
        );
        assert_eq!(
            dingtalk.operations[1].requirements[2].default_env_var,
            Some("DINGTALK_SECRET")
        );

        assert_eq!(
            google_chat.operations[0]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "webhook_url"]
        );
        assert_eq!(
            google_chat.operations[1]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "webhook_url"]
        );
        assert_eq!(
            google_chat.operations[0].requirements[1].default_env_var,
            Some("GOOGLE_CHAT_WEBHOOK_URL")
        );

        assert_eq!(
            teams.operations[0]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "webhook_url"]
        );
        assert_eq!(
            teams.operations[1]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec![
                "enabled",
                "app_id",
                "app_password",
                "tenant_id",
                "allowed_conversation_ids",
            ]
        );
        assert_eq!(
            teams.operations[0].requirements[1].default_env_var,
            Some("TEAMS_WEBHOOK_URL")
        );
        assert_eq!(
            teams.operations[1].requirements[1].default_env_var,
            Some("TEAMS_APP_ID")
        );
        assert_eq!(
            teams.operations[1].requirements[2].default_env_var,
            Some("TEAMS_APP_PASSWORD")
        );
        assert_eq!(
            teams.operations[1].requirements[3].default_env_var,
            Some("TEAMS_TENANT_ID")
        );

        assert_eq!(
            mattermost.operations[0]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "server_url", "bot_token"]
        );
        assert_eq!(
            mattermost.operations[1]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "server_url", "bot_token", "allowed_channel_ids"]
        );
        assert_eq!(
            mattermost.operations[0].requirements[1].default_env_var,
            Some("MATTERMOST_SERVER_URL")
        );
        assert_eq!(
            mattermost.operations[0].requirements[2].default_env_var,
            Some("MATTERMOST_BOT_TOKEN")
        );

        assert_eq!(
            nextcloud_talk.operations[0]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "server_url", "shared_secret"]
        );
        assert_eq!(
            nextcloud_talk.operations[1]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "server_url", "shared_secret"]
        );
        assert_eq!(
            nextcloud_talk.operations[0].requirements[1].default_env_var,
            Some("NEXTCLOUD_TALK_SERVER_URL")
        );
        assert_eq!(
            nextcloud_talk.operations[0].requirements[2].default_env_var,
            Some("NEXTCLOUD_TALK_SHARED_SECRET")
        );

        assert_eq!(
            twitch.operations[0]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "access_token"]
        );
        assert_eq!(
            twitch.operations[1]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "access_token", "channel_names"]
        );
        assert_eq!(
            twitch.operations[0].requirements[1].default_env_var,
            Some("TWITCH_ACCESS_TOKEN")
        );
        assert_eq!(
            twitch.operations[0].requirements[1].env_pointer_paths,
            &[
                "twitch.access_token_env",
                "twitch.accounts.<account>.access_token_env",
            ]
        );

        assert_eq!(
            synology_chat.operations[0]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "incoming_url"]
        );
        assert_eq!(
            synology_chat.operations[1]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "token", "incoming_url", "allowed_user_ids"]
        );
        assert_eq!(
            synology_chat.operations[0].requirements[1].default_env_var,
            Some("SYNOLOGY_CHAT_INCOMING_URL")
        );
        assert_eq!(
            synology_chat.operations[1].requirements[1].default_env_var,
            Some("SYNOLOGY_CHAT_TOKEN")
        );

        assert_eq!(
            imessage.operations[0]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "bridge_url", "bridge_token"]
        );
        assert_eq!(
            imessage.operations[1]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "bridge_url", "bridge_token", "allowed_chat_ids"]
        );
        assert_eq!(
            imessage.operations[0].requirements[1].default_env_var,
            Some("IMESSAGE_BRIDGE_URL")
        );
        assert_eq!(
            imessage.operations[0].requirements[2].default_env_var,
            Some("IMESSAGE_BRIDGE_TOKEN")
        );
    }

    #[test]
    fn channel_catalog_operations_expose_supported_target_kinds() {
        let catalog = list_channel_catalog();
        let telegram = catalog
            .iter()
            .find(|entry| entry.id == "telegram")
            .expect("telegram catalog entry");
        let feishu = catalog
            .iter()
            .find(|entry| entry.id == "feishu")
            .expect("feishu catalog entry");
        let discord = catalog
            .iter()
            .find(|entry| entry.id == "discord")
            .expect("discord catalog entry");
        let teams = catalog
            .iter()
            .find(|entry| entry.id == "teams")
            .expect("teams catalog entry");
        let email = catalog
            .iter()
            .find(|entry| entry.id == "email")
            .expect("email catalog entry");
        let nextcloud_talk = catalog
            .iter()
            .find(|entry| entry.id == "nextcloud-talk")
            .expect("nextcloud talk catalog entry");
        let signal = catalog
            .iter()
            .find(|entry| entry.id == "signal")
            .expect("signal catalog entry");
        let twitch = catalog
            .iter()
            .find(|entry| entry.id == "twitch")
            .expect("twitch catalog entry");
        let synology_chat = catalog
            .iter()
            .find(|entry| entry.id == "synology-chat")
            .expect("synology chat catalog entry");
        let irc = catalog
            .iter()
            .find(|entry| entry.id == "irc")
            .expect("irc catalog entry");
        let imessage = catalog
            .iter()
            .find(|entry| entry.id == "imessage")
            .expect("imessage catalog entry");

        assert_eq!(
            telegram.operations[0].supported_target_kinds,
            &[ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(
            telegram.operations[1].supported_target_kinds,
            &[ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(
            feishu.operations[0].supported_target_kinds,
            &[
                ChannelCatalogTargetKind::ReceiveId,
                ChannelCatalogTargetKind::MessageReply,
            ]
        );
        assert_eq!(
            feishu.operations[1].supported_target_kinds,
            &[ChannelCatalogTargetKind::MessageReply]
        );
        assert_eq!(
            discord.operations[0].supported_target_kinds,
            &[ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(
            discord.operations[1].supported_target_kinds,
            &[ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(
            teams.operations[0].supported_target_kinds,
            &[ChannelCatalogTargetKind::Endpoint]
        );
        assert_eq!(
            teams.operations[1].supported_target_kinds,
            &[ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(
            email.operations[0].supported_target_kinds,
            &[ChannelCatalogTargetKind::Address]
        );
        assert_eq!(
            email.operations[1].supported_target_kinds,
            &[ChannelCatalogTargetKind::Address]
        );
        assert_eq!(
            nextcloud_talk.operations[0].supported_target_kinds,
            &[ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(
            nextcloud_talk.operations[1].supported_target_kinds,
            &[ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(
            signal.operations[0].supported_target_kinds,
            &[ChannelCatalogTargetKind::Address]
        );
        assert_eq!(
            signal.operations[1].supported_target_kinds,
            &[ChannelCatalogTargetKind::Address]
        );
        assert_eq!(
            twitch.operations[0].supported_target_kinds,
            &[ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(
            twitch.operations[1].supported_target_kinds,
            &[ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(
            synology_chat.operations[0].supported_target_kinds,
            &[ChannelCatalogTargetKind::Address]
        );
        assert_eq!(
            synology_chat.operations[1].supported_target_kinds,
            &[ChannelCatalogTargetKind::Address]
        );
        assert_eq!(
            irc.operations[0].supported_target_kinds,
            &[ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(
            irc.operations[1].supported_target_kinds,
            &[ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(
            imessage.operations[0].supported_target_kinds,
            &[ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(
            imessage.operations[1].supported_target_kinds,
            &[ChannelCatalogTargetKind::Conversation]
        );
    }

    #[test]
    fn channel_catalog_operation_exposes_default_target_kind_from_metadata() {
        let telegram =
            resolve_channel_catalog_operation("telegram", "send").expect("telegram send operation");
        let feishu =
            resolve_channel_catalog_operation("feishu", "send").expect("feishu send operation");
        let nextcloud_talk = resolve_channel_catalog_operation("nextcloud-talk", "send")
            .expect("nextcloud talk send operation");
        let webhook =
            resolve_channel_catalog_operation("webhook", "send").expect("webhook send operation");
        let signal =
            resolve_channel_catalog_operation("signal", "send").expect("signal send operation");
        let email =
            resolve_channel_catalog_operation("email", "send").expect("email send operation");
        let teams =
            resolve_channel_catalog_operation("teams", "send").expect("teams send operation");
        let synology_chat = resolve_channel_catalog_operation("synology-chat", "send")
            .expect("synology chat send operation");
        let imessage =
            resolve_channel_catalog_operation("imessage", "send").expect("imessage send operation");

        assert_eq!(
            telegram.default_target_kind(),
            Some(ChannelCatalogTargetKind::Conversation)
        );
        assert!(telegram.supports_target_kind(ChannelCatalogTargetKind::Conversation));
        assert_eq!(
            feishu.default_target_kind(),
            Some(ChannelCatalogTargetKind::ReceiveId)
        );
        assert!(feishu.supports_target_kind(ChannelCatalogTargetKind::ReceiveId));
        assert!(feishu.supports_target_kind(ChannelCatalogTargetKind::MessageReply));
        assert!(!feishu.supports_target_kind(ChannelCatalogTargetKind::Conversation));
        assert_eq!(
            nextcloud_talk.default_target_kind(),
            Some(ChannelCatalogTargetKind::Conversation)
        );
        assert!(nextcloud_talk.supports_target_kind(ChannelCatalogTargetKind::Conversation));
        assert_eq!(
            webhook.default_target_kind(),
            Some(ChannelCatalogTargetKind::Endpoint)
        );
        assert!(webhook.supports_target_kind(ChannelCatalogTargetKind::Endpoint));
        assert!(!webhook.supports_target_kind(ChannelCatalogTargetKind::Conversation));
        assert_eq!(
            signal.default_target_kind(),
            Some(ChannelCatalogTargetKind::Address)
        );
        assert!(signal.supports_target_kind(ChannelCatalogTargetKind::Address));
        assert_eq!(
            email.default_target_kind(),
            Some(ChannelCatalogTargetKind::Address)
        );
        assert!(email.supports_target_kind(ChannelCatalogTargetKind::Address));
        assert_eq!(
            teams.default_target_kind(),
            Some(ChannelCatalogTargetKind::Endpoint)
        );
        assert!(teams.supports_target_kind(ChannelCatalogTargetKind::Endpoint));
        assert_eq!(
            synology_chat.default_target_kind(),
            Some(ChannelCatalogTargetKind::Address)
        );
        assert!(synology_chat.supports_target_kind(ChannelCatalogTargetKind::Address));
        assert_eq!(
            imessage.default_target_kind(),
            Some(ChannelCatalogTargetKind::Conversation)
        );
        assert!(imessage.supports_target_kind(ChannelCatalogTargetKind::Conversation));
    }

    #[test]
    fn multi_target_channel_catalog_operations_declare_explicit_default_target_kind() {
        let catalog = list_channel_catalog();
        for entry in catalog {
            for operation in entry.operations {
                if operation.supported_target_kinds.len() < 2 {
                    continue;
                }

                let default_target_kind = operation.default_target_kind.unwrap_or_else(|| {
                    panic!(
                        "{}:{} must declare an explicit default_target_kind",
                        entry.id, operation.id
                    )
                });

                assert!(
                    operation.supports_target_kind(default_target_kind),
                    "{}:{} default target kind must be supported by the operation",
                    entry.id,
                    operation.id
                );
            }
        }
    }

    #[test]
    fn channel_catalog_surfaces_expose_union_of_supported_target_kinds() {
        let catalog = list_channel_catalog();
        let telegram = catalog
            .iter()
            .find(|entry| entry.id == "telegram")
            .expect("telegram catalog entry");
        let feishu = catalog
            .iter()
            .find(|entry| entry.id == "feishu")
            .expect("feishu catalog entry");
        let discord = catalog
            .iter()
            .find(|entry| entry.id == "discord")
            .expect("discord catalog entry");
        let teams = catalog
            .iter()
            .find(|entry| entry.id == "teams")
            .expect("teams catalog entry");
        let email = catalog
            .iter()
            .find(|entry| entry.id == "email")
            .expect("email catalog entry");
        let nextcloud_talk = catalog
            .iter()
            .find(|entry| entry.id == "nextcloud-talk")
            .expect("nextcloud talk catalog entry");
        let signal = catalog
            .iter()
            .find(|entry| entry.id == "signal")
            .expect("signal catalog entry");
        let synology_chat = catalog
            .iter()
            .find(|entry| entry.id == "synology-chat")
            .expect("synology chat catalog entry");
        let irc = catalog
            .iter()
            .find(|entry| entry.id == "irc")
            .expect("irc catalog entry");
        let imessage = catalog
            .iter()
            .find(|entry| entry.id == "imessage")
            .expect("imessage catalog entry");

        assert_eq!(
            telegram.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(
            feishu.supported_target_kinds,
            vec![
                ChannelCatalogTargetKind::ReceiveId,
                ChannelCatalogTargetKind::MessageReply,
            ]
        );
        assert_eq!(
            discord.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(
            teams.supported_target_kinds,
            vec![
                ChannelCatalogTargetKind::Endpoint,
                ChannelCatalogTargetKind::Conversation,
            ]
        );
        assert_eq!(
            email.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Address]
        );
        assert_eq!(
            nextcloud_talk.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(
            signal.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Address]
        );
        assert_eq!(
            synology_chat.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Address]
        );
        assert_eq!(
            irc.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(
            imessage.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Conversation]
        );
    }

    #[test]
    fn channel_catalog_includes_irc_config_backed_surface() {
        let catalog = list_channel_catalog();
        let irc = catalog
            .iter()
            .find(|entry| entry.id == "irc")
            .expect("irc catalog entry");

        assert_eq!(
            irc.implementation_status,
            ChannelCatalogImplementationStatus::ConfigBacked
        );
        assert_eq!(irc.selection_order, 170);
        assert_eq!(irc.transport, "irc_socket");
        assert_eq!(irc.aliases, Vec::<&str>::new());
        assert_eq!(irc.operations[0].command, "irc-send");
        assert_eq!(irc.operations[1].command, "irc-serve");
        assert_eq!(
            irc.operations[0]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "server", "nickname"]
        );
    }

    #[test]
    fn catalog_only_channel_entries_include_stub_surfaces_for_default_config() {
        let config = LoongClawConfig::default();
        let snapshots = channel_status_snapshots(&config);
        let catalog_only = catalog_only_channel_entries(&snapshots);
        let webchat = catalog_only
            .iter()
            .find(|entry| entry.id == "webchat")
            .expect("webchat catalog entry");

        assert_eq!(
            catalog_only
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec!["zalo", "zalo-personal", "webchat"]
        );
        assert!(!catalog_only.iter().any(|entry| entry.id == "discord"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "slack"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "line"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "dingtalk"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "whatsapp"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "email"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "webhook"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "google-chat"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "signal"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "irc"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "nostr"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "twitch"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "tlon"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "teams"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "mattermost"));
        assert!(
            !catalog_only
                .iter()
                .any(|entry| entry.id == "nextcloud-talk")
        );
        assert!(!catalog_only.iter().any(|entry| entry.id == "synology-chat"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "imessage"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "weixin"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "qqbot"));
        assert!(!catalog_only.iter().any(|entry| entry.id == "onebot"));
        assert_eq!(webchat.operations[1].command, "webchat-serve");
    }

    #[test]
    fn channel_inventory_combines_runtime_and_catalog_surfaces() {
        let config = LoongClawConfig::default();
        let inventory = channel_inventory(&config);

        assert_eq!(
            inventory
                .channels
                .iter()
                .map(|snapshot| snapshot.id)
                .collect::<Vec<_>>(),
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
        assert_eq!(
            inventory
                .catalog_only_channels
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec!["zalo", "zalo-personal", "webchat"]
        );
        assert_eq!(
            inventory
                .channel_catalog
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
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
                "zalo",
                "zalo-personal",
                "webchat",
            ]
        );
    }

    #[test]
    fn channel_catalog_includes_dingtalk_and_google_chat_config_backed_webhook_surfaces() {
        let catalog = list_channel_catalog();
        let dingtalk = catalog
            .iter()
            .find(|entry| entry.id == "dingtalk")
            .expect("dingtalk catalog entry");
        let google_chat = catalog
            .iter()
            .find(|entry| entry.id == "google-chat")
            .expect("google chat catalog entry");

        assert_eq!(
            dingtalk.implementation_status,
            ChannelCatalogImplementationStatus::ConfigBacked
        );
        assert_eq!(dingtalk.selection_order, 80);
        assert_eq!(dingtalk.aliases, vec!["ding", "ding-bot"]);
        assert_eq!(dingtalk.transport, "dingtalk_custom_robot_webhook");
        assert_eq!(
            dingtalk.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Endpoint]
        );
        assert_eq!(dingtalk.operations[0].command, "dingtalk-send");
        assert_eq!(dingtalk.operations[1].command, "dingtalk-serve");
        assert_eq!(
            dingtalk.operations[0].availability,
            ChannelCatalogOperationAvailability::Implemented
        );
        assert_eq!(
            dingtalk.operations[1].availability,
            ChannelCatalogOperationAvailability::Stub
        );

        assert_eq!(
            google_chat.implementation_status,
            ChannelCatalogImplementationStatus::ConfigBacked
        );
        assert_eq!(google_chat.selection_order, 120);
        assert_eq!(google_chat.aliases, vec!["gchat", "googlechat"]);
        assert_eq!(google_chat.transport, "google_chat_incoming_webhook");
        assert_eq!(
            google_chat.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Endpoint]
        );
        assert_eq!(google_chat.operations[0].command, "google-chat-send");
        assert_eq!(google_chat.operations[1].command, "google-chat-serve");
        assert_eq!(
            google_chat.operations[0].availability,
            ChannelCatalogOperationAvailability::Implemented
        );
        assert_eq!(
            google_chat.operations[1].availability,
            ChannelCatalogOperationAvailability::Stub
        );
    }

    #[test]
    fn channel_catalog_includes_email_config_backed_smtp_surface() {
        let catalog = list_channel_catalog();
        let email = catalog
            .iter()
            .find(|entry| entry.id == "email")
            .expect("email catalog entry");

        assert_eq!(
            email.implementation_status,
            ChannelCatalogImplementationStatus::ConfigBacked
        );
        assert_eq!(email.selection_order, 100);
        assert_eq!(email.aliases, vec!["smtp", "imap"]);
        assert_eq!(email.transport, "smtp_imap");
        assert_eq!(
            email.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Address]
        );
        assert_eq!(email.operations[0].command, "email-send");
        assert_eq!(email.operations[1].command, "email-serve");
        assert_eq!(
            email.operations[0].availability,
            ChannelCatalogOperationAvailability::Implemented
        );
        assert_eq!(
            email.operations[1].availability,
            ChannelCatalogOperationAvailability::Stub
        );
    }

    #[test]
    fn channel_catalog_includes_nextcloud_talk_config_backed_bot_surface() {
        let catalog = list_channel_catalog();
        let nextcloud_talk = catalog
            .iter()
            .find(|entry| entry.id == "nextcloud-talk")
            .expect("nextcloud talk catalog entry");
        let synology_chat = catalog
            .iter()
            .find(|entry| entry.id == "synology-chat")
            .expect("synology chat catalog entry");

        assert_eq!(
            nextcloud_talk.implementation_status,
            ChannelCatalogImplementationStatus::ConfigBacked
        );
        assert_eq!(nextcloud_talk.selection_order, 160);
        assert_eq!(nextcloud_talk.aliases, vec!["nextcloud", "nextcloudtalk"]);
        assert_eq!(nextcloud_talk.transport, "nextcloud_talk_bot_api");
        assert_eq!(
            nextcloud_talk.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(nextcloud_talk.operations[0].command, "nextcloud-talk-send");
        assert_eq!(nextcloud_talk.operations[1].command, "nextcloud-talk-serve");
        assert_eq!(
            nextcloud_talk.operations[0].availability,
            ChannelCatalogOperationAvailability::Implemented
        );
        assert_eq!(
            nextcloud_talk.operations[1].availability,
            ChannelCatalogOperationAvailability::Stub
        );

        assert_eq!(
            synology_chat.implementation_status,
            ChannelCatalogImplementationStatus::ConfigBacked
        );
        assert_eq!(synology_chat.selection_order, 165);
        assert_eq!(synology_chat.aliases, vec!["synologychat", "synochat"]);
        assert_eq!(
            synology_chat.transport,
            "synology_chat_outgoing_incoming_webhooks"
        );
        assert_eq!(
            synology_chat.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Address]
        );
        assert_eq!(synology_chat.operations[0].command, "synology-chat-send");
        assert_eq!(synology_chat.operations[1].command, "synology-chat-serve");
        assert_eq!(
            synology_chat.operations[0].availability,
            ChannelCatalogOperationAvailability::Implemented
        );
        assert_eq!(
            synology_chat.operations[1].availability,
            ChannelCatalogOperationAvailability::Stub
        );
    }

    #[test]
    fn channel_status_snapshots_redact_webhook_channel_status_urls() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "dingtalk": {
                "enabled": true,
                "webhook_url": "https://oapi.dingtalk.com/robot/send?access_token=secret-token"
            },
            "google_chat": {
                "enabled": true,
                "webhook_url": "https://chat.googleapis.com/v1/spaces/AAAA/messages?key=secret-key&token=secret-token"
            },
            "teams": {
                "enabled": true,
                "webhook_url": "https://outlook.office.com/webhook/abc123/IncomingWebhook/demo?tenant=secret-tenant&auth=secret-auth"
            },
            "synology_chat": {
                "enabled": true,
                "incoming_url": "https://chat.example.test/webapi/entry.cgi?api=SYNO.Chat.External&method=incoming&version=2&token=secret-token"
            }
        }))
        .expect("deserialize webhook channel config");

        let snapshots = channel_status_snapshots(&config);
        let dingtalk = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "dingtalk")
            .expect("dingtalk snapshot");
        let google_chat = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "google-chat")
            .expect("google chat snapshot");
        let teams = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "teams")
            .expect("teams snapshot");
        let synology_chat = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "synology-chat")
            .expect("synology chat snapshot");

        assert_eq!(
            dingtalk.api_base_url.as_deref(),
            Some("https://oapi.dingtalk.com/robot/send")
        );
        assert_eq!(
            google_chat.api_base_url.as_deref(),
            Some("https://chat.googleapis.com/v1/spaces/AAAA/messages")
        );
        assert_eq!(
            teams.api_base_url.as_deref(),
            Some("https://outlook.office.com/")
        );
        assert_eq!(
            synology_chat.api_base_url.as_deref(),
            Some("https://chat.example.test/webapi/entry.cgi")
        );
    }

    #[test]
    fn channel_status_snapshots_redact_generic_webhook_path_segments() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "webhook": {
                "enabled": true,
                "endpoint_url": "https://hooks.example.test/customer/secret-token/send?trace=secret"
            }
        }))
        .expect("deserialize generic webhook config");

        let webhook = channel_status_snapshots(&config)
            .into_iter()
            .find(|snapshot| snapshot.id == "webhook")
            .expect("generic webhook snapshot");

        assert_eq!(
            webhook.api_base_url.as_deref(),
            Some("https://hooks.example.test/")
        );
    }

    #[test]
    fn email_channel_status_snapshot_reports_smtp_readiness() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "email": {
                "enabled": true,
                "smtp_host": "smtps://smtp.example.test:465?auth=plain",
                "smtp_username": "mailer@example.test",
                "smtp_password": "top-secret",
                "from_address": "LoongClaw <ops@example.test>"
            }
        }))
        .expect("deserialize email channel config");

        let snapshots = channel_status_snapshots(&config);
        let email = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "email")
            .expect("email snapshot");
        let send_operation = email
            .operation(CHANNEL_OPERATION_SEND_ID)
            .expect("email send operation");
        let serve_operation = email
            .operation(CHANNEL_OPERATION_SERVE_ID)
            .expect("email serve operation");

        assert_eq!(
            email.api_base_url.as_deref(),
            Some("smtps://smtp.example.test:465")
        );
        assert!(
            email
                .notes
                .iter()
                .any(|note| note == "from_address=LoongClaw <ops@example.test>")
        );
        assert_eq!(send_operation.health, ChannelOperationHealth::Ready);
        assert_eq!(serve_operation.health, ChannelOperationHealth::Unsupported);
    }

    #[test]
    fn webhook_status_snapshot_rejects_invalid_auth_header_values() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "webhook": {
                "enabled": true,
                "endpoint_url": "https://hooks.example.test/send",
                "auth_token": "token-123",
                "auth_token_prefix": "Bearer\n",
                "signing_secret": "signing-secret"
            }
        }))
        .expect("deserialize generic webhook config");

        let webhook = channel_status_snapshots(&config)
            .into_iter()
            .find(|snapshot| snapshot.id == "webhook")
            .expect("generic webhook snapshot");
        let send = webhook
            .operation(CHANNEL_OPERATION_SEND_ID)
            .expect("webhook send operation");
        let serve = webhook
            .operation(CHANNEL_OPERATION_SERVE_ID)
            .expect("webhook serve operation");

        assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
        assert!(
            send.issues
                .iter()
                .any(|issue| issue.contains("auth header value is invalid")),
            "unexpected issues: {:?}",
            send.issues
        );
        assert_eq!(serve.health, ChannelOperationHealth::Ready);
    }

    #[test]
    fn webhook_status_snapshot_requires_signing_secret_for_serve() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "webhook": {
                "enabled": true,
                "endpoint_url": "https://hooks.example.test/send"
            }
        }))
        .expect("deserialize generic webhook config");

        let webhook = channel_status_snapshots(&config)
            .into_iter()
            .find(|snapshot| snapshot.id == "webhook")
            .expect("generic webhook snapshot");
        let serve = webhook
            .operation(CHANNEL_OPERATION_SERVE_ID)
            .expect("webhook serve operation");

        assert_eq!(serve.health, ChannelOperationHealth::Misconfigured);
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("signing_secret")),
            "unexpected serve issues: {:?}",
            serve.issues
        );
    }

    #[test]
    fn wecom_status_rejects_non_websocket_endpoint_schemes() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "wecom": {
                "enabled": true,
                "bot_id": "wx-bot-id",
                "secret": "wx-secret",
                "allowed_conversation_ids": ["conv-1"],
                "websocket_url": "https://wecom.example.test/aibot"
            }
        }))
        .expect("deserialize wecom config");

        let snapshots = channel_status_snapshots(&config);
        let wecom = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "wecom")
            .expect("wecom snapshot");
        let send = wecom.operation("send").expect("wecom send operation");
        let serve = wecom.operation("serve").expect("wecom serve operation");

        assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
        assert_eq!(serve.health, ChannelOperationHealth::Misconfigured);
        assert!(
            send.issues
                .iter()
                .any(|issue| issue.contains("websocket_url must use ws or wss")),
            "send issues should reject non-websocket schemes: {:?}",
            send.issues
        );
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("websocket_url must use ws or wss")),
            "serve issues should reject non-websocket schemes: {:?}",
            serve.issues
        );
    }

    #[test]
    fn channel_inventory_exposes_grouped_channel_surfaces() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.remove("TELEGRAM_BOT_TOKEN");

        let config = LoongClawConfig::default();
        let inventory = channel_inventory(&config);

        assert_eq!(
            inventory
                .channel_surfaces
                .iter()
                .map(|surface| surface.catalog.id)
                .collect::<Vec<_>>(),
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
                "zalo",
                "zalo-personal",
                "webchat",
            ]
        );

        let telegram = inventory
            .channel_surfaces
            .iter()
            .find(|surface| surface.catalog.id == "telegram")
            .expect("telegram surface");
        assert_eq!(telegram.configured_accounts.len(), 1);
        assert_eq!(
            telegram.default_configured_account_id.as_deref(),
            Some("default")
        );
        assert_eq!(telegram.configured_accounts[0].id, "telegram");

        let discord = inventory
            .channel_surfaces
            .iter()
            .find(|surface| surface.catalog.id == "discord")
            .expect("discord surface");
        assert_eq!(
            discord.catalog.implementation_status,
            ChannelCatalogImplementationStatus::ConfigBacked
        );
        assert_eq!(discord.configured_accounts.len(), 1);
        assert_eq!(
            discord.default_configured_account_id.as_deref(),
            Some("default")
        );
        assert_eq!(discord.configured_accounts[0].id, "discord");
        assert!(
            inventory
                .channel_access_policies
                .iter()
                .any(|policy| policy.channel_id == "telegram"
                    && policy.configured_account_id == "default"
                    && policy.conversation_config_key == "allowed_chat_ids"
                    && policy.sender_config_key == "allowed_sender_ids"),
            "channel inventory should expose structured access policy for telegram"
        );
        let discord_encoded =
            serde_json::to_value(discord).expect("serialize discord channel surface");
        assert!(
            discord_encoded.get("plugin_bridge_discovery").is_none(),
            "channel surface output should omit plugin bridge discovery when absent: {discord_encoded}"
        );
        assert!(
            discord_encoded
                .get("catalog")
                .and_then(serde_json::Value::as_object)
                .map(|catalog| !catalog.contains_key("plugin_bridge_contract"))
                .unwrap_or(false),
            "channel surface catalog output should omit plugin bridge contract when absent: {discord_encoded}"
        );

        let weixin = inventory
            .channel_surfaces
            .iter()
            .find(|surface| surface.catalog.id == "weixin")
            .expect("weixin surface");
        assert_eq!(
            weixin.catalog.implementation_status,
            ChannelCatalogImplementationStatus::PluginBacked
        );
        assert_eq!(weixin.configured_accounts.len(), 1);
        assert_eq!(
            weixin.default_configured_account_id.as_deref(),
            Some("default")
        );
        assert_eq!(weixin.configured_accounts[0].id, "weixin");
        assert_eq!(
            weixin.configured_accounts[0].configured_account_id,
            "default"
        );

        let qqbot = inventory
            .channel_surfaces
            .iter()
            .find(|surface| surface.catalog.id == "qqbot")
            .expect("qqbot surface");
        assert_eq!(
            qqbot.catalog.implementation_status,
            ChannelCatalogImplementationStatus::PluginBacked
        );
        assert_eq!(qqbot.configured_accounts.len(), 1);
        assert_eq!(
            qqbot.default_configured_account_id.as_deref(),
            Some("default")
        );
        assert_eq!(qqbot.configured_accounts[0].id, "qqbot");
        assert_eq!(
            qqbot.configured_accounts[0].configured_account_id,
            "default"
        );

        let onebot = inventory
            .channel_surfaces
            .iter()
            .find(|surface| surface.catalog.id == "onebot")
            .expect("onebot surface");
        assert_eq!(
            onebot.catalog.implementation_status,
            ChannelCatalogImplementationStatus::PluginBacked
        );
        assert_eq!(onebot.configured_accounts.len(), 1);
        assert_eq!(
            onebot.default_configured_account_id.as_deref(),
            Some("default")
        );
        assert_eq!(onebot.configured_accounts[0].id, "onebot");
        assert_eq!(
            onebot.configured_accounts[0].configured_account_id,
            "default"
        );

        let line = inventory
            .channel_surfaces
            .iter()
            .find(|surface| surface.catalog.id == "line")
            .expect("line surface");
        assert_eq!(
            line.catalog.implementation_status,
            ChannelCatalogImplementationStatus::RuntimeBacked
        );
        assert_eq!(line.configured_accounts.len(), 1);
        assert_eq!(
            line.default_configured_account_id.as_deref(),
            Some("default")
        );
        assert_eq!(line.configured_accounts[0].id, "line");

        let wecom = inventory
            .channel_surfaces
            .iter()
            .find(|surface| surface.catalog.id == "wecom")
            .expect("wecom surface");
        assert_eq!(
            wecom.catalog.implementation_status,
            ChannelCatalogImplementationStatus::RuntimeBacked
        );
        assert_eq!(wecom.configured_accounts.len(), 1);
        assert_eq!(
            wecom.default_configured_account_id.as_deref(),
            Some("default")
        );
        assert_eq!(wecom.configured_accounts[0].id, "wecom");

        let mattermost = inventory
            .channel_surfaces
            .iter()
            .find(|surface| surface.catalog.id == "mattermost")
            .expect("mattermost surface");
        assert_eq!(
            mattermost.catalog.implementation_status,
            ChannelCatalogImplementationStatus::ConfigBacked
        );
        assert_eq!(mattermost.configured_accounts.len(), 1);
        assert_eq!(
            mattermost.default_configured_account_id.as_deref(),
            Some("default")
        );
        assert_eq!(mattermost.configured_accounts[0].id, "mattermost");

        let teams = inventory
            .channel_surfaces
            .iter()
            .find(|surface| surface.catalog.id == "teams")
            .expect("teams surface");
        assert_eq!(
            teams.catalog.implementation_status,
            ChannelCatalogImplementationStatus::ConfigBacked
        );
        assert_eq!(teams.configured_accounts.len(), 1);
        assert_eq!(
            teams.default_configured_account_id.as_deref(),
            Some("default")
        );
        assert_eq!(teams.configured_accounts[0].id, "teams");

        let synology_chat = inventory
            .channel_surfaces
            .iter()
            .find(|surface| surface.catalog.id == "synology-chat")
            .expect("synology chat surface");
        assert_eq!(
            synology_chat.catalog.implementation_status,
            ChannelCatalogImplementationStatus::ConfigBacked
        );
        assert_eq!(synology_chat.configured_accounts.len(), 1);
        assert_eq!(
            synology_chat.default_configured_account_id.as_deref(),
            Some("default")
        );
        assert_eq!(synology_chat.configured_accounts[0].id, "synology-chat");

        let imessage = inventory
            .channel_surfaces
            .iter()
            .find(|surface| surface.catalog.id == "imessage")
            .expect("imessage surface");
        assert_eq!(
            imessage.catalog.implementation_status,
            ChannelCatalogImplementationStatus::ConfigBacked
        );
        assert_eq!(imessage.configured_accounts.len(), 1);
        assert_eq!(
            imessage.default_configured_account_id.as_deref(),
            Some("default")
        );
        assert_eq!(imessage.configured_accounts[0].id, "imessage");

        let webchat = inventory
            .channel_surfaces
            .iter()
            .find(|surface| surface.catalog.id == "webchat")
            .expect("webchat surface");
        assert_eq!(
            webchat.catalog.implementation_status,
            ChannelCatalogImplementationStatus::Stub
        );
        assert_eq!(webchat.catalog.aliases, vec!["browser-chat", "web-ui"]);
        assert!(webchat.configured_accounts.is_empty());
    }

    #[test]
    fn catalog_only_channel_entries_skip_platforms_that_already_have_status_snapshots() {
        let catalog = vec![
            ChannelCatalogEntry {
                id: "telegram",
                label: "Telegram",
                selection_order: 10,
                selection_label: "personal and group chat bot",
                blurb: "Shipped Telegram Bot API surface with direct send and reply-loop runtime support.",
                implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
                capabilities: vec![
                    ChannelCapability::RuntimeBacked,
                    ChannelCapability::Send,
                    ChannelCapability::Serve,
                    ChannelCapability::RuntimeTracking,
                ],
                aliases: vec![],
                transport: "telegram_bot_api_polling",
                onboarding: TELEGRAM_ONBOARDING_DESCRIPTOR,
                plugin_bridge_contract: None,
                supported_target_kinds: vec![ChannelCatalogTargetKind::Conversation],
                operations: vec![
                    ChannelCatalogOperation {
                        id: "send",
                        label: "direct send",
                        command: "telegram-send",
                        availability: ChannelCatalogOperationAvailability::Implemented,
                        tracks_runtime: false,
                        requirements: &[],
                        default_target_kind: None,
                        supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
                    },
                    ChannelCatalogOperation {
                        id: "serve",
                        label: "reply loop",
                        command: "telegram-serve",
                        availability: ChannelCatalogOperationAvailability::Implemented,
                        tracks_runtime: true,
                        requirements: &[],
                        default_target_kind: None,
                        supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
                    },
                ],
            },
            ChannelCatalogEntry {
                id: "discord",
                label: "Discord",
                selection_order: 40,
                selection_label: "community server bot",
                blurb: "Shipped Discord outbound message surface with config-backed direct sends; inbound gateway/runtime support remains planned.",
                implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
                capabilities: vec![ChannelCapability::MultiAccount, ChannelCapability::Send],
                aliases: vec![],
                transport: "discord_http_api",
                onboarding: DISCORD_ONBOARDING_DESCRIPTOR,
                plugin_bridge_contract: None,
                supported_target_kinds: vec![ChannelCatalogTargetKind::Conversation],
                operations: vec![
                    ChannelCatalogOperation {
                        id: "send",
                        label: "direct send",
                        command: "discord-send",
                        availability: ChannelCatalogOperationAvailability::Implemented,
                        tracks_runtime: false,
                        requirements: &[],
                        default_target_kind: None,
                        supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
                    },
                    ChannelCatalogOperation {
                        id: "serve",
                        label: "reply loop",
                        command: "discord-serve",
                        availability: ChannelCatalogOperationAvailability::Stub,
                        tracks_runtime: false,
                        requirements: &[],
                        default_target_kind: None,
                        supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
                    },
                ],
            },
        ];
        let snapshots = vec![ChannelStatusSnapshot {
            id: "telegram",
            configured_account_id: "default".to_owned(),
            configured_account_label: "default".to_owned(),
            is_default_account: true,
            default_account_source: ChannelDefaultAccountSelectionSource::Fallback,
            label: "Telegram",
            aliases: vec![],
            transport: "telegram_bot_api_polling",
            compiled: true,
            enabled: false,
            api_base_url: Some("https://api.telegram.org".to_owned()),
            notes: vec![],
            reserved_runtime_fields: Vec::new(),
            operations: vec![ChannelOperationStatus {
                id: "serve",
                label: "reply loop",
                command: "telegram-serve",
                health: ChannelOperationHealth::Disabled,
                detail: "disabled".to_owned(),
                issues: vec![],
                runtime: None,
            }],
        }];

        let catalog_only = catalog_only_channel_entries_from(&catalog, &snapshots);

        assert_eq!(catalog_only.len(), 1);
        assert_eq!(catalog_only[0].id, "discord");
        assert_eq!(
            catalog_only[0].implementation_status,
            ChannelCatalogImplementationStatus::ConfigBacked
        );
        assert_eq!(catalog_only[0].operations[0].command, "discord-send");
    }

    #[test]
    fn shipped_channel_registry_descriptors_define_snapshot_builders() {
        for descriptor in sorted_channel_registry_descriptors() {
            let requires_snapshot_builder = matches!(
                descriptor.implementation_status,
                ChannelCatalogImplementationStatus::RuntimeBacked
                    | ChannelCatalogImplementationStatus::ConfigBacked
                    | ChannelCatalogImplementationStatus::PluginBacked
            );
            if !requires_snapshot_builder {
                continue;
            }

            assert!(
                descriptor.snapshot_builder.is_some(),
                "built-in shipped channel `{}` must define a snapshot builder",
                descriptor.id
            );
        }
    }

    #[test]
    fn channel_registry_stays_physically_sorted_by_selection_order() {
        for descriptor_pair in CHANNEL_REGISTRY.windows(2) {
            let first_descriptor = &descriptor_pair[0];
            let second_descriptor = &descriptor_pair[1];

            assert!(
                first_descriptor.selection_order <= second_descriptor.selection_order,
                "channel registry order drifted: {} ({}) appears before {} ({})",
                first_descriptor.id,
                first_descriptor.selection_order,
                second_descriptor.id,
                second_descriptor.selection_order
            );
        }
    }

    #[test]
    fn telegram_status_reports_ready_when_token_and_allowlist_are_configured() {
        let mut config = LoongClawConfig::default();
        config.telegram.enabled = true;
        config.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
            "123456:token".to_owned(),
        ));
        config.telegram.allowed_chat_ids = vec![123];

        let snapshots = channel_status_snapshots(&config);
        let telegram = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "telegram")
            .expect("telegram snapshot");
        let serve = telegram
            .operation("serve")
            .expect("telegram serve operation");

        assert_eq!(serve.health, ChannelOperationHealth::Ready);
        assert!(serve.is_ready());
        assert_eq!(
            telegram.api_base_url.as_deref(),
            Some("https://api.telegram.org")
        );
        assert!(!serve.runtime.as_ref().expect("telegram runtime").running);
    }

    #[test]
    fn telegram_status_splits_direct_send_and_reply_loop_readiness() {
        let mut config = LoongClawConfig::default();
        config.telegram.enabled = true;
        config.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
            "123456:token".to_owned(),
        ));

        let snapshots = channel_status_snapshots(&config);
        let telegram = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "telegram")
            .expect("telegram snapshot");
        let send = telegram.operation("send").expect("telegram send operation");
        let serve = telegram
            .operation("serve")
            .expect("telegram serve operation");

        assert_eq!(send.health, ChannelOperationHealth::Ready);
        assert_eq!(serve.health, ChannelOperationHealth::Misconfigured);
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("allowed_chat_ids")),
            "serve issues should mention allowlist"
        );
        assert!(send.runtime.is_none());
        assert_eq!(
            serve
                .runtime
                .as_ref()
                .expect("telegram runtime")
                .active_runs,
            0
        );
    }

    #[test]
    fn feishu_status_splits_direct_send_and_webhook_readiness() {
        let mut config = LoongClawConfig::default();
        config.feishu.enabled = true;
        config.feishu.mode = Some(FeishuChannelServeMode::Webhook);
        config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline("app-id".to_owned()));
        config.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
            "app-secret".to_owned(),
        ));

        let snapshots = channel_status_snapshots(&config);
        let feishu = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "feishu")
            .expect("feishu snapshot");
        let send = feishu.operation("send").expect("feishu send operation");
        let serve = feishu.operation("serve").expect("feishu serve operation");

        assert_eq!(send.health, ChannelOperationHealth::Ready);
        assert_eq!(serve.health, ChannelOperationHealth::Misconfigured);
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("allowed_chat_ids")),
            "serve issues should mention allowlist"
        );
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("verification_token")),
            "serve issues should mention verification token"
        );
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("encrypt_key")),
            "serve issues should mention encrypt key"
        );
        assert!(send.runtime.is_none());
        assert_eq!(
            serve.runtime.as_ref().expect("serve runtime").active_runs,
            0
        );
    }

    #[test]
    fn matrix_status_requires_user_id_when_ignoring_self_messages() {
        let mut config = LoongClawConfig::default();
        config.matrix.enabled = true;
        config.matrix.access_token = Some(loongclaw_contracts::SecretRef::Inline(
            "matrix-token".to_owned(),
        ));
        config.matrix.base_url = Some("https://matrix.example.org".to_owned());
        config.matrix.allowed_room_ids = vec!["!ops:example.org".to_owned()];
        config.matrix.ignore_self_messages = true;

        let snapshots = channel_status_snapshots(&config);
        let matrix = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "matrix")
            .expect("matrix snapshot");
        let send = matrix.operation("send").expect("matrix send operation");
        let serve = matrix.operation("serve").expect("matrix serve operation");

        assert_eq!(send.health, ChannelOperationHealth::Ready);
        assert_eq!(serve.health, ChannelOperationHealth::Misconfigured);
        assert!(
            serve.issues.iter().any(|issue| issue.contains("user_id")),
            "serve issues should require user_id when ignore_self_messages is enabled"
        );
    }

    #[test]
    fn discord_status_splits_config_backed_send_and_stub_serve() {
        let mut env = crate::test_support::ScopedEnv::new();
        env.remove(DISCORD_BOT_TOKEN_ENV);
        let mut config = LoongClawConfig::default();
        config.discord.enabled = true;
        config.discord.bot_token_env = None;

        let snapshots = channel_status_snapshots(&config);
        let discord = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "discord")
            .expect("discord snapshot");
        let send = discord.operation("send").expect("discord send operation");
        let serve = discord.operation("serve").expect("discord serve operation");

        assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
        assert!(
            send.issues.iter().any(|issue| issue.contains("bot_token")),
            "send issues should mention the missing discord bot token"
        );
        assert_eq!(serve.health, ChannelOperationHealth::Unsupported);
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("not implemented")),
            "serve issues should explain that discord serve is not implemented"
        );
        assert_eq!(
            discord.api_base_url.as_deref(),
            Some("https://discord.com/api/v10")
        );
        assert!(send.runtime.is_none());
        assert!(serve.runtime.is_none());
    }

    #[test]
    fn discord_status_rejects_non_http_api_base_url() {
        let mut config = LoongClawConfig::default();
        config.discord.enabled = true;
        config.discord.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
            "discord-token".to_owned(),
        ));
        config.discord.api_base_url = Some("file:///tmp/discord-api".to_owned());

        let snapshots = channel_status_snapshots(&config);
        let discord = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "discord")
            .expect("discord snapshot");
        let send = discord.operation("send").expect("discord send operation");

        assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
        assert!(
            send.issues
                .iter()
                .any(|issue| issue.contains("requires http or https")),
            "send issues should reject non-http discord api base urls"
        );
    }

    #[test]
    fn discord_status_rejects_api_base_url_with_query_string() {
        let mut config = LoongClawConfig::default();
        config.discord.enabled = true;
        config.discord.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
            "discord-token".to_owned(),
        ));
        config.discord.api_base_url = Some("https://discord.com/api/v10?debug=1".to_owned());

        let snapshots = channel_status_snapshots(&config);
        let discord = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "discord")
            .expect("discord snapshot");
        let send = discord.operation("send").expect("discord send operation");

        assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
        assert!(
            send.issues
                .iter()
                .any(|issue| issue.contains("must not include a query string")),
            "send issues should reject discord api base urls with query strings: {send:#?}"
        );
    }

    #[test]
    fn discord_status_notes_reserved_future_runtime_fields_when_present() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "discord": {
                "enabled": true,
                "bot_token": "discord-token",
                "application_id": "discord-application-id",
                "allowed_guild_ids": ["guild-a", "guild-b"]
            }
        }))
        .expect("deserialize discord config");

        let snapshots = channel_status_snapshots(&config);
        let discord = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "discord")
            .expect("discord snapshot");

        assert!(
            discord
                .notes
                .iter()
                .any(|note| note == "reserved_runtime_field=application_id"),
            "discord status notes should preserve configured future runtime fields: {discord:#?}"
        );
        assert!(
            discord
                .notes
                .iter()
                .any(|note| note == "reserved_runtime_field=allowed_guild_ids:2"),
            "discord status notes should record allowed_guild_ids count when reserved runtime fields are configured: {discord:#?}"
        );
        assert_eq!(
            discord.reserved_runtime_fields,
            vec![
                "application_id".to_owned(),
                "allowed_guild_ids:2".to_owned()
            ],
            "discord status snapshots should expose structured reserved runtime fields alongside operator notes: {discord:#?}"
        );
    }

    #[test]
    fn slack_status_reports_ready_send_and_stub_serve() {
        let mut config = LoongClawConfig::default();
        config.slack.enabled = true;
        config.slack.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
            "xoxb-test-token".to_owned(),
        ));

        let snapshots = channel_status_snapshots(&config);
        let slack = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "slack")
            .expect("slack snapshot");
        let send = slack.operation("send").expect("slack send operation");
        let serve = slack.operation("serve").expect("slack serve operation");

        assert_eq!(send.health, ChannelOperationHealth::Ready);
        assert_eq!(serve.health, ChannelOperationHealth::Unsupported);
        assert_eq!(slack.api_base_url.as_deref(), Some("https://slack.com/api"));
        assert!(send.runtime.is_none());
        assert!(serve.runtime.is_none());
    }

    #[test]
    fn slack_status_rejects_api_base_url_with_fragment() {
        let mut config = LoongClawConfig::default();
        config.slack.enabled = true;
        config.slack.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
            "xoxb-test-token".to_owned(),
        ));
        config.slack.api_base_url = Some("https://slack.com/api#fragment".to_owned());

        let snapshots = channel_status_snapshots(&config);
        let slack = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "slack")
            .expect("slack snapshot");
        let send = slack.operation("send").expect("slack send operation");

        assert_eq!(send.health, ChannelOperationHealth::Misconfigured);
        assert!(
            send.issues
                .iter()
                .any(|issue| issue.contains("must not include a fragment")),
            "send issues should reject slack api base urls with fragments: {send:#?}"
        );
    }

    #[test]
    fn line_status_reports_ready_send_and_stub_serve() {
        let mut config = LoongClawConfig::default();
        config.line.enabled = true;
        config.line.channel_access_token = Some(loongclaw_contracts::SecretRef::Inline(
            "line-access-token".to_owned(),
        ));

        let snapshots = channel_status_snapshots(&config);
        let line = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "line")
            .expect("line snapshot");
        let send = line.operation("send").expect("line send operation");
        let serve = line.operation("serve").expect("line serve operation");

        assert_eq!(send.health, ChannelOperationHealth::Ready);
        assert_eq!(serve.health, ChannelOperationHealth::Misconfigured);
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue == "channel_secret is missing"),
            "unexpected serve issues: {:?}",
            serve.issues
        );
        assert_eq!(
            line.api_base_url.as_deref(),
            Some("https://api.line.me/v2/bot")
        );
        assert!(send.runtime.is_none());
        assert!(serve.runtime.is_some());
    }

    #[test]
    fn irc_status_reports_ready_send_and_planned_serve() {
        let mut config = LoongClawConfig::default();
        config.irc.enabled = true;
        config.irc.server = Some("ircs://irc.example.test:6697".to_owned());
        config.irc.nickname = Some("loongclaw".to_owned());
        config.irc.username = Some("loongclaw".to_owned());
        config.irc.channel_names = vec!["#ops".to_owned()];

        let snapshots = channel_status_snapshots(&config);
        let irc = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "irc")
            .expect("irc snapshot");
        let send = irc.operation("send").expect("irc send operation");
        let serve = irc.operation("serve").expect("irc serve operation");

        assert_eq!(send.health, ChannelOperationHealth::Ready);
        assert_eq!(serve.health, ChannelOperationHealth::Unsupported);
        assert_eq!(
            irc.api_base_url.as_deref(),
            Some("ircs://irc.example.test:6697")
        );
        assert!(
            irc.notes.iter().any(|note| note == "nickname=loongclaw"),
            "irc notes should include the resolved nickname"
        );
        assert!(
            irc.notes.iter().any(|note| note == "server_transport=ircs"),
            "irc notes should include the parsed transport"
        );
        assert!(
            irc.notes.iter().any(|note| note == "channel_names=#ops"),
            "irc notes should include configured channel names"
        );
        assert!(send.runtime.is_none());
        assert!(serve.runtime.is_none());
    }

    #[test]
    fn irc_status_formats_ipv6_server_endpoint_with_brackets() {
        let mut config = LoongClawConfig::default();
        config.irc.enabled = true;
        config.irc.server = Some("ircs://[2001:db8::42]:6697".to_owned());
        config.irc.nickname = Some("loongclaw".to_owned());

        let snapshots = channel_status_snapshots(&config);
        let irc = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "irc")
            .expect("irc snapshot");

        assert_eq!(
            irc.api_base_url.as_deref(),
            Some("ircs://[2001:db8::42]:6697")
        );
    }

    #[test]
    fn whatsapp_status_reports_ready_send_when_access_token_and_phone_number_id_are_configured() {
        let mut config = LoongClawConfig::default();
        config.whatsapp.enabled = true;
        config.whatsapp.access_token = Some(loongclaw_contracts::SecretRef::Inline(
            "whatsapp-access-token".to_owned(),
        ));
        config.whatsapp.phone_number_id = Some("1234567890".to_owned());

        let snapshots = channel_status_snapshots(&config);
        let whatsapp = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "whatsapp")
            .expect("whatsapp snapshot");
        let send = whatsapp.operation("send").expect("whatsapp send operation");
        let serve = whatsapp
            .operation("serve")
            .expect("whatsapp serve operation");

        assert_eq!(send.health, ChannelOperationHealth::Ready);
        assert_eq!(serve.health, ChannelOperationHealth::Misconfigured);
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("verify_token")),
            "serve issues should mention the missing verify token"
        );
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("app_secret")),
            "serve issues should mention the missing app secret"
        );
        assert_eq!(
            whatsapp.api_base_url.as_deref(),
            Some("https://graph.facebook.com/v25.0")
        );
        assert!(
            whatsapp
                .notes
                .iter()
                .any(|note| note == "phone_number_id=1234567890"),
            "status notes should expose the resolved phone number id"
        );
        assert!(send.runtime.is_none());
        assert!(serve.runtime.is_some());
    }

    #[test]
    fn mattermost_status_reports_ready_send_and_stub_serve() {
        let mut config = LoongClawConfig::default();
        config.mattermost.enabled = true;
        config.mattermost.server_url = Some("https://mattermost.example.test".to_owned());
        config.mattermost.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
            "mattermost-bot-token".to_owned(),
        ));

        let snapshots = channel_status_snapshots(&config);
        let mattermost = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "mattermost")
            .expect("mattermost snapshot");
        let send = mattermost
            .operation("send")
            .expect("mattermost send operation");
        let serve = mattermost
            .operation("serve")
            .expect("mattermost serve operation");

        assert_eq!(send.health, ChannelOperationHealth::Ready);
        assert_eq!(serve.health, ChannelOperationHealth::Unsupported);
        assert_eq!(
            mattermost.api_base_url.as_deref(),
            Some("https://mattermost.example.test")
        );
        assert!(send.runtime.is_none());
        assert!(serve.runtime.is_none());
    }

    #[test]
    fn feishu_websocket_status_uses_websocket_requirements() {
        let mut config = LoongClawConfig::default();
        config.feishu.enabled = true;
        config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline("app-id".to_owned()));
        config.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
            "app-secret".to_owned(),
        ));
        config.feishu.mode = Some(crate::config::FeishuChannelServeMode::Websocket);
        config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];

        let snapshots = channel_status_snapshots(&config);
        let feishu = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "feishu")
            .expect("feishu snapshot");
        let serve = feishu.operation("serve").expect("feishu serve operation");

        assert_eq!(serve.health, ChannelOperationHealth::Ready);
        assert!(
            serve
                .issues
                .iter()
                .all(|issue| !issue.contains("verification_token")),
            "websocket mode must not require a webhook verification token"
        );
        assert!(
            serve
                .issues
                .iter()
                .all(|issue| !issue.contains("encrypt_key")),
            "websocket mode must not require a webhook encrypt key"
        );
        assert!(
            feishu.notes.iter().any(|note| note == "mode=websocket"),
            "status notes should surface the configured feishu serve mode"
        );
        assert!(
            feishu
                .notes
                .iter()
                .all(|note| !note.starts_with("webhook_bind=")),
            "websocket mode notes should not imply a webhook bind address is active"
        );
        assert!(
            feishu
                .notes
                .iter()
                .all(|note| !note.starts_with("webhook_path=")),
            "websocket mode notes should not imply a webhook callback path is active"
        );
    }

    #[test]
    fn channel_status_snapshots_merge_runtime_activity_for_serve_operations() {
        let mut config = LoongClawConfig::default();
        config.feishu.enabled = true;
        config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline("app-id".to_owned()));
        config.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
            "app-secret".to_owned(),
        ));
        config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
        config.feishu.verification_token =
            Some(loongclaw_contracts::SecretRef::Inline("token".to_owned()));
        config.feishu.encrypt_key =
            Some(loongclaw_contracts::SecretRef::Inline("encrypt".to_owned()));

        let runtime_dir = temp_runtime_dir("registry-runtime");
        let now = now_ms();
        state::write_runtime_state_for_test(
            runtime_dir.as_path(),
            ChannelPlatform::Feishu,
            "serve",
            true,
            true,
            2,
            Some(now.saturating_sub(1_000)),
            Some(now.saturating_sub(500)),
            Some(4242),
        )
        .expect("write runtime state");

        let snapshots = channel_status_snapshots_with_now(&config, runtime_dir.as_path(), now);
        let feishu = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "feishu")
            .expect("feishu snapshot");
        let serve = feishu.operation("serve").expect("feishu serve operation");
        let runtime = serve.runtime.as_ref().expect("runtime info");

        assert!(runtime.running);
        assert!(!runtime.stale);
        assert!(runtime.busy);
        assert_eq!(runtime.active_runs, 2);
        assert_eq!(runtime.pid, Some(4242));
    }

    #[test]
    fn channel_status_snapshots_report_resolved_account_identity_in_notes() {
        let mut config = LoongClawConfig::default();
        config.telegram.enabled = true;
        config.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
            "123456:token".to_owned(),
        ));
        config.telegram.allowed_chat_ids = vec![123];

        let snapshots = channel_status_snapshots(&config);
        let telegram = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "telegram")
            .expect("telegram snapshot");

        assert!(
            telegram
                .notes
                .iter()
                .any(|note| note.contains("account_id=bot_123456")),
            "telegram notes should expose the resolved account id"
        );
    }

    #[test]
    fn channel_status_snapshots_report_telegram_acp_bootstrap_mcp_servers_in_notes() {
        let mut config = LoongClawConfig::default();
        config.telegram.enabled = true;
        config.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
            "123456:token".to_owned(),
        ));
        config.telegram.allowed_chat_ids = vec![123];
        config.telegram.acp.bootstrap_mcp_servers = vec!["filesystem".to_owned()];
        config.telegram.acp.working_directory = Some(" /workspace/telegram ".to_owned());

        let snapshots = channel_status_snapshots(&config);
        let telegram = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "telegram")
            .expect("telegram snapshot");

        assert!(
            telegram
                .notes
                .iter()
                .any(|note| note == "acp_bootstrap_mcp_servers=filesystem"),
            "telegram notes should expose configured ACP bootstrap MCP servers"
        );
        assert!(
            telegram
                .notes
                .iter()
                .any(|note| note == "acp_working_directory=/workspace/telegram"),
            "telegram notes should expose configured ACP working directory"
        );
    }

    #[test]
    fn channel_status_snapshots_report_feishu_acp_bootstrap_mcp_servers_in_notes() {
        let mut config = LoongClawConfig::default();
        config.feishu.enabled = true;
        config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline(
            "cli_a1b2c3".to_owned(),
        ));
        config.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
            "app-secret".to_owned(),
        ));
        config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
        config.feishu.verification_token =
            Some(loongclaw_contracts::SecretRef::Inline("token".to_owned()));
        config.feishu.encrypt_key =
            Some(loongclaw_contracts::SecretRef::Inline("encrypt".to_owned()));
        config.feishu.acp.bootstrap_mcp_servers = vec!["search".to_owned()];
        config.feishu.acp.working_directory = Some("/workspace/feishu".to_owned());

        let snapshots = channel_status_snapshots(&config);
        let feishu = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "feishu")
            .expect("feishu snapshot");

        assert!(
            feishu
                .notes
                .iter()
                .any(|note| note == "acp_bootstrap_mcp_servers=search"),
            "feishu notes should expose configured ACP bootstrap MCP servers"
        );
        assert!(
            feishu
                .notes
                .iter()
                .any(|note| note == "acp_working_directory=/workspace/feishu"),
            "feishu notes should expose configured ACP working directory"
        );
    }

    #[test]
    fn channel_status_snapshots_attach_account_identity_to_runtime_view() {
        let mut config = LoongClawConfig::default();
        config.feishu.enabled = true;
        config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline(
            "cli_a1b2c3".to_owned(),
        ));
        config.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
            "app-secret".to_owned(),
        ));
        config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
        config.feishu.verification_token =
            Some(loongclaw_contracts::SecretRef::Inline("token".to_owned()));
        config.feishu.encrypt_key =
            Some(loongclaw_contracts::SecretRef::Inline("encrypt".to_owned()));

        let runtime_dir = temp_runtime_dir("registry-account-runtime");
        let now = now_ms();
        state::write_runtime_state_for_test_with_account_and_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Feishu,
            "serve",
            "feishu_cli_a1b2c3",
            4242,
            true,
            true,
            2,
            Some(now.saturating_sub(1_000)),
            Some(now.saturating_sub(500)),
            Some(4242),
        )
        .expect("write runtime state");

        let snapshots = channel_status_snapshots_with_now(&config, runtime_dir.as_path(), now);
        let feishu = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "feishu")
            .expect("feishu snapshot");
        let serve = feishu.operation("serve").expect("feishu serve operation");
        let runtime = serve.runtime.as_ref().expect("runtime info");

        assert_eq!(runtime.account_id.as_deref(), Some("feishu_cli_a1b2c3"));
        assert_eq!(runtime.account_label.as_deref(), Some("feishu:cli_a1b2c3"));
    }

    #[test]
    fn channel_status_snapshots_preserve_runtime_instance_counts() {
        let mut config = LoongClawConfig::default();
        config.telegram.enabled = true;
        config.telegram.bot_token = Some(loongclaw_contracts::SecretRef::Inline(
            "123456:token".to_owned(),
        ));
        config.telegram.allowed_chat_ids = vec![123];

        let runtime_dir = temp_runtime_dir("registry-duplicate-runtime");
        let now = now_ms();
        state::write_runtime_state_for_test_with_account_and_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            "bot_123456",
            1001,
            true,
            true,
            1,
            Some(now.saturating_sub(300)),
            Some(now.saturating_sub(100)),
            Some(1001),
        )
        .expect("write first runtime state");
        state::write_runtime_state_for_test_with_account_and_pid(
            runtime_dir.as_path(),
            ChannelPlatform::Telegram,
            "serve",
            "bot_123456",
            1002,
            true,
            false,
            0,
            Some(now.saturating_sub(200)),
            Some(now.saturating_sub(50)),
            Some(1002),
        )
        .expect("write second runtime state");

        let snapshots = channel_status_snapshots_with_now(&config, runtime_dir.as_path(), now);
        let telegram = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "telegram")
            .expect("telegram snapshot");
        let serve = telegram
            .operation("serve")
            .expect("telegram serve operation");
        let runtime = serve.runtime.as_ref().expect("runtime info");

        assert_eq!(runtime.instance_count, 2);
        assert_eq!(runtime.running_instances, 2);
        assert_eq!(runtime.stale_instances, 0);
    }

    #[test]
    fn multi_account_registry_emits_one_snapshot_per_configured_account() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "default_account": "Work Bot",
                "allowed_chat_ids": [1001],
                "accounts": {
                    "Work Bot": {
                        "account_id": "Ops-Bot",
                        "bot_token": "123456:token-work",
                        "allowed_chat_ids": [2002]
                    },
                    "Personal": {
                        "bot_token": "654321:token-personal",
                        "allowed_chat_ids": [3003]
                    }
                }
            }
        }))
        .expect("deserialize multi-account config");

        let telegram = channel_status_snapshots(&config)
            .into_iter()
            .filter(|snapshot| snapshot.id == "telegram")
            .collect::<Vec<_>>();

        assert_eq!(telegram.len(), 2);
        assert_eq!(telegram[0].configured_account_id, "personal");
        assert_eq!(telegram[1].configured_account_id, "work-bot");
        assert!(
            telegram[1]
                .notes
                .iter()
                .any(|note| note == "configured_account_id=work-bot")
        );
        assert!(
            telegram[1]
                .notes
                .iter()
                .any(|note| note == "account_id=ops-bot")
        );
    }

    #[test]
    fn multi_account_registry_marks_default_configured_account() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "default_account": "Work Bot",
                "allowed_chat_ids": [1001],
                "accounts": {
                    "Work Bot": {
                        "account_id": "Ops-Bot",
                        "bot_token": "123456:token-work",
                        "allowed_chat_ids": [2002]
                    },
                    "Personal": {
                        "bot_token": "654321:token-personal",
                        "allowed_chat_ids": [3003]
                    }
                }
            }
        }))
        .expect("deserialize multi-account config");

        let telegram = channel_status_snapshots(&config)
            .into_iter()
            .filter(|snapshot| snapshot.id == "telegram")
            .collect::<Vec<_>>();
        let encoded = serde_json::to_value(&telegram).expect("serialize telegram snapshots");

        assert!(
            telegram[1]
                .notes
                .iter()
                .any(|note| note == "default_account=true")
        );
        assert_eq!(
            encoded[0]
                .get("is_default_account")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            encoded[1]
                .get("is_default_account")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            encoded[1]
                .get("default_account_source")
                .and_then(serde_json::Value::as_str),
            Some("explicit_default")
        );
    }

    #[test]
    fn multi_account_registry_records_fallback_default_account_source() {
        let config: LoongClawConfig = serde_json::from_value(serde_json::json!({
            "telegram": {
                "enabled": true,
                "accounts": {
                    "Work": {
                        "bot_token": "123456:token-work",
                        "allowed_chat_ids": [2002]
                    },
                    "Alerts": {
                        "bot_token": "654321:token-alerts",
                        "allowed_chat_ids": [3003]
                    }
                }
            }
        }))
        .expect("deserialize multi-account config");

        let telegram = channel_status_snapshots(&config)
            .into_iter()
            .filter(|snapshot| snapshot.id == "telegram")
            .collect::<Vec<_>>();

        assert!(telegram[0].is_default_account);
        assert_eq!(
            telegram[0].default_account_source,
            ChannelDefaultAccountSelectionSource::Fallback
        );
        assert!(
            telegram[0]
                .notes
                .iter()
                .any(|note| note == "default_account_source=fallback")
        );
    }

    fn temp_runtime_dir(suffix: &str) -> std::path::PathBuf {
        let unique = format!(
            "loongclaw-channel-registry-{suffix}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }
}

#[cfg(test)]
mod trust_boundary_tests;
