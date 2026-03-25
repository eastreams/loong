use std::{collections::BTreeSet, path::Path};

use serde::Serialize;

use crate::config::{
    ChannelDefaultAccountSelectionSource, FEISHU_APP_ID_ENV, FEISHU_APP_SECRET_ENV,
    FEISHU_ENCRYPT_KEY_ENV, FEISHU_VERIFICATION_TOKEN_ENV, FeishuChannelServeMode, LoongClawConfig,
    MATRIX_ACCESS_TOKEN_ENV, ResolvedFeishuChannelConfig, ResolvedMatrixChannelConfig,
    ResolvedTelegramChannelConfig, ResolvedWecomChannelConfig, TELEGRAM_BOT_TOKEN_ENV,
    WECOM_BOT_ID_ENV, WECOM_SECRET_ENV,
};

use super::{ChannelCatalogTargetKind, ChannelOperationRuntime, ChannelPlatform, runtime_state};

pub const CHANNEL_OPERATION_SEND_ID: &str = "send";
pub const CHANNEL_OPERATION_SERVE_ID: &str = "serve";

const DISCORD_BOT_TOKEN_ENV: &str = "DISCORD_BOT_TOKEN";
const DISCORD_APPLICATION_ID_ENV: &str = "DISCORD_APPLICATION_ID";
const SLACK_BOT_TOKEN_ENV: &str = "SLACK_BOT_TOKEN";
const SLACK_APP_TOKEN_ENV: &str = "SLACK_APP_TOKEN";
const SLACK_SIGNING_SECRET_ENV: &str = "SLACK_SIGNING_SECRET";
const LINE_CHANNEL_ACCESS_TOKEN_ENV: &str = "LINE_CHANNEL_ACCESS_TOKEN";
const LINE_CHANNEL_SECRET_ENV: &str = "LINE_CHANNEL_SECRET";
const DINGTALK_APP_KEY_ENV: &str = "DINGTALK_APP_KEY";
const DINGTALK_APP_SECRET_ENV: &str = "DINGTALK_APP_SECRET";
const DINGTALK_ROBOT_CODE_ENV: &str = "DINGTALK_ROBOT_CODE";
const WHATSAPP_ACCESS_TOKEN_ENV: &str = "WHATSAPP_ACCESS_TOKEN";
const WHATSAPP_PHONE_NUMBER_ID_ENV: &str = "WHATSAPP_PHONE_NUMBER_ID";
const WHATSAPP_VERIFY_TOKEN_ENV: &str = "WHATSAPP_VERIFY_TOKEN";
const EMAIL_SMTP_USERNAME_ENV: &str = "EMAIL_SMTP_USERNAME";
const EMAIL_SMTP_PASSWORD_ENV: &str = "EMAIL_SMTP_PASSWORD";
const EMAIL_IMAP_USERNAME_ENV: &str = "EMAIL_IMAP_USERNAME";
const EMAIL_IMAP_PASSWORD_ENV: &str = "EMAIL_IMAP_PASSWORD";
const WEBHOOK_AUTH_TOKEN_ENV: &str = "WEBHOOK_AUTH_TOKEN";
const WEBHOOK_SIGNING_SECRET_ENV: &str = "WEBHOOK_SIGNING_SECRET";
const GOOGLE_CHAT_SERVICE_ACCOUNT_JSON_ENV: &str = "GOOGLE_CHAT_SERVICE_ACCOUNT_JSON";
const GOOGLE_CHAT_VERIFICATION_TOKEN_ENV: &str = "GOOGLE_CHAT_VERIFICATION_TOKEN";
const SIGNAL_SERVICE_URL_ENV: &str = "SIGNAL_SERVICE_URL";
const SIGNAL_ACCOUNT_ENV: &str = "SIGNAL_ACCOUNT";
const TEAMS_APP_ID_ENV: &str = "TEAMS_APP_ID";
const TEAMS_APP_PASSWORD_ENV: &str = "TEAMS_APP_PASSWORD";
const TEAMS_TENANT_ID_ENV: &str = "TEAMS_TENANT_ID";
const MATTERMOST_SERVER_URL_ENV: &str = "MATTERMOST_SERVER_URL";
const MATTERMOST_BOT_TOKEN_ENV: &str = "MATTERMOST_BOT_TOKEN";
const NEXTCLOUD_TALK_SERVER_URL_ENV: &str = "NEXTCLOUD_TALK_SERVER_URL";
const NEXTCLOUD_TALK_APP_PASSWORD_ENV: &str = "NEXTCLOUD_TALK_APP_PASSWORD";
const NEXTCLOUD_TALK_BOT_ACTOR_ID_ENV: &str = "NEXTCLOUD_TALK_BOT_ACTOR_ID";
const SYNOLOGY_CHAT_TOKEN_ENV: &str = "SYNOLOGY_CHAT_TOKEN";
const SYNOLOGY_CHAT_INCOMING_URL_ENV: &str = "SYNOLOGY_CHAT_INCOMING_URL";
const IRC_SERVER_ENV: &str = "IRC_SERVER";
const IRC_NICKNAME_ENV: &str = "IRC_NICKNAME";
const IMESSAGE_BRIDGE_URL_ENV: &str = "IMESSAGE_BRIDGE_URL";
const IMESSAGE_BRIDGE_TOKEN_ENV: &str = "IMESSAGE_BRIDGE_TOKEN";
const NOSTR_RELAY_URLS_ENV: &str = "NOSTR_RELAY_URLS";
const NOSTR_PRIVATE_KEY_ENV: &str = "NOSTR_PRIVATE_KEY";
const TWITCH_BOT_OAUTH_TOKEN_ENV: &str = "TWITCH_BOT_OAUTH_TOKEN";
const TWITCH_CLIENT_ID_ENV: &str = "TWITCH_CLIENT_ID";
const TLON_SHIP_ENV: &str = "TLON_SHIP";
const TLON_URL_ENV: &str = "TLON_URL";
const TLON_CODE_ENV: &str = "TLON_CODE";
const ZALO_APP_ID_ENV: &str = "ZALO_APP_ID";
const ZALO_OA_ACCESS_TOKEN_ENV: &str = "ZALO_OA_ACCESS_TOKEN";
const ZALO_APP_SECRET_ENV: &str = "ZALO_APP_SECRET";
const ZALO_PERSONAL_ACCESS_TOKEN_ENV: &str = "ZALO_PERSONAL_ACCESS_TOKEN";
const WEBCHAT_PUBLIC_BASE_URL_ENV: &str = "WEBCHAT_PUBLIC_BASE_URL";
const WEBCHAT_SESSION_SIGNING_SECRET_ENV: &str = "WEBCHAT_SESSION_SIGNING_SECRET";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelRuntimeCommandDescriptor {
    pub channel_id: &'static str,
    pub platform: ChannelPlatform,
    pub serve_bootstrap_agent_id: &'static str,
}

pub const TELEGRAM_RUNTIME_COMMAND_DESCRIPTOR: ChannelRuntimeCommandDescriptor =
    ChannelRuntimeCommandDescriptor {
        channel_id: "telegram",
        platform: ChannelPlatform::Telegram,
        serve_bootstrap_agent_id: "channel-telegram",
    };

pub const FEISHU_RUNTIME_COMMAND_DESCRIPTOR: ChannelRuntimeCommandDescriptor =
    ChannelRuntimeCommandDescriptor {
        channel_id: "feishu",
        platform: ChannelPlatform::Feishu,
        serve_bootstrap_agent_id: "channel-feishu",
    };

pub const MATRIX_RUNTIME_COMMAND_DESCRIPTOR: ChannelRuntimeCommandDescriptor =
    ChannelRuntimeCommandDescriptor {
        channel_id: "matrix",
        platform: ChannelPlatform::Matrix,
        serve_bootstrap_agent_id: "channel-matrix",
    };

