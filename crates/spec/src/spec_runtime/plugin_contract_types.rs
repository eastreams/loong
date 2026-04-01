use super::*;

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

    pub fn plugin_trust_guard_template() -> Self {
        Self {
            pack: VerticalPackManifest {
                pack_id: "community-plugin-intake".to_owned(),
                domain: "platform".to_owned(),
                version: "0.1.0".to_owned(),
                default_route: ExecutionRoute {
                    harness_kind: HarnessKind::EmbeddedPi,
                    adapter: Some("pi-local".to_owned()),
                },
                allowed_connectors: BTreeSet::new(),
                granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
                metadata: BTreeMap::from([
                    ("owner".to_owned(), "platform-team".to_owned()),
                    ("stage".to_owned(), "plugin-trust-guard".to_owned()),
                ]),
            },
            agent_id: "agent-plugin-trust-guard".to_owned(),
            ttl_s: 120,
            approval: Some(HumanApprovalSpec {
                mode: HumanApprovalMode::Disabled,
                ..HumanApprovalSpec::default()
            }),
            defaults: None,
            self_awareness: None,
            plugin_scan: Some(PluginScanSpec {
                enabled: true,
                roots: vec!["plugins".to_owned()],
            }),
            plugin_setup_readiness: Some(PluginSetupReadinessSpec::default()),
            bridge_support: Some(BridgeSupportSpec {
                enabled: true,
                supported_bridges: vec![
                    PluginBridgeKind::ProcessStdio,
                    PluginBridgeKind::NativeFfi,
                    PluginBridgeKind::WasmComponent,
                    PluginBridgeKind::McpServer,
                    PluginBridgeKind::AcpBridge,
                    PluginBridgeKind::AcpRuntime,
                ],
                supported_adapter_families: Vec::new(),
                supported_compatibility_modes: Vec::new(),
                supported_compatibility_shims: Vec::new(),
                supported_compatibility_shim_profiles: Vec::new(),
                enforce_supported: true,
                policy_version: Some("v1".to_owned()),
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
                allow_process_stdio_auto_apply: Some(true),
                allow_native_ffi_auto_apply: Some(false),
                allow_wasm_component_auto_apply: Some(false),
                allow_mcp_server_auto_apply: Some(false),
                allow_acp_bridge_auto_apply: Some(false),
                allow_acp_runtime_auto_apply: Some(false),
                block_unverified_high_risk_auto_apply: Some(true),
                enforce_ready_execution: Some(true),
                max_tasks: Some(16),
            }),
            auto_provision: None,
            hotfixes: Vec::new(),
            operation: OperationSpec::Task {
                task_id: "review-plugin-trust-guard".to_owned(),
                objective: "review whether scanned plugins are eligible for bootstrap auto-apply under the trust policy".to_owned(),
                required_capabilities: BTreeSet::new(),
                payload: json!({}),
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
    pub plugin_trust_summary: PluginTrustSummary,
    pub tool_search_summary: Option<ToolSearchOperationSummary>,
    pub plugin_bootstrap_queue: Vec<String>,
    pub plugin_absorb_reports: Vec<PluginAbsorbReport>,
    pub security_scan_report: Option<SecurityScanReport>,
    pub auto_provision_plan: Option<ProvisionPlan>,
    pub outcome: Value,
    pub integration_catalog: IntegrationCatalog,
    pub audit_events: Option<Vec<AuditEvent>>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PluginTrustSummary {
    pub scanned_plugins: usize,
    pub official_plugins: usize,
    pub verified_community_plugins: usize,
    pub unverified_plugins: usize,
    pub high_risk_plugins: usize,
    pub high_risk_unverified_plugins: usize,
    pub blocked_auto_apply_plugins: usize,
    pub review_required_plugins: Vec<PluginTrustReviewEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginTrustReviewEntry {
    pub plugin_id: String,
    pub source_path: String,
    pub provenance_summary: String,
    pub trust_tier: PluginTrustTier,
    pub bridge_kind: PluginBridgeKind,
    pub activation_status: PluginActivationStatus,
    pub bootstrap_status: Option<BootstrapTaskStatus>,
    pub reason: String,
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
    pub provenance_summary: Option<String>,
    pub trust_tier: Option<String>,
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
    pub provenance_summary: Option<String>,
    pub trust_tier: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolSearchTrustFilterSummary {
    pub applied: bool,
    pub query_requested_tiers: Vec<String>,
    pub structured_requested_tiers: Vec<String>,
    pub effective_tiers: Vec<String>,
    pub conflicting_requested_tiers: bool,
    pub candidates_before_trust_filter: usize,
    pub candidates_after_trust_filter: usize,
    pub filtered_out_candidates: usize,
    pub filtered_out_tier_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolSearchOperationSummary {
    pub headline: String,
    pub query: String,
    pub returned: usize,
    pub trust_tiers: Vec<String>,
    pub trust_filter_summary: ToolSearchTrustFilterSummary,
    pub top_results: Vec<ToolSearchOperationSummaryEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolSearchOperationSummaryEntry {
    pub tool_id: String,
    pub provider_id: String,
    pub connector_name: String,
    pub trust_tier: Option<String>,
    pub bridge_kind: String,
    pub score: u32,
    pub setup_ready: bool,
    pub deferred: bool,
    pub loaded: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[serde(deny_unknown_fields)]
pub struct PluginPreflightBridgeProfileDelta {
    pub supported_bridges: Vec<String>,
    pub supported_adapter_families: Vec<String>,
    pub supported_compatibility_modes: Vec<String>,
    pub supported_compatibility_shims: Vec<String>,
    pub shim_profile_additions: Vec<PluginPreflightBridgeShimProfileDelta>,
    pub unresolved_blocking_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct PluginPreflightBridgeShimProfileDelta {
    pub shim_id: String,
    pub shim_family: String,
    pub supported_dialects: Vec<String>,
    pub supported_bridges: Vec<String>,
    pub supported_adapter_families: Vec<String>,
    pub supported_source_languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
