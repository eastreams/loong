use serde::{Deserialize, Serialize};

use crate::contracts::{Capability, CapabilityToken, ExecutionRoute};

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionPlane {
    Connector,
    Runtime,
    Tool,
    Memory,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaneTier {
    Legacy,
    Core,
    Extension,
}

#[non_exhaustive]
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
    ProviderFailover {
        pack_id: String,
        provider_id: String,
        reason: String,
        stage: String,
        model: String,
        attempt: usize,
        max_attempts: usize,
        status_code: Option<u16>,
        try_next_model: bool,
        auto_model_mode: bool,
        candidate_index: usize,
        candidate_count: usize,
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
