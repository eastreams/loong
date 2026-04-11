use super::*;

const ZALO_APP_ID_ENV: &str = "ZALO_APP_ID";
const ZALO_OA_ACCESS_TOKEN_ENV: &str = "ZALO_OA_ACCESS_TOKEN";
const ZALO_APP_SECRET_ENV: &str = "ZALO_APP_SECRET";
const ZALO_PERSONAL_ACCESS_TOKEN_ENV: &str = "ZALO_PERSONAL_ACCESS_TOKEN";
const WEBCHAT_PUBLIC_BASE_URL_ENV: &str = "WEBCHAT_PUBLIC_BASE_URL";
const WEBCHAT_SESSION_SIGNING_SECRET_ENV: &str = "WEBCHAT_SESSION_SIGNING_SECRET";

const PLANNED_CHANNEL_CAPABILITIES: &[ChannelCapability] = &[
    ChannelCapability::MultiAccount,
    ChannelCapability::Send,
    ChannelCapability::Serve,
    ChannelCapability::RuntimeTracking,
];

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
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const ZALO_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "official account webhook service",
    command: "zalo-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: ZALO_SERVE_REQUIREMENTS,
    default_target_kind: None,
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
    status_command: "loong channels --json",
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
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Address],
};
const ZALO_PERSONAL_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "personal message bridge",
    command: "zalo-personal-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: ZALO_PERSONAL_SERVE_REQUIREMENTS,
    default_target_kind: None,
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
        status_command: "loong channels --json",
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
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};
const WEBCHAT_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "browser session service",
    command: "webchat-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: WEBCHAT_SERVE_REQUIREMENTS,
    default_target_kind: None,
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
    status_command: "loong channels --json",
    repair_command: None,
};

pub(super) const ZALO_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "zalo",
        runtime: None,
        snapshot_builder: None,
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
    };

pub(super) const ZALO_PERSONAL_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "zalo-personal",
        runtime: None,
        snapshot_builder: None,
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
    };

pub(super) const WEBCHAT_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "webchat",
        runtime: None,
        snapshot_builder: None,
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
    };
