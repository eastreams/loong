use std::collections::BTreeSet;

use serde_json::Value;

use crate::config::ReasoningEffort;

use super::super::ProviderModelCatalogEntry;
use super::normalize_text;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelCandidate {
    id: String,
    display_name: Option<String>,
    description: Option<String>,
    created: Option<i64>,
    created_text: Option<String>,
    is_default: bool,
    hidden: bool,
    deprecated: bool,
    default_reasoning_effort: Option<ReasoningEffort>,
    supported_reasoning_efforts: Vec<ReasoningEffort>,
    supported_reasoning_effort_descriptions: Vec<(ReasoningEffort, String)>,
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn extract_model_ids(body: &Value) -> Vec<String> {
    sorted_model_candidates(body)
        .into_iter()
        .map(|candidate| candidate.id)
        .collect()
}

pub(super) fn extract_model_catalog_entries(body: &Value) -> Vec<ProviderModelCatalogEntry> {
    sorted_model_candidates(body)
        .into_iter()
        .map(|candidate| ProviderModelCatalogEntry {
            model: candidate.id,
            display_name: candidate.display_name,
            description: candidate.description,
            is_default: candidate.is_default,
            hidden: candidate.hidden,
            deprecated: candidate.deprecated,
            default_reasoning_effort: candidate.default_reasoning_effort,
            supported_reasoning_efforts: candidate.supported_reasoning_efforts,
            supported_reasoning_effort_descriptions: candidate
                .supported_reasoning_effort_descriptions,
        })
        .collect()
}

fn sorted_model_candidates(body: &Value) -> Vec<ModelCandidate> {
    let mut candidates = collect_model_candidates(body);
    if candidates.is_empty() {
        return Vec::new();
    }

    candidates.sort_by(|left, right| {
        left.deprecated
            .cmp(&right.deprecated)
            .then_with(|| {
                right
                    .created
                    .cmp(&left.created)
                    .then_with(|| right.created_text.cmp(&left.created_text))
            })
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|candidate| seen.insert(candidate.id.clone()))
        .collect()
}

fn collect_model_candidates(body: &Value) -> Vec<ModelCandidate> {
    let mut out = Vec::new();
    let Some(items) = model_items(body) else {
        return out;
    };

    for item in items {
        if model_is_known_non_chat_candidate(item) {
            continue;
        }
        if let Some(id) = model_id_from_value(item) {
            out.push(ModelCandidate {
                id,
                display_name: model_display_name_from_value(item),
                description: model_description_from_value(item),
                created: model_created_from_value(item),
                created_text: model_created_text_from_value(item),
                is_default: model_is_default(item),
                hidden: model_is_hidden(item),
                deprecated: model_is_deprecated(item),
                default_reasoning_effort: model_default_reasoning_effort_from_value(item),
                supported_reasoning_efforts: model_supported_reasoning_efforts_from_value(item),
                supported_reasoning_effort_descriptions:
                    model_supported_reasoning_effort_descriptions_from_value(item),
            });
        }
    }
    out
}

fn model_is_default(value: &Value) -> bool {
    value
        .get("is_default")
        .or_else(|| value.get("isDefault"))
        .or_else(|| value.get("default"))
        .and_then(Value::as_bool)
        == Some(true)
}

fn model_display_name_from_value(value: &Value) -> Option<String> {
    for key in ["display_name", "displayName", "modelName", "name"] {
        if let Some(text) = value.get(key).and_then(Value::as_str)
            && let Some(normalized) = normalize_text(text)
        {
            return Some(normalized);
        }
    }
    None
}

fn model_description_from_value(value: &Value) -> Option<String> {
    for key in ["description", "modelDescription"] {
        if let Some(text) = value.get(key).and_then(Value::as_str)
            && let Some(normalized) = normalize_text(text)
        {
            return Some(normalized);
        }
    }
    None
}

fn parse_reasoning_effort_token(raw: &str) -> Option<ReasoningEffort> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" | "off" => Some(ReasoningEffort::None),
        "minimal" => Some(ReasoningEffort::Minimal),
        "low" => Some(ReasoningEffort::Low),
        "medium" => Some(ReasoningEffort::Medium),
        "high" => Some(ReasoningEffort::High),
        "xhigh" | "x-high" | "max" => Some(ReasoningEffort::Xhigh),
        _ => None,
    }
}

