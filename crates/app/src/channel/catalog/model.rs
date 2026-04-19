use serde::Serialize;

use super::super::{ChannelCatalogTargetKind, ChannelPlatform};

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

pub const WHATSAPP_RUNTIME_COMMAND_DESCRIPTOR: ChannelRuntimeCommandDescriptor =
    ChannelRuntimeCommandDescriptor {
        channel_id: "whatsapp",
        platform: ChannelPlatform::WhatsApp,
        serve_bootstrap_agent_id: "channel-whatsapp",
    };

pub const LINE_RUNTIME_COMMAND_DESCRIPTOR: ChannelRuntimeCommandDescriptor =
    ChannelRuntimeCommandDescriptor {
        channel_id: "line",
        platform: ChannelPlatform::Line,
        serve_bootstrap_agent_id: "channel-line",
    };

pub const WEBHOOK_RUNTIME_COMMAND_DESCRIPTOR: ChannelRuntimeCommandDescriptor =
    ChannelRuntimeCommandDescriptor {
        channel_id: "webhook",
        platform: ChannelPlatform::Webhook,
        serve_bootstrap_agent_id: "channel-webhook",
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
    #[serde(skip)]
    pub(crate) default_target_kind: Option<ChannelCatalogTargetKind>,
    pub supported_target_kinds: &'static [ChannelCatalogTargetKind],
}

impl ChannelCatalogOperation {
    pub fn supports_target_kind(self, kind: ChannelCatalogTargetKind) -> bool {
        self.supported_target_kinds.contains(&kind)
    }

    pub fn default_target_kind(self) -> Option<ChannelCatalogTargetKind> {
        self.default_target_kind
            .or_else(|| self.supported_target_kinds.first().copied())
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
    ManagedBridge,
    Stub,
}

impl ChannelCatalogOperationAvailability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Implemented => "implemented",
            Self::ManagedBridge => "managed_bridge",
            Self::Stub => "stub",
        }
    }

    pub const fn is_runnable(self) -> bool {
        match self {
            Self::Implemented | Self::ManagedBridge => true,
            Self::Stub => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelCapability {
    RuntimeBacked,
    PluginBacked,
    MultiAccount,
    Send,
    Serve,
    RuntimeTracking,
}

impl ChannelCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeBacked => "runtime_backed",
            Self::PluginBacked => "plugin_backed",
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
    QrRegistration,
    PluginBridge,
    Planned,
}

impl ChannelOnboardingStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManualConfig => "manual_config",
            Self::QrRegistration => "qr_registration",
            Self::PluginBridge => "plugin_bridge",
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
    PluginBridgeHealth,
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
    ConfigBacked,
    PluginBacked,
    Stub,
}

impl ChannelCatalogImplementationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeBacked => "runtime_backed",
            Self::ConfigBacked => "config_backed",
            Self::PluginBacked => "plugin_backed",
            Self::Stub => "stub",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_catalog_operation_prefers_explicit_default_target_kind() {
        let operation = ChannelCatalogOperation {
            id: CHANNEL_OPERATION_SEND_ID,
            label: "direct send",
            command: "feishu-send",
            availability: ChannelCatalogOperationAvailability::Implemented,
            tracks_runtime: false,
            requirements: &[],
            default_target_kind: Some(ChannelCatalogTargetKind::MessageReply),
            supported_target_kinds: &[
                ChannelCatalogTargetKind::ReceiveId,
                ChannelCatalogTargetKind::MessageReply,
            ],
        };

        assert_eq!(
            operation.default_target_kind(),
            Some(ChannelCatalogTargetKind::MessageReply)
        );
    }
}