pub const WECOM_RUNTIME_COMMAND_DESCRIPTOR: ChannelRuntimeCommandDescriptor =
    ChannelRuntimeCommandDescriptor {
        channel_id: "wecom",
        platform: ChannelPlatform::Wecom,
        serve_bootstrap_agent_id: "channel-wecom",
    };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelCommandFamilyDescriptor {
    pub runtime: ChannelRuntimeCommandDescriptor,
    pub catalog: ChannelCatalogCommandFamilyDescriptor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelCatalogCommandFamilyDescriptor {
    pub channel_id: &'static str,
    pub default_send_target_kind: ChannelCatalogTargetKind,
    pub send: ChannelCatalogOperation,
    pub serve: ChannelCatalogOperation,
}

impl ChannelCommandFamilyDescriptor {
    pub fn channel_id(self) -> &'static str {
        self.catalog.channel_id
    }

    pub fn default_send_target_kind(self) -> ChannelCatalogTargetKind {
        self.catalog.default_send_target_kind
    }

    pub fn send(self) -> ChannelCatalogOperation {
        self.catalog.send
    }

    pub fn serve(self) -> ChannelCatalogOperation {
        self.catalog.serve
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ChannelCatalogOperation {
    pub id: &'static str,
    pub label: &'static str,
    pub command: &'static str,
    pub availability: ChannelCatalogOperationAvailability,
    pub tracks_runtime: bool,
    pub requirements: &'static [ChannelCatalogOperationRequirement],
    pub supported_target_kinds: &'static [ChannelCatalogTargetKind],
}

impl ChannelCatalogOperation {
    pub fn supports_target_kind(self, kind: ChannelCatalogTargetKind) -> bool {
        self.supported_target_kinds.contains(&kind)
    }

    pub fn default_target_kind(self) -> Option<ChannelCatalogTargetKind> {
        self.supported_target_kinds.first().copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ChannelCatalogOperationRequirement {
    pub id: &'static str,
    pub label: &'static str,
    pub config_paths: &'static [&'static str],
    pub env_pointer_paths: &'static [&'static str],
    pub default_env_var: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelCatalogOperationAvailability {
    Implemented,
    Stub,
}

impl ChannelCatalogOperationAvailability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Implemented => "implemented",
            Self::Stub => "stub",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelCapability {
    RuntimeBacked,
    MultiAccount,
    Send,
    Serve,
    RuntimeTracking,
}

impl ChannelCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeBacked => "runtime_backed",
            Self::MultiAccount => "multi_account",
            Self::Send => "send",
            Self::Serve => "serve",
            Self::RuntimeTracking => "runtime_tracking",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelOnboardingStrategy {
    ManualConfig,
    Planned,
}

impl ChannelOnboardingStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManualConfig => "manual_config",
            Self::Planned => "planned",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ChannelOnboardingDescriptor {
    pub strategy: ChannelOnboardingStrategy,
    pub setup_hint: &'static str,
    pub status_command: &'static str,
    pub repair_command: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelDoctorCheckTrigger {
    OperationHealth,
    ReadyRuntime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelDoctorCheckSpec {
    pub name: &'static str,
    pub trigger: ChannelDoctorCheckTrigger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelDoctorOperationSpec {
    pub checks: &'static [ChannelDoctorCheckSpec],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelOperationDescriptor {
    pub operation: ChannelCatalogOperation,
    pub doctor: Option<ChannelDoctorOperationSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelCatalogImplementationStatus {
    RuntimeBacked,
    Stub,
}

impl ChannelCatalogImplementationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeBacked => "runtime_backed",
            Self::Stub => "stub",
        }
    }
}

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelSurface {
    pub catalog: ChannelCatalogEntry,
    pub configured_accounts: Vec<ChannelStatusSnapshot>,
    pub default_configured_account_id: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct ChannelRuntimeDescriptor {
    family: ChannelCommandFamilyDescriptor,
    snapshot_builder: ChannelSnapshotBuilder,
}

#[derive(Debug, Clone, Copy)]
struct ChannelRegistryOperationDescriptor {
    operation: ChannelCatalogOperation,
    doctor_checks: &'static [ChannelDoctorCheckSpec],
}

type ChannelSnapshotBuilder =
    fn(&ChannelRegistryDescriptor, &LoongClawConfig, &Path, u64) -> Vec<ChannelStatusSnapshot>;

#[derive(Debug, Clone, Copy)]
struct ChannelRegistryDescriptor {
    id: &'static str,
    runtime: Option<ChannelRuntimeDescriptor>,
    selection_order: u16,
    selection_label: &'static str,
    blurb: &'static str,
    implementation_status: ChannelCatalogImplementationStatus,
    capabilities: &'static [ChannelCapability],
    label: &'static str,
    aliases: &'static [&'static str],
    transport: &'static str,
    onboarding: ChannelOnboardingDescriptor,
    operations: &'static [ChannelRegistryOperationDescriptor],
}

const TELEGRAM_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "direct send",
    command: "telegram-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: TELEGRAM_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const TELEGRAM_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "reply loop",
    command: "telegram-serve",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: true,
    requirements: TELEGRAM_SERVE_REQUIREMENTS,
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
const TELEGRAM_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] =
    &[TELEGRAM_ENABLED_REQUIREMENT, TELEGRAM_BOT_TOKEN_REQUIREMENT];
const TELEGRAM_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    TELEGRAM_ENABLED_REQUIREMENT,
    TELEGRAM_BOT_TOKEN_REQUIREMENT,
    TELEGRAM_ALLOWED_CHAT_IDS_REQUIREMENT,
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
    setup_hint: "configure telegram bot credentials and allowed chat ids in loongclaw.toml under telegram or telegram.accounts.<account>",
    status_command: "loongclaw doctor",
    repair_command: Some("loongclaw doctor --fix"),
};

const FEISHU_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "direct send",
    command: "feishu-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: FEISHU_SEND_REQUIREMENTS,
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
    status_command: "loongclaw doctor",
    repair_command: Some("loongclaw doctor --fix"),
};

const MATRIX_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "direct send",
    command: "matrix-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: MATRIX_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const MATRIX_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "sync reply loop",
    command: "matrix-serve",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: true,
    requirements: MATRIX_SERVE_REQUIREMENTS,
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
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const WECOM_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "long connection reply loop",
    command: "wecom-serve",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: true,
    requirements: WECOM_SERVE_REQUIREMENTS,
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
    setup_hint: "configure matrix access tokens, homeserver base url, and allowed room ids in loongclaw.toml under matrix or matrix.accounts.<account>",
    status_command: "loongclaw doctor",
    repair_command: Some("loongclaw doctor --fix"),
};

const PLANNED_CHANNEL_CAPABILITIES: &[ChannelCapability] = &[
    ChannelCapability::MultiAccount,
    ChannelCapability::Send,
    ChannelCapability::Serve,
    ChannelCapability::RuntimeTracking,
];

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
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: DISCORD_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const DISCORD_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "gateway reply loop",
    command: "discord-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: DISCORD_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const DISCORD_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: DISCORD_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: DISCORD_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const DISCORD_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned Discord gateway surface; catalog metadata reflects the intended bot token, application id, and guild allowlist contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
    repair_command: None,
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
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: SLACK_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const SLACK_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "events reply loop",
    command: "slack-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: SLACK_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const SLACK_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: SLACK_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: SLACK_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const SLACK_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned Slack Events API or Socket Mode surface; catalog metadata reflects the intended bot token, app token, signing secret, and allowlist contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
    repair_command: None,
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
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: LINE_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const LINE_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "webhook reply loop",
    command: "line-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: LINE_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const LINE_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: LINE_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: LINE_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const LINE_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned LINE Messaging API surface; catalog metadata reflects the intended channel access token and webhook secret contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
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
    status_command: "loongclaw doctor",
    repair_command: Some("loongclaw doctor --fix"),
};

const DINGTALK_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["dingtalk.enabled", "dingtalk.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const DINGTALK_APP_KEY_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "app_key",
        label: "app key",
        config_paths: &["dingtalk.app_key", "dingtalk.accounts.<account>.app_key"],
        env_pointer_paths: &[
            "dingtalk.app_key_env",
            "dingtalk.accounts.<account>.app_key_env",
        ],
        default_env_var: Some(DINGTALK_APP_KEY_ENV),
    };
const DINGTALK_APP_SECRET_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "app_secret",
        label: "app secret",
        config_paths: &[
            "dingtalk.app_secret",
            "dingtalk.accounts.<account>.app_secret",
        ],
        env_pointer_paths: &[
            "dingtalk.app_secret_env",
            "dingtalk.accounts.<account>.app_secret_env",
        ],
        default_env_var: Some(DINGTALK_APP_SECRET_ENV),
    };
const DINGTALK_ROBOT_CODE_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "robot_code",
        label: "robot code",
        config_paths: &[
            "dingtalk.robot_code",
            "dingtalk.accounts.<account>.robot_code",
        ],
        env_pointer_paths: &[
            "dingtalk.robot_code_env",
            "dingtalk.accounts.<account>.robot_code_env",
        ],
        default_env_var: Some(DINGTALK_ROBOT_CODE_ENV),
    };
const DINGTALK_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    DINGTALK_ENABLED_REQUIREMENT,
    DINGTALK_APP_KEY_REQUIREMENT,
    DINGTALK_APP_SECRET_REQUIREMENT,
    DINGTALK_ROBOT_CODE_REQUIREMENT,
];
const DINGTALK_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] =
    DINGTALK_SEND_REQUIREMENTS;
const DINGTALK_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "robot send",
    command: "dingtalk-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: DINGTALK_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const DINGTALK_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "event callback service",
    command: "dingtalk-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: DINGTALK_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const DINGTALK_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: DINGTALK_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: DINGTALK_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const DINGTALK_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned DingTalk robot surface; catalog metadata reflects the intended app key, app secret, and robot code contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
    repair_command: None,
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
        default_env_var: None,
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
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: WHATSAPP_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const WHATSAPP_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "cloud webhook service",
    command: "whatsapp-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: WHATSAPP_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const WHATSAPP_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: WHATSAPP_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: WHATSAPP_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const WHATSAPP_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned WhatsApp Cloud API surface; catalog metadata reflects the intended access token, phone number id, webhook verify token, and app secret contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
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
    command: "email-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: EMAIL_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const EMAIL_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "imap reply loop",
    command: "email-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: EMAIL_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const EMAIL_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: EMAIL_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: EMAIL_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const EMAIL_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned email surface; catalog metadata reflects the intended SMTP send and IMAP reply-loop contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
    repair_command: None,
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
        env_pointer_paths: &[],
        default_env_var: None,
    };
const WEBHOOK_AUTH_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "auth_token",
        label: "auth token",
        config_paths: &[
            "webhook.auth_token",
            "webhook.accounts.<account>.auth_token",
        ],
        env_pointer_paths: &[
            "webhook.auth_token_env",
            "webhook.accounts.<account>.auth_token_env",
        ],
        default_env_var: Some(WEBHOOK_AUTH_TOKEN_ENV),
    };
