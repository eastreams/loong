use std::collections::BTreeMap;

use kernel::{
    IntegrationCatalog, PluginActivationCandidate, PluginActivationInventoryEntry,
    PluginActivationPlan, PluginActivationStatus, PluginBridgeKind, PluginDiagnosticFinding,
    PluginScanReport, PluginTranslationReport, plugin_native_extension_declarations_from_metadata,
};

use crate::spec_runtime::{
    PluginInventoryEntry, PluginInventoryResult, provider_plugin_activation_attestation_result,
    provider_plugin_runtime_health_result,
};

pub(super) fn execute_plugin_inventory(
    integration_catalog: &IntegrationCatalog,
    plugin_scan_reports: &[PluginScanReport],
    plugin_translation_reports: &[PluginTranslationReport],
    plugin_activation_plans: &[PluginActivationPlan],
    query: &str,
    limit: usize,
    include_ready: bool,
    include_blocked: bool,
    include_deferred: bool,
    include_examples: bool,
) -> Vec<PluginInventoryResult> {
    collect_plugin_inventory_results(
        integration_catalog,
        plugin_scan_reports,
        plugin_translation_reports,
        plugin_activation_plans,
        query,
        include_ready,
        include_blocked,
        include_deferred,
        include_examples,
    )
    .into_iter()
    .take(limit.clamp(1, 100))
    .collect()
}

