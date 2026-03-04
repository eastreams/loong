use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signature as Ed25519Signature, Verifier, VerifyingKey};
use kernel::{
    ArchitectureBoundaryPolicy, ArchitectureGuardReport, AuditEvent, AuditEventKind, AuditSink,
    AutoProvisionAgent, AutoProvisionRequest, BootstrapPolicy, BootstrapReport,
    BootstrapTaskStatus, BridgeSupportMatrix, Capability, ChumosKernel, Clock,
    CodebaseAwarenessConfig, CodebaseAwarenessEngine, CodebaseAwarenessSnapshot, ConnectorAdapter,
    ConnectorCommand, ConnectorError, ConnectorOutcome, CoreConnectorAdapter, CoreMemoryAdapter,
    CoreRuntimeAdapter, CoreToolAdapter, ExecutionRoute, FixedClock, HarnessAdapter, HarnessError,
    HarnessKind, HarnessOutcome, HarnessRequest, InMemoryAuditSink, IntegrationCatalog,
    IntegrationHotfix, MemoryCoreOutcome, MemoryCoreRequest, MemoryExtensionAdapter,
    MemoryExtensionOutcome, MemoryExtensionRequest, PluginAbsorbReport, PluginActivationPlan,
    PluginActivationStatus, PluginBootstrapExecutor, PluginBridgeKind, PluginDescriptor,
    PluginScanReport, PluginScanner, PluginTranslationReport, PluginTranslator, ProvisionPlan,
    RuntimeCoreOutcome, RuntimeCoreRequest, RuntimeExtensionAdapter, RuntimeExtensionOutcome,
    RuntimeExtensionRequest, StaticPolicyEngine, SystemClock, TaskIntent, ToolCoreOutcome,
    ToolCoreRequest, ToolExtensionAdapter, ToolExtensionOutcome, ToolExtensionRequest,
    VerticalPackManifest,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use wasmparser::{Parser as WasmParser, Payload as WasmPayload};

const DEFAULT_PACK_ID: &str = "dev-automation";
const DEFAULT_AGENT_ID: &str = "agent-dev-01";
static BUNDLED_APPROVAL_RISK_PROFILE: OnceLock<ApprovalRiskProfile> = OnceLock::new();
static BUNDLED_SECURITY_SCAN_PROFILE: OnceLock<SecurityScanProfile> = OnceLock::new();

#[derive(Parser, Debug)]
#[command(name = "daemon", about = "ChumOS low-level runtime daemon")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the original end-to-end bootstrap demo
    Demo,
    /// Execute one task through the kernel+harness path
    RunTask {
        #[arg(long)]
        objective: String,
        #[arg(long, default_value = "{}")]
        payload: String,
    },
    /// Invoke one connector operation through kernel policy gate
    InvokeConnector {
        #[arg(long)]
        operation: String,
        #[arg(long, default_value = "{}")]
        payload: String,
    },
    /// Demonstrate audit lifecycle with fixed clock and token revocation
    AuditDemo,
    /// Generate a runnable JSON spec template for quick vertical customization
    InitSpec {
        #[arg(long, default_value = "chumos.spec.json")]
        output: String,
    },
    /// Run a full workflow from a JSON spec (task/connector/runtime/tool/memory)
    RunSpec {
        #[arg(long)]
        spec: String,
        #[arg(long, default_value_t = false)]
        print_audit: bool,
    },
}

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
    allowed_process_commands: BTreeSet<String>,
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
        let catalog = self.catalog.lock().map_err(|_| {
            ConnectorError::Execution("integration catalog mutex poisoned".to_owned())
        })?;

        let provider = catalog.provider(&self.provider_id).ok_or_else(|| {
            ConnectorError::Execution(format!(
                "provider {} is not registered in integration catalog",
                self.provider_id
            ))
        })?;

        let requested_channel = command
            .payload
            .get("channel_id")
            .and_then(Value::as_str)
            .map(std::string::ToString::to_string);

        let chosen_channel = if let Some(channel_id) = requested_channel {
            let channel = catalog.channel(&channel_id).ok_or_else(|| {
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

        let operation = command.operation.clone();
        let payload = command.payload.clone();
        let bridge_execution = bridge_execution_payload(
            provider,
            &chosen_channel,
            &command,
            &self.bridge_runtime_policy,
        );

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

fn bridge_execution_payload(
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
}

fn maybe_execute_bridge(
    mut execution: Value,
    bridge_kind: PluginBridgeKind,
    provider: &kernel::ProviderConfig,
    channel: &kernel::ChannelConfig,
    command: &ConnectorCommand,
    runtime_policy: &BridgeRuntimePolicy,
) -> Value {
    if runtime_policy.execute_http_json && matches!(bridge_kind, PluginBridgeKind::HttpJson) {
        execution["status"] = Value::String("deferred".to_owned());
        execution["reason"] = Value::String(
            "http_json active execution is not implemented yet; planned request emitted".to_owned(),
        );
        return execution;
    }

    if runtime_policy.execute_process_stdio && matches!(bridge_kind, PluginBridgeKind::ProcessStdio)
    {
        return execute_process_stdio_bridge(execution, provider, channel, command, runtime_policy);
    }

    execution
}

fn execute_process_stdio_bridge(
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
    let envelope = json!({
        "provider_id": provider.provider_id,
        "channel_id": channel.channel_id,
        "operation": command.operation,
        "payload": command.payload,
    });
    let stdin_payload = match serde_json::to_vec(&envelope) {
        Ok(mut bytes) => {
            bytes.push(b'\n');
            bytes
        }
        Err(error) => {
            execution["status"] = Value::String("failed".to_owned());
            execution["reason"] =
                Value::String(format!("failed to encode process envelope: {error}"));
            return execution;
        }
    };

    let output = (|| -> Result<std::process::Output, String> {
        let mut child = Command::new(&program)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("failed to spawn process command {program}: {error}"))?;

        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(&stdin_payload)
                .map_err(|error| format!("failed to write process stdin payload: {error}"))?;
        }

        child
            .wait_with_output()
            .map_err(|error| format!("failed to wait process output: {error}"))
    })();

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let stdout_json = serde_json::from_str::<Value>(&stdout).unwrap_or(Value::Null);
            let success = output.status.success();

            execution["status"] = Value::String(if success {
                "executed".to_owned()
            } else {
                "failed".to_owned()
            });
            if !success {
                execution["reason"] = Value::String(format!(
                    "process command exited with code {:?}",
                    output.status.code()
                ));
            }
            execution["runtime"] = json!({
                "executor": "process_stdio_local",
                "command": program,
                "args": args,
                "exit_code": output.status.code(),
                "stdout": stdout,
                "stderr": stderr,
                "stdout_json": stdout_json,
            });
            execution
        }
        Err(reason) => {
            execution["status"] = Value::String("failed".to_owned());
            execution["reason"] = Value::String(reason);
            execution
        }
    }
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
        Ok(ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "core-tools",
                "tool_name": request.tool_name,
                "payload": request.payload,
            }),
        })
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
        Ok(MemoryCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "adapter": "kv-core",
                "operation": request.operation,
                "payload": request.payload,
            }),
        })
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

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Demo) {
        Commands::Demo => run_demo().await,
        Commands::RunTask { objective, payload } => run_task_cli(&objective, &payload).await,
        Commands::InvokeConnector { operation, payload } => {
            invoke_connector_cli(&operation, &payload).await;
        }
        Commands::AuditDemo => run_audit_demo().await,
        Commands::InitSpec { output } => init_spec_cli(&output),
        Commands::RunSpec { spec, print_audit } => run_spec_cli(&spec, print_audit).await,
    }
}

async fn run_demo() {
    let kernel = bootstrap_kernel_default();
    let token = kernel
        .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 300)
        .expect("token issue failed");

    let task = TaskIntent {
        task_id: "task-bootstrap-01".to_owned(),
        objective: "summarize flaky test clusters".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::MemoryRead]),
        payload: json!({"repo": "chumyin/ChumOS"}),
    };

    let task_dispatch = kernel
        .execute_task(DEFAULT_PACK_ID, &token, task)
        .await
        .expect("task dispatch failed");

    println!(
        "task dispatched via {:?}: {}",
        task_dispatch.adapter_route.harness_kind, task_dispatch.outcome.output
    );

    let connector_dispatch = kernel
        .invoke_connector(
            DEFAULT_PACK_ID,
            &token,
            ConnectorCommand {
                connector_name: "webhook".to_owned(),
                operation: "notify".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({"channel": "ops-alerts", "message": "task complete"}),
            },
        )
        .await
        .expect("connector dispatch failed");

    println!("connector dispatch: {}", connector_dispatch.outcome.payload);
}

async fn run_task_cli(objective: &str, payload_raw: &str) {
    let payload = parse_json_payload(payload_raw, "run-task payload");

    let kernel = bootstrap_kernel_default();
    let token = kernel
        .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 120)
        .expect("token issue failed");

    let dispatch = kernel
        .execute_task(
            DEFAULT_PACK_ID,
            &token,
            TaskIntent {
                task_id: "task-cli-01".to_owned(),
                objective: objective.to_owned(),
                required_capabilities: BTreeSet::from([
                    Capability::InvokeTool,
                    Capability::MemoryRead,
                ]),
                payload,
            },
        )
        .await
        .expect("task dispatch failed");

    println!(
        "{}",
        serde_json::to_string_pretty(&dispatch.outcome).expect("serialize task outcome")
    );
}

async fn invoke_connector_cli(operation: &str, payload_raw: &str) {
    let payload = parse_json_payload(payload_raw, "invoke-connector payload");

    let kernel = bootstrap_kernel_default();
    let token = kernel
        .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 120)
        .expect("token issue failed");

    let dispatch = kernel
        .invoke_connector(
            DEFAULT_PACK_ID,
            &token,
            ConnectorCommand {
                connector_name: "webhook".to_owned(),
                operation: operation.to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload,
            },
        )
        .await
        .expect("connector dispatch failed");

    println!(
        "{}",
        serde_json::to_string_pretty(&dispatch.outcome).expect("serialize connector outcome")
    );
}

async fn run_audit_demo() {
    let fixed_clock = Arc::new(FixedClock::new(1_700_000_000));
    let audit_sink = Arc::new(InMemoryAuditSink::default());

    let kernel = bootstrap_kernel_with_runtime(fixed_clock.clone(), audit_sink.clone());

    let token = kernel
        .issue_token(DEFAULT_PACK_ID, DEFAULT_AGENT_ID, 30)
        .expect("token issue failed");

    let _ = kernel
        .execute_task(
            DEFAULT_PACK_ID,
            &token,
            TaskIntent {
                task_id: "task-audit-01".to_owned(),
                objective: "produce audit evidence".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeTool]),
                payload: json!({}),
            },
        )
        .await
        .expect("task dispatch failed");

    fixed_clock.advance_by(5);

    let _ = kernel
        .invoke_connector(
            DEFAULT_PACK_ID,
            &token,
            ConnectorCommand {
                connector_name: "webhook".to_owned(),
                operation: "notify".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({"channel": "audit"}),
            },
        )
        .await
        .expect("connector invoke failed");

    kernel
        .revoke_token(&token.token_id, Some(DEFAULT_AGENT_ID))
        .expect("token revoke failed");

    println!(
        "{}",
        serde_json::to_string_pretty(&audit_sink.snapshot()).expect("serialize audit events")
    );
}

fn init_spec_cli(output_path: &str) {
    let spec = RunnerSpec::template();
    write_json_file(output_path, &spec);
    println!("spec template written to {}", output_path);
}

async fn run_spec_cli(spec_path: &str, print_audit: bool) {
    let spec = read_spec_file(spec_path);
    let report = execute_spec(spec, print_audit).await;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("serialize spec run report")
    );
}

async fn execute_spec(spec: RunnerSpec, include_audit: bool) -> SpecRunReport {
    let mut spec = spec;
    let audit_sink = Arc::new(InMemoryAuditSink::default());
    let mut kernel =
        bootstrap_kernel_with_runtime(Arc::new(SystemClock) as Arc<dyn Clock>, audit_sink.clone());

    let mut integration_catalog = default_integration_catalog();
    let mut blocked_reason = None;
    let mut bridge_support_checksum = None;
    let mut bridge_support_sha256 = None;
    let approval_guard = evaluate_approval_guard(&spec);
    let mut self_awareness = None;
    let mut architecture_guard = None;
    let mut plugin_scan_reports = Vec::new();
    let mut plugin_translation_reports = Vec::new();
    let mut plugin_activation_plans = Vec::new();
    let mut plugin_bootstrap_reports = Vec::new();
    let mut plugin_bootstrap_queue = Vec::new();
    let mut plugin_absorb_reports = Vec::new();
    let security_scan_policy = match security_scan_policy(&spec) {
        Ok(policy) => policy,
        Err(error) => {
            blocked_reason = Some(match blocked_reason {
                Some(existing) => format!("{existing}; {error}"),
                None => error,
            });
            None
        }
    };
    let security_process_allowlist = security_scan_process_allowlist(&spec);
    let mut security_scan_report = security_scan_policy
        .as_ref()
        .map(|_| SecurityScanReport::default());
    let mut auto_provision_plan = None;

    if !approval_guard.approved {
        blocked_reason = Some(approval_guard.reason.clone());
    }

    if let Some(bridge) = &spec.bridge_support {
        if bridge.enabled {
            let checksum = bridge_support_policy_checksum(bridge);
            let sha256 = bridge_support_policy_sha256(bridge);
            bridge_support_checksum = Some(checksum.clone());
            bridge_support_sha256 = Some(sha256.clone());

            let version = bridge.policy_version.as_deref().unwrap_or("unknown");
            let mut mismatch_reasons = Vec::new();
            if let Some(expected) = &bridge.expected_checksum {
                if !expected.eq_ignore_ascii_case(&checksum) {
                    mismatch_reasons.push(format!(
                        "bridge support policy checksum mismatch (version {version})"
                    ));
                }
            }
            if let Some(expected_sha256) = &bridge.expected_sha256 {
                if !expected_sha256.eq_ignore_ascii_case(&sha256) {
                    mismatch_reasons.push(format!(
                        "bridge support policy sha256 mismatch (version {version})"
                    ));
                }
            }
            if !mismatch_reasons.is_empty() {
                blocked_reason = Some(mismatch_reasons.join("; "));
            }
        }
    }

    if let Some(self_awareness_spec) = &spec.self_awareness {
        if self_awareness_spec.enabled {
            let mut architecture_policy = ArchitectureBoundaryPolicy::default();
            if !self_awareness_spec.immutable_core_paths.is_empty() {
                architecture_policy.immutable_prefixes = self_awareness_spec
                    .immutable_core_paths
                    .iter()
                    .cloned()
                    .collect();
            }
            if !self_awareness_spec.mutable_extension_paths.is_empty() {
                architecture_policy.mutable_prefixes = self_awareness_spec
                    .mutable_extension_paths
                    .iter()
                    .cloned()
                    .collect();
            }

            let engine = CodebaseAwarenessEngine::new();
            let snapshot = engine
                .snapshot(&CodebaseAwarenessConfig {
                    roots: self_awareness_spec.roots.clone(),
                    plugin_roots: self_awareness_spec.plugin_roots.clone(),
                    proposed_mutations: self_awareness_spec.proposed_mutations.clone(),
                    architecture_policy,
                })
                .expect("self-awareness snapshot failed");
            architecture_guard = Some(snapshot.architecture_guard.clone());
            if self_awareness_spec.enforce_guard && snapshot.architecture_guard.has_denials() {
                blocked_reason = Some(
                    "architecture guard blocked proposed mutations outside mutable boundaries"
                        .to_owned(),
                );
            }
            self_awareness = Some(snapshot);
        }
    }

    if let Some(reason) = blocked_reason.clone() {
        return SpecRunReport {
            pack_id: spec.pack.pack_id,
            agent_id: spec.agent_id,
            operation_kind: "blocked",
            blocked_reason: Some(reason.clone()),
            approval_guard,
            bridge_support_checksum,
            bridge_support_sha256,
            self_awareness,
            architecture_guard,
            plugin_scan_reports,
            plugin_translation_reports,
            plugin_activation_plans,
            plugin_bootstrap_reports,
            plugin_bootstrap_queue,
            plugin_absorb_reports,
            security_scan_report: security_scan_report.clone(),
            auto_provision_plan,
            outcome: json!({
                "status": "blocked",
                "reason": reason,
            }),
            integration_catalog,
            audit_events: if include_audit {
                Some(audit_sink.snapshot())
            } else {
                None
            },
        };
    }

    if let Some(plugin_scan) = &spec.plugin_scan {
        if plugin_scan.enabled {
            let scanner = PluginScanner::new();
            let translator = PluginTranslator::new();
            let bootstrap_executor = PluginBootstrapExecutor::new();
            let bootstrap_policy = bootstrap_policy(&spec);
            let (bridge_matrix, enforce_bridge_support) = bridge_support_matrix(&spec);
            let mut pending_absorb_inputs = Vec::new();
            let mut remaining_bootstrap_budget =
                bootstrap_policy.as_ref().map(|policy| policy.max_tasks);
            for root in &plugin_scan.roots {
                let report = scanner.scan_path(root).expect("plugin scan failed");
                let translation = translator.translate_scan_report(&report);
                let activation = translator.plan_activation(&translation, &bridge_matrix);

                if enforce_bridge_support && activation.has_blockers() {
                    blocked_reason = Some(format!(
                        "bridge support enforcement blocked {} plugin(s)",
                        activation.blocked_plugins
                    ));
                }

                let ready_report = filter_scan_report_by_activation(&report, &activation);
                let mut filtered_report = ready_report.clone();
                if let Some(policy) = bootstrap_policy.as_ref() {
                    let mut effective_policy = policy.clone();
                    if let Some(remaining) = remaining_bootstrap_budget {
                        effective_policy.max_tasks = remaining;
                    }
                    let bootstrap_report =
                        bootstrap_executor.execute(&activation, &effective_policy);
                    if blocked_reason.is_none() && bootstrap_report.blocked {
                        blocked_reason =
                            Some(bootstrap_report.block_reason.clone().unwrap_or_else(|| {
                                "bootstrap policy blocked ready plugins".to_owned()
                            }));
                    }

                    if let Some(remaining) = remaining_bootstrap_budget.as_mut() {
                        *remaining = remaining.saturating_sub(bootstrap_report.applied_tasks);
                    }

                    plugin_bootstrap_queue.extend(
                        bootstrap_report
                            .tasks
                            .iter()
                            .filter(|task| matches!(task.status, BootstrapTaskStatus::Applied))
                            .map(|task| task.bootstrap_hint.clone()),
                    );
                    filtered_report =
                        filter_scan_report_by_keys(&report, &bootstrap_report.applied_plugin_keys);
                    plugin_bootstrap_reports.push(bootstrap_report);
                } else {
                    plugin_bootstrap_queue.extend(
                        activation
                            .candidates
                            .iter()
                            .filter(|candidate| {
                                matches!(candidate.status, PluginActivationStatus::Ready)
                            })
                            .map(|candidate| candidate.bootstrap_hint.clone()),
                    );
                }

                let enriched_ready_report =
                    enrich_scan_report_with_translation(&ready_report, &translation);
                let enriched_filtered_report =
                    enrich_scan_report_with_translation(&filtered_report, &translation);

                if let (Some(policy), Some(report)) =
                    (security_scan_policy.as_ref(), security_scan_report.as_mut())
                {
                    let delta = evaluate_plugin_security_scan(
                        &enriched_ready_report,
                        policy,
                        &security_process_allowlist,
                    );
                    apply_security_scan_delta(report, delta);

                    if blocked_reason.is_none() && report.blocked {
                        blocked_reason = report.block_reason.clone();
                    }
                }

                plugin_translation_reports.push(translation);
                plugin_activation_plans.push(activation);
                plugin_scan_reports.push(report);
                pending_absorb_inputs.push(enriched_filtered_report);

                if blocked_reason.is_some() {
                    break;
                }
            }

            if blocked_reason.is_none() {
                for pending in pending_absorb_inputs {
                    let absorb = scanner.absorb(&mut integration_catalog, &mut spec.pack, &pending);
                    plugin_absorb_reports.push(absorb);
                }
            }
        }
    }

    if let (Some(policy), Some(report)) =
        (security_scan_policy.as_ref(), security_scan_report.as_mut())
    {
        if let Some(export_spec) = policy.siem_export.as_ref().filter(|value| value.enabled) {
            match emit_security_scan_siem_record(
                &spec.pack.pack_id,
                &spec.agent_id,
                report,
                export_spec,
            ) {
                Ok(export_report) => report.siem_export = Some(export_report),
                Err(error) => {
                    report.siem_export = Some(SecuritySiemExportReport {
                        enabled: true,
                        path: export_spec.path.clone(),
                        success: false,
                        exported_records: 0,
                        exported_findings: 0,
                        truncated_findings: 0,
                        error: Some(error.clone()),
                    });
                    if export_spec.fail_on_error && blocked_reason.is_none() {
                        blocked_reason = Some(format!("security scan siem export failed: {error}"));
                    }
                }
            }
        }
    }

    if let Some(report) = security_scan_report.as_ref() {
        if let Err(error) =
            emit_security_scan_audit_event(&kernel, &spec.pack.pack_id, &spec.agent_id, report)
        {
            if blocked_reason.is_none() {
                blocked_reason = Some(error);
            }
        }
    }

    if let Some(reason) = blocked_reason.clone() {
        return SpecRunReport {
            pack_id: spec.pack.pack_id,
            agent_id: spec.agent_id,
            operation_kind: "blocked",
            blocked_reason: Some(reason.clone()),
            approval_guard,
            bridge_support_checksum,
            bridge_support_sha256,
            self_awareness,
            architecture_guard,
            plugin_scan_reports,
            plugin_translation_reports,
            plugin_activation_plans,
            plugin_bootstrap_reports,
            plugin_bootstrap_queue,
            plugin_absorb_reports,
            security_scan_report: security_scan_report.clone(),
            auto_provision_plan,
            outcome: json!({
                "status": "blocked",
                "reason": reason,
            }),
            integration_catalog,
            audit_events: if include_audit {
                Some(audit_sink.snapshot())
            } else {
                None
            },
        };
    }

    if let Some(auto) = &spec.auto_provision {
        if auto.enabled {
            let agent = AutoProvisionAgent::new();
            let connector_name = auto
                .connector_name
                .clone()
                .or_else(|| operation_connector_name(&spec.operation));
            let request = AutoProvisionRequest {
                provider_id: auto.provider_id.clone(),
                channel_id: auto.channel_id.clone(),
                connector_name,
                endpoint: auto.endpoint.clone(),
                required_capabilities: auto.required_capabilities.clone(),
            };

            let plan = agent
                .plan(&integration_catalog, &spec.pack, &request)
                .expect("auto-provision planning failed");
            if !plan.is_noop() {
                integration_catalog
                    .apply_plan(&mut spec.pack, &plan)
                    .expect("applying auto-provision plan failed");
                auto_provision_plan = Some(plan);
            }
        }
    }

    for hotfix in &spec.hotfixes {
        integration_catalog
            .apply_hotfix(&hotfix.to_kernel_hotfix())
            .expect("hotfix application failed");
    }

    let shared_catalog = Arc::new(Mutex::new(integration_catalog.clone()));
    let bridge_runtime_policy = bridge_runtime_policy(&spec);
    register_dynamic_catalog_connectors(&mut kernel, shared_catalog, bridge_runtime_policy);

    kernel
        .register_pack(spec.pack.clone())
        .expect("spec pack registration failed");
    apply_default_selection(&mut kernel, spec.defaults.as_ref());

    let token = kernel
        .issue_token(&spec.pack.pack_id, &spec.agent_id, spec.ttl_s)
        .expect("token issue for spec failed");

    let (operation_kind, outcome) =
        execute_spec_operation(&kernel, &spec.pack.pack_id, &token, &spec.operation).await;

    SpecRunReport {
        pack_id: spec.pack.pack_id,
        agent_id: spec.agent_id,
        operation_kind,
        blocked_reason,
        approval_guard,
        bridge_support_checksum,
        bridge_support_sha256,
        self_awareness,
        architecture_guard,
        plugin_scan_reports,
        plugin_translation_reports,
        plugin_activation_plans,
        plugin_bootstrap_reports,
        plugin_bootstrap_queue,
        plugin_absorb_reports,
        security_scan_report,
        auto_provision_plan,
        outcome,
        integration_catalog,
        audit_events: if include_audit {
            Some(audit_sink.snapshot())
        } else {
            None
        },
    }
}

