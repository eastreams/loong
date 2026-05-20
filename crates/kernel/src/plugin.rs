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

mod openclaw;

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
            let descriptor = openclaw::parse_openclaw_manifest_descriptor(path)?;
            descriptors.insert(PathBuf::from(descriptor.path.clone()), descriptor);
            continue;
        }

        if is_package_json_file(path) {
            for descriptor in
                openclaw::parse_openclaw_legacy_package_descriptors(path, &known_files)?
            {
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
mod tests;
