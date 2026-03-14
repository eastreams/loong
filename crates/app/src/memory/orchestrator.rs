#[cfg(test)]
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::config::MemorySystemKind;

use super::{
    MemoryContextEntry, WindowTurn, load_prompt_context, runtime_config::MemoryRuntimeConfig,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydratedMemoryContext {
    pub entries: Vec<MemoryContextEntry>,
    pub recent_window: Vec<WindowTurn>,
    pub diagnostics: MemoryDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryDiagnostics {
    pub system_id: &'static str,
    pub fail_open: bool,
    pub strict_mode_requested: bool,
    pub strict_mode_active: bool,
    pub degraded: bool,
    pub derivation_error: Option<String>,
    pub retrieval_error: Option<String>,
    pub recent_window_count: usize,
    pub entry_count: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BuiltinMemoryOrchestrator;

#[cfg(test)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryOrchestratorTestFaults {
    pub derivation_error: Option<String>,
    pub retrieval_error: Option<String>,
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
    pub fn hydrate(
        &self,
        session_id: &str,
        config: &MemoryRuntimeConfig,
    ) -> Result<HydratedMemoryContext, String> {
        let recent_window = recent_window_records(session_id, config)?;
        let mut entries = load_prompt_context(session_id, config)?;
        let mut derivation_error = None;
        let mut retrieval_error = None;
        let fail_open = config.effective_fail_open();

        match run_derivation_stage(session_id, config, &recent_window) {
            Ok(extra_entries) => entries.extend(extra_entries),
            Err(error) if fail_open => derivation_error = Some(error),
            Err(error) => return Err(format!("memory derivation stage failed: {error}")),
        }

        match run_retrieval_stage(session_id, config, &recent_window) {
            Ok(extra_entries) => entries.extend(extra_entries),
            Err(error) if fail_open => retrieval_error = Some(error),
            Err(error) => return Err(format!("memory retrieval stage failed: {error}")),
        }

        let degraded = derivation_error.is_some() || retrieval_error.is_some();
        let diagnostics = MemoryDiagnostics {
            system_id: config.system.as_str(),
            fail_open,
            strict_mode_requested: config.strict_mode_requested(),
            strict_mode_active: config.strict_mode_active(),
            degraded,
            derivation_error,
            retrieval_error,
            recent_window_count: recent_window.len(),
            entry_count: entries.len(),
        };

        Ok(HydratedMemoryContext {
            entries,
            recent_window,
            diagnostics,
        })
    }
}

fn run_derivation_stage(
    _session_id: &str,
    _config: &MemoryRuntimeConfig,
    _recent_window: &[WindowTurn],
) -> Result<Vec<MemoryContextEntry>, String> {
    #[cfg(test)]
    if let Some(error) =
        active_memory_orchestrator_test_faults().and_then(|faults| faults.derivation_error)
    {
        return Err(error);
    }

    Ok(Vec::new())
}

fn run_retrieval_stage(
    _session_id: &str,
    _config: &MemoryRuntimeConfig,
    _recent_window: &[WindowTurn],
) -> Result<Vec<MemoryContextEntry>, String> {
    #[cfg(test)]
    if let Some(error) =
        active_memory_orchestrator_test_faults().and_then(|faults| faults.retrieval_error)
    {
        return Err(error);
    }

    Ok(Vec::new())
}

pub fn hydrate_memory_context(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<HydratedMemoryContext, String> {
    match config.system {
        MemorySystemKind::Builtin => BuiltinMemoryOrchestrator.hydrate(session_id, config),
    }
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
    use crate::memory::{MemoryContextKind, append_turn_direct};

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
    fn fail_open_memory_derivation_failure_keeps_recent_window_behavior() {
        let _faults = ScopedMemoryOrchestratorTestFaults::set(MemoryOrchestratorTestFaults {
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

        append_turn_direct("fail-open-derivation", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("fail-open-derivation", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("fail-open-derivation", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let hydrated = hydrate_memory_context("fail-open-derivation", &config)
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
        let _faults = ScopedMemoryOrchestratorTestFaults::set(MemoryOrchestratorTestFaults {
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

        append_turn_direct("fail-open-retrieval", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        append_turn_direct("fail-open-retrieval", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        append_turn_direct("fail-open-retrieval", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let hydrated = hydrate_memory_context("fail-open-retrieval", &config)
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
    fn fail_open_memory_strict_mode_remains_reserved_and_disabled_by_default() {
        let _faults = ScopedMemoryOrchestratorTestFaults::set(MemoryOrchestratorTestFaults {
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

        append_turn_direct("fail-open-strict-reserved", "assistant", "turn 1", &config)
            .expect("append turn should succeed");

        let hydrated = hydrate_memory_context("fail-open-strict-reserved", &config)
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
