use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    contracts::Capability,
    integration::IntegrationCatalog,
    plugin::{
        PLUGIN_SLOT_CLAIMS_METADATA_KEY, PluginCompatibility, PluginCompatibilityMode,
        PluginCompatibilityShim, PluginContractDialect, PluginDescriptor, PluginDiagnosticCode,
        PluginDiagnosticFinding, PluginDiagnosticPhase, PluginDiagnosticSeverity, PluginManifest,
        PluginScanReport, PluginSetup, PluginSlotClaim, PluginSlotMode, PluginSourceKind,
        PluginTrustTier, plugin_host_compatibility_issue, slot_modes_conflict,
    },
};

pub const GOVERNED_NATIVE_RUNTIME_EXTENSION_FAMILY: &str = "governed_native_runtime_extension";
pub const GOVERNED_SIDECAR_EXTENSION_TRUST_LANE: &str = "governed_sidecar";
pub const TRUSTED_HOST_EXTENSION_FAMILY: &str = "trusted_host_extension";
pub const TRUSTED_HOST_EXTENSION_TRUST_LANE: &str = "trusted_host";
pub const TRUSTED_HOST_READ_ONLY_EXTENSION_HOOKS: &[&str] = &[
    "session_start",
    "session_shutdown",
    "turn_start",
    "turn_end",
    "message_start",
    "message_end",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PluginBridgeKind {
    HttpJson,
    ProcessStdio,
    NativeFfi,
    WasmComponent,
    McpServer,
    AcpBridge,
    AcpRuntime,
    #[default]
    Unknown,
}

impl PluginBridgeKind {
    #[must_use]
    pub fn parse_label(raw: &str) -> Option<Self> {
        parse_bridge_kind(raw)
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HttpJson => "http_json",
            Self::ProcessStdio => "process_stdio",
            Self::NativeFfi => "native_ffi",
            Self::WasmComponent => "wasm_component",
            Self::McpServer => "mcp_server",
            Self::AcpBridge => "acp_bridge",
            Self::AcpRuntime => "acp_runtime",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PluginRuntimeProfile {
    pub source_language: String,
    pub bridge_kind: PluginBridgeKind,
    pub adapter_family: String,
    pub entrypoint_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRuntimeScaffoldDefaults {
    pub source_language: Option<String>,
    pub bridge_kind: PluginBridgeKind,
    pub adapter_family: String,
    pub entrypoint_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PluginNativeExtensionDeclarations {
    pub contract: Option<String>,
    pub family: Option<String>,
    pub trust_lane: Option<String>,
    pub facets: Vec<String>,
    pub methods: Vec<String>,
    pub events: Vec<String>,
    pub host_hooks: Vec<String>,
    pub host_actions: Vec<String>,
    pub metadata_issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginChannelBridgeReadiness {
    pub ready: bool,
    #[serde(default)]
    pub missing_fields: Vec<String>,
}

impl Default for PluginChannelBridgeReadiness {
    fn default() -> Self {
        Self {
            ready: true,
            missing_fields: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginChannelBridgeContract {
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub setup_surface: Option<String>,
    #[serde(default)]
    pub transport_family: Option<String>,
    #[serde(default)]
    pub target_contract: Option<String>,
    #[serde(default)]
    pub account_scope: Option<String>,
    #[serde(default)]
    pub runtime_contract: Option<String>,
    #[serde(default)]
    pub runtime_operations: Vec<String>,
    #[serde(default)]
    pub runtime_metadata_issues: Vec<String>,
    #[serde(default)]
    pub readiness: PluginChannelBridgeReadiness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginIR {
    pub manifest_api_version: Option<String>,
    pub plugin_version: Option<String>,
    #[serde(default)]
    pub dialect: PluginContractDialect,
    pub dialect_version: Option<String>,
    #[serde(default)]
    pub compatibility_mode: PluginCompatibilityMode,
    pub plugin_id: String,
    pub provider_id: String,
    pub connector_name: String,
    pub channel_id: Option<String>,
    pub endpoint: Option<String>,
    pub capabilities: BTreeSet<Capability>,
    #[serde(default)]
    pub trust_tier: PluginTrustTier,
    pub metadata: BTreeMap<String, String>,
    pub source_path: String,
    pub source_kind: PluginSourceKind,
    pub package_root: String,
    pub package_manifest_path: Option<String>,
    #[serde(default)]
    pub diagnostic_findings: Vec<PluginDiagnosticFinding>,
    pub setup: Option<PluginSetup>,
    #[serde(default)]
    pub channel_bridge: Option<PluginChannelBridgeContract>,
    #[serde(default)]
    pub slot_claims: Vec<PluginSlotClaim>,
    pub compatibility: Option<PluginCompatibility>,
    #[serde(default)]
    pub runtime: PluginRuntimeProfile,
}

#[derive(Debug, Deserialize)]
struct PluginIRSerde {
    manifest_api_version: Option<String>,
    plugin_version: Option<String>,
    dialect: Option<PluginContractDialect>,
    dialect_version: Option<String>,
    compatibility_mode: Option<PluginCompatibilityMode>,
    plugin_id: String,
    provider_id: String,
    connector_name: String,
    channel_id: Option<String>,
    endpoint: Option<String>,
    capabilities: BTreeSet<Capability>,
    #[serde(default)]
    trust_tier: PluginTrustTier,
    metadata: BTreeMap<String, String>,
    source_path: String,
    source_kind: PluginSourceKind,
    package_root: String,
    package_manifest_path: Option<String>,
    #[serde(default)]
    diagnostic_findings: Vec<PluginDiagnosticFinding>,
    setup: Option<PluginSetup>,
    #[serde(default)]
    channel_bridge: Option<PluginChannelBridgeContract>,
    #[serde(default)]
    slot_claims: Vec<PluginSlotClaim>,
    compatibility: Option<PluginCompatibility>,
    runtime: Option<PluginRuntimeProfile>,
}

impl<'de> Deserialize<'de> for PluginIR {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = PluginIRSerde::deserialize(deserializer)?;
        let dialect = raw
            .dialect
            .unwrap_or_else(|| legacy_plugin_ir_dialect(raw.source_kind));
        let compatibility_mode = raw.compatibility_mode.unwrap_or_default();
        let runtime = raw.runtime.unwrap_or_else(|| {
            legacy_plugin_ir_runtime_profile(
                &raw.source_path,
                raw.source_kind,
                &raw.metadata,
                raw.endpoint.as_deref(),
            )
        });

        Ok(Self {
            manifest_api_version: raw.manifest_api_version,
            plugin_version: raw.plugin_version,
            dialect,
            dialect_version: raw.dialect_version,
            compatibility_mode,
            plugin_id: raw.plugin_id,
            provider_id: raw.provider_id,
            connector_name: raw.connector_name,
            channel_id: raw.channel_id,
            endpoint: raw.endpoint,
            capabilities: raw.capabilities,
            trust_tier: raw.trust_tier,
            metadata: raw.metadata,
            source_path: raw.source_path,
            source_kind: raw.source_kind,
            package_root: raw.package_root,
            package_manifest_path: raw.package_manifest_path,
            diagnostic_findings: raw.diagnostic_findings,
            setup: raw.setup,
            channel_bridge: raw.channel_bridge,
            slot_claims: raw.slot_claims,
            compatibility: raw.compatibility,
            runtime,
        })
    }
}

/// Declares which setup requirements are already verified for a plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginSetupReadinessContext {
    pub verified_env_vars: BTreeSet<String>,
    pub verified_config_keys: BTreeSet<String>,
}

/// Summarizes whether manifest-declared setup requirements are satisfied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSetupReadiness {
    pub ready: bool,
    pub missing_required_env_vars: Vec<String>,
    pub missing_required_config_keys: Vec<String>,
}

impl Default for PluginSetupReadiness {
    fn default() -> Self {
        Self {
            ready: true,
            missing_required_env_vars: Vec::new(),
            missing_required_config_keys: Vec::new(),
        }
    }
}

/// Evaluates manifest-declared setup requirements against verified runtime context.
pub fn evaluate_plugin_setup_requirements(
    required_env_vars: &[String],
    required_config_keys: &[String],
    context: &PluginSetupReadinessContext,
) -> PluginSetupReadiness {
    let mut missing_required_env_vars = Vec::new();
    for required_env_var in required_env_vars {
        let env_var_is_verified =
            verified_env_var_names_contain(&context.verified_env_vars, required_env_var);
        if !env_var_is_verified {
            missing_required_env_vars.push(required_env_var.clone());
        }
    }

    let mut missing_required_config_keys = Vec::new();
    for required_config_key in required_config_keys {
        let config_key_is_verified = context.verified_config_keys.contains(required_config_key);
        if !config_key_is_verified {
            missing_required_config_keys.push(required_config_key.clone());
        }
    }

    let env_ready = missing_required_env_vars.is_empty();
    let config_ready = missing_required_config_keys.is_empty();
    let ready = env_ready && config_ready;

    PluginSetupReadiness {
        ready,
        missing_required_env_vars,
        missing_required_config_keys,
    }
}

fn verified_env_var_names_contain(
    verified_env_vars: &BTreeSet<String>,
    required_env_var: &str,
) -> bool {
    #[cfg(windows)]
    {
        verified_env_vars
            .iter()
            .any(|verified_env_var| verified_env_var.eq_ignore_ascii_case(required_env_var))
    }

    #[cfg(not(windows))]
    {
        verified_env_vars.contains(required_env_var)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginTranslationReport {
    pub translated_plugins: usize,
    pub bridge_distribution: BTreeMap<String, usize>,
    pub entries: Vec<PluginIR>,
}

/// Serialized activation outcomes are additive.
///
/// Consumers that deserialize persisted or remote payloads should tolerate newer
/// snake_case variants and treat unknown values as forward-compatible contract
/// growth rather than a malformed payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginActivationStatus {
    Ready,
    /// The runtime surface is supported, but declared setup requirements are not.
    SetupIncomplete,
    BlockedInvalidManifestContract,
    BlockedCompatibilityMode,
    BlockedIncompatibleHost,
    BlockedUnsupportedBridge,
    BlockedUnsupportedAdapterFamily,
    BlockedSlotClaimConflict,
    #[serde(other)]
    Unknown,
}

impl PluginActivationStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::SetupIncomplete => "setup_incomplete",
            Self::BlockedInvalidManifestContract => "blocked_invalid_manifest_contract",
            Self::BlockedCompatibilityMode => "blocked_compatibility_mode",
            Self::BlockedIncompatibleHost => "blocked_incompatible_host",
            Self::BlockedUnsupportedBridge => "blocked_unsupported_bridge",
            Self::BlockedUnsupportedAdapterFamily => "blocked_unsupported_adapter_family",
            Self::BlockedSlotClaimConflict => "blocked_slot_claim_conflict",
            Self::Unknown => "unknown",
        }
    }
}

/// Captures activation planning details for a single plugin candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginActivationCandidate {
    pub plugin_id: String,
    pub source_path: String,
    pub source_kind: PluginSourceKind,
    pub package_root: String,
    pub package_manifest_path: Option<String>,
    #[serde(default)]
    pub trust_tier: PluginTrustTier,
    #[serde(default)]
    pub compatibility_mode: PluginCompatibilityMode,
    #[serde(default)]
    pub compatibility_shim: Option<PluginCompatibilityShim>,
    #[serde(default)]
    pub compatibility_shim_support: Option<PluginCompatibilityShimSupport>,
    #[serde(default)]
    pub compatibility_shim_support_mismatch_reasons: Vec<String>,
    pub bridge_kind: PluginBridgeKind,
    pub adapter_family: String,
    #[serde(default)]
    pub slot_claims: Vec<PluginSlotClaim>,
    #[serde(default)]
    pub diagnostic_findings: Vec<PluginDiagnosticFinding>,
    pub status: PluginActivationStatus,
    pub reason: String,
    #[serde(default)]
    pub missing_required_env_vars: Vec<String>,
    #[serde(default)]
    pub missing_required_config_keys: Vec<String>,
    pub bootstrap_hint: String,
}

/// Summarizes activation readiness across all translated plugin candidates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginActivationPlan {
    pub total_plugins: usize,
    pub ready_plugins: usize,
    #[serde(default)]
    pub setup_incomplete_plugins: usize,
    pub blocked_plugins: usize,
    pub candidates: Vec<PluginActivationCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginActivationInventoryEntry {
    pub manifest_api_version: Option<String>,
    pub plugin_version: Option<String>,
    pub dialect: PluginContractDialect,
    pub dialect_version: Option<String>,
    pub compatibility_mode: PluginCompatibilityMode,
    pub compatibility_shim: Option<PluginCompatibilityShim>,
    pub compatibility_shim_support: Option<PluginCompatibilityShimSupport>,
    pub compatibility_shim_support_mismatch_reasons: Vec<String>,
    pub plugin_id: String,
    pub provider_id: String,
    pub connector_name: String,
    pub source_path: String,
    pub source_kind: PluginSourceKind,
    pub package_root: String,
    pub package_manifest_path: Option<String>,
    pub bridge_kind: PluginBridgeKind,
    pub adapter_family: String,
    pub entrypoint_hint: String,
    pub source_language: String,
    pub slot_claims: Vec<PluginSlotClaim>,
    pub diagnostic_findings: Vec<PluginDiagnosticFinding>,
    pub compatibility: Option<PluginCompatibility>,
    pub activation_status: Option<PluginActivationStatus>,
    pub activation_reason: Option<String>,
    pub bootstrap_hint: Option<String>,
}

impl PluginActivationPlan {
    #[must_use]
    pub fn has_blockers(&self) -> bool {
        self.blocked_plugins > 0
    }

    #[must_use]
    pub fn candidate_for(
        &self,
        source_path: &str,
        plugin_id: &str,
    ) -> Option<&PluginActivationCandidate> {
        self.candidates.iter().find(|candidate| {
            candidate.source_path == source_path && candidate.plugin_id == plugin_id
        })
    }

    #[must_use]
    pub fn inventory_entries(
        &self,
        translation: &PluginTranslationReport,
    ) -> Vec<PluginActivationInventoryEntry> {
        translation
            .entries
            .iter()
            .map(|entry| {
                let candidate = self.candidate_for(&entry.source_path, &entry.plugin_id);
                PluginActivationInventoryEntry {
                    manifest_api_version: entry.manifest_api_version.clone(),
                    plugin_version: entry.plugin_version.clone(),
                    dialect: entry.dialect,
                    dialect_version: entry.dialect_version.clone(),
                    compatibility_mode: entry.compatibility_mode,
                    compatibility_shim: candidate
                        .and_then(|candidate| candidate.compatibility_shim.clone())
                        .or_else(|| PluginCompatibilityShim::for_mode(entry.compatibility_mode)),
                    compatibility_shim_support: candidate
                        .and_then(|candidate| candidate.compatibility_shim_support.clone()),
                    compatibility_shim_support_mismatch_reasons: candidate
                        .map(|candidate| {
                            candidate
                                .compatibility_shim_support_mismatch_reasons
                                .clone()
                        })
                        .unwrap_or_default(),
                    plugin_id: entry.plugin_id.clone(),
                    provider_id: entry.provider_id.clone(),
                    connector_name: entry.connector_name.clone(),
                    source_path: entry.source_path.clone(),
                    source_kind: entry.source_kind,
                    package_root: entry.package_root.clone(),
                    package_manifest_path: entry.package_manifest_path.clone(),
                    bridge_kind: entry.runtime.bridge_kind,
                    adapter_family: entry.runtime.adapter_family.clone(),
                    entrypoint_hint: entry.runtime.entrypoint_hint.clone(),
                    source_language: entry.runtime.source_language.clone(),
                    slot_claims: entry.slot_claims.clone(),
                    diagnostic_findings: candidate
                        .map(|candidate| candidate.diagnostic_findings.clone())
                        .unwrap_or_else(|| entry.diagnostic_findings.clone()),
                    compatibility: entry.compatibility.clone(),
                    activation_status: candidate.map(|candidate| candidate.status),
                    activation_reason: candidate.map(|candidate| candidate.reason.clone()),
                    bootstrap_hint: candidate.map(|candidate| candidate.bootstrap_hint.clone()),
                }
            })
            .collect()
    }

    #[must_use]
    pub fn blocker_summary(&self, limit: usize) -> String {
        if self.blocked_plugins == 0 {
            return "no blocked plugins".to_owned();
        }

        let capped_limit = limit.clamp(1, 16);
        let mut details = self
            .candidates
            .iter()
            .filter(|candidate| {
                matches!(
                    candidate.status,
                    PluginActivationStatus::BlockedInvalidManifestContract
                        | PluginActivationStatus::BlockedCompatibilityMode
                        | PluginActivationStatus::BlockedUnsupportedBridge
                        | PluginActivationStatus::BlockedIncompatibleHost
                        | PluginActivationStatus::BlockedUnsupportedAdapterFamily
                        | PluginActivationStatus::BlockedSlotClaimConflict
                        | PluginActivationStatus::Unknown
                )
            })
            .take(capped_limit)
            .map(|candidate| {
                format!(
                    "{} [{}]: {}",
                    candidate.plugin_id,
                    candidate.status.as_str(),
                    candidate.reason
                )
            })
            .collect::<Vec<_>>();

        if self.blocked_plugins > capped_limit {
            details.push(format!(
                "+{} more blocked plugin(s)",
                self.blocked_plugins - capped_limit
            ));
        }

        details.join("; ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeSupportMatrix {
    pub supported_bridges: BTreeSet<PluginBridgeKind>,
    pub supported_adapter_families: BTreeSet<String>,
    pub supported_compatibility_modes: BTreeSet<PluginCompatibilityMode>,
    pub supported_compatibility_shims: BTreeSet<PluginCompatibilityShim>,
    pub supported_compatibility_shim_profiles:
        BTreeMap<PluginCompatibilityShim, PluginCompatibilityShimSupport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCompatibilityShimSupport {
    pub shim: PluginCompatibilityShim,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub supported_dialects: BTreeSet<PluginContractDialect>,
    #[serde(default)]
    pub supported_bridges: BTreeSet<PluginBridgeKind>,
    #[serde(default)]
    pub supported_adapter_families: BTreeSet<String>,
    #[serde(default)]
    pub supported_source_languages: BTreeSet<String>,
}

impl PluginCompatibilityShimSupport {
    #[must_use]
    pub fn normalized(self) -> Self {
        Self {
            shim: PluginCompatibilityShim {
                shim_id: self.shim.shim_id.trim().to_owned(),
                family: self.shim.family.trim().to_owned(),
            },
            version: self
                .version
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            supported_dialects: self.supported_dialects,
            supported_bridges: self.supported_bridges,
            supported_adapter_families: self
                .supported_adapter_families
                .into_iter()
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| !value.is_empty())
                .collect(),
            supported_source_languages: self
                .supported_source_languages
                .into_iter()
                .map(|value| normalize_language(&value))
                .filter(|value| value != "unknown")
                .collect(),
        }
    }

    fn mismatch_reasons(&self, ir: &PluginIR) -> Vec<String> {
        let mut reasons = Vec::new();

        if !self.supported_dialects.is_empty() && !self.supported_dialects.contains(&ir.dialect) {
            reasons.push(format!("dialect `{}`", ir.dialect.as_str()));
        }

        if !self.supported_bridges.is_empty()
            && !self.supported_bridges.contains(&ir.runtime.bridge_kind)
        {
            reasons.push(format!("bridge kind `{}`", ir.runtime.bridge_kind.as_str()));
        }

        if !self.supported_adapter_families.is_empty()
            && !self
                .supported_adapter_families
                .contains(&ir.runtime.adapter_family.trim().to_ascii_lowercase())
        {
            reasons.push(format!("adapter family `{}`", ir.runtime.adapter_family));
        }

        let normalized_source_language = normalize_language(&ir.runtime.source_language);
        if !self.supported_source_languages.is_empty()
            && !self
                .supported_source_languages
                .contains(&normalized_source_language)
        {
            reasons.push(format!("source language `{}`", ir.runtime.source_language));
        }

        reasons
    }
}

impl Default for BridgeSupportMatrix {
    fn default() -> Self {
        Self {
            supported_bridges: BTreeSet::from([
                PluginBridgeKind::HttpJson,
                PluginBridgeKind::ProcessStdio,
                PluginBridgeKind::NativeFfi,
                PluginBridgeKind::WasmComponent,
                PluginBridgeKind::McpServer,
                PluginBridgeKind::AcpBridge,
                PluginBridgeKind::AcpRuntime,
            ]),
            supported_adapter_families: BTreeSet::new(),
            supported_compatibility_modes: BTreeSet::from([PluginCompatibilityMode::Native]),
            supported_compatibility_shims: BTreeSet::new(),
            supported_compatibility_shim_profiles: BTreeMap::new(),
        }
    }
}

impl BridgeSupportMatrix {
    #[must_use]
    pub fn is_bridge_supported(&self, bridge_kind: PluginBridgeKind) -> bool {
        self.supported_bridges.contains(&bridge_kind)
    }

    #[must_use]
    pub fn is_adapter_family_supported(&self, adapter_family: &str) -> bool {
        self.supported_adapter_families.is_empty()
            || self.supported_adapter_families.contains(adapter_family)
    }

    #[must_use]
    pub fn is_compatibility_mode_supported(
        &self,
        compatibility_mode: PluginCompatibilityMode,
    ) -> bool {
        self.supported_compatibility_modes
            .contains(&compatibility_mode)
    }

    #[must_use]
    pub fn is_compatibility_shim_supported(
        &self,
        compatibility_shim: Option<&PluginCompatibilityShim>,
    ) -> bool {
        compatibility_shim.is_none_or(|shim| {
            self.supported_compatibility_shims.contains(shim)
                || self
                    .supported_compatibility_shim_profiles
                    .contains_key(shim)
        })
    }

    #[must_use]
    pub fn compatibility_shim_support_issue(
        &self,
        ir: &PluginIR,
        compatibility_shim: Option<&PluginCompatibilityShim>,
    ) -> Option<String> {
        let shim = compatibility_shim?;
        let profile = self.supported_compatibility_shim_profiles.get(shim)?;
        let mismatches = profile.mismatch_reasons(ir);
        compatibility_shim_support_issue(shim, profile, &mismatches)
    }

    #[must_use]
    pub fn compatibility_shim_support_profile(
        &self,
        compatibility_shim: Option<&PluginCompatibilityShim>,
    ) -> Option<&PluginCompatibilityShimSupport> {
        compatibility_shim.and_then(|shim| self.supported_compatibility_shim_profiles.get(shim))
    }
}

fn compatibility_shim_support_issue(
    shim: &PluginCompatibilityShim,
    profile: &PluginCompatibilityShimSupport,
    mismatches: &[String],
) -> Option<String> {
    if mismatches.is_empty() {
        return None;
    }

    let version_clause = profile
        .version
        .as_deref()
        .map(|version| format!(" version `{version}`"))
        .unwrap_or_default();

    Some(format!(
        "compatibility shim `{}` ({}) is enabled but its support profile{} does not support {}",
        shim.shim_id,
        shim.family,
        version_clause,
        mismatches.join(", ")
    ))
}

#[derive(Debug, Default)]
pub struct PluginTranslator;

impl PluginTranslator {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    #[must_use]
    pub fn translate_scan_report(&self, report: &PluginScanReport) -> PluginTranslationReport {
        let mut translated = PluginTranslationReport::default();
        let mut diagnostics_by_key: BTreeMap<(String, String), Vec<PluginDiagnosticFinding>> =
            BTreeMap::new();

        for finding in &report.diagnostic_findings {
            let (Some(source_path), Some(plugin_id)) =
                (finding.source_path.clone(), finding.plugin_id.clone())
            else {
                continue;
            };

            diagnostics_by_key
                .entry((source_path, plugin_id))
                .or_default()
                .push(finding.clone());
        }

        for descriptor in &report.descriptors {
            let mut ir = self.translate_descriptor(descriptor);
            ir.diagnostic_findings = diagnostics_by_key
                .remove(&(
                    descriptor.path.clone(),
                    descriptor.manifest.plugin_id.clone(),
                ))
                .unwrap_or_default();
            let bridge = ir.runtime.bridge_kind.as_str().to_owned();
            *translated.bridge_distribution.entry(bridge).or_insert(0) += 1;
            translated.translated_plugins = translated.translated_plugins.saturating_add(1);
            translated.entries.push(ir);
        }

        translated
    }

    #[must_use]
    pub fn translate_descriptor(&self, descriptor: &PluginDescriptor) -> PluginIR {
        let runtime = infer_runtime_profile(&descriptor.language, &descriptor.manifest);
        let channel_bridge = derive_channel_bridge_contract(&descriptor.manifest);

        PluginIR {
            manifest_api_version: descriptor.manifest.api_version.clone(),
            plugin_version: descriptor.manifest.version.clone(),
            dialect: descriptor.dialect,
            dialect_version: descriptor.dialect_version.clone(),
            compatibility_mode: descriptor.compatibility_mode,
            plugin_id: descriptor.manifest.plugin_id.clone(),
            provider_id: descriptor.manifest.provider_id.clone(),
            connector_name: descriptor.manifest.connector_name.clone(),
            channel_id: descriptor.manifest.channel_id.clone(),
            endpoint: descriptor.manifest.endpoint.clone(),
            capabilities: descriptor.manifest.capabilities.clone(),
            trust_tier: descriptor.manifest.trust_tier,
            metadata: descriptor.manifest.metadata.clone(),
            source_path: descriptor.path.clone(),
            source_kind: descriptor.source_kind,
            package_root: descriptor.package_root.clone(),
            package_manifest_path: descriptor.package_manifest_path.clone(),
            diagnostic_findings: Vec::new(),
            setup: descriptor.manifest.setup.clone(),
            channel_bridge,
            slot_claims: descriptor.manifest.slot_claims.clone(),
            compatibility: descriptor.manifest.compatibility.clone(),
            runtime,
        }
    }

    #[must_use]
    pub fn plan_activation(
        &self,
        translation: &PluginTranslationReport,
        matrix: &BridgeSupportMatrix,
        setup_readiness_context: &PluginSetupReadinessContext,
    ) -> PluginActivationPlan {
        self.plan_activation_with_catalog(translation, matrix, setup_readiness_context, None)
    }

    #[must_use]
    pub fn plan_activation_with_catalog(
        &self,
        translation: &PluginTranslationReport,
        matrix: &BridgeSupportMatrix,
        setup_readiness_context: &PluginSetupReadinessContext,
        catalog: Option<&IntegrationCatalog>,
    ) -> PluginActivationPlan {
        let mut plan = PluginActivationPlan::default();
        let slot_conflicts = collect_slot_claim_conflicts(&translation.entries, catalog);

        for ir in &translation.entries {
            plan.total_plugins = plan.total_plugins.saturating_add(1);
            let compatibility_shim = PluginCompatibilityShim::for_mode(ir.compatibility_mode);
            let compatibility_shim_support = matrix
                .compatibility_shim_support_profile(compatibility_shim.as_ref())
                .cloned();
            let compatibility_shim_support_mismatch_reasons = compatibility_shim_support
                .as_ref()
                .map(|profile| profile.mismatch_reasons(ir))
                .unwrap_or_default();

            let setup_readiness =
                evaluate_plugin_setup_readiness(ir.setup.as_ref(), setup_readiness_context);
            let setup_is_incomplete = !setup_readiness.ready;
            let slot_conflict_key = (ir.source_path.clone(), ir.plugin_id.clone());
            let invalid_manifest_contract = plugin_manifest_contract_is_invalid(ir);
            let (status, reason) = if !matrix.is_compatibility_mode_supported(ir.compatibility_mode)
            {
                let shim_clause = compatibility_shim
                    .as_ref()
                    .map(|shim| format!(" via shim `{}` ({})", shim.shim_id, shim.family))
                    .unwrap_or_default();
                (
                    PluginActivationStatus::BlockedCompatibilityMode,
                    format!(
                        "compatibility mode {} requires a host shim that is not enabled in the current runtime matrix{}",
                        ir.compatibility_mode.as_str(),
                        shim_clause
                    ),
                )
            } else if !matrix.is_compatibility_shim_supported(compatibility_shim.as_ref()) {
                let maybe_shim = compatibility_shim.as_ref();
                let missing_shim_reason = format!(
                    "compatibility mode {} did not resolve a canonical shim before runtime-matrix evaluation",
                    ir.compatibility_mode.as_str()
                );
                let reason = match maybe_shim {
                    Some(shim) => {
                        let shim_id = shim.shim_id.as_str();
                        let shim_family = shim.family.as_str();

                        format!(
                            "compatibility mode {} requires compatibility shim `{}` ({}) that is not enabled in the current runtime matrix",
                            ir.compatibility_mode.as_str(),
                            shim_id,
                            shim_family
                        )
                    }
                    None => missing_shim_reason,
                };

                (PluginActivationStatus::BlockedCompatibilityMode, reason)
            } else if let Some(reason) = plugin_host_compatibility_issue(ir.compatibility.as_ref())
            {
                (PluginActivationStatus::BlockedIncompatibleHost, reason)
            } else if let Some(reason) = compatibility_shim
                .as_ref()
                .zip(compatibility_shim_support.as_ref())
                .and_then(|(shim, profile)| {
                    compatibility_shim_support_issue(
                        shim,
                        profile,
                        &compatibility_shim_support_mismatch_reasons,
                    )
                })
            {
                (PluginActivationStatus::BlockedCompatibilityMode, reason)
            } else if let Some(reason) = slot_conflicts.get(&slot_conflict_key) {
                (
                    PluginActivationStatus::BlockedSlotClaimConflict,
                    reason.clone(),
                )
            } else if invalid_manifest_contract {
                (
                    PluginActivationStatus::BlockedInvalidManifestContract,
                    format_invalid_manifest_contract_reason(ir),
                )
            } else if !matrix.is_bridge_supported(ir.runtime.bridge_kind) {
                (
                    PluginActivationStatus::BlockedUnsupportedBridge,
                    format!(
                        "bridge kind {} is not supported by current runtime matrix",
                        ir.runtime.bridge_kind.as_str()
                    ),
                )
            } else if !matrix.is_adapter_family_supported(&ir.runtime.adapter_family) {
                (
                    PluginActivationStatus::BlockedUnsupportedAdapterFamily,
                    format!(
                        "adapter family {} is not supported by current runtime matrix",
                        ir.runtime.adapter_family
                    ),
                )
            } else if setup_is_incomplete {
                (
                    PluginActivationStatus::SetupIncomplete,
                    format_plugin_setup_incomplete_reason(&setup_readiness),
                )
            } else {
                (
                    PluginActivationStatus::Ready,
                    "plugin runtime profile is supported by current runtime matrix".to_owned(),
                )
            };

            let mut diagnostic_findings = ir.diagnostic_findings.clone();
            if let Some(finding) = activation_diagnostic_finding(ir, status, &reason) {
                diagnostic_findings.push(finding);
            }

            match status {
                PluginActivationStatus::Ready => {
                    plan.ready_plugins = plan.ready_plugins.saturating_add(1)
                }
                PluginActivationStatus::SetupIncomplete => {
                    plan.setup_incomplete_plugins = plan.setup_incomplete_plugins.saturating_add(1)
                }
                PluginActivationStatus::BlockedInvalidManifestContract
                | PluginActivationStatus::BlockedCompatibilityMode
                | PluginActivationStatus::BlockedUnsupportedBridge
                | PluginActivationStatus::BlockedIncompatibleHost
                | PluginActivationStatus::BlockedUnsupportedAdapterFamily
                | PluginActivationStatus::BlockedSlotClaimConflict
                | PluginActivationStatus::Unknown => {
                    plan.blocked_plugins = plan.blocked_plugins.saturating_add(1)
                }
            }

            plan.candidates.push(PluginActivationCandidate {
                plugin_id: ir.plugin_id.clone(),
                source_path: ir.source_path.clone(),
                source_kind: ir.source_kind,
                package_root: ir.package_root.clone(),
                package_manifest_path: ir.package_manifest_path.clone(),
                trust_tier: ir.trust_tier,
                compatibility_mode: ir.compatibility_mode,
                compatibility_shim,
                compatibility_shim_support,
                compatibility_shim_support_mismatch_reasons,
                bridge_kind: ir.runtime.bridge_kind,
                adapter_family: ir.runtime.adapter_family.clone(),
                slot_claims: ir.slot_claims.clone(),
                diagnostic_findings,
                status,
                reason,
                missing_required_env_vars: setup_readiness.missing_required_env_vars,
                missing_required_config_keys: setup_readiness.missing_required_config_keys,
                bootstrap_hint: bootstrap_hint(ir),
            });
        }

        plan
    }
}

fn evaluate_plugin_setup_readiness(
    setup: Option<&PluginSetup>,
    context: &PluginSetupReadinessContext,
) -> PluginSetupReadiness {
    let Some(setup) = setup else {
        return PluginSetupReadiness::default();
    };

    evaluate_plugin_setup_requirements(
        &setup.required_env_vars,
        &setup.required_config_keys,
        context,
    )
}

fn format_plugin_setup_incomplete_reason(readiness: &PluginSetupReadiness) -> String {
    let mut reasons = Vec::new();

    if !readiness.missing_required_env_vars.is_empty() {
        let missing_env_vars = readiness.missing_required_env_vars.join(", ");
        let env_reason = format!("missing required env vars: {missing_env_vars}");
        reasons.push(env_reason);
    }

    if !readiness.missing_required_config_keys.is_empty() {
        let missing_config_keys = readiness.missing_required_config_keys.join(", ");
        let config_reason = format!("missing required config keys: {missing_config_keys}");
        reasons.push(config_reason);
    }

    let combined_reasons = reasons.join("; ");
    format!("plugin setup is incomplete: {combined_reasons}")
}

fn plugin_manifest_contract_is_invalid(ir: &PluginIR) -> bool {
    let Some(channel_bridge) = ir.channel_bridge.as_ref() else {
        return false;
    };

    !channel_bridge.readiness.ready
}

fn format_invalid_manifest_contract_reason(ir: &PluginIR) -> String {
    let Some(channel_bridge) = ir.channel_bridge.as_ref() else {
        return "plugin manifest contract is invalid".to_owned();
    };

    let missing_fields = channel_bridge.readiness.missing_fields.join(", ");
    format!("plugin channel bridge contract is incomplete: {missing_fields}")
}

fn activation_diagnostic_finding(
    ir: &PluginIR,
    status: PluginActivationStatus,
    reason: &str,
) -> Option<PluginDiagnosticFinding> {
    let (code, field_path, remediation) = match status {
        PluginActivationStatus::Ready => return None,
        PluginActivationStatus::SetupIncomplete => return None,
        PluginActivationStatus::BlockedInvalidManifestContract => (
            PluginDiagnosticCode::InvalidManifestContract,
            Some("channel_bridge".to_owned()),
            Some(
                "declare the required channel bridge contract fields in the manifest before activation"
                    .to_owned(),
            ),
        ),
        PluginActivationStatus::BlockedCompatibilityMode => (
            PluginDiagnosticCode::CompatibilityShimRequired,
            Some("compatibility_mode".to_owned()),
            Some(
                "enable or widen the required compatibility shim support policy in the runtime bridge matrix, or migrate the plugin to the native Loong contract before activation"
                    .to_owned(),
            ),
        ),
        PluginActivationStatus::BlockedIncompatibleHost => (
            PluginDiagnosticCode::IncompatibleHost,
            Some("compatibility".to_owned()),
            Some(
                "align `compatibility.host_api` / `compatibility.host_version_req` with the current host, or upgrade Loong before activation"
                    .to_owned(),
            ),
        ),
        PluginActivationStatus::BlockedUnsupportedBridge => (
            PluginDiagnosticCode::UnsupportedBridge,
            Some("metadata.bridge_kind".to_owned()),
            Some(
                "switch the plugin to a supported bridge kind or widen the runtime bridge support policy before activation"
                    .to_owned(),
            ),
        ),
        PluginActivationStatus::BlockedUnsupportedAdapterFamily => (
            PluginDiagnosticCode::UnsupportedAdapterFamily,
            Some("metadata.adapter_family".to_owned()),
            Some(
                "switch the plugin adapter family to one supported by the current runtime matrix"
                    .to_owned(),
            ),
        ),
        PluginActivationStatus::BlockedSlotClaimConflict => (
            PluginDiagnosticCode::SlotClaimConflict,
            Some("slot_claims".to_owned()),
            Some(
                "choose a different slot/key pair or relax ownership to shared/advisory only when the surface is intentionally multi-owner"
                    .to_owned(),
            ),
        ),
        PluginActivationStatus::Unknown => return None,
    };

    Some(PluginDiagnosticFinding {
        code,
        severity: PluginDiagnosticSeverity::Error,
        phase: PluginDiagnosticPhase::Activation,
        blocking: true,
        plugin_id: Some(ir.plugin_id.clone()),
        source_path: Some(ir.source_path.clone()),
        source_kind: Some(ir.source_kind),
        field_path,
        message: reason.to_owned(),
        remediation,
    })
}

#[derive(Debug, Clone)]
struct SlotClaimOwner {
    plugin_id: String,
    provider_id: String,
    mode: PluginSlotMode,
    source_path: Option<String>,
}

fn collect_slot_claim_conflicts(
    entries: &[PluginIR],
    catalog: Option<&IntegrationCatalog>,
) -> BTreeMap<(String, String), String> {
    let mut conflicts: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();

    if let Some(catalog) = catalog {
        let existing_claims = existing_slot_claims_from_catalog(catalog);

        for entry in entries {
            for claim in &entry.slot_claims {
                let key = (claim.slot.clone(), claim.key.clone());
                let Some(existing_owners) = existing_claims.get(&key) else {
                    continue;
                };

                for owner in existing_owners {
                    if owner.plugin_id == entry.plugin_id
                        || !slot_modes_conflict(owner.mode, claim.mode)
                    {
                        continue;
                    }

                    conflicts
                        .entry((entry.source_path.clone(), entry.plugin_id.clone()))
                        .or_default()
                        .insert(format!(
                            "slot claim `{}`:`{}` ({}) conflicts with existing plugin `{}` (provider `{}`{})",
                            claim.slot,
                            claim.key,
                            claim.mode.as_str(),
                            owner.plugin_id,
                            owner.provider_id,
                            owner
                                .source_path
                                .as_deref()
                                .map(|path| format!(", source `{path}`"))
                                .unwrap_or_default()
                        ));
                }
            }
        }
    }

    for (index, entry) in entries.iter().enumerate() {
        for other in entries.iter().skip(index + 1) {
            for claim in &entry.slot_claims {
                let Some(other_claim) = other
                    .slot_claims
                    .iter()
                    .find(|candidate| candidate.slot == claim.slot && candidate.key == claim.key)
                else {
                    continue;
                };

                if !slot_modes_conflict(claim.mode, other_claim.mode) {
                    continue;
                }

                conflicts
                    .entry((entry.source_path.clone(), entry.plugin_id.clone()))
                    .or_default()
                    .insert(format!(
                        "slot claim `{}`:`{}` ({}) conflicts with plugin `{}` (provider `{}`, source `{}`) as `{}`",
                        claim.slot,
                        claim.key,
                        claim.mode.as_str(),
                        other.plugin_id,
                        other.provider_id,
                        other.source_path,
                        other_claim.mode.as_str()
                    ));
                conflicts
                    .entry((other.source_path.clone(), other.plugin_id.clone()))
                    .or_default()
                    .insert(format!(
                        "slot claim `{}`:`{}` ({}) conflicts with plugin `{}` (provider `{}`, source `{}`) as `{}`",
                        other_claim.slot,
                        other_claim.key,
                        other_claim.mode.as_str(),
                        entry.plugin_id,
                        entry.provider_id,
                        entry.source_path,
                        claim.mode.as_str()
                    ));
            }
        }
    }

    conflicts
        .into_iter()
        .map(|(key, reasons)| (key, reasons.into_iter().collect::<Vec<_>>().join("; ")))
        .collect()
}

fn existing_slot_claims_from_catalog(
    catalog: &IntegrationCatalog,
) -> BTreeMap<(String, String), Vec<SlotClaimOwner>> {
    let mut registry: BTreeMap<(String, String), Vec<SlotClaimOwner>> = BTreeMap::new();

    for provider in catalog.providers() {
        let Some(raw_json) = provider.metadata.get(PLUGIN_SLOT_CLAIMS_METADATA_KEY) else {
            continue;
        };
        let Ok(claims) = serde_json::from_str::<Vec<PluginSlotClaim>>(raw_json) else {
            continue;
        };

        let plugin_id = provider
            .metadata
            .get("plugin_id")
            .cloned()
            .unwrap_or_else(|| format!("provider:{}", provider.provider_id));
        let source_path = provider.metadata.get("plugin_source_path").cloned();

        for claim in claims {
            registry
                .entry((claim.slot, claim.key))
                .or_default()
                .push(SlotClaimOwner {
                    plugin_id: plugin_id.clone(),
                    provider_id: provider.provider_id.clone(),
                    mode: claim.mode,
                    source_path: source_path.clone(),
                });
        }
    }

    registry
}

fn infer_runtime_profile(language: &str, manifest: &PluginManifest) -> PluginRuntimeProfile {
    let endpoint = manifest.endpoint.as_deref();
    infer_runtime_profile_from_parts(language, &manifest.metadata, endpoint)
}

fn infer_runtime_profile_from_parts(
    language: &str,
    metadata: &BTreeMap<String, String>,
    endpoint: Option<&str>,
) -> PluginRuntimeProfile {
    let source_language = metadata
        .get("source_language")
        .map(|value| normalize_language(value))
        .filter(|value| value != "unknown")
        .unwrap_or_else(|| normalize_language(language));

    let bridge_kind = metadata
        .get("bridge_kind")
        .and_then(|value| parse_bridge_kind(value))
        .or_else(|| {
            metadata
                .get("protocol")
                .filter(|value| value.eq_ignore_ascii_case("mcp"))
                .map(|_| PluginBridgeKind::McpServer)
        })
        .unwrap_or_else(|| default_bridge_kind(&source_language, endpoint));

    let adapter_family = metadata
        .get("adapter_family")
        .cloned()
        .unwrap_or_else(|| default_adapter_family(&source_language, bridge_kind));

    let entrypoint_hint = metadata
        .get("entrypoint")
        .cloned()
        .or_else(|| default_entrypoint_hint(bridge_kind, endpoint))
        .unwrap_or_else(|| "invoke".to_owned());

    PluginRuntimeProfile {
        source_language,
        bridge_kind,
        adapter_family,
        entrypoint_hint,
    }
}

fn derive_channel_bridge_contract(
    manifest: &PluginManifest,
) -> Option<PluginChannelBridgeContract> {
    let channel_id = normalized_optional_value(manifest.channel_id.as_deref());
    let setup_surface = normalized_optional_value(
        manifest
            .setup
            .as_ref()
            .and_then(|setup| setup.surface.as_deref()),
    );
    let transport_family = normalized_manifest_metadata_value(manifest, "transport_family");
    let target_contract = normalized_manifest_metadata_value(manifest, "target_contract");
    let account_scope = normalized_manifest_metadata_value(manifest, "account_scope");
    let runtime_contract = normalized_manifest_metadata_value(manifest, "channel_runtime_contract");
    let runtime_operations =
        normalized_manifest_metadata_string_list(manifest, "channel_runtime_operations_json");
    let mut runtime_metadata_issues = Vec::new();
    let runtime_operations = match runtime_operations {
        Ok(runtime_operations) => runtime_operations,
        Err(issue) => {
            runtime_metadata_issues.push(issue);
            Vec::new()
        }
    };
    let adapter_family = normalized_manifest_metadata_value(manifest, "adapter_family");

    let has_channel_bridge_metadata =
        transport_family.is_some() || target_contract.is_some() || account_scope.is_some();
    let adapter_declares_channel_bridge = adapter_family.as_deref() == Some("channel-bridge");
    // OpenClaw packages can legitimately use setup.surface=channel without being
    // managed channel-bridge plugins. Treat explicit bridge metadata or the
    // sanctioned adapter family as the bridge declaration boundary instead.
    let declares_channel_bridge = has_channel_bridge_metadata || adapter_declares_channel_bridge;

    if !declares_channel_bridge {
        return None;
    }

    let readiness = evaluate_channel_bridge_readiness(
        channel_id.as_deref(),
        setup_surface.as_deref(),
        transport_family.as_deref(),
        target_contract.as_deref(),
    );

    Some(PluginChannelBridgeContract {
        channel_id,
        setup_surface,
        transport_family,
        target_contract,
        account_scope,
        runtime_contract,
        runtime_operations,
        runtime_metadata_issues,
        readiness,
    })
}

fn evaluate_channel_bridge_readiness(
    channel_id: Option<&str>,
    setup_surface: Option<&str>,
    transport_family: Option<&str>,
    target_contract: Option<&str>,
) -> PluginChannelBridgeReadiness {
    let mut missing_fields = Vec::new();

    if channel_id.is_none() {
        missing_fields.push("channel_id".to_owned());
    }

    if setup_surface != Some("channel") {
        missing_fields.push("setup.surface".to_owned());
    }

    if transport_family.is_none() {
        missing_fields.push("metadata.transport_family".to_owned());
    }

    if target_contract.is_none() {
        missing_fields.push("metadata.target_contract".to_owned());
    }

    let ready = missing_fields.is_empty();

    PluginChannelBridgeReadiness {
        ready,
        missing_fields,
    }
}

fn normalized_manifest_metadata_value(manifest: &PluginManifest, key: &str) -> Option<String> {
    let value = manifest.metadata.get(key);
    let value = value.map(String::as_str);
    normalized_optional_value(value)
}

fn normalized_manifest_metadata_string_list(
    manifest: &PluginManifest,
    key: &str,
) -> Result<Vec<String>, String> {
    let Some(raw_value) = manifest.metadata.get(key) else {
        return Ok(Vec::new());
    };

    let parsed_values = serde_json::from_str::<Vec<String>>(raw_value)
        .map_err(|error| format!("metadata.{key} must be valid json string array: {error}"))?;

    let mut normalized_values = Vec::new();
    for parsed_value in parsed_values {
        let trimmed_value = parsed_value.trim();
        if trimmed_value.is_empty() {
            continue;
        }

        let normalized_value = trimmed_value.to_owned();
        normalized_values.push(normalized_value);
    }

    Ok(normalized_values)
}

pub fn plugin_native_extension_declarations_from_metadata(
    metadata: &BTreeMap<String, String>,
) -> PluginNativeExtensionDeclarations {
    let mut declarations = PluginNativeExtensionDeclarations {
        contract: normalized_optional_metadata_value(metadata, "loong_extension_contract"),
        family: normalized_optional_metadata_value(metadata, "loong_extension_family"),
        trust_lane: normalized_optional_metadata_value(metadata, "loong_extension_trust_lane"),
        ..Default::default()
    };
    declarations.facets = normalized_metadata_string_list_with_issue(
        metadata,
        "loong_extension_facets_json",
        &mut declarations.metadata_issues,
    );
    declarations.methods = normalized_metadata_string_list_with_issue(
        metadata,
        "loong_extension_methods_json",
        &mut declarations.metadata_issues,
    );
    declarations.events = normalized_metadata_string_list_with_issue(
        metadata,
        "loong_extension_events_json",
        &mut declarations.metadata_issues,
    );
    declarations.host_hooks = normalized_metadata_string_list_with_issue(
        metadata,
        "loong_extension_host_hooks_json",
        &mut declarations.metadata_issues,
    );
    declarations.host_actions = normalized_metadata_string_list_with_issue(
        metadata,
        "loong_extension_host_actions_json",
        &mut declarations.metadata_issues,
    );
    validate_plugin_native_extension_declarations(&mut declarations);
    declarations
}

fn validate_plugin_native_extension_declarations(
    declarations: &mut PluginNativeExtensionDeclarations,
) {
    if declarations.host_hooks.is_empty() {
        return;
    }

    if declarations.family.as_deref() != Some(TRUSTED_HOST_EXTENSION_FAMILY)
        || declarations.trust_lane.as_deref() != Some(TRUSTED_HOST_EXTENSION_TRUST_LANE)
    {
        declarations.metadata_issues.push(format!(
            "metadata `loong_extension_host_hooks_json` requires loong_extension_family=`{TRUSTED_HOST_EXTENSION_FAMILY}` and loong_extension_trust_lane=`{TRUSTED_HOST_EXTENSION_TRUST_LANE}`; the current governed sidecar lane cannot execute declared host hooks"
        ));
    }

    for hook in &declarations.host_hooks {
        if !TRUSTED_HOST_READ_ONLY_EXTENSION_HOOKS.contains(&hook.as_str()) {
            declarations.metadata_issues.push(format!(
                "metadata `loong_extension_host_hooks_json` declares unsupported host hook `{hook}`; supported read-only hooks are {}",
                TRUSTED_HOST_READ_ONLY_EXTENSION_HOOKS.join(", ")
            ));
        }
    }
}

fn normalized_optional_metadata_value(
    metadata: &BTreeMap<String, String>,
    key: &str,
) -> Option<String> {
    let value = metadata.get(key).map(String::as_str);
    normalized_optional_value(value)
}

fn normalized_metadata_string_list_with_issue(
    metadata: &BTreeMap<String, String>,
    key: &str,
    metadata_issues: &mut Vec<String>,
) -> Vec<String> {
    let Some(raw_value) = metadata.get(key) else {
        return Vec::new();
    };
    if raw_value.trim().is_empty() {
        return Vec::new();
    }

    match serde_json::from_str::<Vec<String>>(raw_value) {
        Ok(parsed_values) => parsed_values
            .into_iter()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .collect(),
        Err(error) => {
            metadata_issues.push(format!(
                "metadata `{key}` must be a JSON string array: {error}"
            ));
            Vec::new()
        }
    }
}

fn normalized_optional_value(raw: Option<&str>) -> Option<String> {
    let value = raw?;
    let trimmed = value.trim();

    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_owned())
}

pub fn plugin_runtime_scaffold_defaults(
    bridge_kind: PluginBridgeKind,
    source_language: Option<&str>,
) -> Result<PluginRuntimeScaffoldDefaults, String> {
    if matches!(bridge_kind, PluginBridgeKind::Unknown) {
        return Err("plugin scaffold does not support bridge_kind `unknown`".to_owned());
    }

    let normalized_source_language = source_language
        .map(normalize_language)
        .filter(|value| value != "unknown" && value != "manifest");

    let source_language_is_required = matches!(
        bridge_kind,
        PluginBridgeKind::ProcessStdio | PluginBridgeKind::NativeFfi
    );

    if source_language_is_required && normalized_source_language.is_none() {
        return Err(format!(
            "plugin scaffold requires an explicit source language for bridge_kind `{}`",
            bridge_kind.as_str()
        ));
    }

    let adapter_language = normalized_source_language.as_deref().unwrap_or("unknown");
    let adapter_family = default_adapter_family(adapter_language, bridge_kind);
    let entrypoint_hint =
        default_entrypoint_hint(bridge_kind, None).unwrap_or_else(|| "invoke".to_owned());

    Ok(PluginRuntimeScaffoldDefaults {
        source_language: normalized_source_language,
        bridge_kind,
        adapter_family,
        entrypoint_hint,
    })
}

fn legacy_plugin_ir_dialect(source_kind: PluginSourceKind) -> PluginContractDialect {
    match source_kind {
        PluginSourceKind::PackageManifest => PluginContractDialect::LoongPackageManifest,
        PluginSourceKind::EmbeddedSource => PluginContractDialect::LoongEmbeddedSource,
    }
}

fn legacy_plugin_ir_runtime_profile(
    source_path: &str,
    source_kind: PluginSourceKind,
    metadata: &BTreeMap<String, String>,
    endpoint: Option<&str>,
) -> PluginRuntimeProfile {
    let source_language = legacy_plugin_ir_source_language(source_path, source_kind);
    infer_runtime_profile_from_parts(&source_language, metadata, endpoint)
}

fn legacy_plugin_ir_source_language(source_path: &str, source_kind: PluginSourceKind) -> String {
    if source_kind == PluginSourceKind::PackageManifest {
        return "unknown".to_owned();
    }

    let extension = Path::new(source_path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    normalize_language(extension)
}

fn normalize_language(language: &str) -> String {
    match language.trim().to_ascii_lowercase().as_str() {
        "rs" => "rust".to_owned(),
        "py" => "python".to_owned(),
        "js" => "javascript".to_owned(),
        "ts" => "typescript".to_owned(),
        "go" => "go".to_owned(),
        "wasm" => "wasm".to_owned(),
        "" => "unknown".to_owned(),
        other => other.to_owned(),
    }
}

fn parse_bridge_kind(raw: &str) -> Option<PluginBridgeKind> {
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

fn default_bridge_kind(language: &str, endpoint: Option<&str>) -> PluginBridgeKind {
    match language {
        "rust" | "go" | "c" | "cpp" | "cxx" => PluginBridgeKind::NativeFfi,
        "python" | "javascript" | "typescript" | "java" => PluginBridgeKind::ProcessStdio,
        "wasm" | "wat" => PluginBridgeKind::WasmComponent,
        _ => {
            if let Some(endpoint) = endpoint
                && (endpoint.starts_with("http://") || endpoint.starts_with("https://"))
            {
                return PluginBridgeKind::HttpJson;
            }
            PluginBridgeKind::Unknown
        }
    }
}

fn default_adapter_family(language: &str, bridge_kind: PluginBridgeKind) -> String {
    match bridge_kind {
        PluginBridgeKind::HttpJson => "http-adapter".to_owned(),
        PluginBridgeKind::ProcessStdio => format!("{language}-stdio-adapter"),
        PluginBridgeKind::NativeFfi => format!("{language}-ffi-adapter"),
        PluginBridgeKind::WasmComponent => "wasm-component-adapter".to_owned(),
        PluginBridgeKind::McpServer => "mcp-adapter".to_owned(),
        PluginBridgeKind::AcpBridge => "acp-bridge-adapter".to_owned(),
        PluginBridgeKind::AcpRuntime => "acp-runtime-adapter".to_owned(),
        PluginBridgeKind::Unknown => format!("{language}-unknown-adapter"),
    }
}

fn default_entrypoint_hint(
    bridge_kind: PluginBridgeKind,
    endpoint: Option<&str>,
) -> Option<String> {
    match bridge_kind {
        PluginBridgeKind::HttpJson => {
            Some(endpoint.unwrap_or("https://localhost/invoke").to_owned())
        }
        PluginBridgeKind::ProcessStdio => Some("stdin/stdout::invoke".to_owned()),
        PluginBridgeKind::NativeFfi => Some("lib::invoke".to_owned()),
        PluginBridgeKind::WasmComponent => Some("component::run".to_owned()),
        PluginBridgeKind::McpServer => Some("mcp::stdio".to_owned()),
        PluginBridgeKind::AcpBridge => Some("acp::bridge".to_owned()),
        PluginBridgeKind::AcpRuntime => Some("acp::turn".to_owned()),
        PluginBridgeKind::Unknown => None,
    }
}

fn bootstrap_hint(ir: &PluginIR) -> String {
    let compatibility_prefix = PluginCompatibilityShim::for_mode(ir.compatibility_mode)
        .map(|shim| {
            format!(
                "enable compatibility shim `{}` ({}) and then ",
                shim.shim_id, shim.family
            )
        })
        .unwrap_or_default();

    match ir.runtime.bridge_kind {
        PluginBridgeKind::HttpJson => format!(
            "{}register http connector adapter for {} at {}",
            compatibility_prefix,
            ir.connector_name,
            ir.endpoint.as_deref().unwrap_or("https://localhost/invoke")
        ),
        PluginBridgeKind::ProcessStdio => format!(
            "{}spawn {} worker and bind stdio bridge {}",
            compatibility_prefix, ir.runtime.source_language, ir.runtime.entrypoint_hint
        ),
        PluginBridgeKind::NativeFfi => format!(
            "{}load native library adapter {} with symbol {}",
            compatibility_prefix, ir.runtime.adapter_family, ir.runtime.entrypoint_hint
        ),
        PluginBridgeKind::WasmComponent => {
            format!(
                "{}load wasm component and invoke {}",
                compatibility_prefix, ir.runtime.entrypoint_hint
            )
        }
        PluginBridgeKind::McpServer => format!(
            "{}register MCP server bridge and handshake capability schema",
            compatibility_prefix
        ),
        PluginBridgeKind::AcpBridge => format!(
            "{}register ACP bridge surface and bind the external gateway/runtime contract",
            compatibility_prefix
        ),
        PluginBridgeKind::AcpRuntime => {
            format!(
                "{}register ACP runtime backend and bind a session-aware control plane",
                compatibility_prefix
            )
        }
        PluginBridgeKind::Unknown => format!(
            "{}inspect plugin metadata and define explicit bridge_kind override",
            compatibility_prefix
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        integration::ProviderConfig,
        plugin::{
            CURRENT_PLUGIN_HOST_API, CURRENT_PLUGIN_MANIFEST_API_VERSION, PluginCompatibility,
            PluginCompatibilityMode, PluginContractDialect, PluginDiagnosticCode,
            PluginDiagnosticFinding, PluginDiagnosticPhase, PluginDiagnosticSeverity,
            PluginManifest, PluginSetup, PluginSetupMode, PluginSlotClaim, PluginSlotMode,
            PluginSourceKind, PluginTrustTier,
        },
    };

    fn descriptor(language: &str, metadata: BTreeMap<String, String>) -> PluginDescriptor {
        let source_kind = if language == "manifest" {
            PluginSourceKind::PackageManifest
        } else {
            PluginSourceKind::EmbeddedSource
        };
        let path = if language == "manifest" {
            "/tmp/loong.plugin.json".to_owned()
        } else {
            format!("/tmp/plugin.{language}")
        };
        let package_manifest_path = if matches!(source_kind, PluginSourceKind::PackageManifest) {
            Some(path.clone())
        } else {
            None
        };

        PluginDescriptor {
            path,
            source_kind,
            dialect: match source_kind {
                PluginSourceKind::PackageManifest => PluginContractDialect::LoongPackageManifest,
                PluginSourceKind::EmbeddedSource => PluginContractDialect::LoongEmbeddedSource,
            },
            dialect_version: matches!(source_kind, PluginSourceKind::PackageManifest)
                .then(|| CURRENT_PLUGIN_MANIFEST_API_VERSION.to_owned()),
            compatibility_mode: PluginCompatibilityMode::Native,
            package_root: "/tmp".to_owned(),
            package_manifest_path,
            language: language.to_owned(),
            manifest: PluginManifest {
                api_version: matches!(source_kind, PluginSourceKind::PackageManifest)
                    .then(|| CURRENT_PLUGIN_MANIFEST_API_VERSION.to_owned()),
                version: Some("1.0.0".to_owned()),
                plugin_id: format!("sample-{language}"),
                provider_id: "sample-provider".to_owned(),
                connector_name: "sample-connector".to_owned(),
                channel_id: Some("primary".to_owned()),
                endpoint: Some("https://example.com/invoke".to_owned()),
                capabilities: BTreeSet::from([Capability::InvokeConnector]),
                trust_tier: PluginTrustTier::VerifiedCommunity,
                metadata,
                summary: None,
                tags: Vec::new(),
                input_examples: Vec::new(),
                output_examples: Vec::new(),
                defer_loading: false,
                setup: Some(PluginSetup {
                    mode: PluginSetupMode::MetadataOnly,
                    surface: Some("web_search".to_owned()),
                    required_env_vars: vec!["TAVILY_API_KEY".to_owned()],
                    recommended_env_vars: vec!["TEAM_TAVILY_KEY".to_owned()],
                    required_config_keys: vec!["tools.web_search.default_provider".to_owned()],
                    default_env_var: Some("TAVILY_API_KEY".to_owned()),
                    docs_urls: vec!["https://docs.example.com/tavily".to_owned()],
                    remediation: Some("set a Tavily credential before enabling search".to_owned()),
                }),
                slot_claims: Vec::new(),
                compatibility: None,
            },
        }
    }

    fn verified_setup_readiness_context() -> PluginSetupReadinessContext {
        PluginSetupReadinessContext {
            verified_env_vars: BTreeSet::from(["TAVILY_API_KEY".to_owned()]),
            verified_config_keys: BTreeSet::from(["tools.web_search.default_provider".to_owned()]),
        }
    }

    fn channel_bridge_descriptor(metadata: BTreeMap<String, String>) -> PluginDescriptor {
        let mut descriptor = descriptor("manifest", metadata);

        descriptor.manifest.plugin_id = "weixin-clawbot-bridge".to_owned();
        descriptor.manifest.provider_id = "weixin-bridge".to_owned();
        descriptor.manifest.connector_name = "weixin-clawbot-http".to_owned();
        descriptor.manifest.channel_id = Some("weixin".to_owned());
        descriptor.manifest.endpoint = Some("http://127.0.0.1:8091/bridge".to_owned());
        descriptor
            .manifest
            .metadata
            .entry("adapter_family".to_owned())
            .or_insert_with(|| "channel-bridge".to_owned());
        descriptor.manifest.setup = Some(PluginSetup {
            mode: PluginSetupMode::MetadataOnly,
            surface: Some("channel".to_owned()),
            required_env_vars: vec!["WEIXIN_BRIDGE_URL".to_owned()],
            recommended_env_vars: vec!["WEIXIN_BRIDGE_ACCESS_TOKEN".to_owned()],
            required_config_keys: vec![
                "weixin.enabled".to_owned(),
                "weixin.bridge_url".to_owned(),
                "weixin.bridge_access_token".to_owned(),
            ],
            default_env_var: Some("WEIXIN_BRIDGE_URL".to_owned()),
            docs_urls: vec!["https://docs.example.com/weixin-bridge".to_owned()],
            remediation: Some("configure the sanctioned weixin bridge contract".to_owned()),
        });

        descriptor
    }

    #[test]
    fn translator_infers_bridge_from_source_language() {
        let scanner_report = PluginScanReport {
            scanned_files: 2,
            matched_plugins: 2,
            diagnostic_findings: Vec::new(),
            descriptors: vec![
                descriptor("rs", BTreeMap::new()),
                descriptor("py", BTreeMap::new()),
            ],
        };

        let translator = PluginTranslator::new();
        let report = translator.translate_scan_report(&scanner_report);

        assert_eq!(report.translated_plugins, 2);
        assert_eq!(
            report.bridge_distribution.get("native_ffi").copied(),
            Some(1)
        );
        assert_eq!(
            report.bridge_distribution.get("process_stdio").copied(),
            Some(1)
        );
        assert!(
            report
                .entries
                .iter()
                .all(|entry| entry.trust_tier == PluginTrustTier::VerifiedCommunity)
        );
    }

    #[test]
    fn translator_honors_metadata_bridge_override() {
        let descriptor = descriptor(
            "js",
            BTreeMap::from([
                ("bridge_kind".to_owned(), "mcp_server".to_owned()),
                ("entrypoint".to_owned(), "custom::run".to_owned()),
            ]),
        );

        let translator = PluginTranslator::new();
        let ir = translator.translate_descriptor(&descriptor);

        assert_eq!(ir.runtime.bridge_kind, PluginBridgeKind::McpServer);
        assert_eq!(ir.runtime.entrypoint_hint, "custom::run");
        assert_eq!(ir.runtime.adapter_family, "mcp-adapter");
    }

    #[test]
    fn translator_honors_metadata_source_language_for_package_manifests() {
        let descriptor = descriptor(
            "manifest",
            BTreeMap::from([
                ("bridge_kind".to_owned(), "process_stdio".to_owned()),
                ("source_language".to_owned(), "py".to_owned()),
            ]),
        );

        let translator = PluginTranslator::new();
        let ir = translator.translate_descriptor(&descriptor);

        assert_eq!(ir.runtime.source_language, "python");
        assert_eq!(ir.runtime.bridge_kind, PluginBridgeKind::ProcessStdio);
        assert_eq!(ir.runtime.adapter_family, "python-stdio-adapter");
        assert_eq!(ir.runtime.entrypoint_hint, "stdin/stdout::invoke");
    }

    #[test]
    fn translator_defaults_manifest_descriptor_with_endpoint_to_http_json() {
        let descriptor = descriptor("manifest", BTreeMap::new());

        let translator = PluginTranslator::new();
        let ir = translator.translate_descriptor(&descriptor);

        assert_eq!(ir.runtime.source_language, "manifest");
        assert_eq!(ir.runtime.bridge_kind, PluginBridgeKind::HttpJson);
        assert_eq!(ir.runtime.adapter_family, "http-adapter");
        assert_eq!(ir.trust_tier, PluginTrustTier::VerifiedCommunity);
        assert_eq!(ir.source_kind, PluginSourceKind::PackageManifest);
        assert_eq!(ir.package_root, "/tmp");
        assert_eq!(
            ir.setup.as_ref().and_then(|setup| setup.surface.as_deref()),
            Some("web_search")
        );
        assert_eq!(
            ir.package_manifest_path,
            Some("/tmp/loong.plugin.json".to_owned())
        );
    }

    #[test]
    fn translator_derives_channel_bridge_contract_from_manifest_conventions() {
        let descriptor = channel_bridge_descriptor(BTreeMap::from([
            (
                "transport_family".to_owned(),
                "wechat_clawbot_ilink_bridge".to_owned(),
            ),
            (
                "target_contract".to_owned(),
                "weixin:<account>:contact:<id> | weixin:<account>:room:<id>".to_owned(),
            ),
            ("account_scope".to_owned(), "multi_account".to_owned()),
            (
                "channel_runtime_contract".to_owned(),
                "loong_channel_bridge_v1".to_owned(),
            ),
            (
                "channel_runtime_operations_json".to_owned(),
                "[\"send\",\"receive_batch\",\"ack_inbound\",\"complete_batch\"]".to_owned(),
            ),
        ]));

        let translator = PluginTranslator::new();
        let ir = translator.translate_descriptor(&descriptor);

        let channel_bridge = ir
            .channel_bridge
            .as_ref()
            .expect("channel bridge contract should exist");

        assert_eq!(channel_bridge.channel_id.as_deref(), Some("weixin"));
        assert_eq!(channel_bridge.setup_surface.as_deref(), Some("channel"));
        assert_eq!(
            channel_bridge.transport_family.as_deref(),
            Some("wechat_clawbot_ilink_bridge")
        );
        assert_eq!(
            channel_bridge.target_contract.as_deref(),
            Some("weixin:<account>:contact:<id> | weixin:<account>:room:<id>")
        );
        assert_eq!(
            channel_bridge.account_scope.as_deref(),
            Some("multi_account")
        );
        assert_eq!(
            channel_bridge.runtime_contract.as_deref(),
            Some("loong_channel_bridge_v1")
        );
        assert_eq!(
            channel_bridge.runtime_operations,
            vec![
                "send".to_owned(),
                "receive_batch".to_owned(),
                "ack_inbound".to_owned(),
                "complete_batch".to_owned(),
            ]
        );
        assert!(channel_bridge.readiness.ready);
        assert!(channel_bridge.readiness.missing_fields.is_empty());
    }

    #[test]
    fn translator_marks_declared_channel_bridge_contract_incomplete_when_required_fields_are_missing()
     {
        let descriptor = channel_bridge_descriptor(BTreeMap::new());

        let translator = PluginTranslator::new();
        let ir = translator.translate_descriptor(&descriptor);

        let channel_bridge = ir
            .channel_bridge
            .as_ref()
            .expect("channel bridge contract should exist");

        assert!(!channel_bridge.readiness.ready);
        assert_eq!(
            channel_bridge.readiness.missing_fields,
            vec![
                "metadata.transport_family".to_owned(),
                "metadata.target_contract".to_owned(),
            ]
        );
    }

    #[test]
    fn translator_preserves_invalid_runtime_operations_metadata_json_as_issue() {
        let descriptor = channel_bridge_descriptor(BTreeMap::from([
            (
                "transport_family".to_owned(),
                "wechat_clawbot_ilink_bridge".to_owned(),
            ),
            (
                "target_contract".to_owned(),
                "weixin:<account>:contact:<id> | weixin:<account>:room:<id>".to_owned(),
            ),
            (
                "channel_runtime_operations_json".to_owned(),
                "{not-json}".to_owned(),
            ),
        ]));

        let translator = PluginTranslator::new();
        let ir = translator.translate_descriptor(&descriptor);
        let channel_bridge = ir
            .channel_bridge
            .as_ref()
            .expect("channel bridge contract should exist");

        assert!(channel_bridge.runtime_operations.is_empty());
        assert_eq!(channel_bridge.runtime_metadata_issues.len(), 1);
        let issue = channel_bridge
            .runtime_metadata_issues
            .first()
            .expect("runtime metadata issue should exist");
        assert!(issue.starts_with(
            "metadata.channel_runtime_operations_json must be valid json string array:"
        ));
    }

    #[test]
    fn translator_does_not_treat_channel_surface_only_setup_as_channel_bridge_contract() {
        let mut descriptor = descriptor("manifest", BTreeMap::new());

        descriptor.manifest.channel_id = Some("weather".to_owned());
        descriptor.manifest.setup = Some(PluginSetup {
            mode: PluginSetupMode::GovernedEntry,
            surface: Some("channel".to_owned()),
            required_env_vars: vec!["WEATHER_API_KEY".to_owned()],
            recommended_env_vars: Vec::new(),
            required_config_keys: vec!["plugins.entries.weather-sdk".to_owned()],
            default_env_var: Some("WEATHER_API_KEY".to_owned()),
            docs_urls: vec!["https://example.com/weather-sdk".to_owned()],
            remediation: Some("configure the weather sdk before activation".to_owned()),
        });

        let translator = PluginTranslator::new();
        let ir = translator.translate_descriptor(&descriptor);

        assert!(
            ir.channel_bridge.is_none(),
            "plain channel-surface setup should not be treated as a managed channel bridge"
        );
    }

    #[test]
    fn plugin_runtime_scaffold_defaults_require_source_language_for_process_stdio() {
        let error = plugin_runtime_scaffold_defaults(PluginBridgeKind::ProcessStdio, None)
            .expect_err("process bridge scaffold should require source language");

        assert!(error.contains("source language"));
        assert!(error.contains("process_stdio"));
    }

    #[test]
    fn plugin_runtime_scaffold_defaults_require_source_language_for_native_ffi() {
        let error = plugin_runtime_scaffold_defaults(PluginBridgeKind::NativeFfi, None)
            .expect_err("native ffi scaffold should require source language");

        assert!(error.contains("source language"));
        assert!(error.contains("native_ffi"));
    }

    #[test]
    fn plugin_runtime_scaffold_defaults_normalize_python_process_bridge() {
        let defaults = plugin_runtime_scaffold_defaults(PluginBridgeKind::ProcessStdio, Some("py"))
            .expect("python process bridge scaffold defaults should resolve");

        assert_eq!(defaults.source_language.as_deref(), Some("python"));
        assert_eq!(defaults.bridge_kind, PluginBridgeKind::ProcessStdio);
        assert_eq!(defaults.adapter_family, "python-stdio-adapter");
        assert_eq!(defaults.entrypoint_hint, "stdin/stdout::invoke");
    }

    #[test]
    fn translator_accepts_acpx_runtime_alias() {
        let descriptor = descriptor(
            "js",
            BTreeMap::from([("bridge_kind".to_owned(), "acpx".to_owned())]),
        );

        let translator = PluginTranslator::new();
        let ir = translator.translate_descriptor(&descriptor);

        assert_eq!(ir.runtime.bridge_kind, PluginBridgeKind::AcpRuntime);
        assert_eq!(ir.runtime.adapter_family, "acp-runtime-adapter");
        assert_eq!(ir.runtime.entrypoint_hint, "acp::turn");
    }

    #[test]
    fn translator_maps_acp_alias_to_bridge_surface() {
        let descriptor = descriptor(
            "js",
            BTreeMap::from([("bridge_kind".to_owned(), "acp".to_owned())]),
        );

        let translator = PluginTranslator::new();
        let ir = translator.translate_descriptor(&descriptor);

        assert_eq!(ir.runtime.bridge_kind, PluginBridgeKind::AcpBridge);
        assert_eq!(ir.runtime.adapter_family, "acp-bridge-adapter");
        assert_eq!(ir.runtime.entrypoint_hint, "acp::bridge");
    }

    #[test]
    fn translator_projects_scan_diagnostics_into_ir() {
        let descriptor = descriptor("js", BTreeMap::new());
        let translation = PluginTranslator::new().translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: vec![PluginDiagnosticFinding {
                code: PluginDiagnosticCode::EmbeddedSourceLegacyContract,
                severity: PluginDiagnosticSeverity::Warning,
                phase: PluginDiagnosticPhase::Scan,
                blocking: false,
                plugin_id: Some("sample-js".to_owned()),
                source_path: Some("/tmp/plugin.js".to_owned()),
                source_kind: Some(PluginSourceKind::EmbeddedSource),
                field_path: None,
                message: "legacy source marker".to_owned(),
                remediation: Some("add loong.plugin.json".to_owned()),
            }],
            descriptors: vec![descriptor],
        });

        assert_eq!(translation.entries.len(), 1);
        assert_eq!(translation.entries[0].diagnostic_findings.len(), 1);
        assert_eq!(
            translation.entries[0].diagnostic_findings[0].code,
            PluginDiagnosticCode::EmbeddedSourceLegacyContract
        );
        assert_eq!(
            translation.entries[0].diagnostic_findings[0].phase,
            PluginDiagnosticPhase::Scan
        );
        assert!(!translation.entries[0].diagnostic_findings[0].blocking);
    }

    #[test]
    fn activation_plan_marks_setup_incomplete_when_required_setup_is_missing() {
        let descriptor = descriptor(
            "js",
            BTreeMap::from([("bridge_kind".to_owned(), "http_json".to_owned())]),
        );
        let translator = PluginTranslator::new();
        let translation = translator.translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
            supported_compatibility_modes: BTreeSet::from([PluginCompatibilityMode::Native]),
            supported_compatibility_shims: BTreeSet::new(),
            supported_compatibility_shim_profiles: BTreeMap::new(),
        };
        let setup_readiness_context = PluginSetupReadinessContext::default();
        let plan = translator.plan_activation(&translation, &matrix, &setup_readiness_context);

        assert_eq!(plan.total_plugins, 1);
        assert_eq!(plan.ready_plugins, 0);
        assert_eq!(plan.setup_incomplete_plugins, 1);
        assert_eq!(plan.blocked_plugins, 0);
        assert_eq!(
            plan.candidates[0].source_kind,
            PluginSourceKind::EmbeddedSource
        );
        assert_eq!(plan.candidates[0].package_root, "/tmp");
        assert_eq!(plan.candidates[0].package_manifest_path, None);
        assert_eq!(
            plan.candidates[0].trust_tier,
            PluginTrustTier::VerifiedCommunity
        );
        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::SetupIncomplete
        ));
        assert_eq!(
            plan.candidates[0].missing_required_env_vars,
            vec!["TAVILY_API_KEY".to_owned()]
        );
        assert_eq!(
            plan.candidates[0].missing_required_config_keys,
            vec!["tools.web_search.default_provider".to_owned()]
        );
    }

    #[test]
    fn activation_plan_blocks_declared_channel_bridge_when_manifest_contract_fields_are_missing() {
        let descriptor = channel_bridge_descriptor(BTreeMap::new());
        let translator = PluginTranslator::new();
        let translation = translator.translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
            supported_compatibility_modes: BTreeSet::from([PluginCompatibilityMode::Native]),
            supported_compatibility_shims: BTreeSet::new(),
            supported_compatibility_shim_profiles: BTreeMap::new(),
        };
        let setup_readiness_context = PluginSetupReadinessContext {
            verified_env_vars: BTreeSet::from(["WEIXIN_BRIDGE_URL".to_owned()]),
            verified_config_keys: BTreeSet::from([
                "weixin.enabled".to_owned(),
                "weixin.bridge_url".to_owned(),
                "weixin.bridge_access_token".to_owned(),
            ]),
        };
        let plan = translator.plan_activation(&translation, &matrix, &setup_readiness_context);

        assert_eq!(plan.ready_plugins, 0);
        assert_eq!(plan.setup_incomplete_plugins, 0);
        assert_eq!(plan.blocked_plugins, 1);
        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::BlockedInvalidManifestContract
        ));
        assert!(
            plan.candidates[0]
                .reason
                .contains("metadata.transport_family"),
            "reason should surface missing transport family"
        );
        assert!(
            plan.candidates[0]
                .reason
                .contains("metadata.target_contract"),
            "reason should surface missing target contract"
        );
        assert!(
            plan.candidates[0]
                .diagnostic_findings
                .iter()
                .any(|finding| finding.code == PluginDiagnosticCode::InvalidManifestContract)
        );
    }

    #[test]
    fn evaluate_plugin_setup_requirements_uses_platform_env_name_rules() {
        let required_env_vars = vec!["PATH".to_owned()];
        let required_config_keys = Vec::new();
        let context = PluginSetupReadinessContext {
            verified_env_vars: BTreeSet::from(["Path".to_owned()]),
            verified_config_keys: BTreeSet::new(),
        };

        let readiness =
            evaluate_plugin_setup_requirements(&required_env_vars, &required_config_keys, &context);

        #[cfg(windows)]
        {
            assert!(readiness.ready);
            assert!(readiness.missing_required_env_vars.is_empty());
        }

        #[cfg(not(windows))]
        {
            assert!(!readiness.ready);
            assert_eq!(readiness.missing_required_env_vars, required_env_vars);
        }
    }

    #[test]
    fn activation_plan_deserializes_legacy_payloads_without_setup_fields() {
        let legacy_payload = r#"{
            "total_plugins": 1,
            "ready_plugins": 1,
            "blocked_plugins": 0,
            "candidates": [
                {
                    "plugin_id": "sample-plugin",
                    "source_path": "/tmp/plugin.py",
                    "source_kind": "embedded_source",
                    "package_root": "/tmp",
                    "package_manifest_path": null,
                    "bridge_kind": "http_json",
                    "adapter_family": "http-adapter",
                    "status": "ready",
                    "reason": "plugin runtime profile is supported by current runtime matrix",
                    "bootstrap_hint": "register http connector adapter"
                }
            ]
        }"#;

        let plan: PluginActivationPlan =
            serde_json::from_str(legacy_payload).expect("legacy payload should deserialize");

        assert_eq!(plan.setup_incomplete_plugins, 0);
        assert_eq!(plan.candidates.len(), 1);
        assert!(plan.candidates[0].missing_required_env_vars.is_empty());
        assert!(plan.candidates[0].missing_required_config_keys.is_empty());
    }

    #[test]
    fn plugin_ir_deserializes_legacy_embedded_source_payload_with_inferred_defaults() {
        let raw = r#"{
            "plugin_id": "legacy-plugin",
            "provider_id": "legacy-provider",
            "connector_name": "legacy-connector",
            "capabilities": [],
            "metadata": {},
            "source_path": "/tmp/legacy-plugin.py",
            "source_kind": "embedded_source",
            "package_root": "/tmp"
        }"#;

        let ir: PluginIR =
            serde_json::from_str(raw).expect("legacy embedded-source payload should deserialize");

        assert_eq!(ir.dialect, PluginContractDialect::LoongEmbeddedSource);
        assert_eq!(ir.compatibility_mode, PluginCompatibilityMode::Native);
        assert!(ir.diagnostic_findings.is_empty());
        assert!(ir.slot_claims.is_empty());
        assert_eq!(ir.runtime.source_language, "python");
        assert_eq!(ir.runtime.bridge_kind, PluginBridgeKind::ProcessStdio);
        assert_eq!(ir.runtime.adapter_family, "python-stdio-adapter");
    }

    #[test]
    fn plugin_ir_deserializes_legacy_package_manifest_payload_with_inferred_defaults() {
        let raw = r#"{
            "plugin_id": "legacy-package",
            "provider_id": "legacy-provider",
            "connector_name": "legacy-connector",
            "endpoint": "https://plugins.example.test/invoke",
            "capabilities": [],
            "metadata": {},
            "source_path": "/tmp/loong.plugin.json",
            "source_kind": "package_manifest",
            "package_root": "/tmp"
        }"#;

        let ir: PluginIR =
            serde_json::from_str(raw).expect("legacy package payload should deserialize");

        assert_eq!(ir.dialect, PluginContractDialect::LoongPackageManifest);
        assert_eq!(ir.compatibility_mode, PluginCompatibilityMode::Native);
        assert_eq!(ir.runtime.source_language, "unknown");
        assert_eq!(ir.runtime.bridge_kind, PluginBridgeKind::HttpJson);
        assert_eq!(
            ir.runtime.entrypoint_hint,
            "https://plugins.example.test/invoke"
        );
    }

    #[test]
    fn plugin_activation_status_deserializes_unknown_variants_as_unknown() {
        let raw = "\"blocked_future_contract_surface\"";
        let status: PluginActivationStatus =
            serde_json::from_str(raw).expect("unknown activation status should deserialize");

        assert_eq!(status, PluginActivationStatus::Unknown);
        assert_eq!(status.as_str(), "unknown");
    }

    #[test]
    fn activation_plan_blocks_unsupported_bridge() {
        let descriptor = descriptor(
            "js",
            BTreeMap::from([("bridge_kind".to_owned(), "mcp_server".to_owned())]),
        );
        let translator = PluginTranslator::new();
        let translation = translator.translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
            supported_compatibility_modes: BTreeSet::from([PluginCompatibilityMode::Native]),
            supported_compatibility_shims: BTreeSet::new(),
            supported_compatibility_shim_profiles: BTreeMap::new(),
        };
        let setup_readiness_context = PluginSetupReadinessContext::default();
        let plan = translator.plan_activation(&translation, &matrix, &setup_readiness_context);

        assert_eq!(plan.total_plugins, 1);
        assert_eq!(plan.ready_plugins, 0);
        assert_eq!(plan.blocked_plugins, 1);
        assert_eq!(
            plan.candidates[0].source_kind,
            PluginSourceKind::EmbeddedSource
        );
        assert_eq!(plan.candidates[0].package_root, "/tmp");
        assert_eq!(plan.candidates[0].package_manifest_path, None);
        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::BlockedUnsupportedBridge
        ));
        assert_eq!(
            plan.candidates[0].diagnostic_findings[0].code,
            PluginDiagnosticCode::UnsupportedBridge
        );
        assert_eq!(
            plan.candidates[0].diagnostic_findings[0].phase,
            PluginDiagnosticPhase::Activation
        );
        assert!(plan.candidates[0].diagnostic_findings[0].blocking);
    }

    #[test]
    fn activation_plan_blocks_unsupported_adapter_family() {
        let descriptor = descriptor(
            "py",
            BTreeMap::from([(
                "adapter_family".to_owned(),
                "python-stdio-adapter".to_owned(),
            )]),
        );
        let translator = PluginTranslator::new();
        let translation = translator.translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
            supported_adapter_families: BTreeSet::from(["rust-stdio-adapter".to_owned()]),
            supported_compatibility_modes: BTreeSet::from([PluginCompatibilityMode::Native]),
            supported_compatibility_shims: BTreeSet::new(),
            supported_compatibility_shim_profiles: BTreeMap::new(),
        };
        let setup_readiness_context = PluginSetupReadinessContext {
            verified_env_vars: BTreeSet::from(["TAVILY_API_KEY".to_owned()]),
            verified_config_keys: BTreeSet::from(["tools.web_search.default_provider".to_owned()]),
        };
        let plan = translator.plan_activation(&translation, &matrix, &setup_readiness_context);

        assert_eq!(plan.total_plugins, 1);
        assert_eq!(plan.ready_plugins, 0);
        assert_eq!(plan.blocked_plugins, 1);
        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::BlockedUnsupportedAdapterFamily
        ));
        assert_eq!(
            plan.candidates[0].diagnostic_findings[0].code,
            PluginDiagnosticCode::UnsupportedAdapterFamily
        );
    }

    #[test]
    fn activation_plan_blocks_conflicting_slot_claims_within_translation() {
        let mut first = descriptor(
            "js",
            BTreeMap::from([("bridge_kind".to_owned(), "http_json".to_owned())]),
        );
        first.manifest.plugin_id = "search-a".to_owned();
        first.manifest.provider_id = "search-a".to_owned();
        first.manifest.connector_name = "search-a".to_owned();
        first.path = "/tmp/search-a.js".to_owned();
        first.manifest.slot_claims = vec![PluginSlotClaim {
            slot: "provider:web_search".to_owned(),
            key: "tavily".to_owned(),
            mode: PluginSlotMode::Exclusive,
        }];

        let mut second = descriptor(
            "ts",
            BTreeMap::from([("bridge_kind".to_owned(), "http_json".to_owned())]),
        );
        second.manifest.plugin_id = "search-b".to_owned();
        second.manifest.provider_id = "search-b".to_owned();
        second.manifest.connector_name = "search-b".to_owned();
        second.path = "/tmp/search-b.ts".to_owned();
        second.manifest.slot_claims = vec![PluginSlotClaim {
            slot: "provider:web_search".to_owned(),
            key: "tavily".to_owned(),
            mode: PluginSlotMode::Exclusive,
        }];

        let translation = PluginTranslator::new().translate_scan_report(&PluginScanReport {
            scanned_files: 2,
            matched_plugins: 2,
            diagnostic_findings: Vec::new(),
            descriptors: vec![first, second],
        });
        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
            supported_compatibility_modes: BTreeSet::from([PluginCompatibilityMode::Native]),
            supported_compatibility_shims: BTreeSet::new(),
            supported_compatibility_shim_profiles: BTreeMap::new(),
        };

        let setup_readiness_context = verified_setup_readiness_context();
        let plan = PluginTranslator::new().plan_activation(
            &translation,
            &matrix,
            &setup_readiness_context,
        );

        assert_eq!(plan.total_plugins, 2);
        assert_eq!(plan.ready_plugins, 0);
        assert_eq!(plan.blocked_plugins, 2);
        assert!(plan.candidates.iter().all(|candidate| matches!(
            candidate.status,
            PluginActivationStatus::BlockedSlotClaimConflict
        )));
        assert!(
            plan.candidates[0].reason.contains("provider:web_search"),
            "slot conflict reason should mention the claimed surface"
        );
        assert!(plan.candidates.iter().all(|candidate| {
            candidate
                .diagnostic_findings
                .iter()
                .any(|finding| finding.code == PluginDiagnosticCode::SlotClaimConflict)
        }));
    }

    #[test]
    fn activation_plan_blocks_slot_claim_conflicts_against_existing_catalog() {
        let mut descriptor = descriptor(
            "js",
            BTreeMap::from([("bridge_kind".to_owned(), "http_json".to_owned())]),
        );
        descriptor.manifest.plugin_id = "incoming-search".to_owned();
        descriptor.manifest.provider_id = "incoming-search".to_owned();
        descriptor.manifest.connector_name = "incoming-search".to_owned();
        descriptor.path = "/tmp/incoming-search.js".to_owned();
        descriptor.manifest.slot_claims = vec![PluginSlotClaim {
            slot: "provider:web_search".to_owned(),
            key: "tavily".to_owned(),
            mode: PluginSlotMode::Exclusive,
        }];

        let translation = PluginTranslator::new().translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });
        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
            supported_compatibility_modes: BTreeSet::from([PluginCompatibilityMode::Native]),
            supported_compatibility_shims: BTreeSet::new(),
            supported_compatibility_shim_profiles: BTreeMap::new(),
        };
        let mut catalog = IntegrationCatalog::new();
        catalog.upsert_provider(ProviderConfig {
            provider_id: "existing-search".to_owned(),
            connector_name: "existing-search".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_id".to_owned(), "existing-search".to_owned()),
                (
                    PLUGIN_SLOT_CLAIMS_METADATA_KEY.to_owned(),
                    "[{\"slot\":\"provider:web_search\",\"key\":\"tavily\",\"mode\":\"exclusive\"}]"
                        .to_owned(),
                ),
                (
                    "plugin_source_path".to_owned(),
                    "/tmp/existing-search.plugin.json".to_owned(),
                ),
            ]),
        });

        let setup_readiness_context = PluginSetupReadinessContext::default();
        let plan = PluginTranslator::new().plan_activation_with_catalog(
            &translation,
            &matrix,
            &setup_readiness_context,
            Some(&catalog),
        );

        assert_eq!(plan.total_plugins, 1);
        assert_eq!(plan.ready_plugins, 0);
        assert_eq!(plan.blocked_plugins, 1);
        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::BlockedSlotClaimConflict
        ));
        assert!(
            plan.candidates[0]
                .reason
                .contains("existing plugin `existing-search`")
        );
        assert!(
            plan.candidates[0]
                .diagnostic_findings
                .iter()
                .any(|finding| { finding.code == PluginDiagnosticCode::SlotClaimConflict })
        );
    }

    #[test]
    fn activation_plan_blocks_incompatible_host_before_bridge_checks() {
        let mut descriptor = descriptor(
            "js",
            BTreeMap::from([("bridge_kind".to_owned(), "mcp_server".to_owned())]),
        );
        descriptor.manifest.compatibility = Some(PluginCompatibility {
            host_api: Some("loong-plugin/v999".to_owned()),
            host_version_req: None,
        });

        let translation = PluginTranslator::new().translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });
        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
            supported_compatibility_modes: BTreeSet::from([PluginCompatibilityMode::Native]),
            supported_compatibility_shims: BTreeSet::new(),
            supported_compatibility_shim_profiles: BTreeMap::new(),
        };

        let setup_readiness_context = verified_setup_readiness_context();
        let plan = PluginTranslator::new().plan_activation(
            &translation,
            &matrix,
            &setup_readiness_context,
        );

        assert_eq!(plan.total_plugins, 1);
        assert_eq!(plan.ready_plugins, 0);
        assert_eq!(plan.blocked_plugins, 1);
        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::BlockedIncompatibleHost
        ));
        assert!(
            plan.candidates[0].reason.contains(CURRENT_PLUGIN_HOST_API),
            "compatibility reason should mention the supported host api"
        );
        assert!(
            plan.candidates[0]
                .diagnostic_findings
                .iter()
                .any(|finding| { finding.code == PluginDiagnosticCode::IncompatibleHost })
        );
    }

    #[test]
    fn activation_plan_projects_inventory_entries_with_activation_truth() {
        let descriptor = descriptor(
            "manifest",
            BTreeMap::from([("bridge_kind".to_owned(), "http_json".to_owned())]),
        );
        let translator = PluginTranslator::new();
        let translation = translator.translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });
        let setup_readiness_context = verified_setup_readiness_context();
        let plan = translator.plan_activation(
            &translation,
            &BridgeSupportMatrix::default(),
            &setup_readiness_context,
        );

        let inventory = plan.inventory_entries(&translation);

        assert_eq!(inventory.len(), 1);
        assert_eq!(inventory[0].plugin_id, "sample-manifest");
        assert_eq!(
            inventory[0].manifest_api_version.as_deref(),
            Some("v1alpha1")
        );
        assert_eq!(inventory[0].plugin_version.as_deref(), Some("1.0.0"));
        assert_eq!(
            inventory[0].dialect,
            PluginContractDialect::LoongPackageManifest
        );
        assert_eq!(
            inventory[0].compatibility_mode,
            PluginCompatibilityMode::Native
        );
        assert_eq!(inventory[0].provider_id, "sample-provider");
        assert_eq!(inventory[0].connector_name, "sample-connector");
        assert_eq!(inventory[0].bridge_kind, PluginBridgeKind::HttpJson);
        assert_eq!(inventory[0].source_language, "manifest");
        assert_eq!(
            inventory[0]
                .activation_status
                .map(|status| status.as_str().to_owned()),
            Some("ready".to_owned())
        );
        assert!(
            inventory[0]
                .activation_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("runtime profile"))
        );
        assert!(inventory[0].bootstrap_hint.is_some());
        assert!(inventory[0].diagnostic_findings.is_empty());
    }

    #[test]
    fn activation_plan_blocker_summary_includes_specific_plugin_reasons() {
        let mut first = descriptor(
            "js",
            BTreeMap::from([("bridge_kind".to_owned(), "http_json".to_owned())]),
        );
        first.manifest.plugin_id = "search-a".to_owned();
        first.manifest.provider_id = "search-a".to_owned();
        first.manifest.connector_name = "search-a".to_owned();
        first.path = "/tmp/search-a.js".to_owned();
        first.manifest.slot_claims = vec![PluginSlotClaim {
            slot: "provider:web_search".to_owned(),
            key: "default".to_owned(),
            mode: PluginSlotMode::Exclusive,
        }];

        let mut second = descriptor(
            "ts",
            BTreeMap::from([("bridge_kind".to_owned(), "http_json".to_owned())]),
        );
        second.manifest.plugin_id = "search-b".to_owned();
        second.manifest.provider_id = "search-b".to_owned();
        second.manifest.connector_name = "search-b".to_owned();
        second.path = "/tmp/search-b.ts".to_owned();
        second.manifest.slot_claims = vec![PluginSlotClaim {
            slot: "provider:web_search".to_owned(),
            key: "default".to_owned(),
            mode: PluginSlotMode::Exclusive,
        }];

        let translation = PluginTranslator::new().translate_scan_report(&PluginScanReport {
            scanned_files: 2,
            matched_plugins: 2,
            diagnostic_findings: Vec::new(),
            descriptors: vec![first, second],
        });
        let setup_readiness_context = PluginSetupReadinessContext::default();
        let plan = PluginTranslator::new().plan_activation(
            &translation,
            &BridgeSupportMatrix::default(),
            &setup_readiness_context,
        );

        let summary = plan.blocker_summary(1);

        assert!(summary.contains("search-a") || summary.contains("search-b"));
        assert!(summary.contains("blocked_slot_claim_conflict"));
        assert!(summary.contains("provider:web_search"));
        assert!(summary.contains("+1 more blocked plugin(s)"));
    }

    #[test]
    fn activation_plan_blocks_unsupported_compatibility_mode() {
        let mut descriptor = descriptor(
            "js",
            BTreeMap::from([("bridge_kind".to_owned(), "process_stdio".to_owned())]),
        );
        descriptor.dialect = PluginContractDialect::OpenClawModernManifest;
        descriptor.dialect_version = Some("openclaw.plugin.json".to_owned());
        descriptor.compatibility_mode = PluginCompatibilityMode::OpenClawModern;

        let translation = PluginTranslator::new().translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });
        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
            supported_adapter_families: BTreeSet::new(),
            supported_compatibility_modes: BTreeSet::from([PluginCompatibilityMode::Native]),
            supported_compatibility_shims: BTreeSet::new(),
            supported_compatibility_shim_profiles: BTreeMap::new(),
        };

        let setup_readiness_context = verified_setup_readiness_context();
        let plan = PluginTranslator::new().plan_activation(
            &translation,
            &matrix,
            &setup_readiness_context,
        );

        assert_eq!(plan.blocked_plugins, 1);
        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::BlockedCompatibilityMode
        ));
        assert_eq!(
            plan.candidates[0].compatibility_mode,
            PluginCompatibilityMode::OpenClawModern
        );
        assert_eq!(
            plan.candidates[0]
                .compatibility_shim
                .as_ref()
                .map(|shim| shim.shim_id.as_str()),
            Some("openclaw-modern-compat")
        );
        assert!(plan.candidates[0].reason.contains("openclaw-modern-compat"));
        assert!(
            plan.candidates[0]
                .bootstrap_hint
                .contains("enable compatibility shim `openclaw-modern-compat`")
        );
        assert!(
            plan.candidates[0]
                .diagnostic_findings
                .iter()
                .any(|finding| {
                    finding.code == PluginDiagnosticCode::CompatibilityShimRequired
                        && finding.blocking
                })
        );
    }

    #[test]
    fn activation_plan_allows_supported_compatibility_mode() {
        let mut descriptor = descriptor(
            "js",
            BTreeMap::from([("bridge_kind".to_owned(), "process_stdio".to_owned())]),
        );
        descriptor.dialect = PluginContractDialect::OpenClawModernManifest;
        descriptor.dialect_version = Some("openclaw.plugin.json".to_owned());
        descriptor.compatibility_mode = PluginCompatibilityMode::OpenClawModern;

        let translation = PluginTranslator::new().translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });
        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
            supported_adapter_families: BTreeSet::new(),
            supported_compatibility_modes: BTreeSet::from([
                PluginCompatibilityMode::Native,
                PluginCompatibilityMode::OpenClawModern,
            ]),
            supported_compatibility_shims: BTreeSet::new(),
            supported_compatibility_shim_profiles: BTreeMap::new(),
        };

        let setup_readiness_context = verified_setup_readiness_context();
        let plan = PluginTranslator::new().plan_activation(
            &translation,
            &matrix,
            &setup_readiness_context,
        );

        assert_eq!(plan.ready_plugins, 0);
        assert_eq!(plan.blocked_plugins, 1);
        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::BlockedCompatibilityMode
        ));
        assert_eq!(
            plan.candidates[0]
                .compatibility_shim
                .as_ref()
                .map(|shim| shim.shim_id.as_str()),
            Some("openclaw-modern-compat")
        );
        assert!(
            plan.candidates[0]
                .reason
                .contains("requires compatibility shim `openclaw-modern-compat`")
        );
        assert!(
            plan.candidates[0]
                .bootstrap_hint
                .contains("enable compatibility shim `openclaw-modern-compat`")
        );
    }

    #[test]
    fn activation_plan_allows_supported_compatibility_mode_when_shim_is_enabled() {
        let mut descriptor = descriptor(
            "js",
            BTreeMap::from([("bridge_kind".to_owned(), "process_stdio".to_owned())]),
        );
        descriptor.dialect = PluginContractDialect::OpenClawModernManifest;
        descriptor.dialect_version = Some("openclaw.plugin.json".to_owned());
        descriptor.compatibility_mode = PluginCompatibilityMode::OpenClawModern;

        let translation = PluginTranslator::new().translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });
        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
            supported_adapter_families: BTreeSet::new(),
            supported_compatibility_modes: BTreeSet::from([
                PluginCompatibilityMode::Native,
                PluginCompatibilityMode::OpenClawModern,
            ]),
            supported_compatibility_shims: BTreeSet::from([PluginCompatibilityShim {
                shim_id: "openclaw-modern-compat".to_owned(),
                family: "openclaw-modern-compat".to_owned(),
            }]),
            supported_compatibility_shim_profiles: BTreeMap::new(),
        };

        let setup_readiness_context = verified_setup_readiness_context();
        let plan = PluginTranslator::new().plan_activation(
            &translation,
            &matrix,
            &setup_readiness_context,
        );

        assert_eq!(plan.ready_plugins, 1);
        assert_eq!(plan.blocked_plugins, 0);
        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::Ready
        ));
    }

    #[test]
    fn activation_plan_blocks_enabled_shim_profile_when_runtime_projection_mismatches() {
        let mut descriptor = descriptor(
            "js",
            BTreeMap::from([("bridge_kind".to_owned(), "process_stdio".to_owned())]),
        );
        descriptor.dialect = PluginContractDialect::OpenClawModernManifest;
        descriptor.dialect_version = Some("openclaw.plugin.json".to_owned());
        descriptor.compatibility_mode = PluginCompatibilityMode::OpenClawModern;

        let translation = PluginTranslator::new().translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });
        let shim = PluginCompatibilityShim {
            shim_id: "openclaw-modern-compat".to_owned(),
            family: "openclaw-modern-compat".to_owned(),
        };
        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
            supported_adapter_families: BTreeSet::new(),
            supported_compatibility_modes: BTreeSet::from([
                PluginCompatibilityMode::Native,
                PluginCompatibilityMode::OpenClawModern,
            ]),
            supported_compatibility_shims: BTreeSet::new(),
            supported_compatibility_shim_profiles: BTreeMap::from([(
                shim.clone(),
                PluginCompatibilityShimSupport {
                    shim,
                    version: Some("openclaw-modern@1".to_owned()),
                    supported_dialects: BTreeSet::from([
                        PluginContractDialect::OpenClawModernManifest,
                    ]),
                    supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
                    supported_adapter_families: BTreeSet::new(),
                    supported_source_languages: BTreeSet::from(["python".to_owned()]),
                },
            )]),
        };

        let setup_readiness_context = verified_setup_readiness_context();
        let plan = PluginTranslator::new().plan_activation(
            &translation,
            &matrix,
            &setup_readiness_context,
        );

        assert_eq!(plan.ready_plugins, 0);
        assert_eq!(plan.blocked_plugins, 1);
        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::BlockedCompatibilityMode
        ));
        assert_eq!(
            plan.candidates[0]
                .compatibility_shim_support
                .as_ref()
                .and_then(|support| support.version.as_deref()),
            Some("openclaw-modern@1")
        );
        assert_eq!(
            plan.candidates[0].compatibility_shim_support_mismatch_reasons,
            vec!["source language `javascript`".to_owned()]
        );
        assert!(
            plan.candidates[0]
                .reason
                .contains("source language `javascript`")
        );
        assert!(plan.candidates[0].reason.contains("openclaw-modern@1"));
    }

    #[test]
    fn activation_plan_allows_enabled_shim_profile_when_runtime_projection_matches() {
        let mut descriptor = descriptor(
            "js",
            BTreeMap::from([("bridge_kind".to_owned(), "process_stdio".to_owned())]),
        );
        descriptor.dialect = PluginContractDialect::OpenClawModernManifest;
        descriptor.dialect_version = Some("openclaw.plugin.json".to_owned());
        descriptor.compatibility_mode = PluginCompatibilityMode::OpenClawModern;

        let translation = PluginTranslator::new().translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });
        let shim = PluginCompatibilityShim {
            shim_id: "openclaw-modern-compat".to_owned(),
            family: "openclaw-modern-compat".to_owned(),
        };
        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
            supported_adapter_families: BTreeSet::new(),
            supported_compatibility_modes: BTreeSet::from([
                PluginCompatibilityMode::Native,
                PluginCompatibilityMode::OpenClawModern,
            ]),
            supported_compatibility_shims: BTreeSet::new(),
            supported_compatibility_shim_profiles: BTreeMap::from([(
                shim.clone(),
                PluginCompatibilityShimSupport {
                    shim,
                    version: Some("openclaw-modern@1".to_owned()),
                    supported_dialects: BTreeSet::from([
                        PluginContractDialect::OpenClawModernManifest,
                    ]),
                    supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
                    supported_adapter_families: BTreeSet::new(),
                    supported_source_languages: BTreeSet::from(["javascript".to_owned()]),
                },
            )]),
        };

        let setup_readiness_context = verified_setup_readiness_context();
        let plan = PluginTranslator::new().plan_activation(
            &translation,
            &matrix,
            &setup_readiness_context,
        );

        assert_eq!(plan.ready_plugins, 1);
        assert_eq!(plan.blocked_plugins, 0);
        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::Ready
        ));
        assert_eq!(
            plan.candidates[0]
                .compatibility_shim_support
                .as_ref()
                .and_then(|support| support.version.as_deref()),
            Some("openclaw-modern@1")
        );
        assert!(
            plan.candidates[0]
                .compatibility_shim_support_mismatch_reasons
                .is_empty()
        );
    }

    #[test]
    fn activation_plan_normalizes_source_language_before_shim_profile_match() {
        let mut descriptor = descriptor(
            "JavaScript",
            BTreeMap::from([("bridge_kind".to_owned(), "process_stdio".to_owned())]),
        );
        descriptor.dialect = PluginContractDialect::OpenClawModernManifest;
        descriptor.dialect_version = Some("openclaw.plugin.json".to_owned());
        descriptor.compatibility_mode = PluginCompatibilityMode::OpenClawModern;

        let translation = PluginTranslator::new().translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });
        let shim = PluginCompatibilityShim {
            shim_id: "openclaw-modern-compat".to_owned(),
            family: "openclaw-modern-compat".to_owned(),
        };
        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
            supported_adapter_families: BTreeSet::new(),
            supported_compatibility_modes: BTreeSet::from([
                PluginCompatibilityMode::Native,
                PluginCompatibilityMode::OpenClawModern,
            ]),
            supported_compatibility_shims: BTreeSet::new(),
            supported_compatibility_shim_profiles: BTreeMap::from([(
                shim.clone(),
                PluginCompatibilityShimSupport {
                    shim,
                    version: Some("openclaw-modern@1".to_owned()),
                    supported_dialects: BTreeSet::from([
                        PluginContractDialect::OpenClawModernManifest,
                    ]),
                    supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
                    supported_adapter_families: BTreeSet::new(),
                    supported_source_languages: BTreeSet::from(["javascript".to_owned()]),
                },
            )]),
        };

        let setup_readiness_context = verified_setup_readiness_context();
        let plan = PluginTranslator::new().plan_activation(
            &translation,
            &matrix,
            &setup_readiness_context,
        );

        assert_eq!(plan.ready_plugins, 1);
        assert_eq!(plan.blocked_plugins, 0);
        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::Ready
        ));
        assert!(
            plan.candidates[0]
                .compatibility_shim_support_mismatch_reasons
                .is_empty()
        );
    }

    #[test]
    fn activation_plan_marks_plugin_ready_when_setup_requirements_are_verified() {
        let descriptor = descriptor("manifest", BTreeMap::new());
        let translator = PluginTranslator::new();
        let translation = translator.translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
            ..BridgeSupportMatrix::default()
        };
        let setup_readiness_context = PluginSetupReadinessContext {
            verified_env_vars: BTreeSet::from(["TAVILY_API_KEY".to_owned()]),
            verified_config_keys: BTreeSet::from(["tools.web_search.default_provider".to_owned()]),
        };
        let plan = translator.plan_activation(&translation, &matrix, &setup_readiness_context);

        assert_eq!(plan.total_plugins, 1);
        assert_eq!(plan.ready_plugins, 1);
        assert_eq!(plan.setup_incomplete_plugins, 0);
        assert_eq!(plan.blocked_plugins, 0);
        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::Ready
        ));
        assert!(plan.candidates[0].missing_required_env_vars.is_empty());
        assert!(plan.candidates[0].missing_required_config_keys.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn activation_plan_matches_verified_env_vars_case_insensitively_on_windows() {
        let descriptor = descriptor("manifest", BTreeMap::new());
        let translator = PluginTranslator::new();
        let translation = translator.translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
            ..BridgeSupportMatrix::default()
        };
        let setup_readiness_context = PluginSetupReadinessContext {
            verified_env_vars: BTreeSet::from(["tavily_api_key".to_owned()]),
            verified_config_keys: BTreeSet::from(["tools.web_search.default_provider".to_owned()]),
        };
        let plan = translator.plan_activation(&translation, &matrix, &setup_readiness_context);

        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::Ready
        ));
    }

    #[cfg(not(windows))]
    #[test]
    fn activation_plan_keeps_verified_env_vars_case_sensitive_off_windows() {
        let descriptor = descriptor("manifest", BTreeMap::new());
        let translator = PluginTranslator::new();
        let translation = translator.translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
            ..BridgeSupportMatrix::default()
        };
        let setup_readiness_context = PluginSetupReadinessContext {
            verified_env_vars: BTreeSet::from(["tavily_api_key".to_owned()]),
            verified_config_keys: BTreeSet::from(["tools.web_search.default_provider".to_owned()]),
        };
        let plan = translator.plan_activation(&translation, &matrix, &setup_readiness_context);

        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::SetupIncomplete
        ));
    }

    #[test]
    fn activation_plan_deserializes_old_payload_without_new_readiness_fields() {
        let raw = r#"
{
  "total_plugins": 1,
  "ready_plugins": 0,
  "blocked_plugins": 1,
  "candidates": [
    {
      "plugin_id": "legacy-plugin",
      "source_path": "/tmp/legacy-plugin.py",
      "source_kind": "embedded_source",
      "package_root": "/tmp",
      "package_manifest_path": null,
      "bridge_kind": "http_json",
      "adapter_family": "web-search",
      "status": "blocked_unsupported_bridge",
      "reason": "legacy payload",
      "bootstrap_hint": "skip"
    }
  ]
}
"#;

        let plan: PluginActivationPlan =
            serde_json::from_str(raw).expect("legacy activation payload should deserialize");

        assert_eq!(plan.setup_incomplete_plugins, 0);
        assert!(plan.candidates[0].missing_required_env_vars.is_empty());
        assert!(plan.candidates[0].missing_required_config_keys.is_empty());
    }

    #[test]
    fn activation_plan_still_blocks_unsupported_bridge_before_setup_readiness() {
        let descriptor = descriptor(
            "js",
            BTreeMap::from([("bridge_kind".to_owned(), "mcp_server".to_owned())]),
        );
        let translator = PluginTranslator::new();
        let translation = translator.translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
            ..BridgeSupportMatrix::default()
        };
        let setup_readiness_context = PluginSetupReadinessContext {
            verified_env_vars: BTreeSet::from(["TAVILY_API_KEY".to_owned()]),
            verified_config_keys: BTreeSet::from(["tools.web_search.default_provider".to_owned()]),
        };
        let plan = translator.plan_activation(&translation, &matrix, &setup_readiness_context);

        assert_eq!(plan.ready_plugins, 0);
        assert_eq!(plan.setup_incomplete_plugins, 0);
        assert_eq!(plan.blocked_plugins, 1);
        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::BlockedUnsupportedBridge
        ));
    }

    #[test]
    fn blocker_summary_excludes_setup_incomplete_candidates() {
        let plan = PluginActivationPlan {
            total_plugins: 2,
            ready_plugins: 0,
            setup_incomplete_plugins: 1,
            blocked_plugins: 1,
            candidates: vec![
                PluginActivationCandidate {
                    plugin_id: "setup-plugin".to_owned(),
                    source_path: "/tmp/setup-plugin.py".to_owned(),
                    source_kind: PluginSourceKind::EmbeddedSource,
                    package_root: "/tmp".to_owned(),
                    package_manifest_path: None,
                    trust_tier: PluginTrustTier::Unverified,
                    compatibility_mode: PluginCompatibilityMode::Native,
                    compatibility_shim: None,
                    compatibility_shim_support: None,
                    compatibility_shim_support_mismatch_reasons: Vec::new(),
                    bridge_kind: PluginBridgeKind::HttpJson,
                    adapter_family: "http-adapter".to_owned(),
                    slot_claims: Vec::new(),
                    diagnostic_findings: Vec::new(),
                    status: PluginActivationStatus::SetupIncomplete,
                    reason: "missing TAVILY_API_KEY".to_owned(),
                    missing_required_env_vars: vec!["TAVILY_API_KEY".to_owned()],
                    missing_required_config_keys: Vec::new(),
                    bootstrap_hint: "export TAVILY_API_KEY".to_owned(),
                },
                PluginActivationCandidate {
                    plugin_id: "blocked-plugin".to_owned(),
                    source_path: "/tmp/blocked-plugin.py".to_owned(),
                    source_kind: PluginSourceKind::EmbeddedSource,
                    package_root: "/tmp".to_owned(),
                    package_manifest_path: None,
                    trust_tier: PluginTrustTier::Unverified,
                    compatibility_mode: PluginCompatibilityMode::Native,
                    compatibility_shim: None,
                    compatibility_shim_support: None,
                    compatibility_shim_support_mismatch_reasons: Vec::new(),
                    bridge_kind: PluginBridgeKind::HttpJson,
                    adapter_family: "http-adapter".to_owned(),
                    slot_claims: Vec::new(),
                    diagnostic_findings: Vec::new(),
                    status: PluginActivationStatus::BlockedUnsupportedBridge,
                    reason: "http_json bridge is disabled".to_owned(),
                    missing_required_env_vars: Vec::new(),
                    missing_required_config_keys: Vec::new(),
                    bootstrap_hint: "enable http bridge".to_owned(),
                },
            ],
        };

        let summary = plan.blocker_summary(4);

        assert!(summary.contains("blocked-plugin"));
        assert!(!summary.contains("setup-plugin"));
    }
}
