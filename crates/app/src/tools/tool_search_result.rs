use loong_contracts::ToolPath;
use serde_json::{Value, json};

use super::SearchableToolEntry;
use super::issue_tool_lease;

pub(super) fn tool_search_result_entry_json(
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
        let path = ToolPath::new([entry.canonical_name.as_str()])
            .map_err(|error| format!("cannot lease invalid tool identity: {error}"))?;
        let lease = issue_tool_lease(&path, payload)?;
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

pub(super) fn tool_search_diagnostics_json(
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
