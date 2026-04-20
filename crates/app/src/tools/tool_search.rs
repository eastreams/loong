use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use loong_contracts::{Capability, ToolCoreOutcome, ToolCoreRequest};
use serde_json::Value;
use serde_json::json;

use super::catalog::{ToolDescriptor, ToolView};
use super::runtime_config;
use super::{
    LOONG_INTERNAL_TOOL_SEARCH_KEY, LOONG_INTERNAL_TOOL_SEARCH_VISIBLE_TOOL_IDS_KEY,
    TOOL_SEARCH_GRANTED_CAPABILITIES_FIELD, canonical_tool_name, issue_tool_lease, memory_tools,
};

#[path = "tool_search_query_support.rs"]
mod query_support;
use query_support::*;

const COARSE_FALLBACK_DISCOVERY_CONCEPTS: &[&str] =
    &["fetch", "inspect", "list", "read", "search", "status"];
const MAX_SEARCH_WHY_REASONS: usize = 4;

#[derive(Debug, Clone)]
pub(super) struct SearchableToolEntry {
    pub(super) tool_id: String,
    pub(super) canonical_name: String,
    pub(super) summary: String,
    pub(super) search_hint: String,
    pub(super) argument_hint: String,
    pub(super) required_fields: Vec<String>,
    pub(super) required_field_groups: Vec<Vec<String>>,
    pub(super) schema_preview: Value,
    pub(super) tags: Vec<String>,
    pub(super) surface_id: Option<String>,
    pub(super) usage_guidance: Option<String>,
    pub(super) requires_lease: bool,
    search_document: SearchDocument,
}

#[derive(Debug, Clone)]
pub(super) struct RankedSearchableToolEntry {
    pub(super) entry: SearchableToolEntry,
    pub(super) why: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ToolSearchRanking {
    pub(super) results: Vec<RankedSearchableToolEntry>,
    pub(super) diagnostics_reason: Option<&'static str>,
}

pub(super) fn execute_tool_search_tool_with_config(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "tool.search payload must be an object".to_owned())?;
    let query = tool_search_query_from_payload(payload).map(Cow::into_owned);
    let requested_exact_tool_id = payload
        .get("exact_tool_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let exact_tool_id = requested_exact_tool_id
        .as_deref()
        .map(canonical_tool_name)
        .map(str::to_owned);

    let limit = payload
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value.clamp(1, 8) as usize)
        .unwrap_or(5);
    let granted_capabilities = payload
        .get(TOOL_SEARCH_GRANTED_CAPABILITIES_FIELD)
        .cloned()
        .and_then(|value| serde_json::from_value::<BTreeSet<Capability>>(value).ok());
    let visible_tool_view = search_tool_view_from_payload(payload, config);
    let visible_external_skills =
        super::external_skills::model_visible_skill_identities_with_config(config);

