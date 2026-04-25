use std::path::Path;

use crate::channel::http;
use crate::channel::runtime::state;
#[cfg(feature = "channel-plugin-bridge")]
use crate::channel::{
    ManagedPluginBridgeRuntimeBinding, resolve_managed_plugin_bridge_runtime_binding,
};
use crate::config::{
    ChannelDefaultAccountSelectionSource, LoongConfig, ONEBOT_ACCESS_TOKEN_ENV,
    ONEBOT_WEBSOCKET_URL_ENV, ResolvedOnebotChannelConfig, ResolvedWeixinChannelConfig,
    ResolvedWhatsappPersonalChannelConfig, WEIXIN_BRIDGE_ACCESS_TOKEN_ENV, WEIXIN_BRIDGE_URL_ENV,
    WHATSAPP_PERSONAL_AUTH_DIR_ENV, WHATSAPP_PERSONAL_BRIDGE_URL_ENV, normalize_channel_account_id,
};

use super::{
    CHANNEL_OPERATION_SEND_ID, CHANNEL_OPERATION_SERVE_ID, ChannelCatalogImplementationStatus,
    ChannelCatalogOperation, ChannelCatalogOperationAvailability,
    ChannelCatalogOperationRequirement, ChannelCatalogTargetKind, ChannelDoctorCheckSpec,
    ChannelDoctorCheckTrigger, ChannelOnboardingDescriptor, ChannelOnboardingStrategy,
    ChannelOperationHealth, ChannelOperationRuntime, ChannelOperationStatus,
    ChannelPluginBridgeStableTarget, ChannelRegistryDescriptor, ChannelRegistryOperationDescriptor,
    ChannelStatusSnapshot, PLUGIN_BACKED_CHANNEL_CAPABILITIES, apply_runtime_attention,
    disabled_operation, misconfigured_operation, unsupported_operation, validate_http_url,
    validate_websocket_url,
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
    command: "channels send weixin",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: false,
    requirements: WEIXIN_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const WEIXIN_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "bridge reply loop",
    command: "channels serve weixin",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: true,
    requirements: WEIXIN_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

#[allow(dead_code)]
pub const WEIXIN_CATALOG_COMMAND_FAMILY_DESCRIPTOR: super::ChannelCatalogCommandFamilyDescriptor =
    super::ChannelCatalogCommandFamilyDescriptor {
        channel_id: "weixin",
        default_send_target_kind: ChannelCatalogTargetKind::Conversation,
        send: WEIXIN_SEND_OPERATION,
        serve: WEIXIN_SERVE_OPERATION,
    };

const WEIXIN_SEND_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[ChannelDoctorCheckSpec {
    name: "weixin bridge send contract",
    trigger: ChannelDoctorCheckTrigger::PluginBridgeHealth,
}];

const WEIXIN_SERVE_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[
    ChannelDoctorCheckSpec {
        name: "weixin bridge serve contract",
        trigger: ChannelDoctorCheckTrigger::PluginBridgeHealth,
    },
    ChannelDoctorCheckSpec {
        name: "weixin bridge serve runtime",
        trigger: ChannelDoctorCheckTrigger::ReadyRuntime,
    },
];

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
    setup_hint: "plugin-bridge weixin surface; run `loong weixin onboard` to provision bridge_url and bridge_access_token through the WeChat ClawBot / iLink QR flow, then let the external bridge or managed plugin keep owning the upstream listener lifecycle",
    status_command: "loong doctor",
    repair_command: Some("loong weixin onboard"),
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
    command: "channels send onebot",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: false,
    requirements: ONEBOT_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const ONEBOT_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "bridge event loop",
    command: "channels serve onebot",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: true,
    requirements: ONEBOT_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

#[allow(dead_code)]
pub const ONEBOT_CATALOG_COMMAND_FAMILY_DESCRIPTOR: super::ChannelCatalogCommandFamilyDescriptor =
    super::ChannelCatalogCommandFamilyDescriptor {
        channel_id: "onebot",
        default_send_target_kind: ChannelCatalogTargetKind::Conversation,
        send: ONEBOT_SEND_OPERATION,
        serve: ONEBOT_SERVE_OPERATION,
    };

const ONEBOT_SEND_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[ChannelDoctorCheckSpec {
    name: "onebot bridge send contract",
    trigger: ChannelDoctorCheckTrigger::PluginBridgeHealth,
}];

const ONEBOT_SERVE_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[
    ChannelDoctorCheckSpec {
        name: "onebot bridge serve contract",
        trigger: ChannelDoctorCheckTrigger::PluginBridgeHealth,
    },
    ChannelDoctorCheckSpec {
        name: "onebot bridge serve runtime",
        trigger: ChannelDoctorCheckTrigger::ReadyRuntime,
    },
];

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

const WHATSAPP_PERSONAL_ENABLED_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "enabled",
        label: "channel enabled",
        config_paths: &[
            "whatsapp_personal.enabled",
            "whatsapp_personal.accounts.<account>.enabled",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };

const WHATSAPP_PERSONAL_BRIDGE_URL_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "bridge_url",
        label: "local bridge url",
        config_paths: &[
            "whatsapp_personal.bridge_url",
            "whatsapp_personal.accounts.<account>.bridge_url",
        ],
        env_pointer_paths: &[
            "whatsapp_personal.bridge_url_env",
            "whatsapp_personal.accounts.<account>.bridge_url_env",
        ],
        default_env_var: Some(WHATSAPP_PERSONAL_BRIDGE_URL_ENV),
    };

