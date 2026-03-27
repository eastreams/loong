use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    contracts::Capability,
    errors::IntegrationError,
    integration::{AutoProvisionRequest, ChannelConfig, IntegrationCatalog, ProviderConfig},
    pack::VerticalPackManifest,
};

const PACKAGE_MANIFEST_FILE_NAME: &str = "loongclaw.plugin.json";

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub path: String,
    pub source_kind: PluginSourceKind,
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
        let source_manifest_descriptors = collect_source_manifest_descriptors(&files)?;
        let package_manifests_by_root =
            collect_package_manifest_descriptors_by_root(&package_manifest_descriptors);

        validate_package_manifest_conflicts(
            &package_manifests_by_root,
            &source_manifest_descriptors,
        )?;

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

            catalog.upsert_provider(ProviderConfig {
                provider_id: manifest.provider_id.clone(),
                connector_name: manifest.connector_name.clone(),
                version: manifest
                    .metadata
                    .get("version")
                    .cloned()
                    .unwrap_or_else(|| "0.1.0".to_owned()),
                metadata: manifest.metadata.clone(),
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

#[derive(Debug, Deserialize)]
struct PackageManifestDocument {
    #[serde(flatten)]
    manifest: PluginManifest,
    #[serde(default)]
    version: Option<String>,
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

    for path in files {
        if !is_package_manifest_file(path) {
            continue;
        }

        let descriptor = parse_package_manifest_descriptor(path)?;
        descriptors.insert(path.clone(), descriptor);
    }

    Ok(descriptors)
}

fn collect_source_manifest_descriptors(
    files: &[PathBuf],
) -> Result<BTreeMap<PathBuf, PluginDescriptor>, IntegrationError> {
    let mut descriptors = BTreeMap::new();

    for path in files {
        let descriptor = parse_source_manifest_descriptor(path)?;
        let Some(descriptor) = descriptor else {
            continue;
        };

        descriptors.insert(path.clone(), descriptor);
    }

    Ok(descriptors)
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

fn parse_package_manifest_descriptor(path: &Path) -> Result<PluginDescriptor, IntegrationError> {
    let manifest = parse_package_manifest_file(path)?;
    let descriptor = build_plugin_descriptor(
        path,
        PluginSourceKind::PackageManifest,
        Some(path),
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

    let mut document: PackageManifestDocument =
        serde_json::from_str(content.trim()).map_err(|error| {
            IntegrationError::PluginManifestParse {
                path: path.display().to_string(),
                reason: error.to_string(),
            }
        })?;

    let version = document.version.take();

    if let Some(version) = version {
        let metadata_has_version = document.manifest.metadata.contains_key("version");

        if !metadata_has_version {
            document
                .manifest
                .metadata
                .insert("version".to_owned(), version);
        }
    }

    let normalized_manifest = normalize_plugin_manifest(document.manifest);

    Ok(normalized_manifest)
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

    let descriptor =
        build_plugin_descriptor(path, PluginSourceKind::EmbeddedSource, None, manifest);

    Ok(Some(descriptor))
}

fn build_plugin_descriptor(
    path: &Path,
    source_kind: PluginSourceKind,
    package_manifest_path: Option<&Path>,
    manifest: PluginManifest,
) -> PluginDescriptor {
    let path_string = path_to_string(path);
    let package_root = package_root_for_path(path);
    let package_manifest_path = package_manifest_path.map(path_to_string);
    let language = detect_language(path);

    PluginDescriptor {
        path: path_string,
        source_kind,
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
    let file_name = path.file_name();
    let file_name = file_name.and_then(|value| value.to_str());

    matches!(file_name, Some(PACKAGE_MANIFEST_FILE_NAME))
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

    let setup_conflict =
        compare_manifest_value("setup", &package_manifest.setup, &source_manifest.setup);
    if setup_conflict.is_some() {
        return setup_conflict;
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
    const START: &str = "LOONGCLAW_PLUGIN_START";
    const END: &str = "LOONGCLAW_PLUGIN_END";

    let Some(start_idx) = content.find(START) else {
        return Ok(None);
    };

    let Some(end_idx) = content[start_idx..].find(END).map(|idx| start_idx + idx) else {
        return Err(IntegrationError::PluginManifestParse {
            path: path.display().to_string(),
            reason: "missing LOONGCLAW_PLUGIN_END".to_owned(),
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
    let normalized_setup = manifest.setup.take().map(PluginSetup::normalized);
    let canonical_setup = normalized_setup.filter(|setup| !setup.is_effectively_empty());
    manifest.setup = canonical_setup;
    manifest
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

    #[test]
    fn scanner_finds_manifest_in_rust_and_python_files() {
        let root = unique_tmp_dir("loongclaw-plugin-scan");
        fs::create_dir_all(&root).expect("create temp root");

        let rust_file = root.join("openrouter.rs");
        fs::write(
            &rust_file,
            r#"
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "openrouter-rs",
//   "provider_id": "openrouter",
//   "connector_name": "openrouter",
//   "channel_id": "primary",
//   "endpoint": "https://openrouter.ai/api/v1/chat/completions",
//   "capabilities": ["InvokeConnector", "ObserveTelemetry"],
//   "metadata": {"version":"0.2.0","lang":"rust"}
// }
// LOONGCLAW_PLUGIN_END
"#,
        )
        .expect("write rust plugin");

        let py_file = root.join("slack_plugin.py");
        fs::write(
            &py_file,
            r#"
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "slack-py",
#   "provider_id": "slack",
#   "connector_name": "slack",
#   "channel_id": "alerts",
#   "endpoint": "https://hooks.slack.com/services/aaa/bbb/ccc",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"version":"1.1.0","lang":"python"}
# }
# LOONGCLAW_PLUGIN_END
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
    }

    #[test]
    fn scanner_finds_package_manifest_file() {
        let root = unique_tmp_dir("loongclaw-plugin-package-manifest");
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
    fn scanner_prefers_package_manifest_over_embedded_source_manifest() {
        let root = unique_tmp_dir("loongclaw-plugin-precedence");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("create temp root");

        let manifest_file = package_root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &manifest_file,
            r#"
{
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
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "package-plugin",
#   "provider_id": "package-provider",
#   "connector_name": "package-connector",
#   "channel_id": "package-channel",
#   "endpoint": "https://package.example/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json"}
# }
# LOONGCLAW_PLUGIN_END
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
    }

    #[test]
    fn scanner_fails_when_package_manifest_conflicts_with_source_manifest() {
        let root = unique_tmp_dir("loongclaw-plugin-conflict");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("create temp root");

        let manifest_file = package_root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &manifest_file,
            r#"
{
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
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "package-plugin",
#   "provider_id": "source-provider",
#   "connector_name": "package-connector",
#   "channel_id": "package-channel",
#   "endpoint": "https://package.example/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json"}
# }
# LOONGCLAW_PLUGIN_END
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
        let root = unique_tmp_dir("loongclaw-plugin-nested-package-root");
        let outer_root = root.join("outer");
        let inner_root = outer_root.join("inner");
        fs::create_dir_all(&inner_root).expect("create nested root");

        let outer_manifest_file = outer_root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &outer_manifest_file,
            r#"
{
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
# LOONGCLAW_PLUGIN_START
# {
#   "plugin_id": "inner-plugin",
#   "provider_id": "inner-provider",
#   "connector_name": "inner-connector",
#   "channel_id": "inner-channel",
#   "endpoint": "https://inner.example/invoke",
#   "capabilities": ["InvokeConnector"],
#   "metadata": {"bridge_kind":"http_json"}
# }
# LOONGCLAW_PLUGIN_END
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
        let root = unique_tmp_dir("loongclaw-plugin-optional-source-fields");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("create temp root");

        let manifest_file = package_root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &manifest_file,
            r#"
{
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
# LOONGCLAW_PLUGIN_START
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
# LOONGCLAW_PLUGIN_END
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
    }

    #[test]
    fn scanner_falls_back_to_embedded_source_manifest_without_package_manifest() {
        let root = unique_tmp_dir("loongclaw-plugin-source-fallback");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("create temp root");

        let source_file = package_root.join("plugin.py");
        fs::write(
            &source_file,
            r#"
# LOONGCLAW_PLUGIN_START
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
# LOONGCLAW_PLUGIN_END
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
        let root = unique_tmp_dir("loongclaw-plugin-empty-setup");
        let package_root = root.join("pkg");
        fs::create_dir_all(&package_root).expect("create temp root");

        let manifest_file = package_root.join(PACKAGE_MANIFEST_FILE_NAME);
        fs::write(
            &manifest_file,
            r#"
{
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
# LOONGCLAW_PLUGIN_START
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
# LOONGCLAW_PLUGIN_END
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
    fn scanner_absorbs_plugins_into_catalog_and_pack() {
        let report = PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            descriptors: vec![PluginDescriptor {
                path: "/tmp/openai.rs".to_owned(),
                source_kind: PluginSourceKind::EmbeddedSource,
                package_root: "/tmp".to_owned(),
                package_manifest_path: None,
                language: "rs".to_owned(),
                manifest: PluginManifest {
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
    fn scanner_skips_non_utf8_files_instead_of_failing() {
        let root = unique_tmp_dir("loongclaw-plugin-binary");
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
            descriptors: vec![
                PluginDescriptor {
                    path: "/tmp/good.rs".to_owned(),
                    source_kind: PluginSourceKind::EmbeddedSource,
                    package_root: "/tmp".to_owned(),
                    package_manifest_path: None,
                    language: "rs".to_owned(),
                    manifest: PluginManifest {
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
                    },
                },
                PluginDescriptor {
                    path: "/tmp/bad.rs".to_owned(),
                    source_kind: PluginSourceKind::EmbeddedSource,
                    package_root: "/tmp".to_owned(),
                    package_manifest_path: None,
                    language: "rs".to_owned(),
                    manifest: PluginManifest {
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
            Some("/tmp/pkg/loongclaw.plugin.json"),
        );

        assert_eq!(
            summary,
            "embedded_source:/tmp/pkg/plugin.py (package_manifest:/tmp/pkg/loongclaw.plugin.json)"
        );
    }
}
