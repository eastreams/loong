use loong_contracts::{AuditEventKind, ExecutionPlane, PlaneTier};
use serde_json::{Value, json};

use super::*;
use crate::conversation::tool_discovery_state::{
    TOOL_DISCOVERY_REFRESHED_EVENT_NAME, ToolDiscoveryState,
};

pub(super) async fn persist_tool_discovery_refresh_event_if_needed<
    R: ConversationRuntime + ?Sized,
>(
    runtime: &R,
    session_id: &str,
    intent: &ToolIntent,
    intent_sequence: usize,
    tool_name: &str,
    outcome: &loong_contracts::ToolCoreOutcome,
    binding: ConversationRuntimeBinding<'_>,
) {
    if tool_name != "tool.search" {
        return;
    }

    if outcome.status != "ok" {
        return;
    }

    let Some(discovery_state) = ToolDiscoveryState::from_tool_search_payload(&outcome.payload)
    else {
        return;
    };
    let Some(discovery_payload) =
        build_tool_discovery_refresh_event_payload(discovery_state, intent, intent_sequence)
    else {
        return;
    };
    let persist_result = persist_conversation_event(
        runtime,
        session_id,
        TOOL_DISCOVERY_REFRESHED_EVENT_NAME,
        discovery_payload,
        binding,
    )
    .await;

    if persist_result.is_ok() {
        return;
    }

    let Some(ctx) = binding.kernel_context() else {
        return;
    };

    let _ = ctx.kernel.record_audit_event(
        Some(ctx.agent_id()),
        AuditEventKind::PlaneInvoked {
            pack_id: ctx.pack_id().to_owned(),
            plane: ExecutionPlane::Runtime,
            tier: PlaneTier::Core,
            primary_adapter: "conversation.runtime".to_owned(),
            delegated_core_adapter: None,
            operation: "conversation.runtime.tool_discovery_persist_failed".to_owned(),
            required_capabilities: Vec::new(),
        },
    );
}

fn build_tool_discovery_refresh_event_payload(
    discovery_state: ToolDiscoveryState,
    intent: &ToolIntent,
    intent_sequence: usize,
) -> Option<Value> {
    let discovery_payload = serde_json::to_value(discovery_state).ok()?;
    let Value::Object(mut discovery_payload) = discovery_payload else {
        return None;
    };
    let turn_id = intent.turn_id.trim();
    let tool_call_id = intent.tool_call_id.trim();

    if !turn_id.is_empty() {
        discovery_payload.insert("turn_id".to_owned(), Value::String(turn_id.to_owned()));
    }

    if !tool_call_id.is_empty() {
        discovery_payload.insert(
            "tool_call_id".to_owned(),
            Value::String(tool_call_id.to_owned()),
        );
    }

    discovery_payload.insert("intent_sequence".to_owned(), json!(intent_sequence));

    Some(Value::Object(discovery_payload))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::build_tool_discovery_refresh_event_payload;
    use crate::conversation::tool_discovery_state::{ToolDiscoveryEntry, ToolDiscoveryState};
    use crate::conversation::turn_engine::ToolIntent;

    #[test]
    fn build_tool_discovery_refresh_event_payload_adds_runtime_metadata() {
        let state = ToolDiscoveryState {
            schema_version: 1,
            query: Some("read note.md".to_owned()),
            exact_tool_id: Some("read".to_owned()),
            entries: vec![ToolDiscoveryEntry {
                tool_id: "read".to_owned(),
                summary: "Read a file.".to_owned(),
                argument_hint: Some("path:string".to_owned()),
                required_fields: vec!["path".to_owned()],
                required_field_groups: vec![vec!["path".to_owned()]],
            }],
            diagnostics: None,
        };
        let intent = ToolIntent {
            tool_name: "tool.search".to_owned(),
            args_json: json!({"query": "read note.md"}),
            source: "provider".to_owned(),
            session_id: "session-1".to_owned(),
            turn_id: " turn-1 ".to_owned(),
            tool_call_id: " call-1 ".to_owned(),
        };

        let payload = build_tool_discovery_refresh_event_payload(state, &intent, 3)
            .expect("discovery payload");

        assert_eq!(payload["query"], "read note.md");
        assert_eq!(payload["intent_sequence"], 3);
        assert_eq!(payload["turn_id"], "turn-1");
        assert_eq!(payload["tool_call_id"], "call-1");
        assert!(payload["entries"][0].get("surface_id").is_none());
        assert!(payload["entries"][0].get("search_hint").is_none());
        assert!(payload["entries"][0].get("usage_guidance").is_none());
    }

    #[test]
    fn build_tool_discovery_refresh_event_payload_skips_blank_runtime_metadata() {
        let state = ToolDiscoveryState {
            schema_version: 1,
            query: None,
            exact_tool_id: None,
            entries: Vec::new(),
            diagnostics: None,
        };
        let intent = ToolIntent {
            tool_name: "tool.search".to_owned(),
            args_json: json!({}),
            source: "provider".to_owned(),
            session_id: "session-1".to_owned(),
            turn_id: "   ".to_owned(),
            tool_call_id: "".to_owned(),
        };

        let payload = build_tool_discovery_refresh_event_payload(state, &intent, 0)
            .expect("discovery payload");

        assert_eq!(payload["intent_sequence"], 0);
        assert!(payload.get("turn_id").is_none());
        assert!(payload.get("tool_call_id").is_none());
    }
}