    let exact_match_entries =
        super::runtime_tool_search_entries(config, Some(&visible_tool_view), false)
            .into_iter()
            .filter(|entry| {
                tool_search_entry_is_capability_usable(
                    entry.canonical_name.as_str(),
                    granted_capabilities.as_ref(),
                )
            })
            .collect::<Vec<_>>();
    let searchable_entries = augment_hidden_skills_surface_with_external_skill_identities(
        collapse_hidden_surface_search_entries(exact_match_entries.clone()),
        visible_external_skills.as_slice(),
    );
    let exact_match_entry = exact_tool_id.as_ref().and_then(|exact_tool_id| {
        let direct_tool_id = super::direct_tool_name_for_hidden_tool(exact_tool_id);
        let direct_tool_id = direct_tool_id.map(str::to_owned);

        searchable_entries
            .iter()
            .find(|entry| {
                let canonical_match = entry.canonical_name == *exact_tool_id;
                let tool_id_match = entry.tool_id == *exact_tool_id;
                let direct_match = direct_tool_id.as_ref().is_some_and(|direct_tool_id| {
                    entry.canonical_name == *direct_tool_id || entry.tool_id == *direct_tool_id
                });
                canonical_match || tool_id_match || direct_match
            })
            .cloned()
            .or_else(|| {
                exact_tool_id_matches_visible_external_skill(
                    exact_tool_id,
                    visible_external_skills.as_slice(),
                )
                .then(|| hidden_skills_surface_entry(&searchable_entries))
                .flatten()
            })
            .or_else(|| {
                exact_match_entries
                    .iter()
                    .find(|entry| {
                        let canonical_match = entry.canonical_name == *exact_tool_id;
                        let tool_id_match = entry.tool_id == *exact_tool_id;
                        let direct_match = direct_tool_id
                            .as_ref()
                            .is_some_and(|direct_tool_id| entry.canonical_name == *direct_tool_id);
                        canonical_match || tool_id_match || direct_match
                    })
                    .cloned()
            })
    });
    let exact_match_found = exact_match_entry.is_some();
    let mut diagnostics_reason = None;
    let results: Vec<Value> = if let Some(entry) = exact_match_entry {
        let why = Vec::new();
        let entry_json = tool_search_result_entry_json(&entry, why, payload)?;
        vec![entry_json]
    } else if let Some(query) = query.as_deref() {
        let ranking = rank_searchable_entries(searchable_entries, query, limit);
        diagnostics_reason = ranking.diagnostics_reason;

        ranking
            .results
            .into_iter()
            .map(|ranked_entry| {
                let RankedSearchableToolEntry { entry, why } = ranked_entry;

                tool_search_result_entry_json(&entry, why, payload)
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        let ranking = rank_searchable_entries(searchable_entries, "", limit);
        diagnostics_reason = ranking.diagnostics_reason;

        ranking
            .results
            .into_iter()
            .map(|ranked_entry| {
                let RankedSearchableToolEntry { entry, why } = ranked_entry;

                tool_search_result_entry_json(&entry, why, payload)
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let diagnostics = tool_search_diagnostics_json(
        requested_exact_tool_id.as_deref(),
        exact_match_found,
        query.as_deref(),
        diagnostics_reason,
    );
    let response_exact_tool_id = if exact_match_found {
        exact_tool_id
    } else {
        requested_exact_tool_id
    };

    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "core-tools",
            "tool_name": request.tool_name,
            "query": query,
            "exact_tool_id": response_exact_tool_id,
            "returned": results.len(),
            "results": results,
            "diagnostics": diagnostics,
        }),
    })
}

fn tool_search_result_entry_json(
    entry: &SearchableToolEntry,
    why: Vec<String>,
    payload: &serde_json::Map<String, Value>,
) -> Result<Value, String> {
    let mut result = serde_json::Map::from_iter([
        ("tool_id".to_owned(), json!(entry.tool_id)),
        ("summary".to_owned(), json!(entry.summary)),
        ("search_hint".to_owned(), json!(entry.search_hint)),
        ("argument_hint".to_owned(), json!(entry.argument_hint)),
        ("required_fields".to_owned(), json!(entry.required_fields)),
        (
            "required_field_groups".to_owned(),
            json!(entry.required_field_groups),
        ),
        ("schema_preview".to_owned(), json!(entry.schema_preview)),
        ("tags".to_owned(), json!(entry.tags)),
        ("why".to_owned(), json!(why)),
    ]);
    if entry.requires_lease {
        let lease = issue_tool_lease(entry.canonical_name.as_str(), payload)?;
        result.insert("lease".to_owned(), json!(lease));
    }
    if let Some(surface_id) = entry.surface_id.as_deref() {
        result.insert(
            "surface_id".to_owned(),
            Value::String(surface_id.to_owned()),
        );
    }
    if let Some(usage_guidance) = entry.usage_guidance.as_deref() {
        result.insert(
            "usage_guidance".to_owned(),
            Value::String(usage_guidance.to_owned()),
        );
    }
    Ok(Value::Object(result))
}

fn tool_search_diagnostics_json(
    requested_exact_tool_id: Option<&str>,
    exact_match_found: bool,
    query: Option<&str>,
    diagnostics_reason: Option<&str>,
) -> Value {
    if let Some(requested_exact_tool_id) = requested_exact_tool_id {
        if exact_match_found {
            return Value::Null;
        }

        return json!({
            "reason": "exact_tool_id_not_visible",
            "requested_tool_id": requested_exact_tool_id,
        });
    }

    if let Some(reason) = diagnostics_reason {
        let diagnostics_query = query.unwrap_or_default();

        return json!({
            "reason": reason,
            "query": diagnostics_query,
        });
    }

    Value::Null
}

fn augment_hidden_skills_surface_with_external_skill_identities(
    entries: Vec<SearchableToolEntry>,
    visible_external_skills: &[super::external_skills::ModelVisibleSkillIdentity],
) -> Vec<SearchableToolEntry> {
    if visible_external_skills.is_empty() {
        return entries;
    }

    let mut discovery_terms = BTreeSet::new();
    for skill in visible_external_skills {
        discovery_terms.insert(skill.skill_id.clone());
        let display_name = skill.display_name.trim();
        if !display_name.is_empty() {
            discovery_terms.insert(display_name.to_owned());
        }
    }
    if discovery_terms.is_empty() {
        return entries;
    }

    entries
        .into_iter()
        .map(|entry| augment_hidden_skills_surface_entry(entry, &discovery_terms))
        .collect()
}

fn augment_hidden_skills_surface_entry(
    entry: SearchableToolEntry,
    discovery_terms: &BTreeSet<String>,
) -> SearchableToolEntry {
    if entry.canonical_name != "skills" && entry.tool_id != "skills" {
        return entry;
    }

    let mut merged_tags = entry.tags.iter().cloned().collect::<BTreeSet<_>>();
    merged_tags.extend(discovery_terms.iter().cloned());

    searchable_entry_from_manual_definition(
        entry.canonical_name.as_str(),
        entry.summary.as_str(),
        entry.argument_hint.as_str(),
        entry.required_fields,
        entry.required_field_groups,
        merged_tags.into_iter().collect(),
    )
}

fn exact_tool_id_matches_visible_external_skill(
    exact_tool_id: &str,
    visible_external_skills: &[super::external_skills::ModelVisibleSkillIdentity],
) -> bool {
    let Some(normalized_skill_id) =
        super::external_skills::normalize_skill_lookup_id(exact_tool_id)
    else {
        return false;
    };

    visible_external_skills
        .iter()
        .any(|skill| skill.skill_id == normalized_skill_id)
}

fn hidden_skills_surface_entry(entries: &[SearchableToolEntry]) -> Option<SearchableToolEntry> {
    entries
        .iter()
        .find(|entry| entry.canonical_name == "skills" || entry.tool_id == "skills")
        .cloned()
}

fn tool_search_query_from_payload(
    payload: &serde_json::Map<String, Value>,
) -> Option<Cow<'_, str>> {
    const QUERY_KEYS: &[&str] = &["query", "input", "text", "prompt", "keyword", "keywords"];