pub(super) fn collect_plugin_inventory_results(
    integration_catalog: &IntegrationCatalog,
    plugin_scan_reports: &[PluginScanReport],
    plugin_translation_reports: &[PluginTranslationReport],
    plugin_activation_plans: &[PluginActivationPlan],
    query: &str,
    include_ready: bool,
    include_blocked: bool,
    include_deferred: bool,
    include_examples: bool,
) -> Vec<PluginInventoryResult> {
    let mut translation_by_key: BTreeMap<
        (String, String),
        (PluginBridgeKind, String, String, String),
    > = BTreeMap::new();
    let mut activation_candidate_by_key: BTreeMap<(String, String), PluginActivationCandidate> =
        BTreeMap::new();
    let mut activation_by_key: BTreeMap<(String, String), PluginActivationInventoryEntry> =
        BTreeMap::new();
    let mut loaded_provider_metadata_by_key: BTreeMap<(String, String), BTreeMap<String, String>> =
        BTreeMap::new();
    let mut scan_diagnostics_by_key: BTreeMap<(String, String), Vec<PluginDiagnosticFinding>> =
        BTreeMap::new();

    for report in plugin_translation_reports {
        for entry in &report.entries {
            translation_by_key.insert(
                (entry.source_path.clone(), entry.plugin_id.clone()),
                (
                    entry.runtime.bridge_kind,
                    entry.runtime.adapter_family.clone(),
                    entry.runtime.entrypoint_hint.clone(),
                    entry.runtime.source_language.clone(),
                ),
            );
        }
    }

    for (translation, plan) in plugin_translation_reports
        .iter()
        .zip(plugin_activation_plans.iter())
    {
        for entry in plan.inventory_entries(translation) {
            activation_by_key.insert((entry.source_path.clone(), entry.plugin_id.clone()), entry);
        }
    }

    for plan in plugin_activation_plans {
        for candidate in &plan.candidates {
            activation_candidate_by_key.insert(
                (candidate.source_path.clone(), candidate.plugin_id.clone()),
                candidate.clone(),
            );
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
        let Some(source_path) = provider.metadata.get("plugin_source_path") else {
            continue;
        };
        let Some(plugin_id) = provider.metadata.get("plugin_id") else {
            continue;
        };
        loaded_provider_metadata_by_key.insert(
            (source_path.clone(), plugin_id.clone()),
            provider.metadata.clone(),
        );
    }

    let mut entries = Vec::new();
    for report in plugin_scan_reports {
        for descriptor in &report.descriptors {
            let manifest = &descriptor.manifest;
            let extension_declarations =
                plugin_native_extension_declarations_from_metadata(&manifest.metadata);
            let key = (descriptor.path.clone(), manifest.plugin_id.clone());
            let translation = translation_by_key.get(&key);
            let activation = activation_by_key.get(&key);
            let activation_candidate = activation_candidate_by_key.get(&key);
            let is_deferred = manifest.defer_loading;
            let activation_status = activation
                .and_then(|entry| entry.activation_status)
                .map(|status| status.as_str().to_owned())
                .or_else(|| {
                    activation_candidate.map(|candidate| candidate.status.as_str().to_owned())
                });
            let is_blocked = activation_status
                .as_deref()
                .is_some_and(plugin_inventory_status_is_blocked);
            let bridge_kind = translation
                .map(|(bridge, _, _, _)| *bridge)
                .or_else(|| activation.map(|entry| entry.bridge_kind))
                .or_else(|| activation_candidate.map(|candidate| candidate.bridge_kind))
                .unwrap_or(PluginBridgeKind::Unknown);
            let adapter_family = translation
                .map(|(_, adapter, _, _)| adapter.clone())
                .or_else(|| activation.map(|entry| entry.adapter_family.clone()))
                .or_else(|| activation_candidate.map(|candidate| candidate.adapter_family.clone()));
            let entrypoint_hint = translation
                .map(|(_, _, entrypoint, _)| entrypoint.clone())
                .or_else(|| activation.map(|entry| entry.entrypoint_hint.clone()))
                .or_else(|| manifest.endpoint.clone());
            let source_language = translation
                .map(|(_, _, _, language)| language.clone())
                .or_else(|| activation.map(|entry| entry.source_language.clone()))
                .or_else(|| Some(descriptor.language.clone()));

            if is_deferred {
                if !include_deferred {
                    continue;
                }
            } else if is_blocked {
                if !include_blocked {
                    continue;
                }
            } else if !include_ready {
                continue;
            }

            entries.push(PluginInventoryEntry {
                manifest_api_version: activation
                    .and_then(|entry| entry.manifest_api_version.clone())
                    .or_else(|| manifest.api_version.clone()),
                plugin_version: activation
                    .and_then(|entry| entry.plugin_version.clone())
                    .or_else(|| manifest.version.clone()),
                dialect: activation
                    .map(|entry| entry.dialect)
                    .unwrap_or(descriptor.dialect),
                dialect_version: activation
                    .and_then(|entry| entry.dialect_version.clone())
                    .or_else(|| descriptor.dialect_version.clone()),
                compatibility_mode: activation
                    .map(|entry| entry.compatibility_mode)
                    .unwrap_or(descriptor.compatibility_mode),
                compatibility_shim: activation
                    .and_then(|entry| entry.compatibility_shim.clone())
                    .or_else(|| {
                        activation_candidate
                            .and_then(|candidate| candidate.compatibility_shim.clone())
                    })
                    .or_else(|| {
                        kernel::PluginCompatibilityShim::for_mode(descriptor.compatibility_mode)
                    }),
                compatibility_shim_support: activation
                    .and_then(|entry| entry.compatibility_shim_support.clone())
                    .or_else(|| {
                        activation_candidate
                            .and_then(|candidate| candidate.compatibility_shim_support.clone())
                    }),
                compatibility_shim_support_mismatch_reasons: activation
                    .map(|entry| entry.compatibility_shim_support_mismatch_reasons.clone())
                    .or_else(|| {
                        activation_candidate.map(|candidate| {
                            candidate
                                .compatibility_shim_support_mismatch_reasons
                                .clone()
                        })
                    })
                    .unwrap_or_default(),
                plugin_id: manifest.plugin_id.clone(),
                connector_name: manifest.connector_name.clone(),
                provider_id: manifest.provider_id.clone(),
                source_path: descriptor.path.clone(),
                source_kind: descriptor.source_kind.as_str().to_owned(),
                package_root: descriptor.package_root.clone(),
                package_manifest_path: descriptor.package_manifest_path.clone(),
                bridge_kind,
                adapter_family,
                entrypoint_hint,
                source_language,
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
                slot_claims: manifest.slot_claims.clone(),
                diagnostic_findings: activation
                    .map(|entry| entry.diagnostic_findings.clone())
                    .or_else(|| {
                        activation_candidate.map(|candidate| candidate.diagnostic_findings.clone())
                    })
                    .unwrap_or_else(|| {
                        scan_diagnostics_by_key
                            .get(&key)
                            .cloned()
                            .unwrap_or_default()
                    }),
                compatibility: activation
                    .and_then(|entry| entry.compatibility.clone())
                    .or_else(|| manifest.compatibility.clone()),
                activation_status,
                activation_reason: activation
                    .and_then(|entry| entry.activation_reason.clone())
                    .or_else(|| activation_candidate.map(|candidate| candidate.reason.clone())),
                activation_attestation: loaded_provider_metadata_by_key
                    .get(&key)
                    .and_then(provider_plugin_activation_attestation_result),
                runtime_health: loaded_provider_metadata_by_key
                    .get(&key)
                    .and_then(provider_plugin_runtime_health_result),
                bootstrap_hint: activation
                    .and_then(|entry| entry.bootstrap_hint.clone())
                    .or_else(|| {
                        activation_candidate.map(|candidate| candidate.bootstrap_hint.clone())
                    }),
                summary: manifest.summary.clone(),
                tags: manifest.tags.clone(),
                capabilities: manifest
                    .capabilities
                    .iter()
                    .map(|capability| capability.as_str().to_owned())
                    .collect(),
                extension_contract: extension_declarations.contract.clone(),
                extension_family: extension_declarations.family.clone(),
                extension_trust_lane: extension_declarations.trust_lane.clone(),
                extension_facets: extension_declarations.facets.clone(),
                extension_methods: extension_declarations.methods.clone(),
                extension_events: extension_declarations.events.clone(),
                extension_host_actions: extension_declarations.host_actions.clone(),
                extension_metadata_issues: extension_declarations.metadata_issues.clone(),
                input_examples: manifest.input_examples.clone(),
                output_examples: manifest.output_examples.clone(),
                deferred: manifest.defer_loading,
                loaded: loaded_provider_metadata_by_key.contains_key(&key),
            });
        }
    }

    let query_normalized = query.trim().to_ascii_lowercase();
    let tokens = query_normalized
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();

    let mut ranked = entries
        .into_iter()
        .filter_map(|entry| {
            let blocked = entry
                .activation_status
                .as_deref()
                .is_some_and(plugin_inventory_status_is_blocked);
            let score = plugin_inventory_score(&entry, &query_normalized, &tokens);
            if query_normalized.is_empty() || score > 0 {
                Some((blocked, score, entry))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    ranked.sort_by(
        |(left_blocked, left_score, left), (right_blocked, right_score, right)| {
            if query_normalized.is_empty() {
                left_blocked
                    .cmp(right_blocked)
                    .then_with(|| right.loaded.cmp(&left.loaded))
                    .then_with(|| left.plugin_id.cmp(&right.plugin_id))
                    .then_with(|| left.source_path.cmp(&right.source_path))
            } else {
                right_score
                    .cmp(left_score)
                    .then_with(|| left_blocked.cmp(right_blocked))
                    .then_with(|| right.loaded.cmp(&left.loaded))
                    .then_with(|| left.plugin_id.cmp(&right.plugin_id))
                    .then_with(|| left.source_path.cmp(&right.source_path))
            }
        },
    );

    ranked
        .into_iter()
        .map(|(_, _, entry)| PluginInventoryResult {
            manifest_api_version: entry.manifest_api_version,
            plugin_version: entry.plugin_version,
            dialect: entry.dialect.as_str().to_owned(),
            dialect_version: entry.dialect_version,
            compatibility_mode: entry.compatibility_mode.as_str().to_owned(),
            compatibility_shim: entry.compatibility_shim,
            compatibility_shim_support: entry.compatibility_shim_support,
            compatibility_shim_support_mismatch_reasons: entry
                .compatibility_shim_support_mismatch_reasons,
            plugin_id: entry.plugin_id,
            connector_name: entry.connector_name,
            provider_id: entry.provider_id,
            source_path: entry.source_path,
            source_kind: entry.source_kind,
            package_root: entry.package_root,
            package_manifest_path: entry.package_manifest_path,
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
            slot_claims: entry.slot_claims,
            diagnostic_findings: entry.diagnostic_findings,
            compatibility: entry.compatibility,
            activation_status: entry.activation_status,
            activation_reason: entry.activation_reason,
            activation_attestation: entry.activation_attestation,
            runtime_health: entry.runtime_health,
            bootstrap_hint: entry.bootstrap_hint,
            summary: entry.summary,
            tags: entry.tags,
            capabilities: entry.capabilities,
            extension_contract: entry.extension_contract,
            extension_family: entry.extension_family,
            extension_trust_lane: entry.extension_trust_lane,
            extension_facets: entry.extension_facets,
            extension_methods: entry.extension_methods,
            extension_events: entry.extension_events,
            extension_host_actions: entry.extension_host_actions,
            extension_metadata_issues: entry.extension_metadata_issues,
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
            deferred: entry.deferred,
            loaded: entry.loaded,
        })
        .collect()
}

fn plugin_inventory_status_is_blocked(status: &str) -> bool {
    if status == PluginActivationStatus::Ready.as_str() {
        return false;
    }

    if status == PluginActivationStatus::SetupIncomplete.as_str() {
        return false;
    }

    true
}

fn plugin_inventory_score(entry: &PluginInventoryEntry, query: &str, tokens: &[String]) -> u32 {
    if query.is_empty() {
        return 0;
    }

    let plugin_id = entry.plugin_id.to_ascii_lowercase();
    let connector = entry.connector_name.to_ascii_lowercase();
    let provider = entry.provider_id.to_ascii_lowercase();
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
    let dialect = entry.dialect.as_str().to_ascii_lowercase();
    let dialect_version = entry
        .dialect_version
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let compatibility_mode = entry.compatibility_mode.as_str().to_ascii_lowercase();
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
    let source_path = entry.source_path.to_ascii_lowercase();
    let source_kind = entry.source_kind.to_ascii_lowercase();
    let package_root = entry.package_root.to_ascii_lowercase();
    let package_manifest_path = entry
        .package_manifest_path
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let bridge_kind = entry.bridge_kind.as_str().to_ascii_lowercase();
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
    let bootstrap_hint = entry
        .bootstrap_hint
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let summary = entry
        .summary
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let tags = entry
        .tags
        .iter()
        .map(|tag| tag.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let setup_required_env_vars = entry
        .setup_required_env_vars
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let setup_recommended_env_vars = entry
        .setup_recommended_env_vars
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let setup_required_config_keys = entry
        .setup_required_config_keys
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let setup_docs_urls = entry
        .setup_docs_urls
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
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
    let slot_claim_tokens = entry
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
        .collect::<Vec<_>>();
    let diagnostics = diagnostic_haystack(&entry.diagnostic_findings).to_ascii_lowercase();

    let mut score = 0_u32;
    if plugin_id == query {
        score = score.saturating_add(160);
    } else if plugin_id.contains(query) {
        score = score.saturating_add(120);
    }
    if connector == query {
        score = score.saturating_add(110);
    } else if connector.contains(query) {
        score = score.saturating_add(80);
    }
    if provider == query {
        score = score.saturating_add(100);
    } else if provider.contains(query) {
        score = score.saturating_add(70);
    }
    if manifest_api_version.contains(query) {
        score = score.saturating_add(18);
    }
    if plugin_version.contains(query) {
        score = score.saturating_add(22);
    }
    if dialect.contains(query) {
        score = score.saturating_add(26);
    }
    if dialect_version.contains(query) {
        score = score.saturating_add(12);
    }
    if compatibility_mode.contains(query) {
        score = score.saturating_add(24);
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
    if source_path.contains(query) {
        score = score.saturating_add(36);
    }
    if source_kind.contains(query) {
        score = score.saturating_add(10);
    }
    if package_root.contains(query) {
        score = score.saturating_add(16);
    }
    if package_manifest_path.contains(query) {
        score = score.saturating_add(20);
    }
    if bridge_kind.contains(query) {
        score = score.saturating_add(20);
    }
    if adapter_family.contains(query) {
        score = score.saturating_add(18);
    }
    if entrypoint_hint.contains(query) {
        score = score.saturating_add(12);
    }
    if source_language.contains(query) {
        score = score.saturating_add(12);
    }
    if setup_mode.contains(query) {
        score = score.saturating_add(12);
    }
    if setup_surface.contains(query) {
        score = score.saturating_add(18);
    }
    if setup_default_env_var.contains(query) {
        score = score.saturating_add(22);
    }
    if setup_remediation.contains(query) {
        score = score.saturating_add(10);
    }
    if compatibility_host_api.contains(query) {
        score = score.saturating_add(16);
    }
    if compatibility_host_version_req.contains(query) {
        score = score.saturating_add(12);
    }
    if activation_status.contains(query) {
        score = score.saturating_add(20);
    }
    if activation_reason.contains(query) {
        score = score.saturating_add(12);
    }
    if activation_attestation_integrity.contains(query) {
        score = score.saturating_add(14);
    }
    if activation_attestation_issue.contains(query) {
        score = score.saturating_add(18);
    }
    if activation_attestation_checksum.contains(query)
        || activation_attestation_computed_checksum.contains(query)
    {
        score = score.saturating_add(10);
    }
    if diagnostics.contains(query) {
        score = score.saturating_add(14);
    }
    if bootstrap_hint.contains(query) {
        score = score.saturating_add(12);
    }
    if summary.contains(query) {
        score = score.saturating_add(45);
    }
    if slot_claim_tokens.iter().any(|token| token == query) {
        score = score.saturating_add(36);
    } else if slot_claim_tokens.iter().any(|token| token.contains(query)) {
        score = score.saturating_add(20);
    }
    if tags.iter().any(|tag| tag == query) {
        score = score.saturating_add(40);
    } else if tags.iter().any(|tag| tag.contains(query)) {
        score = score.saturating_add(20);
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
        score = score.saturating_add(26);
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
    if setup_docs_urls.iter().any(|value| value.contains(query)) {
        score = score.saturating_add(8);
    }

    let haystack = vec![
        plugin_id,
        connector,
        provider,
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
        source_path,
        source_kind,
        package_root,
        package_manifest_path,
        bridge_kind,
        adapter_family,
        entrypoint_hint,
        source_language,
        setup_mode,
        setup_surface,
        setup_default_env_var,
        setup_remediation,
        compatibility_host_api,
        compatibility_host_version_req,
        activation_status,
        activation_reason,
        activation_attestation_integrity,
        activation_attestation_issue,
        activation_attestation_checksum,
        activation_attestation_computed_checksum,
        diagnostics,
        bootstrap_hint,
        summary,
        slot_claim_tokens.join(" "),
        tags.join(" "),
        setup_required_env_vars.join(" "),
        setup_recommended_env_vars.join(" "),
        setup_required_config_keys.join(" "),
    ]
    .join(" ");

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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use kernel::{
        Capability, IntegrationCatalog, PluginActivationCandidate, PluginActivationPlan,
        PluginActivationStatus, PluginBridgeKind, PluginCompatibility, PluginCompatibilityMode,
        PluginContractDialect, PluginDescriptor, PluginDiagnosticCode, PluginDiagnosticFinding,
        PluginDiagnosticPhase, PluginDiagnosticSeverity, PluginManifest, PluginScanReport,
        PluginSetup, PluginSetupMode, PluginSlotClaim, PluginSlotMode, PluginSourceKind,
        PluginTranslationReport, ProviderConfig,
    };

    use super::*;

    #[test]
    fn execute_plugin_inventory_surfaces_blocked_plugins_with_setup_truth() {
        let descriptor = PluginDescriptor {
            path: "/tmp/tavily/loong.plugin.json".to_owned(),
            source_kind: PluginSourceKind::PackageManifest,
            dialect: PluginContractDialect::LoongPackageManifest,
            dialect_version: Some("v1alpha1".to_owned()),
            compatibility_mode: PluginCompatibilityMode::Native,
            package_root: "/tmp/tavily".to_owned(),
            package_manifest_path: Some("/tmp/tavily/loong.plugin.json".to_owned()),
            language: "manifest".to_owned(),
            manifest: PluginManifest {
                api_version: Some("v1alpha1".to_owned()),
                version: Some("0.3.0".to_owned()),
                plugin_id: "tavily-search".to_owned(),
                provider_id: "tavily".to_owned(),
                connector_name: "tavily-http".to_owned(),
                channel_id: Some("primary".to_owned()),
                endpoint: Some("https://example.com/search".to_owned()),
                capabilities: BTreeSet::from([Capability::InvokeConnector]),
                trust_tier: kernel::PluginTrustTier::Unverified,
                metadata: BTreeMap::new(),
                summary: Some("Search plugin".to_owned()),
                tags: vec!["search".to_owned()],
                input_examples: vec![serde_json::json!({"query": "weather"})],
                output_examples: vec![serde_json::json!({"status": "ok"})],
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
                slot_claims: vec![PluginSlotClaim {
                    slot: "provider:web_search".to_owned(),
                    key: "tavily".to_owned(),
                    mode: PluginSlotMode::Exclusive,
                }],
                compatibility: Some(PluginCompatibility {
                    host_api: Some("loong-plugin/v1".to_owned()),
                    host_version_req: Some(">=0.1.0-alpha.1".to_owned()),
                }),
            },
        };
        let scan_reports = vec![PluginScanReport {
            scanned_files: 1,
            matched_plugins: 1,
            diagnostic_findings: Vec::new(),
            descriptors: vec![descriptor.clone()],
        }];
        let translation_reports = vec![PluginTranslationReport {
            translated_plugins: 1,
            bridge_distribution: BTreeMap::from([("http_json".to_owned(), 1)]),
            entries: vec![kernel::PluginIR {
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
                trust_tier: kernel::PluginTrustTier::Unverified,
                metadata: descriptor.manifest.metadata.clone(),
                source_path: descriptor.path.clone(),
                source_kind: descriptor.source_kind,
                package_root: descriptor.package_root.clone(),
                package_manifest_path: descriptor.package_manifest_path.clone(),
                diagnostic_findings: Vec::new(),
                setup: descriptor.manifest.setup.clone(),
                channel_bridge: None,
                slot_claims: descriptor.manifest.slot_claims.clone(),
                compatibility: descriptor.manifest.compatibility,
                runtime: kernel::PluginRuntimeProfile {
                    source_language: "manifest".to_owned(),
                    bridge_kind: PluginBridgeKind::HttpJson,
                    adapter_family: "http-adapter".to_owned(),
                    entrypoint_hint: "https://example.com/search".to_owned(),
                },
            }],
        }];
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
                trust_tier: kernel::PluginTrustTier::Unverified,
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
                bootstrap_hint: "register http connector adapter".to_owned(),
            }],
        }];

        let mut catalog = IntegrationCatalog::new();
        catalog.upsert_provider(ProviderConfig {
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
                    crate::spec_runtime::PLUGIN_RUNTIME_HEALTH_METADATA_KEY.to_owned(),
                    serde_json::json!({
                        "status": "quarantined",
                        "circuit_enabled": true,
                        "circuit_phase": "open",
                        "consecutive_failures": 3,
                        "half_open_remaining_calls": 0,
                        "half_open_successes": 0,
                        "last_failure_reason": "plugin connector tavily-http is circuit-open"
                    })
                    .to_string(),
                ),
            ]),
        });

        let results = execute_plugin_inventory(
            &catalog,
            &scan_reports,
            &translation_reports,
            &activation_plans,
            "TAVILY_API_KEY",
            10,
            false,
            true,
            true,
            false,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].plugin_id, "tavily-search");
        assert_eq!(results[0].manifest_api_version.as_deref(), Some("v1alpha1"));
        assert_eq!(results[0].plugin_version.as_deref(), Some("0.3.0"));
        assert_eq!(results[0].dialect, "loong_package_manifest");
        assert_eq!(results[0].compatibility_mode, "native");
        assert!(results[0].compatibility_shim.is_none());
        assert_eq!(results[0].bridge_kind, "http_json");
        assert_eq!(results[0].setup_surface.as_deref(), Some("web_search"));
        assert_eq!(
            results[0].setup_required_env_vars,
            vec!["TAVILY_API_KEY".to_owned()]
        );
        assert_eq!(
            results[0].activation_status.as_deref(),
            Some("blocked_slot_claim_conflict")
        );
        assert_eq!(
            results[0]
                .compatibility
                .as_ref()
                .and_then(|compatibility| compatibility.host_api.as_deref()),
            Some("loong-plugin/v1")
        );
        assert_eq!(
            results[0]
                .compatibility
                .as_ref()
                .and_then(|compatibility| compatibility.host_version_req.as_deref()),
            Some(">=0.1.0-alpha.1")
        );
        assert!(results[0].loaded);
        assert_eq!(
            results[0]
                .activation_attestation
                .as_ref()
                .map(|attestation| attestation.integrity.as_str()),
            Some("missing")
        );
        assert!(
            results[0]
                .activation_attestation
                .as_ref()
                .and_then(|attestation| attestation.issue.as_deref())
                .is_some_and(|issue| issue.contains("missing activation attestation"))
        );
        assert_eq!(
            results[0]
                .runtime_health
                .as_ref()
                .map(|health| health.status.as_str()),
            Some("quarantined")
        );
        assert_eq!(
            results[0]
                .runtime_health
                .as_ref()
                .map(|health| health.circuit_phase.as_str()),
            Some("open")
        );
        assert_eq!(
            results[0]
                .runtime_health
                .as_ref()
                .map(|health| health.consecutive_failures),
            Some(3)
        );
        assert!(
            results[0]
                .bootstrap_hint
                .as_deref()
                .is_some_and(|hint| hint.contains("register http"))
        );
        assert_eq!(results[0].diagnostic_findings.len(), 1);
        assert_eq!(
            results[0].diagnostic_findings[0].code,
            PluginDiagnosticCode::SlotClaimConflict
        );
        assert_eq!(
            results[0].diagnostic_findings[0].phase,
            PluginDiagnosticPhase::Activation
        );
        assert!(results[0].diagnostic_findings[0].blocking);
        assert!(results[0].input_examples.is_empty());
    }

    #[test]
    fn execute_plugin_inventory_surfaces_canonical_shim_for_foreign_compatibility_modes() {
        let descriptor = PluginDescriptor {
            path: "/tmp/openclaw-weather/openclaw.plugin.json".to_owned(),
            source_kind: PluginSourceKind::PackageManifest,
            dialect: PluginContractDialect::OpenClawModernManifest,
            dialect_version: Some("openclaw.plugin.json".to_owned()),
            compatibility_mode: PluginCompatibilityMode::OpenClawModern,
            package_root: "/tmp/openclaw-weather".to_owned(),
            package_manifest_path: Some("/tmp/openclaw-weather/openclaw.plugin.json".to_owned()),
            language: "manifest".to_owned(),
            manifest: PluginManifest {
                api_version: Some("v1alpha1".to_owned()),
                version: Some("1.2.0".to_owned()),
                plugin_id: "openclaw-weather".to_owned(),
                provider_id: "openclaw-weather".to_owned(),
                connector_name: "weather".to_owned(),
                channel_id: Some("primary".to_owned()),
                endpoint: Some("https://example.com/weather".to_owned()),
                capabilities: BTreeSet::from([Capability::InvokeConnector]),
                trust_tier: kernel::PluginTrustTier::Unverified,
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
        };

        let results = execute_plugin_inventory(
            &IntegrationCatalog::new(),
            &[PluginScanReport {
                scanned_files: 1,
                matched_plugins: 1,
                diagnostic_findings: Vec::new(),
                descriptors: vec![descriptor],
            }],
            &[],
            &[],
            "openclaw-modern-compat",
            10,
            true,
            true,
            true,
            false,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].compatibility_mode, "openclaw_modern");
        assert_eq!(
            results[0]
                .compatibility_shim
                .as_ref()
                .map(|shim| shim.shim_id.as_str()),
            Some("openclaw-modern-compat")
        );
        assert!(results[0].compatibility_shim_support.is_none());
        assert!(
            results[0]
                .compatibility_shim_support_mismatch_reasons
                .is_empty()
        );
    }

    #[test]
    fn execute_plugin_inventory_surfaces_selected_shim_support_profile_and_mismatch_truth() {
        let descriptor = PluginDescriptor {
            path: "/tmp/openclaw-weather/openclaw.plugin.json".to_owned(),
            source_kind: PluginSourceKind::PackageManifest,
            dialect: PluginContractDialect::OpenClawModernManifest,
            dialect_version: Some("openclaw.plugin.json".to_owned()),
            compatibility_mode: PluginCompatibilityMode::OpenClawModern,
            package_root: "/tmp/openclaw-weather".to_owned(),
            package_manifest_path: Some("/tmp/openclaw-weather/openclaw.plugin.json".to_owned()),
            language: "js".to_owned(),
            manifest: PluginManifest {
                api_version: Some("v1alpha1".to_owned()),
                version: Some("1.2.0".to_owned()),
                plugin_id: "openclaw-weather".to_owned(),
                provider_id: "openclaw-weather".to_owned(),
                connector_name: "weather".to_owned(),
                channel_id: Some("primary".to_owned()),
                endpoint: Some("stdio://weather".to_owned()),
                capabilities: BTreeSet::from([Capability::InvokeConnector]),
                trust_tier: kernel::PluginTrustTier::Unverified,
                metadata: BTreeMap::from([("bridge_kind".to_owned(), "process_stdio".to_owned())]),
                summary: None,
                tags: Vec::new(),
                input_examples: Vec::new(),
                output_examples: Vec::new(),
                defer_loading: false,
                setup: None,
                slot_claims: Vec::new(),
                compatibility: None,
            },
        };
        let shim = kernel::PluginCompatibilityShim {
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
                bootstrap_hint: "enable compatibility shim profile".to_owned(),
            }],
        }];

        let results = execute_plugin_inventory(
            &IntegrationCatalog::new(),
            &[PluginScanReport {
                scanned_files: 1,
                matched_plugins: 1,
                diagnostic_findings: Vec::new(),
                descriptors: vec![descriptor],
            }],
            &[],
            &activation_plans,
            "openclaw-modern@1",
            10,
            true,
            true,
            true,
            false,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0]
                .compatibility_shim_support
                .as_ref()
                .and_then(|support| support.version.as_deref()),
            Some("openclaw-modern@1")
        );
        assert_eq!(
            results[0].compatibility_shim_support_mismatch_reasons,
            vec!["source language `javascript`".to_owned()]
        );
    }

    #[test]
    fn execute_plugin_inventory_includes_deferred_entries_without_ready_results() {
        let descriptor = PluginDescriptor {
            path: "/tmp/deferred/loong.plugin.json".to_owned(),
            source_kind: PluginSourceKind::PackageManifest,
            dialect: PluginContractDialect::LoongPackageManifest,
            dialect_version: Some("v1alpha1".to_owned()),
            compatibility_mode: PluginCompatibilityMode::Native,
            package_root: "/tmp/deferred".to_owned(),
            package_manifest_path: Some("/tmp/deferred/loong.plugin.json".to_owned()),
            language: "manifest".to_owned(),
            manifest: PluginManifest {
                api_version: Some("v1alpha1".to_owned()),
                version: Some("1.0.0".to_owned()),
                plugin_id: "deferred-search".to_owned(),
                provider_id: "deferred-search".to_owned(),
                connector_name: "deferred-search".to_owned(),
                channel_id: None,
                endpoint: Some("https://example.com/deferred".to_owned()),
                capabilities: BTreeSet::from([Capability::InvokeConnector]),
                trust_tier: kernel::PluginTrustTier::Unverified,
                metadata: BTreeMap::new(),
                summary: None,
                tags: Vec::new(),
                input_examples: Vec::new(),
                output_examples: Vec::new(),
                defer_loading: true,
                setup: None,
                slot_claims: Vec::new(),
                compatibility: None,
            },
        };

        let results = execute_plugin_inventory(
            &IntegrationCatalog::new(),
            &[PluginScanReport {
                scanned_files: 1,
                matched_plugins: 1,
                diagnostic_findings: Vec::new(),
                descriptors: vec![descriptor],
            }],
            &[],
            &[],
            "deferred",
            10,
            false,
            false,
            true,
            false,
        );

        assert_eq!(results.len(), 1);
        assert!(results[0].deferred);
        assert_eq!(results[0].plugin_id, "deferred-search");
    }

    #[test]
    fn execute_plugin_inventory_falls_back_to_available_runtime_metadata() {
        let descriptor = PluginDescriptor {
            path: "/tmp/fallback/loong.plugin.json".to_owned(),
            source_kind: PluginSourceKind::PackageManifest,
            dialect: PluginContractDialect::LoongPackageManifest,
            dialect_version: Some("v1alpha1".to_owned()),
            compatibility_mode: PluginCompatibilityMode::Native,
            package_root: "/tmp/fallback".to_owned(),
            package_manifest_path: Some("/tmp/fallback/loong.plugin.json".to_owned()),
            language: "manifest".to_owned(),
            manifest: PluginManifest {
                api_version: Some("v1alpha1".to_owned()),
                version: Some("1.0.0".to_owned()),
                plugin_id: "fallback-search".to_owned(),
                provider_id: "fallback-search".to_owned(),
                connector_name: "fallback-search".to_owned(),
                channel_id: None,
                endpoint: Some("https://example.com/fallback".to_owned()),
                capabilities: BTreeSet::from([Capability::InvokeConnector]),
                trust_tier: kernel::PluginTrustTier::Unverified,
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
        };
        let activation_plans = vec![PluginActivationPlan {
            total_plugins: 1,
            ready_plugins: 0,
            setup_incomplete_plugins: 0,
            blocked_plugins: 1,
            candidates: vec![PluginActivationCandidate {
                plugin_id: "fallback-search".to_owned(),
                source_path: "/tmp/fallback/loong.plugin.json".to_owned(),
                source_kind: PluginSourceKind::PackageManifest,
                package_root: "/tmp/fallback".to_owned(),
                package_manifest_path: Some("/tmp/fallback/loong.plugin.json".to_owned()),
                trust_tier: kernel::PluginTrustTier::Unverified,
                compatibility_mode: PluginCompatibilityMode::Native,
                compatibility_shim: None,
                compatibility_shim_support: None,
                compatibility_shim_support_mismatch_reasons: Vec::new(),
                bridge_kind: PluginBridgeKind::ProcessStdio,
                adapter_family: "python-stdio-adapter".to_owned(),
                slot_claims: Vec::new(),
                diagnostic_findings: Vec::new(),
                status: PluginActivationStatus::BlockedUnsupportedBridge,
                reason: "process stdio bridge is disabled".to_owned(),
                missing_required_env_vars: Vec::new(),
                missing_required_config_keys: Vec::new(),
                bootstrap_hint: "python -m fallback".to_owned(),
            }],
        }];

        let results = execute_plugin_inventory(
            &IntegrationCatalog::new(),
            &[PluginScanReport {
                scanned_files: 1,
                matched_plugins: 1,
                diagnostic_findings: Vec::new(),
                descriptors: vec![descriptor],
            }],
            &[],
            &activation_plans,
            "python-stdio-adapter",
            10,
            true,
            true,
            false,
            false,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].bridge_kind, "process_stdio");
        assert_eq!(
            results[0].adapter_family.as_deref(),
            Some("python-stdio-adapter")
        );
        assert_eq!(
            results[0].entrypoint_hint.as_deref(),
            Some("https://example.com/fallback")
        );
        assert_eq!(results[0].source_language.as_deref(), Some("manifest"));
    }

    #[test]
    fn execute_plugin_inventory_keeps_setup_incomplete_entries_when_blocked_filter_is_off() {
        let descriptor = PluginDescriptor {
            path: "/tmp/setup/loong.plugin.json".to_owned(),
            source_kind: PluginSourceKind::PackageManifest,
            dialect: PluginContractDialect::LoongPackageManifest,
            dialect_version: Some("v1alpha1".to_owned()),
            compatibility_mode: PluginCompatibilityMode::Native,
            package_root: "/tmp/setup".to_owned(),
            package_manifest_path: Some("/tmp/setup/loong.plugin.json".to_owned()),
            language: "manifest".to_owned(),
            manifest: PluginManifest {
                api_version: Some("v1alpha1".to_owned()),
                version: Some("1.0.0".to_owned()),
                plugin_id: "setup-search".to_owned(),
                provider_id: "setup-search".to_owned(),
                connector_name: "setup-search".to_owned(),
                channel_id: None,
                endpoint: Some("https://example.com/setup".to_owned()),
                capabilities: BTreeSet::from([Capability::InvokeConnector]),
                trust_tier: kernel::PluginTrustTier::Unverified,
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
        };
        let activation_plans = vec![PluginActivationPlan {
            total_plugins: 1,
            ready_plugins: 0,
            setup_incomplete_plugins: 1,
            blocked_plugins: 0,
            candidates: vec![PluginActivationCandidate {
                plugin_id: "setup-search".to_owned(),
                source_path: "/tmp/setup/loong.plugin.json".to_owned(),
                source_kind: PluginSourceKind::PackageManifest,
                package_root: "/tmp/setup".to_owned(),
                package_manifest_path: Some("/tmp/setup/loong.plugin.json".to_owned()),
                trust_tier: kernel::PluginTrustTier::Unverified,
                compatibility_mode: PluginCompatibilityMode::Native,
                compatibility_shim: None,
                compatibility_shim_support: None,
                compatibility_shim_support_mismatch_reasons: Vec::new(),
                bridge_kind: PluginBridgeKind::HttpJson,
                adapter_family: "http-adapter".to_owned(),
                slot_claims: Vec::new(),
                diagnostic_findings: Vec::new(),
                status: PluginActivationStatus::SetupIncomplete,
                reason: "missing SEARCH_API_KEY".to_owned(),
                missing_required_env_vars: vec!["SEARCH_API_KEY".to_owned()],
                missing_required_config_keys: Vec::new(),
                bootstrap_hint: "configure SEARCH_API_KEY".to_owned(),
            }],
        }];

        let results = execute_plugin_inventory(
            &IntegrationCatalog::new(),
            &[PluginScanReport {
                scanned_files: 1,
                matched_plugins: 1,
                diagnostic_findings: Vec::new(),
                descriptors: vec![descriptor],
            }],
            &[],
            &activation_plans,
            "setup-search",
            10,
            true,
            false,
            false,
            false,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].activation_status.as_deref(),
            Some("setup_incomplete")
        );
    }

    #[test]
    fn execute_plugin_inventory_ranks_ready_entries_before_blocked_entries() {
        let ready_descriptor = PluginDescriptor {
            path: "/tmp/ready/loong.plugin.json".to_owned(),
            source_kind: PluginSourceKind::PackageManifest,
            dialect: PluginContractDialect::LoongPackageManifest,
            dialect_version: Some("v1alpha1".to_owned()),
            compatibility_mode: PluginCompatibilityMode::Native,
            package_root: "/tmp/ready".to_owned(),
            package_manifest_path: Some("/tmp/ready/loong.plugin.json".to_owned()),
            language: "manifest".to_owned(),
            manifest: PluginManifest {
                api_version: Some("v1alpha1".to_owned()),
                version: Some("1.0.0".to_owned()),
                plugin_id: "alpha-search".to_owned(),
                provider_id: "alpha-search".to_owned(),
                connector_name: "alpha-search".to_owned(),
                channel_id: None,
                endpoint: Some("https://example.com/search".to_owned()),
                capabilities: BTreeSet::from([Capability::InvokeConnector]),
                trust_tier: kernel::PluginTrustTier::Unverified,
                metadata: BTreeMap::new(),
                summary: Some("search".to_owned()),
                tags: Vec::new(),
                input_examples: Vec::new(),
                output_examples: Vec::new(),
                defer_loading: false,
                setup: None,
                slot_claims: Vec::new(),
                compatibility: None,
            },
        };
        let blocked_descriptor = PluginDescriptor {
            path: "/tmp/blocked/loong.plugin.json".to_owned(),
            source_kind: PluginSourceKind::PackageManifest,
            dialect: PluginContractDialect::LoongPackageManifest,
            dialect_version: Some("v1alpha1".to_owned()),
            compatibility_mode: PluginCompatibilityMode::Native,
            package_root: "/tmp/blocked".to_owned(),
            package_manifest_path: Some("/tmp/blocked/loong.plugin.json".to_owned()),
            language: "manifest".to_owned(),
            manifest: PluginManifest {
                api_version: Some("v1alpha1".to_owned()),
                version: Some("1.0.0".to_owned()),
                plugin_id: "beta-search".to_owned(),
                provider_id: "beta-search".to_owned(),
                connector_name: "beta-search".to_owned(),
                channel_id: None,
                endpoint: Some("https://example.com/search".to_owned()),
                capabilities: BTreeSet::from([Capability::InvokeConnector]),
                trust_tier: kernel::PluginTrustTier::Unverified,
                metadata: BTreeMap::new(),
                summary: Some("search".to_owned()),
                tags: Vec::new(),
                input_examples: Vec::new(),
                output_examples: Vec::new(),
                defer_loading: false,
                setup: None,
                slot_claims: Vec::new(),
                compatibility: None,
            },
        };
        let activation_plans = vec![PluginActivationPlan {
            total_plugins: 2,
            ready_plugins: 1,
            setup_incomplete_plugins: 0,
            blocked_plugins: 1,
            candidates: vec![
                PluginActivationCandidate {
                    plugin_id: "alpha-search".to_owned(),
                    source_path: "/tmp/ready/loong.plugin.json".to_owned(),
                    source_kind: PluginSourceKind::PackageManifest,
                    package_root: "/tmp/ready".to_owned(),
                    package_manifest_path: Some("/tmp/ready/loong.plugin.json".to_owned()),
                    trust_tier: kernel::PluginTrustTier::Unverified,
                    compatibility_mode: PluginCompatibilityMode::Native,
                    compatibility_shim: None,
                    compatibility_shim_support: None,
                    compatibility_shim_support_mismatch_reasons: Vec::new(),
                    bridge_kind: PluginBridgeKind::HttpJson,
                    adapter_family: "http-adapter".to_owned(),
                    slot_claims: Vec::new(),
                    diagnostic_findings: Vec::new(),
                    status: PluginActivationStatus::Ready,
                    reason: "ready".to_owned(),
                    missing_required_env_vars: Vec::new(),
                    missing_required_config_keys: Vec::new(),
                    bootstrap_hint: "none".to_owned(),
                },
                PluginActivationCandidate {
                    plugin_id: "beta-search".to_owned(),
                    source_path: "/tmp/blocked/loong.plugin.json".to_owned(),
                    source_kind: PluginSourceKind::PackageManifest,
                    package_root: "/tmp/blocked".to_owned(),
                    package_manifest_path: Some("/tmp/blocked/loong.plugin.json".to_owned()),
                    trust_tier: kernel::PluginTrustTier::Unverified,
                    compatibility_mode: PluginCompatibilityMode::Native,
                    compatibility_shim: None,
                    compatibility_shim_support: None,
                    compatibility_shim_support_mismatch_reasons: Vec::new(),
                    bridge_kind: PluginBridgeKind::HttpJson,
                    adapter_family: "http-adapter".to_owned(),
                    slot_claims: Vec::new(),
                    diagnostic_findings: Vec::new(),
                    status: PluginActivationStatus::BlockedUnsupportedBridge,
                    reason: "blocked".to_owned(),
                    missing_required_env_vars: Vec::new(),
                    missing_required_config_keys: Vec::new(),
                    bootstrap_hint: "enable http".to_owned(),
                },
            ],
        }];
        let scan_reports = vec![PluginScanReport {
            scanned_files: 2,
            matched_plugins: 2,
            diagnostic_findings: Vec::new(),
            descriptors: vec![ready_descriptor, blocked_descriptor],
        }];

        let empty_query_results = execute_plugin_inventory(
            &IntegrationCatalog::new(),
            &scan_reports,
            &[],
            &activation_plans,
            "",
            10,
            true,
            true,
            false,
            false,
        );
        let search_query_results = execute_plugin_inventory(
            &IntegrationCatalog::new(),
            &scan_reports,
            &[],
            &activation_plans,
            "search",
            10,
            true,
            true,
            false,
            false,
        );

        assert_eq!(empty_query_results[0].plugin_id, "alpha-search");
        assert_eq!(search_query_results[0].plugin_id, "alpha-search");
    }

    #[test]
    fn execute_plugin_inventory_ranks_setup_incomplete_before_blocked_entries() {
        let setup_descriptor = PluginDescriptor {
            path: "/tmp/setup/loong.plugin.json".to_owned(),
            source_kind: PluginSourceKind::PackageManifest,
            dialect: PluginContractDialect::LoongPackageManifest,
            dialect_version: Some("v1alpha1".to_owned()),
            compatibility_mode: PluginCompatibilityMode::Native,
            package_root: "/tmp/setup".to_owned(),
            package_manifest_path: Some("/tmp/setup/loong.plugin.json".to_owned()),
            language: "manifest".to_owned(),
            manifest: PluginManifest {
                api_version: Some("v1alpha1".to_owned()),
                version: Some("1.0.0".to_owned()),
                plugin_id: "zeta-setup".to_owned(),
                provider_id: "zeta-setup".to_owned(),
                connector_name: "zeta-setup".to_owned(),
                channel_id: None,
                endpoint: Some("https://example.com/setup".to_owned()),
                capabilities: BTreeSet::from([Capability::InvokeConnector]),
                trust_tier: kernel::PluginTrustTier::Unverified,
                metadata: BTreeMap::new(),
                summary: Some("setup".to_owned()),
                tags: Vec::new(),
                input_examples: Vec::new(),
                output_examples: Vec::new(),
                defer_loading: false,
                setup: None,
                slot_claims: Vec::new(),
                compatibility: None,
            },
        };
        let blocked_descriptor = PluginDescriptor {
            path: "/tmp/blocked/loong.plugin.json".to_owned(),
            source_kind: PluginSourceKind::PackageManifest,
            dialect: PluginContractDialect::LoongPackageManifest,
            dialect_version: Some("v1alpha1".to_owned()),
            compatibility_mode: PluginCompatibilityMode::Native,
            package_root: "/tmp/blocked".to_owned(),
            package_manifest_path: Some("/tmp/blocked/loong.plugin.json".to_owned()),
            language: "manifest".to_owned(),
            manifest: PluginManifest {
                api_version: Some("v1alpha1".to_owned()),
                version: Some("1.0.0".to_owned()),
                plugin_id: "alpha-blocked".to_owned(),
                provider_id: "alpha-blocked".to_owned(),
                connector_name: "alpha-blocked".to_owned(),
                channel_id: None,
                endpoint: Some("https://example.com/blocked".to_owned()),
                capabilities: BTreeSet::from([Capability::InvokeConnector]),
                trust_tier: kernel::PluginTrustTier::Unverified,
                metadata: BTreeMap::new(),
                summary: Some("blocked".to_owned()),
                tags: Vec::new(),
                input_examples: Vec::new(),
                output_examples: Vec::new(),
                defer_loading: false,
                setup: None,
                slot_claims: Vec::new(),
                compatibility: None,
            },
        };
        let activation_plans = vec![PluginActivationPlan {
            total_plugins: 2,
            ready_plugins: 0,
            setup_incomplete_plugins: 1,
            blocked_plugins: 1,
            candidates: vec![
                PluginActivationCandidate {
                    plugin_id: "zeta-setup".to_owned(),
                    source_path: "/tmp/setup/loong.plugin.json".to_owned(),
                    source_kind: PluginSourceKind::PackageManifest,
                    package_root: "/tmp/setup".to_owned(),
                    package_manifest_path: Some("/tmp/setup/loong.plugin.json".to_owned()),
                    trust_tier: kernel::PluginTrustTier::Unverified,
                    compatibility_mode: PluginCompatibilityMode::Native,
                    compatibility_shim: None,
                    compatibility_shim_support: None,
                    compatibility_shim_support_mismatch_reasons: Vec::new(),
                    bridge_kind: PluginBridgeKind::HttpJson,
                    adapter_family: "http-adapter".to_owned(),
                    slot_claims: Vec::new(),
                    diagnostic_findings: Vec::new(),
                    status: PluginActivationStatus::SetupIncomplete,
                    reason: "missing SEARCH_API_KEY".to_owned(),
                    missing_required_env_vars: vec!["SEARCH_API_KEY".to_owned()],
                    missing_required_config_keys: Vec::new(),
                    bootstrap_hint: "configure SEARCH_API_KEY".to_owned(),
                },
                PluginActivationCandidate {
                    plugin_id: "alpha-blocked".to_owned(),
                    source_path: "/tmp/blocked/loong.plugin.json".to_owned(),
                    source_kind: PluginSourceKind::PackageManifest,
                    package_root: "/tmp/blocked".to_owned(),
                    package_manifest_path: Some("/tmp/blocked/loong.plugin.json".to_owned()),
                    trust_tier: kernel::PluginTrustTier::Unverified,
                    compatibility_mode: PluginCompatibilityMode::Native,
                    compatibility_shim: None,
                    compatibility_shim_support: None,
                    compatibility_shim_support_mismatch_reasons: Vec::new(),
                    bridge_kind: PluginBridgeKind::HttpJson,
                    adapter_family: "http-adapter".to_owned(),
                    slot_claims: Vec::new(),
                    diagnostic_findings: Vec::new(),
                    status: PluginActivationStatus::BlockedUnsupportedBridge,
                    reason: "blocked".to_owned(),
                    missing_required_env_vars: Vec::new(),
                    missing_required_config_keys: Vec::new(),
                    bootstrap_hint: "enable http".to_owned(),
                },
            ],
        }];
        let scan_reports = vec![PluginScanReport {
            scanned_files: 2,
            matched_plugins: 2,
            diagnostic_findings: Vec::new(),
            descriptors: vec![setup_descriptor, blocked_descriptor],
        }];

        let results = execute_plugin_inventory(
            &IntegrationCatalog::new(),
            &scan_reports,
            &[],
            &activation_plans,
            "",
            10,
            true,
            true,
            false,
            false,
        );

        assert_eq!(results[0].plugin_id, "zeta-setup");
        assert_eq!(results[1].plugin_id, "alpha-blocked");
    }
}