const WEBHOOK_PUBLIC_BASE_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "public_base_url",
        label: "public base url",
        config_paths: &[
            "webhook.public_base_url",
            "webhook.accounts.<account>.public_base_url",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
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
    WEBHOOK_AUTH_TOKEN_REQUIREMENT,
];
const WEBHOOK_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    WEBHOOK_ENABLED_REQUIREMENT,
    WEBHOOK_PUBLIC_BASE_URL_REQUIREMENT,
    WEBHOOK_SIGNING_SECRET_REQUIREMENT,
];
const WEBHOOK_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "http post send",
    command: "webhook-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: WEBHOOK_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Endpoint],
};
const WEBHOOK_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "inbound webhook service",
    command: "webhook-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: WEBHOOK_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Endpoint],
};
const WEBHOOK_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: WEBHOOK_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: WEBHOOK_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const WEBHOOK_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned generic webhook surface; catalog metadata reflects the intended outbound endpoint and inbound signing-secret contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
    repair_command: None,
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
const GOOGLE_CHAT_SERVICE_ACCOUNT_JSON_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "service_account_json",
        label: "service account json",
        config_paths: &[
            "google_chat.service_account_json",
            "google_chat.accounts.<account>.service_account_json",
        ],
        env_pointer_paths: &[
            "google_chat.service_account_json_env",
            "google_chat.accounts.<account>.service_account_json_env",
        ],
        default_env_var: Some(GOOGLE_CHAT_SERVICE_ACCOUNT_JSON_ENV),
    };
const GOOGLE_CHAT_SPACE_ID_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "space_id",
        label: "space id",
        config_paths: &[
            "google_chat.space_id",
            "google_chat.accounts.<account>.space_id",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const GOOGLE_CHAT_ALLOWED_SPACE_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_space_ids",
        label: "allowed space ids",
        config_paths: &[
            "google_chat.allowed_space_ids",
            "google_chat.accounts.<account>.allowed_space_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const GOOGLE_CHAT_VERIFICATION_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "verification_token",
        label: "verification token",
        config_paths: &[
            "google_chat.verification_token",
            "google_chat.accounts.<account>.verification_token",
        ],
        env_pointer_paths: &[
            "google_chat.verification_token_env",
            "google_chat.accounts.<account>.verification_token_env",
        ],
        default_env_var: Some(GOOGLE_CHAT_VERIFICATION_TOKEN_ENV),
    };
const GOOGLE_CHAT_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    GOOGLE_CHAT_ENABLED_REQUIREMENT,
    GOOGLE_CHAT_SERVICE_ACCOUNT_JSON_REQUIREMENT,
    GOOGLE_CHAT_SPACE_ID_REQUIREMENT,
];
const GOOGLE_CHAT_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    GOOGLE_CHAT_ENABLED_REQUIREMENT,
    GOOGLE_CHAT_SERVICE_ACCOUNT_JSON_REQUIREMENT,
    GOOGLE_CHAT_ALLOWED_SPACE_IDS_REQUIREMENT,
    GOOGLE_CHAT_VERIFICATION_TOKEN_REQUIREMENT,
];
const GOOGLE_CHAT_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "space send",
    command: "google-chat-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: GOOGLE_CHAT_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const GOOGLE_CHAT_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "space event service",
    command: "google-chat-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: GOOGLE_CHAT_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const GOOGLE_CHAT_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: GOOGLE_CHAT_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: GOOGLE_CHAT_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const GOOGLE_CHAT_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor =
    ChannelOnboardingDescriptor {
        strategy: ChannelOnboardingStrategy::Planned,
        setup_hint: "planned Google Chat surface; catalog metadata reflects the intended service-account, space routing, and event verification contract, but no runtime adapter is implemented yet",
        status_command: "loongclaw channels --json",
        repair_command: None,
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
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: SIGNAL_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const SIGNAL_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "linked-device listener",
    command: "signal-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: SIGNAL_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const SIGNAL_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: SIGNAL_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: SIGNAL_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const SIGNAL_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned Signal bridge surface; catalog metadata reflects the intended service endpoint, linked-account identity, and sender allowlist contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
    repair_command: None,
};

const TEAMS_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["teams.enabled", "teams.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
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
const TEAMS_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    TEAMS_ENABLED_REQUIREMENT,
    TEAMS_APP_ID_REQUIREMENT,
    TEAMS_APP_PASSWORD_REQUIREMENT,
    TEAMS_TENANT_ID_REQUIREMENT,
];
const TEAMS_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    TEAMS_ENABLED_REQUIREMENT,
    TEAMS_APP_ID_REQUIREMENT,
    TEAMS_APP_PASSWORD_REQUIREMENT,
    TEAMS_TENANT_ID_REQUIREMENT,
    TEAMS_ALLOWED_CONVERSATION_IDS_REQUIREMENT,
];
const TEAMS_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "conversation send",
    command: "teams-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: TEAMS_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const TEAMS_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "bot event service",
    command: "teams-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: TEAMS_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const TEAMS_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: TEAMS_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: TEAMS_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const TEAMS_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned Microsoft Teams surface; catalog metadata reflects the intended app id, app password, tenant binding, and conversation allowlist contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
    repair_command: None,
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
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: MATTERMOST_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const MATTERMOST_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "event websocket service",
    command: "mattermost-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: MATTERMOST_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const MATTERMOST_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: MATTERMOST_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: MATTERMOST_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const MATTERMOST_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned Mattermost surface; catalog metadata reflects the intended self-hosted server url, bot token, and channel allowlist contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
    repair_command: None,
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
const NEXTCLOUD_TALK_APP_PASSWORD_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "app_password",
        label: "app password",
        config_paths: &[
            "nextcloud_talk.app_password",
            "nextcloud_talk.accounts.<account>.app_password",
        ],
        env_pointer_paths: &[
            "nextcloud_talk.app_password_env",
            "nextcloud_talk.accounts.<account>.app_password_env",
        ],
        default_env_var: Some(NEXTCLOUD_TALK_APP_PASSWORD_ENV),
    };
const NEXTCLOUD_TALK_BOT_ACTOR_ID_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "bot_actor_id",
        label: "bot actor id",
        config_paths: &[
            "nextcloud_talk.bot_actor_id",
            "nextcloud_talk.accounts.<account>.bot_actor_id",
        ],
        env_pointer_paths: &[
            "nextcloud_talk.bot_actor_id_env",
            "nextcloud_talk.accounts.<account>.bot_actor_id_env",
        ],
        default_env_var: Some(NEXTCLOUD_TALK_BOT_ACTOR_ID_ENV),
    };
const NEXTCLOUD_TALK_ALLOWED_ROOM_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_room_ids",
        label: "allowed room ids",
        config_paths: &[
            "nextcloud_talk.allowed_room_ids",
            "nextcloud_talk.accounts.<account>.allowed_room_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const NEXTCLOUD_TALK_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    NEXTCLOUD_TALK_ENABLED_REQUIREMENT,
    NEXTCLOUD_TALK_SERVER_URL_REQUIREMENT,
    NEXTCLOUD_TALK_APP_PASSWORD_REQUIREMENT,
    NEXTCLOUD_TALK_BOT_ACTOR_ID_REQUIREMENT,
];
const NEXTCLOUD_TALK_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    NEXTCLOUD_TALK_ENABLED_REQUIREMENT,
    NEXTCLOUD_TALK_SERVER_URL_REQUIREMENT,
    NEXTCLOUD_TALK_APP_PASSWORD_REQUIREMENT,
    NEXTCLOUD_TALK_BOT_ACTOR_ID_REQUIREMENT,
    NEXTCLOUD_TALK_ALLOWED_ROOM_IDS_REQUIREMENT,
];
const NEXTCLOUD_TALK_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "room send",
    command: "nextcloud-talk-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: NEXTCLOUD_TALK_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const NEXTCLOUD_TALK_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "talk room service",
    command: "nextcloud-talk-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: NEXTCLOUD_TALK_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const NEXTCLOUD_TALK_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: NEXTCLOUD_TALK_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: NEXTCLOUD_TALK_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const NEXTCLOUD_TALK_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor =
    ChannelOnboardingDescriptor {
        strategy: ChannelOnboardingStrategy::Planned,
        setup_hint: "planned Nextcloud Talk surface; catalog metadata reflects the intended server url, app password, bot actor id, and room allowlist contract, but no runtime adapter is implemented yet",
        status_command: "loongclaw channels --json",
        repair_command: None,
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
    SYNOLOGY_CHAT_TOKEN_REQUIREMENT,
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
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: SYNOLOGY_CHAT_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const SYNOLOGY_CHAT_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "webhook service",
    command: "synology-chat-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: SYNOLOGY_CHAT_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const SYNOLOGY_CHAT_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: SYNOLOGY_CHAT_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: SYNOLOGY_CHAT_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const SYNOLOGY_CHAT_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor =
    ChannelOnboardingDescriptor {
        strategy: ChannelOnboardingStrategy::Planned,
        setup_hint: "planned Synology Chat surface; catalog metadata reflects the intended outgoing webhook token, incoming webhook url, and user allowlist contract, but no runtime adapter is implemented yet",
        status_command: "loongclaw channels --json",
        repair_command: None,
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
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: IRC_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const IRC_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "relay loop",
    command: "irc-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: IRC_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const IRC_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: IRC_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: IRC_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const IRC_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned IRC surface; catalog metadata reflects the intended server, nick, and channel subscription contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
    repair_command: None,
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
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: IMESSAGE_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const IMESSAGE_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "bridge sync service",
    command: "imessage-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: IMESSAGE_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const IMESSAGE_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: IMESSAGE_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: IMESSAGE_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const IMESSAGE_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned BlueBubbles-backed iMessage bridge surface; catalog metadata reflects the intended bridge url, bridge token, and chat allowlist contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
    repair_command: None,
};

const NOSTR_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["nostr.enabled", "nostr.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const NOSTR_RELAY_URLS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "relay_urls",
        label: "relay urls",
        config_paths: &["nostr.relay_urls", "nostr.accounts.<account>.relay_urls"],
        env_pointer_paths: &[
            "nostr.relay_urls_env",
            "nostr.accounts.<account>.relay_urls_env",
        ],
        default_env_var: Some(NOSTR_RELAY_URLS_ENV),
    };
const NOSTR_PRIVATE_KEY_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "private_key",
        label: "private key",
        config_paths: &["nostr.private_key", "nostr.accounts.<account>.private_key"],
        env_pointer_paths: &[
            "nostr.private_key_env",
            "nostr.accounts.<account>.private_key_env",
        ],
        default_env_var: Some(NOSTR_PRIVATE_KEY_ENV),
    };
