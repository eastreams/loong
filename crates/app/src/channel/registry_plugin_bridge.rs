use super::*;
use std::collections::{BTreeMap, BTreeSet};

use loongclaw_kernel::{
    PluginDescriptor, PluginIR, PluginManifest, PluginScanReport, PluginScanner,
    PluginTranslationReport, PluginTranslator,
};

pub(super) fn plugin_bridge_contract_from_descriptor(
    descriptor: &ChannelRegistryDescriptor,
) -> Option<ChannelPluginBridgeContract> {
    let is_plugin_backed =
        descriptor.implementation_status == ChannelCatalogImplementationStatus::PluginBacked;
    let is_plugin_bridge =
        descriptor.onboarding.strategy == ChannelOnboardingStrategy::PluginBridge;

    if !is_plugin_backed {
        return None;
    }

    if !is_plugin_bridge {
        return None;
    }

    let supported_operations = descriptor
        .operations
        .iter()
        .map(|operation| operation.operation.id)
        .collect();
    let recommended_metadata_keys = PLUGIN_BRIDGE_RECOMMENDED_METADATA_KEYS.to_vec();
    let stable_targets =
        super::bridge::plugin_bridge_stable_targets_for_channel_id(descriptor.id).to_vec();
    let account_scope_note =
        super::bridge::plugin_bridge_account_scope_note_for_channel_id(descriptor.id);

    Some(ChannelPluginBridgeContract {
        manifest_channel_id: descriptor.id,
        required_setup_surface: PLUGIN_BRIDGE_REQUIRED_SETUP_SURFACE,
        runtime_owner: PLUGIN_BRIDGE_RUNTIME_OWNER,
        supported_operations,
        recommended_metadata_keys,
        stable_targets,
        account_scope_note,
    })
}

pub(super) fn channel_surface_plugin_bridge_discovery_by_id(
    config: &LoongClawConfig,
    channel_catalog: &[ChannelCatalogEntry],
) -> BTreeMap<&'static str, ChannelPluginBridgeDiscovery> {
    let plugin_backed_channel_ids = plugin_backed_channel_ids(channel_catalog);
    let managed_install_root = config.external_skills.resolved_install_root();
    let Some(managed_install_root) = managed_install_root else {
        return build_not_configured_discovery_by_id(&plugin_backed_channel_ids);
    };

    let managed_install_root_display = managed_install_root.display().to_string();
    let scanner = PluginScanner::new();
    let scan_result = scanner.scan_path(&managed_install_root);
    let scan_report = match scan_result {
        Ok(scan_report) => scan_report,
        Err(error) => {
            let scan_issue = error.to_string();
            return build_scan_failed_discovery_by_id(
                &plugin_backed_channel_ids,
                managed_install_root_display,
                scan_issue,
            );
        }
    };

    let translator = PluginTranslator::new();
    let translation = translator.translate_scan_report(&scan_report);
    let grouped_matches = discovered_plugin_matches_by_channel_id(
        &scan_report,
        &translation,
        &plugin_backed_channel_ids,
    );

    build_matches_discovery_by_id(
        &plugin_backed_channel_ids,
        managed_install_root_display,
        grouped_matches,
    )
}

