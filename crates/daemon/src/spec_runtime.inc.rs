#[derive(Debug, Clone, Serialize, Deserialize)]
struct DefaultCoreSelection {
    connector: Option<String>,
    runtime: Option<String>,
    tool: Option<String>,
    memory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum OperationSpec {
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
enum ProgrammaticStep {
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
struct ProgrammaticBatchCall {
    call_id: String,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProgrammaticRetryPolicy {
    #[serde(default = "default_programmatic_retry_max_attempts")]
    max_attempts: usize,
    #[serde(default = "default_programmatic_retry_initial_backoff_ms")]
    initial_backoff_ms: u64,
    #[serde(default = "default_programmatic_retry_max_backoff_ms")]
    max_backoff_ms: u64,
    #[serde(default = "default_programmatic_retry_jitter_ratio")]
    jitter_ratio: f64,
    #[serde(default = "default_true")]
    adaptive_jitter: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProgrammaticConnectorRateLimit {
    min_interval_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
enum ProgrammaticPriorityClass {
    High,
    Normal,
    Low,
}

impl ProgrammaticPriorityClass {
    fn as_str(self) -> &'static str {
        match self {
            ProgrammaticPriorityClass::High => "high",
            ProgrammaticPriorityClass::Normal => "normal",
            ProgrammaticPriorityClass::Low => "low",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ProgrammaticFairnessPolicy {
    StrictRoundRobin,
    WeightedRoundRobin,
}

impl ProgrammaticFairnessPolicy {
    fn as_str(self) -> &'static str {
        match self {
            ProgrammaticFairnessPolicy::StrictRoundRobin => "strict_round_robin",
            ProgrammaticFairnessPolicy::WeightedRoundRobin => "weighted_round_robin",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
enum ProgrammaticAdaptiveReduceOn {
    AnyError,
    ConnectorExecutionError,
    CircuitOpen,
    ConnectorNotFound,
    ConnectorNotAllowed,
    CapabilityDenied,
    PolicyDenied,
}

impl ProgrammaticAdaptiveReduceOn {
    fn as_str(self) -> &'static str {
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
struct ProgrammaticConcurrencyPolicy {
    #[serde(default = "default_programmatic_concurrency_max_in_flight")]
    max_in_flight: usize,
    #[serde(default = "default_programmatic_concurrency_min_in_flight")]
    min_in_flight: usize,
    #[serde(default = "default_programmatic_fairness_policy")]
    fairness: ProgrammaticFairnessPolicy,
    #[serde(default = "default_true")]
    adaptive_budget: bool,
    #[serde(default = "default_programmatic_priority_high_weight")]
    high_weight: usize,
    #[serde(default = "default_programmatic_priority_normal_weight")]
    normal_weight: usize,
    #[serde(default = "default_programmatic_priority_low_weight")]
    low_weight: usize,
    #[serde(default = "default_programmatic_adaptive_recovery_successes")]
    adaptive_recovery_successes: usize,
    #[serde(default = "default_programmatic_adaptive_upshift_step")]
    adaptive_upshift_step: usize,
    #[serde(default = "default_programmatic_adaptive_downshift_step")]
    adaptive_downshift_step: usize,
    #[serde(default = "default_programmatic_adaptive_reduce_on")]
    adaptive_reduce_on: BTreeSet<ProgrammaticAdaptiveReduceOn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProgrammaticCircuitBreakerPolicy {
    #[serde(default = "default_programmatic_circuit_failure_threshold")]
    failure_threshold: usize,
    #[serde(default = "default_programmatic_circuit_cooldown_ms")]
    cooldown_ms: u64,
    #[serde(default = "default_programmatic_circuit_half_open_max_calls")]
    half_open_max_calls: usize,
    #[serde(default = "default_programmatic_circuit_success_threshold")]
    success_threshold: usize,
}

#[derive(Debug, Clone)]
struct PreparedProgrammaticBatchCall {
    call_id: String,
    connector_name: String,
    operation: String,
    required_capabilities: BTreeSet<Capability>,
    retry_policy: ProgrammaticRetryPolicy,
    priority_class: ProgrammaticPriorityClass,
    payload: Value,
}

#[derive(Debug, Clone, Serialize)]
struct ProgrammaticInvocationMetrics {
    attempts: usize,
    retries: usize,
    priority_class: String,
    rate_wait_ms_total: u64,
    backoff_ms_total: u64,
    circuit_phase_before: String,
    circuit_phase_after: String,
}

#[derive(Debug, Clone, Serialize)]
struct ProgrammaticBatchExecutionSummary {
    mode: String,
    fairness: String,
    adaptive_budget: bool,
    configured_max_in_flight: usize,
    configured_min_in_flight: usize,
    peak_in_flight: usize,
    final_in_flight_budget: usize,
    budget_reductions: usize,
    budget_increases: usize,
    adaptive_upshift_step: usize,
    adaptive_downshift_step: usize,
    adaptive_reduce_on: Vec<String>,
    scheduler_wait_cycles: usize,
    dispatch_order: Vec<String>,
    priority_dispatch_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProgrammaticCircuitPhase {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone)]
struct ProgrammaticCircuitRuntimeState {
    phase: ProgrammaticCircuitPhase,
    consecutive_failures: usize,
    open_until: Option<TokioInstant>,
    half_open_remaining_calls: usize,
    half_open_successes: usize,
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

fn default_tool_search_limit() -> usize {
    8
}

fn default_programmatic_max_calls() -> usize {
    12
}

fn default_programmatic_retry_max_attempts() -> usize {
    1
}

fn default_programmatic_retry_initial_backoff_ms() -> u64 {
    100
}

fn default_programmatic_retry_max_backoff_ms() -> u64 {
    2_000
}

fn default_programmatic_retry_jitter_ratio() -> f64 {
    0.2
}

fn default_programmatic_priority_class() -> ProgrammaticPriorityClass {
    ProgrammaticPriorityClass::Normal
}

fn default_programmatic_concurrency_max_in_flight() -> usize {
    4
}

fn default_programmatic_concurrency_min_in_flight() -> usize {
    1
}

fn default_programmatic_fairness_policy() -> ProgrammaticFairnessPolicy {
    ProgrammaticFairnessPolicy::WeightedRoundRobin
}

fn default_programmatic_priority_high_weight() -> usize {
    4
}

fn default_programmatic_priority_normal_weight() -> usize {
    2
}

fn default_programmatic_priority_low_weight() -> usize {
    1
}

fn default_programmatic_adaptive_recovery_successes() -> usize {
    2
}

fn default_programmatic_adaptive_upshift_step() -> usize {
    1
}

fn default_programmatic_adaptive_downshift_step() -> usize {
    1
}

fn default_programmatic_adaptive_reduce_on() -> BTreeSet<ProgrammaticAdaptiveReduceOn> {
    BTreeSet::from([
        ProgrammaticAdaptiveReduceOn::ConnectorExecutionError,
        ProgrammaticAdaptiveReduceOn::CircuitOpen,
    ])
}

fn default_programmatic_concurrency_policy() -> ProgrammaticConcurrencyPolicy {
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

fn default_programmatic_circuit_failure_threshold() -> usize {
    3
}

fn default_programmatic_circuit_cooldown_ms() -> u64 {
    1_000
}

fn default_programmatic_circuit_half_open_max_calls() -> usize {
    1
}

fn default_programmatic_circuit_success_threshold() -> usize {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunnerSpec {
    pack: VerticalPackManifest,
    agent_id: String,
    ttl_s: u64,
    #[serde(default)]
    approval: Option<HumanApprovalSpec>,
    defaults: Option<DefaultCoreSelection>,
    self_awareness: Option<SelfAwarenessSpec>,
    plugin_scan: Option<PluginScanSpec>,
    bridge_support: Option<BridgeSupportSpec>,
    bootstrap: Option<BootstrapSpec>,
    auto_provision: Option<AutoProvisionSpec>,
    #[serde(default)]
    hotfixes: Vec<HotfixSpec>,
    operation: OperationSpec,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
enum HumanApprovalMode {
    Disabled,
    #[default]
    MediumBalanced,
    Strict,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
enum HumanApprovalStrategy {
    #[default]
    PerCall,
    OneTimeFullAccess,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
enum HumanApprovalScope {
    #[default]
    ToolCalls,
    AllOperations,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HumanApprovalSpec {
    #[serde(default)]
    mode: HumanApprovalMode,
    #[serde(default)]
    strategy: HumanApprovalStrategy,
    #[serde(default)]
    scope: HumanApprovalScope,
    #[serde(default)]
    one_time_full_access_granted: bool,
    #[serde(default)]
    approved_calls: Vec<String>,
    #[serde(default)]
    denied_calls: Vec<String>,
    #[serde(default)]
    risk_profile_path: Option<String>,
    #[serde(default)]
    one_time_full_access_expires_at_epoch_s: Option<u64>,
    #[serde(default)]
    one_time_full_access_remaining_uses: Option<u32>,
    #[serde(default)]
    high_risk_keywords: Vec<String>,
    #[serde(default)]
    high_risk_tool_names: Vec<String>,
    #[serde(default)]
    high_risk_payload_keys: Vec<String>,
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
enum ApprovalRiskLevel {
    Low,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApprovalDecisionReport {
    mode: HumanApprovalMode,
    strategy: HumanApprovalStrategy,
    scope: HumanApprovalScope,
    now_epoch_s: u64,
    operation_key: String,
    operation_kind: &'static str,
    risk_level: ApprovalRiskLevel,
    risk_score: u8,
    denylisted: bool,
    requires_human_approval: bool,
    approved: bool,
    reason: String,
    matched_keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ApprovalRiskScoring {
    keyword_weight: u8,
    tool_name_weight: u8,
    payload_key_weight: u8,
    keyword_hit_cap: usize,
    payload_key_hit_cap: usize,
    high_risk_threshold: u8,
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
struct ApprovalRiskProfile {
    #[serde(default)]
    high_risk_keywords: Vec<String>,
    #[serde(default)]
    high_risk_tool_names: Vec<String>,
    #[serde(default)]
    high_risk_payload_keys: Vec<String>,
    #[serde(default)]
    scoring: ApprovalRiskScoring,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SelfAwarenessSpec {
    enabled: bool,
    #[serde(default)]
    roots: Vec<String>,
    #[serde(default)]
    plugin_roots: Vec<String>,
    #[serde(default)]
    proposed_mutations: Vec<String>,
    #[serde(default)]
    enforce_guard: bool,
    #[serde(default)]
    immutable_core_paths: Vec<String>,
    #[serde(default)]
    mutable_extension_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PluginScanSpec {
    enabled: bool,
    roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BridgeSupportSpec {
    enabled: bool,
    #[serde(default)]
    supported_bridges: Vec<PluginBridgeKind>,
    #[serde(default)]
    supported_adapter_families: Vec<String>,
    #[serde(default)]
    enforce_supported: bool,
    #[serde(default)]
    policy_version: Option<String>,
    #[serde(default)]
    expected_checksum: Option<String>,
    #[serde(default)]
    expected_sha256: Option<String>,
    #[serde(default)]
    execute_process_stdio: bool,
    #[serde(default)]
    execute_http_json: bool,
    #[serde(default)]
    allowed_process_commands: Vec<String>,
    #[serde(default)]
    enforce_execution_success: bool,
    #[serde(default)]
    security_scan: Option<SecurityScanSpec>,
}

#[derive(Debug, Clone, Default)]
struct BridgeRuntimePolicy {
    execute_process_stdio: bool,
    execute_http_json: bool,
    execute_wasm_component: bool,
    allowed_process_commands: BTreeSet<String>,
    wasm_allowed_path_prefixes: Vec<PathBuf>,
    wasm_max_component_bytes: Option<usize>,
    wasm_fuel_limit: Option<u64>,
    enforce_execution_success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SecurityScanSpec {
    enabled: bool,
    #[serde(default = "default_true")]
    block_on_high: bool,
    #[serde(default)]
    profile_path: Option<String>,
    #[serde(default)]
    profile_sha256: Option<String>,
    #[serde(default)]
    profile_signature: Option<SecurityProfileSignatureSpec>,
    #[serde(default)]
    siem_export: Option<SecuritySiemExportSpec>,
    #[serde(default)]
    runtime: SecurityRuntimeExecutionSpec,
    #[serde(default)]
    high_risk_metadata_keywords: Vec<String>,
    #[serde(default)]
    wasm: WasmSecurityScanSpec,
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
struct SecurityProfileSignatureSpec {
    #[serde(default = "default_security_profile_signature_algorithm")]
    algorithm: String,
    public_key_base64: String,
    signature_base64: String,
}

fn default_security_profile_signature_algorithm() -> String {
    "ed25519".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SecuritySiemExportSpec {
    enabled: bool,
    path: String,
    #[serde(default = "default_true")]
    include_findings: bool,
    #[serde(default)]
    max_findings_per_record: Option<usize>,
    #[serde(default)]
    fail_on_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SecuritySiemExportReport {
    enabled: bool,
    path: String,
    success: bool,
    exported_records: usize,
    exported_findings: usize,
    truncated_findings: usize,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SecurityRuntimeExecutionSpec {
    #[serde(default)]
    execute_wasm_component: bool,
    #[serde(default)]
    allowed_path_prefixes: Vec<String>,
    #[serde(default)]
    max_component_bytes: Option<usize>,
    #[serde(default)]
    fuel_limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WasmSecurityScanSpec {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    max_module_bytes: usize,
    #[serde(default)]
    allow_wasi: bool,
    #[serde(default)]
    blocked_import_prefixes: Vec<String>,
    #[serde(default)]
    allowed_path_prefixes: Vec<String>,
    #[serde(default)]
    require_hash_pin: bool,
    #[serde(default)]
    required_sha256_by_plugin: BTreeMap<String, String>,
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
struct SecurityScanProfile {
    #[serde(default)]
    high_risk_metadata_keywords: Vec<String>,
    #[serde(default)]
    wasm: WasmSecurityScanSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SecurityFindingSeverity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SecurityFinding {
    correlation_id: String,
    severity: SecurityFindingSeverity,
    category: String,
    plugin_id: String,
    source_path: String,
    message: String,
    evidence: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SecurityScanReport {
    enabled: bool,
    scanned_plugins: usize,
    total_findings: usize,
    high_findings: usize,
    medium_findings: usize,
    low_findings: usize,
    blocked: bool,
    block_reason: Option<String>,
    siem_export: Option<SecuritySiemExportReport>,
    findings: Vec<SecurityFinding>,
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

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BootstrapSpec {
    enabled: bool,
    #[serde(default)]
    allow_http_json_auto_apply: Option<bool>,
    #[serde(default)]
    allow_process_stdio_auto_apply: Option<bool>,
    #[serde(default)]
    allow_native_ffi_auto_apply: Option<bool>,
    #[serde(default)]
    allow_wasm_component_auto_apply: Option<bool>,
    #[serde(default)]
    allow_mcp_server_auto_apply: Option<bool>,
    #[serde(default)]
    enforce_ready_execution: Option<bool>,
    #[serde(default)]
    max_tasks: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AutoProvisionSpec {
    enabled: bool,
    provider_id: String,
    channel_id: String,
    connector_name: Option<String>,
    endpoint: Option<String>,
    required_capabilities: BTreeSet<Capability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum HotfixSpec {
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
    fn to_kernel_hotfix(&self) -> IntegrationHotfix {
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
    fn template() -> Self {
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
struct SpecRunReport {
    pack_id: String,
    agent_id: String,
    operation_kind: &'static str,
    blocked_reason: Option<String>,
    approval_guard: ApprovalDecisionReport,
    bridge_support_checksum: Option<String>,
    bridge_support_sha256: Option<String>,
    self_awareness: Option<CodebaseAwarenessSnapshot>,
    architecture_guard: Option<ArchitectureGuardReport>,
    plugin_scan_reports: Vec<PluginScanReport>,
    plugin_translation_reports: Vec<PluginTranslationReport>,
    plugin_activation_plans: Vec<PluginActivationPlan>,
    plugin_bootstrap_reports: Vec<BootstrapReport>,
    plugin_bootstrap_queue: Vec<String>,
    plugin_absorb_reports: Vec<PluginAbsorbReport>,
    security_scan_report: Option<SecurityScanReport>,
    auto_provision_plan: Option<ProvisionPlan>,
    outcome: Value,
    integration_catalog: IntegrationCatalog,
    audit_events: Option<Vec<AuditEvent>>,
}

#[derive(Debug, Clone)]
struct ToolSearchEntry {
    tool_id: String,
    plugin_id: Option<String>,
    connector_name: String,
    provider_id: String,
    source_path: Option<String>,
    bridge_kind: PluginBridgeKind,
    adapter_family: Option<String>,
    entrypoint_hint: Option<String>,
    source_language: Option<String>,
    summary: Option<String>,
    tags: Vec<String>,
    input_examples: Vec<Value>,
    output_examples: Vec<Value>,
    deferred: bool,
    loaded: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ToolSearchResult {
    tool_id: String,
    plugin_id: Option<String>,
    connector_name: String,
    provider_id: String,
    source_path: Option<String>,
    bridge_kind: String,
    adapter_family: Option<String>,
    entrypoint_hint: Option<String>,
    source_language: Option<String>,
    score: u32,
    deferred: bool,
    loaded: bool,
    summary: Option<String>,
    tags: Vec<String>,
    input_examples: Vec<Value>,
    output_examples: Vec<Value>,
}

struct EmbeddedPiHarness {
    seen: Mutex<Vec<String>>,
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
        self.seen
            .lock()
            .expect("mutex poisoned")
            .push(request.task_id.clone());

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

struct WebhookConnector;

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
                    let mut guard = attempts_map.lock().map_err(|_| {
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

struct DynamicCatalogConnector {
    connector_name: String,
    provider_id: String,
    catalog: Arc<Mutex<IntegrationCatalog>>,
    bridge_runtime_policy: BridgeRuntimePolicy,
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
            let catalog = self.catalog.lock().map_err(|_| {
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

async fn bridge_execution_payload(
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

async fn maybe_execute_bridge(
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

#[derive(Debug, Clone)]
struct ConnectorProtocolContext {
    request_method: String,
    request_id: Option<String>,
    route_method: Option<String>,
    required_capability: Option<String>,
    capabilities: BTreeSet<String>,
}

impl ConnectorProtocolContext {
    fn from_connector_command(
        provider: &kernel::ProviderConfig,
        channel: &kernel::ChannelConfig,
        command: &ConnectorCommand,
    ) -> Self {
        Self {
            request_method: "tools/call".to_owned(),
            request_id: Some(format!(
                "{}:{}:{}",
                provider.provider_id, channel.channel_id, command.operation
            )),
            route_method: None,
            required_capability: None,
            capabilities: protocol_capabilities_for_connector_command(command),
        }
    }

    fn capabilities_vec(&self) -> Vec<String> {
        self.capabilities.iter().cloned().collect::<Vec<_>>()
    }

    fn outbound_frame(&self, payload: Value) -> OutboundFrame {
        OutboundFrame {
            method: self.request_method.clone(),
            id: self.request_id.clone(),
            payload,
        }
    }
}

fn authorize_connector_protocol_context(context: &mut ConnectorProtocolContext) -> Result<(), String> {
    let router = ProtocolRouter::default();
    let resolved_route = router
        .resolve(&context.request_method)
        .map_err(|error| {
            format!(
                "protocol method {} is invalid: {error}",
                context.request_method
            )
        })?;
    context.route_method = Some(resolved_route.method().to_owned());
    context.required_capability = resolved_route.policy.required_capability.clone();
    router
        .authorize(
            &resolved_route,
            &RouteAuthorizationRequest {
                authenticated: true,
                capabilities: context.capabilities.clone(),
            },
        )
        .map_err(|error| format!("protocol route authorization failed: {error}"))?;
    Ok(())
}

fn option_string_to_value(value: &Option<String>) -> Value {
    value
        .as_ref()
        .map(|inner| Value::String(inner.clone()))
        .unwrap_or(Value::Null)
}

fn append_connector_protocol_runtime_context(
    runtime: &mut Value,
    context: &ConnectorProtocolContext,
) {
    if let Some(object) = runtime.as_object_mut() {
        object.insert(
            "request_method".to_owned(),
            Value::String(context.request_method.clone()),
        );
        object.insert("request_id".to_owned(), option_string_to_value(&context.request_id));
        object.insert(
            "protocol_route".to_owned(),
            option_string_to_value(&context.route_method),
        );
        object.insert(
            "protocol_required_capability".to_owned(),
            option_string_to_value(&context.required_capability),
        );
        object.insert(
            "protocol_capabilities".to_owned(),
            Value::Array(
                context
                    .capabilities_vec()
                    .into_iter()
                    .map(Value::String)
                    .collect::<Vec<_>>(),
            ),
        );
    }
}

fn execute_http_json_bridge(
    mut execution: Value,
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    command: &ConnectorCommand,
) -> Value {
    let method_label = provider
        .metadata
        .get("http_method")
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "POST".to_owned());
    let method = match Method::from_bytes(method_label.as_bytes()) {
        Ok(method) => method,
        Err(error) => {
            execution["status"] = Value::String("blocked".to_owned());
            execution["reason"] =
                Value::String(format!("invalid http_method {method_label}: {error}"));
            return execution;
        }
    };

    let timeout_ms = parse_http_timeout_ms(provider);
    let enforce_protocol_contract = parse_http_enforce_protocol_contract(provider);
    let mut protocol_context =
        ConnectorProtocolContext::from_connector_command(provider, channel, command);
    if let Err(reason) = authorize_connector_protocol_context(&mut protocol_context) {
        execution["status"] = Value::String("blocked".to_owned());
        execution["reason"] = Value::String(format!("http_json {reason}"));
        let mut runtime = json!({
            "executor": "http_json_reqwest",
            "method": method_label,
            "url": channel.endpoint,
            "timeout_ms": timeout_ms,
            "enforce_protocol_contract": enforce_protocol_contract,
        });
        append_connector_protocol_runtime_context(&mut runtime, &protocol_context);
        execution["runtime"] = runtime;
        return execution;
    }

    let request_payload = json!({
        "provider_id": provider.provider_id,
        "channel_id": channel.channel_id,
        "operation": command.operation,
        "payload": command.payload,
    });
    let url = channel.endpoint.clone();
    let request_payload_for_runtime = request_payload.clone();
    let request_payload_for_worker = request_payload.clone();
    let request_method_for_worker = protocol_context.request_method.clone();
    let request_id_for_worker = protocol_context.request_id.clone();

    let run = std::thread::spawn(move || -> Result<(u16, bool, String, Value, Option<String>, Option<String>), String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .map_err(|error| format!("failed to initialize http_json client: {error}"))?;

        let response = client
            .request(method, &url)
            .header("content-type", "application/json")
            .json(&request_payload_for_worker)
            .send()
            .map_err(|error| format!("http_json bridge request failed: {error}"))?;

        let status = response.status();
        let status_code = status.as_u16();
        let success = status.is_success();
        let body = response
            .text()
            .map_err(|error| format!("failed to read http_json response body: {error}"))?;
        let body_json = serde_json::from_str::<Value>(&body).unwrap_or(Value::Null);

        let response_method = body_json
            .as_object()
            .and_then(|value| value.get("method"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let response_id = body_json
            .as_object()
            .and_then(|value| value.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if enforce_protocol_contract {
            let method = response_method
                .as_deref()
                .ok_or_else(|| "http_json strict protocol contract requires response.method".to_owned())?;
            if method != request_method_for_worker {
                return Err(format!(
                    "http_json response method mismatch: expected `{}`, got `{method}`",
                    request_method_for_worker
                ));
            }
            if response_id != request_id_for_worker {
                return Err(format!(
                    "http_json response id mismatch: expected `{:?}`, got `{:?}`",
                    request_id_for_worker, response_id
                ));
            }
        }

        Ok((status_code, success, body, body_json, response_method, response_id))
    })
    .join();

    match run {
        Ok(Ok((
            status_code,
            success,
            body,
            body_json,
            response_method,
            response_id,
        ))) => {
            execution["status"] = Value::String(if success {
                "executed".to_owned()
            } else {
                "failed".to_owned()
            });
            if !success {
                execution["reason"] = Value::String(format!(
                    "http_json bridge request failed with status {status_code}"
                ));
            }
            let mut runtime = json!({
                "executor": "http_json_reqwest",
                "method": method_label,
                "url": channel.endpoint,
                "status_code": status_code,
                "request": request_payload_for_runtime,
                "response_text": body,
                "response_json": body_json,
                "response_method": response_method,
                "response_id": response_id,
                "timeout_ms": timeout_ms,
                "enforce_protocol_contract": enforce_protocol_contract,
            });
            append_connector_protocol_runtime_context(&mut runtime, &protocol_context);
            execution["runtime"] = runtime;
            execution
        }
        Ok(Err(reason)) => {
            execution["status"] = Value::String("failed".to_owned());
            execution["reason"] = Value::String(reason);
            let mut runtime = json!({
                "executor": "http_json_reqwest",
                "method": method_label,
                "url": channel.endpoint,
                "request": request_payload_for_runtime,
                "timeout_ms": timeout_ms,
                "enforce_protocol_contract": enforce_protocol_contract,
            });
            append_connector_protocol_runtime_context(&mut runtime, &protocol_context);
            execution["runtime"] = runtime;
            execution
        }
        Err(_) => {
            execution["status"] = Value::String("failed".to_owned());
            execution["reason"] =
                Value::String("http_json bridge worker thread panicked".to_owned());
            let mut runtime = json!({
                "executor": "http_json_reqwest",
                "method": method_label,
                "url": channel.endpoint,
                "request": request_payload_for_runtime,
                "timeout_ms": timeout_ms,
                "enforce_protocol_contract": enforce_protocol_contract,
            });
            append_connector_protocol_runtime_context(&mut runtime, &protocol_context);
            execution["runtime"] = runtime;
            execution
        }
    }
}

async fn execute_process_stdio_bridge(
    mut execution: Value,
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    command: &ConnectorCommand,
    runtime_policy: &BridgeRuntimePolicy,
) -> Value {
    let Some(program) = provider.metadata.get("command").cloned() else {
        execution["status"] = Value::String("blocked".to_owned());
        execution["reason"] =
            Value::String("process_stdio execution requires provider metadata.command".to_owned());
        return execution;
    };

    if !is_process_command_allowed(&program, &runtime_policy.allowed_process_commands) {
        execution["status"] = Value::String("blocked".to_owned());
        execution["reason"] = Value::String(format!(
            "process command {program} is not allowed by runtime policy"
        ));
        return execution;
    }

    let args = parse_process_args(provider);
    let timeout_ms = parse_process_timeout_ms(provider);
    let envelope = json!({
        "provider_id": provider.provider_id,
        "channel_id": channel.channel_id,
        "operation": command.operation,
        "payload": command.payload,
    });
    let mut protocol_context =
        ConnectorProtocolContext::from_connector_command(provider, channel, command);
    if let Err(reason) = authorize_connector_protocol_context(&mut protocol_context) {
        execution["status"] = Value::String("blocked".to_owned());
        execution["reason"] = Value::String(format!("process_stdio {reason}"));
        let mut runtime = json!({
            "executor": "process_stdio_local",
            "transport_kind": "json_line",
            "command": program,
            "args": args,
            "timeout_ms": timeout_ms,
        });
        append_connector_protocol_runtime_context(&mut runtime, &protocol_context);
        execution["runtime"] = runtime;
        return execution;
    }

    let exchange_result = run_process_stdio_json_line_exchange(
        &program,
        &args,
        timeout_ms,
        protocol_context.outbound_frame(envelope),
    )
    .await;

    match exchange_result {
        Ok(outcome) => {
            execution["status"] = Value::String(if outcome.success {
                "executed".to_owned()
            } else {
                "failed".to_owned()
            });
            if !outcome.success {
                execution["reason"] = Value::String(format!(
                    "process command exited with code {:?}",
                    outcome.exit_code
                ));
            }
            let mut runtime = json!({
                "executor": "process_stdio_local",
                "transport_kind": "json_line",
                "command": program,
                "args": args,
                "exit_code": outcome.exit_code,
                "stdout": outcome.stdout,
                "stderr": outcome.stderr,
                "stdout_json": outcome.stdout_json,
                "timeout_ms": timeout_ms,
                "response_method": outcome.response_method,
                "response_id": outcome.response_id,
            });
            append_connector_protocol_runtime_context(&mut runtime, &protocol_context);
            execution["runtime"] = runtime;
            execution
        }
        Err(reason) => {
            execution["status"] = Value::String("failed".to_owned());
            execution["reason"] = Value::String(reason);
            let mut runtime = json!({
                "executor": "process_stdio_local",
                "transport_kind": "json_line",
                "command": program,
                "args": args,
                "timeout_ms": timeout_ms,
            });
            append_connector_protocol_runtime_context(&mut runtime, &protocol_context);
            execution["runtime"] = runtime;
            execution
        }
    }
}

struct ProcessStdioExchangeOutcome {
    success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    stdout_json: Value,
    response_method: String,
    response_id: Option<String>,
}

async fn run_process_stdio_json_line_exchange(
    program: &str,
    args: &[String],
    timeout_ms: u64,
    frame: OutboundFrame,
) -> Result<ProcessStdioExchangeOutcome, String> {
    let mut child = TokioCommand::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to spawn process command {program}: {error}"))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| format!("process command {program} stdin is not piped"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("process command {program} stdout is not piped"))?;
    let stderr = child.stderr.take();
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(mut stderr_pipe) = stderr {
            let _ = stderr_pipe.read_to_end(&mut bytes).await;
        }
        bytes
    });

    let transport = JsonLineTransport::new(
        TransportInfo {
            name: format!("process_stdio/{program}"),
            version: "0.1.0".to_owned(),
            secure: false,
        },
        stdout,
        stdin,
    );

    let expected_method = frame.method.clone();
    let expected_id = frame.id.clone();

    if let Err(error) = timeout(Duration::from_millis(timeout_ms), transport.send(frame))
        .await
        .map_err(|_| format!("process_stdio transport send timed out after {timeout_ms}ms"))?
    {
        let _ = child.start_kill();
        let _ = child.wait().await;
        let _ = stderr_task.await;
        return Err(format!("process_stdio transport send failed: {error}"));
    }
    if let Err(error) = timeout(Duration::from_millis(timeout_ms), transport.close())
        .await
        .map_err(|_| format!("process_stdio transport close timed out after {timeout_ms}ms"))?
    {
        let _ = child.start_kill();
        let _ = child.wait().await;
        let _ = stderr_task.await;
        return Err(format!("process_stdio transport close failed: {error}"));
    }

    let response = match timeout(Duration::from_millis(timeout_ms), transport.recv()).await {
        Ok(Ok(Some(frame))) => frame,
        Ok(Ok(None)) => {
            drop(transport);
            let _ = child.wait().await;
            let _ = stderr_task.await;
            return Err("process_stdio transport closed before response frame".to_owned());
        }
        Ok(Err(error)) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            let _ = stderr_task.await;
            return Err(format!("process_stdio transport recv failed: {error}"));
        }
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            let _ = stderr_task.await;
            return Err(format!(
                "process_stdio transport recv timed out after {timeout_ms}ms"
            ));
        }
    };

    if response.method != expected_method {
        let _ = child.start_kill();
        let _ = child.wait().await;
        let _ = stderr_task.await;
        return Err(format!(
            "process_stdio response method mismatch: expected `{expected_method}`, got `{}`",
            response.method
        ));
    }
    if response.id != expected_id {
        let _ = child.start_kill();
        let _ = child.wait().await;
        let _ = stderr_task.await;
        return Err(format!(
            "process_stdio response id mismatch: expected `{:?}`, got `{:?}`",
            expected_id, response.id
        ));
    }

    drop(transport);
    let status = match timeout(Duration::from_millis(timeout_ms), child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            let _ = stderr_task.await;
            return Err(format!("failed to wait process output: {error}"));
        }
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            let _ = stderr_task.await;
            return Err(format!(
                "process command timed out after {timeout_ms}ms waiting for exit"
            ));
        }
    };
    let stderr_bytes = stderr_task
        .await
        .map_err(|error| format!("failed to collect process stderr: {error}"))?;
    let stderr = String::from_utf8_lossy(&stderr_bytes).trim().to_owned();
    let stdout_json = response.payload;
    let stdout = serde_json::to_string(&stdout_json).unwrap_or_else(|_| "null".to_owned());

    Ok(ProcessStdioExchangeOutcome {
        success: status.success(),
        exit_code: status.code(),
        stdout,
        stderr,
        stdout_json,
        response_method: response.method,
        response_id: response.id,
    })
}

fn execute_wasm_component_bridge(
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

    if !runtime_policy.wasm_allowed_path_prefixes.is_empty()
        && !runtime_policy
            .wasm_allowed_path_prefixes
            .iter()
            .any(|prefix| artifact_path.starts_with(prefix))
    {
        execution["status"] = Value::String("blocked".to_owned());
        execution["reason"] =
            Value::String("wasm artifact path is outside runtime allowed_path_prefixes".to_owned());
        execution["runtime"] = json!({
            "executor": "wasmtime_module",
            "artifact_path": artifact_path.display().to_string(),
            "allowed_path_prefixes": runtime_policy
                .wasm_allowed_path_prefixes
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>(),
        });
        return execution;
    }

    let module_bytes = match fs::read(&artifact_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            execution["status"] = Value::String("failed".to_owned());
            execution["reason"] = Value::String(format!("failed to read wasm artifact: {error}"));
            execution["runtime"] = json!({
                "executor": "wasmtime_module",
                "artifact_path": artifact_path.display().to_string(),
            });
            return execution;
        }
    };

    if let Some(limit) = runtime_policy.wasm_max_component_bytes {
        if module_bytes.len() > limit {
            execution["status"] = Value::String("blocked".to_owned());
            execution["reason"] = Value::String(format!(
                "wasm artifact size {} exceeds runtime max_component_bytes {limit}",
                module_bytes.len()
            ));
            execution["runtime"] = json!({
                "executor": "wasmtime_module",
                "artifact_path": artifact_path.display().to_string(),
                "module_size_bytes": module_bytes.len(),
                "max_component_bytes": limit,
            });
            return execution;
        }
    }

    let export_name = resolve_wasm_export_name(provider);

    let run_result = (|| -> Result<Option<u64>, String> {
        let mut config = WasmtimeConfig::new();
        if runtime_policy.wasm_fuel_limit.is_some() {
            config.consume_fuel(true);
        }
        let engine = WasmtimeEngine::new(&config)
            .map_err(|error| format!("failed to initialize wasmtime engine: {error}"))?;
        let module = WasmtimeModule::new(&engine, &module_bytes)
            .map_err(|error| format!("failed to compile wasm module: {error}"))?;
        let mut store = WasmtimeStore::new(&engine, ());
        if let Some(limit) = runtime_policy.wasm_fuel_limit {
            store
                .set_fuel(limit)
                .map_err(|error| format!("failed to set wasm fuel limit: {error}"))?;
        }
        let linker = WasmtimeLinker::new(&engine);
        let instance = linker
            .instantiate(&mut store, &module)
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
                "module_size_bytes": module_bytes.len(),
                "fuel_limit": runtime_policy.wasm_fuel_limit,
                "fuel_consumed": consumed_fuel,
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
            });
            execution
        }
    }
}

fn resolve_wasm_component_artifact_path(
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

fn resolve_wasm_export_name(provider: &kernel::ProviderConfig) -> String {
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

fn parse_process_args(provider: &kernel::ProviderConfig) -> Vec<String> {
    if let Some(args_json) = provider.metadata.get("args_json") {
        if let Ok(args) = serde_json::from_str::<Vec<String>>(args_json) {
            return args;
        }
    }

    provider
        .metadata
        .get("args")
        .map(|value| value.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

fn parse_http_timeout_ms(provider: &kernel::ProviderConfig) -> u64 {
    parse_clamped_timeout_ms(provider, "http_timeout_ms", 8_000, 300_000)
}

fn parse_http_enforce_protocol_contract(provider: &kernel::ProviderConfig) -> bool {
    parse_bool_flag(
        provider
            .metadata
            .get("http_enforce_protocol_contract")
            .map(String::as_str),
    )
}

fn parse_process_timeout_ms(provider: &kernel::ProviderConfig) -> u64 {
    parse_clamped_timeout_ms(provider, "process_timeout_ms", 5_000, 300_000)
}

fn parse_bool_flag(raw: Option<&str>) -> bool {
    raw.map(|value| value.trim().to_ascii_lowercase())
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
}

fn parse_clamped_timeout_ms(
    provider: &kernel::ProviderConfig,
    metadata_key: &str,
    default_ms: u64,
    max_ms: u64,
) -> u64 {
    provider
        .metadata
        .get(metadata_key)
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.min(max_ms))
        .unwrap_or(default_ms)
}

fn protocol_capabilities_for_connector_command(command: &ConnectorCommand) -> BTreeSet<String> {
    let mut capabilities = BTreeSet::new();
    for capability in &command.required_capabilities {
        match capability {
            Capability::MemoryRead
            | Capability::FilesystemRead
            | Capability::ObserveTelemetry => {
                capabilities.insert("discover".to_owned());
            }
            _ => {
                capabilities.insert("invoke".to_owned());
            }
        }
    }
    if capabilities.is_empty() {
        capabilities.insert("invoke".to_owned());
    }
    capabilities
}

fn provider_allowed_callers(provider: &kernel::ProviderConfig) -> BTreeSet<String> {
    let mut allowed = BTreeSet::new();

    if let Some(raw_json) = provider.metadata.get("allowed_callers_json") {
        if let Ok(values) = serde_json::from_str::<Vec<String>>(raw_json) {
            for value in values {
                let normalized = value.trim().to_ascii_lowercase();
                if !normalized.is_empty() {
                    allowed.insert(normalized);
                }
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

fn caller_from_payload(payload: &Value) -> Option<String> {
    payload
        .get("_loongclaw")
        .and_then(Value::as_object)
        .and_then(|meta| meta.get("caller"))
        .and_then(Value::as_str)
        .map(|caller| caller.trim().to_ascii_lowercase())
        .filter(|caller| !caller.is_empty())
}

fn caller_is_allowed(caller: Option<&str>, allowed: &BTreeSet<String>) -> bool {
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

fn is_process_command_allowed(program: &str, allowed: &BTreeSet<String>) -> bool {
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

fn detect_provider_bridge_kind(
    provider: &kernel::ProviderConfig,
    endpoint: &str,
) -> PluginBridgeKind {
    if let Some(raw) = provider.metadata.get("bridge_kind") {
        if let Some(kind) = parse_bridge_kind_label(raw) {
            return kind;
        }
    }

    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        return PluginBridgeKind::HttpJson;
    }

    PluginBridgeKind::Unknown
}

fn parse_bridge_kind_label(raw: &str) -> Option<PluginBridgeKind> {
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

fn default_bridge_adapter_family(bridge_kind: PluginBridgeKind) -> String {
    match bridge_kind {
        PluginBridgeKind::HttpJson => "http-adapter".to_owned(),
        PluginBridgeKind::ProcessStdio => "stdio-adapter".to_owned(),
        PluginBridgeKind::NativeFfi => "ffi-adapter".to_owned(),
        PluginBridgeKind::WasmComponent => "wasm-component-adapter".to_owned(),
        PluginBridgeKind::McpServer => "mcp-adapter".to_owned(),
        PluginBridgeKind::Unknown => "unknown-adapter".to_owned(),
    }
}

fn default_bridge_entrypoint(bridge_kind: PluginBridgeKind, endpoint: &str) -> String {
    match bridge_kind {
        PluginBridgeKind::HttpJson => endpoint.to_owned(),
        PluginBridgeKind::ProcessStdio => "stdin/stdout::invoke".to_owned(),
        PluginBridgeKind::NativeFfi => "lib::invoke".to_owned(),
        PluginBridgeKind::WasmComponent => "component::run".to_owned(),
        PluginBridgeKind::McpServer => "mcp::stdio".to_owned(),
        PluginBridgeKind::Unknown => "unknown::invoke".to_owned(),
    }
}

struct CrmCoreConnector;

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

struct CrmGrpcCoreConnector;

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

struct ShieldedConnectorExtension;

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

struct NativeCoreRuntime;

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

struct FallbackCoreRuntime;

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

struct AcpBridgeRuntimeExtension;

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

struct CoreToolRuntime;

#[async_trait]
impl CoreToolAdapter for CoreToolRuntime {
    fn name(&self) -> &str {
        "core-tools"
    }

    async fn execute_core_tool(
        &self,
        request: ToolCoreRequest,
    ) -> Result<ToolCoreOutcome, kernel::ToolPlaneError> {
        mvp::tools::execute_tool_core(request).map_err(kernel::ToolPlaneError::Execution)
    }
}

struct SqlAnalyticsToolExtension;

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

struct KvCoreMemory;

#[async_trait]
impl CoreMemoryAdapter for KvCoreMemory {
    fn name(&self) -> &str {
        "kv-core"
    }

    async fn execute_core_memory(
        &self,
        request: MemoryCoreRequest,
    ) -> Result<MemoryCoreOutcome, kernel::MemoryPlaneError> {
        mvp::memory::execute_memory_core(request).map_err(kernel::MemoryPlaneError::Execution)
    }
}

struct VectorIndexMemoryExtension;

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
