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
    pub recent_window_count: usize,
    pub entry_count: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BuiltinMemoryOrchestrator;

impl BuiltinMemoryOrchestrator {
    pub fn hydrate(
        &self,
        session_id: &str,
        config: &MemoryRuntimeConfig,
    ) -> Result<HydratedMemoryContext, String> {
        let entries = load_prompt_context(session_id, config)?;
        let recent_window = recent_window_records(session_id, config)?;
        let diagnostics = MemoryDiagnostics {
            system_id: config.system.as_str(),
            fail_open: config.fail_open,
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
}
