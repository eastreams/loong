use serde_json::Value;

#[cfg(feature = "memory-sqlite")]
use super::analytics::{
    DiscoveryFirstEventSummary, FastLaneToolBatchEventSummary, PromptFrameEventSummary,
    SafeLaneEventSummary, TurnCheckpointEventSummary, summarize_discovery_first_events,
    summarize_fast_lane_tool_batch_events, summarize_prompt_frame_events,
    summarize_safe_lane_events, summarize_turn_checkpoint_history,
};
#[cfg(feature = "memory-sqlite")]
use super::runtime::ConversationRuntime;
use crate::{CliResult, Context};

/// Stable failure class for the typed session-window read boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistantHistoryLoadErrorCode {
    Unavailable,
    PolicyDenied,
    BackendFailed,
    MalformedOutput,
}

impl AssistantHistoryLoadErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::PolicyDenied => "policy_denied",
            Self::BackendFailed => "backend_failed",
            Self::MalformedOutput => "malformed_output",
        }
    }
}

/// A session-window read failure with a machine-readable classification.
///
/// The message retains the concrete kernel failure for operators while callers
/// such as the safe-lane governor branch only on [`Self::code`].
#[derive(Debug, thiserror::Error)]
pub enum AssistantHistoryLoadError {
    #[error("{message}")]
    Unavailable { message: String },
    #[error(transparent)]
    Memory(#[from] loong_kernel::access::memory::MemoryAccessError),
}

impl AssistantHistoryLoadError {
    #[cfg(feature = "memory-sqlite")]
    pub(crate) fn unavailable(error: impl std::fmt::Display) -> Self {
        Self::Unavailable {
            message: format!("read session window failed: {error}"),
        }
    }

