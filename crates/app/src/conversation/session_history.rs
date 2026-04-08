#[cfg(feature = "memory-sqlite")]
use std::collections::BTreeSet;

#[cfg(feature = "memory-sqlite")]
use loongclaw_contracts::{Capability, MemoryCoreRequest};
#[cfg(feature = "memory-sqlite")]
use serde_json::{Value, json};

use crate::CliResult;
#[cfg(test)]
use crate::KernelContext;
#[cfg(feature = "memory-sqlite")]
use crate::memory;
#[cfg(feature = "memory-sqlite")]
use crate::memory::runtime_config::MemoryRuntimeConfig;

#[cfg(test)]
use super::analytics::{DiscoveryFirstEventSummary, summarize_discovery_first_events};
use super::analytics::{
    FastLaneToolBatchEventSummary, SafeLaneEventSummary, TurnCheckpointEventSummary,
    summarize_fast_lane_tool_batch_events, summarize_safe_lane_events,
    summarize_turn_checkpoint_history,
};
use super::runtime_binding::ConversationRuntimeBinding;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssistantHistoryLoadErrorCode {
    DirectReadFailed,
    KernelRequestFailed,
    KernelNonOkStatus,
    KernelMalformedPayload,
}

impl AssistantHistoryLoadErrorCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::DirectReadFailed => "direct_read_failed",
            Self::KernelRequestFailed => "kernel_request_failed",
            Self::KernelNonOkStatus => "kernel_non_ok_status",
            Self::KernelMalformedPayload => "kernel_malformed_payload",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AssistantHistoryLoadError {
    code: AssistantHistoryLoadErrorCode,
    message: String,
}

impl AssistantHistoryLoadError {
    #[cfg(feature = "memory-sqlite")]
    fn direct_read_failed(error: impl std::fmt::Display) -> Self {
        Self {
            code: AssistantHistoryLoadErrorCode::DirectReadFailed,
            message: format!("direct read failed: {error}"),
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn kernel_request_failed(error: impl std::fmt::Display) -> Self {
        Self {
            code: AssistantHistoryLoadErrorCode::KernelRequestFailed,
            message: format!("load assistant history via kernel failed: {error}"),
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn kernel_non_ok_status(status: impl AsRef<str>) -> Self {
        Self {
            code: AssistantHistoryLoadErrorCode::KernelNonOkStatus,
            message: format!(
                "load assistant history via kernel returned non-ok status: {}",
                status.as_ref()
            ),
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn kernel_malformed_payload(reason: impl AsRef<str>) -> Self {
        Self {
            code: AssistantHistoryLoadErrorCode::KernelMalformedPayload,
            message: format!(
                "load assistant history via kernel returned malformed payload: {}",
                reason.as_ref()
            ),
        }
    }

    pub(crate) fn code(&self) -> AssistantHistoryLoadErrorCode {
        self.code
    }
}

impl std::fmt::Display for AssistantHistoryLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

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
    #[cfg(test)]
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

#[cfg(test)]
pub(crate) async fn load_turn_checkpoint_event_summary(
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

pub(crate) async fn load_safe_lane_event_summary(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    #[cfg(feature = "memory-sqlite")] memory_config: &MemoryRuntimeConfig,
) -> CliResult<SafeLaneEventSummary> {
    #[cfg(feature = "memory-sqlite")]
    {
        load_assistant_history_summary(session_id, limit, binding, memory_config, |contents| {
            summarize_safe_lane_events(contents.iter().map(String::as_str))
        })
        .await
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, binding);
        Err("safe-lane summary unavailable: memory-sqlite feature disabled".to_owned())
    }
}

pub(crate) async fn load_fast_lane_tool_batch_event_summary(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    #[cfg(feature = "memory-sqlite")] memory_config: &MemoryRuntimeConfig,
) -> CliResult<FastLaneToolBatchEventSummary> {
    #[cfg(feature = "memory-sqlite")]
    {
        load_assistant_history_summary(session_id, limit, binding, memory_config, |contents| {
            summarize_fast_lane_tool_batch_events(contents.iter().map(String::as_str))
        })
        .await
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, binding);
        Err("fast-lane summary unavailable: memory-sqlite feature disabled".to_owned())
    }
}

#[cfg(test)]
pub(crate) async fn load_discovery_first_event_summary_with_binding(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    #[cfg(feature = "memory-sqlite")] memory_config: &MemoryRuntimeConfig,
) -> CliResult<DiscoveryFirstEventSummary> {
    #[cfg(feature = "memory-sqlite")]
    {
        load_assistant_history_summary(session_id, limit, binding, memory_config, |contents| {
            summarize_discovery_first_events(contents.iter().map(String::as_str))
        })
        .await
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (session_id, limit, binding);
        Err("discovery-first summary unavailable: memory-sqlite feature disabled".to_owned())
    }
}

#[cfg(test)]
pub(crate) async fn load_discovery_first_event_summary(
    session_id: &str,
    limit: usize,
    kernel_ctx: Option<&KernelContext>,
    #[cfg(feature = "memory-sqlite")] memory_config: &MemoryRuntimeConfig,
) -> CliResult<DiscoveryFirstEventSummary> {
    load_discovery_first_event_summary_with_binding(
        session_id,
        limit,
        kernel_ctx.map_or_else(
            ConversationRuntimeBinding::direct,
            ConversationRuntimeBinding::kernel,
        ),
        #[cfg(feature = "memory-sqlite")]
        memory_config,
    )
    .await
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
    load_assistant_contents_from_session_window_detailed(session_id, limit, binding, memory_config)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(feature = "memory-sqlite")]
async fn load_assistant_history_summary<T, F>(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    memory_config: &MemoryRuntimeConfig,
    summarize: F,
) -> CliResult<T>
where
    F: FnOnce(&[String]) -> T,
{
    let assistant_contents =
        load_assistant_contents_from_session_window(session_id, limit, binding, memory_config)
            .await?;
    Ok(summarize(&assistant_contents))
}

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn load_assistant_contents_from_session_window_detailed(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    memory_config: &MemoryRuntimeConfig,
) -> Result<Vec<String>, AssistantHistoryLoadError> {
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
            .await
            .map_err(AssistantHistoryLoadError::kernel_request_failed)?;

        if outcome.status != "ok" {
            return Err(AssistantHistoryLoadError::kernel_non_ok_status(
                &outcome.status,
            ));
        }

        return collect_assistant_contents_from_memory_window_payload(outcome.payload.get("turns"));
    }

    let turns = memory::window_direct(session_id, limit, memory_config)
        .map_err(AssistantHistoryLoadError::direct_read_failed)?;
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
) -> Result<Vec<String>, AssistantHistoryLoadError> {
    let turns = turns_payload.and_then(Value::as_array).ok_or_else(|| {
        AssistantHistoryLoadError::kernel_malformed_payload("missing or non-array turns")
    })?;
    let mut assistant_contents = Vec::new();
    for (index, turn) in turns.iter().enumerate() {
        if turn.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }

        let content = turn.get("content").and_then(Value::as_str).ok_or_else(|| {
            AssistantHistoryLoadError::kernel_malformed_payload(format!(
                "assistant turn at index {index} missing or non-string content"
            ))
        })?;
        assistant_contents.push(content.to_owned());
    }

    Ok(assistant_contents)
}