    for key in QUERY_KEYS {
        let Some(value) = payload.get(*key) else {
            continue;
        };

        if let Some(query) = tool_search_query_from_value(value) {
            return Some(query);
        }
    }

    None
}

fn tool_search_query_from_value(value: &Value) -> Option<Cow<'_, str>> {
    let string_value = value.as_str();
    if let Some(string_value) = string_value {
        let trimmed_value = string_value.trim();
        if !trimmed_value.is_empty() {
            return Some(Cow::Borrowed(trimmed_value));
        }
    }

    let values = value.as_array()?;
    let joined_value = join_tool_search_query_values(values);
    if joined_value.is_empty() {
        return None;
    }

    Some(Cow::Owned(joined_value))
}

fn join_tool_search_query_values(values: &[Value]) -> String {
    let mut query_parts = Vec::new();

    for value in values {
        let query_part = tool_search_query_part(value);
        if query_part.is_empty() {
            continue;
        }

        query_parts.push(query_part);
    }

    query_parts.join(" ")
}

fn tool_search_query_part(value: &Value) -> String {
    let string_value = value.as_str();
    if let Some(string_value) = string_value {
        return string_value.trim().to_owned();
    }

    value.to_string()
}

pub(super) fn tool_search_entry_is_runtime_usable(
    tool_name: &str,
    config: &runtime_config::ToolRuntimeConfig,
) -> bool {
    match tool_name {
        "shell.exec" => {
            !config.shell_allow.is_empty()
                || matches!(
                    config.shell_default_mode,
                    crate::tools::shell_policy_ext::ShellPolicyDefault::Allow
                )
        }
        "bash.exec" => config.bash_exec.is_discoverable(),
        "external_skills.fetch"
        | "external_skills.install"
        | "external_skills.inspect"
        | "external_skills.invoke"
        | "external_skills.list"
        | "external_skills.remove" => config.external_skills.enabled,
        #[cfg(feature = "tool-file")]
        "memory_search" => memory_tools::memory_corpus_available(config),
        #[cfg(feature = "tool-file")]
        "memory_get" => memory_tools::workspace_memory_corpus_available(config),
        _ => true,
    }
}

pub(super) fn tool_search_entry_is_capability_usable(
    tool_name: &str,
    granted_capabilities: Option<&BTreeSet<Capability>>,
) -> bool {
    let Some(granted_capabilities) = granted_capabilities else {
        return true;
    };
    let required = super::required_capabilities_for_tool_name_and_payload(tool_name, &json!({}));
    required
        .iter()
        .all(|capability| granted_capabilities.contains(capability))
}

pub(super) fn search_tool_view_from_payload(
    payload: &serde_json::Map<String, Value>,
    config: &runtime_config::ToolRuntimeConfig,
) -> ToolView {
    let payload_value = Value::Object(payload.clone());
    let visible_tool_names = if super::trusted_internal_tool_payload_enabled() {
        super::trusted_internal_tool_context_from_payload(&payload_value)
            .and_then(|body| body.get(LOONG_INTERNAL_TOOL_SEARCH_KEY))
            .and_then(|body| body.get(LOONG_INTERNAL_TOOL_SEARCH_VISIBLE_TOOL_IDS_KEY))
            .and_then(Value::as_array)
            .map(|tool_names| {
                tool_names
                    .iter()
                    .filter_map(Value::as_str)
                    .map(canonical_tool_name)
                    .collect::<Vec<_>>()
            })
    } else {
        None
    };

    match visible_tool_names {
        Some(visible_tool_names) => ToolView::from_tool_names(visible_tool_names),
        None => super::full_runtime_tool_view_for_runtime_config(config),
    }
}