pub fn validate_plugin_channel_bridge_manifest(
    manifest: &PluginManifest,
) -> Option<ChannelPluginBridgeManifestValidation> {
    let raw_channel_id = manifest.channel_id.as_deref();
    let declared_channel_id = normalized_manifest_channel_id(raw_channel_id)?;
    let registry_descriptor = find_channel_registry_descriptor(&declared_channel_id);

    let Some(registry_descriptor) = registry_descriptor else {
        return Some(ChannelPluginBridgeManifestValidation {
            channel_id: declared_channel_id,
            status: ChannelPluginBridgeManifestStatus::UnknownChannel,
            issues: vec!["channel registry entry is unknown".to_owned()],
            recommended_metadata_keys: Vec::new(),
        });
    };

    let resolved_channel_id = registry_descriptor.id.to_owned();
    let plugin_bridge_contract = plugin_bridge_contract_from_descriptor(registry_descriptor);

    let Some(plugin_bridge_contract) = plugin_bridge_contract else {
        return Some(ChannelPluginBridgeManifestValidation {
            channel_id: resolved_channel_id,
            status: ChannelPluginBridgeManifestStatus::UnsupportedChannelSurface,
            issues: vec!["channel does not accept external plugin bridge ownership".to_owned()],
            recommended_metadata_keys: Vec::new(),
        });
    };

    let setup_surface = normalized_manifest_setup_surface(manifest);

    let Some(setup_surface) = setup_surface else {
        return Some(ChannelPluginBridgeManifestValidation {
            channel_id: resolved_channel_id,
            status: ChannelPluginBridgeManifestStatus::MissingSetupSurface,
            issues: vec!["plugin bridge manifest must declare setup.surface".to_owned()],
            recommended_metadata_keys: plugin_bridge_contract.recommended_metadata_keys,
        });
    };

    let required_setup_surface = plugin_bridge_contract.required_setup_surface.to_owned();
    let setup_surface_matches = setup_surface == required_setup_surface;

    if !setup_surface_matches {
        let issue = format!(
            "plugin bridge manifest declares setup.surface={setup_surface}, expected {required_setup_surface}"
        );

        return Some(ChannelPluginBridgeManifestValidation {
            channel_id: resolved_channel_id,
            status: ChannelPluginBridgeManifestStatus::UnsupportedChannelSurface,
            issues: vec![issue],
            recommended_metadata_keys: plugin_bridge_contract.recommended_metadata_keys,
        });
    }

    Some(ChannelPluginBridgeManifestValidation {
        channel_id: resolved_channel_id,
        status: ChannelPluginBridgeManifestStatus::Compatible,
        issues: Vec::new(),
        recommended_metadata_keys: plugin_bridge_contract.recommended_metadata_keys,
    })
}

fn plugin_backed_channel_ids(channel_catalog: &[ChannelCatalogEntry]) -> Vec<&'static str> {
    let mut plugin_backed_channel_ids = Vec::new();

    for channel_entry in channel_catalog {
        let has_plugin_bridge_contract = channel_entry.plugin_bridge_contract.is_some();

        if !has_plugin_bridge_contract {
            continue;
        }

        plugin_backed_channel_ids.push(channel_entry.id);
    }

    plugin_backed_channel_ids
}

fn build_not_configured_discovery_by_id(
    plugin_backed_channel_ids: &[&'static str],
) -> BTreeMap<&'static str, ChannelPluginBridgeDiscovery> {
    let mut discovery_by_id = BTreeMap::new();

    for channel_id in plugin_backed_channel_ids {
        let discovery = ChannelPluginBridgeDiscovery {
            managed_install_root: None,
            status: ChannelPluginBridgeDiscoveryStatus::NotConfigured,
            scan_issue: None,
            ambiguity_status: None,
            compatible_plugins: 0,
            compatible_plugin_ids: Vec::new(),
            incomplete_plugins: 0,
            incompatible_plugins: 0,
            plugins: Vec::new(),
        };

        discovery_by_id.insert(*channel_id, discovery);
    }

    discovery_by_id
}

fn build_scan_failed_discovery_by_id(
    plugin_backed_channel_ids: &[&'static str],
    managed_install_root: String,
    scan_issue: String,
) -> BTreeMap<&'static str, ChannelPluginBridgeDiscovery> {
    let mut discovery_by_id = BTreeMap::new();

    for channel_id in plugin_backed_channel_ids {
        let discovery = ChannelPluginBridgeDiscovery {
            managed_install_root: Some(managed_install_root.clone()),
            status: ChannelPluginBridgeDiscoveryStatus::ScanFailed,
            scan_issue: Some(scan_issue.clone()),
            ambiguity_status: None,
            compatible_plugins: 0,
            compatible_plugin_ids: Vec::new(),
            incomplete_plugins: 0,
            incompatible_plugins: 0,
            plugins: Vec::new(),
        };

        discovery_by_id.insert(*channel_id, discovery);
    }

    discovery_by_id
}

