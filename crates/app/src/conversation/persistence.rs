use serde::Serialize;
use serde_json::{Value, json};

use crate::CliResult;
use crate::acp::{
    AcpTurnResult, PersistedAcpRuntimeEventContext, build_persisted_runtime_event_records,
};
use crate::memory::{
    build_conversation_event_content, build_tool_decision_content, build_tool_outcome_content,
};

use super::runtime::ConversationRuntime;
use super::runtime_binding::ConversationRuntimeBinding;
use super::turn_shared::ReplyPersistenceMode;

pub(super) fn format_provider_error_reply(error: &str) -> String {
    format!("[provider_error] {error}")
}

pub(super) async fn persist_success_turns<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    user_input: &str,
    assistant_reply: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    persist_and_ingest_turn(runtime, session_id, "user", user_input, binding).await?;
    persist_and_ingest_turn(runtime, session_id, "assistant", assistant_reply, binding).await?;
    Ok(())
}

pub(super) async fn persist_reply_turns_with_mode<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    user_input: &str,
    assistant_reply: &str,
    persistence_mode: ReplyPersistenceMode,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    match persistence_mode {
        ReplyPersistenceMode::Success => {
            persist_success_turns(runtime, session_id, user_input, assistant_reply, binding).await
        }
        ReplyPersistenceMode::InlineProviderError => {
            persist_error_turns(runtime, session_id, user_input, assistant_reply, binding).await
        }
    }
}

/// Persist a tool decision as a structured JSON assistant message.
///
/// Uses the existing `persist_turn` mechanism so the DB schema stays unchanged.
/// The content is a single JSON line with `"type": "tool_decision"` plus
/// correlation identifiers (`session_id`, `turn_id`, `tool_call_id`).
pub(super) async fn persist_tool_decision<R, D>(
    runtime: &R,
    session_id: &str,
    turn_id: &str,
    tool_call_id: &str,
    decision: &D,
    binding: ConversationRuntimeBinding<'_>,
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
    persist_and_ingest_turn(runtime, session_id, "assistant", &content, binding).await
}

/// Persist a tool outcome as a structured JSON assistant message.
///
/// Uses the existing `persist_turn` mechanism so the DB schema stays unchanged.
/// The content is a single JSON line with `"type": "tool_outcome"` plus
/// correlation identifiers (`session_id`, `turn_id`, `tool_call_id`).
pub(super) async fn persist_tool_outcome<R, O>(
    runtime: &R,
    session_id: &str,
    turn_id: &str,
    tool_call_id: &str,
    outcome: &O,
    binding: ConversationRuntimeBinding<'_>,
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
    persist_and_ingest_turn(runtime, session_id, "assistant", &content, binding).await
}

pub(super) async fn persist_error_turns<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    user_input: &str,
    synthetic_reply: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    persist_and_ingest_turn(runtime, session_id, "user", user_input, binding).await?;
    persist_and_ingest_turn(runtime, session_id, "assistant", synthetic_reply, binding).await?;
    Ok(())
}

pub(super) async fn persist_success_turns_raw<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    user_input: &str,
    assistant_reply: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    persist_turn_only(runtime, session_id, "user", user_input, binding).await?;
    persist_turn_only(runtime, session_id, "assistant", assistant_reply, binding).await?;
    Ok(())
}

pub(super) async fn persist_reply_turns_raw_with_mode<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    user_input: &str,
    assistant_reply: &str,
    persistence_mode: ReplyPersistenceMode,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    match persistence_mode {
        ReplyPersistenceMode::Success => {
            persist_success_turns_raw(runtime, session_id, user_input, assistant_reply, binding)
                .await
        }
        ReplyPersistenceMode::InlineProviderError => {
            persist_error_turns_raw(runtime, session_id, user_input, assistant_reply, binding).await
        }
    }
}

pub(super) async fn persist_error_turns_raw<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    user_input: &str,
    synthetic_reply: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    persist_turn_only(runtime, session_id, "user", user_input, binding).await?;
    persist_turn_only(runtime, session_id, "assistant", synthetic_reply, binding).await?;
    Ok(())
}

async fn persist_and_ingest_turn<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    role: &str,
    content: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    runtime
        .persist_turn(session_id, role, content, binding)
        .await?;
    if let Some(kernel_ctx) = binding.kernel_context() {
        runtime
            .ingest(
                session_id,
                &json!({
                    "role": role,
                    "content": content,
                }),
                kernel_ctx,
            )
            .await?;
    }
    Ok(())
}

pub(super) async fn persist_conversation_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    event_name: &str,
    payload: Value,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    let content = build_conversation_event_content(event_name, payload);
    runtime
        .persist_turn(session_id, "assistant", &content, binding)
        .await
}

pub(super) async fn persist_acp_runtime_events<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    context: &PersistedAcpRuntimeEventContext,
    events: &[Value],
    result: Option<&AcpTurnResult>,
    error: Option<&str>,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    let records = build_persisted_runtime_event_records(context, events, result, error);
    for record in records {
        persist_conversation_event(runtime, session_id, record.event, record.payload, binding)
            .await?;
    }
    Ok(())
}

async fn persist_turn_only<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    role: &str,
    content: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    runtime
        .persist_turn(session_id, role, content, binding)
        .await
}
