use std::path::Path;

use serde::Serialize;

mod config_backed;
mod descriptors;
mod planned;
mod runtime_backed;
mod tlon;
mod twitch;

use crate::config::{
    ChannelDefaultAccountSelectionSource, DINGTALK_SECRET_ENV, DINGTALK_WEBHOOK_URL_ENV,
    DISCORD_BOT_TOKEN_ENV, FEISHU_APP_ID_ENV, FEISHU_APP_SECRET_ENV, FEISHU_ENCRYPT_KEY_ENV,
    FEISHU_VERIFICATION_TOKEN_ENV, GOOGLE_CHAT_WEBHOOK_URL_ENV, IMESSAGE_BRIDGE_TOKEN_ENV,
    IMESSAGE_BRIDGE_URL_ENV, IRC_NICKNAME_ENV, IRC_SERVER_ENV, LINE_CHANNEL_ACCESS_TOKEN_ENV,
    LINE_CHANNEL_SECRET_ENV, LoongConfig, MATRIX_ACCESS_TOKEN_ENV, MATTERMOST_BOT_TOKEN_ENV,
    MATTERMOST_SERVER_URL_ENV, NEXTCLOUD_TALK_SERVER_URL_ENV, NEXTCLOUD_TALK_SHARED_SECRET_ENV,
    NOSTR_PRIVATE_KEY_ENV, NOSTR_RELAY_URLS_ENV, QQBOT_APP_ID_ENV, QQBOT_CLIENT_SECRET_ENV,
    ResolvedTlonChannelConfig, ResolvedTwitchChannelConfig, SIGNAL_ACCOUNT_ENV,
    SIGNAL_SERVICE_URL_ENV, SLACK_BOT_TOKEN_ENV, SYNOLOGY_CHAT_INCOMING_URL_ENV,
    SYNOLOGY_CHAT_TOKEN_ENV, TEAMS_APP_ID_ENV, TEAMS_APP_PASSWORD_ENV, TEAMS_TENANT_ID_ENV,
    TEAMS_WEBHOOK_URL_ENV, TELEGRAM_BOT_TOKEN_ENV, TWITCH_ACCESS_TOKEN_ENV,
    WEBHOOK_ENDPOINT_URL_ENV, WEBHOOK_SIGNING_SECRET_ENV, WECOM_BOT_ID_ENV, WECOM_SECRET_ENV,
    WHATSAPP_ACCESS_TOKEN_ENV, WHATSAPP_APP_SECRET_ENV, WHATSAPP_PHONE_NUMBER_ID_ENV,
    WHATSAPP_VERIFY_TOKEN_ENV, WebhookPayloadFormat,
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
#[allow(unused_imports)]
pub use bridge::{
    ONEBOT_CATALOG_COMMAND_FAMILY_DESCRIPTOR, WEIXIN_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
    WHATSAPP_PERSONAL_CATALOG_COMMAND_FAMILY_DESCRIPTOR,
};

#[path = "registry_bridge.rs"]
mod bridge;
#[path = "registry_nostr_impl.rs"]
mod nostr_impl;

#[path = "registry_plugin_bridge.rs"]
mod plugin_bridge;

#[path = "registry_surface.rs"]
mod surface_support;

#[path = "registry_status.rs"]
mod status_support;

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
    MATRIX_RUNTIME_COMMAND_DESCRIPTOR, QQBOT_RUNTIME_COMMAND_DESCRIPTOR,
    TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR, WEBHOOK_RUNTIME_COMMAND_DESCRIPTOR,
    WECOM_RUNTIME_COMMAND_DESCRIPTOR, WHATSAPP_RUNTIME_COMMAND_DESCRIPTOR,
    catalog_only_channel_entries, list_channel_catalog, normalize_channel_catalog_id,
    normalize_channel_platform, resolve_channel_catalog_command_family_descriptor,
    resolve_channel_catalog_entry, resolve_channel_catalog_operation,
    resolve_channel_command_family_descriptor, resolve_channel_doctor_operation_spec,
    resolve_channel_onboarding_descriptor, resolve_channel_operation_descriptor,
    resolve_channel_runtime_command_descriptor,
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
use status_support::{
    apply_runtime_attention, attach_runtime, disabled_operation, misconfigured_operation,
    ready_operation, unsupported_operation,
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
    fn(&ChannelRegistryDescriptor, &LoongConfig, &Path, u64) -> Vec<ChannelStatusSnapshot>;

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
    label: "bridge send",
    command: "channels send telegram",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: false,
    requirements: TELEGRAM_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const TELEGRAM_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "bridge serve",
    command: "channels serve telegram",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
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
const TELEGRAM_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::PluginBridge,
    setup_hint: "install and configure a Telegram bridge plugin that declares setup.surface=channel plus telegram bot credentials, allowed chat ids, and optional mention gating before serving the managed bridge surface",
    status_command: "loong doctor",
    repair_command: None,
};

const FEISHU_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "bridge send",
    command: "channels send feishu",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
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
    label: "bridge serve",
    command: "channels serve feishu",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
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
const FEISHU_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::PluginBridge,
    setup_hint: "install and configure a Feishu/Lark bridge plugin that declares setup.surface=channel plus app credentials, allowed chat ids, serve mode, and any webhook verification inputs before serving the managed bridge surface",
    status_command: "loong doctor",
    repair_command: None,
};

