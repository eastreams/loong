use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use kernel::{
    ArchitectureGuardReport, AuditEvent, BootstrapReport, BridgeSupportMatrix,
    CURRENT_PLUGIN_HOST_API, Capability, CodebaseAwarenessSnapshot, ConnectorCommand,
    ConnectorError, ConnectorOutcome, CoreConnectorAdapter, CoreMemoryAdapter, CoreRuntimeAdapter,
    CoreToolAdapter, ExecutionRoute, HarnessAdapter, HarnessError, HarnessKind, HarnessOutcome,
    HarnessRequest, IntegrationCatalog, IntegrationHotfix, MemoryCoreOutcome, MemoryCoreRequest,
    MemoryExtensionAdapter, MemoryExtensionOutcome, MemoryExtensionRequest, PluginAbsorbReport,
    PluginActivationPlan, PluginBridgeKind, PluginCompatibility, PluginCompatibilityMode,
    PluginCompatibilityShim, PluginCompatibilityShimSupport, PluginContractDialect,
    PluginDiagnosticFinding, PluginIR, PluginRuntimeProfile, PluginScanReport, PluginSlotClaim,
    PluginSourceKind, PluginTranslationReport, ProvisionPlan, RuntimeCoreOutcome,
    RuntimeCoreRequest, RuntimeExtensionAdapter, RuntimeExtensionOutcome, RuntimeExtensionRequest,
    ToolCoreOutcome, ToolCoreRequest, ToolExtensionAdapter, ToolExtensionOutcome,
    ToolExtensionRequest, VerticalPackManifest,
};
use loongclaw_contracts::ExecutionSecurityTier;
use loongclaw_protocol::{
    OutboundFrame, PROTOCOL_VERSION, ProtocolRouter, RouteAuthorizationRequest,
};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as TokioMutex;
use tokio::time::Instant as TokioInstant;
#[cfg(any(test, feature = "test-hooks"))]
use tokio::time::sleep;
use wasmtime::{
    Config as WasmtimeConfig, Engine as WasmtimeEngine, Linker as WasmtimeLinker,
    Module as WasmtimeModule, Store as WasmtimeStore,
};

#[cfg(any(test, feature = "test-hooks"))]
use crate::WEBHOOK_TEST_RETRY_STATE;
use crate::spec_execution::{normalize_path_for_policy, resolve_plugin_relative_path};

