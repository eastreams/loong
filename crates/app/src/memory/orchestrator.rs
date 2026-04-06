use std::path::Path;
#[cfg(test)]
use std::sync::{Mutex, MutexGuard, OnceLock};

use super::{
    BuiltinMemoryPreAssemblyExecutor, DEFAULT_MEMORY_SYSTEM_ID, MemoryContextEntry,
    MemoryPreAssemblyContext, MemoryPreAssemblyExecutor, MemoryStageFamily, MemorySystemMetadata,
    StageDiagnostics, StageEnvelope, StageOutcome, WindowTurn, builtin_pre_assembly_stage_families,
    load_prompt_context, resolve_memory_system, runtime_config::MemoryRuntimeConfig,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydratedMemoryContext {
    pub entries: Vec<MemoryContextEntry>,
    pub recent_window: Vec<WindowTurn>,
    pub diagnostics: MemoryDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryDiagnostics {
    pub system_id: String,
    pub fail_open: bool,
    pub strict_mode_requested: bool,
    pub strict_mode_active: bool,
    pub degraded: bool,
    pub derivation_error: Option<String>,
    pub retrieval_error: Option<String>,
    pub rank_error: Option<String>,
    pub recent_window_count: usize,
    pub entry_count: usize,
}

impl MemoryDiagnostics {
    pub fn normalize_system_id(raw: &str) -> Option<String> {
        super::normalize_system_id(raw)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BuiltinMemoryOrchestrator;

#[cfg(test)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryOrchestratorTestFaults {
    pub session_id: Option<String>,
    pub derivation_error: Option<String>,
    pub retrieval_error: Option<String>,
    pub rank_error: Option<String>,
}

#[cfg(test)]
static MEMORY_ORCHESTRATOR_TEST_FAULTS: OnceLock<Mutex<Option<MemoryOrchestratorTestFaults>>> =
    OnceLock::new();
#[cfg(test)]
static MEMORY_ORCHESTRATOR_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[cfg(test)]
fn memory_orchestrator_test_faults() -> &'static Mutex<Option<MemoryOrchestratorTestFaults>> {
    MEMORY_ORCHESTRATOR_TEST_FAULTS.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
fn memory_orchestrator_test_lock() -> &'static Mutex<()> {
    MEMORY_ORCHESTRATOR_TEST_LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
fn active_memory_orchestrator_test_faults() -> Option<MemoryOrchestratorTestFaults> {
    memory_orchestrator_test_faults()
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
}

#[cfg(test)]
fn matching_memory_orchestrator_test_faults(
    session_id: &str,
) -> Option<MemoryOrchestratorTestFaults> {
    active_memory_orchestrator_test_faults().filter(|faults| {
        faults
            .session_id
            .as_deref()
            .is_none_or(|expected| expected == session_id)
    })
}

#[cfg(test)]
pub struct ScopedMemoryOrchestratorTestFaults {
    _guard: MutexGuard<'static, ()>,
}

#[cfg(test)]
impl ScopedMemoryOrchestratorTestFaults {
    pub fn set(faults: MemoryOrchestratorTestFaults) -> Self {
        let guard = memory_orchestrator_test_lock()
            .lock()
            .expect("memory orchestrator test lock");
        *memory_orchestrator_test_faults()
            .lock()
            .expect("memory orchestrator test faults lock") = Some(faults);
        Self { _guard: guard }
    }
}

#[cfg(test)]
impl Drop for ScopedMemoryOrchestratorTestFaults {
    fn drop(&mut self) {
        if let Ok(mut guard) = memory_orchestrator_test_faults().lock() {
            *guard = None;
        }
    }
}

impl BuiltinMemoryOrchestrator {
    pub fn hydrate_stage_envelope(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        metadata: &MemorySystemMetadata,
    ) -> Result<StageEnvelope, String> {
        let executor = BuiltinMemoryPreAssemblyExecutor;

        hydrate_stage_envelope_with_executor(
            session_id,
            workspace_root,
            config,
            metadata,
            &executor,
        )
    }

    pub fn hydrate(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        metadata: &MemorySystemMetadata,
    ) -> Result<HydratedMemoryContext, String> {
        Ok(self
            .hydrate_stage_envelope(session_id, workspace_root, config, metadata)?
            .hydrated)
    }
}

fn hydrate_stage_envelope_with_executor(
    session_id: &str,
    workspace_root: Option<&Path>,
    config: &MemoryRuntimeConfig,
    metadata: &MemorySystemMetadata,
    executor: &dyn MemoryPreAssemblyExecutor,
) -> Result<StageEnvelope, String> {
    let recent_window = recent_window_records(session_id, config)?;
    let mut entries = load_prompt_context(session_id, config)?;
    let context = MemoryPreAssemblyContext {
        session_id,
        workspace_root,
        config,
        recent_window: recent_window.as_slice(),
    };

    let supports_retrieve =
        metadata.supports_pre_assembly_stage_family(MemoryStageFamily::Retrieve);
    let retrieval_request = if supports_retrieve {
        executor.retrieval_request(&context)
    } else {
        None
    };

    let derive = run_pre_assembly_stage(MemoryStageFamily::Derive, metadata, config, || {
        run_derivation_stage(session_id, executor, &context)
    })?;
    entries.extend(derive.records);

    let retrieve = run_pre_assembly_stage(MemoryStageFamily::Retrieve, metadata, config, || {
        run_retrieval_stage(session_id, executor, &context)
    })?;
    entries.extend(retrieve.records);

    let rank = run_rank_stage(entries, metadata, config, executor, &context)?;
    let diagnostics = vec![derive.diagnostics, retrieve.diagnostics, rank.diagnostics];
    let hydrated = HydratedMemoryContext::from_stage_parts(
        rank.records,
        recent_window,
        diagnostics.as_slice(),
        metadata.id,
        config,
    );
    let envelope = StageEnvelope {
        hydrated,
        retrieval_request,
        diagnostics,
    };

    Ok(envelope)
}

impl HydratedMemoryContext {
    fn from_stage_parts(
        entries: Vec<MemoryContextEntry>,
        recent_window: Vec<WindowTurn>,
        stage_diagnostics: &[StageDiagnostics],
        system_id: &str,
        config: &MemoryRuntimeConfig,
    ) -> Self {
        let diagnostics = MemoryDiagnostics::from_stage_diagnostics(
            config,
            &recent_window,
            &entries,
            stage_diagnostics,
            system_id,
        );

        Self {
            entries,
            recent_window,
            diagnostics,
        }
    }
}

impl MemoryDiagnostics {
    fn from_stage_diagnostics(
        config: &MemoryRuntimeConfig,
        recent_window: &[WindowTurn],
        entries: &[MemoryContextEntry],
        stage_diagnostics: &[StageDiagnostics],
        system_id: &str,
    ) -> Self {
        let derivation_error = stage_error_message(stage_diagnostics, MemoryStageFamily::Derive);
        let retrieval_error = stage_error_message(stage_diagnostics, MemoryStageFamily::Retrieve);
        let rank_error = stage_error_message(stage_diagnostics, MemoryStageFamily::Rank);
        let degraded = stage_diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.outcome,
                StageOutcome::Fallback | StageOutcome::Failed
            )
        });

        Self {
            system_id: MemoryDiagnostics::normalize_system_id(system_id)
                .unwrap_or_else(|| system_id.to_owned()),
            fail_open: config.effective_fail_open(),
            strict_mode_requested: config.strict_mode_requested(),
            strict_mode_active: config.strict_mode_active(),
            degraded,
            derivation_error,
            retrieval_error,
            rank_error,
            recent_window_count: recent_window.len(),
            entry_count: entries.len(),
        }
    }
}

