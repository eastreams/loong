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
    ArchitectureGuardReport, AuditEvent, BootstrapReport, BootstrapTaskStatus, BridgeSupportMatrix,
    CURRENT_PLUGIN_HOST_API, Capability, CodebaseAwarenessSnapshot, ConnectorCommand,
    ConnectorError, ConnectorOutcome, CoreConnectorAdapter, CoreMemoryAdapter, CoreRuntimeAdapter,
    CoreToolAdapter, ExecutionRoute, HarnessAdapter, HarnessError, HarnessKind, HarnessOutcome,
    HarnessRequest, IntegrationCatalog, IntegrationHotfix, MemoryCoreOutcome, MemoryCoreRequest,
    MemoryExtensionAdapter, MemoryExtensionOutcome, MemoryExtensionRequest, PluginAbsorbReport,
    PluginActivationPlan, PluginActivationStatus, PluginBridgeKind, PluginCompatibility,
    PluginCompatibilityMode, PluginCompatibilityShim, PluginCompatibilityShimSupport,
    PluginContractDialect, PluginDiagnosticFinding, PluginIR, PluginRuntimeProfile,
    PluginScanReport, PluginSlotClaim, PluginSourceKind, PluginTranslationReport, PluginTrustTier,
    ProvisionPlan, RuntimeCoreOutcome, RuntimeCoreRequest, RuntimeExtensionAdapter,
    RuntimeExtensionOutcome, RuntimeExtensionRequest, ToolCoreOutcome, ToolCoreRequest,
    ToolExtensionAdapter, ToolExtensionOutcome, ToolExtensionRequest, VerticalPackManifest,
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

#[cfg(test)]
mod bridge_kind_tests;
mod dynamic_catalog;
mod http_json_bridge;
mod plugin_contract_types;
mod process_stdio_bridge;
#[cfg(test)]
mod tests;
mod wasm_cache;
mod wasm_runtime_policy;
pub use dynamic_catalog::{DynamicCatalogConnector, bridge_execution_payload};
pub(crate) use dynamic_catalog::{
    default_runtime_adapter_family, normalize_runtime_source_language,
    provider_activation_runtime_contract_state, provider_is_plugin_backed,
};
pub use http_json_bridge::execute_http_json_bridge;
pub use plugin_contract_types::*;
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
        trust_tiers: Vec<PluginTrustTier>,
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
#[serde(deny_unknown_fields)]
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
pub(crate) struct ProviderActivationRuntimeContractState {
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