#[derive(Debug, Default)]
struct SecurityScanDelta {
    scanned_plugins: usize,
    high_findings: usize,
    medium_findings: usize,
    low_findings: usize,
    findings: Vec<SecurityFinding>,
    block_reason: Option<String>,
}

fn security_scan_policy(spec: &RunnerSpec) -> Result<Option<SecurityScanSpec>, String> {
    let Some(mut policy) = spec
        .bridge_support
        .as_ref()
        .filter(|bridge| bridge.enabled)
        .and_then(|bridge| bridge.security_scan.clone())
    else {
        return Ok(None);
    };

    if !policy.enabled {
        return Ok(None);
    }

    validate_security_scan_policy(&policy)?;

    let profile = resolve_security_scan_profile(&policy)?;

    if policy.high_risk_metadata_keywords.is_empty() {
        policy.high_risk_metadata_keywords = profile.high_risk_metadata_keywords;
    }

    if policy.wasm.blocked_import_prefixes.is_empty() {
        policy.wasm.blocked_import_prefixes = profile.wasm.blocked_import_prefixes;
    }

    if policy.wasm.max_module_bytes == 0 {
        policy.wasm.max_module_bytes = profile.wasm.max_module_bytes;
    }

    if policy.wasm.allowed_path_prefixes.is_empty() {
        policy.wasm.allowed_path_prefixes = profile.wasm.allowed_path_prefixes;
    }

    if policy.wasm.required_sha256_by_plugin.is_empty() {
        policy.wasm.required_sha256_by_plugin = profile.wasm.required_sha256_by_plugin;
    }

    Ok(Some(policy))
}

fn validate_security_scan_policy(policy: &SecurityScanSpec) -> Result<(), String> {
    if policy.profile_sha256.is_some() && policy.profile_path.is_none() {
        return Err(
            "security scan profile_sha256 requires security_scan.profile_path to be set".to_owned(),
        );
    }
    if policy.profile_signature.is_some() && policy.profile_path.is_none() {
        return Err(
            "security scan profile_signature requires security_scan.profile_path to be set"
                .to_owned(),
        );
    }
    if let Some(signature) = policy.profile_signature.as_ref() {
        if signature.public_key_base64.trim().is_empty() {
            return Err(
                "security scan profile_signature.public_key_base64 cannot be empty".to_owned(),
            );
        }
        if signature.signature_base64.trim().is_empty() {
            return Err(
                "security scan profile_signature.signature_base64 cannot be empty".to_owned(),
            );
        }
    }
    if let Some(export) = policy.siem_export.as_ref().filter(|value| value.enabled) {
        if export.path.trim().is_empty() {
            return Err("security scan siem_export.path cannot be empty when enabled".to_owned());
        }
    }
    Ok(())
}

fn resolve_security_scan_profile(policy: &SecurityScanSpec) -> Result<SecurityScanProfile, String> {
    if let Some(path) = policy.profile_path.as_deref() {
        let profile = load_security_scan_profile_from_path(path);
        match profile {
            Ok(profile) => {
                if let Some(expected_sha256) = policy.profile_sha256.as_deref() {
                    let actual_sha256 = security_scan_profile_sha256(&profile);
                    if !expected_sha256.eq_ignore_ascii_case(&actual_sha256) {
                        return Err(format!(
                            "security scan profile sha256 mismatch for {path}: expected {expected_sha256}, actual {actual_sha256}",
                        ));
                    }
                }
                if let Some(signature) = policy.profile_signature.as_ref() {
                    verify_security_scan_profile_signature(&profile, signature).map_err(|error| {
                        format!(
                            "security scan profile signature verification failed for {path}: {error}"
                        )
                    })?;
                }
                return Ok(profile);
            }
            Err(error) if policy.profile_sha256.is_some() || policy.profile_signature.is_some() => {
                return Err(format!(
                    "failed to load security scan profile at {path} while profile integrity is pinned: {error}",
                ));
            }
            Err(_) => {}
        }
    }

    Ok(bundled_security_scan_profile())
}

fn load_security_scan_profile_from_path(path: &str) -> Result<SecurityScanProfile, String> {
    let content =
        fs::read_to_string(path).map_err(|error| format!("read profile failed: {error}"))?;
    serde_json::from_str::<SecurityScanProfile>(&content)
        .map_err(|error| format!("parse profile failed: {error}"))
}

fn bundled_security_scan_profile() -> SecurityScanProfile {
    BUNDLED_SECURITY_SCAN_PROFILE
        .get_or_init(|| {
            let raw = include_str!("../config/security-scan-medium-balanced.json");
            serde_json::from_str(raw).unwrap_or_else(|error| {
                panic!("invalid bundled security scan profile config: {error}");
            })
        })
        .clone()
}

fn verify_security_scan_profile_signature(
    profile: &SecurityScanProfile,
    signature: &SecurityProfileSignatureSpec,
) -> Result<(), String> {
    let algorithm = signature.algorithm.trim().to_ascii_lowercase();
    if algorithm != "ed25519" {
        return Err(format!(
            "unsupported profile signature algorithm: {algorithm} (expected ed25519)"
        ));
    }

    let public_key_bytes = BASE64_STANDARD
        .decode(signature.public_key_base64.trim())
        .map_err(|error| format!("invalid public_key_base64: {error}"))?;
    let public_key_bytes: [u8; 32] = public_key_bytes
        .as_slice()
        .try_into()
        .map_err(|_| "invalid ed25519 public key length (expected 32 bytes)".to_owned())?;
    let verifying_key = VerifyingKey::from_bytes(&public_key_bytes)
        .map_err(|error| format!("invalid ed25519 public key bytes: {error}"))?;

    let signature_bytes = BASE64_STANDARD
        .decode(signature.signature_base64.trim())
        .map_err(|error| format!("invalid signature_base64: {error}"))?;
    let signature_bytes: [u8; 64] = signature_bytes
        .as_slice()
        .try_into()
        .map_err(|_| "invalid ed25519 signature length (expected 64 bytes)".to_owned())?;
    let signature = Ed25519Signature::from_bytes(&signature_bytes);

    let message = security_scan_profile_message(profile);
    verifying_key
        .verify(&message, &signature)
        .map_err(|error| format!("ed25519 verification failed: {error}"))
}

