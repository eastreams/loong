use std::borrow::Cow;
use std::collections::BTreeSet;

use loong_contracts::{Capability, ToolCoreOutcome, ToolCoreRequest};
use serde_json::Value;
use serde_json::json;

use super::catalog::{ToolDescriptor, ToolView};
use super::runtime_config;
use super::{
    LOONG_INTERNAL_TOOL_SEARCH_KEY, LOONG_INTERNAL_TOOL_SEARCH_VISIBLE_TOOL_IDS_KEY,
    TOOL_SEARCH_GRANTED_CAPABILITIES_FIELD, canonical_tool_name, issue_tool_lease, memory_tools,
};

#[path = "tool_search_entry.rs"]
mod entry;
#[path = "tool_search_query_support.rs"]
mod query_support;
#[path = "tool_search_rank.rs"]
mod rank;
#[path = "tool_search_result.rs"]
mod result;
#[path = "tool_search_view.rs"]
mod view;
#[cfg(test)]
use entry::schema_required_field_groups;
pub(crate) use entry::{
    SearchableToolEntry, searchable_entry_from_manual_definition,
    searchable_entry_from_provider_definition,
};
#[cfg(test)]
use query_support::*;
pub(crate) use rank::rank_searchable_entries;
use result::{tool_search_diagnostics_json, tool_search_result_entry_json};
#[cfg(test)]
pub(crate) use view::runtime_discoverable_tool_entries;
pub(crate) use view::runtime_tool_search_entries;
use view::searchable_entry_from_descriptor_for_view;
#[cfg(test)]
pub(crate) use view::tool_id_visible_in_view;

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
    let visible_requested_exact_tool_id = exact_tool_id
        .as_deref()
        .map(super::user_visible_tool_name)
        .or_else(|| {
            requested_exact_tool_id
                .as_deref()
                .map(super::user_visible_tool_name)
        });

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
    let searchable_entries = runtime_tool_search_entries(config, Some(&visible_tool_view), false)
        .into_iter()
        .filter(|entry| {
            tool_search_entry_is_capability_usable(
                entry.canonical_name.as_str(),
                granted_capabilities.as_ref(),
            )
        })
        .collect::<Vec<_>>();
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
        visible_requested_exact_tool_id.as_deref(),
        exact_match_found,
        query.as_deref(),
        diagnostics_reason,
    );
    let response_exact_tool_id = if exact_match_found {
        visible_requested_exact_tool_id.clone()
    } else {
        visible_requested_exact_tool_id
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

pub(super) fn searchable_entry_from_descriptor(descriptor: &ToolDescriptor) -> SearchableToolEntry {
    searchable_entry_from_descriptor_for_view(descriptor, None)
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
