use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use loongclaw_contracts::ToolCoreOutcome;
use serde_json::{Value, json};

use super::payload::{optional_payload_string, required_payload_string};

#[cfg(test)]
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DelegateRequest {
    pub task: String,
    pub label: Option<String>,
    pub timeout_seconds: u64,
}

#[cfg(test)]
pub(crate) fn parse_delegate_request(payload: &Value) -> Result<DelegateRequest, String> {
    parse_delegate_request_with_default_timeout(payload, DEFAULT_TIMEOUT_SECONDS)
}

pub(crate) fn parse_delegate_request_with_default_timeout(
    payload: &Value,
    default_timeout_seconds: u64,
) -> Result<DelegateRequest, String> {
    let task = required_payload_string(payload, "task", "delegate tool")?;
    let label = optional_payload_string(payload, "label");
    let timeout_seconds = payload
        .get("timeout_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(default_timeout_seconds);

    Ok(DelegateRequest {
        task,
        label,
        timeout_seconds,
    })
}

pub(crate) fn next_delegate_session_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(1);

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("delegate:{now_ms:x}{counter:x}")
}

pub(crate) fn delegate_success_outcome(
    child_session_id: String,
    label: Option<String>,
    final_output: String,
    turn_count: usize,
    duration_ms: u64,
) -> ToolCoreOutcome {
    ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "child_session_id": child_session_id,
            "label": label,
            "final_output": final_output,
            "turn_count": turn_count,
            "duration_ms": duration_ms,
        }),
    }
}

pub(crate) fn delegate_async_queued_outcome(
    child_session_id: String,
    label: Option<String>,
    timeout_seconds: u64,
) -> ToolCoreOutcome {
    ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "child_session_id": child_session_id,
            "label": label,
            "mode": "async",
            "state": "queued",
            "timeout_seconds": timeout_seconds,
        }),
    }
}

pub(crate) fn delegate_timeout_outcome(
    child_session_id: String,
    label: Option<String>,
    duration_ms: u64,
) -> ToolCoreOutcome {
    ToolCoreOutcome {
        status: "timeout".to_owned(),
        payload: json!({
            "child_session_id": child_session_id,
            "label": label,
            "duration_ms": duration_ms,
            "error": "delegate_timeout",
        }),
    }
}

pub(crate) fn delegate_error_outcome(
    child_session_id: String,
    label: Option<String>,
    error: String,
    duration_ms: u64,
) -> ToolCoreOutcome {
    ToolCoreOutcome {
        status: "error".to_owned(),
        payload: json!({
            "child_session_id": child_session_id,
            "label": label,
            "duration_ms": duration_ms,
            "error": error,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_delegate_request_requires_task() {
        let error =
            parse_delegate_request(&json!({})).expect_err("missing task should be rejected");
        assert!(error.contains("payload.task"), "error: {error}");
    }

    #[test]
    fn parse_delegate_request_uses_defaults() {
        let request = parse_delegate_request(&json!({
            "task": "research"
        }))
        .expect("delegate request");
        assert_eq!(request.task, "research");
        assert_eq!(request.label, None);
        assert_eq!(request.timeout_seconds, DEFAULT_TIMEOUT_SECONDS);
    }

    #[test]
    fn delegate_session_ids_use_expected_prefix() {
        let session_id = next_delegate_session_id();
        assert!(session_id.starts_with("delegate:"));
    }
}