const NOSTR_ALLOWED_PUBKEYS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_pubkeys",
        label: "allowed pubkeys",
        config_paths: &[
            "nostr.allowed_pubkeys",
            "nostr.accounts.<account>.allowed_pubkeys",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const NOSTR_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    NOSTR_ENABLED_REQUIREMENT,
    NOSTR_RELAY_URLS_REQUIREMENT,
    NOSTR_PRIVATE_KEY_REQUIREMENT,
];
const NOSTR_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    NOSTR_ENABLED_REQUIREMENT,
    NOSTR_RELAY_URLS_REQUIREMENT,
    NOSTR_PRIVATE_KEY_REQUIREMENT,
    NOSTR_ALLOWED_PUBKEYS_REQUIREMENT,
];
const NOSTR_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "relay publish",
    command: "nostr-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: NOSTR_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const NOSTR_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "relay subscriber",
    command: "nostr-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: NOSTR_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const NOSTR_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: NOSTR_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: NOSTR_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const NOSTR_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned Nostr surface; catalog metadata reflects the intended relay list, signing key, and pubkey allowlist contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
    repair_command: None,
};

const TWITCH_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["twitch.enabled", "twitch.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const TWITCH_BOT_OAUTH_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "bot_oauth_token",
        label: "bot oauth token",
        config_paths: &[
            "twitch.bot_oauth_token",
            "twitch.accounts.<account>.bot_oauth_token",
        ],
        env_pointer_paths: &[
            "twitch.bot_oauth_token_env",
            "twitch.accounts.<account>.bot_oauth_token_env",
        ],
        default_env_var: Some(TWITCH_BOT_OAUTH_TOKEN_ENV),
    };
const TWITCH_CLIENT_ID_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "client_id",
        label: "client id",
        config_paths: &["twitch.client_id", "twitch.accounts.<account>.client_id"],
        env_pointer_paths: &[
            "twitch.client_id_env",
            "twitch.accounts.<account>.client_id_env",
        ],
        default_env_var: Some(TWITCH_CLIENT_ID_ENV),
    };
const TWITCH_CHANNEL_NAMES_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "channel_names",
        label: "channel names",
        config_paths: &[
            "twitch.channel_names",
            "twitch.accounts.<account>.channel_names",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const TWITCH_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    TWITCH_ENABLED_REQUIREMENT,
    TWITCH_BOT_OAUTH_TOKEN_REQUIREMENT,
    TWITCH_CLIENT_ID_REQUIREMENT,
];
const TWITCH_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    TWITCH_ENABLED_REQUIREMENT,
    TWITCH_BOT_OAUTH_TOKEN_REQUIREMENT,
    TWITCH_CLIENT_ID_REQUIREMENT,
    TWITCH_CHANNEL_NAMES_REQUIREMENT,
];
const TWITCH_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "chat send",
    command: "twitch-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: TWITCH_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const TWITCH_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "chat listener",
    command: "twitch-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: TWITCH_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const TWITCH_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: TWITCH_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: TWITCH_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const TWITCH_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned Twitch chat surface; catalog metadata reflects the intended bot oauth token, client id, and channel subscription contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
    repair_command: None,
};

const TLON_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["tlon.enabled", "tlon.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const TLON_SHIP_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "ship",
        label: "ship",
        config_paths: &["tlon.ship", "tlon.accounts.<account>.ship"],
        env_pointer_paths: &["tlon.ship_env", "tlon.accounts.<account>.ship_env"],
        default_env_var: Some(TLON_SHIP_ENV),
    };
const TLON_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "url",
        label: "ship url",
        config_paths: &["tlon.url", "tlon.accounts.<account>.url"],
        env_pointer_paths: &["tlon.url_env", "tlon.accounts.<account>.url_env"],
        default_env_var: Some(TLON_URL_ENV),
    };
const TLON_CODE_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "code",
        label: "login code",
        config_paths: &["tlon.code", "tlon.accounts.<account>.code"],
        env_pointer_paths: &["tlon.code_env", "tlon.accounts.<account>.code_env"],
        default_env_var: Some(TLON_CODE_ENV),
    };
const TLON_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    TLON_ENABLED_REQUIREMENT,
    TLON_SHIP_REQUIREMENT,
    TLON_URL_REQUIREMENT,
    TLON_CODE_REQUIREMENT,
];
const TLON_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    TLON_ENABLED_REQUIREMENT,
    TLON_SHIP_REQUIREMENT,
    TLON_URL_REQUIREMENT,
    TLON_CODE_REQUIREMENT,
];
const TLON_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "ship message send",
    command: "tlon-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: TLON_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const TLON_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "ship event service",
    command: "tlon-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: TLON_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const TLON_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: TLON_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: TLON_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const TLON_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned Tlon surface; catalog metadata reflects the intended ship identity, ship url, and login-code contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
    repair_command: None,
};

const ZALO_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["zalo.enabled", "zalo.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const ZALO_APP_ID_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "app_id",
        label: "app id",
        config_paths: &["zalo.app_id", "zalo.accounts.<account>.app_id"],
        env_pointer_paths: &["zalo.app_id_env", "zalo.accounts.<account>.app_id_env"],
        default_env_var: Some(ZALO_APP_ID_ENV),
    };
const ZALO_OA_ACCESS_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "oa_access_token",
        label: "official account access token",
        config_paths: &[
            "zalo.oa_access_token",
            "zalo.accounts.<account>.oa_access_token",
        ],
        env_pointer_paths: &[
            "zalo.oa_access_token_env",
            "zalo.accounts.<account>.oa_access_token_env",
        ],
        default_env_var: Some(ZALO_OA_ACCESS_TOKEN_ENV),
    };
const ZALO_APP_SECRET_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "app_secret",
        label: "app secret",
        config_paths: &["zalo.app_secret", "zalo.accounts.<account>.app_secret"],
        env_pointer_paths: &[
            "zalo.app_secret_env",
            "zalo.accounts.<account>.app_secret_env",
        ],
        default_env_var: Some(ZALO_APP_SECRET_ENV),
    };
const ZALO_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    ZALO_ENABLED_REQUIREMENT,
    ZALO_APP_ID_REQUIREMENT,
    ZALO_OA_ACCESS_TOKEN_REQUIREMENT,
];
const ZALO_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    ZALO_ENABLED_REQUIREMENT,
    ZALO_APP_ID_REQUIREMENT,
    ZALO_OA_ACCESS_TOKEN_REQUIREMENT,
    ZALO_APP_SECRET_REQUIREMENT,
];
const ZALO_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "official account send",
    command: "zalo-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: ZALO_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const ZALO_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "official account webhook service",
    command: "zalo-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: ZALO_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const ZALO_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: ZALO_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: ZALO_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const ZALO_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned Zalo official account surface; catalog metadata reflects the intended app id, official account access token, and webhook secret contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
    repair_command: None,
};

const ZALO_PERSONAL_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &[
            "zalo_personal.enabled",
            "zalo_personal.accounts.<account>.enabled",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const ZALO_PERSONAL_ACCESS_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "access_token",
        label: "personal bridge access token",
        config_paths: &[
            "zalo_personal.access_token",
            "zalo_personal.accounts.<account>.access_token",
        ],
        env_pointer_paths: &[
            "zalo_personal.access_token_env",
            "zalo_personal.accounts.<account>.access_token_env",
        ],
        default_env_var: Some(ZALO_PERSONAL_ACCESS_TOKEN_ENV),
    };
const ZALO_PERSONAL_ALLOWED_CONTACT_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_contact_ids",
        label: "allowed contact ids",
        config_paths: &[
            "zalo_personal.allowed_contact_ids",
            "zalo_personal.accounts.<account>.allowed_contact_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const ZALO_PERSONAL_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    ZALO_PERSONAL_ENABLED_REQUIREMENT,
    ZALO_PERSONAL_ACCESS_TOKEN_REQUIREMENT,
];
const ZALO_PERSONAL_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    ZALO_PERSONAL_ENABLED_REQUIREMENT,
    ZALO_PERSONAL_ACCESS_TOKEN_REQUIREMENT,
    ZALO_PERSONAL_ALLOWED_CONTACT_IDS_REQUIREMENT,
];
const ZALO_PERSONAL_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "personal send",
    command: "zalo-personal-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: ZALO_PERSONAL_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const ZALO_PERSONAL_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "personal message bridge",
    command: "zalo-personal-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: ZALO_PERSONAL_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const ZALO_PERSONAL_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: ZALO_PERSONAL_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: ZALO_PERSONAL_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const ZALO_PERSONAL_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor =
    ChannelOnboardingDescriptor {
        strategy: ChannelOnboardingStrategy::Planned,
        setup_hint: "planned Zalo personal bridge surface; catalog metadata reflects the intended bridge access token and contact allowlist contract, but no runtime adapter is implemented yet",
        status_command: "loongclaw channels --json",
        repair_command: None,
    };

const WEBCHAT_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["webchat.enabled", "webchat.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const WEBCHAT_PUBLIC_BASE_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "public_base_url",
        label: "public base url",
        config_paths: &[
            "webchat.public_base_url",
            "webchat.accounts.<account>.public_base_url",
        ],
        env_pointer_paths: &[
            "webchat.public_base_url_env",
            "webchat.accounts.<account>.public_base_url_env",
        ],
        default_env_var: Some(WEBCHAT_PUBLIC_BASE_URL_ENV),
    };
