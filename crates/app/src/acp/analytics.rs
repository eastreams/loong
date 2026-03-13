use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::backend::{
    ACP_TURN_METADATA_ACK_CURSOR, ACP_TURN_METADATA_ROUTING_INTENT,
    ACP_TURN_METADATA_ROUTING_ORIGIN, ACP_TURN_METADATA_SOURCE_MESSAGE_ID,
    ACP_TURN_METADATA_TRACE_ID, AcpSessionState, AcpTurnResult, AcpTurnStopReason,
};
use super::binding::AcpSessionBindingScope;

pub const ACP_TURN_EVENT_RECORD: &str = "acp_turn_event";
pub const ACP_TURN_FINAL_RECORD: &str = "acp_turn_final";

#[derive(Debug, Clone, PartialEq)]
pub struct PersistedAcpConversationEventRecord {
    pub event: &'static str,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedAcpRuntimeEventContext {
    pub backend_id: String,
    pub agent_id: String,
    pub session_key: String,
    pub conversation_id: Option<String>,
    pub binding: Option<AcpSessionBindingScope>,
    pub request_metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpTurnEventSummary {
    pub turn_event_records: u32,
    pub final_records: u32,
    pub done_events: u32,
    pub error_events: u32,
    pub text_events: u32,
    pub usage_update_events: u32,
    pub turns_succeeded: u32,
    pub turns_cancelled: u32,
    pub turns_failed: u32,
    pub event_type_counts: BTreeMap<String, u32>,
    pub stop_reason_counts: BTreeMap<String, u32>,
    pub routing_intent_counts: BTreeMap<String, u32>,
    pub routing_origin_counts: BTreeMap<String, u32>,
    pub last_backend_id: Option<String>,
    pub last_agent_id: Option<String>,
    pub last_session_key: Option<String>,
    pub last_conversation_id: Option<String>,
    pub last_binding_route_session_id: Option<String>,
    pub last_channel_id: Option<String>,
    pub last_account_id: Option<String>,
    pub last_channel_conversation_id: Option<String>,
    pub last_channel_thread_id: Option<String>,
    pub last_routing_intent: Option<String>,
    pub last_routing_origin: Option<String>,
    pub last_trace_id: Option<String>,
    pub last_source_message_id: Option<String>,
    pub last_ack_cursor: Option<String>,
    pub last_turn_state: Option<String>,
    pub last_stop_reason: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConversationEventRecord {
    event: String,
    payload: Value,
}

pub fn build_persisted_turn_event_payload(
    backend_id: &str,
    agent_id: &str,
    session_key: &str,
    conversation_id: Option<&str>,
    binding: Option<&AcpSessionBindingScope>,
    request_metadata: Option<&BTreeMap<String, String>>,
    sequence: usize,
    event: &Value,
) -> Value {
    let mut payload = serde_json::Map::from_iter([
        ("backend_id".to_owned(), json!(backend_id)),
        ("agent_id".to_owned(), json!(agent_id)),
        ("session_key".to_owned(), json!(session_key)),
        ("sequence".to_owned(), json!(sequence)),
        ("raw_event".to_owned(), event.clone()),
    ]);
    if let Some(conversation_id) = conversation_id {
        payload.insert("conversation_id".to_owned(), json!(conversation_id));
    }
    append_binding_scope_fields(&mut payload, binding);
    append_request_metadata_fields(&mut payload, request_metadata);
    if let Some(event_type) = event_type_label(event) {
        payload.insert("event_type".to_owned(), json!(event_type));
    }
    if let Some(stop_reason) = stop_reason_label(event) {
        payload.insert("stop_reason".to_owned(), json!(stop_reason));
    }
    Value::Object(payload)
}

pub fn build_persisted_turn_final_payload(
    backend_id: &str,
    agent_id: &str,
    session_key: &str,
    conversation_id: Option<&str>,
    binding: Option<&AcpSessionBindingScope>,
    request_metadata: Option<&BTreeMap<String, String>>,
    event_count: usize,
    result: Option<&AcpTurnResult>,
    error: Option<&str>,
) -> Value {
    let mut payload = serde_json::Map::from_iter([
        ("backend_id".to_owned(), json!(backend_id)),
        ("agent_id".to_owned(), json!(agent_id)),
        ("session_key".to_owned(), json!(session_key)),
        ("event_count".to_owned(), json!(event_count)),
    ]);
    if let Some(conversation_id) = conversation_id {
        payload.insert("conversation_id".to_owned(), json!(conversation_id));
    }
    append_binding_scope_fields(&mut payload, binding);
    append_request_metadata_fields(&mut payload, request_metadata);

    match result {
        Some(result) => {
            payload.insert("state".to_owned(), json!(result.state));
            if let Some(stop_reason) = result.stop_reason {
                payload.insert(
                    "stop_reason".to_owned(),
                    json!(match stop_reason {
                        AcpTurnStopReason::Completed => "completed",
                        AcpTurnStopReason::Cancelled => "cancelled",
                    }),
                );
            }
            if let Some(usage) = result.usage.clone() {
                payload.insert("usage".to_owned(), usage);
            }
        }
        None => {
            payload.insert("state".to_owned(), json!(AcpSessionState::Error));
        }
    }

    if let Some(error) = error {
        payload.insert("error".to_owned(), json!(error));
    }

    Value::Object(payload)
}

pub fn build_persisted_runtime_event_records(
    context: &PersistedAcpRuntimeEventContext,
    events: &[Value],
    result: Option<&AcpTurnResult>,
    error: Option<&str>,
) -> Vec<PersistedAcpConversationEventRecord> {
    let mut records = events
        .iter()
        .enumerate()
        .map(|(sequence, event)| PersistedAcpConversationEventRecord {
            event: ACP_TURN_EVENT_RECORD,
            payload: build_persisted_turn_event_payload(
                context.backend_id.as_str(),
                context.agent_id.as_str(),
                context.session_key.as_str(),
                context.conversation_id.as_deref(),
                context.binding.as_ref(),
                Some(&context.request_metadata),
                sequence,
                event,
            ),
        })
        .collect::<Vec<_>>();
    records.push(PersistedAcpConversationEventRecord {
        event: ACP_TURN_FINAL_RECORD,
        payload: build_persisted_turn_final_payload(
            context.backend_id.as_str(),
            context.agent_id.as_str(),
            context.session_key.as_str(),
            context.conversation_id.as_deref(),
            context.binding.as_ref(),
            Some(&context.request_metadata),
            events.len(),
            result,
            error,
        ),
    });
    records
}

pub fn summarize_turn_events<'a, I>(contents: I) -> AcpTurnEventSummary
where
    I: IntoIterator<Item = &'a str>,
{
    let mut summary = AcpTurnEventSummary::default();

    for content in contents {
        let Some(record) = parse_conversation_event(content) else {
            continue;
        };

        match record.event.as_str() {
            ACP_TURN_EVENT_RECORD => {
                summary.turn_event_records = summary.turn_event_records.saturating_add(1);
                if let Some(label) = record
                    .payload
                    .get("event_type")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                {
                    bump_count(&mut summary.event_type_counts, label.as_str());
                    match label.as_str() {
                        "done" => summary.done_events = summary.done_events.saturating_add(1),
                        "error" => summary.error_events = summary.error_events.saturating_add(1),
                        "text" | "agent_message_chunk" => {
                            summary.text_events = summary.text_events.saturating_add(1);
                        }
                        "usage_update" => {
                            summary.usage_update_events =
                                summary.usage_update_events.saturating_add(1);
                        }
                        _ => {}
                    }
                }
                if let Some(stop_reason) = record
                    .payload
                    .get("stop_reason")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                {
                    bump_count(&mut summary.stop_reason_counts, stop_reason.as_str());
                }
                fold_last_turn_context(&record.payload, &mut summary);
            }
            ACP_TURN_FINAL_RECORD => {
                summary.final_records = summary.final_records.saturating_add(1);
                fold_last_turn_context(&record.payload, &mut summary);
                if let Some(stop_reason) = record
                    .payload
                    .get("stop_reason")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                {
                    bump_count(&mut summary.stop_reason_counts, stop_reason.as_str());
                    summary.last_stop_reason = Some(stop_reason.clone());
                    if stop_reason == "cancelled" {
                        summary.turns_cancelled = summary.turns_cancelled.saturating_add(1);
                    }
                }
                let is_failed = record
                    .payload
                    .get("error")
                    .and_then(Value::as_str)
                    .is_some();
                if let Some(routing_intent) = record
                    .payload
                    .get("routing_intent")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                {
                    bump_count(&mut summary.routing_intent_counts, routing_intent.as_str());
                }
                if let Some(routing_origin) = record
                    .payload
                    .get("routing_origin")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                {
                    bump_count(&mut summary.routing_origin_counts, routing_origin.as_str());
                }
                if is_failed {
                    summary.turns_failed = summary.turns_failed.saturating_add(1);
                    summary.last_error = record
                        .payload
                        .get("error")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned);
                } else if record.payload.get("stop_reason").and_then(Value::as_str)
                    == Some("cancelled")
                {
                    // already counted above
                } else {
                    summary.turns_succeeded = summary.turns_succeeded.saturating_add(1);
                }
            }
            _ => {}
        }
    }

    summary
}

pub fn merge_turn_events(result_events: &[Value], streamed_events: &[Value]) -> Vec<Value> {
    if streamed_events.is_empty() {
        return result_events.to_vec();
    }
    if result_events.is_empty() {
        return streamed_events.to_vec();
    }
    if result_events == streamed_events {
        return result_events.to_vec();
    }

    let mut merged = result_events.to_vec();
    for event in streamed_events {
        if !merged.iter().any(|candidate| candidate == event) {
            merged.push(event.clone());
        }
    }
    merged
}

fn fold_last_turn_context(payload: &Value, summary: &mut AcpTurnEventSummary) {
    summary.last_backend_id = payload
        .get("backend_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| summary.last_backend_id.clone());
    summary.last_agent_id = payload
        .get("agent_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            payload
                .get("session_key")
                .and_then(Value::as_str)
                .and_then(session_key_agent_id)
                .map(ToOwned::to_owned)
        })
        .or_else(|| summary.last_agent_id.clone());
    summary.last_session_key = payload
        .get("session_key")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| summary.last_session_key.clone());
    summary.last_conversation_id = payload
        .get("conversation_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| summary.last_conversation_id.clone());
    summary.last_binding_route_session_id = payload
        .get("binding_route_session_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| summary.last_binding_route_session_id.clone());
    summary.last_channel_id = payload
        .get("channel_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| summary.last_channel_id.clone());
    summary.last_account_id = payload
        .get("account_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| summary.last_account_id.clone());
    summary.last_channel_conversation_id = payload
        .get("channel_conversation_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| summary.last_channel_conversation_id.clone());
    summary.last_channel_thread_id = payload
        .get("channel_thread_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| summary.last_channel_thread_id.clone());
    summary.last_routing_intent = payload
        .get("routing_intent")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| summary.last_routing_intent.clone());
    summary.last_routing_origin = payload
        .get("routing_origin")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| summary.last_routing_origin.clone());
    summary.last_trace_id = payload
        .get("trace_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| summary.last_trace_id.clone());
    summary.last_source_message_id = payload
        .get("source_message_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| summary.last_source_message_id.clone());
    summary.last_ack_cursor = payload
        .get("ack_cursor")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| summary.last_ack_cursor.clone());
    summary.last_turn_state = payload
        .get("state")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| summary.last_turn_state.clone());
}

fn append_binding_scope_fields(
    payload: &mut serde_json::Map<String, Value>,
    binding: Option<&AcpSessionBindingScope>,
) {
    let Some(binding) = binding else {
        return;
    };
    payload.insert(
        "binding_route_session_id".to_owned(),
        json!(binding.route_session_id),
    );
    if let Some(channel_id) = binding.channel_id.as_deref() {
        payload.insert("channel_id".to_owned(), json!(channel_id));
    }
    if let Some(account_id) = binding.account_id.as_deref() {
        payload.insert("account_id".to_owned(), json!(account_id));
    }
    if let Some(conversation_id) = binding.conversation_id.as_deref() {
        payload.insert("channel_conversation_id".to_owned(), json!(conversation_id));
    }
    if let Some(thread_id) = binding.thread_id.as_deref() {
        payload.insert("channel_thread_id".to_owned(), json!(thread_id));
    }
}

fn append_request_metadata_fields(
    payload: &mut serde_json::Map<String, Value>,
    request_metadata: Option<&BTreeMap<String, String>>,
) {
    let Some(request_metadata) = request_metadata else {
        return;
    };
    if let Some(trace_id) = request_metadata
        .get(ACP_TURN_METADATA_TRACE_ID)
        .map(String::as_str)
    {
        payload.insert("trace_id".to_owned(), json!(trace_id));
    }
    if let Some(source_message_id) = request_metadata
        .get(ACP_TURN_METADATA_SOURCE_MESSAGE_ID)
        .map(String::as_str)
    {
        payload.insert("source_message_id".to_owned(), json!(source_message_id));
    }
    if let Some(ack_cursor) = request_metadata
        .get(ACP_TURN_METADATA_ACK_CURSOR)
        .map(String::as_str)
    {
        payload.insert("ack_cursor".to_owned(), json!(ack_cursor));
    }
    if let Some(routing_intent) = request_metadata
        .get(ACP_TURN_METADATA_ROUTING_INTENT)
        .map(String::as_str)
    {
        payload.insert("routing_intent".to_owned(), json!(routing_intent));
    }
    if let Some(routing_origin) = request_metadata
        .get(ACP_TURN_METADATA_ROUTING_ORIGIN)
        .map(String::as_str)
    {
        payload.insert("routing_origin".to_owned(), json!(routing_origin));
    }
}

fn parse_conversation_event(content: &str) -> Option<ConversationEventRecord> {
    let parsed = serde_json::from_str::<Value>(content).ok()?;
    if parsed.get("type")?.as_str()? != "conversation_event" {
        return None;
    }
    let event = parsed.get("event")?.as_str()?.to_owned();
    let payload = parsed.get("payload").cloned().unwrap_or(Value::Null);
    Some(ConversationEventRecord { event, payload })
}

fn event_type_label(event: &Value) -> Option<String> {
    value_string(event, "type")
        .or_else(|| value_string(event, "sessionUpdate"))
        .or_else(|| {
            event
                .get("params")
                .and_then(|params| params.get("update"))
                .and_then(|update| value_string(update, "sessionUpdate"))
        })
}

fn stop_reason_label(event: &Value) -> Option<String> {
    value_string(event, "stopReason")
        .or_else(|| value_string(event, "stop_reason"))
        .map(|value| value.to_ascii_lowercase())
}

fn value_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn bump_count(map: &mut BTreeMap<String, u32>, key: &str) {
    *map.entry(key.to_owned()).or_insert(0) += 1;
}

fn session_key_agent_id(session_key: &str) -> Option<&str> {
    session_key
        .strip_prefix("agent:")
        .and_then(|remainder| remainder.split_once(':').map(|(agent, _rest)| agent.trim()))
        .filter(|agent| !agent.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::{
        ACP_TURN_METADATA_ACK_CURSOR, ACP_TURN_METADATA_ROUTING_INTENT,
        ACP_TURN_METADATA_SOURCE_MESSAGE_ID, ACP_TURN_METADATA_TRACE_ID,
    };
    use std::collections::BTreeMap;

    #[test]
    fn summarize_turn_events_tracks_event_and_final_records() {
        let payloads = [
            json!({
                "type": "conversation_event",
                "event": ACP_TURN_EVENT_RECORD,
                "payload": {
                    "backend_id": "acpx",
                    "agent_id": "codex",
                    "session_key": "agent:codex:telegram:42",
                    "conversation_id": "telegram:42",
                    "binding_route_session_id": "telegram:bot_123456:42",
                    "channel_id": "telegram",
                    "account_id": "bot_123456",
                    "channel_conversation_id": "42",
                    "routing_intent": "explicit",
                    "routing_origin": "explicit_request",
                    "trace_id": "trace-123",
                    "source_message_id": "msg-42",
                    "ack_cursor": "cursor-77",
                    "sequence": 0,
                    "event_type": "text",
                    "raw_event": {
                        "type": "text",
                        "content": "hello"
                    }
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": ACP_TURN_EVENT_RECORD,
                "payload": {
                    "backend_id": "acpx",
                    "agent_id": "codex",
                    "session_key": "agent:codex:telegram:42",
                    "conversation_id": "telegram:42",
                    "binding_route_session_id": "telegram:bot_123456:42",
                    "channel_id": "telegram",
                    "account_id": "bot_123456",
                    "channel_conversation_id": "42",
                    "routing_intent": "explicit",
                    "routing_origin": "explicit_request",
                    "trace_id": "trace-123",
                    "source_message_id": "msg-42",
                    "ack_cursor": "cursor-77",
                    "sequence": 1,
                    "event_type": "done",
                    "stop_reason": "completed",
                    "raw_event": {
                        "type": "done",
                        "stopReason": "completed"
                    }
                }
            })
            .to_string(),
            json!({
                "type": "conversation_event",
                "event": ACP_TURN_FINAL_RECORD,
                "payload": {
                    "backend_id": "acpx",
                    "agent_id": "codex",
                    "session_key": "agent:codex:telegram:42",
                    "conversation_id": "telegram:42",
                    "binding_route_session_id": "telegram:bot_123456:42",
                    "channel_id": "telegram",
                    "account_id": "bot_123456",
                    "channel_conversation_id": "42",
                    "routing_intent": "explicit",
                    "routing_origin": "explicit_request",
                    "trace_id": "trace-123",
                    "source_message_id": "msg-42",
                    "ack_cursor": "cursor-77",
                    "event_count": 2,
                    "state": "ready",
                    "stop_reason": "completed"
                }
            })
            .to_string(),
        ];

        let summary = summarize_turn_events(payloads.iter().map(String::as_str));

        assert_eq!(summary.turn_event_records, 2);
        assert_eq!(summary.final_records, 1);
        assert_eq!(summary.text_events, 1);
        assert_eq!(summary.done_events, 1);
        assert_eq!(summary.turns_succeeded, 1);
        assert_eq!(summary.routing_intent_counts.get("explicit"), Some(&1));
        assert_eq!(
            summary.routing_origin_counts.get("explicit_request"),
            Some(&1)
        );
        assert_eq!(summary.last_agent_id.as_deref(), Some("codex"));
        assert_eq!(
            summary.last_session_key.as_deref(),
            Some("agent:codex:telegram:42")
        );
        assert_eq!(
            summary.last_binding_route_session_id.as_deref(),
            Some("telegram:bot_123456:42")
        );
        assert_eq!(summary.last_channel_id.as_deref(), Some("telegram"));
        assert_eq!(summary.last_account_id.as_deref(), Some("bot_123456"));
        assert_eq!(summary.last_channel_conversation_id.as_deref(), Some("42"));
        assert_eq!(summary.last_routing_intent.as_deref(), Some("explicit"));
        assert_eq!(
            summary.last_routing_origin.as_deref(),
            Some("explicit_request")
        );
        assert_eq!(summary.last_trace_id.as_deref(), Some("trace-123"));
        assert_eq!(summary.last_source_message_id.as_deref(), Some("msg-42"));
        assert_eq!(summary.last_ack_cursor.as_deref(), Some("cursor-77"));
        assert_eq!(summary.last_stop_reason.as_deref(), Some("completed"));
    }

    #[test]
    fn build_persisted_turn_event_payload_includes_binding_scope() {
        let binding = super::super::binding::AcpSessionBindingScope {
            route_session_id: "feishu:lark-prod:oc_123:om_thread_1".to_owned(),
            channel_id: Some("feishu".to_owned()),
            account_id: Some("lark-prod".to_owned()),
            conversation_id: Some("oc_123".to_owned()),
            thread_id: Some("om_thread_1".to_owned()),
        };
        let payload = build_persisted_turn_event_payload(
            "acpx",
            "codex",
            "agent:codex:opaque-session",
            Some("opaque-session"),
            Some(&binding),
            None,
            0,
            &json!({
                "type": "text",
                "content": "hello"
            }),
        );

        assert_eq!(
            payload["binding_route_session_id"],
            "feishu:lark-prod:oc_123:om_thread_1"
        );
        assert_eq!(payload["channel_id"], "feishu");
        assert_eq!(payload["account_id"], "lark-prod");
        assert_eq!(payload["channel_conversation_id"], "oc_123");
        assert_eq!(payload["channel_thread_id"], "om_thread_1");
    }

    #[test]
    fn build_persisted_turn_event_payload_includes_request_provenance() {
        let mut request_metadata = BTreeMap::new();
        request_metadata.insert(
            ACP_TURN_METADATA_ROUTING_INTENT.to_owned(),
            "explicit".to_owned(),
        );
        request_metadata.insert(
            ACP_TURN_METADATA_ROUTING_ORIGIN.to_owned(),
            "explicit_request".to_owned(),
        );
        request_metadata.insert(
            ACP_TURN_METADATA_TRACE_ID.to_owned(),
            "trace-abc".to_owned(),
        );
        request_metadata.insert(
            ACP_TURN_METADATA_SOURCE_MESSAGE_ID.to_owned(),
            "source-55".to_owned(),
        );
        request_metadata.insert(
            ACP_TURN_METADATA_ACK_CURSOR.to_owned(),
            "cursor-88".to_owned(),
        );

        let payload = build_persisted_turn_event_payload(
            "acpx",
            "codex",
            "agent:codex:telegram:42",
            Some("telegram:42"),
            None,
            Some(&request_metadata),
            0,
            &json!({
                "type": "text",
                "content": "hello"
            }),
        );

        assert_eq!(payload["routing_intent"], "explicit");
        assert_eq!(payload["routing_origin"], "explicit_request");
        assert_eq!(payload["trace_id"], "trace-abc");
        assert_eq!(payload["source_message_id"], "source-55");
        assert_eq!(payload["ack_cursor"], "cursor-88");
    }

    #[test]
    fn merge_turn_events_prefers_streamed_events_when_result_is_empty() {
        let streamed = vec![json!({"type": "text"}), json!({"type": "done"})];
        let merged = merge_turn_events(&[], &streamed);

        assert_eq!(merged, streamed);
    }

    #[test]
    fn build_persisted_turn_final_payload_marks_errors() {
        let binding = super::super::binding::AcpSessionBindingScope {
            route_session_id: "telegram:bot_123456:42".to_owned(),
            channel_id: Some("telegram".to_owned()),
            account_id: Some("bot_123456".to_owned()),
            conversation_id: Some("42".to_owned()),
            thread_id: Some("thread-a".to_owned()),
        };
        let payload = build_persisted_turn_final_payload(
            "acpx",
            "codex",
            "agent:codex:test",
            Some("test"),
            Some(&binding),
            None,
            0,
            None,
            Some("synthetic failure"),
        );

        assert_eq!(payload["backend_id"], "acpx");
        assert_eq!(payload["agent_id"], "codex");
        assert_eq!(payload["session_key"], "agent:codex:test");
        assert_eq!(payload["conversation_id"], "test");
        assert_eq!(
            payload["binding_route_session_id"],
            "telegram:bot_123456:42"
        );
        assert_eq!(payload["channel_id"], "telegram");
        assert_eq!(payload["account_id"], "bot_123456");
        assert_eq!(payload["channel_conversation_id"], "42");
        assert_eq!(payload["channel_thread_id"], "thread-a");
        assert_eq!(payload["state"], "error");
        assert_eq!(payload["error"], "synthetic failure");
    }

    #[test]
    fn build_persisted_runtime_event_records_emits_event_records_then_final_record() {
        let context = PersistedAcpRuntimeEventContext {
            backend_id: "acpx".to_owned(),
            agent_id: "codex".to_owned(),
            session_key: "agent:codex:telegram:42".to_owned(),
            conversation_id: Some("telegram:42".to_owned()),
            binding: None,
            request_metadata: BTreeMap::new(),
        };
        let records = build_persisted_runtime_event_records(
            &context,
            &[json!({"type": "text", "content": "hello"})],
            Some(&AcpTurnResult {
                output_text: "hello".to_owned(),
                state: AcpSessionState::Ready,
                usage: None,
                events: Vec::new(),
                stop_reason: Some(AcpTurnStopReason::Completed),
            }),
            None,
        );

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].event, ACP_TURN_EVENT_RECORD);
        assert_eq!(records[0].payload["sequence"], 0);
        assert_eq!(records[1].event, ACP_TURN_FINAL_RECORD);
        assert_eq!(records[1].payload["event_count"], 1);
        assert_eq!(records[1].payload["stop_reason"], "completed");
    }

    #[test]
    fn summarize_turn_events_derives_agent_id_from_legacy_session_key() {
        let payloads = [json!({
            "type": "conversation_event",
            "event": ACP_TURN_FINAL_RECORD,
            "payload": {
                "backend_id": "acpx",
                "session_key": "agent:claude:legacy-thread",
                "conversation_id": "legacy-thread",
                "event_count": 0,
                "state": "ready",
                "stop_reason": "completed"
            }
        })
        .to_string()];

        let summary = summarize_turn_events(payloads.iter().map(String::as_str));
        assert_eq!(summary.last_agent_id.as_deref(), Some("claude"));
    }
}
