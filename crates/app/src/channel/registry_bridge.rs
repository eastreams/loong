use std::path::Path;

use crate::channel::http;
use crate::config::{
    ChannelDefaultAccountSelectionSource, LoongClawConfig, ONEBOT_ACCESS_TOKEN_ENV,
    ONEBOT_WEBSOCKET_URL_ENV, QQBOT_APP_ID_ENV, QQBOT_CLIENT_SECRET_ENV,
    ResolvedOnebotChannelConfig, ResolvedQqbotChannelConfig, ResolvedWeixinChannelConfig,
    WEIXIN_BRIDGE_ACCESS_TOKEN_ENV, WEIXIN_BRIDGE_URL_ENV, normalize_channel_account_id,
};

use super::{
    CHANNEL_OPERATION_SEND_ID, CHANNEL_OPERATION_SERVE_ID, ChannelCatalogImplementationStatus,
    ChannelCatalogOperation, ChannelCatalogOperationAvailability,
    ChannelCatalogOperationRequirement, ChannelCatalogTargetKind, ChannelDoctorCheckSpec,
    ChannelDoctorCheckTrigger, ChannelOnboardingDescriptor, ChannelOnboardingStrategy,
    ChannelPluginBridgeStableTarget, ChannelRegistryDescriptor, ChannelRegistryOperationDescriptor,
    ChannelStatusSnapshot, PLUGIN_BACKED_CHANNEL_CAPABILITIES, disabled_operation,
    misconfigured_operation, unsupported_operation, validate_http_url, validate_websocket_url,
};

const WEIXIN_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["weixin.enabled", "weixin.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };

const WEIXIN_BRIDGE_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "bridge_url",
        label: "clawbot bridge url",
        config_paths: &["weixin.bridge_url", "weixin.accounts.<account>.bridge_url"],
        env_pointer_paths: &[
            "weixin.bridge_url_env",
            "weixin.accounts.<account>.bridge_url_env",
        ],
        default_env_var: Some(WEIXIN_BRIDGE_URL_ENV),
    };

const WEIXIN_BRIDGE_ACCESS_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "bridge_access_token",
        label: "bridge access token",
        config_paths: &[
            "weixin.bridge_access_token",
            "weixin.accounts.<account>.bridge_access_token",
        ],
        env_pointer_paths: &[
            "weixin.bridge_access_token_env",
            "weixin.accounts.<account>.bridge_access_token_env",
        ],
        default_env_var: Some(WEIXIN_BRIDGE_ACCESS_TOKEN_ENV),
    };

const WEIXIN_ALLOWED_CONTACT_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_contact_ids",
        label: "allowed contact ids",
        config_paths: &[
            "weixin.allowed_contact_ids",
            "weixin.accounts.<account>.allowed_contact_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };

const WEIXIN_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    WEIXIN_ENABLED_REQUIREMENT,
    WEIXIN_BRIDGE_URL_REQUIREMENT,
    WEIXIN_BRIDGE_ACCESS_TOKEN_REQUIREMENT,
];

const WEIXIN_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    WEIXIN_ENABLED_REQUIREMENT,
    WEIXIN_BRIDGE_URL_REQUIREMENT,
    WEIXIN_BRIDGE_ACCESS_TOKEN_REQUIREMENT,
    WEIXIN_ALLOWED_CONTACT_IDS_REQUIREMENT,
];

const WEIXIN_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "bridge send",
    command: "weixin-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: WEIXIN_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const WEIXIN_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "bridge reply loop",
    command: "weixin-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: WEIXIN_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const WEIXIN_SEND_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[ChannelDoctorCheckSpec {
    name: "weixin bridge send contract",
    trigger: ChannelDoctorCheckTrigger::PluginBridgeHealth,
}];

const WEIXIN_SERVE_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[ChannelDoctorCheckSpec {
    name: "weixin bridge serve contract",
    trigger: ChannelDoctorCheckTrigger::PluginBridgeHealth,
}];

const WEIXIN_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: WEIXIN_SEND_OPERATION,
        doctor_checks: WEIXIN_SEND_DOCTOR_CHECKS,
    },
    ChannelRegistryOperationDescriptor {
        operation: WEIXIN_SERVE_OPERATION,
        doctor_checks: WEIXIN_SERVE_DOCTOR_CHECKS,
    },
];

