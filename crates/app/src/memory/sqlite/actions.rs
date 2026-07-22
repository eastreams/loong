use loong_core::policy::grant::Granted;
use loong_kernel::access::memory::{
    MemoryAppendTurnAction, MemoryBackendError, MemoryReplaceTurnsAction,
    MemoryReplaceTurnsOutcome, MemorySnapshot, MemoryTranscriptAction, MemoryTurn,
    MemoryWindowAction,
};

use super::{
    MemoryRuntimeConfig, ReplaceTurnsFailure, WindowTurn, append_turn_internal,
    default_window_size, load_window_internal, replace_turns_internal, transcript_direct_paged,
};

/// Execute an authorized append against the configured SQLite memory store.
pub(in crate::memory) fn append_turn_granted(
    granted: Granted<MemoryAppendTurnAction>,
    config: &MemoryRuntimeConfig,
) -> Result<(), MemoryBackendError> {
    let action = granted.as_ref();
    append_turn_internal(action.session_id(), action.role(), action.content(), config)
        .map(|_| ())
        .map_err(|source| MemoryBackendError::Execution {
            operation: "append_turn",
            source: source.into(),
        })
}

/// Execute an authorized bounded window read without a JSON protocol roundtrip.
pub(in crate::memory) fn window_granted(
    granted: Granted<MemoryWindowAction>,
    config: &MemoryRuntimeConfig,
) -> Result<MemorySnapshot, MemoryBackendError> {
    let action = granted.as_ref();
    let hard_limit_cap = if action.allow_extended_limit() {
        512
    } else {
        128
    };
    let requested_limit = action.limit().clamp(1, hard_limit_cap);
    let limit = if action.allow_extended_limit() {
        requested_limit
    } else {
        requested_limit.min(default_window_size(config).max(1))
    };
    let window = load_window_internal(
        action.session_id(),
        limit,
        action.allow_extended_limit(),
        config,
    )
    .map_err(|source| MemoryBackendError::Execution {
        operation: "window",
        source: source.into(),
    })?;
    let turns = window
        .turns
        .into_iter()
        .map(|turn| MemoryTurn {
            role: turn.role,
            content: turn.content,
            ts: Some(turn.ts),
        })
        .collect::<Vec<_>>();
    let turn_count = window.turn_count.unwrap_or(turns.len());
    Ok(MemorySnapshot { turns, turn_count })
}

/// Execute an authorized full transcript read in bounded SQLite pages.
pub(in crate::memory) fn transcript_granted(
    granted: Granted<MemoryTranscriptAction>,
    config: &MemoryRuntimeConfig,
) -> Result<MemorySnapshot, MemoryBackendError> {
    let action = granted.as_ref();
    let page_size = action.page_size().clamp(1, 512);
    let turns = transcript_direct_paged(action.session_id(), page_size, config)
        .map_err(|source| MemoryBackendError::Execution {
            operation: "transcript",
            source: source.into(),
        })?
        .into_iter()
        .map(|turn| MemoryTurn {
            role: turn.role,
            content: turn.content,
            ts: Some(turn.ts),
        })
        .collect::<Vec<_>>();
    let turn_count = turns.len();
    Ok(MemorySnapshot { turns, turn_count })
}

/// Execute an authorized compare-and-swap transcript replacement.
pub(in crate::memory) fn replace_turns_granted(
    granted: Granted<MemoryReplaceTurnsAction>,
    config: &MemoryRuntimeConfig,
) -> Result<MemoryReplaceTurnsOutcome, MemoryBackendError> {
    let action = granted.as_ref();
    let turns = action
        .turns()
        .iter()
        .map(|turn| WindowTurn {
            role: turn.role.clone(),
            content: turn.content.clone(),
            ts: turn.ts,
        })
        .collect::<Vec<_>>();
    match replace_turns_internal(
        action.session_id(),
        &turns,
        action.expected_turn_count(),
        config,
    ) {
        Ok(_) => Ok(MemoryReplaceTurnsOutcome::Replaced),
        Err(ReplaceTurnsFailure::Conflict { .. }) => Ok(MemoryReplaceTurnsOutcome::Conflict),
        Err(ReplaceTurnsFailure::Message(source)) => Err(MemoryBackendError::Execution {
            operation: "replace_turns",
            source: source.into(),
        }),
    }
}
