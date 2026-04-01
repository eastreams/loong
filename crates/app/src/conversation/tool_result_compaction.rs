use serde_json::Map;
use serde_json::Value;

pub(crate) fn compact_tool_search_payload_summary_str(payload_summary: &str) -> Option<String> {
    let payload_json = serde_json::from_str::<Value>(payload_summary).ok()?;
    let compacted_summary = compact_tool_search_payload_summary(&payload_json)?;
    let compacted_summary_str = serde_json::to_string(&compacted_summary).ok()?;
    let is_smaller = compacted_summary_str.len() < payload_summary.len();

    is_smaller.then_some(compacted_summary_str)
}

pub(crate) fn compact_tool_search_payload_summary(payload: &Value) -> Option<Value> {
    let payload_object = payload.as_object()?;
    let results = payload_object.get("results")?.as_array()?;
    let mut compacted = Map::new();

    if let Some(query) = payload_object.get("query") {
        compacted.insert("query".to_owned(), query.clone());
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