const WEIXIN_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::PluginBridge,
    setup_hint: "plugin-bridge weixin surface; connect a compatible WeChat ClawBot or iLink bridge under weixin or weixin.accounts.<account> and let that bridge own the upstream login flow until a native LoongClaw adapter exists",
    status_command: "loong doctor",
    repair_command: None,
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
    label: "gateway send",
    command: "qqbot-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: QQBOT_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const QQBOT_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "gateway reply loop",
    command: "qqbot-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: QQBOT_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const QQBOT_SEND_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[ChannelDoctorCheckSpec {
    name: "qqbot bridge send contract",
    trigger: ChannelDoctorCheckTrigger::PluginBridgeHealth,
}];

const QQBOT_SERVE_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[ChannelDoctorCheckSpec {
    name: "qqbot bridge serve contract",
    trigger: ChannelDoctorCheckTrigger::PluginBridgeHealth,
}];

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
    setup_hint: "plugin-bridge qqbot surface; connect an official QQ Bot gateway or compatible plugin bridge under qqbot or qqbot.accounts.<account> and keep target routing stable across c2c, group, and guild-style conversations",
    status_command: "loong doctor",
    repair_command: None,
};

const ONEBOT_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &["onebot.enabled", "onebot.accounts.<account>.enabled"],
        env_pointer_paths: &[],
        default_env_var: None,
    };

const ONEBOT_WEBSOCKET_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "websocket_url",
        label: "onebot websocket url",
        config_paths: &[
            "onebot.websocket_url",
            "onebot.accounts.<account>.websocket_url",
        ],
        env_pointer_paths: &[
            "onebot.websocket_url_env",
            "onebot.accounts.<account>.websocket_url_env",
        ],
        default_env_var: Some(ONEBOT_WEBSOCKET_URL_ENV),
    };

const ONEBOT_ACCESS_TOKEN_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "access_token",
        label: "onebot access token",
        config_paths: &[
            "onebot.access_token",
            "onebot.accounts.<account>.access_token",
        ],
        env_pointer_paths: &[
            "onebot.access_token_env",
            "onebot.accounts.<account>.access_token_env",
        ],
        default_env_var: Some(ONEBOT_ACCESS_TOKEN_ENV),
    };

const ONEBOT_ALLOWED_GROUP_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_group_ids",
        label: "allowed group ids",
        config_paths: &[
            "onebot.allowed_group_ids",
            "onebot.accounts.<account>.allowed_group_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };

const ONEBOT_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    ONEBOT_ENABLED_REQUIREMENT,
    ONEBOT_WEBSOCKET_URL_REQUIREMENT,
    ONEBOT_ACCESS_TOKEN_REQUIREMENT,
];

const ONEBOT_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    ONEBOT_ENABLED_REQUIREMENT,
    ONEBOT_WEBSOCKET_URL_REQUIREMENT,
    ONEBOT_ACCESS_TOKEN_REQUIREMENT,
    ONEBOT_ALLOWED_GROUP_IDS_REQUIREMENT,
];

const ONEBOT_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "bridge send",
    command: "onebot-send",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: false,
    requirements: ONEBOT_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const ONEBOT_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "bridge event loop",
    command: "onebot-serve",
    availability: ChannelCatalogOperationAvailability::Stub,
    tracks_runtime: true,
    requirements: ONEBOT_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const ONEBOT_SEND_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[ChannelDoctorCheckSpec {
    name: "onebot bridge send contract",
    trigger: ChannelDoctorCheckTrigger::PluginBridgeHealth,
}];

const ONEBOT_SERVE_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[ChannelDoctorCheckSpec {
    name: "onebot bridge serve contract",
    trigger: ChannelDoctorCheckTrigger::PluginBridgeHealth,
}];

const ONEBOT_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: ONEBOT_SEND_OPERATION,
        doctor_checks: ONEBOT_SEND_DOCTOR_CHECKS,
    },
    ChannelRegistryOperationDescriptor {
        operation: ONEBOT_SERVE_OPERATION,
        doctor_checks: ONEBOT_SERVE_DOCTOR_CHECKS,
    },
];