mod http_json_bridge;
mod process_stdio_bridge;
mod wasm_cache;
mod wasm_runtime_policy;
pub use http_json_bridge::execute_http_json_bridge;
pub use process_stdio_bridge::{
    ProcessStdioExchangeOutcome, execute_process_stdio_bridge, run_process_stdio_json_line_exchange,
};
#[cfg(test)]
use wasm_cache::WasmModuleCache;
use wasm_cache::{
    CachedWasmModule, WasmArtifactFileIdentity, build_wasm_module_cache_key,
    insert_cached_wasm_module, lookup_cached_wasm_module, modified_unix_nanos,
    wasm_artifact_file_identity, wasm_module_cache_capacity, wasm_module_cache_max_bytes,
};
use wasm_runtime_policy::wasm_signals_based_traps_enabled_from_env;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultCoreSelection {
    pub connector: Option<String>,
    pub runtime: Option<String>,
    pub tool: Option<String>,
    pub memory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OperationSpec {
    Task {
        task_id: String,
        objective: String,
        required_capabilities: BTreeSet<Capability>,
        payload: Value,
    },
    ConnectorLegacy {
        connector_name: String,
        operation: String,
        required_capabilities: BTreeSet<Capability>,
        payload: Value,
    },
    ConnectorCore {
        connector_name: String,
        operation: String,
        required_capabilities: BTreeSet<Capability>,
        payload: Value,
        core: Option<String>,
    },
    ConnectorExtension {
        connector_name: String,
        operation: String,
        required_capabilities: BTreeSet<Capability>,
        payload: Value,
        extension: String,
        core: Option<String>,
    },
    RuntimeCore {
        action: String,
        required_capabilities: BTreeSet<Capability>,
        payload: Value,
        core: Option<String>,
    },
    RuntimeExtension {
        action: String,
        required_capabilities: BTreeSet<Capability>,
        payload: Value,
        extension: String,
        core: Option<String>,
    },
    ToolCore {
        tool_name: String,
        required_capabilities: BTreeSet<Capability>,
        payload: Value,
        core: Option<String>,
    },
    ToolExtension {
        extension_action: String,
        required_capabilities: BTreeSet<Capability>,
        payload: Value,
        extension: String,
        core: Option<String>,
    },
    MemoryCore {
        operation: String,
        required_capabilities: BTreeSet<Capability>,
        payload: Value,
        core: Option<String>,
    },
    MemoryExtension {
        operation: String,
        required_capabilities: BTreeSet<Capability>,
        payload: Value,
        extension: String,
        core: Option<String>,
    },
    ToolSearch {
        query: String,
        #[serde(default = "default_tool_search_limit")]
        limit: usize,
        #[serde(default)]
        include_deferred: bool,
        #[serde(default)]
        include_examples: bool,
    },
    PluginInventory {
        #[serde(default)]
        query: String,
        #[serde(default = "default_plugin_inventory_limit")]
        limit: usize,
        #[serde(default = "default_true")]
        include_ready: bool,
        #[serde(default = "default_true")]
        include_blocked: bool,
        #[serde(default = "default_true")]
        include_deferred: bool,
        #[serde(default)]
        include_examples: bool,
    },
    PluginPreflight {
        #[serde(default)]
        query: String,
        #[serde(default = "default_plugin_preflight_limit")]
        limit: usize,
        #[serde(default = "default_plugin_preflight_profile")]
        profile: PluginPreflightProfile,
        #[serde(default)]
        policy_path: Option<String>,
        #[serde(default)]
        policy_sha256: Option<String>,
        #[serde(default)]
        policy_signature: Option<SecurityProfileSignatureSpec>,
        #[serde(default = "default_true")]
        include_passed: bool,
        #[serde(default = "default_true")]
        include_warned: bool,
        #[serde(default = "default_true")]
        include_blocked: bool,
        #[serde(default = "default_true")]
        include_deferred: bool,
        #[serde(default)]
        include_examples: bool,
    },
    ProgrammaticToolCall {
        caller: String,
        #[serde(default = "default_programmatic_max_calls")]
        max_calls: usize,
        #[serde(default)]
        include_intermediate: bool,
        #[serde(default)]
        allowed_connectors: BTreeSet<String>,
        #[serde(default)]
        connector_rate_limits: BTreeMap<String, ProgrammaticConnectorRateLimit>,
        #[serde(default)]
        connector_circuit_breakers: BTreeMap<String, ProgrammaticCircuitBreakerPolicy>,
        #[serde(default = "default_programmatic_concurrency_policy")]
        concurrency: ProgrammaticConcurrencyPolicy,
        #[serde(default)]
        return_step: Option<String>,
        steps: Vec<ProgrammaticStep>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProgrammaticStep {
    SetLiteral {
        step_id: String,
        value: Value,
    },
    JsonPointer {
        step_id: String,
        from_step: String,
        pointer: String,
    },
    ConnectorCall {
        step_id: String,
        connector_name: String,
        operation: String,
        #[serde(default)]
        required_capabilities: BTreeSet<Capability>,
        #[serde(default)]
        retry: Option<ProgrammaticRetryPolicy>,
        #[serde(default = "default_programmatic_priority_class")]
        priority_class: ProgrammaticPriorityClass,
        #[serde(default)]
        payload: Value,
    },
    ConnectorBatch {
        step_id: String,
        #[serde(default = "default_true")]
        parallel: bool,
        #[serde(default)]
        continue_on_error: bool,
        calls: Vec<ProgrammaticBatchCall>,
    },
    Conditional {
        step_id: String,
        from_step: String,
        #[serde(default)]
        pointer: Option<String>,
        #[serde(default)]
        equals: Option<Value>,
        #[serde(default)]
        exists: Option<bool>,
        when_true: Value,
        #[serde(default)]
        when_false: Option<Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgrammaticBatchCall {
    pub call_id: String,
    pub connector_name: String,
    pub operation: String,
    #[serde(default)]
    pub required_capabilities: BTreeSet<Capability>,
    #[serde(default)]
    pub retry: Option<ProgrammaticRetryPolicy>,
    #[serde(default = "default_programmatic_priority_class")]
    pub priority_class: ProgrammaticPriorityClass,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgrammaticRetryPolicy {
    #[serde(default = "default_programmatic_retry_max_attempts")]
    pub max_attempts: usize,
    #[serde(default = "default_programmatic_retry_initial_backoff_ms")]
    pub initial_backoff_ms: u64,
    #[serde(default = "default_programmatic_retry_max_backoff_ms")]
    pub max_backoff_ms: u64,
    #[serde(default = "default_programmatic_retry_jitter_ratio")]
    pub jitter_ratio: f64,
    #[serde(default = "default_true")]
    pub adaptive_jitter: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgrammaticConnectorRateLimit {
    pub min_interval_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProgrammaticPriorityClass {
    High,
    Normal,
    Low,
}

impl ProgrammaticPriorityClass {
    pub fn as_str(self) -> &'static str {
        match self {
            ProgrammaticPriorityClass::High => "high",
            ProgrammaticPriorityClass::Normal => "normal",
            ProgrammaticPriorityClass::Low => "low",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProgrammaticFairnessPolicy {
    StrictRoundRobin,
    WeightedRoundRobin,
}

impl ProgrammaticFairnessPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            ProgrammaticFairnessPolicy::StrictRoundRobin => "strict_round_robin",
            ProgrammaticFairnessPolicy::WeightedRoundRobin => "weighted_round_robin",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProgrammaticAdaptiveReduceOn {
    AnyError,
    ConnectorExecutionError,
    CircuitOpen,
    ConnectorNotFound,
    ConnectorNotAllowed,
    CapabilityDenied,
    PolicyDenied,
}

impl ProgrammaticAdaptiveReduceOn {
    pub fn as_str(self) -> &'static str {
        match self {
            ProgrammaticAdaptiveReduceOn::AnyError => "any_error",
            ProgrammaticAdaptiveReduceOn::ConnectorExecutionError => "connector_execution_error",
            ProgrammaticAdaptiveReduceOn::CircuitOpen => "circuit_open",
            ProgrammaticAdaptiveReduceOn::ConnectorNotFound => "connector_not_found",
            ProgrammaticAdaptiveReduceOn::ConnectorNotAllowed => "connector_not_allowed",
            ProgrammaticAdaptiveReduceOn::CapabilityDenied => "capability_denied",
            ProgrammaticAdaptiveReduceOn::PolicyDenied => "policy_denied",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgrammaticConcurrencyPolicy {
    #[serde(default = "default_programmatic_concurrency_max_in_flight")]
    pub max_in_flight: usize,
    #[serde(default = "default_programmatic_concurrency_min_in_flight")]
    pub min_in_flight: usize,
    #[serde(default = "default_programmatic_fairness_policy")]
    pub fairness: ProgrammaticFairnessPolicy,
    #[serde(default = "default_true")]
    pub adaptive_budget: bool,
    #[serde(default = "default_programmatic_priority_high_weight")]
    pub high_weight: usize,
    #[serde(default = "default_programmatic_priority_normal_weight")]
    pub normal_weight: usize,
    #[serde(default = "default_programmatic_priority_low_weight")]
    pub low_weight: usize,
    #[serde(default = "default_programmatic_adaptive_recovery_successes")]
    pub adaptive_recovery_successes: usize,
    #[serde(default = "default_programmatic_adaptive_upshift_step")]
    pub adaptive_upshift_step: usize,
    #[serde(default = "default_programmatic_adaptive_downshift_step")]
    pub adaptive_downshift_step: usize,
    #[serde(default = "default_programmatic_adaptive_reduce_on")]
    pub adaptive_reduce_on: BTreeSet<ProgrammaticAdaptiveReduceOn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorCircuitBreakerPolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_programmatic_circuit_failure_threshold")]
    pub failure_threshold: usize,
    #[serde(default = "default_programmatic_circuit_cooldown_ms")]
    pub cooldown_ms: u64,
    #[serde(default = "default_programmatic_circuit_half_open_max_calls")]
    pub half_open_max_calls: usize,
    #[serde(default = "default_programmatic_circuit_success_threshold")]
    pub success_threshold: usize,
}

impl Default for ConnectorCircuitBreakerPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            failure_threshold: default_programmatic_circuit_failure_threshold(),
            cooldown_ms: default_programmatic_circuit_cooldown_ms(),
            half_open_max_calls: default_programmatic_circuit_half_open_max_calls(),
            success_threshold: default_programmatic_circuit_success_threshold(),
        }
    }
}

pub type ProgrammaticCircuitBreakerPolicy = ConnectorCircuitBreakerPolicy;

#[derive(Debug, Clone)]
pub struct PreparedProgrammaticBatchCall {
    pub call_id: String,
    pub connector_name: String,
    pub operation: String,
    pub required_capabilities: BTreeSet<Capability>,
    pub retry_policy: ProgrammaticRetryPolicy,
    pub priority_class: ProgrammaticPriorityClass,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgrammaticInvocationMetrics {
    pub attempts: usize,
    pub retries: usize,
    pub priority_class: String,
    pub rate_wait_ms_total: u64,
    pub backoff_ms_total: u64,
    pub circuit_phase_before: String,
    pub circuit_phase_after: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgrammaticBatchExecutionSummary {
    pub mode: String,
    pub fairness: String,
    pub adaptive_budget: bool,
    pub configured_max_in_flight: usize,
    pub configured_min_in_flight: usize,
    pub peak_in_flight: usize,
    pub final_in_flight_budget: usize,
    pub budget_reductions: usize,
    pub budget_increases: usize,
    pub adaptive_upshift_step: usize,
    pub adaptive_downshift_step: usize,
    pub adaptive_reduce_on: Vec<String>,
    pub scheduler_wait_cycles: usize,
    pub dispatch_order: Vec<String>,
    pub priority_dispatch_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorCircuitPhase {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone)]
pub struct ConnectorCircuitRuntimeState {
    pub phase: ConnectorCircuitPhase,
    pub consecutive_failures: usize,
    pub open_until: Option<TokioInstant>,
    pub half_open_remaining_calls: usize,
    pub half_open_successes: usize,
}

impl Default for ConnectorCircuitRuntimeState {
    fn default() -> Self {
        Self {
            phase: ConnectorCircuitPhase::Closed,
            consecutive_failures: 0,
            open_until: None,
            half_open_remaining_calls: 0,
            half_open_successes: 0,
        }
    }
}

pub type ProgrammaticCircuitPhase = ConnectorCircuitPhase;
pub type ProgrammaticCircuitRuntimeState = ConnectorCircuitRuntimeState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorCircuitAcquireError {
    Open { remaining_cooldown_ms: u64 },
    HalfOpenReopened,
}

pub fn validate_connector_circuit_breaker_policy(
    policy: &ConnectorCircuitBreakerPolicy,
    context: &str,
) -> Result<(), String> {
    if !policy.enabled {
        return Ok(());
    }
    if policy.failure_threshold == 0 {
        return Err(format!(
            "{context} failure_threshold must be greater than 0"
        ));
    }
    if policy.cooldown_ms == 0 {
        return Err(format!("{context} cooldown_ms must be greater than 0"));
    }
    if policy.half_open_max_calls == 0 {
        return Err(format!(
            "{context} half_open_max_calls must be greater than 0"
        ));
    }
    if policy.success_threshold == 0 {
        return Err(format!(
            "{context} success_threshold must be greater than 0"
        ));
    }
    if policy.success_threshold > policy.half_open_max_calls {
        return Err(format!(
            "{context} success_threshold must be <= half_open_max_calls"
        ));
    }
    Ok(())
}

pub const fn connector_circuit_phase_label(phase: ConnectorCircuitPhase) -> &'static str {
    match phase {
        ConnectorCircuitPhase::Closed => "closed",
        ConnectorCircuitPhase::Open => "open",
        ConnectorCircuitPhase::HalfOpen => "half_open",
    }
}

pub fn connector_circuit_remaining_cooldown_ms(
    state: &ConnectorCircuitRuntimeState,
    now: TokioInstant,
) -> Option<u64> {
    let open_until = state.open_until?;
    if open_until <= now {
        return Some(0);
    }

    let remaining_duration = open_until.duration_since(now);
    let remaining_ms = remaining_duration.as_millis();
    let clamped_ms = remaining_ms.min(u128::from(u64::MAX));

    Some(clamped_ms as u64)
}

pub fn acquire_connector_circuit_slot_for_state(
    policy: &ConnectorCircuitBreakerPolicy,
    state: &mut ConnectorCircuitRuntimeState,
    now: TokioInstant,
) -> Result<&'static str, ConnectorCircuitAcquireError> {
    if !policy.enabled {
        return Ok("disabled");
    }

    if state.phase == ConnectorCircuitPhase::Open {
        let remaining_cooldown_ms = connector_circuit_remaining_cooldown_ms(state, now);
        if let Some(remaining_cooldown_ms) = remaining_cooldown_ms
            && remaining_cooldown_ms > 0
        {
            return Err(ConnectorCircuitAcquireError::Open {
                remaining_cooldown_ms,
            });
        }

        state.phase = ConnectorCircuitPhase::HalfOpen;
        state.open_until = None;
        state.half_open_remaining_calls = policy.half_open_max_calls;
        state.half_open_successes = 0;
    }

    if state.phase == ConnectorCircuitPhase::HalfOpen {
        if state.half_open_remaining_calls == 0 {
            let reopen_deadline = now + Duration::from_millis(policy.cooldown_ms);

            state.phase = ConnectorCircuitPhase::Open;
            state.open_until = Some(reopen_deadline);
            return Err(ConnectorCircuitAcquireError::HalfOpenReopened);
        }

        state.half_open_remaining_calls = state.half_open_remaining_calls.saturating_sub(1);
        return Ok("half_open");
    }

    Ok("closed")
}

pub fn record_connector_circuit_outcome_for_state(
    policy: &ConnectorCircuitBreakerPolicy,
    state: &mut ConnectorCircuitRuntimeState,
    success: bool,
    now: TokioInstant,
) -> &'static str {
    if !policy.enabled {
        return "disabled";
    }

    match state.phase {
        ConnectorCircuitPhase::Closed => {
            if success {
                state.consecutive_failures = 0;
            } else {
                state.consecutive_failures = state.consecutive_failures.saturating_add(1);
                if state.consecutive_failures >= policy.failure_threshold {
                    let reopen_deadline = now + Duration::from_millis(policy.cooldown_ms);

                    state.phase = ConnectorCircuitPhase::Open;
                    state.open_until = Some(reopen_deadline);
                    state.half_open_remaining_calls = 0;
                    state.half_open_successes = 0;
                }
            }
        }
        ConnectorCircuitPhase::HalfOpen => {
            if success {
                state.half_open_successes = state.half_open_successes.saturating_add(1);
                if state.half_open_successes >= policy.success_threshold {
                    state.phase = ConnectorCircuitPhase::Closed;
                    state.consecutive_failures = 0;
                    state.open_until = None;
                    state.half_open_remaining_calls = 0;
                    state.half_open_successes = 0;
                } else if state.half_open_remaining_calls == 0 {
                    let reopen_deadline = now + Duration::from_millis(policy.cooldown_ms);

                    state.phase = ConnectorCircuitPhase::Open;
                    state.open_until = Some(reopen_deadline);
                    state.half_open_successes = 0;
                }
            } else {
                let reopen_deadline = now + Duration::from_millis(policy.cooldown_ms);

                state.phase = ConnectorCircuitPhase::Open;
                state.open_until = Some(reopen_deadline);
                state.half_open_remaining_calls = 0;
                state.half_open_successes = 0;
            }
        }
        ConnectorCircuitPhase::Open => {}
    }

    connector_circuit_phase_label(state.phase)
}

pub fn default_tool_search_limit() -> usize {
    8
}

pub fn default_plugin_inventory_limit() -> usize {
    24
}

pub fn default_plugin_preflight_limit() -> usize {
    100
}

pub fn default_programmatic_max_calls() -> usize {
    12
}

pub fn default_programmatic_retry_max_attempts() -> usize {
    1
}

pub fn default_programmatic_retry_initial_backoff_ms() -> u64 {
    100
}

pub fn default_programmatic_retry_max_backoff_ms() -> u64 {
    2_000
}

pub fn default_programmatic_retry_jitter_ratio() -> f64 {
    0.2
}

pub fn default_programmatic_priority_class() -> ProgrammaticPriorityClass {
    ProgrammaticPriorityClass::Normal
}

pub fn default_programmatic_concurrency_max_in_flight() -> usize {
    4
}

pub fn default_programmatic_concurrency_min_in_flight() -> usize {
    1
}

pub fn default_programmatic_fairness_policy() -> ProgrammaticFairnessPolicy {
    ProgrammaticFairnessPolicy::WeightedRoundRobin
}

pub fn default_programmatic_priority_high_weight() -> usize {
    4
}

pub fn default_programmatic_priority_normal_weight() -> usize {
    2
}

pub fn default_programmatic_priority_low_weight() -> usize {
    1
}

pub fn default_programmatic_adaptive_recovery_successes() -> usize {
    2
}

pub fn default_programmatic_adaptive_upshift_step() -> usize {
    1
}

pub fn default_programmatic_adaptive_downshift_step() -> usize {
    1
}

pub fn default_programmatic_adaptive_reduce_on() -> BTreeSet<ProgrammaticAdaptiveReduceOn> {
    BTreeSet::from([
        ProgrammaticAdaptiveReduceOn::ConnectorExecutionError,
        ProgrammaticAdaptiveReduceOn::CircuitOpen,
    ])
}

pub fn default_programmatic_concurrency_policy() -> ProgrammaticConcurrencyPolicy {
    ProgrammaticConcurrencyPolicy {
        max_in_flight: default_programmatic_concurrency_max_in_flight(),
        min_in_flight: default_programmatic_concurrency_min_in_flight(),
        fairness: default_programmatic_fairness_policy(),
        adaptive_budget: default_true(),
        high_weight: default_programmatic_priority_high_weight(),
        normal_weight: default_programmatic_priority_normal_weight(),
        low_weight: default_programmatic_priority_low_weight(),
        adaptive_recovery_successes: default_programmatic_adaptive_recovery_successes(),
        adaptive_upshift_step: default_programmatic_adaptive_upshift_step(),
        adaptive_downshift_step: default_programmatic_adaptive_downshift_step(),
        adaptive_reduce_on: default_programmatic_adaptive_reduce_on(),
    }
}

pub fn default_programmatic_circuit_failure_threshold() -> usize {
    3
}

pub fn default_programmatic_circuit_cooldown_ms() -> u64 {
    1_000
}

pub fn default_programmatic_circuit_half_open_max_calls() -> usize {
    1
}

pub fn default_programmatic_circuit_success_threshold() -> usize {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunnerSpec {
    pub pack: VerticalPackManifest,
    pub agent_id: String,
    pub ttl_s: u64,
    #[serde(default)]
    pub approval: Option<HumanApprovalSpec>,
    pub defaults: Option<DefaultCoreSelection>,
    pub self_awareness: Option<SelfAwarenessSpec>,
    pub plugin_scan: Option<PluginScanSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_setup_readiness: Option<PluginSetupReadinessSpec>,
    pub bridge_support: Option<BridgeSupportSpec>,
    pub bootstrap: Option<BootstrapSpec>,
    pub auto_provision: Option<AutoProvisionSpec>,
    #[serde(default)]
    pub hotfixes: Vec<HotfixSpec>,
    pub operation: OperationSpec,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HumanApprovalMode {
    Disabled,
    #[default]
    MediumBalanced,
    Strict,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HumanApprovalStrategy {
    #[default]
    PerCall,
    OneTimeFullAccess,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HumanApprovalScope {
    #[default]
    ToolCalls,
    AllOperations,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanApprovalSpec {
    #[serde(default)]
    pub mode: HumanApprovalMode,
    #[serde(default)]
    pub strategy: HumanApprovalStrategy,
    #[serde(default)]
    pub scope: HumanApprovalScope,
    #[serde(default)]
    pub one_time_full_access_granted: bool,
    #[serde(default)]
    pub approved_calls: Vec<String>,
    #[serde(default)]
    pub denied_calls: Vec<String>,
    #[serde(default)]
    pub risk_profile_path: Option<String>,
    #[serde(default)]
    pub one_time_full_access_expires_at_epoch_s: Option<u64>,
    #[serde(default)]
    pub one_time_full_access_remaining_uses: Option<u32>,
    #[serde(default)]
    pub high_risk_keywords: Vec<String>,
    #[serde(default)]
    pub high_risk_tool_names: Vec<String>,
    #[serde(default)]
    pub high_risk_payload_keys: Vec<String>,
}

impl Default for HumanApprovalSpec {
    fn default() -> Self {
        Self {
            mode: HumanApprovalMode::MediumBalanced,
            strategy: HumanApprovalStrategy::PerCall,
            scope: HumanApprovalScope::ToolCalls,
            one_time_full_access_granted: false,
            approved_calls: Vec::new(),
            denied_calls: Vec::new(),
            risk_profile_path: None,
            one_time_full_access_expires_at_epoch_s: None,
            one_time_full_access_remaining_uses: None,
            // Keep inline overrides empty by default so policy is profile-driven.
            high_risk_keywords: Vec::new(),
            high_risk_tool_names: Vec::new(),
            high_risk_payload_keys: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRiskLevel {
    Low,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalDecisionReport {
    pub mode: HumanApprovalMode,
    pub strategy: HumanApprovalStrategy,
    pub scope: HumanApprovalScope,
    pub now_epoch_s: u64,
    pub operation_key: String,
    pub operation_kind: &'static str,
    pub risk_level: ApprovalRiskLevel,
    pub risk_score: u8,
    pub denylisted: bool,
    pub requires_human_approval: bool,
    pub approved: bool,
    pub reason: String,
    pub matched_keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRiskScoring {
    pub keyword_weight: u8,
    pub tool_name_weight: u8,
    pub payload_key_weight: u8,
    pub keyword_hit_cap: usize,
    pub payload_key_hit_cap: usize,
    pub high_risk_threshold: u8,
}

impl Default for ApprovalRiskScoring {
    fn default() -> Self {
        Self {
            keyword_weight: 20,
            tool_name_weight: 40,
            payload_key_weight: 20,
            keyword_hit_cap: 4,
            payload_key_hit_cap: 2,
            high_risk_threshold: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRiskProfile {
    #[serde(default)]
    pub high_risk_keywords: Vec<String>,
    #[serde(default)]
    pub high_risk_tool_names: Vec<String>,
    #[serde(default)]
    pub high_risk_payload_keys: Vec<String>,
    #[serde(default)]
    pub scoring: ApprovalRiskScoring,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfAwarenessSpec {
    pub enabled: bool,
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default)]
    pub plugin_roots: Vec<String>,
    #[serde(default)]
    pub proposed_mutations: Vec<String>,
    #[serde(default)]
    pub enforce_guard: bool,
    #[serde(default)]
    pub immutable_core_paths: Vec<String>,
    #[serde(default)]
    pub mutable_extension_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginScanSpec {
    pub enabled: bool,
    pub roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginSetupReadinessSpec {
    #[serde(default = "default_true")]
    pub inherit_process_env: bool,
    #[serde(default)]
    pub verified_env_vars: Vec<String>,
    #[serde(default)]
    pub verified_config_keys: Vec<String>,
}

impl Default for PluginSetupReadinessSpec {
    fn default() -> Self {
        Self {
            inherit_process_env: true,
            verified_env_vars: Vec::new(),
            verified_config_keys: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeSupportSpec {
    pub enabled: bool,
    #[serde(default)]
    pub supported_bridges: Vec<PluginBridgeKind>,
    #[serde(default)]
    pub supported_adapter_families: Vec<String>,
    #[serde(default)]
    pub supported_compatibility_modes: Vec<PluginCompatibilityMode>,
    #[serde(default)]
    pub supported_compatibility_shims: Vec<PluginCompatibilityShim>,
    #[serde(default)]
    pub supported_compatibility_shim_profiles: Vec<PluginCompatibilityShimSupport>,
    #[serde(default)]
    pub enforce_supported: bool,
    #[serde(default)]
    pub policy_version: Option<String>,
    #[serde(default)]
    pub expected_checksum: Option<String>,
    #[serde(default)]
    pub expected_sha256: Option<String>,
    #[serde(default)]
    pub execute_process_stdio: bool,
    #[serde(default)]
    pub execute_http_json: bool,
    #[serde(default)]
    pub allowed_process_commands: Vec<String>,
    #[serde(default)]
    pub enforce_execution_success: bool,
    #[serde(default)]
    pub security_scan: Option<SecurityScanSpec>,
}

#[derive(Debug, Clone, Default)]
pub struct BridgeRuntimePolicy {
    pub execute_process_stdio: bool,
    pub execute_http_json: bool,
    pub execute_wasm_component: bool,
    pub compatibility_matrix: BridgeSupportMatrix,
    pub allowed_process_commands: BTreeSet<String>,
    pub bridge_circuit_breaker: ConnectorCircuitBreakerPolicy,
    pub wasm_allowed_path_prefixes: Vec<PathBuf>,
    pub wasm_max_component_bytes: Option<usize>,
    pub wasm_fuel_limit: Option<u64>,
    pub wasm_require_hash_pin: bool,
    pub wasm_required_sha256_by_plugin: BTreeMap<String, String>,
    pub enforce_execution_success: bool,
}

pub(crate) const PLUGIN_ACTIVATION_RUNTIME_CONTRACT_METADATA_KEY: &str =
    "plugin_activation_contract_json";
pub(crate) const PLUGIN_ACTIVATION_RUNTIME_CONTRACT_CHECKSUM_METADATA_KEY: &str =
    "plugin_activation_contract_checksum";
pub(crate) const PLUGIN_RUNTIME_HEALTH_METADATA_KEY: &str = "plugin_runtime_health_json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginActivationRuntimeContract {
    pub plugin_id: String,
    pub source_path: String,
    pub source_kind: PluginSourceKind,
    pub dialect: PluginContractDialect,
    pub dialect_version: Option<String>,
    pub compatibility_mode: PluginCompatibilityMode,
    pub compatibility_shim: Option<PluginCompatibilityShim>,
    pub bridge_kind: PluginBridgeKind,
    pub adapter_family: String,
    pub entrypoint_hint: String,
    pub source_language: String,
    pub compatibility: Option<PluginCompatibility>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginActivationAttestationResult {
    pub attested: bool,
    pub verified: bool,
    pub integrity: String,
    #[serde(default)]
    pub checksum: Option<String>,
    #[serde(default)]
    pub computed_checksum: Option<String>,
    #[serde(default)]
    pub issue: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginRuntimeHealthResult {
    pub status: String,
    pub circuit_enabled: bool,
    pub circuit_phase: String,
    pub consecutive_failures: usize,
    pub half_open_remaining_calls: usize,
    pub half_open_successes: usize,
    #[serde(default)]
    pub last_failure_reason: Option<String>,
    #[serde(default)]
    pub issue: Option<String>,
}

fn invalid_plugin_runtime_health_result(reason: String) -> PluginRuntimeHealthResult {
    PluginRuntimeHealthResult {
        status: "unknown".to_owned(),
        circuit_enabled: false,
        circuit_phase: "unknown".to_owned(),
        consecutive_failures: 0,
        half_open_remaining_calls: 0,
        half_open_successes: 0,
        last_failure_reason: None,
        issue: Some(reason),
    }
}

fn plugin_runtime_health_status(
    circuit_enabled: bool,
    circuit_phase: &str,
    consecutive_failures: usize,
) -> String {
    if !circuit_enabled || circuit_phase == "disabled" {
        return "disabled".to_owned();
    }

    if circuit_phase == "open" {
        return "quarantined".to_owned();
    }

    if circuit_phase == "half_open" {
        return "degraded".to_owned();
    }

    if circuit_phase == "closed" && consecutive_failures > 0 {
        return "degraded".to_owned();
    }

    if circuit_phase == "closed" {
        return "healthy".to_owned();
    }

    "unknown".to_owned()
}

fn build_plugin_runtime_health_result(
    policy: &ConnectorCircuitBreakerPolicy,
    circuit_phase: String,
    consecutive_failures: usize,
    half_open_remaining_calls: usize,
    half_open_successes: usize,
    last_failure_reason: Option<String>,
) -> PluginRuntimeHealthResult {
    let circuit_enabled = policy.enabled;
    let status = plugin_runtime_health_status(
        circuit_enabled,
        circuit_phase.as_str(),
        consecutive_failures,
    );

    PluginRuntimeHealthResult {
        status,
        circuit_enabled,
        circuit_phase,
        consecutive_failures,
        half_open_remaining_calls,
        half_open_successes,
        last_failure_reason,
        issue: None,
    }
}

fn encode_plugin_runtime_health_result(
    health: &PluginRuntimeHealthResult,
) -> Result<String, String> {
    serde_json::to_string(health)
        .map_err(|error| format!("serialize plugin runtime health failed: {error}"))
}

pub(crate) fn activation_runtime_contract_checksum_hex(bytes: &[u8]) -> String {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

pub(crate) fn plugin_activation_runtime_contract_value(
    contract: &PluginActivationRuntimeContract,
) -> Value {
    let mut value = Map::new();
    let plugin_id = contract.plugin_id.clone();
    let source_path = contract.source_path.clone();
    let source_kind = contract.source_kind.as_str().to_owned();
    let dialect = contract.dialect.as_str().to_owned();
    let compatibility_mode = contract.compatibility_mode.as_str().to_owned();
    let bridge_kind = contract.bridge_kind.as_str().to_owned();
    let adapter_family = contract.adapter_family.clone();
    let entrypoint_hint = contract.entrypoint_hint.clone();
    let source_language = contract.source_language.clone();

    value.insert("plugin_id".to_owned(), Value::String(plugin_id));
    value.insert("source_path".to_owned(), Value::String(source_path));
    value.insert("source_kind".to_owned(), Value::String(source_kind));
    value.insert("dialect".to_owned(), Value::String(dialect));
    value.insert(
        "compatibility_mode".to_owned(),
        Value::String(compatibility_mode),
    );
    value.insert("bridge_kind".to_owned(), Value::String(bridge_kind));
    value.insert("adapter_family".to_owned(), Value::String(adapter_family));
    value.insert("entrypoint_hint".to_owned(), Value::String(entrypoint_hint));
    value.insert("source_language".to_owned(), Value::String(source_language));

    if let Some(dialect_version) = &contract.dialect_version {
        let dialect_version = dialect_version.clone();

        value.insert("dialect_version".to_owned(), Value::String(dialect_version));
    }
    if let Some(compatibility_shim) = &contract.compatibility_shim {
        let mut compatibility_shim_value = Map::new();
        let shim_id = compatibility_shim.shim_id.clone();
        let family = compatibility_shim.family.clone();

        compatibility_shim_value.insert("shim_id".to_owned(), Value::String(shim_id));
        compatibility_shim_value.insert("family".to_owned(), Value::String(family));
        value.insert(
            "compatibility_shim".to_owned(),
            Value::Object(compatibility_shim_value),
        );
    }
    if let Some(compatibility) = &contract.compatibility {
        let mut compatibility_value = Map::new();

        if let Some(host_api) = &compatibility.host_api {
            let host_api = host_api.clone();

            compatibility_value.insert("host_api".to_owned(), Value::String(host_api));
        }
        if let Some(host_version_req) = &compatibility.host_version_req {
            let host_version_req = host_version_req.clone();

            compatibility_value.insert(
                "host_version_req".to_owned(),
                Value::String(host_version_req),
            );
        }

        value.insert(
            "compatibility".to_owned(),
            Value::Object(compatibility_value),
        );
    }

    Value::Object(value)
}

pub(crate) fn plugin_activation_runtime_contract_json(
    contract: &PluginActivationRuntimeContract,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&plugin_activation_runtime_contract_value(contract))
}

fn parse_plugin_activation_runtime_source_kind(raw: &str) -> Option<PluginSourceKind> {
    match raw {
        "package_manifest" => Some(PluginSourceKind::PackageManifest),
        "embedded_source" => Some(PluginSourceKind::EmbeddedSource),
        _ => None,
    }
}

pub(crate) fn parse_plugin_activation_runtime_dialect(raw: &str) -> Option<PluginContractDialect> {
    match raw {
        "loongclaw_package_manifest" | "loong_claw_package_manifest" => {
            Some(PluginContractDialect::LoongClawPackageManifest)
        }
        "loongclaw_embedded_source" | "loong_claw_embedded_source" => {
            Some(PluginContractDialect::LoongClawEmbeddedSource)
        }
        "openclaw_modern_manifest" | "open_claw_modern_manifest" => {
            Some(PluginContractDialect::OpenClawModernManifest)
        }
        "openclaw_legacy_package" | "open_claw_legacy_package" => {
            Some(PluginContractDialect::OpenClawLegacyPackage)
        }
        _ => None,
    }
}

pub(crate) fn parse_plugin_activation_runtime_mode(raw: &str) -> Option<PluginCompatibilityMode> {
    match raw {
        "native" => Some(PluginCompatibilityMode::Native),
        "openclaw_modern" | "open_claw_modern" => Some(PluginCompatibilityMode::OpenClawModern),
        "openclaw_legacy" | "open_claw_legacy" => Some(PluginCompatibilityMode::OpenClawLegacy),
        _ => None,
    }
}

pub(crate) fn parse_plugin_activation_runtime_bridge_kind(raw: &str) -> Option<PluginBridgeKind> {
    match raw {
        "http_json" => Some(PluginBridgeKind::HttpJson),
        "process_stdio" => Some(PluginBridgeKind::ProcessStdio),
        "native_ffi" => Some(PluginBridgeKind::NativeFfi),
        "wasm_component" => Some(PluginBridgeKind::WasmComponent),
        "mcp_server" => Some(PluginBridgeKind::McpServer),
        "acp_bridge" => Some(PluginBridgeKind::AcpBridge),
        "acp_runtime" => Some(PluginBridgeKind::AcpRuntime),
        "unknown" => Some(PluginBridgeKind::Unknown),
        _ => None,
    }
}

fn optional_contract_string_field(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<String>, String> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!(
            "plugin activation contract field `{key}` must be a string"
        )),
    }
}

fn required_contract_string_field(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String, String> {
    optional_contract_string_field(object, key)?
        .ok_or_else(|| format!("plugin activation contract field `{key}` is required"))
}

pub(crate) fn parse_plugin_activation_runtime_contract(
    raw: &str,
) -> Result<PluginActivationRuntimeContract, String> {
    let value: Value = serde_json::from_str(raw).map_err(|error| {
        format!("plugin activation contract payload must be valid JSON: {error}")
    })?;
    let object = value
        .as_object()
        .ok_or_else(|| "plugin activation contract payload must be a JSON object".to_owned())?;

    let compatibility_shim = match object.get("compatibility_shim") {
        None | Some(Value::Null) => None,
        Some(Value::Object(shim)) => Some(PluginCompatibilityShim {
            shim_id: required_contract_string_field(shim, "shim_id")?,
            family: required_contract_string_field(shim, "family")?,
        }),
        Some(_) => {
            return Err(
                "plugin activation contract field `compatibility_shim` must be an object"
                    .to_owned(),
            );
        }
    };

    let compatibility = match object.get("compatibility") {
        None | Some(Value::Null) => None,
        Some(Value::Object(compatibility)) => Some(PluginCompatibility {
            host_api: optional_contract_string_field(compatibility, "host_api")?,
            host_version_req: optional_contract_string_field(compatibility, "host_version_req")?,
        }),
        Some(_) => {
            return Err(
                "plugin activation contract field `compatibility` must be an object".to_owned(),
            );
        }
    };

    let source_kind_raw = required_contract_string_field(object, "source_kind")?;
    let dialect_raw = required_contract_string_field(object, "dialect")?;
    let compatibility_mode_raw = required_contract_string_field(object, "compatibility_mode")?;
    let bridge_kind_raw = required_contract_string_field(object, "bridge_kind")?;

    Ok(PluginActivationRuntimeContract {
        plugin_id: required_contract_string_field(object, "plugin_id")?,
        source_path: required_contract_string_field(object, "source_path")?,
        source_kind: parse_plugin_activation_runtime_source_kind(&source_kind_raw).ok_or_else(
            || format!("plugin activation contract field `source_kind` has unsupported value `{source_kind_raw}`"),
        )?,
        dialect: parse_plugin_activation_runtime_dialect(&dialect_raw).ok_or_else(|| {
            format!("plugin activation contract field `dialect` has unsupported value `{dialect_raw}`")
        })?,
        dialect_version: optional_contract_string_field(object, "dialect_version")?,
        compatibility_mode: parse_plugin_activation_runtime_mode(&compatibility_mode_raw)
            .ok_or_else(|| {
                format!(
                    "plugin activation contract field `compatibility_mode` has unsupported value `{compatibility_mode_raw}`"
                )
            })?,
        compatibility_shim,
        bridge_kind: parse_plugin_activation_runtime_bridge_kind(&bridge_kind_raw).ok_or_else(
            || format!("plugin activation contract field `bridge_kind` has unsupported value `{bridge_kind_raw}`"),
        )?,
        adapter_family: required_contract_string_field(object, "adapter_family")?,
        entrypoint_hint: required_contract_string_field(object, "entrypoint_hint")?,
        source_language: required_contract_string_field(object, "source_language")?,
        compatibility,
    })
}

#[derive(Debug, Default)]
struct ProviderActivationRuntimeContractState {
    metadata_present: bool,
    contract: Option<PluginActivationRuntimeContract>,
    checksum: Option<String>,
    computed_checksum: Option<String>,
    integrity_issue: Option<String>,
}

pub(crate) fn provider_plugin_activation_attestation_result(
    metadata: &BTreeMap<String, String>,
) -> Option<PluginActivationAttestationResult> {
    let is_plugin_backed = provider_is_plugin_backed(metadata);
    let state = provider_activation_runtime_contract_state(metadata);

    if !is_plugin_backed && !state.metadata_present {
        return None;
    }

    let issue = if !state.metadata_present {
        Some(
            "plugin-backed provider metadata is missing activation attestation metadata".to_owned(),
        )
    } else {
        state.integrity_issue.clone()
    };

    Some(PluginActivationAttestationResult {
        attested: state.metadata_present,
        verified: state.contract.is_some() && state.integrity_issue.is_none(),
        integrity: if !state.metadata_present {
            "missing".to_owned()
        } else if state.integrity_issue.is_some() {
            "invalid".to_owned()
        } else {
            "verified".to_owned()
        },
        checksum: state.checksum,
        computed_checksum: state.computed_checksum,
        issue,
    })
}

pub(crate) fn provider_plugin_runtime_health_result(
    metadata: &BTreeMap<String, String>,
) -> Option<PluginRuntimeHealthResult> {
    let raw = metadata.get(PLUGIN_RUNTIME_HEALTH_METADATA_KEY)?;
    let parsed = serde_json::from_str::<PluginRuntimeHealthResult>(raw);

    match parsed {
        Ok(health) => {
            let status = health.status.trim().to_owned();
            let circuit_phase = health.circuit_phase.trim().to_owned();
            if status.is_empty() {
                let reason =
                    "plugin runtime health metadata is invalid: `status` must not be empty";
                return Some(invalid_plugin_runtime_health_result(reason.to_owned()));
            }
            if circuit_phase.is_empty() {
                let reason =
                    "plugin runtime health metadata is invalid: `circuit_phase` must not be empty";
                return Some(invalid_plugin_runtime_health_result(reason.to_owned()));
            }

            Some(PluginRuntimeHealthResult {
                status,
                circuit_enabled: health.circuit_enabled,
                circuit_phase,
                consecutive_failures: health.consecutive_failures,
                half_open_remaining_calls: health.half_open_remaining_calls,
                half_open_successes: health.half_open_successes,
                last_failure_reason: health.last_failure_reason,
                issue: None,
            })
        }
        Err(error) => {
            let reason = format!("plugin runtime health metadata is invalid: {error}");
            Some(invalid_plugin_runtime_health_result(reason))
        }
    }
}

impl BridgeRuntimePolicy {
    #[must_use]
    pub fn process_stdio_execution_security_tier(&self) -> ExecutionSecurityTier {
        if self.execute_process_stdio && !self.allowed_process_commands.is_empty() {
            ExecutionSecurityTier::Balanced
        } else {
            ExecutionSecurityTier::Restricted
        }
    }

    #[must_use]
    pub const fn wasm_execution_security_tier(&self) -> ExecutionSecurityTier {
        let _ = self;
        ExecutionSecurityTier::Restricted
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityScanSpec {
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub block_on_high: bool,
    #[serde(default)]
    pub profile_path: Option<String>,
    #[serde(default)]
    pub profile_sha256: Option<String>,
    #[serde(default)]
    pub profile_signature: Option<SecurityProfileSignatureSpec>,
    #[serde(default)]
    pub siem_export: Option<SecuritySiemExportSpec>,
    #[serde(default)]
    pub runtime: SecurityRuntimeExecutionSpec,
    #[serde(default)]
    pub high_risk_metadata_keywords: Vec<String>,
    #[serde(default)]
    pub wasm: WasmSecurityScanSpec,
}

impl Default for SecurityScanSpec {
    fn default() -> Self {
        Self {
            enabled: false,
            block_on_high: true,
            profile_path: None,
            profile_sha256: None,
            profile_signature: None,
            siem_export: None,
            runtime: SecurityRuntimeExecutionSpec::default(),
            high_risk_metadata_keywords: Vec::new(),
            wasm: WasmSecurityScanSpec::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityProfileSignatureSpec {
    #[serde(default = "default_security_profile_signature_algorithm")]
    pub algorithm: String,
    pub public_key_base64: String,
    pub signature_base64: String,
}

pub fn default_security_profile_signature_algorithm() -> String {
    "ed25519".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecuritySiemExportSpec {
    pub enabled: bool,
    pub path: String,
    #[serde(default = "default_true")]
    pub include_findings: bool,
    #[serde(default)]
    pub max_findings_per_record: Option<usize>,
    #[serde(default)]
    pub fail_on_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecuritySiemExportReport {
    pub enabled: bool,
    pub path: String,
    pub success: bool,
    pub exported_records: usize,
    pub exported_findings: usize,
    pub truncated_findings: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecurityRuntimeExecutionSpec {
    #[serde(default)]
    pub execute_wasm_component: bool,
    #[serde(default)]
    pub allowed_path_prefixes: Vec<String>,
    #[serde(default)]
    pub max_component_bytes: Option<usize>,
    #[serde(default)]
    pub fuel_limit: Option<u64>,
    #[serde(default)]
    pub bridge_circuit_breaker: ConnectorCircuitBreakerPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmSecurityScanSpec {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub max_module_bytes: usize,
    #[serde(default)]
    pub allow_wasi: bool,
    #[serde(default)]
    pub blocked_import_prefixes: Vec<String>,
    #[serde(default)]
    pub allowed_path_prefixes: Vec<String>,
    #[serde(default)]
    pub require_hash_pin: bool,
    #[serde(default)]
    pub required_sha256_by_plugin: BTreeMap<String, String>,
}

impl Default for WasmSecurityScanSpec {
    fn default() -> Self {
        Self {
            enabled: true,
            max_module_bytes: 0,
            allow_wasi: false,
            blocked_import_prefixes: Vec::new(),
            allowed_path_prefixes: Vec::new(),
            require_hash_pin: false,
            required_sha256_by_plugin: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityScanProfile {
    #[serde(default)]
    pub high_risk_metadata_keywords: Vec<String>,
    #[serde(default)]
    pub wasm: WasmSecurityScanSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityFindingSeverity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityFinding {
    pub correlation_id: String,
    pub severity: SecurityFindingSeverity,
    pub category: String,
    pub plugin_id: String,
    pub source_path: String,
    pub message: String,
    pub evidence: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityScanReport {
    pub enabled: bool,
    pub scanned_plugins: usize,
    pub total_findings: usize,
    pub high_findings: usize,
    pub medium_findings: usize,
    pub low_findings: usize,
    pub blocked: bool,
    pub block_reason: Option<String>,
    pub siem_export: Option<SecuritySiemExportReport>,
    pub findings: Vec<SecurityFinding>,
}

impl Default for SecurityScanReport {
    fn default() -> Self {
        Self {
            enabled: true,
            scanned_plugins: 0,
            total_findings: 0,
            high_findings: 0,
            medium_findings: 0,
            low_findings: 0,
            blocked: false,
            block_reason: None,
            siem_export: None,
            findings: Vec::new(),
        }
    }
}

pub fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PluginPreflightProfile {
    #[default]
    RuntimeActivation,
    SdkRelease,
    MarketplaceSubmission,
}

impl PluginPreflightProfile {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeActivation => "runtime_activation",
            Self::SdkRelease => "sdk_release",
            Self::MarketplaceSubmission => "marketplace_submission",
        }
    }
}

pub fn default_plugin_preflight_profile() -> PluginPreflightProfile {
    PluginPreflightProfile::RuntimeActivation
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginPreflightVerdict {
    Pass,
    Warn,
    Block,
}

impl PluginPreflightVerdict {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Block => "block",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginPreflightRemediationClass {
    MigrateToPackageManifest,
    MigrateForeignDialect,
    ModernizeLegacyOpenClawContract,
    EnableCompatibilityShim,
    AlignCompatibilityShimProfile,
    QuarantineLoadedProvider,
    RepairRuntimeAttestation,
    RemoveLegacyMetadataVersion,
    RemoveShadowedEmbeddedSource,
    ResolveHostCompatibility,
    SwitchSupportedBridge,
    SwitchSupportedAdapterFamily,
    ResolveSlotOwnershipConflict,
    ResolveActivationBlockers,
    ReviewAdvisoryDiagnostics,
}

impl PluginPreflightRemediationClass {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MigrateToPackageManifest => "migrate_to_package_manifest",
            Self::MigrateForeignDialect => "migrate_foreign_dialect",
            Self::ModernizeLegacyOpenClawContract => "modernize_legacy_openclaw_contract",
            Self::EnableCompatibilityShim => "enable_compatibility_shim",
            Self::AlignCompatibilityShimProfile => "align_compatibility_shim_profile",
            Self::QuarantineLoadedProvider => "quarantine_loaded_provider",
            Self::RepairRuntimeAttestation => "repair_runtime_attestation",
            Self::RemoveLegacyMetadataVersion => "remove_legacy_metadata_version",
            Self::RemoveShadowedEmbeddedSource => "remove_shadowed_embedded_source",
            Self::ResolveHostCompatibility => "resolve_host_compatibility",
            Self::SwitchSupportedBridge => "switch_supported_bridge",
            Self::SwitchSupportedAdapterFamily => "switch_supported_adapter_family",
            Self::ResolveSlotOwnershipConflict => "resolve_slot_ownership_conflict",
            Self::ResolveActivationBlockers => "resolve_activation_blockers",
            Self::ReviewAdvisoryDiagnostics => "review_advisory_diagnostics",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginPreflightOperatorActionSurface {
    HostRuntime,
    BridgePolicy,
    PluginPackage,
    OperatorReview,
}

impl PluginPreflightOperatorActionSurface {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HostRuntime => "host_runtime",
            Self::BridgePolicy => "bridge_policy",
            Self::PluginPackage => "plugin_package",
            Self::OperatorReview => "operator_review",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginPreflightOperatorActionKind {
    QuarantineLoadedProvider,
    ReabsorbPlugin,
    UpdateBridgeSupportPolicy,
    UpdatePluginPackage,
    ResolveSlotOwnership,
    ReviewDiagnostics,
}

impl PluginPreflightOperatorActionKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::QuarantineLoadedProvider => "quarantine_loaded_provider",
            Self::ReabsorbPlugin => "reabsorb_plugin",
            Self::UpdateBridgeSupportPolicy => "update_bridge_support_policy",
            Self::UpdatePluginPackage => "update_plugin_package",
            Self::ResolveSlotOwnership => "resolve_slot_ownership",
            Self::ReviewDiagnostics => "review_diagnostics",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPreflightOperatorAction {
    #[serde(default)]
    pub action_id: String,
    pub surface: PluginPreflightOperatorActionSurface,
    pub kind: PluginPreflightOperatorActionKind,
    pub target_plugin_id: String,
    #[serde(default)]
    pub target_provider_id: Option<String>,
    pub target_source_path: String,
    #[serde(default)]
    pub target_manifest_path: Option<String>,
    #[serde(default)]
    pub follow_up_profile: Option<PluginPreflightProfile>,
    #[serde(default)]
    pub requires_reload: bool,
}

fn plugin_preflight_operator_action_value(action: &PluginPreflightOperatorAction) -> Value {
    let mut value = Map::new();
    let surface = action.surface.as_str().to_owned();
    let kind = action.kind.as_str().to_owned();
    let target_plugin_id = action.target_plugin_id.clone();
    let target_source_path = action.target_source_path.clone();
    let target_provider_id = match &action.target_provider_id {
        Some(target_provider_id) => Value::String(target_provider_id.clone()),
        None => Value::Null,
    };
    let target_manifest_path = match &action.target_manifest_path {
        Some(target_manifest_path) => Value::String(target_manifest_path.clone()),
        None => Value::Null,
    };
    let follow_up_profile = match action.follow_up_profile {
        Some(follow_up_profile) => Value::String(follow_up_profile.as_str().to_owned()),
        None => Value::Null,
    };
    let requires_reload = action.requires_reload;

    value.insert("surface".to_owned(), Value::String(surface));
    value.insert("kind".to_owned(), Value::String(kind));
    value.insert(
        "target_plugin_id".to_owned(),
        Value::String(target_plugin_id),
    );
    value.insert("target_provider_id".to_owned(), target_provider_id);
    value.insert(
        "target_source_path".to_owned(),
        Value::String(target_source_path),
    );
    value.insert("target_manifest_path".to_owned(), target_manifest_path);
    value.insert("follow_up_profile".to_owned(), follow_up_profile);
    value.insert("requires_reload".to_owned(), Value::Bool(requires_reload));

    Value::Object(value)
}

fn plugin_preflight_operator_action_message(action: &PluginPreflightOperatorAction) -> Vec<u8> {
    serde_json::to_vec(&plugin_preflight_operator_action_value(action)).unwrap_or_default()
}

#[must_use]
pub fn plugin_preflight_operator_action_sha256(action: &PluginPreflightOperatorAction) -> String {
    let digest = Sha256::digest(plugin_preflight_operator_action_message(action));
    let mut encoded = String::with_capacity(digest.len().saturating_mul(2));
    for byte in digest {
        encoded.push_str(&format!("{byte:02x}"));
    }
    encoded
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginPreflightOperatorActionSupport {
    pub remediation_class: PluginPreflightRemediationClass,
    #[serde(default)]
    pub diagnostic_code: Option<String>,
    #[serde(default)]
    pub field_path: Option<String>,
    pub blocking: bool,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginPreflightOperatorActionPlanItem {
    pub action: PluginPreflightOperatorAction,
    pub supporting_results: usize,
    pub blocked_results: usize,
    pub warned_results: usize,
    pub passed_results: usize,
    pub supporting_remediations: Vec<PluginPreflightOperatorActionSupport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPreflightRecommendedAction {
    pub remediation_class: PluginPreflightRemediationClass,
    #[serde(default)]
    pub diagnostic_code: Option<String>,
    #[serde(default)]
    pub field_path: Option<String>,
    pub blocking: bool,
    pub summary: String,
    #[serde(default)]
    pub operator_action: Option<PluginPreflightOperatorAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPreflightPolicyException {
    pub exception_id: String,
    pub plugin_id: String,
    #[serde(default)]
    pub plugin_version_req: Option<String>,
    #[serde(default)]
    pub profiles: Vec<PluginPreflightProfile>,
    #[serde(default)]
    pub waive_policy_flags: Vec<String>,
    #[serde(default)]
    pub waive_diagnostic_codes: Vec<String>,
    pub reason: String,
    pub ticket_ref: String,
    pub approved_by: String,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPreflightAppliedException {
    pub exception_id: String,
    #[serde(default)]
    pub plugin_version_req: Option<String>,
    pub reason: String,
    pub ticket_ref: String,
    pub approved_by: String,
    #[serde(default)]
    pub expires_at: Option<String>,
    pub waived_policy_flags: Vec<String>,
    pub waived_diagnostic_codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPreflightRuleProfile {
    #[serde(default = "default_true")]
    pub block_on_activation_blocked: bool,
    #[serde(default = "default_true")]
    pub block_on_blocking_diagnostics: bool,
    #[serde(default = "default_true")]
    pub warn_on_advisory_diagnostics: bool,
    #[serde(default)]
    pub block_on_invalid_runtime_attestation: bool,
    #[serde(default)]
    pub block_on_foreign_dialect_contract: bool,
    #[serde(default)]
    pub block_on_legacy_openclaw_contract: bool,
    #[serde(default)]
    pub block_on_compatibility_shim_required: bool,
    #[serde(default)]
    pub block_on_compatibility_shim_profile_mismatch: bool,
    #[serde(default)]
    pub block_on_embedded_source_contract: bool,
    #[serde(default)]
    pub block_on_legacy_metadata_version: bool,
    #[serde(default)]
    pub block_on_shadowed_embedded_source: bool,
}

impl Default for PluginPreflightRuleProfile {
    fn default() -> Self {
        Self {
            block_on_activation_blocked: true,
            block_on_blocking_diagnostics: true,
            warn_on_advisory_diagnostics: true,
            block_on_invalid_runtime_attestation: false,
            block_on_foreign_dialect_contract: false,
            block_on_legacy_openclaw_contract: false,
            block_on_compatibility_shim_required: false,
            block_on_compatibility_shim_profile_mismatch: false,
            block_on_embedded_source_contract: false,
            block_on_legacy_metadata_version: false,
            block_on_shadowed_embedded_source: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPreflightPolicyProfile {
    #[serde(default)]
    pub policy_version: Option<String>,
    #[serde(default)]
    pub runtime_activation: PluginPreflightRuleProfile,
    #[serde(default = "default_sdk_release_preflight_rule_profile")]
    pub sdk_release: PluginPreflightRuleProfile,
    #[serde(default = "default_marketplace_submission_preflight_rule_profile")]
    pub marketplace_submission: PluginPreflightRuleProfile,
    #[serde(default)]
    pub exceptions: Vec<PluginPreflightPolicyException>,
}

impl Default for PluginPreflightPolicyProfile {
    fn default() -> Self {
        Self {
            policy_version: Some("medium-balanced".to_owned()),
            runtime_activation: default_runtime_activation_preflight_rule_profile(),
            sdk_release: default_sdk_release_preflight_rule_profile(),
            marketplace_submission: default_marketplace_submission_preflight_rule_profile(),
            exceptions: Vec::new(),
        }
    }
}

impl PluginPreflightPolicyProfile {
    #[must_use]
    pub fn rules_for(&self, profile: PluginPreflightProfile) -> &PluginPreflightRuleProfile {
        match profile {
            PluginPreflightProfile::RuntimeActivation => &self.runtime_activation,
            PluginPreflightProfile::SdkRelease => &self.sdk_release,
            PluginPreflightProfile::MarketplaceSubmission => &self.marketplace_submission,
        }
    }
}

pub fn default_runtime_activation_preflight_rule_profile() -> PluginPreflightRuleProfile {
    PluginPreflightRuleProfile {
        block_on_invalid_runtime_attestation: true,
        ..PluginPreflightRuleProfile::default()
    }
}

pub fn default_sdk_release_preflight_rule_profile() -> PluginPreflightRuleProfile {
    PluginPreflightRuleProfile {
        block_on_legacy_openclaw_contract: true,
        block_on_compatibility_shim_required: true,
        block_on_compatibility_shim_profile_mismatch: true,
        block_on_embedded_source_contract: true,
        ..PluginPreflightRuleProfile::default()
    }
}

pub fn default_marketplace_submission_preflight_rule_profile() -> PluginPreflightRuleProfile {
    PluginPreflightRuleProfile {
        block_on_legacy_openclaw_contract: true,
        block_on_compatibility_shim_required: true,
        block_on_compatibility_shim_profile_mismatch: true,
        block_on_embedded_source_contract: true,
        block_on_legacy_metadata_version: true,
        block_on_shadowed_embedded_source: true,
        ..PluginPreflightRuleProfile::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapSpec {
    pub enabled: bool,
    #[serde(default)]
    pub allow_http_json_auto_apply: Option<bool>,
    #[serde(default)]
    pub allow_process_stdio_auto_apply: Option<bool>,
    #[serde(default)]
    pub allow_native_ffi_auto_apply: Option<bool>,
    #[serde(default)]
    pub allow_wasm_component_auto_apply: Option<bool>,
    #[serde(default)]
    pub allow_mcp_server_auto_apply: Option<bool>,
    #[serde(default)]
    pub allow_acp_bridge_auto_apply: Option<bool>,
    #[serde(default)]
    pub allow_acp_runtime_auto_apply: Option<bool>,
    #[serde(default)]
    pub enforce_ready_execution: Option<bool>,
    #[serde(default)]
    pub max_tasks: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoProvisionSpec {
    pub enabled: bool,
    pub provider_id: String,
    pub channel_id: String,
    pub connector_name: Option<String>,
    pub endpoint: Option<String>,
    pub required_capabilities: BTreeSet<Capability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HotfixSpec {
    ProviderVersion {
        provider_id: String,
        new_version: String,
    },
    ProviderConnector {
        provider_id: String,
        new_connector_name: String,
    },
    ChannelEndpoint {
        channel_id: String,
        new_endpoint: String,
    },
    ChannelEnabled {
        channel_id: String,
        enabled: bool,
    },
}

impl HotfixSpec {
    pub fn to_kernel_hotfix(&self) -> IntegrationHotfix {
        match self {
            Self::ProviderVersion {
                provider_id,
                new_version,
            } => IntegrationHotfix::ProviderVersion {
                provider_id: provider_id.clone(),
                new_version: new_version.clone(),
            },
            Self::ProviderConnector {
                provider_id,
                new_connector_name,
            } => IntegrationHotfix::ProviderConnector {
                provider_id: provider_id.clone(),
                new_connector_name: new_connector_name.clone(),
            },
            Self::ChannelEndpoint {
                channel_id,
                new_endpoint,
            } => IntegrationHotfix::ChannelEndpoint {
                channel_id: channel_id.clone(),
                new_endpoint: new_endpoint.clone(),
            },
            Self::ChannelEnabled {
                channel_id,
                enabled,
            } => IntegrationHotfix::ChannelEnabled {
                channel_id: channel_id.clone(),
                enabled: *enabled,
            },
        }
    }
}

impl RunnerSpec {
    pub fn template() -> Self {
        Self {
            pack: VerticalPackManifest {
                pack_id: "sales-intel-local".to_owned(),
                domain: "sales".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::from(["webhook".to_owned(), "crm".to_owned()]),
                granted_capabilities: BTreeSet::from([
                    Capability::InvokeTool,
                    Capability::InvokeConnector,
                    Capability::MemoryRead,
                    Capability::MemoryWrite,
                    Capability::ObserveTelemetry,
                ]),
                metadata: BTreeMap::from([
                    ("owner".to_owned(), "platform-team".to_owned()),
                    ("stage".to_owned(), "prototype".to_owned()),
                ]),
            },
            agent_id: "agent-spec-01".to_owned(),
            ttl_s: 300,
            approval: Some(HumanApprovalSpec::default()),
            defaults: Some(DefaultCoreSelection {
                connector: Some("http-core".to_owned()),
                runtime: Some("native-core".to_owned()),
                tool: Some("core-tools".to_owned()),
                memory: Some("kv-core".to_owned()),
            }),
            self_awareness: None,
            plugin_scan: None,
            plugin_setup_readiness: Some(PluginSetupReadinessSpec {
                inherit_process_env: true,
                verified_env_vars: Vec::new(),
                verified_config_keys: Vec::new(),
            }),
            bridge_support: None,
            bootstrap: None,
            auto_provision: Some(AutoProvisionSpec {
                enabled: true,
                provider_id: "openrouter".to_owned(),
                channel_id: "primary".to_owned(),
                connector_name: Some("openrouter".to_owned()),
                endpoint: Some("https://openrouter.ai/api/v1/chat/completions".to_owned()),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            }),
            hotfixes: Vec::new(),
            operation: OperationSpec::RuntimeExtension {
                action: "start-session".to_owned(),
                required_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
                payload: json!({"session_id": "s-42"}),
                extension: "acp-bridge".to_owned(),
                core: None,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SpecRunReport {
    pub schema_version: u32,
    pub schema: JsonSchemaDescriptor,
    pub pack_id: String,
    pub agent_id: String,
    pub operation_kind: &'static str,
    pub blocked_reason: Option<String>,
    pub approval_guard: ApprovalDecisionReport,
    pub bridge_support_source: Option<String>,
    pub bridge_support_checksum: Option<String>,
    pub bridge_support_sha256: Option<String>,
    pub bridge_support_delta_source: Option<String>,
    pub bridge_support_delta_sha256: Option<String>,
    pub self_awareness: Option<CodebaseAwarenessSnapshot>,
    pub architecture_guard: Option<ArchitectureGuardReport>,
    pub plugin_scan_reports: Vec<PluginScanReport>,
    pub plugin_translation_reports: Vec<PluginTranslationReport>,
    pub plugin_activation_plans: Vec<PluginActivationPlan>,
    pub plugin_bootstrap_reports: Vec<BootstrapReport>,
    pub plugin_bootstrap_queue: Vec<String>,
    pub plugin_absorb_reports: Vec<PluginAbsorbReport>,
    pub security_scan_report: Option<SecurityScanReport>,
    pub auto_provision_plan: Option<ProvisionPlan>,
    pub outcome: Value,
    pub integration_catalog: IntegrationCatalog,
    pub audit_events: Option<Vec<AuditEvent>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JsonSchemaDescriptor {
    pub version: u32,
    pub surface: String,
    pub purpose: String,
}

pub fn json_schema_descriptor(version: u32, surface: &str, purpose: &str) -> JsonSchemaDescriptor {
    let surface = surface.to_owned();
    let purpose = purpose.to_owned();

    JsonSchemaDescriptor {
        version,
        surface,
        purpose,
    }
}

pub const SPEC_RUN_REPORT_SCHEMA_VERSION: u32 = 1;
pub const SPEC_RUN_REPORT_SCHEMA_SURFACE: &str = "spec_run_report";
pub const SPEC_RUN_REPORT_SCHEMA_PURPOSE: &str = "runtime_execution";

#[derive(Debug, Clone)]
pub struct ToolSearchEntry {
    pub tool_id: String,
    pub plugin_id: Option<String>,
    pub manifest_api_version: Option<String>,
    pub plugin_version: Option<String>,
    pub dialect: Option<PluginContractDialect>,
    pub dialect_version: Option<String>,
    pub compatibility_mode: Option<PluginCompatibilityMode>,
    pub compatibility_shim: Option<PluginCompatibilityShim>,
    pub compatibility_shim_support: Option<PluginCompatibilityShimSupport>,
    pub compatibility_shim_support_mismatch_reasons: Vec<String>,
    pub connector_name: String,
    pub provider_id: String,
    pub source_path: Option<String>,
    pub source_kind: Option<String>,
    pub package_root: Option<String>,
    pub package_manifest_path: Option<String>,
    pub bridge_kind: PluginBridgeKind,
    pub adapter_family: Option<String>,
    pub entrypoint_hint: Option<String>,
    pub source_language: Option<String>,
    pub setup_mode: Option<String>,
    pub setup_surface: Option<String>,
    pub setup_required_env_vars: Vec<String>,
    pub setup_recommended_env_vars: Vec<String>,
    pub setup_required_config_keys: Vec<String>,
    pub setup_default_env_var: Option<String>,
    pub setup_docs_urls: Vec<String>,
    pub setup_remediation: Option<String>,
    pub setup_ready: bool,
    pub missing_required_env_vars: Vec<String>,
    pub missing_required_config_keys: Vec<String>,
    pub slot_claims: Vec<PluginSlotClaim>,
    pub diagnostic_findings: Vec<PluginDiagnosticFinding>,
    pub compatibility: Option<PluginCompatibility>,
    pub activation_status: Option<String>,
    pub activation_reason: Option<String>,
    pub activation_attestation: Option<PluginActivationAttestationResult>,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub input_examples: Vec<Value>,
    pub output_examples: Vec<Value>,
    pub deferred: bool,
    pub loaded: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolSearchResult {
    pub tool_id: String,
    pub plugin_id: Option<String>,
    pub manifest_api_version: Option<String>,
    pub plugin_version: Option<String>,
    pub dialect: Option<String>,
    pub dialect_version: Option<String>,
    pub compatibility_mode: Option<String>,
    pub compatibility_shim: Option<PluginCompatibilityShim>,
    pub compatibility_shim_support: Option<PluginCompatibilityShimSupport>,
    pub compatibility_shim_support_mismatch_reasons: Vec<String>,
    pub connector_name: String,
    pub provider_id: String,
    pub source_path: Option<String>,
    pub source_kind: Option<String>,
    pub package_root: Option<String>,
    pub package_manifest_path: Option<String>,
    pub bridge_kind: String,
    pub adapter_family: Option<String>,
    pub entrypoint_hint: Option<String>,
    pub source_language: Option<String>,
    pub setup_mode: Option<String>,
    pub setup_surface: Option<String>,
    pub setup_required_env_vars: Vec<String>,
    pub setup_recommended_env_vars: Vec<String>,
    pub setup_required_config_keys: Vec<String>,
    pub setup_default_env_var: Option<String>,
    pub setup_docs_urls: Vec<String>,
    pub setup_remediation: Option<String>,
    pub setup_ready: bool,
    pub missing_required_env_vars: Vec<String>,
    pub missing_required_config_keys: Vec<String>,
    pub slot_claims: Vec<PluginSlotClaim>,
    pub diagnostic_findings: Vec<PluginDiagnosticFinding>,
    pub compatibility: Option<PluginCompatibility>,
    pub activation_status: Option<String>,
    pub activation_reason: Option<String>,
    pub activation_attestation: Option<PluginActivationAttestationResult>,
    pub score: u32,
    pub deferred: bool,
    pub loaded: bool,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub input_examples: Vec<Value>,
    pub output_examples: Vec<Value>,
}

#[derive(Debug, Clone)]
pub struct PluginInventoryEntry {
    pub manifest_api_version: Option<String>,
    pub plugin_version: Option<String>,
    pub dialect: PluginContractDialect,
    pub dialect_version: Option<String>,
    pub compatibility_mode: PluginCompatibilityMode,
    pub compatibility_shim: Option<PluginCompatibilityShim>,
    pub compatibility_shim_support: Option<PluginCompatibilityShimSupport>,
    pub compatibility_shim_support_mismatch_reasons: Vec<String>,
    pub plugin_id: String,
    pub connector_name: String,
    pub provider_id: String,
    pub source_path: String,
    pub source_kind: String,
    pub package_root: String,
    pub package_manifest_path: Option<String>,
    pub bridge_kind: PluginBridgeKind,
    pub adapter_family: Option<String>,
    pub entrypoint_hint: Option<String>,
    pub source_language: Option<String>,
    pub setup_mode: Option<String>,
    pub setup_surface: Option<String>,
    pub setup_required_env_vars: Vec<String>,
    pub setup_recommended_env_vars: Vec<String>,
    pub setup_required_config_keys: Vec<String>,
    pub setup_default_env_var: Option<String>,
    pub setup_docs_urls: Vec<String>,
    pub setup_remediation: Option<String>,
    pub slot_claims: Vec<PluginSlotClaim>,
    pub diagnostic_findings: Vec<PluginDiagnosticFinding>,
    pub compatibility: Option<PluginCompatibility>,
    pub activation_status: Option<String>,
    pub activation_reason: Option<String>,
    pub activation_attestation: Option<PluginActivationAttestationResult>,
    pub runtime_health: Option<PluginRuntimeHealthResult>,
    pub bootstrap_hint: Option<String>,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub input_examples: Vec<Value>,
    pub output_examples: Vec<Value>,
    pub deferred: bool,
    pub loaded: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginInventoryResult {
    pub manifest_api_version: Option<String>,
    pub plugin_version: Option<String>,
    pub dialect: String,
    pub dialect_version: Option<String>,
    pub compatibility_mode: String,
    pub compatibility_shim: Option<PluginCompatibilityShim>,
    pub compatibility_shim_support: Option<PluginCompatibilityShimSupport>,
    pub compatibility_shim_support_mismatch_reasons: Vec<String>,
    pub plugin_id: String,
    pub connector_name: String,
    pub provider_id: String,
    pub source_path: String,
    pub source_kind: String,
    pub package_root: String,
    pub package_manifest_path: Option<String>,
    pub bridge_kind: String,
    pub adapter_family: Option<String>,
    pub entrypoint_hint: Option<String>,
    pub source_language: Option<String>,
    pub setup_mode: Option<String>,
    pub setup_surface: Option<String>,
    pub setup_required_env_vars: Vec<String>,
    pub setup_recommended_env_vars: Vec<String>,
    pub setup_required_config_keys: Vec<String>,
    pub setup_default_env_var: Option<String>,
    pub setup_docs_urls: Vec<String>,
    pub setup_remediation: Option<String>,
    pub slot_claims: Vec<PluginSlotClaim>,
    pub diagnostic_findings: Vec<PluginDiagnosticFinding>,
    pub compatibility: Option<PluginCompatibility>,
    pub activation_status: Option<String>,
    pub activation_reason: Option<String>,
    pub activation_attestation: Option<PluginActivationAttestationResult>,
    pub runtime_health: Option<PluginRuntimeHealthResult>,
    pub bootstrap_hint: Option<String>,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub input_examples: Vec<Value>,
    pub output_examples: Vec<Value>,
    pub deferred: bool,
    pub loaded: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginPreflightSummary {
    pub schema_version: u32,
    pub schema: JsonSchemaDescriptor,
    pub profile: String,
    pub policy_source: String,
    pub policy_version: Option<String>,
    pub policy_checksum: String,
    pub policy_sha256: String,
    pub matched_plugins: usize,
    pub returned_plugins: usize,
    pub truncated: bool,
    pub baseline_passed_plugins: usize,
    pub baseline_warned_plugins: usize,
    pub baseline_blocked_plugins: usize,
    pub clean_passed_plugins: usize,
    pub waived_passed_plugins: usize,
    pub passed_plugins: usize,
    pub warned_plugins: usize,
    pub blocked_plugins: usize,
    pub waived_plugins: usize,
    pub applied_exception_count: usize,
    pub ready_activation_plugins: usize,
    pub blocked_activation_plugins: usize,
    pub total_diagnostics: usize,
    pub blocking_diagnostics: usize,
    pub error_diagnostics: usize,
    pub warning_diagnostics: usize,
    pub info_diagnostics: usize,
    pub dialect_distribution: BTreeMap<String, usize>,
    pub compatibility_mode_distribution: BTreeMap<String, usize>,
    pub bridge_kind_distribution: BTreeMap<String, usize>,
    pub source_language_distribution: BTreeMap<String, usize>,
    pub findings_by_code: BTreeMap<String, usize>,
    pub findings_by_phase: BTreeMap<String, usize>,
    pub findings_by_severity: BTreeMap<String, usize>,
    pub remediation_counts: BTreeMap<String, usize>,
    pub operator_action_plan: Vec<PluginPreflightOperatorActionPlanItem>,
    pub operator_action_counts_by_surface: BTreeMap<String, usize>,
    pub operator_action_counts_by_kind: BTreeMap<String, usize>,
    pub operator_actions_requiring_reload: usize,
    pub operator_actions_without_reload: usize,
    pub waived_policy_flags: BTreeMap<String, usize>,
    pub waived_diagnostic_codes: BTreeMap<String, usize>,
    pub exception_counts_by_ticket: BTreeMap<String, usize>,
    pub exception_counts_by_approver: BTreeMap<String, usize>,
    pub source_kind_distribution: BTreeMap<String, usize>,
    pub active_bridge_profile: Option<String>,
    pub recommended_bridge_profile: Option<String>,
    pub recommended_bridge_profile_source: Option<String>,
    pub active_bridge_profile_matches_recommended: Option<bool>,
    pub active_bridge_support_fits_all_plugins: Option<bool>,
    pub bridge_profile_fits: Vec<PluginPreflightBridgeProfileFit>,
    pub bridge_profile_recommendation: Option<PluginPreflightBridgeProfileRecommendation>,
}

pub const PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_VERSION: u32 = 1;
pub const PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_SURFACE: &str = "plugin_preflight_summary";
pub const PLUGIN_PREFLIGHT_SUMMARY_SCHEMA_PURPOSE: &str = "plugin_governance_evaluation";

#[derive(Debug, Clone, Serialize)]
pub struct PluginPreflightBridgeProfileFit {
    pub profile_id: String,
    pub source: String,
    pub policy_version: Option<String>,
    pub checksum: String,
    pub sha256: String,
    pub fits_all_plugins: bool,
    pub supported_plugins: usize,
    pub blocked_plugins: usize,
    pub blocking_reasons: BTreeMap<String, usize>,
    pub sample_blocked_plugins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginPreflightBridgeProfileRecommendation {
    pub kind: PluginPreflightBridgeProfileRecommendationKind,
    pub target_profile_id: String,
    pub target_profile_source: String,
    pub target_policy_version: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub delta: Option<PluginPreflightBridgeProfileDelta>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginPreflightBridgeProfileRecommendationKind {
    AdoptBundledProfile,
    AuthorBridgeProfileDelta,
}

impl PluginPreflightBridgeProfileRecommendationKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AdoptBundledProfile => "adopt_bundled_profile",
            Self::AuthorBridgeProfileDelta => "author_bridge_profile_delta",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PluginPreflightBridgeProfileDelta {
    pub supported_bridges: Vec<String>,
    pub supported_adapter_families: Vec<String>,
    pub supported_compatibility_modes: Vec<String>,
    pub supported_compatibility_shims: Vec<String>,
    pub shim_profile_additions: Vec<PluginPreflightBridgeShimProfileDelta>,
    pub unresolved_blocking_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PluginPreflightBridgeShimProfileDelta {
    pub shim_id: String,
    pub shim_family: String,
    pub supported_dialects: Vec<String>,
    pub supported_bridges: Vec<String>,
    pub supported_adapter_families: Vec<String>,
    pub supported_source_languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginPreflightResult {
    pub profile: String,
    pub baseline_verdict: String,
    pub verdict: String,
    pub exception_applied: bool,
    pub activation_ready: bool,
    pub policy_flags: Vec<String>,
    pub effective_policy_flags: Vec<String>,
    pub waived_policy_flags: Vec<String>,
    pub policy_summary: String,
    pub blocking_diagnostic_codes: Vec<String>,
    pub advisory_diagnostic_codes: Vec<String>,
    pub effective_blocking_diagnostic_codes: Vec<String>,
    pub effective_advisory_diagnostic_codes: Vec<String>,
    pub waived_diagnostic_codes: Vec<String>,
    pub applied_exceptions: Vec<PluginPreflightAppliedException>,
    pub remediation_classes: Vec<PluginPreflightRemediationClass>,
    pub recommended_actions: Vec<PluginPreflightRecommendedAction>,
    pub plugin: PluginInventoryResult,
}

pub struct EmbeddedPiHarness {
    pub seen: Mutex<Vec<String>>,
}

#[async_trait]
impl HarnessAdapter for EmbeddedPiHarness {
    fn name(&self) -> &str {
        "pi-local"
    }

    fn kind(&self) -> HarnessKind {
        HarnessKind::EmbeddedPi
    }

    async fn execute(&self, request: HarnessRequest) -> Result<HarnessOutcome, HarnessError> {
        match self.seen.lock() {
            Ok(mut guard) => guard.push(request.task_id.clone()),
            Err(_) => {
                return Err(HarnessError::Execution(
                    "EmbeddedPiHarness mutex poisoned".to_owned(),
                ));
            }
        }

        Ok(HarnessOutcome {
            status: "ok".to_owned(),
            output: json!({
                "adapter": "pi-local",
                "task": request.task_id,
                "objective": request.objective,
            }),
        })
    }
}

pub struct WebhookConnector;

#[async_trait]
impl CoreConnectorAdapter for WebhookConnector {
    fn name(&self) -> &str {
        "webhook"
    }

    async fn invoke_core(
        &self,
        command: ConnectorCommand,
    ) -> Result<ConnectorOutcome, ConnectorError> {
        #[cfg(any(test, feature = "test-hooks"))]
        if let Some(test_config) = command
            .payload
            .as_object()
            .and_then(|payload| payload.get("_loongclaw_test"))
            .and_then(Value::as_object)
        {
            let delay_ms = test_config
                .get("delay_ms")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if delay_ms > 0 {
                sleep(Duration::from_millis(delay_ms)).await;
            }
            let request_id = test_config
                .get("request_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned();
            let failures_before_success = test_config
                .get("failures_before_success")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            if !request_id.is_empty() && failures_before_success > 0 {
                let attempts_map =
                    WEBHOOK_TEST_RETRY_STATE.get_or_init(|| Mutex::new(BTreeMap::new()));
                let current_attempt = {
                    let mut guard = attempts_map.lock().map_err(|_err| {
                        ConnectorError::Execution("retry test state mutex poisoned".to_owned())
                    })?;
                    let entry = guard.entry(request_id.clone()).or_insert(0);
                    *entry = entry.saturating_add(1);
                    *entry
                };
                if current_attempt <= failures_before_success {
                    return Err(ConnectorError::Execution(format!(
                        "simulated transient failure for request_id={request_id}, attempt={current_attempt}, threshold={failures_before_success}"
                    )));
                }
            }
        }

        Ok(ConnectorOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "connector": "webhook",
                "operation": command.operation,
                "payload": command.payload,
            }),
        })
    }
}

pub struct DynamicCatalogConnector {
    pub connector_name: String,
    pub provider_id: String,
    pub catalog: Arc<Mutex<IntegrationCatalog>>,
    pub bridge_runtime_policy: BridgeRuntimePolicy,
    pub bridge_circuit_state: Arc<TokioMutex<ConnectorCircuitRuntimeState>>,
}

#[derive(Debug, Clone)]
struct ProviderPluginCompatibilityContext {
    payload: Value,
    blocking_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct BridgeCircuitObservation {
    phase_before: String,
    phase_after: String,
    consecutive_failures: usize,
    half_open_remaining_calls: usize,
    half_open_successes: usize,
    remaining_cooldown_ms: Option<u64>,
}

impl BridgeCircuitObservation {
    fn disabled() -> Self {
        Self {
            phase_before: "disabled".to_owned(),
            phase_after: "disabled".to_owned(),
            consecutive_failures: 0,
            half_open_remaining_calls: 0,
            half_open_successes: 0,
            remaining_cooldown_ms: None,
        }
    }
}

impl DynamicCatalogConnector {
    pub fn new(
        connector_name: String,
        provider_id: String,
        catalog: Arc<Mutex<IntegrationCatalog>>,
        bridge_runtime_policy: BridgeRuntimePolicy,
    ) -> Self {
        let bridge_circuit_state =
            Arc::new(TokioMutex::new(ConnectorCircuitRuntimeState::default()));

        Self {
            connector_name,
            provider_id,
            catalog,
            bridge_runtime_policy,
            bridge_circuit_state,
        }
    }

    async fn acquire_bridge_circuit_phase(&self) -> Result<String, ConnectorError> {
        let policy = &self.bridge_runtime_policy.bridge_circuit_breaker;
        if !policy.enabled {
            return Ok("disabled".to_owned());
        }

        let mut state = self.bridge_circuit_state.lock().await;
        let now = TokioInstant::now();
        let acquire_result = acquire_connector_circuit_slot_for_state(policy, &mut state, now);

        match acquire_result {
            Ok(phase) => Ok(phase.to_owned()),
            Err(ConnectorCircuitAcquireError::Open {
                remaining_cooldown_ms,
            }) => {
                let reason = format!(
                    "plugin connector {} is circuit-open (remaining_cooldown_ms={remaining_cooldown_ms})",
                    self.connector_name
                );
                let circuit_phase = connector_circuit_phase_label(state.phase).to_owned();
                let consecutive_failures = state.consecutive_failures;
                let half_open_remaining_calls = state.half_open_remaining_calls;
                let half_open_successes = state.half_open_successes;
                let last_failure_reason = Some(reason.clone());
                drop(state);

                let persist_result = self
                    .persist_plugin_runtime_health(
                        policy,
                        circuit_phase,
                        consecutive_failures,
                        half_open_remaining_calls,
                        half_open_successes,
                        last_failure_reason,
                    )
                    .await;
                if let Err(error) = persist_result {
                    let combined_reason =
                        format!("{reason}; failed to persist plugin runtime health: {error}");
                    return Err(ConnectorError::Execution(combined_reason));
                }

                Err(ConnectorError::Execution(reason))
            }
            Err(ConnectorCircuitAcquireError::HalfOpenReopened) => {
                let reason = format!(
                    "plugin connector {} half-open window exhausted and re-opened",
                    self.connector_name
                );
                let circuit_phase = connector_circuit_phase_label(state.phase).to_owned();
                let consecutive_failures = state.consecutive_failures;
                let half_open_remaining_calls = state.half_open_remaining_calls;
                let half_open_successes = state.half_open_successes;
                let last_failure_reason = Some(reason.clone());
                drop(state);

                let persist_result = self
                    .persist_plugin_runtime_health(
                        policy,
                        circuit_phase,
                        consecutive_failures,
                        half_open_remaining_calls,
                        half_open_successes,
                        last_failure_reason,
                    )
                    .await;
                if let Err(error) = persist_result {
                    let combined_reason =
                        format!("{reason}; failed to persist plugin runtime health: {error}");
                    return Err(ConnectorError::Execution(combined_reason));
                }

                Err(ConnectorError::Execution(reason))
            }
        }
    }

    async fn record_bridge_circuit_outcome(
        &self,
        success: bool,
        phase_before: &str,
    ) -> BridgeCircuitObservation {
        let policy = &self.bridge_runtime_policy.bridge_circuit_breaker;
        if !policy.enabled {
            return BridgeCircuitObservation::disabled();
        }

        let mut state = self.bridge_circuit_state.lock().await;
        let now = TokioInstant::now();
        let phase_after =
            record_connector_circuit_outcome_for_state(policy, &mut state, success, now);
        let remaining_cooldown_ms = connector_circuit_remaining_cooldown_ms(&state, now);

        BridgeCircuitObservation {
            phase_before: phase_before.to_owned(),
            phase_after: phase_after.to_owned(),
            consecutive_failures: state.consecutive_failures,
            half_open_remaining_calls: state.half_open_remaining_calls,
            half_open_successes: state.half_open_successes,
            remaining_cooldown_ms,
        }
    }

    async fn persist_plugin_runtime_health(
        &self,
        policy: &ConnectorCircuitBreakerPolicy,
        circuit_phase: String,
        consecutive_failures: usize,
        half_open_remaining_calls: usize,
        half_open_successes: usize,
        last_failure_reason: Option<String>,
    ) -> Result<(), String> {
        let health = build_plugin_runtime_health_result(
            policy,
            circuit_phase,
            consecutive_failures,
            half_open_remaining_calls,
            half_open_successes,
            last_failure_reason,
        );
        let encoded = encode_plugin_runtime_health_result(&health)?;
        let metadata_key = PLUGIN_RUNTIME_HEALTH_METADATA_KEY.to_owned();
        let mut catalog = self
            .catalog
            .lock()
            .map_err(|_err| "integration catalog mutex poisoned".to_owned())?;
        let provider = catalog
            .provider(&self.provider_id)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "provider {} is not registered in integration catalog",
                    self.provider_id
                )
            })?;
        let is_plugin_backed = provider_is_plugin_backed(&provider.metadata);
        if !is_plugin_backed {
            return Ok(());
        }

        let mut updated_provider = provider;
        updated_provider.metadata.insert(metadata_key, encoded);
        catalog.upsert_provider(updated_provider);

        Ok(())
    }
}

fn bridge_execution_status_is_failure(bridge_execution: &Value) -> bool {
    bridge_execution
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| matches!(status, "blocked" | "failed"))
}

fn bridge_circuit_breaker_runtime_value(
    policy: &ConnectorCircuitBreakerPolicy,
    observation: &BridgeCircuitObservation,
) -> Value {
    let mut payload = Map::new();
    let enabled = policy.enabled;
    let phase_before = observation.phase_before.clone();
    let phase_after = observation.phase_after.clone();
    let failure_threshold = policy.failure_threshold as u64;
    let cooldown_ms = policy.cooldown_ms;
    let half_open_max_calls = policy.half_open_max_calls as u64;
    let success_threshold = policy.success_threshold as u64;
    let consecutive_failures = observation.consecutive_failures as u64;
    let half_open_remaining_calls = observation.half_open_remaining_calls as u64;
    let half_open_successes = observation.half_open_successes as u64;

    payload.insert("enabled".to_owned(), Value::Bool(enabled));
    payload.insert("phase_before".to_owned(), Value::String(phase_before));
    payload.insert("phase_after".to_owned(), Value::String(phase_after));
    payload.insert(
        "failure_threshold".to_owned(),
        Value::Number(failure_threshold.into()),
    );
    payload.insert("cooldown_ms".to_owned(), Value::Number(cooldown_ms.into()));
    payload.insert(
        "half_open_max_calls".to_owned(),
        Value::Number(half_open_max_calls.into()),
    );
    payload.insert(
        "success_threshold".to_owned(),
        Value::Number(success_threshold.into()),
    );
    payload.insert(
        "consecutive_failures".to_owned(),
        Value::Number(consecutive_failures.into()),
    );
    payload.insert(
        "half_open_remaining_calls".to_owned(),
        Value::Number(half_open_remaining_calls.into()),
    );
    payload.insert(
        "half_open_successes".to_owned(),
        Value::Number(half_open_successes.into()),
    );

    let remaining_cooldown_ms = observation
        .remaining_cooldown_ms
        .map(|value| Value::Number(value.into()))
        .unwrap_or(Value::Null);
    payload.insert("remaining_cooldown_ms".to_owned(), remaining_cooldown_ms);

    Value::Object(payload)
}

fn attach_bridge_circuit_breaker_runtime(
    bridge_execution: &mut Value,
    policy: &ConnectorCircuitBreakerPolicy,
    observation: &BridgeCircuitObservation,
) {
    let Some(bridge_execution_object) = bridge_execution.as_object_mut() else {
        return;
    };

    let runtime_value = bridge_circuit_breaker_runtime_value(policy, observation);
    bridge_execution_object.insert("circuit_breaker".to_owned(), runtime_value);
}

fn bridge_execution_reason(bridge_execution: &Value) -> Option<String> {
    bridge_execution
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn format_bridge_execution_failure_reason(
    reason: &str,
    observation: &BridgeCircuitObservation,
) -> String {
    let phase_before = observation.phase_before.as_str();
    let phase_after = observation.phase_after.as_str();

    format!(
        "{reason} (bridge_circuit_phase_before={phase_before}, bridge_circuit_phase_after={phase_after})"
    )
}

#[async_trait]
impl CoreConnectorAdapter for DynamicCatalogConnector {
    fn name(&self) -> &str {
        &self.connector_name
    }

    async fn invoke_core(
        &self,
        command: ConnectorCommand,
    ) -> Result<ConnectorOutcome, ConnectorError> {
        let requested_channel = command
            .payload
            .get("channel_id")
            .and_then(Value::as_str)
            .map(std::string::ToString::to_string);

        let (provider, chosen_channel) = {
            let catalog = self.catalog.lock().map_err(|_err| {
                ConnectorError::Execution("integration catalog mutex poisoned".to_owned())
            })?;

            let provider = catalog.provider(&self.provider_id).ok_or_else(|| {
                ConnectorError::Execution(format!(
                    "provider {} is not registered in integration catalog",
                    self.provider_id
                ))
            })?;

            let allowed_callers = provider_allowed_callers(provider);
            if !allowed_callers.is_empty() {
                let caller = caller_from_payload(&command.payload);
                if !caller_is_allowed(caller.as_deref(), &allowed_callers) {
                    let caller_label = caller.unwrap_or_else(|| "unknown".to_owned());
                    return Err(ConnectorError::Execution(format!(
                        "caller {caller_label} is not allowed for connector {} (allowed_callers={})",
                        self.connector_name,
                        allowed_callers
                            .iter()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(",")
                    )));
                }
            }

            let chosen_channel = if let Some(channel_id) = requested_channel.as_ref() {
                let channel = catalog.channel(channel_id).ok_or_else(|| {
                    ConnectorError::Execution(format!("channel {channel_id} not found"))
                })?;
                if !channel.enabled {
                    return Err(ConnectorError::Execution(format!(
                        "channel {channel_id} is disabled"
                    )));
                }
                if channel.provider_id != provider.provider_id {
                    return Err(ConnectorError::Execution(format!(
                        "channel {} does not belong to provider {}",
                        channel.channel_id, provider.provider_id
                    )));
                }
                channel.clone()
            } else {
                catalog
                    .channels_for_provider(&provider.provider_id)
                    .into_iter()
                    .find(|channel| channel.enabled)
                    .ok_or_else(|| {
                        ConnectorError::Execution(format!(
                            "no enabled channel for provider {}",
                            provider.provider_id
                        ))
                    })?
            };

            (provider.clone(), chosen_channel)
        };

        let circuit_phase_before = self.acquire_bridge_circuit_phase().await?;
        let operation = command.operation.clone();
        let payload = command.payload.clone();
        let mut bridge_execution = bridge_execution_payload(
            &provider,
            &chosen_channel,
            &command,
            &self.bridge_runtime_policy,
        )
        .await;
        let bridge_execution_success = !bridge_execution_status_is_failure(&bridge_execution);
        let circuit_observation = self
            .record_bridge_circuit_outcome(bridge_execution_success, &circuit_phase_before)
            .await;
        attach_bridge_circuit_breaker_runtime(
            &mut bridge_execution,
            &self.bridge_runtime_policy.bridge_circuit_breaker,
            &circuit_observation,
        );
        let last_failure_reason = if bridge_execution_success {
            None
        } else {
            bridge_execution_reason(&bridge_execution)
        };
        let persist_result = self
            .persist_plugin_runtime_health(
                &self.bridge_runtime_policy.bridge_circuit_breaker,
                circuit_observation.phase_after.clone(),
                circuit_observation.consecutive_failures,
                circuit_observation.half_open_remaining_calls,
                circuit_observation.half_open_successes,
                last_failure_reason,
            )
            .await;
        if let Err(error) = persist_result {
            let reason = format!("failed to persist plugin runtime health: {error}");
            return Err(ConnectorError::Execution(reason));
        }

        if bridge_execution
            .get("block_class")
            .and_then(Value::as_str)
            .is_some_and(|value| value == "compatibility_contract")
        {
            let reason = bridge_execution
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("plugin compatibility contract blocked bridge execution");
            let reason = format_bridge_execution_failure_reason(reason, &circuit_observation);
            return Err(ConnectorError::Execution(reason));
        }

        if self.bridge_runtime_policy.enforce_execution_success
            && bridge_execution
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|status| matches!(status, "blocked" | "failed"))
        {
            let reason = bridge_execution
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("bridge execution failed under strict runtime policy");
            let reason = format_bridge_execution_failure_reason(reason, &circuit_observation);
            return Err(ConnectorError::Execution(reason));
        }

        Ok(ConnectorOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "connector": self.connector_name,
                "provider_id": provider.provider_id,
                "provider_version": provider.version,
                "channel_id": chosen_channel.channel_id,
                "endpoint": chosen_channel.endpoint,
                "operation": operation,
                "payload": payload,
                "bridge_execution": bridge_execution,
            }),
        })
    }
}

pub async fn bridge_execution_payload(
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    command: &ConnectorCommand,
    runtime_policy: &BridgeRuntimePolicy,
) -> Value {
    let bridge_kind = detect_provider_bridge_kind(provider, &channel.endpoint);
    let source_kind = inferred_provider_source_kind(&provider.metadata);
    let source_path = provider_source_path(provider);
    let source_language =
        provider_source_language(&provider.metadata, Some(source_path.as_str()), source_kind);
    let adapter_family = provider
        .metadata
        .get("adapter_family")
        .cloned()
        .unwrap_or_else(|| default_runtime_adapter_family(&source_language, bridge_kind));
    let entrypoint = provider
        .metadata
        .get("entrypoint")
        .or_else(|| provider.metadata.get("entrypoint_hint"))
        .cloned()
        .unwrap_or_else(|| default_bridge_entrypoint(bridge_kind, &channel.endpoint));
    let plugin_compatibility = provider_plugin_compatibility_context(
        provider,
        channel,
        bridge_kind,
        source_kind,
        &source_path,
        &source_language,
        &adapter_family,
        &entrypoint,
        runtime_policy,
    );

    let mut plan = match bridge_kind {
        PluginBridgeKind::HttpJson => {
            let method = provider
                .metadata
                .get("http_method")
                .map(|value| value.to_ascii_uppercase())
                .unwrap_or_else(|| "POST".to_owned());
            json!({
                "status": "planned",
                "bridge_kind": bridge_kind.as_str(),
                "adapter_family": adapter_family,
                "entrypoint": entrypoint,
                "request": {
                    "method": method,
                    "url": channel.endpoint,
                    "operation": command.operation,
                    "payload": command.payload.clone(),
                }
            })
        }
        PluginBridgeKind::ProcessStdio => json!({
            "status": "planned",
            "bridge_kind": bridge_kind.as_str(),
            "adapter_family": adapter_family,
            "entrypoint": entrypoint,
            "stdio": {
                "stdin_envelope": {
                    "operation": command.operation,
                    "payload": command.payload.clone(),
                },
                "stdout_contract": "json",
            }
        }),
        PluginBridgeKind::NativeFfi => json!({
            "status": "planned",
            "bridge_kind": bridge_kind.as_str(),
            "adapter_family": adapter_family,
            "entrypoint": entrypoint,
            "ffi": {
                "library": provider
                    .metadata
                    .get("library")
                    .cloned()
                    .unwrap_or_else(|| format!("lib{}.so", provider.provider_id)),
                "symbol": entrypoint,
            }
        }),
        PluginBridgeKind::WasmComponent => json!({
            "status": "planned",
            "bridge_kind": bridge_kind.as_str(),
            "adapter_family": adapter_family,
            "entrypoint": entrypoint,
            "wasm": {
                "component": provider
                    .metadata
                    .get("component")
                    .cloned()
                    .unwrap_or_else(|| format!("{}.wasm", provider.provider_id)),
                "function": entrypoint,
            }
        }),
        PluginBridgeKind::McpServer => json!({
            "status": "planned",
            "bridge_kind": bridge_kind.as_str(),
            "adapter_family": adapter_family,
            "entrypoint": entrypoint,
            "mcp": {
                "transport": provider
                    .metadata
                    .get("transport")
                    .cloned()
                    .unwrap_or_else(|| "stdio".to_owned()),
                "handshake": "capability_schema_exchange",
            }
        }),
        PluginBridgeKind::AcpBridge => json!({
            "status": "planned",
            "bridge_kind": bridge_kind.as_str(),
            "adapter_family": adapter_family,
            "entrypoint": entrypoint,
            "acp": {
                "surface": "bridge",
                "gateway_contract": "external_bridge_runtime",
                "turn_contract": "bridge_forwarded_prompt_response",
            }
        }),
        PluginBridgeKind::AcpRuntime => json!({
            "status": "planned",
            "bridge_kind": bridge_kind.as_str(),
            "adapter_family": adapter_family,
            "entrypoint": entrypoint,
            "acp": {
                "surface": "runtime",
                "session_bootstrap": "required",
                "control_plane": "external_runtime",
                "turn_contract": "session_scoped_prompt_response",
            }
        }),
        PluginBridgeKind::Unknown => json!({
            "status": "deferred",
            "bridge_kind": bridge_kind.as_str(),
            "reason": "provider metadata does not declare a resolvable bridge_kind",
            "next_action": "set metadata.bridge_kind and rerun bootstrap",
        }),
    };

    if let Some(plugin_compatibility) = plugin_compatibility {
        let blocking_reason = plugin_compatibility.blocking_reason.clone();
        let plugin_compatibility_payload = plugin_compatibility.payload;
        let Some(plan_object) = plan.as_object_mut() else {
            return plan;
        };

        plan_object.insert(
            "plugin_compatibility".to_owned(),
            plugin_compatibility_payload,
        );
        if let Some(reason) = blocking_reason {
            plan_object.insert("status".to_owned(), Value::String("blocked".to_owned()));
            plan_object.insert("reason".to_owned(), Value::String(reason));
            plan_object.insert(
                "block_class".to_owned(),
                Value::String("compatibility_contract".to_owned()),
            );
            return plan;
        }
    }

    maybe_execute_bridge(
        plan,
        bridge_kind,
        provider,
        channel,
        command,
        runtime_policy,
    )
    .await
}

fn provider_metadata_optional_string(
    metadata: &BTreeMap<String, String>,
    key: &str,
) -> Option<String> {
    metadata
        .get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn provider_plugin_compatibility_mode(
    metadata: &BTreeMap<String, String>,
) -> Option<PluginCompatibilityMode> {
    provider_metadata_optional_string(metadata, "plugin_compatibility_mode").and_then(|value| {
        match value.as_str() {
            "native" => Some(PluginCompatibilityMode::Native),
            "openclaw_modern" => Some(PluginCompatibilityMode::OpenClawModern),
            "openclaw_legacy" => Some(PluginCompatibilityMode::OpenClawLegacy),
            _ => None,
        }
    })
}

fn provider_plugin_dialect(metadata: &BTreeMap<String, String>) -> Option<PluginContractDialect> {
    provider_metadata_optional_string(metadata, "plugin_dialect").and_then(|value| {
        match value.as_str() {
            "loongclaw_package_manifest" => Some(PluginContractDialect::LoongClawPackageManifest),
            "loongclaw_embedded_source" => Some(PluginContractDialect::LoongClawEmbeddedSource),
            "openclaw_modern_manifest" => Some(PluginContractDialect::OpenClawModernManifest),
            "openclaw_legacy_package" => Some(PluginContractDialect::OpenClawLegacyPackage),
            _ => None,
        }
    })
}

fn provider_plugin_source_kind(metadata: &BTreeMap<String, String>) -> Option<PluginSourceKind> {
    provider_metadata_optional_string(metadata, "plugin_source_kind").and_then(|value| match value
        .as_str()
    {
        "package_manifest" => Some(PluginSourceKind::PackageManifest),
        "embedded_source" => Some(PluginSourceKind::EmbeddedSource),
        _ => None,
    })
}

fn provider_plugin_compatibility_shim(
    metadata: &BTreeMap<String, String>,
    compatibility_mode: Option<PluginCompatibilityMode>,
) -> Option<PluginCompatibilityShim> {
    let shim_id = provider_metadata_optional_string(metadata, "plugin_compatibility_shim_id");
    let family = provider_metadata_optional_string(metadata, "plugin_compatibility_shim_family");

    match (shim_id, family) {
        (None, None) => compatibility_mode.and_then(PluginCompatibilityShim::for_mode),
        (Some(shim_id), None) => Some(PluginCompatibilityShim {
            family: shim_id.clone(),
            shim_id,
        }),
        (None, Some(family)) => Some(PluginCompatibilityShim {
            shim_id: family.clone(),
            family,
        }),
        (Some(shim_id), Some(family)) => Some(PluginCompatibilityShim { shim_id, family }),
    }
}

fn provider_plugin_contract_compatibility(
    metadata: &BTreeMap<String, String>,
) -> Option<PluginCompatibility> {
    let host_api = provider_metadata_optional_string(metadata, "plugin_compatibility_host_api");
    let host_version_req =
        provider_metadata_optional_string(metadata, "plugin_compatibility_host_version_req");

    if host_api.is_none() && host_version_req.is_none() {
        return None;
    }

    Some(PluginCompatibility {
        host_api,
        host_version_req,
    })
}

fn provider_is_plugin_backed(metadata: &BTreeMap<String, String>) -> bool {
    [
        "plugin_id",
        "plugin_source_path",
        "plugin_manifest_api_version",
        "plugin_dialect",
        "plugin_compatibility_mode",
    ]
    .iter()
    .any(|key| metadata.contains_key(*key))
}

fn inferred_provider_source_kind(metadata: &BTreeMap<String, String>) -> PluginSourceKind {
    provider_plugin_source_kind(metadata)
        .or_else(|| {
            provider_plugin_dialect(metadata).map(|dialect| match dialect {
                PluginContractDialect::LoongClawPackageManifest
                | PluginContractDialect::OpenClawModernManifest
                | PluginContractDialect::OpenClawLegacyPackage => PluginSourceKind::PackageManifest,
                PluginContractDialect::LoongClawEmbeddedSource => PluginSourceKind::EmbeddedSource,
            })
        })
        .or_else(|| {
            provider_metadata_optional_string(metadata, "plugin_package_manifest_path")
                .map(|_| PluginSourceKind::PackageManifest)
        })
        .unwrap_or(PluginSourceKind::EmbeddedSource)
}

fn provider_source_path(provider: &kernel::ProviderConfig) -> String {
    provider_metadata_optional_string(&provider.metadata, "plugin_source_path")
        .unwrap_or_else(|| format!("provider://{}", provider.provider_id))
}

fn provider_activation_runtime_contract_state(
    metadata: &BTreeMap<String, String>,
) -> ProviderActivationRuntimeContractState {
    let raw_contract = metadata
        .get(PLUGIN_ACTIVATION_RUNTIME_CONTRACT_METADATA_KEY)
        .cloned();
    let checksum = provider_metadata_optional_string(
        metadata,
        PLUGIN_ACTIVATION_RUNTIME_CONTRACT_CHECKSUM_METADATA_KEY,
    )
    .map(|value| value.to_ascii_lowercase());
    let metadata_present = raw_contract.is_some() || checksum.is_some();

    let Some(raw_contract) = raw_contract else {
        return ProviderActivationRuntimeContractState {
            metadata_present,
            contract: None,
            checksum: checksum.clone(),
            computed_checksum: None,
            integrity_issue: checksum.as_ref().map(|_| {
                "plugin activation contract metadata declares an attested checksum but no activation contract payload".to_owned()
            }),
        };
    };

    let computed_checksum = activation_runtime_contract_checksum_hex(raw_contract.as_bytes());
    let Some(checksum) = checksum else {
        return ProviderActivationRuntimeContractState {
            metadata_present,
            contract: None,
            checksum: None,
            computed_checksum: Some(computed_checksum),
            integrity_issue: Some(
                "plugin activation contract metadata is missing attested checksum".to_owned(),
            ),
        };
    };

    if checksum != computed_checksum {
        return ProviderActivationRuntimeContractState {
            metadata_present,
            contract: None,
            checksum: Some(checksum.clone()),
            computed_checksum: Some(computed_checksum.clone()),
            integrity_issue: Some(format!(
                "plugin activation contract checksum mismatch: metadata declares `{checksum}` but payload hashes to `{computed_checksum}`"
            )),
        };
    }

    match parse_plugin_activation_runtime_contract(&raw_contract) {
        Ok(contract) => ProviderActivationRuntimeContractState {
            metadata_present,
            contract: Some(contract),
            checksum: Some(checksum),
            computed_checksum: Some(computed_checksum),
            integrity_issue: None,
        },
        Err(error) => ProviderActivationRuntimeContractState {
            metadata_present,
            contract: None,
            checksum: Some(checksum),
            computed_checksum: Some(computed_checksum),
            integrity_issue: Some(format!(
                "plugin activation contract payload is invalid: {error}"
            )),
        },
    }
}

pub(crate) fn normalize_runtime_source_language(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "rs" => "rust".to_owned(),
        "py" => "python".to_owned(),
        "js" | "mjs" | "cjs" | "cts" | "mts" | "jsx" => "javascript".to_owned(),
        "ts" | "tsx" => "typescript".to_owned(),
        "go" => "go".to_owned(),
        "wasm" => "wasm".to_owned(),
        "manifest" => "manifest".to_owned(),
        "unknown" | "" => "unknown".to_owned(),
        other => other.to_owned(),
    }
}

fn provider_source_language(
    metadata: &BTreeMap<String, String>,
    source_path: Option<&str>,
    source_kind: PluginSourceKind,
) -> String {
    provider_metadata_optional_string(metadata, "source_language")
        .map(|value| normalize_runtime_source_language(&value))
        .unwrap_or_else(|| {
            if matches!(source_kind, PluginSourceKind::PackageManifest) {
                return "manifest".to_owned();
            }

            source_path
                .and_then(|path| Path::new(path).extension().and_then(|ext| ext.to_str()))
                .map(normalize_runtime_source_language)
                .unwrap_or_else(|| "unknown".to_owned())
        })
}

pub(crate) fn default_runtime_adapter_family(
    source_language: &str,
    bridge_kind: PluginBridgeKind,
) -> String {
    match bridge_kind {
        PluginBridgeKind::HttpJson => "http-adapter".to_owned(),
        PluginBridgeKind::ProcessStdio => format!("{source_language}-stdio-adapter"),
        PluginBridgeKind::NativeFfi => format!("{source_language}-ffi-adapter"),
        PluginBridgeKind::WasmComponent => "wasm-component-adapter".to_owned(),
        PluginBridgeKind::McpServer => "mcp-adapter".to_owned(),
        PluginBridgeKind::AcpBridge => "acp-bridge-adapter".to_owned(),
        PluginBridgeKind::AcpRuntime => "acp-runtime-adapter".to_owned(),
        PluginBridgeKind::Unknown => format!("{source_language}-unknown-adapter"),
    }
}

fn projected_plugin_activation_runtime_contract(
    provider: &kernel::ProviderConfig,
    source_path: &str,
    source_kind: PluginSourceKind,
    source_language: &str,
    bridge_kind: PluginBridgeKind,
    adapter_family: &str,
    entrypoint_hint: &str,
) -> Option<PluginActivationRuntimeContract> {
    let dialect = provider_plugin_dialect(&provider.metadata)?;
    let compatibility_mode = provider_plugin_compatibility_mode(&provider.metadata)?;

    Some(PluginActivationRuntimeContract {
        plugin_id: provider_metadata_optional_string(&provider.metadata, "plugin_id")
            .unwrap_or_else(|| provider.provider_id.clone()),
        source_path: source_path.to_owned(),
        source_kind,
        dialect,
        dialect_version: provider_metadata_optional_string(
            &provider.metadata,
            "plugin_dialect_version",
        ),
        compatibility_mode,
        compatibility_shim: provider_plugin_compatibility_shim(
            &provider.metadata,
            Some(compatibility_mode),
        ),
        bridge_kind,
        adapter_family: adapter_family.to_owned(),
        entrypoint_hint: entrypoint_hint.to_owned(),
        source_language: source_language.to_owned(),
        compatibility: provider_plugin_contract_compatibility(&provider.metadata),
    })
}

fn canonical_dialect_for_mode(
    compatibility_mode: PluginCompatibilityMode,
    source_kind: PluginSourceKind,
) -> PluginContractDialect {
    match compatibility_mode {
        PluginCompatibilityMode::Native => match source_kind {
            PluginSourceKind::PackageManifest => PluginContractDialect::LoongClawPackageManifest,
            PluginSourceKind::EmbeddedSource => PluginContractDialect::LoongClawEmbeddedSource,
        },
        PluginCompatibilityMode::OpenClawModern => PluginContractDialect::OpenClawModernManifest,
        PluginCompatibilityMode::OpenClawLegacy => PluginContractDialect::OpenClawLegacyPackage,
    }
}

fn provider_plugin_compatibility_projection_issue(
    is_plugin_backed: bool,
    dialect: Option<PluginContractDialect>,
    compatibility_mode: Option<PluginCompatibilityMode>,
    compatibility_shim: Option<&PluginCompatibilityShim>,
    source_kind: PluginSourceKind,
) -> Option<String> {
    if !is_plugin_backed
        && dialect.is_none()
        && compatibility_mode.is_none()
        && compatibility_shim.is_none()
    {
        return None;
    }

    let Some(compatibility_mode) = compatibility_mode else {
        return Some(
            "plugin-backed provider metadata drifted: missing `plugin_compatibility_mode`"
                .to_owned(),
        );
    };

    let Some(dialect) = dialect else {
        return Some(
            "plugin-backed provider metadata drifted: missing `plugin_dialect`".to_owned(),
        );
    };

    let canonical_dialect = canonical_dialect_for_mode(compatibility_mode, source_kind);
    if dialect != canonical_dialect {
        return Some(format!(
            "plugin compatibility projection drifted: mode `{}` expects canonical dialect `{}` but provider metadata declares `{}`",
            compatibility_mode.as_str(),
            canonical_dialect.as_str(),
            dialect.as_str()
        ));
    }

    let canonical_shim = PluginCompatibilityShim::for_mode(compatibility_mode);
    match (canonical_shim.as_ref(), compatibility_shim) {
        (None, Some(actual)) => {
            return Some(format!(
                "plugin compatibility projection drifted: native mode must not declare compatibility shim `{}` ({})",
                actual.shim_id, actual.family
            ));
        }
        (Some(expected), None) => {
            return Some(format!(
                "plugin compatibility projection drifted: mode `{}` requires canonical compatibility shim `{}` ({})",
                compatibility_mode.as_str(),
                expected.shim_id,
                expected.family
            ));
        }
        (Some(expected), Some(actual)) if actual != expected => {
            return Some(format!(
                "plugin compatibility projection drifted: mode `{}` expects compatibility shim `{}` ({}) but provider metadata declares `{}` ({})",
                compatibility_mode.as_str(),
                expected.shim_id,
                expected.family,
                actual.shim_id,
                actual.family
            ));
        }
        _ => {}
    }

    None
}

fn format_plugin_compatibility_shim(shim: Option<&PluginCompatibilityShim>) -> String {
    shim.map(|shim| format!("`{}` ({})", shim.shim_id, shim.family))
        .unwrap_or_else(|| "none".to_owned())
}

fn format_plugin_contract_compatibility(compatibility: Option<&PluginCompatibility>) -> String {
    compatibility
        .map(|compatibility| {
            format!(
                "host_api={}, host_version_req={}",
                compatibility.host_api.as_deref().unwrap_or("none"),
                compatibility.host_version_req.as_deref().unwrap_or("none")
            )
        })
        .unwrap_or_else(|| "none".to_owned())
}

fn activation_runtime_contract_drift_issue(
    attested: &PluginActivationRuntimeContract,
    current: Option<&PluginActivationRuntimeContract>,
    self_projection_issue: Option<&str>,
) -> Option<String> {
    if let Some(issue) = self_projection_issue {
        return Some(format!(
            "plugin activation contract drifted after registration: {issue}"
        ));
    }

    let Some(current) = current else {
        return Some(
            "plugin activation contract drifted after registration: current provider metadata no longer projects a complete plugin runtime contract"
                .to_owned(),
        );
    };

    if current.plugin_id != attested.plugin_id {
        return Some(format!(
            "plugin activation contract drifted after registration: approved plugin_id `{}` but current projection resolves `{}`",
            attested.plugin_id, current.plugin_id
        ));
    }
    if current.source_path != attested.source_path {
        return Some(format!(
            "plugin activation contract drifted after registration: approved source_path `{}` but current projection resolves `{}`",
            attested.source_path, current.source_path
        ));
    }
    if current.source_kind != attested.source_kind {
        return Some(format!(
            "plugin activation contract drifted after registration: approved source_kind `{}` but current projection resolves `{}`",
            attested.source_kind.as_str(),
            current.source_kind.as_str()
        ));
    }
    if current.dialect != attested.dialect {
        return Some(format!(
            "plugin activation contract drifted after registration: approved dialect `{}` but current projection resolves `{}`",
            attested.dialect.as_str(),
            current.dialect.as_str()
        ));
    }
    if current.dialect_version != attested.dialect_version {
        return Some(format!(
            "plugin activation contract drifted after registration: approved dialect_version `{}` but current projection resolves `{}`",
            attested.dialect_version.as_deref().unwrap_or("none"),
            current.dialect_version.as_deref().unwrap_or("none")
        ));
    }
    if current.compatibility_mode != attested.compatibility_mode {
        return Some(format!(
            "plugin activation contract drifted after registration: approved compatibility_mode `{}` but current projection resolves `{}`",
            attested.compatibility_mode.as_str(),
            current.compatibility_mode.as_str()
        ));
    }
    if current.compatibility_shim != attested.compatibility_shim {
        return Some(format!(
            "plugin activation contract drifted after registration: approved compatibility_shim {} but current projection resolves {}",
            format_plugin_compatibility_shim(attested.compatibility_shim.as_ref()),
            format_plugin_compatibility_shim(current.compatibility_shim.as_ref())
        ));
    }
    if current.bridge_kind != attested.bridge_kind {
        return Some(format!(
            "plugin activation contract drifted after registration: approved bridge_kind `{}` but current projection resolves `{}`",
            attested.bridge_kind.as_str(),
            current.bridge_kind.as_str()
        ));
    }
    if current.adapter_family != attested.adapter_family {
        return Some(format!(
            "plugin activation contract drifted after registration: approved adapter_family `{}` but current projection resolves `{}`",
            attested.adapter_family, current.adapter_family
        ));
    }
    if current.entrypoint_hint != attested.entrypoint_hint {
        return Some(format!(
            "plugin activation contract drifted after registration: approved entrypoint_hint `{}` but current projection resolves `{}`",
            attested.entrypoint_hint, current.entrypoint_hint
        ));
    }
    if current.source_language != attested.source_language {
        return Some(format!(
            "plugin activation contract drifted after registration: approved source_language `{}` but current projection resolves `{}`",
            attested.source_language, current.source_language
        ));
    }
    if current.compatibility != attested.compatibility {
        return Some(format!(
            "plugin activation contract drifted after registration: approved compatibility `{}` but current projection resolves `{}`",
            format_plugin_contract_compatibility(attested.compatibility.as_ref()),
            format_plugin_contract_compatibility(current.compatibility.as_ref())
        ));
    }

    None
}

fn shim_support_profile_mismatch_reasons(
    profile: &PluginCompatibilityShimSupport,
    ir: &PluginIR,
) -> Vec<String> {
    let mut reasons = Vec::new();

    if !profile.supported_dialects.is_empty() && !profile.supported_dialects.contains(&ir.dialect) {
        reasons.push(format!("dialect `{}`", ir.dialect.as_str()));
    }

    if !profile.supported_bridges.is_empty()
        && !profile.supported_bridges.contains(&ir.runtime.bridge_kind)
    {
        reasons.push(format!("bridge kind `{}`", ir.runtime.bridge_kind.as_str()));
    }

    if !profile.supported_adapter_families.is_empty()
        && !profile
            .supported_adapter_families
            .contains(&ir.runtime.adapter_family.trim().to_ascii_lowercase())
    {
        reasons.push(format!("adapter family `{}`", ir.runtime.adapter_family));
    }

    if !profile.supported_source_languages.is_empty()
        && !profile
            .supported_source_languages
            .contains(&ir.runtime.source_language)
    {
        reasons.push(format!("source language `{}`", ir.runtime.source_language));
    }

    reasons
}

fn provider_plugin_host_compatibility_issue(
    compatibility: Option<&PluginCompatibility>,
) -> Option<String> {
    let compatibility = compatibility?;

    if let Some(host_api) = compatibility.host_api.as_deref()
        && host_api != CURRENT_PLUGIN_HOST_API
    {
        return Some(format!(
            "plugin compatibility.host_api `{host_api}` is not supported by current host api `{CURRENT_PLUGIN_HOST_API}`"
        ));
    }

    if let Some(host_version_req) = compatibility.host_version_req.as_deref() {
        let parsed_req = match VersionReq::parse(host_version_req) {
            Ok(parsed_req) => parsed_req,
            Err(error) => {
                return Some(format!(
                    "plugin compatibility.host_version_req `{host_version_req}` is invalid: {error}"
                ));
            }
        };
        let current_version_string = env!("CARGO_PKG_VERSION");
        let current_version = match Version::parse(current_version_string) {
            Ok(current_version) => current_version,
            Err(error) => {
                return Some(format!(
                    "current host version `{current_version_string}` is invalid semver: {error}"
                ));
            }
        };
        if !parsed_req.matches(&current_version) {
            return Some(format!(
                "plugin compatibility.host_version_req `{host_version_req}` does not match current host version `{current_version}`"
            ));
        }
    }

    None
}

fn plugin_ir_from_runtime_contract(
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    contract: &PluginActivationRuntimeContract,
) -> PluginIR {
    PluginIR {
        manifest_api_version: provider_metadata_optional_string(
            &provider.metadata,
            "plugin_manifest_api_version",
        ),
        plugin_version: provider_metadata_optional_string(&provider.metadata, "plugin_version")
            .or_else(|| provider_metadata_optional_string(&provider.metadata, "version"))
            .or_else(|| Some(provider.version.clone())),
        dialect: contract.dialect,
        dialect_version: contract.dialect_version.clone(),
        compatibility_mode: contract.compatibility_mode,
        plugin_id: contract.plugin_id.clone(),
        provider_id: provider.provider_id.clone(),
        connector_name: provider.connector_name.clone(),
        channel_id: Some(channel.channel_id.clone()),
        endpoint: Some(channel.endpoint.clone()),
        capabilities: BTreeSet::new(),
        metadata: provider.metadata.clone(),
        source_path: contract.source_path.clone(),
        source_kind: contract.source_kind,
        package_root: provider_metadata_optional_string(&provider.metadata, "plugin_package_root")
            .unwrap_or_else(|| contract.source_path.clone()),
        package_manifest_path: provider_metadata_optional_string(
            &provider.metadata,
            "plugin_package_manifest_path",
        ),
        diagnostic_findings: Vec::new(),
        setup: None,
        slot_claims: Vec::new(),
        compatibility: contract.compatibility.clone(),
        runtime: PluginRuntimeProfile {
            source_language: contract.source_language.clone(),
            bridge_kind: contract.bridge_kind,
            adapter_family: contract.adapter_family.clone(),
            entrypoint_hint: contract.entrypoint_hint.clone(),
        },
    }
}

fn shim_support_profile_payload(profile: &PluginCompatibilityShimSupport) -> Value {
    let mut payload = Map::new();

    if let Some(version) = &profile.version {
        let version = version.clone();

        payload.insert("version".to_owned(), Value::String(version));
    }
    if !profile.supported_dialects.is_empty() {
        let supported_dialects = profile
            .supported_dialects
            .iter()
            .map(|dialect| Value::String(dialect.as_str().to_owned()))
            .collect();

        payload.insert(
            "supported_dialects".to_owned(),
            Value::Array(supported_dialects),
        );
    }
    if !profile.supported_bridges.is_empty() {
        let supported_bridges = profile
            .supported_bridges
            .iter()
            .map(|bridge| Value::String(bridge.as_str().to_owned()))
            .collect();

        payload.insert(
            "supported_bridges".to_owned(),
            Value::Array(supported_bridges),
        );
    }
    if !profile.supported_adapter_families.is_empty() {
        let supported_adapter_families = profile
            .supported_adapter_families
            .iter()
            .map(|family| Value::String(family.clone()))
            .collect();

        payload.insert(
            "supported_adapter_families".to_owned(),
            Value::Array(supported_adapter_families),
        );
    }
    if !profile.supported_source_languages.is_empty() {
        let supported_source_languages = profile
            .supported_source_languages
            .iter()
            .map(|language| Value::String(language.clone()))
            .collect();

        payload.insert(
            "supported_source_languages".to_owned(),
            Value::Array(supported_source_languages),
        );
    }

    Value::Object(payload)
}

fn provider_plugin_compatibility_context(
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    bridge_kind: PluginBridgeKind,
    source_kind: PluginSourceKind,
    source_path: &str,
    source_language: &str,
    adapter_family: &str,
    entrypoint: &str,
    runtime_policy: &BridgeRuntimePolicy,
) -> Option<ProviderPluginCompatibilityContext> {
    let current_dialect = provider_plugin_dialect(&provider.metadata);
    let current_dialect_version =
        provider_metadata_optional_string(&provider.metadata, "plugin_dialect_version");
    let current_compatibility_mode = provider_plugin_compatibility_mode(&provider.metadata);
    let current_compatibility_shim =
        provider_plugin_compatibility_shim(&provider.metadata, current_compatibility_mode);
    let current_compatibility = provider_plugin_contract_compatibility(&provider.metadata);
    let attestation_state = provider_activation_runtime_contract_state(&provider.metadata);
    let attested_contract = attestation_state.contract.as_ref();
    let attested_contract_checksum = attestation_state.checksum.clone();
    let attested_contract_computed_checksum = attestation_state.computed_checksum.clone();
    let attestation_integrity_issue = attestation_state.integrity_issue.clone();
    let current_contract = projected_plugin_activation_runtime_contract(
        provider,
        source_path,
        source_kind,
        source_language,
        bridge_kind,
        adapter_family,
        entrypoint,
    );
    let effective_contract = attested_contract.or(current_contract.as_ref());
    let shim_support_profile = effective_contract
        .and_then(|contract| {
            runtime_policy
                .compatibility_matrix
                .compatibility_shim_support_profile(contract.compatibility_shim.as_ref())
        })
        .cloned();
    let is_plugin_backed =
        provider_is_plugin_backed(&provider.metadata) || attestation_state.metadata_present;

    if !is_plugin_backed
        && current_dialect.is_none()
        && current_dialect_version.is_none()
        && current_compatibility_mode.is_none()
        && current_compatibility_shim.is_none()
        && current_compatibility.is_none()
        && shim_support_profile.is_none()
    {
        return None;
    }

    let projection_issue = provider_plugin_compatibility_projection_issue(
        is_plugin_backed,
        current_dialect,
        current_compatibility_mode,
        current_compatibility_shim.as_ref(),
        source_kind,
    );
    let activation_contract_drift_issue = attested_contract.as_ref().and_then(|contract| {
        activation_runtime_contract_drift_issue(
            contract,
            current_contract.as_ref(),
            projection_issue.as_deref(),
        )
    });

    let runtime_ir = effective_contract
        .map(|contract| plugin_ir_from_runtime_contract(provider, channel, contract));

    let shim_support_mismatch_reasons = runtime_ir
        .as_ref()
        .zip(shim_support_profile.as_ref())
        .map(|(ir, profile)| shim_support_profile_mismatch_reasons(profile, ir))
        .unwrap_or_default();

    let blocking_reason = attestation_integrity_issue.clone()
        .or(activation_contract_drift_issue)
        .or_else(|| {
            let contract = effective_contract?;
            let compatibility_mode = contract.compatibility_mode;
            if !runtime_policy
                .compatibility_matrix
                .is_compatibility_mode_supported(compatibility_mode)
            {
                let shim_clause = contract
                    .compatibility_shim
                    .as_ref()
                    .map(|shim| format!(" via shim `{}` ({})", shim.shim_id, shim.family))
                    .unwrap_or_default();
                return Some(format!(
                    "compatibility mode {} requires a host shim that is not enabled in the current runtime matrix{}",
                    compatibility_mode.as_str(),
                    shim_clause
                ));
            }

            if !runtime_policy
                .compatibility_matrix
                .is_compatibility_shim_supported(contract.compatibility_shim.as_ref())
            {
                let shim = match contract.compatibility_shim.as_ref() {
                    Some(shim) => shim,
                    None => {
                        return Some(format!(
                            "compatibility mode {} requires a canonical compatibility shim, but none was resolved in the activation contract",
                            compatibility_mode.as_str()
                        ));
                    }
                };
                return Some(format!(
                    "compatibility mode {} requires compatibility shim `{}` ({}) that is not enabled in the current runtime matrix",
                    compatibility_mode.as_str(),
                    shim.shim_id,
                    shim.family
                ));
            }

            if let Some(reason) =
                provider_plugin_host_compatibility_issue(contract.compatibility.as_ref())
            {
                return Some(reason);
            }

            let ir = runtime_ir.as_ref()?;
            if let Some(reason) = runtime_policy
                .compatibility_matrix
                .compatibility_shim_support_issue(ir, contract.compatibility_shim.as_ref())
            {
                return Some(reason);
            }

            if !runtime_policy
                .compatibility_matrix
                .is_bridge_supported(contract.bridge_kind)
            {
                return Some(format!(
                    "bridge kind {} is not supported by current runtime matrix",
                    contract.bridge_kind.as_str()
                ));
            }

            if !runtime_policy
                .compatibility_matrix
                .is_adapter_family_supported(&contract.adapter_family)
            {
                return Some(format!(
                    "adapter family {} is not supported by current runtime matrix",
                    contract.adapter_family
                ));
            }

            None
        })
        .or_else(|| {
            effective_contract
                .and_then(|contract| provider_plugin_host_compatibility_issue(contract.compatibility.as_ref()))
        })
        .or_else(|| projection_issue.clone());

    let mut payload = Map::new();
    if let Some(contract) = effective_contract {
        let dialect = contract.dialect.as_str().to_owned();

        payload.insert("dialect".to_owned(), Value::String(dialect));
        if let Some(dialect_version) = &contract.dialect_version {
            let dialect_version = dialect_version.clone();

            payload.insert("dialect_version".to_owned(), Value::String(dialect_version));
        }
        let compatibility_mode = contract.compatibility_mode.as_str().to_owned();

        payload.insert("mode".to_owned(), Value::String(compatibility_mode));
        if let Some(compatibility_shim) = &contract.compatibility_shim {
            let mut shim_payload = Map::new();
            let shim_id = compatibility_shim.shim_id.clone();
            let family = compatibility_shim.family.clone();

            shim_payload.insert("shim_id".to_owned(), Value::String(shim_id));
            shim_payload.insert("family".to_owned(), Value::String(family));
            payload.insert("shim".to_owned(), Value::Object(shim_payload));
        }
        if let Some(compatibility) = &contract.compatibility {
            if let Some(host_api) = &compatibility.host_api {
                let host_api = host_api.clone();

                payload.insert("host_api".to_owned(), Value::String(host_api));
            }
            if let Some(host_version_req) = &compatibility.host_version_req {
                let host_version_req = host_version_req.clone();

                payload.insert(
                    "host_version_req".to_owned(),
                    Value::String(host_version_req),
                );
            }
        }
    }
    if let Some(shim_support_profile) = shim_support_profile {
        let shim_support = shim_support_profile_payload(&shim_support_profile);

        payload.insert("shim_support".to_owned(), shim_support);
    }
    if !shim_support_mismatch_reasons.is_empty() {
        let mismatch_reasons = shim_support_mismatch_reasons
            .iter()
            .cloned()
            .map(Value::String)
            .collect();

        payload.insert(
            "shim_support_mismatch_reasons".to_owned(),
            Value::Array(mismatch_reasons),
        );
    }
    if attestation_integrity_issue.is_none()
        && let Some(contract) = attested_contract
    {
        let activation_contract = plugin_activation_runtime_contract_value(contract);

        payload.insert("activation_contract".to_owned(), activation_contract);
    }
    if let Some(checksum) = attested_contract_checksum {
        payload.insert(
            "activation_contract_checksum".to_owned(),
            Value::String(checksum),
        );
    }

    let mut runtime_projection = Map::new();
    let source_path = source_path.to_owned();
    let source_kind = source_kind.as_str().to_owned();
    let source_language = source_language.to_owned();
    let bridge_kind = bridge_kind.as_str().to_owned();
    let adapter_family = adapter_family.to_owned();
    let entrypoint_hint = entrypoint.to_owned();

    runtime_projection.insert("source_path".to_owned(), Value::String(source_path));
    runtime_projection.insert("source_kind".to_owned(), Value::String(source_kind));
    runtime_projection.insert("source_language".to_owned(), Value::String(source_language));
    runtime_projection.insert("bridge_kind".to_owned(), Value::String(bridge_kind));
    runtime_projection.insert("adapter_family".to_owned(), Value::String(adapter_family));
    runtime_projection.insert("entrypoint_hint".to_owned(), Value::String(entrypoint_hint));
    if let Some(dialect) = current_dialect {
        let dialect = dialect.as_str().to_owned();

        runtime_projection.insert("dialect".to_owned(), Value::String(dialect));
    }
    if let Some(dialect_version) = current_dialect_version {
        runtime_projection.insert("dialect_version".to_owned(), Value::String(dialect_version));
    }
    if let Some(compatibility_mode) = current_compatibility_mode {
        let compatibility_mode = compatibility_mode.as_str().to_owned();

        runtime_projection.insert("mode".to_owned(), Value::String(compatibility_mode));
    }
    if let Some(compatibility_shim) = current_compatibility_shim {
        let mut shim_payload = Map::new();
        let shim_id = compatibility_shim.shim_id;
        let family = compatibility_shim.family;

        shim_payload.insert("shim_id".to_owned(), Value::String(shim_id));
        shim_payload.insert("family".to_owned(), Value::String(family));
        runtime_projection.insert("shim".to_owned(), Value::Object(shim_payload));
    }
    payload.insert(
        "runtime_projection".to_owned(),
        Value::Object(runtime_projection),
    );

    let runtime_guard_status = if blocking_reason.is_some() {
        "blocked".to_owned()
    } else {
        "passed".to_owned()
    };
    let activation_contract_attested = attestation_state.metadata_present;
    let activation_contract_verified =
        attested_contract.is_some() && attestation_integrity_issue.is_none();
    let activation_contract_integrity = if !attestation_state.metadata_present {
        "missing".to_owned()
    } else if attestation_integrity_issue.is_some() {
        "invalid".to_owned()
    } else {
        "verified".to_owned()
    };
    let mut runtime_guard = Map::new();

    runtime_guard.insert("status".to_owned(), Value::String(runtime_guard_status));
    runtime_guard.insert(
        "kind".to_owned(),
        Value::String("compatibility_contract".to_owned()),
    );
    runtime_guard.insert(
        "activation_contract_attested".to_owned(),
        Value::Bool(activation_contract_attested),
    );
    runtime_guard.insert(
        "activation_contract_verified".to_owned(),
        Value::Bool(activation_contract_verified),
    );
    runtime_guard.insert(
        "activation_contract_integrity".to_owned(),
        Value::String(activation_contract_integrity),
    );
    if let Some(checksum) = attested_contract_computed_checksum {
        runtime_guard.insert(
            "activation_contract_computed_checksum".to_owned(),
            Value::String(checksum),
        );
    }
    if let Some(issue) = attestation_integrity_issue.as_ref() {
        let issue = issue.clone();

        runtime_guard.insert(
            "activation_contract_integrity_issue".to_owned(),
            Value::String(issue),
        );
    }
    if let Some(reason) = blocking_reason.as_ref() {
        let reason = reason.clone();

        runtime_guard.insert("reason".to_owned(), Value::String(reason));
    }
    payload.insert("runtime_guard".to_owned(), Value::Object(runtime_guard));

    Some(ProviderPluginCompatibilityContext {
        payload: Value::Object(payload),
        blocking_reason,
    })
}

pub async fn maybe_execute_bridge(
    execution: Value,
    bridge_kind: PluginBridgeKind,
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    command: &ConnectorCommand,
    runtime_policy: &BridgeRuntimePolicy,
) -> Value {
    if runtime_policy.execute_http_json && matches!(bridge_kind, PluginBridgeKind::HttpJson) {
        return execute_http_json_bridge(execution, provider, channel, command);
    }

    if runtime_policy.execute_process_stdio && matches!(bridge_kind, PluginBridgeKind::ProcessStdio)
    {
        return execute_process_stdio_bridge(execution, provider, channel, command, runtime_policy)
            .await;
    }

    if runtime_policy.execute_wasm_component
        && matches!(bridge_kind, PluginBridgeKind::WasmComponent)
    {
        return execute_wasm_component_bridge(
            execution,
            provider,
            channel,
            command,
            runtime_policy,
        );
    }

    execution
}

include!("spec_bridge_protocol.inc.rs");

fn with_execution_security_tier(
    mut runtime: Value,
    execution_tier: ExecutionSecurityTier,
) -> Value {
    if let Some(object) = runtime.as_object_mut() {
        object.insert(
            "execution_tier".to_owned(),
            Value::String(execution_tier.to_string()),
        );
    }
    runtime
}

fn normalize_allowed_wasm_path_prefixes(prefixes: &[PathBuf]) -> Vec<PathBuf> {
    prefixes
        .iter()
        .map(|prefix| normalize_path_for_policy(prefix))
        .collect()
}

fn normalize_sha256_pin(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("wasm sha256 pin must not be empty".to_owned());
    }

    let lowered = trimmed.to_ascii_lowercase();
    let digest = lowered.strip_prefix("sha256:").unwrap_or(&lowered).trim();
    if digest.len() != 64 || !digest.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(
            "wasm sha256 pin must be 64 hex characters (optional prefix `sha256:`)".to_owned(),
        );
    }

    Ok(digest.to_owned())
}

fn resolve_expected_wasm_sha256(
    provider: &kernel::ProviderConfig,
    runtime_policy: &BridgeRuntimePolicy,
) -> Result<Option<String>, String> {
    let plugin_id = provider
        .metadata
        .get("plugin_id")
        .cloned()
        .unwrap_or_else(|| provider.provider_id.clone());

    let mut metadata_pins = Vec::new();
    for key in [
        "component_sha256",
        "component_sha256_pin",
        "component_sha256_hex",
    ] {
        if let Some(raw_pin) = provider.metadata.get(key) {
            let pin = normalize_sha256_pin(raw_pin)
                .map_err(|reason| format!("provider metadata `{key}` invalid: {reason}"))?;
            metadata_pins.push((format!("metadata.{key}"), pin));
        }
    }
    let metadata_pin = if let Some((first_source, first_pin)) = metadata_pins.first() {
        if let Some((source, _)) = metadata_pins.iter().find(|(_, pin)| pin != first_pin) {
            return Err(format!(
                "conflicting wasm sha256 pins for plugin `{plugin_id}`: {first_source} and {source} differ"
            ));
        }
        Some(first_pin.clone())
    } else {
        None
    };

    let policy_pin = if let Some(raw_pin) = runtime_policy
        .wasm_required_sha256_by_plugin
        .get(&plugin_id)
    {
        Some(normalize_sha256_pin(raw_pin).map_err(|reason| {
            format!(
                "security_scan.wasm.required_sha256_by_plugin pin invalid for plugin `{plugin_id}`: {reason}"
            )
        })?)
    } else {
        None
    };

    if let (Some(metadata_pin), Some(policy_pin)) = (metadata_pin.as_ref(), policy_pin.as_ref())
        && metadata_pin != policy_pin
    {
        return Err(format!(
            "conflicting wasm sha256 pins for plugin `{plugin_id}` between provider metadata and security_scan.wasm.required_sha256_by_plugin"
        ));
    }

    let resolved_pin = policy_pin.or(metadata_pin);

    if runtime_policy.wasm_require_hash_pin && resolved_pin.is_none() {
        return Err(format!(
            "wasm sha256 pin is required for plugin `{plugin_id}` but no pin was provided"
        ));
    }

    Ok(resolved_pin)
}

fn wasm_artifact_sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{byte:02x}"));
    }
    out
}

#[derive(Debug)]
struct WasmArtifactBytes {
    bytes: Vec<u8>,
    modified_unix_ns: Option<u128>,
    file_identity: Option<WasmArtifactFileIdentity>,
}

fn read_wasm_artifact_bytes(artifact_path: &Path) -> Result<WasmArtifactBytes, String> {
    let mut artifact_file = fs::File::open(artifact_path)
        .map_err(|error| format!("failed to open wasm artifact: {error}"))?;
    let artifact_metadata = artifact_file
        .metadata()
        .map_err(|error| format!("failed to read wasm artifact metadata: {error}"))?;
    if !artifact_metadata.file_type().is_file() {
        return Err("wasm artifact path must reference a regular file".to_owned());
    }

    let expected_size = artifact_metadata.len().min(8 * 1024 * 1024_u64) as usize;
    let mut bytes = Vec::with_capacity(expected_size);
    artifact_file
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read wasm artifact: {error}"))?;

    Ok(WasmArtifactBytes {
        bytes,
        modified_unix_ns: modified_unix_nanos(&artifact_metadata),
        file_identity: wasm_artifact_file_identity(&artifact_metadata),
    })
}

fn compile_wasm_module(
    module_bytes: &[u8],
    fuel_enabled: bool,
    artifact_sha256: Option<String>,
) -> Result<CachedWasmModule, String> {
    let mut config = WasmtimeConfig::new();
    // On macOS, default to `false` because Wasmtime's signal-based trap path
    // relies on a global machports handler thread, which has shown intermittent
    // aborts under highly parallel bridge tests.
    config.signals_based_traps(wasm_signals_based_traps_enabled_from_env());
    if fuel_enabled {
        config.consume_fuel(true);
    }
    let engine = WasmtimeEngine::new(&config)
        .map_err(|error| format!("failed to initialize wasmtime engine: {error}"))?;
    let module = WasmtimeModule::new(&engine, module_bytes)
        .map_err(|error| format!("failed to compile wasm module: {error}"))?;
    Ok(CachedWasmModule {
        engine,
        module,
        artifact_sha256,
    })
}

#[allow(clippy::indexing_slicing)] // serde_json::Value string-keyed IndexMut is infallible
pub fn execute_wasm_component_bridge(
    mut execution: Value,
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    command: &ConnectorCommand,
    runtime_policy: &BridgeRuntimePolicy,
) -> Value {
    let execution_tier = runtime_policy.wasm_execution_security_tier();
    let artifact_path = match resolve_wasm_component_artifact_path(provider, &channel.endpoint) {
        Ok(path) => path,
        Err(reason) => {
            execution["status"] = Value::String("blocked".to_owned());
            execution["reason"] = Value::String(reason);
            execution["runtime"] = with_execution_security_tier(
                json!({
                    "executor": "wasmtime_module",
                }),
                execution_tier,
            );
            return execution;
        }
    };
    let artifact_path = match fs::canonicalize(&artifact_path) {
        Ok(path) => normalize_path_for_policy(&path),
        Err(error) => {
            execution["status"] = Value::String("failed".to_owned());
            execution["reason"] = Value::String(format!(
                "failed to canonicalize wasm artifact path: {error}"
            ));
            execution["runtime"] = with_execution_security_tier(
                json!({
                    "executor": "wasmtime_module",
                    "artifact_path": artifact_path.display().to_string(),
                }),
                execution_tier,
            );
            return execution;
        }
    };
    let normalized_allowed_prefixes =
        normalize_allowed_wasm_path_prefixes(&runtime_policy.wasm_allowed_path_prefixes);

    if !normalized_allowed_prefixes.is_empty()
        && !normalized_allowed_prefixes
            .iter()
            .any(|prefix| artifact_path.starts_with(prefix))
    {
        execution["status"] = Value::String("blocked".to_owned());
        execution["reason"] =
            Value::String("wasm artifact path is outside runtime allowed_path_prefixes".to_owned());
        execution["runtime"] = with_execution_security_tier(
            json!({
                "executor": "wasmtime_module",
                "artifact_path": artifact_path.display().to_string(),
                "allowed_path_prefixes": normalized_allowed_prefixes
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>(),
            }),
            execution_tier,
        );
        return execution;
    }

    let artifact_metadata = match fs::metadata(&artifact_path) {
        Ok(metadata) => metadata,
        Err(error) => {
            execution["status"] = Value::String("failed".to_owned());
            execution["reason"] =
                Value::String(format!("failed to read wasm artifact metadata: {error}"));
            execution["runtime"] = with_execution_security_tier(
                json!({
                    "executor": "wasmtime_module",
                    "artifact_path": artifact_path.display().to_string(),
                }),
                execution_tier,
            );
            return execution;
        }
    };
    let artifact_modified_unix_ns = modified_unix_nanos(&artifact_metadata);
    let artifact_file_identity = wasm_artifact_file_identity(&artifact_metadata);
    let mut module_size_bytes = artifact_metadata.len() as usize;
    if !artifact_metadata.file_type().is_file() {
        execution["status"] = Value::String("blocked".to_owned());
        execution["reason"] =
            Value::String("wasm artifact path must reference a regular file".to_owned());
        execution["runtime"] = with_execution_security_tier(
            json!({
                "executor": "wasmtime_module",
                "artifact_path": artifact_path.display().to_string(),
            }),
            execution_tier,
        );
        return execution;
    }

    if let Some(limit) = runtime_policy.wasm_max_component_bytes
        && module_size_bytes > limit
    {
        execution["status"] = Value::String("blocked".to_owned());
        execution["reason"] = Value::String(format!(
            "wasm artifact size {} exceeds runtime max_component_bytes {limit}",
            module_size_bytes
        ));
        execution["runtime"] = with_execution_security_tier(
            json!({
                "executor": "wasmtime_module",
                "artifact_path": artifact_path.display().to_string(),
                "module_size_bytes": module_size_bytes,
                "max_component_bytes": limit,
            }),
            execution_tier,
        );
        return execution;
    }

    let export_name = resolve_wasm_export_name(provider);
    let fuel_enabled = runtime_policy.wasm_fuel_limit.is_some();
    let cache_capacity = wasm_module_cache_capacity();
    let cache_max_bytes = wasm_module_cache_max_bytes();
    let expected_sha256 = match resolve_expected_wasm_sha256(provider, runtime_policy) {
        Ok(pin) => pin,
        Err(reason) => {
            execution["status"] = Value::String("blocked".to_owned());
            execution["reason"] = Value::String(reason);
            execution["runtime"] = with_execution_security_tier(
                json!({
                    "executor": "wasmtime_module",
                    "artifact_path": artifact_path.display().to_string(),
                    "export": export_name,
                    "operation": command.operation,
                    "payload": command.payload,
                    "module_size_bytes": module_size_bytes,
                    "fuel_limit": runtime_policy.wasm_fuel_limit,
                    "cache_hit": false,
                    "cache_miss": true,
                    "cache_evicted_entries": 0,
                    "cache_entries": 0,
                    "cache_capacity": cache_capacity,
                    "cache_total_module_bytes": 0,
                    "cache_max_bytes": cache_max_bytes,
                    "cache_inserted": false,
                    "integrity_check_required": true,
                    "integrity_check_passed": false,
                }),
                execution_tier,
            );
            return execution;
        }
    };

    let initial_cache_key = build_wasm_module_cache_key(
        &artifact_path,
        module_size_bytes as u64,
        artifact_modified_unix_ns,
        artifact_file_identity,
        expected_sha256.clone(),
        fuel_enabled,
    );
    let (cached_module, cache_lookup) = match lookup_cached_wasm_module(&initial_cache_key) {
        Ok(Some(hit)) => hit,
        Ok(None) => {
            let artifact_bytes = match read_wasm_artifact_bytes(&artifact_path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    execution["status"] = Value::String("failed".to_owned());
                    execution["reason"] = Value::String(error);
                    execution["runtime"] = with_execution_security_tier(
                        json!({
                            "executor": "wasmtime_module",
                            "artifact_path": artifact_path.display().to_string(),
                            "export": export_name,
                            "operation": command.operation,
                            "payload": command.payload,
                            "module_size_bytes": module_size_bytes,
                            "fuel_limit": runtime_policy.wasm_fuel_limit,
                            "cache_hit": false,
                            "cache_miss": true,
                            "cache_evicted_entries": 0,
                            "cache_entries": 0,
                            "cache_capacity": cache_capacity,
                            "cache_total_module_bytes": 0,
                            "cache_max_bytes": cache_max_bytes,
                            "cache_inserted": false,
                        }),
                        execution_tier,
                    );
                    return execution;
                }
            };
            let module_bytes = artifact_bytes.bytes;

            module_size_bytes = module_bytes.len();
            if let Some(limit) = runtime_policy.wasm_max_component_bytes
                && module_size_bytes > limit
            {
                execution["status"] = Value::String("blocked".to_owned());
                execution["reason"] = Value::String(format!(
                    "wasm artifact size {} exceeds runtime max_component_bytes {limit}",
                    module_size_bytes
                ));
                execution["runtime"] = with_execution_security_tier(
                    json!({
                        "executor": "wasmtime_module",
                        "artifact_path": artifact_path.display().to_string(),
                        "module_size_bytes": module_size_bytes,
                        "max_component_bytes": limit,
                    }),
                    execution_tier,
                );
                return execution;
            }

            let artifact_sha256 = if let Some(expected) = expected_sha256.as_deref() {
                let actual = wasm_artifact_sha256_hex(&module_bytes);
                if actual != expected {
                    execution["status"] = Value::String("blocked".to_owned());
                    execution["reason"] = Value::String(format!(
                        "wasm artifact sha256 mismatch: expected {expected}, actual {actual}"
                    ));
                    execution["runtime"] = with_execution_security_tier(
                        json!({
                            "executor": "wasmtime_module",
                            "artifact_path": artifact_path.display().to_string(),
                            "module_size_bytes": module_size_bytes,
                            "expected_sha256": expected,
                            "artifact_sha256": actual,
                            "integrity_check_required": true,
                            "integrity_check_passed": false,
                        }),
                        execution_tier,
                    );
                    return execution;
                }
                Some(actual)
            } else {
                None
            };

            let refreshed_cache_key = build_wasm_module_cache_key(
                &artifact_path,
                module_size_bytes as u64,
                artifact_bytes
                    .modified_unix_ns
                    .or(artifact_modified_unix_ns),
                artifact_bytes.file_identity.or(artifact_file_identity),
                expected_sha256.clone(),
                fuel_enabled,
            );

            match lookup_cached_wasm_module(&refreshed_cache_key) {
                Ok(Some(hit)) => hit,
                Ok(None) => {
                    let compiled =
                        match compile_wasm_module(&module_bytes, fuel_enabled, artifact_sha256) {
                            Ok(module) => Arc::new(module),
                            Err(reason) => {
                                execution["status"] = Value::String("failed".to_owned());
                                execution["reason"] = Value::String(reason);
                                execution["runtime"] = with_execution_security_tier(
                                    json!({
                                        "executor": "wasmtime_module",
                                        "artifact_path": artifact_path.display().to_string(),
                                        "export": export_name,
                                        "operation": command.operation,
                                        "payload": command.payload,
                                        "module_size_bytes": module_size_bytes,
                                        "fuel_limit": runtime_policy.wasm_fuel_limit,
                                        "cache_hit": false,
                                        "cache_miss": true,
                                        "cache_evicted_entries": 0,
                                        "cache_entries": 0,
                                        "cache_capacity": cache_capacity,
                                        "cache_total_module_bytes": 0,
                                        "cache_max_bytes": cache_max_bytes,
                                        "cache_inserted": false,
                                    }),
                                    execution_tier,
                                );
                                return execution;
                            }
                        };
                    let cache_lookup = match insert_cached_wasm_module(
                        refreshed_cache_key,
                        compiled.clone(),
                        module_size_bytes,
                    ) {
                        Ok(lookup) => lookup,
                        Err(reason) => {
                            execution["status"] = Value::String("failed".to_owned());
                            execution["reason"] = Value::String(reason);
                            execution["runtime"] = with_execution_security_tier(
                                json!({
                                    "executor": "wasmtime_module",
                                    "artifact_path": artifact_path.display().to_string(),
                                    "export": export_name,
                                    "operation": command.operation,
                                    "payload": command.payload,
                                    "module_size_bytes": module_size_bytes,
                                    "fuel_limit": runtime_policy.wasm_fuel_limit,
                                    "cache_hit": false,
                                    "cache_miss": true,
                                    "cache_evicted_entries": 0,
                                    "cache_entries": 0,
                                    "cache_capacity": cache_capacity,
                                    "cache_total_module_bytes": 0,
                                    "cache_max_bytes": cache_max_bytes,
                                    "cache_inserted": false,
                                }),
                                execution_tier,
                            );
                            return execution;
                        }
                    };
                    (compiled, cache_lookup)
                }
                Err(reason) => {
                    execution["status"] = Value::String("failed".to_owned());
                    execution["reason"] = Value::String(reason);
                    execution["runtime"] = with_execution_security_tier(
                        json!({
                            "executor": "wasmtime_module",
                            "artifact_path": artifact_path.display().to_string(),
                            "export": export_name,
                            "operation": command.operation,
                            "payload": command.payload,
                            "module_size_bytes": module_size_bytes,
                            "fuel_limit": runtime_policy.wasm_fuel_limit,
                            "cache_hit": false,
                            "cache_miss": true,
                            "cache_evicted_entries": 0,
                            "cache_entries": 0,
                            "cache_capacity": cache_capacity,
                            "cache_total_module_bytes": 0,
                            "cache_max_bytes": cache_max_bytes,
                            "cache_inserted": false,
                        }),
                        execution_tier,
                    );
                    return execution;
                }
            }
        }
        Err(reason) => {
            execution["status"] = Value::String("failed".to_owned());
            execution["reason"] = Value::String(reason);
            execution["runtime"] = with_execution_security_tier(
                json!({
                    "executor": "wasmtime_module",
                    "artifact_path": artifact_path.display().to_string(),
                    "export": export_name,
                    "operation": command.operation,
                    "payload": command.payload,
                    "module_size_bytes": module_size_bytes,
                    "fuel_limit": runtime_policy.wasm_fuel_limit,
                    "cache_hit": false,
                    "cache_miss": true,
                    "cache_evicted_entries": 0,
                    "cache_entries": 0,
                    "cache_capacity": cache_capacity,
                    "cache_total_module_bytes": 0,
                    "cache_max_bytes": cache_max_bytes,
                    "cache_inserted": false,
                }),
                execution_tier,
            );
            return execution;
        }
    };

    let run_result = (|| -> Result<Option<u64>, String> {
        let mut store = WasmtimeStore::new(&cached_module.engine, ());
        if let Some(limit) = runtime_policy.wasm_fuel_limit {
            store
                .set_fuel(limit)
                .map_err(|error| format!("failed to set wasm fuel limit: {error}"))?;
        }
        let linker = WasmtimeLinker::new(&cached_module.engine);
        let instance = linker
            .instantiate(&mut store, &cached_module.module)
            .map_err(|error| format!("failed to instantiate wasm module: {error}"))?;
        let func = instance
            .get_typed_func::<(), ()>(&mut store, export_name.as_str())
            .map_err(|error| {
                format!("failed to resolve exported wasm function {export_name}: {error}")
            })?;
        func.call(&mut store, ())
            .map_err(|error| format!("wasm function call failed: {error}"))?;
        let consumed_fuel = runtime_policy
            .wasm_fuel_limit
            .map(|limit| {
                store
                    .get_fuel()
                    .map(|remaining| limit.saturating_sub(remaining))
            })
            .transpose()
            .map_err(|error| format!("failed to query wasm fuel: {error}"))?;
        Ok(consumed_fuel)
    })();

    match run_result {
        Ok(consumed_fuel) => {
            execution["status"] = Value::String("executed".to_owned());
            execution["runtime"] = with_execution_security_tier(
                json!({
                    "executor": "wasmtime_module",
                    "artifact_path": artifact_path.display().to_string(),
                    "export": export_name,
                    "operation": command.operation,
                    "payload": command.payload,
                    "module_size_bytes": module_size_bytes,
                    "fuel_limit": runtime_policy.wasm_fuel_limit,
                    "fuel_consumed": consumed_fuel,
                    "cache_hit": cache_lookup.hit,
                    "cache_miss": !cache_lookup.hit,
                    "cache_evicted_entries": cache_lookup.evicted_entries,
                    "cache_entries": cache_lookup.cache_len,
                    "cache_capacity": cache_lookup.cache_capacity,
                    "cache_total_module_bytes": cache_lookup.cache_total_module_bytes,
                    "cache_max_bytes": cache_lookup.cache_max_bytes,
                    "cache_inserted": cache_lookup.inserted,
                    "expected_sha256": expected_sha256,
                    "artifact_sha256": cached_module.artifact_sha256.clone(),
                    "integrity_check_required": expected_sha256.is_some(),
                    "integrity_check_passed": expected_sha256.is_none() || cached_module.artifact_sha256.is_some(),
                }),
                execution_tier,
            );
            execution
        }
        Err(reason) => {
            execution["status"] = Value::String("failed".to_owned());
            execution["reason"] = Value::String(reason);
            execution["runtime"] = with_execution_security_tier(
                json!({
                    "executor": "wasmtime_module",
                    "artifact_path": artifact_path.display().to_string(),
                    "export": export_name,
                    "operation": command.operation,
                    "payload": command.payload,
                    "module_size_bytes": module_size_bytes,
                    "fuel_limit": runtime_policy.wasm_fuel_limit,
                    "cache_hit": cache_lookup.hit,
                    "cache_miss": !cache_lookup.hit,
                    "cache_evicted_entries": cache_lookup.evicted_entries,
                    "cache_entries": cache_lookup.cache_len,
                    "cache_capacity": cache_lookup.cache_capacity,
                    "cache_total_module_bytes": cache_lookup.cache_total_module_bytes,
                    "cache_max_bytes": cache_lookup.cache_max_bytes,
                    "cache_inserted": cache_lookup.inserted,
                    "expected_sha256": expected_sha256,
                    "artifact_sha256": cached_module.artifact_sha256.clone(),
                    "integrity_check_required": expected_sha256.is_some(),
                    "integrity_check_passed": expected_sha256.is_none() || cached_module.artifact_sha256.is_some(),
                }),
                execution_tier,
            );
            execution
        }
    }
}

pub fn resolve_wasm_component_artifact_path(
    provider: &kernel::ProviderConfig,
    channel_endpoint: &str,
) -> Result<PathBuf, String> {
    let raw = provider
        .metadata
        .get("component_resolved_path")
        .cloned()
        .or_else(|| provider.metadata.get("component_path").cloned())
        .or_else(|| provider.metadata.get("component").cloned())
        .or_else(|| {
            let endpoint = channel_endpoint.trim();
            endpoint
                .to_ascii_lowercase()
                .ends_with(".wasm")
                .then(|| endpoint.to_owned())
        })
        .ok_or_else(|| "wasm_component execution requires component artifact path".to_owned())?;

    if raw.starts_with("http://") || raw.starts_with("https://") {
        return Err(
            "wasm_component execution requires a local artifact path, remote URL is not allowed"
                .to_owned(),
        );
    }

    let candidate = PathBuf::from(&raw);
    let resolved = if candidate.is_absolute() {
        candidate
    } else if let Some(source_path) = provider.metadata.get("plugin_source_path") {
        resolve_plugin_relative_path(source_path, &raw)
    } else {
        candidate
    };

    Ok(normalize_path_for_policy(&resolved))
}

pub fn resolve_wasm_export_name(provider: &kernel::ProviderConfig) -> String {
    let raw = provider
        .metadata
        .get("entrypoint")
        .or_else(|| provider.metadata.get("entrypoint_hint"))
        .cloned()
        .unwrap_or_else(|| "run".to_owned());
    raw.split([':', '/'])
        .rfind(|segment| !segment.trim().is_empty())
        .unwrap_or("run")
        .to_owned()
}

pub fn parse_process_args(provider: &kernel::ProviderConfig) -> Vec<String> {
    if let Some(args_json) = provider.metadata.get("args_json")
        && let Ok(args) = serde_json::from_str::<Vec<String>>(args_json)
    {
        return args;
    }

    provider
        .metadata
        .get("args")
        .map(|value| value.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

pub fn provider_allowed_callers(provider: &kernel::ProviderConfig) -> BTreeSet<String> {
    let mut allowed = BTreeSet::new();

    if let Some(raw_json) = provider.metadata.get("allowed_callers_json")
        && let Ok(values) = serde_json::from_str::<Vec<String>>(raw_json)
    {
        for value in values {
            let normalized = value.trim().to_ascii_lowercase();
            if !normalized.is_empty() {
                allowed.insert(normalized);
            }
        }
    }

    if let Some(raw_list) = provider.metadata.get("allowed_callers") {
        for token in raw_list.split([',', ';', ' ']) {
            let normalized = token.trim().to_ascii_lowercase();
            if !normalized.is_empty() {
                allowed.insert(normalized);
            }
        }
    }

    allowed
}

pub fn caller_from_payload(payload: &Value) -> Option<String> {
    payload
        .get("_loongclaw")
        .and_then(Value::as_object)
        .and_then(|meta| meta.get("caller"))
        .and_then(Value::as_str)
        .map(|caller| caller.trim().to_ascii_lowercase())
        .filter(|caller| !caller.is_empty())
}

pub fn caller_is_allowed(caller: Option<&str>, allowed: &BTreeSet<String>) -> bool {
    if allowed.is_empty() {
        return true;
    }
    if allowed.contains("*") {
        return true;
    }
    caller
        .map(|value| value.trim().to_ascii_lowercase())
        .is_some_and(|value| allowed.contains(&value))
}

pub fn is_process_command_allowed(program: &str, allowed: &BTreeSet<String>) -> bool {
    if allowed.is_empty() {
        return false;
    }

    let normalized = program.trim().to_ascii_lowercase();
    if allowed.contains(&normalized) {
        return true;
    }

    Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| allowed.contains(&name.to_ascii_lowercase()))
        .unwrap_or(false)
}

pub fn detect_provider_bridge_kind(
    provider: &kernel::ProviderConfig,
    endpoint: &str,
) -> PluginBridgeKind {
    if let Some(raw) = provider.metadata.get("bridge_kind")
        && let Some(kind) = parse_bridge_kind_label(raw)
    {
        return kind;
    }

    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        return PluginBridgeKind::HttpJson;
    }

    PluginBridgeKind::Unknown
}

pub fn parse_bridge_kind_label(raw: &str) -> Option<PluginBridgeKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "http_json" | "http" => Some(PluginBridgeKind::HttpJson),
        "process_stdio" | "stdio" => Some(PluginBridgeKind::ProcessStdio),
        "native_ffi" | "ffi" => Some(PluginBridgeKind::NativeFfi),
        "wasm_component" | "wasm" => Some(PluginBridgeKind::WasmComponent),
        "mcp_server" | "mcp" => Some(PluginBridgeKind::McpServer),
        "acp_bridge" | "acp" => Some(PluginBridgeKind::AcpBridge),
        "acp_runtime" | "acpx" => Some(PluginBridgeKind::AcpRuntime),
        "unknown" => Some(PluginBridgeKind::Unknown),
        _ => None,
    }
}

pub fn default_bridge_adapter_family(bridge_kind: PluginBridgeKind) -> String {
    match bridge_kind {
        PluginBridgeKind::HttpJson => "http-adapter".to_owned(),
        PluginBridgeKind::ProcessStdio => "stdio-adapter".to_owned(),
        PluginBridgeKind::NativeFfi => "ffi-adapter".to_owned(),
        PluginBridgeKind::WasmComponent => "wasm-component-adapter".to_owned(),
        PluginBridgeKind::McpServer => "mcp-adapter".to_owned(),
        PluginBridgeKind::AcpBridge => "acp-bridge-adapter".to_owned(),
        PluginBridgeKind::AcpRuntime => "acp-runtime-adapter".to_owned(),
        PluginBridgeKind::Unknown => "unknown-adapter".to_owned(),
    }
}

pub fn default_bridge_entrypoint(bridge_kind: PluginBridgeKind, endpoint: &str) -> String {
    match bridge_kind {
        PluginBridgeKind::HttpJson => endpoint.to_owned(),
        PluginBridgeKind::ProcessStdio => "stdin/stdout::invoke".to_owned(),
        PluginBridgeKind::NativeFfi => "lib::invoke".to_owned(),
        PluginBridgeKind::WasmComponent => "component::run".to_owned(),
        PluginBridgeKind::McpServer => "mcp::stdio".to_owned(),
        PluginBridgeKind::AcpBridge => "acp::bridge".to_owned(),
        PluginBridgeKind::AcpRuntime => "acp::turn".to_owned(),
        PluginBridgeKind::Unknown => "unknown::invoke".to_owned(),
    }
}

#[cfg(test)]
mod bridge_kind_tests {
    use super::*;

    #[test]
    fn parse_bridge_kind_label_distinguishes_acp_bridge_and_runtime() {
        assert_eq!(
            parse_bridge_kind_label("acp"),
            Some(PluginBridgeKind::AcpBridge)
        );
        assert_eq!(
            parse_bridge_kind_label("acp_bridge"),
            Some(PluginBridgeKind::AcpBridge)
        );
        assert_eq!(
            parse_bridge_kind_label("acpx"),
            Some(PluginBridgeKind::AcpRuntime)
        );
        assert_eq!(
            parse_bridge_kind_label("acp_runtime"),
            Some(PluginBridgeKind::AcpRuntime)
        );
    }

    #[test]
    fn default_bridge_defaults_keep_acp_surfaces_distinct() {
        assert_eq!(
            default_bridge_adapter_family(PluginBridgeKind::AcpBridge),
            "acp-bridge-adapter"
        );
        assert_eq!(
            default_bridge_adapter_family(PluginBridgeKind::AcpRuntime),
            "acp-runtime-adapter"
        );
        assert_eq!(
            default_bridge_entrypoint(PluginBridgeKind::AcpBridge, "https://example.test"),
            "acp::bridge"
        );
        assert_eq!(
            default_bridge_entrypoint(PluginBridgeKind::AcpRuntime, "https://example.test"),
            "acp::turn"
        );
    }
}

pub struct CrmCoreConnector;

#[async_trait]
impl CoreConnectorAdapter for CrmCoreConnector {
    fn name(&self) -> &str {
        "http-core"
    }

    async fn invoke_core(
        &self,
        command: ConnectorCommand,
    ) -> Result<ConnectorOutcome, ConnectorError> {
        Ok(ConnectorOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "tier": "core",
                "adapter": "http-core",
                "connector": command.connector_name,
                "operation": command.operation,
                "payload": command.payload,
            }),
        })
    }
}

pub struct CrmGrpcCoreConnector;

#[async_trait]
impl CoreConnectorAdapter for CrmGrpcCoreConnector {
    fn name(&self) -> &str {
        "grpc-core"
    }

    async fn invoke_core(
        &self,
        command: ConnectorCommand,
    ) -> Result<ConnectorOutcome, ConnectorError> {
        Ok(ConnectorOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "tier": "core",
                "adapter": "grpc-core",
                "connector": command.connector_name,
                "operation": command.operation,
                "payload": command.payload,
            }),
        })
    }
}

pub struct ShieldedConnectorExtension;

#[async_trait]
impl kernel::ConnectorExtensionAdapter for ShieldedConnectorExtension {
    fn name(&self) -> &str {
        "shielded-bridge"
    }

    async fn invoke_extension(
        &self,
        command: ConnectorCommand,
        core: &(dyn CoreConnectorAdapter + Sync),
    ) -> Result<ConnectorOutcome, ConnectorError> {
        let probe = core
            .invoke_core(ConnectorCommand {
                connector_name: command.connector_name.clone(),
                operation: "probe".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            })
            .await?;
        Ok(ConnectorOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "tier": "extension",
                "extension": "shielded-bridge",
                "operation": command.operation,
                "core_probe": probe.payload,
                "payload": command.payload,
            }),
        })
    }
}

pub struct NativeCoreRuntime;

#[async_trait]
impl CoreRuntimeAdapter for NativeCoreRuntime {
    fn name(&self) -> &str {
        "native-core"
    }

    async fn execute_core(
        &self,
        request: RuntimeCoreRequest,
    ) -> Result<RuntimeCoreOutcome, kernel::RuntimePlaneError> {
        Ok(RuntimeCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "native-core",
                "action": request.action,
                "payload": request.payload,
            }),
        })
    }
}

pub struct FallbackCoreRuntime;

#[async_trait]
impl CoreRuntimeAdapter for FallbackCoreRuntime {
    fn name(&self) -> &str {
        "fallback-core"
    }

    async fn execute_core(
        &self,
        request: RuntimeCoreRequest,
    ) -> Result<RuntimeCoreOutcome, kernel::RuntimePlaneError> {
        Ok(RuntimeCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "fallback-core",
                "action": request.action,
                "payload": request.payload,
            }),
        })
    }
}

pub struct AcpBridgeRuntimeExtension;

#[async_trait]
impl RuntimeExtensionAdapter for AcpBridgeRuntimeExtension {
    fn name(&self) -> &str {
        "acp-bridge"
    }

    async fn execute_extension(
        &self,
        request: RuntimeExtensionRequest,
        core: &(dyn CoreRuntimeAdapter + Sync),
    ) -> Result<RuntimeExtensionOutcome, kernel::RuntimePlaneError> {
        let core_probe = core
            .execute_core(RuntimeCoreRequest {
                action: "probe".to_owned(),
                payload: json!({}),
            })
            .await?;
        Ok(RuntimeExtensionOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "extension": "acp-bridge",
                "action": request.action,
                "core_probe": core_probe.payload,
                "payload": request.payload,
            }),
        })
    }
}

// Local stubs — spec adapters don't execute real tools/memory
fn stub_tool_core(request: ToolCoreRequest) -> Result<ToolCoreOutcome, String> {
    Ok(ToolCoreOutcome {
        status: "ok".to_string(),
        payload: json!({ "adapter": "core-tools", "tool": request.tool_name }),
    })
}

fn maybe_execute_native_tool(
    request: &ToolCoreRequest,
    native_tool_executor: Option<crate::NativeToolExecutor>,
) -> Option<Result<ToolCoreOutcome, String>> {
    if let Some(executor) = native_tool_executor
        && let Some(result) = executor(request.clone())
    {
        return Some(result);
    }
    if crate::tool_name_requires_native_tool_executor(request.tool_name.as_str()) {
        return Some(Err(format!(
            "native tool executor required for tool `{}`",
            request.tool_name
        )));
    }
    None
}

fn stub_memory_core(request: MemoryCoreRequest) -> Result<MemoryCoreOutcome, String> {
    Ok(MemoryCoreOutcome {
        status: "ok".to_string(),
        payload: json!({ "adapter": "kv-core", "operation": request.operation }),
    })
}

#[derive(Clone, Copy, Default)]
pub struct CoreToolRuntime {
    native_tool_executor: Option<crate::NativeToolExecutor>,
}

impl CoreToolRuntime {
    pub const fn new(native_tool_executor: Option<crate::NativeToolExecutor>) -> Self {
        Self {
            native_tool_executor,
        }
    }
}

#[async_trait]
impl CoreToolAdapter for CoreToolRuntime {
    fn name(&self) -> &str {
        "core-tools"
    }

    async fn execute_core_tool(
        &self,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, kernel::ToolPlaneError> {
        if let Some(result) = maybe_execute_native_tool(&request, self.native_tool_executor) {
            return result.map_err(kernel::ToolPlaneError::Execution);
        }
        stub_tool_core(request).map_err(kernel::ToolPlaneError::Execution)
    }
}

pub struct SqlAnalyticsToolExtension;

#[async_trait]
impl ToolExtensionAdapter for SqlAnalyticsToolExtension {
    fn name(&self) -> &str {
        "sql-analytics"
    }

    async fn execute_tool_extension(
        &self,
        request: ToolExtensionRequest,
        core: &(dyn CoreToolAdapter + Sync),
    ) -> Result<ToolExtensionOutcome, kernel::ToolPlaneError> {
        let core_probe = core
            .execute_core_tool(ToolCoreRequest {
                tool_name: "schema_probe".to_owned(),
                payload: json!({}),
            })
            .await?;
        Ok(ToolExtensionOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "extension": "sql-analytics",
                "action": request.extension_action,
                "core_probe": core_probe.payload,
                "payload": request.payload,
            }),
        })
    }
}

pub struct ClawMigrationToolExtension;

#[async_trait]
impl ToolExtensionAdapter for ClawMigrationToolExtension {
    fn name(&self) -> &str {
        "claw-migration"
    }

    async fn execute_tool_extension(
        &self,
        request: ToolExtensionRequest,
        core: &(dyn CoreToolAdapter + Sync),
    ) -> Result<ToolExtensionOutcome, kernel::ToolPlaneError> {
        let mut payload = request.payload.clone();
        if payload.get("mode").is_none()
            && let Some(object) = payload.as_object_mut()
        {
            object.insert(
                "mode".to_owned(),
                Value::String(request.extension_action.clone()),
            );
        }

        let core_outcome = core
            .execute_core_tool(ToolCoreRequest {
                tool_name: "claw.migrate".to_owned(),
                payload,
            })
            .await?;
        let mut response = serde_json::Map::new();
        response.insert(
            "extension".to_owned(),
            Value::String("claw-migration".to_owned()),
        );
        response.insert(
            "action".to_owned(),
            Value::String(request.extension_action.clone()),
        );
        response.insert("core_outcome".to_owned(), core_outcome.payload.clone());
        if let Some(core_object) = core_outcome.payload.as_object() {
            for (key, value) in core_object {
                response.entry(key.clone()).or_insert_with(|| value.clone());
            }
        } else {
            response.insert("result".to_owned(), core_outcome.payload);
        }
        Ok(ToolExtensionOutcome {
            status: "ok".to_owned(),
            payload: Value::Object(response),
        })
    }
}

pub struct KvCoreMemory;

#[async_trait]
impl CoreMemoryAdapter for KvCoreMemory {
    fn name(&self) -> &str {
        "kv-core"
    }

    async fn execute_core_memory(
        &self,
        request: MemoryCoreRequest,
    ) -> Result<MemoryCoreOutcome, kernel::MemoryPlaneError> {
        stub_memory_core(request).map_err(kernel::MemoryPlaneError::Execution)
    }
}

pub struct VectorIndexMemoryExtension;

#[async_trait]
impl MemoryExtensionAdapter for VectorIndexMemoryExtension {
    fn name(&self) -> &str {
        "vector-index"
    }

    async fn execute_memory_extension(
        &self,
        request: MemoryExtensionRequest,
        core: &(dyn CoreMemoryAdapter + Sync),
    ) -> Result<MemoryExtensionOutcome, kernel::MemoryPlaneError> {
        let core_probe = core
            .execute_core_memory(MemoryCoreRequest {
                operation: "probe".to_owned(),
                payload: json!({}),
            })
            .await?;
        Ok(MemoryExtensionOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "extension": "vector-index",
                "operation": request.operation,
                "core_probe": core_probe.payload,
                "payload": request.payload,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        path::Path,
        sync::{Arc, Mutex},
        time::Duration,
    };
    #[cfg(unix)]
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[cfg(unix)]
    use super::wasm_artifact_file_identity;
    use super::wasm_runtime_policy::{
        DEFAULT_WASM_MODULE_CACHE_CAPACITY, DEFAULT_WASM_MODULE_CACHE_MAX_BYTES,
        MAX_WASM_MODULE_CACHE_CAPACITY, MAX_WASM_MODULE_CACHE_MAX_BYTES,
        MIN_WASM_MODULE_CACHE_MAX_BYTES, default_wasm_signals_based_traps,
        parse_wasm_module_cache_capacity, parse_wasm_module_cache_max_bytes,
        parse_wasm_signals_based_traps,
    };
    use super::{
        BridgeRuntimePolicy, ConnectorCircuitBreakerPolicy, ConnectorProtocolContext,
        CoreToolRuntime, DynamicCatalogConnector,
        PLUGIN_ACTIVATION_RUNTIME_CONTRACT_CHECKSUM_METADATA_KEY,
        PLUGIN_ACTIVATION_RUNTIME_CONTRACT_METADATA_KEY, PluginActivationRuntimeContract,
        PluginRuntimeHealthResult, WasmModuleCache, activation_runtime_contract_checksum_hex,
        build_wasm_module_cache_key, compile_wasm_module, normalize_sha256_pin,
        plugin_activation_runtime_contract_json, process_stdio_runtime_evidence,
        provider_plugin_runtime_health_result, resolve_expected_wasm_sha256,
    };
    use kernel::{
        BridgeSupportMatrix, CoreConnectorAdapter, CoreToolAdapter, IntegrationCatalog,
        PluginBridgeKind, PluginCompatibilityMode, PluginCompatibilityShim,
        PluginCompatibilityShimSupport, PluginContractDialect, PluginSourceKind, ToolCoreOutcome,
        ToolCoreRequest,
    };
    use loongclaw_protocol::PROTOCOL_VERSION;
    use serde_json::{Value, json};
    use tokio::time::sleep;

    const EMPTY_WASM_MODULE: [u8; 8] = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];

    #[test]
    fn parse_wasm_module_cache_capacity_defaults_for_missing_or_invalid_values() {
        assert_eq!(
            parse_wasm_module_cache_capacity(None),
            DEFAULT_WASM_MODULE_CACHE_CAPACITY
        );
        assert_eq!(
            parse_wasm_module_cache_capacity(Some("")),
            DEFAULT_WASM_MODULE_CACHE_CAPACITY
        );
        assert_eq!(
            parse_wasm_module_cache_capacity(Some("invalid")),
            DEFAULT_WASM_MODULE_CACHE_CAPACITY
        );
        assert_eq!(
            parse_wasm_module_cache_capacity(Some("0")),
            DEFAULT_WASM_MODULE_CACHE_CAPACITY
        );
    }

    #[test]
    fn parse_wasm_module_cache_capacity_respects_positive_values_and_upper_bound() {
        assert_eq!(parse_wasm_module_cache_capacity(Some("1")), 1);
        assert_eq!(parse_wasm_module_cache_capacity(Some("128")), 128);

        let over_limit = format!("{}", MAX_WASM_MODULE_CACHE_CAPACITY + 1);
        assert_eq!(
            parse_wasm_module_cache_capacity(Some(over_limit.as_str())),
            MAX_WASM_MODULE_CACHE_CAPACITY
        );
    }

    #[test]
    fn parse_wasm_module_cache_max_bytes_defaults_for_missing_or_invalid_values() {
        assert_eq!(
            parse_wasm_module_cache_max_bytes(None),
            DEFAULT_WASM_MODULE_CACHE_MAX_BYTES
        );
        assert_eq!(
            parse_wasm_module_cache_max_bytes(Some("")),
            DEFAULT_WASM_MODULE_CACHE_MAX_BYTES
        );
        assert_eq!(
            parse_wasm_module_cache_max_bytes(Some("invalid")),
            DEFAULT_WASM_MODULE_CACHE_MAX_BYTES
        );
        assert_eq!(
            parse_wasm_module_cache_max_bytes(Some("0")),
            DEFAULT_WASM_MODULE_CACHE_MAX_BYTES
        );
    }

    #[test]
    fn parse_wasm_module_cache_max_bytes_respects_bounds() {
        assert_eq!(
            parse_wasm_module_cache_max_bytes(Some("1")),
            MIN_WASM_MODULE_CACHE_MAX_BYTES
        );
        assert_eq!(
            parse_wasm_module_cache_max_bytes(Some("1048576")),
            1_048_576
        );

        let over_limit = format!("{}", MAX_WASM_MODULE_CACHE_MAX_BYTES + 1);
        assert_eq!(
            parse_wasm_module_cache_max_bytes(Some(over_limit.as_str())),
            MAX_WASM_MODULE_CACHE_MAX_BYTES
        );
    }

    #[test]
    fn parse_wasm_signals_based_traps_defaults_to_platform_policy() {
        assert_eq!(
            parse_wasm_signals_based_traps(None),
            default_wasm_signals_based_traps()
        );
        assert_eq!(
            parse_wasm_signals_based_traps(Some("")),
            default_wasm_signals_based_traps()
        );
        assert_eq!(
            parse_wasm_signals_based_traps(Some("invalid-value")),
            default_wasm_signals_based_traps()
        );
    }

    #[test]
    fn parse_wasm_signals_based_traps_accepts_boolean_aliases() {
        for raw in ["1", "true", "yes", "on", "enabled", "TRUE", " On "] {
            assert!(
                parse_wasm_signals_based_traps(Some(raw)),
                "expected true for {raw}"
            );
        }
        for raw in ["0", "false", "no", "off", "disabled", "FALSE", " Off "] {
            assert!(
                !parse_wasm_signals_based_traps(Some(raw)),
                "expected false for {raw}"
            );
        }
    }

    #[test]
    fn normalize_sha256_pin_accepts_plain_or_prefixed_hex() {
        let expected = "ab".repeat(32);
        assert_eq!(
            normalize_sha256_pin(expected.as_str()).expect("plain digest should parse"),
            expected
        );
        assert_eq!(
            normalize_sha256_pin(format!("sha256:{expected}").as_str())
                .expect("prefixed digest should parse"),
            expected
        );
        assert_eq!(
            normalize_sha256_pin(format!("  SHA256:{expected}  ").as_str())
                .expect("prefix should be case-insensitive"),
            expected
        );
    }

    #[test]
    fn normalize_sha256_pin_rejects_invalid_values() {
        assert!(normalize_sha256_pin("").is_err());
        assert!(normalize_sha256_pin("sha256:").is_err());
        assert!(normalize_sha256_pin("deadbeef").is_err());
        assert!(normalize_sha256_pin(&"z".repeat(64)).is_err());
    }

    fn provider_with_metadata(metadata: BTreeMap<String, String>) -> kernel::ProviderConfig {
        kernel::ProviderConfig {
            provider_id: "provider-x".to_owned(),
            connector_name: "connector-x".to_owned(),
            version: "1.0.0".to_owned(),
            metadata,
        }
    }

    fn openclaw_process_stdio_provider_with_command(
        command: &str,
        args: Vec<String>,
    ) -> kernel::ProviderConfig {
        let mut metadata = openclaw_process_stdio_metadata();

        metadata.insert("command".to_owned(), command.to_owned());
        if !args.is_empty() {
            let args_json = serde_json::to_string(&args).expect("encode process args");
            metadata.insert("args_json".to_owned(), args_json);
        }

        provider_with_metadata(metadata)
    }

    fn process_stdio_channel_for_provider(
        provider: &kernel::ProviderConfig,
    ) -> kernel::ChannelConfig {
        kernel::ChannelConfig {
            channel_id: "channel-process".to_owned(),
            provider_id: provider.provider_id.clone(),
            endpoint: "stdio://connector".to_owned(),
            enabled: true,
            metadata: BTreeMap::new(),
        }
    }

    fn process_stdio_request_id(
        provider: &kernel::ProviderConfig,
        channel: &kernel::ChannelConfig,
        operation: &str,
    ) -> String {
        format!(
            "{}:{}:{operation}",
            provider.provider_id, channel.channel_id
        )
    }

    fn provider_runtime_health_from_catalog(
        catalog: &Arc<Mutex<IntegrationCatalog>>,
        provider_id: &str,
    ) -> PluginRuntimeHealthResult {
        let guard = catalog.lock().expect("catalog mutex poisoned");
        let provider = guard
            .provider(provider_id)
            .expect("provider should exist in catalog");
        let health = provider_plugin_runtime_health_result(&provider.metadata);

        health.expect("provider metadata should carry runtime health")
    }

    fn process_stdio_success_args(request_id: &str) -> Vec<String> {
        let response = json!({
            "method": "tools/call",
            "id": request_id,
            "payload": {
                "ok": true,
            },
            "version": PROTOCOL_VERSION,
        });
        let response_text = response.to_string();
        let script = format!("IFS= read -r _line; printf '%s\\n' '{response_text}'");

        vec!["-c".to_owned(), script]
    }

    fn openclaw_attested_runtime_contract() -> PluginActivationRuntimeContract {
        PluginActivationRuntimeContract {
            plugin_id: "weather-sdk".to_owned(),
            source_path: "/tmp/weather-sdk/dist/index.js".to_owned(),
            source_kind: PluginSourceKind::PackageManifest,
            dialect: PluginContractDialect::OpenClawModernManifest,
            dialect_version: Some("openclaw.plugin.json".to_owned()),
            compatibility_mode: PluginCompatibilityMode::OpenClawModern,
            compatibility_shim: Some(PluginCompatibilityShim {
                shim_id: "openclaw-modern-compat".to_owned(),
                family: "openclaw-modern-compat".to_owned(),
            }),
            bridge_kind: PluginBridgeKind::ProcessStdio,
            adapter_family: "openclaw-modern-compat".to_owned(),
            entrypoint_hint: "stdin/stdout::invoke".to_owned(),
            source_language: "javascript".to_owned(),
            compatibility: None,
        }
    }

    fn openclaw_attested_runtime_contract_json() -> String {
        plugin_activation_runtime_contract_json(&openclaw_attested_runtime_contract())
            .expect("encode activation contract")
    }

    fn openclaw_attested_runtime_contract_checksum() -> String {
        activation_runtime_contract_checksum_hex(
            openclaw_attested_runtime_contract_json().as_bytes(),
        )
    }

    fn openclaw_process_stdio_metadata() -> BTreeMap<String, String> {
        let mut metadata = BTreeMap::from([
            ("plugin_id".to_owned(), "weather-sdk".to_owned()),
            (
                "plugin_source_path".to_owned(),
                "/tmp/weather-sdk/dist/index.js".to_owned(),
            ),
            (
                "plugin_source_kind".to_owned(),
                "package_manifest".to_owned(),
            ),
            (
                "plugin_dialect".to_owned(),
                "openclaw_modern_manifest".to_owned(),
            ),
            (
                "plugin_dialect_version".to_owned(),
                "openclaw.plugin.json".to_owned(),
            ),
            (
                "plugin_compatibility_mode".to_owned(),
                "openclaw_modern".to_owned(),
            ),
            (
                "plugin_compatibility_shim_id".to_owned(),
                "openclaw-modern-compat".to_owned(),
            ),
            (
                "plugin_compatibility_shim_family".to_owned(),
                "openclaw-modern-compat".to_owned(),
            ),
            ("bridge_kind".to_owned(), "process_stdio".to_owned()),
            (
                "adapter_family".to_owned(),
                "openclaw-modern-compat".to_owned(),
            ),
            ("source_language".to_owned(), "javascript".to_owned()),
        ]);
        let raw_contract = openclaw_attested_runtime_contract_json();
        metadata.insert(
            PLUGIN_ACTIVATION_RUNTIME_CONTRACT_METADATA_KEY.to_owned(),
            raw_contract.clone(),
        );
        metadata.insert(
            PLUGIN_ACTIVATION_RUNTIME_CONTRACT_CHECKSUM_METADATA_KEY.to_owned(),
            activation_runtime_contract_checksum_hex(raw_contract.as_bytes()),
        );
        metadata
    }

    fn openclaw_runtime_matrix(supported_source_languages: &[&str]) -> BridgeSupportMatrix {
        let profile = PluginCompatibilityShimSupport {
            shim: PluginCompatibilityShim {
                shim_id: "openclaw-modern-compat".to_owned(),
                family: "openclaw-modern-compat".to_owned(),
            },
            version: Some("openclaw-modern@1".to_owned()),
            supported_dialects: BTreeSet::from([PluginContractDialect::OpenClawModernManifest]),
            supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
            supported_adapter_families: BTreeSet::new(),
            supported_source_languages: supported_source_languages
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
        .normalized();

        BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
            supported_adapter_families: BTreeSet::new(),
            supported_compatibility_modes: BTreeSet::from([
                PluginCompatibilityMode::Native,
                PluginCompatibilityMode::OpenClawModern,
            ]),
            supported_compatibility_shims: BTreeSet::from([profile.shim.clone()]),
            supported_compatibility_shim_profiles: BTreeMap::from([(
                profile.shim.clone(),
                profile,
            )]),
        }
    }

    #[tokio::test]
    async fn bridge_execution_payload_surfaces_plugin_compatibility_context() {
        let provider = provider_with_metadata(openclaw_process_stdio_metadata());
        let channel = kernel::ChannelConfig {
            channel_id: "channel-compat".to_owned(),
            provider_id: provider.provider_id.clone(),
            endpoint: "stdio://compat".to_owned(),
            enabled: true,
            metadata: BTreeMap::new(),
        };
        let command = kernel::ConnectorCommand {
            connector_name: provider.connector_name.clone(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({"city": "shanghai"}),
        };
        let runtime_policy = BridgeRuntimePolicy {
            compatibility_matrix: openclaw_runtime_matrix(&["javascript"]),
            ..BridgeRuntimePolicy::default()
        };

        let execution =
            super::bridge_execution_payload(&provider, &channel, &command, &runtime_policy).await;

        assert_eq!(execution["status"], json!("planned"));
        assert_eq!(
            execution["plugin_compatibility"]["dialect"],
            json!("openclaw_modern_manifest")
        );
        assert_eq!(
            execution["plugin_compatibility"]["dialect_version"],
            json!("openclaw.plugin.json")
        );
        assert_eq!(
            execution["plugin_compatibility"]["mode"],
            json!("openclaw_modern")
        );
        assert_eq!(
            execution["plugin_compatibility"]["shim"]["shim_id"],
            json!("openclaw-modern-compat")
        );
        assert_eq!(
            execution["plugin_compatibility"]["shim"]["family"],
            json!("openclaw-modern-compat")
        );
        assert_eq!(
            execution["plugin_compatibility"]["shim_support"]["version"],
            json!("openclaw-modern@1")
        );
        assert_eq!(
            execution["plugin_compatibility"]["shim_support"]["supported_dialects"][0],
            json!("openclaw_modern_manifest")
        );
        assert_eq!(
            execution["plugin_compatibility"]["runtime_projection"]["source_language"],
            json!("javascript")
        );
        assert_eq!(
            execution["plugin_compatibility"]["runtime_guard"]["status"],
            json!("passed")
        );
        assert_eq!(
            execution["plugin_compatibility"]["activation_contract"]["source_kind"],
            json!("package_manifest")
        );
        assert_eq!(
            execution["plugin_compatibility"]["activation_contract_checksum"],
            json!(openclaw_attested_runtime_contract_checksum())
        );
        assert_eq!(
            execution["plugin_compatibility"]["runtime_guard"]["activation_contract_verified"],
            json!(true)
        );
        assert_eq!(
            execution["plugin_compatibility"]["runtime_guard"]["activation_contract_integrity"],
            json!("verified")
        );
    }

    #[tokio::test]
    async fn bridge_execution_payload_blocks_when_compatibility_shim_profile_drifts_at_runtime() {
        let provider = provider_with_metadata(openclaw_process_stdio_metadata());
        let channel = kernel::ChannelConfig {
            channel_id: "channel-compat".to_owned(),
            provider_id: provider.provider_id.clone(),
            endpoint: "stdio://compat".to_owned(),
            enabled: true,
            metadata: BTreeMap::new(),
        };
        let command = kernel::ConnectorCommand {
            connector_name: provider.connector_name.clone(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({"city": "shanghai"}),
        };
        let runtime_policy = BridgeRuntimePolicy {
            compatibility_matrix: openclaw_runtime_matrix(&["python"]),
            ..BridgeRuntimePolicy::default()
        };

        let execution =
            super::bridge_execution_payload(&provider, &channel, &command, &runtime_policy).await;

        assert_eq!(execution["status"], json!("blocked"));
        assert_eq!(execution["block_class"], json!("compatibility_contract"));
        assert!(
            execution["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("source language `javascript`"))
        );
        assert_eq!(
            execution["plugin_compatibility"]["runtime_guard"]["status"],
            json!("blocked")
        );
        assert_eq!(
            execution["plugin_compatibility"]["shim_support_mismatch_reasons"][0],
            json!("source language `javascript`")
        );
        assert_eq!(
            execution["plugin_compatibility"]["runtime_guard"]["activation_contract_attested"],
            json!(true)
        );
        assert_eq!(
            execution["plugin_compatibility"]["runtime_guard"]["activation_contract_verified"],
            json!(true)
        );
    }

    #[tokio::test]
    async fn bridge_execution_payload_blocks_when_activation_contract_checksum_drifts_at_runtime() {
        let mut metadata = openclaw_process_stdio_metadata();
        metadata.insert(
            PLUGIN_ACTIVATION_RUNTIME_CONTRACT_CHECKSUM_METADATA_KEY.to_owned(),
            "deadbeefdeadbeef".to_owned(),
        );
        let provider = provider_with_metadata(metadata);
        let channel = kernel::ChannelConfig {
            channel_id: "channel-compat".to_owned(),
            provider_id: provider.provider_id.clone(),
            endpoint: "stdio://compat".to_owned(),
            enabled: true,
            metadata: BTreeMap::new(),
        };
        let command = kernel::ConnectorCommand {
            connector_name: provider.connector_name.clone(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({"city": "shanghai"}),
        };
        let runtime_policy = BridgeRuntimePolicy {
            compatibility_matrix: openclaw_runtime_matrix(&["javascript"]),
            ..BridgeRuntimePolicy::default()
        };

        let execution =
            super::bridge_execution_payload(&provider, &channel, &command, &runtime_policy).await;

        assert_eq!(execution["status"], json!("blocked"));
        assert_eq!(execution["block_class"], json!("compatibility_contract"));
        assert!(
            execution["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("checksum mismatch"))
        );
        assert_eq!(
            execution["plugin_compatibility"]["activation_contract"],
            Value::Null
        );
        assert_eq!(
            execution["plugin_compatibility"]["activation_contract_checksum"],
            json!("deadbeefdeadbeef")
        );
        assert_eq!(
            execution["plugin_compatibility"]["runtime_guard"]["activation_contract_attested"],
            json!(true)
        );
        assert_eq!(
            execution["plugin_compatibility"]["runtime_guard"]["activation_contract_verified"],
            json!(false)
        );
        assert_eq!(
            execution["plugin_compatibility"]["runtime_guard"]["activation_contract_integrity"],
            json!("invalid")
        );
        assert_eq!(
            execution["plugin_compatibility"]["runtime_guard"]["activation_contract_computed_checksum"],
            json!(openclaw_attested_runtime_contract_checksum())
        );
    }

    #[tokio::test]
    async fn dynamic_catalog_connector_rejects_compatibility_projection_drift_after_registration() {
        let mut metadata = openclaw_process_stdio_metadata();
        metadata.insert(
            "plugin_compatibility_shim_id".to_owned(),
            "openclaw-legacy-compat".to_owned(),
        );
        metadata.insert(
            "plugin_compatibility_shim_family".to_owned(),
            "openclaw-legacy-compat".to_owned(),
        );
        let provider = provider_with_metadata(metadata);
        let channel = kernel::ChannelConfig {
            channel_id: "channel-compat".to_owned(),
            provider_id: provider.provider_id.clone(),
            endpoint: "stdio://compat".to_owned(),
            enabled: true,
            metadata: BTreeMap::new(),
        };
        let mut catalog = IntegrationCatalog::new();
        catalog.upsert_provider(provider.clone());
        catalog.upsert_channel(channel.clone());
        let connector = DynamicCatalogConnector::new(
            provider.connector_name.clone(),
            provider.provider_id.clone(),
            Arc::new(Mutex::new(catalog)),
            BridgeRuntimePolicy {
                compatibility_matrix: openclaw_runtime_matrix(&["javascript"]),
                ..BridgeRuntimePolicy::default()
            },
        );

        let error = connector
            .invoke_core(kernel::ConnectorCommand {
                connector_name: provider.connector_name.clone(),
                operation: "invoke".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({"channel_id":"channel-compat","city":"shanghai"}),
            })
            .await
            .expect_err("compatibility projection drift must block execution");

        assert!(
            error
                .to_string()
                .contains("plugin activation contract drifted after registration")
        );
    }

    #[tokio::test]
    async fn dynamic_catalog_connector_circuit_breaker_isolates_repeated_bridge_failures() {
        let provider = openclaw_process_stdio_provider_with_command("false", Vec::new());
        let channel = process_stdio_channel_for_provider(&provider);
        let request_id = process_stdio_request_id(&provider, &channel, "invoke");
        let recovery_args = process_stdio_success_args(&request_id);
        let recovery_provider = openclaw_process_stdio_provider_with_command("sh", recovery_args);
        let mut catalog = IntegrationCatalog::new();

        catalog.upsert_provider(provider.clone());
        catalog.upsert_channel(channel.clone());

        let connector = DynamicCatalogConnector::new(
            provider.connector_name.clone(),
            provider.provider_id.clone(),
            Arc::new(Mutex::new(catalog)),
            BridgeRuntimePolicy {
                execute_process_stdio: true,
                allowed_process_commands: BTreeSet::from(["false".to_owned(), "sh".to_owned()]),
                bridge_circuit_breaker: ConnectorCircuitBreakerPolicy {
                    enabled: true,
                    failure_threshold: 2,
                    cooldown_ms: 25,
                    half_open_max_calls: 1,
                    success_threshold: 1,
                },
                enforce_execution_success: true,
                compatibility_matrix: openclaw_runtime_matrix(&["javascript"]),
                ..BridgeRuntimePolicy::default()
            },
        );

        let command = kernel::ConnectorCommand {
            connector_name: provider.connector_name.clone(),
            operation: "invoke".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({"channel_id": channel.channel_id}),
        };

        let first_error = connector
            .invoke_core(command.clone())
            .await
            .expect_err("first failing bridge call should error");
        let first_error_text = first_error.to_string();
        assert!(first_error_text.contains("bridge_circuit_phase_before=closed"));
        assert!(first_error_text.contains("bridge_circuit_phase_after=closed"));
        let first_health =
            provider_runtime_health_from_catalog(&connector.catalog, &provider.provider_id);
        assert_eq!(first_health.status, "degraded");
        assert_eq!(first_health.circuit_phase, "closed");
        assert_eq!(first_health.consecutive_failures, 1);
        assert!(first_health.last_failure_reason.is_some());

        let second_error = connector
            .invoke_core(command.clone())
            .await
            .expect_err("second failing bridge call should open circuit");
        let second_error_text = second_error.to_string();
        assert!(second_error_text.contains("bridge_circuit_phase_before=closed"));
        assert!(second_error_text.contains("bridge_circuit_phase_after=open"));
        let second_health =
            provider_runtime_health_from_catalog(&connector.catalog, &provider.provider_id);
        assert_eq!(second_health.status, "quarantined");
        assert_eq!(second_health.circuit_phase, "open");
        assert_eq!(second_health.consecutive_failures, 2);

        {
            let mut catalog = connector.catalog.lock().expect("catalog mutex poisoned");

            catalog.upsert_provider(recovery_provider);
        }

        let open_error = connector
            .invoke_core(command.clone())
            .await
            .expect_err("open circuit should short-circuit before re-execution");
        let open_error_text = open_error.to_string();
        assert!(open_error_text.contains("circuit-open"));
        let open_health =
            provider_runtime_health_from_catalog(&connector.catalog, &provider.provider_id);
        assert_eq!(open_health.status, "quarantined");
        assert_eq!(open_health.circuit_phase, "open");
        assert!(
            open_health
                .last_failure_reason
                .as_deref()
                .map(|reason| reason.contains("circuit-open"))
                .unwrap_or(false)
        );

        sleep(Duration::from_millis(30)).await;

        let recovered = connector
            .invoke_core(command)
            .await
            .expect("half-open recovery call should succeed");
        assert_eq!(
            recovered.payload["bridge_execution"]["status"],
            json!("executed")
        );
        assert_eq!(
            recovered.payload["bridge_execution"]["circuit_breaker"]["phase_before"],
            json!("half_open")
        );
        assert_eq!(
            recovered.payload["bridge_execution"]["circuit_breaker"]["phase_after"],
            json!("closed")
        );
        let recovered_health =
            provider_runtime_health_from_catalog(&connector.catalog, &provider.provider_id);
        assert_eq!(recovered_health.status, "healthy");
        assert_eq!(recovered_health.circuit_phase, "closed");
        assert_eq!(recovered_health.consecutive_failures, 0);
        assert!(recovered_health.last_failure_reason.is_none());
    }

    #[test]
    fn resolve_expected_wasm_sha256_rejects_conflicting_metadata_pins() {
        let provider = provider_with_metadata(BTreeMap::from([
            ("plugin_id".to_owned(), "plugin-a".to_owned()),
            ("component_sha256".to_owned(), "aa".repeat(32)),
            ("component_sha256_pin".to_owned(), "bb".repeat(32)),
        ]));
        let policy = BridgeRuntimePolicy::default();
        let error = resolve_expected_wasm_sha256(&provider, &policy)
            .expect_err("conflicting metadata pins should be rejected");
        assert!(error.contains("conflicting wasm sha256 pins"));
    }

    #[test]
    fn resolve_expected_wasm_sha256_rejects_metadata_and_policy_conflict() {
        let provider = provider_with_metadata(BTreeMap::from([
            ("plugin_id".to_owned(), "plugin-a".to_owned()),
            ("component_sha256".to_owned(), "aa".repeat(32)),
        ]));
        let mut policy = BridgeRuntimePolicy::default();
        policy
            .wasm_required_sha256_by_plugin
            .insert("plugin-a".to_owned(), "bb".repeat(32));

        let error = resolve_expected_wasm_sha256(&provider, &policy)
            .expect_err("metadata/policy conflict should be rejected");
        assert!(error.contains("between provider metadata"));
    }

    #[test]
    fn process_stdio_runtime_evidence_reports_balanced_execution_tier() {
        let provider = provider_with_metadata(BTreeMap::new());
        let channel = kernel::ChannelConfig {
            channel_id: "channel-x".to_owned(),
            endpoint: "stdio://connector".to_owned(),
            provider_id: provider.provider_id.clone(),
            enabled: true,
            metadata: BTreeMap::new(),
        };
        let command = kernel::ConnectorCommand {
            connector_name: "connector-x".to_owned(),
            operation: "call".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        };
        let mut context =
            ConnectorProtocolContext::from_connector_command(&provider, &channel, &command);
        super::authorize_connector_protocol_context(&mut context)
            .expect("protocol context should authorize");

        let runtime = process_stdio_runtime_evidence(
            &context,
            BridgeRuntimePolicy {
                execute_process_stdio: true,
                allowed_process_commands: BTreeSet::from(["demo-connector".to_owned()]),
                ..BridgeRuntimePolicy::default()
            }
            .process_stdio_execution_security_tier(),
            "demo-connector",
            &["--serve".to_owned()],
            5_000,
            super::ProcessStdioRuntimeEvidenceKind::BaseOnly,
        );

        assert_eq!(runtime["execution_tier"], json!("balanced"));
    }

    #[test]
    fn execute_wasm_component_bridge_reports_restricted_execution_tier() {
        let unique = format!(
            "loongclaw-wasm-tier-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).expect("create temp wasm root");
        let wasm_path = root.join("fixture.wasm");
        std::fs::write(&wasm_path, EMPTY_WASM_MODULE).expect("write wasm fixture");

        let provider = provider_with_metadata(BTreeMap::from([
            ("component".to_owned(), wasm_path.display().to_string()),
            ("plugin_id".to_owned(), "plugin-a".to_owned()),
        ]));
        let channel = kernel::ChannelConfig {
            channel_id: "channel-wasm".to_owned(),
            endpoint: "local://fixture".to_owned(),
            provider_id: provider.provider_id.clone(),
            enabled: true,
            metadata: BTreeMap::new(),
        };
        let command = kernel::ConnectorCommand {
            connector_name: "connector-x".to_owned(),
            operation: "call".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        };
        let runtime_policy = BridgeRuntimePolicy {
            execute_wasm_component: true,
            wasm_allowed_path_prefixes: vec![root.clone()],
            ..BridgeRuntimePolicy::default()
        };

        let execution = super::execute_wasm_component_bridge(
            json!({"status": "planned"}),
            &provider,
            &channel,
            &command,
            &runtime_policy,
        );

        assert_eq!(execution["runtime"]["execution_tier"], json!("restricted"));
        let _ = std::fs::remove_file(&wasm_path);
        let _ = std::fs::remove_dir(&root);
    }

    #[test]
    fn execute_wasm_component_bridge_reports_runtime_on_artifact_resolution_failure() {
        let provider = provider_with_metadata(BTreeMap::new());
        let channel = kernel::ChannelConfig {
            channel_id: "channel-wasm".to_owned(),
            endpoint: "local://fixture".to_owned(),
            provider_id: provider.provider_id.clone(),
            enabled: true,
            metadata: BTreeMap::new(),
        };
        let command = kernel::ConnectorCommand {
            connector_name: "connector-x".to_owned(),
            operation: "call".to_owned(),
            required_capabilities: BTreeSet::new(),
            payload: json!({}),
        };
        let runtime_policy = BridgeRuntimePolicy {
            execute_wasm_component: true,
            ..BridgeRuntimePolicy::default()
        };

        let execution = super::execute_wasm_component_bridge(
            json!({"status": "planned"}),
            &provider,
            &channel,
            &command,
            &runtime_policy,
        );

        assert_eq!(execution["status"], json!("blocked"));
        assert_eq!(
            execution["reason"],
            json!("wasm_component execution requires component artifact path")
        );
        assert_eq!(execution["runtime"]["executor"], json!("wasmtime_module"));
        assert_eq!(execution["runtime"]["execution_tier"], json!("restricted"));
    }

    #[test]
    fn wasm_module_cache_key_distinguishes_expected_sha256_pin() {
        let path = Path::new("/tmp/pin-test.wasm");
        let pin_a = "aa".repeat(32);
        let pin_b = "bb".repeat(32);
        let key_a = build_wasm_module_cache_key(path, 8, Some(1), None, Some(pin_a), false);
        let key_b = build_wasm_module_cache_key(path, 8, Some(1), None, Some(pin_b), false);
        assert_ne!(key_a, key_b);
    }

    #[test]
    fn wasm_module_cache_evicts_lru_entries_when_byte_budget_exceeded() {
        let compiled = Arc::new(
            compile_wasm_module(&EMPTY_WASM_MODULE, false, None)
                .expect("empty wasm module should compile"),
        );
        let mut cache = WasmModuleCache::default();
        let key_a =
            build_wasm_module_cache_key(Path::new("/tmp/a.wasm"), 6, Some(1), None, None, false);
        let key_b =
            build_wasm_module_cache_key(Path::new("/tmp/b.wasm"), 6, Some(2), None, None, false);

        let first = cache.insert(key_a.clone(), compiled.clone(), 6, 8, 10);
        assert!(first.inserted);
        assert_eq!(first.evicted_entries, 0);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.total_module_bytes(), 6);

        let second = cache.insert(key_b.clone(), compiled, 6, 8, 10);
        assert!(second.inserted);
        assert_eq!(second.evicted_entries, 1);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.total_module_bytes(), 6);
        assert!(cache.get(&key_a).is_none());
        assert!(cache.get(&key_b).is_some());
    }

    #[test]
    fn wasm_module_cache_skips_single_module_larger_than_byte_budget() {
        let compiled = Arc::new(
            compile_wasm_module(&EMPTY_WASM_MODULE, false, None)
                .expect("empty wasm module should compile"),
        );
        let mut cache = WasmModuleCache::default();
        let baseline =
            build_wasm_module_cache_key(Path::new("/tmp/base.wasm"), 4, Some(1), None, None, false);
        let oversized = build_wasm_module_cache_key(
            Path::new("/tmp/oversized.wasm"),
            11,
            Some(2),
            None,
            None,
            false,
        );

        let baseline_insert = cache.insert(baseline.clone(), compiled.clone(), 4, 8, 10);
        assert!(baseline_insert.inserted);

        let oversized_insert = cache.insert(oversized.clone(), compiled, 11, 8, 10);
        assert!(!oversized_insert.inserted);
        assert_eq!(oversized_insert.evicted_entries, 0);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.total_module_bytes(), 4);
        assert!(cache.get(&baseline).is_some());
        assert!(cache.get(&oversized).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn wasm_artifact_file_identity_distinguishes_different_files() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let base = std::env::temp_dir().join(format!("loongclaw-wasm-file-identity-{unique}"));
        fs::create_dir_all(&base).expect("create temp dir");
        let file_a = base.join("a.wasm");
        let file_b = base.join("b.wasm");
        fs::write(&file_a, b"(module)").expect("write file a");
        fs::write(&file_b, b"(module)").expect("write file b");

        let metadata_a = fs::metadata(&file_a).expect("metadata file a");
        let metadata_b = fs::metadata(&file_b).expect("metadata file b");
        let identity_a =
            wasm_artifact_file_identity(&metadata_a).expect("file identity for file a exists");
        let identity_b =
            wasm_artifact_file_identity(&metadata_b).expect("file identity for file b exists");

        assert_ne!(identity_a, identity_b);
        let _ = fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn core_tool_runtime_claw_migrate_without_native_executor_fails_closed() {
        let error = CoreToolRuntime::default()
            .execute_core_tool(ToolCoreRequest {
                tool_name: "claw.migrate".to_owned(),
                payload: json!({"mode": "plan"}),
            })
            .await
            .expect_err("native-only tool execution should fail without an injected executor");

        assert!(error.to_string().contains("native tool executor"));
    }

    fn test_native_tool_executor(
        request: ToolCoreRequest,
    ) -> Option<Result<ToolCoreOutcome, String>> {
        if request.tool_name != "claw.migrate" {
            return None;
        }
        Some(Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "native-tools",
                "tool": request.tool_name,
            }),
        }))
    }

    #[tokio::test]
    async fn core_tool_runtime_uses_explicit_native_executor_when_present() {
        let outcome = CoreToolRuntime::new(Some(test_native_tool_executor))
            .execute_core_tool(ToolCoreRequest {
                tool_name: "claw.migrate".to_owned(),
                payload: json!({"mode": "plan"}),
            })
            .await
            .expect("native tool execution should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["adapter"], "native-tools");
        assert_eq!(outcome.payload["tool"], "claw.migrate");
    }

    fn declining_native_tool_executor(
        request: ToolCoreRequest,
    ) -> Option<Result<ToolCoreOutcome, String>> {
        if request.tool_name == "claw.migrate" {
            return None;
        }
        Some(Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "native-tools",
                "tool": request.tool_name,
            }),
        }))
    }

    #[tokio::test]
    async fn core_tool_runtime_claw_migrate_fails_closed_when_executor_declines_request() {
        let error = CoreToolRuntime::new(Some(declining_native_tool_executor))
            .execute_core_tool(ToolCoreRequest {
                tool_name: "claw.migrate".to_owned(),
                payload: json!({"mode": "plan"}),
            })
            .await
            .expect_err("native-only tool execution should fail closed when executor declines");

        assert!(error.to_string().contains("native tool executor"));
    }
}
