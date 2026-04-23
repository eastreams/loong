use loong_contracts::{MemoryCoreOutcome, MemoryCoreRequest};
use serde_json::{Value, json};

use crate::config::MemoryMode;
use crate::runtime_identity;

#[cfg(feature = "memory-sqlite")]
use super::sqlite;
use super::{
    DerivedMemoryKind, MEMORY_OP_READ_CONTEXT, MEMORY_OP_READ_STAGE_ENVELOPE, MemoryAuthority,
    MemoryContextProvenance, MemoryProvenanceSourceKind, MemoryRecallMode, MemoryRecordStatus,
    MemoryScope, MemoryTrustLevel, encode_stage_envelope_payload,
    orchestrator::hydrate_stage_envelope_with_workspace_root,
    protocol::{MemoryContextEntry, MemoryContextKind},
    runtime_config::MemoryRuntimeConfig,
};

pub(crate) fn read_context(
    request: MemoryCoreRequest,
    config: &MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    let (session_id, envelope) =
        read_stage_envelope_from_request_payload(&request, MEMORY_OP_READ_CONTEXT, config)?;
    let entries = envelope.hydrated.entries;

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

fn read_context_workspace_root(
    payload: &serde_json::Map<String, Value>,
    operation: &str,
) -> Result<Option<std::path::PathBuf>, String> {
    payload
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
            Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => Err(format!(
                "{operation} payload.workspace_root must be a string or null"
            )),
        })
        .transpose()
        .map(Option::flatten)
}

pub(crate) fn read_stage_envelope(
    request: MemoryCoreRequest,
    config: &MemoryRuntimeConfig,
) -> Result<MemoryCoreOutcome, String> {
    let (session_id, envelope) =
        read_stage_envelope_from_request_payload(&request, MEMORY_OP_READ_STAGE_ENVELOPE, config)?;
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

fn read_stage_envelope_from_request_payload<'a>(
    request: &'a MemoryCoreRequest,
    operation: &str,
    config: &MemoryRuntimeConfig,
) -> Result<(&'a str, super::StageEnvelope), String> {
    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| format!("{operation} payload must be an object"))?;
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{operation} requires payload.session_id"))?;
    let workspace_root = read_context_workspace_root(payload, operation)?;
    let envelope =
        hydrate_stage_envelope_with_workspace_root(session_id, workspace_root.as_deref(), config)?;

    Ok((session_id, envelope))
}

pub fn load_prompt_context(
    session_id: &str,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<MemoryContextEntry>, String> {
    let mut entries = Vec::new();
    let profile_entry = build_profile_entry(config);
    if let Some(profile_entry) = profile_entry {
        entries.push(profile_entry);
    }
    let selected_system_id = super::selected_prompt_hydration_system_id(config);

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
                provenance: vec![
                    MemoryContextProvenance::new(
                        selected_system_id.as_str(),
                        MemoryProvenanceSourceKind::SummaryCheckpoint,
                        Some("summary_checkpoint".to_owned()),
                        None,
                        Some(MemoryScope::Session),
                        MemoryRecallMode::PromptAssembly,
                    )
                    .with_trust_level(MemoryTrustLevel::Derived)
                    .with_authority(MemoryAuthority::Advisory)
                    .with_derived_kind(DerivedMemoryKind::Summary)
                    .with_record_status(MemoryRecordStatus::Active),
                ],
            });
        }
        for turn in snapshot.window_turns {
            let turn_role = turn.role;
            let turn_content = turn.content;
            let provenance = MemoryContextProvenance::new(
                selected_system_id.as_str(),
                MemoryProvenanceSourceKind::RecentWindowTurn,
                Some("recent_window_turn".to_owned()),
                None,
                Some(MemoryScope::Session),
                MemoryRecallMode::PromptAssembly,
            )
            .with_trust_level(MemoryTrustLevel::Session)
            .with_authority(MemoryAuthority::Advisory)
            .with_record_status(MemoryRecordStatus::Active);
            entries.push(MemoryContextEntry {
                kind: MemoryContextKind::Turn,
                role: turn_role,
                content: turn_content,
                provenance: vec![provenance],
            });
        }
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = session_id;
    }

    Ok(entries)
}