    pub fn code(&self) -> AssistantHistoryLoadErrorCode {
        match self {
            Self::Unavailable { .. } => AssistantHistoryLoadErrorCode::Unavailable,
            Self::Memory(loong_kernel::access::memory::MemoryAccessError::Authorization(_)) => {
                AssistantHistoryLoadErrorCode::PolicyDenied
            }
            Self::Memory(loong_kernel::access::memory::MemoryAccessError::Backend(
                loong_kernel::access::memory::MemoryBackendError::Execution { .. },
            )) => AssistantHistoryLoadErrorCode::BackendFailed,
            Self::Memory(loong_kernel::access::memory::MemoryAccessError::Backend(
                loong_kernel::access::memory::MemoryBackendError::MalformedOutput { .. },
            )) => AssistantHistoryLoadErrorCode::MalformedOutput,
        }
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

pub async fn load_turn_checkpoint_event_summary<R: ConversationRuntime + ?Sized>(
    limit: usize,
    ctx: &Context<'_>,
    runtime: &R,
) -> CliResult<TurnCheckpointEventSummary> {
    #[cfg(feature = "memory-sqlite")]
    {
        Ok(load_turn_checkpoint_history_snapshot(limit, ctx, runtime)
            .await?
            .into_summary())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (limit, ctx, runtime);
        Err("turn checkpoint summary unavailable: memory-sqlite feature disabled".to_owned())
    }
}

pub async fn load_safe_lane_event_summary<R: ConversationRuntime + ?Sized>(
    limit: usize,
    ctx: &Context<'_>,
    runtime: &R,
) -> CliResult<SafeLaneEventSummary> {
    #[cfg(feature = "memory-sqlite")]
    {
        load_assistant_history_summary(limit, ctx, runtime, |contents| {
            summarize_safe_lane_events(contents.iter().map(String::as_str))
        })
        .await
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (limit, ctx, runtime);
        Err("safe-lane summary unavailable: memory-sqlite feature disabled".to_owned())
    }
}

pub async fn load_fast_lane_tool_batch_event_summary<R: ConversationRuntime + ?Sized>(
    limit: usize,
    ctx: &Context<'_>,
    runtime: &R,
) -> CliResult<FastLaneToolBatchEventSummary> {
    #[cfg(feature = "memory-sqlite")]
    {
        load_assistant_history_summary(limit, ctx, runtime, |contents| {
            summarize_fast_lane_tool_batch_events(contents.iter().map(String::as_str))
        })
        .await
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (limit, ctx, runtime);
        Err("fast-lane summary unavailable: memory-sqlite feature disabled".to_owned())
    }
}

pub async fn load_prompt_frame_event_summary<R: ConversationRuntime + ?Sized>(
    limit: usize,
    ctx: &Context<'_>,
    runtime: &R,
) -> CliResult<PromptFrameEventSummary> {
    #[cfg(feature = "memory-sqlite")]
    {
        load_assistant_history_summary(limit, ctx, runtime, |contents| {
            summarize_prompt_frame_events(contents.iter().map(String::as_str))
        })
        .await
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (limit, ctx, runtime);
        Err("prompt-frame summary unavailable: memory-sqlite feature disabled".to_owned())
    }
}

pub async fn load_discovery_first_event_summary<R: ConversationRuntime + ?Sized>(
    limit: usize,
    ctx: &Context<'_>,
    runtime: &R,
) -> CliResult<DiscoveryFirstEventSummary> {
    #[cfg(feature = "memory-sqlite")]
    {
        load_assistant_history_summary(limit, ctx, runtime, |contents| {
            summarize_discovery_first_events(contents.iter().map(String::as_str))
        })
        .await
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (limit, ctx, runtime);
        Err("discovery-first summary unavailable: memory-sqlite feature disabled".to_owned())
    }
}

pub(crate) async fn load_latest_turn_checkpoint_entry<R: ConversationRuntime + ?Sized>(
    limit: usize,
    ctx: &Context<'_>,
    runtime: &R,
) -> CliResult<Option<TurnCheckpointLatestEntry>> {
    #[cfg(feature = "memory-sqlite")]
    {
        Ok(load_turn_checkpoint_history_snapshot(limit, ctx, runtime)
            .await?
            .into_latest_entry())
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (limit, ctx, runtime);
        Err("turn checkpoint entry unavailable: memory-sqlite feature disabled".to_owned())
    }
}

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn load_turn_checkpoint_history_snapshot<R: ConversationRuntime + ?Sized>(
    limit: usize,
    ctx: &Context<'_>,
    runtime: &R,
) -> CliResult<TurnCheckpointHistorySnapshot> {
    let assistant_contents =
        load_assistant_contents_from_session_window(limit, ctx, runtime).await?;
    Ok(build_turn_checkpoint_history_snapshot(&assistant_contents))
}

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn load_assistant_contents_from_session_window<R: ConversationRuntime + ?Sized>(
    limit: usize,
    ctx: &Context<'_>,
    runtime: &R,
) -> CliResult<Vec<String>> {
    load_assistant_contents_from_session_window_detailed(limit, ctx, runtime)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(feature = "memory-sqlite")]
async fn load_assistant_history_summary<R, T, F>(
    limit: usize,
    ctx: &Context<'_>,
    runtime: &R,
    summarize: F,
) -> CliResult<T>
where
    R: ConversationRuntime + ?Sized,
    F: FnOnce(&[String]) -> T,
{
    let assistant_contents =
        load_assistant_contents_from_session_window(limit, ctx, runtime).await?;
    Ok(summarize(&assistant_contents))
}

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn load_assistant_contents_from_session_window_detailed<
    R: ConversationRuntime + ?Sized,
>(
    limit: usize,
    ctx: &Context<'_>,
    runtime: &R,
) -> Result<Vec<String>, AssistantHistoryLoadError> {
    Ok(runtime
        .read_session_window(limit, ctx)
        .await?
        .into_iter()
        .filter(|turn| turn.role == "assistant")
        .map(|turn| turn.content)
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