fn reasoning_effort_from_value(value: &Value) -> Option<ReasoningEffort> {
    value
        .as_str()
        .and_then(parse_reasoning_effort_token)
        .or_else(|| {
            value
                .get("effort")
                .and_then(Value::as_str)
                .and_then(parse_reasoning_effort_token)
        })
        .or_else(|| {
            value
                .get("reasoning_effort")
                .and_then(Value::as_str)
                .and_then(parse_reasoning_effort_token)
        })
        .or_else(|| {
            value
                .get("reasoningEffort")
                .and_then(Value::as_str)
                .and_then(parse_reasoning_effort_token)
        })
}

fn model_default_reasoning_effort_from_value(value: &Value) -> Option<ReasoningEffort> {
    for key in [
        "default_reasoning_effort",
        "defaultReasoningEffort",
        "default_reasoning_level",
        "defaultReasoningLevel",
    ] {
        if let Some(effort) = value.get(key).and_then(reasoning_effort_from_value) {
            return Some(effort);
        }
    }
    None
}

fn model_supported_reasoning_efforts_from_value(value: &Value) -> Vec<ReasoningEffort> {
    for key in [
        "supported_reasoning_efforts",
        "supportedReasoningEfforts",
        "supported_reasoning_levels",
        "supportedReasoningLevels",
    ] {
        if let Some(items) = value.get(key).and_then(Value::as_array) {
            let mut supported = Vec::new();
            for item in items {
                if let Some(effort) = reasoning_effort_from_value(item)
                    && !supported.contains(&effort)
                {
                    supported.push(effort);
                }
            }
            if !supported.is_empty() {
                return supported;
            }
        }
    }
    Vec::new()
}

fn model_supported_reasoning_effort_descriptions_from_value(
    value: &Value,
) -> Vec<(ReasoningEffort, String)> {
    for key in [
        "supported_reasoning_efforts",
        "supportedReasoningEfforts",
        "supported_reasoning_levels",
        "supportedReasoningLevels",
    ] {
        if let Some(items) = value.get(key).and_then(Value::as_array) {
            let mut descriptions = Vec::new();
            for item in items {
                let Some(effort) = reasoning_effort_from_value(item) else {
                    continue;
                };
                let Some(description) = item
                    .get("description")
                    .and_then(Value::as_str)
                    .and_then(normalize_text)
                else {
                    continue;
                };
                if descriptions
                    .iter()
                    .any(|(candidate, _)| *candidate == effort)
                {
                    continue;
                }
                descriptions.push((effort, description));
            }
            if !descriptions.is_empty() {
                return descriptions;
            }
        }
    }
    Vec::new()
}

fn model_items(body: &Value) -> Option<&[Value]> {
    if let Some(data) = body.get("data").and_then(Value::as_array) {
        return Some(data);
    }
    if let Some(models) = body.get("modelSummaries").and_then(Value::as_array) {
        return Some(models);
    }
    if let Some(models) = body.get("models").and_then(Value::as_array) {
        return Some(models);
    }
    if let Some(models) = body
        .get("Result")
        .and_then(|value| value.get("Items"))
        .and_then(Value::as_array)
    {
        return Some(models);
    }
    if let Some(models) = body
        .get("result")
        .and_then(|value| value.get("models"))
        .and_then(Value::as_array)
    {
        return Some(models);
    }
    body.as_array().map(Vec::as_slice)
}

fn model_id_from_value(value: &Value) -> Option<String> {
    if let Some(id) = value.as_str() {
        return normalize_text(id);
    }
    if let Some(id) = value.get("id").and_then(Value::as_str) {
        return normalize_text(id);
    }
    if let Some(id) = value.get("modelId").and_then(Value::as_str) {
        return normalize_text(id);
    }
    if let Some(id) = value.get("model").and_then(Value::as_str) {
        return normalize_text(id);
    }
    if let Some(id) = value.get("name").and_then(Value::as_str) {
        return normalize_text(id);
    }
    None
}

fn model_is_known_non_chat_candidate(value: &Value) -> bool {
    if model_has_explicit_non_chat_endpoint_compatibility(value) {
        return true;
    }

    if model_has_explicit_non_chat_completion_capability(value) {
        return true;
    }

    if model_is_archived(value) {
        return true;
    }

    if model_has_explicit_non_text_output_capability(value) {
        return true;
    }

    false
}