fn build_matches_discovery_by_id(
    plugin_backed_channel_ids: &[&'static str],
    managed_install_root: String,
    grouped_matches: BTreeMap<&'static str, Vec<ChannelDiscoveredPluginBridge>>,
) -> BTreeMap<&'static str, ChannelPluginBridgeDiscovery> {
    let mut discovery_by_id = BTreeMap::new();

    for channel_id in plugin_backed_channel_ids {
        let grouped_plugins = grouped_matches.get(channel_id);
        let plugins = grouped_plugins.cloned().unwrap_or_default();
        let compatible_plugins = count_compatible_plugins(&plugins);
        let compatible_plugin_ids = compatible_plugin_ids(&plugins);
        let ambiguity_status = discovery_ambiguity_status(&compatible_plugin_ids);
        let incomplete_plugins = count_incomplete_plugins(&plugins);
        let incompatible_plugins = count_incompatible_plugins(&plugins);
        let has_plugins = !plugins.is_empty();
        let status = match has_plugins {
            true => ChannelPluginBridgeDiscoveryStatus::MatchesFound,
            false => ChannelPluginBridgeDiscoveryStatus::NoMatches,
        };
        let discovery = ChannelPluginBridgeDiscovery {
            managed_install_root: Some(managed_install_root.clone()),
            status,
            scan_issue: None,
            ambiguity_status,
            compatible_plugins,
            compatible_plugin_ids,
            incomplete_plugins,
            incompatible_plugins,
            plugins,
        };

        discovery_by_id.insert(*channel_id, discovery);
    }

    discovery_by_id
}

fn count_compatible_plugins(plugins: &[ChannelDiscoveredPluginBridge]) -> usize {
    let mut compatible_plugins = 0;

    for plugin in plugins {
        let is_compatible = plugin.status == ChannelDiscoveredPluginBridgeStatus::CompatibleReady;

        if !is_compatible {
            continue;
        }

        compatible_plugins += 1;
    }

    compatible_plugins
}

fn compatible_plugin_ids(plugins: &[ChannelDiscoveredPluginBridge]) -> Vec<String> {
    let mut compatible_plugin_ids = Vec::new();

    for plugin in plugins {
        let is_compatible = plugin.status == ChannelDiscoveredPluginBridgeStatus::CompatibleReady;

        if !is_compatible {
            continue;
        }

        compatible_plugin_ids.push(plugin.plugin_id.clone());
    }

    compatible_plugin_ids
}

fn discovery_ambiguity_status(
    compatible_plugin_ids: &[String],
) -> Option<ChannelPluginBridgeDiscoveryAmbiguityStatus> {
    let has_multiple_compatible_plugins = compatible_plugin_ids.len() > 1;

    if !has_multiple_compatible_plugins {
        return None;
    }

    Some(ChannelPluginBridgeDiscoveryAmbiguityStatus::MultipleCompatiblePlugins)
}

fn count_incomplete_plugins(plugins: &[ChannelDiscoveredPluginBridge]) -> usize {
    let mut incomplete_plugins = 0;

    for plugin in plugins {
        let is_incomplete = matches!(
            plugin.status,
            ChannelDiscoveredPluginBridgeStatus::CompatibleIncompleteContract
                | ChannelDiscoveredPluginBridgeStatus::MissingSetupSurface
        );

        if !is_incomplete {
            continue;
        }

        incomplete_plugins += 1;
    }

    incomplete_plugins
}

fn count_incompatible_plugins(plugins: &[ChannelDiscoveredPluginBridge]) -> usize {
    let mut incompatible_plugins = 0;

    for plugin in plugins {
        let is_incompatible =
            plugin.status == ChannelDiscoveredPluginBridgeStatus::UnsupportedChannelSurface;

        if !is_incompatible {
            continue;
        }

        incompatible_plugins += 1;
    }

    incompatible_plugins
}