#[derive(Debug, Clone)]
struct SearchDocument {
    name: SearchSignalSet,
    summary: SearchSignalSet,
    arguments: SearchSignalSet,
    schema: SearchSignalSet,
    tags: SearchSignalSet,
    concepts: BTreeSet<String>,
    categories: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct SearchScore {
    score: u32,
    why: Vec<String>,
}

#[derive(Debug, Clone)]
struct ScoredSearchableToolEntry {
    entry: SearchableToolEntry,
    score: u32,
    why: Vec<String>,
}

#[derive(Debug, Clone)]
struct SchemaArgumentField {
    name: String,
    schema_type: String,
    required: bool,
    preferred_index: usize,
}

impl SchemaArgumentField {
    fn format(self) -> String {
        let suffix = if self.required { "" } else { "?" };
        format!("{}{}:{}", self.name, suffix, self.schema_type)
    }
}

impl SearchDocument {
    fn new(
        name_fragments: Vec<String>,
        summary_fragments: Vec<String>,
        argument_fragments: Vec<String>,
        schema_fragments: Vec<String>,
        tag_fragments: Vec<String>,
    ) -> Self {
        let name = SearchSignalSet::from_fragments(&name_fragments);
        let summary = SearchSignalSet::from_fragments(&summary_fragments);
        let arguments = SearchSignalSet::from_fragments(&argument_fragments);
        let schema = SearchSignalSet::from_fragments(&schema_fragments);
        let tags = SearchSignalSet::from_fragments(&tag_fragments);

        let mut all_fragments = Vec::new();
        all_fragments.extend(name_fragments);
        all_fragments.extend(summary_fragments);
        all_fragments.extend(argument_fragments);
        all_fragments.extend(schema_fragments);
        all_fragments.extend(tag_fragments);

        let all_signals = SearchSignalSet::from_fragments(&all_fragments);
        let (concepts, categories) = extract_concepts_and_categories(&all_signals);

        Self {
            name,
            summary,
            arguments,
            schema,
            tags,
            concepts,
            categories,
        }
    }
}

pub(super) fn searchable_entry_from_descriptor(descriptor: &ToolDescriptor) -> SearchableToolEntry {
    searchable_entry_from_descriptor_for_view(descriptor, None)
}

pub(super) fn searchable_entry_from_descriptor_for_runtime_view(
    descriptor: &ToolDescriptor,
    view: &ToolView,
) -> SearchableToolEntry {
    searchable_entry_from_descriptor_for_view(descriptor, Some(view))
}

fn searchable_entry_from_descriptor_for_view(
    descriptor: &ToolDescriptor,
    view: Option<&ToolView>,
) -> SearchableToolEntry {
    let definition = match view {
        Some(view) => super::provider_definition_for_view(descriptor, view),
        None => descriptor.provider_definition(),
    };
    let function = definition.get("function");

    let summary_value = function.and_then(|value| value.get("description"));
    let summary = summary_value
        .and_then(Value::as_str)
        .unwrap_or(descriptor.description)
        .to_owned();

    let parameters_value = function.and_then(|value| value.get("parameters"));
    let parameters = parameters_value.unwrap_or(&Value::Null);
    let tags = descriptor
        .tags()
        .iter()
        .map(|tag| (*tag).to_owned())
        .collect::<Vec<_>>();
    let search_hint = direct_search_hint_for_runtime_view(descriptor, view)
        .unwrap_or_else(|| descriptor.search_hint().to_owned());
    let surface_id = descriptor.surface_id().map(str::to_owned);
    let usage_guidance = direct_usage_guidance_for_runtime_view(descriptor, view)
        .or_else(|| descriptor.usage_guidance().map(str::to_owned));
    let requires_lease = !descriptor.is_provider_exposed();
    let tool_id = super::tool_surface::discovery_tool_name_for_tool_name(descriptor.name);

    searchable_entry_from_provider_definition(
        descriptor.name,
        descriptor.provider_name,
        descriptor.aliases,
        tool_id,
        summary,
        search_hint,
        parameters,
        descriptor.parameter_types(),
        tags,
        surface_id,
        usage_guidance,
        requires_lease,
    )
}

fn direct_search_hint_for_runtime_view(
    descriptor: &ToolDescriptor,
    view: Option<&ToolView>,
) -> Option<String> {
    let view = view?;
    if descriptor.name != "web" {
        return None;
    }

    let web_runtime_modes = super::tool_surface::direct_web_runtime_modes_for_view(view);
    let search_hint = web_runtime_modes.search_hint()?;
    Some(search_hint.to_owned())
}

fn direct_usage_guidance_for_runtime_view(
    descriptor: &ToolDescriptor,
    view: Option<&ToolView>,
) -> Option<String> {
    let view = view?;
    if !descriptor.is_direct() {
        return None;
    }

    super::tool_surface::visible_direct_tool_states_for_view(view)
        .into_iter()
        .find(|state| state.surface_id == descriptor.name)
        .map(|state| state.usage_guidance)
}

pub(super) fn searchable_entry_from_provider_definition(
    canonical_name: &str,
    provider_name: &str,
    aliases: &[&str],
    tool_id: String,
    summary: String,
    search_hint: String,
    parameters: &Value,
    preferred_parameter_order: &[(&str, &str)],
    tags: Vec<String>,
    surface_id: Option<String>,
    usage_guidance: Option<String>,
    requires_lease: bool,
) -> SearchableToolEntry {
    let required_fields = schema_required_fields(parameters);
    let required_field_groups = schema_required_field_groups(parameters);
    let required_field_groups =
        default_required_field_groups(&required_fields, required_field_groups);
    let argument_hint =
        search_argument_hint_from_provider_definition(parameters, preferred_parameter_order);
    let schema_preview = build_schema_preview(&required_fields, &required_field_groups, parameters);

    let name_fragments = build_name_fragments(canonical_name, provider_name, aliases);
    let mut summary_fragments = vec![summary.clone()];

    if search_hint.trim() != summary.trim() {
        summary_fragments.push(search_hint.clone());
    }

    let argument_fragments = build_argument_fragments(
        argument_hint.as_str(),
        &required_fields,
        &required_field_groups,
    );
    let schema_fragments = collect_schema_search_terms(parameters);
    let tag_fragments = tags.clone();
    let search_document = SearchDocument::new(
        name_fragments,
        summary_fragments,
        argument_fragments,
        schema_fragments,
        tag_fragments,
    );

    let (surface_id, usage_guidance) =
        enrich_discovery_prompt_metadata(canonical_name, surface_id, usage_guidance);

    SearchableToolEntry {
        tool_id,
        canonical_name: canonical_name.to_owned(),
        summary,
        search_hint,
        argument_hint,
        required_fields,
        required_field_groups,
        schema_preview,
        tags,
        surface_id,
        usage_guidance,
        requires_lease,
        search_document,
    }
}

fn enrich_discovery_prompt_metadata(
    canonical_name: &str,
    surface_id: Option<String>,
    usage_guidance: Option<String>,
) -> (Option<String>, Option<String>) {
    (
        surface_id.or_else(|| discovery_surface_id(canonical_name)),
        usage_guidance.or_else(|| discovery_usage_guidance(canonical_name)),
    )
}

fn discovery_surface_id(canonical_name: &str) -> Option<String> {
    super::tool_surface::tool_surface_id_for_name(canonical_name).map(str::to_owned)
}

fn discovery_usage_guidance(canonical_name: &str) -> Option<String> {
    super::tool_surface::tool_surface_usage_guidance(canonical_name).map(str::to_owned)
}

fn build_schema_preview(
    required_fields: &[String],
    required_field_groups: &[Vec<String>],
    parameters: &Value,
) -> Value {
    let properties = parameters
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut required_field_names = BTreeSet::new();

    for required_field in required_fields {
        required_field_names.insert(required_field.clone());
    }

    for group in required_field_groups {
        for field_name in group {
            required_field_names.insert(field_name.clone());
        }
    }

    let mut common_optional_fields = Vec::new();

    for field_name in properties.keys() {
        let is_required = required_field_names.contains(field_name);

        if is_required {
            continue;
        }

        common_optional_fields.push(field_name.clone());
    }

    json!({
        "required_fields": required_fields,
        "required_field_groups": required_field_groups,
        "common_optional_fields": common_optional_fields,
    })
}

pub(super) fn collapse_hidden_surface_search_entries(
    entries: Vec<SearchableToolEntry>,
) -> Vec<SearchableToolEntry> {
    let mut grouped_members = BTreeMap::<String, Vec<SearchableToolEntry>>::new();
    let mut passthrough_entries = Vec::new();

    for entry in entries {
        let Some(surface_id) = entry.surface_id.as_deref() else {
            passthrough_entries.push(entry);
            continue;
        };
        // Collapse grouped hidden lanes into one high-signal card per surface.
        // `channel` stays separate from `agent`/`skills`, but it is still one
        // addon surface instead of a long tail of individual ids.
        let collapse_surface = matches!(surface_id, "agent" | "skills" | "channel");
        if !collapse_surface {
            passthrough_entries.push(entry);
            continue;
        }

        grouped_members
            .entry(surface_id.to_owned())
            .or_default()
            .push(entry);
    }

    let mut collapsed_entries = Vec::new();
    for (surface_id, members) in grouped_members {
        let Some(summary) = super::tool_surface::hidden_surface_search_summary(surface_id.as_str())
        else {
            continue;
        };
        let Some(argument_hint) =
            super::tool_surface::hidden_surface_search_argument_hint(surface_id.as_str())
        else {
            continue;
        };
        let mut tags = BTreeSet::new();
        tags.insert(surface_id.clone());
        for member in &members {
            tags.insert(member.tool_id.clone());
            tags.insert(member.canonical_name.clone());
            for tag in &member.tags {
                tags.insert(tag.clone());
            }
        }

        let entry = searchable_entry_from_manual_definition(
            surface_id.as_str(),
            summary,
            argument_hint,
            Vec::new(),
            Vec::new(),
            tags.into_iter().collect(),
        );
        collapsed_entries.push(entry);
    }

    passthrough_entries.extend(collapsed_entries);
    passthrough_entries
}

pub(super) fn searchable_entry_from_manual_definition(
    canonical_name: &str,
    summary: &str,
    argument_hint: &str,
    required_fields: Vec<String>,
    required_field_groups: Vec<Vec<String>>,
    tags: Vec<String>,
) -> SearchableToolEntry {
    let tool_id = super::tool_surface::discovery_tool_name_for_tool_name(canonical_name);
    let mut name_fragments = vec![canonical_name.to_owned(), tool_id.clone()];
    let canonical_name_variant = identifier_phrase_variant(canonical_name);
    let variant_is_distinct = canonical_name_variant != canonical_name;
    if variant_is_distinct {
        name_fragments.push(canonical_name_variant);
    }

    let summary_text = summary.to_owned();
    let search_hint = summary.to_owned();
    let argument_hint_text = argument_hint.to_owned();
    let argument_fragments =
        build_argument_fragments(argument_hint, &required_fields, &required_field_groups);
    let schema_preview = json!({
        "required_fields": required_fields,
        "required_field_groups": required_field_groups,
        "common_optional_fields": []
    });

    let mut schema_fragments = required_fields.clone();
    for required_field_group in &required_field_groups {
        let group_fragment = required_field_group.join(" ");
        schema_fragments.push(group_fragment);
    }

    let search_document = SearchDocument::new(
        name_fragments,
        vec![summary_text.clone()],
        argument_fragments,
        schema_fragments,
        tags.clone(),
    );

    SearchableToolEntry {
        tool_id,
        canonical_name: canonical_name.to_owned(),
        summary: summary_text,
        search_hint,
        argument_hint: argument_hint_text,
        required_fields,
        required_field_groups,
        schema_preview,
        tags,
        surface_id: discovery_surface_id(canonical_name),
        usage_guidance: discovery_usage_guidance(canonical_name),
        requires_lease: true,
        search_document,
    }
}

pub(super) fn search_argument_hint_from_provider_definition(
    parameters: &Value,
    preferred_parameter_order: &[(&str, &str)],
) -> String {
    let Some(properties) = parameters.get("properties").and_then(Value::as_object) else {
        return String::new();
    };

    let required = schema_required_fields(parameters)
        .into_iter()
        .collect::<BTreeSet<_>>();

    let mut fields = Vec::new();
    for (name, schema) in properties {
        let schema_type = schema_argument_type(schema);
        let is_required = required.contains(name.as_str());
        let preferred_index = preferred_parameter_index(name.as_str(), preferred_parameter_order);
        let field = SchemaArgumentField {
            name: name.to_owned(),
            schema_type,
            required: is_required,
            preferred_index,
        };
        fields.push(field);
    }

    fields.sort_by(|left, right| {
        let left_required_rank = if left.required { 0usize } else { 1usize };
        let right_required_rank = if right.required { 0usize } else { 1usize };

        left_required_rank
            .cmp(&right_required_rank)
            .then_with(|| left.preferred_index.cmp(&right.preferred_index))
            .then_with(|| left.name.cmp(&right.name))
    });

    let total_field_count = fields.len();
    let compact_fields = compact_argument_hint_fields(fields);
    let omitted_field_count = total_field_count.saturating_sub(compact_fields.len());
    let mut fragments = compact_fields
        .into_iter()
        .map(|field| field.format())
        .collect::<Vec<_>>();

    if omitted_field_count > 0 {
        fragments.push(format!("+{omitted_field_count} more"));
    }

    fragments.join(",")
}

pub(super) fn schema_required_fields(parameters: &Value) -> Vec<String> {
    parameters
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn schema_required_field_groups(parameters: &Value) -> Vec<Vec<String>> {
    let root_required_fields = schema_required_fields(parameters);
    let mut groups = Vec::new();

    for key in ["anyOf", "oneOf"] {
        let Some(options) = parameters.get(key).and_then(Value::as_array) else {
            continue;
        };

        for schema in options {
            let branch_required_fields = schema_required_fields(schema);
            let merged_required_fields = merge_required_field_group(
                root_required_fields.as_slice(),
                branch_required_fields.as_slice(),
            );
            let duplicate_group = groups.iter().any(|group| group == &merged_required_fields);

            if duplicate_group {
                continue;
            }

            groups.push(merged_required_fields);
        }
    }

    groups
}

fn merge_required_field_group(
    root_required_fields: &[String],
    branch_required_fields: &[String],
) -> Vec<String> {
    let mut merged_required_fields = root_required_fields.to_vec();

    for field_name in branch_required_fields {
        let already_present = merged_required_fields
            .iter()
            .any(|existing_name| existing_name == field_name);

        if already_present {
            continue;
        }

        merged_required_fields.push(field_name.clone());
    }

    merged_required_fields
}

pub(super) fn default_required_field_groups(
    required_fields: &[String],
    mut required_field_groups: Vec<Vec<String>>,
) -> Vec<Vec<String>> {
    let missing_groups = required_field_groups.is_empty();
    let has_required_fields = !required_fields.is_empty();

    if missing_groups && has_required_fields {
        required_field_groups.push(required_fields.to_vec());
    }

    required_field_groups
}

pub(super) fn rank_searchable_entries(
    entries: Vec<SearchableToolEntry>,
    query: &str,
    limit: usize,
) -> ToolSearchRanking {
    if entries.is_empty() {
        return ToolSearchRanking {
            results: Vec::new(),
            diagnostics_reason: Some("no_visible_tools"),
        };
    }

    let search_query = SearchQuery::new(query);
    let mut ranked = Vec::new();

    for entry in &entries {
        let score = score_entry(entry, &search_query);
        let Some(score) = score else {
            continue;
        };

        let ranked_entry = ScoredSearchableToolEntry {
            entry: entry.clone(),
            score: score.score,
            why: score.why,
        };
        ranked.push(ranked_entry);
    }

    sort_scored_entries(&mut ranked);

    if !ranked.is_empty() {
        let results = ranked
            .into_iter()
            .take(limit)
            .map(|entry| RankedSearchableToolEntry {
                entry: entry.entry,
                why: entry.why,
            })
            .collect();

        return ToolSearchRanking {
            results,
            diagnostics_reason: None,
        };
    }

    coarse_fallback(entries, limit)
}

fn score_entry(entry: &SearchableToolEntry, query: &SearchQuery) -> Option<SearchScore> {
    let mut score = 0u32;
    let mut why = BTreeSet::new();

    let normalized_query = query.signal.normalized_text.as_str();
    let query_tokens = &query.signal.tokens;

    let _name_phrase_hit = add_phrase_score(
        "name",
        64,
        &entry.search_document.name,
        normalized_query,
        &mut score,
        &mut why,
    );

    let _summary_phrase_hit = add_phrase_score(
        "summary",
        42,
        &entry.search_document.summary,
        normalized_query,
        &mut score,
        &mut why,
    );

    let _argument_phrase_hit = add_phrase_score(
        "argument",
        30,
        &entry.search_document.arguments,
        normalized_query,
        &mut score,
        &mut why,
    );

    let _schema_phrase_hit = add_phrase_score(
        "schema",
        28,
        &entry.search_document.schema,
        normalized_query,
        &mut score,
        &mut why,
    );

    let _tag_phrase_hit = add_phrase_score(
        "tag",
        24,
        &entry.search_document.tags,
        normalized_query,
        &mut score,
        &mut why,
    );

    let _name_token_hit = add_token_scores(
        "name",
        20,
        &entry.search_document.name,
        query_tokens,
        &mut score,
        &mut why,
    );

    let _summary_token_hit = add_token_scores(
        "summary",
        12,
        &entry.search_document.summary,
        query_tokens,
        &mut score,
        &mut why,
    );

    let _argument_token_hit = add_token_scores(
        "argument",
        10,
        &entry.search_document.arguments,
        query_tokens,
        &mut score,
        &mut why,
    );

    let _schema_token_hit = add_token_scores(
        "schema",
        9,
        &entry.search_document.schema,
        query_tokens,
        &mut score,
        &mut why,
    );

    let _tag_token_hit = add_token_scores(
        "tag",
        14,
        &entry.search_document.tags,
        query_tokens,
        &mut score,
        &mut why,
    );

    let concept_overlap = ordered_overlap(&query.concepts, &entry.search_document.concepts);
    for concept in concept_overlap {
        score += 26;
        why.insert(format!("concept:{concept}"));
    }

    let category_overlap = ordered_overlap(&query.categories, &entry.search_document.categories);
    for category in category_overlap {
        score += 12;
        why.insert(format!("category:{category}"));
    }

    if score == 0 {
        return None;
    }

    let mut why = why.into_iter().collect::<Vec<_>>();
    why.truncate(MAX_SEARCH_WHY_REASONS);

    Some(SearchScore { score, why })
}

fn add_phrase_score(
    label: &str,
    weight: u32,
    signal: &SearchSignalSet,
    normalized_query: &str,
    score: &mut u32,
    why: &mut BTreeSet<String>,
) -> bool {
    let phrase_allowed = phrase_search_allowed(normalized_query);
    if !phrase_allowed {
        return false;
    }

    let contains_query = signal.normalized_text.contains(normalized_query);
    if !contains_query {
        return false;
    }

    *score += weight;
    why.insert(format!("{label}_phrase"));
    true
}

fn add_token_scores(
    label: &str,
    weight: u32,
    signal: &SearchSignalSet,
    query_tokens: &BTreeSet<String>,
    score: &mut u32,
    why: &mut BTreeSet<String>,
) -> bool {
    let overlaps = ordered_overlap(query_tokens, &signal.tokens);
    if overlaps.is_empty() {
        return false;
    }

    for token in overlaps {
        *score += weight;
        why.insert(format!("{label}:{token}"));
    }

    true
}

fn phrase_search_allowed(normalized_query: &str) -> bool {
    if normalized_query.is_empty() {
        return false;
    }

    let character_count = normalized_query.chars().count();
    if normalized_query.is_ascii() {
        return character_count >= 2;
    }

    character_count >= 1
}

fn coarse_fallback(entries: Vec<SearchableToolEntry>, limit: usize) -> ToolSearchRanking {
    let mut ranked = Vec::new();

    for entry in entries {
        let (score, why) = coarse_fallback_score(&entry);
        let ranked_entry = ScoredSearchableToolEntry { entry, score, why };
        ranked.push(ranked_entry);
    }

    sort_scored_entries(&mut ranked);

    let results = ranked
        .into_iter()
        .take(limit)
        .map(|entry| RankedSearchableToolEntry {
            entry: entry.entry,
            why: entry.why,
        })
        .collect();

    ToolSearchRanking {
        results,
        diagnostics_reason: Some("coarse_fallback"),
    }
}

fn coarse_fallback_score(entry: &SearchableToolEntry) -> (u32, Vec<String>) {
    let mut score = 1u32;
    let mut why = BTreeSet::new();

    why.insert("coarse_fallback".to_owned());

    let mut discovery_bonus = 0u32;
    for concept in COARSE_FALLBACK_DISCOVERY_CONCEPTS {
        let contains_concept = entry.search_document.concepts.contains(*concept);
        if !contains_concept {
            continue;
        }

        discovery_bonus += 1;
    }

    if discovery_bonus > 0 {
        let discovery_score = 40u32 + discovery_bonus * 6u32;
        score += discovery_score;
        why.insert("coarse_discovery_tool".to_owned());
    }

    let category_score = entry.search_document.categories.len() as u32;
    score += category_score;

    let concept_score = entry.search_document.concepts.len() as u32;
    score += concept_score;

    let why = why.into_iter().collect::<Vec<_>>();

    (score, why)
}

fn sort_scored_entries(entries: &mut [ScoredSearchableToolEntry]) {
    entries.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.entry.canonical_name.cmp(&right.entry.canonical_name))
    });
}

