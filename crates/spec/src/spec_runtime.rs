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
    ArchitectureGuardReport, AuditEvent, BootstrapReport, Capability, CodebaseAwarenessSnapshot,
    ConnectorAdapter, ConnectorCommand, ConnectorError, ConnectorOutcome, CoreConnectorAdapter,
    CoreMemoryAdapter, CoreRuntimeAdapter, CoreToolAdapter, ExecutionRoute, HarnessAdapter,
    HarnessError, HarnessKind, HarnessOutcome, HarnessRequest, IntegrationCatalog,
    IntegrationHotfix, MemoryCoreOutcome, MemoryCoreRequest, MemoryExtensionAdapter,
    MemoryExtensionOutcome, MemoryExtensionRequest, PluginAbsorbReport, PluginActivationPlan,
    PluginBridgeKind, PluginScanReport, PluginTranslationReport, ProvisionPlan, RuntimeCoreOutcome,
    RuntimeCoreRequest, RuntimeExtensionAdapter, RuntimeExtensionOutcome, RuntimeExtensionRequest,
    ToolCoreOutcome, ToolCoreRequest, ToolExtensionAdapter, ToolExtensionOutcome,
    ToolExtensionRequest, VerticalPackManifest,
};
use loongclaw_protocol::{OutboundFrame, ProtocolRouter, RouteAuthorizationRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::time::{Instant as TokioInstant, sleep};
use wasmtime::{
    Config as WasmtimeConfig, Engine as WasmtimeEngine, Linker as WasmtimeLinker,
    Module as WasmtimeModule, Store as WasmtimeStore,
};

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
pub struct ProgrammaticCircuitBreakerPolicy {
    #[serde(default = "default_programmatic_circuit_failure_threshold")]
    pub failure_threshold: usize,
    #[serde(default = "default_programmatic_circuit_cooldown_ms")]
    pub cooldown_ms: u64,
    #[serde(default = "default_programmatic_circuit_half_open_max_calls")]
    pub half_open_max_calls: usize,
    #[serde(default = "default_programmatic_circuit_success_threshold")]
    pub success_threshold: usize,
}

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
pub enum ProgrammaticCircuitPhase {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone)]
pub struct ProgrammaticCircuitRuntimeState {
    pub phase: ProgrammaticCircuitPhase,
    pub consecutive_failures: usize,
    pub open_until: Option<TokioInstant>,
    pub half_open_remaining_calls: usize,
    pub half_open_successes: usize,
}

impl Default for ProgrammaticCircuitRuntimeState {
    fn default() -> Self {
        Self {
            phase: ProgrammaticCircuitPhase::Closed,
            consecutive_failures: 0,
            open_until: None,
            half_open_remaining_calls: 0,
            half_open_successes: 0,
        }
    }
}