fn model_has_explicit_non_chat_endpoint_compatibility(value: &Value) -> bool {
    let Some(array) = value
        .get("supportedEndpointTypes")
        .or_else(|| value.get("supported_endpoint_types"))
        .and_then(Value::as_array)
    else {
        return false;
    };
    let endpoints = array
        .iter()
        .filter_map(Value::as_str)
        .map(|entry| entry.to_ascii_lowercase())
        .collect::<Vec<_>>();
    !endpoints.is_empty()
        && !endpoints.iter().any(|entry| {
            matches!(
                entry.as_str(),
                "chat" | "chat_completion" | "chat-completion"
            )
        })
}

fn model_has_explicit_non_chat_completion_capability(value: &Value) -> bool {
    if value
        .get("supports_chat")
        .and_then(Value::as_bool)
        .is_some_and(|enabled| !enabled)
    {
        return true;
    }
    if value
        .get("chat_completion")
        .and_then(Value::as_bool)
        .is_some_and(|enabled| !enabled)
    {
        return true;
    }
    false
}

fn model_is_archived(value: &Value) -> bool {
    value
        .get("archived")
        .and_then(Value::as_bool)
        .or_else(|| value.get("is_archived").and_then(Value::as_bool))
        == Some(true)
}

fn model_is_hidden(value: &Value) -> bool {
    if value
        .get("hidden")
        .and_then(Value::as_bool)
        .is_some_and(|hidden| hidden)
    {
        return true;
    }
    if value
        .get("show_in_picker")
        .and_then(Value::as_bool)
        .is_some_and(|show| !show)
    {
        return true;
    }
    if value
        .get("showInPicker")
        .and_then(Value::as_bool)
        .is_some_and(|show| !show)
    {
        return true;
    }
    if let Some(visibility) = value
        .get("visibility")
        .and_then(Value::as_str)
        .map(|visibility| visibility.trim().to_ascii_lowercase())
    {
        return matches!(visibility.as_str(), "hide" | "hidden" | "none");
    }
    false
}

fn model_has_explicit_non_text_output_capability(value: &Value) -> bool {
    let Some(output_modalities) = value
        .get("output_modalities")
        .or_else(|| value.get("outputModalities"))
        .and_then(Value::as_array)
    else {
        return false;
    };

    let modalities = output_modalities
        .iter()
        .filter_map(Value::as_str)
        .map(|entry| entry.to_ascii_lowercase())
        .collect::<Vec<_>>();
    !modalities.is_empty() && !modalities.iter().any(|entry| entry == "text")
}

fn model_created_from_value(value: &Value) -> Option<i64> {
    if let Some(created) = value.get("created").and_then(Value::as_i64) {
        return Some(created);
    }
    if let Some(created) = value.get("created").and_then(Value::as_u64) {
        return i64::try_from(created).ok();
    }
    if let Some(created) = value.get("created_at").and_then(Value::as_i64) {
        return Some(created);
    }
    if let Some(created) = value.get("created_at").and_then(Value::as_u64) {
        return i64::try_from(created).ok();
    }
    None
}

fn model_created_text_from_value(value: &Value) -> Option<String> {
    for key in ["created_at", "createdAt", "release_date", "releaseDate"] {
        if let Some(text) = value.get(key).and_then(Value::as_str)
            && let Some(normalized) = normalize_text(text)
        {
            return Some(normalized);
        }
    }
    None
}

