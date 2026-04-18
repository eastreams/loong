use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    contracts::Capability,
    errors::IntegrationError,
    integration::{AutoProvisionRequest, ChannelConfig, IntegrationCatalog, ProviderConfig},
    pack::VerticalPackManifest,
};

pub const PACKAGE_MANIFEST_FILE_NAME: &str = "loong.plugin.json";
const OPENCLAW_PACKAGE_MANIFEST_FILE_NAME: &str = "openclaw.plugin.json";
const PACKAGE_JSON_FILE_NAME: &str = "package.json";
const OPENCLAW_MODERN_COMPATIBILITY_ADAPTER_FAMILY: &str = "openclaw-modern-compat";
const OPENCLAW_LEGACY_COMPATIBILITY_ADAPTER_FAMILY: &str = "openclaw-legacy-compat";
pub const CURRENT_PLUGIN_MANIFEST_API_VERSION: &str = "v1alpha1";
pub const CURRENT_PLUGIN_HOST_API: &str = "loong-plugin/v1";
const RESERVED_PACKAGE_METADATA_PREFIX: &str = "plugin_";
pub(crate) const PLUGIN_MANIFEST_API_VERSION_METADATA_KEY: &str = "plugin_manifest_api_version";
pub(crate) const PLUGIN_VERSION_METADATA_KEY: &str = "plugin_version";
pub(crate) const PLUGIN_DIALECT_METADATA_KEY: &str = "plugin_dialect";
pub(crate) const PLUGIN_DIALECT_VERSION_METADATA_KEY: &str = "plugin_dialect_version";
pub(crate) const PLUGIN_COMPATIBILITY_MODE_METADATA_KEY: &str = "plugin_compatibility_mode";
pub(crate) const PLUGIN_COMPATIBILITY_SHIM_ID_METADATA_KEY: &str = "plugin_compatibility_shim_id";
pub(crate) const PLUGIN_COMPATIBILITY_SHIM_FAMILY_METADATA_KEY: &str =
    "plugin_compatibility_shim_family";
pub(crate) const PLUGIN_SLOT_CLAIMS_METADATA_KEY: &str = "plugin_slot_claims_json";
pub(crate) const PLUGIN_COMPATIBILITY_HOST_API_METADATA_KEY: &str = "plugin_compatibility_host_api";
pub(crate) const PLUGIN_COMPATIBILITY_HOST_VERSION_REQ_METADATA_KEY: &str =
    "plugin_compatibility_host_version_req";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PluginTrustTier {
    Official,
    #[serde(alias = "verified_community")]
    VerifiedCommunity,
    #[default]
    Unverified,
}

impl PluginTrustTier {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::VerifiedCommunity => "verified-community",
            Self::Unverified => "unverified",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PluginSetupMode {
    #[default]
    MetadataOnly,
    GovernedEntry,
}

impl PluginSetupMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MetadataOnly => "metadata_only",
            Self::GovernedEntry => "governed_entry",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PluginSetup {
    #[serde(default)]
    pub mode: PluginSetupMode,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub required_env_vars: Vec<String>,
    #[serde(default)]
    pub recommended_env_vars: Vec<String>,
    #[serde(default)]
    pub required_config_keys: Vec<String>,
    #[serde(default)]
    pub default_env_var: Option<String>,
    #[serde(default)]
    pub docs_urls: Vec<String>,
    #[serde(default)]
    pub remediation: Option<String>,
}

impl PluginSetup {
    #[must_use]
    pub fn normalized(self) -> Self {
        let mode = self.mode;
        let surface = normalize_optional_manifest_string(self.surface);
        let required_env_vars = normalize_manifest_string_list(self.required_env_vars);
        let recommended_env_vars = normalize_manifest_string_list(self.recommended_env_vars);
        let required_config_keys = normalize_manifest_string_list(self.required_config_keys);
        let default_env_var = normalize_optional_manifest_string(self.default_env_var);
        let docs_urls = normalize_manifest_string_list(self.docs_urls);
        let remediation = normalize_optional_manifest_string(self.remediation);

        Self {
            mode,
            surface,
            required_env_vars,
            recommended_env_vars,
            required_config_keys,
            default_env_var,
            docs_urls,
            remediation,
        }
    }

