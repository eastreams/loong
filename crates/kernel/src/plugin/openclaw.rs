use super::*;
use serde::Deserialize;

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

pub(super) fn parse_openclaw_manifest_descriptor(
    path: &Path,
) -> Result<PluginDescriptor, IntegrationError> {
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

pub(super) fn parse_openclaw_legacy_package_descriptors(
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
