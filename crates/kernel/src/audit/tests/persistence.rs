use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;

use super::super::{JsonlAuditSink, repair_jsonl_audit_journal, verify_jsonl_audit_journal};

#[test]
fn historical_tool_invocation_journal_reopens_verifies_and_repairs() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "loong-historical-tool-invocation-{}-{nonce}.jsonl",
        std::process::id(),
    ));
    let event = json!({
        "event_id": "evt-historical-tool-invocation",
        "timestamp_epoch_s": 1,
        "agent_id": "agent:historical",
        "kind": {
            "ToolInvocation": {
                "pack_id": "historical-pack",
                "path_display": "read",
                "required_capabilities": [],
                "outcome": "Completed"
            }
        }
    });
    fs::write(&path, format!("{event}\n")).expect("write historical audit fixture");

    let sink = JsonlAuditSink::new(path.clone())
        .expect("historical tool invocation tail should remain readable");
    drop(sink);
    let verification = verify_jsonl_audit_journal(&path).expect("historical journal should verify");
    let repair = repair_jsonl_audit_journal(&path).expect("historical journal should repair");

    assert!(verification.valid);
    assert_eq!(verification.total_events, 1);
    assert_eq!(repair.repaired_events, 1);
    let repaired = fs::read_to_string(&path).expect("read repaired historical journal");
    assert!(repaired.contains("historical-pack"));
    let _ = fs::remove_file(path);
}