    #[must_use]
    pub fn is_effectively_empty(&self) -> bool {
        let has_surface = self.surface.is_some();
        let has_required_env_vars = !self.required_env_vars.is_empty();
        let has_recommended_env_vars = !self.recommended_env_vars.is_empty();
        let has_required_config_keys = !self.required_config_keys.is_empty();
        let has_default_env_var = self.default_env_var.is_some();
        let has_docs_urls = !self.docs_urls.is_empty();
        let has_remediation = self.remediation.is_some();
        let has_non_default_payload = has_surface
            || has_required_env_vars
            || has_recommended_env_vars
            || has_required_config_keys
            || has_default_env_var
            || has_docs_urls
            || has_remediation;

        if has_non_default_payload {
            return false;
        }

        matches!(self.mode, PluginSetupMode::MetadataOnly)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PluginSlotMode {
    #[default]
    Exclusive,
    Shared,
    Advisory,
}

impl PluginSlotMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exclusive => "exclusive",
            Self::Shared => "shared",
            Self::Advisory => "advisory",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSlotClaim {
    pub slot: String,
    pub key: String,
    pub mode: PluginSlotMode,
}

impl PluginSlotClaim {
    #[must_use]
    pub fn normalized(self) -> Self {
        Self {
            slot: self.slot.trim().to_owned(),
            key: self.key.trim().to_owned(),
            mode: self.mode,
        }
    }

    #[must_use]
    pub fn canonical_label(&self) -> String {
        format!("{}#{}@{}", self.slot, self.key, self.mode.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PluginCompatibility {
    #[serde(default)]
    pub host_api: Option<String>,
    #[serde(default)]
    pub host_version_req: Option<String>,
}

impl PluginCompatibility {
    #[must_use]
    pub fn normalized(self) -> Self {
        Self {
            host_api: normalize_optional_manifest_string(self.host_api),
            host_version_req: normalize_optional_manifest_string(self.host_version_req),
        }
    }

    #[must_use]
    pub fn is_effectively_empty(&self) -> bool {
        self.host_api.is_none() && self.host_version_req.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    #[serde(default)]
    pub api_version: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    pub plugin_id: String,
    pub provider_id: String,
    pub connector_name: String,
    pub channel_id: Option<String>,
    pub endpoint: Option<String>,
    pub capabilities: BTreeSet<Capability>,
    #[serde(default)]
    pub trust_tier: PluginTrustTier,
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub input_examples: Vec<Value>,
    #[serde(default)]
    pub output_examples: Vec<Value>,
    #[serde(default)]
    pub defer_loading: bool,
    #[serde(default)]
    pub setup: Option<PluginSetup>,
    #[serde(default)]
    pub slot_claims: Vec<PluginSlotClaim>,
    #[serde(default)]
    pub compatibility: Option<PluginCompatibility>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginSourceKind {
    PackageManifest,
    EmbeddedSource,
}

impl PluginSourceKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PackageManifest => "package_manifest",
            Self::EmbeddedSource => "embedded_source",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PluginContractDialect {
    #[default]
    LoongPackageManifest,
    LoongEmbeddedSource,
    OpenClawModernManifest,
    OpenClawLegacyPackage,
}

impl PluginContractDialect {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LoongPackageManifest => "loong_package_manifest",
            Self::LoongEmbeddedSource => "loong_embedded_source",
            Self::OpenClawModernManifest => "openclaw_modern_manifest",
            Self::OpenClawLegacyPackage => "openclaw_legacy_package",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PluginCompatibilityMode {
    #[default]
    Native,
    OpenClawModern,
    OpenClawLegacy,
}

impl PluginCompatibilityMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::OpenClawModern => "openclaw_modern",
            Self::OpenClawLegacy => "openclaw_legacy",
        }
    }

    #[must_use]
    pub const fn is_native(self) -> bool {
        matches!(self, Self::Native)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PluginCompatibilityShim {
    pub shim_id: String,
    pub family: String,
}

impl PluginCompatibilityShim {
    #[must_use]
    pub fn for_mode(mode: PluginCompatibilityMode) -> Option<Self> {
        match mode {
            PluginCompatibilityMode::Native => None,
            PluginCompatibilityMode::OpenClawModern => Some(Self {
                shim_id: OPENCLAW_MODERN_COMPATIBILITY_ADAPTER_FAMILY.to_owned(),
                family: OPENCLAW_MODERN_COMPATIBILITY_ADAPTER_FAMILY.to_owned(),
            }),
            PluginCompatibilityMode::OpenClawLegacy => Some(Self {
                shim_id: OPENCLAW_LEGACY_COMPATIBILITY_ADAPTER_FAMILY.to_owned(),
                family: OPENCLAW_LEGACY_COMPATIBILITY_ADAPTER_FAMILY.to_owned(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

impl PluginDiagnosticSeverity {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PluginDiagnosticPhase {
    #[default]
    Unknown,
    Scan,
    Translation,
    Activation,
}

impl PluginDiagnosticPhase {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Scan => "scan",
            Self::Translation => "translation",
            Self::Activation => "activation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginDiagnosticCode {
    EmbeddedSourceLegacyContract,
    LegacyMetadataVersion,
    ShadowedEmbeddedSource,
    ForeignDialectContract,
    LegacyOpenClawContract,
    InvalidManifestContract,
    CompatibilityShimRequired,
    IncompatibleHost,
    UnsupportedBridge,
    UnsupportedAdapterFamily,
    SlotClaimConflict,
}

impl PluginDiagnosticCode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EmbeddedSourceLegacyContract => "embedded_source_legacy_contract",
            Self::LegacyMetadataVersion => "legacy_metadata_version",
            Self::ShadowedEmbeddedSource => "shadowed_embedded_source",
            Self::ForeignDialectContract => "foreign_dialect_contract",
            Self::LegacyOpenClawContract => "legacy_openclaw_contract",
            Self::InvalidManifestContract => "invalid_manifest_contract",
            Self::CompatibilityShimRequired => "compatibility_shim_required",
            Self::IncompatibleHost => "incompatible_host",
            Self::UnsupportedBridge => "unsupported_bridge",
            Self::UnsupportedAdapterFamily => "unsupported_adapter_family",
            Self::SlotClaimConflict => "slot_claim_conflict",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDiagnosticFinding {
    pub code: PluginDiagnosticCode,
    pub severity: PluginDiagnosticSeverity,
    #[serde(default)]
    pub phase: PluginDiagnosticPhase,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub source_kind: Option<PluginSourceKind>,
    #[serde(default)]
    pub field_path: Option<String>,
    pub message: String,
    #[serde(default)]
    pub remediation: Option<String>,
}

impl PluginDiagnosticFinding {
    #[must_use]
    pub fn matches_plugin(&self, source_path: &str, plugin_id: &str) -> bool {
        self.source_path.as_deref() == Some(source_path)
            && self.plugin_id.as_deref() == Some(plugin_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub path: String,
    pub source_kind: PluginSourceKind,
    pub dialect: PluginContractDialect,
    pub dialect_version: Option<String>,
    pub compatibility_mode: PluginCompatibilityMode,
    pub package_root: String,
    pub package_manifest_path: Option<String>,
    pub language: String,
    pub manifest: PluginManifest,
}

#[must_use]
pub fn format_plugin_provenance_summary(
    source_kind: PluginSourceKind,
    source_path: &str,
    package_manifest_path: Option<&str>,
) -> String {
    if let Some(package_manifest_path) = package_manifest_path
        && !matches!(source_kind, PluginSourceKind::PackageManifest)
    {
        return format!(
            "{}:{} (package_manifest:{package_manifest_path})",
            source_kind.as_str(),
            source_path
        );
    }

    format!("{}:{source_path}", source_kind.as_str())
}

#[must_use]
pub fn plugin_provenance_summary_for_descriptor(descriptor: &PluginDescriptor) -> String {
    format_plugin_provenance_summary(
        descriptor.source_kind,
        &descriptor.path,
        descriptor.package_manifest_path.as_deref(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginScanReport {
    pub scanned_files: usize,
    pub matched_plugins: usize,
    #[serde(default)]
    pub diagnostic_findings: Vec<PluginDiagnosticFinding>,
    pub descriptors: Vec<PluginDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginAbsorbReport {
    pub absorbed_plugins: usize,
    pub provider_upserts: usize,
    pub channel_upserts: usize,
    pub connectors_added_to_pack: BTreeSet<String>,
    pub capabilities_added_to_pack: BTreeSet<Capability>,
}

#[derive(Debug, Default)]
pub struct PluginScanner;

impl PluginScanner {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    pub fn scan_path<P: AsRef<Path>>(&self, root: P) -> Result<PluginScanReport, IntegrationError> {
        let root = root.as_ref();
        if !root.exists() {
            return Err(IntegrationError::PluginScanRootNotFound(
                root.display().to_string(),
            ));
        }

        let mut report = PluginScanReport::default();
        let mut files = Vec::new();
        collect_files(root, &mut files)?;
        files.sort();
        report.scanned_files = files.len();

        let package_manifest_descriptors = collect_package_manifest_descriptors(&files)?;
        let source_manifest_collection = collect_source_manifest_descriptors(&files)?;
        report
            .diagnostic_findings
            .extend(source_manifest_collection.diagnostic_findings.clone());
        for descriptor in package_manifest_descriptors.values() {
            report
                .diagnostic_findings
                .extend(descriptor_contract_diagnostic_findings(descriptor));
        }
        let source_manifest_descriptors = source_manifest_collection.descriptors;
        let package_manifests_by_root =
            collect_package_manifest_descriptors_by_root(&package_manifest_descriptors);

        validate_package_manifest_conflicts(
            &package_manifests_by_root,
            &source_manifest_descriptors,
        )?;

        for (source_path, source_descriptor) in &source_manifest_descriptors {
            let Some(package_descriptor) =
                find_covering_package_manifest_descriptor(source_path, &package_manifests_by_root)
            else {
                continue;
            };

            report
                .diagnostic_findings
                .push(shadowed_embedded_source_finding(
                    source_descriptor,
                    package_descriptor,
                ));
        }

        for path in &files {
            if let Some(descriptor) = package_manifest_descriptors.get(path) {
                push_descriptor(&mut report, descriptor.clone());
                continue;
            }

            let covering_package_manifest =
                find_covering_package_manifest_descriptor(path, &package_manifests_by_root);

            if covering_package_manifest.is_some() {
                continue;
            }

            if let Some(descriptor) = source_manifest_descriptors.get(path) {
                push_descriptor(&mut report, descriptor.clone());
            }
        }

        Ok(report)
    }

    /// Absorb plugin descriptors into the catalog and pack manifest.
    ///
    /// Uses clone-and-restore rollback: if any operation fails partway through,
    /// both `catalog` and `pack` are restored to their pre-absorb state so
    /// callers never observe a partially-mutated configuration.
    pub fn absorb(
        &self,
        catalog: &mut IntegrationCatalog,
        pack: &mut VerticalPackManifest,
        report: &PluginScanReport,
    ) -> Result<PluginAbsorbReport, IntegrationError> {
        let catalog_snapshot = catalog.clone();
        let pack_snapshot = pack.clone();

        let result = self.absorb_inner(catalog, pack, report);

        if result.is_err() {
            *catalog = catalog_snapshot;
            *pack = pack_snapshot;
        }

        result
    }

    fn absorb_inner(
        &self,
        catalog: &mut IntegrationCatalog,
        pack: &mut VerticalPackManifest,
        report: &PluginScanReport,
    ) -> Result<PluginAbsorbReport, IntegrationError> {
        let mut absorbed = PluginAbsorbReport::default();
        let mut claimed_slots = collect_claimed_slots(catalog)?;

        for descriptor in &report.descriptors {
            let manifest = &descriptor.manifest;

            if manifest.provider_id.is_empty() {
                return Err(IntegrationError::PluginAbsorbFailed {
                    plugin_id: manifest.plugin_id.clone(),
                    reason: "provider_id must not be empty".to_owned(),
                });
            }

            if manifest.connector_name.is_empty() {
                return Err(IntegrationError::PluginAbsorbFailed {
                    plugin_id: manifest.plugin_id.clone(),
                    reason: "connector_name must not be empty".to_owned(),
                });
            }

            validate_plugin_slot_claims(manifest)?;
            validate_plugin_host_compatibility(manifest)?;
            register_plugin_slot_claims(manifest, &mut claimed_slots)?;

            let mut provider_metadata = manifest.metadata.clone();
            stamp_plugin_manifest_contract_metadata(&mut provider_metadata, manifest);
            stamp_plugin_descriptor_contract_metadata(&mut provider_metadata, descriptor);
            stamp_plugin_slot_claims_metadata(&mut provider_metadata, &manifest.slot_claims)?;
            stamp_plugin_compatibility_metadata(
                &mut provider_metadata,
                manifest.compatibility.as_ref(),
            );
            catalog.upsert_provider(ProviderConfig {
                provider_id: manifest.provider_id.clone(),
                connector_name: manifest.connector_name.clone(),
                version: manifest
                    .version
                    .clone()
                    .or_else(|| manifest.metadata.get("version").cloned())
                    .unwrap_or_else(|| "0.1.0".to_owned()),
                metadata: provider_metadata,
            });
            absorbed.provider_upserts = absorbed.provider_upserts.saturating_add(1);

            if let Some(channel_id) = &manifest.channel_id {
                catalog.upsert_channel(ChannelConfig {
                    channel_id: channel_id.clone(),
                    provider_id: manifest.provider_id.clone(),
                    endpoint: manifest.endpoint.clone().unwrap_or_else(|| {
                        format!("https://{}.local/{channel_id}/invoke", manifest.provider_id)
                    }),
                    enabled: true,
                    metadata: BTreeMap::from([(
                        "source_plugin".to_owned(),
                        manifest.plugin_id.clone(),
                    )]),
                });
                absorbed.channel_upserts = absorbed.channel_upserts.saturating_add(1);
            }

            if pack
                .allowed_connectors
                .insert(manifest.connector_name.clone())
            {
                absorbed
                    .connectors_added_to_pack
                    .insert(manifest.connector_name.clone());
            }

            if pack
                .granted_capabilities
                .insert(Capability::InvokeConnector)
            {
                absorbed
                    .capabilities_added_to_pack
                    .insert(Capability::InvokeConnector);
            }

            for capability in &manifest.capabilities {
                if pack.granted_capabilities.insert(*capability) {
                    absorbed.capabilities_added_to_pack.insert(*capability);
                }
            }

            absorbed.absorbed_plugins = absorbed.absorbed_plugins.saturating_add(1);
        }

        Ok(absorbed)
    }

    #[must_use]
    pub fn to_auto_provision_requests(
        &self,
        report: &PluginScanReport,
    ) -> Vec<AutoProvisionRequest> {
        report
            .descriptors
            .iter()
            .map(|descriptor| AutoProvisionRequest {
                provider_id: descriptor.manifest.provider_id.clone(),
                channel_id: descriptor
                    .manifest
                    .channel_id
                    .clone()
                    .unwrap_or_else(|| format!("{}-default", descriptor.manifest.provider_id)),
                connector_name: Some(descriptor.manifest.connector_name.clone()),
                endpoint: descriptor.manifest.endpoint.clone(),
                required_capabilities: descriptor.manifest.capabilities.clone(),
            })
            .collect()
    }
}

#[derive(Debug, Default)]
struct SourceManifestCollection {
    descriptors: BTreeMap<PathBuf, PluginDescriptor>,
    diagnostic_findings: Vec<PluginDiagnosticFinding>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageManifestDocument {
    #[serde(default)]
    api_version: Option<String>,
    #[serde(default)]
    version: Option<String>,
    plugin_id: String,
    provider_id: String,
    connector_name: String,
    channel_id: Option<String>,
    endpoint: Option<String>,
    capabilities: BTreeSet<Capability>,
    metadata: BTreeMap<String, String>,
    #[serde(default)]
    trust_tier: PluginTrustTier,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    input_examples: Vec<Value>,
    #[serde(default)]
    output_examples: Vec<Value>,
    #[serde(default)]
    defer_loading: bool,
    #[serde(default)]
    setup: Option<PluginSetup>,
    #[serde(default)]
    slot_claims: Vec<PluginSlotClaim>,
    #[serde(default)]
    compatibility: Option<PluginCompatibility>,
}

impl PackageManifestDocument {
    fn into_manifest(self) -> PluginManifest {
        PluginManifest {
            api_version: self.api_version,
            version: self.version,
            plugin_id: self.plugin_id,
            provider_id: self.provider_id,
            connector_name: self.connector_name,
            channel_id: self.channel_id,
            endpoint: self.endpoint,
            capabilities: self.capabilities,
            trust_tier: self.trust_tier,
            metadata: self.metadata,
            summary: self.summary,
            tags: self.tags,
            input_examples: self.input_examples,
            output_examples: self.output_examples,
            defer_loading: self.defer_loading,
            setup: self.setup,
            slot_claims: self.slot_claims,
            compatibility: self.compatibility,
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenClawManifestDocument {
    id: String,
    #[serde(default, rename = "configSchema")]
    config_schema: Option<Value>,
    #[serde(default, rename = "enabledByDefault")]
    enabled_by_default: bool,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    channels: Vec<String>,
    #[serde(default)]
    providers: Vec<String>,
    #[serde(default, rename = "providerAuthEnvVars")]
    provider_auth_env_vars: BTreeMap<String, Vec<String>>,
    #[serde(default, rename = "providerAuthChoices")]
    provider_auth_choices: Vec<Value>,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default, rename = "uiHints")]
    ui_hints: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenClawPackageJsonDocument {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    openclaw: Option<OpenClawPackageMetadataDocument>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenClawPackageMetadataDocument {
    #[serde(default)]
    extensions: Vec<String>,
    #[serde(default, rename = "setupEntry")]
    setup_entry: Option<String>,
    #[serde(default)]
    channel: Option<OpenClawPackageChannelDocument>,
    #[serde(default)]
    install: Option<OpenClawPackageInstallDocument>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenClawPackageChannelDocument {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default, rename = "docsPath")]
    docs_path: Option<String>,
    #[serde(default)]
    blurb: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenClawPackageInstallDocument {
    #[serde(default, rename = "npmSpec")]
    npm_spec: Option<String>,
    #[serde(default, rename = "localPath")]
    local_path: Option<String>,
    #[serde(default, rename = "minHostVersion")]
    min_host_version: Option<String>,
}

fn collect_files(path: &Path, acc: &mut Vec<PathBuf>) -> Result<(), IntegrationError> {
    let metadata = fs::metadata(path).map_err(|error| IntegrationError::PluginFileRead {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;

    if metadata.is_file() {
        acc.push(path.to_path_buf());
        return Ok(());
    }

    for entry in fs::read_dir(path).map_err(|error| IntegrationError::PluginFileRead {
        path: path.display().to_string(),
        reason: error.to_string(),
    })? {
        let entry = entry.map_err(|error| IntegrationError::PluginFileRead {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
        let child = entry.path();
        if child.is_dir() {
            if should_skip_dir(&child) {
                continue;
            }
            collect_files(&child, acc)?;
        } else if child.is_file() {
            acc.push(child);
        }
    }
    Ok(())
}

fn collect_package_manifest_descriptors(
    files: &[PathBuf],
) -> Result<BTreeMap<PathBuf, PluginDescriptor>, IntegrationError> {
    let mut descriptors = BTreeMap::new();
    let known_files = files.iter().cloned().collect::<BTreeSet<_>>();

    for path in files {
        if is_loong_package_manifest_file(path) {
            let descriptor = parse_package_manifest_descriptor(path)?;
            descriptors.insert(path.clone(), descriptor);
            continue;
        }

        if is_openclaw_package_manifest_file(path) {
            let descriptor = parse_openclaw_manifest_descriptor(path)?;
            descriptors.insert(PathBuf::from(descriptor.path.clone()), descriptor);
            continue;
        }

        if is_package_json_file(path) {
            for descriptor in parse_openclaw_legacy_package_descriptors(path, &known_files)? {
                descriptors.insert(PathBuf::from(descriptor.path.clone()), descriptor);
            }
        }
    }

    Ok(descriptors)
}

fn collect_source_manifest_descriptors(
    files: &[PathBuf],
) -> Result<SourceManifestCollection, IntegrationError> {
    let mut collection = SourceManifestCollection::default();

    for path in files {
        let descriptor = parse_source_manifest_descriptor(path)?;
        let Some(descriptor) = descriptor else {
            continue;
        };

        collection
            .diagnostic_findings
            .extend(descriptor_contract_diagnostic_findings(&descriptor));
        collection.descriptors.insert(path.clone(), descriptor);
    }

    Ok(collection)
}

fn collect_package_manifest_descriptors_by_root(
    descriptors: &BTreeMap<PathBuf, PluginDescriptor>,
) -> BTreeMap<PathBuf, PluginDescriptor> {
    let mut manifests_by_root = BTreeMap::new();

    for (path, descriptor) in descriptors {
        let Some(parent) = path.parent() else {
            continue;
        };

        let package_root = parent.to_path_buf();
        let descriptor = descriptor.clone();

        manifests_by_root.insert(package_root, descriptor);
    }

    manifests_by_root
}

fn push_descriptor(report: &mut PluginScanReport, descriptor: PluginDescriptor) {
    report.matched_plugins = report.matched_plugins.saturating_add(1);
    report.descriptors.push(descriptor);
}

fn descriptor_contract_diagnostic_findings(
    descriptor: &PluginDescriptor,
) -> Vec<PluginDiagnosticFinding> {
    let mut findings = Vec::new();

    if matches!(descriptor.source_kind, PluginSourceKind::EmbeddedSource) {
        findings.push(PluginDiagnosticFinding {
            code: PluginDiagnosticCode::EmbeddedSourceLegacyContract,
            severity: PluginDiagnosticSeverity::Warning,
            phase: PluginDiagnosticPhase::Scan,
            blocking: false,
            plugin_id: Some(descriptor.manifest.plugin_id.clone()),
            source_path: Some(descriptor.path.clone()),
            source_kind: Some(descriptor.source_kind),
            field_path: None,
            message:
                "embedded source manifests remain a migration-only contract; package manifests are the preferred public SDK surface"
                    .to_owned(),
            remediation: Some(
                "add a `loong.plugin.json` package manifest and keep source markers only as a temporary compatibility bridge"
                    .to_owned(),
            ),
        });
    }

    if matches!(descriptor.source_kind, PluginSourceKind::EmbeddedSource)
        && descriptor.manifest.metadata.contains_key("version")
    {
        findings.push(PluginDiagnosticFinding {
            code: PluginDiagnosticCode::LegacyMetadataVersion,
            severity: PluginDiagnosticSeverity::Warning,
            phase: PluginDiagnosticPhase::Scan,
            blocking: false,
            plugin_id: Some(descriptor.manifest.plugin_id.clone()),
            source_path: Some(descriptor.path.clone()),
            source_kind: Some(descriptor.source_kind),
            field_path: Some("metadata.version".to_owned()),
            message:
                "embedded source manifest still carries legacy metadata.version; typed top-level version is the stable contract"
                    .to_owned(),
            remediation: Some(
                "move plugin version truth to top-level `version` and remove legacy metadata.version once package manifests are in place"
                    .to_owned(),
            ),
        });
    }

    if !descriptor.compatibility_mode.is_native() {
        findings.push(PluginDiagnosticFinding {
            code: PluginDiagnosticCode::ForeignDialectContract,
            severity: PluginDiagnosticSeverity::Info,
            phase: PluginDiagnosticPhase::Scan,
            blocking: false,
            plugin_id: Some(descriptor.manifest.plugin_id.clone()),
            source_path: Some(descriptor.path.clone()),
            source_kind: Some(descriptor.source_kind),
            field_path: Some("dialect".to_owned()),
            message: format!(
                "plugin contract dialect `{}` is projected through compatibility mode `{}` before native activation",
                descriptor.dialect.as_str(),
                descriptor.compatibility_mode.as_str()
            ),
            remediation: Some(
                "keep compatibility intake on the adapter boundary, or migrate the plugin to a native `loong.plugin.json` contract for first-class SDK support"
                    .to_owned(),
            ),
        });
    }

    if matches!(
        descriptor.compatibility_mode,
        PluginCompatibilityMode::OpenClawLegacy
    ) {
        findings.push(PluginDiagnosticFinding {
            code: PluginDiagnosticCode::LegacyOpenClawContract,
            severity: PluginDiagnosticSeverity::Warning,
            phase: PluginDiagnosticPhase::Scan,
            blocking: false,
            plugin_id: Some(descriptor.manifest.plugin_id.clone()),
            source_path: Some(descriptor.path.clone()),
            source_kind: Some(descriptor.source_kind),
            field_path: Some("package.json#openclaw.extensions".to_owned()),
            message:
                "legacy OpenClaw package metadata remains compatibility-only; modern openclaw.plugin.json manifests are the preferred foreign contract"
                    .to_owned(),
            remediation: Some(
                "add `openclaw.plugin.json` and keep package.json openclaw metadata only for entrypoint/setup declarations during migration"
                    .to_owned(),
            ),
        });
    }

    findings
}

fn shadowed_embedded_source_finding(
    source_descriptor: &PluginDescriptor,
    package_descriptor: &PluginDescriptor,
) -> PluginDiagnosticFinding {
    PluginDiagnosticFinding {
        code: PluginDiagnosticCode::ShadowedEmbeddedSource,
        severity: PluginDiagnosticSeverity::Warning,
        phase: PluginDiagnosticPhase::Scan,
        blocking: false,
        plugin_id: Some(source_descriptor.manifest.plugin_id.clone()),
        source_path: Some(source_descriptor.path.clone()),
        source_kind: Some(source_descriptor.source_kind),
        field_path: None,
        message: format!(
            "embedded source manifest is shadowed by package manifest `{}` and no longer acts as the authoritative contract",
            package_descriptor.path
        ),
        remediation: Some(
            "remove the shadowed marker block or keep it strictly migration-compatible until the package manifest is the sole source of truth"
                .to_owned(),
        ),
    }
}

fn parse_package_manifest_descriptor(path: &Path) -> Result<PluginDescriptor, IntegrationError> {
    let manifest = parse_package_manifest_file(path)?;
    let descriptor = build_plugin_descriptor(
        path,
        PluginSourceKind::PackageManifest,
        PluginContractDialect::LoongPackageManifest,
        Some(CURRENT_PLUGIN_MANIFEST_API_VERSION.to_owned()),
        PluginCompatibilityMode::Native,
        Some(path),
        None,
        manifest,
    );

    Ok(descriptor)
}

fn parse_package_manifest_file(path: &Path) -> Result<PluginManifest, IntegrationError> {
    let bytes = fs::read(path).map_err(|error| IntegrationError::PluginFileRead {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;

    let content =
        String::from_utf8(bytes).map_err(|error| IntegrationError::PluginManifestParse {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;

    let document: PackageManifestDocument =
        serde_json::from_str(content.trim()).map_err(|error| {
            IntegrationError::PluginManifestParse {
                path: path.display().to_string(),
                reason: error.to_string(),
            }
        })?;

    validate_package_manifest_document_contract(&document, path)?;

    let normalized_manifest = normalize_plugin_manifest(document.into_manifest());
    validate_plugin_manifest_contract(
        &normalized_manifest,
        PluginSourceKind::PackageManifest,
        path,
    )?;

    Ok(normalized_manifest)
}

fn parse_openclaw_manifest_descriptor(path: &Path) -> Result<PluginDescriptor, IntegrationError> {
    let document = parse_json_document::<OpenClawManifestDocument>(path)?;
    validate_openclaw_manifest_document(&document, path)?;

    let package_json_path = path
        .parent()
        .map(|parent| parent.join(PACKAGE_JSON_FILE_NAME))
        .filter(|candidate| candidate.is_file());
    let package_document = package_json_path
        .as_deref()
        .map(parse_json_document::<OpenClawPackageJsonDocument>)
        .transpose()?;

    let package_root = path.parent().unwrap_or(path);
    let primary_entry_path =
        resolve_openclaw_primary_entry_path(package_root, package_document.as_ref(), true);
    let setup_entry_path = package_document
        .as_ref()
        .and_then(|package| package.openclaw.as_ref())
        .and_then(|metadata| metadata.setup_entry.as_deref())
        .and_then(|entry| resolve_openclaw_relative_path(package_root, entry));
    let manifest = build_openclaw_manifest(
        &document,
        package_document.as_ref(),
        primary_entry_path.as_deref(),
        setup_entry_path.as_deref(),
        PluginCompatibilityMode::OpenClawModern,
    );
    let descriptor_path = primary_entry_path.as_deref().unwrap_or(path);
    let descriptor = build_plugin_descriptor(
        descriptor_path,
        PluginSourceKind::PackageManifest,
        PluginContractDialect::OpenClawModernManifest,
        Some("openclaw.plugin.json".to_owned()),
        PluginCompatibilityMode::OpenClawModern,
        Some(path),
        primary_entry_path.as_deref(),
        manifest,
    );

    Ok(descriptor)
}

fn parse_openclaw_legacy_package_descriptors(
    path: &Path,
    known_files: &BTreeSet<PathBuf>,
) -> Result<Vec<PluginDescriptor>, IntegrationError> {
    let document = parse_json_document::<OpenClawPackageJsonDocument>(path)?;
    let Some(openclaw) = document.openclaw.as_ref() else {
        return Ok(Vec::new());
    };

    let package_root = path.parent().unwrap_or(path);
    let sibling_openclaw_manifest = package_root.join(OPENCLAW_PACKAGE_MANIFEST_FILE_NAME);
    if known_files.contains(&sibling_openclaw_manifest) || sibling_openclaw_manifest.is_file() {
        return Ok(Vec::new());
    }

    let extension_entries = resolve_openclaw_legacy_extension_entries(package_root, &document);
    if extension_entries.is_empty() {
        return Ok(Vec::new());
    }

    let multiple_entries = extension_entries.len() > 1;
    let setup_entry_path = openclaw
        .setup_entry
        .as_deref()
        .and_then(|entry| resolve_openclaw_relative_path(package_root, entry));
    let mut descriptors = Vec::new();

    for entry_path in extension_entries {
        let plugin_id = derive_openclaw_legacy_plugin_id(
            document.name.as_deref(),
            &entry_path,
            multiple_entries,
        );
        let manifest = build_openclaw_legacy_manifest(
            &document,
            plugin_id,
            &entry_path,
            setup_entry_path.as_deref(),
        );
        descriptors.push(build_plugin_descriptor(
            &entry_path,
            PluginSourceKind::PackageManifest,
            PluginContractDialect::OpenClawLegacyPackage,
            Some("package.json#openclaw".to_owned()),
            PluginCompatibilityMode::OpenClawLegacy,
            Some(path),
            Some(&entry_path),
            manifest,
        ));
    }

    Ok(descriptors)
}

fn parse_json_document<T>(path: &Path) -> Result<T, IntegrationError>
where
    T: for<'de> Deserialize<'de>,
{
    let content = read_utf8_file(path)?;
    serde_json::from_str(content.trim()).map_err(|error| IntegrationError::PluginManifestParse {
        path: path.display().to_string(),
        reason: error.to_string(),
    })
}

fn read_utf8_file(path: &Path) -> Result<String, IntegrationError> {
    let bytes = fs::read(path).map_err(|error| IntegrationError::PluginFileRead {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;

    String::from_utf8(bytes).map_err(|error| IntegrationError::PluginManifestParse {
        path: path.display().to_string(),
        reason: error.to_string(),
    })
}

fn validate_openclaw_manifest_document(
    document: &OpenClawManifestDocument,
    path: &Path,
) -> Result<(), IntegrationError> {
    if document.id.trim().is_empty() {
        return Err(IntegrationError::PluginManifestParse {
            path: path.display().to_string(),
            reason: "openclaw.plugin.json must declare id".to_owned(),
        });
    }

    if !matches!(document.config_schema.as_ref(), Some(Value::Object(_))) {
        return Err(IntegrationError::PluginManifestParse {
            path: path.display().to_string(),
            reason: "openclaw.plugin.json must declare configSchema object".to_owned(),
        });
    }

    Ok(())
}

fn build_openclaw_manifest(
    document: &OpenClawManifestDocument,
    package_document: Option<&OpenClawPackageJsonDocument>,
    primary_entry_path: Option<&Path>,
    setup_entry_path: Option<&Path>,
    compatibility_mode: PluginCompatibilityMode,
) -> PluginManifest {
    let mut metadata = BTreeMap::new();

    metadata.insert("bridge_kind".to_owned(), "process_stdio".to_owned());
    metadata.insert(
        "adapter_family".to_owned(),
        match compatibility_mode {
            PluginCompatibilityMode::Native => "native".to_owned(),
            PluginCompatibilityMode::OpenClawModern => {
                OPENCLAW_MODERN_COMPATIBILITY_ADAPTER_FAMILY.to_owned()
            }
            PluginCompatibilityMode::OpenClawLegacy => {
                OPENCLAW_LEGACY_COMPATIBILITY_ADAPTER_FAMILY.to_owned()
            }
        },
    );

    if let Some(entry) = primary_entry_path {
        metadata.insert("entrypoint".to_owned(), path_to_string(entry));
    }
    if let Some(setup_entry) = setup_entry_path {
        metadata.insert("setup_entrypoint".to_owned(), path_to_string(setup_entry));
    }
    if let Some(kind) = normalize_optional_manifest_string(document.kind.clone()) {
        metadata.insert("openclaw_kind".to_owned(), kind);
    }
    if let Some(package_document) = package_document {
        if let Some(name) = normalize_optional_manifest_string(package_document.name.clone()) {
            metadata.insert("openclaw_package_name".to_owned(), name);
        }
        if let Some(version) = normalize_optional_manifest_string(package_document.version.clone())
        {
            metadata.insert("openclaw_package_version".to_owned(), version);
        }
        if let Some(description) =
            normalize_optional_manifest_string(package_document.description.clone())
        {
            metadata.insert("openclaw_package_description".to_owned(), description);
        }
        if let Some(channel) = package_document
            .openclaw
            .as_ref()
            .and_then(|openclaw| openclaw.channel.as_ref())
        {
            if let Some(channel_id) = normalize_optional_manifest_string(channel.id.clone()) {
                metadata.insert("openclaw_channel_id".to_owned(), channel_id);
            }
            if let Some(label) = normalize_optional_manifest_string(channel.label.clone()) {
                metadata.insert("openclaw_channel_label".to_owned(), label);
            }
            if let Some(blurb) = normalize_optional_manifest_string(channel.blurb.clone()) {
                metadata.insert("openclaw_channel_blurb".to_owned(), blurb);
            }
            if let Some(docs_path) = normalize_optional_manifest_string(channel.docs_path.clone()) {
                metadata.insert("openclaw_channel_docs_path".to_owned(), docs_path);
            }
            let aliases = normalize_manifest_string_list(channel.aliases.clone());
            if !aliases.is_empty()
                && let Ok(encoded) = serde_json::to_string(&aliases)
            {
                metadata.insert("openclaw_channel_aliases_json".to_owned(), encoded);
            }
        }
        if let Some(install) = package_document
            .openclaw
            .as_ref()
            .and_then(|openclaw| openclaw.install.as_ref())
        {
            if let Some(npm_spec) = normalize_optional_manifest_string(install.npm_spec.clone()) {
                metadata.insert("openclaw_install_npm_spec".to_owned(), npm_spec);
            }
            if let Some(local_path) = normalize_optional_manifest_string(install.local_path.clone())
            {
                metadata.insert("openclaw_install_local_path".to_owned(), local_path);
            }
            if let Some(min_host_version) =
                normalize_optional_manifest_string(install.min_host_version.clone())
            {
                metadata.insert(
                    "openclaw_install_min_host_version".to_owned(),
                    min_host_version,
                );
            }
        }
    }

    if !document.channels.is_empty()
        && let Ok(encoded) =
            serde_json::to_string(&normalize_manifest_string_list(document.channels.clone()))
    {
        metadata.insert("openclaw_channels_json".to_owned(), encoded);
    }
    if !document.providers.is_empty()
        && let Ok(encoded) =
            serde_json::to_string(&normalize_manifest_string_list(document.providers.clone()))
    {
        metadata.insert("openclaw_providers_json".to_owned(), encoded);
    }
    if !document.skills.is_empty()
        && let Ok(encoded) =
            serde_json::to_string(&normalize_manifest_string_list(document.skills.clone()))
    {
        metadata.insert("openclaw_skills_json".to_owned(), encoded);
    }
    if !document.provider_auth_env_vars.is_empty()
        && let Ok(encoded) = serde_json::to_string(&document.provider_auth_env_vars)
    {
        metadata.insert("openclaw_provider_auth_env_vars_json".to_owned(), encoded);
    }
    if !document.provider_auth_choices.is_empty()
        && let Ok(encoded) = serde_json::to_string(&document.provider_auth_choices)
    {
        metadata.insert("openclaw_provider_auth_choices_json".to_owned(), encoded);
    }
    if !document.ui_hints.is_empty()
        && let Ok(encoded) = serde_json::to_string(&document.ui_hints)
    {
        metadata.insert("openclaw_ui_hints_json".to_owned(), encoded);
    }
    if document.enabled_by_default {
        metadata.insert("openclaw_enabled_by_default".to_owned(), "true".to_owned());
    }
    if let Some(language) = primary_entry_path
        .map(detect_language)
        .filter(|language| language != "unknown")
    {
        metadata.insert(
            "source_language".to_owned(),
            normalize_language_name(&language),
        );
    }

    normalize_plugin_manifest(PluginManifest {
        api_version: Some(CURRENT_PLUGIN_MANIFEST_API_VERSION.to_owned()),
        version: normalize_optional_manifest_string(document.version.clone()).or_else(|| {
            package_document
                .and_then(|package| normalize_optional_manifest_string(package.version.clone()))
        }),
        plugin_id: document.id.trim().to_owned(),
        provider_id: document.id.trim().to_owned(),
        connector_name: document.id.trim().to_owned(),
        channel_id: None,
        endpoint: None,
        capabilities: derive_openclaw_capabilities(
            document.providers.as_slice(),
            document.channels.as_slice(),
            document.skills.as_slice(),
            document.kind.as_deref(),
        ),
        trust_tier: PluginTrustTier::default(),
        metadata,
        summary: normalize_optional_manifest_string(
            document
                .description
                .clone()
                .or_else(|| document.name.clone()),
        ),
        tags: derive_openclaw_tags(
            compatibility_mode,
            document.providers.as_slice(),
            document.channels.as_slice(),
            document.skills.as_slice(),
            document.kind.as_deref(),
        ),
        input_examples: Vec::new(),
        output_examples: Vec::new(),
        defer_loading: setup_entry_path.is_some(),
        setup: derive_openclaw_setup(document, setup_entry_path),
        slot_claims: derive_openclaw_slot_claims(document.kind.as_deref()),
        compatibility: None,
    })
}

fn build_openclaw_legacy_manifest(
    package_document: &OpenClawPackageJsonDocument,
    plugin_id: String,
    primary_entry_path: &Path,
    setup_entry_path: Option<&Path>,
) -> PluginManifest {
    let synthetic_document = OpenClawManifestDocument {
        id: plugin_id,
        config_schema: Some(Value::Object(Default::default())),
        enabled_by_default: false,
        kind: None,
        channels: Vec::new(),
        providers: Vec::new(),
        provider_auth_env_vars: BTreeMap::new(),
        provider_auth_choices: Vec::new(),
        skills: Vec::new(),
        name: package_document.name.clone(),
        description: package_document.description.clone(),
        version: package_document.version.clone(),
        ui_hints: BTreeMap::new(),
    };

    let mut manifest = build_openclaw_manifest(
        &synthetic_document,
        Some(package_document),
        Some(primary_entry_path),
        setup_entry_path,
        PluginCompatibilityMode::OpenClawLegacy,
    );
    manifest
        .metadata
        .insert("openclaw_legacy_package".to_owned(), "true".to_owned());
    manifest.summary = normalize_optional_manifest_string(
        package_document
            .description
            .clone()
            .or_else(|| package_document.name.clone()),
    );
    manifest
}

fn resolve_openclaw_primary_entry_path(
    package_root: &Path,
    package_document: Option<&OpenClawPackageJsonDocument>,
    prefer_declared_extension: bool,
) -> Option<PathBuf> {
    if prefer_declared_extension && let Some(package_document) = package_document {
        let entries = resolve_openclaw_extension_entries(package_root, package_document);
        if let Some(first) = entries.into_iter().next() {
            return Some(first);
        }
    }

    resolve_openclaw_default_entry_path(package_root)
}

fn resolve_openclaw_legacy_extension_entries(
    package_root: &Path,
    package_document: &OpenClawPackageJsonDocument,
) -> Vec<PathBuf> {
    let declared = resolve_openclaw_extension_entries(package_root, package_document);
    if !declared.is_empty() {
        return declared;
    }

    resolve_openclaw_default_entry_path(package_root)
        .into_iter()
        .collect()
}

fn resolve_openclaw_extension_entries(
    package_root: &Path,
    package_document: &OpenClawPackageJsonDocument,
) -> Vec<PathBuf> {
    package_document
        .openclaw
        .as_ref()
        .map(|metadata| metadata.extensions.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| resolve_openclaw_relative_path(package_root, entry))
        .collect()
}

fn resolve_openclaw_default_entry_path(package_root: &Path) -> Option<PathBuf> {
    for candidate in ["index.ts", "index.js", "index.mjs", "index.cjs"] {
        let entry = package_root.join(candidate);
        if entry.is_file() {
            return Some(entry);
        }
    }

    None
}

fn resolve_openclaw_relative_path(package_root: &Path, raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let candidate = package_root.join(trimmed);
    Some(candidate)
}

fn derive_openclaw_legacy_plugin_id(
    package_name: Option<&str>,
    entry_path: &Path,
    has_multiple_extensions: bool,
) -> String {
    let base = entry_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::trim)
        .filter(|stem| !stem.is_empty())
        .unwrap_or("plugin");

    let Some(package_name) = package_name.map(str::trim).filter(|name| !name.is_empty()) else {
        return base.to_owned();
    };

    let unscoped = package_name.rsplit('/').next().unwrap_or(package_name);
    let canonical = unscoped
        .strip_suffix("-provider")
        .unwrap_or(unscoped)
        .trim();

    if !has_multiple_extensions {
        return canonical.to_owned();
    }

    format!("{canonical}/{base}")
}

fn derive_openclaw_capabilities(
    providers: &[String],
    channels: &[String],
    skills: &[String],
    kind: Option<&str>,
) -> BTreeSet<Capability> {
    let mut capabilities = BTreeSet::new();
    if !providers.is_empty() || !channels.is_empty() {
        capabilities.insert(Capability::InvokeConnector);
    }
    if !skills.is_empty() {
        capabilities.insert(Capability::InvokeTool);
    }

    match kind.map(|value| value.trim().to_ascii_lowercase()) {
        Some(kind) if kind == "memory" => {
            capabilities.insert(Capability::MemoryRead);
            capabilities.insert(Capability::MemoryWrite);
        }
        Some(kind) if kind == "context-engine" => {
            capabilities.insert(Capability::ObserveTelemetry);
        }
        _ => {}
    }

    capabilities
}

fn derive_openclaw_tags(
    compatibility_mode: PluginCompatibilityMode,
    providers: &[String],
    channels: &[String],
    skills: &[String],
    kind: Option<&str>,
) -> Vec<String> {
    let mut tags = vec![
        "openclaw".to_owned(),
        compatibility_mode.as_str().to_owned(),
        "compat".to_owned(),
    ];
    if !providers.is_empty() {
        tags.push("provider".to_owned());
    }
    if !channels.is_empty() {
        tags.push("channel".to_owned());
    }
    if !skills.is_empty() {
        tags.push("skill".to_owned());
    }
    if let Some(kind) = kind.map(str::trim).filter(|kind| !kind.is_empty()) {
        tags.push(kind.to_ascii_lowercase());
    }

    normalize_manifest_string_list(tags)
}

fn derive_openclaw_setup(
    document: &OpenClawManifestDocument,
    setup_entry_path: Option<&Path>,
) -> Option<PluginSetup> {
    let required_env_vars = document
        .provider_auth_env_vars
        .values()
        .flat_map(|values| values.iter().cloned())
        .collect::<Vec<_>>();
    let docs_urls = document
        .provider_auth_choices
        .iter()
        .filter_map(|choice| choice.get("docsUrl"))
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let surface = if !document.channels.is_empty() {
        Some("channel".to_owned())
    } else if !document.providers.is_empty() {
        Some("provider".to_owned())
    } else if !document.skills.is_empty() {
        Some("skill".to_owned())
    } else {
        Some("plugin".to_owned())
    };
    let remediation = Some(
        "enable the required OpenClaw compatibility shim and configure plugin settings before activation"
            .to_owned(),
    );
    let setup = PluginSetup {
        mode: if setup_entry_path.is_some() {
            PluginSetupMode::GovernedEntry
        } else {
            PluginSetupMode::MetadataOnly
        },
        surface,
        required_env_vars: normalize_manifest_string_list(required_env_vars),
        recommended_env_vars: Vec::new(),
        required_config_keys: vec![format!("plugins.entries.{}", document.id.trim())],
        default_env_var: document
            .provider_auth_env_vars
            .values()
            .flat_map(|values| values.iter())
            .next()
            .cloned(),
        docs_urls: normalize_manifest_string_list(docs_urls),
        remediation,
    };

    (!setup.is_effectively_empty()).then_some(setup.normalized())
}

fn derive_openclaw_slot_claims(kind: Option<&str>) -> Vec<PluginSlotClaim> {
    match kind.map(|value| value.trim().to_ascii_lowercase()) {
        Some(kind) if kind == "memory" => vec![PluginSlotClaim {
            slot: "openclaw_kind".to_owned(),
            key: "memory".to_owned(),
            mode: PluginSlotMode::Exclusive,
        }],
        Some(kind) if kind == "context-engine" => vec![PluginSlotClaim {
            slot: "openclaw_kind".to_owned(),
            key: "context_engine".to_owned(),
            mode: PluginSlotMode::Exclusive,
        }],
        _ => Vec::new(),
    }
}

fn validate_package_manifest_document_contract(
    document: &PackageManifestDocument,
    path: &Path,
) -> Result<(), IntegrationError> {
    if normalize_optional_manifest_string(document.version.clone()).is_none() {
        return Err(IntegrationError::PluginManifestParse {
            path: path.display().to_string(),
            reason: "package manifest must declare top-level version".to_owned(),
        });
    }

    if let Some(version) = document
        .metadata
        .get("version")
        .cloned()
        .and_then(|value| normalize_optional_manifest_string(Some(value)))
    {
        return Err(IntegrationError::PluginManifestParse {
            path: path.display().to_string(),
            reason: format!(
                "package manifest must declare version via top-level `version`, not metadata.version (`{version}`)"
            ),
        });
    }

    if let Some(reserved_key) = document
        .metadata
        .keys()
        .find(|key| key.starts_with(RESERVED_PACKAGE_METADATA_PREFIX))
    {
        return Err(IntegrationError::PluginManifestParse {
            path: path.display().to_string(),
            reason: format!(
                "package manifest metadata key `{reserved_key}` is reserved for host-managed projection"
            ),
        });
    }

    Ok(())
}

fn parse_source_manifest_descriptor(
    path: &Path,
) -> Result<Option<PluginDescriptor>, IntegrationError> {
    let bytes = fs::read(path).map_err(|error| IntegrationError::PluginFileRead {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;

    let content = match String::from_utf8(bytes) {
        Ok(content) => content,
        Err(_) => return Ok(None),
    };

    let Some(manifest) = parse_manifest_block(&content, path)? else {
        return Ok(None);
    };

    let descriptor = build_plugin_descriptor(
        path,
        PluginSourceKind::EmbeddedSource,
        PluginContractDialect::LoongEmbeddedSource,
        None,
        PluginCompatibilityMode::Native,
        None,
        None,
        manifest,
    );

    Ok(Some(descriptor))
}

fn build_plugin_descriptor(
    path: &Path,
    source_kind: PluginSourceKind,
    dialect: PluginContractDialect,
    dialect_version: Option<String>,
    compatibility_mode: PluginCompatibilityMode,
    package_manifest_path: Option<&Path>,
    runtime_entry_path: Option<&Path>,
    manifest: PluginManifest,
) -> PluginDescriptor {
    let path_string = path_to_string(path);
    let package_root = package_manifest_path
        .and_then(Path::parent)
        .map(path_to_string)
        .unwrap_or_else(|| package_root_for_path(path));
    let package_manifest_path = package_manifest_path.map(path_to_string);
    let language = runtime_entry_path
        .map(detect_language)
        .unwrap_or_else(|| detect_language(path));

    PluginDescriptor {
        path: path_string,
        source_kind,
        dialect,
        dialect_version,
        compatibility_mode,
        package_root,
        package_manifest_path,
        language,
        manifest,
    }
}

fn package_root_for_path(path: &Path) -> String {
    let package_root = path.parent().unwrap_or(path);

    path_to_string(package_root)
}

fn path_to_string(path: &Path) -> String {
    path.display().to_string()
}

fn is_package_manifest_file(path: &Path) -> bool {
    is_loong_package_manifest_file(path) || is_openclaw_package_manifest_file(path)
}

fn is_loong_package_manifest_file(path: &Path) -> bool {
    let file_name = path.file_name();
    let file_name = file_name.and_then(|value| value.to_str());

    matches!(file_name, Some(PACKAGE_MANIFEST_FILE_NAME))
}

fn is_openclaw_package_manifest_file(path: &Path) -> bool {
    let file_name = path.file_name();
    let file_name = file_name.and_then(|value| value.to_str());

    matches!(file_name, Some(OPENCLAW_PACKAGE_MANIFEST_FILE_NAME))
}

fn is_package_json_file(path: &Path) -> bool {
    let file_name = path.file_name();
    let file_name = file_name.and_then(|value| value.to_str());

    matches!(file_name, Some(PACKAGE_JSON_FILE_NAME))
}

fn find_covering_package_manifest_descriptor<'a>(
    path: &Path,
    package_manifests_by_root: &'a BTreeMap<PathBuf, PluginDescriptor>,
) -> Option<&'a PluginDescriptor> {
    let mut best_match: Option<(&PathBuf, &PluginDescriptor)> = None;

    for (package_root, descriptor) in package_manifests_by_root {
        if !path.starts_with(package_root) {
            continue;
        }

        let candidate_depth = package_root.components().count();
        let Some((best_root, _)) = best_match else {
            best_match = Some((package_root, descriptor));
            continue;
        };

        let best_depth = best_root.components().count();

        if candidate_depth > best_depth {
            best_match = Some((package_root, descriptor));
        }
    }

    best_match.map(|(_, descriptor)| descriptor)
}

fn validate_package_manifest_conflicts(
    package_manifests_by_root: &BTreeMap<PathBuf, PluginDescriptor>,
    source_manifest_descriptors: &BTreeMap<PathBuf, PluginDescriptor>,
) -> Result<(), IntegrationError> {
    for (source_path, source_descriptor) in source_manifest_descriptors {
        let package_descriptor =
            find_covering_package_manifest_descriptor(source_path, package_manifests_by_root);

        let Some(package_descriptor) = package_descriptor else {
            continue;
        };

        validate_package_manifest_pair(package_descriptor, source_descriptor)?;
    }

    Ok(())
}

fn validate_package_manifest_pair(
    package_descriptor: &PluginDescriptor,
    source_descriptor: &PluginDescriptor,
) -> Result<(), IntegrationError> {
    let conflict =
        first_manifest_conflict(&package_descriptor.manifest, &source_descriptor.manifest);

    let Some(conflict) = conflict else {
        return Ok(());
    };

    Err(IntegrationError::PluginManifestConflict {
        package_manifest_path: package_descriptor.path.clone(),
        source_path: source_descriptor.path.clone(),
        field: conflict.field,
        package_value: conflict.package_value,
        source_value: conflict.source_value,
    })
}

fn first_manifest_conflict(
    package_manifest: &PluginManifest,
    source_manifest: &PluginManifest,
) -> Option<ManifestFieldConflict> {
    let plugin_id_conflict = compare_manifest_value(
        "plugin_id",
        &package_manifest.plugin_id,
        &source_manifest.plugin_id,
    );
    if plugin_id_conflict.is_some() {
        return plugin_id_conflict;
    }

    let provider_id_conflict = compare_manifest_value(
        "provider_id",
        &package_manifest.provider_id,
        &source_manifest.provider_id,
    );
    if provider_id_conflict.is_some() {
        return provider_id_conflict;
    }

    let connector_name_conflict = compare_manifest_value(
        "connector_name",
        &package_manifest.connector_name,
        &source_manifest.connector_name,
    );
    if connector_name_conflict.is_some() {
        return connector_name_conflict;
    }

    let channel_id_conflict = compare_manifest_value(
        "channel_id",
        &package_manifest.channel_id,
        &source_manifest.channel_id,
    );
    if channel_id_conflict.is_some() {
        return channel_id_conflict;
    }

    let endpoint_conflict = compare_manifest_value(
        "endpoint",
        &package_manifest.endpoint,
        &source_manifest.endpoint,
    );
    if endpoint_conflict.is_some() {
        return endpoint_conflict;
    }

    let capabilities_conflict = compare_manifest_value(
        "capabilities",
        &package_manifest.capabilities,
        &source_manifest.capabilities,
    );
    if capabilities_conflict.is_some() {
        return capabilities_conflict;
    }

    let metadata_conflict =
        first_shared_metadata_conflict(&package_manifest.metadata, &source_manifest.metadata);
    if metadata_conflict.is_some() {
        return metadata_conflict;
    }

    let summary_conflict = compare_optional_fill_value(
        "summary",
        &package_manifest.summary,
        &source_manifest.summary,
    );
    if summary_conflict.is_some() {
        return summary_conflict;
    }

    let tags_conflict =
        compare_optional_fill_sequence("tags", &package_manifest.tags, &source_manifest.tags);
    if tags_conflict.is_some() {
        return tags_conflict;
    }

    let input_examples_conflict = compare_optional_fill_sequence(
        "input_examples",
        &package_manifest.input_examples,
        &source_manifest.input_examples,
    );
    if input_examples_conflict.is_some() {
        return input_examples_conflict;
    }

    let output_examples_conflict = compare_optional_fill_sequence(
        "output_examples",
        &package_manifest.output_examples,
        &source_manifest.output_examples,
    );
    if output_examples_conflict.is_some() {
        return output_examples_conflict;
    }

    let api_version_conflict = compare_optional_fill_value(
        "api_version",
        &package_manifest.api_version,
        &source_manifest.api_version,
    );
    if api_version_conflict.is_some() {
        return api_version_conflict;
    }

    let version_conflict = compare_optional_fill_value(
        "version",
        &package_manifest.version,
        &source_manifest.version,
    );
    if version_conflict.is_some() {
        return version_conflict;
    }

    let setup_conflict =
        compare_manifest_value("setup", &package_manifest.setup, &source_manifest.setup);
    if setup_conflict.is_some() {
        return setup_conflict;
    }

    let slot_claims_conflict = compare_manifest_value(
        "slot_claims",
        &package_manifest.slot_claims,
        &source_manifest.slot_claims,
    );
    if slot_claims_conflict.is_some() {
        return slot_claims_conflict;
    }

    let compatibility_conflict = compare_optional_fill_value(
        "compatibility",
        &package_manifest.compatibility,
        &source_manifest.compatibility,
    );
    if compatibility_conflict.is_some() {
        return compatibility_conflict;
    }

    compare_manifest_value(
        "defer_loading",
        &package_manifest.defer_loading,
        &source_manifest.defer_loading,
    )
}

fn compare_manifest_value<T>(
    field: &str,
    package_value: &T,
    source_value: &T,
) -> Option<ManifestFieldConflict>
where
    T: ?Sized + PartialEq + Serialize,
{
    if package_value == source_value {
        return None;
    }

    let package_value = serialize_manifest_value(package_value);
    let source_value = serialize_manifest_value(source_value);

    Some(ManifestFieldConflict {
        field: field.to_owned(),
        package_value,
        source_value,
    })
}

fn compare_optional_fill_value<T>(
    field: &str,
    package_value: &Option<T>,
    source_value: &Option<T>,
) -> Option<ManifestFieldConflict>
where
    T: PartialEq + Serialize,
{
    let package_value = package_value.as_ref()?;
    let source_value = source_value.as_ref()?;

    compare_manifest_value(field, package_value, source_value)
}

fn compare_optional_fill_sequence<T>(
    field: &str,
    package_value: &[T],
    source_value: &[T],
) -> Option<ManifestFieldConflict>
where
    T: PartialEq + Serialize,
{
    if package_value.is_empty() {
        return None;
    }

    if source_value.is_empty() {
        return None;
    }

    compare_manifest_value(field, package_value, source_value)
}

fn first_shared_metadata_conflict(
    package_metadata: &BTreeMap<String, String>,
    source_metadata: &BTreeMap<String, String>,
) -> Option<ManifestFieldConflict> {
    for (key, package_value) in package_metadata {
        let Some(source_value) = source_metadata.get(key) else {
            continue;
        };

        if package_value == source_value {
            continue;
        }

        let field = format!("metadata.{key}");
        let package_value = serialize_manifest_value(package_value);
        let source_value = serialize_manifest_value(source_value);

        return Some(ManifestFieldConflict {
            field,
            package_value,
            source_value,
        });
    }

    None
}

fn serialize_manifest_value<T>(value: &T) -> String
where
    T: ?Sized + Serialize,
{
    let serialized = serde_json::to_string(value);

    match serialized {
        Ok(serialized) => serialized,
        Err(error) => format!("\"<serialization_error:{error}>\""),
    }
}

fn should_skip_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".git" | "target" | "node_modules" | ".venv" | ".idea" | ".codex")
    )
}

fn parse_manifest_block(
    content: &str,
    path: &Path,
) -> Result<Option<PluginManifest>, IntegrationError> {
    const START: &str = "LOONG_PLUGIN_START";
    const END: &str = "LOONG_PLUGIN_END";

    let Some(start_idx) = content.find(START) else {
        return Ok(None);
    };

    let Some(end_idx) = content[start_idx..].find(END).map(|idx| start_idx + idx) else {
        return Err(IntegrationError::PluginManifestParse {
            path: path.display().to_string(),
            reason: "missing LOONG_PLUGIN_END".to_owned(),
        });
    };

    let block = &content[start_idx + START.len()..end_idx];
    let cleaned = block
        .lines()
        .map(clean_manifest_line)
        .collect::<Vec<_>>()
        .join("\n");

    let manifest: PluginManifest = serde_json::from_str(cleaned.trim()).map_err(|error| {
        IntegrationError::PluginManifestParse {
            path: path.display().to_string(),
            reason: error.to_string(),
        }
    })?;

    let normalized_manifest = normalize_plugin_manifest(manifest);
    validate_plugin_manifest_contract(
        &normalized_manifest,
        PluginSourceKind::EmbeddedSource,
        path,
    )?;

    Ok(Some(normalized_manifest))
}

fn clean_manifest_line(line: &str) -> String {
    let trimmed = line.trim_start();
    for prefix in ["//", "#", "--", ";", "/*", "*", "*/"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.trim_start().to_owned();
        }
    }
    trimmed.to_owned()
}

fn normalize_plugin_manifest(mut manifest: PluginManifest) -> PluginManifest {
    let normalized_api_version = normalize_optional_manifest_string(manifest.api_version.take());
    let normalized_version =
        normalize_optional_manifest_string(manifest.version.take()).or_else(|| {
            manifest
                .metadata
                .get("version")
                .cloned()
                .and_then(|value| normalize_optional_manifest_string(Some(value)))
        });
    let normalized_setup = manifest.setup.take().map(PluginSetup::normalized);
    let canonical_setup = normalized_setup.filter(|setup| !setup.is_effectively_empty());
    let normalized_slot_claims = normalize_plugin_slot_claims(manifest.slot_claims);
    let normalized_compatibility = manifest
        .compatibility
        .take()
        .map(PluginCompatibility::normalized)
        .filter(|compatibility| !compatibility.is_effectively_empty());
    manifest.api_version = normalized_api_version;
    manifest.version = normalized_version.clone();
    manifest.setup = canonical_setup;
    manifest.slot_claims = normalized_slot_claims;
    manifest.compatibility = normalized_compatibility;
    if let Some(version) = normalized_version {
        manifest
            .metadata
            .entry("version".to_owned())
            .or_insert(version);
    }
    manifest
}

fn validate_plugin_manifest_contract(
    manifest: &PluginManifest,
    source_kind: PluginSourceKind,
    path: &Path,
) -> Result<(), IntegrationError> {
    if matches!(source_kind, PluginSourceKind::PackageManifest) && manifest.api_version.is_none() {
        return Err(IntegrationError::PluginManifestParse {
            path: path.display().to_string(),
            reason: "package manifest must declare api_version".to_owned(),
        });
    }

    if let Some(api_version) = manifest.api_version.as_deref()
        && api_version != CURRENT_PLUGIN_MANIFEST_API_VERSION
    {
        return Err(IntegrationError::PluginManifestParse {
            path: path.display().to_string(),
            reason: format!(
                "plugin api_version `{api_version}` is not supported by current manifest api `{CURRENT_PLUGIN_MANIFEST_API_VERSION}`"
            ),
        });
    }

    if matches!(source_kind, PluginSourceKind::PackageManifest) && manifest.version.is_none() {
        return Err(IntegrationError::PluginManifestParse {
            path: path.display().to_string(),
            reason: "package manifest must declare top-level version".to_owned(),
        });
    }

    if let Some(version) = manifest.version.as_deref()
        && let Err(error) = Version::parse(version)
    {
        return Err(IntegrationError::PluginManifestParse {
            path: path.display().to_string(),
            reason: format!("plugin version `{version}` is invalid semver: {error}"),
        });
    }

    if let Some(version) = manifest.version.as_deref()
        && let Some(metadata_version) = manifest
            .metadata
            .get("version")
            .cloned()
            .and_then(|value| normalize_optional_manifest_string(Some(value)))
        && metadata_version != version
    {
        return Err(IntegrationError::PluginManifestParse {
            path: path.display().to_string(),
            reason: format!(
                "plugin version conflict: top-level version `{version}` does not match metadata.version `{metadata_version}`"
            ),
        });
    }

    Ok(())
}

fn normalize_plugin_slot_claims(mut claims: Vec<PluginSlotClaim>) -> Vec<PluginSlotClaim> {
    let mut normalized_claims = claims
        .drain(..)
        .map(PluginSlotClaim::normalized)
        .collect::<Vec<_>>();
    normalized_claims.sort();
    normalized_claims.dedup();
    normalized_claims
}

#[derive(Debug, Clone)]
struct RegisteredSlotClaim {
    plugin_id: String,
    provider_id: String,
    mode: PluginSlotMode,
}

type ClaimedSlotRegistry = BTreeMap<(String, String), Vec<RegisteredSlotClaim>>;

fn collect_claimed_slots(
    catalog: &IntegrationCatalog,
) -> Result<ClaimedSlotRegistry, IntegrationError> {
    let mut registry = ClaimedSlotRegistry::new();

    for provider in catalog.providers() {
        let Some(raw_json) = provider.metadata.get(PLUGIN_SLOT_CLAIMS_METADATA_KEY) else {
            continue;
        };
        let claims = serde_json::from_str::<Vec<PluginSlotClaim>>(raw_json).map_err(|error| {
            IntegrationError::PluginAbsorbFailed {
                plugin_id: provider
                    .metadata
                    .get("plugin_id")
                    .cloned()
                    .unwrap_or_else(|| format!("provider:{}", provider.provider_id)),
                reason: format!(
                    "existing provider `{}` has invalid {PLUGIN_SLOT_CLAIMS_METADATA_KEY}: {error}",
                    provider.provider_id
                ),
            }
        })?;

        let plugin_id = provider
            .metadata
            .get("plugin_id")
            .cloned()
            .unwrap_or_else(|| format!("provider:{}", provider.provider_id));

        for claim in claims {
            registry
                .entry((claim.slot, claim.key))
                .or_default()
                .push(RegisteredSlotClaim {
                    plugin_id: plugin_id.clone(),
                    provider_id: provider.provider_id.clone(),
                    mode: claim.mode,
                });
        }
    }

    Ok(registry)
}

fn validate_plugin_slot_claims(manifest: &PluginManifest) -> Result<(), IntegrationError> {
    let mut seen_modes = BTreeMap::<(String, String), PluginSlotMode>::new();

    for claim in &manifest.slot_claims {
        if claim.slot.is_empty() {
            return Err(IntegrationError::PluginAbsorbFailed {
                plugin_id: manifest.plugin_id.clone(),
                reason: "slot claim slot must not be empty".to_owned(),
            });
        }
        if claim.key.is_empty() {
            return Err(IntegrationError::PluginAbsorbFailed {
                plugin_id: manifest.plugin_id.clone(),
                reason: "slot claim key must not be empty".to_owned(),
            });
        }

        let slot_key = (claim.slot.clone(), claim.key.clone());
        if let Some(existing_mode) = seen_modes.insert(slot_key.clone(), claim.mode)
            && existing_mode != claim.mode
        {
            return Err(IntegrationError::PluginAbsorbFailed {
                plugin_id: manifest.plugin_id.clone(),
                reason: format!(
                    "slot claim `{}`:`{}` declares conflicting modes `{}` and `{}`",
                    slot_key.0,
                    slot_key.1,
                    existing_mode.as_str(),
                    claim.mode.as_str()
                ),
            });
        }
    }

    Ok(())
}

pub(crate) fn plugin_host_compatibility_issue(
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
        let current_version = match current_plugin_host_version() {
            Ok(current_version) => current_version,
            Err(error) => {
                return Some(error);
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

fn validate_plugin_host_compatibility(manifest: &PluginManifest) -> Result<(), IntegrationError> {
    let Some(issue) = plugin_host_compatibility_issue(manifest.compatibility.as_ref()) else {
        return Ok(());
    };

    Err(IntegrationError::PluginAbsorbFailed {
        plugin_id: manifest.plugin_id.clone(),
        reason: issue,
    })
}

fn register_plugin_slot_claims(
    manifest: &PluginManifest,
    registry: &mut ClaimedSlotRegistry,
) -> Result<(), IntegrationError> {
    for claim in &manifest.slot_claims {
        let slot_key = (claim.slot.clone(), claim.key.clone());

        if let Some(existing_claims) = registry.get(&slot_key)
            && let Some(existing) = existing_claims.iter().find(|existing| {
                existing.plugin_id != manifest.plugin_id
                    && slot_modes_conflict(existing.mode, claim.mode)
            })
        {
            return Err(IntegrationError::PluginAbsorbFailed {
                plugin_id: manifest.plugin_id.clone(),
                reason: format!(
                    "slot claim conflict on `{}`:`{}` with plugin `{}` (provider `{}`): `{}` cannot coexist with `{}`",
                    claim.slot,
                    claim.key,
                    existing.plugin_id,
                    existing.provider_id,
                    claim.mode.as_str(),
                    existing.mode.as_str()
                ),
            });
        }

        registry
            .entry(slot_key)
            .or_default()
            .push(RegisteredSlotClaim {
                plugin_id: manifest.plugin_id.clone(),
                provider_id: manifest.provider_id.clone(),
                mode: claim.mode,
            });
    }

    Ok(())
}

pub(crate) fn slot_modes_conflict(existing: PluginSlotMode, incoming: PluginSlotMode) -> bool {
    matches!(
        (existing, incoming),
        (PluginSlotMode::Exclusive, _) | (_, PluginSlotMode::Exclusive)
    )
}

fn stamp_plugin_slot_claims_metadata(
    metadata: &mut BTreeMap<String, String>,
    slot_claims: &[PluginSlotClaim],
) -> Result<(), IntegrationError> {
    if slot_claims.is_empty() {
        metadata.remove(PLUGIN_SLOT_CLAIMS_METADATA_KEY);
        return Ok(());
    }

    let encoded = serde_json::to_string(slot_claims).map_err(|error| {
        IntegrationError::PluginAbsorbFailed {
            plugin_id: metadata
                .get("plugin_id")
                .cloned()
                .unwrap_or_else(|| "unknown-plugin".to_owned()),
            reason: format!("serialize plugin slot claims metadata failed: {error}"),
        }
    })?;
    metadata.insert(PLUGIN_SLOT_CLAIMS_METADATA_KEY.to_owned(), encoded);
    Ok(())
}

fn stamp_plugin_manifest_contract_metadata(
    metadata: &mut BTreeMap<String, String>,
    manifest: &PluginManifest,
) {
    if let Some(api_version) = manifest.api_version.clone() {
        metadata.insert(
            PLUGIN_MANIFEST_API_VERSION_METADATA_KEY.to_owned(),
            api_version,
        );
    } else {
        metadata.remove(PLUGIN_MANIFEST_API_VERSION_METADATA_KEY);
    }

    if let Some(version) = manifest.version.clone() {
        metadata.insert(PLUGIN_VERSION_METADATA_KEY.to_owned(), version);
    } else {
        metadata.remove(PLUGIN_VERSION_METADATA_KEY);
    }
}

fn stamp_plugin_descriptor_contract_metadata(
    metadata: &mut BTreeMap<String, String>,
    descriptor: &PluginDescriptor,
) {
    metadata.insert(
        PLUGIN_DIALECT_METADATA_KEY.to_owned(),
        descriptor.dialect.as_str().to_owned(),
    );

    if let Some(dialect_version) = descriptor.dialect_version.clone() {
        metadata.insert(
            PLUGIN_DIALECT_VERSION_METADATA_KEY.to_owned(),
            dialect_version,
        );
    } else {
        metadata.remove(PLUGIN_DIALECT_VERSION_METADATA_KEY);
    }

    metadata.insert(
        PLUGIN_COMPATIBILITY_MODE_METADATA_KEY.to_owned(),
        descriptor.compatibility_mode.as_str().to_owned(),
    );

    if let Some(shim) = PluginCompatibilityShim::for_mode(descriptor.compatibility_mode) {
        metadata.insert(
            PLUGIN_COMPATIBILITY_SHIM_ID_METADATA_KEY.to_owned(),
            shim.shim_id,
        );
        metadata.insert(
            PLUGIN_COMPATIBILITY_SHIM_FAMILY_METADATA_KEY.to_owned(),
            shim.family,
        );
    } else {
        metadata.remove(PLUGIN_COMPATIBILITY_SHIM_ID_METADATA_KEY);
        metadata.remove(PLUGIN_COMPATIBILITY_SHIM_FAMILY_METADATA_KEY);
    }
}

fn stamp_plugin_compatibility_metadata(
    metadata: &mut BTreeMap<String, String>,
    compatibility: Option<&PluginCompatibility>,
) {
    let Some(compatibility) = compatibility else {
        metadata.remove(PLUGIN_COMPATIBILITY_HOST_API_METADATA_KEY);
        metadata.remove(PLUGIN_COMPATIBILITY_HOST_VERSION_REQ_METADATA_KEY);
        return;
    };

    if let Some(host_api) = compatibility.host_api.clone() {
        metadata.insert(
            PLUGIN_COMPATIBILITY_HOST_API_METADATA_KEY.to_owned(),
            host_api,
        );
    } else {
        metadata.remove(PLUGIN_COMPATIBILITY_HOST_API_METADATA_KEY);
    }

    if let Some(host_version_req) = compatibility.host_version_req.clone() {
        metadata.insert(
            PLUGIN_COMPATIBILITY_HOST_VERSION_REQ_METADATA_KEY.to_owned(),
            host_version_req,
        );
    } else {
        metadata.remove(PLUGIN_COMPATIBILITY_HOST_VERSION_REQ_METADATA_KEY);
    }
}

fn current_plugin_host_version() -> Result<Version, String> {
    let raw_version = env!("CARGO_PKG_VERSION");
    let parsed_version = Version::parse(raw_version);

    parsed_version.map_err(|error| {
        format!("current host version `{raw_version}` is invalid and cannot satisfy plugin compatibility checks: {error}")
    })
}

fn normalize_optional_manifest_string(raw: Option<String>) -> Option<String> {
    let value = raw?;
    let trimmed = value.trim();

    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_owned())
}

fn normalize_manifest_string_list(values: Vec<String>) -> Vec<String> {
    let mut normalized_values = Vec::new();

    for value in values {
        let trimmed = value.trim();
        let is_empty = trimmed.is_empty();

        if is_empty {
            continue;
        }

        let candidate = trimmed.to_owned();
        let is_duplicate = normalized_values
            .iter()
            .any(|existing| existing == &candidate);

        if is_duplicate {
            continue;
        }

        normalized_values.push(candidate);
    }

    normalized_values
}

fn detect_language(path: &Path) -> String {
    if is_package_manifest_file(path) {
        return "manifest".to_owned();
    }

    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn normalize_language_name(language: &str) -> String {
    match language.trim().to_ascii_lowercase().as_str() {
        "rs" => "rust".to_owned(),
        "py" => "python".to_owned(),
        "js" => "javascript".to_owned(),
        "ts" => "typescript".to_owned(),
        "mjs" | "cjs" | "cts" | "mts" => "javascript".to_owned(),
        "unknown" | "" => "unknown".to_owned(),
        other => other.to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManifestFieldConflict {
    field: String,
    package_value: String,
    source_value: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_tmp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        std::env::temp_dir().join(format!("{}-{}", prefix, nanos))
    }

    fn sample_pack() -> VerticalPackManifest {
        VerticalPackManifest {
            pack_id: "sample-pack".to_owned(),
            domain: "engineering".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: crate::contracts::ExecutionRoute {
                harness_kind: crate::contracts::HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        }
    }

    fn scan_diagnostic<'a>(
        report: &'a PluginScanReport,
        code: PluginDiagnosticCode,
        plugin_id: &str,
    ) -> Option<&'a PluginDiagnosticFinding> {
        report
            .diagnostic_findings
            .iter()
            .find(|finding| finding.code == code && finding.plugin_id.as_deref() == Some(plugin_id))
    }

    #[test]
    fn scanner_finds_manifest_in_rust_and_python_files() {
        let root = unique_tmp_dir("loong-plugin-scan");
        fs::create_dir_all(&root).expect("create temp root");

        let rust_file = root.join("openrouter.rs");
        fs::write(
            &rust_file,
            r#"
// LOONG_PLUGIN_START
// {
//   "plugin_id": "openrouter-rs",
//   "provider_id": "openrouter",
//   "connector_name": "openrouter",
//   "channel_id": "primary",
//   "endpoint": "https://openrouter.ai/api/v1/chat/completions",
//   "capabilities": ["InvokeConnector", "ObserveTelemetry"],
//   "metadata": {"version":"0.2.0","lang":"rust"}
// }
// LOONG_PLUGIN_END
"#,
        )
        .expect("write rust plugin");

        let py_file = root.join("slack_plugin.py");
        fs::write(
            &py_file,
            r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "slack-py",
#   "provider_id": "slack",
#   "connector_name": "slack",
#   "channel_id": "alerts",
#   "endpoint": "https://hooks.slack.com/services/aaa/bbb/ccc",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"version":"1.1.0","lang":"python"}
# }
# LOONG_PLUGIN_END
"#,
        )
        .expect("write python plugin");

        let scanner = PluginScanner::new();
        let report = scanner.scan_path(&root).expect("scan should succeed");
        assert_eq!(report.matched_plugins, 2);
        assert!(
            report
                .descriptors
                .iter()
                .any(|descriptor| descriptor.manifest.provider_id == "openrouter")
        );
        assert!(
            report
                .descriptors
                .iter()
                .all(|descriptor| descriptor.source_kind == PluginSourceKind::EmbeddedSource)
        );
        assert!(
            report
                .descriptors
                .iter()
                .all(|descriptor| descriptor.package_manifest_path.is_none())
        );
        assert!(report.descriptors.iter().all(|descriptor| matches!(
            descriptor.manifest.trust_tier,
            PluginTrustTier::Unverified
        )));
        assert!(
            report
                .descriptors
                .iter()
                .any(|descriptor| descriptor.manifest.provider_id == "slack")
        );
        assert_eq!(
            report
                .diagnostic_findings
                .iter()
                .filter(|finding| finding.code == PluginDiagnosticCode::EmbeddedSourceLegacyContract)
                .count(),
            2
        );
        assert_eq!(
            report
                .diagnostic_findings
                .iter()
                .filter(|finding| finding.code == PluginDiagnosticCode::LegacyMetadataVersion)
                .count(),
            2
        );
    }

    #[test]
    fn scanner_finds_package_manifest_file() {
        let root = unique_tmp_dir("loong-plugin-package-manifest");
        fs::create_dir_all(&root).expect("create temp root");

        let manifest_file = root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &manifest_file,
            r#"
{
  "api_version": "v1alpha1",
  "plugin_id": "tavily-search",
  "version": "0.3.0",
  "provider_id": "tavily",
  "connector_name": "tavily-http",
  "endpoint": "https://api.tavily.com/search",
  "capabilities": ["InvokeConnector"],
  "trust_tier": "verified-community",
  "metadata": {
    "bridge_kind": "http_json",
    "adapter_family": "web-search"
  },
  "summary": "Manifest-discovered Tavily package",
  "tags": ["search", "provider"],
  "setup": {
    "mode": "metadata_only",
    "surface": " web_search ",
    "required_env_vars": ["TAVILY_API_KEY", " ", "TAVILY_API_KEY"],
    "recommended_env_vars": ["TEAM_TAVILY_KEY"],
    "required_config_keys": ["tools.web_search.default_provider"],
    "default_env_var": " TAVILY_API_KEY ",
    "docs_urls": ["https://docs.example.com/tavily", "https://docs.example.com/tavily"],
    "remediation": " set a Tavily credential before enabling search "
  }
}
"#,
        )
        .expect("write package manifest");

        let scanner = PluginScanner::new();
        let report = scanner.scan_path(&root).expect("scan should succeed");

        assert_eq!(report.scanned_files, 1);
        assert_eq!(report.matched_plugins, 1);
        assert_eq!(report.descriptors.len(), 1);
        assert_eq!(
            report.descriptors[0].path,
            manifest_file.display().to_string()
        );
        assert_eq!(report.descriptors[0].language, "manifest");
        assert_eq!(
            report.descriptors[0].manifest.api_version.as_deref(),
            Some(CURRENT_PLUGIN_MANIFEST_API_VERSION)
        );
        assert_eq!(
            report.descriptors[0].manifest.version.as_deref(),
            Some("0.3.0")
        );
        assert_eq!(report.descriptors[0].manifest.plugin_id, "tavily-search");
        assert_eq!(report.descriptors[0].manifest.provider_id, "tavily");
        assert_eq!(
            report.descriptors[0]
                .manifest
                .metadata
                .get("version")
                .map(String::as_str),
            Some("0.3.0")
        );
        assert_eq!(
            report.descriptors[0].source_kind,
            PluginSourceKind::PackageManifest
        );
        assert_eq!(
            report.descriptors[0].package_root,
            root.display().to_string()
        );
        assert_eq!(
            report.descriptors[0].package_manifest_path,
            Some(manifest_file.display().to_string())
        );
        assert_eq!(
            report.descriptors[0].manifest.trust_tier,
            PluginTrustTier::VerifiedCommunity
        );
        assert_eq!(
            report.descriptors[0].manifest.setup,
            Some(PluginSetup {
                mode: PluginSetupMode::MetadataOnly,
                surface: Some("web_search".to_owned()),
                required_env_vars: vec!["TAVILY_API_KEY".to_owned()],
                recommended_env_vars: vec!["TEAM_TAVILY_KEY".to_owned()],
                required_config_keys: vec!["tools.web_search.default_provider".to_owned()],
                default_env_var: Some("TAVILY_API_KEY".to_owned()),
                docs_urls: vec!["https://docs.example.com/tavily".to_owned()],
                remediation: Some("set a Tavily credential before enabling search".to_owned()),
            })
        );
    }

    #[test]
    fn scanner_requires_api_version_for_package_manifest() {
        let root = unique_tmp_dir("loong-plugin-package-api-required");
        fs::create_dir_all(&root).expect("create temp root");

        let manifest_file = root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &manifest_file,
            r#"
{
  "version": "1.0.0",
  "plugin_id": "missing-api-version",
  "provider_id": "missing-api-version",
  "connector_name": "missing-api-version",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
        )
        .expect("write package manifest");

        let error = PluginScanner::new()
            .scan_path(&root)
            .expect_err("package manifests must declare api_version");

        let rendered = error.to_string();
        assert!(rendered.contains("api_version"));
        assert!(rendered.contains("package manifest"));
    }

    #[test]
    fn scanner_requires_top_level_version_for_package_manifest() {
        let root = unique_tmp_dir("loong-plugin-package-version-required");
        fs::create_dir_all(&root).expect("create temp root");

        let manifest_file = root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &manifest_file,
            r#"
{
  "api_version": "v1alpha1",
  "plugin_id": "missing-version",
  "provider_id": "missing-version",
  "connector_name": "missing-version",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
        )
        .expect("write package manifest");

        let error = PluginScanner::new()
            .scan_path(&root)
            .expect_err("package manifests must declare top-level version");

        let rendered = error.to_string();
        assert!(rendered.contains("top-level version"));
        assert!(rendered.contains("package manifest"));
    }

    #[test]
    fn scanner_rejects_legacy_version_metadata_in_package_manifest() {
        let root = unique_tmp_dir("loong-plugin-package-legacy-version");
        fs::create_dir_all(&root).expect("create temp root");

        let manifest_file = root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &manifest_file,
            r#"
{
  "api_version": "v1alpha1",
  "version": "1.2.3",
  "plugin_id": "legacy-version-metadata",
  "provider_id": "legacy-version-metadata",
  "connector_name": "legacy-version-metadata",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json",
    "version": "1.2.3"
  }
}
"#,
        )
        .expect("write package manifest");

        let error = PluginScanner::new()
            .scan_path(&root)
            .expect_err("package manifests should reject metadata.version");

        let rendered = error.to_string();
        assert!(rendered.contains("metadata.version"));
        assert!(rendered.contains("top-level `version`"));
    }

    #[test]
    fn scanner_rejects_reserved_metadata_namespace_in_package_manifest() {
        let root = unique_tmp_dir("loong-plugin-package-reserved-metadata");
        fs::create_dir_all(&root).expect("create temp root");

        let manifest_file = root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &manifest_file,
            r#"
{
  "api_version": "v1alpha1",
  "version": "1.2.3",
  "plugin_id": "reserved-metadata",
  "provider_id": "reserved-metadata",
  "connector_name": "reserved-metadata",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json",
    "plugin_version": "1.2.3"
  }
}
"#,
        )
        .expect("write package manifest");

        let error = PluginScanner::new()
            .scan_path(&root)
            .expect_err("package manifests should reject reserved metadata namespace");

        let rendered = error.to_string();
        assert!(rendered.contains("plugin_version"));
        assert!(rendered.contains("reserved"));
    }

    #[test]
    fn scanner_rejects_invalid_top_level_plugin_version() {
        let root = unique_tmp_dir("loong-plugin-invalid-version");
        fs::create_dir_all(&root).expect("create temp root");

        let manifest_file = root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &manifest_file,
            r#"
{
  "api_version": "v1alpha1",
  "version": "not-a-semver",
  "plugin_id": "bad-version",
  "provider_id": "bad-version",
  "connector_name": "bad-version",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
        )
        .expect("write package manifest");

        let error = PluginScanner::new()
            .scan_path(&root)
            .expect_err("invalid plugin version should fail parse");

        let rendered = error.to_string();
        assert!(rendered.contains("invalid semver"));
        assert!(rendered.contains("not-a-semver"));
    }

    #[test]
    fn scanner_rejects_conflicting_top_level_and_metadata_version_in_source_manifest() {
        let root = unique_tmp_dir("loong-plugin-source-version-conflict");
        fs::create_dir_all(&root).expect("create temp root");

        let source_file = root.join("plugin.py");
        fs::write(
            &source_file,
            r#"
# LOONG_PLUGIN_START
# {
#   "version": "1.2.3",
#   "plugin_id": "source-version-conflict",
#   "provider_id": "source-version-conflict",
#   "connector_name": "source-version-conflict",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {
#     "bridge_kind": "http_json",
#     "version": "9.9.9"
#   }
# }
# LOONG_PLUGIN_END
"#,
        )
        .expect("write source manifest");

        let error = PluginScanner::new()
            .scan_path(&root)
            .expect_err("source manifests should reject conflicting version truth");

        let rendered = error.to_string();
        assert!(rendered.contains("plugin version conflict"));
        assert!(rendered.contains("1.2.3"));
        assert!(rendered.contains("9.9.9"));
    }

    #[test]
    fn scanner_rejects_unknown_package_manifest_fields() {
        let root = unique_tmp_dir("loong-plugin-unknown-package-field");
        fs::create_dir_all(&root).expect("create temp root");

        let manifest_file = root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &manifest_file,
            r#"
{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "unknown-field",
  "provider_id": "unknown-field",
  "connector_name": "unknown-field",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  },
  "slot_claim": []
}
"#,
        )
        .expect("write package manifest");

        let error = PluginScanner::new()
            .scan_path(&root)
            .expect_err("unknown package manifest fields should fail parse");

        let rendered = error.to_string();
        assert!(rendered.contains("unknown field"));
        assert!(rendered.contains("slot_claim"));
    }

    #[test]
    fn scanner_prefers_package_manifest_over_embedded_source_manifest() {
        let root = unique_tmp_dir("loong-plugin-precedence");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("create temp root");

        let manifest_file = package_root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &manifest_file,
            r#"
{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "package-plugin",
  "provider_id": "package-provider",
  "connector_name": "package-connector",
  "channel_id": "package-channel",
  "endpoint": "https://package.example/invoke",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
        )
        .expect("write package manifest");

        let source_file = package_root.join("plugin.py");
        fs::write(
            &source_file,
            r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "package-plugin",
#   "provider_id": "package-provider",
#   "connector_name": "package-connector",
#   "channel_id": "package-channel",
#   "endpoint": "https://package.example/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json"}
# }
# LOONG_PLUGIN_END
"#,
        )
        .expect("write source plugin");

        let scanner = PluginScanner::new();
        let report = scanner.scan_path(&root).expect("scan should succeed");

        assert_eq!(report.scanned_files, 2);
        assert_eq!(report.matched_plugins, 1);
        assert_eq!(report.descriptors.len(), 1);
        assert_eq!(
            report.descriptors[0].path,
            manifest_file.display().to_string()
        );
        assert_eq!(
            report.descriptors[0].source_kind,
            PluginSourceKind::PackageManifest
        );
        assert_eq!(
            report.descriptors[0].package_root,
            package_root.display().to_string()
        );
        assert_eq!(
            report.descriptors[0].package_manifest_path,
            Some(manifest_file.display().to_string())
        );
        assert_eq!(report.descriptors[0].manifest.plugin_id, "package-plugin");
        assert_eq!(
            report.descriptors[0].manifest.provider_id,
            "package-provider"
        );
        let finding = scan_diagnostic(
            &report,
            PluginDiagnosticCode::ShadowedEmbeddedSource,
            "package-plugin",
        )
        .expect("shadowed embedded source finding");
        assert_eq!(finding.phase, PluginDiagnosticPhase::Scan);
        assert!(!finding.blocking);
    }

    #[test]
    fn scanner_fails_when_package_manifest_conflicts_with_source_manifest() {
        let root = unique_tmp_dir("loong-plugin-conflict");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("create temp root");

        let manifest_file = package_root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &manifest_file,
            r#"
{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "package-plugin",
  "provider_id": "package-provider",
  "connector_name": "package-connector",
  "channel_id": "package-channel",
  "endpoint": "https://package.example/invoke",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
        )
        .expect("write package manifest");

        let source_file = package_root.join("plugin.py");
        fs::write(
            &source_file,
            r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "package-plugin",
#   "provider_id": "source-provider",
#   "connector_name": "package-connector",
#   "channel_id": "package-channel",
#   "endpoint": "https://package.example/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json"}
# }
# LOONG_PLUGIN_END
"#,
        )
        .expect("write source plugin");

        let scanner = PluginScanner::new();
        let error = scanner
            .scan_path(&root)
            .expect_err("conflicting manifests should fail");

        assert_eq!(
            error,
            IntegrationError::PluginManifestConflict {
                package_manifest_path: manifest_file.display().to_string(),
                source_path: source_file.display().to_string(),
                field: "provider_id".to_owned(),
                package_value: "\"package-provider\"".to_owned(),
                source_value: "\"source-provider\"".to_owned(),
            }
        );
    }

    #[test]
    fn scanner_uses_nearest_package_manifest_for_nested_package_roots() {
        let root = unique_tmp_dir("loong-plugin-nested-package-root");
        let outer_root = root.join("outer");
        let inner_root = outer_root.join("inner");
        fs::create_dir_all(&inner_root).expect("create nested root");

        let outer_manifest_file = outer_root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &outer_manifest_file,
            r#"
{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "outer-plugin",
  "provider_id": "outer-provider",
  "connector_name": "outer-connector",
  "channel_id": "outer-channel",
  "endpoint": "https://outer.example/invoke",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
        )
        .expect("write outer package manifest");

        let inner_manifest_file = inner_root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &inner_manifest_file,
            r#"
{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "inner-plugin",
  "provider_id": "inner-provider",
  "connector_name": "inner-connector",
  "channel_id": "inner-channel",
  "endpoint": "https://inner.example/invoke",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
        )
        .expect("write inner package manifest");

        let source_file = inner_root.join("plugin.py");
        fs::write(
            &source_file,
            r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "inner-plugin",
#   "provider_id": "inner-provider",
#   "connector_name": "inner-connector",
#   "channel_id": "inner-channel",
#   "endpoint": "https://inner.example/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json"}
# }
# LOONG_PLUGIN_END
"#,
        )
        .expect("write nested source plugin");

        let scanner = PluginScanner::new();
        let report = scanner.scan_path(&root).expect("scan should succeed");

        assert_eq!(report.matched_plugins, 2);
        assert_eq!(report.descriptors.len(), 2);
        assert!(
            report
                .descriptors
                .iter()
                .any(|descriptor| descriptor.path == outer_manifest_file.display().to_string())
        );
        assert!(
            report
                .descriptors
                .iter()
                .any(|descriptor| descriptor.path == inner_manifest_file.display().to_string())
        );
    }

    #[test]
    fn scanner_allows_source_only_optional_fields_under_package_manifest() {
        let root = unique_tmp_dir("loong-plugin-optional-source-fields");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("create temp root");

        let manifest_file = package_root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &manifest_file,
            r#"
{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "package-plugin",
  "provider_id": "package-provider",
  "connector_name": "package-connector",
  "channel_id": "package-channel",
  "endpoint": "https://package.example/invoke",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
        )
        .expect("write package manifest");

        let source_file = package_root.join("plugin.py");
        fs::write(
            &source_file,
            r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "package-plugin",
#   "provider_id": "package-provider",
#   "connector_name": "package-connector",
#   "channel_id": "package-channel",
#   "endpoint": "https://package.example/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json","legacy_source":"true"},
#   "summary": "legacy source summary",
#   "tags": ["legacy", "source"],
#   "input_examples": [{"query":"hello"}]
# }
# LOONG_PLUGIN_END
"#,
        )
        .expect("write source plugin");

        let scanner = PluginScanner::new();
        let report = scanner.scan_path(&root).expect("scan should succeed");

        assert_eq!(report.scanned_files, 2);
        assert_eq!(report.matched_plugins, 1);
        assert_eq!(report.descriptors.len(), 1);
        assert_eq!(
            report.descriptors[0].path,
            manifest_file.display().to_string()
        );
        assert_eq!(report.descriptors[0].manifest.summary, None);
        assert!(report.descriptors[0].manifest.tags.is_empty());
        assert!(report.descriptors[0].manifest.input_examples.is_empty());
        assert!(
            !report.descriptors[0]
                .manifest
                .metadata
                .contains_key("legacy_source")
        );
        assert_eq!(
            report.descriptors[0].manifest.provider_id,
            "package-provider"
        );
        assert_eq!(report.descriptors[0].language, "manifest");
        let finding = scan_diagnostic(
            &report,
            PluginDiagnosticCode::ShadowedEmbeddedSource,
            "package-plugin",
        )
        .expect("shadowed embedded source finding");
        assert_eq!(finding.phase, PluginDiagnosticPhase::Scan);
        assert!(!finding.blocking);
    }

    #[test]
    fn scanner_falls_back_to_embedded_source_manifest_without_package_manifest() {
        let root = unique_tmp_dir("loong-plugin-source-fallback");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("create temp root");

        let source_file = package_root.join("plugin.py");
        fs::write(
            &source_file,
            r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "source-plugin",
#   "provider_id": "source-provider",
#   "connector_name": "source-connector",
#   "channel_id": "source-channel",
#   "endpoint": "https://source.example/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"process_stdio"},
#   "setup": {
#     "surface": "channel",
#     "required_env_vars": ["SOURCE_TOKEN"],
#     "default_env_var": "SOURCE_TOKEN"
#   }
# }
# LOONG_PLUGIN_END
"#,
        )
        .expect("write source plugin");

        let scanner = PluginScanner::new();
        let report = scanner.scan_path(&root).expect("scan should succeed");

        assert_eq!(report.scanned_files, 1);
        assert_eq!(report.matched_plugins, 1);
        assert_eq!(report.descriptors.len(), 1);
        assert_eq!(
            report.descriptors[0].path,
            source_file.display().to_string()
        );
        assert_eq!(
            report.descriptors[0].source_kind,
            PluginSourceKind::EmbeddedSource
        );
        assert_eq!(
            report.descriptors[0].package_root,
            package_root.display().to_string()
        );
        assert_eq!(report.descriptors[0].package_manifest_path, None);
        assert_eq!(report.descriptors[0].language, "py");
        assert_eq!(report.descriptors[0].manifest.plugin_id, "source-plugin");
        assert_eq!(
            report.descriptors[0].manifest.provider_id,
            "source-provider"
        );
        let finding = scan_diagnostic(
            &report,
            PluginDiagnosticCode::EmbeddedSourceLegacyContract,
            "source-plugin",
        )
        .expect("embedded source legacy finding");
        assert_eq!(finding.phase, PluginDiagnosticPhase::Scan);
        assert!(!finding.blocking);
        assert_eq!(
            report.descriptors[0].manifest.setup,
            Some(PluginSetup {
                mode: PluginSetupMode::MetadataOnly,
                surface: Some("channel".to_owned()),
                required_env_vars: vec!["SOURCE_TOKEN".to_owned()],
                recommended_env_vars: Vec::new(),
                required_config_keys: Vec::new(),
                default_env_var: Some("SOURCE_TOKEN".to_owned()),
                docs_urls: Vec::new(),
                remediation: None,
            })
        );
    }

    #[test]
    fn scanner_treats_empty_metadata_only_setup_as_absent() {
        let root = unique_tmp_dir("loong-plugin-empty-setup");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("create temp root");

        let manifest_file = package_root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &manifest_file,
            r#"
{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "package-plugin",
  "provider_id": "package-provider",
  "connector_name": "package-connector",
  "channel_id": "package-channel",
  "endpoint": "https://package.example/invoke",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "http_json"
  }
}
"#,
        )
        .expect("write package manifest");

        let source_file = package_root.join("plugin.py");
        fs::write(
            &source_file,
            r#"
# LOONG_PLUGIN_START
# {
#   "plugin_id": "package-plugin",
#   "provider_id": "package-provider",
#   "connector_name": "package-connector",
#   "channel_id": "package-channel",
#   "endpoint": "https://package.example/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json"},
#   "setup": {}
# }
# LOONG_PLUGIN_END
"#,
        )
        .expect("write source plugin");

        let scanner = PluginScanner::new();
        let report = scanner.scan_path(&root).expect("scan should succeed");

        assert_eq!(report.scanned_files, 2);
        assert_eq!(report.matched_plugins, 1);
        assert_eq!(report.descriptors.len(), 1);
        assert_eq!(report.descriptors[0].manifest.setup, None);
    }

    #[test]
    fn scanner_recognizes_openclaw_modern_manifest_through_explicit_compatibility_boundary() {
        let root = unique_tmp_dir("loong-openclaw-modern");
        let package_root = root.join("pkg");
        fs::create_dir_all(package_root.join("dist")).expect("create temp root");

        let package_manifest = package_root.join(OPENCLAW_PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &package_manifest,
            r#"
{
  "id": "search-sdk",
  "name": "Search SDK",
  "description": "OpenClaw search integration",
  "version": "1.2.3",
  "kind": "provider",
  "providers": ["web_search"],
  "channels": ["search"],
  "skills": ["search"],
  "configSchema": {}
}
"#,
        )
        .expect("write openclaw manifest");

        let package_json = package_root.join(PACKAGE_JSON_FILE_NAME);
        fs::write(
            &package_json,
            r#"
{
  "name": "@acme/search-provider",
  "version": "1.2.3",
  "description": "Search provider package",
  "openclaw": {
    "extensions": ["dist/index.js"],
    "setupEntry": "dist/setup.js",
    "channel": {
      "id": "search",
      "label": "Search",
      "aliases": ["web-search"]
    }
  }
}
"#,
        )
        .expect("write package.json");
        fs::write(package_root.join("dist/index.js"), "export {};\n").expect("write entry");
        fs::write(package_root.join("dist/setup.js"), "export {};\n").expect("write setup");

        let report = PluginScanner::new()
            .scan_path(&root)
            .expect("scan should succeed");

        assert_eq!(report.matched_plugins, 1);
        assert_eq!(report.descriptors.len(), 1);
        assert_eq!(
            report.descriptors[0].dialect,
            PluginContractDialect::OpenClawModernManifest
        );
        assert_eq!(
            report.descriptors[0].compatibility_mode,
            PluginCompatibilityMode::OpenClawModern
        );
        assert_eq!(report.descriptors[0].language, "js");
        assert_eq!(report.descriptors[0].manifest.plugin_id, "search-sdk");
        assert_eq!(
            report.descriptors[0].package_manifest_path,
            Some(package_manifest.display().to_string())
        );
        assert_eq!(
            report.descriptors[0].path,
            package_root.join("dist/index.js").display().to_string()
        );
        let foreign = scan_diagnostic(
            &report,
            PluginDiagnosticCode::ForeignDialectContract,
            "search-sdk",
        )
        .expect("foreign dialect diagnostic");
        assert_eq!(foreign.phase, PluginDiagnosticPhase::Scan);
        assert!(!foreign.blocking);
        assert_eq!(
            report.descriptors[0]
                .manifest
                .metadata
                .get("adapter_family")
                .map(String::as_str),
            Some(OPENCLAW_MODERN_COMPATIBILITY_ADAPTER_FAMILY)
        );
    }

    #[test]
    fn scanner_recognizes_openclaw_legacy_package_metadata_without_promoting_it_to_native() {
        let root = unique_tmp_dir("loong-openclaw-legacy");
        let package_root = root.join("pkg");
        fs::create_dir_all(package_root.join("dist")).expect("create temp root");

        let package_json = package_root.join(PACKAGE_JSON_FILE_NAME);
        fs::write(
            &package_json,
            r#"
{
  "name": "@acme/search-provider",
  "version": "0.9.0",
  "description": "Legacy OpenClaw package",
  "openclaw": {
    "extensions": ["dist/index.js"],
    "setupEntry": "dist/setup.js"
  }
}
"#,
        )
        .expect("write legacy package.json");
        fs::write(package_root.join("dist/index.js"), "export {};\n").expect("write entry");
        fs::write(package_root.join("dist/setup.js"), "export {};\n").expect("write setup");

        let report = PluginScanner::new()
            .scan_path(&root)
            .expect("scan should succeed");

        assert_eq!(report.matched_plugins, 1);
        assert_eq!(report.descriptors.len(), 1);
        assert_eq!(
            report.descriptors[0].dialect,
            PluginContractDialect::OpenClawLegacyPackage
        );
        assert_eq!(
            report.descriptors[0].compatibility_mode,
            PluginCompatibilityMode::OpenClawLegacy
        );
        assert_eq!(report.descriptors[0].manifest.plugin_id, "search");
        assert_eq!(
            report.descriptors[0].package_manifest_path,
            Some(package_json.display().to_string())
        );
        let foreign = scan_diagnostic(
            &report,
            PluginDiagnosticCode::ForeignDialectContract,
            "search",
        )
        .expect("foreign dialect diagnostic");
        assert_eq!(foreign.phase, PluginDiagnosticPhase::Scan);
        let legacy = scan_diagnostic(
            &report,
            PluginDiagnosticCode::LegacyOpenClawContract,
            "search",
        )
        .expect("legacy openclaw diagnostic");
        assert_eq!(legacy.phase, PluginDiagnosticPhase::Scan);
        assert_eq!(
            report.descriptors[0]
                .manifest
                .metadata
                .get("adapter_family")
                .map(String::as_str),
            Some(OPENCLAW_LEGACY_COMPATIBILITY_ADAPTER_FAMILY)
        );
    }

    #[test]
    fn scanner_absorbs_plugins_into_catalog_and_pack() {
        let report = PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![PluginDescriptor {
                path: "/tmp/openai.rs".to_owned(),
                source_kind: PluginSourceKind::EmbeddedSource,
                dialect: PluginContractDialect::LoongEmbeddedSource,
                dialect_version: None,
                compatibility_mode: PluginCompatibilityMode::Native,
                package_root: "/tmp".to_owned(),
                package_manifest_path: None,
                language: "rs".to_owned(),
                manifest: PluginManifest {
                    api_version: None,
                    version: Some("1.3.0".to_owned()),
                    plugin_id: "openai-rs".to_owned(),
                    provider_id: "openai".to_owned(),
                    connector_name: "openai".to_owned(),
                    channel_id: Some("chat-main".to_owned()),
                    endpoint: Some("https://api.openai.com/v1/chat/completions".to_owned()),
                    capabilities: BTreeSet::from([
                        Capability::InvokeConnector,
                        Capability::ObserveTelemetry,
                    ]),
                    trust_tier: PluginTrustTier::Official,
                    metadata: BTreeMap::from([("version".to_owned(), "1.3.0".to_owned())]),
                    summary: None,
                    tags: Vec::new(),
                    input_examples: Vec::new(),
                    output_examples: Vec::new(),
                    defer_loading: false,
                    setup: None,
                    slot_claims: Vec::new(),
                    compatibility: None,
                },
            }],
        };

        let mut catalog = IntegrationCatalog::new();
        let mut pack = sample_pack();
        let scanner = PluginScanner::new();

        let absorb = scanner
            .absorb(&mut catalog, &mut pack, &report)
            .expect("absorb should succeed");
        assert_eq!(absorb.absorbed_plugins, 1);
        assert_eq!(absorb.provider_upserts, 1);
        assert_eq!(absorb.channel_upserts, 1);
        assert!(catalog.provider("openai").is_some());
        assert!(catalog.channel("chat-main").is_some());
        assert!(pack.allowed_connectors.contains("openai"));
        assert!(
            pack.granted_capabilities
                .contains(&Capability::InvokeConnector)
        );
    }

    #[test]
    fn absorb_rejects_conflicting_exclusive_slot_claims() {
        let report = PluginScanReport {
            scanned_files: 2,
            matched_plugins: 2,
            diagnostic_findings: Vec::new(),
            descriptors: vec![
                PluginDescriptor {
                    path: "/tmp/search-a.py".to_owned(),
                    source_kind: PluginSourceKind::EmbeddedSource,
                    dialect: PluginContractDialect::LoongEmbeddedSource,
                    dialect_version: None,
                    compatibility_mode: PluginCompatibilityMode::Native,
                    package_root: "/tmp".to_owned(),
                    package_manifest_path: None,
                    language: "py".to_owned(),
                    manifest: PluginManifest {
                        api_version: None,
                        version: None,
                        plugin_id: "search-a".to_owned(),
                        provider_id: "search-a".to_owned(),
                        connector_name: "search-a".to_owned(),
                        channel_id: None,
                        endpoint: None,
                        capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        trust_tier: PluginTrustTier::Unverified,
                        metadata: BTreeMap::new(),
                        summary: None,
                        tags: Vec::new(),
                        input_examples: Vec::new(),
                        output_examples: Vec::new(),
                        defer_loading: false,
                        setup: None,
                        slot_claims: vec![PluginSlotClaim {
                            slot: "provider:web_search".to_owned(),
                            key: "tavily".to_owned(),
                            mode: PluginSlotMode::Exclusive,
                        }],
                        compatibility: None,
                    },
                },
                PluginDescriptor {
                    path: "/tmp/search-b.py".to_owned(),
                    source_kind: PluginSourceKind::EmbeddedSource,
                    dialect: PluginContractDialect::LoongEmbeddedSource,
                    dialect_version: None,
                    compatibility_mode: PluginCompatibilityMode::Native,
                    package_root: "/tmp".to_owned(),
                    package_manifest_path: None,
                    language: "py".to_owned(),
                    manifest: PluginManifest {
                        api_version: None,
                        version: None,
                        plugin_id: "search-b".to_owned(),
                        provider_id: "search-b".to_owned(),
                        connector_name: "search-b".to_owned(),
                        channel_id: None,
                        endpoint: None,
                        capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        trust_tier: PluginTrustTier::Unverified,
                        metadata: BTreeMap::new(),
                        summary: None,
                        tags: Vec::new(),
                        input_examples: Vec::new(),
                        output_examples: Vec::new(),
                        defer_loading: false,
                        setup: None,
                        slot_claims: vec![PluginSlotClaim {
                            slot: "provider:web_search".to_owned(),
                            key: "tavily".to_owned(),
                            mode: PluginSlotMode::Exclusive,
                        }],
                        compatibility: None,
                    },
                },
            ],
        };

        let mut catalog = IntegrationCatalog::new();
        let mut pack = sample_pack();

        let error = PluginScanner::new()
            .absorb(&mut catalog, &mut pack, &report)
            .expect_err("conflicting exclusive slot claims should fail");

        let rendered = error.to_string();
        assert!(rendered.contains("slot claim conflict"));
        assert!(catalog.provider("search-a").is_none());
        assert!(catalog.provider("search-b").is_none());
    }

    #[test]
    fn absorb_allows_shared_and_advisory_slot_claims_and_projects_metadata() {
        let report = PluginScanReport {
            scanned_files: 2,
            matched_plugins: 2,
            diagnostic_findings: Vec::new(),
            descriptors: vec![
                PluginDescriptor {
                    path: "/tmp/search-shared.py".to_owned(),
                    source_kind: PluginSourceKind::EmbeddedSource,
                    dialect: PluginContractDialect::LoongEmbeddedSource,
                    dialect_version: None,
                    compatibility_mode: PluginCompatibilityMode::Native,
                    package_root: "/tmp".to_owned(),
                    package_manifest_path: None,
                    language: "py".to_owned(),
                    manifest: PluginManifest {
                        api_version: None,
                        version: Some("1.0.0".to_owned()),
                        plugin_id: "search-shared".to_owned(),
                        provider_id: "search-shared".to_owned(),
                        connector_name: "search-shared".to_owned(),
                        channel_id: None,
                        endpoint: None,
                        capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        trust_tier: PluginTrustTier::Unverified,
                        metadata: BTreeMap::new(),
                        summary: None,
                        tags: Vec::new(),
                        input_examples: Vec::new(),
                        output_examples: Vec::new(),
                        defer_loading: false,
                        setup: None,
                        slot_claims: vec![PluginSlotClaim {
                            slot: "tool:search".to_owned(),
                            key: "web".to_owned(),
                            mode: PluginSlotMode::Shared,
                        }],
                        compatibility: Some(PluginCompatibility {
                            host_api: Some(CURRENT_PLUGIN_HOST_API.to_owned()),
                            host_version_req: Some(">=0.1.0-alpha.1".to_owned()),
                        }),
                    },
                },
                PluginDescriptor {
                    path: "/tmp/search-advisory.py".to_owned(),
                    source_kind: PluginSourceKind::EmbeddedSource,
                    dialect: PluginContractDialect::LoongEmbeddedSource,
                    dialect_version: None,
                    compatibility_mode: PluginCompatibilityMode::Native,
                    package_root: "/tmp".to_owned(),
                    package_manifest_path: None,
                    language: "py".to_owned(),
                    manifest: PluginManifest {
                        api_version: None,
                        version: None,
                        plugin_id: "search-advisory".to_owned(),
                        provider_id: "search-advisory".to_owned(),
                        connector_name: "search-advisory".to_owned(),
                        channel_id: None,
                        endpoint: None,
                        capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        trust_tier: PluginTrustTier::Unverified,
                        metadata: BTreeMap::new(),
                        summary: None,
                        tags: Vec::new(),
                        input_examples: Vec::new(),
                        output_examples: Vec::new(),
                        defer_loading: false,
                        setup: None,
                        slot_claims: vec![PluginSlotClaim {
                            slot: "tool:search".to_owned(),
                            key: "web".to_owned(),
                            mode: PluginSlotMode::Advisory,
                        }],
                        compatibility: None,
                    },
                },
            ],
        };

        let mut catalog = IntegrationCatalog::new();
        let mut pack = sample_pack();

        let absorb = PluginScanner::new()
            .absorb(&mut catalog, &mut pack, &report)
            .expect("shared and advisory slot claims should coexist");

        assert_eq!(absorb.absorbed_plugins, 2);
        let shared_provider = catalog
            .provider("search-shared")
            .expect("shared provider should be registered");
        assert_eq!(
            shared_provider
                .metadata
                .get(PLUGIN_SLOT_CLAIMS_METADATA_KEY)
                .map(String::as_str),
            Some("[{\"slot\":\"tool:search\",\"key\":\"web\",\"mode\":\"shared\"}]")
        );
        assert_eq!(
            shared_provider
                .metadata
                .get(PLUGIN_COMPATIBILITY_HOST_API_METADATA_KEY)
                .map(String::as_str),
            Some(CURRENT_PLUGIN_HOST_API)
        );
        assert_eq!(
            shared_provider
                .metadata
                .get(PLUGIN_COMPATIBILITY_HOST_VERSION_REQ_METADATA_KEY)
                .map(String::as_str),
            Some(">=0.1.0-alpha.1")
        );
    }

    #[test]
    fn absorb_rejects_incompatible_host_api() {
        let report = PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![PluginDescriptor {
                path: "/tmp/incompatible-host.py".to_owned(),
                source_kind: PluginSourceKind::EmbeddedSource,
                dialect: PluginContractDialect::LoongEmbeddedSource,
                dialect_version: None,
                compatibility_mode: PluginCompatibilityMode::Native,
                package_root: "/tmp".to_owned(),
                package_manifest_path: None,
                language: "py".to_owned(),
                manifest: PluginManifest {
                    api_version: None,
                    version: None,
                    plugin_id: "incompatible-host".to_owned(),
                    provider_id: "incompatible-host".to_owned(),
                    connector_name: "incompatible-host".to_owned(),
                    channel_id: None,
                    endpoint: None,
                    capabilities: BTreeSet::from([Capability::InvokeConnector]),
                    trust_tier: PluginTrustTier::Unverified,
                    metadata: BTreeMap::new(),
                    summary: None,
                    tags: Vec::new(),
                    input_examples: Vec::new(),
                    output_examples: Vec::new(),
                    defer_loading: false,
                    setup: None,
                    slot_claims: Vec::new(),
                    compatibility: Some(PluginCompatibility {
                        host_api: Some("loong-plugin/v999".to_owned()),
                        host_version_req: None,
                    }),
                },
            }],
        };

        let mut catalog = IntegrationCatalog::new();
        let mut pack = sample_pack();

        let error = PluginScanner::new()
            .absorb(&mut catalog, &mut pack, &report)
            .expect_err("incompatible host api should fail closed");

        let rendered = error.to_string();
        assert!(rendered.contains("compatibility.host_api"));
        assert!(rendered.contains(CURRENT_PLUGIN_HOST_API));
        assert!(catalog.provider("incompatible-host").is_none());
    }

    #[test]
    fn absorb_rejects_invalid_host_version_requirement() {
        let report = PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![PluginDescriptor {
                path: "/tmp/invalid-version.py".to_owned(),
                source_kind: PluginSourceKind::EmbeddedSource,
                dialect: PluginContractDialect::LoongEmbeddedSource,
                dialect_version: None,
                compatibility_mode: PluginCompatibilityMode::Native,
                package_root: "/tmp".to_owned(),
                package_manifest_path: None,
                language: "py".to_owned(),
                manifest: PluginManifest {
                    api_version: None,
                    version: None,
                    plugin_id: "invalid-version".to_owned(),
                    provider_id: "invalid-version".to_owned(),
                    connector_name: "invalid-version".to_owned(),
                    channel_id: None,
                    endpoint: None,
                    capabilities: BTreeSet::from([Capability::InvokeConnector]),
                    trust_tier: PluginTrustTier::Unverified,
                    metadata: BTreeMap::new(),
                    summary: None,
                    tags: Vec::new(),
                    input_examples: Vec::new(),
                    output_examples: Vec::new(),
                    defer_loading: false,
                    setup: None,
                    slot_claims: Vec::new(),
                    compatibility: Some(PluginCompatibility {
                        host_api: Some(CURRENT_PLUGIN_HOST_API.to_owned()),
                        host_version_req: Some("not-a-semver-req".to_owned()),
                    }),
                },
            }],
        };

        let mut catalog = IntegrationCatalog::new();
        let mut pack = sample_pack();

        let error = PluginScanner::new()
            .absorb(&mut catalog, &mut pack, &report)
            .expect_err("invalid host version requirement should fail closed");

        let rendered = error.to_string();
        assert!(rendered.contains("compatibility.host_version_req"));
        assert!(rendered.contains("invalid"));
        assert!(catalog.provider("invalid-version").is_none());
    }

    #[test]
    fn scanner_skips_non_utf8_files_instead_of_failing() {
        let root = unique_tmp_dir("loong-plugin-binary");
        fs::create_dir_all(&root).expect("create temp root");
        let binary = root.join("compiled.bin");
        fs::write(&binary, [0xff_u8, 0xfe, 0x00, 0x81]).expect("write binary file");

        let scanner = PluginScanner::new();
        let report = scanner
            .scan_path(&root)
            .expect("binary files should be skipped, not fail");
        assert_eq!(report.scanned_files, 1);
        assert_eq!(report.matched_plugins, 0);
    }

    #[test]
    fn absorb_rolls_back_catalog_and_pack_on_validation_failure() {
        // First descriptor is valid, second has an empty provider_id which
        // triggers validation failure. The rollback must undo the first
        // descriptor's mutations so catalog and pack remain unchanged.
        let report = PluginScanReport {
            scanned_files: 2,
            matched_plugins: 2,
            diagnostic_findings: Vec::new(),
            descriptors: vec![
                PluginDescriptor {
                    path: "/tmp/good.rs".to_owned(),
                    source_kind: PluginSourceKind::EmbeddedSource,
                    dialect: PluginContractDialect::LoongEmbeddedSource,
                    dialect_version: None,
                    compatibility_mode: PluginCompatibilityMode::Native,
                    package_root: "/tmp".to_owned(),
                    package_manifest_path: None,
                    language: "rs".to_owned(),
                    manifest: PluginManifest {
                        api_version: None,
                        version: Some("1.0.0".to_owned()),
                        plugin_id: "good-plugin".to_owned(),
                        provider_id: "good-provider".to_owned(),
                        connector_name: "good-connector".to_owned(),
                        channel_id: Some("good-channel".to_owned()),
                        endpoint: Some("https://good.local/invoke".to_owned()),
                        capabilities: BTreeSet::from([Capability::InvokeConnector]),
                        trust_tier: PluginTrustTier::VerifiedCommunity,
                        metadata: BTreeMap::from([("version".to_owned(), "1.0.0".to_owned())]),
                        summary: None,
                        tags: Vec::new(),
                        input_examples: Vec::new(),
                        output_examples: Vec::new(),
                        defer_loading: false,
                        setup: None,
                        slot_claims: Vec::new(),
                        compatibility: None,
                    },
                },
                PluginDescriptor {
                    path: "/tmp/bad.rs".to_owned(),
                    source_kind: PluginSourceKind::EmbeddedSource,
                    dialect: PluginContractDialect::LoongEmbeddedSource,
                    dialect_version: None,
                    compatibility_mode: PluginCompatibilityMode::Native,
                    package_root: "/tmp".to_owned(),
                    package_manifest_path: None,
                    language: "rs".to_owned(),
                    manifest: PluginManifest {
                        api_version: None,
                        version: None,
                        plugin_id: "bad-plugin".to_owned(),
                        provider_id: String::new(), // empty — triggers validation error
                        connector_name: "bad-connector".to_owned(),
                        channel_id: None,
                        endpoint: None,
                        capabilities: BTreeSet::new(),
                        trust_tier: PluginTrustTier::Unverified,
                        metadata: BTreeMap::new(),
                        summary: None,
                        tags: Vec::new(),
                        input_examples: Vec::new(),
                        output_examples: Vec::new(),
                        defer_loading: false,
                        setup: None,
                        slot_claims: Vec::new(),
                        compatibility: None,
                    },
                },
            ],
        };

        let mut catalog = IntegrationCatalog::new();
        let mut pack = sample_pack();
        let scanner = PluginScanner::new();

        let catalog_before = catalog.clone();
        let pack_before = pack.clone();

        let result = scanner.absorb(&mut catalog, &mut pack, &report);
        assert!(result.is_err(), "absorb should fail on empty provider_id");

        // Verify rollback: catalog and pack are identical to their pre-absorb state.
        assert_eq!(catalog, catalog_before, "catalog must be rolled back");
        assert_eq!(pack, pack_before, "pack must be rolled back");
    }

    #[test]
    fn format_plugin_provenance_summary_prefers_package_manifest_context() {
        let summary = format_plugin_provenance_summary(
            PluginSourceKind::EmbeddedSource,
            "/tmp/pkg/plugin.py",
            Some("/tmp/pkg/loong.plugin.json"),
        );

        assert_eq!(
            summary,
            "embedded_source:/tmp/pkg/plugin.py (package_manifest:/tmp/pkg/loong.plugin.json)"
        );
    }
}