fn discovered_plugin_matches_by_channel_id(
    scan_report: &PluginScanReport,
    translation: &PluginTranslationReport,
    plugin_backed_channel_ids: &[&'static str],
) -> BTreeMap<&'static str, Vec<ChannelDiscoveredPluginBridge>> {
    let plugin_backed_channel_id_set = plugin_backed_channel_id_set(plugin_backed_channel_ids);
    let translation_by_key = translation_entries_by_key(translation);
    let mut grouped_matches = BTreeMap::new();

    for descriptor in &scan_report.descriptors {
        let validation = validate_plugin_channel_bridge_manifest(&descriptor.manifest);
        let Some(validation) = validation else {
            continue;
        };

        let resolved_channel_id = normalize_channel_catalog_id(&validation.channel_id);
        let Some(resolved_channel_id) = resolved_channel_id else {
            continue;
        };

        let channel_is_plugin_backed = plugin_backed_channel_id_set.contains(&resolved_channel_id);

        if !channel_is_plugin_backed {
            continue;
        }

        let translation_key = plugin_translation_key(descriptor);
        let translation_entry = translation_by_key.get(&translation_key).copied();
        let match_entry =
            discovered_plugin_match_from_descriptor(descriptor, translation_entry, validation);

        grouped_matches
            .entry(resolved_channel_id)
            .or_insert_with(Vec::new)
            .push(match_entry);
    }

    grouped_matches
}

fn plugin_backed_channel_id_set(
    plugin_backed_channel_ids: &[&'static str],
) -> BTreeSet<&'static str> {
    let mut plugin_backed_channel_id_set = BTreeSet::new();

    for channel_id in plugin_backed_channel_ids {
        plugin_backed_channel_id_set.insert(*channel_id);
    }

    plugin_backed_channel_id_set
}

fn translation_entries_by_key(
    translation: &PluginTranslationReport,
) -> BTreeMap<(String, String), &PluginIR> {
    let mut translation_by_key = BTreeMap::new();

    for entry in &translation.entries {
        let key = (entry.source_path.clone(), entry.plugin_id.clone());
        translation_by_key.insert(key, entry);
    }

    translation_by_key
}

fn plugin_translation_key(descriptor: &PluginDescriptor) -> (String, String) {
    let source_path = descriptor.path.clone();
    let plugin_id = descriptor.manifest.plugin_id.clone();

    (source_path, plugin_id)
}

fn discovered_plugin_match_from_descriptor(
    descriptor: &PluginDescriptor,
    translation_entry: Option<&PluginIR>,
    validation: ChannelPluginBridgeManifestValidation,
) -> ChannelDiscoveredPluginBridge {
    let channel_bridge = translation_entry.and_then(plugin_ir_channel_bridge);
    let runtime_bridge_kind = translation_entry.map(plugin_ir_bridge_kind);
    let bridge_kind = runtime_bridge_kind
        .map(plugin_bridge_kind_label)
        .unwrap_or_else(|| "unknown".to_owned());
    let runtime_adapter_family = translation_entry.map(plugin_ir_adapter_family);
    let manifest_adapter_family = descriptor.manifest.metadata.get("adapter_family").cloned();
    let adapter_family = runtime_adapter_family
        .or(manifest_adapter_family)
        .unwrap_or_else(|| "unknown".to_owned());
    let transport_family = channel_bridge_transport_family(channel_bridge);
    let target_contract = channel_bridge_target_contract(channel_bridge);
    let account_scope = channel_bridge_account_scope(channel_bridge);
    let missing_fields = channel_bridge_missing_fields(channel_bridge);
    let setup_details = plugin_bridge_setup_details(&descriptor.manifest);
    let manifest_status = validation.status;
    let status = discovered_plugin_bridge_status_from_validation(manifest_status, channel_bridge);

    ChannelDiscoveredPluginBridge {
        plugin_id: descriptor.manifest.plugin_id.clone(),
        source_path: descriptor.path.clone(),
        package_root: descriptor.package_root.clone(),
        package_manifest_path: descriptor.package_manifest_path.clone(),
        bridge_kind,
        adapter_family,
        transport_family,
        target_contract,
        account_scope,
        status,
        issues: validation.issues,
        missing_fields,
        required_env_vars: setup_details.required_env_vars,
        recommended_env_vars: setup_details.recommended_env_vars,
        required_config_keys: setup_details.required_config_keys,
        default_env_var: setup_details.default_env_var,
        setup_docs_urls: setup_details.setup_docs_urls,
        setup_remediation: setup_details.setup_remediation,
    }
}

#[derive(Debug, Default)]
struct PluginBridgeSetupDetails {
    required_env_vars: Vec<String>,
    recommended_env_vars: Vec<String>,
    required_config_keys: Vec<String>,
    default_env_var: Option<String>,
    setup_docs_urls: Vec<String>,
    setup_remediation: Option<String>,
}

