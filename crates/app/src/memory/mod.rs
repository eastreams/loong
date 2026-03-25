#[cfg(feature = "memory-sqlite")]
use std::path::PathBuf;
#[cfg(test)]
use std::{
    sync::{Mutex, OnceLock},
    thread::ThreadId,
};

use loongclaw_contracts::{MemoryCoreOutcome, MemoryCoreRequest};
use serde_json::json;

use crate::config::MemoryBackendKind;
use crate::runtime_identity;

mod canonical;
mod context;
#[cfg(feature = "memory-sqlite")]
mod durable_flush;
mod durable_recall;
mod kernel_adapter;
mod orchestrator;
mod protocol;
pub mod runtime_config;
#[cfg(feature = "memory-sqlite")]
mod sqlite;
mod stage;
mod system;
mod system_registry;
#[cfg(test)]
mod tests;
mod workspace_files;

pub use canonical::{
    CANONICAL_MEMORY_RECORD_TYPE, CanonicalMemoryKind, CanonicalMemoryRecord,
    INTERNAL_PERSISTED_RECORD_MARKER, MemoryScope, build_conversation_event_content,
    build_tool_decision_content, build_tool_outcome_content,
    canonical_memory_record_from_persisted_turn,
};
pub use context::load_prompt_context;
#[cfg(feature = "memory-sqlite")]
pub(crate) use durable_flush::flush_pre_compaction_durable_memory;
pub(crate) use durable_recall::load_durable_recall_entries;
pub use kernel_adapter::MvpMemoryAdapter;
pub(crate) use orchestrator::run_compact_stage;
pub use orchestrator::{
    BuiltinMemoryOrchestrator, HydratedMemoryContext, MemoryDiagnostics, hydrate_memory_context,
    hydrate_memory_context_with_workspace_root, hydrate_stage_envelope,
};
#[cfg(test)]
pub use orchestrator::{MemoryOrchestratorTestFaults, ScopedMemoryOrchestratorTestFaults};
pub use protocol::{
    MEMORY_OP_APPEND_TURN, MEMORY_OP_CLEAR_SESSION, MEMORY_OP_READ_CONTEXT,
    MEMORY_OP_READ_STAGE_ENVELOPE, MEMORY_OP_REPLACE_TURNS, MEMORY_OP_WINDOW, MemoryContextEntry,
    MemoryContextKind, WindowTurn, build_append_turn_request, build_read_context_request,
    build_read_stage_envelope_request, build_read_stage_envelope_request_with_workspace_root,
    build_replace_turns_request, build_replace_turns_request_with_expectation,
    build_window_request, decode_memory_context_entries, decode_stage_envelope,
    decode_window_turn_count, decode_window_turns, encode_stage_envelope_payload,
};
#[cfg(feature = "memory-sqlite")]
pub use sqlite::{ConversationTurn, SqliteBootstrapDiagnostics, SqliteContextLoadDiagnostics};
pub use stage::{
    DerivedMemoryKind, MemoryRetrievalRequest, MemoryStageFamily, StageDiagnostics, StageEnvelope,
    StageOutcome, builtin_post_turn_stage_families, builtin_pre_assembly_stage_families,
};
pub use system::{
    BuiltinMemorySystem, DEFAULT_MEMORY_SYSTEM_ID, MEMORY_SYSTEM_API_VERSION, MemorySystem,
    MemorySystemCapability, MemorySystemMetadata,
};
pub use system_registry::{
    MEMORY_SYSTEM_ENV, MemorySystemPolicySnapshot, MemorySystemRuntimeSnapshot,
    MemorySystemSelection, MemorySystemSelectionSource, collect_memory_system_runtime_snapshot,
    describe_memory_system, list_memory_system_ids, list_memory_system_metadata,
    memory_system_id_from_env, register_memory_system, resolve_memory_system,
    resolve_memory_system_selection, supported_memory_system_kind_from_env,
};
pub(crate) use system_registry::{
    registered_memory_system_id, registered_memory_system_id_from_env,
};
pub(crate) use workspace_files::{
    WorkspaceMemoryDocumentKind, WorkspaceMemoryDocumentLocation,
    collect_workspace_memory_document_locations,
};

pub(crate) fn normalize_system_id(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

pub fn execute_memory_core(request: MemoryCoreRequest) -> Result<MemoryCoreOutcome, String> {
    execute_memory_core_with_config(request, runtime_config::get_memory_runtime_config())
}

pub fn execute_memory_core_with_config(
    request: MemoryCoreRequest,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    #[cfg(test)]
    test_support::record_core_dispatch();

    match config.backend {
        MemoryBackendKind::Sqlite => match request.operation.as_str() {
            MEMORY_OP_APPEND_TURN => append_turn(request, config),
            MEMORY_OP_WINDOW => load_window(request, config),
            MEMORY_OP_CLEAR_SESSION => clear_session(request, config),
            MEMORY_OP_READ_CONTEXT => context::read_context(request, config),
            MEMORY_OP_REPLACE_TURNS => replace_turns(request, config),
            MEMORY_OP_READ_STAGE_ENVELOPE => context::read_stage_envelope(request, config),
            _ => Ok(MemoryCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "adapter": "kv-core",
                    "operation": request.operation,
                    "payload": request.payload,
                }),
            }),
        },
    }
}

fn append_turn(
    request: MemoryCoreRequest,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (request, config);
        return Err(
            "sqlite memory is disabled in this build (enable feature `memory-sqlite`)".to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        sqlite::append_turn(request, config)
    }
}

