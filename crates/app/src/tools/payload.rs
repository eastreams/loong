use serde_json::Value;

/// Extract a required string field from a JSON payload.
///
/// Returns a trimmed, non-empty owned string or an error naming the tool and field.
pub(super) fn required_payload_string(
    payload: &Value,
    field: &str,
    tool_name: &str,
) -> Result<String, String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{tool_name} requires payload.{field}"))
}

/// Extract an optional string field from a JSON payload.
///
/// Returns `Some(trimmed_value)` when the field exists and is a non-empty string,
/// `None` otherwise.
pub(super) fn optional_payload_string(payload: &Value, field: &str) -> Option<String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

/// Extract an optional unsigned integer field, clamped to `[1, max]`.
///
/// Returns `default` when the field is absent or not a valid `u64`.
pub(super) fn optional_payload_limit(
    payload: &Value,
    field: &str,
    default: usize,
    max: usize,
) -> usize {
    payload
        .get(field)
        .and_then(Value::as_u64)
        .map(|value| value.clamp(1, max as u64) as usize)
        .unwrap_or(default)
}

/// Extract an optional non-negative integer field.
///
/// Returns `default` when the field is absent or not a valid `u64`.
pub(super) fn optional_payload_offset(payload: &Value, field: &str, default: usize) -> usize {
    let raw_value = payload.get(field);
    let parsed_value = raw_value.and_then(Value::as_u64);
    let max_value = usize::MAX as u64;
    let bounded_value = parsed_value.map(|value| value.min(max_value));
    let offset = bounded_value.map(|value| value as usize);
    offset.unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn required_string_returns_trimmed_value() {
        let payload = json!({"name": "  hello  "});
        let result = required_payload_string(&payload, "name", "test tool");
        assert_eq!(result.unwrap(), "hello");
    }

    #[test]
    fn required_string_rejects_missing_field() {
        let payload = json!({});
        let error = required_payload_string(&payload, "name", "test tool").unwrap_err();
        assert!(error.contains("test tool"), "error: {error}");
        assert!(error.contains("payload.name"), "error: {error}");
    }

    #[test]
    fn required_string_rejects_empty_string() {
        let payload = json!({"name": "   "});
        let error = required_payload_string(&payload, "name", "test tool").unwrap_err();
        assert!(error.contains("payload.name"), "error: {error}");
    }

    #[test]
    fn required_string_rejects_non_string() {
        let payload = json!({"name": 42});
        let error = required_payload_string(&payload, "name", "test tool").unwrap_err();
        assert!(error.contains("payload.name"), "error: {error}");
    }

    #[test]
    fn optional_string_returns_trimmed_value() {
        let payload = json!({"tag": "  rust  "});
        assert_eq!(optional_payload_string(&payload, "tag").unwrap(), "rust");
    }

    #[test]
    fn optional_string_returns_none_for_missing() {
        let payload = json!({});
        assert!(optional_payload_string(&payload, "tag").is_none());
    }

    #[test]
    fn optional_string_returns_none_for_empty() {
        let payload = json!({"tag": "  "});
        assert!(optional_payload_string(&payload, "tag").is_none());
    }

    #[test]
    fn optional_limit_returns_clamped_value() {
        let payload = json!({"limit": 50});
        assert_eq!(optional_payload_limit(&payload, "limit", 10, 20), 20);
    }

    #[test]
    fn optional_limit_returns_default_for_missing() {
        let payload = json!({});
        assert_eq!(optional_payload_limit(&payload, "limit", 10, 20), 10);
    }

    #[test]
    fn optional_limit_clamps_to_minimum_one() {
        let payload = json!({"limit": 0});
        assert_eq!(optional_payload_limit(&payload, "limit", 10, 20), 1);
    }

    #[test]
    fn optional_limit_returns_value_in_normal_range() {
        let payload = json!({"limit": 5});
        assert_eq!(optional_payload_limit(&payload, "limit", 10, 20), 5);
    }

    #[test]
    fn optional_limit_returns_default_for_negative() {
        let payload = json!({"limit": -3});
        assert_eq!(optional_payload_limit(&payload, "limit", 10, 20), 10);
    }

    #[test]
    fn optional_offset_returns_default_for_missing() {
        let payload = json!({});
        assert_eq!(optional_payload_offset(&payload, "offset", 0), 0);
    }

    #[test]
    fn optional_offset_returns_value_in_normal_range() {
        let payload = json!({"offset": 7});
        assert_eq!(optional_payload_offset(&payload, "offset", 0), 7);
    }

    #[test]
    fn optional_offset_returns_default_for_non_numeric() {
        let payload = json!({"offset": "bad"});
        let offset = optional_payload_offset(&payload, "offset", 0);
        assert_eq!(offset, 0);
    }

    #[test]
    fn optional_offset_returns_default_for_negative() {
        let payload = json!({"offset": -2});
        assert_eq!(optional_payload_offset(&payload, "offset", 0), 0);
    }

    #[test]
    fn optional_string_returns_none_for_non_string() {
        let payload = json!({"tag": 42});
        assert!(optional_payload_string(&payload, "tag").is_none());
    }
}
