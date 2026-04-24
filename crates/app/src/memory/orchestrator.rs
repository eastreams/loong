use std::path::Path;
#[cfg(test)]
use std::sync::{Mutex, MutexGuard, OnceLock};

use super::{
    MemoryContextEntry, MemoryStageFamily, MemorySystem, MemorySystemMetadata, StageDiagnostics,
    StageEnvelope, StageOutcome, WindowTurn, load_prompt_context, resolve_memory_system_runtime,
    runtime_config::MemoryRuntimeConfig,
};
use crate::memory::stage::MemoryRetrievalPlanResult;

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
        system: &dyn MemorySystem,
        metadata: &MemorySystemMetadata,
    ) -> Result<StageEnvelope, String> {
        let recent_window = recent_window_records(session_id, config)?;
        let mut entries = load_prompt_context(session_id, config)?;
        let retrieval_plan =
            system.build_retrieval_plan_result(session_id, workspace_root, config, &recent_window);

        let derive = run_pre_assembly_stage(MemoryStageFamily::Derive, metadata, config, || {
            run_derivation_stage(system, session_id, config, &recent_window)
        })?;
        entries.extend(derive.records);

        let retrieve =
            run_pre_assembly_stage(MemoryStageFamily::Retrieve, metadata, config, || {
                run_retrieval_stage(
                    system,
                    session_id,
                    retrieval_plan.as_ref(),
                    workspace_root,
                    config,
                    &recent_window,
                )
            })?;
        let mut retrieve = retrieve;
        annotate_retrieval_diagnostics_from_plan_result(
            &mut retrieve.diagnostics,
            retrieval_plan.as_ref(),
        );
        entries.extend(retrieve.records);

        let (retrieval_request, retrieval_planner_snapshot) = retrieval_plan
            .map(MemoryRetrievalPlanResult::into_parts)
            .map(|(request, planner_snapshot)| (Some(request), Some(planner_snapshot)))
            .unwrap_or((None, None));
        let rank = run_rank_stage(system, session_id, entries, metadata, config)?;
        let diagnostics = vec![derive.diagnostics, retrieve.diagnostics, rank.diagnostics];

        Ok(StageEnvelope {
            hydrated: HydratedMemoryContext::from_stage_parts(
                rank.records,
                recent_window,
                diagnostics.as_slice(),
                metadata.id,
                config,
            ),
            retrieval_request,
            retrieval_planner_snapshot,
            diagnostics,
        })
    }

    pub fn hydrate(
        &self,
        session_id: &str,
        workspace_root: Option<&Path>,
        config: &MemoryRuntimeConfig,
        system: &dyn MemorySystem,
        metadata: &MemorySystemMetadata,
    ) -> Result<HydratedMemoryContext, String> {
        Ok(self
            .hydrate_stage_envelope(session_id, workspace_root, config, system, metadata)?
            .hydrated)
    }
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

fn missing_execution_adapter_message(family: MemoryStageFamily) -> String {
    format!(
        "memory system declares `{}` stage support but provides no execution adapter",
        family.as_str()
    )
}

fn run_pre_assembly_stage<F>(
    family: MemoryStageFamily,
    metadata: &MemorySystemMetadata,
    config: &MemoryRuntimeConfig,
    runner: F,
) -> Result<StageRunResult, String>
where
    F: FnOnce() -> Result<Option<Vec<MemoryContextEntry>>, String>,
{
    if !metadata.supports_pre_assembly_stage_family(family) {
        return Ok(StageRunResult {
            records: Vec::new(),
            diagnostics: skipped_stage_diagnostics(family, None),
        });
    }

    match runner() {
        Ok(Some(records)) => Ok(StageRunResult {
            records,
            diagnostics: StageDiagnostics::succeeded(family),
        }),
        Ok(None) => Ok(StageRunResult {
            records: Vec::new(),
            diagnostics: skipped_stage_diagnostics(
                family,
                Some(missing_execution_adapter_message(family)),
            ),
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
                planner_snapshot: None,
            },
        }),
        Err(error) => Err(format!("memory {} stage failed: {error}", family.as_str())),
    }
}

fn annotate_retrieval_diagnostics_from_plan_result(
    diagnostics: &mut StageDiagnostics,
    plan_result: Option<&MemoryRetrievalPlanResult>,
) {
    let Some(plan_result) = plan_result else {
        return;
    };

    diagnostics.planner_snapshot = Some(plan_result.planner_snapshot().clone());
    let planner_summary = plan_result.planner_summary_message();
    match diagnostics.message.as_mut() {
        Some(existing) => {
            existing.push_str(" | ");
            existing.push_str(planner_summary.as_str());
        }
        None => diagnostics.message = Some(planner_summary),
    }
}

fn run_rank_stage(
    system: &dyn MemorySystem,
    session_id: &str,
    entries: Vec<MemoryContextEntry>,
    metadata: &MemorySystemMetadata,
    config: &MemoryRuntimeConfig,
) -> Result<StageRunResult, String> {
    let _ = session_id;

    if !metadata.supports_pre_assembly_stage_family(MemoryStageFamily::Rank) {
        return Ok(StageRunResult {
            records: entries,
            diagnostics: skipped_stage_diagnostics(MemoryStageFamily::Rank, None),
        });
    }

    let rank_family = MemoryStageFamily::Rank;
    let pass_through_entries = entries.clone();

    #[cfg(test)]
    if let Some(error) =
        matching_memory_orchestrator_test_faults(session_id).and_then(|faults| faults.rank_error)
    {
        return handle_rank_error(rank_family, error, pass_through_entries, config);
    }

    let (records, diagnostics) = match system.run_rank_stage(entries, config) {
        Ok(Some(records)) => (records, StageDiagnostics::succeeded(rank_family)),
        Ok(None) => (
            pass_through_entries,
            skipped_stage_diagnostics(
                rank_family,
                Some(missing_execution_adapter_message(rank_family)),
            ),
        ),
        Err(error) => return handle_rank_error(rank_family, error, pass_through_entries, config),
    };

    Ok(StageRunResult {
        records,
        diagnostics,
    })
}

