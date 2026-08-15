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

/// One entry of a session transcript.
///
/// The order of items in the log is the source of truth; individual items do
/// not carry a transcript-wide id. Tool invocations carry a `call_id` on both
/// the `ToolCall` and its `ToolResult`, which is the only cross-item identity
/// this vocabulary needs.
///
/// The `kind` tag keeps every JSON line flat while naming the variant.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TranscriptItem {
    /// Plain text from one participant.
    Message { role: Role, text: String },
    /// The assistant asked a tool to run.
    ///
    /// `call_id` links this call to its eventual `ToolResult`.
    /// `arguments` is pre-serialized JSON text owned by the caller. The
    /// store replays bytes; it never interprets tool payloads.
    ToolCall {
        call_id: Uuid,
        name: String,
        arguments: String,
    },
    /// A tool finished and produced output.
    ///
    /// `call_id` points at the matching `ToolCall`.
    ToolResult { call_id: Uuid, output: String },
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
    use uuid::Uuid;

    use super::{Role, TranscriptItem};

    fn call_id(digits: u64) -> Uuid {
        Uuid::from_u128(u128::from(digits))
    }

    fn sample() -> Vec<TranscriptItem> {
        let id = call_id(1);
        vec![
            TranscriptItem::Message {
                role: Role::System,
                text: "be terse".to_string(),
            },
            TranscriptItem::ToolCall {
                call_id: id,
                name: "echo".to_string(),
                arguments: json!({"text": "hi"}).to_string(),
            },
            TranscriptItem::ToolResult {
                call_id: id,
                output: "hi".to_string(),
            },
        ]
    }

    #[test]
    fn json_shape_uses_kind_tags_and_snake_case_roles() {
        let json = serde_json::to_value(sample()).unwrap();
        assert_eq!(json[0]["kind"], "message");
        assert_eq!(json[0]["role"], "system");
        assert_eq!(json[0].get("id"), None);
        assert_eq!(json[1]["kind"], "tool_call");
        assert_eq!(json[1]["arguments"], json!({"text": "hi"}).to_string());
        assert_eq!(json[1]["call_id"], json[2]["call_id"]);
        assert_eq!(json[2]["kind"], "tool_result");
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