fn plugin_bridge_setup_details(manifest: &PluginManifest) -> PluginBridgeSetupDetails {
    let Some(setup) = manifest.setup.as_ref() else {
        return PluginBridgeSetupDetails::default();
    };

    let required_env_vars = setup.required_env_vars.clone();
    let recommended_env_vars = setup.recommended_env_vars.clone();
    let required_config_keys = setup.required_config_keys.clone();
    let default_env_var = setup.default_env_var.clone();
    let setup_docs_urls = setup.docs_urls.clone();
    let setup_remediation = setup.remediation.clone();

    PluginBridgeSetupDetails {
        required_env_vars,
        recommended_env_vars,
        required_config_keys,
        default_env_var,
        setup_docs_urls,
        setup_remediation,
    }
}

fn discovered_plugin_bridge_status_from_validation(
    manifest_status: ChannelPluginBridgeManifestStatus,
    channel_bridge: Option<&loongclaw_kernel::PluginChannelBridgeContract>,
) -> ChannelDiscoveredPluginBridgeStatus {
    match manifest_status {
        ChannelPluginBridgeManifestStatus::Compatible => {
            let contract_is_ready = match channel_bridge {
                Some(channel_bridge) => channel_bridge.readiness.ready,
                None => true,
            };

            if contract_is_ready {
                return ChannelDiscoveredPluginBridgeStatus::CompatibleReady;
            }

            ChannelDiscoveredPluginBridgeStatus::CompatibleIncompleteContract
        }
        ChannelPluginBridgeManifestStatus::MissingSetupSurface => {
            ChannelDiscoveredPluginBridgeStatus::MissingSetupSurface
        }
        ChannelPluginBridgeManifestStatus::UnsupportedChannelSurface
        | ChannelPluginBridgeManifestStatus::UnknownChannel => {
            ChannelDiscoveredPluginBridgeStatus::UnsupportedChannelSurface
        }
    }
}

fn plugin_ir_channel_bridge(
    plugin_ir: &PluginIR,
) -> Option<&loongclaw_kernel::PluginChannelBridgeContract> {
    plugin_ir.channel_bridge.as_ref()
}

fn plugin_ir_bridge_kind(plugin_ir: &PluginIR) -> loongclaw_kernel::PluginBridgeKind {
    plugin_ir.runtime.bridge_kind
}

fn plugin_bridge_kind_label(bridge_kind: loongclaw_kernel::PluginBridgeKind) -> String {
    bridge_kind.as_str().to_owned()
}

fn plugin_ir_adapter_family(plugin_ir: &PluginIR) -> String {
    plugin_ir.runtime.adapter_family.clone()
}

fn channel_bridge_transport_family(
    channel_bridge: Option<&loongclaw_kernel::PluginChannelBridgeContract>,
) -> Option<String> {
    let channel_bridge = channel_bridge?;

    channel_bridge.transport_family.clone()
}

fn channel_bridge_target_contract(
    channel_bridge: Option<&loongclaw_kernel::PluginChannelBridgeContract>,
) -> Option<String> {
    let channel_bridge = channel_bridge?;

    channel_bridge.target_contract.clone()
}

fn channel_bridge_account_scope(
    channel_bridge: Option<&loongclaw_kernel::PluginChannelBridgeContract>,
) -> Option<String> {
    let channel_bridge = channel_bridge?;

    channel_bridge.account_scope.clone()
}

fn channel_bridge_missing_fields(
    channel_bridge: Option<&loongclaw_kernel::PluginChannelBridgeContract>,
) -> Vec<String> {
    let Some(channel_bridge) = channel_bridge else {
        return Vec::new();
    };

    channel_bridge.readiness.missing_fields.clone()
}

fn normalized_manifest_channel_id(raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_ascii_lowercase())
}

fn normalized_manifest_setup_surface(
    manifest: &loongclaw_kernel::PluginManifest,
) -> Option<String> {
    let setup = manifest.setup.as_ref()?;
    let surface = setup.surface.as_deref()?;
    let trimmed = surface.trim();

    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_ascii_lowercase())
}
