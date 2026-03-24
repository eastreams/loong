use loongclaw_contracts::MemoryCoreRequest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MEMORY_OP_APPEND_TURN: &str = "append_turn";
pub const MEMORY_OP_WINDOW: &str = "window";
pub const MEMORY_OP_CLEAR_SESSION: &str = "clear_session";
pub const MEMORY_OP_READ_CONTEXT: &str = "read_context";
pub const MEMORY_OP_REPLACE_TURNS: &str = "replace_turns";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowTurn {
    pub role: String,
    pub content: String,
    pub ts: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryContextKind {
    Profile,
    Summary,
    Turn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryContextEntry {
    pub kind: MemoryContextKind,
    pub role: String,
    pub content: String,
}

pub fn build_append_turn_request(session_id: &str, role: &str, content: &str) -> MemoryCoreRequest {
    MemoryCoreRequest {
        operation: MEMORY_OP_APPEND_TURN.to_owned(),
        payload: json!({
            "session_id": session_id,
            "role": role,
            "content": content,
        }),
    }
}

pub fn build_window_request(session_id: &str, limit: usize) -> MemoryCoreRequest {
    MemoryCoreRequest {
        operation: MEMORY_OP_WINDOW.to_owned(),
        payload: json!({
            "session_id": session_id,
            "limit": limit,
        }),
    }
}

pub fn build_read_context_request(session_id: &str) -> MemoryCoreRequest {
    MemoryCoreRequest {
        operation: MEMORY_OP_READ_CONTEXT.to_owned(),
        payload: json!({
            "session_id": session_id,
        }),
    }
}

pub fn build_replace_turns_request(session_id: &str, turns: &[WindowTurn]) -> MemoryCoreRequest {
    build_replace_turns_request_with_expectation(session_id, turns, None)
}

pub fn build_replace_turns_request_with_expectation(
    session_id: &str,
    turns: &[WindowTurn],
    expected_turn_count: Option<usize>,
) -> MemoryCoreRequest {
    let mut payload = serde_json::Map::from_iter([
        ("session_id".to_owned(), json!(session_id)),
        ("turns".to_owned(), json!(turns)),
    ]);
    if let Some(expected_turn_count) = expected_turn_count {
        payload.insert("expected_turn_count".to_owned(), json!(expected_turn_count));
    }

    MemoryCoreRequest {
        operation: MEMORY_OP_REPLACE_TURNS.to_owned(),
        payload: Value::Object(payload),
    }
}

pub fn decode_window_turns(payload: &Value) -> Vec<WindowTurn> {
    payload
        .get("turns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|turn| WindowTurn {
            role: turn
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("assistant")
                .to_owned(),
            content: turn
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            ts: turn.get("ts").and_then(Value::as_i64),
        })
        .collect()
}

pub fn decode_window_turn_count(payload: &Value) -> Option<usize> {
    payload
        .get("turn_count")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
}

pub fn decode_memory_context_entries(payload: &Value) -> Vec<MemoryContextEntry> {
    payload
        .get("entries")
        .cloned()
        .and_then(|entries| serde_json::from_value(entries).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_window_turns_tolerates_partial_payload_shape() {
        let payload = json!({
            "turns": [
                {"role": "user", "content": "hello", "ts": 1},
                {"role": "assistant"},
                {"content": "only-content"},
                {}
            ]
        });
        let turns = decode_window_turns(&payload);
        assert_eq!(turns.len(), 4);
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[0].content, "hello");
        assert_eq!(turns[0].ts, Some(1));
        assert_eq!(turns[1].role, "assistant");
        assert_eq!(turns[1].content, "");
        assert_eq!(turns[2].role, "assistant");
        assert_eq!(turns[2].content, "only-content");
        assert_eq!(turns[3].role, "assistant");
        assert_eq!(turns[3].content, "");
    }

    #[test]
    fn decode_window_turns_returns_empty_for_missing_turns() {
        assert!(decode_window_turns(&json!({})).is_empty());
        assert!(decode_window_turns(&json!({"turns": null})).is_empty());
        assert!(decode_window_turns(&json!({"turns": "invalid"})).is_empty());
    }

    #[test]
    fn decode_window_turn_count_returns_optional_count() {
        assert_eq!(decode_window_turn_count(&json!({"turn_count": 7})), Some(7));
        assert_eq!(decode_window_turn_count(&json!({"turn_count": null})), None);
        assert_eq!(decode_window_turn_count(&json!({})), None);
    }
}
