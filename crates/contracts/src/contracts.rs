use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Capability {
    InvokeTool,
    InvokeConnector,
    MemoryRead,
    MemoryWrite,
    FilesystemRead,
    FilesystemWrite,
    NetworkEgress,
    ScheduleTask,
    ObserveTelemetry,
}

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
