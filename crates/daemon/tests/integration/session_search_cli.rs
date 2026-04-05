#![allow(unsafe_code)]
#![allow(
    clippy::disallowed_methods,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks
)]

use super::*;
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
    assert_eq!(artifact.hits.len(), 2);
    assert_eq!(artifact.hits[0].session.session_id, "child-session");
    assert_eq!(artifact.hits[1].session.session_id, "root-session");
}
