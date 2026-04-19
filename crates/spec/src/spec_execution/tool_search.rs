use std::collections::{BTreeMap, BTreeSet};

use kernel::{
    IntegrationCatalog, PluginActivationCandidate, PluginActivationInventoryEntry,
    PluginActivationPlan, PluginBridgeKind, PluginCompatibility, PluginCompatibilityMode,
    PluginCompatibilityShim, PluginContractDialect, PluginDiagnosticFinding, PluginScanReport,
    PluginSetupReadinessContext, PluginSlotClaim, PluginTranslationReport, PluginTrustTier,
    evaluate_plugin_setup_requirements, plugin_provenance_summary_for_descriptor,
};
use serde_json::Value;

use super::descriptor_bridge_kind;
use crate::spec_runtime::{
    ToolSearchEntry, ToolSearchResult, ToolSearchTrustFilterSummary, detect_provider_bridge_kind,
    provider_plugin_activation_attestation_result,
};

#[derive(Debug)]
pub(super) struct ToolSearchExecutionReport {
    pub results: Vec<ToolSearchResult>,
    pub trust_filter_summary: ToolSearchTrustFilterSummary,
}

#[derive(Clone)]
struct ToolSearchTranslationSnapshot {
    bridge_kind: PluginBridgeKind,
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

pub(super) fn execute_tool_search(
    integration_catalog: &IntegrationCatalog,
    plugin_scan_reports: &[PluginScanReport],
    plugin_translation_reports: &[PluginTranslationReport],
    setup_readiness_context: &PluginSetupReadinessContext,
    plugin_activation_plans: &[PluginActivationPlan],
    query: &str,
    limit: usize,
    trust_tiers: &[PluginTrustTier],
    include_deferred: bool,
    include_examples: bool,
) -> ToolSearchExecutionReport {
    let mut entries: BTreeMap<String, ToolSearchEntry> = BTreeMap::new();
    let mut translation_by_key: BTreeMap<(String, String), ToolSearchTranslationSnapshot> =
        BTreeMap::new();
    let mut activation_candidate_by_key: BTreeMap<(String, String), PluginActivationCandidate> =
        BTreeMap::new();
    let mut activation_by_key: BTreeMap<(String, String), (String, String)> = BTreeMap::new();
    let mut activation_diagnostics_by_key: BTreeMap<
        (String, String),
        Vec<PluginDiagnosticFinding>,
    > = BTreeMap::new();
    let mut activation_inventory_by_key: BTreeMap<
        (String, String),
        PluginActivationInventoryEntry,
    > = BTreeMap::new();
    let mut scan_diagnostics_by_key: BTreeMap<(String, String), Vec<PluginDiagnosticFinding>> =
        BTreeMap::new();

    for report in plugin_translation_reports {
        for entry in &report.entries {
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

            translation_by_key.insert(
                (entry.source_path.clone(), entry.plugin_id.clone()),
                ToolSearchTranslationSnapshot {
                    bridge_kind: entry.runtime.bridge_kind,
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
    }

    for plan in plugin_activation_plans {
        for candidate in &plan.candidates {
            activation_candidate_by_key.insert(
                (candidate.source_path.clone(), candidate.plugin_id.clone()),
                candidate.clone(),
            );
            activation_by_key.insert(
                (candidate.source_path.clone(), candidate.plugin_id.clone()),
                (
                    candidate.status.as_str().to_owned(),
                    candidate.reason.clone(),
                ),
            );
            activation_diagnostics_by_key.insert(
                (candidate.source_path.clone(), candidate.plugin_id.clone()),
                candidate.diagnostic_findings.clone(),
            );
        }
    }

    for (translation, plan) in plugin_translation_reports
        .iter()
        .zip(plugin_activation_plans.iter())
    {
        for entry in plan.inventory_entries(translation) {
            activation_inventory_by_key
                .insert((entry.source_path.clone(), entry.plugin_id.clone()), entry);
        }
    }

    for report in plugin_scan_reports {
        for finding in &report.diagnostic_findings {
            let (Some(source_path), Some(plugin_id)) =
                (finding.source_path.clone(), finding.plugin_id.clone())
            else {
                continue;
            };
            scan_diagnostics_by_key
                .entry((source_path, plugin_id))
                .or_default()
                .push(finding.clone());
        }
    }

    for provider in integration_catalog.providers() {
        let channel_endpoint = integration_catalog
            .channels_for_provider(&provider.provider_id)
            .into_iter()
            .find(|channel| channel.enabled)
            .map(|channel| channel.endpoint)
            .unwrap_or_default();
        let bridge_kind = detect_provider_bridge_kind(&provider, &channel_endpoint);
        let tool_id = format!("{}::{}", provider.provider_id, provider.connector_name);
        let summary = provider.metadata.get("summary").cloned();
        let tags = metadata_tags(&provider.metadata);
        let input_examples = metadata_examples(&provider.metadata, "input_examples_json");
        let output_examples = metadata_examples(&provider.metadata, "output_examples_json");
        let deferred = metadata_bool(&provider.metadata, "defer_loading").unwrap_or(false);
        let setup_mode = provider.metadata.get("plugin_setup_mode").cloned();
        let setup_surface = provider.metadata.get("plugin_setup_surface").cloned();
        let setup_required_env_vars =
            metadata_strings(&provider.metadata, "plugin_setup_required_env_vars_json");
        let setup_recommended_env_vars =
            metadata_strings(&provider.metadata, "plugin_setup_recommended_env_vars_json");
        let setup_required_config_keys =
            metadata_strings(&provider.metadata, "plugin_setup_required_config_keys_json");
        let setup_default_env_var = provider
            .metadata
            .get("plugin_setup_default_env_var")
            .cloned();
        let setup_docs_urls = metadata_strings(&provider.metadata, "plugin_setup_docs_urls_json");
        let setup_remediation = provider.metadata.get("plugin_setup_remediation").cloned();
        let provenance_summary = provider.metadata.get("plugin_provenance_summary").cloned();
        let trust_tier = provider.metadata.get("plugin_trust_tier").cloned();
        let slot_claims = metadata_slot_claims(&provider.metadata);
        let mut manifest_api_version =
            metadata_optional_string(&provider.metadata, "plugin_manifest_api_version");
        let mut plugin_version = metadata_optional_string(&provider.metadata, "plugin_version")
            .or_else(|| metadata_optional_string(&provider.metadata, "version"));
        let mut dialect = metadata_plugin_dialect(&provider.metadata, "plugin_dialect");
        let mut dialect_version =
            metadata_optional_string(&provider.metadata, "plugin_dialect_version");
        let mut compatibility_mode =
            metadata_plugin_compatibility_mode(&provider.metadata, "plugin_compatibility_mode");
        let mut compatibility_shim = metadata_plugin_compatibility_shim(&provider.metadata)
            .or_else(|| compatibility_mode.and_then(PluginCompatibilityShim::for_mode));
        let mut compatibility_shim_support = None;
        let mut compatibility_shim_support_mismatch_reasons = Vec::new();
        let mut compatibility = metadata_plugin_compatibility(&provider.metadata);
        let mut activation_status = None;
        let mut activation_reason = None;
        let mut diagnostic_findings = Vec::new();
        let mut channel_id = provider.metadata.get("plugin_channel_id").cloned();
        let mut channel_bridge_transport_family = provider
            .metadata
            .get("plugin_channel_bridge_transport_family")
            .cloned();
        let mut channel_bridge_target_contract = provider
            .metadata
            .get("plugin_channel_bridge_target_contract")
            .cloned();
        let mut channel_bridge_account_scope = provider
            .metadata
            .get("plugin_channel_bridge_account_scope")
            .cloned();
        let mut channel_bridge_ready =
            metadata_bool(&provider.metadata, "plugin_channel_bridge_ready");
        let mut channel_bridge_missing_fields = metadata_strings(
            &provider.metadata,
            "plugin_channel_bridge_missing_fields_json",
        );
        let mut adapter_family = provider.metadata.get("adapter_family").cloned();
        let mut entrypoint_hint = provider
            .metadata
            .get("entrypoint")
            .or_else(|| provider.metadata.get("entrypoint_hint"))
            .cloned();
        let mut source_language = provider.metadata.get("source_language").cloned();
        let mut resolved_bridge_kind = bridge_kind;

        let source_path = provider.metadata.get("plugin_source_path");
        let plugin_id = provider.metadata.get("plugin_id");

        if let (Some(source_path), Some(plugin_id)) = (source_path, plugin_id) {
            let translation_key = (source_path.clone(), plugin_id.clone());
            let snapshot = translation_by_key.get(&translation_key);

            if let Some(snapshot) = snapshot {
                resolved_bridge_kind = snapshot.bridge_kind;
                adapter_family = Some(snapshot.adapter_family.clone());
                entrypoint_hint = Some(snapshot.entrypoint_hint.clone());
                source_language = Some(snapshot.source_language.clone());
                channel_id = snapshot.channel_id.clone().or(channel_id);
                channel_bridge_transport_family = snapshot
                    .channel_bridge_transport_family
                    .clone()
                    .or(channel_bridge_transport_family);
                channel_bridge_target_contract = snapshot
                    .channel_bridge_target_contract
                    .clone()
                    .or(channel_bridge_target_contract);
                channel_bridge_account_scope = snapshot
                    .channel_bridge_account_scope
                    .clone()
                    .or(channel_bridge_account_scope);

                if let Some(snapshot_channel_bridge_ready) = snapshot.channel_bridge_ready {
                    channel_bridge_ready = Some(snapshot_channel_bridge_ready);
                    channel_bridge_missing_fields = snapshot.channel_bridge_missing_fields.clone();
                }
            }
        }
        if let (Some(source_path), Some(plugin_id)) = (
            provider.metadata.get("plugin_source_path"),
            provider.metadata.get("plugin_id"),
        ) && let Some(activation_entry) =
            activation_inventory_by_key.get(&(source_path.clone(), plugin_id.clone()))
        {
            manifest_api_version = activation_entry
                .manifest_api_version
                .clone()
                .or(manifest_api_version);
            plugin_version = activation_entry.plugin_version.clone().or(plugin_version);
            dialect = Some(activation_entry.dialect).or(dialect);
            dialect_version = activation_entry.dialect_version.clone().or(dialect_version);
            compatibility_mode = Some(activation_entry.compatibility_mode).or(compatibility_mode);
            compatibility_shim = activation_entry
                .compatibility_shim
                .clone()
                .or(compatibility_shim);
            compatibility_shim_support = activation_entry.compatibility_shim_support.clone();
            compatibility_shim_support_mismatch_reasons = activation_entry
                .compatibility_shim_support_mismatch_reasons
                .clone();
            compatibility = activation_entry.compatibility.clone().or(compatibility);
            activation_status = activation_entry
                .activation_status
                .map(|status| status.as_str().to_owned());
            activation_reason = activation_entry.activation_reason.clone();
            diagnostic_findings = activation_entry.diagnostic_findings.clone();
        } else if let (Some(source_path), Some(plugin_id)) = (
            provider.metadata.get("plugin_source_path"),
            provider.metadata.get("plugin_id"),
        ) && let Some((status, reason)) =
            activation_by_key.get(&(source_path.clone(), plugin_id.clone()))
        {
            activation_status = Some(status.clone());
            activation_reason = Some(reason.clone());
            compatibility_shim_support = activation_candidate_by_key
                .get(&(source_path.clone(), plugin_id.clone()))
                .and_then(|candidate| candidate.compatibility_shim_support.clone());
            compatibility_shim_support_mismatch_reasons = activation_candidate_by_key
                .get(&(source_path.clone(), plugin_id.clone()))
                .map(|candidate| {
                    candidate
                        .compatibility_shim_support_mismatch_reasons
                        .clone()
                })
                .unwrap_or_default();
            diagnostic_findings = activation_diagnostics_by_key
                .get(&(source_path.clone(), plugin_id.clone()))
                .cloned()
                .or_else(|| {
                    scan_diagnostics_by_key
                        .get(&(source_path.clone(), plugin_id.clone()))
                        .cloned()
                })
                .unwrap_or_default();
        } else if let (Some(source_path), Some(plugin_id)) = (
            provider.metadata.get("plugin_source_path"),
            provider.metadata.get("plugin_id"),
        ) {
            compatibility_shim_support = activation_candidate_by_key
                .get(&(source_path.clone(), plugin_id.clone()))
                .and_then(|candidate| candidate.compatibility_shim_support.clone());
            compatibility_shim_support_mismatch_reasons = activation_candidate_by_key
                .get(&(source_path.clone(), plugin_id.clone()))
                .map(|candidate| {
                    candidate
                        .compatibility_shim_support_mismatch_reasons
                        .clone()
                })
                .unwrap_or_default();
            diagnostic_findings = activation_diagnostics_by_key
                .get(&(source_path.clone(), plugin_id.clone()))
                .cloned()
                .or_else(|| {
                    scan_diagnostics_by_key
                        .get(&(source_path.clone(), plugin_id.clone()))
                        .cloned()
                })
                .unwrap_or_default();
        }

        entries.insert(
            tool_id.clone(),
            ToolSearchEntry {
                tool_id,
                plugin_id: provider.metadata.get("plugin_id").cloned(),
                manifest_api_version,
                plugin_version,
                dialect,
                dialect_version,
                compatibility_mode,
                compatibility_shim,
                compatibility_shim_support,
                compatibility_shim_support_mismatch_reasons,
                connector_name: provider.connector_name.clone(),
                provider_id: provider.provider_id.clone(),
                channel_id,
                source_path: provider.metadata.get("plugin_source_path").cloned(),
                source_kind: provider.metadata.get("plugin_source_kind").cloned(),
                package_root: provider.metadata.get("plugin_package_root").cloned(),
                package_manifest_path: provider
                    .metadata
                    .get("plugin_package_manifest_path")
                    .cloned(),
                provenance_summary,
                trust_tier,
                bridge_kind: resolved_bridge_kind,
                adapter_family,
                entrypoint_hint,
                source_language,
                setup_mode,
                setup_surface,
                setup_required_env_vars,
                setup_recommended_env_vars,
                setup_required_config_keys,
                setup_default_env_var,
                setup_docs_urls,
                setup_remediation,
                channel_bridge_transport_family,
                channel_bridge_target_contract,
                channel_bridge_account_scope,
                channel_bridge_ready,
                channel_bridge_missing_fields,
                setup_ready: true,
                missing_required_env_vars: Vec::new(),
                missing_required_config_keys: Vec::new(),
                slot_claims,
                diagnostic_findings,
                compatibility,
                activation_status,
                activation_reason,
                activation_attestation: provider_plugin_activation_attestation_result(
                    &provider.metadata,
                ),
                summary,
                tags,
                input_examples,
                output_examples,
                deferred,
                loaded: true,
            },
        );
    }

    for report in plugin_scan_reports {
        for descriptor in &report.descriptors {
            let manifest = &descriptor.manifest;
            let tool_id = format!("{}::{}", manifest.provider_id, manifest.connector_name);
            let translation =
                translation_by_key.get(&(descriptor.path.clone(), manifest.plugin_id.clone()));
            let bridge_kind = translation
                .map(|snapshot| snapshot.bridge_kind)
                .unwrap_or_else(|| descriptor_bridge_kind(descriptor));
            let adapter_family = translation.map(|snapshot| snapshot.adapter_family.clone());
            let entrypoint_hint = translation.map(|snapshot| snapshot.entrypoint_hint.clone());
            let source_language = translation.map(|snapshot| snapshot.source_language.clone());
            let activation = activation_inventory_by_key
                .get(&(descriptor.path.clone(), manifest.plugin_id.clone()));
            let activation_fallback =
                activation_by_key.get(&(descriptor.path.clone(), manifest.plugin_id.clone()));
            let channel_id = translation
                .and_then(|snapshot| snapshot.channel_id.clone())
                .or_else(|| manifest.metadata.get("plugin_channel_id").cloned())
                .or_else(|| manifest.channel_id.clone());
            let channel_bridge_transport_family = translation
                .and_then(|snapshot| snapshot.channel_bridge_transport_family.clone())
                .or_else(|| {
                    manifest
                        .metadata
                        .get("plugin_channel_bridge_transport_family")
                        .cloned()
                })
                .or_else(|| manifest.metadata.get("transport_family").cloned());
            let channel_bridge_target_contract = translation
                .and_then(|snapshot| snapshot.channel_bridge_target_contract.clone())
                .or_else(|| {
                    manifest
                        .metadata
                        .get("plugin_channel_bridge_target_contract")
                        .cloned()
                })
                .or_else(|| manifest.metadata.get("target_contract").cloned());
            let channel_bridge_account_scope = translation
                .and_then(|snapshot| snapshot.channel_bridge_account_scope.clone())
                .or_else(|| {
                    manifest
                        .metadata
                        .get("plugin_channel_bridge_account_scope")
                        .cloned()
                })
                .or_else(|| manifest.metadata.get("account_scope").cloned());
            let channel_bridge_ready = translation
                .and_then(|snapshot| snapshot.channel_bridge_ready)
                .or_else(|| metadata_bool(&manifest.metadata, "plugin_channel_bridge_ready"));
            let channel_bridge_missing_fields = translation
                .map(|snapshot| snapshot.channel_bridge_missing_fields.clone())
                .unwrap_or_else(|| {
                    metadata_strings(
                        &manifest.metadata,
                        "plugin_channel_bridge_missing_fields_json",
                    )
                });

            let entry = entries
                .entry(tool_id.clone())
                .or_insert_with(|| ToolSearchEntry {
                    tool_id: tool_id.clone(),
                    plugin_id: Some(manifest.plugin_id.clone()),
                    manifest_api_version: manifest.api_version.clone(),
                    plugin_version: manifest.version.clone(),
                    dialect: Some(descriptor.dialect),
                    dialect_version: descriptor.dialect_version.clone(),
                    compatibility_mode: Some(descriptor.compatibility_mode),
                    compatibility_shim: PluginCompatibilityShim::for_mode(
                        descriptor.compatibility_mode,
                    ),
                    compatibility_shim_support: activation
                        .and_then(|entry| entry.compatibility_shim_support.clone()),
                    compatibility_shim_support_mismatch_reasons: activation
                        .map(|entry| entry.compatibility_shim_support_mismatch_reasons.clone())
                        .unwrap_or_default(),
                    connector_name: manifest.connector_name.clone(),
                    provider_id: manifest.provider_id.clone(),
                    channel_id: channel_id.clone(),
                    source_path: Some(descriptor.path.clone()),
                    source_kind: Some(descriptor.source_kind.as_str().to_owned()),
                    package_root: Some(descriptor.package_root.clone()),
                    package_manifest_path: descriptor.package_manifest_path.clone(),
                    provenance_summary: Some(plugin_provenance_summary_for_descriptor(descriptor)),
                    trust_tier: Some(manifest.trust_tier.as_str().to_owned()),
                    bridge_kind,
                    adapter_family: adapter_family.clone(),
                    entrypoint_hint: entrypoint_hint.clone(),
                    source_language: source_language.clone(),
                    setup_mode: manifest
                        .setup
                        .as_ref()
                        .map(|setup| setup.mode.as_str().to_owned()),
                    setup_surface: manifest
                        .setup
                        .as_ref()
                        .and_then(|setup| setup.surface.clone()),
                    setup_required_env_vars: manifest
                        .setup
                        .as_ref()
                        .map(|setup| setup.required_env_vars.clone())
                        .unwrap_or_default(),
                    setup_recommended_env_vars: manifest
                        .setup
                        .as_ref()
                        .map(|setup| setup.recommended_env_vars.clone())
                        .unwrap_or_default(),
                    setup_required_config_keys: manifest
                        .setup
                        .as_ref()
                        .map(|setup| setup.required_config_keys.clone())
                        .unwrap_or_default(),
                    setup_default_env_var: manifest
                        .setup
                        .as_ref()
                        .and_then(|setup| setup.default_env_var.clone()),
                    setup_docs_urls: manifest
                        .setup
                        .as_ref()
                        .map(|setup| setup.docs_urls.clone())
                        .unwrap_or_default(),
                    setup_remediation: manifest
                        .setup
                        .as_ref()
                        .and_then(|setup| setup.remediation.clone()),
                    channel_bridge_transport_family: channel_bridge_transport_family.clone(),
                    channel_bridge_target_contract: channel_bridge_target_contract.clone(),
                    channel_bridge_account_scope: channel_bridge_account_scope.clone(),
                    channel_bridge_ready,
                    channel_bridge_missing_fields: channel_bridge_missing_fields.clone(),
                    setup_ready: true,
                    missing_required_env_vars: Vec::new(),
                    missing_required_config_keys: Vec::new(),
                    slot_claims: manifest.slot_claims.clone(),
                    diagnostic_findings: activation
                        .map(|entry| entry.diagnostic_findings.clone())
                        .unwrap_or_else(|| {
                            activation_diagnostics_by_key
                                .get(&(descriptor.path.clone(), manifest.plugin_id.clone()))
                                .cloned()
                                .or_else(|| {
                                    scan_diagnostics_by_key
                                        .get(&(descriptor.path.clone(), manifest.plugin_id.clone()))
                                        .cloned()
                                })
                                .unwrap_or_default()
                        }),
                    compatibility: activation
                        .and_then(|entry| entry.compatibility.clone())
                        .or_else(|| manifest.compatibility.clone()),
                    activation_status: activation
                        .and_then(|entry| entry.activation_status)
                        .map(|status| status.as_str().to_owned())
                        .or_else(|| activation_fallback.map(|(status, _)| status.clone())),
                    activation_reason: activation
                        .and_then(|entry| entry.activation_reason.clone())
                        .or_else(|| activation_fallback.map(|(_, reason)| reason.clone())),
                    activation_attestation: None,
                    summary: manifest.summary.clone(),
                    tags: manifest.tags.clone(),
                    input_examples: manifest.input_examples.clone(),
                    output_examples: manifest.output_examples.clone(),
                    deferred: manifest.defer_loading,
                    loaded: false,
                });

            if entry.plugin_id.is_none() {
                entry.plugin_id = Some(manifest.plugin_id.clone());
            }
            if entry.manifest_api_version.is_none() {
                entry.manifest_api_version = activation
                    .and_then(|entry| entry.manifest_api_version.clone())
                    .or_else(|| manifest.api_version.clone());
            }
            if entry.plugin_version.is_none() {
                entry.plugin_version = activation
                    .and_then(|entry| entry.plugin_version.clone())
                    .or_else(|| manifest.version.clone());
            }
            if entry.dialect.is_none() {
                entry.dialect = activation
                    .map(|entry| entry.dialect)
                    .or(Some(descriptor.dialect));
            }
            if entry.dialect_version.is_none() {
                entry.dialect_version = activation
                    .and_then(|entry| entry.dialect_version.clone())
                    .or_else(|| descriptor.dialect_version.clone());
            }
            if entry.compatibility_mode.is_none() {
                entry.compatibility_mode = activation
                    .map(|entry| entry.compatibility_mode)
                    .or(Some(descriptor.compatibility_mode));
            }
            if entry.compatibility_shim.is_none() {
                entry.compatibility_shim = activation
                    .and_then(|entry| entry.compatibility_shim.clone())
                    .or_else(|| PluginCompatibilityShim::for_mode(descriptor.compatibility_mode));
            }
            if entry.compatibility_shim_support.is_none() {
                entry.compatibility_shim_support =
                    activation.and_then(|entry| entry.compatibility_shim_support.clone());
            }
            if entry.compatibility_shim_support_mismatch_reasons.is_empty() {
                entry.compatibility_shim_support_mismatch_reasons = activation
                    .map(|entry| entry.compatibility_shim_support_mismatch_reasons.clone())
                    .unwrap_or_default();
            }
            if entry.channel_id.is_none() {
                entry.channel_id = channel_id.clone();
            }
            if entry.source_path.is_none() {
                entry.source_path = Some(descriptor.path.clone());
            }
            if entry.source_kind.is_none() {
                entry.source_kind = Some(descriptor.source_kind.as_str().to_owned());
            }
            if entry.package_root.is_none() {
                entry.package_root = Some(descriptor.package_root.clone());
            }
            if entry.package_manifest_path.is_none() {
                entry.package_manifest_path = descriptor.package_manifest_path.clone();
            }
            if entry.provenance_summary.is_none() {
                entry.provenance_summary =
                    Some(plugin_provenance_summary_for_descriptor(descriptor));
            }
            if entry.trust_tier.is_none() {
                entry.trust_tier = Some(manifest.trust_tier.as_str().to_owned());
            }
            if entry.summary.is_none() {
                entry.summary = manifest.summary.clone();
            }
            if entry.adapter_family.is_none() {
                entry.adapter_family = adapter_family.clone();
            }
            if entry.entrypoint_hint.is_none() {
                entry.entrypoint_hint = entrypoint_hint.clone();
            }
            if entry.source_language.is_none() {
                entry.source_language = source_language.clone();
            }
            if entry.setup_mode.is_none() {
                entry.setup_mode = manifest
                    .setup
                    .as_ref()
                    .map(|setup| setup.mode.as_str().to_owned());
            }
            if entry.setup_surface.is_none() {
                entry.setup_surface = manifest
                    .setup
                    .as_ref()
                    .and_then(|setup| setup.surface.clone());
            }
            if entry.setup_required_env_vars.is_empty() {
                entry.setup_required_env_vars = manifest
                    .setup
                    .as_ref()
                    .map(|setup| setup.required_env_vars.clone())
                    .unwrap_or_default();
            }
            if entry.setup_recommended_env_vars.is_empty() {
                entry.setup_recommended_env_vars = manifest
                    .setup
                    .as_ref()
                    .map(|setup| setup.recommended_env_vars.clone())
                    .unwrap_or_default();
            }
            if entry.setup_required_config_keys.is_empty() {
                entry.setup_required_config_keys = manifest
                    .setup
                    .as_ref()
                    .map(|setup| setup.required_config_keys.clone())
                    .unwrap_or_default();
            }
            if entry.slot_claims.is_empty() {
                entry.slot_claims = manifest.slot_claims.clone();
            }
            if entry.diagnostic_findings.is_empty() {
                entry.diagnostic_findings = activation
                    .map(|entry| entry.diagnostic_findings.clone())
                    .unwrap_or_else(|| {
                        activation_diagnostics_by_key
                            .get(&(descriptor.path.clone(), manifest.plugin_id.clone()))
                            .cloned()
                            .or_else(|| {
                                scan_diagnostics_by_key
                                    .get(&(descriptor.path.clone(), manifest.plugin_id.clone()))
                                    .cloned()
                            })
                            .unwrap_or_default()
                    });
            }
            if entry.compatibility.is_none() {
                entry.compatibility = activation
                    .and_then(|entry| entry.compatibility.clone())
                    .or_else(|| manifest.compatibility.clone());
            }
            if entry.activation_status.is_none() {
                entry.activation_status = activation
                    .and_then(|entry| entry.activation_status)
                    .map(|status| status.as_str().to_owned())
                    .or_else(|| activation_fallback.map(|(status, _)| status.clone()));
            }
            if entry.activation_reason.is_none() {
                entry.activation_reason = activation
                    .and_then(|entry| entry.activation_reason.clone())
                    .or_else(|| activation_fallback.map(|(_, reason)| reason.clone()));
            }
            if entry.setup_default_env_var.is_none() {
                entry.setup_default_env_var = manifest
                    .setup
                    .as_ref()
                    .and_then(|setup| setup.default_env_var.clone());
            }
            if entry.setup_docs_urls.is_empty() {
                entry.setup_docs_urls = manifest
                    .setup
                    .as_ref()
                    .map(|setup| setup.docs_urls.clone())
                    .unwrap_or_default();
            }
            if entry.setup_remediation.is_none() {
                entry.setup_remediation = manifest
                    .setup
                    .as_ref()
                    .and_then(|setup| setup.remediation.clone());
            }
            if entry.channel_bridge_transport_family.is_none() {
                entry.channel_bridge_transport_family = channel_bridge_transport_family.clone();
            }
            if entry.channel_bridge_target_contract.is_none() {
                entry.channel_bridge_target_contract = channel_bridge_target_contract.clone();
            }
            if entry.channel_bridge_account_scope.is_none() {
                entry.channel_bridge_account_scope = channel_bridge_account_scope.clone();
            }
            if entry.channel_bridge_ready.is_none() {
                entry.channel_bridge_ready = channel_bridge_ready;
            }
            if entry.channel_bridge_missing_fields.is_empty() {
                entry.channel_bridge_missing_fields = channel_bridge_missing_fields.clone();
            }
            if entry.input_examples.is_empty() {
                entry.input_examples = manifest.input_examples.clone();
            }
            if entry.output_examples.is_empty() {
                entry.output_examples = manifest.output_examples.clone();
            }
            for tag in &manifest.tags {
                if !entry.tags.iter().any(|existing| existing == tag) {
                    entry.tags.push(tag.clone());
                }
            }
            if !entry.loaded {
                entry.deferred = manifest.defer_loading;
                entry.bridge_kind = bridge_kind;
            }
        }
    }

    for entry in entries.values_mut() {
        let readiness = evaluate_plugin_setup_requirements(
            &entry.setup_required_env_vars,
            &entry.setup_required_config_keys,
            setup_readiness_context,
        );
        entry.setup_ready = readiness.ready;
        entry.missing_required_env_vars = readiness.missing_required_env_vars;
        entry.missing_required_config_keys = readiness.missing_required_config_keys;
    }

    let parsed_query = parse_tool_search_query(query, trust_tiers);

    let deferred_visible_entries: Vec<ToolSearchEntry> = entries
        .into_values()
        .filter(|entry| include_deferred || !entry.deferred || entry.loaded)
        .collect();
    let candidates_before_trust_filter = deferred_visible_entries.len();
    let (trust_matched_entries, trust_filtered_entries): (Vec<_>, Vec<_>) =
        deferred_visible_entries
            .into_iter()
            .partition(|entry| tool_search_matches_trust_tier_filter(entry, &parsed_query));

    let mut ranked: Vec<(u32, ToolSearchEntry)> = trust_matched_entries
        .into_iter()
        .filter_map(|entry| {
            let score =
                tool_search_score(&entry, &parsed_query.normalized_text, &parsed_query.tokens);
            if parsed_query.normalized_text.is_empty() || score > 0 {
                Some((score, entry))
            } else {
                None
            }
        })
        .collect();

    ranked.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| right.loaded.cmp(&left.loaded))
            .then_with(|| {
                trust_tier_sort_rank(right.trust_tier.as_deref())
                    .cmp(&trust_tier_sort_rank(left.trust_tier.as_deref()))
            })
            .then_with(|| left.tool_id.cmp(&right.tool_id))
    });

    let capped_limit = limit.clamp(1, 50);
    let results = ranked
        .into_iter()
        .take(capped_limit)
        .map(|(score, entry)| ToolSearchResult {
            tool_id: entry.tool_id,
            plugin_id: entry.plugin_id,
            manifest_api_version: entry.manifest_api_version,
            plugin_version: entry.plugin_version,
            dialect: entry.dialect.map(|dialect| dialect.as_str().to_owned()),
            dialect_version: entry.dialect_version,
            compatibility_mode: entry
                .compatibility_mode
                .map(|mode| mode.as_str().to_owned()),
            compatibility_shim: entry.compatibility_shim,
            compatibility_shim_support: entry.compatibility_shim_support,
            compatibility_shim_support_mismatch_reasons: entry
                .compatibility_shim_support_mismatch_reasons,
            connector_name: entry.connector_name,
            provider_id: entry.provider_id,
            channel_id: entry.channel_id,
            source_path: entry.source_path,
            source_kind: entry.source_kind,
            package_root: entry.package_root,
            package_manifest_path: entry.package_manifest_path,
            provenance_summary: entry.provenance_summary,
            trust_tier: entry.trust_tier,
            bridge_kind: entry.bridge_kind.as_str().to_owned(),
            adapter_family: entry.adapter_family,
            entrypoint_hint: entry.entrypoint_hint,
            source_language: entry.source_language,
            setup_mode: entry.setup_mode,
            setup_surface: entry.setup_surface,
            setup_required_env_vars: entry.setup_required_env_vars,
            setup_recommended_env_vars: entry.setup_recommended_env_vars,
            setup_required_config_keys: entry.setup_required_config_keys,
            setup_default_env_var: entry.setup_default_env_var,
            setup_docs_urls: entry.setup_docs_urls,
            setup_remediation: entry.setup_remediation,
            channel_bridge_transport_family: entry.channel_bridge_transport_family,
            channel_bridge_target_contract: entry.channel_bridge_target_contract,
            channel_bridge_account_scope: entry.channel_bridge_account_scope,
            channel_bridge_ready: entry.channel_bridge_ready,
            channel_bridge_missing_fields: entry.channel_bridge_missing_fields,
            setup_ready: entry.setup_ready,
            missing_required_env_vars: entry.missing_required_env_vars,
            missing_required_config_keys: entry.missing_required_config_keys,
            slot_claims: entry.slot_claims,
            diagnostic_findings: entry.diagnostic_findings,
            compatibility: entry.compatibility,
            activation_status: entry.activation_status,
            activation_reason: entry.activation_reason,
            activation_attestation: entry.activation_attestation,
            score,
            deferred: entry.deferred,
            loaded: entry.loaded,
            summary: entry.summary,
            tags: entry.tags,
            input_examples: if include_examples {
                entry.input_examples
            } else {
                Vec::new()
            },
            output_examples: if include_examples {
                entry.output_examples
            } else {
                Vec::new()
            },
        })
        .collect();

    ToolSearchExecutionReport {
        results,
        trust_filter_summary: ToolSearchTrustFilterSummary {
            applied: parsed_query.trust_filter_requested,
            query_requested_tiers: parsed_query.query_requested_tiers.into_iter().collect(),
            structured_requested_tiers: parsed_query
                .structured_requested_tiers
                .into_iter()
                .collect(),
            effective_tiers: parsed_query.effective_trust_tiers.into_iter().collect(),
            conflicting_requested_tiers: parsed_query.conflicting_requested_tiers,
            candidates_before_trust_filter,
            candidates_after_trust_filter: candidates_before_trust_filter
                .saturating_sub(trust_filtered_entries.len()),
            filtered_out_candidates: trust_filtered_entries.len(),
            filtered_out_tier_counts: build_filtered_out_tier_counts(&trust_filtered_entries),
        },
    }
}

fn metadata_optional_string(metadata: &BTreeMap<String, String>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn metadata_plugin_dialect(
    metadata: &BTreeMap<String, String>,
    key: &str,
) -> Option<PluginContractDialect> {
    metadata_optional_string(metadata, key).and_then(|value| match value.as_str() {
        "loong_package_manifest" => Some(PluginContractDialect::LoongPackageManifest),
        "loong_embedded_source" => Some(PluginContractDialect::LoongEmbeddedSource),
        "openclaw_modern_manifest" => Some(PluginContractDialect::OpenClawModernManifest),
        "openclaw_legacy_package" => Some(PluginContractDialect::OpenClawLegacyPackage),
        _ => None,
    })
}

fn metadata_plugin_compatibility_mode(
    metadata: &BTreeMap<String, String>,
    key: &str,
) -> Option<PluginCompatibilityMode> {
    metadata_optional_string(metadata, key).and_then(|value| match value.as_str() {
        "native" => Some(PluginCompatibilityMode::Native),
        "openclaw_modern" => Some(PluginCompatibilityMode::OpenClawModern),
        "openclaw_legacy" => Some(PluginCompatibilityMode::OpenClawLegacy),
        _ => None,
    })
}

fn metadata_plugin_compatibility_shim(
    metadata: &BTreeMap<String, String>,
) -> Option<PluginCompatibilityShim> {
    let shim_id = metadata_optional_string(metadata, "plugin_compatibility_shim_id");
    let family = metadata_optional_string(metadata, "plugin_compatibility_shim_family");
    match (shim_id, family) {
        (None, None) => None,
        (Some(shim_id), None) => Some(PluginCompatibilityShim {
            family: shim_id.clone(),
            shim_id,
        }),
        (None, Some(family)) => Some(PluginCompatibilityShim {
            shim_id: family.clone(),
            family,
        }),
        (Some(shim_id), Some(family)) => Some(PluginCompatibilityShim { shim_id, family }),
    }
}

fn metadata_tags(metadata: &BTreeMap<String, String>) -> Vec<String> {
    if let Some(raw_json) = metadata.get("tags_json")
        && let Ok(values) = serde_json::from_str::<Vec<String>>(raw_json)
    {
        return values;
    }

    metadata
        .get("tags")
        .map(|raw| {
            raw.split([',', ';'])
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn metadata_slot_claims(metadata: &BTreeMap<String, String>) -> Vec<PluginSlotClaim> {
    let Some(raw_json) = metadata.get("plugin_slot_claims_json") else {
        return Vec::new();
    };

    serde_json::from_str::<Vec<PluginSlotClaim>>(raw_json).unwrap_or_default()
}

fn diagnostic_haystack(findings: &[PluginDiagnosticFinding]) -> String {
    findings
        .iter()
        .map(|finding| {
            format!(
                "{} {} {} {} {} {} {}",
                finding.code.as_str(),
                finding.severity.as_str(),
                finding.phase.as_str(),
                if finding.blocking {
                    "blocking"
                } else {
                    "non_blocking"
                },
                finding.field_path.as_deref().unwrap_or_default(),
                finding.message,
                finding.remediation.as_deref().unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn metadata_plugin_compatibility(
    metadata: &BTreeMap<String, String>,
) -> Option<PluginCompatibility> {
    let host_api = metadata
        .get("plugin_compatibility_host_api")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let host_version_req = metadata
        .get("plugin_compatibility_host_version_req")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    if host_api.is_none() && host_version_req.is_none() {
        return None;
    }

    Some(PluginCompatibility {
        host_api,
        host_version_req,
    })
}

fn metadata_examples(metadata: &BTreeMap<String, String>, key: &str) -> Vec<Value> {
    metadata
        .get(key)
        .and_then(|raw| serde_json::from_str::<Vec<Value>>(raw).ok())
        .unwrap_or_default()
}

fn metadata_strings(metadata: &BTreeMap<String, String>, key: &str) -> Vec<String> {
    metadata
        .get(key)
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default()
}

fn metadata_bool(metadata: &BTreeMap<String, String>, key: &str) -> Option<bool> {
    metadata
        .get(key)
        .and_then(|raw| match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "y" | "on" => Some(true),
            "false" | "0" | "no" | "n" | "off" => Some(false),
            _ => None,
        })
}

#[derive(Debug, Default)]
struct ParsedToolSearchQuery {
    normalized_text: String,
    tokens: Vec<String>,
    query_requested_tiers: BTreeSet<String>,
    structured_requested_tiers: BTreeSet<String>,
    effective_trust_tiers: BTreeSet<String>,
    trust_filter_requested: bool,
    conflicting_requested_tiers: bool,
}

fn parse_tool_search_query(
    query: &str,
    structured_trust_tiers: &[PluginTrustTier],
) -> ParsedToolSearchQuery {
    let mut freeform_terms = Vec::new();
    let mut query_trust_tiers = BTreeSet::new();

    for term in query
        .split_whitespace()
        .map(str::trim)
        .filter(|term| !term.is_empty())
    {
        if let Some((raw_key, raw_value)) = term.split_once(':')
            && matches!(
                normalize_tool_search_filter_key(raw_key).as_str(),
                "trust" | "tier" | "trust-tier" | "trust_tier"
            )
            && let Some(trust_tier) = normalize_trust_tier_label(raw_value)
        {
            query_trust_tiers.insert(trust_tier.to_owned());
            continue;
        }

        freeform_terms.push(term.to_owned());
    }

    let structured_requested_tiers = structured_trust_tiers
        .iter()
        .map(|trust_tier| trust_tier.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    let trust_filter_requested =
        !query_trust_tiers.is_empty() || !structured_requested_tiers.is_empty();
    let effective_trust_tiers = if structured_requested_tiers.is_empty() {
        query_trust_tiers.clone()
    } else if query_trust_tiers.is_empty() {
        structured_requested_tiers.clone()
    } else {
        structured_requested_tiers
            .intersection(&query_trust_tiers)
            .cloned()
            .collect()
    };
    let conflicting_requested_tiers = trust_filter_requested
        && !query_trust_tiers.is_empty()
        && !structured_requested_tiers.is_empty()
        && effective_trust_tiers.is_empty();
    let normalized_text = freeform_terms.join(" ").trim().to_ascii_lowercase();
    let tokens = tokenize_tool_search_text(&normalized_text);
    ParsedToolSearchQuery {
        normalized_text,
        tokens,
        query_requested_tiers: query_trust_tiers,
        structured_requested_tiers,
        effective_trust_tiers,
        trust_filter_requested,
        conflicting_requested_tiers,
    }
}

fn normalize_tool_search_filter_key(key: &str) -> String {
    key.trim().to_ascii_lowercase()
}

fn tokenize_tool_search_text(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

fn normalize_trust_tier_label(value: &str) -> Option<&'static str> {
    let normalized = value
        .trim()
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
        .to_ascii_lowercase()
        .replace('_', "-");

    match normalized.as_str() {
        "official" => Some("official"),
        "verified-community" | "verifiedcommunity" | "verified" => Some("verified-community"),
        "unverified" => Some("unverified"),
        _ => None,
    }
}

fn tool_search_matches_trust_tier_filter(
    entry: &ToolSearchEntry,
    query: &ParsedToolSearchQuery,
) -> bool {
    if !query.trust_filter_requested {
        return true;
    }

    entry
        .trust_tier
        .as_deref()
        .and_then(normalize_trust_tier_label)
        .is_some_and(|trust_tier| query.effective_trust_tiers.contains(trust_tier))
}

fn build_filtered_out_tier_counts(entries: &[ToolSearchEntry]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for entry in entries {
        let label = entry
            .trust_tier
            .as_deref()
            .and_then(normalize_trust_tier_label)
            .unwrap_or("unknown")
            .to_owned();
        *counts.entry(label).or_insert(0) += 1;
    }
    counts
}

fn trust_tier_sort_rank(trust_tier: Option<&str>) -> u8 {
    match trust_tier.and_then(normalize_trust_tier_label) {
        Some("official") => 3,
        Some("verified-community") => 2,
        // Keep missing or legacy metadata neutral instead of treating it as unverified.
        Some("unverified") => 0,
        None => 1,
        Some(_) => 1,
    }
}

fn tool_search_score(entry: &ToolSearchEntry, query: &str, tokens: &[String]) -> u32 {
    if query.is_empty() {
        return if entry.loaded { 10 } else { 5 };
    }

    let connector = entry.connector_name.to_ascii_lowercase();
    let provider = entry.provider_id.to_ascii_lowercase();
    let tool_id = entry.tool_id.to_ascii_lowercase();
    let manifest_api_version = entry
        .manifest_api_version
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let plugin_version = entry
        .plugin_version
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let dialect = entry
        .dialect
        .map(|dialect| dialect.as_str().to_owned())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let dialect_version = entry
        .dialect_version
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let compatibility_mode = entry
        .compatibility_mode
        .map(|mode| mode.as_str().to_owned())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let compatibility_shim_id = entry
        .compatibility_shim
        .as_ref()
        .map(|shim| shim.shim_id.to_ascii_lowercase())
        .unwrap_or_default();
    let compatibility_shim_family = entry
        .compatibility_shim
        .as_ref()
        .map(|shim| shim.family.to_ascii_lowercase())
        .unwrap_or_default();
    let compatibility_shim_support_version = entry
        .compatibility_shim_support
        .as_ref()
        .and_then(|support| support.version.as_deref())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let compatibility_shim_supported_dialects = entry
        .compatibility_shim_support
        .as_ref()
        .map(|support| {
            support
                .supported_dialects
                .iter()
                .map(|dialect| dialect.as_str().to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let compatibility_shim_supported_bridges = entry
        .compatibility_shim_support
        .as_ref()
        .map(|support| {
            support
                .supported_bridges
                .iter()
                .map(|bridge| bridge.as_str().to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let compatibility_shim_supported_adapter_families = entry
        .compatibility_shim_support
        .as_ref()
        .map(|support| {
            support
                .supported_adapter_families
                .iter()
                .map(|family| family.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let compatibility_shim_supported_source_languages = entry
        .compatibility_shim_support
        .as_ref()
        .map(|support| {
            support
                .supported_source_languages
                .iter()
                .map(|language| language.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let compatibility_shim_support_mismatch_reasons = entry
        .compatibility_shim_support_mismatch_reasons
        .iter()
        .map(|reason| reason.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let summary = entry
        .summary
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let source_path = entry
        .source_path
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let source_kind = entry
        .source_kind
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let package_root = entry
        .package_root
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let package_manifest_path = entry
        .package_manifest_path
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let provenance_summary = entry
        .provenance_summary
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let trust_tier = entry
        .trust_tier
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let adapter_family = entry
        .adapter_family
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let entrypoint_hint = entry
        .entrypoint_hint
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let source_language = entry
        .source_language
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let channel_id = entry
        .channel_id
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let setup_mode = entry
        .setup_mode
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let setup_surface = entry
        .setup_surface
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let setup_default_env_var = entry
        .setup_default_env_var
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let setup_remediation = entry
        .setup_remediation
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let channel_bridge_transport_family = entry
        .channel_bridge_transport_family
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let channel_bridge_target_contract = entry
        .channel_bridge_target_contract
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let channel_bridge_account_scope = entry
        .channel_bridge_account_scope
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let channel_bridge_missing_fields = entry
        .channel_bridge_missing_fields
        .join(" ")
        .to_ascii_lowercase();
    let tags: Vec<String> = entry
        .tags
        .iter()
        .map(|tag| tag.to_ascii_lowercase())
        .collect();
    let setup_required_env_vars: Vec<String> = entry
        .setup_required_env_vars
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect();
    let setup_recommended_env_vars: Vec<String> = entry
        .setup_recommended_env_vars
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect();
    let setup_required_config_keys: Vec<String> = entry
        .setup_required_config_keys
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect();
    let setup_docs_urls: Vec<String> = entry
        .setup_docs_urls
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect();
    let compatibility_host_api = entry
        .compatibility
        .as_ref()
        .and_then(|compatibility| compatibility.host_api.as_deref())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let compatibility_host_version_req = entry
        .compatibility
        .as_ref()
        .and_then(|compatibility| compatibility.host_version_req.as_deref())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let activation_status = entry
        .activation_status
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let activation_reason = entry
        .activation_reason
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let activation_attestation_integrity = entry
        .activation_attestation
        .as_ref()
        .map(|attestation| attestation.integrity.to_ascii_lowercase())
        .unwrap_or_default();
    let activation_attestation_issue = entry
        .activation_attestation
        .as_ref()
        .and_then(|attestation| attestation.issue.as_deref())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let activation_attestation_checksum = entry
        .activation_attestation
        .as_ref()
        .and_then(|attestation| attestation.checksum.as_deref())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let activation_attestation_computed_checksum = entry
        .activation_attestation
        .as_ref()
        .and_then(|attestation| attestation.computed_checksum.as_deref())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let slot_claim_tokens: Vec<String> = entry
        .slot_claims
        .iter()
        .flat_map(|claim| {
            [
                claim.slot.to_ascii_lowercase(),
                claim.key.to_ascii_lowercase(),
                claim.mode.as_str().to_ascii_lowercase(),
                format!("{}:{}", claim.slot, claim.key).to_ascii_lowercase(),
            ]
        })
        .collect();
    let diagnostics = diagnostic_haystack(&entry.diagnostic_findings).to_ascii_lowercase();

    let mut score = 0_u32;
    if connector == query {
        score = score.saturating_add(150);
    } else if connector.contains(query) {
        score = score.saturating_add(110);
    }
    if provider == query {
        score = score.saturating_add(120);
    } else if provider.contains(query) {
        score = score.saturating_add(80);
    }
    if tool_id.contains(query) {
        score = score.saturating_add(60);
    }
    if manifest_api_version.contains(query) {
        score = score.saturating_add(18);
    }
    if plugin_version.contains(query) {
        score = score.saturating_add(20);
    }
    if dialect.contains(query) {
        score = score.saturating_add(24);
    }
    if dialect_version.contains(query) {
        score = score.saturating_add(12);
    }
    if compatibility_mode.contains(query) {
        score = score.saturating_add(22);
    }
    if compatibility_shim_id.contains(query) {
        score = score.saturating_add(18);
    }
    if compatibility_shim_family.contains(query) {
        score = score.saturating_add(18);
    }
    if compatibility_shim_support_version.contains(query) {
        score = score.saturating_add(18);
    }
    if compatibility_shim_supported_dialects
        .iter()
        .any(|value| value.contains(query))
    {
        score = score.saturating_add(14);
    }
    if compatibility_shim_supported_bridges
        .iter()
        .any(|value| value.contains(query))
    {
        score = score.saturating_add(14);
    }
    if compatibility_shim_supported_adapter_families
        .iter()
        .any(|value| value.contains(query))
    {
        score = score.saturating_add(14);
    }
    if compatibility_shim_supported_source_languages
        .iter()
        .any(|value| value.contains(query))
    {
        score = score.saturating_add(12);
    }
    if compatibility_shim_support_mismatch_reasons
        .iter()
        .any(|value| value.contains(query))
    {
        score = score.saturating_add(16);
    }
    if summary.contains(query) {
        score = score.saturating_add(55);
    }
    if source_path.contains(query) {
        score = score.saturating_add(35);
    }
    if source_kind.contains(query) {
        score = score.saturating_add(12);
    }
    if package_root.contains(query) {
        score = score.saturating_add(20);
    }
    if package_manifest_path.contains(query) {
        score = score.saturating_add(20);
    }
    if provenance_summary.contains(query) {
        score = score.saturating_add(18);
    }
    if trust_tier == query {
        score = score.saturating_add(32);
    } else if trust_tier.contains(query) {
        score = score.saturating_add(16);
    }
    if adapter_family.contains(query) {
        score = score.saturating_add(18);
    }
    if entrypoint_hint.contains(query) {
        score = score.saturating_add(12);
    }
    if source_language.contains(query) {
        score = score.saturating_add(10);
    }
    if channel_id == query {
        score = score.saturating_add(90);
    } else if channel_id.contains(query) {
        score = score.saturating_add(60);
    }
    if setup_mode.contains(query) {
        score = score.saturating_add(12);
    }
    if setup_surface.contains(query) {
        score = score.saturating_add(18);
    }
    if setup_default_env_var.contains(query) {
        score = score.saturating_add(20);
    }
    if setup_remediation.contains(query) {
        score = score.saturating_add(10);
    }
    if channel_bridge_transport_family.contains(query) {
        score = score.saturating_add(20);
    }
    if channel_bridge_target_contract.contains(query) {
        score = score.saturating_add(20);
    }
    if channel_bridge_account_scope.contains(query) {
        score = score.saturating_add(16);
    }
    if channel_bridge_missing_fields.contains(query) {
        score = score.saturating_add(12);
    }
    if setup_docs_urls.iter().any(|value| value.contains(query)) {
        score = score.saturating_add(8);
    }
    if compatibility_host_api.contains(query) {
        score = score.saturating_add(16);
    }
    if compatibility_host_version_req.contains(query) {
        score = score.saturating_add(12);
    }
    if activation_status.contains(query) {
        score = score.saturating_add(14);
    }
    if activation_reason.contains(query) {
        score = score.saturating_add(10);
    }
    if activation_attestation_integrity.contains(query) {
        score = score.saturating_add(12);
    }
    if activation_attestation_issue.contains(query) {
        score = score.saturating_add(14);
    }
    if activation_attestation_checksum.contains(query)
        || activation_attestation_computed_checksum.contains(query)
    {
        score = score.saturating_add(10);
    }
    if diagnostics.contains(query) {
        score = score.saturating_add(14);
    }
    if slot_claim_tokens.iter().any(|token| token == query) {
        score = score.saturating_add(36);
    } else if slot_claim_tokens.iter().any(|token| token.contains(query)) {
        score = score.saturating_add(20);
    }
    if tags.iter().any(|tag| tag == query) {
        score = score.saturating_add(45);
    } else if tags.iter().any(|tag| tag.contains(query)) {
        score = score.saturating_add(25);
    }
    if setup_required_env_vars.iter().any(|value| value == query) {
        score = score.saturating_add(40);
    } else if setup_required_env_vars
        .iter()
        .any(|value| value.contains(query))
    {
        score = score.saturating_add(24);
    }
    if setup_recommended_env_vars
        .iter()
        .any(|value| value == query)
    {
        score = score.saturating_add(28);
    } else if setup_recommended_env_vars
        .iter()
        .any(|value| value.contains(query))
    {
        score = score.saturating_add(16);
    }
    if setup_required_config_keys
        .iter()
        .any(|value| value == query)
    {
        score = score.saturating_add(32);
    } else if setup_required_config_keys
        .iter()
        .any(|value| value.contains(query))
    {
        score = score.saturating_add(18);
    }

    let haystack_parts = vec![
        connector,
        provider,
        tool_id,
        manifest_api_version,
        plugin_version,
        dialect,
        dialect_version,
        compatibility_mode,
        compatibility_shim_id,
        compatibility_shim_family,
        compatibility_shim_support_version,
        compatibility_shim_supported_dialects.join(" "),
        compatibility_shim_supported_bridges.join(" "),
        compatibility_shim_supported_adapter_families.join(" "),
        compatibility_shim_supported_source_languages.join(" "),
        compatibility_shim_support_mismatch_reasons.join(" "),
        summary,
        source_path,
        source_kind,
        package_root,
        package_manifest_path,
        provenance_summary,
        trust_tier,
        adapter_family,
        entrypoint_hint,
        source_language,
        channel_id,
        setup_mode,
        setup_surface,
        setup_default_env_var,
        setup_remediation,
        channel_bridge_transport_family,
        channel_bridge_target_contract,
        channel_bridge_account_scope,
        channel_bridge_missing_fields,
        tags.join(" "),
        setup_required_env_vars.join(" "),
        setup_recommended_env_vars.join(" "),
        setup_required_config_keys.join(" "),
        setup_docs_urls.join(" "),
    ];
    let haystack = haystack_parts.join(" ");
    for token in tokens {
        if haystack.contains(token) {
            score = score.saturating_add(8);
        }
    }

    if entry.loaded {
        score = score.saturating_add(4);
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::{
        IntegrationCatalog, PluginActivationCandidate, PluginActivationPlan,
        PluginActivationStatus, PluginBridgeKind, PluginCompatibilityMode, PluginContractDialect,
        PluginDiagnosticCode, PluginDiagnosticFinding, PluginDiagnosticPhase,
        PluginDiagnosticSeverity, PluginSetupReadinessContext, PluginSlotClaim, PluginSlotMode,
        PluginSourceKind, ProviderConfig,
    };
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn execute_tool_search_surfaces_plugin_provenance_and_setup_metadata() {
        let mut catalog = IntegrationCatalog::new();
        let provider = ProviderConfig {
            provider_id: "tavily".to_owned(),
            connector_name: "tavily-http".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_id".to_owned(), "tavily-search".to_owned()),
                (
                    "plugin_source_path".to_owned(),
                    "/tmp/tavily/loong.plugin.json".to_owned(),
                ),
                (
                    "plugin_source_kind".to_owned(),
                    "package_manifest".to_owned(),
                ),
                ("plugin_package_root".to_owned(), "/tmp/tavily".to_owned()),
                (
                    "plugin_package_manifest_path".to_owned(),
                    "/tmp/tavily/loong.plugin.json".to_owned(),
                ),
                (
                    "plugin_provenance_summary".to_owned(),
                    "package_manifest:/tmp/tavily/loong.plugin.json".to_owned(),
                ),
                ("plugin_trust_tier".to_owned(), "official".to_owned()),
                (
                    "plugin_manifest_api_version".to_owned(),
                    "v1alpha1".to_owned(),
                ),
                ("plugin_version".to_owned(), "0.3.0".to_owned()),
                ("plugin_setup_mode".to_owned(), "metadata_only".to_owned()),
                ("plugin_setup_surface".to_owned(), "web_search".to_owned()),
                (
                    "plugin_setup_required_env_vars_json".to_owned(),
                    "[\"TAVILY_API_KEY\"]".to_owned(),
                ),
                (
                    "plugin_setup_recommended_env_vars_json".to_owned(),
                    "[\"TEAM_TAVILY_KEY\"]".to_owned(),
                ),
                (
                    "plugin_setup_required_config_keys_json".to_owned(),
                    "[\"tools.web_search.default_provider\"]".to_owned(),
                ),
                (
                    "plugin_setup_default_env_var".to_owned(),
                    "TAVILY_API_KEY".to_owned(),
                ),
                (
                    "plugin_setup_docs_urls_json".to_owned(),
                    "[\"https://docs.example.com/tavily\"]".to_owned(),
                ),
                (
                    "plugin_setup_remediation".to_owned(),
                    "set a Tavily credential before enabling search".to_owned(),
                ),
                (
                    "plugin_slot_claims_json".to_owned(),
                    "[{\"slot\":\"provider:web_search\",\"key\":\"tavily\",\"mode\":\"exclusive\"}]"
                        .to_owned(),
                ),
                (
                    "plugin_compatibility_host_api".to_owned(),
                    "loong-plugin/v1".to_owned(),
                ),
                (
                    "plugin_compatibility_host_version_req".to_owned(),
                    ">=0.1.0-alpha.1".to_owned(),
                ),
                ("bridge_kind".to_owned(), "http_json".to_owned()),
            ]),
        };
        catalog.upsert_provider(provider);

        let activation_plans = vec![PluginActivationPlan {
            total_plugins: 1,
            ready_plugins: 0,
            setup_incomplete_plugins: 0,
            blocked_plugins: 1,
            candidates: vec![PluginActivationCandidate {
                plugin_id: "tavily-search".to_owned(),
                source_path: "/tmp/tavily/loong.plugin.json".to_owned(),
                source_kind: PluginSourceKind::PackageManifest,
                package_root: "/tmp/tavily".to_owned(),
                package_manifest_path: Some("/tmp/tavily/loong.plugin.json".to_owned()),
                trust_tier: kernel::PluginTrustTier::Official,
                compatibility_mode: PluginCompatibilityMode::Native,
                compatibility_shim: None,
                compatibility_shim_support: None,
                compatibility_shim_support_mismatch_reasons: Vec::new(),
                bridge_kind: PluginBridgeKind::HttpJson,
                adapter_family: "http-adapter".to_owned(),
                slot_claims: vec![PluginSlotClaim {
                    slot: "provider:web_search".to_owned(),
                    key: "tavily".to_owned(),
                    mode: PluginSlotMode::Exclusive,
                }],
                diagnostic_findings: vec![PluginDiagnosticFinding {
                    code: PluginDiagnosticCode::SlotClaimConflict,
                    severity: PluginDiagnosticSeverity::Error,
                    phase: PluginDiagnosticPhase::Activation,
                    blocking: true,
                    plugin_id: Some("tavily-search".to_owned()),
                    source_path: Some("/tmp/tavily/loong.plugin.json".to_owned()),
                    source_kind: Some(PluginSourceKind::PackageManifest),
                    field_path: Some("slot_claims".to_owned()),
                    message: "slot claim `provider:web_search`:`tavily` conflicts with existing plugin `web-search`".to_owned(),
                    remediation: Some("choose a different slot or relax ownership intentionally".to_owned()),
                }],
                status: PluginActivationStatus::BlockedSlotClaimConflict,
                reason: "slot claim `provider:web_search`:`tavily` conflicts with existing plugin `web-search`".to_owned(),
                missing_required_env_vars: Vec::new(),
                missing_required_config_keys: Vec::new(),
                bootstrap_hint: "register http".to_owned(),
            }],
        }];
        let setup_readiness_context = PluginSetupReadinessContext::default();
        let report = execute_tool_search(
            &catalog,
            &[],
            &[],
            &setup_readiness_context,
            &activation_plans,
            "TAVILY_API_KEY",
            10,
            &[],
            true,
            false,
        );

        assert_eq!(report.results.len(), 1);
        assert!(!report.trust_filter_summary.applied);
        assert_eq!(
            report.results[0].manifest_api_version.as_deref(),
            Some("v1alpha1")
        );
        assert_eq!(report.results[0].plugin_version.as_deref(), Some("0.3.0"));
        assert_eq!(
            report.results[0].source_kind.as_deref(),
            Some("package_manifest")
        );
        assert_eq!(
            report.results[0].package_root.as_deref(),
            Some("/tmp/tavily")
        );
        assert_eq!(
            report.results[0].package_manifest_path.as_deref(),
            Some("/tmp/tavily/loong.plugin.json")
        );
        assert!(report.results[0].compatibility_shim.is_none());
        assert_eq!(
            report.results[0].provenance_summary.as_deref(),
            Some("package_manifest:/tmp/tavily/loong.plugin.json")
        );
        assert_eq!(report.results[0].trust_tier.as_deref(), Some("official"));
        assert_eq!(
            report.results[0].setup_mode.as_deref(),
            Some("metadata_only")
        );
        assert_eq!(
            report.results[0].setup_surface.as_deref(),
            Some("web_search")
        );
        assert_eq!(
            report.results[0].setup_default_env_var.as_deref(),
            Some("TAVILY_API_KEY")
        );
        assert_eq!(
            report.results[0].setup_required_env_vars,
            vec!["TAVILY_API_KEY".to_owned()]
        );
        assert!(!report.results[0].setup_ready);
        assert_eq!(
            report.results[0].missing_required_env_vars,
            vec!["TAVILY_API_KEY".to_owned()]
        );
        assert_eq!(
            report.results[0].missing_required_config_keys,
            vec!["tools.web_search.default_provider".to_owned()]
        );
        assert_eq!(
            report.results[0].slot_claims,
            vec![PluginSlotClaim {
                slot: "provider:web_search".to_owned(),
                key: "tavily".to_owned(),
                mode: PluginSlotMode::Exclusive,
            }]
        );
        assert_eq!(
            report.results[0]
                .compatibility
                .as_ref()
                .and_then(|compatibility| compatibility.host_api.as_deref()),
            Some("loong-plugin/v1")
        );
        assert_eq!(
            report.results[0]
                .compatibility
                .as_ref()
                .and_then(|compatibility| compatibility.host_version_req.as_deref()),
            Some(">=0.1.0-alpha.1")
        );
        assert_eq!(
            report.results[0].activation_status.as_deref(),
            Some("blocked_slot_claim_conflict")
        );
        assert!(
            report.results[0]
                .activation_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("provider:web_search"))
        );
        assert_eq!(report.results[0].diagnostic_findings.len(), 1);
        assert_eq!(
            report.results[0].diagnostic_findings[0].code,
            PluginDiagnosticCode::SlotClaimConflict
        );
        assert_eq!(
            report.results[0].diagnostic_findings[0].phase,
            PluginDiagnosticPhase::Activation
        );
        assert!(report.results[0].diagnostic_findings[0].blocking);
    }

    #[test]
    fn execute_tool_search_surfaces_verified_activation_attestation_for_loaded_plugins() {
        let contract = crate::spec_runtime::PluginActivationRuntimeContract {
            plugin_id: "openclaw-weather".to_owned(),
            source_path: "/tmp/openclaw-weather/openclaw.plugin.json".to_owned(),
            source_kind: PluginSourceKind::PackageManifest,
            dialect: PluginContractDialect::OpenClawModernManifest,
            dialect_version: Some("openclaw.plugin.json".to_owned()),
            compatibility_mode: PluginCompatibilityMode::OpenClawModern,
            compatibility_shim: Some(kernel::PluginCompatibilityShim {
                shim_id: "openclaw-modern-compat".to_owned(),
                family: "openclaw-modern-compat".to_owned(),
            }),
            bridge_kind: PluginBridgeKind::ProcessStdio,
            adapter_family: "openclaw-modern-compat".to_owned(),
            entrypoint_hint: "stdin/stdout::invoke".to_owned(),
            source_language: "javascript".to_owned(),
            compatibility: None,
        };
        let raw_contract = crate::spec_runtime::plugin_activation_runtime_contract_json(&contract)
            .expect("encode activation contract");
        let checksum =
            crate::spec_runtime::activation_runtime_contract_checksum_hex(raw_contract.as_bytes());

        let mut catalog = IntegrationCatalog::new();
        catalog.upsert_provider(ProviderConfig {
            provider_id: "openclaw-weather".to_owned(),
            connector_name: "weather".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_id".to_owned(), "openclaw-weather".to_owned()),
                (
                    "plugin_source_path".to_owned(),
                    "/tmp/openclaw-weather/openclaw.plugin.json".to_owned(),
                ),
                (
                    "plugin_dialect".to_owned(),
                    "openclaw_modern_manifest".to_owned(),
                ),
                (
                    "plugin_compatibility_mode".to_owned(),
                    "openclaw_modern".to_owned(),
                ),
                ("plugin_activation_contract_json".to_owned(), raw_contract),
                (
                    "plugin_activation_contract_checksum".to_owned(),
                    checksum.clone(),
                ),
                ("bridge_kind".to_owned(), "process_stdio".to_owned()),
            ]),
        });

        let setup_readiness_context = PluginSetupReadinessContext::default();
        let activation_plans: &[PluginActivationPlan] = &[];
        let report = execute_tool_search(
            &catalog,
            &[],
            &[],
            &setup_readiness_context,
            activation_plans,
            "verified",
            10,
            &[],
            true,
            false,
        );

        assert_eq!(report.results.len(), 1);
        assert_eq!(
            report.results[0]
                .activation_attestation
                .as_ref()
                .map(|attestation| attestation.integrity.as_str()),
            Some("verified")
        );
        assert_eq!(
            report.results[0]
                .activation_attestation
                .as_ref()
                .and_then(|attestation| attestation.checksum.as_deref()),
            Some(checksum.as_str())
        );
    }

    #[test]
    fn execute_tool_search_marks_setup_ready_when_requirements_are_verified() {
        let mut catalog = IntegrationCatalog::new();
        let provider = ProviderConfig {
            provider_id: "tavily".to_owned(),
            connector_name: "tavily-http".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                (
                    "plugin_setup_required_env_vars_json".to_owned(),
                    "[\"TAVILY_API_KEY\"]".to_owned(),
                ),
                (
                    "plugin_setup_required_config_keys_json".to_owned(),
                    "[\"tools.web_search.default_provider\"]".to_owned(),
                ),
            ]),
        };
        catalog.upsert_provider(provider);

        let setup_readiness_context = PluginSetupReadinessContext {
            verified_env_vars: BTreeSet::from(["TAVILY_API_KEY".to_owned()]),
            verified_config_keys: BTreeSet::from(["tools.web_search.default_provider".to_owned()]),
        };

        let report = execute_tool_search(
            &catalog,
            &[],
            &[],
            &setup_readiness_context,
            &[],
            "tavily",
            10,
            &[],
            true,
            false,
        );

        assert_eq!(report.results.len(), 1);
        assert!(report.results[0].setup_ready);
        assert!(report.results[0].missing_required_env_vars.is_empty());
        assert!(report.results[0].missing_required_config_keys.is_empty());
    }

    #[test]
    fn execute_tool_search_prefers_higher_trust_tier_when_scores_tie() {
        let mut catalog = IntegrationCatalog::new();
        catalog.upsert_provider(ProviderConfig {
            provider_id: "aaa-unverified".to_owned(),
            connector_name: "search-alpha".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_id".to_owned(), "aaa-unverified".to_owned()),
                ("plugin_trust_tier".to_owned(), "unverified".to_owned()),
                ("plugin_source_path".to_owned(), "/tmp/aaa.rs".to_owned()),
            ]),
        });
        catalog.upsert_provider(ProviderConfig {
            provider_id: "zzz-official".to_owned(),
            connector_name: "search-zeta".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_id".to_owned(), "zzz-official".to_owned()),
                ("plugin_trust_tier".to_owned(), "official".to_owned()),
                ("plugin_source_path".to_owned(), "/tmp/zzz.rs".to_owned()),
            ]),
        });

        let report = execute_tool_search(
            &catalog,
            &[],
            &[],
            &PluginSetupReadinessContext::default(),
            &[],
            "",
            10,
            &[],
            true,
            false,
        );

        assert_eq!(report.results.len(), 2);
        assert_eq!(report.results[0].trust_tier.as_deref(), Some("official"));
        assert_eq!(report.results[1].trust_tier.as_deref(), Some("unverified"));
    }

    #[test]
    fn execute_tool_search_filters_by_trust_tier_query_prefix() {
        let mut catalog = IntegrationCatalog::new();
        catalog.upsert_provider(ProviderConfig {
            provider_id: "official-search".to_owned(),
            connector_name: "official-search".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_id".to_owned(), "official-search".to_owned()),
                ("plugin_trust_tier".to_owned(), "official".to_owned()),
                (
                    "summary".to_owned(),
                    "Search across official docs".to_owned(),
                ),
            ]),
        });
        catalog.upsert_provider(ProviderConfig {
            provider_id: "verified-search".to_owned(),
            connector_name: "verified-search".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_id".to_owned(), "verified-search".to_owned()),
                (
                    "plugin_trust_tier".to_owned(),
                    "verified-community".to_owned(),
                ),
                (
                    "summary".to_owned(),
                    "Search across community docs".to_owned(),
                ),
            ]),
        });
        catalog.upsert_provider(ProviderConfig {
            provider_id: "unverified-search".to_owned(),
            connector_name: "unverified-search".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_id".to_owned(), "unverified-search".to_owned()),
                ("plugin_trust_tier".to_owned(), "unverified".to_owned()),
                ("summary".to_owned(), "Search across random docs".to_owned()),
            ]),
        });

        let report = execute_tool_search(
            &catalog,
            &[],
            &[],
            &PluginSetupReadinessContext::default(),
            &[],
            "tier:verified_community search",
            10,
            &[],
            true,
            false,
        );

        assert_eq!(report.results.len(), 1);
        assert!(report.trust_filter_summary.applied);
        assert_eq!(
            report.trust_filter_summary.query_requested_tiers,
            vec!["verified-community".to_owned()]
        );
        assert_eq!(
            report.trust_filter_summary.effective_tiers,
            vec!["verified-community".to_owned()]
        );
        assert!(!report.trust_filter_summary.conflicting_requested_tiers);
        assert_eq!(report.trust_filter_summary.filtered_out_candidates, 2);
        assert_eq!(
            report
                .trust_filter_summary
                .filtered_out_tier_counts
                .get("official"),
            Some(&1)
        );
        assert_eq!(
            report
                .trust_filter_summary
                .filtered_out_tier_counts
                .get("unverified"),
            Some(&1)
        );
        assert_eq!(report.results[0].provider_id, "verified-search");
        assert_eq!(
            report.results[0].trust_tier.as_deref(),
            Some("verified-community")
        );
    }

    #[test]
    fn execute_tool_search_filters_by_structured_trust_tiers() {
        let mut catalog = IntegrationCatalog::new();
        catalog.upsert_provider(ProviderConfig {
            provider_id: "official-search".to_owned(),
            connector_name: "official-search".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_id".to_owned(), "official-search".to_owned()),
                ("plugin_trust_tier".to_owned(), "official".to_owned()),
                (
                    "summary".to_owned(),
                    "Search across official docs".to_owned(),
                ),
            ]),
        });
        catalog.upsert_provider(ProviderConfig {
            provider_id: "verified-search".to_owned(),
            connector_name: "verified-search".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_id".to_owned(), "verified-search".to_owned()),
                (
                    "plugin_trust_tier".to_owned(),
                    "verified-community".to_owned(),
                ),
                (
                    "summary".to_owned(),
                    "Search across community docs".to_owned(),
                ),
            ]),
        });

        let report = execute_tool_search(
            &catalog,
            &[],
            &[],
            &PluginSetupReadinessContext::default(),
            &[],
            "search",
            10,
            &[PluginTrustTier::Official],
            true,
            false,
        );

        assert_eq!(report.results.len(), 1);
        assert!(report.trust_filter_summary.applied);
        assert_eq!(
            report.trust_filter_summary.structured_requested_tiers,
            vec!["official".to_owned()]
        );
        assert_eq!(
            report.trust_filter_summary.effective_tiers,
            vec!["official".to_owned()]
        );
        assert!(!report.trust_filter_summary.conflicting_requested_tiers);
        assert_eq!(report.trust_filter_summary.filtered_out_candidates, 1);
        assert_eq!(report.results[0].provider_id, "official-search");
        assert_eq!(report.results[0].trust_tier.as_deref(), Some("official"));
    }

    #[test]
    fn execute_tool_search_conflicting_query_and_structured_trust_filters_fail_closed() {
        let mut catalog = IntegrationCatalog::new();
        catalog.upsert_provider(ProviderConfig {
            provider_id: "official-search".to_owned(),
            connector_name: "official-search".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_id".to_owned(), "official-search".to_owned()),
                ("plugin_trust_tier".to_owned(), "official".to_owned()),
                (
                    "summary".to_owned(),
                    "Search across official docs".to_owned(),
                ),
            ]),
        });
        catalog.upsert_provider(ProviderConfig {
            provider_id: "verified-search".to_owned(),
            connector_name: "verified-search".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_id".to_owned(), "verified-search".to_owned()),
                (
                    "plugin_trust_tier".to_owned(),
                    "verified-community".to_owned(),
                ),
                (
                    "summary".to_owned(),
                    "Search across community docs".to_owned(),
                ),
            ]),
        });

        let report = execute_tool_search(
            &catalog,
            &[],
            &[],
            &PluginSetupReadinessContext::default(),
            &[],
            "trust:official search",
            10,
            &[PluginTrustTier::VerifiedCommunity],
            true,
            false,
        );

        assert!(report.results.is_empty());
        assert!(report.trust_filter_summary.applied);
        assert_eq!(
            report.trust_filter_summary.query_requested_tiers,
            vec!["official".to_owned()]
        );
        assert_eq!(
            report.trust_filter_summary.structured_requested_tiers,
            vec!["verified-community".to_owned()]
        );
        assert!(report.trust_filter_summary.effective_tiers.is_empty());
        assert!(report.trust_filter_summary.conflicting_requested_tiers);
        assert_eq!(report.trust_filter_summary.filtered_out_candidates, 2);
        assert_eq!(
            report
                .trust_filter_summary
                .filtered_out_tier_counts
                .get("official"),
            Some(&1)
        );
        assert_eq!(
            report
                .trust_filter_summary
                .filtered_out_tier_counts
                .get("verified-community"),
            Some(&1)
        );
    }

    #[test]
    fn execute_tool_search_derives_canonical_shim_from_compatibility_mode_metadata() {
        let mut catalog = IntegrationCatalog::new();
        catalog.upsert_provider(ProviderConfig {
            provider_id: "openclaw-weather".to_owned(),
            connector_name: "weather".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_id".to_owned(), "openclaw-weather".to_owned()),
                (
                    "plugin_source_path".to_owned(),
                    "/tmp/openclaw-weather/openclaw.plugin.json".to_owned(),
                ),
                (
                    "plugin_dialect".to_owned(),
                    "openclaw_modern_manifest".to_owned(),
                ),
                (
                    "plugin_compatibility_mode".to_owned(),
                    "openclaw_modern".to_owned(),
                ),
                ("bridge_kind".to_owned(), "process_stdio".to_owned()),
            ]),
        });

        let setup_readiness_context = PluginSetupReadinessContext::default();
        let activation_plans: &[PluginActivationPlan] = &[];
        let report = execute_tool_search(
            &catalog,
            &[],
            &[],
            &setup_readiness_context,
            activation_plans,
            "openclaw-modern-compat",
            10,
            &[],
            true,
            false,
        );

        assert_eq!(report.results.len(), 1);
        assert_eq!(
            report.results[0].compatibility_mode.as_deref(),
            Some("openclaw_modern")
        );
        assert_eq!(
            report.results[0]
                .compatibility_shim
                .as_ref()
                .map(|shim| shim.shim_id.as_str()),
            Some("openclaw-modern-compat")
        );
        assert!(report.results[0].compatibility_shim_support.is_none());
        assert!(
            report.results[0]
                .compatibility_shim_support_mismatch_reasons
                .is_empty()
        );
    }

    #[test]
    fn execute_tool_search_surfaces_shim_support_profile_and_mismatch_reasons() {
        let mut catalog = IntegrationCatalog::new();
        catalog.upsert_provider(ProviderConfig {
            provider_id: "openclaw-weather".to_owned(),
            connector_name: "weather".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_id".to_owned(), "openclaw-weather".to_owned()),
                (
                    "plugin_source_path".to_owned(),
                    "/tmp/openclaw-weather/openclaw.plugin.json".to_owned(),
                ),
                (
                    "plugin_dialect".to_owned(),
                    "openclaw_modern_manifest".to_owned(),
                ),
                (
                    "plugin_compatibility_mode".to_owned(),
                    "openclaw_modern".to_owned(),
                ),
                ("bridge_kind".to_owned(), "process_stdio".to_owned()),
            ]),
        });

        let shim = PluginCompatibilityShim {
            shim_id: "openclaw-modern-compat".to_owned(),
            family: "openclaw-modern-compat".to_owned(),
        };
        let activation_plans = vec![PluginActivationPlan {
            total_plugins: 1,
            ready_plugins: 0,
            setup_incomplete_plugins: 0,
            blocked_plugins: 1,
            candidates: vec![PluginActivationCandidate {
                plugin_id: "openclaw-weather".to_owned(),
                source_path: "/tmp/openclaw-weather/openclaw.plugin.json".to_owned(),
                source_kind: PluginSourceKind::PackageManifest,
                package_root: "/tmp/openclaw-weather".to_owned(),
                package_manifest_path: Some(
                    "/tmp/openclaw-weather/openclaw.plugin.json".to_owned(),
                ),
                trust_tier: kernel::PluginTrustTier::Unverified,
                compatibility_mode: PluginCompatibilityMode::OpenClawModern,
                compatibility_shim: Some(shim.clone()),
                compatibility_shim_support: Some(kernel::PluginCompatibilityShimSupport {
                    shim,
                    version: Some("openclaw-modern@1".to_owned()),
                    supported_dialects: BTreeSet::from([
                        PluginContractDialect::OpenClawModernManifest,
                    ]),
                    supported_bridges: BTreeSet::from([PluginBridgeKind::ProcessStdio]),
                    supported_adapter_families: BTreeSet::new(),
                    supported_source_languages: BTreeSet::from(["python".to_owned()]),
                }),
                compatibility_shim_support_mismatch_reasons: vec![
                    "source language `javascript`".to_owned(),
                ],
                bridge_kind: PluginBridgeKind::ProcessStdio,
                adapter_family: "javascript-stdio-adapter".to_owned(),
                slot_claims: Vec::new(),
                diagnostic_findings: Vec::new(),
                status: PluginActivationStatus::BlockedCompatibilityMode,
                reason: "compatibility shim profile mismatch".to_owned(),
                missing_required_env_vars: Vec::new(),
                missing_required_config_keys: Vec::new(),
                bootstrap_hint: "align compatibility shim profile".to_owned(),
            }],
        }];

        let setup_readiness_context = PluginSetupReadinessContext::default();
        let report = execute_tool_search(
            &catalog,
            &[],
            &[],
            &setup_readiness_context,
            &activation_plans,
            "openclaw-modern@1",
            10,
            &[],
            true,
            false,
        );

        assert_eq!(report.results.len(), 1);
        assert_eq!(
            report.results[0]
                .compatibility_shim_support
                .as_ref()
                .and_then(|support| support.version.as_deref()),
            Some("openclaw-modern@1")
        );
        assert_eq!(
            report.results[0].compatibility_shim_support_mismatch_reasons,
            vec!["source language `javascript`".to_owned()]
        );
    }

    #[test]
    fn execute_tool_search_surfaces_channel_bridge_contract_fields() {
        let mut catalog = IntegrationCatalog::new();
        let provider = ProviderConfig {
            provider_id: "weixin-bridge".to_owned(),
            connector_name: "weixin-clawbot-http".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_id".to_owned(), "weixin-clawbot-bridge".to_owned()),
                ("plugin_channel_id".to_owned(), "weixin".to_owned()),
                (
                    "plugin_channel_bridge_transport_family".to_owned(),
                    "wechat_clawbot_ilink_bridge".to_owned(),
                ),
                (
                    "plugin_channel_bridge_target_contract".to_owned(),
                    "weixin:<account>:contact:<id> | weixin:<account>:room:<id>".to_owned(),
                ),
                (
                    "plugin_channel_bridge_account_scope".to_owned(),
                    "multi_account".to_owned(),
                ),
                ("plugin_channel_bridge_ready".to_owned(), "true".to_owned()),
                (
                    "plugin_channel_bridge_missing_fields_json".to_owned(),
                    "[]".to_owned(),
                ),
                (
                    "summary".to_owned(),
                    "ClawBot-compatible bridge for the weixin channel surface".to_owned(),
                ),
            ]),
        };
        catalog.upsert_provider(provider);

        let report = execute_tool_search(
            &catalog,
            &[],
            &[],
            &PluginSetupReadinessContext::default(),
            &[],
            "weixin clawbot",
            10,
            &[],
            true,
            false,
        );

        assert_eq!(report.results.len(), 1);
        assert_eq!(report.results[0].channel_id.as_deref(), Some("weixin"));
        assert_eq!(
            report.results[0].channel_bridge_transport_family.as_deref(),
            Some("wechat_clawbot_ilink_bridge")
        );
        assert_eq!(
            report.results[0].channel_bridge_target_contract.as_deref(),
            Some("weixin:<account>:contact:<id> | weixin:<account>:room:<id>")
        );
        assert_eq!(
            report.results[0].channel_bridge_account_scope.as_deref(),
            Some("multi_account")
        );
        assert_eq!(report.results[0].channel_bridge_ready, Some(true));
        assert!(report.results[0].channel_bridge_missing_fields.is_empty());
    }

    #[test]
    fn execute_tool_search_surfaces_incomplete_channel_bridge_contract_fields() {
        let mut catalog = IntegrationCatalog::new();
        let provider = ProviderConfig {
            provider_id: "weixin-bridge".to_owned(),
            connector_name: "weixin-clawbot-http".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([
                ("plugin_id".to_owned(), "weixin-clawbot-bridge".to_owned()),
                ("plugin_channel_id".to_owned(), "weixin".to_owned()),
                ("plugin_channel_bridge_ready".to_owned(), "false".to_owned()),
                (
                    "plugin_channel_bridge_missing_fields_json".to_owned(),
                    "[\"metadata.transport_family\",\"metadata.target_contract\"]".to_owned(),
                ),
            ]),
        };
        catalog.upsert_provider(provider);

        let report = execute_tool_search(
            &catalog,
            &[],
            &[],
            &PluginSetupReadinessContext::default(),
            &[],
            "weixin",
            10,
            &[],
            true,
            false,
        );

        assert_eq!(report.results.len(), 1);
        assert_eq!(report.results[0].channel_bridge_ready, Some(false));
        assert_eq!(
            report.results[0].channel_bridge_missing_fields,
            vec![
                "metadata.transport_family".to_owned(),
                "metadata.target_contract".to_owned(),
            ]
        );
    }
}