pub(super) fn build_profile_entry(config: &MemoryRuntimeConfig) -> Option<MemoryContextEntry> {
    let profile_plus_window_mode = matches!(config.mode, MemoryMode::ProfilePlusWindow);
    if !profile_plus_window_mode {
        return None;
    }

    let profile_note = config.profile_note.as_deref();
    let personalization = config.personalization.as_ref();
    let profile_section =
        runtime_identity::render_session_profile_section(profile_note, personalization)?;

    Some(MemoryContextEntry {
        kind: MemoryContextKind::Profile,
        role: "system".to_owned(),
        content: profile_section,
        provenance: vec![
            MemoryContextProvenance::new(
                super::selected_prompt_hydration_system_id(config).as_str(),
                MemoryProvenanceSourceKind::ProfileNote,
                Some("profile_note".to_owned()),
                None,
                Some(MemoryScope::Session),
                MemoryRecallMode::PromptAssembly,
            )
            .with_trust_level(MemoryTrustLevel::Derived)
            .with_authority(MemoryAuthority::Advisory)
            .with_derived_kind(DerivedMemoryKind::Profile)
            .with_record_status(MemoryRecordStatus::Active),
        ],
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    #[cfg(feature = "memory-sqlite")]
    use crate::config::{MemoryProfile, MemorySystemKind};
    use crate::memory::{
        build_read_stage_envelope_request, build_read_stage_envelope_request_with_workspace_root,
        decode_memory_context_entries, decode_stage_envelope,
    };

    #[cfg(feature = "memory-sqlite")]
    fn sqlite_memory_config(db_path: std::path::PathBuf) -> MemoryRuntimeConfig {
        MemoryRuntimeConfig::for_sqlite_path(db_path)
    }

    #[cfg(feature = "memory-sqlite")]
    fn sqlite_memory_config_with_profile(
        db_path: std::path::PathBuf,
        profile: MemoryProfile,
        sliding_window: usize,
    ) -> MemoryRuntimeConfig {
        let mut config = sqlite_memory_config(db_path);
        config.profile = profile;
        config.mode = profile.mode();
        config.sliding_window = sliding_window;
        config
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn window_plus_summary_includes_condensed_older_context() {
        let tmp = std::env::temp_dir().join(format!("loong-summary-memory-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("summary.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);

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
        let summary_entry = hydrated
            .iter()
            .find(|entry| entry.kind == MemoryContextKind::Summary)
            .expect("summary entry");
        assert_eq!(summary_entry.provenance.len(), 1);
        assert_eq!(
            summary_entry.provenance[0].source_kind,
            MemoryProvenanceSourceKind::SummaryCheckpoint
        );
        assert_eq!(
            summary_entry.provenance[0].source_label.as_deref(),
            Some("summary_checkpoint")
        );
        assert_eq!(
            summary_entry.provenance[0].scope,
            Some(MemoryScope::Session)
        );
        assert_eq!(
            summary_entry.provenance[0].record_status,
            Some(MemoryRecordStatus::Active)
        );
        let turn_entry = hydrated
            .iter()
            .find(|entry| entry.kind == MemoryContextKind::Turn)
            .expect("turn entry");
        assert_eq!(turn_entry.provenance.len(), 1);
        assert_eq!(
            turn_entry.provenance[0].source_kind,
            MemoryProvenanceSourceKind::RecentWindowTurn
        );
        assert_eq!(
            turn_entry.provenance[0].source_label.as_deref(),
            Some("recent_window_turn")
        );
        assert_eq!(
            turn_entry.provenance[0].record_status,
            Some(MemoryRecordStatus::Active)
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn profile_plus_window_includes_profile_note_block() {
        let tmp = std::env::temp_dir().join(format!("loong-profile-memory-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("profile.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let mut config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::ProfilePlusWindow, 2);
        config.profile_note = Some("Imported ZeroClaw preferences".to_owned());

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
        let profile_entry = hydrated
            .iter()
            .find(|entry| entry.kind == MemoryContextKind::Profile)
            .expect("profile entry");
        assert_eq!(profile_entry.provenance.len(), 1);
        assert_eq!(
            profile_entry.provenance[0].source_kind,
            MemoryProvenanceSourceKind::ProfileNote
        );
        assert_eq!(
            profile_entry.provenance[0].source_label.as_deref(),
            Some("profile_note")
        );
        assert_eq!(
            profile_entry.provenance[0].scope,
            Some(MemoryScope::Session)
        );
        assert_eq!(
            profile_entry.provenance[0].record_status,
            Some(MemoryRecordStatus::Active)
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn profile_plus_window_includes_typed_personalization_section() {
        let tmp = std::env::temp_dir().join(format!(
            "loong-personalization-memory-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("personalization.sqlite3");
        let _ = std::fs::remove_file(&db_path);
        let default_personalization = crate::config::PersonalizationConfig::default();
        let schema_version = default_personalization.schema_version;
        let personalization = crate::config::PersonalizationConfig {
            preferred_name: Some("Chum".to_owned()),
            response_density: Some(crate::config::ResponseDensity::Thorough),
            initiative_level: Some(crate::config::InitiativeLevel::HighInitiative),
            standing_boundaries: Some("Ask before destructive actions.".to_owned()),
            timezone: Some("Asia/Shanghai".to_owned()),
            locale: None,
            prompt_state: crate::config::PersonalizationPromptState::Configured,
            schema_version,
            updated_at_epoch_seconds: Some(1_775_095_200),
        };
        let mut config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::ProfilePlusWindow, 2);
        config.personalization = Some(personalization);

        super::super::append_turn_direct("personalization-session", "user", "recent turn", &config)
            .expect("append turn should succeed");

        let hydrated =
            load_prompt_context("personalization-session", &config).expect("load prompt context");
        let profile_entry = hydrated
            .iter()
            .find(|entry| entry.kind == MemoryContextKind::Profile)
            .expect("profile entry");
        let profile_content = profile_entry.content.as_str();

        assert!(profile_content.contains("## Session Profile"));
        assert!(profile_content.contains("Preferred name: Chum"));
        assert!(profile_content.contains("Response density: thorough"));
        assert!(profile_content.contains("Initiative level: high_initiative"));
        assert!(profile_content.contains("Ask before destructive actions."));
        assert!(profile_content.contains("Timezone: Asia/Shanghai"));
        assert!(!profile_content.contains("## Resolved Runtime Identity"));

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn window_only_ignores_typed_personalization_section() {
        let tmp = std::env::temp_dir().join(format!(
            "loong-window-only-personalization-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("window-only-personalization.sqlite3");
        let _ = std::fs::remove_file(&db_path);
        let default_personalization = crate::config::PersonalizationConfig::default();
        let schema_version = default_personalization.schema_version;
        let personalization = crate::config::PersonalizationConfig {
            preferred_name: Some("Chum".to_owned()),
            response_density: Some(crate::config::ResponseDensity::Balanced),
            initiative_level: Some(crate::config::InitiativeLevel::AskBeforeActing),
            standing_boundaries: Some("Ask before destructive actions.".to_owned()),
            timezone: Some("Asia/Shanghai".to_owned()),
            locale: None,
            prompt_state: crate::config::PersonalizationPromptState::Configured,
            schema_version,
            updated_at_epoch_seconds: Some(1_775_095_200),
        };
        let mut config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);
        config.personalization = Some(personalization);

        super::super::append_turn_direct(
            "window-only-personalization-session",
            "user",
            "recent turn",
            &config,
        )
        .expect("append turn should succeed");

        let hydrated = load_prompt_context("window-only-personalization-session", &config)
            .expect("load prompt context");
        let has_profile_entry = hydrated
            .iter()
            .any(|entry| entry.kind == MemoryContextKind::Profile);

        assert!(
            !has_profile_entry,
            "window-only should not project personalization"
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn profile_plus_window_omits_legacy_identity_blocks_from_profile_projection() {
        let tmp = std::env::temp_dir().join(format!(
            "loong-profile-memory-projection-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("profile-projection.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let profile_note = "## Imported IDENTITY.md\n# Identity\n\n- Name: Legacy build copilot\n\n## Imported External Skills Artifacts\n- kind=skills_catalog\n- declared=custom/skill-a";
        let mut config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::ProfilePlusWindow, 2);
        config.profile_note = Some(profile_note.to_owned());

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
        let tmp = std::env::temp_dir().join(format!(
            "loong-profile-memory-identity-only-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("profile-identity-only.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let profile_note = "## Imported IDENTITY.md\n# Identity\n\n- Name: Legacy build copilot";
        let mut config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::ProfilePlusWindow, 2);
        config.profile_note = Some(profile_note.to_owned());

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
        let tmp =
            std::env::temp_dir().join(format!("loong-read-context-memory-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("read-context.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);

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
        let maybe_summary_entry = entries
            .iter()
            .find(|entry| entry.get("kind") == Some(&json!("summary")));
        let summary_entry = maybe_summary_entry.expect("summary entry");
        assert_eq!(
            summary_entry["provenance"][0]["source_label"],
            "summary_checkpoint"
        );
        assert_eq!(summary_entry["provenance"][0]["record_status"], "active");

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn read_stage_envelope_operation_serializes_hydrated_entries_and_diagnostics() {
        let tmp = std::env::temp_dir().join(format!(
            "loong-read-stage-envelope-memory-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("read-stage-envelope.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config =
            sqlite_memory_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);

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
        let maybe_summary_entry = envelope
            .hydrated
            .entries
            .iter()
            .find(|entry| entry.kind == MemoryContextKind::Summary);
        let summary_entry = maybe_summary_entry.expect("summary entry");
        assert_eq!(
            summary_entry.provenance[0].source_label.as_deref(),
            Some("summary_checkpoint")
        );
        assert_eq!(
            summary_entry.provenance[0].record_status,
            Some(MemoryRecordStatus::Active)
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn execute_memory_core_dispatches_read_stage_envelope_operation() {
        let tmp = std::env::temp_dir().join(format!(
            "loong-dispatch-stage-envelope-memory-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        let db_path = tmp.join("dispatch-stage-envelope.sqlite3");
        let _ = std::fs::remove_file(&db_path);

        let config = sqlite_memory_config(db_path.clone());

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
    fn read_context_operation_projects_entries_from_stage_envelope() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let workspace_root = temp_dir.path();
        let curated_memory_path = workspace_root.join("MEMORY.md");

        std::fs::write(
            &curated_memory_path,
            "# Durable Notes\n\nRemember the deploy freeze window.\n",
        )
        .expect("write durable recall");

        let db_path = workspace_root.join("read-context-stage-envelope.sqlite3");
        let config =
            sqlite_memory_config_with_profile(db_path, MemoryProfile::WindowPlusSummary, 2);

        super::super::append_turn_direct(
            "read-context-envelope-session",
            "user",
            "turn 1",
            &config,
        )
        .expect("append turn 1 should succeed");
        super::super::append_turn_direct(
            "read-context-envelope-session",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");
        super::super::append_turn_direct(
            "read-context-envelope-session",
            "user",
            "deploy freeze timing",
            &config,
        )
        .expect("append turn 3 should succeed");

        let read_context_request = MemoryCoreRequest {
            operation: MEMORY_OP_READ_CONTEXT.to_owned(),
            payload: json!({
                "session_id": "read-context-envelope-session",
                "workspace_root": workspace_root,
            }),
        };
        let read_context_outcome =
            super::super::execute_memory_core_with_config(read_context_request, &config)
                .expect("read_context should succeed");
        let context_entries = decode_memory_context_entries(&read_context_outcome.payload);

        let staged_outcome = super::super::execute_memory_core_with_config(
            build_read_stage_envelope_request_with_workspace_root(
                "read-context-envelope-session",
                Some(workspace_root),
            ),
            &config,
        )
        .expect("read_stage_envelope should succeed");
        let staged_envelope =
            decode_stage_envelope(&staged_outcome.payload).expect("decode staged envelope");

        assert_eq!(context_entries, staged_envelope.hydrated.entries);
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
        let config = sqlite_memory_config(db_path);

        let outcome = super::super::execute_memory_core_with_config(
            build_read_stage_envelope_request_with_workspace_root(
                "durable-recall-stage-envelope-session",
                Some(workspace_root),
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

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn read_stage_envelope_operation_ignores_request_level_memory_form_overrides() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = temp_dir
            .path()
            .join("read-stage-envelope-request-overrides.sqlite3");
        let config = sqlite_memory_config_with_profile(db_path, MemoryProfile::WindowOnly, 2);

        super::super::append_turn_direct(
            "read-stage-envelope-ignore-overrides",
            "user",
            "turn 1",
            &config,
        )
        .expect("append turn 1 should succeed");
        super::super::append_turn_direct(
            "read-stage-envelope-ignore-overrides",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");
        super::super::append_turn_direct(
            "read-stage-envelope-ignore-overrides",
            "user",
            "turn 3",
            &config,
        )
        .expect("append turn 3 should succeed");

        let outcome = super::super::execute_memory_core_with_config(
            MemoryCoreRequest {
                operation: MEMORY_OP_READ_STAGE_ENVELOPE.to_owned(),
                payload: json!({
                    "session_id": "read-stage-envelope-ignore-overrides",
                    "profile": "window_plus_summary",
                    "system": MemorySystemKind::RecallFirst.as_str(),
                    "system_id": MemorySystemKind::RecallFirst.as_str(),
                    "sliding_window": 64,
                    "summary_max_chars": 4096,
                    "profile_note": "request level profile note should be ignored",
                }),
            },
            &config,
        )
        .expect("read_stage_envelope operation should succeed");

        let envelope = decode_stage_envelope(&outcome.payload).expect("decode staged envelope");
        assert!(
            envelope
                .hydrated
                .entries
                .iter()
                .all(|entry| entry.kind != MemoryContextKind::Summary),
            "request-level summary overrides should not change the canonical staged memory form"
        );
        assert!(
            envelope
                .hydrated
                .entries
                .iter()
                .all(|entry| entry.kind != MemoryContextKind::Profile),
            "request-level profile overrides should not change the canonical staged memory form"
        );
        assert_eq!(
            envelope.hydrated.diagnostics.system_id,
            super::super::selected_prompt_hydration_system_id(&config),
            "request-level system overrides should not change the canonical staged memory form"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn read_context_operation_ignores_request_level_memory_form_overrides() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = temp_dir
            .path()
            .join("read-context-request-overrides.sqlite3");
        let config = sqlite_memory_config_with_profile(db_path, MemoryProfile::WindowOnly, 2);

        super::super::append_turn_direct(
            "read-context-ignore-overrides",
            "user",
            "turn 1",
            &config,
        )
        .expect("append turn 1 should succeed");
        super::super::append_turn_direct(
            "read-context-ignore-overrides",
            "assistant",
            "turn 2",
            &config,
        )
        .expect("append turn 2 should succeed");
        super::super::append_turn_direct(
            "read-context-ignore-overrides",
            "user",
            "turn 3",
            &config,
        )
        .expect("append turn 3 should succeed");

        let outcome = super::super::execute_memory_core_with_config(
            MemoryCoreRequest {
                operation: MEMORY_OP_READ_CONTEXT.to_owned(),
                payload: json!({
                    "session_id": "read-context-ignore-overrides",
                    "profile": "window_plus_summary",
                    "system": MemorySystemKind::RecallFirst.as_str(),
                    "system_id": MemorySystemKind::RecallFirst.as_str(),
                    "sliding_window": 64,
                    "summary_max_chars": 4096,
                    "profile_note": "request level profile note should be ignored",
                }),
            },
            &config,
        )
        .expect("read_context operation should succeed");

        let entries = decode_memory_context_entries(&outcome.payload);
        assert!(
            entries
                .iter()
                .all(|entry| entry.kind != MemoryContextKind::Summary),
            "request-level summary overrides should not change the canonical runtime memory form"
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry.kind != MemoryContextKind::Profile),
            "request-level profile overrides should not change the canonical runtime memory form"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn load_prompt_context_uses_selected_memory_system_id_in_provenance() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("selected-system.sqlite3");

        let mut config = sqlite_memory_config_with_profile(db_path, MemoryProfile::WindowOnly, 10);
        config.resolved_system_id =
            Some(crate::memory::WORKSPACE_RECALL_MEMORY_SYSTEM_ID.to_owned());

        super::super::append_turn_direct("selected-system-session", "user", "hello", &config)
            .expect("append turn should succeed");

        let entries =
            load_prompt_context("selected-system-session", &config).expect("load prompt context");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].provenance.len(), 1);
        assert_eq!(
            entries[0].provenance[0].memory_system_id,
            crate::memory::WORKSPACE_RECALL_MEMORY_SYSTEM_ID
        );
    }
}