fn build_name_fragments(
    canonical_name: &str,
    provider_name: &str,
    aliases: &[&str],
) -> Vec<String> {
    let canonical_name_fragment = canonical_name.to_owned();
    let canonical_name_variant = identifier_phrase_variant(canonical_name);
    let provider_name_fragment = provider_name.to_owned();
    let provider_name_variant = identifier_phrase_variant(provider_name);
    let mut fragments = Vec::from([
        canonical_name_fragment,
        canonical_name_variant,
        provider_name_fragment,
        provider_name_variant,
    ]);

    for alias in aliases {
        fragments.push((*alias).to_owned());
        fragments.push(identifier_phrase_variant(alias));
    }

    fragments
}

fn build_argument_fragments(
    argument_hint: &str,
    required_fields: &[String],
    required_field_groups: &[Vec<String>],
) -> Vec<String> {
    let mut fragments = Vec::new();

    if !argument_hint.is_empty() {
        fragments.push(argument_hint.to_owned());
    }

    if !required_fields.is_empty() {
        let required_joined = required_fields.join(" ");
        fragments.push(required_joined);
    }

    for group in required_field_groups {
        let group_joined = group.join(" ");
        fragments.push(group_joined);
    }

    fragments
}

fn collect_schema_search_terms(schema: &Value) -> Vec<String> {
    let mut fragments = Vec::new();
    collect_schema_search_terms_into(schema, &mut fragments);
    fragments
}

