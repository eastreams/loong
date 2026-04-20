use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
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
use loong_contracts::ExecutionSecurityTier;
use loong_protocol::{OutboundFrame, PROTOCOL_VERSION, ProtocolRouter, RouteAuthorizationRequest};
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
    Module as WasmtimeModule, Store as WasmtimeStore, Trap as WasmtimeTrap,
};

#[cfg(any(test, feature = "test-hooks"))]
use crate::WEBHOOK_TEST_RETRY_STATE;
use crate::{normalize_path_for_policy, resolve_plugin_relative_path};

mod adapter_stubs;
#[cfg(test)]
mod bridge_kind_tests;
mod bridge_support;
mod connector_circuit;
mod dynamic_catalog;
mod embedded_pi_harness;
mod http_json_bridge;
mod plugin_contract_types;
mod process_stdio_bridge;
mod wasm_cache;
mod wasm_execution_support;
mod wasm_host_abi;
mod wasm_runtime_policy;
#[cfg(test)]
mod wasm_runtime_tests;
pub use adapter_stubs::{
    AcpBridgeRuntimeExtension, ClawMigrationToolExtension, CoreToolRuntime, CrmCoreConnector,
    CrmGrpcCoreConnector, FallbackCoreRuntime, KvCoreMemory, NativeCoreRuntime,
    ShieldedConnectorExtension, SqlAnalyticsToolExtension, VectorIndexMemoryExtension,
    WebhookConnector,
};
pub use bridge_support::{
    caller_from_payload, caller_is_allowed, default_bridge_adapter_family,
    default_bridge_entrypoint, detect_provider_bridge_kind, is_process_command_allowed,
    parse_bridge_kind_label, parse_process_args, provider_allowed_callers,
    resolve_wasm_component_artifact_path, resolve_wasm_export_name,
};
pub use connector_circuit::{
    ConnectorCircuitAcquireError, ConnectorCircuitPhase, ConnectorCircuitRuntimeState,
    ProgrammaticCircuitPhase, ProgrammaticCircuitRuntimeState,
    acquire_connector_circuit_slot_for_state, connector_circuit_phase_label,
    connector_circuit_remaining_cooldown_ms, record_connector_circuit_outcome_for_state,
    validate_connector_circuit_breaker_policy,
};
pub use dynamic_catalog::{DynamicCatalogConnector, bridge_execution_payload};
pub(crate) use dynamic_catalog::{
    default_runtime_adapter_family, normalize_runtime_source_language,
    provider_activation_runtime_contract_state, provider_is_plugin_backed,
};
pub use embedded_pi_harness::EmbeddedPiHarness;
pub use http_json_bridge::execute_http_json_bridge;
pub use plugin_contract_types::*;
pub(crate) use plugin_contract_types::{
    activation_runtime_contract_checksum_hex, build_plugin_runtime_health_result,
    encode_plugin_runtime_health_result, invalid_plugin_runtime_health_result,
    parse_plugin_activation_runtime_contract, plugin_activation_runtime_contract_json,
    plugin_activation_runtime_contract_value,
};
pub use process_stdio_bridge::{
    ProcessStdioExchangeOutcome, execute_process_stdio_bridge, run_process_stdio_json_line_exchange,
};
#[cfg(test)]
use wasm_cache::WasmModuleCache;
use wasm_cache::{
    CachedWasmModule, WasmArtifactFileIdentity, WasmModuleCacheLookup, build_wasm_module_cache_key,
    insert_cached_wasm_module, lookup_cached_wasm_module, modified_unix_nanos,
    wasm_artifact_file_identity, wasm_module_cache_capacity, wasm_module_cache_max_bytes,
};
use wasm_execution_support::{
    WasmEntrypointSignature, WasmEpochDeadlineController, WasmRunEvidence, WasmRunOutcome,
    WasmRunResult, WasmRuntimeExecutionContext, boxed_wasm_run_failure, compile_wasm_module,
    read_wasm_artifact_bytes, wasm_artifact_sha256_hex, wasm_bridge_request_payload,
    wasm_cache_lookup_disabled, wasm_runtime_execution_evidence, wasm_runtime_failure_reason,
    wasm_snapshot_from_store,
};
pub(crate) use wasm_host_abi::{
    WASM_GUEST_CONFIG_CHANNEL_PREFIX, WASM_GUEST_CONFIG_PROVIDER_PREFIX,
    wasm_guest_config_key_is_supported,
};
use wasm_host_abi::{
    WasmHostAbiSnapshot, WasmHostAbiStoreData, build_wasm_guest_config, link_wasm_host_abi,
    module_requires_wasm_host_abi_memory, module_uses_wasm_host_abi,
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
    pub wasm_guest_readable_config_keys: BTreeSet<String>,
    pub wasm_max_component_bytes: Option<usize>,
    pub wasm_max_output_bytes: Option<usize>,
    pub wasm_fuel_limit: Option<u64>,
    pub wasm_timeout_ms: Option<u64>,
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

pub(crate) fn parse_plugin_activation_runtime_dialect(raw: &str) -> Option<PluginContractDialect> {
    match raw {
        "loong_package_manifest" | "loong_claw_package_manifest" => {
            Some(PluginContractDialect::LoongPackageManifest)
        }
        "loong_embedded_source" | "loong_claw_embedded_source" => {
            Some(PluginContractDialect::LoongEmbeddedSource)
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
    pub guest_readable_config_keys: Vec<String>,
    #[serde(default)]
    pub max_component_bytes: Option<usize>,
    #[serde(default)]
    pub max_output_bytes: Option<usize>,
    #[serde(default)]
    pub fuel_limit: Option<u64>,
    #[serde(default)]
    pub bridge_circuit_breaker: ConnectorCircuitBreakerPolicy,
    pub timeout_ms: Option<u64>,
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
    pub block_unverified_high_risk_auto_apply: Option<bool>,
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
    let timeout_ms = runtime_policy.wasm_timeout_ms;
    let epoch_interruption_enabled = timeout_ms.is_some();
    let cache_enabled = !epoch_interruption_enabled;
    let cache_capacity = wasm_module_cache_capacity();
    let cache_max_bytes = wasm_module_cache_max_bytes();
    let disabled_cache_lookup = wasm_cache_lookup_disabled(cache_capacity, cache_max_bytes);
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
                    "timeout_ms": timeout_ms,
                    "timeout_triggered": false,
                    "cache_enabled": cache_enabled,
                    "cache_hit": false,
                    "cache_miss": cache_enabled,
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

    let (cached_module, cache_lookup) = if cache_enabled {
        let initial_cache_key = build_wasm_module_cache_key(
            &artifact_path,
            module_size_bytes as u64,
            artifact_modified_unix_ns,
            artifact_file_identity,
            expected_sha256.clone(),
            fuel_enabled,
            epoch_interruption_enabled,
        );
        match lookup_cached_wasm_module(&initial_cache_key) {
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
                                "timeout_ms": timeout_ms,
                                "timeout_triggered": false,
                                "cache_enabled": cache_enabled,
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
                            "timeout_ms": timeout_ms,
                            "timeout_triggered": false,
                            "cache_enabled": cache_enabled,
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
                                "timeout_ms": timeout_ms,
                                "timeout_triggered": false,
                                "cache_enabled": cache_enabled,
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
                    epoch_interruption_enabled,
                );

                match lookup_cached_wasm_module(&refreshed_cache_key) {
                    Ok(Some(hit)) => hit,
                    Ok(None) => {
                        let compiled = match compile_wasm_module(
                            &module_bytes,
                            fuel_enabled,
                            epoch_interruption_enabled,
                            artifact_sha256,
                        ) {
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
                                        "timeout_ms": timeout_ms,
                                        "timeout_triggered": false,
                                        "cache_enabled": cache_enabled,
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
                                        "timeout_ms": timeout_ms,
                                        "timeout_triggered": false,
                                        "cache_enabled": cache_enabled,
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
                                "timeout_ms": timeout_ms,
                                "timeout_triggered": false,
                                "cache_enabled": cache_enabled,
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
                        "timeout_ms": timeout_ms,
                        "timeout_triggered": false,
                        "cache_enabled": cache_enabled,
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
    } else {
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
                        "timeout_ms": timeout_ms,
                        "timeout_triggered": false,
                        "cache_enabled": cache_enabled,
                        "cache_hit": false,
                        "cache_miss": false,
                        "cache_evicted_entries": disabled_cache_lookup.evicted_entries,
                        "cache_entries": disabled_cache_lookup.cache_len,
                        "cache_capacity": disabled_cache_lookup.cache_capacity,
                        "cache_total_module_bytes": disabled_cache_lookup.cache_total_module_bytes,
                        "cache_max_bytes": disabled_cache_lookup.cache_max_bytes,
                        "cache_inserted": disabled_cache_lookup.inserted,
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
                    "timeout_ms": timeout_ms,
                    "timeout_triggered": false,
                    "cache_enabled": cache_enabled,
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
                        "timeout_ms": timeout_ms,
                        "timeout_triggered": false,
                        "cache_enabled": cache_enabled,
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

        let compiled = match compile_wasm_module(
            &module_bytes,
            fuel_enabled,
            epoch_interruption_enabled,
            artifact_sha256,
        ) {
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
                        "timeout_ms": timeout_ms,
                        "timeout_triggered": false,
                        "cache_enabled": cache_enabled,
                        "cache_hit": false,
                        "cache_miss": false,
                        "cache_evicted_entries": disabled_cache_lookup.evicted_entries,
                        "cache_entries": disabled_cache_lookup.cache_len,
                        "cache_capacity": disabled_cache_lookup.cache_capacity,
                        "cache_total_module_bytes": disabled_cache_lookup.cache_total_module_bytes,
                        "cache_max_bytes": disabled_cache_lookup.cache_max_bytes,
                        "cache_inserted": disabled_cache_lookup.inserted,
                    }),
                    execution_tier,
                );
                return execution;
            }
        };
        (compiled, disabled_cache_lookup)
    };

    let cache_miss = if cache_enabled {
        !cache_lookup.hit
    } else {
        false
    };
    let request_payload = wasm_bridge_request_payload(provider, channel, command);
    let runtime_execution_context = WasmRuntimeExecutionContext {
        artifact_path: artifact_path.display().to_string(),
        export_name: export_name.clone(),
        operation: command.operation.clone(),
        payload: command.payload.clone(),
        request: request_payload.clone(),
        module_size_bytes,
        fuel_limit: runtime_policy.wasm_fuel_limit,
        max_output_bytes: runtime_policy.wasm_max_output_bytes,
        timeout_ms,
        cache_enabled,
        cache_lookup,
        cache_miss,
        expected_sha256,
        artifact_sha256: cached_module.artifact_sha256.clone(),
    };
    let run_result = (|| -> WasmRunResult<WasmRunOutcome> {
        let input_bytes = serde_json::to_vec(&request_payload).map_err(|error| {
            boxed_wasm_run_failure(
                format!("failed to serialize wasm bridge request payload: {error}"),
                false,
                None,
                WasmRunEvidence::default(),
            )
        })?;
        let guest_config = build_wasm_guest_config(
            provider,
            channel,
            &runtime_policy.wasm_guest_readable_config_keys,
        );
        let store_data = WasmHostAbiStoreData::try_new(
            input_bytes,
            guest_config,
            runtime_policy.wasm_max_output_bytes,
        )
        .map_err(|reason| {
            boxed_wasm_run_failure(reason, false, None, WasmRunEvidence::default())
        })?;
        let mut store = WasmtimeStore::new(&cached_module.engine, store_data);
        if let Some(limit) = runtime_policy.wasm_fuel_limit {
            store.set_fuel(limit).map_err(|error| {
                boxed_wasm_run_failure(
                    format!("failed to set wasm fuel limit: {error}"),
                    false,
                    None,
                    WasmRunEvidence::default(),
                )
            })?;
        }
        if timeout_ms.is_some() {
            store.epoch_deadline_trap();
            store.set_epoch_deadline(1);
        }
        let host_abi_enabled = module_uses_wasm_host_abi(&cached_module.module);
        let host_abi_requires_memory = module_requires_wasm_host_abi_memory(&cached_module.module);
        let mut linker = WasmtimeLinker::new(&cached_module.engine);
        link_wasm_host_abi(&mut linker).map_err(|reason| {
            boxed_wasm_run_failure(reason, false, None, WasmRunEvidence::default())
        })?;
        let timeout_controller = match timeout_ms {
            Some(timeout_ms) => Some(
                WasmEpochDeadlineController::start(&cached_module.engine, timeout_ms).map_err(
                    |reason| {
                        boxed_wasm_run_failure(reason, false, None, WasmRunEvidence::default())
                    },
                )?,
            ),
            None => None,
        };
        let instance_result = linker.instantiate(&mut store, &cached_module.module);
        let instance = match instance_result {
            Ok(instance) => instance,
            Err(error) => {
                let store_data = store.data().clone();
                let abort_code = store_data.abort_code;
                let host_abi = store_data.snapshot(host_abi_enabled);
                let evidence = WasmRunEvidence {
                    host_abi,
                    ..WasmRunEvidence::default()
                };
                let (instantiate_reason, timeout_triggered) = wasm_runtime_failure_reason(
                    &error,
                    timeout_ms,
                    "failed to instantiate wasm module",
                );
                let reason = if let Some(code) = abort_code {
                    format!("wasm guest aborted with code {code}")
                } else {
                    instantiate_reason
                };
                return Err(boxed_wasm_run_failure(
                    reason,
                    timeout_triggered,
                    None,
                    evidence,
                ));
            }
        };
        if host_abi_requires_memory && instance.get_memory(&mut store, "memory").is_none() {
            let evidence = WasmRunEvidence {
                host_abi: wasm_snapshot_from_store(&store, host_abi_enabled),
                ..WasmRunEvidence::default()
            };
            return Err(boxed_wasm_run_failure(
                "wasm host ABI requires exported memory",
                false,
                None,
                evidence,
            ));
        }
        let resolved_i32 = instance.get_typed_func::<(), i32>(&mut store, export_name.as_str());
        let entrypoint_signature = if resolved_i32.is_ok() {
            WasmEntrypointSignature::I32
        } else {
            WasmEntrypointSignature::Unit
        };
        let mut evidence = WasmRunEvidence {
            entrypoint_signature: Some(entrypoint_signature.as_str()),
            ..WasmRunEvidence::default()
        };
        let call_result = match resolved_i32 {
            Ok(func) => {
                let call_result = func.call(&mut store, ());
                match call_result {
                    Ok(code) => {
                        evidence.guest_exit_code = Some(code);
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            }
            Err(first_error) => {
                let func = instance
                    .get_typed_func::<(), ()>(&mut store, export_name.as_str())
                    .map_err(|second_error| {
                        boxed_wasm_run_failure(
                            format!(
                                "failed to resolve exported wasm function {export_name}: {first_error}; fallback to () -> () also failed: {second_error}"
                            ),
                            false,
                            None,
                            WasmRunEvidence::default(),
                        )
                    })?;
                func.call(&mut store, ())
            }
        };
        drop(timeout_controller);
        if let Err(error) = call_result {
            let store_data = store.data().clone();
            let abort_code = store_data.abort_code;
            evidence.host_abi = store_data.snapshot(host_abi_enabled);
            let (call_reason, timeout_triggered) =
                wasm_runtime_failure_reason(&error, timeout_ms, "wasm function call failed");
            let reason = if let Some(code) = abort_code {
                format!("wasm guest aborted with code {code}")
            } else {
                call_reason
            };
            return Err(boxed_wasm_run_failure(
                reason,
                timeout_triggered,
                None,
                evidence,
            ));
        }
        let consumed_fuel = runtime_policy
            .wasm_fuel_limit
            .map(|limit| {
                store
                    .get_fuel()
                    .map(|remaining| limit.saturating_sub(remaining))
            })
            .transpose()
            .map_err(|error| {
                boxed_wasm_run_failure(
                    format!("failed to query wasm fuel: {error}"),
                    false,
                    None,
                    evidence.clone(),
                )
            })?;
        let store_data = store.data().clone();
        evidence.host_abi = store_data.snapshot(host_abi_enabled);
        if let Some(reason) = store_data.output_error.clone() {
            return Err(boxed_wasm_run_failure(
                reason,
                false,
                consumed_fuel,
                evidence,
            ));
        }
        let output_json = store_data.parse_output_json().map_err(|reason| {
            boxed_wasm_run_failure(reason, false, consumed_fuel, evidence.clone())
        })?;
        evidence.host_abi.output_json = output_json;
        if let Some(code) = evidence.guest_exit_code
            && code != 0
        {
            return Err(boxed_wasm_run_failure(
                format!("wasm guest returned non-zero exit code {code}"),
                false,
                consumed_fuel,
                evidence,
            ));
        }
        Ok(WasmRunOutcome {
            consumed_fuel,
            timeout_triggered: false,
            evidence,
        })
    })();

    match run_result {
        Ok(outcome) => {
            execution["status"] = Value::String("executed".to_owned());
            execution["runtime"] = with_execution_security_tier(
                wasm_runtime_execution_evidence(
                    &runtime_execution_context,
                    outcome.timeout_triggered,
                    outcome.consumed_fuel,
                    &outcome.evidence,
                ),
                execution_tier,
            );
            execution
        }
        Err(failure) => {
            let failure = *failure;
            let failure_reason = failure.reason;
            let timeout_triggered = failure.timeout_triggered;
            execution["status"] = Value::String("failed".to_owned());
            execution["reason"] = Value::String(failure_reason);
            execution["runtime"] = with_execution_security_tier(
                wasm_runtime_execution_evidence(
                    &runtime_execution_context,
                    timeout_triggered,
                    failure.consumed_fuel,
                    &failure.evidence,
                ),
                execution_tier,
            );
            execution
        }
    }
}
