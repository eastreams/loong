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
