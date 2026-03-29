use loongclaw_contracts::{MemoryCoreOutcome, MemoryCoreRequest};
use serde_json::{Value, json};

use crate::config::MemoryMode;
use crate::config::MemoryProfile;
use crate::runtime_identity;

#[cfg(feature = "memory-sqlite")]
use super::sqlite;
use super::{
    MEMORY_OP_READ_CONTEXT, MEMORY_OP_READ_STAGE_ENVELOPE, encode_stage_envelope_payload,
    orchestrator::hydrate_stage_envelope_with_workspace_root,
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
    let runtime_config = read_context_runtime_config(payload, config)?;
    let entries = load_prompt_context(session_id, &runtime_config)?;

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

fn read_context_runtime_config(
    payload: &serde_json::Map<String, Value>,
    config: &MemoryRuntimeConfig,
) -> Result<MemoryRuntimeConfig, String> {
    let mut runtime_config = config.clone();

    if let Some(profile_value) = payload.get("profile") {
        let profile_text = profile_value
            .as_str()
            .ok_or_else(|| "memory.read_context payload.profile must be a string".to_owned())?;
        let profile = MemoryProfile::parse_id(profile_text).ok_or_else(|| {
            format!("memory.read_context payload.profile `{profile_text}` is unsupported")
        })?;
        let mode = profile.mode();

        runtime_config.profile = profile;
        runtime_config.mode = mode;
    }

    if let Some(sliding_window_value) = payload.get("sliding_window") {
        let sliding_window = sliding_window_value.as_u64().ok_or_else(|| {
            "memory.read_context payload.sliding_window must be a positive integer".to_owned()
        })?;
        let sliding_window = usize::try_from(sliding_window).map_err(|conversion_error| {
            format!("memory.read_context payload.sliding_window exceeds usize: {conversion_error}")
        })?;
        if sliding_window == 0 {
            return Err("memory.read_context payload.sliding_window must be at least 1".to_owned());
        }

        runtime_config.sliding_window = sliding_window;
    }

    if let Some(summary_max_chars_value) = payload.get("summary_max_chars") {
        let summary_max_chars = summary_max_chars_value.as_u64().ok_or_else(|| {
            "memory.read_context payload.summary_max_chars must be a positive integer".to_owned()
        })?;
        let summary_max_chars = usize::try_from(summary_max_chars).map_err(|conversion_error| {
            format!(
                "memory.read_context payload.summary_max_chars exceeds usize: {conversion_error}"
            )
        })?;
        if summary_max_chars == 0 {
            return Err(
                "memory.read_context payload.summary_max_chars must be at least 1".to_owned(),
            );
        }

        runtime_config.summary_max_chars = summary_max_chars;
    }

    if let Some(profile_note_value) = payload.get("profile_note") {
        let profile_note = match profile_note_value {
            Value::Null => None,
            Value::String(value) => {
                let trimmed_value = value.trim();
                if trimmed_value.is_empty() {
                    None
                } else {
                    Some(trimmed_value.to_owned())
                }
            }
            Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
                return Err(
                    "memory.read_context payload.profile_note must be a string or null".to_owned(),
                );
            }
        };

        runtime_config.profile_note = profile_note;
    }

    Ok(runtime_config)
}

