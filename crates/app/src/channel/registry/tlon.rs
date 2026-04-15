use super::*;

const TLON_SHIP_ENV: &str = "TLON_SHIP";
const TLON_URL_ENV: &str = "TLON_URL";
const TLON_CODE_ENV: &str = "TLON_CODE";

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
const TLON_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    TLON_ENABLED_REQUIREMENT,
    TLON_SHIP_REQUIREMENT,
    TLON_URL_REQUIREMENT,
    TLON_CODE_REQUIREMENT,
];

pub(super) const TLON_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "ship message send",
    command: "tlon-send",
    availability: ChannelCatalogOperationAvailability::Implemented,
    tracks_runtime: false,
    requirements: TLON_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

pub(super) const TLON_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "ship event service",
    command: "tlon-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: TLON_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

pub const TLON_CATALOG_COMMAND_FAMILY_DESCRIPTOR: ChannelCatalogCommandFamilyDescriptor =
    ChannelCatalogCommandFamilyDescriptor {
        channel_id: "tlon",
        default_send_target_kind: ChannelCatalogTargetKind::Conversation,
        send: TLON_SEND_OPERATION,
        serve: TLON_SERVE_OPERATION,
    };

pub(super) const TLON_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: TLON_CATALOG_COMMAND_FAMILY_DESCRIPTOR.send,
        doctor_checks: &[],
    },
    ChannelRegistryOperationDescriptor {
        operation: TLON_CATALOG_COMMAND_FAMILY_DESCRIPTOR.serve,
        doctor_checks: &[],
    },
];

pub(super) const TLON_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor =
    ChannelOnboardingDescriptor {
        strategy: ChannelOnboardingStrategy::ManualConfig,
        setup_hint: "configure a Tlon ship account in loong.toml under tlon or tlon.accounts.<account>; outbound ship sends are shipped for DMs and chat groups, while inbound serve support remains planned",
        status_command: "loong doctor",
        repair_command: Some("loong doctor --fix"),
    };

pub(super) const TLON_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "tlon",
        runtime: None,
        snapshot_builder: Some(tlon_support::build_tlon_snapshots),
        selection_order: 205,
        selection_label: "urbit ship bot",
        blurb: "Shipped Tlon outbound surface with config-backed Urbit DMs and group sends through a ship-backed poke API; inbound serve support remains planned.",
        implementation_status: ChannelCatalogImplementationStatus::ConfigBacked,
        capabilities: CONFIG_BACKED_SEND_CHANNEL_CAPABILITIES,
        label: "Tlon",
        aliases: &["urbit"],
        transport: "tlon_urbit_ship_api",
        onboarding: TLON_ONBOARDING_DESCRIPTOR,
        operations: TLON_OPERATIONS,
    };
