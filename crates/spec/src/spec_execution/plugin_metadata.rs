use kernel::PluginDescriptor;

use super::*;

#[derive(Clone)]
pub(super) struct PluginTranslationMetadataSnapshot {
    bridge_kind: String,
    adapter_family: String,
    entrypoint_hint: String,
    source_language: String,
    channel_id: Option<String>,
    channel_bridge_transport_family: Option<String>,
    channel_bridge_target_contract: Option<String>,
    channel_bridge_account_scope: Option<String>,
    channel_bridge_ready: Option<bool>,
    channel_bridge_missing_fields: Vec<String>,
}

pub(super) fn enrich_scan_report_with_translation(
    report: &PluginScanReport,
    translation: &PluginTranslationReport,
    activation: Option<&PluginActivationPlan>,
) -> PluginScanReport {
    let mut runtime_by_key: BTreeMap<(String, String), PluginTranslationMetadataSnapshot> =
        BTreeMap::new();
    let mut activation_contracts_by_key: BTreeMap<
        (String, String),
        PluginActivationRuntimeContract,
    > = BTreeMap::new();

    for entry in &translation.entries {
        let channel_bridge = entry.channel_bridge.as_ref();
        let channel_id = channel_bridge
            .and_then(|bridge| bridge.channel_id.clone())
            .or_else(|| entry.channel_id.clone());
        let channel_bridge_transport_family =
            channel_bridge.and_then(|bridge| bridge.transport_family.clone());
        let channel_bridge_target_contract =
            channel_bridge.and_then(|bridge| bridge.target_contract.clone());
        let channel_bridge_account_scope =
            channel_bridge.and_then(|bridge| bridge.account_scope.clone());
        let channel_bridge_ready = channel_bridge.map(|bridge| bridge.readiness.ready);
        let channel_bridge_missing_fields = channel_bridge
            .map(|bridge| bridge.readiness.missing_fields.clone())
            .unwrap_or_default();

        runtime_by_key.insert(
            (entry.source_path.clone(), entry.plugin_id.clone()),
            PluginTranslationMetadataSnapshot {
                bridge_kind: entry.runtime.bridge_kind.as_str().to_owned(),
                adapter_family: entry.runtime.adapter_family.clone(),
                entrypoint_hint: entry.runtime.entrypoint_hint.clone(),
                source_language: entry.runtime.source_language.clone(),
                channel_id,
                channel_bridge_transport_family,
                channel_bridge_target_contract,
                channel_bridge_account_scope,
                channel_bridge_ready,
                channel_bridge_missing_fields,
            },
        );
    }

    if let Some(activation) = activation {
        for entry in &translation.entries {
            let Some(candidate) = activation.candidate_for(&entry.source_path, &entry.plugin_id)
            else {
                continue;
            };
            if !matches!(candidate.status, PluginActivationStatus::Ready) {
                continue;
            }

            activation_contracts_by_key.insert(
                (entry.source_path.clone(), entry.plugin_id.clone()),
                PluginActivationRuntimeContract {
                    plugin_id: entry.plugin_id.clone(),
                    source_path: entry.source_path.clone(),
                    source_kind: entry.source_kind,
                    dialect: entry.dialect,
                    dialect_version: entry.dialect_version.clone(),
                    compatibility_mode: entry.compatibility_mode,
                    compatibility_shim: candidate.compatibility_shim.clone(),
                    bridge_kind: entry.runtime.bridge_kind,
                    adapter_family: entry.runtime.adapter_family.clone(),
                    entrypoint_hint: entry.runtime.entrypoint_hint.clone(),
                    source_language: entry.runtime.source_language.clone(),
                    compatibility: entry.compatibility.clone(),
                },
            );
        }
    }

    let descriptors: Vec<PluginDescriptor> = report
        .descriptors
        .iter()
        .cloned()
        .map(|mut descriptor| {
            stamp_plugin_provenance_metadata(&mut descriptor);
            descriptor.manifest.metadata.insert(
                "plugin_id".to_owned(),
                descriptor.manifest.plugin_id.clone(),
            );
            descriptor.manifest.metadata.insert(
                "defer_loading".to_owned(),
                descriptor.manifest.defer_loading.to_string(),
            );
            let setup = descriptor.manifest.setup.clone();
            insert_plugin_setup_metadata(&mut descriptor.manifest.metadata, setup.as_ref());
            insert_plugin_slot_claims_metadata(
                &mut descriptor.manifest.metadata,
                &descriptor.manifest.slot_claims,
            );
            let manifest_api_version = descriptor.manifest.api_version.clone();
            let plugin_version = descriptor.manifest.version.clone();
            insert_plugin_manifest_contract_metadata(
                &mut descriptor.manifest.metadata,
                manifest_api_version,
                plugin_version,
            );
            insert_plugin_compatibility_metadata(
                &mut descriptor.manifest.metadata,
                descriptor.manifest.compatibility.as_ref(),
            );
            if let Some(summary) = descriptor.manifest.summary.clone() {
                descriptor
                    .manifest
                    .metadata
                    .entry("summary".to_owned())
                    .or_insert(summary);
            }
            if !descriptor.manifest.tags.is_empty()
                && let Ok(tags_json) = serde_json::to_string(&descriptor.manifest.tags)
            {
                descriptor
                    .manifest
                    .metadata
                    .entry("tags_json".to_owned())
                    .or_insert(tags_json);
            }
            if !descriptor.manifest.input_examples.is_empty()
                && let Ok(input_examples_json) =
                    serde_json::to_string(&descriptor.manifest.input_examples)
            {
                descriptor
                    .manifest
                    .metadata
                    .entry("input_examples_json".to_owned())
                    .or_insert(input_examples_json);
            }
            if !descriptor.manifest.output_examples.is_empty()
                && let Ok(output_examples_json) =
                    serde_json::to_string(&descriptor.manifest.output_examples)
            {
                descriptor
                    .manifest
                    .metadata
                    .entry("output_examples_json".to_owned())
                    .or_insert(output_examples_json);
            }
            if let Some(component) = descriptor.manifest.metadata.get("component").cloned() {
                let resolved = resolve_plugin_relative_path(&descriptor.path, &component);
                let normalized = normalize_path_for_policy(&resolved);
                descriptor.manifest.metadata.insert(
                    "component_resolved_path".to_owned(),
                    normalized.display().to_string(),
                );
            } else {
                descriptor
                    .manifest
                    .metadata
                    .remove("component_resolved_path");
            }

            if let Some(runtime_snapshot) = runtime_by_key.get(&(
                descriptor.path.clone(),
                descriptor.manifest.plugin_id.clone(),
            )) {
                let bridge_kind = runtime_snapshot.bridge_kind.clone();
                let adapter_family = runtime_snapshot.adapter_family.clone();
                let entrypoint_hint = runtime_snapshot.entrypoint_hint.clone();
                let source_language = runtime_snapshot.source_language.clone();
                descriptor
                    .manifest
                    .metadata
                    .entry("bridge_kind".to_owned())
                    .or_insert(bridge_kind);
                descriptor
                    .manifest
                    .metadata
                    .entry("adapter_family".to_owned())
                    .or_insert(adapter_family);
                descriptor
                    .manifest
                    .metadata
                    .entry("entrypoint_hint".to_owned())
                    .or_insert(entrypoint_hint);
                descriptor
                    .manifest
                    .metadata
                    .entry("source_language".to_owned())
                    .or_insert(source_language);

                insert_plugin_channel_bridge_metadata(
                    &mut descriptor.manifest.metadata,
                    Some(runtime_snapshot),
                );
            } else {
                insert_plugin_channel_bridge_metadata(&mut descriptor.manifest.metadata, None);
            }
            insert_plugin_activation_runtime_contract_metadata(
                &mut descriptor.manifest.metadata,
                activation_contracts_by_key.get(&(
                    descriptor.path.clone(),
                    descriptor.manifest.plugin_id.clone(),
                )),
            );
            descriptor
        })
        .collect();

    PluginScanReport {
        scanned_files: report.scanned_files,
        matched_plugins: descriptors.len(),
        diagnostic_findings: report.diagnostic_findings.clone(),
        descriptors,
    }
}