const WEBCHAT_SESSION_SIGNING_SECRET_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "session_signing_secret",
        label: "session signing secret",
        config_paths: &[
            "webchat.session_signing_secret",
            "webchat.accounts.<account>.session_signing_secret",
        ],
        env_pointer_paths: &[
            "webchat.session_signing_secret_env",
            "webchat.accounts.<account>.session_signing_secret_env",
        ],
        default_env_var: Some(WEBCHAT_SESSION_SIGNING_SECRET_ENV),
    };
const WEBCHAT_ALLOWED_ORIGINS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_origins",
        label: "allowed origins",
        config_paths: &[
            "webchat.allowed_origins",
            "webchat.accounts.<account>.allowed_origins",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };
const WEBCHAT_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    WEBCHAT_ENABLED_REQUIREMENT,
    WEBCHAT_PUBLIC_BASE_URL_REQUIREMENT,
    WEBCHAT_SESSION_SIGNING_SECRET_REQUIREMENT,
];
const WEBCHAT_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    WEBCHAT_ENABLED_REQUIREMENT,
    WEBCHAT_PUBLIC_BASE_URL_REQUIREMENT,
    WEBCHAT_SESSION_SIGNING_SECRET_REQUIREMENT,
    WEBCHAT_ALLOWED_ORIGINS_REQUIREMENT,
];
const WEBCHAT_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "browser session send",
    command: "webchat-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: WEBCHAT_SEND_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const WEBCHAT_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "browser session service",
    command: "webchat-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: WEBCHAT_SERVE_REQUIREMENTS,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const WEBCHAT_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: WEBCHAT_SEND_OPERATION,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: WEBCHAT_SERVE_OPERATION,
        doctor_checks: &[],
    },
];
const WEBCHAT_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::Planned,
    setup_hint: "planned web chat surface; catalog metadata reflects the intended public base url, browser session signing secret, and origin allowlist contract, but no runtime adapter is implemented yet",
    status_command: "loongclaw channels --json",
    repair_command: None,
};

const CHANNEL_REGISTRY: &[ChannelRegistryDescriptor] = &[
    ChannelRegistryDescriptor {
        id: "telegram",
        runtime: Some(ChannelRuntimeDescriptor {
            family: TELEGRAM_COMMAND_FAMILY_DESCRIPTOR,
            snapshot_builder: build_telegram_snapshots,
        }),
        selection_order: 10,
        selection_label: "personal and group chat bot",
        blurb: "Shipped Telegram Bot API surface with direct send and reply-loop runtime support.",
        implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
        capabilities: TELEGRAM_CAPABILITIES,
        label: "Telegram",
        aliases: &[],
        transport: "telegram_bot_api_polling",
        onboarding: TELEGRAM_ONBOARDING_DESCRIPTOR,
        operations: TELEGRAM_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "feishu",
        runtime: Some(ChannelRuntimeDescriptor {
            family: FEISHU_COMMAND_FAMILY_DESCRIPTOR,
            snapshot_builder: build_feishu_snapshots,
        }),
        selection_order: 20,
        selection_label: "enterprise chat app",
        blurb: "Shipped Feishu/Lark app surface with webhook or websocket ingress and account-aware runtime state.",
        implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
        capabilities: FEISHU_CAPABILITIES,
        label: "Feishu/Lark",
        aliases: &["lark"],
        transport: "feishu_openapi_webhook_or_websocket",
        onboarding: FEISHU_ONBOARDING_DESCRIPTOR,
        operations: FEISHU_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "matrix",
        runtime: Some(ChannelRuntimeDescriptor {
            family: MATRIX_COMMAND_FAMILY_DESCRIPTOR,
            snapshot_builder: build_matrix_snapshots,
        }),
        selection_order: 30,
        selection_label: "federated room sync bot",
        blurb: "Shipped Matrix surface with direct send and sync-based reply-loop support.",
        implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
        capabilities: MATRIX_CAPABILITIES,
        label: "Matrix",
        aliases: &[],
        transport: "matrix_client_server_sync",
        onboarding: MATRIX_ONBOARDING_DESCRIPTOR,
        operations: MATRIX_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "wecom",
        runtime: Some(ChannelRuntimeDescriptor {
            family: WECOM_COMMAND_FAMILY_DESCRIPTOR,
            snapshot_builder: build_wecom_snapshots,
        }),
        selection_order: 35,
        selection_label: "enterprise aibot",
        blurb: "Shipped WeCom AIBot long-connection surface with proactive send and account-aware runtime state.",
        implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
        capabilities: WECOM_CAPABILITIES,
        label: "WeCom",
        aliases: &["wechat-work", "qywx"],
        transport: "wecom_aibot_long_connection",
        onboarding: WECOM_ONBOARDING_DESCRIPTOR,
        operations: WECOM_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "discord",
        runtime: None,
        selection_order: 40,
        selection_label: "community server bot",
        blurb: "Planned Discord gateway and thread-aware bot surface with documented credential expectations.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "Discord",
        aliases: &["discord-bot"],
        transport: "discord_gateway",
        onboarding: DISCORD_ONBOARDING_DESCRIPTOR,
        operations: DISCORD_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "slack",
        runtime: None,
        selection_order: 50,
        selection_label: "workspace event bot",
        blurb: "Planned Slack Events API or Socket Mode surface with richer bot-token and signing metadata.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "Slack",
        aliases: &["slack-bot"],
        transport: "slack_events_api",
        onboarding: SLACK_ONBOARDING_DESCRIPTOR,
        operations: SLACK_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "line",
        runtime: None,
        selection_order: 60,
        selection_label: "consumer messaging bot",
        blurb: "Planned LINE Messaging API surface for push sends and webhook-driven reply loops.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "LINE",
        aliases: &["line-bot"],
        transport: "line_messaging_api",
        onboarding: LINE_ONBOARDING_DESCRIPTOR,
        operations: LINE_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "dingtalk",
        runtime: None,
        selection_order: 80,
        selection_label: "dingtalk robot app",
        blurb: "Planned DingTalk robot and event-callback surface with explicit app and robot credential metadata.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "DingTalk",
        aliases: &["ding", "ding-bot"],
        transport: "dingtalk_stream_or_callback_api",
        onboarding: DINGTALK_ONBOARDING_DESCRIPTOR,
        operations: DINGTALK_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "whatsapp",
        runtime: None,
        selection_order: 90,
        selection_label: "business messaging app",
        blurb: "Planned WhatsApp Cloud API surface covering outbound business sends and inbound webhook events.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "WhatsApp",
        aliases: &["wa", "whatsapp-cloud"],
        transport: "whatsapp_cloud_api",
        onboarding: WHATSAPP_ONBOARDING_DESCRIPTOR,
        operations: WHATSAPP_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "email",
        runtime: None,
        selection_order: 100,
        selection_label: "mailbox agent",
        blurb: "Planned email surface for SMTP outbound delivery and IMAP-backed reply loops.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "Email",
        aliases: &["smtp", "imap"],
        transport: "smtp_imap",
        onboarding: EMAIL_ONBOARDING_DESCRIPTOR,
        operations: EMAIL_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "webhook",
        runtime: None,
        selection_order: 110,
        selection_label: "generic http integration",
        blurb: "Planned generic webhook surface for outbound POST delivery and signed inbound callback handling.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "Webhook",
        aliases: &["http-webhook"],
        transport: "generic_webhook",
        onboarding: WEBHOOK_ONBOARDING_DESCRIPTOR,
        operations: WEBHOOK_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "google-chat",
        runtime: None,
        selection_order: 120,
        selection_label: "workspace thread bot",
        blurb: "Planned Google Chat surface for space-targeted sends and verified event delivery.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "Google Chat",
        aliases: &["gchat", "googlechat"],
        transport: "google_chat_events_api",
        onboarding: GOOGLE_CHAT_ONBOARDING_DESCRIPTOR,
        operations: GOOGLE_CHAT_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "signal",
        runtime: None,
        selection_order: 130,
        selection_label: "private messenger bridge",
        blurb: "Planned Signal surface for linked-device sends and inbound contact routing.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "Signal",
        aliases: &["signal-cli"],
        transport: "signal_service_bridge",
        onboarding: SIGNAL_ONBOARDING_DESCRIPTOR,
        operations: SIGNAL_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "teams",
        runtime: None,
        selection_order: 140,
        selection_label: "enterprise meeting bot",
        blurb: "Planned Microsoft Teams surface for bot-framework conversations and tenant-scoped routing.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "Microsoft Teams",
        aliases: &["msteams", "ms-teams"],
        transport: "microsoft_teams_bot_framework",
        onboarding: TEAMS_ONBOARDING_DESCRIPTOR,
        operations: TEAMS_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "mattermost",
        runtime: None,
        selection_order: 150,
        selection_label: "self-hosted workspace bot",
        blurb: "Planned Mattermost surface for self-hosted team chat sends and websocket event handling.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "Mattermost",
        aliases: &["mm"],
        transport: "mattermost_websocket_api",
        onboarding: MATTERMOST_ONBOARDING_DESCRIPTOR,
        operations: MATTERMOST_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "nextcloud-talk",
        runtime: None,
        selection_order: 160,
        selection_label: "self-hosted room bot",
        blurb: "Planned Nextcloud Talk surface for room delivery on self-hosted collaboration stacks.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "Nextcloud Talk",
        aliases: &["nextcloud", "nextcloudtalk"],
        transport: "nextcloud_talk_api",
        onboarding: NEXTCLOUD_TALK_ONBOARDING_DESCRIPTOR,
        operations: NEXTCLOUD_TALK_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "synology-chat",
        runtime: None,
        selection_order: 165,
        selection_label: "nas webhook bot",
        blurb: "Planned Synology Chat surface for self-hosted NAS chat delivery through outgoing and incoming webhooks.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "Synology Chat",
        aliases: &["synologychat", "synochat"],
        transport: "synology_chat_outgoing_incoming_webhooks",
        onboarding: SYNOLOGY_CHAT_ONBOARDING_DESCRIPTOR,
        operations: SYNOLOGY_CHAT_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "irc",
        runtime: None,
        selection_order: 170,
        selection_label: "relay and channel bot",
        blurb: "Planned IRC surface for classic channel relays and direct nick interactions.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "IRC",
        aliases: &[],
        transport: "irc_socket",
        onboarding: IRC_ONBOARDING_DESCRIPTOR,
        operations: IRC_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "imessage",
        runtime: None,
        selection_order: 180,
        selection_label: "apple message bridge",
        blurb: "Planned BlueBubbles-backed iMessage surface for Apple message delivery and sync.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "iMessage",
        aliases: &["bluebubbles", "blue-bubbles"],
        transport: "imessage_bridge_api",
        onboarding: IMESSAGE_ONBOARDING_DESCRIPTOR,
        operations: IMESSAGE_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "nostr",
        runtime: None,
        selection_order: 190,
        selection_label: "relay-signed social bot",
        blurb: "Planned Nostr surface for relay publication, inbound subscriptions, and key-based routing.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "Nostr",
        aliases: &[],
        transport: "nostr_relays",
        onboarding: NOSTR_ONBOARDING_DESCRIPTOR,
        operations: NOSTR_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "twitch",
        runtime: None,
        selection_order: 200,
        selection_label: "livestream chat bot",
        blurb: "Planned Twitch surface for stream chat participation and channel-scoped routing.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "Twitch",
        aliases: &["tmi"],
        transport: "twitch_irc_or_eventsub",
        onboarding: TWITCH_ONBOARDING_DESCRIPTOR,
        operations: TWITCH_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "tlon",
        runtime: None,
        selection_order: 205,
        selection_label: "urbit ship bot",
        blurb: "Planned Tlon surface for Urbit DMs and group mentions with ship-backed routing.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "Tlon",
        aliases: &["urbit"],
        transport: "tlon_urbit_ship_api",
        onboarding: TLON_ONBOARDING_DESCRIPTOR,
        operations: TLON_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "zalo",
        runtime: None,
        selection_order: 210,
        selection_label: "official account bot",
        blurb: "Planned Zalo official account surface for business messaging and webhook-backed delivery.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "Zalo",
        aliases: &["zalo-oa"],
        transport: "zalo_official_account_api",
        onboarding: ZALO_ONBOARDING_DESCRIPTOR,
        operations: ZALO_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "zalo-personal",
        runtime: None,
        selection_order: 220,
        selection_label: "personal chat bridge",
        blurb: "Planned Zalo personal bridge surface for direct personal-message automation flows.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "Zalo Personal",
        aliases: &["zalo-pm"],
        transport: "zalo_personal_bridge",
        onboarding: ZALO_PERSONAL_ONBOARDING_DESCRIPTOR,
        operations: ZALO_PERSONAL_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "webchat",
        runtime: None,
        selection_order: 230,
        selection_label: "embedded web inbox",
        blurb: "Planned web chat surface for browser-hosted sessions with signed conversation routing.",
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: PLANNED_CHANNEL_CAPABILITIES,
        label: "WebChat",
        aliases: &["browser-chat", "web-ui"],
        transport: "webchat_websocket",
        onboarding: WEBCHAT_ONBOARDING_DESCRIPTOR,
        operations: WEBCHAT_OPERATIONS,
    },
];

