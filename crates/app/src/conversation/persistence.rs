#[cfg(feature = "memory-sqlite")]
use std::collections::BTreeSet;

#[cfg(feature = "memory-sqlite")]
use loongclaw_contracts::{Capability, MemoryCoreRequest};
use serde_json::json;

#[cfg(feature = "memory-sqlite")]
use crate::memory::runtime_config::MemoryRuntimeConfig;
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
    let content = tool_decision_turn_content(turn_id, tool_call_id, decision)?;
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
    let content = tool_outcome_turn_content(turn_id, tool_call_id, outcome)?;
    runtime
        .persist_turn(session_id, "assistant", &content.to_string(), kernel_ctx)
        .await
}

fn tool_decision_turn_content(
    turn_id: &str,
    tool_call_id: &str,
    decision: &ToolDecision,
) -> Result<serde_json::Value, String> {
    Ok(json!({
        "type": "tool_decision",
        "turn_id": turn_id,
        "tool_call_id": tool_call_id,
        "decision": serde_json::to_value(decision)
            .map_err(|e| format!("serialize tool decision: {e}"))?,
    }))
}

fn tool_outcome_turn_content(
    turn_id: &str,
    tool_call_id: &str,
    outcome: &ToolOutcome,
) -> Result<serde_json::Value, String> {
    Ok(json!({
        "type": "tool_outcome",
        "turn_id": turn_id,
        "tool_call_id": tool_call_id,
        "outcome": serde_json::to_value(outcome)
            .map_err(|e| format!("serialize tool outcome: {e}"))?,
    }))
}

#[cfg(feature = "memory-sqlite")]
async fn persist_structured_assistant_turn_with_memory_config(
    memory_config: &MemoryRuntimeConfig,
    session_id: &str,
    content: serde_json::Value,
    kernel_ctx: Option<&KernelContext>,
) -> Result<(), String> {
    let serialized_content = content.to_string();
    if let Some(ctx) = kernel_ctx {
        let request = MemoryCoreRequest {
            operation: "append_turn".to_owned(),
            payload: json!({
                "session_id": session_id,
                "role": "assistant",
                "content": serialized_content,
            }),
        };
        let caps = BTreeSet::from([Capability::MemoryWrite]);
        ctx.kernel
            .execute_memory_core(ctx.pack_id(), &ctx.token, &caps, None, request)
            .await
            .map_err(|error| format!("persist assistant turn via kernel failed: {error}"))?;
        return Ok(());
    }

    crate::memory::append_turn_direct(session_id, "assistant", &serialized_content, memory_config)
        .map_err(|error| format!("persist assistant turn failed: {error}"))
}

#[cfg(feature = "memory-sqlite")]
pub(super) async fn persist_tool_decision_with_memory_config(
    memory_config: &MemoryRuntimeConfig,
    session_id: &str,
    turn_id: &str,
    tool_call_id: &str,
    decision: &ToolDecision,
    kernel_ctx: Option<&KernelContext>,
) -> Result<(), String> {
    let content = tool_decision_turn_content(turn_id, tool_call_id, decision)?;
    persist_structured_assistant_turn_with_memory_config(
        memory_config,
        session_id,
        content,
        kernel_ctx,
    )
    .await
}

#[cfg(feature = "memory-sqlite")]
pub(super) async fn persist_tool_outcome_with_memory_config(
    memory_config: &MemoryRuntimeConfig,
    session_id: &str,
    turn_id: &str,
    tool_call_id: &str,
    outcome: &ToolOutcome,
    kernel_ctx: Option<&KernelContext>,
) -> Result<(), String> {
    let content = tool_outcome_turn_content(turn_id, tool_call_id, outcome)?;
    persist_structured_assistant_turn_with_memory_config(
        memory_config,
        session_id,
        content,
        kernel_ctx,
    )
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