pub fn default_tool_search_limit() -> usize {
    8
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeSupportSpec {
    pub enabled: bool,
    #[serde(default)]
    pub supported_bridges: Vec<PluginBridgeKind>,
    #[serde(default)]
    pub supported_adapter_families: Vec<String>,
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
    pub allowed_process_commands: BTreeSet<String>,
    pub wasm_allowed_path_prefixes: Vec<PathBuf>,
    pub wasm_max_component_bytes: Option<usize>,
    pub wasm_fuel_limit: Option<u64>,
    pub wasm_require_hash_pin: bool,
    pub wasm_required_sha256_by_plugin: BTreeMap<String, String>,
    pub enforce_execution_success: bool,
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
    pub pack_id: String,
    pub agent_id: String,
    pub operation_kind: &'static str,
    pub blocked_reason: Option<String>,
    pub approval_guard: ApprovalDecisionReport,
    pub bridge_support_checksum: Option<String>,
    pub bridge_support_sha256: Option<String>,
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

#[derive(Debug, Clone)]
pub struct ToolSearchEntry {
    pub tool_id: String,
    pub plugin_id: Option<String>,
    pub connector_name: String,
    pub provider_id: String,
    pub source_path: Option<String>,
    pub bridge_kind: PluginBridgeKind,
    pub adapter_family: Option<String>,
    pub entrypoint_hint: Option<String>,
    pub source_language: Option<String>,
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
    pub connector_name: String,
    pub provider_id: String,
    pub source_path: Option<String>,
    pub bridge_kind: String,
    pub adapter_family: Option<String>,
    pub entrypoint_hint: Option<String>,
    pub source_language: Option<String>,
    pub score: u32,
    pub deferred: bool,
    pub loaded: bool,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub input_examples: Vec<Value>,
    pub output_examples: Vec<Value>,
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
impl ConnectorAdapter for WebhookConnector {
    fn name(&self) -> &str {
        "webhook"
    }

    async fn invoke(&self, command: ConnectorCommand) -> Result<ConnectorOutcome, ConnectorError> {
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
}

#[async_trait]
impl ConnectorAdapter for DynamicCatalogConnector {
    fn name(&self) -> &str {
        &self.connector_name
    }

    async fn invoke(&self, command: ConnectorCommand) -> Result<ConnectorOutcome, ConnectorError> {
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

        let operation = command.operation.clone();
        let payload = command.payload.clone();
        let bridge_execution = bridge_execution_payload(
            &provider,
            &chosen_channel,
            &command,
            &self.bridge_runtime_policy,
        )
        .await;

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
            return Err(ConnectorError::Execution(reason.to_owned()));
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
    let adapter_family = provider
        .metadata
        .get("adapter_family")
        .cloned()
        .unwrap_or_else(|| default_bridge_adapter_family(bridge_kind));
    let entrypoint = provider
        .metadata
        .get("entrypoint")
        .or_else(|| provider.metadata.get("entrypoint_hint"))
        .cloned()
        .unwrap_or_else(|| default_bridge_entrypoint(bridge_kind, &channel.endpoint));

    let plan = match bridge_kind {
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
        PluginBridgeKind::Unknown => json!({
            "status": "deferred",
            "bridge_kind": bridge_kind.as_str(),
            "reason": "provider metadata does not declare a resolvable bridge_kind",
            "next_action": "set metadata.bridge_kind and rerun bootstrap",
        }),
    };

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
    let artifact_path = match resolve_wasm_component_artifact_path(provider, &channel.endpoint) {
        Ok(path) => path,
        Err(reason) => {
            execution["status"] = Value::String("blocked".to_owned());
            execution["reason"] = Value::String(reason);
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
            execution["runtime"] = json!({
                "executor": "wasmtime_module",
                "artifact_path": artifact_path.display().to_string(),
            });
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
        execution["runtime"] = json!({
            "executor": "wasmtime_module",
            "artifact_path": artifact_path.display().to_string(),
            "allowed_path_prefixes": normalized_allowed_prefixes
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>(),
        });
        return execution;
    }

    let artifact_metadata = match fs::metadata(&artifact_path) {
        Ok(metadata) => metadata,
        Err(error) => {
            execution["status"] = Value::String("failed".to_owned());
            execution["reason"] =
                Value::String(format!("failed to read wasm artifact metadata: {error}"));
            execution["runtime"] = json!({
                "executor": "wasmtime_module",
                "artifact_path": artifact_path.display().to_string(),
            });
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
        execution["runtime"] = json!({
            "executor": "wasmtime_module",
            "artifact_path": artifact_path.display().to_string(),
        });
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
        execution["runtime"] = json!({
            "executor": "wasmtime_module",
            "artifact_path": artifact_path.display().to_string(),
            "module_size_bytes": module_size_bytes,
            "max_component_bytes": limit,
        });
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
            execution["runtime"] = json!({
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
            });
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
                    execution["runtime"] = json!({
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
                    });
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
                execution["runtime"] = json!({
                    "executor": "wasmtime_module",
                    "artifact_path": artifact_path.display().to_string(),
                    "module_size_bytes": module_size_bytes,
                    "max_component_bytes": limit,
                });
                return execution;
            }

            let artifact_sha256 = if let Some(expected) = expected_sha256.as_deref() {
                let actual = wasm_artifact_sha256_hex(&module_bytes);
                if actual != expected {
                    execution["status"] = Value::String("blocked".to_owned());
                    execution["reason"] = Value::String(format!(
                        "wasm artifact sha256 mismatch: expected {expected}, actual {actual}"
                    ));
                    execution["runtime"] = json!({
                        "executor": "wasmtime_module",
                        "artifact_path": artifact_path.display().to_string(),
                        "module_size_bytes": module_size_bytes,
                        "expected_sha256": expected,
                        "artifact_sha256": actual,
                        "integrity_check_required": true,
                        "integrity_check_passed": false,
                    });
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
                                execution["runtime"] = json!({
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
                                });
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
                            execution["runtime"] = json!({
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
                            });
                            return execution;
                        }
                    };
                    (compiled, cache_lookup)
                }
                Err(reason) => {
                    execution["status"] = Value::String("failed".to_owned());
                    execution["reason"] = Value::String(reason);
                    execution["runtime"] = json!({
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
                    });
                    return execution;
                }
            }
        }
        Err(reason) => {
            execution["status"] = Value::String("failed".to_owned());
            execution["reason"] = Value::String(reason);
            execution["runtime"] = json!({
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
            });
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
            execution["runtime"] = json!({
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
                "expected_sha256": expected_sha256.clone(),
                "artifact_sha256": cached_module.artifact_sha256.clone(),
                "integrity_check_required": expected_sha256.is_some(),
                "integrity_check_passed": expected_sha256.is_none() || cached_module.artifact_sha256.is_some(),
            });
            execution
        }
        Err(reason) => {
            execution["status"] = Value::String("failed".to_owned());
            execution["reason"] = Value::String(reason);
            execution["runtime"] = json!({
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
                "expected_sha256": expected_sha256.clone(),
                "artifact_sha256": cached_module.artifact_sha256.clone(),
                "integrity_check_required": expected_sha256.is_some(),
                "integrity_check_passed": expected_sha256.is_none() || cached_module.artifact_sha256.is_some(),
            });
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

fn maybe_execute_native_app_tool(
    request: &ToolCoreRequest,
) -> Option<Result<ToolCoreOutcome, String>> {
    if loongclaw_app::tools::canonical_tool_name(request.tool_name.as_str()) != "claw.import" {
        return None;
    }
    Some(loongclaw_app::tools::execute_tool_core(request.clone()))
}

fn stub_memory_core(request: MemoryCoreRequest) -> Result<MemoryCoreOutcome, String> {
    Ok(MemoryCoreOutcome {
        status: "ok".to_string(),
        payload: json!({ "adapter": "kv-core", "operation": request.operation }),
    })
}

pub struct CoreToolRuntime;

#[async_trait]
impl CoreToolAdapter for CoreToolRuntime {
    fn name(&self) -> &str {
        "core-tools"
    }

    async fn execute_core_tool(
        &self,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, kernel::ToolPlaneError> {
        if let Some(result) = maybe_execute_native_app_tool(&request) {
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
        if payload.get("mode").is_none() {
            if let Some(object) = payload.as_object_mut() {
                object.insert(
                    "mode".to_owned(),
                    Value::String(request.extension_action.clone()),
                );
            }
        }

        let core_outcome = core
            .execute_core_tool(ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload,
            })
            .await?;
        Ok(ToolExtensionOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "extension": "claw-migration",
                "action": request.extension_action,
                "core_outcome": core_outcome.payload,
            }),
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
        collections::BTreeMap,
        fs,
        path::Path,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::wasm_runtime_policy::{
        DEFAULT_WASM_MODULE_CACHE_CAPACITY, DEFAULT_WASM_MODULE_CACHE_MAX_BYTES,
        MAX_WASM_MODULE_CACHE_CAPACITY, MAX_WASM_MODULE_CACHE_MAX_BYTES,
        MIN_WASM_MODULE_CACHE_MAX_BYTES, default_wasm_signals_based_traps,
        parse_wasm_module_cache_capacity, parse_wasm_module_cache_max_bytes,
        parse_wasm_signals_based_traps,
    };
    use super::{
        BridgeRuntimePolicy, WasmModuleCache, build_wasm_module_cache_key, compile_wasm_module,
        normalize_sha256_pin, resolve_expected_wasm_sha256, wasm_artifact_file_identity,
    };

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

        let second = cache.insert(key_b.clone(), compiled.clone(), 6, 8, 10);
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
}