const ONEBOT_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor = ChannelOnboardingDescriptor {
    strategy: ChannelOnboardingStrategy::PluginBridge,
    setup_hint: "plugin-bridge OneBot surface; connect a OneBot-compatible bridge such as NapCat or LLOneBot under onebot or onebot.accounts.<account> and use this surface as the stable protocol contract until a native adapter exists",
    status_command: "loong doctor",
    repair_command: None,
};

const EMPTY_PLUGIN_BRIDGE_STABLE_TARGETS: &[ChannelPluginBridgeStableTarget] = &[];

const WEIXIN_PLUGIN_BRIDGE_STABLE_TARGETS: &[ChannelPluginBridgeStableTarget] = &[
    ChannelPluginBridgeStableTarget {
        template: "weixin:<account>:contact:<id>",
        target_kind: ChannelCatalogTargetKind::Conversation,
        description: "direct contact conversation",
    },
    ChannelPluginBridgeStableTarget {
        template: "weixin:<account>:room:<id>",
        target_kind: ChannelCatalogTargetKind::Conversation,
        description: "group room conversation",
    },
];

const QQBOT_PLUGIN_BRIDGE_STABLE_TARGETS: &[ChannelPluginBridgeStableTarget] = &[
    ChannelPluginBridgeStableTarget {
        template: "qqbot:<account>:c2c:<openid>",
        target_kind: ChannelCatalogTargetKind::Conversation,
        description: "direct message openid",
    },
    ChannelPluginBridgeStableTarget {
        template: "qqbot:<account>:group:<openid>",
        target_kind: ChannelCatalogTargetKind::Conversation,
        description: "group openid",
    },
    ChannelPluginBridgeStableTarget {
        template: "qqbot:<account>:channel:<id>",
        target_kind: ChannelCatalogTargetKind::Conversation,
        description: "guild channel id",
    },
];

const ONEBOT_PLUGIN_BRIDGE_STABLE_TARGETS: &[ChannelPluginBridgeStableTarget] = &[
    ChannelPluginBridgeStableTarget {
        template: "onebot:<account>:private:<user_id>",
        target_kind: ChannelCatalogTargetKind::Conversation,
        description: "private conversation user id",
    },
    ChannelPluginBridgeStableTarget {
        template: "onebot:<account>:group:<group_id>",
        target_kind: ChannelCatalogTargetKind::Conversation,
        description: "group conversation id",
    },
];

const QQBOT_PLUGIN_BRIDGE_ACCOUNT_SCOPE_NOTE: &str =
    "openids are scoped to the selected qq bot account";

const ONEBOT_PLUGIN_BRIDGE_ACCOUNT_SCOPE_NOTE: &str =
    "keep <account> stable so personal-account bridge routes stay unambiguous";

pub(super) const WEIXIN_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "weixin",
        runtime: None,
        snapshot_builder: Some(build_weixin_snapshots),
        selection_order: 36,
        selection_label: "wechat clawbot bridge",
        blurb: "Plugin-backed Weixin surface for ClawBot-compatible personal-chat bridges and stable route semantics.",
        implementation_status: ChannelCatalogImplementationStatus::PluginBacked,
        capabilities: PLUGIN_BACKED_CHANNEL_CAPABILITIES,
        label: "Weixin",
        aliases: &["wechat", "wx", "wechat-clawbot"],
        transport: "wechat_clawbot_ilink_bridge",
        onboarding: WEIXIN_ONBOARDING_DESCRIPTOR,
        operations: WEIXIN_OPERATIONS,
    };

pub(super) const QQBOT_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "qqbot",
        runtime: None,
        snapshot_builder: Some(build_qqbot_snapshots),
        selection_order: 37,
        selection_label: "qq gateway bot",
        blurb: "Plugin-backed QQ Bot surface for official gateway bots and compatible bridge plugins.",
        implementation_status: ChannelCatalogImplementationStatus::PluginBacked,
        capabilities: PLUGIN_BACKED_CHANNEL_CAPABILITIES,
        label: "QQ Bot",
        aliases: &["qq", "qq-bot", "tencent-qq"],
        transport: "qq_official_bot_gateway_or_plugin_bridge",
        onboarding: QQBOT_ONBOARDING_DESCRIPTOR,
        operations: QQBOT_OPERATIONS,
    };

