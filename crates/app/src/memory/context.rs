use loongclaw_contracts::{MemoryCoreOutcome, MemoryCoreRequest};
use serde_json::{Value, json};

use crate::config::MemoryMode;
use crate::runtime_identity;

#[cfg(feature = "memory-sqlite")]
use super::sqlite;
use super::{
    MEMORY_OP_READ_CONTEXT,
    protocol::{MemoryContextEntry, MemoryContextKind},
    runtime_config::MemoryRuntimeConfig,
};

pub(crate) fn read_context(
    request: MemoryCoreRequest,
    config: &MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "memory.read_context payload must be an object".to_owned())?;
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "memory.read_context requires payload.session_id".to_owned())?;
    let entries = load_prompt_context(session_id, config)?;

    Ok(MemoryCoreOutcome {
        status: "ok".to_owned(),
        payload: json!({
            "adapter": "sqlite-core",
            "operation": MEMORY_OP_READ_CONTEXT,
            "session_id": session_id,
            "entries": entries,
        }),
    })
}

pub fn load_prompt_context(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<MemoryContextEntry>, String> {
    let mut entries = Vec::new();

    let profile_section =
        runtime_identity::render_session_profile_section(config.profile_note.as_deref());
    if matches!(config.mode, MemoryMode::ProfilePlusWindow)
        && let Some(profile_section) = profile_section
    {
        entries.push(MemoryContextEntry {
            kind: MemoryContextKind::Profile,
            role: "system".to_owned(),
            content: profile_section,
        });
    }

    #[cfg(feature = "memory-sqlite")]
    {
        let snapshot = sqlite::load_context_snapshot(session_id, config)?;
        if matches!(config.mode, MemoryMode::WindowPlusSummary)
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
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = session_id;
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn window_plus_summary_includes_condensed_older_context() {
        use crate::config::{MemoryMode, MemoryProfile};

        let tmp =
            std::env::temp_dir().join(format!("loongclaw-summary-memory-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("summary.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..MemoryRuntimeConfig::default()
        };

        super::super::append_turn_direct("summary-session", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        super::super::append_turn_direct("summary-session", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        super::super::append_turn_direct("summary-session", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");
        super::super::append_turn_direct("summary-session", "assistant", "turn 4", &config)
            .expect("append turn 4 should succeed");

        let hydrated =
            load_prompt_context("summary-session", &config).expect("load prompt context");

        assert!(
            hydrated
                .iter()
                .any(|entry| entry.kind == MemoryContextKind::Summary),
            "expected a summary entry"
        );
        assert!(
            hydrated
                .iter()
                .any(|entry| entry.content.contains("turn 1")),
            "expected summary to mention older turns"
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn profile_plus_window_includes_profile_note_block() {
        use crate::config::{MemoryMode, MemoryProfile};

        let tmp =
            std::env::temp_dir().join(format!("loongclaw-profile-memory-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("profile.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = MemoryRuntimeConfig {
            profile: MemoryProfile::ProfilePlusWindow,
            mode: MemoryMode::ProfilePlusWindow,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            profile_note: Some("Imported ZeroClaw preferences".to_owned()),
            ..MemoryRuntimeConfig::default()
        };

        super::super::append_turn_direct("profile-session", "user", "recent turn", &config)
            .expect("append turn should succeed");

        let hydrated =
            load_prompt_context("profile-session", &config).expect("load prompt context");

        assert!(
            hydrated
                .iter()
                .any(|entry| entry.kind == MemoryContextKind::Profile),
            "expected a profile entry"
        );
        assert!(
            hydrated
                .iter()
                .any(|entry| entry.content.contains("Imported ZeroClaw preferences")),
            "expected profile note content"
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn profile_plus_window_omits_legacy_identity_blocks_from_profile_projection() {
        use crate::config::{MemoryMode, MemoryProfile};

        let tmp = std::env::temp_dir().join(format!(
            "loongclaw-profile-memory-projection-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("profile-projection.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let profile_note = "## Imported IDENTITY.md\n# Identity\n\n- Name: Legacy build copilot\n\n## Imported External Skills Artifacts\n- kind=skills_catalog\n- declared=custom/skill-a";
        let config = MemoryRuntimeConfig {
            profile: MemoryProfile::ProfilePlusWindow,
            mode: MemoryMode::ProfilePlusWindow,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            profile_note: Some(profile_note.to_owned()),
            ..MemoryRuntimeConfig::default()
        };

        super::super::append_turn_direct(
            "profile-projection-session",
            "user",
            "recent turn",
            &config,
        )
        .expect("append turn should succeed");

        let hydrated = load_prompt_context("profile-projection-session", &config)
            .expect("load prompt context");
        let profile_entry = hydrated
            .iter()
            .find(|entry| entry.kind == MemoryContextKind::Profile)
            .expect("profile entry");

        assert!(
            profile_entry
                .content
                .contains("Imported External Skills Artifacts")
        );
        assert!(!profile_entry.content.contains("Legacy build copilot"));

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn profile_plus_window_drops_profile_entry_when_only_legacy_identity_exists() {
        use crate::config::{MemoryMode, MemoryProfile};

        let tmp = std::env::temp_dir().join(format!(
            "loongclaw-profile-memory-identity-only-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("profile-identity-only.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let profile_note = "## Imported IDENTITY.md\n# Identity\n\n- Name: Legacy build copilot";
        let config = MemoryRuntimeConfig {
            profile: MemoryProfile::ProfilePlusWindow,
            mode: MemoryMode::ProfilePlusWindow,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            profile_note: Some(profile_note.to_owned()),
            ..MemoryRuntimeConfig::default()
        };

        super::super::append_turn_direct(
            "profile-identity-only-session",
            "user",
            "recent turn",
            &config,
        )
        .expect("append turn should succeed");

        let hydrated = load_prompt_context("profile-identity-only-session", &config)
            .expect("load prompt context");
        let profile_entries = hydrated
            .iter()
            .filter(|entry| entry.kind == MemoryContextKind::Profile)
            .count();

        assert_eq!(profile_entries, 0);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn read_context_operation_serializes_prompt_context_entries() {
        use crate::config::{MemoryMode, MemoryProfile};

        let tmp = std::env::temp_dir().join(format!(
            "loongclaw-read-context-memory-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("read-context.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..MemoryRuntimeConfig::default()
        };

        super::super::append_turn_direct("read-context-session", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        super::super::append_turn_direct("read-context-session", "assistant", "turn 2", &config)
            .expect("append turn 2 should succeed");
        super::super::append_turn_direct("read-context-session", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let outcome = super::super::execute_memory_core_with_config(
            MemoryCoreRequest {
                operation: MEMORY_OP_READ_CONTEXT.to_owned(),
                payload: json!({
                    "session_id": "read-context-session",
                }),
            },
            &config,
        )
        .expect("read_context operation should succeed");

        let entries = outcome
            .payload
            .get("entries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            !entries.is_empty(),
            "expected read_context payload to include serialized entries"
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.get("kind") == Some(&json!("summary"))),
            "expected read_context payload to include a summary entry"
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }
}
