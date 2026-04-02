use super::tests::{cleanup_chat_test_memory, init_chat_test_memory};
use super::*;
use crate::conversation::ConversationRuntimeBinding;
use crate::session::repository::{NewSessionRecord, SessionKind, SessionRepository, SessionState};
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

fn create_root_session(repo: &SessionRepository, session_id: &str) {
    repo.create_session(NewSessionRecord {
        session_id: session_id.to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some(session_id.to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");
}

fn create_delegate_child_session(
    repo: &SessionRepository,
    session_id: &str,
    parent_session_id: &str,
) {
    repo.create_session(NewSessionRecord {
        session_id: session_id.to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some(parent_session_id.to_owned()),
        label: Some(session_id.to_owned()),
        state: SessionState::Ready,
    })
    .expect("create delegate child session");
}

fn append_session_turn(
    session_id: &str,
    role: &str,
    content: &str,
    memory_config: &MemoryRuntimeConfig,
) {
    crate::memory::append_turn_direct(session_id, role, content, memory_config)
        .expect("append session turn");
}

fn open_chat_test_connection(sqlite_path: &Path) -> Connection {
    Connection::open(sqlite_path).expect("open chat test sqlite connection")
}

fn set_chat_test_session_updated_at(sqlite_path: &Path, session_id: &str, updated_at: i64) {
    let conn = open_chat_test_connection(sqlite_path);
    conn.execute(
        "UPDATE sessions
         SET updated_at = ?2
         WHERE session_id = ?1",
        params![session_id, updated_at],
    )
    .expect("set chat test session updated_at");
}

fn set_chat_test_turn_timestamps(sqlite_path: &Path, session_id: &str, ts: i64) {
    let conn = open_chat_test_connection(sqlite_path);
    conn.execute(
        "UPDATE turns
         SET ts = ?2
         WHERE session_id = ?1",
        params![session_id, ts],
    )
    .expect("set chat test turn timestamps");
}

fn archive_chat_test_session(sqlite_path: &Path, session_id: &str, archived_at: i64) {
    let conn = open_chat_test_connection(sqlite_path);
    conn.execute(
        "INSERT INTO session_events(
            session_id,
            event_kind,
            actor_session_id,
            payload_json,
            ts
         ) VALUES (?1, ?2, NULL, ?3, ?4)",
        params![session_id, "session_archived", "{}", archived_at],
    )
    .expect("insert chat test archive event");
}

#[test]
fn cli_runtime_resolves_latest_session_selector_to_latest_resumable_root() {
    let (config, memory_config, sqlite_path) = init_chat_test_memory("latest-selector");
    let repo = SessionRepository::new(&memory_config).expect("repository");
    create_root_session(&repo, "selected-session");
    append_session_turn("selected-session", "user", "hello", &memory_config);

    let runtime = initialize_cli_turn_runtime_with_loaded_config(
        PathBuf::from("/tmp/loongclaw.toml"),
        config,
        Some("latest"),
        &CliChatOptions::default(),
        "cli-chat-latest-selector-test",
        CliSessionRequirement::AllowImplicitDefault,
        false,
    )
    .expect("latest selector runtime");

    assert_eq!(runtime.session_id, "selected-session");

    cleanup_chat_test_memory(&sqlite_path);
}

#[test]
fn cli_runtime_latest_session_selector_updates_startup_summary_session_id() {
    let (config, memory_config, sqlite_path) = init_chat_test_memory("latest-summary");
    let repo = SessionRepository::new(&memory_config).expect("repository");
    create_root_session(&repo, "selected-session");
    append_session_turn("selected-session", "user", "selected hello", &memory_config);

    let options = CliChatOptions::default();
    let runtime = initialize_cli_turn_runtime_with_loaded_config(
        PathBuf::from("/tmp/loongclaw.toml"),
        config,
        Some("latest"),
        &options,
        "cli-chat-latest-summary-test",
        CliSessionRequirement::AllowImplicitDefault,
        false,
    )
    .expect("latest selector runtime");
    let summary =
        build_cli_chat_startup_summary(&runtime, &options).expect("build startup summary");

    assert_eq!(runtime.session_id, "selected-session");
    assert_eq!(summary.session_id, "selected-session");
    assert_ne!(summary.session_id, crate::session::LATEST_SESSION_SELECTOR);

    cleanup_chat_test_memory(&sqlite_path);
}

#[test]
fn cli_runtime_rejects_latest_session_selector_when_no_resumable_root_exists() {
    let (config, _memory_config, sqlite_path) = init_chat_test_memory("latest-selector-none");
    let result = initialize_cli_turn_runtime_with_loaded_config(
        PathBuf::from("/tmp/loongclaw.toml"),
        config,
        Some("latest"),
        &CliChatOptions::default(),
        "cli-chat-latest-selector-empty-test",
        CliSessionRequirement::AllowImplicitDefault,
        false,
    );
    let error = match result {
        Ok(_) => panic!("latest selector should fail without a resumable root session"),
        Err(error) => error,
    };

    assert!(error.contains("latest"), "unexpected error: {error}");

    cleanup_chat_test_memory(&sqlite_path);
}

#[test]
fn cli_runtime_keeps_default_session_when_no_hint_is_provided() {
    let (config, _memory_config, sqlite_path) = init_chat_test_memory("default-selector");
    let runtime = initialize_cli_turn_runtime_with_loaded_config(
        PathBuf::from("/tmp/loongclaw.toml"),
        config,
        None,
        &CliChatOptions::default(),
        "cli-chat-default-selector-test",
        CliSessionRequirement::AllowImplicitDefault,
        false,
    )
    .expect("default session runtime");

    assert_eq!(runtime.session_id, "default");

    cleanup_chat_test_memory(&sqlite_path);
}

#[test]
fn cli_runtime_keeps_explicit_literal_session_id() {
    let (config, _memory_config, sqlite_path) = init_chat_test_memory("literal-selector");
    let runtime = initialize_cli_turn_runtime_with_loaded_config(
        PathBuf::from("/tmp/loongclaw.toml"),
        config,
        Some("custom-session"),
        &CliChatOptions::default(),
        "cli-chat-literal-selector-test",
        CliSessionRequirement::AllowImplicitDefault,
        false,
    )
    .expect("literal session runtime");

    assert_eq!(runtime.session_id, "custom-session");

    cleanup_chat_test_memory(&sqlite_path);
}

#[test]
fn concurrent_cli_runtime_keeps_latest_literal_when_explicit_session_is_required() {
    let (config, _memory_config, sqlite_path) = init_chat_test_memory("concurrent-latest");
    let runtime = initialize_cli_turn_runtime_with_loaded_config(
        PathBuf::from("/tmp/loongclaw.toml"),
        config,
        Some("latest"),
        &CliChatOptions::default(),
        "cli-chat-concurrent-latest-test",
        CliSessionRequirement::RequireExplicit,
        false,
    )
    .expect("concurrent runtime");

    assert_eq!(runtime.session_id, "latest");

    cleanup_chat_test_memory(&sqlite_path);
}

#[test]
fn cli_runtime_latest_session_selector_prefers_newest_resumable_root() {
    let (config, memory_config, sqlite_path) = init_chat_test_memory("latest-runtime-order");
    let repo = SessionRepository::new(&memory_config).expect("repository");

    create_root_session(&repo, "root-old");
    append_session_turn("root-old", "user", "old root turn", &memory_config);
    set_chat_test_session_updated_at(&sqlite_path, "root-old", 100);
    set_chat_test_turn_timestamps(&sqlite_path, "root-old", 100);

    create_root_session(&repo, "root-new");
    append_session_turn("root-new", "user", "new root turn", &memory_config);
    set_chat_test_session_updated_at(&sqlite_path, "root-new", 200);
    set_chat_test_turn_timestamps(&sqlite_path, "root-new", 200);

    create_delegate_child_session(&repo, "delegate-child", "root-new");
    append_session_turn(
        "delegate-child",
        "assistant",
        "delegate child turn",
        &memory_config,
    );
    set_chat_test_session_updated_at(&sqlite_path, "delegate-child", 400);
    set_chat_test_turn_timestamps(&sqlite_path, "delegate-child", 400);

    create_root_session(&repo, "root-archived");
    append_session_turn(
        "root-archived",
        "assistant",
        "archived root turn",
        &memory_config,
    );
    set_chat_test_session_updated_at(&sqlite_path, "root-archived", 500);
    set_chat_test_turn_timestamps(&sqlite_path, "root-archived", 500);
    archive_chat_test_session(&sqlite_path, "root-archived", 600);

    let runtime = initialize_cli_turn_runtime_with_loaded_config(
        PathBuf::from("/tmp/loongclaw.toml"),
        config,
        Some("latest"),
        &CliChatOptions::default(),
        "cli-chat-latest-runtime-order-test",
        CliSessionRequirement::AllowImplicitDefault,
        false,
    )
    .expect("latest selector runtime");

    assert_eq!(runtime.session_id, "root-new");

    cleanup_chat_test_memory(&sqlite_path);
}

#[tokio::test]
async fn cli_runtime_latest_session_selector_drives_history_loads() {
    let (config, memory_config, sqlite_path) = init_chat_test_memory("latest-history");
    let repo = SessionRepository::new(&memory_config).expect("repository");

    create_root_session(&repo, "root-old");
    append_session_turn("root-old", "user", "old user turn", &memory_config);
    append_session_turn(
        "root-old",
        "assistant",
        "old assistant turn",
        &memory_config,
    );
    set_chat_test_session_updated_at(&sqlite_path, "root-old", 100);
    set_chat_test_turn_timestamps(&sqlite_path, "root-old", 100);

    create_root_session(&repo, "root-new");
    append_session_turn("root-new", "user", "selected user turn", &memory_config);
    append_session_turn(
        "root-new",
        "assistant",
        "selected assistant turn",
        &memory_config,
    );
    set_chat_test_session_updated_at(&sqlite_path, "root-new", 200);
    set_chat_test_turn_timestamps(&sqlite_path, "root-new", 200);

    create_delegate_child_session(&repo, "delegate-child", "root-new");
    append_session_turn(
        "delegate-child",
        "assistant",
        "delegate child turn",
        &memory_config,
    );
    set_chat_test_session_updated_at(&sqlite_path, "delegate-child", 400);
    set_chat_test_turn_timestamps(&sqlite_path, "delegate-child", 400);

    create_root_session(&repo, "root-archived");
    append_session_turn(
        "root-archived",
        "assistant",
        "archived root turn",
        &memory_config,
    );
    set_chat_test_session_updated_at(&sqlite_path, "root-archived", 500);
    set_chat_test_turn_timestamps(&sqlite_path, "root-archived", 500);
    archive_chat_test_session(&sqlite_path, "root-archived", 600);

    let runtime = initialize_cli_turn_runtime_with_loaded_config(
        PathBuf::from("/tmp/loongclaw.toml"),
        config,
        Some("latest"),
        &CliChatOptions::default(),
        "cli-chat-latest-history-test",
        CliSessionRequirement::AllowImplicitDefault,
        false,
    )
    .expect("latest selector runtime");
    let history_lines = load_history_lines(
        &runtime.session_id,
        32,
        ConversationRuntimeBinding::direct(),
        &memory_config,
    )
    .await
    .expect("load history lines");

    assert_eq!(runtime.session_id, "root-new");
    assert_eq!(
        history_lines,
        vec![
            "user: selected user turn".to_owned(),
            "assistant: selected assistant turn".to_owned(),
        ]
    );

    cleanup_chat_test_memory(&sqlite_path);
}