pub(super) const ONEBOT_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "onebot",
        runtime: None,
        snapshot_builder: Some(build_onebot_snapshots),
        selection_order: 38,
        selection_label: "protocol bridge relay",
        blurb: "Plugin-backed OneBot surface for QQ and personal-account bridge ecosystems that already speak OneBot v11.",
        implementation_status: ChannelCatalogImplementationStatus::PluginBacked,
        capabilities: PLUGIN_BACKED_CHANNEL_CAPABILITIES,
        label: "OneBot",
        aliases: &["onebot-v11", "napcat", "llonebot"],
        transport: "onebot_v11_bridge",
        onboarding: ONEBOT_ONBOARDING_DESCRIPTOR,
        operations: ONEBOT_OPERATIONS,
    };

pub(super) fn plugin_bridge_stable_targets_for_channel_id(
    channel_id: &str,
) -> &'static [ChannelPluginBridgeStableTarget] {
    match channel_id {
        "weixin" => WEIXIN_PLUGIN_BRIDGE_STABLE_TARGETS,
        "qqbot" => QQBOT_PLUGIN_BRIDGE_STABLE_TARGETS,
        "onebot" => ONEBOT_PLUGIN_BRIDGE_STABLE_TARGETS,
        _ => EMPTY_PLUGIN_BRIDGE_STABLE_TARGETS,
    }
}

pub(super) fn plugin_bridge_account_scope_note_for_channel_id(
    channel_id: &str,
) -> Option<&'static str> {
    match channel_id {
        "qqbot" => Some(QQBOT_PLUGIN_BRIDGE_ACCOUNT_SCOPE_NOTE),
        "onebot" => Some(ONEBOT_PLUGIN_BRIDGE_ACCOUNT_SCOPE_NOTE),
        _ => None,
    }
}

fn build_weixin_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = true;
    let http_policy = http::outbound_http_policy_from_config(config);
    let default_selection = config.weixin.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;

    config
        .weixin
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            let configured_enabled =
                configured_weixin_account_enabled(config, configured_account_id.as_str());
            match config
                .weixin
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_weixin_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                ),
                Err(error) => build_invalid_weixin_snapshot(
                    descriptor,
                    compiled,
                    configured_enabled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                ),
            }
        })
        .collect()
}

fn build_qqbot_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = true;
    let default_selection = config.qqbot.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;

    config
        .qqbot
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            let configured_enabled =
                configured_qqbot_account_enabled(config, configured_account_id.as_str());
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
                ),
                Err(error) => build_invalid_qqbot_snapshot(
                    descriptor,
                    compiled,
                    configured_enabled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                ),
            }
        })
        .collect()
}

fn build_onebot_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongClawConfig,
    _runtime_dir: &Path,
    _now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = true;
    let default_selection = config.onebot.default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;

    config
        .onebot
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            let configured_enabled =
                configured_onebot_account_enabled(config, configured_account_id.as_str());
            match config
                .onebot
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_onebot_snapshot_for_account(
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                ),
                Err(error) => build_invalid_onebot_snapshot(
                    descriptor,
                    compiled,
                    configured_enabled,
                    configured_account_id.as_str(),
                    is_default_account,
                    default_account_source,
                    error,
                ),
            }
        })
        .collect()
}

