#[cfg(feature = "memory-sqlite")]
use crate::memory;
#[cfg(feature = "memory-sqlite")]
use crate::{CliResult, Context};

#[cfg(feature = "memory-sqlite")]
use super::context_engine::DefaultContextEngine;

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
    fn from_memory_snapshot(snapshot: loong_kernel::access::memory::MemorySnapshot) -> Self {
        let turns = snapshot
            .turns
            .into_iter()
            .map(|turn| memory::WindowTurn {
                role: turn.role,
                content: turn.content,
                ts: turn.ts,
            })
            .collect();
        let turn_count = snapshot.turn_count;
        Self { turns, turn_count }
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.turns.len() >= self.turn_count
    }
}

#[cfg(feature = "memory-sqlite")]
impl DefaultContextEngine {
    pub(super) async fn load_compaction_session_snapshot(
        &self,
        context: &Context<'_>,
    ) -> CliResult<CompactionSessionSnapshot> {
        let window_snapshot = self.load_compaction_window_snapshot(context).await?;
        if window_snapshot.is_complete() {
            return Ok(window_snapshot);
        }

        let transcript_snapshot = match self.load_compaction_transcript_snapshot(context).await {
            Ok(snapshot) => snapshot,
            Err(_error) => return Ok(window_snapshot),
        };
        if !transcript_snapshot.is_complete()
            || transcript_snapshot.turn_count < window_snapshot.turn_count
        {
            return Ok(window_snapshot);
        }

        Ok(transcript_snapshot)
    }

    async fn load_compaction_window_snapshot(
        &self,
        context: &Context<'_>,
    ) -> CliResult<CompactionSessionSnapshot> {
        let snapshot = context
            .access()
            .memory()
            .window(MAX_COMPACTION_WINDOW_TURNS, true)
            .await
            .map_err(|error| format!("load compaction window failed: {error}"))?;

        Ok(CompactionSessionSnapshot::from_memory_snapshot(snapshot))
    }

    async fn load_compaction_transcript_snapshot(
        &self,
        context: &Context<'_>,
    ) -> CliResult<CompactionSessionSnapshot> {
        let snapshot = context
            .access()
            .memory()
            .transcript(DEFAULT_COMPACTION_TRANSCRIPT_PAGE_SIZE)
            .await
            .map_err(|error| format!("load compaction transcript failed: {error}"))?;

        Ok(CompactionSessionSnapshot::from_memory_snapshot(snapshot))
    }
}
