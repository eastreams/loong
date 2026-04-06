use regex::Regex;
use serde_json::Value;

const MAX_LOGGED_JSON_KEYS: usize = 8;
const MAX_LOGGED_JSON_KEY_CHARS: usize = 48;
const MAX_ERROR_CHARS: usize = 240;
const REDACTED_VALUE: &str = "[REDACTED]";

pub(crate) fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

pub(crate) fn top_level_json_keys(value: &Value) -> Vec<String> {
    let Value::Object(map) = value else {
        return Vec::new();
    };

    let mut keys = map
        .keys()
        .take(MAX_LOGGED_JSON_KEYS)
        .map(|key| truncate_logged_json_key(key))
        .collect::<Vec<_>>();
    if map.len() > MAX_LOGGED_JSON_KEYS {
        keys.push(format!("+{}", map.len() - MAX_LOGGED_JSON_KEYS));
    }
    keys
}

fn truncate_logged_json_key(key: &str) -> String {
    let key_chars = key.chars().count();
    if key_chars <= MAX_LOGGED_JSON_KEY_CHARS {
        return key.to_owned();
    }

    let visible_chars = MAX_LOGGED_JSON_KEY_CHARS.saturating_sub(3);
    let truncated = key.chars().take(visible_chars).collect::<String>();
    format!("{truncated}...")
}

pub(crate) fn summarize_error(error: &str) -> String {
    let compact = error.split_whitespace().collect::<Vec<_>>().join(" ");
    let redacted = redact_sensitive_error_fragments(compact.as_str());
    if redacted.chars().count() <= MAX_ERROR_CHARS {
        return redacted;
    }

    let truncated = redacted
        .chars()
        .take(MAX_ERROR_CHARS.saturating_sub(3))
        .collect::<String>();
    format!("{truncated}...")
}

fn redact_sensitive_error_fragments(input: &str) -> String {
    let redacted_emails = redact_email_addresses(input);
    let redacted_bearer = redact_bearer_tokens(redacted_emails.as_str());
    let redacted_query_params = redact_signed_query_params(redacted_bearer.as_str());
    let redacted_key_value = redact_key_value_secrets(redacted_query_params.as_str());
    redact_long_hex_tokens(redacted_key_value.as_str())
}

fn redact_email_addresses(input: &str) -> String {
    let pattern = r"(?i)\b[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}\b";
    let Ok(regex) = Regex::new(pattern) else {
        return input.to_owned();
    };
    regex.replace_all(input, REDACTED_VALUE).into_owned()
}

fn redact_bearer_tokens(input: &str) -> String {
    let pattern = r"(?i)\bbearer\s+[a-z0-9._~+/=\-]+\b";
    let Ok(regex) = Regex::new(pattern) else {
        return input.to_owned();
    };
    let replacement = format!("Bearer {REDACTED_VALUE}");
    regex.replace_all(input, replacement).into_owned()
}

fn redact_signed_query_params(input: &str) -> String {
    let pattern =
        r"(?i)([?&](?:sig|signature|x-amz-signature|x-goog-signature|access_token|token)=)[^&\s]+";
    let Ok(regex) = Regex::new(pattern) else {
        return input.to_owned();
    };
    regex
        .replace_all(input, |captures: &regex::Captures| {
            let prefix = captures.get(1).map_or("", |value| value.as_str());
            format!("{prefix}{REDACTED_VALUE}")
        })
        .into_owned()
}

fn redact_key_value_secrets(input: &str) -> String {
    let pattern =
        r"(?i)\b(api[_-]?key|access[_-]?token|token|secret|password)\b(\s*[:=]\s*)([^\s,;]+)";
    let Ok(regex) = Regex::new(pattern) else {
        return input.to_owned();
    };
    regex
        .replace_all(input, |captures: &regex::Captures| {
            let key = captures.get(1).map_or("", |value| value.as_str());
            let separator = captures.get(2).map_or("", |value| value.as_str());
            format!("{key}{separator}{REDACTED_VALUE}")
        })
        .into_owned()
}

fn redact_long_hex_tokens(input: &str) -> String {
    let pattern = r"\b[a-f0-9]{32,}\b";
    let Ok(regex) = Regex::new(pattern) else {
        return input.to_owned();
    };
    regex.replace_all(input, REDACTED_VALUE).into_owned()
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, Value, json};

    use super::{json_value_kind, summarize_error, top_level_json_keys};

    #[test]
    fn json_value_kind_labels_common_shapes() {
        assert_eq!(json_value_kind(&json!(null)), "null");
        assert_eq!(json_value_kind(&json!(true)), "bool");
        assert_eq!(json_value_kind(&json!(1)), "number");
        assert_eq!(json_value_kind(&json!("hello")), "string");
        assert_eq!(json_value_kind(&json!([1, 2, 3])), "array");
        assert_eq!(json_value_kind(&json!({"command": "pwd"})), "object");
    }

    #[test]
    fn top_level_json_keys_limits_output() {
        let value = json!({
            "a": 1,
            "b": 2,
            "c": 3,
            "d": 4,
            "e": 5,
            "f": 6,
            "g": 7,
            "h": 8,
            "i": 9
        });

        assert_eq!(
            top_level_json_keys(&value),
            vec![
                "a".to_owned(),
                "b".to_owned(),
                "c".to_owned(),
                "d".to_owned(),
                "e".to_owned(),
                "f".to_owned(),
                "g".to_owned(),
                "h".to_owned(),
                "+1".to_owned()
            ]
        );
    }

    #[test]
    fn top_level_json_keys_truncates_individual_key_length() {
        let mut map = Map::new();
        let long_key = "k".repeat(80);

        map.insert(long_key, json!(1));

        let value = Value::Object(map);
        let keys = top_level_json_keys(&value);
        let first_key = keys.first().expect("first key should exist");

        assert!(first_key.chars().count() <= 48);
    }

    #[test]
    fn summarize_error_collapses_whitespace_and_truncates() {
        let repeated = "detail ".repeat(64);
        let summary = summarize_error(&format!("line one\nline two\t{repeated}"));

        assert!(!summary.contains('\n'));
        assert!(!summary.contains('\t'));
        assert!(summary.ends_with("..."));
        assert!(summary.chars().count() <= 240);
    }

    #[test]
    fn summarize_error_redacts_sensitive_fragments() {
        let error = "Bearer sk-super-secret-token user=alice@example.com url=https://example.com/callback?sig=abcdef123456 api_key=token-1234567890abcdef hash=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let summary = summarize_error(error);

        assert!(summary.contains("Bearer [REDACTED]"));
        assert!(summary.contains("user=[REDACTED]"));
        assert!(summary.contains("sig=[REDACTED]"));
        assert!(summary.contains("api_key=[REDACTED]"));
        assert!(!summary.contains("alice@example.com"));
        assert!(!summary.contains("sk-super-secret-token"));
        assert!(!summary.contains("0123456789abcdef0123456789abcdef"));
    }
}