fn build_weixin_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedWeixinChannelConfig,
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
        .and_then(|url| validate_http_url("bridge_url", url, http_policy, &mut send_issues));

    let bridge_access_token = resolved.bridge_access_token();
    if bridge_access_token.is_none() {
        send_issues.push("bridge_access_token is missing".to_owned());
    }

    let mut serve_issues = send_issues.clone();
    let has_allowed_contact_ids = resolved
        .allowed_contact_ids
        .iter()
        .any(|value| !value.trim().is_empty());
    if !has_allowed_contact_ids {
        serve_issues.push("allowed_contact_ids is empty".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            WEIXIN_SEND_OPERATION,
            "weixin bridge surface is unavailable in this build".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            WEIXIN_SEND_OPERATION,
            "disabled by weixin account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(WEIXIN_SEND_OPERATION, send_issues)
    } else {
        unsupported_operation(
            WEIXIN_SEND_OPERATION,
            "weixin-send is reserved for the surface contract; outbound delivery still belongs to an external bridge plugin".to_owned(),
        )
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            WEIXIN_SERVE_OPERATION,
            "weixin bridge surface is unavailable in this build".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            WEIXIN_SERVE_OPERATION,
            "disabled by weixin account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(WEIXIN_SERVE_OPERATION, serve_issues)
    } else {
        unsupported_operation(
            WEIXIN_SERVE_OPERATION,
            "weixin-serve is reserved for the surface contract; the reply loop still belongs to an external bridge plugin".to_owned(),
        )
    };

    let allowed_contact_ids_count = resolved
        .allowed_contact_ids
        .iter()
        .filter(|value| !value.trim().is_empty())
        .count();

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
        "bridge_runtime_owner=external_plugin".to_owned(),
    ];
    if bridge_access_token.is_some() {
        notes.push("bridge_access_token_configured=true".to_owned());
    }
    if allowed_contact_ids_count > 0 {
        notes.push(format!(
            "allowed_contact_ids_count={allowed_contact_ids_count}"
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
        api_base_url: validated_bridge_url
            .as_ref()
            .and(bridge_url.as_deref())
            .and_then(http::redact_endpoint_status_url),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_qqbot_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedQqbotChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();

    let app_id = resolved.app_id();
    if app_id.is_none() {
        send_issues.push("app_id is missing".to_owned());
    }

    let client_secret = resolved.client_secret();
    if client_secret.is_none() {
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
            "qqbot bridge surface is unavailable in this build".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            QQBOT_SEND_OPERATION,
            "disabled by qqbot account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(QQBOT_SEND_OPERATION, send_issues)
    } else {
        unsupported_operation(
            QQBOT_SEND_OPERATION,
            "qqbot-send is reserved for the surface contract; outbound delivery still belongs to the external gateway or bridge plugin".to_owned(),
        )
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            QQBOT_SERVE_OPERATION,
            "qqbot bridge surface is unavailable in this build".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            QQBOT_SERVE_OPERATION,
            "disabled by qqbot account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(QQBOT_SERVE_OPERATION, serve_issues)
    } else {
        unsupported_operation(
            QQBOT_SERVE_OPERATION,
            "qqbot-serve is reserved for the surface contract; the reply loop still belongs to the external gateway or bridge plugin".to_owned(),
        )
    };

    let allowed_peer_ids_count = resolved
        .allowed_peer_ids
        .iter()
        .filter(|value| !value.trim().is_empty())
        .count();

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
        "bridge_runtime_owner=external_plugin".to_owned(),
    ];
    if let Some(app_id_value) = &app_id {
        notes.push(format!("app_id={app_id_value}"));
    }
    if client_secret.is_some() {
        notes.push("client_secret_configured=true".to_owned());
    }
    if allowed_peer_ids_count > 0 {
        notes.push(format!("allowed_peer_ids_count={allowed_peer_ids_count}"));
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
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_onebot_snapshot_for_account(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedOnebotChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();

    let websocket_url = resolved.websocket_url();
    if websocket_url.is_none() {
        send_issues.push("websocket_url is missing".to_owned());
    }
    if let Some(websocket_url_value) = websocket_url.as_deref() {
        validate_websocket_url("websocket_url", websocket_url_value, &mut send_issues);
    }

    let access_token = resolved.access_token();
    if access_token.is_none() {
        send_issues.push("access_token is missing".to_owned());
    }

    let mut serve_issues = send_issues.clone();
    let has_allowed_group_ids = resolved
        .allowed_group_ids
        .iter()
        .any(|value| !value.trim().is_empty());
    if !has_allowed_group_ids {
        serve_issues.push("allowed_group_ids is empty".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            ONEBOT_SEND_OPERATION,
            "onebot bridge surface is unavailable in this build".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            ONEBOT_SEND_OPERATION,
            "disabled by onebot account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(ONEBOT_SEND_OPERATION, send_issues)
    } else {
        unsupported_operation(
            ONEBOT_SEND_OPERATION,
            "onebot-send is reserved for the surface contract; outbound delivery still belongs to the external bridge runtime".to_owned(),
        )
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            ONEBOT_SERVE_OPERATION,
            "onebot bridge surface is unavailable in this build".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            ONEBOT_SERVE_OPERATION,
            "disabled by onebot account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(ONEBOT_SERVE_OPERATION, serve_issues)
    } else {
        unsupported_operation(
            ONEBOT_SERVE_OPERATION,
            "onebot-serve is reserved for the surface contract; the event loop still belongs to the external bridge runtime".to_owned(),
        )
    };

    let allowed_group_ids_count = resolved
        .allowed_group_ids
        .iter()
        .filter(|value| !value.trim().is_empty())
        .count();

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
        "bridge_runtime_owner=external_plugin".to_owned(),
    ];
    if access_token.is_some() {
        notes.push("access_token_configured=true".to_owned());
    }
    if allowed_group_ids_count > 0 {
        notes.push(format!("allowed_group_ids_count={allowed_group_ids_count}"));
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
        api_base_url: websocket_url
            .as_deref()
            .and_then(http::redact_endpoint_status_url),
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn configured_weixin_account_enabled(
    config: &LoongClawConfig,
    configured_account_id: &str,
) -> bool {
    let account_enabled = config
        .weixin
        .accounts
        .iter()
        .find_map(|(raw_account_id, account)| {
            let normalized_account_id = normalize_channel_account_id(raw_account_id);
            if normalized_account_id != configured_account_id {
                return None;
            }
            Some(account.enabled.unwrap_or(true))
        })
        .unwrap_or(true);
    config.weixin.enabled && account_enabled
}

fn configured_qqbot_account_enabled(config: &LoongClawConfig, configured_account_id: &str) -> bool {
    let account_enabled = config
        .qqbot
        .accounts
        .iter()
        .find_map(|(raw_account_id, account)| {
            let normalized_account_id = normalize_channel_account_id(raw_account_id);
            if normalized_account_id != configured_account_id {
                return None;
            }
            Some(account.enabled.unwrap_or(true))
        })
        .unwrap_or(true);
    config.qqbot.enabled && account_enabled
}

fn configured_onebot_account_enabled(
    config: &LoongClawConfig,
    configured_account_id: &str,
) -> bool {
    let account_enabled = config
        .onebot
        .accounts
        .iter()
        .find_map(|(raw_account_id, account)| {
            let normalized_account_id = normalize_channel_account_id(raw_account_id);
            if normalized_account_id != configured_account_id {
                return None;
            }
            Some(account.enabled.unwrap_or(true))
        })
        .unwrap_or(true);
    config.onebot.enabled && account_enabled
}

fn build_invalid_weixin_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_enabled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            WEIXIN_SEND_OPERATION,
            "weixin bridge surface is unavailable in this build".to_owned(),
        )
    } else {
        misconfigured_operation(WEIXIN_SEND_OPERATION, vec![error.clone()])
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            WEIXIN_SERVE_OPERATION,
            "weixin bridge surface is unavailable in this build".to_owned(),
        )
    } else {
        misconfigured_operation(WEIXIN_SERVE_OPERATION, vec![error.clone()])
    };

    let mut notes = vec![
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
        "bridge_runtime_owner=external_plugin".to_owned(),
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
        enabled: configured_enabled,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_qqbot_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_enabled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            QQBOT_SEND_OPERATION,
            "qqbot bridge surface is unavailable in this build".to_owned(),
        )
    } else {
        misconfigured_operation(QQBOT_SEND_OPERATION, vec![error.clone()])
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            QQBOT_SERVE_OPERATION,
            "qqbot bridge surface is unavailable in this build".to_owned(),
        )
    } else {
        misconfigured_operation(QQBOT_SERVE_OPERATION, vec![error.clone()])
    };

    let mut notes = vec![
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
        "bridge_runtime_owner=external_plugin".to_owned(),
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
        enabled: configured_enabled,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

fn build_invalid_onebot_snapshot(
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    configured_enabled: bool,
    configured_account_id: &str,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    error: String,
) -> ChannelStatusSnapshot {
    let send_operation = if !compiled {
        unsupported_operation(
            ONEBOT_SEND_OPERATION,
            "onebot bridge surface is unavailable in this build".to_owned(),
        )
    } else {
        misconfigured_operation(ONEBOT_SEND_OPERATION, vec![error.clone()])
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            ONEBOT_SERVE_OPERATION,
            "onebot bridge surface is unavailable in this build".to_owned(),
        )
    } else {
        misconfigured_operation(ONEBOT_SERVE_OPERATION, vec![error.clone()])
    };

    let mut notes = vec![
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
        "bridge_runtime_owner=external_plugin".to_owned(),
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
        enabled: configured_enabled,
        api_base_url: None,
        notes,
        reserved_runtime_fields: Vec::new(),
        operations: vec![send_operation, serve_operation],
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::channel::registry::{
        ChannelOperationHealth, channel_status_snapshots, list_channel_catalog,
    };

    #[test]
    fn channel_catalog_includes_plugin_backed_weixin_qqbot_and_onebot_surfaces() {
        let catalog = list_channel_catalog();
        let weixin = catalog
            .iter()
            .find(|entry| entry.id == "weixin")
            .expect("weixin catalog entry");
        let qqbot = catalog
            .iter()
            .find(|entry| entry.id == "qqbot")
            .expect("qqbot catalog entry");
        let onebot = catalog
            .iter()
            .find(|entry| entry.id == "onebot")
            .expect("onebot catalog entry");

        assert_eq!(weixin.implementation_status.as_str(), "plugin_backed");
        assert_eq!(weixin.aliases, vec!["wechat", "wx", "wechat-clawbot"]);
        assert_eq!(weixin.transport, "wechat_clawbot_ilink_bridge");
        assert_eq!(
            weixin.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Conversation]
        );

        assert_eq!(qqbot.implementation_status.as_str(), "plugin_backed");
        assert_eq!(qqbot.aliases, vec!["qq", "qq-bot", "tencent-qq"]);
        assert_eq!(qqbot.transport, "qq_official_bot_gateway_or_plugin_bridge");
        assert_eq!(
            qqbot.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Conversation]
        );

        assert_eq!(onebot.implementation_status.as_str(), "plugin_backed");
        assert_eq!(onebot.aliases, vec!["onebot-v11", "napcat", "llonebot"]);
        assert_eq!(onebot.transport, "onebot_v11_bridge");
        assert_eq!(
            onebot.supported_target_kinds,
            vec![ChannelCatalogTargetKind::Conversation]
        );
    }

    #[test]
    fn weixin_status_reports_configured_bridge_surface_without_native_runtime() {
        let config: LoongClawConfig = serde_json::from_value(json!({
            "weixin": {
                "enabled": true,
                "bridge_url": "https://bridge.example.test/api?access_token=secret-token",
                "bridge_access_token": "bridge-token",
                "allowed_contact_ids": ["wxid_alice"]
            }
        }))
        .expect("deserialize weixin config");

        let snapshots = channel_status_snapshots(&config);
        let weixin = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "weixin")
            .expect("weixin snapshot");
        let send = weixin.operation("send").expect("weixin send operation");
        let serve = weixin.operation("serve").expect("weixin serve operation");

        assert_eq!(weixin.configured_account_id, "default");
        assert_eq!(
            weixin.api_base_url.as_deref(),
            Some("https://bridge.example.test/api")
        );
        assert_eq!(send.health, ChannelOperationHealth::Unsupported);
        assert_eq!(serve.health, ChannelOperationHealth::Unsupported);
        assert!(
            send.issues
                .iter()
                .any(|issue| issue.contains("external bridge plugin")),
            "send issues should explain bridge ownership"
        );
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("reply loop still belongs")),
            "serve issues should explain runtime ownership"
        );
        assert!(
            weixin
                .notes
                .iter()
                .any(|note| note == "bridge_runtime_owner=external_plugin")
        );
        assert!(
            weixin
                .notes
                .iter()
                .any(|note| note == "bridge_access_token_configured=true")
        );
        assert!(
            weixin
                .notes
                .iter()
                .any(|note| note == "allowed_contact_ids_count=1")
        );
    }

    #[test]
    fn qqbot_status_reports_configured_bridge_surface_without_native_runtime() {
        let config: LoongClawConfig = serde_json::from_value(json!({
            "qqbot": {
                "enabled": true,
                "app_id": "10001",
                "client_secret": "client-secret",
                "allowed_peer_ids": ["openid-user-a"]
            }
        }))
        .expect("deserialize qqbot config");

        let snapshots = channel_status_snapshots(&config);
        let qqbot = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "qqbot")
            .expect("qqbot snapshot");
        let send = qqbot.operation("send").expect("qqbot send operation");
        let serve = qqbot.operation("serve").expect("qqbot serve operation");

        assert_eq!(qqbot.configured_account_id, "qqbot_10001");
        assert_eq!(qqbot.api_base_url, None);
        assert_eq!(send.health, ChannelOperationHealth::Unsupported);
        assert_eq!(serve.health, ChannelOperationHealth::Unsupported);
        assert!(
            send.issues
                .iter()
                .any(|issue| issue.contains("external gateway or bridge plugin")),
            "send issues should explain gateway ownership"
        );
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("reply loop still belongs")),
            "serve issues should explain runtime ownership"
        );
        assert!(
            qqbot
                .notes
                .iter()
                .any(|note| note == "bridge_runtime_owner=external_plugin")
        );
        assert!(qqbot.notes.iter().any(|note| note == "app_id=10001"));
        assert!(
            qqbot
                .notes
                .iter()
                .any(|note| note == "client_secret_configured=true")
        );
        assert!(
            qqbot
                .notes
                .iter()
                .any(|note| note == "allowed_peer_ids_count=1")
        );
    }

    #[test]
    fn onebot_status_reports_configured_bridge_surface_without_native_runtime() {
        let config: LoongClawConfig = serde_json::from_value(json!({
            "onebot": {
                "enabled": true,
                "websocket_url": "ws://127.0.0.1:5700?access_token=secret-token",
                "access_token": "bridge-token",
                "allowed_group_ids": ["123456"]
            }
        }))
        .expect("deserialize onebot config");

        let snapshots = channel_status_snapshots(&config);
        let onebot = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "onebot")
            .expect("onebot snapshot");
        let send = onebot.operation("send").expect("onebot send operation");
        let serve = onebot.operation("serve").expect("onebot serve operation");

        assert_eq!(onebot.configured_account_id, "default");
        assert_eq!(onebot.api_base_url.as_deref(), Some("ws://127.0.0.1:5700/"));
        assert_eq!(send.health, ChannelOperationHealth::Unsupported);
        assert_eq!(serve.health, ChannelOperationHealth::Unsupported);
        assert!(
            send.issues
                .iter()
                .any(|issue| issue.contains("external bridge runtime")),
            "send issues should explain bridge ownership"
        );
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("event loop still belongs")),
            "serve issues should explain runtime ownership"
        );
        assert!(
            onebot
                .notes
                .iter()
                .any(|note| note == "bridge_runtime_owner=external_plugin")
        );
        assert!(
            onebot
                .notes
                .iter()
                .any(|note| note == "access_token_configured=true")
        );
        assert!(
            onebot
                .notes
                .iter()
                .any(|note| note == "allowed_group_ids_count=1")
        );
        assert!(
            onebot
                .notes
                .iter()
                .any(|note| note == "account_id=onebot_127-0-0-1-5700")
        );
    }

    #[test]
    fn invalid_weixin_snapshot_preserves_configured_enabled_state() {
        let snapshot = build_invalid_weixin_snapshot(
            &WEIXIN_CHANNEL_REGISTRY_DESCRIPTOR,
            true,
            true,
            "default",
            true,
            ChannelDefaultAccountSelectionSource::ExplicitDefault,
            "selection failed".to_owned(),
        );

        assert!(
            snapshot.enabled,
            "invalid weixin snapshots should keep the configured enabled state"
        );
    }

    #[test]
    fn invalid_qqbot_snapshot_preserves_configured_enabled_state() {
        let snapshot = build_invalid_qqbot_snapshot(
            &QQBOT_CHANNEL_REGISTRY_DESCRIPTOR,
            true,
            true,
            "default",
            true,
            ChannelDefaultAccountSelectionSource::ExplicitDefault,
            "selection failed".to_owned(),
        );

        assert!(
            snapshot.enabled,
            "invalid qqbot snapshots should keep the configured enabled state"
        );
    }

    #[test]
    fn invalid_onebot_snapshot_preserves_configured_enabled_state() {
        let snapshot = build_invalid_onebot_snapshot(
            &ONEBOT_CHANNEL_REGISTRY_DESCRIPTOR,
            true,
            true,
            "default",
            true,
            ChannelDefaultAccountSelectionSource::ExplicitDefault,
            "selection failed".to_owned(),
        );

        assert!(
            snapshot.enabled,
            "invalid onebot snapshots should keep the configured enabled state"
        );
    }
}
