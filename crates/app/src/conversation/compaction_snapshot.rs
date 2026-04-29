#[cfg(feature = "memory-sqlite")]
use std::collections::BTreeSet;

#[cfg(feature = "memory-sqlite")]
use loong_contracts::Capability;
#[cfg(feature = "memory-sqlite")]
use serde_json::json;

#[cfg(feature = "memory-sqlite")]
use crate::memory;
#[cfg(feature = "memory-sqlite")]
use crate::{CliResult, KernelContext};

#[cfg(feature = "memory-sqlite")]
const MAX_COMPACTION_WINDOW_TURNS: usize = 512;
#[cfg(feature = "memory-sqlite")]
const DEFAULT_COMPACTION_TRANSCRIPT_PAGE_SIZE: usize = 256;

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompactionSessionSnapshot {
    pub(crate) turns: Vec<memory::WindowTurn>,
    pub(crate) turn_count: usize,
}

#[cfg(feature = "memory-sqlite")]
impl CompactionSessionSnapshot {
    fn from_memory_core_payload(payload: &serde_json::Value) -> Self {
        let turns = memory::decode_window_turns(payload);
        let turn_count = memory::decode_window_turn_count(payload).unwrap_or(turns.len());
        Self { turns, turn_count }
    }

    fn is_complete(&self) -> bool {
        self.turns.len() >= self.turn_count
    }
}

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn load_compaction_session_snapshot(
    session_id: &str,
    kernel_ctx: &KernelContext,
) -> CliResult<CompactionSessionSnapshot> {
    let window_snapshot = load_compaction_window_snapshot(session_id, kernel_ctx).await?;
    if window_snapshot.is_complete() {
        return Ok(window_snapshot);
    }

    let transcript_snapshot = load_compaction_transcript_snapshot(session_id, kernel_ctx).await?;
    if !transcript_snapshot.is_complete()
        || transcript_snapshot.turn_count < window_snapshot.turn_count
    {
        return Err(format!(
            "load compaction transcript via kernel returned incomplete snapshot: window_turn_count={} transcript_turn_count={} transcript_len={}",
            window_snapshot.turn_count,
            transcript_snapshot.turn_count,
            transcript_snapshot.turns.len()
        ));
    }

    Ok(transcript_snapshot)
}

#[cfg(feature = "memory-sqlite")]
async fn load_compaction_window_snapshot(
    session_id: &str,
    kernel_ctx: &KernelContext,
) -> CliResult<CompactionSessionSnapshot> {
    let mut request = memory::build_window_request(session_id, MAX_COMPACTION_WINDOW_TURNS);
    let Some(payload) = request.payload.as_object_mut() else {
        return Err("load compaction window via kernel built a non-object payload".to_owned());
    };
    payload.insert("allow_extended_limit".to_owned(), json!(true));
    let caps = BTreeSet::from([Capability::MemoryRead]);
    let outcome = kernel_ctx
        .kernel
        .execute_memory_core(
            kernel_ctx.pack_id(),
            &kernel_ctx.token,
            &caps,
            None,
            request,
        )
        .await
        .map_err(|error| format!("load compaction window via kernel failed: {error}"))?;

    if outcome.status != "ok" {
        return Err(format!(
            "load compaction window via kernel returned non-ok status: {}",
            outcome.status
        ));
    }

    Ok(CompactionSessionSnapshot::from_memory_core_payload(
        &outcome.payload,
    ))
}

#[cfg(feature = "memory-sqlite")]
async fn load_compaction_transcript_snapshot(
    session_id: &str,
    kernel_ctx: &KernelContext,
) -> CliResult<CompactionSessionSnapshot> {
    let request =
        memory::build_transcript_request(session_id, DEFAULT_COMPACTION_TRANSCRIPT_PAGE_SIZE);
    let caps = BTreeSet::from([Capability::MemoryRead]);
    let outcome = kernel_ctx
        .kernel
        .execute_memory_core(
            kernel_ctx.pack_id(),
            &kernel_ctx.token,
            &caps,
            None,
            request,
        )
        .await
        .map_err(|error| format!("load compaction transcript via kernel failed: {error}"))?;

    if outcome.status != "ok" {
        return Err(format!(
            "load compaction transcript via kernel returned non-ok status: {}",
            outcome.status
        ));
    }

    Ok(CompactionSessionSnapshot::from_memory_core_payload(
        &outcome.payload,
    ))
}
