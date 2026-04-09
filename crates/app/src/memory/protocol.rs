use std::path::Path;

use loongclaw_contracts::MemoryCoreRequest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::memory::runtime_config::MemoryRuntimeConfig;

use super::{
    DerivedMemoryKind, HydratedMemoryContext, MemoryContextProvenance, MemoryDiagnostics,
    MemoryRecallMode, MemoryRetrievalRequest, MemoryScope, MemoryStageFamily, StageDiagnostics,
    StageEnvelope, StageOutcome,
};

pub const MEMORY_OP_APPEND_TURN: &str = "append_turn";
pub const MEMORY_OP_WINDOW: &str = "window";
pub const MEMORY_OP_CLEAR_SESSION: &str = "clear_session";
pub const MEMORY_OP_READ_CONTEXT: &str = "read_context";
pub const MEMORY_OP_REPLACE_TURNS: &str = "replace_turns";
pub const MEMORY_OP_READ_STAGE_ENVELOPE: &str = "read_stage_envelope";

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
    Derived,
    RetrievedMemory,
    Turn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryContextEntry {
    pub kind: MemoryContextKind,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub provenance: Vec<MemoryContextProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WindowTurnPayload {
    role: String,
    content: String,
    #[serde(default)]
    ts: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MemoryDiagnosticsPayload {
    system_id: String,
    #[serde(default)]
    fail_open: bool,
    #[serde(default)]
    strict_mode_requested: bool,
    #[serde(default)]
    strict_mode_active: bool,
    #[serde(default)]
    degraded: bool,
    #[serde(default)]
    derivation_error: Option<String>,
    #[serde(default)]
    retrieval_error: Option<String>,
    #[serde(default)]
    rank_error: Option<String>,
    #[serde(default)]
    recent_window_count: usize,
    #[serde(default)]
    entry_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct HydratedMemoryContextPayload {
    #[serde(default)]
    entries: Vec<MemoryContextEntry>,
    #[serde(default)]
    recent_window: Vec<WindowTurnPayload>,
    diagnostics: Option<MemoryDiagnosticsPayload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MemoryRetrievalRequestPayload {
    session_id: String,
    #[serde(default)]
    memory_system_id: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    recall_mode: Option<MemoryRecallMode>,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    budget_items: Option<usize>,
    #[serde(default)]
    allowed_kinds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StageDiagnosticsPayload {
    family: String,
    outcome: String,
    #[serde(default)]
    budget_ms: Option<u64>,
    #[serde(default)]
    elapsed_ms: Option<u64>,
    #[serde(default)]
    fallback_activated: bool,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StageEnvelopePayload {
    hydrated: Option<HydratedMemoryContextPayload>,
    #[serde(default)]
    retrieval_request: Option<MemoryRetrievalRequestPayload>,
    #[serde(default)]
    diagnostics: Vec<StageDiagnosticsPayload>,
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

pub fn build_read_context_request(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> MemoryCoreRequest {
    MemoryCoreRequest {
        operation: MEMORY_OP_READ_CONTEXT.to_owned(),
        payload: json!({
            "session_id": session_id,
            "profile": config.profile.as_str(),
            "system": config.system.as_str(),
            "system_id": config.resolved_system_id.as_deref(),
            "sliding_window": config.sliding_window,
            "summary_max_chars": config.summary_max_chars,
            "profile_note": config.profile_note,
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

pub fn build_read_stage_envelope_request(session_id: &str) -> MemoryCoreRequest {
    MemoryCoreRequest {
        operation: MEMORY_OP_READ_STAGE_ENVELOPE.to_owned(),
        payload: json!({
            "session_id": session_id,
        }),
    }
}

pub fn build_read_stage_envelope_request_with_workspace_root(
    session_id: &str,
    workspace_root: Option<&Path>,
    config: &MemoryRuntimeConfig,
) -> MemoryCoreRequest {
    let mut payload = serde_json::Map::from_iter([("session_id".to_owned(), json!(session_id))]);

    if let Some(workspace_root) = workspace_root {
        payload.insert(
            "workspace_root".to_owned(),
            json!(workspace_root.to_string_lossy().to_string()),
        );
    }

    payload.insert("profile".to_owned(), json!(config.profile.as_str()));
    payload.insert("system".to_owned(), json!(config.system.as_str()));
    payload.insert(
        "system_id".to_owned(),
        json!(config.resolved_system_id.as_deref()),
    );
    payload.insert("sliding_window".to_owned(), json!(config.sliding_window));
    payload.insert(
        "summary_max_chars".to_owned(),
        json!(config.summary_max_chars),
    );
    payload.insert("profile_note".to_owned(), json!(config.profile_note));

    MemoryCoreRequest {
        operation: MEMORY_OP_READ_STAGE_ENVELOPE.to_owned(),
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

pub fn encode_stage_envelope_payload(envelope: &StageEnvelope) -> Value {
    serde_json::to_value(StageEnvelopePayload::from(envelope)).unwrap_or(Value::Null)
}

pub fn decode_stage_envelope(payload: &Value) -> Option<StageEnvelope> {
    let payload = serde_json::from_value::<StageEnvelopePayload>(payload.clone()).ok()?;

    Some(StageEnvelope {
        hydrated: decode_hydrated_memory_context_payload(payload.hydrated?)?,
        retrieval_request: payload
            .retrieval_request
            .and_then(decode_memory_retrieval_request_payload),
        diagnostics: payload
            .diagnostics
            .into_iter()
            .filter_map(decode_stage_diagnostics_payload)
            .collect(),
    })
}

impl From<&WindowTurn> for WindowTurnPayload {
    fn from(value: &WindowTurn) -> Self {
        Self {
            role: value.role.clone(),
            content: value.content.clone(),
            ts: value.ts,
        }
    }
}

impl From<&MemoryDiagnostics> for MemoryDiagnosticsPayload {
    fn from(value: &MemoryDiagnostics) -> Self {
        Self {
            system_id: value.system_id.to_owned(),
            fail_open: value.fail_open,
            strict_mode_requested: value.strict_mode_requested,
            strict_mode_active: value.strict_mode_active,
            degraded: value.degraded,
            derivation_error: value.derivation_error.clone(),
            retrieval_error: value.retrieval_error.clone(),
            rank_error: value.rank_error.clone(),
            recent_window_count: value.recent_window_count,
            entry_count: value.entry_count,
        }
    }
}

impl From<&HydratedMemoryContext> for HydratedMemoryContextPayload {
    fn from(value: &HydratedMemoryContext) -> Self {
        Self {
            entries: value.entries.clone(),
            recent_window: value
                .recent_window
                .iter()
                .map(WindowTurnPayload::from)
                .collect(),
            diagnostics: Some(MemoryDiagnosticsPayload::from(&value.diagnostics)),
        }
    }
}

impl From<&MemoryRetrievalRequest> for MemoryRetrievalRequestPayload {
    fn from(value: &MemoryRetrievalRequest) -> Self {
        Self {
            session_id: value.session_id.clone(),
            memory_system_id: Some(value.memory_system_id.clone()),
            query: value.query.clone(),
            recall_mode: Some(value.recall_mode),
            scopes: value
                .scopes
                .iter()
                .copied()
                .map(MemoryScope::as_str)
                .map(str::to_owned)
                .collect(),
            budget_items: Some(value.budget_items),
            allowed_kinds: value
                .allowed_kinds
                .iter()
                .copied()
                .map(DerivedMemoryKind::as_str)
                .map(str::to_owned)
                .collect(),
        }
    }
}

impl From<&StageDiagnostics> for StageDiagnosticsPayload {
    fn from(value: &StageDiagnostics) -> Self {
        Self {
            family: value.family.as_str().to_owned(),
            outcome: value.outcome.as_str().to_owned(),
            budget_ms: value.budget_ms,
            elapsed_ms: value.elapsed_ms,
            fallback_activated: value.fallback_activated,
            message: value.message.clone(),
        }
    }
}

impl From<&StageEnvelope> for StageEnvelopePayload {
    fn from(value: &StageEnvelope) -> Self {
        Self {
            hydrated: Some(HydratedMemoryContextPayload::from(&value.hydrated)),
            retrieval_request: value
                .retrieval_request
                .as_ref()
                .map(MemoryRetrievalRequestPayload::from),
            diagnostics: value
                .diagnostics
                .iter()
                .map(StageDiagnosticsPayload::from)
                .collect(),
        }
    }
}

fn decode_hydrated_memory_context_payload(
    payload: HydratedMemoryContextPayload,
) -> Option<HydratedMemoryContext> {
    Some(HydratedMemoryContext {
        entries: payload.entries,
        recent_window: payload
            .recent_window
            .into_iter()
            .map(decode_window_turn_payload)
            .collect(),
        diagnostics: decode_memory_diagnostics_payload(payload.diagnostics?)?,
    })
}

fn decode_window_turn_payload(payload: WindowTurnPayload) -> WindowTurn {
    WindowTurn {
        role: payload.role,
        content: payload.content,
        ts: payload.ts,
    }
}

fn decode_memory_diagnostics_payload(
    payload: MemoryDiagnosticsPayload,
) -> Option<MemoryDiagnostics> {
    Some(MemoryDiagnostics {
        system_id: MemoryDiagnostics::normalize_system_id(payload.system_id.as_str())?,
        fail_open: payload.fail_open,
        strict_mode_requested: payload.strict_mode_requested,
        strict_mode_active: payload.strict_mode_active,
        degraded: payload.degraded,
        derivation_error: payload.derivation_error,
        retrieval_error: payload.retrieval_error,
        rank_error: payload.rank_error,
        recent_window_count: payload.recent_window_count,
        entry_count: payload.entry_count,
    })
}

fn decode_memory_retrieval_request_payload(
    payload: MemoryRetrievalRequestPayload,
) -> Option<MemoryRetrievalRequest> {
    if payload.session_id.trim().is_empty() {
        return None;
    }

    Some(MemoryRetrievalRequest {
        session_id: payload.session_id,
        memory_system_id: payload
            .memory_system_id
            .unwrap_or_else(|| crate::memory::DEFAULT_MEMORY_SYSTEM_ID.to_owned()),
        query: payload.query,
        recall_mode: payload.recall_mode.unwrap_or_default(),
        scopes: payload
            .scopes
            .into_iter()
            .filter_map(|scope| MemoryScope::parse_id(scope.as_str()))
            .collect(),
        budget_items: payload.budget_items.unwrap_or_default(),
        allowed_kinds: payload
            .allowed_kinds
            .into_iter()
            .filter_map(|kind| DerivedMemoryKind::parse_id(kind.as_str()))
            .collect(),
    })
}

fn decode_stage_diagnostics_payload(payload: StageDiagnosticsPayload) -> Option<StageDiagnostics> {
    Some(StageDiagnostics {
        family: MemoryStageFamily::parse_id(payload.family.as_str())?,
        outcome: StageOutcome::parse_id(payload.outcome.as_str())?,
        budget_ms: payload.budget_ms,
        elapsed_ms: payload.elapsed_ms,
        fallback_activated: payload.fallback_activated,
        message: payload.message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MemoryStageFamily, StageOutcome};

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

    #[test]
    fn decode_stage_envelope_tolerates_omitted_optional_fields() {
        let payload = json!({
            "hydrated": {
                "diagnostics": {
                    "system_id": " builtin "
                }
            },
            "retrieval_request": {
                "session_id": "session-123"
            },
            "diagnostics": [
                {
                    "family": "derive",
                    "outcome": "succeeded"
                },
                {
                    "family": "rank",
                    "outcome": "skipped"
                }
            ]
        });

        let envelope = decode_stage_envelope(&payload).expect("decode stage envelope");
        assert!(envelope.hydrated.entries.is_empty());
        assert!(envelope.hydrated.recent_window.is_empty());
        assert_eq!(envelope.hydrated.diagnostics.system_id, "builtin");
        assert!(!envelope.hydrated.diagnostics.fail_open);
        assert!(!envelope.hydrated.diagnostics.strict_mode_requested);
        assert!(!envelope.hydrated.diagnostics.strict_mode_active);
        assert!(!envelope.hydrated.diagnostics.degraded);
        assert_eq!(envelope.hydrated.diagnostics.derivation_error, None);
        assert_eq!(envelope.hydrated.diagnostics.retrieval_error, None);
        assert_eq!(envelope.hydrated.diagnostics.rank_error, None);
        assert_eq!(envelope.hydrated.diagnostics.recent_window_count, 0);
        assert_eq!(envelope.hydrated.diagnostics.entry_count, 0);

        let retrieval_request = envelope
            .retrieval_request
            .expect("retrieval request should decode");
        assert_eq!(retrieval_request.session_id, "session-123");
        assert_eq!(retrieval_request.memory_system_id, "builtin");
        assert_eq!(retrieval_request.query, None);
        assert_eq!(
            retrieval_request.recall_mode,
            MemoryRecallMode::PromptAssembly
        );
        assert!(retrieval_request.scopes.is_empty());
        assert_eq!(retrieval_request.budget_items, 0);
        assert!(retrieval_request.allowed_kinds.is_empty());

        assert_eq!(envelope.diagnostics.len(), 2);
        assert_eq!(envelope.diagnostics[0].family, MemoryStageFamily::Derive);
        assert_eq!(envelope.diagnostics[0].outcome, StageOutcome::Succeeded);
        assert_eq!(envelope.diagnostics[0].budget_ms, None);
        assert_eq!(envelope.diagnostics[0].elapsed_ms, None);
        assert!(!envelope.diagnostics[0].fallback_activated);
        assert_eq!(envelope.diagnostics[0].message, None);
        assert_eq!(envelope.diagnostics[1].family, MemoryStageFamily::Rank);
        assert_eq!(envelope.diagnostics[1].outcome, StageOutcome::Skipped);
    }

    #[test]
    fn decode_stage_envelope_preserves_rank_error_when_present() {
        let payload = json!({
            "hydrated": {
                "diagnostics": {
                    "system_id": "builtin",
                    "rank_error": "rank stage timeout"
                }
            },
            "diagnostics": []
        });

        let envelope = decode_stage_envelope(&payload).expect("decode stage envelope");
        let rank_error = envelope.hydrated.diagnostics.rank_error;

        assert_eq!(rank_error.as_deref(), Some("rank stage timeout"));
    }
}