const WHATSAPP_PERSONAL_AUTH_DIR_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "auth_dir",
        label: "bridge auth dir",
        config_paths: &[
            "whatsapp_personal.auth_dir",
            "whatsapp_personal.accounts.<account>.auth_dir",
        ],
        env_pointer_paths: &[
            "whatsapp_personal.auth_dir_env",
            "whatsapp_personal.accounts.<account>.auth_dir_env",
        ],
        default_env_var: Some(WHATSAPP_PERSONAL_AUTH_DIR_ENV),
    };

const WHATSAPP_PERSONAL_ALLOWED_CHAT_IDS_REQUIREMENT: ChannelCatalogOperationRequirement =
    ChannelCatalogOperationRequirement {
        id: "allowed_chat_ids",
        label: "allowed chat ids",
        config_paths: &[
            "whatsapp_personal.allowed_chat_ids",
            "whatsapp_personal.accounts.<account>.allowed_chat_ids",
        ],
        env_pointer_paths: &[],
        default_env_var: None,
    };

const WHATSAPP_PERSONAL_SEND_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    WHATSAPP_PERSONAL_ENABLED_REQUIREMENT,
    WHATSAPP_PERSONAL_BRIDGE_URL_REQUIREMENT,
];

const WHATSAPP_PERSONAL_SERVE_REQUIREMENTS: &[ChannelCatalogOperationRequirement] = &[
    WHATSAPP_PERSONAL_ENABLED_REQUIREMENT,
    WHATSAPP_PERSONAL_BRIDGE_URL_REQUIREMENT,
    WHATSAPP_PERSONAL_AUTH_DIR_REQUIREMENT,
    WHATSAPP_PERSONAL_ALLOWED_CHAT_IDS_REQUIREMENT,
];

const WHATSAPP_PERSONAL_SEND_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SEND_ID,
    label: "personal bridge send",
    command: "channels send whatsapp-personal",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: false,
    requirements: WHATSAPP_PERSONAL_SEND_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

const WHATSAPP_PERSONAL_SERVE_OPERATION: ChannelCatalogOperation = ChannelCatalogOperation {
    id: CHANNEL_OPERATION_SERVE_ID,
    label: "personal bridge reply loop",
    command: "channels serve whatsapp-personal",
    availability: ChannelCatalogOperationAvailability::ManagedBridge,
    tracks_runtime: true,
    requirements: WHATSAPP_PERSONAL_SERVE_REQUIREMENTS,
    default_target_kind: None,
    supported_target_kinds: &[ChannelCatalogTargetKind::Conversation],
};

#[allow(dead_code)]
pub const WHATSAPP_PERSONAL_CATALOG_COMMAND_FAMILY_DESCRIPTOR:
    super::ChannelCatalogCommandFamilyDescriptor = super::ChannelCatalogCommandFamilyDescriptor {
    channel_id: "whatsapp-personal",
    default_send_target_kind: ChannelCatalogTargetKind::Conversation,
    send: WHATSAPP_PERSONAL_SEND_OPERATION,
    serve: WHATSAPP_PERSONAL_SERVE_OPERATION,
};

const WHATSAPP_PERSONAL_SEND_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[ChannelDoctorCheckSpec {
    name: "whatsapp-personal bridge send contract",
    trigger: ChannelDoctorCheckTrigger::PluginBridgeHealth,
}];

const WHATSAPP_PERSONAL_SERVE_DOCTOR_CHECKS: &[ChannelDoctorCheckSpec] = &[
    ChannelDoctorCheckSpec {
        name: "whatsapp-personal bridge serve contract",
        trigger: ChannelDoctorCheckTrigger::PluginBridgeHealth,
    },
    ChannelDoctorCheckSpec {
        name: "whatsapp-personal bridge serve runtime",
        trigger: ChannelDoctorCheckTrigger::ReadyRuntime,
    },
];

