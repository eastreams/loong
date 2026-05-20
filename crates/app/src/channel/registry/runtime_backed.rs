use std::path::Path;

use crate::config::{
    ChannelDefaultAccountSelectionSource, FeishuChannelServeMode, LoongConfig,
    ResolvedFeishuChannelConfig, ResolvedLineChannelConfig, ResolvedMatrixChannelConfig,
    ResolvedQqbotChannelConfig, ResolvedTelegramChannelConfig, ResolvedWecomChannelConfig,
    ResolvedWhatsappChannelConfig,
};

use super::status_support::{
    build_invalid_feishu_snapshot, build_invalid_line_snapshot, build_invalid_matrix_snapshot,
    build_invalid_telegram_snapshot, build_invalid_wecom_snapshot, build_invalid_whatsapp_snapshot,
};
use crate::config::{
    FEISHU_APP_ID_ENV, FEISHU_APP_SECRET_ENV, FEISHU_ENCRYPT_KEY_ENV,
    FEISHU_VERIFICATION_TOKEN_ENV, MATRIX_ACCESS_TOKEN_ENV, QQBOT_APP_ID_ENV,
    QQBOT_CLIENT_SECRET_ENV, TELEGRAM_BOT_TOKEN_ENV,
};

use super::*;

pub(super) const TELEGRAM_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "bridge send",
    command: "channels send telegram",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: false,
    requirements: TELEGRAM_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

pub(super) const TELEGRAM_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
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
pub(super) const TELEGRAM_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: TELEGRAM_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: TELEGRAM_SERVE_DOCTOR_CHECKS,
    },
];
pub(super) const TELEGRAM_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::PluginBridge,
    setup_hint: "install and configure a Telegram bridge plugin that declares setup.surface=channel plus telegram bot credentials, allowed chat ids, and optional mention gating before serving the managed bridge surface",
    status_command: "loong doctor",
    repair_command: None,
};

pub(super) const FEISHU_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
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

pub(super) const FEISHU_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
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
pub(super) const FEISHU_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: FEISHU_SEND_DOCTOR_CHECKS,
    },
    ChannelRegistryOperationDescriptor {
        operation: FEISHU_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: FEISHU_SERVE_DOCTOR_CHECKS,
    },
];
pub(super) const FEISHU_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
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

pub(super) const QQBOT_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "gateway send",
    command: "channels send qqbot",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: QQBOT_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

pub(super) const QQBOT_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "gateway serve",
    command: "channels serve qqbot",
    availability: ChannelCatalogOperationAvailability::Implemented,
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

pub(super) const QQBOT_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: QQBOT_SEND_OPERATION,
        doctor_checks: QQBOT_SEND_DOCTOR_CHECKS,
    },
    ChannelRegistryOperationDescriptor {
        operation: QQBOT_SERVE_OPERATION,
        doctor_checks: QQBOT_SERVE_DOCTOR_CHECKS,
    },
];

pub(super) const QQBOT_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::ManualConfig,
    setup_hint: "configure qqbot app credentials plus allowed_peer_ids in loong.toml; Loong owns the native QQ gateway runtime and serves it directly through `channels serve qqbot`",
    status_command: "loong doctor",
    repair_command: Some("loong doctor --fix"),
};

pub(super) const MATRIX_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "bridge send",
    command: "channels send matrix",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: false,
    requirements: MATRIX_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

pub(super) const MATRIX_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
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

pub(super) const WECOM_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "bridge send",
    command: "channels send wecom",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: false,
    requirements: WECOM_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

pub(super) const WECOM_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
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
pub(super) const MATRIX_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: MATRIX_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: MATRIX_SEND_DOCTOR_CHECKS,
    },
    ChannelRegistryOperationDescriptor {
        operation: MATRIX_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: MATRIX_SERVE_DOCTOR_CHECKS,
    },
];
pub(super) const MATRIX_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::PluginBridge,
    setup_hint: "install and configure a Matrix bridge plugin that declares setup.surface=channel plus matrix access tokens, homeserver base url, allowed room ids, and optional mention gating before serving the managed bridge surface",
    status_command: "loong doctor",
    repair_command: None,
};

