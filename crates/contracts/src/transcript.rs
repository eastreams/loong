//! The canonical transcript vocabulary for one session.
//!
//! Every Loong component that builds or reads agent context speaks this
//! vocabulary. Storage, adapters, and channels map to their own formats at
//! the edge; they never invent parallel item shapes here.
//!
//! `transcript` is deliberate. The items record a full session in order,
//! including tool interactions. `history` would collide later with the
//! compacted vs working context split.

use alloc::string::String;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A stable id for one transcript item.
///
/// The same id space addresses tool calls and their results. Callers should
/// generate v7 ids so id order matches creation order. `loong-contracts`
/// holds ids only; generation lives in std callers.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TranscriptItemId(Uuid);

impl From<Uuid> for TranscriptItemId {
    fn from(id: Uuid) -> Self {
        Self(id)
    }
}

impl From<TranscriptItemId> for Uuid {
    fn from(id: TranscriptItemId) -> Self {
        id.0
    }
}

/// One entry of a session transcript.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TranscriptItem {
    pub id: TranscriptItemId,
    /// Flattened so the JSON line is one flat record with a `kind` tag.
    #[serde(flatten)]
    pub kind: TranscriptItemKind,
}

/// What kind of record an item carries.
///
/// Tool interactions are first-class. Flattening them into message text
/// would drop the call identity and arguments needed for replay.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TranscriptItemKind {
    /// Plain text from one participant.
    Message { role: Role, text: String },
    /// The assistant asked a tool to run.
    ///
    /// `arguments` is pre-serialized JSON text owned by the caller. The
    /// store replays bytes; it never interprets tool payloads.
    ToolCall { name: String, arguments: String },
    /// A tool finished and produced output.
    ///
    /// `call_id` points at the matching `ToolCall` item.
    ToolResult {
        call_id: TranscriptItemId,
        output: String,
    },
}

/// Who produced a message.
///
/// This applies to messages only. Tool records have no role.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;
    use alloc::vec;
    use alloc::vec::Vec;

    use serde_json::json;

    use super::{Role, TranscriptItem, TranscriptItemId, TranscriptItemKind};

    fn id(digits: u64) -> TranscriptItemId {
        TranscriptItemId::from(uuid::Uuid::from_u128(u128::from(digits)))
    }

    fn sample() -> Vec<TranscriptItem> {
        let call = id(1);
        vec![
            TranscriptItem {
                id: id(2),
                kind: TranscriptItemKind::Message {
                    role: Role::System,
                    text: "be terse".to_string(),
                },
            },
            TranscriptItem {
                id: call,
                kind: TranscriptItemKind::ToolCall {
                    name: "echo".to_string(),
                    arguments: json!({"text": "hi"}).to_string(),
                },
            },
            TranscriptItem {
                id: id(3),
                kind: TranscriptItemKind::ToolResult {
                    call_id: call,
                    output: "hi".to_string(),
                },
            },
        ]
    }

    #[test]
    fn json_shape_uses_kind_tags_and_snake_case_roles() {
        let json = serde_json::to_value(sample()).unwrap();
        assert_eq!(json[0]["kind"], "message");
        assert_eq!(json[0]["role"], "system");
        assert_eq!(json[1]["kind"], "tool_call");
        assert_eq!(json[1]["arguments"], json!({"text": "hi"}).to_string());
        assert_eq!(json[2]["kind"], "tool_result");
        assert_eq!(json[2]["call_id"], json[1]["id"]);
    }

    #[test]
    fn json_round_trip_preserves_items() {
        let items = sample();
        let json = serde_json::to_string(&items).unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<TranscriptItem>>(&json).unwrap(),
            items
        );
    }
}