pub(crate) fn read_stage_envelope(
    request: MemoryCoreRequest,
    config: &MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "memory.read_stage_envelope payload must be an object".to_owned())?;
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "memory.read_stage_envelope requires payload.session_id".to_owned())?;
    let runtime_config = read_context_runtime_config(payload, config)?;
    let workspace_root = payload
        .get("workspace_root")
        .map(|value| match value {
            Value::Null => Ok(None),
            Value::String(raw_path) => {
                let trimmed_path = raw_path.trim();
                if trimmed_path.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(std::path::PathBuf::from(trimmed_path)))
                }
            }
            Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => Err(
                "memory.read_stage_envelope payload.workspace_root must be a string or null"
                    .to_owned(),
            ),
        })
        .transpose()?
        .flatten();
    let envelope = hydrate_stage_envelope_with_workspace_root(
        session_id,
        workspace_root.as_deref(),
        &runtime_config,
    )?;
    let mut response_payload = encode_stage_envelope_payload(&envelope);

    if let Some(map) = response_payload.as_object_mut() {
        map.insert("adapter".to_owned(), json!("sqlite-core"));
        map.insert("operation".to_owned(), json!(MEMORY_OP_READ_STAGE_ENVELOPE));
        map.insert("session_id".to_owned(), json!(session_id));
    }

    Ok(MemoryCoreOutcome {
        status: "ok".to_owned(),
        payload: response_payload,
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
    use crate::memory::{
        build_read_stage_envelope_request, build_read_stage_envelope_request_with_workspace_root,
        decode_stage_envelope,
    };

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

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn read_stage_envelope_operation_serializes_hydrated_entries_and_diagnostics() {
        use crate::config::{MemoryMode, MemoryProfile};

        let tmp = std::env::temp_dir().join(format!(
            "loongclaw-read-stage-envelope-memory-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("read-stage-envelope.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = MemoryRuntimeConfig {
            profile: MemoryProfile::WindowPlusSummary,
            mode: MemoryMode::WindowPlusSummary,
            sqlite_path: Some(db_path.clone()),
            sliding_window: 2,
            ..MemoryRuntimeConfig::default()
        };

        super::super::append_turn_direct("read-stage-envelope-session", "user", "turn 1", &config)
            .expect("append turn 1 should succeed");
        super::super::append_turn_direct(
            "read-stage-envelope-session",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");
        super::super::append_turn_direct("read-stage-envelope-session", "user", "turn 3", &config)
            .expect("append turn 3 should succeed");

        let outcome = super::super::execute_memory_core_with_config(
            build_read_stage_envelope_request("read-stage-envelope-session"),
            &config,
        )
        .expect("read_stage_envelope should succeed");

        let envelope = decode_stage_envelope(&outcome.payload).expect("decode staged envelope");
        assert!(!envelope.hydrated.entries.is_empty());
        assert!(!envelope.diagnostics.is_empty());
        assert_eq!(envelope.hydrated.diagnostics.system_id, "builtin");

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn execute_memory_core_dispatches_read_stage_envelope_operation() {
        let tmp = std::env::temp_dir().join(format!(
            "loongclaw-dispatch-stage-envelope-memory-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("dispatch-stage-envelope.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = MemoryRuntimeConfig {
            sqlite_path: Some(db_path.clone()),
            ..MemoryRuntimeConfig::default()
        };

        let outcome = super::super::execute_memory_core_with_config(
            build_read_stage_envelope_request("dispatch-session"),
            &config,
        )
        .expect("dispatch read_stage_envelope");

        assert_eq!(outcome.status, "ok");
        assert_eq!(
            outcome.payload["operation"],
            json!(MEMORY_OP_READ_STAGE_ENVELOPE)
        );
        assert!(decode_stage_envelope(&outcome.payload).is_some());

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn read_stage_envelope_operation_preserves_durable_recall_with_workspace_root() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let curated_memory_path = workspace_root.join("MEMORY.md");

        std::fs::write(
            &curated_memory_path,
            "# Durable Notes\n\nRemember the deploy freeze window.\n",
        )
        .expect("write durable recall");

        let db_path = workspace_root.join("stage-envelope-durable-recall.sqlite3");
        let config = MemoryRuntimeConfig {
            sqlite_path: Some(db_path),
            ..MemoryRuntimeConfig::default()
        };

        let outcome = super::super::execute_memory_core_with_config(
            build_read_stage_envelope_request_with_workspace_root(
                "durable-recall-stage-envelope-session",
                Some(workspace_root),
                &config,
            ),
            &config,
        )
        .expect("read_stage_envelope should preserve durable recall");

        let envelope = decode_stage_envelope(&outcome.payload).expect("decode staged envelope");
        let has_durable_recall = envelope.hydrated.entries.iter().any(|entry| {
            entry.kind == MemoryContextKind::RetrievedMemory
                && entry.content.contains("Remember the deploy freeze window.")
        });

        assert!(
            has_durable_recall,
            "expected staged envelope payload to keep workspace durable recall"
        );
    }
}