struct StageRunResult {
    records: Vec<MemoryContextEntry>,
    diagnostics: StageDiagnostics,
}

fn run_pre_assembly_stage<F>(
    family: MemoryStageFamily,
    metadata: &MemorySystemMetadata,
    config: &MemoryRuntimeConfig,
    runner: F,
) -> Result<StageRunResult, String>
where
    F: FnOnce() -> Result<Vec<MemoryContextEntry>, String>,
{
    if !metadata.supports_pre_assembly_stage_family(family) {
        return Ok(StageRunResult {
            records: Vec::new(),
            diagnostics: skipped_stage_diagnostics(family, None),
        });
    }

    match runner() {
        Ok(records) => Ok(StageRunResult {
            records,
            diagnostics: StageDiagnostics::succeeded(family),
        }),
        Err(error) if config.effective_fail_open() => Ok(StageRunResult {
            records: Vec::new(),
            diagnostics: StageDiagnostics {
                family,
                outcome: StageOutcome::Fallback,
                budget_ms: None,
                elapsed_ms: None,
                fallback_activated: true,
                message: Some(error),
            },
        }),
        Err(error) => Err(format!("memory {} stage failed: {error}", family.as_str())),
    }
}

fn run_rank_stage(
    entries: Vec<MemoryContextEntry>,
    metadata: &MemorySystemMetadata,
    config: &MemoryRuntimeConfig,
    executor: &dyn MemoryPreAssemblyExecutor,
    context: &MemoryPreAssemblyContext<'_>,
) -> Result<StageRunResult, String> {
    if !metadata.supports_pre_assembly_stage_family(MemoryStageFamily::Rank) {
        return Ok(StageRunResult {
            records: entries,
            diagnostics: skipped_stage_diagnostics(MemoryStageFamily::Rank, None),
        });
    }

    #[cfg(test)]
    if let Some(error) = matching_memory_orchestrator_test_faults(context.session_id)
        .and_then(|faults| faults.rank_error)
    {
        return if config.effective_fail_open() {
            let diagnostics = StageDiagnostics {
                family: MemoryStageFamily::Rank,
                outcome: StageOutcome::Fallback,
                budget_ms: None,
                elapsed_ms: None,
                fallback_activated: true,
                message: Some(error),
            };
            let outcome = StageRunResult {
                records: entries,
                diagnostics,
            };
            Ok(outcome)
        } else {
            Err(format!("memory rank stage failed: {error}"))
        };
    }

    let fallback_entries = if config.effective_fail_open() {
        Some(entries.clone())
    } else {
        None
    };
    let rank_result = executor.rank(entries, context);

    match rank_result {
        Ok(records) => Ok(StageRunResult {
            records,
            diagnostics: StageDiagnostics::succeeded(MemoryStageFamily::Rank),
        }),
        Err(error) if config.effective_fail_open() => {
            let records = fallback_entries.unwrap_or_default();
            let diagnostics = StageDiagnostics {
                family: MemoryStageFamily::Rank,
                outcome: StageOutcome::Fallback,
                budget_ms: None,
                elapsed_ms: None,
                fallback_activated: true,
                message: Some(error),
            };
            let outcome = StageRunResult {
                records,
                diagnostics,
            };
            Ok(outcome)
        }
        Err(error) => Err(format!("memory rank stage failed: {error}")),
    }
}

