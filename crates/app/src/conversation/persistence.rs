use serde::Serialize;
use serde_json::{Value, json};

use crate::acp::{
    AcpTurnResult, PersistedAcpRuntimeEventContext, build_persisted_runtime_event_records,
};
use crate::memory::{
    build_conversation_event_content, build_tool_decision_content, build_tool_outcome_content,
};
use crate::{CliResult, Context};

use super::runtime::ConversationRuntime;

const PROVIDER_ERROR_REPLY_PREFIX: &str = "[provider_error] ";

pub(super) fn format_provider_error_reply(error: &str) -> String {
    format!("{PROVIDER_ERROR_REPLY_PREFIX}{error}")
}

pub(super) fn provider_error_reply_body(reply: &str) -> Option<&str> {
    reply.strip_prefix(PROVIDER_ERROR_REPLY_PREFIX)
}

pub(super) async fn persist_reply_turns<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    user_input: &str,
    assistant_reply: &str,
    ctx: &Context<'_>,
) -> CliResult<()> {
    persist_and_ingest_turn(runtime, "user", user_input, ctx).await?;
    persist_and_ingest_turn(runtime, "assistant", assistant_reply, ctx).await
}

/// Persist a tool decision as a structured JSON assistant message.
///
/// Uses the existing `persist_turn` mechanism so the DB schema stays unchanged.
/// The content is a single JSON line with `"type": "tool_decision"` plus
/// correlation identifiers (`session_id`, `turn_id`, `tool_call_id`).
pub(super) async fn persist_tool_decision<R, D>(
    runtime: &R,
    turn_id: &str,
    tool_call_id: &str,
    decision: &D,
    ctx: &Context<'_>,
) -> CliResult<()>
where
    R: ConversationRuntime + ?Sized,
    D: Serialize + ?Sized,
{
    let content = build_tool_decision_content(
        turn_id,
        tool_call_id,
        serde_json::to_value(decision).map_err(|e| format!("serialize tool decision: {e}"))?,
    );
    persist_and_ingest_turn(runtime, "assistant", &content, ctx).await
}

/// Persist a tool outcome as a structured JSON assistant message.
///
/// Uses the existing `persist_turn` mechanism so the DB schema stays unchanged.
/// The content is a single JSON line with `"type": "tool_outcome"` plus
/// correlation identifiers (`session_id`, `turn_id`, `tool_call_id`).
pub(super) async fn persist_tool_outcome<R, O>(
    runtime: &R,
    turn_id: &str,
    tool_call_id: &str,
    outcome: &O,
    ctx: &Context<'_>,
) -> CliResult<()>
where
    R: ConversationRuntime + ?Sized,
    O: Serialize + ?Sized,
{
    let content = build_tool_outcome_content(
        turn_id,
        tool_call_id,
        serde_json::to_value(outcome).map_err(|e| format!("serialize tool outcome: {e}"))?,
    );
    persist_and_ingest_turn(runtime, "assistant", &content, ctx).await
}

pub(super) async fn persist_reply_turns_raw<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    user_input: &str,
    assistant_reply: &str,
    ctx: &Context<'_>,
) -> CliResult<()> {
    runtime.persist_turn("user", user_input, ctx).await?;
    runtime
        .persist_turn("assistant", assistant_reply, ctx)
        .await
}

/// Preserve the intentional two-stage boundary shared by structured turns:
/// durable legacy persistence first, then recursive context ingestion.
async fn persist_and_ingest_turn<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    role: &str,
    content: &str,
    ctx: &Context<'_>,
) -> CliResult<()> {
    runtime.persist_turn(role, content, ctx).await?;
    runtime
        .ingest(
            &json!({
                "role": role,
                "content": content,
            }),
            ctx,
        )
        .await?;
    Ok(())
}

pub(super) async fn persist_conversation_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    event_name: &str,
    payload: Value,
    ctx: &Context<'_>,
) -> CliResult<()> {
    let content = build_conversation_event_content(event_name, payload);
    runtime.persist_turn("assistant", &content, ctx).await
}

pub(super) async fn persist_acp_runtime_events<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    context: &PersistedAcpRuntimeEventContext,
    events: &[Value],
    result: Option<&AcpTurnResult>,
    error: Option<&str>,
    ctx: &Context<'_>,
) -> CliResult<()> {
    let records = build_persisted_runtime_event_records(context, events, result, error);
    for record in records {
        persist_conversation_event(runtime, record.event, record.payload, ctx).await?;
    }
    Ok(())
}
