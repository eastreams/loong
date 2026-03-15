#[cfg(feature = "memory-sqlite")]
use std::collections::BTreeSet;

#[cfg(feature = "memory-sqlite")]
use loongclaw_contracts::{Capability, MemoryCoreRequest};
#[cfg(feature = "memory-sqlite")]
use serde_json::{Value, json};

use crate::CliResult;
#[cfg(feature = "memory-sqlite")]
use crate::memory;
#[cfg(feature = "memory-sqlite")]
use crate::memory::runtime_config::MemoryRuntimeConfig;

use super::analytics::{
    SafeLaneEventSummary, TurnCheckpointEventSummary, summarize_safe_lane_events,
    summarize_turn_checkpoint_history,
};
use super::runtime_binding::ConversationRuntimeBinding;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TurnCheckpointLatestEntry {
    pub summary: TurnCheckpointEventSummary,
    pub checkpoint: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TurnCheckpointHistorySnapshot {
    summary: TurnCheckpointEventSummary,
    latest_checkpoint: Option<Value>,
}

impl TurnCheckpointHistorySnapshot {
    pub(crate) fn into_summary(self) -> TurnCheckpointEventSummary {
        self.summary
    }

    pub(crate) fn into_latest_entry(self) -> Option<TurnCheckpointLatestEntry> {
        self.latest_checkpoint
            .map(|checkpoint| TurnCheckpointLatestEntry {
                summary: self.summary,
                checkpoint,
            })
    }

    pub(crate) fn into_summary_and_latest_entry(
        self,
    ) -> (
        TurnCheckpointEventSummary,
        Option<TurnCheckpointLatestEntry>,
    ) {
        let summary = self.summary;
        let latest_entry = self
            .latest_checkpoint
            .map(|checkpoint| TurnCheckpointLatestEntry {
                summary: summary.clone(),
                checkpoint,
            });
        (summary, latest_entry)
    }
}

pub async fn load_turn_checkpoint_event_summary(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    #[cfg(feature = "memory-sqlite")] memory_config: &MemoryRuntimeConfig,
) -> CliResult<TurnCheckpointEventSummary> {
    #[cfg(feature = "memory-sqlite")]
    {
        Ok(
            load_turn_checkpoint_history_snapshot(session_id, limit, binding, memory_config)
                .await?
                .into_summary(),
        )
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, binding);
        Err("turn checkpoint summary unavailable: memory-sqlite feature disabled".to_owned())
    }
}

pub async fn load_safe_lane_event_summary(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    #[cfg(feature = "memory-sqlite")] memory_config: &MemoryRuntimeConfig,
) -> CliResult<SafeLaneEventSummary> {
    #[cfg(feature = "memory-sqlite")]
    {
        let assistant_contents =
            load_assistant_contents_from_session_window(session_id, limit, binding, memory_config)
                .await?;
        Ok(summarize_safe_lane_events(
            assistant_contents.iter().map(String::as_str),
        ))
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, binding);
        Err("safe-lane summary unavailable: memory-sqlite feature disabled".to_owned())
    }
}

pub(crate) async fn load_latest_turn_checkpoint_entry(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    #[cfg(feature = "memory-sqlite")] memory_config: &MemoryRuntimeConfig,
) -> CliResult<Option<TurnCheckpointLatestEntry>> {
    #[cfg(feature = "memory-sqlite")]
    {
        Ok(
            load_turn_checkpoint_history_snapshot(session_id, limit, binding, memory_config)
                .await?
                .into_latest_entry(),
        )
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, binding);
        Err("turn checkpoint entry unavailable: memory-sqlite feature disabled".to_owned())
    }
}

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn load_turn_checkpoint_history_snapshot(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    memory_config: &MemoryRuntimeConfig,
) -> CliResult<TurnCheckpointHistorySnapshot> {
    let assistant_contents =
        load_assistant_contents_from_session_window(session_id, limit, binding, memory_config)
            .await?;
    Ok(build_turn_checkpoint_history_snapshot(&assistant_contents))
}

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn load_assistant_contents_from_session_window(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    memory_config: &MemoryRuntimeConfig,
) -> CliResult<Vec<String>> {
    if let Some(ctx) = binding.kernel_context() {
        let request = MemoryCoreRequest {
            operation: memory::MEMORY_OP_WINDOW.to_owned(),
            payload: json!({
                "session_id": session_id,
                "limit": limit,
                "allow_extended_limit": true,
            }),
        };
        let caps = BTreeSet::from([Capability::MemoryRead]);
        let outcome = ctx
            .kernel
            .execute_memory_core(ctx.pack_id(), &ctx.token, &caps, None, request)
            .await;
        if let Ok(outcome) = outcome
            && outcome.status == "ok"
        {
            return Ok(collect_assistant_contents_from_memory_window_payload(
                outcome.payload.get("turns"),
            ));
        }
    }

    let turns = memory::window_direct(session_id, limit, memory_config)
        .map_err(|error| format!("load turn checkpoint summary failed: {error}"))?;
    Ok(turns
        .iter()
        .filter_map(|turn| (turn.role == "assistant").then_some(turn.content.clone()))
        .collect())
}

#[cfg(feature = "memory-sqlite")]
fn build_turn_checkpoint_history_snapshot(
    assistant_contents: &[String],
) -> TurnCheckpointHistorySnapshot {
    let projection =
        summarize_turn_checkpoint_history(assistant_contents.iter().map(String::as_str));
    TurnCheckpointHistorySnapshot {
        summary: projection.summary,
        latest_checkpoint: projection.latest_checkpoint,
    }
}

#[cfg(feature = "memory-sqlite")]
fn collect_assistant_contents_from_memory_window_payload(
    turns_payload: Option<&Value>,
) -> Vec<String> {
    turns_payload
        .and_then(Value::as_array)
        .map(|turns| {
            turns
                .iter()
                .filter_map(|turn| {
                    (turn.get("role").and_then(Value::as_str) == Some("assistant"))
                        .then(|| {
                            turn.get("content")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                        })
                        .map(ToOwned::to_owned)
                })
                .collect()
        })
        .unwrap_or_default()
}