pub(super) fn build_telegram_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
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
    if !access_policy.has_conversation_restrictions() {
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

pub(super) fn build_feishu_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
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

    let access_policy = ChannelInboundAccessPolicy::from_string_lists(
        resolved.allowed_chat_ids.as_slice(),
        resolved.allowed_sender_ids.as_slice(),
        true,
    );
    let mut serve_issues = send_issues.clone();
    if !access_policy.has_conversation_restrictions() {
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

pub(super) fn build_matrix_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
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
    if !access_policy.has_conversation_restrictions() {
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

pub(super) fn build_wecom_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
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
    if !access_policy.has_conversation_restrictions() {
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

pub(super) fn build_line_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-line");
    let http_policy = super::super::http::outbound_http_policy_from_config(config);
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

fn build_line_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedLineChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: super::super::http::ChannelOutboundHttpPolicy,
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
        api_base_url: api_base_url.as_ref().and_then(|_| {
            super::super::http::redact_endpoint_status_url(resolved_api_base_url.as_str())
        }),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

pub(super) fn build_whatsapp_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-whatsapp");
    let http_policy = super::super::http::outbound_http_policy_from_config(config);
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

fn build_whatsapp_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedWhatsappChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: super::super::http::ChannelOutboundHttpPolicy,
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
        api_base_url: api_base_url.as_ref().and_then(|_| {
            super::super::http::redact_endpoint_status_url(resolved_api_base_url.as_str())
        }),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

pub(super) fn build_qqbot_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = cfg!(feature = "channel-qqbot");
    let default_selection = config.qqbot.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;
    config
        .qqbot
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            match config
                .qqbot
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_qqbot_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    runtime_dir,
                    now_ms,
                ),
                Err(error) => build_invalid_qqbot_snapshot(
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

fn build_qqbot_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedQqbotChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();
    if resolved.app_id().is_none() {
        send_issues.push("app_id is missing".to_owned());
    }
    if resolved.client_secret().is_none() {
        send_issues.push("client_secret is missing".to_owned());
    }

    let mut serve_issues = send_issues.clone();
    let has_allowed_peer_ids = resolved
        .allowed_peer_ids
        .iter()
        .any(|value| !value.trim().is_empty());
    if !has_allowed_peer_ids {
        serve_issues.push("allowed_peer_ids is empty".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            QQBOT_SEND_OPERATION,
            "binary built without feature `channel-qqbot`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            QQBOT_SEND_OPERATION,
            "disabled by qqbot account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(QQBOT_SEND_OPERATION, send_issues)
    } else {
        ready_operation(QQBOT_SEND_OPERATION)
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            QQBOT_SERVE_OPERATION,
            "binary built without feature `channel-qqbot`".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            QQBOT_SERVE_OPERATION,
            "disabled by qqbot account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(QQBOT_SERVE_OPERATION, serve_issues)
    } else {
        ready_operation(QQBOT_SERVE_OPERATION)
    };

    let send_operation = attach_runtime(
        ChannelPlatform::Qqbot,
        QQBOT_SEND_OPERATION,
        send_operation,
        resolved.account.id.as_str(),
        resolved.account.label.as_str(),
        runtime_dir,
        now_ms,
    );
    let serve_operation = attach_runtime(
        ChannelPlatform::Qqbot,
        QQBOT_SERVE_OPERATION,
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
    if !resolved.allowed_peer_ids.is_empty() {
        let allowed_peer_ids = resolved.allowed_peer_ids.join(",");
        notes.push(format!("allowed_peer_ids={allowed_peer_ids}"));
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
        api_base_url: Some("https://api.sgroup.qq.com".to_owned()),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_qqbot_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            QQBOT_SEND_OPERATION,
            "binary built without feature `channel-qqbot`".to_owned(),
        )
    } else {
        misconfigured_operation(QQBOT_SEND_OPERATION, vec![error.clone()])
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            QQBOT_SERVE_OPERATION,
            "binary built without feature `channel-qqbot`".to_owned(),
        )
    } else {
        misconfigured_operation(QQBOT_SERVE_OPERATION, vec![error.clone()])
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
        enabled: true,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}