const WHATSAPP_PERSONAL_OPERATIONS: &[ChannelRegistryOperationDescriptor] = &[
    ChannelRegistryOperationDescriptor {
        operation: WHATSAPP_PERSONAL_SEND_OPERATION,
        doctor_checks: WHATSAPP_PERSONAL_SEND_DOCTOR_CHECKS,
    },
    ChannelRegistryOperationDescriptor {
        operation: WHATSAPP_PERSONAL_SERVE_OPERATION,
        doctor_checks: WHATSAPP_PERSONAL_SERVE_DOCTOR_CHECKS,
    },
];

const WHATSAPP_PERSONAL_ONBOARDING_DESCRIPTOR: ChannelOnboardingDescriptor =
    ChannelOnboardingDescriptor {
        strategy: ChannelOnboardingStrategy::PluginBridge,
        setup_hint: "plugin-bridge personal WhatsApp surface; run the bundled WhatsApp Web QR bridge under whatsapp_personal or whatsapp_personal.accounts.<account>, then use this surface as the stable Loong contract while the bridge owns QR login and session persistence",
        status_command: "loong doctor",
        repair_command: Some("./scripts/whatsapp-personal-bridge-run.sh"),
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

const WHATSAPP_PERSONAL_PLUGIN_BRIDGE_STABLE_TARGETS: &[ChannelPluginBridgeStableTarget] = &[
    ChannelPluginBridgeStableTarget {
        template: "whatsapp-personal:<account>:contact:<e164-or-jid>",
        target_kind: ChannelCatalogTargetKind::Conversation,
        description: "direct contact route via an E.164 number or personal WhatsApp JID",
    },
    ChannelPluginBridgeStableTarget {
        template: "whatsapp-personal:<account>:group:<jid>",
        target_kind: ChannelCatalogTargetKind::Conversation,
        description: "group conversation route via a WhatsApp group JID",
    },
];

const ONEBOT_PLUGIN_BRIDGE_ACCOUNT_SCOPE_NOTE: &str =
    "keep <account> stable so personal-account bridge routes stay unambiguous";

const WHATSAPP_PERSONAL_PLUGIN_BRIDGE_ACCOUNT_SCOPE_NOTE: &str = "keep <account> stable so QR-linked personal WhatsApp sessions and route ownership stay legible";

const MANAGED_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION: &str = "send_message";
const MANAGED_BRIDGE_RUNTIME_RECEIVE_BATCH_OPERATION: &str = "receive_batch";
const MANAGED_BRIDGE_RUNTIME_ACK_INBOUND_OPERATION: &str = "ack_inbound";
const MANAGED_BRIDGE_RUNTIME_COMPLETE_BATCH_OPERATION: &str = "complete_batch";

#[cfg(not(feature = "channel-plugin-bridge"))]
fn managed_bridge_operation_status(
    _config: &LoongConfig,
    _channel_id: &str,
    _configured_account_id: &str,
    operation: ChannelCatalogOperation,
    _required_runtime_operations: &[&str],
    _runtime_dir: &Path,
    _now_ms: u64,
) -> ChannelOperationStatus {
    let detail = "managed bridge runtime is unavailable in this feature set; enable channel-plugin-bridge (managed bridge runtime is disabled)".to_owned();
    unsupported_operation(operation, detail)
}

#[cfg(feature = "channel-plugin-bridge")]
fn managed_bridge_operation_status(
    config: &LoongConfig,
    channel_id: &str,
    configured_account_id: &str,
    operation: ChannelCatalogOperation,
    required_runtime_operations: &[&str],
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelOperationStatus {
    let binding_result = resolve_managed_plugin_bridge_runtime_binding(
        config,
        channel_id,
        Some(configured_account_id),
    );
    let binding = match binding_result {
        Ok(binding) => binding,
        Err(error) => {
            return unsupported_operation(operation, error);
        }
    };

    let missing_runtime_operations =
        missing_managed_bridge_runtime_operations(&binding, required_runtime_operations);
    if !missing_runtime_operations.is_empty() {
        let rendered_operations = missing_runtime_operations.join(",");
        let issue = format!(
            "managed bridge plugin {} is missing runtime operations: {rendered_operations}",
            binding.plugin.plugin_id
        );
        return unsupported_operation(operation, issue);
    }

    let detail = format!(
        "managed bridge runtime ready via plugin {} (bridge_kind={}, runtime_contract={})",
        binding.plugin.plugin_id,
        binding.plugin.runtime.bridge_kind.as_str(),
        binding.runtime_contract
    );

    let mut status = ChannelOperationStatus {
        id: operation.id,
        label: operation.label,
        command: operation.command,
        health: ChannelOperationHealth::Ready,
        detail,
        issues: Vec::new(),
        runtime: None,
    };

    if operation.tracks_runtime {
        status.runtime = state::load_channel_operation_runtime_for_account_from_dir(
            runtime_dir,
            binding.platform,
            operation.id,
            binding.account_id.as_str(),
            now_ms,
        )
        .map(|mut runtime| {
            if runtime.account_id.is_none() {
                runtime.account_id = Some(binding.account_id.clone());
            }
            if runtime.account_label.is_none() {
                runtime.account_label = Some(binding.account_label.clone());
            }
            runtime
        })
        .or(Some(ChannelOperationRuntime {
            running: false,
            stale: false,
            busy: false,
            active_runs: 0,
            consecutive_failures: 0,
            last_run_activity_at: None,
            last_heartbeat_at: None,
            last_failure_at: None,
            last_recovery_at: None,
            last_error: None,
            last_duplicate_reclaim_at: None,
            pid: None,
            account_id: Some(binding.account_id.clone()),
            account_label: Some(binding.account_label.clone()),
            instance_count: 0,
            running_instances: 0,
            stale_instances: 0,
            duplicate_owner_pids: Vec::new(),
            last_duplicate_reclaim_cleanup_owner_pids: Vec::new(),
            recent_incidents: Vec::new(),
        }));
        apply_runtime_attention(&mut status);
    }

    status
}

#[cfg(feature = "channel-plugin-bridge")]
fn missing_managed_bridge_runtime_operations(
    binding: &ManagedPluginBridgeRuntimeBinding,
    required_runtime_operations: &[&str],
) -> Vec<String> {
    let mut missing_operations = Vec::new();

    for required_runtime_operation in required_runtime_operations {
        let supports_operation = binding.supports_operation(required_runtime_operation);
        if supports_operation {
            continue;
        }

        let missing_operation = (*required_runtime_operation).to_owned();
        missing_operations.push(missing_operation);
    }

    missing_operations
}

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

pub(super) const WHATSAPP_PERSONAL_CHANNEL_REGISTRY_DESCRIPTOR: ChannelRegistryDescriptor =
    ChannelRegistryDescriptor {
        id: "whatsapp-personal",
        runtime: None,
        snapshot_builder: Some(build_whatsapp_personal_snapshots),
        selection_order: 91,
        selection_label: "personal qr-linked account bridge",
        blurb: "Plugin-backed personal WhatsApp surface that delegates QR login and session ownership to a local WhatsApp Web bridge while Loong owns the stable send and reply-loop contract.",
        implementation_status: ChannelCatalogImplementationStatus::PluginBacked,
        capabilities: PLUGIN_BACKED_CHANNEL_CAPABILITIES,
        label: "WhatsApp Personal",
        aliases: &["whatsapp-web", "wa-personal", "whatsapp-personal-bridge"],
        transport: "whatsapp_web_baileys_bridge",
        onboarding: WHATSAPP_PERSONAL_ONBOARDING_DESCRIPTOR,
        operations: WHATSAPP_PERSONAL_OPERATIONS,
    };

pub(super) fn plugin_bridge_stable_targets_for_channel_id(
    channel_id: &str,
) -> &'static [ChannelPluginBridgeStableTarget] {
    match channel_id {
        "weixin" => WEIXIN_PLUGIN_BRIDGE_STABLE_TARGETS,
        "onebot" => ONEBOT_PLUGIN_BRIDGE_STABLE_TARGETS,
        "whatsapp-personal" => WHATSAPP_PERSONAL_PLUGIN_BRIDGE_STABLE_TARGETS,
        _ => EMPTY_PLUGIN_BRIDGE_STABLE_TARGETS,
    }
}

pub(super) fn plugin_bridge_account_scope_note_for_channel_id(
    channel_id: &str,
) -> Option<&'static str> {
    match channel_id {
        "onebot" => Some(ONEBOT_PLUGIN_BRIDGE_ACCOUNT_SCOPE_NOTE),
        "whatsapp-personal" => Some(WHATSAPP_PERSONAL_PLUGIN_BRIDGE_ACCOUNT_SCOPE_NOTE),
        _ => None,
    }
}

