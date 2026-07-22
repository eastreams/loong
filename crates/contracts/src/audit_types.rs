use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    AuthorizationEvidence, AuthorizationSubject, Capabilities, GrantId, ToolInputError,
    contracts::{Capability, CapabilityToken, ExecutionRoute},
};

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionPlane {
    /// Connector invocations that cross a named integration boundary.
    Connector,
    /// Agent/runtime execution such as harness-backed tasks or provider turns.
    Runtime,
    /// Tool invocations exposed to the agent or conversation runtime.
    Tool,
    /// Memory reads/writes and memory-index operations.
    Memory,
}

/// Where an invocation lives within an [`ExecutionPlane`].
///
/// `ExecutionPlane` answers "what kind of subsystem is being invoked", while
/// `PlaneTier` answers "which layer inside that subsystem handled it". Keep
/// policy and audit checks that care about broad capability domains on
/// `ExecutionPlane`; use `PlaneTier` only when distinguishing built-in/core
/// execution from extension-mediated execution.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaneTier {
    /// Compatibility path that predates the core/extension split.
    Legacy,
    /// Built-in kernel-owned implementation for the plane.
    Core,
    /// Add-on implementation that wraps or delegates to a core implementation.
    Extension,
}

/// Execution evidence for one granted typed action.
///
/// The authorization event referenced by `grant_id` owns action identity,
/// required capabilities, policy report, and subject attribution. Repeating
/// those fields here would permit the execution record to drift from its grant.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionExecutionEvent {
    /// The domain executor accepted the grant and is about to dispatch.
    Started,
    /// The concrete action completed normally.
    Completed,
    /// Post-grant input parsing rejected the action payload.
    InputRejected { error: ToolInputError },
    /// Dispatch reached a terminal non-input failure.
    Failed { reason: String },
    /// Dispatch started, but its owner was dropped before observing an outcome.
    ///
    /// This deliberately records only the fact known by the owner. It does not
    /// guess whether the action completed, failed, or was externally cancelled.
    OutcomeUnknown,
}

/// Terminal shape used by tool invocation events written before grant-linked
/// execution evidence existed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoricalToolInvocationOutcome {
    Completed,
    Failed { error_kind: String, reason: String },
}

/// Result of an app-runtime operation that is observable but carries no grant.
///
/// This evidence must not be interpreted as authorization or action execution;
/// those stronger claims are represented by `Authorization` and
/// `ActionExecution`, whose writers require the corresponding typed proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeOperationOutcome {
    Completed,
    Failed { reason: String },
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditEventKind {
    /// Typed evidence emitted by the authorization engine.
    Authorization {
        evidence: AuthorizationEvidence,
    },
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
    /// Execution evidence correlated to one real typed action grant.
    ActionExecution {
        grant_id: GrantId,
        event: ActionExecutionEvent,
    },
    /// A tool invocation requested authority outside its declared default.
    ToolCapabilityOverrideRejected {
        subject: AuthorizationSubject,
        path_display: String,
        requested: Capabilities,
        declared: Capabilities,
    },
    /// Pack-free operational evidence emitted by the app runtime owner.
    RuntimeOperation {
        operation: String,
        outcome: RuntimeOperationOutcome,
    },
    /// Historical pre-grant-linked tool invocation journal record.
    ///
    /// New code must use `ActionExecution`; this variant remains decodable
    /// because append-only audit journals are immutable input.
    ToolInvocation {
        pack_id: String,
        path_display: String,
        required_capabilities: Vec<Capability>,
        outcome: HistoricalToolInvocationOutcome,
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
    PluginTrustEvaluated {
        pack_id: String,
        scanned_plugins: usize,
        official_plugins: usize,
        verified_community_plugins: usize,
        unverified_plugins: usize,
        high_risk_plugins: usize,
        high_risk_unverified_plugins: usize,
        blocked_auto_apply_plugins: usize,
        review_required_plugin_ids: Vec<String>,
        review_required_bridges: Vec<String>,
    },
    ToolSearchEvaluated {
        pack_id: String,
        query: String,
        returned: usize,
        trust_filter_applied: bool,
        query_requested_tiers: Vec<String>,
        structured_requested_tiers: Vec<String>,
        effective_tiers: Vec<String>,
        conflicting_requested_tiers: bool,
        filtered_out_candidates: usize,
        filtered_out_tier_counts: BTreeMap<String, usize>,
        top_provider_ids: Vec<String>,
    },
    /// Provider retry evidence belongs to the runtime request, not a legacy pack.
    ProviderFailover {
        provider_id: String,
        reason: String,
        stage: String,
        model: String,
        attempt: usize,
        max_attempts: usize,
        status_code: Option<u16>,
        request_id: Option<String>,
        cf_ray: Option<String>,
        auth_error: Option<String>,
        auth_error_code: Option<String>,
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