fn load_window(
    request: MemoryCoreRequest,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (request, config);
        return Err(
            "sqlite memory is disabled in this build (enable feature `memory-sqlite`)".to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        sqlite::load_window(request, config)
    }
}

fn clear_session(
    request: MemoryCoreRequest,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (request, config);
        return Err(
            "sqlite memory is disabled in this build (enable feature `memory-sqlite`)".to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        sqlite::clear_session(request, config)
    }
}

fn replace_turns(
    request: MemoryCoreRequest,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (request, config);
        return Err(
            "sqlite memory is disabled in this build (enable feature `memory-sqlite`)".to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        sqlite::replace_turns(request, config)
    }
}

#[cfg(feature = "memory-sqlite")]
pub fn append_turn_direct(
    session_id: &str,
    role: &str,
    content: &str,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<(), String> {
    sqlite::append_turn_direct(session_id, role, content, config)
}

#[cfg(feature = "memory-sqlite")]
#[cfg(test)]
pub fn replace_session_turns_direct(
    session_id: &str,
    turns: &[WindowTurn],
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<(), String> {
    sqlite::replace_session_turns_direct(session_id, turns, config)
}

#[cfg(feature = "memory-sqlite")]
pub fn window_direct(
    session_id: &str,
    limit: usize,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<Vec<ConversationTurn>, String> {
    sqlite::window_direct(session_id, limit, config)
}

#[cfg(feature = "memory-sqlite")]
pub fn window_direct_extended(
    session_id: &str,
    limit: usize,
) -> Result<Vec<ConversationTurn>, String> {
    sqlite::window_direct_with_options(
        session_id,
        limit,
        true,
        runtime_config::get_memory_runtime_config(),
    )
}

#[cfg(feature = "memory-sqlite")]
pub fn ensure_memory_db_ready(
    path: Option<PathBuf>,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<PathBuf, String> {
    sqlite::ensure_memory_db_ready(path, config)
}

#[cfg(feature = "memory-sqlite")]
pub fn ensure_memory_db_ready_with_diagnostics(
    path: Option<PathBuf>,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<(PathBuf, SqliteBootstrapDiagnostics), String> {
    sqlite::ensure_memory_db_ready_with_diagnostics(path, config)
}

#[cfg(feature = "memory-sqlite")]
pub fn load_prompt_context_with_diagnostics(
    session_id: &str,
    config: &runtime_config::MemoryRuntimeConfig,
) -> Result<(Vec<MemoryContextEntry>, SqliteContextLoadDiagnostics), String> {
    let mut profile_entry = None;
    let profile_section =
        runtime_identity::render_session_profile_section(config.profile_note.as_deref());
    if matches!(config.mode, crate::config::MemoryMode::ProfilePlusWindow)
        && let Some(profile_section) = profile_section
    {
        profile_entry = Some(MemoryContextEntry {
            kind: MemoryContextKind::Profile,
            role: "system".to_owned(),
            content: profile_section,
        });
    }

    let (snapshot, diagnostics) =
        sqlite::load_context_snapshot_with_diagnostics(session_id, config)?;
    let mut entries = Vec::with_capacity(
        snapshot.window_turns.len()
            + usize::from(profile_entry.is_some())
            + usize::from(snapshot.summary_body.is_some()),
    );
    if let Some(profile) = profile_entry.take() {
        entries.push(profile);
    }
    if matches!(config.mode, crate::config::MemoryMode::WindowPlusSummary)
        && let Some(summary) = snapshot
            .summary_body
            .as_deref()
            .and_then(sqlite::format_summary_block)
    {
        entries.push(MemoryContextEntry {
            kind: MemoryContextKind::Summary,
            role: "system".to_owned(),
            content: summary,
        });
    }
    for turn in snapshot.window_turns {
        entries.push(MemoryContextEntry {
            kind: MemoryContextKind::Turn,
            role: turn.role,
            content: turn.content,
        });
    }

    Ok((entries, diagnostics))
}

#[cfg(feature = "memory-sqlite")]
pub fn drop_cached_sqlite_runtime(path: &std::path::Path) -> Result<bool, String> {
    sqlite::drop_cached_sqlite_runtime(path)
}

#[cfg(test)]
mod test_support {
    use super::*;

    #[derive(Default)]
    struct CoreDispatchCapture {
        active_thread: Option<ThreadId>,
        count: usize,
    }

    fn core_dispatch_capture() -> &'static Mutex<CoreDispatchCapture> {
        static CORE_DISPATCH_CAPTURE: OnceLock<Mutex<CoreDispatchCapture>> = OnceLock::new();
        CORE_DISPATCH_CAPTURE.get_or_init(|| Mutex::new(CoreDispatchCapture::default()))
    }

    fn lock_capture() -> std::sync::MutexGuard<'static, CoreDispatchCapture> {
        core_dispatch_capture()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn record_core_dispatch() {
        let current_thread = std::thread::current().id();
        let mut capture = lock_capture();
        if capture.active_thread == Some(current_thread) {
            capture.count += 1;
        }
    }

    pub(super) fn begin_core_dispatch_capture() {
        let current_thread = std::thread::current().id();
        let mut capture = lock_capture();
        capture.active_thread = Some(current_thread);
        capture.count = 0;
    }

    pub(super) fn core_dispatch_count() -> usize {
        lock_capture().count
    }

    pub(super) fn end_core_dispatch_capture() {
        let mut capture = lock_capture();
        capture.active_thread = None;
        capture.count = 0;
    }
}