fn insert_plugin_activation_runtime_contract_metadata(
    metadata: &mut BTreeMap<String, String>,
    contract: Option<&PluginActivationRuntimeContract>,
) {
    let Some(contract) = contract else {
        metadata.remove(PLUGIN_ACTIVATION_RUNTIME_CONTRACT_METADATA_KEY);
        metadata.remove(PLUGIN_ACTIVATION_RUNTIME_CONTRACT_CHECKSUM_METADATA_KEY);
        return;
    };

    let Ok(serialized) = plugin_activation_runtime_contract_json(contract) else {
        metadata.remove(PLUGIN_ACTIVATION_RUNTIME_CONTRACT_METADATA_KEY);
        metadata.remove(PLUGIN_ACTIVATION_RUNTIME_CONTRACT_CHECKSUM_METADATA_KEY);
        return;
    };

    metadata.insert(
        PLUGIN_ACTIVATION_RUNTIME_CONTRACT_METADATA_KEY.to_owned(),
        serialized.clone(),
    );
    metadata.insert(
        PLUGIN_ACTIVATION_RUNTIME_CONTRACT_CHECKSUM_METADATA_KEY.to_owned(),
        activation_runtime_contract_checksum_hex(serialized.as_bytes()),
    );
}

fn stamp_plugin_provenance_metadata(descriptor: &mut PluginDescriptor) {
    let source_path_key = "plugin_source_path".to_owned();
    let source_path_value = descriptor.path.clone();
    let source_kind_key = "plugin_source_kind".to_owned();
    let source_kind_value = descriptor.source_kind.as_str().to_owned();
    let dialect_key = "plugin_dialect".to_owned();
    let dialect_value = descriptor.dialect.as_str().to_owned();
    let compatibility_mode_key = "plugin_compatibility_mode".to_owned();
    let compatibility_mode_value = descriptor.compatibility_mode.as_str().to_owned();
    let package_root_key = "plugin_package_root".to_owned();
    let package_root_value = descriptor.package_root.clone();
    let package_manifest_path_value = descriptor.package_manifest_path.clone();
    let provenance_summary_key = "plugin_provenance_summary".to_owned();
    let provenance_summary_value = plugin_provenance_summary_for_descriptor(descriptor);
    let trust_tier_key = "plugin_trust_tier".to_owned();
    let trust_tier_value = descriptor.manifest.trust_tier.as_str().to_owned();
    let metadata = &mut descriptor.manifest.metadata;

    metadata.insert(source_path_key, source_path_value);
    metadata.insert(source_kind_key, source_kind_value);
    metadata.insert(dialect_key, dialect_value);
    metadata.insert(compatibility_mode_key, compatibility_mode_value);
    metadata.insert(package_root_key, package_root_value);
    metadata.insert(provenance_summary_key, provenance_summary_value);
    metadata.insert(trust_tier_key, trust_tier_value);

    if let Some(shim) = kernel::PluginCompatibilityShim::for_mode(descriptor.compatibility_mode) {
        metadata.insert("plugin_compatibility_shim_id".to_owned(), shim.shim_id);
        metadata.insert("plugin_compatibility_shim_family".to_owned(), shim.family);
    } else {
        metadata.remove("plugin_compatibility_shim_id");
        metadata.remove("plugin_compatibility_shim_family");
    }

    if let Some(dialect_version) = descriptor.dialect_version.clone() {
        metadata.insert("plugin_dialect_version".to_owned(), dialect_version);
    } else {
        metadata.remove("plugin_dialect_version");
    }

    if let Some(package_manifest_path_value) = package_manifest_path_value {
        let package_manifest_path_key = "plugin_package_manifest_path".to_owned();

        metadata.insert(package_manifest_path_key, package_manifest_path_value);
    } else {
        metadata.remove("plugin_package_manifest_path");
    }
}

