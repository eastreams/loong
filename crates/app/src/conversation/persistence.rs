use serde_json::{json, Value};

use crate::CliResult;
use crate::KernelContext;

use super::runtime::ConversationRuntime;
use super::turn_engine::{ToolDecision, ToolOutcome};

pub(super) fn format_provider_error_reply(error: &str) -> String {
    format!("[provider_error] {error}")
}

pub(super) async fn persist_success_turns<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    user_input: &str,
    assistant_reply: &str,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<()> {
    runtime
        .persist_turn(session_id, "user", user_input, kernel_ctx)
        .await?;
    runtime
        .persist_turn(session_id, "assistant", assistant_reply, kernel_ctx)
        .await?;
    Ok(())
}

/// Persist a tool decision as a structured JSON assistant message.
///
/// Uses the existing `persist_turn` mechanism so the DB schema stays unchanged.
/// The content is a single JSON line with `"type": "tool_decision"` plus
/// correlation identifiers (`session_id`, `turn_id`, `tool_call_id`).
#[allow(dead_code)] // Will be wired into TurnEngine in a follow-up task
pub(super) async fn persist_tool_decision<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    turn_id: &str,
    tool_call_id: &str,
    decision: &ToolDecision,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<()> {
    let content = json!({
        "type": "tool_decision",
        "turn_id": turn_id,
        "tool_call_id": tool_call_id,
        "decision": serde_json::to_value(decision)
            .map_err(|e| format!("serialize tool decision: {e}"))?,
    });
    runtime
        .persist_turn(session_id, "assistant", &content.to_string(), kernel_ctx)
        .await
}

/// Persist a tool outcome as a structured JSON assistant message.
///
/// Uses the existing `persist_turn` mechanism so the DB schema stays unchanged.
/// The content is a single JSON line with `"type": "tool_outcome"` plus
/// correlation identifiers (`session_id`, `turn_id`, `tool_call_id`).
#[allow(dead_code)] // Will be wired into TurnEngine in a follow-up task
pub(super) async fn persist_tool_outcome<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    turn_id: &str,
    tool_call_id: &str,
    outcome: &ToolOutcome,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<()> {
    let content = json!({
        "type": "tool_outcome",
        "turn_id": turn_id,
        "tool_call_id": tool_call_id,
        "outcome": serde_json::to_value(outcome)
            .map_err(|e| format!("serialize tool outcome: {e}"))?,
    });
    runtime
        .persist_turn(session_id, "assistant", &content.to_string(), kernel_ctx)
        .await
}

pub(super) async fn persist_error_turns<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    user_input: &str,
    synthetic_reply: &str,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<()> {
    runtime
        .persist_turn(session_id, "user", user_input, kernel_ctx)
        .await?;
    runtime
        .persist_turn(session_id, "assistant", synthetic_reply, kernel_ctx)
        .await?;
    Ok(())
}

pub(super) async fn persist_conversation_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    event_name: &str,
    payload: Value,
    kernel_ctx: Option<&KernelContext>,
) -> CliResult<()> {
    let content = json!({
        "type": "conversation_event",
        "event": event_name,
        "payload": payload,
    });
    runtime
        .persist_turn(session_id, "assistant", &content.to_string(), kernel_ctx)
        .await
}
