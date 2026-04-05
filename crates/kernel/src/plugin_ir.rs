use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    contracts::Capability,
    plugin::{
        PluginDescriptor, PluginManifest, PluginOwnershipSlot, PluginOwnershipSlotMode,
        PluginScanReport, PluginSetup, PluginSourceKind,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginBridgeKind {
    HttpJson,
    ProcessStdio,
    NativeFfi,
    WasmComponent,
    McpServer,
    AcpBridge,
    AcpRuntime,
    Unknown,
}

impl PluginBridgeKind {
    #[must_use]
    pub fn parse_label(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "http_json" | "http" => Some(Self::HttpJson),
            "process_stdio" | "stdio" => Some(Self::ProcessStdio),
            "native_ffi" | "ffi" => Some(Self::NativeFfi),
            "wasm_component" | "wasm" => Some(Self::WasmComponent),
            "mcp_server" | "mcp" => Some(Self::McpServer),
            "acp_bridge" | "acp" => Some(Self::AcpBridge),
            "acp_runtime" | "acpx" => Some(Self::AcpRuntime),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRuntimeProfile {
    pub source_language: String,
    pub bridge_kind: PluginBridgeKind,
    pub adapter_family: String,
    pub entrypoint_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginIR {
    pub plugin_id: String,
    pub provider_id: String,
    pub connector_name: String,
    pub channel_id: Option<String>,
    pub endpoint: Option<String>,
    pub capabilities: BTreeSet<Capability>,
    pub metadata: BTreeMap<String, String>,
    pub source_path: String,
    pub source_kind: PluginSourceKind,
    pub package_root: String,
    pub package_manifest_path: Option<String>,
    pub setup: Option<PluginSetup>,
    pub slots: Vec<PluginOwnershipSlot>,
    pub runtime: PluginRuntimeProfile,
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
    BlockedUnsupportedBridge,
    BlockedUnsupportedAdapterFamily,
    BlockedInvalidOwnershipSlots,
    BlockedOwnershipConflict,
}

/// Captures activation planning details for a single plugin candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginActivationCandidate {
    pub plugin_id: String,
    pub source_path: String,
    pub source_kind: PluginSourceKind,
    pub package_root: String,
    pub package_manifest_path: Option<String>,
    pub bridge_kind: PluginBridgeKind,
    pub adapter_family: String,
    pub status: PluginActivationStatus,
    pub reason: String,
    #[serde(default)]
    pub missing_required_env_vars: Vec<String>,
    #[serde(default)]
    pub missing_required_config_keys: Vec<String>,
    #[serde(default)]
    pub slot_claims: Vec<String>,
    #[serde(default)]
    pub conflicting_slot_claims: Vec<String>,
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

impl PluginActivationPlan {
    #[must_use]
    pub fn has_blockers(&self) -> bool {
        self.blocked_plugins > 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeSupportMatrix {
    pub supported_bridges: BTreeSet<PluginBridgeKind>,
    pub supported_adapter_families: BTreeSet<String>,
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

        for descriptor in &report.descriptors {
            let ir = self.translate_descriptor(descriptor);
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

        PluginIR {
            plugin_id: descriptor.manifest.plugin_id.clone(),
            provider_id: descriptor.manifest.provider_id.clone(),
            connector_name: descriptor.manifest.connector_name.clone(),
            channel_id: descriptor.manifest.channel_id.clone(),
            endpoint: descriptor.manifest.endpoint.clone(),
            capabilities: descriptor.manifest.capabilities.clone(),
            metadata: descriptor.manifest.metadata.clone(),
            source_path: descriptor.path.clone(),
            source_kind: descriptor.source_kind,
            package_root: descriptor.package_root.clone(),
            package_manifest_path: descriptor.package_manifest_path.clone(),
            setup: descriptor.manifest.setup.clone(),
            slots: descriptor.manifest.slots.clone(),
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
        let mut plan = PluginActivationPlan::default();
        let slot_conflict_state = evaluate_slot_conflicts(&translation.entries);

        for ir in &translation.entries {
            plan.total_plugins = plan.total_plugins.saturating_add(1);

            let setup_readiness =
                evaluate_plugin_setup_readiness(ir.setup.as_ref(), setup_readiness_context);
            let setup_is_incomplete = !setup_readiness.ready;
            let plugin_key = activation_plugin_key(ir);
            let slot_claims = slot_claim_strings(&ir.slots);
            let conflicting_slot_claims = slot_conflict_state
                .conflicting_slot_claims_by_plugin
                .get(&plugin_key)
                .cloned()
                .unwrap_or_default();
            let invalid_slot_reasons = slot_conflict_state
                .invalid_reasons_by_plugin
                .get(&plugin_key)
                .cloned()
                .unwrap_or_default();

            let (status, reason) = if !invalid_slot_reasons.is_empty() {
                (
                    PluginActivationStatus::BlockedInvalidOwnershipSlots,
                    format!(
                        "plugin ownership slots are invalid: {}",
                        invalid_slot_reasons.join("; ")
                    ),
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
            } else if !conflicting_slot_claims.is_empty() {
                (
                    PluginActivationStatus::BlockedOwnershipConflict,
                    format!(
                        "plugin ownership slot conflict on {}",
                        conflicting_slot_claims.join(", ")
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

            match status {
                PluginActivationStatus::Ready => {
                    plan.ready_plugins = plan.ready_plugins.saturating_add(1)
                }
                PluginActivationStatus::SetupIncomplete => {
                    plan.setup_incomplete_plugins = plan.setup_incomplete_plugins.saturating_add(1)
                }
                PluginActivationStatus::BlockedUnsupportedBridge
                | PluginActivationStatus::BlockedUnsupportedAdapterFamily
                | PluginActivationStatus::BlockedInvalidOwnershipSlots
                | PluginActivationStatus::BlockedOwnershipConflict => {
                    plan.blocked_plugins = plan.blocked_plugins.saturating_add(1)
                }
            }

            plan.candidates.push(PluginActivationCandidate {
                plugin_id: ir.plugin_id.clone(),
                source_path: ir.source_path.clone(),
                source_kind: ir.source_kind,
                package_root: ir.package_root.clone(),
                package_manifest_path: ir.package_manifest_path.clone(),
                bridge_kind: ir.runtime.bridge_kind,
                adapter_family: ir.runtime.adapter_family.clone(),
                status,
                reason,
                missing_required_env_vars: setup_readiness.missing_required_env_vars,
                missing_required_config_keys: setup_readiness.missing_required_config_keys,
                slot_claims,
                conflicting_slot_claims,
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

type ActivationPluginKey = (String, String);

#[derive(Debug, Default)]
struct SlotConflictState {
    invalid_reasons_by_plugin: BTreeMap<ActivationPluginKey, Vec<String>>,
    conflicting_slot_claims_by_plugin: BTreeMap<ActivationPluginKey, Vec<String>>,
}

fn activation_plugin_key(ir: &PluginIR) -> ActivationPluginKey {
    (ir.source_path.clone(), ir.plugin_id.clone())
}

fn slot_claim_strings(slots: &[PluginOwnershipSlot]) -> Vec<String> {
    slots
        .iter()
        .cloned()
        .map(PluginOwnershipSlot::normalized)
        .map(|slot| slot.canonical_label())
        .collect()
}

fn evaluate_slot_conflicts(entries: &[PluginIR]) -> SlotConflictState {
    let mut state = SlotConflictState::default();
    let mut claimants_by_surface: BTreeMap<
        (String, String),
        Vec<(ActivationPluginKey, PluginOwnershipSlotMode)>,
    > = BTreeMap::new();

    for ir in entries {
        let plugin_key = activation_plugin_key(ir);

        for raw_slot in &ir.slots {
            let slot = raw_slot.clone().normalized();
            if !slot.is_valid() {
                let reason = format_invalid_slot_reason(&slot);
                let reasons = state
                    .invalid_reasons_by_plugin
                    .entry(plugin_key.clone())
                    .or_default();
                if !reasons.iter().any(|existing| existing == &reason) {
                    reasons.push(reason);
                }
                continue;
            }

            if matches!(slot.mode, PluginOwnershipSlotMode::Advisory) {
                continue;
            }

            claimants_by_surface
                .entry((slot.slot.clone(), slot.key.clone()))
                .or_default()
                .push((plugin_key.clone(), slot.mode));
        }
    }

    for ((slot, key), claimants) in claimants_by_surface {
        if !slot_surface_has_conflict(&claimants) {
            continue;
        }

        for (plugin_key, mode) in claimants {
            let conflicting_claims = state
                .conflicting_slot_claims_by_plugin
                .entry(plugin_key)
                .or_default();
            let label = format!("{slot}#{key}@{}", mode.as_str());
            if !conflicting_claims.iter().any(|existing| existing == &label) {
                conflicting_claims.push(label);
            }
        }
    }

    state
}

fn slot_surface_has_conflict(claimants: &[(ActivationPluginKey, PluginOwnershipSlotMode)]) -> bool {
    if claimants.len() <= 1 {
        return false;
    }

    claimants
        .iter()
        .any(|(_, mode)| matches!(mode, PluginOwnershipSlotMode::Exclusive))
}

fn format_invalid_slot_reason(slot: &PluginOwnershipSlot) -> String {
    match (slot.slot.is_empty(), slot.key.is_empty()) {
        (true, true) => format!(
            "slot claim `{}` is missing both slot and key",
            slot.canonical_label()
        ),
        (true, false) => format!("slot claim `{}` is missing slot", slot.canonical_label()),
        (false, true) => format!("slot claim `{}` is missing key", slot.canonical_label()),
        (false, false) => format!("slot claim `{}` is invalid", slot.canonical_label()),
    }
}

fn infer_runtime_profile(language: &str, manifest: &PluginManifest) -> PluginRuntimeProfile {
    let source_language = normalize_language(language);

    let bridge_kind = manifest
        .metadata
        .get("bridge_kind")
        .and_then(|value| parse_bridge_kind(value))
        .or_else(|| {
            manifest
                .metadata
                .get("protocol")
                .filter(|value| value.eq_ignore_ascii_case("mcp"))
                .map(|_| PluginBridgeKind::McpServer)
        })
        .unwrap_or_else(|| default_bridge_kind(&source_language, manifest.endpoint.as_deref()));

    let adapter_family = manifest
        .metadata
        .get("adapter_family")
        .cloned()
        .unwrap_or_else(|| default_adapter_family(&source_language, bridge_kind));

    let entrypoint_hint = manifest
        .metadata
        .get("entrypoint")
        .cloned()
        .or_else(|| default_entrypoint_hint(bridge_kind, manifest.endpoint.as_deref()))
        .unwrap_or_else(|| "invoke".to_owned());

    PluginRuntimeProfile {
        source_language,
        bridge_kind,
        adapter_family,
        entrypoint_hint,
    }
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
    match ir.runtime.bridge_kind {
        PluginBridgeKind::HttpJson => format!(
            "register http connector adapter for {} at {}",
            ir.connector_name,
            ir.endpoint.as_deref().unwrap_or("https://localhost/invoke")
        ),
        PluginBridgeKind::ProcessStdio => format!(
            "spawn {} worker and bind stdio bridge {}",
            ir.runtime.source_language, ir.runtime.entrypoint_hint
        ),
        PluginBridgeKind::NativeFfi => format!(
            "load native library adapter {} with symbol {}",
            ir.runtime.adapter_family, ir.runtime.entrypoint_hint
        ),
        PluginBridgeKind::WasmComponent => {
            format!(
                "load wasm component and invoke {}",
                ir.runtime.entrypoint_hint
            )
        }
        PluginBridgeKind::McpServer => {
            "register MCP server bridge and handshake capability schema".to_owned()
        }
        PluginBridgeKind::AcpBridge => {
            "register ACP bridge surface and bind the external gateway/runtime contract".to_owned()
        }
        PluginBridgeKind::AcpRuntime => {
            "register ACP runtime backend and bind a session-aware control plane".to_owned()
        }
        PluginBridgeKind::Unknown => {
            "inspect plugin metadata and define explicit bridge_kind override".to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::{
        PluginManifest, PluginOwnershipSlot, PluginOwnershipSlotMode, PluginSetup, PluginSetupMode,
        PluginSourceKind,
    };

    fn descriptor(language: &str, metadata: BTreeMap<String, String>) -> PluginDescriptor {
        let source_kind = if language == "manifest" {
            PluginSourceKind::PackageManifest
        } else {
            PluginSourceKind::EmbeddedSource
        };
        let path = if language == "manifest" {
            "/tmp/loongclaw.plugin.json".to_owned()
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
            package_root: "/tmp".to_owned(),
            package_manifest_path,
            language: language.to_owned(),
            manifest: PluginManifest {
                plugin_id: format!("sample-{language}"),
                provider_id: "sample-provider".to_owned(),
                connector_name: "sample-connector".to_owned(),
                channel_id: Some("primary".to_owned()),
                endpoint: Some("https://example.com/invoke".to_owned()),
                capabilities: BTreeSet::from([Capability::InvokeConnector]),
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
                slots: Vec::new(),
            },
        }
    }

    fn slot_claim(slot: &str, key: &str, mode: PluginOwnershipSlotMode) -> PluginOwnershipSlot {
        PluginOwnershipSlot {
            slot: slot.to_owned(),
            key: key.to_owned(),
            mode,
        }
    }

    #[test]
    fn translator_infers_bridge_from_source_language() {
        let scanner_report = PluginScanReport {
            scanned_files: 2,
            matched_plugins: 2,
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
    fn translator_defaults_manifest_descriptor_with_endpoint_to_http_json() {
        let descriptor = descriptor("manifest", BTreeMap::new());

        let translator = PluginTranslator::new();
        let ir = translator.translate_descriptor(&descriptor);

        assert_eq!(ir.runtime.source_language, "manifest");
        assert_eq!(ir.runtime.bridge_kind, PluginBridgeKind::HttpJson);
        assert_eq!(ir.runtime.adapter_family, "http-adapter");
        assert_eq!(ir.source_kind, PluginSourceKind::PackageManifest);
        assert_eq!(ir.package_root, "/tmp");
        assert_eq!(
            ir.setup.as_ref().and_then(|setup| setup.surface.as_deref()),
            Some("web_search")
        );
        assert_eq!(
            ir.package_manifest_path,
            Some("/tmp/loongclaw.plugin.json".to_owned())
        );
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
    fn activation_plan_marks_setup_incomplete_when_required_setup_is_missing() {
        let descriptor = descriptor(
            "js",
            BTreeMap::from([("bridge_kind".to_owned(), "http_json".to_owned())]),
        );
        let translator = PluginTranslator::new();
        let translation = translator.translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            descriptors: vec![descriptor],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
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
    fn activation_plan_blocks_invalid_slot_declarations() {
        let mut descriptor = descriptor("manifest", BTreeMap::new());
        descriptor.path = "/tmp/invalid/loongclaw.plugin.json".to_owned();
        descriptor.package_root = "/tmp/invalid".to_owned();
        descriptor.package_manifest_path = Some(descriptor.path.clone());
        descriptor.manifest.slots = vec![slot_claim(
            "provider:web_search",
            " ",
            PluginOwnershipSlotMode::Exclusive,
        )];

        let translator = PluginTranslator::new();
        let translation = translator.translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            descriptors: vec![descriptor],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
        };
        let setup_readiness_context = PluginSetupReadinessContext {
            verified_env_vars: BTreeSet::from(["TAVILY_API_KEY".to_owned()]),
            verified_config_keys: BTreeSet::from(["tools.web_search.default_provider".to_owned()]),
        };
        let plan = translator.plan_activation(&translation, &matrix, &setup_readiness_context);

        assert_eq!(plan.ready_plugins, 0);
        assert_eq!(plan.blocked_plugins, 1);
        assert!(matches!(
            plan.candidates[0].status,
            PluginActivationStatus::BlockedInvalidOwnershipSlots
        ));
        assert!(plan.candidates[0].reason.contains("missing key"));
        assert_eq!(
            plan.candidates[0].slot_claims,
            vec!["provider:web_search#@exclusive".to_owned()]
        );
    }

    #[test]
    fn activation_plan_blocks_exclusive_slot_conflicts() {
        let mut first = descriptor("manifest", BTreeMap::new());
        first.path = "/tmp/tavily/loongclaw.plugin.json".to_owned();
        first.package_root = "/tmp/tavily".to_owned();
        first.package_manifest_path = Some(first.path.clone());
        first.manifest.plugin_id = "tavily-search".to_owned();
        first.manifest.provider_id = "tavily".to_owned();
        first.manifest.connector_name = "tavily-http".to_owned();
        first.manifest.slots = vec![slot_claim(
            "provider:web_search",
            "default",
            PluginOwnershipSlotMode::Exclusive,
        )];

        let mut second = descriptor("manifest", BTreeMap::new());
        second.path = "/tmp/serper/loongclaw.plugin.json".to_owned();
        second.package_root = "/tmp/serper".to_owned();
        second.package_manifest_path = Some(second.path.clone());
        second.manifest.plugin_id = "serper-search".to_owned();
        second.manifest.provider_id = "serper".to_owned();
        second.manifest.connector_name = "serper-http".to_owned();
        second.manifest.endpoint = Some("https://example.com/serper".to_owned());
        second.manifest.slots = vec![slot_claim(
            "provider:web_search",
            "default",
            PluginOwnershipSlotMode::Exclusive,
        )];

        let translator = PluginTranslator::new();
        let translation = translator.translate_scan_report(&PluginScanReport {
            scanned_files: 2,
            matched_plugins: 2,
            descriptors: vec![first, second],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
        };
        let setup_readiness_context = PluginSetupReadinessContext {
            verified_env_vars: BTreeSet::from(["TAVILY_API_KEY".to_owned()]),
            verified_config_keys: BTreeSet::from(["tools.web_search.default_provider".to_owned()]),
        };
        let plan = translator.plan_activation(&translation, &matrix, &setup_readiness_context);

        assert_eq!(plan.ready_plugins, 0);
        assert_eq!(plan.blocked_plugins, 2);
        assert!(plan.candidates.iter().all(|candidate| matches!(
            candidate.status,
            PluginActivationStatus::BlockedOwnershipConflict
        )));
        assert!(plan.candidates.iter().all(|candidate| {
            candidate
                .conflicting_slot_claims
                .contains(&"provider:web_search#default@exclusive".to_owned())
        }));
    }

    #[test]
    fn activation_plan_allows_shared_slot_claims_to_coexist() {
        let mut first = descriptor("manifest", BTreeMap::new());
        first.path = "/tmp/tavily/loongclaw.plugin.json".to_owned();
        first.package_root = "/tmp/tavily".to_owned();
        first.package_manifest_path = Some(first.path.clone());
        first.manifest.plugin_id = "tavily-search".to_owned();
        first.manifest.provider_id = "tavily".to_owned();
        first.manifest.connector_name = "tavily-http".to_owned();
        first.manifest.slots = vec![slot_claim(
            "tool:search",
            "web",
            PluginOwnershipSlotMode::Shared,
        )];

        let mut second = descriptor("manifest", BTreeMap::new());
        second.path = "/tmp/openrouter/loongclaw.plugin.json".to_owned();
        second.package_root = "/tmp/openrouter".to_owned();
        second.package_manifest_path = Some(second.path.clone());
        second.manifest.plugin_id = "openrouter-search".to_owned();
        second.manifest.provider_id = "openrouter".to_owned();
        second.manifest.connector_name = "openrouter-search".to_owned();
        second.manifest.endpoint = Some("https://example.com/openrouter".to_owned());
        second.manifest.slots = vec![slot_claim(
            "tool:search",
            "web",
            PluginOwnershipSlotMode::Shared,
        )];

        let translator = PluginTranslator::new();
        let translation = translator.translate_scan_report(&PluginScanReport {
            scanned_files: 2,
            matched_plugins: 2,
            descriptors: vec![first, second],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
        };
        let setup_readiness_context = PluginSetupReadinessContext {
            verified_env_vars: BTreeSet::from(["TAVILY_API_KEY".to_owned()]),
            verified_config_keys: BTreeSet::from(["tools.web_search.default_provider".to_owned()]),
        };
        let plan = translator.plan_activation(&translation, &matrix, &setup_readiness_context);

        assert_eq!(plan.ready_plugins, 2);
        assert_eq!(plan.blocked_plugins, 0);
        assert!(
            plan.candidates
                .iter()
                .all(|candidate| matches!(candidate.status, PluginActivationStatus::Ready))
        );
        assert!(
            plan.candidates
                .iter()
                .all(|candidate| candidate.conflicting_slot_claims.is_empty())
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
            descriptors: vec![descriptor],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
            supported_adapter_families: BTreeSet::from(["rust-stdio-adapter".to_owned()]),
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
    }

    #[test]
    fn activation_plan_marks_plugin_ready_when_setup_requirements_are_verified() {
        let descriptor = descriptor("manifest", BTreeMap::new());
        let translator = PluginTranslator::new();
        let translation = translator.translate_scan_report(&PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            descriptors: vec![descriptor],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
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
            descriptors: vec![descriptor],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
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
            descriptors: vec![descriptor],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
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
            descriptors: vec![descriptor],
        });

        let matrix = BridgeSupportMatrix {
            supported_bridges: BTreeSet::from([PluginBridgeKind::HttpJson]),
            supported_adapter_families: BTreeSet::new(),
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
}