pub const QQBOT_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "qqbot",
        default_send_target_kind: ChannelCatalogTargetKind::Conversation,
        send: QQBOT_SEND_OPERATION,
        serve: QQBOT_SERVE_OPERATION,
    };

const QQBOT_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["qqbot.enabled", "qqbot.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };

const QQBOT_APP_ID_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "app_id",
        label: "qq bot app id",
        config_paths: &["qqbot.app_id", "qqbot.accounts.<account>.app_id"],
        env_pointer_paths: &["qqbot.app_id_env", "qqbot.accounts.<account>.app_id_env"],
        default_env_var: Some(QQBOT_APP_ID_ENV),
    };

const QQBOT_CLIENT_SECRET_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "client_secret",
        label: "qq bot client secret",
        config_paths: &[
            "qqbot.client_secret",
            "qqbot.accounts.<account>.client_secret",
        ],
        env_pointer_paths: &[
            "qqbot.client_secret_env",
            "qqbot.accounts.<account>.client_secret_env",
        ],
        default_env_var: Some(QQBOT_CLIENT_SECRET_ENV),
    };

const QQBOT_ALLOWED_PEER_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_peer_ids",
        label: "allowed peer ids",
        config_paths: &[
            "qqbot.allowed_peer_ids",
            "qqbot.accounts.<account>.allowed_peer_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };

const QQBOT_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    QQBOT_ENABLED_REQUIREMENT,
    QQBOT_APP_ID_REQUIREMENT,
    QQBOT_CLIENT_SECRET_REQUIREMENT,
];

const QQBOT_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    QQBOT_ENABLED_REQUIREMENT,
    QQBOT_APP_ID_REQUIREMENT,
    QQBOT_CLIENT_SECRET_REQUIREMENT,
    QQBOT_ALLOWED_PEER_IDS_REQUIREMENT,
];

const QQBOT_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "bridge send",
    command: "channels send qqbot",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: false,
    requirements: QQBOT_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const QQBOT_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "bridge serve",
    command: "channels serve qqbot",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: true,
    requirements: QQBOT_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const QQBOT_SEND_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[ChannelDoctorCheckSpec {
    name: "qqbot channel",
    trigger: ChannelDoctorCheckTrigger::OperationHealth,
}];

const QQBOT_SERVE_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[
    ChannelDoctorCheckSpec {
        name: "qqbot channel",
        trigger: ChannelDoctorCheckTrigger::OperationHealth,
    },
    ChannelDoctorCheckSpec {
        name: "qqbot serve runtime",
        trigger: ChannelDoctorCheckTrigger::ReadyRuntime,
    },
];

const QQBOT_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: QQBOT_SEND_OPERATION,
        doctor_checks: QQBOT_SEND_DOCTOR_CHECKS,
    },
    ChannelRegistryOperationDescriptor {
        operation: QQBOT_SERVE_OPERATION,
        doctor_checks: QQBOT_SERVE_DOCTOR_CHECKS,
    },
];

const QQBOT_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::PluginBridge,
    setup_hint: "install and configure a QQBot bridge plugin that declares setup.surface=channel plus qqbot app_id, client_secret, and allowed_peer_ids requirements before serving the managed bridge surface",
    status_command: "loong doctor",
    repair_command: None,
};