fn skipped_stage_diagnostics(
    family: MemoryStageFamily,
    message: Option<String>,
) -> StageDiagnostics {
    StageDiagnostics {
        family,
        outcome: StageOutcome::Skipped,
        budget_ms: None,
        elapsed_ms: None,
        fallback_activated: false,
        message,
    }
}

pub async fn run_compact_stage(
    session_id: &str,
    workspace_root: Option<&Path>,
    config: &MemoryRuntimeConfig,
) -> Result<StageDiagnostics, String> {
    run_builtin_compact_stage(session_id, workspace_root, config).await
}

#[cfg(not(feature = "memory-sqlite"))]
async fn run_builtin_compact_stage(
    _session_id: &str,
    _workspace_root: Option<&Path>,
    _config: &MemoryRuntimeConfig,
) -> Result<StageDiagnostics, String> {
    Ok(StageDiagnostics {
        family: MemoryStageFamily::Compact,
        outcome: StageOutcome::Skipped,
        budget_ms: None,
        elapsed_ms: None,
        fallback_activated: false,
        message: None,
    })
}

#[cfg(feature = "memory-sqlite")]
async fn run_builtin_compact_stage(
    session_id: &str,
    workspace_root: Option<&Path>,
    config: &MemoryRuntimeConfig,
) -> Result<StageDiagnostics, String> {
    match super::flush_pre_compaction_durable_memory(session_id, workspace_root, config).await {
        Ok(super::durable_flush::PreCompactionDurableFlushOutcome::Flushed { .. }) => {
            Ok(StageDiagnostics::succeeded(MemoryStageFamily::Compact))
        }
        Ok(super::durable_flush::PreCompactionDurableFlushOutcome::SkippedDuplicate)
        | Ok(super::durable_flush::PreCompactionDurableFlushOutcome::SkippedMissingWorkspaceRoot)
        | Ok(super::durable_flush::PreCompactionDurableFlushOutcome::SkippedNoSummary) => {
            Ok(StageDiagnostics {
                family: MemoryStageFamily::Compact,
                outcome: StageOutcome::Skipped,
                budget_ms: None,
                elapsed_ms: None,
                fallback_activated: false,
                message: None,
            })
        }
        Err(error) if config.effective_fail_open() => Ok(StageDiagnostics {
            family: MemoryStageFamily::Compact,
            outcome: StageOutcome::Fallback,
            budget_ms: None,
            elapsed_ms: None,
            fallback_activated: true,
            message: Some(error),
        }),
        Err(error) => Err(format!("memory compact stage failed: {error}")),
    }
}

fn stage_error_message(
    stage_diagnostics: &[StageDiagnostics],
    family: MemoryStageFamily,
) -> Option<String> {
    stage_diagnostics
        .iter()
        .find(|diagnostic| diagnostic.family == family)
        .and_then(|diagnostic| match diagnostic.outcome {
            StageOutcome::Fallback | StageOutcome::Failed => diagnostic.message.clone(),
            StageOutcome::Succeeded | StageOutcome::Skipped => None,
        })
}

fn run_derivation_stage(
    session_id: &str,
    executor: &dyn MemoryPreAssemblyExecutor,
    context: &MemoryPreAssemblyContext<'_>,
) -> Result<Vec<MemoryContextEntry>, String> {
    let _ = session_id;

    #[cfg(test)]
    if let Some(error) = matching_memory_orchestrator_test_faults(session_id)
        .and_then(|faults| faults.derivation_error)
    {
        return Err(error);
    }

    executor.derive(context)
}

fn run_retrieval_stage(
    session_id: &str,
    executor: &dyn MemoryPreAssemblyExecutor,
    context: &MemoryPreAssemblyContext<'_>,
) -> Result<Vec<MemoryContextEntry>, String> {
    let _ = session_id;

    #[cfg(test)]
    if let Some(error) = matching_memory_orchestrator_test_faults(session_id)
        .and_then(|faults| faults.retrieval_error)
    {
        return Err(error);
    }

    executor.retrieve(context)
}

