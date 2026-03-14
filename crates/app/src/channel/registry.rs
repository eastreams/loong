use std::{collections::BTreeSet, path::Path};

use serde::Serialize;

use crate::config::{
    ChannelDefaultAccountSelectionSource, FEISHU_APP_ID_ENV, FEISHU_APP_SECRET_ENV,
    FEISHU_ENCRYPT_KEY_ENV, FEISHU_VERIFICATION_TOKEN_ENV, LoongClawConfig,
    ResolvedFeishuChannelConfig, ResolvedTelegramChannelConfig, TELEGRAM_BOT_TOKEN_ENV,
};

use super::{ChannelCatalogTargetKind, ChannelOperationRuntime, ChannelPlatform, runtime_state};

pub const CHANNEL_OPERATION_SEND_ID: &str = "send";
pub const CHANNEL_OPERATION_SERVE_ID: &str = "serve";

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
    pub implementation_status: ChannelCatalogImplementationStatus,
    pub capabilities: Vec<ChannelCapability>,
    pub aliases: Vec<&'static str>,
    pub transport: &'static str,
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
    implementation_status: ChannelCatalogImplementationStatus,
    capabilities: &'static [ChannelCapability],
    label: &'static str,
    aliases: &'static [&'static str],
    transport: &'static str,
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
    label: "webhook reply server",
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
const FEISHU_VERIFICATION_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "verification_token",
        label: "verification token",
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
        label: "encrypt key",
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
        name: "feishu webhook verification",
        trigger: ChannelDoctorCheckTrigger::OperationHealth,
    },
    ChannelDoctorCheckSpec {
        name: "feishu webhook runtime",
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

const DISCORD_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "direct send",
    command: "discord-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: &[],
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const DISCORD_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "gateway reply loop",
    command: "discord-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: &[],
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
const DISCORD_CAPABILITIES: &[ChannelCapability] = &[
    ChannelCapability::Send,
    ChannelCapability::Serve,
    ChannelCapability::RuntimeTracking,
];

const SLACK_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "direct send",
    command: "slack-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: &[],
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const SLACK_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "events reply loop",
    command: "slack-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: &[],
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
const SLACK_CAPABILITIES: &[ChannelCapability] = &[
    ChannelCapability::Send,
    ChannelCapability::Serve,
    ChannelCapability::RuntimeTracking,
];

const CHANNEL_REGISTRY: &[ChannelRegistryDescriptor] = &[
    ChannelRegistryDescriptor {
        id: "telegram",
        runtime: Some(ChannelRuntimeDescriptor {
            family: TELEGRAM_COMMAND_FAMILY_DESCRIPTOR,
            snapshot_builder: build_telegram_snapshots,
        }),
        implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
        capabilities: TELEGRAM_CAPABILITIES,
        label: "Telegram",
        aliases: &[],
        transport: "telegram_bot_api_polling",
        operations: TELEGRAM_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "feishu",
        runtime: Some(ChannelRuntimeDescriptor {
            family: FEISHU_COMMAND_FAMILY_DESCRIPTOR,
            snapshot_builder: build_feishu_snapshots,
        }),
        implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
        capabilities: FEISHU_CAPABILITIES,
        label: "Feishu/Lark",
        aliases: &["lark"],
        transport: "feishu_openapi_webhook",
        operations: FEISHU_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "discord",
        runtime: None,
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: DISCORD_CAPABILITIES,
        label: "Discord",
        aliases: &["discord-bot"],
        transport: "discord_gateway",
        operations: DISCORD_OPERATIONS,
    },
    ChannelRegistryDescriptor {
        id: "slack",
        runtime: None,
        implementation_status: ChannelCatalogImplementationStatus::Stub,
        capabilities: SLACK_CAPABILITIES,
        label: "Slack",
        aliases: &["slack-bot"],
        transport: "slack_events_api",
        operations: SLACK_OPERATIONS,
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
        implementation_status: descriptor.implementation_status,
        capabilities: descriptor.capabilities.to_vec(),
        aliases: descriptor.aliases.to_vec(),
        transport: descriptor.transport,
        supported_target_kinds,
        operations: descriptor
            .operations
            .iter()
            .map(|descriptor| descriptor.operation)
            .collect(),
    }
}

pub fn list_channel_catalog() -> Vec<ChannelCatalogEntry> {
    CHANNEL_REGISTRY
        .iter()
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
    CHANNEL_REGISTRY
        .iter()
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
    if resolved.verification_token().is_none() {
        serve_issues.push("verification_token is missing".to_owned());
    }
    if resolved.encrypt_key().is_none() {
        serve_issues.push("encrypt_key is missing".to_owned());
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
        format!("receive_id_type={}", resolved.receive_id_type),
        format!("webhook_bind={}", resolved.webhook_bind),
        format!("webhook_path={}", resolved.webhook_path),
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
        api_base_url: Some(resolved.resolved_base_url()),
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
            vec!["telegram", "feishu"]
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

        assert_eq!(telegram.channel_id, "telegram");
        assert_eq!(telegram.platform, ChannelPlatform::Telegram);
        assert_eq!(telegram.serve_bootstrap_agent_id, "channel-telegram");

        assert_eq!(lark.channel_id, "feishu");
        assert_eq!(lark.platform, ChannelPlatform::Feishu);
        assert_eq!(lark.serve_bootstrap_agent_id, "channel-feishu");
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
            vec!["feishu webhook verification", "feishu webhook runtime"]
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
                    "feishu webhook verification",
                    ChannelDoctorCheckTrigger::OperationHealth,
                ),
                (
                    "feishu webhook runtime",
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
            discord.implementation_status,
            ChannelCatalogImplementationStatus::Stub
        );
        assert_eq!(discord.transport, "discord_gateway");
        assert_eq!(discord.aliases, vec!["discord-bot"]);
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
            discord_json
                .get("capabilities")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .collect::<Vec<_>>()
                }),
            Some(vec!["send", "serve", "runtime_tracking"])
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
            Some(vec!["send", "serve", "runtime_tracking"])
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
                "allowed_chat_ids",
                "verification_token",
                "encrypt_key",
            ]
        );
        assert_eq!(
            feishu.operations[1].requirements[4].default_env_var,
            Some("FEISHU_VERIFICATION_TOKEN")
        );
        assert_eq!(
            feishu.operations[1].requirements[5].default_env_var,
            Some("FEISHU_ENCRYPT_KEY")
        );

        assert!(
            discord
                .operations
                .iter()
                .all(|operation| operation.requirements.is_empty())
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
    }

    #[test]
    fn channel_catalog_operation_exposes_default_target_kind_from_metadata() {
        let telegram =
            resolve_channel_catalog_operation("telegram", "send").expect("telegram send operation");
        let feishu =
            resolve_channel_catalog_operation("feishu", "send").expect("feishu send operation");

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
    }

    #[test]
    fn catalog_only_channel_entries_include_stub_surfaces_for_default_config() {
        let config = LoongClawConfig::default();
        let snapshots = channel_status_snapshots(&config);
        let catalog_only = catalog_only_channel_entries(&snapshots);

        assert_eq!(
            catalog_only
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec!["discord", "slack"]
        );
        assert_eq!(catalog_only[0].operations[0].command, "discord-send");
        assert_eq!(catalog_only[0].operations[1].command, "discord-serve");
        assert_eq!(catalog_only[1].operations[0].command, "slack-send");
        assert_eq!(catalog_only[1].operations[1].command, "slack-serve");
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
            vec!["telegram", "feishu"]
        );
        assert_eq!(
            inventory
                .catalog_only_channels
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec!["discord", "slack"]
        );
        assert_eq!(
            inventory
                .channel_catalog
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec!["telegram", "feishu", "discord", "slack"]
        );
    }

    #[test]
    fn channel_inventory_exposes_grouped_channel_surfaces() {
        let config = LoongClawConfig::default();
        let inventory = channel_inventory(&config);

        assert_eq!(
            inventory
                .channel_surfaces
                .iter()
                .map(|surface| surface.catalog.id)
                .collect::<Vec<_>>(),
            vec!["telegram", "feishu", "discord", "slack"]
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
    }

    #[test]
    fn catalog_only_channel_entries_skip_platforms_that_already_have_status_snapshots() {
        let catalog = vec![
            ChannelCatalogEntry {
                id: "telegram",
                label: "Telegram",
                implementation_status: ChannelCatalogImplementationStatus::RuntimeBacked,
                capabilities: vec![
                    ChannelCapability::RuntimeBacked,
                    ChannelCapability::Send,
                    ChannelCapability::Serve,
                    ChannelCapability::RuntimeTracking,
                ],
                aliases: vec![],
                transport: "telegram_bot_api_polling",
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
                implementation_status: ChannelCatalogImplementationStatus::Stub,
                capabilities: vec![
                    ChannelCapability::Send,
                    ChannelCapability::Serve,
                    ChannelCapability::RuntimeTracking,
                ],
                aliases: vec![],
                transport: "discord_gateway",
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
        config.telegram.bot_token = Some("123456:token".to_owned());
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
        config.telegram.bot_token = Some("123456:token".to_owned());

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
        config.feishu.app_id = Some("app-id".to_owned());
        config.feishu.app_secret = Some("app-secret".to_owned());

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
    fn channel_status_snapshots_merge_runtime_activity_for_serve_operations() {
        let mut config = LoongClawConfig::default();
        config.feishu.enabled = true;
        config.feishu.app_id = Some("app-id".to_owned());
        config.feishu.app_secret = Some("app-secret".to_owned());
        config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
        config.feishu.verification_token = Some("token".to_owned());
        config.feishu.encrypt_key = Some("encrypt".to_owned());

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
        config.telegram.bot_token = Some("123456:token".to_owned());
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
        config.telegram.bot_token = Some("123456:token".to_owned());
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
        config.feishu.app_id = Some("cli_a1b2c3".to_owned());
        config.feishu.app_secret = Some("app-secret".to_owned());
        config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
        config.feishu.verification_token = Some("token".to_owned());
        config.feishu.encrypt_key = Some("encrypt".to_owned());
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
        config.feishu.app_id = Some("cli_a1b2c3".to_owned());
        config.feishu.app_secret = Some("app-secret".to_owned());
        config.feishu.allowed_chat_ids = vec!["oc_123".to_owned()];
        config.feishu.verification_token = Some("token".to_owned());
        config.feishu.encrypt_key = Some("encrypt".to_owned());

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
        config.telegram.bot_token = Some("123456:token".to_owned());
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