fn collect_schema_search_terms_into(schema: &Value, fragments: &mut Vec<String>) {
    let Value::Object(map) = schema else {
        return;
    };

    for key in ["title", "description"] {
        let value = map.get(key);
        let Some(text) = value.and_then(Value::as_str) else {
            continue;
        };

        fragments.push(text.to_owned());
    }

    let property_names = map.get("properties").and_then(Value::as_object);
    if let Some(property_names) = property_names {
        for (name, property_schema) in property_names {
            fragments.push(name.to_owned());
            collect_schema_search_terms_into(property_schema, fragments);
        }
    }

    for key in [
        "items",
        "additionalProperties",
        "contains",
        "if",
        "then",
        "else",
        "not",
    ] {
        let nested_schema = map.get(key);
        let Some(nested_schema) = nested_schema else {
            continue;
        };

        collect_schema_search_terms_into(nested_schema, fragments);
    }

    for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        let nested_schemas = map.get(key).and_then(Value::as_array);
        let Some(nested_schemas) = nested_schemas else {
            continue;
        };

        for nested_schema in nested_schemas {
            collect_schema_search_terms_into(nested_schema, fragments);
        }
    }

    let enum_values = map.get("enum").and_then(Value::as_array);
    if let Some(enum_values) = enum_values {
        for enum_value in enum_values {
            let Some(text) = enum_value.as_str() else {
                continue;
            };

            fragments.push(text.to_owned());
        }
    }

    let example_values = map.get("examples").and_then(Value::as_array);
    if let Some(example_values) = example_values {
        for example_value in example_values {
            let Some(text) = example_value.as_str() else {
                continue;
            };

            fragments.push(text.to_owned());
        }
    }

    let const_value = map.get("const").and_then(Value::as_str);
    if let Some(const_value) = const_value {
        fragments.push(const_value.to_owned());
    }
}

