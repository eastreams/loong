use serde::{Deserialize, Serialize};

use crate::contracts::{Capability, CapabilityToken, ExecutionRoute};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionPlane {
    Connector,
    Runtime,
    Tool,
    Memory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaneTier {
    Legacy,
    Core,
    Extension,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditEventKind {
    TokenIssued {
        token: CapabilityToken,
    },
    TokenRevoked {
        token_id: String,
    },
    TaskDispatched {
        pack_id: String,
        task_id: String,
        route: ExecutionRoute,
        required_capabilities: Vec<Capability>,
    },
    ConnectorInvoked {
        pack_id: String,
        connector_name: String,
        operation: String,
        required_capabilities: Vec<Capability>,
    },
    PlaneInvoked {
        pack_id: String,
        plane: ExecutionPlane,
        tier: PlaneTier,
        primary_adapter: String,
        delegated_core_adapter: Option<String>,
        operation: String,
        required_capabilities: Vec<Capability>,
    },
    SecurityScanEvaluated {
        pack_id: String,
        scanned_plugins: usize,
        total_findings: usize,
        high_findings: usize,
        medium_findings: usize,
        low_findings: usize,
        blocked: bool,
        block_reason: Option<String>,
        categories: Vec<String>,
        finding_ids: Vec<String>,
    },
    AuthorizationDenied {
        pack_id: String,
        token_id: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: String,
    pub timestamp_epoch_s: u64,
    pub agent_id: Option<String>,
    pub kind: AuditEventKind,
}