fn security_scan_process_allowlist(spec: &RunnerSpec) -> BTreeSet<String> {
    spec.bridge_support
        .as_ref()
        .filter(|bridge| bridge.enabled)
        .map(|bridge| {
            bridge
                .allowed_process_commands
                .iter()
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| !value.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn apply_security_scan_delta(report: &mut SecurityScanReport, delta: SecurityScanDelta) {
    report.scanned_plugins = report.scanned_plugins.saturating_add(delta.scanned_plugins);
    report.high_findings = report.high_findings.saturating_add(delta.high_findings);
    report.medium_findings = report.medium_findings.saturating_add(delta.medium_findings);
    report.low_findings = report.low_findings.saturating_add(delta.low_findings);
    report.total_findings = report
        .high_findings
        .saturating_add(report.medium_findings)
        .saturating_add(report.low_findings);
    report.findings.extend(delta.findings);
    if let Some(reason) = delta.block_reason {
        report.blocked = true;
        report.block_reason = Some(reason);
    }
}

fn emit_security_scan_audit_event(
    kernel: &ChumosKernel<StaticPolicyEngine>,
    pack_id: &str,
    agent_id: &str,
    report: &SecurityScanReport,
) -> Result<(), String> {
    if report.scanned_plugins == 0 && report.total_findings == 0 {
        return Ok(());
    }

    let categories: Vec<String> = report
        .findings
        .iter()
        .map(|finding| finding.category.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let finding_ids: Vec<String> = report
        .findings
        .iter()
        .map(|finding| finding.correlation_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    kernel
        .record_audit_event(
            Some(agent_id),
            AuditEventKind::SecurityScanEvaluated {
                pack_id: pack_id.to_owned(),
                scanned_plugins: report.scanned_plugins,
                total_findings: report.total_findings,
                high_findings: report.high_findings,
                medium_findings: report.medium_findings,
                low_findings: report.low_findings,
                blocked: report.blocked,
                block_reason: report.block_reason.clone(),
                categories,
                finding_ids,
            },
        )
        .map_err(|error| format!("failed to record security scan audit event: {error}"))
}

fn emit_security_scan_siem_record(
    pack_id: &str,
    agent_id: &str,
    report: &SecurityScanReport,
    export: &SecuritySiemExportSpec,
) -> Result<SecuritySiemExportReport, String> {
    let mut findings = report.findings.clone();
    let mut truncated_findings = 0usize;

    if export.include_findings {
        if let Some(limit) = export.max_findings_per_record {
            if findings.len() > limit {
                truncated_findings = findings.len().saturating_sub(limit);
                findings.truncate(limit);
            }
        }
    } else {
        truncated_findings = report.findings.len();
        findings.clear();
    }

    let categories: Vec<String> = report
        .findings
        .iter()
        .map(|finding| finding.category.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let finding_ids: Vec<String> = report
        .findings
        .iter()
        .map(|finding| finding.correlation_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let record = json!({
        "event_type": "security_scan_report",
        "ts_epoch_s": current_epoch_s(),
        "pack_id": pack_id,
        "agent_id": agent_id,
        "blocked": report.blocked,
        "block_reason": report.block_reason.clone(),
        "counts": {
            "scanned_plugins": report.scanned_plugins,
            "total_findings": report.total_findings,
            "high_findings": report.high_findings,
            "medium_findings": report.medium_findings,
            "low_findings": report.low_findings,
        },
        "categories": categories,
        "finding_ids": finding_ids,
        "truncated_findings": truncated_findings,
        "findings": findings,
    });

    let path = Path::new(&export.path);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create siem export directory failed: {error}"))?;
        }
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("open siem export file failed: {error}"))?;
    serde_json::to_writer(&mut file, &record)
        .map_err(|error| format!("serialize siem export record failed: {error}"))?;
    file.write_all(b"\n")
        .map_err(|error| format!("flush siem export record failed: {error}"))?;

    Ok(SecuritySiemExportReport {
        enabled: true,
        path: export.path.clone(),
        success: true,
        exported_records: 1,
        exported_findings: findings.len(),
        truncated_findings,
        error: None,
    })
}

fn build_security_finding(
    severity: SecurityFindingSeverity,
    category: impl Into<String>,
    plugin_id: impl Into<String>,
    source_path: impl Into<String>,
    message: impl Into<String>,
    evidence: Value,
) -> SecurityFinding {
    let category = category.into();
    let plugin_id = plugin_id.into();
    let source_path = source_path.into();
    let message = message.into();
    let correlation_id = security_finding_correlation_id(
        &severity,
        &category,
        &plugin_id,
        &source_path,
        &message,
        &evidence,
    );

    SecurityFinding {
        correlation_id,
        severity,
        category,
        plugin_id,
        source_path,
        message,
        evidence,
    }
}

fn security_finding_correlation_id(
    severity: &SecurityFindingSeverity,
    category: &str,
    plugin_id: &str,
    source_path: &str,
    message: &str,
    evidence: &Value,
) -> String {
    let canonical = json!({
        "severity": severity,
        "category": category,
        "plugin_id": plugin_id,
        "source_path": source_path,
        "message": message,
        "evidence": evidence,
    });
    let payload =
        serde_json::to_vec(&canonical).expect("serialize security finding correlation payload");
    let digest = Sha256::digest(&payload);
    let full = hex_lower(&digest);
    format!("sf-{}", &full[..16])
}

fn evaluate_plugin_security_scan(
    report: &PluginScanReport,
    policy: &SecurityScanSpec,
    process_allowlist: &BTreeSet<String>,
) -> SecurityScanDelta {
    let mut delta = SecurityScanDelta::default();
    let metadata_keywords = normalize_signal_list(policy.high_risk_metadata_keywords.clone());
    let blocked_import_prefixes =
        normalize_signal_list(policy.wasm.blocked_import_prefixes.clone());
    let allowed_path_prefixes = normalize_allowed_path_prefixes(&policy.wasm.allowed_path_prefixes);

    for descriptor in &report.descriptors {
        delta.scanned_plugins = delta.scanned_plugins.saturating_add(1);
        let bridge_kind = descriptor_bridge_kind(descriptor);
        let metadata_finding = scan_descriptor_metadata_keywords(descriptor, &metadata_keywords);
        accumulate_security_findings(&mut delta, metadata_finding);

        match bridge_kind {
            PluginBridgeKind::ProcessStdio => {
                let findings = scan_process_stdio_security(descriptor, process_allowlist);
                accumulate_security_findings(&mut delta, findings);
            }
            PluginBridgeKind::NativeFfi => {
                let finding = build_security_finding(
                    SecurityFindingSeverity::Medium,
                    "native_ffi_review",
                    descriptor.manifest.plugin_id.clone(),
                    descriptor.path.clone(),
                    "native_ffi plugin requires manual review and stronger sandboxing",
                    json!({
                        "bridge_kind": bridge_kind.as_str(),
                        "recommendation": "prefer wasm_component for untrusted community plugins",
                    }),
                );
                accumulate_security_findings(&mut delta, vec![finding]);
            }
            PluginBridgeKind::WasmComponent if policy.wasm.enabled => {
                let findings = scan_wasm_plugin_security(
                    descriptor,
                    &policy.wasm,
                    &blocked_import_prefixes,
                    &allowed_path_prefixes,
                );
                accumulate_security_findings(&mut delta, findings);
            }
            PluginBridgeKind::HttpJson
            | PluginBridgeKind::McpServer
            | PluginBridgeKind::Unknown
            | PluginBridgeKind::WasmComponent => {}
        }
    }

    if policy.block_on_high && delta.high_findings > 0 {
        delta.block_reason = Some(format!(
            "security scan blocked {} high-risk finding(s)",
            delta.high_findings
        ));
    }

    delta
}

fn accumulate_security_findings(delta: &mut SecurityScanDelta, findings: Vec<SecurityFinding>) {
    for finding in findings {
        match finding.severity {
            SecurityFindingSeverity::High => {
                delta.high_findings = delta.high_findings.saturating_add(1)
            }
            SecurityFindingSeverity::Medium => {
                delta.medium_findings = delta.medium_findings.saturating_add(1)
            }
            SecurityFindingSeverity::Low => {
                delta.low_findings = delta.low_findings.saturating_add(1)
            }
        }
        delta.findings.push(finding);
    }
}

fn scan_descriptor_metadata_keywords(
    descriptor: &PluginDescriptor,
    keywords: &[String],
) -> Vec<SecurityFinding> {
    if keywords.is_empty() {
        return Vec::new();
    }

    let mut haystack_parts = Vec::new();
    for (key, value) in &descriptor.manifest.metadata {
        haystack_parts.push(key.to_ascii_lowercase());
        haystack_parts.push(value.to_ascii_lowercase());
    }
    let haystack = haystack_parts.join(" ");

    keywords
        .iter()
        .filter(|keyword| haystack.contains(keyword.as_str()))
        .map(|keyword| {
            build_security_finding(
                SecurityFindingSeverity::Medium,
                "metadata_keyword",
                descriptor.manifest.plugin_id.clone(),
                descriptor.path.clone(),
                format!("metadata contains high-risk keyword: {keyword}"),
                json!({
                    "keyword": keyword,
                    "metadata": descriptor.manifest.metadata.clone(),
                }),
            )
        })
        .collect()
}

fn scan_process_stdio_security(
    descriptor: &PluginDescriptor,
    process_allowlist: &BTreeSet<String>,
) -> Vec<SecurityFinding> {
    let mut findings = Vec::new();
    let command = descriptor.manifest.metadata.get("command").cloned();

    match command {
        Some(command) => {
            if !is_process_command_allowed(&command, process_allowlist) {
                findings.push(build_security_finding(
                    SecurityFindingSeverity::High,
                    "process_command_not_allowlisted",
                    descriptor.manifest.plugin_id.clone(),
                    descriptor.path.clone(),
                    format!("process_stdio command {command} is not in runtime allowlist"),
                    json!({
                        "command": command,
                        "allowlist": process_allowlist,
                    }),
                ));
            }
        }
        None => findings.push(build_security_finding(
            SecurityFindingSeverity::Medium,
            "process_command_missing",
            descriptor.manifest.plugin_id.clone(),
            descriptor.path.clone(),
            "process_stdio plugin does not declare metadata.command",
            json!({
                "recommendation": "declare a fixed command and keep bridge allowlist strict",
            }),
        )),
    }

    findings
}

fn scan_wasm_plugin_security(
    descriptor: &PluginDescriptor,
    policy: &WasmSecurityScanSpec,
    blocked_import_prefixes: &[String],
    allowed_path_prefixes: &[PathBuf],
) -> Vec<SecurityFinding> {
    let mut findings = Vec::new();
    let artifact = descriptor_wasm_artifact(descriptor);
    let Some(raw_artifact) = artifact else {
        findings.push(build_security_finding(
            SecurityFindingSeverity::High,
            "wasm_artifact_missing",
            descriptor.manifest.plugin_id.clone(),
            descriptor.path.clone(),
            "wasm plugin does not declare metadata.component/metadata.wasm_path/endpoint artifact",
            json!({}),
        ));
        return findings;
    };

    if raw_artifact.starts_with("http://") || raw_artifact.starts_with("https://") {
        findings.push(build_security_finding(
            SecurityFindingSeverity::High,
            "wasm_remote_artifact",
            descriptor.manifest.plugin_id.clone(),
            descriptor.path.clone(),
            "remote wasm artifact cannot be statically verified for local hotplug safety",
            json!({
                "artifact": raw_artifact,
            }),
        ));
        return findings;
    }

    let artifact_path = resolve_plugin_relative_path(&descriptor.path, &raw_artifact);
    let normalized_artifact_path = normalize_path_for_policy(&artifact_path);
    if !allowed_path_prefixes.is_empty()
        && !allowed_path_prefixes
            .iter()
            .any(|prefix| normalized_artifact_path.starts_with(prefix))
    {
        findings.push(build_security_finding(
            SecurityFindingSeverity::High,
            "wasm_artifact_path_not_allowed",
            descriptor.manifest.plugin_id.clone(),
            descriptor.path.clone(),
            "wasm artifact path is outside allowed_path_prefixes",
            json!({
                "artifact_path": normalized_artifact_path.display().to_string(),
                "allowed_path_prefixes": allowed_path_prefixes
                    .iter()
                    .map(|prefix| prefix.display().to_string())
                    .collect::<Vec<_>>(),
            }),
        ));
        return findings;
    }

    let bytes = match fs::read(&normalized_artifact_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            findings.push(build_security_finding(
                SecurityFindingSeverity::High,
                "wasm_artifact_unreadable",
                descriptor.manifest.plugin_id.clone(),
                descriptor.path.clone(),
                "wasm artifact cannot be read from filesystem",
                json!({
                    "artifact_path": normalized_artifact_path.display().to_string(),
                    "error": error.to_string(),
                }),
            ));
            return findings;
        }
    };

    if bytes.len() > policy.max_module_bytes {
        findings.push(build_security_finding(
            SecurityFindingSeverity::High,
            "wasm_module_too_large",
            descriptor.manifest.plugin_id.clone(),
            descriptor.path.clone(),
            format!(
                "wasm module size {} exceeds max_module_bytes {}",
                bytes.len(),
                policy.max_module_bytes
            ),
            json!({
                "artifact_path": normalized_artifact_path.display().to_string(),
                "module_size_bytes": bytes.len(),
                "max_module_bytes": policy.max_module_bytes,
            }),
        ));
    }

    if !bytes.starts_with(&[0x00, 0x61, 0x73, 0x6d]) {
        findings.push(build_security_finding(
            SecurityFindingSeverity::High,
            "wasm_magic_header_invalid",
            descriptor.manifest.plugin_id.clone(),
            descriptor.path.clone(),
            "artifact does not contain valid wasm magic header",
            json!({
                "artifact_path": normalized_artifact_path.display().to_string(),
            }),
        ));
        return findings;
    }

    let digest = Sha256::digest(&bytes);
    let digest_hex = hex_lower(&digest);

    if let Some(expected) = policy
        .required_sha256_by_plugin
        .get(&descriptor.manifest.plugin_id)
    {
        if !expected.eq_ignore_ascii_case(&digest_hex) {
            findings.push(build_security_finding(
                SecurityFindingSeverity::High,
                "wasm_sha256_mismatch",
                descriptor.manifest.plugin_id.clone(),
                descriptor.path.clone(),
                "wasm sha256 does not match required pin",
                json!({
                    "expected_sha256": expected,
                    "actual_sha256": digest_hex,
                }),
            ));
        }
    } else if policy.require_hash_pin {
        findings.push(build_security_finding(
            SecurityFindingSeverity::High,
            "wasm_sha256_pin_missing",
            descriptor.manifest.plugin_id.clone(),
            descriptor.path.clone(),
            "wasm hash pin is required but missing for plugin",
            json!({
                "required_sha256_by_plugin": policy.required_sha256_by_plugin,
            }),
        ));
    }

    let imports = match parse_wasm_import_modules(&bytes) {
        Ok(imports) => imports,
        Err(error) => {
            findings.push(build_security_finding(
                SecurityFindingSeverity::High,
                "wasm_parse_failed",
                descriptor.manifest.plugin_id.clone(),
                descriptor.path.clone(),
                "wasm parser failed while reading module imports",
                json!({
                    "artifact_path": normalized_artifact_path.display().to_string(),
                    "error": error,
                }),
            ));
            return findings;
        }
    };

    for module_name in &imports {
        let module_name_lower = module_name.to_ascii_lowercase();
        if !policy.allow_wasi && module_name_lower.starts_with("wasi") {
            findings.push(build_security_finding(
                SecurityFindingSeverity::High,
                "wasm_wasi_import_blocked",
                descriptor.manifest.plugin_id.clone(),
                descriptor.path.clone(),
                "wasi import is blocked by wasm security policy",
                json!({
                    "import_module": module_name,
                }),
            ));
        }
        if blocked_import_prefixes
            .iter()
            .any(|prefix| module_name_lower.starts_with(prefix))
        {
            findings.push(build_security_finding(
                SecurityFindingSeverity::High,
                "wasm_import_prefix_blocked",
                descriptor.manifest.plugin_id.clone(),
                descriptor.path.clone(),
                "wasm import module matched blocked prefix",
                json!({
                    "import_module": module_name,
                    "blocked_import_prefixes": blocked_import_prefixes,
                }),
            ));
        }
    }

    findings.push(build_security_finding(
        SecurityFindingSeverity::Low,
        "wasm_digest_observed",
        descriptor.manifest.plugin_id.clone(),
        descriptor.path.clone(),
        "wasm artifact digest captured for audit",
        json!({
            "artifact_path": normalized_artifact_path.display().to_string(),
            "sha256": digest_hex,
            "imports": imports,
        }),
    ));

    findings
}

fn parse_wasm_import_modules(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut modules = Vec::new();
    for payload in WasmParser::new(0).parse_all(bytes) {
        match payload {
            Ok(WasmPayload::ImportSection(section)) => {
                for import in section {
                    let import = import.map_err(|error| error.to_string())?;
                    modules.push(import.module.to_owned());
                }
            }
            Ok(_) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(modules)
}

fn descriptor_wasm_artifact(descriptor: &PluginDescriptor) -> Option<String> {
    descriptor
        .manifest
        .metadata
        .get("component")
        .cloned()
        .or_else(|| descriptor.manifest.metadata.get("wasm_path").cloned())
        .or_else(|| {
            descriptor
                .manifest
                .endpoint
                .clone()
                .filter(|value| value.to_ascii_lowercase().ends_with(".wasm"))
        })
}

fn resolve_plugin_relative_path(source_path: &str, artifact: &str) -> PathBuf {
    let candidate = PathBuf::from(artifact);
    if candidate.is_absolute() {
        return candidate;
    }

    let source = Path::new(source_path);
    if let Some(parent) = source.parent() {
        parent.join(candidate)
    } else {
        candidate
    }
}

fn normalize_allowed_path_prefixes(prefixes: &[String]) -> Vec<PathBuf> {
    prefixes
        .iter()
        .map(|prefix| normalize_path_for_policy(&PathBuf::from(prefix)))
        .collect()
}

fn normalize_path_for_policy(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }

    if path.is_absolute() {
        return path.to_path_buf();
    }

    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

fn descriptor_bridge_kind(descriptor: &PluginDescriptor) -> PluginBridgeKind {
    if let Some(raw) = descriptor.manifest.metadata.get("bridge_kind") {
        if let Some(kind) = parse_bridge_kind_label(raw) {
            return kind;
        }
    }

    let language = descriptor.language.trim().to_ascii_lowercase();
    match language.as_str() {
        "wasm" | "wat" => return PluginBridgeKind::WasmComponent,
        "rust" | "go" | "c" | "cpp" | "cxx" => return PluginBridgeKind::NativeFfi,
        "python" | "javascript" | "typescript" | "java" => return PluginBridgeKind::ProcessStdio,
        _ => {}
    }

    if let Some(endpoint) = descriptor.manifest.endpoint.as_deref() {
        let endpoint_lower = endpoint.to_ascii_lowercase();
        if endpoint_lower.starts_with("http://") || endpoint_lower.starts_with("https://") {
            return PluginBridgeKind::HttpJson;
        }
        if endpoint_lower.ends_with(".wasm") {
            return PluginBridgeKind::WasmComponent;
        }
    }

    PluginBridgeKind::Unknown
}

fn bridge_support_matrix(spec: &RunnerSpec) -> (BridgeSupportMatrix, bool) {
    match &spec.bridge_support {
        Some(bridge) if bridge.enabled => {
            let mut matrix = BridgeSupportMatrix::default();
            if !bridge.supported_bridges.is_empty() {
                matrix.supported_bridges = bridge.supported_bridges.iter().copied().collect();
            }
            if !bridge.supported_adapter_families.is_empty() {
                matrix.supported_adapter_families =
                    bridge.supported_adapter_families.iter().cloned().collect();
            }
            (matrix, bridge.enforce_supported)
        }
        _ => (BridgeSupportMatrix::default(), false),
    }
}

fn bridge_runtime_policy(spec: &RunnerSpec) -> BridgeRuntimePolicy {
    let Some(bridge) = &spec.bridge_support else {
        return BridgeRuntimePolicy::default();
    };
    if !bridge.enabled {
        return BridgeRuntimePolicy::default();
    }

    BridgeRuntimePolicy {
        execute_process_stdio: bridge.execute_process_stdio,
        execute_http_json: bridge.execute_http_json,
        allowed_process_commands: bridge
            .allowed_process_commands
            .iter()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .collect(),
        enforce_execution_success: bridge.enforce_execution_success,
    }
}

fn evaluate_approval_guard(spec: &RunnerSpec) -> ApprovalDecisionReport {
    let policy = spec.approval.clone().unwrap_or_default();
    let now_epoch_s = current_epoch_s();
    let operation_key = operation_approval_key(&spec.operation);
    let operation_kind = operation_approval_kind(&spec.operation);
    let target_in_scope = is_operation_in_approval_scope(&spec.operation, policy.scope);
    let denylisted = is_operation_preapproved(&operation_key, &policy.denied_calls);

    let (risk_level, matched_keywords, risk_score) =
        match operation_risk_profile(&spec.operation, &policy) {
            (ApprovalRiskLevel::High, matched, score) => (ApprovalRiskLevel::High, matched, score),
            (_, _, score) => (ApprovalRiskLevel::Low, Vec::new(), score),
        };

    if denylisted {
        return ApprovalDecisionReport {
            mode: policy.mode,
            strategy: policy.strategy,
            scope: policy.scope,
            now_epoch_s,
            operation_key,
            operation_kind,
            risk_level,
            risk_score,
            denylisted: true,
            requires_human_approval: true,
            approved: false,
            reason: "operation is denylisted by human approval policy".to_owned(),
            matched_keywords,
        };
    }

    let one_time_full_access_active = policy.one_time_full_access_granted
        && policy
            .one_time_full_access_expires_at_epoch_s
            .map(|deadline| now_epoch_s <= deadline)
            .unwrap_or(true)
        && policy
            .one_time_full_access_remaining_uses
            .map(|remaining| remaining > 0)
            .unwrap_or(true);

    let one_time_full_access_rejected_reason = if policy.one_time_full_access_granted {
        if let Some(deadline) = policy.one_time_full_access_expires_at_epoch_s {
            if now_epoch_s > deadline {
                Some(format!(
                    "one-time full access grant expired at {deadline}, now is {now_epoch_s}"
                ))
            } else {
                None
            }
        } else if matches!(policy.one_time_full_access_remaining_uses, Some(0)) {
            Some("one-time full access grant has no remaining uses".to_owned())
        } else {
            None
        }
    } else {
        None
    };

    let requires_human_approval = if !target_in_scope {
        false
    } else {
        match policy.mode {
            HumanApprovalMode::Disabled => false,
            HumanApprovalMode::MediumBalanced => matches!(risk_level, ApprovalRiskLevel::High),
            HumanApprovalMode::Strict => true,
        }
    };

    let (approved, reason) = if !requires_human_approval {
        (
            true,
            "operation is allowed by default medium-balanced approval policy".to_owned(),
        )
    } else {
        match policy.strategy {
            HumanApprovalStrategy::OneTimeFullAccess if one_time_full_access_active => (
                true,
                "human granted one-time full access for this execution".to_owned(),
            ),
            HumanApprovalStrategy::PerCall
                if is_operation_preapproved(&operation_key, &policy.approved_calls) =>
            {
                (
                    true,
                    format!("operation {operation_key} is pre-approved by human policy"),
                )
            }
            HumanApprovalStrategy::PerCall => (
                false,
                format!(
                    "human approval required for high-risk operation {operation_key}; \
                     add to approval.approved_calls or switch to one_time_full_access"
                ),
            ),
            HumanApprovalStrategy::OneTimeFullAccess => (false, one_time_full_access_rejected_reason
                .unwrap_or_else(|| {
                    format!(
                        "human one-time full access is not granted for high-risk operation {operation_key}"
                    )
                })),
        }
    };

    ApprovalDecisionReport {
        mode: policy.mode,
        strategy: policy.strategy,
        scope: policy.scope,
        now_epoch_s,
        operation_key,
        operation_kind,
        risk_level,
        risk_score,
        denylisted: false,
        requires_human_approval,
        approved,
        reason,
        matched_keywords,
    }
}

fn operation_approval_key(operation: &OperationSpec) -> String {
    match operation {
        OperationSpec::Task { task_id, .. } => format!("task:{task_id}"),
        OperationSpec::ConnectorLegacy {
            connector_name,
            operation,
            ..
        } => {
            format!("connector_legacy:{connector_name}:{operation}")
        }
        OperationSpec::ConnectorCore {
            connector_name,
            operation,
            ..
        } => {
            format!("connector_core:{connector_name}:{operation}")
        }
        OperationSpec::ConnectorExtension {
            connector_name,
            operation,
            extension,
            ..
        } => {
            format!("connector_extension:{extension}:{connector_name}:{operation}")
        }
        OperationSpec::RuntimeCore { action, .. } => format!("runtime_core:{action}"),
        OperationSpec::RuntimeExtension {
            extension, action, ..
        } => {
            format!("runtime_extension:{extension}:{action}")
        }
        OperationSpec::ToolCore { tool_name, .. } => format!("tool_core:{tool_name}"),
        OperationSpec::ToolExtension {
            extension,
            extension_action,
            ..
        } => {
            format!("tool_extension:{extension}:{extension_action}")
        }
        OperationSpec::MemoryCore { operation, .. } => format!("memory_core:{operation}"),
        OperationSpec::MemoryExtension {
            extension,
            operation,
            ..
        } => {
            format!("memory_extension:{extension}:{operation}")
        }
    }
}

fn operation_approval_kind(operation: &OperationSpec) -> &'static str {
    match operation {
        OperationSpec::Task { .. } => "task",
        OperationSpec::ConnectorLegacy { .. } => "connector_legacy",
        OperationSpec::ConnectorCore { .. } => "connector_core",
        OperationSpec::ConnectorExtension { .. } => "connector_extension",
        OperationSpec::RuntimeCore { .. } => "runtime_core",
        OperationSpec::RuntimeExtension { .. } => "runtime_extension",
        OperationSpec::ToolCore { .. } => "tool_core",
        OperationSpec::ToolExtension { .. } => "tool_extension",
        OperationSpec::MemoryCore { .. } => "memory_core",
        OperationSpec::MemoryExtension { .. } => "memory_extension",
    }
}

fn is_operation_in_approval_scope(operation: &OperationSpec, scope: HumanApprovalScope) -> bool {
    match scope {
        HumanApprovalScope::ToolCalls => matches!(
            operation,
            OperationSpec::ToolCore { .. } | OperationSpec::ToolExtension { .. }
        ),
        HumanApprovalScope::AllOperations => true,
    }
}

fn operation_risk_profile(
    operation: &OperationSpec,
    policy: &HumanApprovalSpec,
) -> (ApprovalRiskLevel, Vec<String>, u8) {
    let profile = resolve_approval_risk_profile(policy);
    let keywords = normalize_signal_list(profile.high_risk_keywords);
    let high_risk_tool_names = normalize_signal_list(profile.high_risk_tool_names);
    let high_risk_payload_keys = normalize_signal_list(profile.high_risk_payload_keys);
    let scoring = sanitize_risk_scoring(profile.scoring);

    let haystack = operation_risk_haystack(operation);
    let haystack_lower = haystack.to_ascii_lowercase();

    let matched_keywords: Vec<String> = keywords
        .iter()
        .filter(|keyword| haystack_lower.contains(keyword.as_str()))
        .cloned()
        .collect();

    let matched_tool_name = operation_tool_name(operation)
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| high_risk_tool_names.iter().any(|value| value == name))
        .map(|name| vec![format!("tool:{name}")])
        .unwrap_or_default();

    let payload_keys = operation_payload_keys(operation);
    let matched_payload_keys: Vec<String> = payload_keys
        .iter()
        .map(|key| key.trim().to_ascii_lowercase())
        .filter(|key| high_risk_payload_keys.iter().any(|value| value == key))
        .map(|key| format!("payload_key:{key}"))
        .collect();

    let mut matched = Vec::new();
    matched.extend(matched_keywords.clone());
    matched.extend(matched_tool_name.clone());
    matched.extend(matched_payload_keys.clone());
    matched.sort();
    matched.dedup();

    let keyword_score = (matched_keywords.len().min(scoring.keyword_hit_cap) as u16)
        * u16::from(scoring.keyword_weight);
    let tool_score = if matched_tool_name.is_empty() {
        0
    } else {
        u16::from(scoring.tool_name_weight)
    };
    let payload_key_score = (matched_payload_keys.len().min(scoring.payload_key_hit_cap) as u16)
        * u16::from(scoring.payload_key_weight);
    let risk_score = keyword_score
        .saturating_add(tool_score)
        .saturating_add(payload_key_score)
        .min(100) as u8;

    if matched.is_empty() || risk_score < scoring.high_risk_threshold {
        (ApprovalRiskLevel::Low, Vec::new(), 0)
    } else {
        (ApprovalRiskLevel::High, matched, risk_score)
    }
}

fn resolve_approval_risk_profile(policy: &HumanApprovalSpec) -> ApprovalRiskProfile {
    let mut profile = policy
        .risk_profile_path
        .as_deref()
        .and_then(load_approval_risk_profile_from_path)
        .unwrap_or_else(bundled_approval_risk_profile);

    if !policy.high_risk_keywords.is_empty() {
        profile.high_risk_keywords = policy.high_risk_keywords.clone();
    }
    if !policy.high_risk_tool_names.is_empty() {
        profile.high_risk_tool_names = policy.high_risk_tool_names.clone();
    }
    if !policy.high_risk_payload_keys.is_empty() {
        profile.high_risk_payload_keys = policy.high_risk_payload_keys.clone();
    }

    profile
}

fn load_approval_risk_profile_from_path(path: &str) -> Option<ApprovalRiskProfile> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str::<ApprovalRiskProfile>(&content).ok()
}

fn bundled_approval_risk_profile() -> ApprovalRiskProfile {
    BUNDLED_APPROVAL_RISK_PROFILE
        .get_or_init(|| {
            let raw = include_str!("../config/approval-medium-balanced.json");
            serde_json::from_str(raw).unwrap_or_else(|error| {
                panic!("invalid bundled approval risk profile config: {error}");
            })
        })
        .clone()
}

fn normalize_signal_list(list: Vec<String>) -> Vec<String> {
    let mut normalized: Vec<String> = list
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn sanitize_risk_scoring(mut scoring: ApprovalRiskScoring) -> ApprovalRiskScoring {
    if scoring.keyword_hit_cap == 0 {
        scoring.keyword_hit_cap = 1;
    }
    if scoring.payload_key_hit_cap == 0 {
        scoring.payload_key_hit_cap = 1;
    }
    if scoring.high_risk_threshold == 0 {
        scoring.high_risk_threshold = 20;
    }
    scoring
}

fn operation_tool_name(operation: &OperationSpec) -> Option<&str> {
    match operation {
        OperationSpec::ToolCore { tool_name, .. } => Some(tool_name.as_str()),
        OperationSpec::ToolExtension {
            extension_action, ..
        } => Some(extension_action.as_str()),
        _ => None,
    }
}

fn operation_payload_keys(operation: &OperationSpec) -> Vec<String> {
    let mut keys = Vec::new();
    let payload = match operation {
        OperationSpec::Task { payload, .. }
        | OperationSpec::ConnectorLegacy { payload, .. }
        | OperationSpec::ConnectorCore { payload, .. }
        | OperationSpec::ConnectorExtension { payload, .. }
        | OperationSpec::RuntimeCore { payload, .. }
        | OperationSpec::RuntimeExtension { payload, .. }
        | OperationSpec::ToolCore { payload, .. }
        | OperationSpec::ToolExtension { payload, .. }
        | OperationSpec::MemoryCore { payload, .. }
        | OperationSpec::MemoryExtension { payload, .. } => payload,
    };
    collect_json_keys(payload, &mut keys);
    keys
}

fn collect_json_keys(value: &Value, keys: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                keys.push(key.clone());
                collect_json_keys(child, keys);
            }
        }
        Value::Array(list) => {
            for child in list {
                collect_json_keys(child, keys);
            }
        }
        Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn operation_risk_haystack(operation: &OperationSpec) -> String {
    let mut text = String::new();
    text.push_str(operation_approval_kind(operation));
    text.push(' ');
    text.push_str(&operation_approval_key(operation));
    text.push(' ');
    for value in operation_payload_strings(operation) {
        text.push_str(&value);
        text.push(' ');
    }
    text
}

fn operation_payload_strings(operation: &OperationSpec) -> Vec<String> {
    let mut values = Vec::new();
    let payload = match operation {
        OperationSpec::Task { payload, .. }
        | OperationSpec::ConnectorLegacy { payload, .. }
        | OperationSpec::ConnectorCore { payload, .. }
        | OperationSpec::ConnectorExtension { payload, .. }
        | OperationSpec::RuntimeCore { payload, .. }
        | OperationSpec::RuntimeExtension { payload, .. }
        | OperationSpec::ToolCore { payload, .. }
        | OperationSpec::ToolExtension { payload, .. }
        | OperationSpec::MemoryCore { payload, .. }
        | OperationSpec::MemoryExtension { payload, .. } => payload,
    };
    collect_json_strings(payload, &mut values);
    values
}

fn collect_json_strings(value: &Value, values: &mut Vec<String>) {
    match value {
        Value::String(string) => values.push(string.clone()),
        Value::Array(array) => {
            for entry in array {
                collect_json_strings(entry, values);
            }
        }
        Value::Object(map) => {
            for (key, entry) in map {
                values.push(key.clone());
                collect_json_strings(entry, values);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn is_operation_preapproved(operation_key: &str, approvals: &[String]) -> bool {
    let operation_key_lower = operation_key.to_ascii_lowercase();
    approvals.iter().any(|raw| {
        let normalized = raw.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return false;
        }
        if normalized == "*" {
            return true;
        }
        if let Some(prefix) = normalized.strip_suffix('*') {
            return operation_key_lower.starts_with(prefix);
        }
        normalized == operation_key_lower
    })
}

fn current_epoch_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn bootstrap_policy(spec: &RunnerSpec) -> Option<BootstrapPolicy> {
    let bootstrap = spec.bootstrap.as_ref()?;
    if !bootstrap.enabled {
        return None;
    }

    let mut policy = BootstrapPolicy::default();
    if let Some(value) = bootstrap.allow_http_json_auto_apply {
        policy.allow_http_json_auto_apply = value;
    }
    if let Some(value) = bootstrap.allow_process_stdio_auto_apply {
        policy.allow_process_stdio_auto_apply = value;
    }
    if let Some(value) = bootstrap.allow_native_ffi_auto_apply {
        policy.allow_native_ffi_auto_apply = value;
    }
    if let Some(value) = bootstrap.allow_wasm_component_auto_apply {
        policy.allow_wasm_component_auto_apply = value;
    }
    if let Some(value) = bootstrap.allow_mcp_server_auto_apply {
        policy.allow_mcp_server_auto_apply = value;
    }
    if let Some(value) = bootstrap.enforce_ready_execution {
        policy.enforce_ready_execution = value;
    }
    if let Some(value) = bootstrap.max_tasks {
        policy.max_tasks = value.max(1);
    }
    Some(policy)
}

fn filter_scan_report_by_activation(
    report: &PluginScanReport,
    activation: &PluginActivationPlan,
) -> PluginScanReport {
    let ready_keys: BTreeSet<(String, String)> = activation
        .candidates
        .iter()
        .filter(|candidate| matches!(candidate.status, PluginActivationStatus::Ready))
        .map(|candidate| (candidate.source_path.clone(), candidate.plugin_id.clone()))
        .collect();

    filter_scan_report_by_keys(report, &ready_keys)
}

fn filter_scan_report_by_keys(
    report: &PluginScanReport,
    allowed_keys: &BTreeSet<(String, String)>,
) -> PluginScanReport {
    let descriptors: Vec<PluginDescriptor> = report
        .descriptors
        .iter()
        .filter(|descriptor| {
            allowed_keys.contains(&(
                descriptor.path.clone(),
                descriptor.manifest.plugin_id.clone(),
            ))
        })
        .cloned()
        .collect();

    PluginScanReport {
        scanned_files: report.scanned_files,
        matched_plugins: descriptors.len(),
        descriptors,
    }
}

fn enrich_scan_report_with_translation(
    report: &PluginScanReport,
    translation: &PluginTranslationReport,
) -> PluginScanReport {
    let mut runtime_by_key: BTreeMap<(String, String), (String, String, String, String)> =
        BTreeMap::new();

    for entry in &translation.entries {
        runtime_by_key.insert(
            (entry.source_path.clone(), entry.plugin_id.clone()),
            (
                entry.runtime.bridge_kind.as_str().to_owned(),
                entry.runtime.adapter_family.clone(),
                entry.runtime.entrypoint_hint.clone(),
                entry.runtime.source_language.clone(),
            ),
        );
    }

    let descriptors: Vec<PluginDescriptor> = report
        .descriptors
        .iter()
        .cloned()
        .map(|mut descriptor| {
            if let Some((bridge_kind, adapter_family, entrypoint_hint, source_language)) =
                runtime_by_key.get(&(
                    descriptor.path.clone(),
                    descriptor.manifest.plugin_id.clone(),
                ))
            {
                descriptor
                    .manifest
                    .metadata
                    .entry("bridge_kind".to_owned())
                    .or_insert_with(|| bridge_kind.clone());
                descriptor
                    .manifest
                    .metadata
                    .entry("adapter_family".to_owned())
                    .or_insert_with(|| adapter_family.clone());
                descriptor
                    .manifest
                    .metadata
                    .entry("entrypoint_hint".to_owned())
                    .or_insert_with(|| entrypoint_hint.clone());
                descriptor
                    .manifest
                    .metadata
                    .entry("source_language".to_owned())
                    .or_insert_with(|| source_language.clone());
            }
            descriptor
        })
        .collect();

    PluginScanReport {
        scanned_files: report.scanned_files,
        matched_plugins: descriptors.len(),
        descriptors,
    }
}

fn bridge_support_policy_checksum(bridge: &BridgeSupportSpec) -> String {
    let encoded = bridge_support_policy_canonical_json(bridge);
    fnv1a64_hex(encoded.as_bytes())
}

fn bridge_support_policy_sha256(bridge: &BridgeSupportSpec) -> String {
    let encoded = bridge_support_policy_canonical_json(bridge);
    let digest = Sha256::digest(encoded.as_bytes());
    hex_lower(&digest)
}

fn bridge_support_policy_canonical_json(bridge: &BridgeSupportSpec) -> String {
    let mut bridges = bridge.supported_bridges.clone();
    bridges.sort();

    let mut adapter_families = bridge.supported_adapter_families.clone();
    adapter_families.sort();
    let mut allowed_commands = bridge.allowed_process_commands.clone();
    allowed_commands.sort();
    let security_scan = canonical_security_scan_value(bridge.security_scan.as_ref());

    let canonical = json!({
        "supported_bridges": bridges,
        "supported_adapter_families": adapter_families,
        "enforce_supported": bridge.enforce_supported,
        "execute_process_stdio": bridge.execute_process_stdio,
        "execute_http_json": bridge.execute_http_json,
        "allowed_process_commands": allowed_commands,
        "enforce_execution_success": bridge.enforce_execution_success,
        "security_scan": security_scan,
    });

    serde_json::to_string(&canonical).expect("serialize bridge support checksum payload")
}

fn canonical_security_scan_value(security_scan: Option<&SecurityScanSpec>) -> Value {
    let Some(scan) = security_scan else {
        return Value::Null;
    };

    let mut keywords = scan.high_risk_metadata_keywords.clone();
    keywords.sort();

    let mut blocked_import_prefixes = scan.wasm.blocked_import_prefixes.clone();
    blocked_import_prefixes.sort();

    let mut allowed_path_prefixes = scan.wasm.allowed_path_prefixes.clone();
    allowed_path_prefixes.sort();

    let required_sha256_by_plugin = scan
        .wasm
        .required_sha256_by_plugin
        .iter()
        .map(|(plugin, digest)| (plugin.clone(), digest.clone()))
        .collect::<BTreeMap<_, _>>();
    let profile_signature =
        canonical_security_scan_profile_signature_value(scan.profile_signature.as_ref());
    let siem_export = canonical_security_scan_siem_export_value(scan.siem_export.as_ref());

    json!({
        "enabled": scan.enabled,
        "block_on_high": scan.block_on_high,
        "profile_path": scan.profile_path,
        "profile_sha256": scan.profile_sha256,
        "profile_signature": profile_signature,
        "siem_export": siem_export,
        "high_risk_metadata_keywords": keywords,
        "wasm": {
            "enabled": scan.wasm.enabled,
            "max_module_bytes": scan.wasm.max_module_bytes,
            "allow_wasi": scan.wasm.allow_wasi,
            "blocked_import_prefixes": blocked_import_prefixes,
            "allowed_path_prefixes": allowed_path_prefixes,
            "require_hash_pin": scan.wasm.require_hash_pin,
            "required_sha256_by_plugin": required_sha256_by_plugin,
        },
    })
}

fn canonical_security_scan_profile_signature_value(
    signature: Option<&SecurityProfileSignatureSpec>,
) -> Value {
    let Some(signature) = signature else {
        return Value::Null;
    };
    json!({
        "algorithm": signature.algorithm.trim().to_ascii_lowercase(),
        "public_key_base64": signature.public_key_base64,
        "signature_base64": signature.signature_base64,
    })
}

fn canonical_security_scan_siem_export_value(export: Option<&SecuritySiemExportSpec>) -> Value {
    let Some(export) = export else {
        return Value::Null;
    };
    json!({
        "enabled": export.enabled,
        "path": export.path,
        "include_findings": export.include_findings,
        "max_findings_per_record": export.max_findings_per_record,
        "fail_on_error": export.fail_on_error,
    })
}

fn canonical_security_scan_profile_value(profile: &SecurityScanProfile) -> Value {
    let mut keywords = profile.high_risk_metadata_keywords.clone();
    keywords.sort();

    let mut blocked_import_prefixes = profile.wasm.blocked_import_prefixes.clone();
    blocked_import_prefixes.sort();

    let mut allowed_path_prefixes = profile.wasm.allowed_path_prefixes.clone();
    allowed_path_prefixes.sort();

    let required_sha256_by_plugin = profile
        .wasm
        .required_sha256_by_plugin
        .iter()
        .map(|(plugin, digest)| (plugin.clone(), digest.clone()))
        .collect::<BTreeMap<_, _>>();

    json!({
        "high_risk_metadata_keywords": keywords,
        "wasm": {
            "enabled": profile.wasm.enabled,
            "max_module_bytes": profile.wasm.max_module_bytes,
            "allow_wasi": profile.wasm.allow_wasi,
            "blocked_import_prefixes": blocked_import_prefixes,
            "allowed_path_prefixes": allowed_path_prefixes,
            "require_hash_pin": profile.wasm.require_hash_pin,
            "required_sha256_by_plugin": required_sha256_by_plugin,
        }
    })
}

fn security_scan_profile_sha256(profile: &SecurityScanProfile) -> String {
    let encoded = security_scan_profile_message(profile);
    let digest = Sha256::digest(&encoded);
    hex_lower(&digest)
}

fn security_scan_profile_message(profile: &SecurityScanProfile) -> Vec<u8> {
    let canonical = canonical_security_scan_profile_value(profile);
    serde_json::to_vec(&canonical).expect("serialize security scan profile canonical payload")
}

fn fnv1a64_hex(bytes: &[u8]) -> String {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push_str(&format!("{byte:02x}"));
    }
    encoded
}

fn default_integration_catalog() -> IntegrationCatalog {
    let mut catalog = IntegrationCatalog::new();
    for (provider_id, connector, version, class) in [
        ("openai", "openai", "1.0.0", "llm"),
        ("anthropic", "anthropic", "1.0.0", "llm"),
        ("github", "github", "1.0.0", "devops"),
        ("slack", "slack", "1.0.0", "messaging"),
        ("notion", "notion", "1.0.0", "workspace"),
    ] {
        catalog.register_template(kernel::ProviderTemplate {
            provider_id: provider_id.to_owned(),
            default_connector_name: connector.to_owned(),
            default_version: version.to_owned(),
            metadata: BTreeMap::from([("class".to_owned(), class.to_owned())]),
        });
    }
    catalog
}

fn register_dynamic_catalog_connectors(
    kernel: &mut ChumosKernel<StaticPolicyEngine>,
    catalog: Arc<Mutex<IntegrationCatalog>>,
    bridge_runtime_policy: BridgeRuntimePolicy,
) {
    let snapshot = {
        let guard = catalog
            .lock()
            .expect("integration catalog mutex poisoned during registration");
        guard.providers()
    };

    for provider in snapshot {
        kernel.register_connector(DynamicCatalogConnector {
            connector_name: provider.connector_name,
            provider_id: provider.provider_id,
            catalog: catalog.clone(),
            bridge_runtime_policy: bridge_runtime_policy.clone(),
        });
    }
}

fn operation_connector_name(operation: &OperationSpec) -> Option<String> {
    match operation {
        OperationSpec::ConnectorLegacy { connector_name, .. }
        | OperationSpec::ConnectorCore { connector_name, .. }
        | OperationSpec::ConnectorExtension { connector_name, .. } => Some(connector_name.clone()),
        _ => None,
    }
}

async fn execute_spec_operation(
    kernel: &ChumosKernel<StaticPolicyEngine>,
    pack_id: &str,
    token: &kernel::CapabilityToken,
    operation: &OperationSpec,
) -> (&'static str, Value) {
    match operation {
        OperationSpec::Task {
            task_id,
            objective,
            required_capabilities,
            payload,
        } => {
            let dispatch = kernel
                .execute_task(
                    pack_id,
                    token,
                    TaskIntent {
                        task_id: task_id.clone(),
                        objective: objective.clone(),
                        required_capabilities: required_capabilities.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .expect("task execution from spec failed");
            (
                "task",
                json!({
                    "route": dispatch.adapter_route,
                    "outcome": dispatch.outcome,
                }),
            )
        }
        OperationSpec::ConnectorLegacy {
            connector_name,
            operation,
            required_capabilities,
            payload,
        } => {
            let dispatch = kernel
                .invoke_connector(
                    pack_id,
                    token,
                    ConnectorCommand {
                        connector_name: connector_name.clone(),
                        operation: operation.clone(),
                        required_capabilities: required_capabilities.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .expect("legacy connector execution from spec failed");
            (
                "connector_legacy",
                json!({
                    "connector_name": dispatch.connector_name,
                    "outcome": dispatch.outcome,
                }),
            )
        }
        OperationSpec::ConnectorCore {
            connector_name,
            operation,
            required_capabilities,
            payload,
            core,
        } => {
            let dispatch = kernel
                .execute_connector_core(
                    pack_id,
                    token,
                    core.as_deref(),
                    ConnectorCommand {
                        connector_name: connector_name.clone(),
                        operation: operation.clone(),
                        required_capabilities: required_capabilities.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .expect("core connector execution from spec failed");
            (
                "connector_core",
                json!({
                    "connector_name": dispatch.connector_name,
                    "outcome": dispatch.outcome,
                }),
            )
        }
        OperationSpec::ConnectorExtension {
            connector_name,
            operation,
            required_capabilities,
            payload,
            extension,
            core,
        } => {
            let dispatch = kernel
                .execute_connector_extension(
                    pack_id,
                    token,
                    extension,
                    core.as_deref(),
                    ConnectorCommand {
                        connector_name: connector_name.clone(),
                        operation: operation.clone(),
                        required_capabilities: required_capabilities.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .expect("extension connector execution from spec failed");
            (
                "connector_extension",
                json!({
                    "connector_name": dispatch.connector_name,
                    "outcome": dispatch.outcome,
                }),
            )
        }
        OperationSpec::RuntimeCore {
            action,
            required_capabilities,
            payload,
            core,
        } => {
            let outcome = kernel
                .execute_runtime_core(
                    pack_id,
                    token,
                    required_capabilities,
                    core.as_deref(),
                    RuntimeCoreRequest {
                        action: action.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .expect("runtime core execution from spec failed");
            ("runtime_core", json!({ "outcome": outcome }))
        }
        OperationSpec::RuntimeExtension {
            action,
            required_capabilities,
            payload,
            extension,
            core,
        } => {
            let outcome = kernel
                .execute_runtime_extension(
                    pack_id,
                    token,
                    required_capabilities,
                    extension,
                    core.as_deref(),
                    RuntimeExtensionRequest {
                        action: action.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .expect("runtime extension execution from spec failed");
            ("runtime_extension", json!({ "outcome": outcome }))
        }
        OperationSpec::ToolCore {
            tool_name,
            required_capabilities,
            payload,
            core,
        } => {
            let outcome = kernel
                .execute_tool_core(
                    pack_id,
                    token,
                    required_capabilities,
                    core.as_deref(),
                    ToolCoreRequest {
                        tool_name: tool_name.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .expect("tool core execution from spec failed");
            ("tool_core", json!({ "outcome": outcome }))
        }
        OperationSpec::ToolExtension {
            extension_action,
            required_capabilities,
            payload,
            extension,
            core,
        } => {
            let outcome = kernel
                .execute_tool_extension(
                    pack_id,
                    token,
                    required_capabilities,
                    extension,
                    core.as_deref(),
                    ToolExtensionRequest {
                        extension_action: extension_action.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .expect("tool extension execution from spec failed");
            ("tool_extension", json!({ "outcome": outcome }))
        }
        OperationSpec::MemoryCore {
            operation,
            required_capabilities,
            payload,
            core,
        } => {
            let outcome = kernel
                .execute_memory_core(
                    pack_id,
                    token,
                    required_capabilities,
                    core.as_deref(),
                    MemoryCoreRequest {
                        operation: operation.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .expect("memory core execution from spec failed");
            ("memory_core", json!({ "outcome": outcome }))
        }
        OperationSpec::MemoryExtension {
            operation,
            required_capabilities,
            payload,
            extension,
            core,
        } => {
            let outcome = kernel
                .execute_memory_extension(
                    pack_id,
                    token,
                    required_capabilities,
                    extension,
                    core.as_deref(),
                    MemoryExtensionRequest {
                        operation: operation.clone(),
                        payload: payload.clone(),
                    },
                )
                .await
                .expect("memory extension execution from spec failed");
            ("memory_extension", json!({ "outcome": outcome }))
        }
    }
}

fn apply_default_selection(
    kernel: &mut ChumosKernel<StaticPolicyEngine>,
    defaults: Option<&DefaultCoreSelection>,
) {
    if let Some(defaults) = defaults {
        if let Some(connector) = defaults.connector.as_deref() {
            kernel
                .set_default_core_connector_adapter(connector)
                .expect("invalid default connector core adapter");
        }
        if let Some(runtime) = defaults.runtime.as_deref() {
            kernel
                .set_default_core_runtime_adapter(runtime)
                .expect("invalid default runtime core adapter");
        }
        if let Some(tool) = defaults.tool.as_deref() {
            kernel
                .set_default_core_tool_adapter(tool)
                .expect("invalid default tool core adapter");
        }
        if let Some(memory) = defaults.memory.as_deref() {
            kernel
                .set_default_core_memory_adapter(memory)
                .expect("invalid default memory core adapter");
        }
    }
}

fn parse_json_payload(raw: &str, context: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|error| {
        panic!("invalid JSON for {context}: {error}");
    })
}

fn read_spec_file(path: &str) -> RunnerSpec {
    let raw = fs::read_to_string(path).unwrap_or_else(|error| {
        panic!("failed to read spec file {path}: {error}");
    });
    serde_json::from_str(&raw).unwrap_or_else(|error| {
        panic!("failed to parse spec file {path}: {error}");
    })
}

fn write_json_file<T: Serialize>(path: &str, value: &T) {
    let serialized =
        serde_json::to_string_pretty(value).expect("serialize JSON value for output file");
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).expect("create output directory");
        }
    }
    fs::write(path, serialized).expect("write JSON output file");
}

fn bootstrap_kernel_default() -> ChumosKernel<StaticPolicyEngine> {
    let mut kernel = ChumosKernel::new(StaticPolicyEngine::default());
    register_builtin_adapters(&mut kernel);
    kernel
        .register_pack(default_pack_manifest())
        .expect("pack registration failed");
    kernel
}

fn bootstrap_kernel_with_runtime(
    clock: Arc<dyn Clock>,
    audit: Arc<dyn AuditSink>,
) -> ChumosKernel<StaticPolicyEngine> {
    let mut kernel = ChumosKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);
    register_builtin_adapters(&mut kernel);
    kernel
        .register_pack(default_pack_manifest())
        .expect("pack registration failed");
    kernel
}

fn register_builtin_adapters(kernel: &mut ChumosKernel<StaticPolicyEngine>) {
    kernel.register_harness_adapter(EmbeddedPiHarness {
        seen: Mutex::new(Vec::new()),
    });
    kernel.register_connector(WebhookConnector);
    kernel.register_core_connector_adapter(CrmCoreConnector);
    kernel.register_core_connector_adapter(CrmGrpcCoreConnector);
    kernel.register_connector_extension_adapter(ShieldedConnectorExtension);

    kernel.register_core_runtime_adapter(NativeCoreRuntime);
    kernel.register_core_runtime_adapter(FallbackCoreRuntime);
    kernel.register_runtime_extension_adapter(AcpBridgeRuntimeExtension);

    kernel.register_core_tool_adapter(CoreToolRuntime);
    kernel.register_tool_extension_adapter(SqlAnalyticsToolExtension);

    kernel.register_core_memory_adapter(KvCoreMemory);
    kernel.register_memory_extension_adapter(VectorIndexMemoryExtension);
}

fn default_pack_manifest() -> VerticalPackManifest {
    VerticalPackManifest {
        pack_id: DEFAULT_PACK_ID.to_owned(),
        domain: "engineering".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: Some("pi-local".to_owned()),
        },
        allowed_connectors: BTreeSet::from([
            "webhook".to_owned(),
            "crm".to_owned(),
            "erp".to_owned(),
        ]),
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval_test_operation(tool_name: &str, payload: Value) -> OperationSpec {
        OperationSpec::ToolCore {
            tool_name: tool_name.to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload,
            core: None,
        }
    }

    fn write_temp_risk_profile(path: &Path, body: &str) {
        fs::create_dir_all(
            path.parent()
                .expect("temp risk profile path should have parent directory"),
        )
        .expect("create temp risk profile directory");
        fs::write(path, body).expect("write temp risk profile");
    }

    fn sign_security_scan_profile_for_test(profile: &SecurityScanProfile) -> (String, String) {
        use ed25519_dalek::{Signer, SigningKey};

        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let signature = signing_key.sign(&security_scan_profile_message(profile));
        let public_key_base64 = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
        let signature_base64 = BASE64_STANDARD.encode(signature.to_bytes());
        (public_key_base64, signature_base64)
    }

    #[test]
    fn template_spec_is_json_roundtrip_stable() {
        let spec = RunnerSpec::template();
        let encoded = serde_json::to_string_pretty(&spec).expect("encode spec");
        let decoded: RunnerSpec = serde_json::from_str(&encoded).expect("decode spec");
        assert_eq!(decoded.pack.pack_id, "sales-intel-local");
        assert!(matches!(
            decoded.operation,
            OperationSpec::RuntimeExtension { .. }
        ));
    }

    #[test]
    fn approval_uses_external_risk_profile_without_inline_overrides() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("chumos-risk-profile-{unique}.json"));
        write_temp_risk_profile(
            &path,
            r#"{
  "high_risk_keywords": ["irrelevant"],
  "high_risk_tool_names": ["irrelevant-tool"],
  "high_risk_payload_keys": ["irrelevant_key"],
  "scoring": {
    "keyword_weight": 10,
    "tool_name_weight": 10,
    "payload_key_weight": 10,
    "keyword_hit_cap": 2,
    "payload_key_hit_cap": 2,
    "high_risk_threshold": 10
  }
}"#,
        );

        let policy = HumanApprovalSpec {
            risk_profile_path: Some(path.display().to_string()),
            ..HumanApprovalSpec::default()
        };
        let operation = approval_test_operation("delete-file", json!({"path":"/tmp/demo.txt"}));
        let (risk_level, matched, score) = operation_risk_profile(&operation, &policy);

        assert_eq!(risk_level, ApprovalRiskLevel::Low);
        assert!(matched.is_empty());
        assert_eq!(score, 0);
    }

    #[test]
    fn approval_inline_risk_signals_override_external_profile() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("chumos-risk-profile-override-{unique}.json"));
        write_temp_risk_profile(
            &path,
            r#"{
  "high_risk_keywords": ["irrelevant"],
  "high_risk_tool_names": ["irrelevant-tool"],
  "high_risk_payload_keys": ["irrelevant_key"],
  "scoring": {
    "keyword_weight": 10,
    "tool_name_weight": 10,
    "payload_key_weight": 10,
    "keyword_hit_cap": 2,
    "payload_key_hit_cap": 2,
    "high_risk_threshold": 10
  }
}"#,
        );

        let policy = HumanApprovalSpec {
            risk_profile_path: Some(path.display().to_string()),
            high_risk_tool_names: vec!["delete-file".to_owned()],
            ..HumanApprovalSpec::default()
        };
        let operation = approval_test_operation("delete-file", json!({"path":"/tmp/demo.txt"}));
        let (risk_level, matched, score) = operation_risk_profile(&operation, &policy);

        assert_eq!(risk_level, ApprovalRiskLevel::High);
        assert!(matched.iter().any(|value| value == "tool:delete-file"));
        assert_eq!(score, 10);
    }

    #[test]
    fn approval_falls_back_to_bundled_profile_when_path_missing() {
        let policy = HumanApprovalSpec {
            risk_profile_path: Some("/tmp/chumos-risk-profile-missing.json".to_owned()),
            ..HumanApprovalSpec::default()
        };
        let operation = approval_test_operation("delete-file", json!({"path":"/tmp/demo.txt"}));
        let (risk_level, matched, score) = operation_risk_profile(&operation, &policy);

        assert_eq!(risk_level, ApprovalRiskLevel::High);
        assert!(matched.iter().any(|value| value == "tool:delete-file"));
        assert!(score >= 20);
    }

    #[test]
    fn security_scan_profile_path_overrides_bundled_defaults() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("chumos-security-profile-{unique}.json"));
        fs::write(
            &path,
            r#"{
  "high_risk_metadata_keywords": ["custom-danger-keyword"],
  "wasm": {
    "enabled": true,
    "max_module_bytes": 123456,
    "allow_wasi": false,
    "blocked_import_prefixes": ["wasi-custom"],
    "allowed_path_prefixes": [],
    "require_hash_pin": false,
    "required_sha256_by_plugin": {}
  }
}"#,
        )
        .expect("write security scan profile");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-security-profile-path".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-security-profile-path".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::WasmComponent],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,
                execute_process_stdio: false,
                execute_http_json: false,
                allowed_process_commands: Vec::new(),
                enforce_execution_success: false,
                security_scan: Some(SecurityScanSpec {
                    enabled: true,
                    block_on_high: true,
                    profile_path: Some(path.display().to_string()),
                    profile_sha256: None,
                    profile_signature: None,
                    siem_export: None,
                    high_risk_metadata_keywords: Vec::new(),
                    wasm: WasmSecurityScanSpec {
                        enabled: true,
                        max_module_bytes: 0,
                        allow_wasi: false,
                        blocked_import_prefixes: Vec::new(),
                        allowed_path_prefixes: Vec::new(),
                        require_hash_pin: false,
                        required_sha256_by_plugin: BTreeMap::new(),
                    },
                }),
            }),
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-security-profile-path".to_owned(),
                objective: "verify profile loading".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let policy = security_scan_policy(&spec)
            .expect("security scan policy should resolve")
            .expect("security scan policy should be enabled");
        assert_eq!(
            policy.high_risk_metadata_keywords,
            vec!["custom-danger-keyword".to_owned()]
        );
        assert_eq!(policy.wasm.max_module_bytes, 123456);
        assert_eq!(
            policy.wasm.blocked_import_prefixes,
            vec!["wasi-custom".to_owned()]
        );
    }

    #[test]
    fn security_scan_profile_sha256_pin_accepts_matching_profile() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("chumos-security-profile-sha-match-{unique}.json"));
        fs::write(
            &path,
            r#"{
  "high_risk_metadata_keywords": ["pinned-danger"],
  "wasm": {
    "enabled": true,
    "max_module_bytes": 654321,
    "allow_wasi": false,
    "blocked_import_prefixes": ["wasi-custom"],
    "allowed_path_prefixes": [],
    "require_hash_pin": false,
    "required_sha256_by_plugin": {}
  }
}"#,
        )
        .expect("write pinned profile");

        let profile = load_security_scan_profile_from_path(path.to_str().expect("utf8 path"))
            .expect("profile should load");
        let profile_sha256 = security_scan_profile_sha256(&profile);

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-security-profile-pin".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-security-profile-pin".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::WasmComponent],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,
                execute_process_stdio: false,
                execute_http_json: false,
                allowed_process_commands: Vec::new(),
                enforce_execution_success: false,
                security_scan: Some(SecurityScanSpec {
                    enabled: true,
                    block_on_high: true,
                    profile_path: Some(path.display().to_string()),
                    profile_sha256: Some(profile_sha256),
                    profile_signature: None,
                    siem_export: None,
                    high_risk_metadata_keywords: Vec::new(),
                    wasm: WasmSecurityScanSpec {
                        enabled: true,
                        max_module_bytes: 0,
                        allow_wasi: false,
                        blocked_import_prefixes: Vec::new(),
                        allowed_path_prefixes: Vec::new(),
                        require_hash_pin: false,
                        required_sha256_by_plugin: BTreeMap::new(),
                    },
                }),
            }),
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-security-profile-pin".to_owned(),
                objective: "verify profile sha pin".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let policy = security_scan_policy(&spec)
            .expect("security scan policy should resolve")
            .expect("security scan policy should be enabled");
        assert_eq!(
            policy.high_risk_metadata_keywords,
            vec!["pinned-danger".to_owned()]
        );
        assert_eq!(policy.wasm.max_module_bytes, 654321);
    }

    #[tokio::test]
    async fn execute_spec_blocks_when_security_scan_profile_sha256_mismatches() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "chumos-security-profile-sha-mismatch-{unique}.json"
        ));
        fs::write(
            &path,
            r#"{
  "high_risk_metadata_keywords": ["mismatch-danger"],
  "wasm": {
    "enabled": true,
    "max_module_bytes": 1024,
    "allow_wasi": false,
    "blocked_import_prefixes": [],
    "allowed_path_prefixes": [],
    "require_hash_pin": false,
    "required_sha256_by_plugin": {}
  }
}"#,
        )
        .expect("write mismatched profile");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-security-profile-mismatch".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-security-profile-mismatch".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::WasmComponent],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,
                execute_process_stdio: false,
                execute_http_json: false,
                allowed_process_commands: Vec::new(),
                enforce_execution_success: false,
                security_scan: Some(SecurityScanSpec {
                    enabled: true,
                    block_on_high: true,
                    profile_path: Some(path.display().to_string()),
                    profile_sha256: Some("deadbeef".repeat(8)),
                    profile_signature: None,
                    siem_export: None,
                    high_risk_metadata_keywords: Vec::new(),
                    wasm: WasmSecurityScanSpec {
                        enabled: true,
                        max_module_bytes: 0,
                        allow_wasi: false,
                        blocked_import_prefixes: Vec::new(),
                        allowed_path_prefixes: Vec::new(),
                        require_hash_pin: false,
                        required_sha256_by_plugin: BTreeMap::new(),
                    },
                }),
            }),
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-security-profile-mismatch".to_owned(),
                objective: "mismatch pin should block".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "blocked");
        assert!(report
            .blocked_reason
            .expect("blocked reason should exist")
            .contains("profile sha256 mismatch"));
    }

    #[test]
    fn security_scan_profile_signature_accepts_matching_signature() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "chumos-security-profile-signature-match-{unique}.json"
        ));
        fs::write(
            &path,
            r#"{
  "high_risk_metadata_keywords": ["signed-danger"],
  "wasm": {
    "enabled": true,
    "max_module_bytes": 2048,
    "allow_wasi": false,
    "blocked_import_prefixes": ["wasi"],
    "allowed_path_prefixes": [],
    "require_hash_pin": false,
    "required_sha256_by_plugin": {}
  }
}"#,
        )
        .expect("write signed profile");

        let profile = load_security_scan_profile_from_path(path.to_str().expect("utf8 path"))
            .expect("profile should load");
        let (public_key_base64, signature_base64) = sign_security_scan_profile_for_test(&profile);

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-security-signature-pin".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-security-signature-pin".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::WasmComponent],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,
                execute_process_stdio: false,
                execute_http_json: false,
                allowed_process_commands: Vec::new(),
                enforce_execution_success: false,
                security_scan: Some(SecurityScanSpec {
                    enabled: true,
                    block_on_high: true,
                    profile_path: Some(path.display().to_string()),
                    profile_sha256: None,
                    profile_signature: Some(SecurityProfileSignatureSpec {
                        algorithm: "ed25519".to_owned(),
                        public_key_base64,
                        signature_base64,
                    }),
                    siem_export: None,
                    high_risk_metadata_keywords: Vec::new(),
                    wasm: WasmSecurityScanSpec {
                        enabled: true,
                        max_module_bytes: 0,
                        allow_wasi: false,
                        blocked_import_prefixes: Vec::new(),
                        allowed_path_prefixes: Vec::new(),
                        require_hash_pin: false,
                        required_sha256_by_plugin: BTreeMap::new(),
                    },
                }),
            }),
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-security-signature-pin".to_owned(),
                objective: "verify profile signature pin".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let policy = security_scan_policy(&spec)
            .expect("security scan policy should resolve")
            .expect("security scan policy should be enabled");
        assert_eq!(
            policy.high_risk_metadata_keywords,
            vec!["signed-danger".to_owned()]
        );
    }

    #[tokio::test]
    async fn execute_spec_blocks_when_security_scan_profile_signature_mismatches() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "chumos-security-profile-signature-mismatch-{unique}.json"
        ));
        fs::write(
            &path,
            r#"{
  "high_risk_metadata_keywords": ["signed-mismatch"],
  "wasm": {
    "enabled": true,
    "max_module_bytes": 1024,
    "allow_wasi": false,
    "blocked_import_prefixes": [],
    "allowed_path_prefixes": [],
    "require_hash_pin": false,
    "required_sha256_by_plugin": {}
  }
}"#,
        )
        .expect("write signed mismatch profile");

        let profile = load_security_scan_profile_from_path(path.to_str().expect("utf8 path"))
            .expect("profile should load");
        let (public_key_base64, mut signature_base64) =
            sign_security_scan_profile_for_test(&profile);
        let replacement = if signature_base64.starts_with('A') {
            "B"
        } else {
            "A"
        };
        signature_base64.replace_range(0..1, replacement);

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-security-signature-mismatch".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-security-signature-mismatch".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::WasmComponent],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,
                execute_process_stdio: false,
                execute_http_json: false,
                allowed_process_commands: Vec::new(),
                enforce_execution_success: false,
                security_scan: Some(SecurityScanSpec {
                    enabled: true,
                    block_on_high: true,
                    profile_path: Some(path.display().to_string()),
                    profile_sha256: None,
                    profile_signature: Some(SecurityProfileSignatureSpec {
                        algorithm: "ed25519".to_owned(),
                        public_key_base64,
                        signature_base64,
                    }),
                    siem_export: None,
                    high_risk_metadata_keywords: Vec::new(),
                    wasm: WasmSecurityScanSpec {
                        enabled: true,
                        max_module_bytes: 0,
                        allow_wasi: false,
                        blocked_import_prefixes: Vec::new(),
                        allowed_path_prefixes: Vec::new(),
                        require_hash_pin: false,
                        required_sha256_by_plugin: BTreeMap::new(),
                    },
                }),
            }),
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-security-signature-mismatch".to_owned(),
                objective: "signature mismatch should block".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "blocked");
        assert!(report
            .blocked_reason
            .expect("blocked reason should exist")
            .contains("profile signature verification failed"));
    }

    #[tokio::test]
    async fn execute_spec_runs_runtime_extension_and_captures_audit() {
        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-test-pack".to_owned(),
                domain: "engineering".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::from(["crm".to_owned()]),
                granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-spec-test".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: Some(DefaultCoreSelection {
                connector: None,
                runtime: Some("fallback-core".to_owned()),
                tool: None,
                memory: None,
            }),
            self_awareness: None,
            plugin_scan: None,
            bridge_support: None,
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::RuntimeExtension {
                action: "start".to_owned(),
                required_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
                payload: json!({}),
                extension: "acp-bridge".to_owned(),
                core: None,
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "runtime_extension");
        assert_eq!(report.outcome["outcome"]["status"], "ok");
        let events = report.audit_events.expect("audit should be included");
        assert!(events.iter().any(|event| {
            matches!(
                event.kind,
                kernel::AuditEventKind::PlaneInvoked {
                    plane: kernel::ExecutionPlane::Runtime,
                    tier: kernel::PlaneTier::Extension,
                    ..
                }
            )
        }));
    }

    #[tokio::test]
    async fn execute_spec_auto_provisions_provider_and_channel_when_missing() {
        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-auto-provision".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-auto".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
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
            operation: OperationSpec::ConnectorLegacy {
                connector_name: "openrouter".to_owned(),
                operation: "chat".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "connector_legacy");
        assert_eq!(report.outcome["outcome"]["status"], "ok");
        assert_eq!(
            report.outcome["outcome"]["payload"]["provider_id"],
            "openrouter"
        );
        assert!(report.auto_provision_plan.is_some());
        assert!(report.integration_catalog.provider("openrouter").is_some());
        assert!(report.integration_catalog.channel("primary").is_some());
    }

    #[tokio::test]
    async fn execute_spec_applies_hotfix_endpoint_before_invocation() {
        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-hotfix".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-hotfix".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: None,
            bootstrap: None,
            auto_provision: Some(AutoProvisionSpec {
                enabled: true,
                provider_id: "slack".to_owned(),
                channel_id: "alerts".to_owned(),
                connector_name: Some("slack".to_owned()),
                endpoint: Some("https://old.slack.invalid/hook".to_owned()),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
            }),
            hotfixes: vec![HotfixSpec::ChannelEndpoint {
                channel_id: "alerts".to_owned(),
                new_endpoint: "https://hooks.slack.com/services/new".to_owned(),
            }],
            operation: OperationSpec::ConnectorLegacy {
                connector_name: "slack".to_owned(),
                operation: "notify".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({"channel_id": "alerts"}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "connector_legacy");
        assert_eq!(
            report.outcome["outcome"]["payload"]["endpoint"],
            "https://hooks.slack.com/services/new"
        );
    }

    #[tokio::test]
    async fn execute_spec_scans_plugin_files_and_absorbs_them_for_hotplug() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let plugin_root = std::env::temp_dir().join(format!("chumos-plugin-{}", unique));
        fs::create_dir_all(&plugin_root).expect("create plugin root");

        let plugin_file = plugin_root.join("openrouter_plugin.rs");
        fs::write(
            &plugin_file,
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "openrouter-rs",
//   "provider_id": "openrouter",
//   "connector_name": "openrouter",
//   "channel_id": "primary",
//   "endpoint": "https://openrouter.ai/api/v1/chat/completions",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"version":"0.4.0","source":"community"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write plugin file");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-plugin-scan".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-plugin-scan".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![plugin_root.display().to_string()],
            }),
            bridge_support: None,
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::ConnectorLegacy {
                connector_name: "openrouter".to_owned(),
                operation: "chat".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "connector_legacy");
        assert_eq!(report.outcome["outcome"]["status"], "ok");
        assert_eq!(report.plugin_scan_reports.len(), 1);
        assert_eq!(report.plugin_scan_reports[0].matched_plugins, 1);
        assert_eq!(report.plugin_translation_reports.len(), 1);
        assert_eq!(report.plugin_translation_reports[0].translated_plugins, 1);
        assert_eq!(report.plugin_activation_plans.len(), 1);
        assert_eq!(report.plugin_activation_plans[0].ready_plugins, 1);
        assert_eq!(report.plugin_bootstrap_queue.len(), 1);
        assert_eq!(report.plugin_absorb_reports.len(), 1);
        assert_eq!(report.plugin_absorb_reports[0].absorbed_plugins, 1);
        assert!(report.integration_catalog.provider("openrouter").is_some());
        assert!(report.integration_catalog.channel("primary").is_some());
    }

    #[tokio::test]
    async fn execute_spec_blocks_when_bridge_matrix_does_not_support_plugin() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let plugin_root = std::env::temp_dir().join(format!("chumos-plugin-bridge-{}", unique));
        fs::create_dir_all(&plugin_root).expect("create plugin root");

        let plugin_file = plugin_root.join("openrouter_plugin.rs");
        fs::write(
            &plugin_file,
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "openrouter-rs",
//   "provider_id": "openrouter",
//   "connector_name": "openrouter",
//   "channel_id": "primary",
//   "endpoint": "https://openrouter.ai/api/v1/chat/completions",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"version":"0.4.0","source":"community"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write plugin file");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-plugin-bridge-block".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-plugin-bridge-block".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![plugin_root.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::HttpJson],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,

                execute_process_stdio: false,

                execute_http_json: false,

                allowed_process_commands: Vec::new(),

                enforce_execution_success: false,
                security_scan: None,
            }),
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::ConnectorLegacy {
                connector_name: "openrouter".to_owned(),
                operation: "chat".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "blocked");
        assert_eq!(report.outcome["status"], "blocked");
        assert_eq!(report.plugin_activation_plans.len(), 1);
        assert_eq!(report.plugin_activation_plans[0].blocked_plugins, 1);
        assert!(report.plugin_bootstrap_queue.is_empty());
        assert!(report.plugin_absorb_reports.is_empty());
        assert!(report.integration_catalog.provider("openrouter").is_none());
    }

    #[tokio::test]
    async fn execute_spec_skips_blocked_plugins_when_bridge_enforcement_is_disabled() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let plugin_root =
            std::env::temp_dir().join(format!("chumos-plugin-bridge-selective-{}", unique));
        fs::create_dir_all(&plugin_root).expect("create plugin root");

        let rust_plugin = plugin_root.join("openrouter.rs");
        fs::write(
            &rust_plugin,
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "openrouter-rs",
//   "provider_id": "openrouter",
//   "connector_name": "openrouter",
//   "channel_id": "primary",
//   "endpoint": "https://openrouter.ai/api/v1/chat/completions",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"version":"0.4.0"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write rust plugin");

        let http_plugin = plugin_root.join("webhook.js");
        fs::write(
            &http_plugin,
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "webhook-js",
//   "provider_id": "webhookx",
//   "connector_name": "webhookx",
//   "channel_id": "primary",
//   "endpoint": "https://hooks.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write http plugin");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-plugin-bridge-selective".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-plugin-bridge-selective".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![plugin_root.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::HttpJson],
                supported_adapter_families: Vec::new(),
                enforce_supported: false,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,

                execute_process_stdio: false,

                execute_http_json: false,

                allowed_process_commands: Vec::new(),

                enforce_execution_success: false,
                security_scan: None,
            }),
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::ConnectorLegacy {
                connector_name: "webhookx".to_owned(),
                operation: "notify".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "connector_legacy");
        assert_eq!(report.outcome["outcome"]["status"], "ok");
        assert_eq!(report.plugin_activation_plans.len(), 1);
        assert_eq!(report.plugin_activation_plans[0].ready_plugins, 1);
        assert_eq!(report.plugin_activation_plans[0].blocked_plugins, 1);
        assert_eq!(report.plugin_bootstrap_queue.len(), 1);
        assert_eq!(report.plugin_absorb_reports.len(), 1);
        assert_eq!(report.plugin_absorb_reports[0].absorbed_plugins, 1);
        assert!(report.integration_catalog.provider("webhookx").is_some());
        assert!(report.integration_catalog.provider("openrouter").is_none());
    }

    #[tokio::test]
    async fn execute_spec_bootstrap_applies_only_bridges_allowed_by_bootstrap_policy() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let plugin_root =
            std::env::temp_dir().join(format!("chumos-plugin-bootstrap-selective-{}", unique));
        fs::create_dir_all(&plugin_root).expect("create plugin root");

        fs::write(
            plugin_root.join("ffi_plugin.rs"),
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "ffi-plugin",
//   "provider_id": "ffi-provider",
//   "connector_name": "ffi-provider",
//   "channel_id": "primary",
//   "endpoint": "https://ffi.invalid/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"native_ffi","version":"1.0.0"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write ffi plugin");

        fs::write(
            plugin_root.join("http_plugin.js"),
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "http-plugin",
//   "provider_id": "http-provider",
//   "connector_name": "http-provider",
//   "channel_id": "primary",
//   "endpoint": "https://hooks.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write http plugin");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-bootstrap-selective".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-bootstrap-selective".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![plugin_root.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::HttpJson, PluginBridgeKind::NativeFfi],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,

                execute_process_stdio: false,

                execute_http_json: false,

                allowed_process_commands: Vec::new(),

                enforce_execution_success: false,
                security_scan: None,
            }),
            bootstrap: Some(BootstrapSpec {
                enabled: true,
                allow_http_json_auto_apply: Some(true),
                allow_process_stdio_auto_apply: Some(false),
                allow_native_ffi_auto_apply: Some(false),
                allow_wasm_component_auto_apply: Some(false),
                allow_mcp_server_auto_apply: Some(false),
                enforce_ready_execution: Some(false),
                max_tasks: Some(10),
            }),
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::ConnectorLegacy {
                connector_name: "http-provider".to_owned(),
                operation: "notify".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "connector_legacy");
        assert_eq!(report.outcome["outcome"]["status"], "ok");
        assert_eq!(report.plugin_activation_plans.len(), 1);
        assert_eq!(report.plugin_activation_plans[0].ready_plugins, 2);
        assert_eq!(report.plugin_bootstrap_reports.len(), 1);
        assert_eq!(report.plugin_bootstrap_reports[0].applied_tasks, 1);
        assert_eq!(report.plugin_bootstrap_reports[0].deferred_tasks, 1);
        assert_eq!(report.plugin_absorb_reports[0].absorbed_plugins, 1);
        assert_eq!(report.plugin_bootstrap_queue.len(), 1);
        assert!(report
            .integration_catalog
            .provider("http-provider")
            .is_some());
        assert!(report
            .integration_catalog
            .provider("ffi-provider")
            .is_none());
    }

    #[tokio::test]
    async fn execute_spec_bootstrap_enforcement_blocks_when_ready_plugins_are_deferred() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let plugin_root =
            std::env::temp_dir().join(format!("chumos-plugin-bootstrap-enforce-{}", unique));
        fs::create_dir_all(&plugin_root).expect("create plugin root");

        fs::write(
            plugin_root.join("ffi_plugin.rs"),
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "ffi-plugin",
//   "provider_id": "ffi-provider",
//   "connector_name": "ffi-provider",
//   "channel_id": "primary",
//   "endpoint": "https://ffi.invalid/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"native_ffi","version":"1.0.0"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write ffi plugin");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-bootstrap-enforce".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-bootstrap-enforce".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![plugin_root.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::NativeFfi],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,

                execute_process_stdio: false,

                execute_http_json: false,

                allowed_process_commands: Vec::new(),

                enforce_execution_success: false,
                security_scan: None,
            }),
            bootstrap: Some(BootstrapSpec {
                enabled: true,
                allow_http_json_auto_apply: Some(true),
                allow_process_stdio_auto_apply: Some(false),
                allow_native_ffi_auto_apply: Some(false),
                allow_wasm_component_auto_apply: Some(false),
                allow_mcp_server_auto_apply: Some(false),
                enforce_ready_execution: Some(true),
                max_tasks: Some(10),
            }),
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-bootstrap-enforce".to_owned(),
                objective: "must be blocked by bootstrap enforcement".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "blocked");
        assert_eq!(report.outcome["status"], "blocked");
        assert!(report
            .blocked_reason
            .expect("blocked reason must exist")
            .contains("bootstrap policy blocked"));
        assert_eq!(report.plugin_bootstrap_reports.len(), 1);
        assert_eq!(report.plugin_bootstrap_reports[0].applied_tasks, 0);
        assert_eq!(report.plugin_bootstrap_reports[0].deferred_tasks, 1);
        assert!(report.plugin_absorb_reports.is_empty());
        assert!(report
            .integration_catalog
            .provider("ffi-provider")
            .is_none());
    }

    #[tokio::test]
    async fn execute_spec_blocks_on_bridge_support_checksum_mismatch() {
        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-bridge-checksum".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-bridge-checksum".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::HttpJson],
                supported_adapter_families: vec!["http-adapter".to_owned()],
                enforce_supported: true,
                policy_version: Some("v1".to_owned()),
                expected_checksum: Some("deadbeef".to_owned()),
                expected_sha256: None,

                execute_process_stdio: false,

                execute_http_json: false,

                allowed_process_commands: Vec::new(),

                enforce_execution_success: false,
                security_scan: None,
            }),
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-bridge-checksum".to_owned(),
                objective: "should be blocked before execution".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "blocked");
        assert_eq!(report.outcome["status"], "blocked");
        assert!(report
            .blocked_reason
            .expect("blocked reason should be present")
            .contains("checksum mismatch"));
        assert!(report.bridge_support_checksum.is_some());
    }

    #[tokio::test]
    async fn execute_spec_blocks_on_bridge_support_sha256_mismatch() {
        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-bridge-sha256".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-bridge-sha256".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::HttpJson],
                supported_adapter_families: vec!["http-adapter".to_owned()],
                enforce_supported: true,
                policy_version: Some("v2".to_owned()),
                expected_checksum: None,
                expected_sha256: Some("badbad".to_owned()),

                execute_process_stdio: false,

                execute_http_json: false,

                allowed_process_commands: Vec::new(),

                enforce_execution_success: false,
                security_scan: None,
            }),
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-bridge-sha256".to_owned(),
                objective: "should be blocked before execution".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "blocked");
        assert_eq!(report.outcome["status"], "blocked");
        assert!(report
            .blocked_reason
            .expect("blocked reason should be present")
            .contains("sha256 mismatch"));
        assert!(report.bridge_support_sha256.is_some());
    }

    #[tokio::test]
    async fn execute_spec_allows_execution_when_bridge_support_sha256_matches() {
        let mut bridge_support = BridgeSupportSpec {
            enabled: true,
            supported_bridges: vec![PluginBridgeKind::HttpJson, PluginBridgeKind::ProcessStdio],
            supported_adapter_families: vec!["http-adapter".to_owned()],
            enforce_supported: false,
            policy_version: Some("v2".to_owned()),
            expected_checksum: None,
            expected_sha256: None,
            execute_process_stdio: false,
            execute_http_json: false,
            allowed_process_commands: Vec::new(),
            enforce_execution_success: false,
            security_scan: None,
        };
        bridge_support.expected_sha256 = Some(bridge_support_policy_sha256(&bridge_support));

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-bridge-sha256-match".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-bridge-sha256-match".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: Some(bridge_support),
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-bridge-sha256-match".to_owned(),
                objective: "should pass".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "task");
        assert_eq!(report.outcome["outcome"]["status"], "ok");
        assert!(report.blocked_reason.is_none());
        assert!(report.bridge_support_sha256.is_some());
    }

    #[tokio::test]
    async fn execute_spec_enriches_plugin_bridge_metadata_and_emits_bridge_execution() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let plugin_root =
            std::env::temp_dir().join(format!("chumos-plugin-bridge-enrich-{unique}"));
        fs::create_dir_all(&plugin_root).expect("create plugin root");

        fs::write(
            plugin_root.join("ffi_plugin.rs"),
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "ffi-plugin",
//   "provider_id": "ffi-provider",
//   "connector_name": "ffi-provider",
//   "channel_id": "primary",
//   "endpoint": "https://ffi.invalid/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"version":"1.0.0"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write ffi plugin");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-bridge-enrich".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-bridge-enrich".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![plugin_root.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::NativeFfi],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,

                execute_process_stdio: false,

                execute_http_json: false,

                allowed_process_commands: Vec::new(),

                enforce_execution_success: false,
                security_scan: None,
            }),
            bootstrap: Some(BootstrapSpec {
                enabled: true,
                allow_http_json_auto_apply: Some(false),
                allow_process_stdio_auto_apply: Some(false),
                allow_native_ffi_auto_apply: Some(true),
                allow_wasm_component_auto_apply: Some(false),
                allow_mcp_server_auto_apply: Some(false),
                enforce_ready_execution: Some(true),
                max_tasks: Some(10),
            }),
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::ConnectorLegacy {
                connector_name: "ffi-provider".to_owned(),
                operation: "invoke".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({"input":"demo"}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "connector_legacy");
        assert_eq!(report.outcome["outcome"]["status"], "ok");
        assert_eq!(
            report.outcome["outcome"]["payload"]["bridge_execution"]["bridge_kind"],
            "native_ffi"
        );
        assert_eq!(
            report.outcome["outcome"]["payload"]["bridge_execution"]["entrypoint"],
            "lib::invoke"
        );
        assert_eq!(
            report
                .integration_catalog
                .provider("ffi-provider")
                .expect("provider should exist")
                .metadata
                .get("bridge_kind")
                .cloned(),
            Some("native_ffi".to_owned())
        );
    }

    #[tokio::test]
    async fn execute_spec_process_stdio_bridge_executes_when_enabled_and_allowed() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let plugin_root =
            std::env::temp_dir().join(format!("chumos-plugin-process-stdio-run-{unique}"));
        fs::create_dir_all(&plugin_root).expect("create plugin root");

        fs::write(
            plugin_root.join("stdio_plugin.py"),
            r#"
# CHUMOS_PLUGIN_START
# {
#   "plugin_id": "stdio-plugin",
#   "provider_id": "stdio-provider",
#   "connector_name": "stdio-provider",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-provider",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"cat",
#     "version":"1.0.0"
#   }
# }
# CHUMOS_PLUGIN_END
"#,
        )
        .expect("write stdio plugin");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-process-stdio-run".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-process-stdio-run".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![plugin_root.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::ProcessStdio],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,
                execute_process_stdio: true,
                execute_http_json: false,
                allowed_process_commands: vec!["cat".to_owned()],
                enforce_execution_success: true,
                security_scan: None,
            }),
            bootstrap: Some(BootstrapSpec {
                enabled: true,
                allow_http_json_auto_apply: Some(false),
                allow_process_stdio_auto_apply: Some(true),
                allow_native_ffi_auto_apply: Some(false),
                allow_wasm_component_auto_apply: Some(false),
                allow_mcp_server_auto_apply: Some(false),
                enforce_ready_execution: Some(true),
                max_tasks: Some(10),
            }),
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::ConnectorLegacy {
                connector_name: "stdio-provider".to_owned(),
                operation: "invoke".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({"question":"ping"}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "connector_legacy");
        assert_eq!(report.outcome["outcome"]["status"], "ok");
        assert_eq!(
            report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
            "executed"
        );
        assert_eq!(
            report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["executor"],
            "process_stdio_local"
        );
        assert_eq!(
            report.outcome["outcome"]["payload"]["bridge_execution"]["runtime"]["stdout_json"]
                ["operation"],
            "invoke"
        );
    }

    #[tokio::test]
    async fn execute_spec_process_stdio_bridge_blocks_when_command_not_allowlisted() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let plugin_root =
            std::env::temp_dir().join(format!("chumos-plugin-process-stdio-block-{unique}"));
        fs::create_dir_all(&plugin_root).expect("create plugin root");

        fs::write(
            plugin_root.join("stdio_plugin.py"),
            r#"
# CHUMOS_PLUGIN_START
# {
#   "plugin_id": "stdio-plugin",
#   "provider_id": "stdio-provider",
#   "connector_name": "stdio-provider",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-provider",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"cat",
#     "version":"1.0.0"
#   }
# }
# CHUMOS_PLUGIN_END
"#,
        )
        .expect("write stdio plugin");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-process-stdio-block".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-process-stdio-block".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![plugin_root.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::ProcessStdio],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,
                execute_process_stdio: true,
                execute_http_json: false,
                allowed_process_commands: vec!["python3".to_owned()],
                enforce_execution_success: false,
                security_scan: None,
            }),
            bootstrap: Some(BootstrapSpec {
                enabled: true,
                allow_http_json_auto_apply: Some(false),
                allow_process_stdio_auto_apply: Some(true),
                allow_native_ffi_auto_apply: Some(false),
                allow_wasm_component_auto_apply: Some(false),
                allow_mcp_server_auto_apply: Some(false),
                enforce_ready_execution: Some(true),
                max_tasks: Some(10),
            }),
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::ConnectorLegacy {
                connector_name: "stdio-provider".to_owned(),
                operation: "invoke".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeConnector]),
                payload: json!({"question":"ping"}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "connector_legacy");
        assert_eq!(report.outcome["outcome"]["status"], "ok");
        assert_eq!(
            report.outcome["outcome"]["payload"]["bridge_execution"]["status"],
            "blocked"
        );
        assert!(
            report.outcome["outcome"]["payload"]["bridge_execution"]["reason"]
                .as_str()
                .expect("blocked reason should be string")
                .contains("not allowed")
        );
    }

    #[tokio::test]
    async fn execute_spec_security_scan_blocks_wasm_plugin_with_wasi_import() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let plugin_root = std::env::temp_dir().join(format!("chumos-security-wasm-block-{unique}"));
        fs::create_dir_all(&plugin_root).expect("create plugin root");

        fs::write(
            plugin_root.join("plugin.rs"),
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "wasm-risky",
//   "provider_id": "wasm-risky",
//   "connector_name": "wasm-risky",
//   "channel_id": "primary",
//   "endpoint": "local://wasm-risky/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {
//     "bridge_kind":"wasm_component",
//     "component":"plugin.wasm",
//     "version":"1.0.0"
//   }
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write plugin manifest");

        let wasm_bytes = wat::parse_str(
            r#"(module
                 (import "wasi_snapshot_preview1" "fd_write"
                   (func $fd_write (param i32 i32 i32 i32) (result i32)))
               )"#,
        )
        .expect("compile wasm");
        fs::write(plugin_root.join("plugin.wasm"), wasm_bytes).expect("write wasm module");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-security-wasm-block".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-security-wasm-block".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![plugin_root.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::WasmComponent],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,
                execute_process_stdio: false,
                execute_http_json: false,
                allowed_process_commands: Vec::new(),
                enforce_execution_success: false,
                security_scan: Some(SecurityScanSpec {
                    enabled: true,
                    block_on_high: true,
                    profile_path: None,
                    profile_sha256: None,
                    profile_signature: None,
                    siem_export: None,
                    high_risk_metadata_keywords: vec!["shell".to_owned()],
                    wasm: WasmSecurityScanSpec {
                        enabled: true,
                        max_module_bytes: 128 * 1024,
                        allow_wasi: false,
                        blocked_import_prefixes: vec!["wasi".to_owned()],
                        allowed_path_prefixes: vec![plugin_root.display().to_string()],
                        require_hash_pin: false,
                        required_sha256_by_plugin: BTreeMap::new(),
                    },
                }),
            }),
            bootstrap: Some(BootstrapSpec {
                enabled: true,
                allow_http_json_auto_apply: Some(false),
                allow_process_stdio_auto_apply: Some(false),
                allow_native_ffi_auto_apply: Some(false),
                allow_wasm_component_auto_apply: Some(true),
                allow_mcp_server_auto_apply: Some(false),
                enforce_ready_execution: Some(true),
                max_tasks: Some(10),
            }),
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-security-wasm-block".to_owned(),
                objective: "security scan should block risky wasm".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "blocked");
        assert!(report
            .blocked_reason
            .expect("blocked reason should exist")
            .contains("security scan blocked"));
        let security = report
            .security_scan_report
            .expect("security scan report should exist");
        assert!(security.blocked);
        assert!(security.high_findings > 0);
        assert!(security
            .findings
            .iter()
            .any(|finding| finding.category.contains("wasi")));
        let audit = report.audit_events.expect("audit events should exist");
        assert!(audit.iter().any(|event| {
            matches!(
                &event.kind,
                AuditEventKind::SecurityScanEvaluated {
                    blocked,
                    high_findings,
                    ..
                } if *blocked && *high_findings > 0
            )
        }));
    }

    #[tokio::test]
    async fn execute_spec_security_scan_allows_clean_wasm_with_hash_pin() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let plugin_root = std::env::temp_dir().join(format!("chumos-security-wasm-pass-{unique}"));
        fs::create_dir_all(&plugin_root).expect("create plugin root");

        fs::write(
            plugin_root.join("plugin.rs"),
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "wasm-clean",
//   "provider_id": "wasm-clean",
//   "connector_name": "wasm-clean",
//   "channel_id": "primary",
//   "endpoint": "local://wasm-clean/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {
//     "bridge_kind":"wasm_component",
//     "component":"plugin.wasm",
//     "version":"1.0.0"
//   }
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write plugin manifest");

        let wasm_bytes = wat::parse_str(r#"(module (func (export "run")))"#).expect("compile wasm");
        let digest = Sha256::digest(&wasm_bytes);
        let digest_hex = hex_lower(&digest);
        fs::write(plugin_root.join("plugin.wasm"), wasm_bytes).expect("write wasm module");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-security-wasm-pass".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-security-wasm-pass".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![plugin_root.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::WasmComponent],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,
                execute_process_stdio: false,
                execute_http_json: false,
                allowed_process_commands: Vec::new(),
                enforce_execution_success: false,
                security_scan: Some(SecurityScanSpec {
                    enabled: true,
                    block_on_high: true,
                    profile_path: None,
                    profile_sha256: None,
                    profile_signature: None,
                    siem_export: None,
                    high_risk_metadata_keywords: vec!["shell".to_owned()],
                    wasm: WasmSecurityScanSpec {
                        enabled: true,
                        max_module_bytes: 128 * 1024,
                        allow_wasi: false,
                        blocked_import_prefixes: vec!["wasi".to_owned()],
                        allowed_path_prefixes: vec![plugin_root.display().to_string()],
                        require_hash_pin: true,
                        required_sha256_by_plugin: BTreeMap::from([(
                            "wasm-clean".to_owned(),
                            digest_hex.clone(),
                        )]),
                    },
                }),
            }),
            bootstrap: Some(BootstrapSpec {
                enabled: true,
                allow_http_json_auto_apply: Some(false),
                allow_process_stdio_auto_apply: Some(false),
                allow_native_ffi_auto_apply: Some(false),
                allow_wasm_component_auto_apply: Some(true),
                allow_mcp_server_auto_apply: Some(false),
                enforce_ready_execution: Some(true),
                max_tasks: Some(10),
            }),
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-security-wasm-pass".to_owned(),
                objective: "security scan should allow clean wasm".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "task");
        assert_eq!(report.outcome["outcome"]["status"], "ok");
        let security = report
            .security_scan_report
            .expect("security scan report should exist");
        assert!(!security.blocked);
        assert_eq!(security.high_findings, 0);
        assert!(security
            .findings
            .iter()
            .any(|finding| finding.category == "wasm_digest_observed"));
        assert!(report.integration_catalog.provider("wasm-clean").is_some());
    }

    #[tokio::test]
    async fn execute_spec_security_scan_emits_audit_summary_when_not_blocking() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let plugin_root = std::env::temp_dir().join(format!("chumos-security-audit-pass-{unique}"));
        fs::create_dir_all(&plugin_root).expect("create plugin root");

        fs::write(
            plugin_root.join("plugin.py"),
            r#"
# CHUMOS_PLUGIN_START
# {
#   "plugin_id": "stdio-audit",
#   "provider_id": "stdio-audit",
#   "connector_name": "stdio-audit",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-audit/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"python3",
#     "version":"1.0.0"
#   }
# }
# CHUMOS_PLUGIN_END
"#,
        )
        .expect("write plugin manifest");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-security-audit-pass".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-security-audit-pass".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![plugin_root.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::ProcessStdio],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,
                execute_process_stdio: false,
                execute_http_json: false,
                allowed_process_commands: vec!["cat".to_owned()],
                enforce_execution_success: false,
                security_scan: Some(SecurityScanSpec {
                    enabled: true,
                    block_on_high: false,
                    profile_path: None,
                    profile_sha256: None,
                    profile_signature: None,
                    siem_export: None,
                    high_risk_metadata_keywords: Vec::new(),
                    wasm: WasmSecurityScanSpec {
                        enabled: false,
                        max_module_bytes: 0,
                        allow_wasi: false,
                        blocked_import_prefixes: Vec::new(),
                        allowed_path_prefixes: Vec::new(),
                        require_hash_pin: false,
                        required_sha256_by_plugin: BTreeMap::new(),
                    },
                }),
            }),
            bootstrap: Some(BootstrapSpec {
                enabled: true,
                allow_http_json_auto_apply: Some(false),
                allow_process_stdio_auto_apply: Some(true),
                allow_native_ffi_auto_apply: Some(false),
                allow_wasm_component_auto_apply: Some(false),
                allow_mcp_server_auto_apply: Some(false),
                enforce_ready_execution: Some(false),
                max_tasks: Some(5),
            }),
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-security-audit-pass".to_owned(),
                objective: "security scan should emit audit summary".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "task");
        let security = report
            .security_scan_report
            .expect("security scan report should exist");
        assert!(!security.blocked);
        assert!(security.high_findings >= 1);

        let audit = report.audit_events.expect("audit events should exist");
        let summary = audit.iter().find_map(|event| match &event.kind {
            AuditEventKind::SecurityScanEvaluated {
                blocked,
                high_findings,
                categories,
                finding_ids,
                ..
            } => Some((
                *blocked,
                *high_findings,
                categories.clone(),
                finding_ids.clone(),
            )),
            _ => None,
        });

        let (blocked, high_findings, categories, finding_ids) =
            summary.expect("security scan audit summary should exist");
        assert!(!blocked);
        assert!(high_findings >= 1);
        assert!(categories
            .iter()
            .any(|value| value == "process_command_not_allowlisted"));
        assert!(!finding_ids.is_empty());
        assert!(finding_ids.iter().all(|value| value.starts_with("sf-")));
    }

    #[tokio::test]
    async fn execute_spec_security_scan_exports_siem_record_with_truncation() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let plugin_root = std::env::temp_dir().join(format!("chumos-security-siem-pass-{unique}"));
        let siem_path =
            std::env::temp_dir().join(format!("chumos-security-siem-pass-{unique}.jsonl"));
        fs::create_dir_all(&plugin_root).expect("create plugin root");

        fs::write(
            plugin_root.join("plugin.py"),
            r#"
# CHUMOS_PLUGIN_START
# {
#   "plugin_id": "stdio-siem",
#   "provider_id": "stdio-siem",
#   "connector_name": "stdio-siem",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-siem/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"python3",
#     "note":"shell-enabled",
#     "version":"1.0.0"
#   }
# }
# CHUMOS_PLUGIN_END
"#,
        )
        .expect("write plugin manifest");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-security-siem-pass".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-security-siem-pass".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![plugin_root.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::ProcessStdio],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,
                execute_process_stdio: false,
                execute_http_json: false,
                allowed_process_commands: vec!["cat".to_owned()],
                enforce_execution_success: false,
                security_scan: Some(SecurityScanSpec {
                    enabled: true,
                    block_on_high: false,
                    profile_path: None,
                    profile_sha256: None,
                    profile_signature: None,
                    siem_export: Some(SecuritySiemExportSpec {
                        enabled: true,
                        path: siem_path.display().to_string(),
                        include_findings: true,
                        max_findings_per_record: Some(1),
                        fail_on_error: true,
                    }),
                    high_risk_metadata_keywords: vec!["shell".to_owned()],
                    wasm: WasmSecurityScanSpec {
                        enabled: false,
                        max_module_bytes: 0,
                        allow_wasi: false,
                        blocked_import_prefixes: Vec::new(),
                        allowed_path_prefixes: Vec::new(),
                        require_hash_pin: false,
                        required_sha256_by_plugin: BTreeMap::new(),
                    },
                }),
            }),
            bootstrap: Some(BootstrapSpec {
                enabled: true,
                allow_http_json_auto_apply: Some(false),
                allow_process_stdio_auto_apply: Some(true),
                allow_native_ffi_auto_apply: Some(false),
                allow_wasm_component_auto_apply: Some(false),
                allow_mcp_server_auto_apply: Some(false),
                enforce_ready_execution: Some(false),
                max_tasks: Some(5),
            }),
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-security-siem-pass".to_owned(),
                objective: "security scan should export siem record".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "task");
        let security = report
            .security_scan_report
            .expect("security scan report should exist");
        let siem = security
            .siem_export
            .expect("siem export report should exist");
        assert!(siem.success);
        assert_eq!(siem.exported_records, 1);
        assert_eq!(siem.exported_findings, 1);
        assert!(siem.truncated_findings >= 1);

        let siem_body = fs::read_to_string(&siem_path).expect("read siem record");
        let first_line = siem_body.lines().next().expect("one siem line");
        let record: Value = serde_json::from_str(first_line).expect("parse siem json");
        assert_eq!(record["event_type"], "security_scan_report");
        assert_eq!(record["pack_id"], "spec-security-siem-pass");
        assert_eq!(record["agent_id"], "agent-security-siem-pass");
        assert!(record["findings"].as_array().map_or(0, Vec::len) == 1);
        assert!(record["truncated_findings"].as_u64().unwrap_or_default() >= 1);
        assert!(record["finding_ids"].as_array().map_or(0, Vec::len) >= 2);
    }

    #[tokio::test]
    async fn execute_spec_security_scan_siem_fail_closed_blocks_execution() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let plugin_root = std::env::temp_dir().join(format!("chumos-security-siem-block-{unique}"));
        let invalid_parent =
            std::env::temp_dir().join(format!("chumos-security-siem-parent-file-{unique}.tmp"));
        let invalid_siem_path = invalid_parent.join("events.jsonl");
        fs::create_dir_all(&plugin_root).expect("create plugin root");
        fs::write(&invalid_parent, "not-a-directory").expect("create invalid parent marker file");

        fs::write(
            plugin_root.join("plugin.py"),
            r#"
# CHUMOS_PLUGIN_START
# {
#   "plugin_id": "stdio-siem-block",
#   "provider_id": "stdio-siem-block",
#   "connector_name": "stdio-siem-block",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-siem-block/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"python3",
#     "version":"1.0.0"
#   }
# }
# CHUMOS_PLUGIN_END
"#,
        )
        .expect("write plugin manifest");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-security-siem-block".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-security-siem-block".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![plugin_root.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::ProcessStdio],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,
                execute_process_stdio: false,
                execute_http_json: false,
                allowed_process_commands: vec!["cat".to_owned()],
                enforce_execution_success: false,
                security_scan: Some(SecurityScanSpec {
                    enabled: true,
                    block_on_high: false,
                    profile_path: None,
                    profile_sha256: None,
                    profile_signature: None,
                    siem_export: Some(SecuritySiemExportSpec {
                        enabled: true,
                        path: invalid_siem_path.display().to_string(),
                        include_findings: true,
                        max_findings_per_record: None,
                        fail_on_error: true,
                    }),
                    high_risk_metadata_keywords: Vec::new(),
                    wasm: WasmSecurityScanSpec {
                        enabled: false,
                        max_module_bytes: 0,
                        allow_wasi: false,
                        blocked_import_prefixes: Vec::new(),
                        allowed_path_prefixes: Vec::new(),
                        require_hash_pin: false,
                        required_sha256_by_plugin: BTreeMap::new(),
                    },
                }),
            }),
            bootstrap: Some(BootstrapSpec {
                enabled: true,
                allow_http_json_auto_apply: Some(false),
                allow_process_stdio_auto_apply: Some(true),
                allow_native_ffi_auto_apply: Some(false),
                allow_wasm_component_auto_apply: Some(false),
                allow_mcp_server_auto_apply: Some(false),
                enforce_ready_execution: Some(false),
                max_tasks: Some(5),
            }),
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-security-siem-block".to_owned(),
                objective: "siem fail closed should block".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "blocked");
        assert!(report
            .blocked_reason
            .expect("blocked reason should exist")
            .contains("siem export failed"));
        let security = report
            .security_scan_report
            .expect("security scan report should exist");
        let siem = security
            .siem_export
            .expect("siem export report should exist");
        assert!(!siem.success);
        assert!(siem.error.is_some());
    }

    #[tokio::test]
    async fn execute_spec_security_scan_covers_deferred_plugins_not_only_applied_subset() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let plugin_root =
            std::env::temp_dir().join(format!("chumos-security-deferred-ready-{unique}"));
        fs::create_dir_all(&plugin_root).expect("create plugin root");

        fs::write(
            plugin_root.join("01-safe.py"),
            r#"
# CHUMOS_PLUGIN_START
# {
#   "plugin_id": "stdio-safe",
#   "provider_id": "stdio-safe",
#   "connector_name": "stdio-safe",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-safe/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"cat",
#     "version":"1.0.0"
#   }
# }
# CHUMOS_PLUGIN_END
"#,
        )
        .expect("write safe plugin");

        fs::write(
            plugin_root.join("02-risky.py"),
            r#"
# CHUMOS_PLUGIN_START
# {
#   "plugin_id": "stdio-risky",
#   "provider_id": "stdio-risky",
#   "connector_name": "stdio-risky",
#   "channel_id": "primary",
#   "endpoint": "local://stdio-risky/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind":"process_stdio",
#     "command":"python3",
#     "version":"1.0.0"
#   }
# }
# CHUMOS_PLUGIN_END
"#,
        )
        .expect("write risky plugin");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-security-deferred-ready".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-security-deferred-ready".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![plugin_root.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::ProcessStdio],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,
                execute_process_stdio: false,
                execute_http_json: false,
                allowed_process_commands: vec!["cat".to_owned()],
                enforce_execution_success: false,
                security_scan: Some(SecurityScanSpec {
                    enabled: true,
                    block_on_high: true,
                    profile_path: None,
                    profile_sha256: None,
                    profile_signature: None,
                    siem_export: None,
                    high_risk_metadata_keywords: Vec::new(),
                    wasm: WasmSecurityScanSpec {
                        enabled: false,
                        max_module_bytes: 0,
                        allow_wasi: false,
                        blocked_import_prefixes: Vec::new(),
                        allowed_path_prefixes: Vec::new(),
                        require_hash_pin: false,
                        required_sha256_by_plugin: BTreeMap::new(),
                    },
                }),
            }),
            bootstrap: Some(BootstrapSpec {
                enabled: true,
                allow_http_json_auto_apply: Some(false),
                allow_process_stdio_auto_apply: Some(true),
                allow_native_ffi_auto_apply: Some(false),
                allow_wasm_component_auto_apply: Some(false),
                allow_mcp_server_auto_apply: Some(false),
                enforce_ready_execution: Some(false),
                max_tasks: Some(1),
            }),
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-security-deferred-ready".to_owned(),
                objective: "security scan must inspect deferred ready plugins".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "blocked");
        assert!(report
            .blocked_reason
            .expect("blocked reason should exist")
            .contains("security scan blocked"));
        assert_eq!(report.plugin_bootstrap_reports.len(), 1);
        assert!(report.plugin_bootstrap_reports[0].total_tasks >= 1);
        assert_eq!(report.plugin_scan_reports[0].matched_plugins, 2);

        let security = report
            .security_scan_report
            .expect("security scan report should exist");
        assert!(security.blocked);
        assert!(security.high_findings >= 1);
        assert!(security
            .findings
            .iter()
            .any(|finding| finding.plugin_id == "stdio-risky"));
        assert!(report.plugin_absorb_reports.is_empty());
    }

    #[tokio::test]
    async fn execute_spec_default_medium_policy_blocks_high_risk_tool_call_without_approval() {
        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-approval-default-block".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-approval-default".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: None,
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::ToolCore {
                tool_name: "delete-file".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeTool]),
                payload: json!({"path":"/tmp/demo.txt"}),
                core: None,
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "blocked");
        assert_eq!(report.outcome["status"], "blocked");
        assert!(report.approval_guard.requires_human_approval);
        assert!(!report.approval_guard.approved);
        assert!(report
            .blocked_reason
            .expect("blocked reason should exist")
            .contains("human approval required"));
    }

    #[tokio::test]
    async fn execute_spec_per_call_approval_allows_high_risk_tool_call() {
        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-approval-per-call".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-approval-per-call".to_owned(),
            ttl_s: 120,
            approval: Some(HumanApprovalSpec {
                mode: HumanApprovalMode::MediumBalanced,
                strategy: HumanApprovalStrategy::PerCall,
                approved_calls: vec!["tool_core:delete-file".to_owned()],
                ..HumanApprovalSpec::default()
            }),
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: None,
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::ToolCore {
                tool_name: "delete-file".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeTool]),
                payload: json!({"path":"/tmp/demo.txt"}),
                core: None,
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "tool_core");
        assert_eq!(report.outcome["outcome"]["status"], "ok");
        assert!(report.approval_guard.requires_human_approval);
        assert!(report.approval_guard.approved);
    }

    #[tokio::test]
    async fn execute_spec_one_time_full_access_allows_high_risk_tool_call() {
        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-approval-once-full".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-approval-once-full".to_owned(),
            ttl_s: 120,
            approval: Some(HumanApprovalSpec {
                mode: HumanApprovalMode::MediumBalanced,
                strategy: HumanApprovalStrategy::OneTimeFullAccess,
                one_time_full_access_granted: true,
                ..HumanApprovalSpec::default()
            }),
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: None,
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::ToolCore {
                tool_name: "delete-file".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeTool]),
                payload: json!({"path":"/tmp/demo.txt"}),
                core: None,
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "tool_core");
        assert_eq!(report.outcome["outcome"]["status"], "ok");
        assert!(report.approval_guard.requires_human_approval);
        assert!(report.approval_guard.approved);
    }

    #[tokio::test]
    async fn execute_spec_strict_mode_requires_approval_for_low_risk_tool_call() {
        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-approval-strict".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-approval-strict".to_owned(),
            ttl_s: 120,
            approval: Some(HumanApprovalSpec {
                mode: HumanApprovalMode::Strict,
                strategy: HumanApprovalStrategy::PerCall,
                ..HumanApprovalSpec::default()
            }),
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: None,
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::ToolCore {
                tool_name: "read-schema".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeTool]),
                payload: json!({"scope":"analytics"}),
                core: None,
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "blocked");
        assert!(report.approval_guard.requires_human_approval);
        assert!(!report.approval_guard.approved);
    }

    #[tokio::test]
    async fn execute_spec_default_medium_policy_allows_low_risk_tool_call_without_approval() {
        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-approval-default-allow".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-approval-default-allow".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: None,
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::ToolCore {
                tool_name: "list-schema".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeTool]),
                payload: json!({"scope":"analytics"}),
                core: None,
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "tool_core");
        assert_eq!(report.outcome["outcome"]["status"], "ok");
        assert!(!report.approval_guard.requires_human_approval);
        assert!(report.approval_guard.approved);
        assert_eq!(report.approval_guard.risk_level, ApprovalRiskLevel::Low);
    }

    #[tokio::test]
    async fn execute_spec_denylist_overrides_other_approvals() {
        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-approval-denylist".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-approval-denylist".to_owned(),
            ttl_s: 120,
            approval: Some(HumanApprovalSpec {
                mode: HumanApprovalMode::Disabled,
                strategy: HumanApprovalStrategy::PerCall,
                approved_calls: vec!["tool_core:delete-file".to_owned()],
                denied_calls: vec!["tool_core:delete-file".to_owned()],
                ..HumanApprovalSpec::default()
            }),
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: None,
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::ToolCore {
                tool_name: "delete-file".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeTool]),
                payload: json!({"path":"/tmp/demo.txt"}),
                core: None,
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "blocked");
        assert!(report.approval_guard.denylisted);
        assert!(!report.approval_guard.approved);
        assert!(report
            .blocked_reason
            .expect("blocked reason should exist")
            .contains("denylisted"));
    }

    #[tokio::test]
    async fn execute_spec_one_time_full_access_expired_is_rejected() {
        let now = current_epoch_s();
        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-approval-full-expired".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-approval-full-expired".to_owned(),
            ttl_s: 120,
            approval: Some(HumanApprovalSpec {
                mode: HumanApprovalMode::Strict,
                strategy: HumanApprovalStrategy::OneTimeFullAccess,
                one_time_full_access_granted: true,
                one_time_full_access_expires_at_epoch_s: Some(now.saturating_sub(1)),
                one_time_full_access_remaining_uses: Some(1),
                ..HumanApprovalSpec::default()
            }),
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: None,
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::ToolCore {
                tool_name: "delete-file".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeTool]),
                payload: json!({"path":"/tmp/demo.txt"}),
                core: None,
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "blocked");
        assert!(!report.approval_guard.approved);
        assert!(report.approval_guard.reason.contains("expired"));
    }

    #[tokio::test]
    async fn execute_spec_one_time_full_access_with_zero_remaining_uses_is_rejected() {
        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-approval-full-zero-uses".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-approval-full-zero-uses".to_owned(),
            ttl_s: 120,
            approval: Some(HumanApprovalSpec {
                mode: HumanApprovalMode::Strict,
                strategy: HumanApprovalStrategy::OneTimeFullAccess,
                one_time_full_access_granted: true,
                one_time_full_access_remaining_uses: Some(0),
                ..HumanApprovalSpec::default()
            }),
            defaults: None,
            self_awareness: None,
            plugin_scan: None,
            bridge_support: None,
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::ToolCore {
                tool_name: "delete-file".to_owned(),
                required_capabilities: BTreeSet::from([Capability::InvokeTool]),
                payload: json!({"path":"/tmp/demo.txt"}),
                core: None,
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "blocked");
        assert!(!report.approval_guard.approved);
        assert!(report.approval_guard.reason.contains("no remaining uses"));
    }

    #[tokio::test]
    async fn execute_spec_bootstrap_max_tasks_limits_applied_plugins() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let plugin_root =
            std::env::temp_dir().join(format!("chumos-plugin-bootstrap-limit-{unique}"));
        fs::create_dir_all(&plugin_root).expect("create plugin root");

        fs::write(
            plugin_root.join("http_a.js"),
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "http-a",
//   "provider_id": "http-a",
//   "connector_name": "http-a",
//   "channel_id": "primary",
//   "endpoint": "https://a.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write http plugin a");

        fs::write(
            plugin_root.join("http_b.js"),
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "http-b",
//   "provider_id": "http-b",
//   "connector_name": "http-b",
//   "channel_id": "primary",
//   "endpoint": "https://b.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write http plugin b");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-bootstrap-limit".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-bootstrap-limit".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![plugin_root.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::HttpJson],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,

                execute_process_stdio: false,

                execute_http_json: false,

                allowed_process_commands: Vec::new(),

                enforce_execution_success: false,
                security_scan: None,
            }),
            bootstrap: Some(BootstrapSpec {
                enabled: true,
                allow_http_json_auto_apply: Some(true),
                allow_process_stdio_auto_apply: Some(false),
                allow_native_ffi_auto_apply: Some(false),
                allow_wasm_component_auto_apply: Some(false),
                allow_mcp_server_auto_apply: Some(false),
                enforce_ready_execution: Some(false),
                max_tasks: Some(1),
            }),
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-bootstrap-limit".to_owned(),
                objective: "run regardless of selective bootstrap".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "task");
        assert_eq!(report.outcome["outcome"]["status"], "ok");
        assert_eq!(report.plugin_bootstrap_reports.len(), 1);
        assert_eq!(report.plugin_bootstrap_reports[0].applied_tasks, 1);
        assert_eq!(report.plugin_bootstrap_reports[0].skipped_tasks, 1);
        assert_eq!(report.plugin_absorb_reports.len(), 1);
        assert_eq!(report.plugin_absorb_reports[0].absorbed_plugins, 1);
    }

    #[tokio::test]
    async fn execute_spec_scans_multiple_roots_and_absorbs_per_root() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();

        let root_a = std::env::temp_dir().join(format!("chumos-plugin-root-a-{unique}"));
        let root_b = std::env::temp_dir().join(format!("chumos-plugin-root-b-{unique}"));
        fs::create_dir_all(&root_a).expect("create root a");
        fs::create_dir_all(&root_b).expect("create root b");

        fs::write(
            root_a.join("a.js"),
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "root-a",
//   "provider_id": "root-a",
//   "connector_name": "root-a",
//   "channel_id": "primary",
//   "endpoint": "https://a.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write root a plugin");

        fs::write(
            root_b.join("b.js"),
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "root-b",
//   "provider_id": "root-b",
//   "connector_name": "root-b",
//   "channel_id": "primary",
//   "endpoint": "https://b.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write root b plugin");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-multi-root".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-multi-root".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![root_a.display().to_string(), root_b.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::HttpJson],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,

                execute_process_stdio: false,

                execute_http_json: false,

                allowed_process_commands: Vec::new(),

                enforce_execution_success: false,
                security_scan: None,
            }),
            bootstrap: Some(BootstrapSpec {
                enabled: true,
                allow_http_json_auto_apply: Some(true),
                allow_process_stdio_auto_apply: Some(false),
                allow_native_ffi_auto_apply: Some(false),
                allow_wasm_component_auto_apply: Some(false),
                allow_mcp_server_auto_apply: Some(false),
                enforce_ready_execution: Some(true),
                max_tasks: Some(8),
            }),
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-multi-root".to_owned(),
                objective: "validate multi-root scan".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "task");
        assert_eq!(report.plugin_scan_reports.len(), 2);
        assert_eq!(report.plugin_absorb_reports.len(), 2);
        let absorbed_total: usize = report
            .plugin_absorb_reports
            .iter()
            .map(|entry| entry.absorbed_plugins)
            .sum();
        assert_eq!(absorbed_total, 2);
        assert!(report.integration_catalog.provider("root-a").is_some());
        assert!(report.integration_catalog.provider("root-b").is_some());
    }

    #[tokio::test]
    async fn execute_spec_plugin_scan_is_transactional_when_blocked() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();

        let root_a = std::env::temp_dir().join(format!("chumos-plugin-rollback-a-{unique}"));
        let root_b = std::env::temp_dir().join(format!("chumos-plugin-rollback-b-{unique}"));
        fs::create_dir_all(&root_a).expect("create root a");
        fs::create_dir_all(&root_b).expect("create root b");

        fs::write(
            root_a.join("a.js"),
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "rollback-a",
//   "provider_id": "rollback-a",
//   "connector_name": "rollback-a",
//   "channel_id": "primary",
//   "endpoint": "https://a.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write root a plugin");

        fs::write(
            root_b.join("b.rs"),
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "rollback-b",
//   "provider_id": "rollback-b",
//   "connector_name": "rollback-b",
//   "channel_id": "primary",
//   "endpoint": "https://b.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"native_ffi","version":"1.0.0"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write root b plugin");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-plugin-rollback".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-plugin-rollback".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![root_a.display().to_string(), root_b.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::HttpJson],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,

                execute_process_stdio: false,

                execute_http_json: false,

                allowed_process_commands: Vec::new(),

                enforce_execution_success: false,
                security_scan: None,
            }),
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-plugin-rollback".to_owned(),
                objective: "must block and rollback staged plugin absorb".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "blocked");
        assert!(report
            .blocked_reason
            .expect("blocked reason")
            .contains("bridge support enforcement blocked"));
        assert_eq!(report.plugin_scan_reports.len(), 2);
        assert!(report.plugin_absorb_reports.is_empty());
        assert!(report.integration_catalog.provider("rollback-a").is_none());
        assert!(report.integration_catalog.provider("rollback-b").is_none());
    }

    #[tokio::test]
    async fn execute_spec_bootstrap_budget_is_global_across_multiple_roots() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();

        let root_a = std::env::temp_dir().join(format!("chumos-bootstrap-global-a-{unique}"));
        let root_b = std::env::temp_dir().join(format!("chumos-bootstrap-global-b-{unique}"));
        fs::create_dir_all(&root_a).expect("create root a");
        fs::create_dir_all(&root_b).expect("create root b");

        fs::write(
            root_a.join("a.js"),
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "global-a",
//   "provider_id": "global-a",
//   "connector_name": "global-a",
//   "channel_id": "primary",
//   "endpoint": "https://a.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write root a plugin");

        fs::write(
            root_b.join("b.js"),
            r#"
// CHUMOS_PLUGIN_START
// {
//   "plugin_id": "global-b",
//   "provider_id": "global-b",
//   "connector_name": "global-b",
//   "channel_id": "primary",
//   "endpoint": "https://b.example.com/invoke",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"bridge_kind":"http_json","version":"1.0.0"}
// }
// CHUMOS_PLUGIN_END
"#,
        )
        .expect("write root b plugin");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-bootstrap-global".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-bootstrap-global".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec![root_a.display().to_string(), root_b.display().to_string()],
            }),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![PluginBridgeKind::HttpJson],
                supported_adapter_families: Vec::new(),
                enforce_supported: true,
                policy_version: None,
                expected_checksum: None,
                expected_sha256: None,

                execute_process_stdio: false,

                execute_http_json: false,

                allowed_process_commands: Vec::new(),

                enforce_execution_success: false,
                security_scan: None,
            }),
            bootstrap: Some(BootstrapSpec {
                enabled: true,
                allow_http_json_auto_apply: Some(true),
                allow_process_stdio_auto_apply: Some(false),
                allow_native_ffi_auto_apply: Some(false),
                allow_wasm_component_auto_apply: Some(false),
                allow_mcp_server_auto_apply: Some(false),
                enforce_ready_execution: Some(false),
                max_tasks: Some(1),
            }),
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-bootstrap-global".to_owned(),
                objective: "max_tasks must be global across roots".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "task");
        assert_eq!(report.plugin_bootstrap_reports.len(), 2);
        let total_applied: usize = report
            .plugin_bootstrap_reports
            .iter()
            .map(|entry| entry.applied_tasks)
            .sum();
        let total_skipped: usize = report
            .plugin_bootstrap_reports
            .iter()
            .map(|entry| entry.skipped_tasks)
            .sum();
        assert_eq!(total_applied, 1);
        assert_eq!(total_skipped, 1);

        let total_absorbed: usize = report
            .plugin_absorb_reports
            .iter()
            .map(|entry| entry.absorbed_plugins)
            .sum();
        assert_eq!(total_absorbed, 1);
        assert!(report.integration_catalog.provider("global-a").is_some());
        assert!(report.integration_catalog.provider("global-b").is_none());
    }

    #[tokio::test]
    async fn execute_spec_allows_execution_with_clean_architecture_guard() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("chumos-guard-clean-{unique}"));
        fs::create_dir_all(&root).expect("create awareness root");
        fs::write(root.join("pack.md"), "# awareness\n").expect("write awareness file");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-guard-clean".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-guard-clean".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: Some(SelfAwarenessSpec {
                enabled: true,
                roots: vec![root.display().to_string()],
                plugin_roots: Vec::new(),
                proposed_mutations: vec!["examples/spec/runtime-extension.json".to_owned()],
                enforce_guard: true,
                immutable_core_paths: Vec::new(),
                mutable_extension_paths: Vec::new(),
            }),
            plugin_scan: None,
            bridge_support: None,
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-guard-clean".to_owned(),
                objective: "run with clean guard".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "task");
        assert_eq!(report.outcome["outcome"]["status"], "ok");
        assert!(report.blocked_reason.is_none());
        assert!(report.self_awareness.is_some());
        assert!(report
            .architecture_guard
            .expect("guard report should be present")
            .denied_paths
            .is_empty());
    }

    #[tokio::test]
    async fn execute_spec_blocks_when_architecture_guard_detects_core_mutation() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("chumos-guard-{unique}"));
        fs::create_dir_all(&root).expect("create awareness root");
        fs::write(root.join("notes.md"), "# guard demo\n").expect("write awareness file");

        let spec = RunnerSpec {
            pack: VerticalPackManifest {
                pack_id: "spec-guard-block".to_owned(),
                domain: "ops".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::new(),
                metadata: BTreeMap::new(),
            },
            agent_id: "agent-guard".to_owned(),
            ttl_s: 120,
            approval: None,
            defaults: None,
            self_awareness: Some(SelfAwarenessSpec {
                enabled: true,
                roots: vec![root.display().to_string()],
                plugin_roots: Vec::new(),
                proposed_mutations: vec![
                    "crates/kernel/src/kernel.rs".to_owned(),
                    "examples/spec/runtime-extension.json".to_owned(),
                ],
                enforce_guard: true,
                immutable_core_paths: Vec::new(),
                mutable_extension_paths: Vec::new(),
            }),
            plugin_scan: None,
            bridge_support: None,
            bootstrap: None,
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "t-guard".to_owned(),
                objective: "should not run".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
            },
        };

        let report = execute_spec(spec, true).await;
        assert_eq!(report.operation_kind, "blocked");
        assert_eq!(report.outcome["status"], "blocked");
        assert!(report.blocked_reason.is_some());
        assert!(report
            .architecture_guard
            .expect("guard report should be present")
            .has_denials());
    }
}
