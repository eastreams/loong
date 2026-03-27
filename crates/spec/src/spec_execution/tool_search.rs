use std::collections::{BTreeMap, BTreeSet};

use kernel::{
    IntegrationCatalog, PluginBridgeKind, PluginScanReport, PluginSetupReadinessContext,
    PluginTranslationReport, PluginTrustTier, evaluate_plugin_setup_requirements,
    plugin_provenance_summary_for_descriptor,
};
use serde_json::Value;

use super::descriptor_bridge_kind;
use crate::spec_runtime::{
    ToolSearchEntry, ToolSearchResult, ToolSearchTrustFilterSummary, detect_provider_bridge_kind,
};

#[derive(Debug)]
pub(super) struct ToolSearchExecutionReport {
    pub results: Vec<ToolSearchResult>,
    pub trust_filter_summary: ToolSearchTrustFilterSummary,
}

pub(super) fn execute_tool_search(
    integration_catalog: &IntegrationCatalog,
    plugin_scan_reports: &[PluginScanReport],
    plugin_translation_reports: &[PluginTranslationReport],
    setup_readiness_context: &PluginSetupReadinessContext,
    query: &str,
    limit: usize,
    trust_tiers: &[PluginTrustTier],
    include_deferred: bool,
    include_examples: bool,
) -> ToolSearchExecutionReport {
    let mut entries: BTreeMap<String, ToolSearchEntry> = BTreeMap::new();
    let mut translation_by_key: BTreeMap<
        (String, String),
        (PluginBridgeKind, String, String, String),
    > = BTreeMap::new();

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
        let mut adapter_family = provider.metadata.get("adapter_family").cloned();
        let mut entrypoint_hint = provider
            .metadata
            .get("entrypoint")
            .or_else(|| provider.metadata.get("entrypoint_hint"))
            .cloned();
        let mut source_language = provider.metadata.get("source_language").cloned();
        let mut resolved_bridge_kind = bridge_kind;

        if let (Some(source_path), Some(plugin_id)) = (
            provider.metadata.get("plugin_source_path"),
            provider.metadata.get("plugin_id"),
        ) && let Some((bridge, adapter, entrypoint, language)) =
            translation_by_key.get(&(source_path.clone(), plugin_id.clone()))
        {
            resolved_bridge_kind = *bridge;
            adapter_family = Some(adapter.clone());
            entrypoint_hint = Some(entrypoint.clone());
            source_language = Some(language.clone());
        }

        entries.insert(
            tool_id.clone(),
            ToolSearchEntry {
                tool_id,
                plugin_id: provider.metadata.get("plugin_id").cloned(),
                connector_name: provider.connector_name.clone(),
                provider_id: provider.provider_id.clone(),
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
                setup_ready: true,
                missing_required_env_vars: Vec::new(),
                missing_required_config_keys: Vec::new(),
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
                .map(|(bridge, _, _, _)| *bridge)
                .unwrap_or_else(|| descriptor_bridge_kind(descriptor));
            let adapter_family = translation.map(|(_, adapter, _, _)| adapter.clone());
            let entrypoint_hint = translation.map(|(_, _, entrypoint, _)| entrypoint.clone());
            let source_language = translation.map(|(_, _, _, language)| language.clone());

            let entry = entries
                .entry(tool_id.clone())
                .or_insert_with(|| ToolSearchEntry {
                    tool_id: tool_id.clone(),
                    plugin_id: Some(manifest.plugin_id.clone()),
                    connector_name: manifest.connector_name.clone(),
                    provider_id: manifest.provider_id.clone(),
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
                    setup_ready: true,
                    missing_required_env_vars: Vec::new(),
                    missing_required_config_keys: Vec::new(),
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
            connector_name: entry.connector_name,
            provider_id: entry.provider_id,
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
            setup_ready: entry.setup_ready,
            missing_required_env_vars: entry.missing_required_env_vars,
            missing_required_config_keys: entry.missing_required_config_keys,
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
    if setup_docs_urls.iter().any(|value| value.contains(query)) {
        score = score.saturating_add(8);
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

    let haystack = format!(
        "{} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}",
        connector,
        provider,
        tool_id,
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
        setup_mode,
        setup_surface,
        setup_default_env_var,
        setup_remediation,
        tags.join(" "),
        setup_required_env_vars.join(" "),
        setup_recommended_env_vars.join(" "),
        setup_required_config_keys.join(" "),
        setup_docs_urls.join(" ")
    );
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
    use kernel::{IntegrationCatalog, PluginSetupReadinessContext, ProviderConfig};
    use std::collections::BTreeMap;

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
                    "/tmp/tavily/loongclaw.plugin.json".to_owned(),
                ),
                (
                    "plugin_source_kind".to_owned(),
                    "package_manifest".to_owned(),
                ),
                ("plugin_package_root".to_owned(), "/tmp/tavily".to_owned()),
                (
                    "plugin_package_manifest_path".to_owned(),
                    "/tmp/tavily/loongclaw.plugin.json".to_owned(),
                ),
                (
                    "plugin_provenance_summary".to_owned(),
                    "package_manifest:/tmp/tavily/loongclaw.plugin.json".to_owned(),
                ),
                ("plugin_trust_tier".to_owned(), "official".to_owned()),
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
                ("bridge_kind".to_owned(), "http_json".to_owned()),
            ]),
        };
        catalog.upsert_provider(provider);

        let report = execute_tool_search(
            &catalog,
            &[],
            &[],
            &PluginSetupReadinessContext::default(),
            "TAVILY_API_KEY",
            10,
            &[],
            true,
            false,
        );

        assert_eq!(report.results.len(), 1);
        assert!(!report.trust_filter_summary.applied);
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
            Some("/tmp/tavily/loongclaw.plugin.json")
        );
        assert_eq!(
            report.results[0].provenance_summary.as_deref(),
            Some("package_manifest:/tmp/tavily/loongclaw.plugin.json")
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
            verified_env_vars: std::collections::BTreeSet::from(["TAVILY_API_KEY".to_owned()]),
            verified_config_keys: std::collections::BTreeSet::from([
                "tools.web_search.default_provider".to_owned(),
            ]),
        };

        let report = execute_tool_search(
            &catalog,
            &[],
            &[],
            &setup_readiness_context,
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
}
