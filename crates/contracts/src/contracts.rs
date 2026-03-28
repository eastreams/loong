use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Capability {
    InvokeTool,
    InvokeConnector,
    MemoryRead,
    MemoryWrite,
    FilesystemRead,
    FilesystemWrite,
    NetworkEgress,
    ObserveTelemetry,
}

impl Capability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvokeTool => "invoke_tool",
            Self::InvokeConnector => "invoke_connector",
            Self::MemoryRead => "memory_read",
            Self::MemoryWrite => "memory_write",
            Self::FilesystemRead => "filesystem_read",
            Self::FilesystemWrite => "filesystem_write",
            Self::NetworkEgress => "network_egress",
            Self::ObserveTelemetry => "observe_telemetry",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "invoke_tool" => Some(Self::InvokeTool),
            "invoke_connector" => Some(Self::InvokeConnector),
            "memory_read" => Some(Self::MemoryRead),
            "memory_write" => Some(Self::MemoryWrite),
            "filesystem_read" => Some(Self::FilesystemRead),
            "filesystem_write" => Some(Self::FilesystemWrite),
            "network_egress" => Some(Self::NetworkEgress),
            "observe_telemetry" => Some(Self::ObserveTelemetry),
            _ => None,
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HarnessKind {
    EmbeddedPi,
    Acp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionRoute {
    pub harness_kind: HarnessKind,
    pub adapter: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityToken {
    pub token_id: String,
    pub pack_id: String,
    pub agent_id: String,
    pub allowed_capabilities: BTreeSet<Capability>,
    pub issued_at_epoch_s: u64,
    pub expires_at_epoch_s: u64,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskIntent {
    pub task_id: String,
    pub objective: String,
    pub required_capabilities: BTreeSet<Capability>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarnessRequest {
    pub token_id: String,
    pub pack_id: String,
    pub agent_id: String,
    pub task_id: String,
    pub objective: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarnessOutcome {
    pub status: String,
    pub output: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorCommand {
    pub connector_name: String,
    pub operation: String,
    pub required_capabilities: BTreeSet<Capability>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectorOutcome {
    pub status: String,
    pub payload: Value,
}

#[cfg(test)]
mod tests {
    use super::Capability;

    #[test]
    fn capability_round_trips_through_canonical_names() {
        let fixtures = [
            (Capability::InvokeTool, "invoke_tool"),
            (Capability::InvokeConnector, "invoke_connector"),
            (Capability::MemoryRead, "memory_read"),
            (Capability::MemoryWrite, "memory_write"),
            (Capability::FilesystemRead, "filesystem_read"),
            (Capability::FilesystemWrite, "filesystem_write"),
            (Capability::NetworkEgress, "network_egress"),
            (Capability::ObserveTelemetry, "observe_telemetry"),
        ];

        for (capability, expected_name) in fixtures {
            assert_eq!(capability.as_str(), expected_name);
            assert_eq!(Capability::parse(expected_name), Some(capability));
            assert_eq!(
                Capability::parse(&expected_name.to_ascii_uppercase()),
                Some(capability)
            );
            assert_eq!(
                Capability::parse(&expected_name.replace('_', "-")),
                Some(capability)
            );
        }
    }

    #[test]
    fn capability_parse_rejects_unknown_values() {
        assert_eq!(Capability::parse("totally_unknown"), None);
        assert_eq!(Capability::parse(""), None);
        assert_eq!(Capability::parse("   "), None);
        assert_eq!(Capability::parse("schedule_task"), None);
    }
}