fn handle_rank_error(
    rank_family: MemoryStageFamily,
    error: String,
    pass_through_entries: Vec<MemoryContextEntry>,
    config: &MemoryRuntimeConfig,
) -> Result<StageRunResult, String> {
    if config.effective_fail_open() {
        let diagnostics = StageDiagnostics {
            family: rank_family,
            outcome: StageOutcome::Fallback,
            budget_ms: None,
            elapsed_ms: None,
            fallback_activated: true,
            message: Some(error),
            planner_snapshot: None,
        };
        let outcome = StageRunResult {
            records: pass_through_entries,
            diagnostics,
        };
        return Ok(outcome);
    }

    Err(format!(
        "memory {} stage failed: {error}",
        rank_family.as_str()
    ))
}

pub(crate) fn skipped_stage_diagnostics(
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
        planner_snapshot: None,
    }
}

pub(crate) fn skip_compact_stage_without_execution_adapter(
    family: MemoryStageFamily,
) -> StageDiagnostics {
    let message = Some(
        "memory system is registered but has no compact-stage execution adapter yet".to_owned(),
    );

    skipped_stage_diagnostics(family, message)
}

pub async fn run_compact_stage(
    session_id: &str,
    workspace_root: Option<&Path>,
    config: &MemoryRuntimeConfig,
) -> Result<StageDiagnostics, String> {
    let runtime = resolve_memory_system_runtime(config)?;

    runtime.run_compact_stage(session_id, workspace_root).await
}

#[cfg(not(feature = "memory-sqlite"))]
pub(crate) async fn run_builtin_compact_stage(
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
        planner_snapshot: None,
    })
}

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn run_builtin_compact_stage(
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
                planner_snapshot: None,
            })
        }
        Err(error) if config.effective_fail_open() => Ok(StageDiagnostics {
            family: MemoryStageFamily::Compact,
            outcome: StageOutcome::Fallback,
            budget_ms: None,
            elapsed_ms: None,
            fallback_activated: true,
            message: Some(error),
            planner_snapshot: None,
        }),
        Err(error) => Err(format!("memory compact stage failed: {error}")),
    }
}

pub(super) fn retrieval_query_from_recent_window(recent_window: &[WindowTurn]) -> Option<String> {
    const MAX_QUERY_TURNS: usize = 3;
    const MAX_QUERY_CHARS_PER_TURN: usize = 160;

    let mut fragments = Vec::new();

    for turn in recent_window.iter().rev() {
        if turn.role != "user" {
            continue;
        }

        let trimmed_content = turn.content.trim();
        if trimmed_content.is_empty() {
            continue;
        }
        if is_low_signal_retrieval_query_fragment(trimmed_content) {
            continue;
        }

        let normalized_fragment =
            truncate_retrieval_query_fragment(trimmed_content, MAX_QUERY_CHARS_PER_TURN);
        if normalized_fragment.is_empty() {
            continue;
        }
        if fragments.contains(&normalized_fragment) {
            continue;
        }

        fragments.push(normalized_fragment);
        if fragments.len() >= MAX_QUERY_TURNS {
            break;
        }
    }

    if fragments.is_empty() {
        return None;
    }

    fragments.reverse();
    Some(fragments.join("\n"))
}

fn truncate_retrieval_query_fragment(input: &str, max_chars: usize) -> String {
    let char_count = input.chars().count();
    if char_count <= max_chars {
        return input.to_owned();
    }

    input.chars().take(max_chars).collect()
}

fn is_low_signal_retrieval_query_fragment(input: &str) -> bool {
    let normalized = input.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return true;
    }

    if normalized.chars().count() <= 2 {
        return true;
    }

    matches!(
        normalized.as_str(),
        "ok" | "okay"
            | "thanks"
            | "thank you"
            | "continue"
            | "go on"
            | "carry on"
            | "keep going"
            | "proceed"
            | "sure"
            | "yes"
            | "yep"
            | "done"
    )
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn truncate_recall_content(input: &str, max_chars: usize) -> String {
    let char_count = input.chars().count();
    if char_count <= max_chars {
        return input.to_owned();
    }
    if max_chars <= 3 {
        return input.chars().take(max_chars).collect();
    }

    let prefix = input.chars().take(max_chars - 3).collect::<String>();
    format!("{prefix}...")
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
    system: &dyn MemorySystem,
    _session_id: &str,
    _config: &MemoryRuntimeConfig,
    _recent_window: &[WindowTurn],
) -> Result<Option<Vec<MemoryContextEntry>>, String> {
    #[cfg(test)]
    if let Some(error) = matching_memory_orchestrator_test_faults(_session_id)
        .and_then(|faults| faults.derivation_error)
    {
        return Err(error);
    }

    system.run_derive_stage(_session_id, _config, _recent_window)
}

fn run_retrieval_stage(
    system: &dyn MemorySystem,
    _session_id: &str,
    retrieval_plan: Option<&MemoryRetrievalPlanResult>,
    workspace_root: Option<&Path>,
    config: &MemoryRuntimeConfig,
    recent_window: &[WindowTurn],
) -> Result<Option<Vec<MemoryContextEntry>>, String> {
    #[cfg(test)]
    if let Some(error) = matching_memory_orchestrator_test_faults(_session_id)
        .and_then(|faults| faults.retrieval_error)
    {
        return Err(error);
    }

    let Some(retrieval_plan) = retrieval_plan else {
        return Ok(None);
    };

    system.run_retrieve_stage(
        retrieval_plan.request(),
        workspace_root,
        config,
        recent_window,
    )
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
    let runtime = resolve_memory_system_runtime(config)?;

    runtime.hydrate_stage_envelope(session_id, workspace_root)
}

