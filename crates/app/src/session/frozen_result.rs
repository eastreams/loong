use std::time::SystemTime;

use loong_contracts::ToolCoreOutcome;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const FALLBACK_ERROR_CODE: &str = "delegate_failed";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrozenResult {
    pub content: FrozenContent,
    pub captured_at: SystemTime,
    pub byte_len: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrozenContent {
    Text(String),
    ToolResult(Value),
    Error { code: String, message: String },
}

pub(crate) fn capture_frozen_result(
    outcome: &ToolCoreOutcome,
    max_frozen_bytes: usize,
) -> FrozenResult {
    let effective_max_frozen_bytes = max_frozen_bytes.max(1);
    let frozen_capture = freeze_tool_outcome(outcome, effective_max_frozen_bytes);

    FrozenResult {
        content: frozen_capture.content,
        captured_at: SystemTime::now(),
        byte_len: frozen_capture.byte_len,
        truncated: frozen_capture.truncated,
    }
}

#[derive(Debug, Clone, PartialEq)]
struct FrozenCapture {
    content: FrozenContent,
    byte_len: usize,
    truncated: bool,
}

fn freeze_tool_outcome(outcome: &ToolCoreOutcome, max_frozen_bytes: usize) -> FrozenCapture {
    let status = outcome.status.trim();
    let payload = &outcome.payload;

    if status == "ok" {
        return freeze_success_payload(payload, max_frozen_bytes);
    }

    if status == "error" || status == "timeout" {
        return freeze_error_payload(status, payload, max_frozen_bytes);
    }

    freeze_structured_payload(payload, max_frozen_bytes)
}

fn freeze_success_payload(payload: &Value, max_frozen_bytes: usize) -> FrozenCapture {
    let final_output = payload.get("final_output");
    if let Some(final_output_value) = final_output {
        let final_output_text = final_output_value.as_str();
        if let Some(final_output_text) = final_output_text {
            return freeze_text(final_output_text, max_frozen_bytes);
        }

        return freeze_structured_payload(final_output_value, max_frozen_bytes);
    }

    freeze_structured_payload(payload, max_frozen_bytes)
}

fn freeze_error_payload(status: &str, payload: &Value, max_frozen_bytes: usize) -> FrozenCapture {
    let error_code = extract_error_code(status, payload);
    let error_message = extract_error_message(status, payload);
    let truncated_code = truncate_utf8(&error_code, max_frozen_bytes);
    let stored_code = truncated_code.text;
    let code_byte_len = stored_code.len();
    let available_message_bytes = max_frozen_bytes.saturating_sub(code_byte_len);
    let truncated_message = truncate_utf8(&error_message, available_message_bytes);
    let stored_message = truncated_message.text;
    let byte_len = code_byte_len + stored_message.len();
    let truncated = truncated_code.truncated || truncated_message.truncated;

    FrozenCapture {
        content: FrozenContent::Error {
            code: stored_code,
            message: stored_message,
        },
        byte_len,
        truncated,
    }
}

fn freeze_structured_payload(payload: &Value, max_frozen_bytes: usize) -> FrozenCapture {
    let encoded_payload = serde_json::to_string(payload).unwrap_or_else(|_| "null".to_owned());
    let encoded_payload_bytes = encoded_payload.len();
    if encoded_payload_bytes <= max_frozen_bytes {
        return FrozenCapture {
            content: FrozenContent::ToolResult(payload.clone()),
            byte_len: encoded_payload_bytes,
            truncated: false,
        };
    }

    freeze_text(&encoded_payload, max_frozen_bytes)
}

fn freeze_text(text: &str, max_frozen_bytes: usize) -> FrozenCapture {
    let truncated_text = truncate_utf8(text, max_frozen_bytes);
    let stored_text = truncated_text.text;
    let byte_len = stored_text.len();
    let truncated = truncated_text.truncated;

    FrozenCapture {
        content: FrozenContent::Text(stored_text),
        byte_len,
        truncated,
    }
}

fn extract_error_code(status: &str, payload: &Value) -> String {
    let payload_error = payload.get("error");
    let payload_error_text = payload_error.and_then(Value::as_str);
    let trimmed_payload_error = payload_error_text.map(str::trim);
    let meaningful_payload_error = trimmed_payload_error.filter(|value| !value.is_empty());
    if let Some(meaningful_payload_error) = meaningful_payload_error {
        return meaningful_payload_error.to_owned();
    }

    let trimmed_status = status.trim();
    if !trimmed_status.is_empty() {
        return trimmed_status.to_owned();
    }

    FALLBACK_ERROR_CODE.to_owned()
}

fn extract_error_message(status: &str, payload: &Value) -> String {
    let payload_message = payload.get("message");
    let payload_message_text = payload_message.and_then(Value::as_str);
    let trimmed_payload_message = payload_message_text.map(str::trim);
    let meaningful_payload_message = trimmed_payload_message.filter(|value| !value.is_empty());
    if let Some(meaningful_payload_message) = meaningful_payload_message {
        return meaningful_payload_message.to_owned();
    }

    let payload_error = payload.get("error");
    let payload_error_text = payload_error.and_then(Value::as_str);
    let trimmed_payload_error = payload_error_text.map(str::trim);
    let meaningful_payload_error = trimmed_payload_error.filter(|value| !value.is_empty());
    if let Some(meaningful_payload_error) = meaningful_payload_error {
        return meaningful_payload_error.to_owned();
    }

    let encoded_payload = serde_json::to_string(payload).unwrap_or_else(|_| "null".to_owned());
    if encoded_payload != "null" && encoded_payload != "{}" {
        return encoded_payload;
    }

    extract_error_code(status, payload)
}

struct TruncatedText {
    text: String,
    truncated: bool,
}

fn truncate_utf8(text: &str, max_bytes: usize) -> TruncatedText {
    let text_byte_len = text.len();
    if text_byte_len <= max_bytes {
        return TruncatedText {
            text: text.to_owned(),
            truncated: false,
        };
    }

    let mut end = max_bytes.min(text_byte_len);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }

    let truncated_text = text[..end].to_owned();

    TruncatedText {
        text: truncated_text,
        truncated: true,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{FrozenContent, capture_frozen_result};

    #[test]
    fn capture_frozen_result_uses_final_output_text_for_success() {
        let outcome = loong_contracts::ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "final_output": "delegate completed",
                "child_session_id": "child-session",
            }),
        };

        let frozen_result = capture_frozen_result(&outcome, 256);

        assert_eq!(
            frozen_result.content,
            FrozenContent::Text("delegate completed".to_owned())
        );
        assert_eq!(frozen_result.byte_len, "delegate completed".len());
        assert!(!frozen_result.truncated);
    }

    #[test]
    fn capture_frozen_result_truncates_utf8_text_without_splitting_codepoints() {
        let outcome = loong_contracts::ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "final_output": "你好世界",
            }),
        };

        let frozen_result = capture_frozen_result(&outcome, 5);

        assert_eq!(frozen_result.content, FrozenContent::Text("你".to_owned()));
        assert_eq!(frozen_result.byte_len, "你".len());
        assert!(frozen_result.truncated);
    }

    #[test]
    fn capture_frozen_result_uses_error_variant_for_failures() {
        let outcome = loong_contracts::ToolCoreOutcome {
            status: "error".to_owned(),
            payload: json!({
                "error": "delegate_panic",
            }),
        };

        let frozen_result = capture_frozen_result(&outcome, 256);

        assert_eq!(
            frozen_result.content,
            FrozenContent::Error {
                code: "delegate_panic".to_owned(),
                message: "delegate_panic".to_owned(),
            }
        );
        assert!(!frozen_result.truncated);
    }

    #[test]
    fn capture_frozen_result_prefers_error_message_field_when_present() {
        let outcome = loong_contracts::ToolCoreOutcome {
            status: "error".to_owned(),
            payload: json!({
                "error": "delegate_timeout",
                "message": "timed out after 30s",
            }),
        };

        let frozen_result = capture_frozen_result(&outcome, 256);

        assert_eq!(
            frozen_result.content,
            FrozenContent::Error {
                code: "delegate_timeout".to_owned(),
                message: "timed out after 30s".to_owned(),
            }
        );
        assert!(!frozen_result.truncated);
    }

    #[test]
    fn capture_frozen_result_bounds_error_code_and_message() {
        let long_error = "delegate_panic_with_a_very_long_error_code";
        let outcome = loong_contracts::ToolCoreOutcome {
            status: "error".to_owned(),
            payload: json!({
                "error": long_error,
            }),
        };

        let frozen_result = capture_frozen_result(&outcome, 8);

        assert!(frozen_result.truncated);
        assert!(frozen_result.byte_len <= 8);
        assert_eq!(
            frozen_result.content,
            FrozenContent::Error {
                code: "delegate".to_owned(),
                message: "".to_owned(),
            }
        );
    }

    #[test]
    fn capture_frozen_result_preserves_structured_payload_when_it_fits() {
        let payload = json!({
            "items": ["a", "b"],
        });
        let outcome = loong_contracts::ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: payload.clone(),
        };

        let frozen_result = capture_frozen_result(&outcome, 256);

        assert_eq!(frozen_result.content, FrozenContent::ToolResult(payload));
        assert!(!frozen_result.truncated);
    }
}