fn insert_plugin_setup_metadata(
    metadata: &mut BTreeMap<String, String>,
    setup: Option<&PluginSetup>,
) {
    let Some(setup) = setup else {
        metadata.remove("plugin_setup_mode");
        metadata.remove("plugin_setup_surface");
        metadata.remove("plugin_setup_required_env_vars_json");
        metadata.remove("plugin_setup_recommended_env_vars_json");
        metadata.remove("plugin_setup_required_config_keys_json");
        metadata.remove("plugin_setup_default_env_var");
        metadata.remove("plugin_setup_docs_urls_json");
        metadata.remove("plugin_setup_remediation");
        return;
    };

    let mode_key = "plugin_setup_mode".to_owned();
    let mode_value = setup.mode.as_str().to_owned();
    metadata.insert(mode_key, mode_value);

    if let Some(surface) = setup.surface.clone() {
        let surface_key = "plugin_setup_surface".to_owned();
        metadata.insert(surface_key, surface);
    } else {
        metadata.remove("plugin_setup_surface");
    }

    insert_plugin_setup_string_list_metadata(
        metadata,
        "plugin_setup_required_env_vars_json",
        &setup.required_env_vars,
    );
    insert_plugin_setup_string_list_metadata(
        metadata,
        "plugin_setup_recommended_env_vars_json",
        &setup.recommended_env_vars,
    );
    insert_plugin_setup_string_list_metadata(
        metadata,
        "plugin_setup_required_config_keys_json",
        &setup.required_config_keys,
    );

    if let Some(default_env_var) = setup.default_env_var.clone() {
        let default_env_var_key = "plugin_setup_default_env_var".to_owned();
        metadata.insert(default_env_var_key, default_env_var);
    } else {
        metadata.remove("plugin_setup_default_env_var");
    }

    insert_plugin_setup_string_list_metadata(
        metadata,
        "plugin_setup_docs_urls_json",
        &setup.docs_urls,
    );

    if let Some(remediation) = setup.remediation.clone() {
        let remediation_key = "plugin_setup_remediation".to_owned();
        metadata.insert(remediation_key, remediation);
    } else {
        metadata.remove("plugin_setup_remediation");
    }
}

