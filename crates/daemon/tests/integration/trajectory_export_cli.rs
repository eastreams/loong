#![allow(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks
)]

use super::*;
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}

fn write_trajectory_export_config(root: &Path) -> PathBuf {
    fs::create_dir_all(root).expect("create fixture root");

    let mut config = mvp::config::LoongClawConfig::default();
    config.tools.file_root = Some(root.display().to_string());
    config.memory.sqlite_path = root.join("memory.sqlite3").display().to_string();
    let config_path = root.join("loongclaw.toml");
    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write config fixture");
    config_path
}

#[test]
fn collect_trajectory_export_artifact_includes_turns_and_events() {
    let root = unique_temp_dir("loongclaw-trajectory-export");
    let config_path = write_trajectory_export_config(&root);
    let (_, config) = mvp::config::load(Some(
        config_path
            .to_str()
            .expect("config path should be valid utf-8"),
    ))
    .expect("load config fixture");
    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = mvp::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");

    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: mvp::session::repository::SessionState::Completed,
    })
    .expect("create root session");
    mvp::memory::append_turn_direct("root-session", "user", "hello", &memory_config)
        .expect("append user turn");
    mvp::memory::append_turn_direct("root-session", "assistant", "world", &memory_config)
        .expect("append assistant turn");
    repo.append_event(mvp::session::repository::NewSessionEvent {
        session_id: "root-session".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: serde_json::json!({"mode": "async"}),
    })
    .expect("append session event");

    let (_resolved_path, artifact) = collect_trajectory_export_artifact(
        Some(config_path.to_string_lossy().as_ref()),
        Some("root-session"),
    )
    .expect("collect trajectory artifact");

    assert_eq!(
        artifact.schema.version,
        TRAJECTORY_EXPORT_ARTIFACT_JSON_SCHEMA_VERSION
    );
    assert_eq!(artifact.session.session_id, "root-session");
    assert_eq!(artifact.turns.len(), 2);
    assert_eq!(artifact.turns[0].role, "user");
    assert_eq!(artifact.turns[1].role, "assistant");
    assert_eq!(artifact.events.len(), 1);
    assert_eq!(artifact.events[0].event_kind, "delegate_started");
}

#[test]
fn load_trajectory_export_artifact_rejects_wrong_schema_surface() {
    let root = unique_temp_dir("loongclaw-trajectory-inspect-invalid");
    fs::create_dir_all(&root).expect("create fixture root");
    let artifact_path = root.join("trajectory.json");
    fs::write(
        &artifact_path,
        serde_json::to_string_pretty(&json!({
            "schema": {
                "version": TRAJECTORY_EXPORT_ARTIFACT_JSON_SCHEMA_VERSION,
                "surface": "wrong_surface",
                "purpose": "session_replay_evidence"
            },
            "exported_at": "2026-04-05T00:00:00Z",
            "config": "/tmp/loongclaw.toml",
            "session": {
                "session_id": "root-session",
                "kind": "root",
                "parent_session_id": null,
                "label": "Root",
                "state": "completed",
                "created_at": 1,
                "updated_at": 2,
                "archived_at": null,
                "turn_count": 1,
                "last_turn_at": 2,
                "last_error": null
            },
            "turns": [],
            "events": []
        }))
        .expect("encode invalid artifact"),
    )
    .expect("write invalid artifact");

    let error = load_trajectory_export_artifact(
        artifact_path
            .to_str()
            .expect("artifact path should be valid utf-8"),
    )
    .expect_err("wrong schema surface should fail");

    assert!(error.contains("unsupported schema surface"));
}

#[test]
fn load_trajectory_export_artifact_round_trips_written_json() {
    let root = unique_temp_dir("loongclaw-trajectory-inspect");
    let config_path = write_trajectory_export_config(&root);
    let (_, config) = mvp::config::load(Some(
        config_path
            .to_str()
            .expect("config path should be valid utf-8"),
    ))
    .expect("load config fixture");
    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = mvp::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");

    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: mvp::session::repository::SessionState::Completed,
    })
    .expect("create root session");
    mvp::memory::append_turn_direct("root-session", "user", "hello", &memory_config)
        .expect("append user turn");

    let (_resolved_path, artifact) = collect_trajectory_export_artifact(
        Some(config_path.to_string_lossy().as_ref()),
        Some("root-session"),
    )
    .expect("collect trajectory artifact");

    let artifact_path = root.join("artifacts").join("trajectory.json");
    let encoded = serde_json::to_string_pretty(&artifact).expect("encode trajectory artifact");
    fs::create_dir_all(
        artifact_path
            .parent()
            .expect("artifact path should have parent"),
    )
    .expect("create artifact directory");
    fs::write(&artifact_path, encoded).expect("write artifact");

    let loaded = load_trajectory_export_artifact(
        artifact_path
            .to_str()
            .expect("artifact path should be valid utf-8"),
    )
    .expect("load trajectory artifact");
    assert_eq!(loaded.session.session_id, "root-session");
    assert_eq!(loaded.turns.len(), 1);
}
