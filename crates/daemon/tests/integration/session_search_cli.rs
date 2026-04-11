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
    process,
    time::{SystemTime, UNIX_EPOCH},
};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    let process_id = process::id();
    std::env::temp_dir().join(format!("{prefix}-{process_id}-{nanos}"))
}

fn isolated_memory_env() -> loongclaw_daemon::test_support::ScopedEnv {
    let mut env = loongclaw_daemon::test_support::ScopedEnv::new();
    for key in [
        "LOONGCLAW_CONFIG_PATH",
        "LOONGCLAW_FILE_ROOT",
        "LOONGCLAW_MEMORY_BACKEND",
        "LOONGCLAW_MEMORY_PROFILE",
        "LOONGCLAW_MEMORY_SUMMARY_MAX_CHARS",
        "LOONGCLAW_SLIDING_WINDOW",
        "LOONGCLAW_SQLITE_PATH",
    ] {
        env.remove(key);
    }
    env
}

fn write_session_search_config(root: &Path) -> PathBuf {
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
fn collect_session_search_artifact_includes_visible_hits() {
    let _env = isolated_memory_env();
    let root = unique_temp_dir("loongclaw-session-search-artifact");
    let config_path = write_session_search_config(&root);
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
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: mvp::session::repository::SessionState::Running,
    })
    .expect("create child session");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "other-session".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Other".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create other session");

    mvp::memory::append_turn_direct(
        "root-session",
        "user",
        "deploy freeze starts Friday",
        &memory_config,
    )
    .expect("append root turn");
    mvp::memory::append_turn_direct(
        "child-session",
        "assistant",
        "deploy freeze checklist updated",
        &memory_config,
    )
    .expect("append child turn");
    mvp::memory::append_turn_direct(
        "other-session",
        "user",
        "deploy freeze hidden",
        &memory_config,
    )
    .expect("append hidden turn");

    let (_resolved_path, artifact) = collect_session_search_artifact(
        Some(config_path.to_string_lossy().as_ref()),
        Some("root-session"),
        "deploy freeze",
        10,
        false,
    )
    .expect("collect session-search artifact");

    assert_eq!(
        artifact.schema.version,
        SESSION_SEARCH_ARTIFACT_JSON_SCHEMA_VERSION
    );
    assert_eq!(artifact.scope_session_id, "root-session");
    assert_eq!(artifact.query, "deploy freeze");
    assert_eq!(artifact.returned_count, 2);
    assert_eq!(artifact.matched_session_count, 2);
    assert_eq!(artifact.results.len(), 2);
    assert_eq!(artifact.results[0].session_id, "child-session");
    assert_eq!(artifact.results[1].session_id, "root-session");
}

#[test]
fn load_session_search_artifact_round_trips_written_json() {
    let _env = isolated_memory_env();
    let root = unique_temp_dir("loongclaw-session-search-inspect");
    let config_path = write_session_search_config(&root);
    let config_path_str = config_path
        .to_str()
        .expect("config path should be valid utf-8");
    let loaded_config = mvp::config::load(Some(config_path_str));
    let (_, config) = loaded_config.expect("load config fixture");

    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = mvp::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");

    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    mvp::memory::append_turn_direct(
        "root-session",
        "user",
        "deploy freeze starts Friday",
        &memory_config,
    )
    .expect("append root turn");

    let artifact_path = root.join("artifacts").join("session-search.json");
    let artifact_path_str = artifact_path
        .to_str()
        .expect("artifact path should be valid utf-8");
    run_session_search_cli(
        Some(config_path_str),
        Some("root-session"),
        "deploy freeze",
        5,
        Some(artifact_path_str),
        false,
        false,
    )
    .expect("run session-search cli");

    let loaded_artifact = load_session_search_artifact(artifact_path_str);
    let loaded = loaded_artifact.expect("load session-search artifact");

    assert_eq!(loaded.scope_session_id, "root-session");
    assert_eq!(loaded.returned_count, 1);
    assert_eq!(loaded.results.len(), 1);
}

#[test]
fn load_session_search_artifact_rejects_inconsistent_counts() {
    let _env = isolated_memory_env();
    let root = unique_temp_dir("loongclaw-session-search-invalid-counts");
    fs::create_dir_all(&root).expect("create fixture root");
    let artifact_path = root.join("session-search.json");
    fs::write(
        &artifact_path,
        serde_json::to_string_pretty(&json!({
            "schema": {
                "version": SESSION_SEARCH_ARTIFACT_JSON_SCHEMA_VERSION,
                "surface": "session_search",
                "purpose": "session_recall_evidence"
            },
            "exported_at": "2026-04-05T00:00:00Z",
            "scope_session_id": "root-session",
            "query": "deploy freeze",
            "limit": 5,
            "include_archived": false,
            "include_turns": true,
            "include_events": true,
            "returned_count": 2,
            "matched_session_count": 1,
            "searched_session_count": 2,
            "results": [{
                "session_id": "child-session",
                "label": "Child",
                "session_state": "running",
                "archived": false,
                "source": "turn",
                "source_id": 12,
                "role": "assistant",
                "event_kind": null,
                "ts": 123,
                "snippet": "deploy freeze checklist updated",
                "score": 140
            }]
        }))
        .expect("encode invalid artifact"),
    )
    .expect("write invalid artifact");

    let artifact_path_str = artifact_path
        .to_str()
        .expect("artifact path should be valid utf-8");
    let error = load_session_search_artifact(artifact_path_str)
        .expect_err("inconsistent returned_count should fail");

    assert!(error.contains("returned_count=2"));
    assert!(error.contains("contains 1 result"));
}
