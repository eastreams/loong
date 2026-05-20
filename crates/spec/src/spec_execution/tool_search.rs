use std::collections::{BTreeMap, BTreeSet};

use kernel::{
    AuditEventKind, IntegrationCatalog, LoongKernel, PluginActivationCandidate,
    PluginActivationInventoryEntry, PluginActivationPlan, PluginBridgeKind, PluginCompatibility,
    PluginCompatibilityMode, PluginCompatibilityShim, PluginContractDialect,
    PluginDiagnosticFinding, PluginScanReport, PluginSetupReadinessContext, PluginSlotClaim,
    PluginTranslationReport, PluginTrustTier, StaticPolicyEngine,
    evaluate_plugin_setup_requirements, plugin_provenance_summary_for_descriptor,
};
use serde_json::Value;

use super::descriptor_bridge_kind;
use crate::spec_runtime::{
    ToolSearchChannelBridgeSnapshot, ToolSearchEntry, ToolSearchOperationSummary,
    ToolSearchOperationSummaryEntry, ToolSearchResult, ToolSearchTrustFilterSummary,
    detect_provider_bridge_kind, provider_plugin_activation_attestation_result,
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
    channel_bridge: Option<kernel::CanonicalPluginChannelBridgeContract>,
}

pub(super) fn build_tool_search_operation_summary(
    outcome: &Value,
) -> Option<ToolSearchOperationSummary> {
    let payload = outcome.as_object()?;
    let results = payload.get("results")?.as_array()?;
    let top_results = results
        .iter()
        .take(3)
        .filter_map(build_tool_search_operation_summary_entry)
        .collect::<Vec<_>>();
    let trust_filter_summary = payload
        .get("trust_filter_summary")
        .cloned()
        .and_then(|value| serde_json::from_value::<ToolSearchTrustFilterSummary>(value).ok())
        .unwrap_or_default();
    let query = payload
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let returned = payload
        .get("returned")
        .and_then(Value::as_u64)
        .map_or(results.len(), |value| value as usize);
    let trust_tiers = payload
        .get("trust_tiers")
        .and_then(Value::as_array)
        .map(|tiers| {
            tiers
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(ToolSearchOperationSummary {
        headline: build_tool_search_operation_headline(
            &query,
            returned,
            &trust_tiers,
            &trust_filter_summary,
            &top_results,
        ),
        query,
        returned,
        trust_tiers,
        trust_filter_summary,
        top_results,
    })
}

fn build_tool_search_operation_summary_entry(
    value: &Value,
) -> Option<ToolSearchOperationSummaryEntry> {
    let entry = value.as_object()?;
    Some(ToolSearchOperationSummaryEntry {
        tool_id: entry.get("tool_id")?.as_str()?.to_owned(),
        provider_id: entry.get("provider_id")?.as_str()?.to_owned(),
        connector_name: entry.get("connector_name")?.as_str()?.to_owned(),
        trust_tier: entry
            .get("trust_tier")
            .and_then(Value::as_str)
            .map(str::to_owned),
        bridge_kind: entry.get("bridge_kind")?.as_str()?.to_owned(),
        score: entry
            .get("score")
            .and_then(Value::as_u64)
            .map_or(0, |value| value as u32),
        setup_ready: entry
            .get("setup_ready")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        deferred: entry
            .get("deferred")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        loaded: entry
            .get("loaded")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn build_tool_search_operation_headline(
    query: &str,
    returned: usize,
    trust_tiers: &[String],
    trust_filter_summary: &ToolSearchTrustFilterSummary,
    top_results: &[ToolSearchOperationSummaryEntry],
) -> String {
    let result_noun = if returned == 1 { "result" } else { "results" };
    let mut parts = vec![format!("returned {returned} {result_noun}")];

    if trust_filter_summary.applied {
        let scope = if trust_filter_summary.effective_tiers.is_empty() {
            "none".to_owned()
        } else {
            trust_filter_summary.effective_tiers.join(",")
        };
        parts.push(format!("trust_scope={scope}"));
        if trust_filter_summary.filtered_out_candidates > 0 {
            let filtered_noun = if trust_filter_summary.filtered_out_candidates == 1 {
                "candidate"
            } else {
                "candidates"
            };
            parts.push(format!(
                "filtered_out={} {filtered_noun}",
                trust_filter_summary.filtered_out_candidates
            ));
        }
        if trust_filter_summary.conflicting_requested_tiers {
            parts.push("conflicting_trust_filters=true".to_owned());
        }
    } else if !trust_tiers.is_empty() {
        parts.push(format!("requested_tiers={}", trust_tiers.join(",")));
    }

    if let Some(first) = top_results.first() {
        parts.push(format!("top_match={}", first.provider_id));
    }

    if query.is_empty() {
        parts.join("; ")
    } else {
        format!("query=\"{query}\"; {}", parts.join("; "))
    }
}

pub(super) fn emit_tool_search_audit_event(
    kernel: &LoongKernel<StaticPolicyEngine>,
    pack_id: &str,
    agent_id: &str,
    summary: &ToolSearchOperationSummary,
) -> Result<(), String> {
    let top_provider_ids = summary
        .top_results
        .iter()
        .map(|entry| entry.provider_id.clone())
        .collect::<Vec<_>>();

    kernel
        .record_audit_event(
            Some(agent_id),
            AuditEventKind::ToolSearchEvaluated {
                pack_id: pack_id.to_owned(),
                query: summary.query.clone(),
                returned: summary.returned,
                trust_filter_applied: summary.trust_filter_summary.applied,
                query_requested_tiers: summary.trust_filter_summary.query_requested_tiers.clone(),
                structured_requested_tiers: summary
                    .trust_filter_summary
                    .structured_requested_tiers
                    .clone(),
                effective_tiers: summary.trust_filter_summary.effective_tiers.clone(),
                conflicting_requested_tiers: summary
                    .trust_filter_summary
                    .conflicting_requested_tiers,
                filtered_out_candidates: summary.trust_filter_summary.filtered_out_candidates,
                filtered_out_tier_counts: summary
                    .trust_filter_summary
                    .filtered_out_tier_counts
                    .clone(),
                top_provider_ids,
            },
        )
        .map_err(|error| format!("failed to record tool search audit event: {error}"))
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
            let channel_bridge = channel_bridge.map(kernel::canonical_channel_bridge_contract);

            translation_by_key.insert(
                (entry.source_path.clone(), entry.plugin_id.clone()),
                ToolSearchTranslationSnapshot {
                    bridge_kind: entry.runtime.bridge_kind,
                    adapter_family: entry.runtime.adapter_family.clone(),
                    entrypoint_hint: entry.runtime.entrypoint_hint.clone(),
                    source_language: entry.runtime.source_language.clone(),
                    channel_bridge,
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
        let mut channel_id = tool_search_channel_id_from_provider_metadata(&provider.metadata);
        let mut channel_bridge =
            tool_search_bridge_snapshot_from_provider_metadata(&provider.metadata);
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
                channel_id = snapshot
                    .channel_bridge
                    .as_ref()
                    .and_then(|bridge| bridge.channel_id.clone())
                    .or(channel_id);
                merge_tool_search_bridge_snapshot(
                    &mut channel_bridge,
                    tool_search_bridge_snapshot_from_canonical_translation(
                        snapshot.channel_bridge.as_ref(),
                    ),
                );
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
                channel_bridge,
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
                .and_then(|snapshot| snapshot.channel_bridge.as_ref())
                .and_then(|bridge| bridge.channel_id.clone());
            let mut channel_bridge =
                tool_search_bridge_snapshot_from_manifest_metadata(&manifest.metadata);
            merge_tool_search_bridge_snapshot(
                &mut channel_bridge,
                translation.and_then(|snapshot| {
                    tool_search_bridge_snapshot_from_canonical_translation(
                        snapshot.channel_bridge.as_ref(),
                    )
                }),
            );

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
                    channel_bridge: channel_bridge.clone(),
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
            merge_tool_search_bridge_snapshot(&mut entry.channel_bridge, Some(channel_bridge));
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
            channel_bridge: entry.channel_bridge,
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

fn tool_search_bridge_snapshot_from_provider_metadata(
    metadata: &BTreeMap<String, String>,
) -> ToolSearchChannelBridgeSnapshot {
    let canonical = metadata
        .get(crate::spec_runtime::PLUGIN_CHANNEL_BRIDGE_CONTRACT_METADATA_KEY)
        .and_then(|raw| {
            serde_json::from_str::<kernel::CanonicalPluginChannelBridgeContract>(raw).ok()
        });

    ToolSearchChannelBridgeSnapshot {
        transport_family: canonical
            .as_ref()
            .and_then(|bridge| bridge.transport_family.clone()),
        target_contract: canonical
            .as_ref()
            .and_then(|bridge| bridge.target_contract.clone()),
        account_scope: canonical
            .as_ref()
            .and_then(|bridge| bridge.account_scope.clone()),
        ready: canonical.as_ref().map(|bridge| bridge.readiness.ready),
        missing_fields: canonical
            .map(|bridge| bridge.readiness.missing_fields)
            .unwrap_or_default(),
    }
}

fn tool_search_channel_id_from_provider_metadata(
    metadata: &BTreeMap<String, String>,
) -> Option<String> {
    metadata
        .get(crate::spec_runtime::PLUGIN_CHANNEL_BRIDGE_CONTRACT_METADATA_KEY)
        .and_then(|raw| {
            serde_json::from_str::<kernel::CanonicalPluginChannelBridgeContract>(raw).ok()
        })
        .and_then(|bridge| bridge.channel_id)
}

fn tool_search_bridge_snapshot_from_manifest_metadata(
    metadata: &BTreeMap<String, String>,
) -> ToolSearchChannelBridgeSnapshot {
    let canonical = metadata
        .get(crate::spec_runtime::PLUGIN_CHANNEL_BRIDGE_CONTRACT_METADATA_KEY)
        .and_then(|raw| {
            serde_json::from_str::<kernel::CanonicalPluginChannelBridgeContract>(raw).ok()
        });

    ToolSearchChannelBridgeSnapshot {
        transport_family: canonical
            .as_ref()
            .and_then(|bridge| bridge.transport_family.clone()),
        target_contract: canonical
            .as_ref()
            .and_then(|bridge| bridge.target_contract.clone()),
        account_scope: canonical
            .as_ref()
            .and_then(|bridge| bridge.account_scope.clone()),
        ready: canonical.as_ref().map(|bridge| bridge.readiness.ready),
        missing_fields: canonical
            .map(|bridge| bridge.readiness.missing_fields)
            .unwrap_or_default(),
    }
}

fn tool_search_bridge_snapshot_from_canonical_translation(
    bridge: Option<&kernel::CanonicalPluginChannelBridgeContract>,
) -> Option<ToolSearchChannelBridgeSnapshot> {
    bridge.map(|bridge| ToolSearchChannelBridgeSnapshot {
        transport_family: bridge.transport_family.clone(),
        target_contract: bridge.target_contract.clone(),
        account_scope: bridge.account_scope.clone(),
        ready: Some(bridge.readiness.ready),
        missing_fields: bridge.readiness.missing_fields.clone(),
    })
}

fn merge_tool_search_bridge_snapshot(
    target: &mut ToolSearchChannelBridgeSnapshot,
    source: Option<ToolSearchChannelBridgeSnapshot>,
) {
    let Some(source) = source else {
        return;
    };

    target.transport_family = source.transport_family.or(target.transport_family.take());
    target.target_contract = source.target_contract.or(target.target_contract.take());
    target.account_scope = source.account_scope.or(target.account_scope.take());
    if let Some(ready) = source.ready {
        target.ready = Some(ready);
        target.missing_fields = source.missing_fields;
    } else if target.missing_fields.is_empty() {
        target.missing_fields = source.missing_fields;
    }
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
        .channel_bridge
        .transport_family
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let channel_bridge_target_contract = entry
        .channel_bridge
        .target_contract
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let channel_bridge_account_scope = entry
        .channel_bridge
        .account_scope
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let channel_bridge_missing_fields = entry
        .channel_bridge
        .missing_fields
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
mod tests;
