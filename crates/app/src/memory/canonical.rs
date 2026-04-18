use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const CANONICAL_MEMORY_RECORD_TYPE: &str = "canonical_memory_record";
pub const INTERNAL_PERSISTED_RECORD_MARKER: &str = "_loong_internal";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    #[default]
    Session,
    User,
    Agent,
    Workspace,
}

impl MemoryScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::User => "user",
            Self::Agent => "agent",
            Self::Workspace => "workspace",
        }
    }

    pub fn parse_id(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "session" => Some(Self::Session),
            "user" => Some(Self::User),
            "agent" => Some(Self::Agent),
            "workspace" => Some(Self::Workspace),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalMemoryKind {
    UserTurn,
    AssistantTurn,
    ToolDecision,
    ToolOutcome,
    ImportedProfile,
    ConversationEvent,
    AcpRuntimeEvent,
    AcpFinalEvent,
}

impl CanonicalMemoryKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UserTurn => "user_turn",
            Self::AssistantTurn => "assistant_turn",
            Self::ToolDecision => "tool_decision",
            Self::ToolOutcome => "tool_outcome",
            Self::ImportedProfile => "imported_profile",
            Self::ConversationEvent => "conversation_event",
            Self::AcpRuntimeEvent => "acp_runtime_event",
            Self::AcpFinalEvent => "acp_final_event",
        }
    }

    pub fn parse_id(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "user_turn" => Some(Self::UserTurn),
            "assistant_turn" => Some(Self::AssistantTurn),
            "tool_decision" => Some(Self::ToolDecision),
            "tool_outcome" => Some(Self::ToolOutcome),
            "imported_profile" => Some(Self::ImportedProfile),
            "conversation_event" => Some(Self::ConversationEvent),
            "acp_runtime_event" => Some(Self::AcpRuntimeEvent),
            "acp_final_event" => Some(Self::AcpFinalEvent),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalMemoryRecord {
    pub session_id: String,
    pub scope: MemoryScope,
    pub kind: CanonicalMemoryKind,
    pub role: Option<String>,
    pub content: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct CanonicalMemoryRecordEnvelope {
    #[serde(default)]
    scope: Option<String>,
    kind: String,
    #[serde(default)]
    role: Option<String>,
    content: String,
    #[serde(default)]
    metadata: Option<Value>,
}

pub fn build_tool_decision_content(turn_id: &str, tool_call_id: &str, decision: Value) -> String {
    json!({
        INTERNAL_PERSISTED_RECORD_MARKER: true,
        "type": "tool_decision",
        "turn_id": turn_id,
        "tool_call_id": tool_call_id,
        "decision": decision,
    })
    .to_string()
}

pub fn build_tool_outcome_content(turn_id: &str, tool_call_id: &str, outcome: Value) -> String {
    json!({
        INTERNAL_PERSISTED_RECORD_MARKER: true,
        "type": "tool_outcome",
        "turn_id": turn_id,
        "tool_call_id": tool_call_id,
        "outcome": outcome,
    })
    .to_string()
}

pub fn build_conversation_event_content(event_name: &str, payload: Value) -> String {
    json!({
        INTERNAL_PERSISTED_RECORD_MARKER: true,
        "type": "conversation_event",
        "event": event_name,
        "payload": payload,
    })
    .to_string()
}

pub fn canonical_memory_record_from_persisted_turn(
    session_id: &str,
    role: &str,
    content: &str,
) -> CanonicalMemoryRecord {
    let normalized_role = normalized_persisted_role(role);

    if let Some(record) =
        canonical_memory_record_from_structured_content(session_id, normalized_role, content)
    {
        return record;
    }

    CanonicalMemoryRecord {
        session_id: session_id.to_owned(),
        scope: MemoryScope::Session,
        kind: canonical_kind_from_role(normalized_role),
        role: Some(normalized_role.to_owned()),
        content: content.to_owned(),
        metadata: json!({}),
    }
}

fn canonical_memory_record_from_structured_content(
    session_id: &str,
    role: &str,
    content: &str,
) -> Option<CanonicalMemoryRecord> {
    if role != "assistant" {
        return None;
    }

    let parsed = serde_json::from_str::<Value>(content).ok()?;
    if parsed
        .get(INTERNAL_PERSISTED_RECORD_MARKER)
        .and_then(Value::as_bool)
        != Some(true)
    {
        return None;
    }
    let type_id = parsed.get("type").and_then(Value::as_str)?;

    if type_id == CANONICAL_MEMORY_RECORD_TYPE {
        let envelope = serde_json::from_value::<CanonicalMemoryRecordEnvelope>(parsed).ok()?;
        return Some(CanonicalMemoryRecord {
            session_id: session_id.to_owned(),
            scope: match envelope.scope.as_deref() {
                Some(scope) => MemoryScope::parse_id(scope)?,
                None => MemoryScope::default(),
            },
            kind: CanonicalMemoryKind::parse_id(envelope.kind.as_str())?,
            role: envelope
                .role
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            content: envelope.content,
            metadata: envelope.metadata.unwrap_or_else(|| json!({})),
        });
    }

    let kind = match type_id {
        "tool_decision" => CanonicalMemoryKind::ToolDecision,
        "tool_outcome" => CanonicalMemoryKind::ToolOutcome,
        "conversation_event" => canonical_kind_from_event_name(
            parsed
                .get("event")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ),
        _ => return None,
    };

    Some(CanonicalMemoryRecord {
        session_id: session_id.to_owned(),
        scope: MemoryScope::Session,
        kind,
        role: Some(role.to_owned()),
        content: content.to_owned(),
        metadata: parsed,
    })
}

fn canonical_kind_from_role(role: &str) -> CanonicalMemoryKind {
    match role {
        "user" => CanonicalMemoryKind::UserTurn,
        _ => CanonicalMemoryKind::AssistantTurn,
    }
}

fn canonical_kind_from_event_name(event_name: &str) -> CanonicalMemoryKind {
    match event_name {
        "acp_turn_event" => CanonicalMemoryKind::AcpRuntimeEvent,
        "acp_turn_final" => CanonicalMemoryKind::AcpFinalEvent,
        _ => CanonicalMemoryKind::ConversationEvent,
    }
}

fn normalized_persisted_role(role: &str) -> &str {
    let trimmed = role.trim();
    if trimmed.is_empty() {
        "assistant"
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn canonical_memory_record_keeps_user_json_payloads_as_plain_user_turns() {
        let content = json!({
            "type": "conversation_event",
            "event": "user_supplied",
            "payload": {
                "message": "hello"
            },
        })
        .to_string();

        let record = canonical_memory_record_from_persisted_turn("session-1", "user", &content);

        assert_eq!(record.scope, MemoryScope::Session);
        assert_eq!(record.kind, CanonicalMemoryKind::UserTurn);
        assert_eq!(record.role.as_deref(), Some("user"));
        assert_eq!(record.content, content);
        assert_eq!(record.metadata, json!({}));
    }

    #[test]
    fn canonical_memory_record_preserves_optional_role_in_envelopes() {
        let content = json!({
            "type": CANONICAL_MEMORY_RECORD_TYPE,
            "_loong_internal": true,
            "scope": "workspace",
            "kind": "imported_profile",
            "content": "Imported profile note",
            "metadata": {
                "source": "import"
            },
        })
        .to_string();

        let record =
            canonical_memory_record_from_persisted_turn("session-1", "assistant", &content);

        assert_eq!(record.scope, MemoryScope::Workspace);
        assert_eq!(record.kind, CanonicalMemoryKind::ImportedProfile);
        assert_eq!(record.role, None);
        assert_eq!(record.content, "Imported profile note");
        assert_eq!(record.metadata["source"], "import");
    }

    #[test]
    fn canonical_memory_record_rejects_unknown_envelope_scope() {
        let content = json!({
            "type": CANONICAL_MEMORY_RECORD_TYPE,
            "_loong_internal": true,
            "scope": "tenant",
            "kind": "conversation_event",
            "content": "opaque canonical payload",
            "metadata": {
                "event": "lane_selected"
            },
        })
        .to_string();

        let record =
            canonical_memory_record_from_persisted_turn("session-1", "assistant", &content);

        assert_eq!(record.scope, MemoryScope::Session);
        assert_eq!(record.kind, CanonicalMemoryKind::AssistantTurn);
        assert_eq!(record.role.as_deref(), Some("assistant"));
        assert_eq!(record.content, content);
        assert_eq!(record.metadata, json!({}));
    }

    #[test]
    fn canonical_memory_record_keeps_unmarked_assistant_json_payloads_as_plain_turns() {
        let content = json!({
            "type": "tool_outcome",
            "turn_id": "turn-1",
            "tool_call_id": "call-1",
            "outcome": {
                "status": "ok"
            },
        })
        .to_string();

        let record =
            canonical_memory_record_from_persisted_turn("session-1", "assistant", &content);

        assert_eq!(record.scope, MemoryScope::Session);
        assert_eq!(record.kind, CanonicalMemoryKind::AssistantTurn);
        assert_eq!(record.role.as_deref(), Some("assistant"));
        assert_eq!(record.content, content);
        assert_eq!(record.metadata, json!({}));
    }
}