fn find_channel_registry_descriptor(raw: &str) -> Option<&'static ChannelRegistryDescriptor> {
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

fn sorted_channel_registry_descriptors() -> Vec<&'static ChannelRegistryDescriptor> {
    let mut descriptors = CHANNEL_REGISTRY.iter().collect::<Vec<_>>();
    descriptors.sort_by_key(|descriptor| (descriptor.selection_order, descriptor.id));
    descriptors
}

fn channel_catalog_entry_from_descriptor(
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
        supported_target_kinds,
        operations: descriptor
            .operations
            .iter()
            .map(|descriptor| descriptor.operation)
            .collect(),
    }
}

pub fn resolve_channel_onboarding_descriptor(raw: &str) -> Option<ChannelOnboardingDescriptor> {
    find_channel_registry_descriptor(raw).map(|descriptor| descriptor.onboarding)
}

pub fn list_channel_catalog() -> Vec<ChannelCatalogEntry> {
    sorted_channel_registry_descriptors()
        .into_iter()
        .map(channel_catalog_entry_from_descriptor)
        .collect()
}

pub fn normalize_channel_catalog_id(raw: &str) -> Option<&'static str> {
    find_channel_registry_descriptor(raw).map(|descriptor| descriptor.id)
}

pub fn resolve_channel_catalog_entry(raw: &str) -> Option<ChannelCatalogEntry> {
    find_channel_registry_descriptor(raw).map(channel_catalog_entry_from_descriptor)
}

pub fn resolve_channel_catalog_operation(
    raw_channel_id: &str,
    operation_id: &str,
) -> Option<ChannelCatalogOperation> {
    resolve_channel_operation_descriptor(raw_channel_id, operation_id)
        .map(|descriptor| descriptor.operation)
}

pub fn resolve_channel_operation_descriptor(
    raw_channel_id: &str,
    operation_id: &str,
) -> Option<ChannelOperationDescriptor> {
    let descriptor = find_channel_registry_descriptor(raw_channel_id)?
        .operations
        .iter()
        .find(|descriptor| descriptor.operation.id == operation_id)?;
    Some(ChannelOperationDescriptor {
        operation: descriptor.operation,
        doctor: (!descriptor.doctor_checks.is_empty()).then_some(ChannelDoctorOperationSpec {
            checks: descriptor.doctor_checks,
        }),
    })
}

pub fn resolve_channel_doctor_operation_spec(
    raw_channel_id: &str,
    operation_id: &str,
) -> Option<ChannelDoctorOperationSpec> {
    resolve_channel_operation_descriptor(raw_channel_id, operation_id)
        .and_then(|descriptor| descriptor.doctor)
}

pub fn catalog_only_channel_entries(
    snapshots: &[ChannelStatusSnapshot],
) -> Vec<ChannelCatalogEntry> {
    let catalog = list_channel_catalog();
    catalog_only_channel_entries_from(&catalog, snapshots)
}

fn catalog_only_channel_entries_from(
    catalog: &[ChannelCatalogEntry],
    snapshots: &[ChannelStatusSnapshot],
) -> Vec<ChannelCatalogEntry> {
    let snapshot_ids = snapshots
        .iter()
        .map(|snapshot| snapshot.id)
        .collect::<BTreeSet<_>>();
    catalog
        .iter()
        .filter(|entry| !snapshot_ids.contains(entry.id))
        .cloned()
        .collect()
}

pub fn normalize_channel_platform(raw: &str) -> Option<ChannelPlatform> {
    find_channel_registry_descriptor(raw).and_then(|descriptor| {
        descriptor
            .runtime
            .map(|runtime| runtime.family.runtime.platform)
    })
}

pub fn resolve_channel_command_family_descriptor(
    raw: &str,
) -> Option<ChannelCommandFamilyDescriptor> {
    find_channel_registry_descriptor(raw)
        .and_then(|descriptor| descriptor.runtime.map(|runtime| runtime.family))
}

pub fn resolve_channel_catalog_command_family_descriptor(
    raw: &str,
) -> Option<ChannelCatalogCommandFamilyDescriptor> {
    let descriptor = find_channel_registry_descriptor(raw)?;
    let send = descriptor
        .operations
        .iter()
        .find(|descriptor| descriptor.operation.id == CHANNEL_OPERATION_SEND_ID)?
        .operation;
    let serve = descriptor
        .operations
        .iter()
        .find(|descriptor| descriptor.operation.id == CHANNEL_OPERATION_SERVE_ID)?
        .operation;
    Some(ChannelCatalogCommandFamilyDescriptor {
        channel_id: descriptor.id,
        default_send_target_kind: send.default_target_kind()?,
        send,
        serve,
    })
}

pub fn resolve_channel_runtime_command_descriptor(
    raw: &str,
) -> Option<ChannelRuntimeCommandDescriptor> {
    find_channel_registry_descriptor(raw)
        .and_then(|descriptor| descriptor.runtime.map(|runtime| runtime.family.runtime))
}

pub fn channel_inventory(config: &LoongClawConfig) -> ChannelInventory {
    channel_inventory_with_now(
        config,
        runtime_state::default_channel_runtime_state_dir().as_path(),
        now_ms(),
    )
}