pub fn hydrate_memory_context(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<HydratedMemoryContext, String> {
    hydrate_memory_context_with_workspace_root(session_id, None, config)
}

pub fn hydrate_memory_context_with_workspace_root(
    session_id: &str,
    workspace_root: Option<&Path>,
    config: &MemoryRuntimeConfig,
) -> Result<HydratedMemoryContext, String> {
    Ok(hydrate_stage_envelope_with_workspace_root(session_id, workspace_root, config)?.hydrated)
}

pub fn hydrate_stage_envelope(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<StageEnvelope, String> {
    hydrate_stage_envelope_with_workspace_root(session_id, None, config)
}

pub(crate) fn hydrate_stage_envelope_with_workspace_root(
    session_id: &str,
    workspace_root: Option<&Path>,
    config: &MemoryRuntimeConfig,
) -> Result<StageEnvelope, String> {
    let selected_system_id = super::registered_memory_system_id(Some(config.selected_system_id()))
        .unwrap_or_else(|| DEFAULT_MEMORY_SYSTEM_ID.to_owned());
    let system = resolve_memory_system(Some(selected_system_id.as_str()))?;
    let metadata = system.metadata();
    let executor = system.pre_assembly_executor();

    if let Some(executor) = executor {
        return hydrate_stage_envelope_with_executor(
            session_id,
            workspace_root,
            config,
            &metadata,
            executor.as_ref(),
        );
    }

    hydrate_stage_envelope_without_execution_adapter(session_id, config, &metadata)
}

fn hydrate_stage_envelope_without_execution_adapter(
    session_id: &str,
    config: &MemoryRuntimeConfig,
    metadata: &MemorySystemMetadata,
) -> Result<StageEnvelope, String> {
    let recent_window = recent_window_records(session_id, config)?;
    let entries = recent_window
        .iter()
        .map(|turn| MemoryContextEntry {
            kind: super::MemoryContextKind::Turn,
            role: turn.role.clone(),
            content: turn.content.clone(),
        })
        .collect::<Vec<_>>();
    let diagnostics = builtin_pre_assembly_stage_families()
        .into_iter()
        .map(|family| {
            let message = metadata
                .supports_pre_assembly_stage_family(family)
                .then(|| {
                    "memory system is registered but has no pre-assembly execution adapter yet"
                        .to_owned()
                });
            skipped_stage_diagnostics(family, message)
        })
        .collect::<Vec<_>>();

    Ok(StageEnvelope {
        hydrated: HydratedMemoryContext::from_stage_parts(
            entries,
            recent_window,
            &diagnostics,
            metadata.id,
            config,
        ),
        retrieval_request: None,
        diagnostics,
    })
}

#[cfg(feature = "memory-sqlite")]
fn recent_window_records(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<WindowTurn>, String> {
    let turns = super::window_direct(session_id, config.sliding_window, config)?;
    Ok(turns
        .into_iter()
        .map(|turn| WindowTurn {
            role: turn.role,
            content: turn.content,
            ts: Some(turn.ts),
        })
        .collect())
}

#[cfg(not(feature = "memory-sqlite"))]
fn recent_window_records(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<WindowTurn>, String> {
    let _ = (session_id, config);
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{MemoryMode, MemoryProfile};
    use crate::memory::{
        DerivedMemoryKind, MemoryContextKind, MemoryScope, MemoryStageFamily, MemorySystem,
        MemorySystemCapability, MemorySystemMetadata, RECALL_FIRST_MEMORY_SYSTEM_ID, StageOutcome,
        append_turn_direct, builtin_pre_assembly_stage_families, register_memory_system,
    };

    const REGISTRY_RETRIEVE_ONLY_SYSTEM_ID: &str = "registry-retrieve-only";
    const REGISTRY_RETRIEVE_ONLY_COMPACT_SYSTEM_ID: &str = "registry-retrieve-only-compact";

    struct RegistryRetrieveOnlyMemorySystem {
        id: &'static str,
    }

    impl MemorySystem for RegistryRetrieveOnlyMemorySystem {
        fn id(&self) -> &'static str {
            self.id
        }

        fn metadata(&self) -> MemorySystemMetadata {
            MemorySystemMetadata::new(
                self.id,
                [MemorySystemCapability::PromptHydration],
                "Registry system without an execution adapter yet",
            )
            .with_supported_pre_assembly_stage_families([MemoryStageFamily::Retrieve])
        }
    }

    fn hydrated_memory_temp_dir(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}", std::process::id()))
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrated_memory_builtin_orchestrator_returns_recent_window_records() {
        let tmp = hydrated_memory_temp_dir("loongclaw-hydrated-window");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("window.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowOnly,
            mode: MemoryMode::WindowOnly,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        append_turn_direct("hydrated-window", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("hydrated-window", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("hydrated-window", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let hydrated =
            hydrate_memory_context("hydrated-window", &config).expect("hydrate memory context");

        assert_eq!(hydrated.recent_window.len(), 2);
        assert_eq!(hydrated.recent_window[0].content, "turn 2");
        assert_eq!(hydrated.recent_window[1].content, "turn 3");

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrated_memory_builtin_orchestrator_reports_deterministic_diagnostics() {
        let tmp = hydrated_memory_temp_dir("loongclaw-hydrated-diagnostics");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("diagnostics.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        let hydrated = hydrate_memory_context("hydrated-diagnostics", &config)
            .expect("hydrate memory context");

        assert_eq!(hydrated.diagnostics.system_id, "builtin");
        assert!(hydrated.diagnostics.fail_open);
        assert!(!hydrated.diagnostics.strict_mode_requested);
        assert!(!hydrated.diagnostics.strict_mode_active);
        assert!(!hydrated.diagnostics.degraded);
        assert_eq!(hydrated.diagnostics.derivation_error, None);
        assert_eq!(hydrated.diagnostics.retrieval_error, None);
        assert_eq!(hydrated.diagnostics.recent_window_count, 0);
        assert_eq!(hydrated.diagnostics.entry_count, 0);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrated_memory_builtin_orchestrator_preserves_summary_behavior() {
        let tmp = hydrated_memory_temp_dir("loongclaw-hydrated-summary");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("summary.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        append_turn_direct("hydrated-summary", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("hydrated-summary", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("hydrated-summary", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");
        append_turn_direct("hydrated-summary", "assistant", "turn 4", &config)
            .expect("append turn 4 should succeed");

        let hydrated =
            hydrate_memory_context("hydrated-summary", &config).expect("hydrate memory context");

        assert!(
            hydrated
                .entries
                .iter()
                .any(|entry| entry.kind == MemoryContextKind::Summary),
            "expected summary entry"
        );
        assert!(
            hydrated
                .entries
                .iter()
                .any(|entry| entry.content.contains("turn 1")),
            "expected summary to mention older turns"
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrated_memory_builtin_orchestrator_retrieves_cross_session_recall_hits() {
        let tmp = hydrated_memory_temp_dir("loongclaw-hydrated-cross-session-recall");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("cross-session-recall.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 8,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        append_turn_direct(
            "prior-session",
            "assistant",
            "Deployment cutoff is 17:00 Beijing time and requires a release note.",
            &config,
        )
        .expect("append prior session recall candidate");
        append_turn_direct(
            "active-session",
            "user",
            "What is the deployment cutoff for today's release?",
            &config,
        )
        .expect("append active user turn");

        let hydrated =
            hydrate_memory_context("active-session", &config).expect("hydrate memory context");

        let recalled = hydrated
            .entries
            .iter()
            .find(|entry| {
                entry.kind == MemoryContextKind::RetrievedMemory
                    && entry.content.contains("prior-session")
            })
            .expect("expected cross-session retrieved memory entry");
        assert!(
            recalled
                .content
                .contains("Deployment cutoff is 17:00 Beijing time")
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrated_memory_builtin_orchestrator_preserves_profile_behavior() {
        let tmp = hydrated_memory_temp_dir("loongclaw-hydrated-profile");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("profile.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::ProfilePlusWindow,
            mode: MemoryMode::ProfilePlusWindow,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            profile_note: Some("Imported ZeroClaw preferences".to_owned()),
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        let hydrated =
            hydrate_memory_context("hydrated-profile", &config).expect("hydrate memory context");

        assert!(
            hydrated
                .entries
                .iter()
                .any(|entry| entry.kind == MemoryContextKind::Profile),
            "expected profile entry"
        );
        assert!(
            hydrated
                .entries
                .iter()
                .any(|entry| entry.content.contains("Imported ZeroClaw preferences")),
            "expected profile note content"
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrate_stage_envelope_emits_builtin_stage_diagnostics_in_order() {
        let tmp = hydrated_memory_temp_dir("loongclaw-stage-envelope-order");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("stage-order.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        append_turn_direct("stage-order", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("stage-order", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("stage-order", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let envelope =
            hydrate_stage_envelope("stage-order", &config).expect("hydrate staged envelope");

        assert_eq!(
            envelope
                .diagnostics
                .iter()
                .map(|diag| diag.family)
                .collect::<Vec<_>>(),
            vec![
                MemoryStageFamily::Derive,
                MemoryStageFamily::Retrieve,
                MemoryStageFamily::Rank,
            ]
        );
        assert!(
            envelope
                .diagnostics
                .iter()
                .all(|diag| diag.outcome == StageOutcome::Succeeded)
        );
        assert_eq!(
            envelope
                .retrieval_request
                .as_ref()
                .map(|req| req.budget_items),
            Some(config.sliding_window)
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrate_stage_envelope_fail_open_marks_fallback_without_losing_recent_window() {
        let session_id = "stage-fail-open-derivation";
        let _faults = ScopedMemoryOrchestratorTestFaults::set(MemoryOrchestratorTestFaults {
            session_id: Some(session_id.to_owned()),
            derivation_error: Some("synthetic derivation failure".to_owned()),
            ..MemoryOrchestratorTestFaults::default()
        });
        let tmp = hydrated_memory_temp_dir("loongclaw-stage-envelope-fallback");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("stage-fallback.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowOnly,
            mode: MemoryMode::WindowOnly,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        append_turn_direct(session_id, "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct(session_id, "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct(session_id, "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let envelope = hydrate_stage_envelope(session_id, &config)
            .expect("fail-open staged hydration should succeed");

        assert_eq!(envelope.diagnostics[0].family, MemoryStageFamily::Derive);
        assert_eq!(envelope.diagnostics[0].outcome, StageOutcome::Fallback);
        assert!(envelope.diagnostics[0].fallback_activated);
        assert_eq!(
            envelope.diagnostics[0].message.as_deref(),
            Some("synthetic derivation failure")
        );
        assert_eq!(envelope.hydrated.recent_window.len(), 2);
        assert_eq!(envelope.hydrated.recent_window[0].content, "turn 2");
        assert_eq!(envelope.hydrated.recent_window[1].content, "turn 3");

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrate_stage_envelope_window_plus_summary_keeps_summary_retrieval_request() {
        let tmp = hydrated_memory_temp_dir("loongclaw-stage-envelope-window-plus-summary");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("window-plus-summary.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 4,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        let envelope = hydrate_stage_envelope("stage-window-plus-summary", &config)
            .expect("hydrate staged envelope");

        let retrieval_request = envelope
            .retrieval_request
            .expect("window-plus-summary should advertise retrieval request");
        assert_eq!(retrieval_request.budget_items, 6);
        assert_eq!(
            retrieval_request.allowed_kinds,
            vec![
                DerivedMemoryKind::Profile,
                DerivedMemoryKind::Fact,
                DerivedMemoryKind::Episode,
                DerivedMemoryKind::Procedure,
                DerivedMemoryKind::Overview,
            ]
        );
        assert_eq!(
            retrieval_request.scopes,
            vec![
                MemoryScope::Session,
                MemoryScope::Workspace,
                MemoryScope::Agent,
                MemoryScope::User,
            ]
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn retrieval_query_from_recent_window_skips_blank_latest_user_turn() {
        let recent_window = vec![
            WindowTurn {
                role: "user".to_owned(),
                content: "release rollback plan".to_owned(),
                ts: None,
            },
            WindowTurn {
                role: "assistant".to_owned(),
                content: "working on it".to_owned(),
                ts: None,
            },
            WindowTurn {
                role: "user".to_owned(),
                content: "   ".to_owned(),
                ts: None,
            },
        ];

        let query =
            crate::memory::execution::retrieval_query_from_recent_window(recent_window.as_slice())
                .expect("query fallback");

        assert_eq!(query, "release rollback plan");
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn render_cross_session_recall_block_keeps_roleless_records_neutral() {
        let hits = vec![crate::memory::CanonicalMemorySearchHit {
            record: crate::memory::CanonicalMemoryRecord {
                session_id: "workspace-session".to_owned(),
                scope: crate::memory::MemoryScope::Workspace,
                kind: crate::memory::CanonicalMemoryKind::ImportedProfile,
                role: None,
                content: "Imported release checklist with smoke tests.".to_owned(),
                metadata: serde_json::json!({
                    "source": "workspace-import"
                }),
            },
            session_turn_index: Some(2),
        }];

        let rendered = crate::memory::execution::render_cross_session_recall_block(hits.as_slice());

        assert!(
            rendered.contains("Imported release checklist with smoke tests."),
            "expected rendered recall content: {rendered}"
        );
        assert!(
            !rendered.contains("assistant: Imported release checklist with smoke tests."),
            "roleless recall should not fabricate assistant provenance: {rendered}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrate_stage_envelope_derives_retrieval_query_from_latest_user_turn() {
        let tmp = hydrated_memory_temp_dir("loongclaw-stage-envelope-retrieval-query");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("retrieval-query.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 4,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        append_turn_direct(
            "stage-retrieval-query",
            "user",
            "Find the rollback checklist for database migration",
            &config,
        )
        .expect("append retrieval query turn");

        let envelope = hydrate_stage_envelope("stage-retrieval-query", &config)
            .expect("hydrate staged envelope");
        let retrieval_request = envelope
            .retrieval_request
            .expect("expected retrieval request");
        assert_eq!(
            retrieval_request.query.as_deref(),
            Some("Find the rollback checklist for database migration")
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrate_stage_envelope_window_only_omits_summary_retrieval_request() {
        let tmp = hydrated_memory_temp_dir("loongclaw-stage-envelope-window-only");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("window-only.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowOnly,
            mode: MemoryMode::WindowOnly,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 4,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        let envelope =
            hydrate_stage_envelope("stage-window-only", &config).expect("hydrate staged envelope");

        assert_eq!(envelope.retrieval_request, None);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrate_stage_envelope_profile_plus_window_omits_summary_retrieval_request() {
        let tmp = hydrated_memory_temp_dir("loongclaw-stage-envelope-profile-plus-window");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("profile-plus-window.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::ProfilePlusWindow,
            mode: MemoryMode::ProfilePlusWindow,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 4,
            profile_note: Some("Imported ZeroClaw preferences".to_owned()),
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        let envelope = hydrate_stage_envelope("stage-profile-plus-window", &config)
            .expect("hydrate staged envelope");

        assert_eq!(envelope.retrieval_request, None);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn registry_selected_system_skips_builtin_pre_assembly_execution() {
        register_memory_system(REGISTRY_RETRIEVE_ONLY_SYSTEM_ID, || {
            Box::new(RegistryRetrieveOnlyMemorySystem {
                id: REGISTRY_RETRIEVE_ONLY_SYSTEM_ID,
            })
        })
        .expect("register registry-selected memory system");

        let tmp = hydrated_memory_temp_dir("loongclaw-stage-envelope-registry-selected");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("registry-selected.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let mut config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };
        config.resolved_system_id = Some(REGISTRY_RETRIEVE_ONLY_SYSTEM_ID.to_owned());

        append_turn_direct("registry-selected", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("registry-selected", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("registry-selected", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let envelope = hydrate_stage_envelope("registry-selected", &config)
            .expect("hydrate staged envelope for registry-selected system");

        assert_eq!(
            envelope.hydrated.diagnostics.system_id,
            REGISTRY_RETRIEVE_ONLY_SYSTEM_ID
        );
        assert_eq!(envelope.hydrated.recent_window.len(), 2);
        assert_eq!(
            envelope
                .hydrated
                .entries
                .iter()
                .filter(|entry| entry.kind == MemoryContextKind::Turn)
                .map(|entry| (entry.role.as_str(), entry.content.as_str()))
                .collect::<Vec<_>>(),
            vec![("assistant", "turn 2"), ("user", "turn 3")]
        );
        assert_eq!(envelope.retrieval_request, None);
        assert_eq!(
            envelope
                .diagnostics
                .iter()
                .map(|diag| (diag.family, diag.outcome))
                .collect::<Vec<_>>(),
            builtin_pre_assembly_stage_families()
                .into_iter()
                .map(|family| (family, StageOutcome::Skipped))
                .collect::<Vec<_>>()
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn recall_first_selected_system_executes_real_pre_assembly_pipeline() {
        let tmp = hydrated_memory_temp_dir("loongclaw-stage-envelope-recall-first");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("recall-first.sqlite3");
        let memory_file = tmp.join("MEMORY.md");
        let _ = std::fs::remove_file(&db_path);

        std::fs::write(
            &memory_file,
            "Release checklist includes rollback verification and smoke tests.",
        )
        .expect("write durable recall file");

        let mut config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };
        config.resolved_system_id = Some(RECALL_FIRST_MEMORY_SYSTEM_ID.to_owned());

        append_turn_direct("recall-first", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("recall-first", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("recall-first", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let envelope = hydrate_stage_envelope_with_workspace_root(
            "recall-first",
            Some(tmp.as_path()),
            &config,
        )
        .expect("hydrate staged envelope for recall-first");

        assert_eq!(
            envelope.hydrated.diagnostics.system_id,
            RECALL_FIRST_MEMORY_SYSTEM_ID
        );
        assert_eq!(
            envelope
                .diagnostics
                .iter()
                .map(|diag| (diag.family, diag.outcome))
                .collect::<Vec<_>>(),
            vec![
                (MemoryStageFamily::Derive, StageOutcome::Skipped),
                (MemoryStageFamily::Retrieve, StageOutcome::Succeeded),
                (MemoryStageFamily::Rank, StageOutcome::Succeeded),
            ]
        );

        let ranked_kinds = envelope
            .hydrated
            .entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>();
        assert_eq!(
            ranked_kinds,
            vec![
                MemoryContextKind::RetrievedMemory,
                MemoryContextKind::Turn,
                MemoryContextKind::Turn,
            ]
        );
        assert!(
            envelope.hydrated.entries[0]
                .content
                .contains("## Advisory Durable Recall"),
            "expected recall-first system to prioritize durable recall content"
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&memory_file);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn unknown_registry_selected_system_falls_back_to_builtin_hydration() {
        let tmp = hydrated_memory_temp_dir("loongclaw-stage-envelope-unknown-selected");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("unknown-selected.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let mut config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };
        config.resolved_system_id = Some("lucid".to_owned());

        append_turn_direct("unknown-selected", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("unknown-selected", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("unknown-selected", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let envelope = hydrate_stage_envelope("unknown-selected", &config)
            .expect("unknown selected system should fall back to builtin");

        assert_eq!(
            envelope.hydrated.diagnostics.system_id,
            DEFAULT_MEMORY_SYSTEM_ID
        );
        assert!(
            envelope
                .hydrated
                .entries
                .iter()
                .any(|entry| entry.kind == MemoryContextKind::Summary),
            "builtin summary projection should remain available after fallback"
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn compact_stage_emits_succeeded_diagnostics_when_durable_flush_runs() {
        let tmp = hydrated_memory_temp_dir("loongclaw-compact-stage-succeeded");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("compact-stage.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        append_turn_direct("compact-stage-succeeded", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("compact-stage-succeeded", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("compact-stage-succeeded", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let diagnostics =
            run_compact_stage("compact-stage-succeeded", Some(tmp.as_path()), &config)
                .await
                .expect("run compact stage");

        assert_eq!(diagnostics.family, MemoryStageFamily::Compact);
        assert_eq!(diagnostics.outcome, StageOutcome::Succeeded);
        assert!(!diagnostics.fallback_activated);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn compact_stage_skips_when_workspace_root_is_absent() {
        let tmp = hydrated_memory_temp_dir("loongclaw-compact-stage-skipped");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("compact-stage-skipped.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        append_turn_direct("compact-stage-skipped", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("compact-stage-skipped", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");

        let diagnostics = run_compact_stage("compact-stage-skipped", None, &config)
            .await
            .expect("run compact stage");

        assert_eq!(diagnostics.family, MemoryStageFamily::Compact);
        assert_eq!(diagnostics.outcome, StageOutcome::Skipped);
        assert!(!diagnostics.fallback_activated);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn compact_stage_skips_when_durable_flush_is_duplicate() {
        let tmp = hydrated_memory_temp_dir("loongclaw-compact-stage-duplicate");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("compact-stage-duplicate.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        append_turn_direct("compact-stage-duplicate", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("compact-stage-duplicate", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("compact-stage-duplicate", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        run_compact_stage("compact-stage-duplicate", Some(tmp.as_path()), &config)
            .await
            .expect("first compact stage run");

        let diagnostics =
            run_compact_stage("compact-stage-duplicate", Some(tmp.as_path()), &config)
                .await
                .expect("second compact stage run");

        assert_eq!(diagnostics.family, MemoryStageFamily::Compact);
        assert_eq!(diagnostics.outcome, StageOutcome::Skipped);
        assert!(!diagnostics.fallback_activated);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn compact_stage_remains_runtime_owned_for_registry_selected_system_without_executor() {
        register_memory_system(REGISTRY_RETRIEVE_ONLY_COMPACT_SYSTEM_ID, || {
            Box::new(RegistryRetrieveOnlyMemorySystem {
                id: REGISTRY_RETRIEVE_ONLY_COMPACT_SYSTEM_ID,
            })
        })
        .expect("register registry-selected memory system");

        let tmp = hydrated_memory_temp_dir("loongclaw-compact-stage-registry-selected");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("compact-stage-registry-selected.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let mut config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };
        config.resolved_system_id = Some(REGISTRY_RETRIEVE_ONLY_COMPACT_SYSTEM_ID.to_owned());

        append_turn_direct("compact-stage-registry-selected", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct(
            "compact-stage-registry-selected",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct("compact-stage-registry-selected", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let diagnostics = run_compact_stage(
            "compact-stage-registry-selected",
            Some(tmp.as_path()),
            &config,
        )
        .await
        .expect("run compact stage");

        assert_eq!(diagnostics.family, MemoryStageFamily::Compact);
        assert_eq!(diagnostics.outcome, StageOutcome::Succeeded);
        assert!(!diagnostics.fallback_activated);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn compact_stage_remains_runtime_owned_for_recall_first_selected_system() {
        let tmp = hydrated_memory_temp_dir("loongclaw-compact-stage-recall-first");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("compact-stage-recall-first.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let mut config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };
        config.resolved_system_id = Some(RECALL_FIRST_MEMORY_SYSTEM_ID.to_owned());

        append_turn_direct("compact-stage-recall-first", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("compact-stage-recall-first", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("compact-stage-recall-first", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let diagnostics =
            run_compact_stage("compact-stage-recall-first", Some(tmp.as_path()), &config)
                .await
                .expect("run compact stage");

        assert_eq!(diagnostics.family, MemoryStageFamily::Compact);
        assert_eq!(diagnostics.outcome, StageOutcome::Succeeded);
        assert!(!diagnostics.fallback_activated);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn fail_open_memory_derivation_failure_keeps_recent_window_behavior() {
        let session_id = "fail-open-derivation";
        let _faults = ScopedMemoryOrchestratorTestFaults::set(MemoryOrchestratorTestFaults {
            session_id: Some(session_id.to_owned()),
            derivation_error: Some("simulated derivation failure".to_owned()),
            ..MemoryOrchestratorTestFaults::default()
        });
        let tmp = hydrated_memory_temp_dir("loongclaw-fail-open-derivation");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("derivation.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowOnly,
            mode: MemoryMode::WindowOnly,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        append_turn_direct(session_id, "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct(session_id, "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct(session_id, "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let hydrated = hydrate_memory_context(session_id, &config)
            .expect("fail-open derivation should preserve hydration");

        let turn_entries = hydrated
            .entries
            .iter()
            .filter(|entry| entry.kind == MemoryContextKind::Turn)
            .collect::<Vec<_>>();
        assert_eq!(turn_entries.len(), 2);
        assert_eq!(turn_entries[0].content, "turn 2");
        assert_eq!(turn_entries[1].content, "turn 3");
        assert_eq!(
            hydrated.diagnostics.derivation_error.as_deref(),
            Some("simulated derivation failure")
        );
        assert!(hydrated.diagnostics.degraded);
        assert!(hydrated.diagnostics.fail_open);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn fail_open_memory_retrieval_failure_keeps_recent_window_behavior() {
        let session_id = "fail-open-retrieval";
        let _faults = ScopedMemoryOrchestratorTestFaults::set(MemoryOrchestratorTestFaults {
            session_id: Some(session_id.to_owned()),
            retrieval_error: Some("simulated retrieval failure".to_owned()),
            ..MemoryOrchestratorTestFaults::default()
        });
        let tmp = hydrated_memory_temp_dir("loongclaw-fail-open-retrieval");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("retrieval.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowOnly,
            mode: MemoryMode::WindowOnly,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        append_turn_direct(session_id, "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct(session_id, "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct(session_id, "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let hydrated = hydrate_memory_context(session_id, &config)
            .expect("fail-open retrieval should preserve hydration");

        let turn_entries = hydrated
            .entries
            .iter()
            .filter(|entry| entry.kind == MemoryContextKind::Turn)
            .collect::<Vec<_>>();
        assert_eq!(turn_entries.len(), 2);
        assert_eq!(turn_entries[0].content, "turn 2");
        assert_eq!(turn_entries[1].content, "turn 3");
        assert_eq!(
            hydrated.diagnostics.retrieval_error.as_deref(),
            Some("simulated retrieval failure")
        );
        assert!(hydrated.diagnostics.degraded);
        assert!(hydrated.diagnostics.fail_open);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn fail_open_memory_rank_failure_keeps_recent_window_behavior() {
        let session_id = "fail-open-rank";
        let _faults = ScopedMemoryOrchestratorTestFaults::set(MemoryOrchestratorTestFaults {
            session_id: Some(session_id.to_owned()),
            rank_error: Some("simulated rank failure".to_owned()),
            ..MemoryOrchestratorTestFaults::default()
        });
        let tmp = hydrated_memory_temp_dir("loongclaw-fail-open-rank");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("rank.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowOnly,
            mode: MemoryMode::WindowOnly,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };

        append_turn_direct(session_id, "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct(session_id, "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct(session_id, "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let hydrated = hydrate_memory_context(session_id, &config)
            .expect("fail-open rank should preserve hydration");

        let turn_entries = hydrated
            .entries
            .iter()
            .filter(|entry| entry.kind == MemoryContextKind::Turn)
            .collect::<Vec<_>>();
        assert_eq!(turn_entries.len(), 2);
        assert_eq!(turn_entries[0].content, "turn 2");
        assert_eq!(turn_entries[1].content, "turn 3");
        assert_eq!(
            hydrated.diagnostics.rank_error.as_deref(),
            Some("simulated rank failure")
        );
        assert!(hydrated.diagnostics.degraded);
        assert!(hydrated.diagnostics.fail_open);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn fail_open_memory_strict_mode_remains_reserved_and_disabled_by_default() {
        let session_id = "fail-open-strict-reserved";
        let _faults = ScopedMemoryOrchestratorTestFaults::set(MemoryOrchestratorTestFaults {
            session_id: Some(session_id.to_owned()),
            derivation_error: Some("strict mode should stay disabled".to_owned()),
            ..MemoryOrchestratorTestFaults::default()
        });
        let tmp = hydrated_memory_temp_dir("loongclaw-fail-open-strict-reserved");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("strict-reserved.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let mut config = crate::memory::runtime_config::MemoryRuntimeConfig {
            profile: MemoryProfile::WindowOnly,
            mode: MemoryMode::WindowOnly,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..crate::memory::runtime_config::MemoryRuntimeConfig::default()
        };
        config.fail_open = false;

        append_turn_direct(session_id, "assistant", "turn 1", &config)
            .expect("append turn should succeed");

        let hydrated = hydrate_memory_context(session_id, &config)
            .expect("strict mode should remain reserved and disabled");

        assert!(hydrated.diagnostics.strict_mode_requested);
        assert!(!hydrated.diagnostics.strict_mode_active);
        assert!(hydrated.diagnostics.fail_open);
        assert_eq!(
            hydrated.diagnostics.derivation_error.as_deref(),
            Some("strict mode should stay disabled")
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }
}