fn model_is_deprecated(value: &Value) -> bool {
    if value
        .get("deprecated")
        .and_then(Value::as_bool)
        .is_some_and(|deprecated| deprecated)
    {
        return true;
    }
    if value
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| {
            matches!(
                status.trim().to_ascii_lowercase().as_str(),
                "deprecated" | "deprecation" | "retired" | "sunset"
            )
        })
    {
        return true;
    }
    if let Some(tags) = value.get("tags").and_then(Value::as_array) {
        let normalized = tags
            .iter()
            .filter_map(Value::as_str)
            .map(|entry| entry.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        if normalized.contains("deprecated") || normalized.contains("retired") {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn extract_model_ids_prefers_newer_timestamp_when_available() {
        let body = json!({
            "data": [
                {"id": "model-v1", "created": 100},
                {"id": "model-v2", "created": 200}
            ]
        });
        let ids = extract_model_ids(&body);
        assert_eq!(ids, vec!["model-v2", "model-v1"]);
    }

    #[test]
    fn extract_model_ids_supports_models_array_and_strings() {
        let body = json!({
            "models": [
                "model-c",
                {"name": "model-b"},
                {"model": "model-a"}
            ]
        });
        let ids = extract_model_ids(&body);
        assert_eq!(ids, vec!["model-a", "model-b", "model-c"]);
    }

    #[test]
    fn extract_model_ids_supports_bedrock_model_summaries() {
        let body = json!({
            "modelSummaries": [
                {
                    "modelId": "amazon.nova-lite-v1:0",
                    "modelName": "Nova Lite",
                    "providerName": "Amazon"
                },
                {
                    "modelId": "anthropic.claude-3-7-sonnet-20250219-v1:0",
                    "modelName": "Claude 3.7 Sonnet",
                    "providerName": "Anthropic"
                }
            ]
        });
        let ids = extract_model_ids(&body);
        assert_eq!(
            ids,
            vec![
                "amazon.nova-lite-v1:0",
                "anthropic.claude-3-7-sonnet-20250219-v1:0"
            ]
        );
    }

    #[test]
    fn extract_model_ids_deduplicates_results() {
        let body = json!({
            "data": [
                {"id": "model-a", "created": 200},
                {"id": "model-a", "created": 100},
                {"id": "model-b", "created": 150}
            ]
        });
        let ids = extract_model_ids(&body);
        assert_eq!(ids, vec!["model-a", "model-b"]);
    }

    #[test]
    fn extract_model_catalog_entries_surfaces_reasoning_metadata_when_present() {
        let body = json!({
            "data": [
                {
                    "id": "gpt-5.4",
                    "default_reasoning_level": "xhigh",
                    "supported_reasoning_levels": [
                        {"effort": "low", "description": "Fast responses with lighter reasoning"},
                        {"effort": "medium", "description": "Balances speed and reasoning depth"},
                        {"effort": "xhigh", "description": "Extra high reasoning depth"}
                    ]
                }
            ]
        });

        let entries = extract_model_catalog_entries(&body);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].model, "gpt-5.4");
        assert_eq!(
            entries[0].default_reasoning_effort,
            Some(ReasoningEffort::Xhigh)
        );
        assert_eq!(
            entries[0].supported_reasoning_efforts,
            vec![
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::Xhigh
            ]
        );
        assert_eq!(
            entries[0].supported_reasoning_effort_descriptions,
            vec![
                (
                    ReasoningEffort::Low,
                    "Fast responses with lighter reasoning".to_owned()
                ),
                (
                    ReasoningEffort::Medium,
                    "Balances speed and reasoning depth".to_owned()
                ),
                (
                    ReasoningEffort::Xhigh,
                    "Extra high reasoning depth".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn extract_model_catalog_entries_supports_reasoning_effort_alias_keys() {
        let body = json!({
            "models": [
                {
                    "model": "gpt-5.5",
                    "defaultReasoningEffort": "medium",
                    "supportedReasoningEfforts": [
                        {"reasoningEffort": "low"},
                        {"reasoningEffort": "high"}
                    ]
                }
            ]
        });

        let entries = extract_model_catalog_entries(&body);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].model, "gpt-5.5");
        assert_eq!(
            entries[0].default_reasoning_effort,
            Some(ReasoningEffort::Medium)
        );
        assert_eq!(
            entries[0].supported_reasoning_efforts,
            vec![ReasoningEffort::Low, ReasoningEffort::High]
        );
    }

    #[test]
    fn extract_model_catalog_entries_surfaces_hidden_and_deprecated_flags() {
        let body = json!({
            "data": [
                {
                    "id": "hidden-model",
                    "hidden": true,
                    "default_reasoning_level": "medium"
                },
                {
                    "id": "deprecated-model",
                    "deprecated": true,
                    "supported_reasoning_levels": [{"effort": "low"}]
                }
            ]
        });

        let entries = extract_model_catalog_entries(&body);
        assert_eq!(entries.len(), 2);
        let hidden = entries
            .iter()
            .find(|entry| entry.model == "hidden-model")
            .expect("hidden entry");
        assert!(!hidden.is_default);
        assert!(hidden.hidden);
        assert!(!hidden.deprecated);
        let deprecated = entries
            .iter()
            .find(|entry| entry.model == "deprecated-model")
            .expect("deprecated entry");
        assert!(!deprecated.is_default);
        assert!(!deprecated.hidden);
        assert!(deprecated.deprecated);
    }

    #[test]
    fn extract_model_catalog_entries_surfaces_catalog_default_flag() {
        let body = json!({
            "data": [
                {
                    "id": "default-model",
                    "is_default": true,
                    "supported_reasoning_levels": [{"effort": "medium"}]
                }
            ]
        });

        let entries = extract_model_catalog_entries(&body);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_default);
    }
}