pub fn channel_status_snapshots(config: &LoongClawConfig) -> Vec<ChannelStatusSnapshot> {
    channel_status_snapshots_with_now(
        config,
        runtime_state::default_channel_runtime_state_dir().as_path(),
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
    let channel_surfaces = build_channel_surfaces(&channel_catalog, &channels);
    ChannelInventory {
        channels,
        catalog_only_channels,
        channel_catalog,
        channel_surfaces,
    }
}

fn build_channel_surfaces(
    channel_catalog: &[ChannelCatalogEntry],
    channels: &[ChannelStatusSnapshot],
) -> Vec<ChannelSurface> {
    channel_catalog
        .iter()
        .map(|catalog| {
            let configured_accounts = channels
                .iter()
                .filter(|snapshot| snapshot.id == catalog.id)
                .cloned()
                .collect::<Vec<_>>();
            let default_configured_account_id = configured_accounts
                .iter()
                .find(|snapshot| snapshot.is_default_account)
                .map(|snapshot| snapshot.configured_account_id.clone());
            ChannelSurface {
                catalog: catalog.clone(),
                configured_accounts,
                default_configured_account_id,
            }
        })
        .collect()
}

fn channel_status_snapshots_with_now(
    config: &LoongClawConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let mut snapshots = Vec::new();
    for descriptor in runtime_backed_channel_registry_descriptors() {
        let Some(runtime) = descriptor.runtime else {
            continue;
        };
        snapshots.extend((runtime.snapshot_builder)(
            descriptor,
            config,
            runtime_dir,
            now_ms,
        ));
    }
    snapshots
}

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

    let mut serve_issues = send_issues.clone();
    if resolved.allowed_chat_ids.is_empty() {
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

    let mut serve_issues = send_issues.clone();
    if !resolved
        .allowed_chat_ids
        .iter()
        .any(|value| !value.trim().is_empty())
    {
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

    let mut serve_issues = send_issues.clone();
    if !resolved
        .allowed_room_ids
        .iter()
        .any(|value| !value.trim().is_empty())
    {
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

    let mut serve_issues = send_issues.clone();
    let has_allowlist = resolved
        .allowed_conversation_ids
        .iter()
        .any(|value| !value.trim().is_empty());
    if !has_allowlist {
        serve_issues.push("allowed_conversation_ids is empty".to_owned());
    }

    let websocket_url = resolved.resolved_websocket_url();
    let websocket_parse = reqwest::Url::parse(websocket_url.as_str());
    if websocket_parse.is_err() {
        let error = websocket_parse
            .err()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown url parse error".to_owned());
        let issue = format!("websocket_url is invalid: {error}");
        send_issues.push(issue.clone());
        serve_issues.push(issue);
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
        status.runtime = runtime_state::load_channel_operation_runtime_for_account_from_dir(
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
    fn normalize_channel_catalog_id_maps_runtime_and_stub_aliases() {
        assert_eq!(normalize_channel_catalog_id("lark"), Some("feishu"));
        assert_eq!(normalize_channel_catalog_id(" TELEGRAM "), Some("telegram"));
        assert_eq!(normalize_channel_catalog_id("discord-bot"), Some("discord"));
        assert_eq!(normalize_channel_catalog_id("slack"), Some("slack"));
        assert_eq!(normalize_channel_catalog_id("gchat"), Some("google-chat"));
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
            vec!["telegram", "feishu", "matrix", "wecom"]
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
        let wecom =
            resolve_channel_runtime_command_descriptor("wecom").expect("wecom runtime descriptor");

        assert_eq!(telegram.channel_id, "telegram");
        assert_eq!(telegram.platform, ChannelPlatform::Telegram);
        assert_eq!(telegram.serve_bootstrap_agent_id, "channel-telegram");

        assert_eq!(lark.channel_id, "feishu");
        assert_eq!(lark.platform, ChannelPlatform::Feishu);
        assert_eq!(lark.serve_bootstrap_agent_id, "channel-feishu");

        assert_eq!(wecom.channel_id, "wecom");
        assert_eq!(wecom.platform, ChannelPlatform::Wecom);
        assert_eq!(wecom.serve_bootstrap_agent_id, "channel-wecom");
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
    fn resolve_channel_catalog_command_family_descriptor_includes_runtime_and_stub_channels() {
        let telegram = resolve_channel_catalog_command_family_descriptor("telegram")
            .expect("telegram catalog command family");
        let lark = resolve_channel_catalog_command_family_descriptor("lark")
            .expect("lark catalog command family");
        let slack = resolve_channel_catalog_command_family_descriptor("slack-bot")
            .expect("slack alias catalog command family");
        let google_chat = resolve_channel_catalog_command_family_descriptor("gchat")
            .expect("google chat alias catalog command family");
        let synology_chat = resolve_channel_catalog_command_family_descriptor("synochat")
            .expect("synology chat alias catalog command family");
        let imessage = resolve_channel_catalog_command_family_descriptor("bluebubbles")
            .expect("imessage alias catalog command family");
        let tlon = resolve_channel_catalog_command_family_descriptor("urbit")
            .expect("tlon alias catalog command family");

        assert_eq!(telegram.channel_id, "telegram");
        assert_eq!(telegram.send.id, CHANNEL_OPERATION_SEND_ID);
        assert_eq!(telegram.send.command, "telegram-send");
        assert_eq!(telegram.serve.id, CHANNEL_OPERATION_SERVE_ID);
        assert_eq!(telegram.serve.command, "telegram-serve");
        assert_eq!(
            telegram.default_send_target_kind,
            ChannelCatalogTargetKind::Conversation
        );

        assert_eq!(lark.channel_id, "feishu");
        assert_eq!(lark.send.command, "feishu-send");
        assert_eq!(lark.serve.command, "feishu-serve");
        assert_eq!(
            lark.default_send_target_kind,
            ChannelCatalogTargetKind::ReceiveId
        );

        assert_eq!(slack.channel_id, "slack");
        assert_eq!(slack.send.command, "slack-send");
        assert_eq!(slack.serve.command, "slack-serve");
        assert_eq!(
            slack.default_send_target_kind,
            ChannelCatalogTargetKind::Conversation
        );

        assert_eq!(google_chat.channel_id, "google-chat");
        assert_eq!(google_chat.send.command, "google-chat-send");
        assert_eq!(google_chat.serve.command, "google-chat-serve");
        assert_eq!(
            google_chat.default_send_target_kind,
            ChannelCatalogTargetKind::Conversation
        );

        assert_eq!(synology_chat.channel_id, "synology-chat");
        assert_eq!(synology_chat.send.command, "synology-chat-send");
        assert_eq!(synology_chat.serve.command, "synology-chat-serve");
        assert_eq!(
            synology_chat.default_send_target_kind,
            ChannelCatalogTargetKind::Address
        );

        assert_eq!(imessage.channel_id, "imessage");
        assert_eq!(imessage.send.command, "imessage-send");
        assert_eq!(imessage.serve.command, "imessage-serve");
        assert_eq!(
            imessage.default_send_target_kind,
            ChannelCatalogTargetKind::Conversation
        );

        assert_eq!(tlon.channel_id, "tlon");
        assert_eq!(tlon.send.command, "tlon-send");
        assert_eq!(tlon.serve.command, "tlon-serve");
        assert_eq!(
            tlon.default_send_target_kind,
            ChannelCatalogTargetKind::Conversation
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
    fn resolve_channel_catalog_entry_returns_stub_metadata_for_alias_lookup() {
        let discord = resolve_channel_catalog_entry("discord-bot").expect("discord stub entry");
        let encoded = serde_json::to_value(&discord).expect("serialize discord entry");

        assert_eq!(discord.id, "discord");
        assert_eq!(discord.selection_order, 40);
        assert_eq!(discord.selection_label, "community server bot");
        assert!(discord.blurb.contains("Discord gateway"));
        assert_eq!(
            discord.implementation_status,
            ChannelCatalogImplementationStatus::Stub
        );
        assert_eq!(discord.transport, "discord_gateway");
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
            Some(vec!["stub", "stub"])
        );
        assert_eq!(
            encoded
                .get("onboarding")
                .and_then(|onboarding| onboarding.get("strategy"))
                .and_then(serde_json::Value::as_str),
            Some("planned")
        );
    }

    #[test]
    fn resolve_channel_catalog_entry_exposes_onboarding_contracts() {
        let telegram = resolve_channel_catalog_entry("telegram").expect("telegram entry");
        let lark = resolve_channel_catalog_entry("lark").expect("lark entry");
        let discord = resolve_channel_catalog_entry("discord").expect("discord entry");

        assert_eq!(
            telegram.onboarding.strategy,
            ChannelOnboardingStrategy::ManualConfig
        );
        assert_eq!(telegram.onboarding.status_command, "loongclaw doctor");
        assert_eq!(
            telegram.onboarding.repair_command,
            Some("loongclaw doctor --fix")
        );
        assert!(telegram.onboarding.setup_hint.contains("loongclaw.toml"));

        assert_eq!(
            lark.onboarding.strategy,
            ChannelOnboardingStrategy::ManualConfig
        );
        assert_eq!(lark.onboarding.status_command, "loongclaw doctor");

        assert_eq!(
            discord.onboarding.strategy,
            ChannelOnboardingStrategy::Planned
        );
        assert_eq!(discord.onboarding.repair_command, None);
        assert!(
            discord
                .onboarding
                .setup_hint
                .contains("planned Discord gateway")
        );
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
    fn channel_catalog_includes_discord_and_slack_stub_surfaces() {
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
            ChannelCatalogImplementationStatus::Stub
        );
        assert_eq!(discord.transport, "discord_gateway");
        assert_eq!(discord.aliases, vec!["discord-bot"]);
        assert_eq!(discord.selection_order, 40);
        assert_eq!(discord.selection_label, "community server bot");
        assert_eq!(discord.operations.len(), 2);
        assert_eq!(discord.operations[0].command, "discord-send");
        assert_eq!(discord.operations[1].command, "discord-serve");

        assert_eq!(
            slack.implementation_status,
            ChannelCatalogImplementationStatus::Stub
        );
        assert_eq!(slack.transport, "slack_events_api");
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
            Some(vec!["multi_account", "send", "serve", "runtime_tracking"])
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
            Some(vec!["multi_account", "send", "serve", "runtime_tracking"])
        );
    }

    #[test]
    fn channel_catalog_includes_openclaw_inspired_planned_surfaces() {
        let catalog = list_channel_catalog();
        let google_chat = catalog
            .iter()
            .find(|entry| entry.id == "google-chat")
            .expect("google chat catalog entry");
        let signal = catalog
            .iter()
            .find(|entry| entry.id == "signal")
            .expect("signal catalog entry");
        let synology_chat = catalog
            .iter()
            .find(|entry| entry.id == "synology-chat")
            .expect("synology chat catalog entry");
        let imessage = catalog
            .iter()
            .find(|entry| entry.id == "imessage")
            .expect("imessage catalog entry");
        let tlon = catalog
            .iter()
            .find(|entry| entry.id == "tlon")
            .expect("tlon catalog entry");
        let webchat = catalog
            .iter()
            .find(|entry| entry.id == "webchat")
            .expect("webchat catalog entry");

        assert_eq!(
            google_chat.implementation_status,
            ChannelCatalogImplementationStatus::Stub
        );
        assert_eq!(google_chat.selection_order, 120);
        assert_eq!(google_chat.aliases, vec!["gchat", "googlechat"]);
        assert_eq!(google_chat.transport, "google_chat_events_api");
        assert_eq!(
            google_chat.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(google_chat.operations[0].command, "google-chat-send");
        assert_eq!(google_chat.operations[1].command, "google-chat-serve");

        assert_eq!(
            signal.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Address]
        );
        assert_eq!(signal.operations[0].command, "signal-send");
        assert_eq!(signal.operations[1].command, "signal-serve");

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

        assert_eq!(imessage.aliases, vec!["bluebubbles", "blue-bubbles"]);
        assert_eq!(imessage.selection_order, 180);
        assert!(imessage.blurb.contains("BlueBubbles"));

        assert_eq!(tlon.selection_order, 205);
        assert_eq!(tlon.aliases, vec!["urbit"]);
        assert_eq!(tlon.transport, "tlon_urbit_ship_api");
        assert_eq!(
            tlon.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Conversation]
        );
        assert_eq!(tlon.operations[0].command, "tlon-send");
        assert_eq!(tlon.operations[1].command, "tlon-serve");

        assert_eq!(webchat.selection_order, 230);
        assert_eq!(webchat.aliases, vec!["browser-chat", "web-ui"]);
        assert_eq!(webchat.transport, "webchat_websocket");
        assert_eq!(
            webchat.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Conversation]
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
        let google_chat = catalog
            .iter()
            .find(|entry| entry.id == "google-chat")
            .expect("google chat catalog entry");

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
            vec!["enabled", "bot_token", "allowed_chat_ids"]
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
                "verification_token",
                "encrypt_key",
            ]
        );
        assert_eq!(
            feishu.operations[1].requirements[5].default_env_var,
            Some("FEISHU_VERIFICATION_TOKEN")
        );
        assert_eq!(
            feishu.operations[1].requirements[6].default_env_var,
            Some("FEISHU_ENCRYPT_KEY")
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
            google_chat.operations[0]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec!["enabled", "service_account_json", "space_id"]
        );
        assert_eq!(
            google_chat.operations[1]
                .requirements
                .iter()
                .map(|requirement| requirement.id)
                .collect::<Vec<_>>(),
            vec![
                "enabled",
                "service_account_json",
                "allowed_space_ids",
                "verification_token",
            ]
        );
        assert_eq!(
            google_chat.operations[0].requirements[1].default_env_var,
            Some("GOOGLE_CHAT_SERVICE_ACCOUNT_JSON")
        );
        assert_eq!(
            google_chat.operations[1].requirements[3].default_env_var,
            Some("GOOGLE_CHAT_VERIFICATION_TOKEN")
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
        let signal = catalog
            .iter()
            .find(|entry| entry.id == "signal")
            .expect("signal catalog entry");

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
            signal.operations[0].supported_target_kinds,
            &[ChannelCatalogTargetKind::Address]
        );
        assert_eq!(
            signal.operations[1].supported_target_kinds,
            &[ChannelCatalogTargetKind::Address]
        );
    }

    #[test]
    fn channel_catalog_operation_exposes_default_target_kind_from_metadata() {
        let telegram =
            resolve_channel_catalog_operation("telegram", "send").expect("telegram send operation");
        let feishu =
            resolve_channel_catalog_operation("feishu", "send").expect("feishu send operation");
        let webhook =
            resolve_channel_catalog_operation("webhook", "send").expect("webhook send operation");
        let signal =
            resolve_channel_catalog_operation("signal", "send").expect("signal send operation");

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
        let signal = catalog
            .iter()
            .find(|entry| entry.id == "signal")
            .expect("signal catalog entry");

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
            signal.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Address]
        );
    }

    #[test]
    fn catalog_only_channel_entries_include_stub_surfaces_for_default_config() {
        let config = LoongClawConfig::default();
        let snapshots = channel_status_snapshots(&config);
        let catalog_only = catalog_only_channel_entries(&snapshots);
        let discord = catalog_only
            .iter()
            .find(|entry| entry.id == "discord")
            .expect("discord catalog entry");
        let slack = catalog_only
            .iter()
            .find(|entry| entry.id == "slack")
            .expect("slack catalog entry");
        let line = catalog_only
            .iter()
            .find(|entry| entry.id == "line")
            .expect("line catalog entry");
        let webhook = catalog_only
            .iter()
            .find(|entry| entry.id == "webhook")
            .expect("webhook catalog entry");
        let google_chat = catalog_only
            .iter()
            .find(|entry| entry.id == "google-chat")
            .expect("google chat catalog entry");
        let synology_chat = catalog_only
            .iter()
            .find(|entry| entry.id == "synology-chat")
            .expect("synology chat catalog entry");
        let tlon = catalog_only
            .iter()
            .find(|entry| entry.id == "tlon")
            .expect("tlon catalog entry");
        let webchat = catalog_only
            .iter()
            .find(|entry| entry.id == "webchat")
            .expect("webchat catalog entry");

        assert_eq!(
            catalog_only
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec![
                "discord",
                "slack",
                "line",
                "dingtalk",
                "whatsapp",
                "email",
                "webhook",
                "google-chat",
                "signal",
                "teams",
                "mattermost",
                "nextcloud-talk",
                "synology-chat",
                "irc",
                "imessage",
                "nostr",
                "twitch",
                "tlon",
                "zalo",
                "zalo-personal",
                "webchat",
            ]
        );
        assert_eq!(discord.operations[0].command, "discord-send");
        assert_eq!(discord.operations[1].command, "discord-serve");
        assert_eq!(slack.operations[0].command, "slack-send");
        assert_eq!(slack.operations[1].command, "slack-serve");
        assert_eq!(line.operations[0].command, "line-send");
        assert_eq!(webhook.operations[1].command, "webhook-serve");
        assert_eq!(google_chat.operations[0].command, "google-chat-send");
        assert_eq!(synology_chat.operations[1].command, "synology-chat-serve");
        assert_eq!(tlon.operations[0].command, "tlon-send");
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
            vec!["telegram", "feishu", "matrix", "wecom"]
        );
        assert_eq!(
            inventory
                .catalog_only_channels
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec![
                "discord",
                "slack",
                "line",
                "dingtalk",
                "whatsapp",
                "email",
                "webhook",
                "google-chat",
                "signal",
                "teams",
                "mattermost",
                "nextcloud-talk",
                "synology-chat",
                "irc",
                "imessage",
                "nostr",
                "twitch",
                "tlon",
                "zalo",
                "zalo-personal",
                "webchat",
            ]
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
                "discord",
                "slack",
                "line",
                "dingtalk",
                "whatsapp",
                "email",
                "webhook",
                "google-chat",
                "signal",
                "teams",
                "mattermost",
                "nextcloud-talk",
                "synology-chat",
                "irc",
                "imessage",
                "nostr",
                "twitch",
                "tlon",
                "zalo",
                "zalo-personal",
                "webchat",
            ]
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
                "discord",
                "slack",
                "line",
                "dingtalk",
                "whatsapp",
                "email",
                "webhook",
                "google-chat",
                "signal",
                "teams",
                "mattermost",
                "nextcloud-talk",
                "synology-chat",
                "irc",
                "imessage",
                "nostr",
                "twitch",
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
            ChannelCatalogImplementationStatus::Stub
        );
        assert!(discord.configured_accounts.is_empty());
        assert_eq!(discord.default_configured_account_id, None);

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
                supported_target_kinds: vec![ChannelCatalogTargetKind::Conversation],
                operations: vec![
                    ChannelCatalogOperation {
                        id: "send",
                        label: "direct send",
                        command: "telegram-send",
                        availability: ChannelCatalogOperationAvailability::Implemented,
                        tracks_runtime: false,
                        requirements: &[],
                        supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
                    },
                    ChannelCatalogOperation {
                        id: "serve",
                        label: "reply loop",
                        command: "telegram-serve",
                        availability: ChannelCatalogOperationAvailability::Implemented,
                        tracks_runtime: true,
                        requirements: &[],
                        supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
                    },
                ],
            },
            ChannelCatalogEntry {
                id: "discord",
                label: "Discord",
                selection_order: 40,
                selection_label: "community server bot",
                blurb: "Planned Discord gateway and thread-aware bot surface with documented credential expectations.",
                implementation_status: ChannelCatalogImplementationStatus::Stub,
                capabilities: vec![
                    ChannelCapability::Send,
                    ChannelCapability::Serve,
                    ChannelCapability::RuntimeTracking,
                ],
                aliases: vec![],
                transport: "discord_gateway",
                onboarding: DISCORD_ONBOARDING_DESCRIPTOR,
                supported_target_kinds: vec![ChannelCatalogTargetKind::Conversation],
                operations: vec![ChannelCatalogOperation {
                    id: "send",
                    label: "direct send",
                    command: "discord-send",
                    availability: ChannelCatalogOperationAvailability::Stub,
                    tracks_runtime: false,
                    requirements: &[],
                    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
                }],
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
            ChannelCatalogImplementationStatus::Stub
        );
        assert_eq!(catalog_only[0].operations[0].command, "discord-send");
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
        runtime_state::write_runtime_state_for_test(
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
        runtime_state::write_runtime_state_for_test_with_account_and_pid(
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
        runtime_state::write_runtime_state_for_test_with_account_and_pid(
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
        runtime_state::write_runtime_state_for_test_with_account_and_pid(
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