fn build_weixin_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    runtime_dir: &Path,
    now_ms: u64,
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
                    config,
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                    runtime_dir,
                    now_ms,
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

fn build_onebot_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    runtime_dir: &Path,
    now_ms: u64,
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
                    config,
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    runtime_dir,
                    now_ms,
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

fn build_whatsapp_personal_snapshots(
    descriptor: &ChannelRegistryDescriptor,
    config: &LoongConfig,
    runtime_dir: &Path,
    now_ms: u64,
) -> Vec<ChannelStatusSnapshot> {
    let compiled = true;
    let http_policy = http::outbound_http_policy_from_config(config);
    let default_selection = config
        .whatsapp_personal
        .default_configured_account_selection();
    let default_configured_account_id = default_selection.id.clone();
    let default_account_source = default_selection.source;

    config
        .whatsapp_personal
        .configured_account_ids()
        .into_iter()
        .map(|configured_account_id| {
            let is_default_account = configured_account_id == default_configured_account_id;
            let configured_enabled = configured_whatsapp_personal_account_enabled(
                config,
                configured_account_id.as_str(),
            );
            match config
                .whatsapp_personal
                .resolve_account(Some(configured_account_id.as_str()))
            {
                Ok(resolved) => build_whatsapp_personal_snapshot_for_account(
                    config,
                    descriptor,
                    compiled,
                    resolved,
                    is_default_account,
                    default_account_source,
                    http_policy,
                    runtime_dir,
                    now_ms,
                ),
                Err(error) => build_invalid_whatsapp_personal_snapshot(
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
    config: &LoongConfig,
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedWeixinChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: http::ChannelOutboundHttpPolicy,
    runtime_dir: &Path,
    now_ms: u64,
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
        managed_bridge_operation_status(
            config,
            descriptor.id,
            resolved.configured_account_id.as_str(),
            WEIXIN_SEND_OPERATION,
            &[MANAGED_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION],
            runtime_dir,
            now_ms,
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
        managed_bridge_operation_status(
            config,
            descriptor.id,
            resolved.configured_account_id.as_str(),
            WEIXIN_SERVE_OPERATION,
            &[
                MANAGED_BRIDGE_RUNTIME_RECEIVE_BATCH_OPERATION,
                MANAGED_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION,
                MANAGED_BRIDGE_RUNTIME_ACK_INBOUND_OPERATION,
                MANAGED_BRIDGE_RUNTIME_COMPLETE_BATCH_OPERATION,
            ],
            runtime_dir,
            now_ms,
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

fn build_whatsapp_personal_snapshot_for_account(
    config: &LoongConfig,
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedWhatsappPersonalChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    http_policy: http::ChannelOutboundHttpPolicy,
    runtime_dir: &Path,
    now_ms: u64,
) -> ChannelStatusSnapshot {
    let mut send_issues = Vec::new();

    let bridge_url = resolved.bridge_url();
    if bridge_url.is_none() {
        send_issues.push("bridge_url is missing".to_owned());
    }
    let validated_bridge_url = bridge_url
        .as_deref()
        .and_then(|url| validate_http_url("bridge_url", url, http_policy, &mut send_issues));

    let mut serve_issues = send_issues.clone();
    let auth_dir = resolved.auth_dir();
    if auth_dir.is_none() {
        serve_issues.push("auth_dir is missing".to_owned());
    }

    let send_operation = if !compiled {
        unsupported_operation(
            WHATSAPP_PERSONAL_SEND_OPERATION,
            "whatsapp-personal bridge surface is unavailable in this build".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            WHATSAPP_PERSONAL_SEND_OPERATION,
            "disabled by whatsapp_personal account configuration".to_owned(),
        )
    } else if !send_issues.is_empty() {
        misconfigured_operation(WHATSAPP_PERSONAL_SEND_OPERATION, send_issues)
    } else {
        managed_bridge_operation_status(
            config,
            descriptor.id,
            resolved.configured_account_id.as_str(),
            WHATSAPP_PERSONAL_SEND_OPERATION,
            &[MANAGED_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION],
            runtime_dir,
            now_ms,
        )
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            WHATSAPP_PERSONAL_SERVE_OPERATION,
            "whatsapp-personal bridge surface is unavailable in this build".to_owned(),
        )
    } else if !resolved.enabled {
        disabled_operation(
            WHATSAPP_PERSONAL_SERVE_OPERATION,
            "disabled by whatsapp_personal account configuration".to_owned(),
        )
    } else if !serve_issues.is_empty() {
        misconfigured_operation(WHATSAPP_PERSONAL_SERVE_OPERATION, serve_issues)
    } else {
        managed_bridge_operation_status(
            config,
            descriptor.id,
            resolved.configured_account_id.as_str(),
            WHATSAPP_PERSONAL_SERVE_OPERATION,
            &[
                MANAGED_BRIDGE_RUNTIME_RECEIVE_BATCH_OPERATION,
                MANAGED_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION,
                MANAGED_BRIDGE_RUNTIME_ACK_INBOUND_OPERATION,
                MANAGED_BRIDGE_RUNTIME_COMPLETE_BATCH_OPERATION,
            ],
            runtime_dir,
            now_ms,
        )
    };

    let allowed_chat_ids_count = resolved
        .allowed_chat_ids
        .iter()
        .filter(|value| !value.trim().is_empty())
        .count();

    let mut notes = vec![
        format!("configured_account_id={}", resolved.configured_account_id),
        format!("configured_account={}", resolved.configured_account_label),
        format!("account_id={}", resolved.account.id),
        format!("account={}", resolved.account.label),
        "bridge_runtime_owner=external_plugin".to_owned(),
        "bridge_login=qr_registration".to_owned(),
    ];
    if auth_dir.is_some() {
        notes.push("auth_dir_configured=true".to_owned());
    }
    if allowed_chat_ids_count > 0 {
        notes.push(format!("allowed_chat_ids_count={allowed_chat_ids_count}"));
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

fn build_onebot_snapshot_for_account(
    config: &LoongConfig,
    descriptor: &ChannelRegistryDescriptor,
    compiled: bool,
    resolved: ResolvedOnebotChannelConfig,
    is_default_account: bool,
    default_account_source: ChannelDefaultAccountSelectionSource,
    runtime_dir: &Path,
    now_ms: u64,
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
        managed_bridge_operation_status(
            config,
            descriptor.id,
            resolved.configured_account_id.as_str(),
            ONEBOT_SEND_OPERATION,
            &[MANAGED_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION],
            runtime_dir,
            now_ms,
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
        managed_bridge_operation_status(
            config,
            descriptor.id,
            resolved.configured_account_id.as_str(),
            ONEBOT_SERVE_OPERATION,
            &[
                MANAGED_BRIDGE_RUNTIME_RECEIVE_BATCH_OPERATION,
                MANAGED_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION,
                MANAGED_BRIDGE_RUNTIME_ACK_INBOUND_OPERATION,
                MANAGED_BRIDGE_RUNTIME_COMPLETE_BATCH_OPERATION,
            ],
            runtime_dir,
            now_ms,
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

fn configured_weixin_account_enabled(config: &LoongConfig, configured_account_id: &str) -> bool {
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

fn configured_onebot_account_enabled(config: &LoongConfig, configured_account_id: &str) -> bool {
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

fn configured_whatsapp_personal_account_enabled(
    config: &LoongConfig,
    configured_account_id: &str,
) -> bool {
    let account_enabled = config
        .whatsapp_personal
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
    config.whatsapp_personal.enabled && account_enabled
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

fn build_invalid_whatsapp_personal_snapshot(
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
            WHATSAPP_PERSONAL_SEND_OPERATION,
            "whatsapp-personal bridge surface is unavailable in this build".to_owned(),
        )
    } else {
        misconfigured_operation(WHATSAPP_PERSONAL_SEND_OPERATION, vec![error.clone()])
    };

    let serve_operation = if !compiled {
        unsupported_operation(
            WHATSAPP_PERSONAL_SERVE_OPERATION,
            "whatsapp-personal bridge surface is unavailable in this build".to_owned(),
        )
    } else {
        misconfigured_operation(WHATSAPP_PERSONAL_SERVE_OPERATION, vec![error.clone()])
    };

    let mut notes = vec![
        format!("configured_account_id={configured_account_id}"),
        format!("selection_error={error}"),
        "bridge_runtime_owner=external_plugin".to_owned(),
        "bridge_login=qr_registration".to_owned(),
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
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::Path;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::channel::registry::{ChannelOperationHealth, channel_status_snapshots};

    fn write_runtime_manifest(
        root: &Path,
        directory_name: &str,
        channel_id: &str,
        runtime_operations: Vec<&str>,
    ) {
        let runtime_operations = runtime_operations
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let runtime_operations_json =
            serde_json::to_string(&runtime_operations).expect("serialize runtime operations");
        let metadata = BTreeMap::from([
            ("bridge_kind".to_owned(), "http_json".to_owned()),
            ("adapter_family".to_owned(), "channel-bridge".to_owned()),
            (
                "transport_family".to_owned(),
                "wechat_clawbot_ilink_bridge".to_owned(),
            ),
            ("target_contract".to_owned(), "weixin_reply_loop".to_owned()),
            (
                "channel_runtime_contract".to_owned(),
                crate::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_CONTRACT_V1.to_owned(),
            ),
            (
                "channel_runtime_operations_json".to_owned(),
                runtime_operations_json,
            ),
        ]);
        let manifest = loong_kernel::PluginManifest {
            api_version: Some("v1alpha1".to_owned()),
            version: Some("1.0.0".to_owned()),
            plugin_id: "weixin-managed-runtime".to_owned(),
            provider_id: "weixin-managed-runtime-provider".to_owned(),
            connector_name: "weixin-managed-runtime-connector".to_owned(),
            channel_id: Some(channel_id.to_owned()),
            endpoint: Some("http://127.0.0.1:9999/invoke".to_owned()),
            capabilities: BTreeSet::new(),
            trust_tier: loong_kernel::PluginTrustTier::Unverified,
            metadata,
            summary: None,
            tags: Vec::new(),
            input_examples: Vec::new(),
            output_examples: Vec::new(),
            defer_loading: false,
            setup: Some(loong_kernel::PluginSetup {
                mode: loong_kernel::PluginSetupMode::MetadataOnly,
                surface: Some("channel".to_owned()),
                required_env_vars: Vec::new(),
                recommended_env_vars: Vec::new(),
                required_config_keys: Vec::new(),
                default_env_var: None,
                docs_urls: Vec::new(),
                remediation: None,
            }),
            slot_claims: Vec::new(),
            compatibility: None,
        };
        let plugin_directory = root.join(directory_name);
        let manifest_path = plugin_directory.join("loong.plugin.json");
        let encoded_manifest =
            serde_json::to_string_pretty(&manifest).expect("serialize runtime manifest");

        fs::create_dir_all(&plugin_directory).expect("create runtime plugin directory");
        fs::write(&manifest_path, encoded_manifest).expect("write runtime plugin manifest");
    }

    #[test]
    fn weixin_status_reports_configured_bridge_surface_without_native_runtime() {
        let config: LoongConfig = serde_json::from_value(json!({
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
                .any(|issue| issue.contains("managed bridge runtime is disabled")),
            "send issues should explain managed bridge runtime requirements"
        );
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("managed bridge runtime is disabled")),
            "serve issues should explain managed bridge runtime requirements"
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
    fn weixin_status_reports_ready_when_managed_bridge_runtime_is_resolved() {
        let runtime_root = TempDir::new().expect("create runtime plugin root");
        write_runtime_manifest(
            runtime_root.path(),
            "weixin-managed-runtime",
            "weixin",
            vec![
                crate::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION,
                crate::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_RECEIVE_BATCH_OPERATION,
                crate::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_ACK_INBOUND_OPERATION,
                crate::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_COMPLETE_BATCH_OPERATION,
            ],
        );

        let mut config: LoongConfig = serde_json::from_value(json!({
            "weixin": {
                "enabled": true,
                "bridge_url": "https://bridge.example.test/api",
                "bridge_access_token": "bridge-token",
                "allowed_contact_ids": ["wxid_alice"]
            }
        }))
        .expect("deserialize weixin config");
        config.runtime_plugins.enabled = true;
        config.runtime_plugins.roots = vec![runtime_root.path().display().to_string()];
        config.runtime_plugins.supported_bridges = vec!["http_json".to_owned()];

        let snapshots = channel_status_snapshots(&config);
        let weixin = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "weixin")
            .expect("weixin snapshot");
        let send = weixin.operation("send").expect("weixin send operation");
        let serve = weixin.operation("serve").expect("weixin serve operation");

        assert_eq!(send.health, ChannelOperationHealth::Ready);
        assert_eq!(serve.health, ChannelOperationHealth::Ready);
        assert!(
            send.detail
                .contains("managed bridge runtime ready via plugin")
        );
        assert!(
            serve
                .detail
                .contains("managed bridge runtime ready via plugin")
        );
        assert!(serve.runtime.is_some());
        let runtime = serve.runtime.as_ref().expect("serve runtime");
        assert!(!runtime.running);
        assert_eq!(runtime.account_id.as_deref(), Some("default"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn weixin_status_attaches_live_runtime_state_when_managed_bridge_serve_is_running() {
        let runtime_root = TempDir::new().expect("create runtime plugin root");
        write_runtime_manifest(
            runtime_root.path(),
            "weixin-managed-runtime",
            "weixin",
            vec![
                crate::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION,
                crate::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_RECEIVE_BATCH_OPERATION,
                crate::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_ACK_INBOUND_OPERATION,
                crate::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_COMPLETE_BATCH_OPERATION,
            ],
        );

        let mut config: LoongConfig = serde_json::from_value(json!({
            "weixin": {
                "enabled": true,
                "bridge_url": "https://bridge.example.test/api",
                "bridge_access_token": "bridge-token",
                "allowed_contact_ids": ["wxid_alice"]
            }
        }))
        .expect("deserialize weixin config");
        config.runtime_plugins.enabled = true;
        config.runtime_plugins.roots = vec![runtime_root.path().display().to_string()];
        config.runtime_plugins.supported_bridges = vec!["http_json".to_owned()];

        let spec = crate::channel::runtime::serve::ChannelServeRuntimeSpec {
            platform: crate::channel::ChannelPlatform::Weixin,
            operation_id: crate::channel::CHANNEL_OPERATION_SERVE_ID,
            account_id: "default",
            account_label: "default",
        };

        let snapshots = crate::channel::runtime::serve::with_channel_serve_runtime_in_dir(
            runtime_root.path(),
            4242,
            spec,
            |_runtime| async {
                Ok(super::super::channel_status_snapshots_with_now(
                    &config,
                    runtime_root.path(),
                    crate::channel::runtime::serve::channel_runtime_now_ms(),
                ))
            },
        )
        .await
        .expect("produce runtime-backed snapshots");
        let weixin = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "weixin")
            .expect("weixin snapshot");
        let serve = weixin.operation("serve").expect("weixin serve operation");
        let runtime = serve.runtime.as_ref().expect("serve runtime");

        assert!(runtime.running);
        assert!(!runtime.stale);
        assert_eq!(runtime.account_id.as_deref(), Some("default"));
        assert_eq!(runtime.account_label.as_deref(), Some("default"));
        assert_eq!(runtime.running_instances, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn weixin_status_surfaces_retrying_runtime_attention_for_managed_bridge_serve() {
        let runtime_root = TempDir::new().expect("create runtime plugin root");
        write_runtime_manifest(
            runtime_root.path(),
            "weixin-managed-runtime",
            "weixin",
            vec![
                crate::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_SEND_MESSAGE_OPERATION,
                crate::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_RECEIVE_BATCH_OPERATION,
                crate::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_ACK_INBOUND_OPERATION,
                crate::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_COMPLETE_BATCH_OPERATION,
            ],
        );

        let mut config: LoongConfig = serde_json::from_value(json!({
            "weixin": {
                "enabled": true,
                "bridge_url": "https://bridge.example.test/api",
                "bridge_access_token": "bridge-token",
                "allowed_contact_ids": ["wxid_alice"]
            }
        }))
        .expect("deserialize weixin config");
        config.runtime_plugins.enabled = true;
        config.runtime_plugins.roots = vec![runtime_root.path().display().to_string()];
        config.runtime_plugins.supported_bridges = vec!["http_json".to_owned()];
        let runtime_root_path = runtime_root.path().to_path_buf();
        let runtime_root_path_for_snapshots = runtime_root_path.clone();

        let spec = crate::channel::runtime::serve::ChannelServeRuntimeSpec {
            platform: crate::channel::ChannelPlatform::Weixin,
            operation_id: crate::channel::CHANNEL_OPERATION_SERVE_ID,
            account_id: "default",
            account_label: "default",
        };

        let snapshots = crate::channel::runtime::serve::with_channel_serve_runtime_in_dir(
            runtime_root_path.as_path(),
            4343,
            spec,
            |runtime| async move {
                runtime
                    .record_failure("temporary bridge timeout")
                    .await
                    .expect("record runtime failure");
                Ok(super::super::channel_status_snapshots_with_now(
                    &config,
                    runtime_root_path_for_snapshots.as_path(),
                    crate::channel::runtime::serve::channel_runtime_now_ms(),
                ))
            },
        )
        .await
        .expect("produce retrying runtime snapshots");
        let weixin = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "weixin")
            .expect("weixin snapshot");
        let serve = weixin.operation("serve").expect("weixin serve operation");
        let runtime = serve.runtime.as_ref().expect("serve runtime");

        assert!(
            serve
                .detail
                .contains("runtime retrying after transient failures")
        );
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("consecutive_failures=1"))
        );
        assert_eq!(runtime.consecutive_failures, 1);
        assert_eq!(
            runtime.last_error.as_deref(),
            Some("temporary bridge timeout")
        );
    }

    #[test]
    fn onebot_status_reports_configured_bridge_surface_without_native_runtime() {
        let config: LoongConfig = serde_json::from_value(json!({
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
                .any(|issue| issue.contains("managed bridge runtime is disabled")),
            "send issues should explain managed bridge runtime requirements"
        );
        assert!(
            serve
                .issues
                .iter()
                .any(|issue| issue.contains("managed bridge runtime is disabled")),
            "serve issues should explain managed bridge runtime requirements"
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
