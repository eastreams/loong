use std::collections::BTreeSet;

use serde_json::Value;

pub(super) fn extract_message_content(body: &Value) -> Option<String> {
    let content = body
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))?;

    if let Some(text) = content.as_str() {
        return normalize_text(text);
    }

    let parts = content.as_array()?;
    let mut merged = Vec::new();
    for part in parts {
        if let Some(text) = extract_content_part_text(part) {
            merged.push(text);
        }
    }
    if merged.is_empty() {
        return None;
    }
    normalize_text(&merged.join("\n"))
}

fn extract_content_part_text(part: &Value) -> Option<String> {
    if let Some(text) = part.get("text").and_then(Value::as_str) {
        return normalize_text(text);
    }
    if let Some(text) = part
        .get("text")
        .and_then(|value| value.get("value"))
        .and_then(Value::as_str)
    {
        return normalize_text(text);
    }
    None
}

fn normalize_text(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelCandidate {
    id: String,
    created: Option<i64>,
}

pub(super) fn extract_model_ids(body: &Value) -> Vec<String> {
    let mut candidates = collect_model_candidates(body);
    if candidates.is_empty() {
        return Vec::new();
    }

    candidates.sort_by(|left, right| {
        right
            .created
            .cmp(&left.created)
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut seen = BTreeSet::new();
    let mut ids = Vec::new();
    for candidate in candidates {
        if seen.insert(candidate.id.clone()) {
            ids.push(candidate.id);
        }
    }
    ids
}

fn collect_model_candidates(body: &Value) -> Vec<ModelCandidate> {
    let mut out = Vec::new();
    let Some(items) = model_items(body) else {
        return out;
    };

    for item in items {
        if let Some(id) = model_id_from_value(item) {
            out.push(ModelCandidate {
                id,
                created: model_created_from_value(item),
            });
        }
    }
    out
}

fn model_items(body: &Value) -> Option<&[Value]> {
    if let Some(data) = body.get("data").and_then(Value::as_array) {
        return Some(data);
    }
    if let Some(models) = body.get("models").and_then(Value::as_array) {
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
    if let Some(id) = value.get("model").and_then(Value::as_str) {
        return normalize_text(id);
    }
    if let Some(id) = value.get("name").and_then(Value::as_str) {
        return normalize_text(id);
    }
    None
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn extract_message_content_supports_part_array_shape() {
        let body = json!({
            "choices": [{
                "message": {
                    "content": [
                        {"type": "text", "text": "line1"},
                        {"type": "text", "text": {"value": "line2"}}
                    ]
                }
            }]
        });
        let content = extract_message_content(&body).expect("content");
        assert_eq!(content, "line1\nline2");
    }

    #[test]
    fn extract_message_content_keeps_plain_string_shape() {
        let body = json!({
            "choices": [{
                "message": {
                    "content": "  hello world  "
                }
            }]
        });
        let content = extract_message_content(&body).expect("content");
        assert_eq!(content, "hello world");
    }

    #[test]
    fn extract_message_content_ignores_empty_parts() {
        let body = json!({
            "choices": [{
                "message": {
                    "content": [
                        {"type": "text", "text": "   "},
                        {"type": "text", "text": {"value": ""}}
                    ]
                }
            }]
        });
        assert!(extract_message_content(&body).is_none());
    }

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
}