const MATRIX_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "bridge send",
    command: "channels send matrix",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: false,
    requirements: MATRIX_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const MATRIX_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "bridge serve",
    command: "channels serve matrix",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
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
    label: "bridge send",
    command: "channels send wecom",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: false,
    requirements: WECOM_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const WECOM_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "bridge serve",
    command: "channels serve wecom",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
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
const MATRIX_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::PluginBridge,
    setup_hint: "install and configure a Matrix bridge plugin that declares setup.surface=channel plus matrix access tokens, homeserver base url, allowed room ids, and optional mention gating before serving the managed bridge surface",
    status_command: "loong doctor",
    repair_command: None,
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
    "channel_runtime_contract",
    "channel_runtime_operations_json",
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
    command: "channels send discord",
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
    setup_hint: "configure discord bot credentials in loong.toml under discord or discord.accounts.<account>; outbound direct send is shipped, while gateway-based serve support remains planned",
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
    setup_hint: "configure slack bot credentials in loong.toml under slack or slack.accounts.<account>; outbound direct send is shipped, while Events API or Socket Mode serve support remains planned",
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
    label: "bridge send",
    command: "channels send line",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: false,
    requirements: LINE_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const LINE_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "bridge serve",
    command: "channels serve line",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
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
const LINE_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::PluginBridge,
    setup_hint: "install and configure a LINE bridge plugin that declares setup.surface=channel plus line channel access tokens, channel secrets, and any webhook/runtime requirements before serving the managed bridge surface",
    status_command: "loong doctor",
    repair_command: None,
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
const WECOM_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::PluginBridge,
    setup_hint: "install and configure a WeCom bridge plugin that declares setup.surface=channel plus wecom bot_id, secret, allowed conversation ids, and optional websocket overrides before serving the managed bridge surface",
    status_command: "loong doctor",
    repair_command: None,
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
    command: "channels send dingtalk",
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
    setup_hint: "configure DingTalk custom robot webhook credentials in loong.toml under dingtalk or dingtalk.accounts.<account>; outbound webhook send is shipped, while inbound outgoing-callback serve support remains planned",
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
    label: "bridge send",
    command: "channels send whatsapp",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: false,
    requirements: WHATSAPP_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const WHATSAPP_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "bridge serve",
    command: "channels serve whatsapp",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
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
const WHATSAPP_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::PluginBridge,
    setup_hint: "install and configure a WhatsApp Cloud bridge plugin that declares setup.surface=channel plus access_token, phone_number_id, verify_token, and app_secret requirements before serving the managed bridge surface",
    status_command: "loong doctor",
    repair_command: None,
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
    command: "channels send email",
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
    label: "bridge send",
    command: "channels send webhook",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: false,
    requirements: WEBHOOK_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Endpoint],
};
const WEBHOOK_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "bridge serve",
    command: "channels serve webhook",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
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
const WEBHOOK_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::PluginBridge,
    setup_hint: "install and configure a webhook bridge plugin that declares setup.surface=channel plus endpoint delivery details, signed inbound webhook requirements, and any bind/path serve requirements before exposing the managed bridge surface",
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
    command: "channels send google-chat",
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
        setup_hint: "configure Google Chat incoming webhook credentials in loong.toml under google_chat or google_chat.accounts.<account>; outbound webhook send is shipped, while interactive event serve support remains planned",
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
    command: "channels send signal",
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
    setup_hint: "configure signal bridge connection details in loong.toml under signal or signal.accounts.<account>; outbound direct send is shipped, while inbound listener support remains planned",
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
    command: "channels send teams",
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
    setup_hint: "configure Microsoft Teams webhook delivery in loong.toml under teams or teams.accounts.<account>; outbound incoming-webhook send is shipped, while bot-framework serve support remains planned",
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
    command: "channels send mattermost",
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
    setup_hint: "configure Mattermost server and bot credentials in loong.toml under mattermost or mattermost.accounts.<account>; outbound post send is shipped, while inbound websocket serve support remains planned",
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
    command: "channels send nextcloud-talk",
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
        setup_hint: "configure Nextcloud Talk bot credentials in loong.toml under nextcloud_talk or nextcloud_talk.accounts.<account>; outbound room send is shipped, while inbound bot callback serve support remains planned",
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
    command: "channels send synology-chat",
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
        setup_hint: "configure Synology Chat incoming webhook credentials in loong.toml under synology_chat or synology_chat.accounts.<account>; outbound incoming-webhook send is shipped, while inbound outgoing-webhook serve support remains planned",
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
    command: "channels send irc",
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
    setup_hint: "configure IRC connection details in loong.toml under irc or irc.accounts.<account>; outbound send is shipped, while long-lived relay-loop serve support remains planned",
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
    command: "channels send imessage",
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
    setup_hint: "configure BlueBubbles bridge credentials in loong.toml under imessage or imessage.accounts.<account>; outbound chat send is shipped, while inbound bridge sync serve support remains planned",
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

pub fn channel_inventory(config: &LoongConfig) -> ChannelInventory {
    channel_inventory_with_now(
        config,
        state::default_channel_runtime_state_dir().as_path(),
        now_ms(),
    )
}

pub fn channel_status_snapshots(config: &LoongConfig) -> Vec<ChannelStatusSnapshot> {
    channel_status_snapshots_with_now(
        config,
        state::default_channel_runtime_state_dir().as_path(),
        now_ms(),
    )
}

fn channel_inventory_with_now(
    config: &LoongConfig,
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
    config: &LoongConfig,
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
    config: &LoongConfig,
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
    config: &LoongConfig,
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
    config: &LoongConfig,
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
    config: &LoongConfig,
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
    config: &LoongConfig,
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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
fn runtime_backed_channel_registry_descriptors() -> Vec<&'static ChannelRegistryDescriptor> {
    sorted_channel_registry_descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.runtime.is_some())
        .collect()
}

mod tlon_support;

#[cfg(test)]
mod hotspot_tests;

#[cfg(test)]
mod core_tests;

#[cfg(test)]
mod trust_boundary_tests;
