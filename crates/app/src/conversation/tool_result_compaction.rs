use serde_json::Map;
use serde_json::Value;

pub(crate) fn compact_tool_search_payload_summary_str(payload_summary: &str) -> Option<String> {
    let payload_json = serde_json::from_str::<Value>(payload_summary).ok()?;
    let compacted_summary = compact_tool_search_payload_summary(&payload_json)?;
    let compacted_summary_str = serde_json::to_string(&compacted_summary).ok()?;
    let is_smaller = compacted_summary_str.len() < payload_summary.len();

    is_smaller.then_some(compacted_summary_str)
}

pub(crate) fn compact_tool_result_payload_value(tool_name: &str, payload: &Value) -> Value {
    if let Some(compacted_payload) = compact_continuation_payload_summary(payload) {
        return compacted_payload;
    }

    if tool_name == "tool.search" {
        if let Some(compacted_payload) = compact_tool_search_payload_summary(payload) {
            return compacted_payload;
        }

        if let Some(compacted_payload) = compact_tool_payload_summary_carrier(payload) {
            return compacted_payload;
        }
    }

    payload.clone()
}

pub(crate) fn compact_tool_search_payload_summary(payload: &Value) -> Option<Value> {
    let payload_object = payload.as_object()?;
    let results = payload_object.get("results")?.as_array()?;
    let mut compacted = Map::new();

    if let Some(query) = payload_object.get("query") {
        compacted.insert("query".to_owned(), query.clone());
    }

    if let Some(exact_tool_id) = payload_object.get("exact_tool_id") {
        compacted.insert("exact_tool_id".to_owned(), exact_tool_id.clone());
    }

    if let Some(diagnostics) = payload_object.get("diagnostics") {
        compacted.insert("diagnostics".to_owned(), diagnostics.clone());
    }

    if let Some(returned) = payload_object.get("returned") {
        compacted.insert("returned".to_owned(), returned.clone());
    }

    compacted.insert(
        "results".to_owned(),
        Value::Array(
            results
                .iter()
                .map(compact_tool_search_payload_result)
                .collect(),
        ),
    );

    Some(Value::Object(compacted))
}

fn compact_continuation_payload_summary(payload: &Value) -> Option<Value> {
    let payload_object = payload.as_object()?;
    let continuation_object = payload_object.get("continuation")?.as_object()?;

    let mut compacted = serde_json::Map::new();
    for key in [
        "mode",
        "profile",
        "label",
        "state",
        "wait_status",
        "task_id",
    ] {
        if let Some(value) = payload_object.get(key) {
            compacted.insert(key.to_owned(), value.clone());
        }
    }

    let mut compacted_continuation = serde_json::Map::new();
    for key in [
        "state",
        "is_terminal",
        "recommended_tool",
        "recommended_payload",
    ] {
        if let Some(value) = continuation_object.get(key) {
            compacted_continuation.insert(key.to_owned(), value.clone());
        }
    }
    compacted.insert(
        "continuation".to_owned(),
        Value::Object(compacted_continuation),
    );
    Some(Value::Object(compacted))
}

fn compact_tool_payload_summary_carrier(payload: &Value) -> Option<Value> {
    let payload_object = payload.as_object()?;
    let payload_summary = payload_object.get("payload_summary")?.as_str()?;
    let compacted_summary = compact_tool_search_payload_summary_str(payload_summary)?;
    let mut compacted = payload_object.clone();
    compacted.insert(
        "payload_summary".to_owned(),
        Value::String(compacted_summary),
    );
    Some(Value::Object(compacted))
}

fn compact_tool_search_payload_result(result: &Value) -> Value {
    let Some(result_object) = result.as_object() else {
        return result.clone();
    };

    let mut compacted = Map::new();

    clone_field_if_present(result_object, &mut compacted, "tool_id");
    clone_field_if_present(result_object, &mut compacted, "summary");
    clone_field_if_present(result_object, &mut compacted, "argument_hint");
    clone_array_field_if_present(result_object, &mut compacted, "required_fields");
    clone_array_field_if_present(result_object, &mut compacted, "required_field_groups");
    clone_field_if_present(result_object, &mut compacted, "lease");

    Value::Object(compacted)
}

fn clone_field_if_present(source: &Map<String, Value>, target: &mut Map<String, Value>, key: &str) {
    if let Some(value) = source.get(key) {
        target.insert(key.to_owned(), value.clone());
    }
}

fn clone_array_field_if_present(
    source: &Map<String, Value>,
    target: &mut Map<String, Value>,
    key: &str,
) {
    let Some(value) = source.get(key) else {
        return;
    };
    let Some(values) = value.as_array() else {
        return;
    };

    target.insert(key.to_owned(), Value::Array(values.clone()));
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::compact_tool_search_payload_summary;
    use crate::conversation::tool_discovery_state::ToolDiscoveryState;

    #[test]
    fn compact_tool_search_payload_summary_keeps_runtime_usable_leases_and_advisory_metadata() {
        let payload = json!({
            "adapter": "core-tools",
            "tool_name": "tool.search",
            "query": "read note.md",
            "exact_tool_id": "file.read",
            "returned": 1,
            "results": [
                {
                    "tool_id": "file.read",
                    "summary": "Read a file.",
                    "search_hint": "Use for UTF-8 text files.",
                    "argument_hint": "path:string",
                    "required_fields": ["path"],
                    "required_field_groups": [["path"]],
                    "schema_preview": {
                        "type": "object"
                    },
                    "why": ["matched query"],
                    "lease": "lease-file"
                }
            ],
            "diagnostics": {
                "reason": "exact_tool_id_not_visible",
                "requested_tool_id": "file.read"
            }
        });

        let compacted =
            compact_tool_search_payload_summary(&payload).expect("compacted tool search payload");
        let compacted_result = compacted["results"][0]
            .as_object()
            .expect("compacted result object");
        let recovered_state = ToolDiscoveryState::from_tool_search_payload(&compacted)
            .expect("compacted payload should still recover discovery state");

        assert_eq!(compacted["query"], json!("read note.md"));
        assert_eq!(compacted["exact_tool_id"], json!("file.read"));
        assert_eq!(compacted["returned"], json!(1));
        assert_eq!(
            compacted["diagnostics"]["reason"],
            json!("exact_tool_id_not_visible")
        );
        assert_eq!(compacted_result.get("lease"), Some(&json!("lease-file")));
        assert_eq!(compacted_result.get("tool_id"), Some(&json!("file.read")));
        assert_eq!(
            compacted_result.get("summary"),
            Some(&json!("Read a file."))
        );
        assert_eq!(
            compacted_result.get("argument_hint"),
            Some(&json!("path:string"))
        );
        assert!(!compacted_result.contains_key("schema_preview"));
        assert!(!compacted_result.contains_key("why"));
        assert!(!compacted_result.contains_key("search_hint"));
        assert_eq!(recovered_state.exact_tool_id.as_deref(), Some("read"));
        assert_eq!(recovered_state.entries.len(), 1);
        assert_eq!(recovered_state.entries[0].tool_id.as_str(), "read");
        assert_eq!(
            recovered_state.entries[0].argument_hint.as_deref(),
            Some("path:string")
        );
    }
}