pub(crate) fn hydrate_stage_envelope_without_execution_adapter(
    session_id: &str,
    config: &MemoryRuntimeConfig,
    metadata: &MemorySystemMetadata,
) -> Result<StageEnvelope, String> {
    let recent_window = recent_window_records(session_id, config)?;
    let entries = load_prompt_context(session_id, config)?;
    let diagnostics = metadata
        .supported_pre_assembly_stage_families
        .iter()
        .copied()
        .map(|family| {
            let message = Some(missing_execution_adapter_message(family));

            skipped_stage_diagnostics(family, message)
        })
        .collect::<Vec<_>>();

    let hydrated = HydratedMemoryContext::from_stage_parts(
        entries,
        recent_window,
        &diagnostics,
        metadata.id,
        config,
    );
    let envelope = StageEnvelope {
        hydrated,
        retrieval_request: None,
        retrieval_planner_snapshot: None,
        diagnostics,
    };

    Ok(envelope)
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
    use crate::config::MemoryProfile;
    use crate::memory::{
        DEFAULT_MEMORY_SYSTEM_ID, DerivedMemoryKind, MemoryContextKind, MemoryRecallMode,
        MemoryScope, MemoryStageFamily, MemorySystem, MemorySystemCapability, MemorySystemMetadata,
        RECALL_FIRST_MEMORY_SYSTEM_ID, StageOutcome, WORKSPACE_RECALL_MEMORY_SYSTEM_ID,
        append_turn_direct, register_memory_system,
    };
    use serde_json::json;

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
        crate::test_support::unique_temp_dir(prefix)
    }

    #[cfg(feature = "memory-sqlite")]
    fn sqlite_memory_config(
        db_path: std::path::PathBuf,
    ) -> crate::memory::runtime_config::MemoryRuntimeConfig {
        crate::memory::runtime_config::MemoryRuntimeConfig::for_sqlite_path(db_path)
    }

    #[cfg(feature = "memory-sqlite")]
    fn sqlite_memory_config_with_profile(
        db_path: std::path::PathBuf,
        profile: MemoryProfile,
        sliding_window: usize,
    ) -> crate::memory::runtime_config::MemoryRuntimeConfig {
        let mut config = sqlite_memory_config(db_path);
        config.profile = profile;
        config.mode = profile.mode();
        config.sliding_window = sliding_window;
        config
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrated_memory_builtin_orchestrator_returns_recent_window_records() {
        let tmp = hydrated_memory_temp_dir("loong-hydrated-window");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("window.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

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
        let tmp = hydrated_memory_temp_dir("loong-hydrated-diagnostics");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("diagnostics.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = sqlite_memory_config(db_path.clone());

        let hydrated = hydrate_memory_context("hydrated-diagnostics", &config)
            .expect("hydrate memory context");

        assert_eq!(hydrated.diagnostics.system_id, "builtin");
        assert!(hydrated.diagnostics.fail_open);
        assert!(!hydrated.diagnostics.strict_mode_requested);
        assert!(!hydrated.diagnostics.strict_mode_active);
        assert!(!hydrated.diagnostics.degraded);
        assert_eq!(hydrated.diagnostics.derivation_error, None);
        assert_eq!(hydrated.diagnostics.retrieval_error, None);
        assert_eq!(hydrated.diagnostics.rank_error, None);
        assert_eq!(hydrated.diagnostics.recent_window_count, 0);
        assert_eq!(hydrated.diagnostics.entry_count, 0);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrated_memory_builtin_orchestrator_preserves_summary_behavior() {
        let tmp = hydrated_memory_temp_dir("loong-hydrated-summary");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("summary.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);

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
        let tmp = hydrated_memory_temp_dir("loong-hydrated-cross-session-recall");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("cross-session-recall.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 8);

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
        assert_eq!(recalled.provenance.len(), 1);
        assert_eq!(
            recalled.provenance[0].source_kind,
            crate::memory::MemoryProvenanceSourceKind::CanonicalMemoryRecord
        );
        assert_eq!(
            recalled.provenance[0].memory_system_id,
            crate::memory::DEFAULT_MEMORY_SYSTEM_ID
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrated_memory_builtin_orchestrator_preserves_profile_behavior() {
        let tmp = hydrated_memory_temp_dir("loong-hydrated-profile");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("profile.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let mut config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::ProfilePlusWindow, 2);
        config.profile_note = Some("Imported ZeroClaw preferences".to_owned());

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
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-order");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("stage-order.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);

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
        assert!(envelope.diagnostics[0].outcome == StageOutcome::Succeeded);
        assert_eq!(envelope.diagnostics[1].outcome, StageOutcome::Succeeded);
        assert_eq!(envelope.diagnostics[2].outcome, StageOutcome::Succeeded);
        let retrieval_request = envelope
            .retrieval_request
            .expect("window-plus-summary should advertise retrieval request");
        assert_eq!(
            retrieval_request.strategy,
            crate::memory::MemoryRetrievalStrategy::RecentUserQuery
        );
        assert_eq!(retrieval_request.query.as_deref(), Some("turn 3"));

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
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-fallback");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("stage-fallback.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

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
    fn hydrate_stage_envelope_with_workspace_root_uses_recent_user_query_with_workspace_strategy() {
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-query-with-workspace");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("query-with-workspace.sqlite3");
        let _ = std::fs::remove_file(&db_path);
        let memory_file_path = tmp.join("MEMORY.md");
        std::fs::write(&memory_file_path, "remember this").expect("write memory file");

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 4);

        append_turn_direct(
            "stage-query-with-workspace",
            "user",
            "initial deploy checklist",
            &config,
        )
        .expect("append turn 1 should succeed");
        append_turn_direct(
            "stage-query-with-workspace",
            "assistant",
            "working on it",
            &config,
        )
        .expect("append turn 2 should succeed");
        append_turn_direct(
            "stage-query-with-workspace",
            "user",
            "release freeze timing",
            &config,
        )
        .expect("append turn 3 should succeed");

        let envelope = hydrate_stage_envelope_with_workspace_root(
            "stage-query-with-workspace",
            Some(tmp.as_path()),
            &config,
        )
        .expect("hydrate staged envelope");

        let retrieval_request = envelope
            .retrieval_request
            .expect("window-plus-summary with workspace should advertise retrieval request");
        assert_eq!(
            retrieval_request.strategy,
            crate::memory::MemoryRetrievalStrategy::RecentUserQueryWithWorkspace
        );
        assert_eq!(
            retrieval_request.query.as_deref(),
            Some("initial deploy checklist\nrelease freeze timing")
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&memory_file_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrate_stage_envelope_with_workspace_root_falls_back_when_latest_user_turn_is_low_signal() {
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-low-signal-workspace");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("low-signal-workspace.sqlite3");
        let _ = std::fs::remove_file(&db_path);
        let memory_file_path = tmp.join("MEMORY.md");
        std::fs::write(&memory_file_path, "remember deploy freeze").expect("write memory file");

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 4);

        append_turn_direct("stage-low-signal-workspace", "user", "continue", &config)
            .expect("append turn should succeed");

        let envelope = hydrate_stage_envelope_with_workspace_root(
            "stage-low-signal-workspace",
            Some(tmp.as_path()),
            &config,
        )
        .expect("hydrate staged envelope");

        let retrieval_request = envelope
            .retrieval_request
            .expect("workspace-backed low-signal request should still hydrate");
        assert_eq!(
            retrieval_request.strategy,
            crate::memory::MemoryRetrievalStrategy::WorkspaceReferenceOnly
        );
        assert_eq!(retrieval_request.query, None);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&memory_file_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrate_stage_envelope_with_workspace_root_uses_structured_signal_strategy_without_query() {
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-structured-signal-workspace");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("structured-signal-workspace.sqlite3");
        let _ = std::fs::remove_file(&db_path);
        let memory_file_path = tmp.join("MEMORY.md");
        std::fs::write(&memory_file_path, "remember deploy freeze").expect("write memory file");

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 4);

        append_turn_direct(
            "stage-structured-signal-workspace",
            "assistant",
            crate::memory::build_tool_decision_content(
                "turn-1",
                "tool-1",
                json!({"tool_name": "shell.exec"}),
            )
            .as_str(),
            &config,
        )
        .expect("append tool decision should succeed");

        let envelope = hydrate_stage_envelope_with_workspace_root(
            "stage-structured-signal-workspace",
            Some(tmp.as_path()),
            &config,
        )
        .expect("hydrate staged envelope");

        let retrieval_request = envelope
            .retrieval_request
            .expect("workspace-backed structured signal should advertise retrieval request");
        assert_eq!(
            retrieval_request.strategy,
            crate::memory::MemoryRetrievalStrategy::StructuredSignalQueryWithWorkspace
        );
        assert_eq!(retrieval_request.query.as_deref(), Some("shell.exec"));
        assert_eq!(
            retrieval_request.allowed_kinds,
            vec![
                DerivedMemoryKind::Reference,
                DerivedMemoryKind::Procedure,
                DerivedMemoryKind::Fact,
            ]
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&memory_file_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrate_stage_envelope_with_workspace_root_uses_task_progress_intent_query_strategy() {
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-task-progress-workspace");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("task-progress-workspace.sqlite3");
        let _ = std::fs::remove_file(&db_path);
        let memory_file_path = tmp.join("MEMORY.md");
        std::fs::write(&memory_file_path, "remember deploy freeze").expect("write memory file");

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 4);
        super::super::ensure_memory_db_ready(Some(db_path.clone()), &config)
            .expect("ensure memory db ready");
        let conn = rusqlite::Connection::open(&db_path).expect("open sqlite db");
        conn.execute(
            "INSERT INTO session_events(session_id, event_kind, actor_session_id, payload_json, search_text, ts)
             VALUES (?1, ?2, NULL, ?3, '', 1)",
            rusqlite::params![
                "stage-task-progress-workspace",
                crate::task_progress::TASK_PROGRESS_EVENT_KIND,
                serde_json::to_string(&crate::task_progress::task_progress_event_payload(
                    "unit_test",
                    &crate::task_progress::TaskProgressRecord {
                        task_id: "stage-task-progress-workspace".to_owned(),
                        owner_kind: "conversation_turn".to_owned(),
                        status: crate::task_progress::TaskProgressStatus::Waiting,
                        intent_summary: Some("diagnose release freeze".to_owned()),
                        verification_state: None,
                        active_handles: Vec::new(),
                        resume_recipe: None,
                        updated_at: 1,
                    },
                ))
                .expect("encode task progress payload"),
            ],
        )
        .expect("insert task progress event");

        let envelope = hydrate_stage_envelope_with_workspace_root(
            "stage-task-progress-workspace",
            Some(tmp.as_path()),
            &config,
        )
        .expect("hydrate staged envelope");

        let retrieval_request = envelope
            .retrieval_request
            .expect("workspace-backed task progress should advertise retrieval request");
        assert_eq!(
            retrieval_request.strategy,
            crate::memory::MemoryRetrievalStrategy::TaskProgressIntentQueryWithWorkspace
        );
        assert_eq!(
            retrieval_request.query.as_deref(),
            Some("diagnose release freeze")
        );
        assert_eq!(retrieval_request.budget_items, 2);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&memory_file_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrate_stage_envelope_with_workspace_root_uses_delegate_label_query_strategy() {
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-delegate-label-workspace");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("delegate-label-workspace.sqlite3");
        let _ = std::fs::remove_file(&db_path);
        let memory_file_path = tmp.join("MEMORY.md");
        std::fs::write(&memory_file_path, "remember deploy freeze").expect("write memory file");

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 4);
        super::super::ensure_memory_db_ready(Some(db_path.clone()), &config)
            .expect("ensure memory db ready");
        let conn = rusqlite::Connection::open(&db_path).expect("open sqlite db");
        conn.execute(
            "INSERT INTO sessions(session_id, kind, parent_session_id, label, state, created_at, updated_at, last_error)
             VALUES (?1, 'root', NULL, ?2, 'ready', 1, 1, NULL)",
            rusqlite::params!["root-session", "Root Session"],
        )
        .expect("insert root session");
        conn.execute(
            "INSERT INTO sessions(session_id, kind, parent_session_id, label, state, created_at, updated_at, last_error)
             VALUES (?1, ?2, ?3, ?4, 'ready', 1, 1, NULL)",
            rusqlite::params![
                "stage-delegate-label-workspace",
                "delegate_child",
                "root-session",
                "Release Child",
            ],
        )
        .expect("insert delegate child session");

        let envelope = hydrate_stage_envelope_with_workspace_root(
            "stage-delegate-label-workspace",
            Some(tmp.as_path()),
            &config,
        )
        .expect("hydrate staged envelope");

        let retrieval_request = envelope
            .retrieval_request
            .expect("delegate child workspace should advertise retrieval request");
        assert_eq!(
            retrieval_request.strategy,
            crate::memory::MemoryRetrievalStrategy::DelegateLineageQueryWithWorkspace
        );
        assert_eq!(
            retrieval_request.query.as_deref(),
            Some("Release Child\nRoot Session")
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&memory_file_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrate_stage_envelope_with_workspace_root_uses_workflow_task_query_strategy() {
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-delegate-task-workspace");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("delegate-task-workspace.sqlite3");
        let _ = std::fs::remove_file(&db_path);
        let memory_file_path = tmp.join("MEMORY.md");
        std::fs::write(&memory_file_path, "remember deploy freeze").expect("write memory file");

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 4);
        super::super::ensure_memory_db_ready(Some(db_path.clone()), &config)
            .expect("ensure memory db ready");
        let conn = rusqlite::Connection::open(&db_path).expect("open sqlite db");
        conn.execute(
            "INSERT INTO sessions(session_id, kind, parent_session_id, label, state, created_at, updated_at, last_error)
             VALUES (?1, ?2, ?3, ?4, 'ready', 1, 1, NULL)",
            rusqlite::params![
                "stage-delegate-task-workspace",
                "delegate_child",
                "root-session",
                "Release Child",
            ],
        )
        .expect("insert delegate child session");
        conn.execute(
            "INSERT INTO session_events(session_id, event_kind, actor_session_id, payload_json, search_text, ts)
             VALUES (?1, 'delegate_started', ?2, ?3, '', 1)",
            rusqlite::params![
                "stage-delegate-task-workspace",
                "root-session",
                serde_json::to_string(&json!({
                    "task": "investigate release freeze",
                }))
                .expect("encode delegate task payload"),
            ],
        )
        .expect("insert delegate task event");

        let envelope = hydrate_stage_envelope_with_workspace_root(
            "stage-delegate-task-workspace",
            Some(tmp.as_path()),
            &config,
        )
        .expect("hydrate staged envelope");

        let retrieval_request = envelope
            .retrieval_request
            .expect("delegate task workspace should advertise retrieval request");
        let envelope_snapshot = envelope
            .retrieval_planner_snapshot
            .as_ref()
            .expect("envelope planner snapshot");
        assert_eq!(
            retrieval_request.strategy,
            crate::memory::MemoryRetrievalStrategy::WorkflowTaskQueryWithWorkspace
        );
        assert_eq!(
            retrieval_request.query.as_deref(),
            Some("investigate release freeze")
        );
        assert_eq!(retrieval_request.budget_items, 2);
        assert_eq!(
            envelope_snapshot.strategy,
            crate::memory::MemoryRetrievalStrategy::WorkflowTaskQueryWithWorkspace
        );
        let planner_snapshot = envelope.diagnostics[1]
            .planner_snapshot
            .as_ref()
            .expect("planner snapshot");
        assert_eq!(
            planner_snapshot.strategy,
            crate::memory::MemoryRetrievalStrategy::WorkflowTaskQueryWithWorkspace
        );
        assert_eq!(planner_snapshot.memory_system_id, DEFAULT_MEMORY_SYSTEM_ID);
        assert_eq!(planner_snapshot.budget_items, 2);
        assert!(planner_snapshot.query_present);
        assert!(
            planner_snapshot
                .planning_notes
                .iter()
                .any(|note| note.contains("workflow task seed"))
        );
        assert!(
            envelope.diagnostics[1]
                .message
                .as_deref()
                .is_some_and(|message| message.contains(
                    "planner system=builtin strategy=workflow_task_query_with_workspace"
                ))
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&memory_file_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrate_stage_envelope_with_workspace_root_completed_workflow_task_includes_phase_hint() {
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-complete-workflow-task");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("complete-workflow-task.sqlite3");
        let _ = std::fs::remove_file(&db_path);
        let memory_file_path = tmp.join("MEMORY.md");
        std::fs::write(&memory_file_path, "remember deploy freeze").expect("write memory file");

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 4);
        super::super::ensure_memory_db_ready(Some(db_path.clone()), &config)
            .expect("ensure memory db ready");
        let conn = rusqlite::Connection::open(&db_path).expect("open sqlite db");
        conn.execute(
            "INSERT INTO sessions(session_id, kind, parent_session_id, label, state, created_at, updated_at, last_error)
             VALUES (?1, ?2, ?3, ?4, 'completed', 1, 1, NULL)",
            rusqlite::params![
                "stage-complete-workflow-task",
                "delegate_child",
                "root-session",
                "Release Child",
            ],
        )
        .expect("insert delegate child session");
        conn.execute(
            "INSERT INTO session_events(session_id, event_kind, actor_session_id, payload_json, search_text, ts)
             VALUES (?1, 'delegate_completed', ?2, ?3, '', 1)",
            rusqlite::params![
                "stage-complete-workflow-task",
                "root-session",
                serde_json::to_string(&json!({
                    "task": "investigate release freeze",
                }))
                .expect("encode delegate task payload"),
            ],
        )
        .expect("insert delegate task event");

        let envelope = hydrate_stage_envelope_with_workspace_root(
            "stage-complete-workflow-task",
            Some(tmp.as_path()),
            &config,
        )
        .expect("hydrate staged envelope");

        let retrieval_request = envelope
            .retrieval_request
            .expect("completed workflow task should advertise retrieval request");
        assert_eq!(
            retrieval_request.strategy,
            crate::memory::MemoryRetrievalStrategy::WorkflowTaskQueryWithWorkspace
        );
        assert_eq!(
            retrieval_request.query.as_deref(),
            Some("investigate release freeze\nphase: complete")
        );
        assert_eq!(retrieval_request.budget_items, 1);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&memory_file_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrate_stage_envelope_window_plus_summary_keeps_summary_retrieval_request() {
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-window-plus-summary");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("window-plus-summary.sqlite3");
        let _ = std::fs::remove_file(&db_path);
        let memory_dir = tmp.join("memory");
        let _ = std::fs::create_dir_all(&memory_dir);
        let memory_file_path = tmp.join("MEMORY.md");
        std::fs::write(&memory_file_path, "remember this").expect("write memory file");
        let memory_file_path_text = memory_file_path
            .canonicalize()
            .expect("canonical memory file path")
            .display()
            .to_string();

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 4);

        let envelope = hydrate_stage_envelope_with_workspace_root(
            "stage-window-plus-summary",
            Some(tmp.as_path()),
            &config,
        )
        .expect("hydrate staged envelope");

        let retrieval_request = envelope
            .retrieval_request
            .expect("window-plus-summary should advertise retrieval request");
        assert_eq!(retrieval_request.memory_system_id, "builtin");
        assert_eq!(
            retrieval_request.strategy,
            crate::memory::MemoryRetrievalStrategy::WorkspaceReferenceOnly
        );
        assert_eq!(
            retrieval_request.recall_mode,
            crate::memory::MemoryRecallMode::PromptAssembly
        );
        assert_eq!(retrieval_request.query, None);
        assert_eq!(retrieval_request.budget_items, 1);
        assert_eq!(
            retrieval_request.allowed_kinds,
            vec![DerivedMemoryKind::Reference]
        );
        assert_eq!(
            retrieval_request.scopes,
            vec![MemoryScope::Workspace, MemoryScope::Session]
        );
        assert_eq!(envelope.hydrated.entries.len(), 1);
        assert_eq!(
            envelope.hydrated.entries[0].kind,
            crate::memory::MemoryContextKind::RetrievedMemory
        );
        assert_eq!(envelope.hydrated.entries[0].provenance.len(), 1);
        assert_eq!(
            envelope.hydrated.entries[0].provenance[0]
                .source_path
                .as_deref(),
            Some(memory_file_path_text.as_str())
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
            retrieval_query_from_recent_window(recent_window.as_slice()).expect("query fallback");

        assert_eq!(query, "release rollback plan");
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn retrieval_query_from_recent_window_combines_recent_non_empty_user_turns() {
        let recent_window = vec![
            WindowTurn {
                role: "user".to_owned(),
                content: "initial deploy checklist".to_owned(),
                ts: None,
            },
            WindowTurn {
                role: "assistant".to_owned(),
                content: "working on it".to_owned(),
                ts: None,
            },
            WindowTurn {
                role: "user".to_owned(),
                content: "rollback smoke test".to_owned(),
                ts: None,
            },
            WindowTurn {
                role: "user".to_owned(),
                content: "release freeze timing".to_owned(),
                ts: None,
            },
        ];

        let query =
            retrieval_query_from_recent_window(recent_window.as_slice()).expect("query fallback");

        assert_eq!(
            query,
            "initial deploy checklist\nrollback smoke test\nrelease freeze timing"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn retrieval_query_from_recent_window_dedupes_recent_user_turns() {
        let recent_window = vec![
            WindowTurn {
                role: "user".to_owned(),
                content: "rollback smoke test".to_owned(),
                ts: None,
            },
            WindowTurn {
                role: "assistant".to_owned(),
                content: "noted".to_owned(),
                ts: None,
            },
            WindowTurn {
                role: "user".to_owned(),
                content: "rollback smoke test".to_owned(),
                ts: None,
            },
        ];

        let query =
            retrieval_query_from_recent_window(recent_window.as_slice()).expect("query fallback");

        assert_eq!(query, "rollback smoke test");
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn retrieval_query_from_recent_window_skips_low_signal_followups() {
        let recent_window = vec![
            WindowTurn {
                role: "user".to_owned(),
                content: "rollback smoke test".to_owned(),
                ts: None,
            },
            WindowTurn {
                role: "assistant".to_owned(),
                content: "working on it".to_owned(),
                ts: None,
            },
            WindowTurn {
                role: "user".to_owned(),
                content: "continue".to_owned(),
                ts: None,
            },
        ];

        let query =
            retrieval_query_from_recent_window(recent_window.as_slice()).expect("query fallback");

        assert_eq!(query, "rollback smoke test");
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn retrieval_query_from_recent_window_returns_none_for_only_low_signal_turns() {
        let recent_window = vec![
            WindowTurn {
                role: "user".to_owned(),
                content: "ok".to_owned(),
                ts: None,
            },
            WindowTurn {
                role: "assistant".to_owned(),
                content: "done".to_owned(),
                ts: None,
            },
            WindowTurn {
                role: "user".to_owned(),
                content: "continue".to_owned(),
                ts: None,
            },
        ];

        assert_eq!(
            retrieval_query_from_recent_window(recent_window.as_slice()),
            None
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn truncate_recall_content_keeps_roleless_records_neutral() {
        let hit = crate::memory::CanonicalMemorySearchHit {
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
        };

        let rendered = truncate_recall_content(hit.record.content.as_str(), 280);

        assert!(
            rendered.contains("Imported release checklist with smoke tests."),
            "expected rendered recall content: {rendered}"
        );
        assert!(
            !rendered.contains("assistant:"),
            "roleless recall truncation should not fabricate assistant provenance: {rendered}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrate_stage_envelope_derives_retrieval_query_from_latest_user_turn() {
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-retrieval-query");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("retrieval-query.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 4);

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
        assert_eq!(retrieval_request.memory_system_id, "builtin");
        assert_eq!(
            retrieval_request.recall_mode,
            crate::memory::MemoryRecallMode::PromptAssembly
        );
        assert_eq!(retrieval_request.budget_items, 4);
        assert_eq!(
            retrieval_request.scopes,
            vec![
                MemoryScope::Session,
                MemoryScope::Workspace,
                MemoryScope::Agent,
                MemoryScope::User,
            ]
        );
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

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrate_stage_envelope_window_only_omits_summary_retrieval_request() {
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-window-only");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("window-only.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 4);

        let envelope =
            hydrate_stage_envelope("stage-window-only", &config).expect("hydrate staged envelope");

        assert_eq!(envelope.retrieval_request, None);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hydrate_stage_envelope_profile_plus_window_omits_summary_retrieval_request() {
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-profile-plus-window");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("profile-plus-window.sqlite3");
        let _ = std::fs::remove_file(&db_path);
        let memory_dir = tmp.join("memory");
        let _ = std::fs::create_dir_all(&memory_dir);
        let daily_log_path = memory_dir.join("2026-04-06.md");
        std::fs::write(&daily_log_path, "recent durable note").expect("write daily log");
        let daily_log_path_text = daily_log_path
            .canonicalize()
            .expect("canonical daily log path")
            .display()
            .to_string();

        let mut config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::ProfilePlusWindow, 4);
        config.profile_note = Some("Imported ZeroClaw preferences".to_owned());

        let envelope = hydrate_stage_envelope_with_workspace_root(
            "stage-profile-plus-window",
            Some(tmp.as_path()),
            &config,
        )
        .expect("hydrate staged envelope");

        let retrieval_request = envelope
            .retrieval_request
            .expect("profile-plus-window should advertise retrieval request");
        assert_eq!(retrieval_request.memory_system_id, "builtin");
        assert_eq!(
            retrieval_request.strategy,
            crate::memory::MemoryRetrievalStrategy::WorkspaceReferenceOnly
        );
        assert_eq!(
            retrieval_request.scopes,
            vec![MemoryScope::Workspace, MemoryScope::Session]
        );
        assert_eq!(
            retrieval_request.allowed_kinds,
            vec![DerivedMemoryKind::Reference]
        );
        let retrieved_entry = envelope
            .hydrated
            .entries
            .iter()
            .find(|entry| entry.kind == crate::memory::MemoryContextKind::RetrievedMemory)
            .expect("retrieved entry");
        assert_eq!(retrieved_entry.provenance.len(), 1);
        assert_eq!(
            retrieved_entry.provenance[0].scope,
            Some(MemoryScope::Session)
        );
        assert_eq!(
            retrieved_entry.provenance[0].source_path.as_deref(),
            Some(daily_log_path_text.as_str())
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&daily_log_path);
        let _ = std::fs::remove_dir(&memory_dir);
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

        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-registry-selected");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("registry-selected.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let mut config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);
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
            vec![
                (MemoryStageFamily::Derive, StageOutcome::Skipped),
                (MemoryStageFamily::Retrieve, StageOutcome::Skipped),
                (MemoryStageFamily::Rank, StageOutcome::Skipped),
            ]
        );
        assert_eq!(
            envelope.diagnostics[1].message.as_deref(),
            Some(
                "memory system declares `retrieve` stage support but provides no execution adapter"
            )
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn workspace_recall_system_executes_retrieve_and_rank_stages() {
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-workspace-recall");
        let memory_dir = tmp.join("memory");
        let memory_file_path = tmp.join("MEMORY.md");
        let _ = std::fs::create_dir_all(&memory_dir);
        let _ = std::fs::write(
            &memory_file_path,
            "# Durable Notes\n\nRemember the deploy freeze window.\n",
        );
        let memory_file_path_text = memory_file_path
            .canonicalize()
            .expect("canonical memory file path")
            .display()
            .to_string();
        let _ = std::fs::write(
            memory_dir.join("2026-03-22.md"),
            "## Durable Recall\n\nCustomer migration starts tomorrow.\n",
        );

        let db_path = tmp.join("workspace-recall.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let mut config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);
        config.resolved_system_id = Some(WORKSPACE_RECALL_MEMORY_SYSTEM_ID.to_owned());

        append_turn_direct("workspace-recall", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("workspace-recall", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("workspace-recall", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let envelope = hydrate_stage_envelope_with_workspace_root(
            "workspace-recall",
            Some(tmp.as_path()),
            &config,
        )
        .expect("hydrate staged envelope for workspace-recall system");

        let retrieval_request = envelope
            .retrieval_request
            .as_ref()
            .expect("workspace recall should advertise retrieval request");
        assert_eq!(
            retrieval_request.memory_system_id,
            WORKSPACE_RECALL_MEMORY_SYSTEM_ID
        );
        assert_eq!(
            retrieval_request.strategy,
            crate::memory::MemoryRetrievalStrategy::WorkspaceReferenceOnly
        );
        assert_eq!(
            retrieval_request.recall_mode,
            MemoryRecallMode::PromptAssembly
        );
        assert_eq!(retrieval_request.budget_items, 2);
        assert_eq!(
            envelope.retrieval_planner_snapshot,
            Some(retrieval_request.planner_snapshot())
        );
        assert_eq!(
            envelope.diagnostics[1].planner_snapshot,
            Some(retrieval_request.planner_snapshot())
        );

        let entry_kinds = envelope
            .hydrated
            .entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>();
        assert!(
            entry_kinds.contains(&MemoryContextKind::RetrievedMemory),
            "expected at least one retrieved-memory entry"
        );
        assert_eq!(
            entry_kinds
                .iter()
                .filter(|kind| **kind == MemoryContextKind::Turn)
                .count(),
            2
        );

        let retrieved_entry = envelope
            .hydrated
            .entries
            .iter()
            .find(|entry| {
                let first_provenance = entry.provenance.first();
                let entry_label =
                    first_provenance.and_then(|provenance| provenance.source_label.as_deref());
                entry.kind == MemoryContextKind::RetrievedMemory && entry_label == Some("MEMORY.md")
            })
            .expect("curated retrieved memory entry");
        let retrieved_provenance = retrieved_entry
            .provenance
            .first()
            .expect("retrieved provenance");
        assert_eq!(
            retrieved_provenance.memory_system_id,
            WORKSPACE_RECALL_MEMORY_SYSTEM_ID
        );
        assert_eq!(
            retrieved_provenance.source_path.as_deref(),
            Some(memory_file_path_text.as_str())
        );

        let diagnostics = envelope
            .diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.family, diagnostic.outcome))
            .collect::<Vec<_>>();
        assert_eq!(
            diagnostics,
            vec![
                (MemoryStageFamily::Derive, StageOutcome::Succeeded),
                (MemoryStageFamily::Retrieve, StageOutcome::Succeeded),
                (MemoryStageFamily::Rank, StageOutcome::Succeeded),
            ]
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn recall_first_system_stage_envelope_preserves_selected_identity_and_snapshot() {
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-recall-first");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("recall-first.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let mut config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);
        config.resolved_system_id = Some(RECALL_FIRST_MEMORY_SYSTEM_ID.to_owned());

        append_turn_direct("recall-first", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("recall-first", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct(
            "recall-first",
            "user",
            "investigate deploy freeze timing",
            &config,
        )
        .expect("append turn 3 should succeed");

        let envelope = hydrate_stage_envelope_with_workspace_root(
            "recall-first",
            Some(tmp.as_path()),
            &config,
        )
        .expect("hydrate staged envelope for recall-first");

        let retrieval_request = envelope
            .retrieval_request
            .as_ref()
            .expect("recall-first should advertise retrieval request");
        assert_eq!(
            retrieval_request.memory_system_id,
            RECALL_FIRST_MEMORY_SYSTEM_ID
        );
        assert_eq!(
            envelope.hydrated.diagnostics.system_id,
            RECALL_FIRST_MEMORY_SYSTEM_ID
        );
        assert_eq!(
            envelope.retrieval_planner_snapshot,
            Some(retrieval_request.planner_snapshot())
        );
        assert_eq!(
            envelope.diagnostics[1].planner_snapshot,
            Some(retrieval_request.planner_snapshot())
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn workspace_recall_system_reorders_retrieved_entries_ahead_of_history_turns() {
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-workspace-recall");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("workspace-recall.sqlite3");
        let _ = std::fs::remove_file(&db_path);
        let memory_dir = tmp.join("memory");
        let _ = std::fs::create_dir_all(&memory_dir);
        let memory_file_path = tmp.join("MEMORY.md");
        std::fs::write(&memory_file_path, "curated workspace fact").expect("write memory file");

        let mut config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);
        config.resolved_system_id =
            Some(crate::memory::WORKSPACE_RECALL_MEMORY_SYSTEM_ID.to_owned());

        append_turn_direct("workspace-recall", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("workspace-recall", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");

        let envelope = hydrate_stage_envelope_with_workspace_root(
            "workspace-recall",
            Some(tmp.as_path()),
            &config,
        )
        .expect("hydrate staged envelope for workspace-recall");

        assert_eq!(
            envelope.hydrated.diagnostics.system_id,
            crate::memory::WORKSPACE_RECALL_MEMORY_SYSTEM_ID
        );
        assert_eq!(envelope.diagnostics[0].outcome, StageOutcome::Succeeded);
        assert_eq!(envelope.diagnostics[1].outcome, StageOutcome::Succeeded);
        assert_eq!(envelope.diagnostics[2].outcome, StageOutcome::Succeeded);
        assert_eq!(
            envelope.hydrated.entries[0].kind,
            MemoryContextKind::RetrievedMemory
        );
        assert_eq!(envelope.hydrated.entries[0].provenance.len(), 1);
        assert_eq!(
            envelope.hydrated.entries[0].provenance[0].memory_system_id,
            crate::memory::WORKSPACE_RECALL_MEMORY_SYSTEM_ID
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&memory_file_path);
        let _ = std::fs::remove_dir(&memory_dir);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn unknown_registry_selected_system_falls_back_to_builtin_hydration() {
        let tmp = hydrated_memory_temp_dir("loong-stage-envelope-unknown-selected");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("unknown-selected.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let mut config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);
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
        let tmp = hydrated_memory_temp_dir("loong-compact-stage-succeeded");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("compact-stage.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);

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
        let tmp = hydrated_memory_temp_dir("loong-compact-stage-skipped");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("compact-stage-skipped.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);

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
        let tmp = hydrated_memory_temp_dir("loong-compact-stage-duplicate");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("compact-stage-duplicate.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);

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
    async fn compact_stage_skips_for_registry_selected_system_without_executor() {
        register_memory_system(REGISTRY_RETRIEVE_ONLY_COMPACT_SYSTEM_ID, || {
            Box::new(RegistryRetrieveOnlyMemorySystem {
                id: REGISTRY_RETRIEVE_ONLY_COMPACT_SYSTEM_ID,
            })
        })
        .expect("register registry-selected memory system");

        let tmp = hydrated_memory_temp_dir("loong-compact-stage-registry-selected");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("compact-stage-registry-selected.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let mut config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);
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
        assert_eq!(diagnostics.outcome, StageOutcome::Skipped);
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
        let tmp = hydrated_memory_temp_dir("loong-fail-open-derivation");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("derivation.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

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
        let tmp = hydrated_memory_temp_dir("loong-fail-open-retrieval");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("retrieval.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

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
        let tmp = hydrated_memory_temp_dir("loong-fail-open-rank");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("rank.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

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
        let tmp = hydrated_memory_temp_dir("loong-fail-open-strict-reserved");
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("strict-reserved.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let mut config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);
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