fn insert_plugin_channel_bridge_metadata(
    metadata: &mut BTreeMap<String, String>,
    snapshot: Option<&PluginTranslationMetadataSnapshot>,
) {
    let Some(snapshot) = snapshot else {
        remove_plugin_channel_bridge_metadata(metadata);
        return;
    };

    upsert_or_remove_metadata_value(metadata, "plugin_channel_id", snapshot.channel_id.as_ref());
    upsert_or_remove_metadata_value(
        metadata,
        "plugin_channel_bridge_transport_family",
        snapshot.channel_bridge_transport_family.as_ref(),
    );
    upsert_or_remove_metadata_value(
        metadata,
        "plugin_channel_bridge_target_contract",
        snapshot.channel_bridge_target_contract.as_ref(),
    );
    upsert_or_remove_metadata_value(
        metadata,
        "plugin_channel_bridge_account_scope",
        snapshot.channel_bridge_account_scope.as_ref(),
    );

    if let Some(channel_bridge_ready) = snapshot.channel_bridge_ready {
        let ready_key = "plugin_channel_bridge_ready".to_owned();
        let ready_value = channel_bridge_ready.to_string();
        metadata.insert(ready_key, ready_value);
    } else {
        metadata.remove("plugin_channel_bridge_ready");
    }

    upsert_or_remove_json_string_list_metadata(
        metadata,
        "plugin_channel_bridge_missing_fields_json",
        &snapshot.channel_bridge_missing_fields,
    );
}

fn remove_plugin_channel_bridge_metadata(metadata: &mut BTreeMap<String, String>) {
    metadata.remove("plugin_channel_id");
    metadata.remove("plugin_channel_bridge_transport_family");
    metadata.remove("plugin_channel_bridge_target_contract");
    metadata.remove("plugin_channel_bridge_account_scope");
    metadata.remove("plugin_channel_bridge_ready");
    metadata.remove("plugin_channel_bridge_missing_fields_json");
}

fn upsert_or_remove_metadata_value(
    metadata: &mut BTreeMap<String, String>,
    key: &str,
    value: Option<&String>,
) {
    let Some(value) = value else {
        metadata.remove(key);
        return;
    };

    let metadata_key = key.to_owned();
    let metadata_value = value.clone();
    metadata.insert(metadata_key, metadata_value);
}

fn upsert_or_remove_json_string_list_metadata(
    metadata: &mut BTreeMap<String, String>,
    key: &str,
    values: &[String],
) {
    let serialized = serde_json::to_string(values);
    let Ok(serialized) = serialized else {
        metadata.remove(key);
        return;
    };

    let metadata_key = key.to_owned();
    metadata.insert(metadata_key, serialized);
}

fn insert_plugin_setup_string_list_metadata(
    metadata: &mut BTreeMap<String, String>,
    key: &str,
    values: &[String],
) {
    if values.is_empty() {
        metadata.remove(key);
        return;
    }

    let serialized = serde_json::to_string(values);
    let Ok(serialized) = serialized else {
        metadata.remove(key);
        return;
    };

    let metadata_key = key.to_owned();
    metadata.insert(metadata_key, serialized);
}

fn insert_plugin_slot_claims_metadata(
    metadata: &mut BTreeMap<String, String>,
    slot_claims: &[PluginSlotClaim],
) {
    if slot_claims.is_empty() {
        metadata.remove("plugin_slot_claims_json");
        return;
    }

    if let Ok(serialized) = serde_json::to_string(slot_claims) {
        metadata.insert("plugin_slot_claims_json".to_owned(), serialized);
    }
}

fn insert_plugin_manifest_contract_metadata(
    metadata: &mut BTreeMap<String, String>,
    manifest_api_version: Option<String>,
    plugin_version: Option<String>,
) {
    if let Some(api_version) = manifest_api_version {
        metadata.insert("plugin_manifest_api_version".to_owned(), api_version);
    } else {
        metadata.remove("plugin_manifest_api_version");
    }

    if let Some(version) = plugin_version {
        metadata.insert("plugin_version".to_owned(), version);
    } else {
        metadata.remove("plugin_version");
    }
}

fn insert_plugin_compatibility_metadata(
    metadata: &mut BTreeMap<String, String>,
    compatibility: Option<&PluginCompatibility>,
) {
    let Some(compatibility) = compatibility else {
        metadata.remove("plugin_compatibility_host_api");
        metadata.remove("plugin_compatibility_host_version_req");
        return;
    };

    if let Some(host_api) = compatibility.host_api.clone() {
        metadata.insert("plugin_compatibility_host_api".to_owned(), host_api);
    } else {
        metadata.remove("plugin_compatibility_host_api");
    }

    if let Some(host_version_req) = compatibility.host_version_req.clone() {
        metadata.insert(
            "plugin_compatibility_host_version_req".to_owned(),
            host_version_req,
        );
    } else {
        metadata.remove("plugin_compatibility_host_version_req");
    }
}