fn schema_argument_type(schema: &Value) -> String {
    let schema_type = schema
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("value");

    if schema_type != "array" {
        return schema_type.to_owned();
    }

    let item_type = schema
        .get("items")
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str);

    let Some(item_type) = item_type else {
        return "array".to_owned();
    };

    format!("{item_type}[]")
}

fn preferred_parameter_index(
    parameter_name: &str,
    preferred_parameter_order: &[(&str, &str)],
) -> usize {
    for (index, (preferred_name, _)) in preferred_parameter_order.iter().enumerate() {
        if *preferred_name == parameter_name {
            return index;
        }
    }

    usize::MAX
}

fn compact_argument_hint_fields(fields: Vec<SchemaArgumentField>) -> Vec<SchemaArgumentField> {
    if fields.len() <= 4 {
        return fields;
    }

    let mut compacted = Vec::new();
    let mut required_fields = 0usize;
    let mut optional_fields = 0usize;

    for field in fields {
        if field.required {
            if required_fields >= 2 {
                continue;
            }

            required_fields += 1;
            compacted.push(field);
            continue;
        }

        if optional_fields >= 1 {
            continue;
        }

        optional_fields += 1;
        compacted.push(field);
    }

    if compacted.is_empty() {
        return Vec::new();
    }

    compacted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_search_text_keeps_ascii_queries_stable() {
        let normalized = normalize_search_text("Find README.md");
        assert_eq!(normalized, "find readme.md");
    }

    #[test]
    fn english_concepts_extract_from_prompt_style_queries() {
        let fragments = vec!["install skill".to_owned()];
        let signal = SearchSignalSet::from_fragments(&fragments);
        let (concepts, categories) = extract_concepts_and_categories(&signal);

        assert!(concepts.contains("install"));
        assert!(concepts.contains("skill"));
        assert!(categories.contains("extension"));
        assert!(categories.contains("mutation"));
    }

    #[test]
    fn structural_query_hints_detect_file_references() {
        let query = SearchQuery::new("read note.md");
        assert!(query.concepts.contains("file"));
        assert!(query.categories.contains("workspace"));
    }

    #[test]
    fn schema_required_field_groups_merge_root_and_branch_requirements() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["url"],
            "properties": {
                "url": {"type": "string"},
                "content": {"type": "string"},
                "content_path": {"type": "string"}
            },
            "anyOf": [
                {"required": ["content"]},
                {}
            ]
        });
        let required_field_groups = schema_required_field_groups(&schema);

        assert_eq!(
            required_field_groups,
            vec![
                vec!["url".to_owned(), "content".to_owned()],
                vec!["url".to_owned()],
            ]
        );
    }

    #[test]
    fn structural_query_hints_do_not_treat_lone_domains_as_files() {
        let query = SearchQuery::new("example.com");

        assert!(!query.concepts.contains("file"));
        assert!(!query.categories.contains("workspace"));
    }

    #[test]
    fn structural_query_hints_do_not_treat_domain_paths_as_files() {
        let query = SearchQuery::new("example.com/path");

        assert!(!query.concepts.contains("file"));
        assert!(!query.categories.contains("workspace"));
    }

    #[test]
    fn structural_query_hints_do_not_treat_version_tokens_as_files() {
        let version_query = SearchQuery::new("gpt-4.1");
        let numeric_query = SearchQuery::new("3.14");

        assert!(!version_query.concepts.contains("file"));
        assert!(!numeric_query.concepts.contains("file"));
    }

    #[test]
    fn structural_query_hints_do_not_treat_generic_tree_queries_as_directories() {
        let query = SearchQuery::new("binary tree traversal");

        assert!(!query.concepts.contains("directory"));
        assert!(!query.categories.contains("workspace"));
    }

    #[test]
    fn single_dotted_identifier_queries_keep_search_signals() {
        let query = SearchQuery::new("bash.exec");

        assert!(query.signal.contains_term("bash"));
        assert!(query.signal.contains_term("exec"));
        assert!(query.signal.normalized_text.contains("bash.exec"));
        assert!(!query.concepts.contains("file"));
    }
}
