use super::*;
use crate::config::MemoryProfile;
use crate::test_support::ScopedCurrentDir;
use serde_json::json;

fn sqlite_test_config(db_path: impl Into<PathBuf>) -> MemoryRuntimeConfig {
    MemoryRuntimeConfig::for_sqlite_path(db_path)
}

fn sqlite_test_config_with_profile(
    db_path: impl Into<PathBuf>,
    profile: MemoryProfile,
    sliding_window: usize,
) -> MemoryRuntimeConfig {
    let mut config = sqlite_test_config(db_path);
    config.profile = profile;
    config.mode = profile.mode();
    config.sliding_window = sliding_window;
    config
}

fn sqlite_test_summary_config(
    db_path: impl Into<PathBuf>,
    sliding_window: usize,
    summary_max_chars: usize,
) -> MemoryRuntimeConfig {
    let mut config = sqlite_test_config_with_profile(
        db_path,
        MemoryProfile::WindowPlusSummary,
        sliding_window,
    );
    config.summary_max_chars = summary_max_chars;
    config
}

fn read_summary_checkpoint(
    config: &MemoryRuntimeConfig,
    session_id: &str,
) -> Result<(i64, String, i64, i64), String> {
    let runtime = acquire_memory_runtime(config)?;
    runtime.with_connection("test.read_summary_checkpoint", |conn| {
        conn.query_row(
            "SELECT checkpoint.summarized_through_turn_id,
                    body.summary_body,
                    checkpoint.summary_budget_chars,
                    checkpoint.summary_window_size
             FROM memory_summary_checkpoints AS checkpoint
             JOIN memory_summary_checkpoint_bodies AS body
               ON body.session_id = checkpoint.session_id
             WHERE checkpoint.session_id = ?1",
            rusqlite::params![session_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .map_err(|error| format!("read summary checkpoint failed: {error}"))
    })
}

fn count_summary_checkpoints(
    config: &MemoryRuntimeConfig,
    session_id: &str,
) -> Result<i64, String> {
    let runtime = acquire_memory_runtime(config)?;
    runtime.with_connection("test.count_summary_checkpoints", |conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM memory_summary_checkpoints WHERE session_id = ?1",
            rusqlite::params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("count summary checkpoints failed: {error}"))
    })
}

fn read_summary_checkpoint_boundary_turn_id(
    config: &MemoryRuntimeConfig,
    session_id: &str,
) -> Result<Option<i64>, String> {
    let runtime = acquire_memory_runtime(config)?;
    runtime.with_connection("test.read_summary_checkpoint_boundary_turn_id", |conn| {
        conn.query_row(
            "SELECT summary_before_turn_id
             FROM memory_summary_checkpoints
             WHERE session_id = ?1",
            rusqlite::params![session_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .map_err(|error| format!("read summary checkpoint boundary turn id failed: {error}"))
    })
}

fn read_session_turn_indices(
    config: &MemoryRuntimeConfig,
    session_id: &str,
) -> Result<Vec<i64>, String> {
    let runtime = acquire_memory_runtime(config)?;
    runtime.with_connection("test.read_session_turn_indices", |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT session_turn_index
                 FROM turns
                 WHERE session_id = ?1
                 ORDER BY id ASC",
            )
            .map_err(|error| format!("prepare session turn index query failed: {error}"))?;
        let rows = stmt
            .query_map(rusqlite::params![session_id], |row| row.get::<_, i64>(0))
            .map_err(|error| format!("query session turn indices failed: {error}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("decode session turn indices failed: {error}"))
    })
}

fn read_session_turn_count(
    config: &MemoryRuntimeConfig,
    session_id: &str,
) -> Result<Option<i64>, String> {
    let runtime = acquire_memory_runtime(config)?;
    runtime.with_connection("test.read_session_turn_count", |conn| {
        conn.query_row(
            "SELECT turn_count
             FROM memory_session_state
             WHERE session_id = ?1",
            rusqlite::params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .map(Some)
        .or_else(|error| {
            if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
                Ok(None)
            } else {
                Err(error)
            }
        })
        .map_err(|error| format!("read session turn count failed: {error}"))
    })
}

#[test]
fn load_window_includes_turn_count_in_payload() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-window-turn-count-payload-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("window-turn-count.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

    append_turn_direct("window-turn-count-session", "user", "turn 1", &config)
        .expect("append turn 1 should succeed");
    append_turn_direct("window-turn-count-session", "assistant", "turn 2", &config)
        .expect("append turn 2 should succeed");
    append_turn_direct("window-turn-count-session", "user", "turn 3", &config)
        .expect("append turn 3 should succeed");

    let outcome = load_window(
        crate::memory::build_window_request("window-turn-count-session", 2),
        &config,
    )
    .expect("window load should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["turn_count"], 3);
    assert_eq!(
        outcome.payload["turns"]
            .as_array()
            .expect("window payload turns")
            .len(),
        2
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn replace_turns_requires_object_payload() {
    let error = replace_turns(
        MemoryCoreRequest {
            operation: MEMORY_OP_REPLACE_TURNS.to_owned(),
            payload: json!("not-an-object"),
        },
        &MemoryRuntimeConfig::default(),
    )
    .expect_err("replace_turns should reject non-object payloads");

    assert_eq!(error, "memory.replace_turns payload must be an object");
}

#[test]
fn replace_turns_rejects_malformed_expected_turn_count() {
    let error = replace_turns(
        MemoryCoreRequest {
            operation: MEMORY_OP_REPLACE_TURNS.to_owned(),
            payload: json!({
                "session_id": "replace-turns-invalid-expected-count",
                "turns": [],
                "expected_turn_count": "invalid",
            }),
        },
        &MemoryRuntimeConfig::default(),
    )
    .expect_err("replace_turns should reject malformed expected_turn_count");

    assert_eq!(
        error,
        "memory.replace_turns payload.expected_turn_count must be a non-negative integer"
    );
}

#[test]
fn replace_turns_uses_turn_rows_when_session_state_metadata_is_missing() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-replace-turns-fallback-count-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("replace-turns-fallback-count.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 4);
    let session_id = "replace-turns-fallback-count-session";

    append_turn_direct(session_id, "user", "turn 1", &config)
        .expect("append turn 1 should succeed");
    append_turn_direct(session_id, "assistant", "turn 2", &config)
        .expect("append turn 2 should succeed");
    append_turn_direct(session_id, "user", "turn 3", &config)
        .expect("append turn 3 should succeed");

    let runtime = acquire_memory_runtime(&config).expect("acquire runtime");
    runtime
        .with_connection_mut("test.delete_turn_count_before_replace", |conn| {
            conn.execute(
                "DELETE FROM memory_session_state WHERE session_id = ?1",
                rusqlite::params![session_id],
            )
            .map_err(|error| format!("delete session turn count metadata failed: {error}"))
        })
        .expect("delete turn count metadata");

    let outcome = replace_turns(
        MemoryCoreRequest {
            operation: MEMORY_OP_REPLACE_TURNS.to_owned(),
            payload: json!({
                "session_id": session_id,
                "turns": [
                    {"role": "user", "content": "replacement 1", "ts": 11},
                    {"role": "assistant", "content": "replacement 2", "ts": 12},
                ],
                "expected_turn_count": 3,
            }),
        },
        &config,
    )
    .expect("replace_turns should fall back to turn rows when session metadata is missing");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["replaced_turns"], 2);
    assert_eq!(
        read_session_turn_count(&config, session_id).expect("read session turn count"),
        Some(2)
    );
    assert_eq!(
        window_direct(session_id, 4, &config)
            .expect("load replacement turns")
            .into_iter()
            .map(|turn| turn.content)
            .collect::<Vec<_>>(),
        vec!["replacement 1".to_owned(), "replacement 2".to_owned()]
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn memory_operations_reuse_cached_sqlite_runtime_for_same_path() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-sqlite-runtime-reuse-same-path-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("runtime-reuse.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

    ensure_memory_db_ready(Some(db_path.clone()), &config).expect("ensure memory db ready");
    let turns = window_direct_with_options("runtime-reuse-session", 2, true, &config)
        .expect("window query should succeed");

    assert!(turns.is_empty(), "expected no turns for a fresh session");
    assert_eq!(
        sqlite_bootstrap_count_for_tests(&db_path),
        1,
        "expected same-path operations to reuse one SQLite runtime bootstrap"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[test]
fn concurrent_same_path_bootstrap_reuses_one_cold_runtime() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-sqlite-runtime-concurrent-bootstrap-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("runtime-concurrent.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

    configure_sqlite_runtime_cache_miss_for_tests(&db_path, 2);

    let start_barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let thread_a_barrier = start_barrier.clone();
    let thread_a_path = db_path.clone();
    let thread_a_config = config.clone();
    let thread_a = std::thread::spawn(move || {
        thread_a_barrier.wait();
        ensure_memory_db_ready(Some(thread_a_path), &thread_a_config)
    });

    let thread_b_barrier = start_barrier.clone();
    let thread_b_path = db_path.clone();
    let thread_b_config = config;
    let thread_b = std::thread::spawn(move || {
        thread_b_barrier.wait();
        ensure_memory_db_ready(Some(thread_b_path), &thread_b_config)
    });

    start_barrier.wait();

    let thread_a_result = thread_a.join().expect("join bootstrap thread a");
    let thread_b_result = thread_b.join().expect("join bootstrap thread b");

    clear_sqlite_runtime_cache_miss_for_tests();

    thread_a_result.expect("bootstrap thread a should succeed");
    thread_b_result.expect("bootstrap thread b should succeed");

    assert_eq!(
        sqlite_bootstrap_count_for_tests(&db_path),
        1,
        "expected concurrent cold access to serialize same-path bootstrap work"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn distinct_sqlite_paths_get_distinct_runtime_bootstraps() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-sqlite-runtime-reuse-distinct-paths-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path_a = tmp.join("runtime-a.sqlite3");
    let db_path_b = tmp.join("runtime-b.sqlite3");
    let _ = fs::remove_file(&db_path_a);
    let _ = fs::remove_file(&db_path_b);

    let config_a =
        sqlite_test_config_with_profile(db_path_a.clone(), MemoryProfile::WindowOnly, 2);
    let config_b =
        sqlite_test_config_with_profile(db_path_b.clone(), MemoryProfile::WindowOnly, 2);

    ensure_memory_db_ready(Some(db_path_a.clone()), &config_a).expect("ensure db a ready");
    window_direct_with_options("runtime-a-session", 2, true, &config_a)
        .expect("window query for db a should succeed");
    ensure_memory_db_ready(Some(db_path_b.clone()), &config_b).expect("ensure db b ready");
    window_direct_with_options("runtime-b-session", 2, true, &config_b)
        .expect("window query for db b should succeed");

    assert_eq!(
        sqlite_bootstrap_count_for_tests(&db_path_a),
        1,
        "expected db path a to bootstrap once"
    );
    assert_eq!(
        sqlite_bootstrap_count_for_tests(&db_path_b),
        1,
        "expected db path b to bootstrap once"
    );

    let _ = fs::remove_file(&db_path_a);
    let _ = fs::remove_file(&db_path_b);
    let _ = fs::remove_dir(&tmp);
}

#[test]
fn resetting_cached_runtime_forces_runtime_recreation_on_next_access() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-sqlite-runtime-reuse-reset-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("runtime-reset.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

    ensure_memory_db_ready(Some(db_path.clone()), &config).expect("ensure memory db ready");
    window_direct_with_options("runtime-reset-session", 2, true, &config)
        .expect("initial window query should succeed");
    drop_cached_sqlite_runtime_for_tests(&db_path);
    window_direct_with_options("runtime-reset-session", 2, true, &config)
        .expect("window query after reset should succeed");

    assert_eq!(
        sqlite_bootstrap_count_for_tests(&db_path),
        2,
        "expected cached runtime reset to force exactly one additional bootstrap"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[test]
fn dropping_one_cached_runtime_preserves_other_cached_runtimes() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-sqlite-runtime-drop-one-preserve-others-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path_a = tmp.join("runtime-a.sqlite3");
    let db_path_b = tmp.join("runtime-b.sqlite3");
    let _ = fs::remove_file(&db_path_a);
    let _ = fs::remove_file(&db_path_b);

    let config_a =
        sqlite_test_config_with_profile(db_path_a.clone(), MemoryProfile::WindowOnly, 2);
    let config_b =
        sqlite_test_config_with_profile(db_path_b.clone(), MemoryProfile::WindowOnly, 2);

    ensure_memory_db_ready(Some(db_path_a.clone()), &config_a).expect("ensure db a ready");
    window_direct_with_options("runtime-drop-a", 2, true, &config_a)
        .expect("window query for db a should succeed");
    ensure_memory_db_ready(Some(db_path_b.clone()), &config_b).expect("ensure db b ready");
    window_direct_with_options("runtime-drop-b", 2, true, &config_b)
        .expect("window query for db b should succeed");

    drop_cached_sqlite_runtime(&db_path_a).expect("drop cached runtime a");

    window_direct_with_options("runtime-drop-a", 2, true, &config_a)
        .expect("window query for db a after drop should succeed");
    window_direct_with_options("runtime-drop-b", 2, true, &config_b)
        .expect("window query for db b after dropping db a should still succeed");

    assert_eq!(
        sqlite_bootstrap_count_for_tests(&db_path_a),
        2,
        "expected dropping db a to force one additional bootstrap for db a"
    );
    assert_eq!(
        sqlite_bootstrap_count_for_tests(&db_path_b),
        1,
        "expected dropping db a to preserve the cached runtime for db b"
    );

    let _ = fs::remove_file(&db_path_a);
    let _ = fs::remove_file(&db_path_b);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn equivalent_relative_and_absolute_paths_share_one_runtime() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-sqlite-runtime-alias-relative-absolute-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("data").join("alias.sqlite3");
    let _cwd_guard = ScopedCurrentDir::new(&tmp);

    let relative_config =
        sqlite_test_config_with_profile("data/alias.sqlite3", MemoryProfile::WindowOnly, 2);
    let absolute_config =
        sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

    ensure_memory_db_ready(None, &relative_config).expect("ensure relative db ready");
    window_direct_with_options("relative-alias-session", 2, true, &relative_config)
        .expect("relative alias window query should succeed");
    ensure_memory_db_ready(None, &absolute_config).expect("ensure absolute db ready");
    window_direct_with_options("absolute-alias-session", 2, true, &absolute_config)
        .expect("absolute alias window query should succeed");

    assert_eq!(
        sqlite_bootstrap_count_under_prefix_for_tests(&tmp),
        1,
        "expected equivalent relative and absolute aliases to share one bootstrap"
    );
    assert_eq!(
        sqlite_bootstrap_count_for_tests(&db_path),
        1,
        "expected normalized bootstrap count to be recorded under the canonical path"
    );

    drop(_cwd_guard);
    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn dot_dot_aliases_share_one_runtime_after_normalization() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-sqlite-runtime-alias-dotdot-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    let cwd = tmp.join("workspace").join("nested");
    fs::create_dir_all(&cwd).expect("create nested cwd dir");
    let db_path = tmp.join("workspace").join("data").join("alias.sqlite3");
    let _cwd_guard = ScopedCurrentDir::new(&cwd);

    let alias_a =
        sqlite_test_config_with_profile("../data/alias.sqlite3", MemoryProfile::WindowOnly, 2);
    let alias_b = sqlite_test_config_with_profile(
        "../nested/../data/./alias.sqlite3",
        MemoryProfile::WindowOnly,
        2,
    );

    ensure_memory_db_ready(None, &alias_a).expect("ensure dot-dot alias a ready");
    window_direct_with_options("dotdot-alias-a", 2, true, &alias_a)
        .expect("dot-dot alias a window query should succeed");
    ensure_memory_db_ready(None, &alias_b).expect("ensure dot-dot alias b ready");
    window_direct_with_options("dotdot-alias-b", 2, true, &alias_b)
        .expect("dot-dot alias b window query should succeed");

    assert_eq!(
        sqlite_bootstrap_count_under_prefix_for_tests(&tmp),
        1,
        "expected dot-dot aliases to resolve to one runtime bootstrap"
    );
    assert_eq!(
        sqlite_bootstrap_count_for_tests(&db_path),
        1,
        "expected normalized bootstrap count to land on the canonical db path"
    );

    drop(_cwd_guard);
    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn ensure_memory_db_ready_stamps_current_schema_version() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-sqlite-runtime-schema-version-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("schema-version.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

    ensure_memory_db_ready(Some(db_path.clone()), &config).expect("ensure memory db ready");

    let runtime = acquire_memory_runtime(&config).expect("acquire runtime");
    let user_version = runtime
        .with_connection("test.read_user_version", |conn| {
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .map_err(|error| format!("read sqlite user_version failed: {error}"))
        })
        .expect("read sqlite user_version");

    assert_eq!(user_version, SQLITE_MEMORY_SCHEMA_VERSION);

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn ensure_memory_db_ready_repairs_session_terminal_outcome_frozen_result_column() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-session-terminal-outcome-frozen-column-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("terminal-outcome.sqlite3");
    let _ = fs::remove_file(&db_path);

    let conn = Connection::open(&db_path).expect("open legacy sqlite db");
    configure_sqlite_connection(&conn).expect("configure legacy sqlite db");
    conn.execute_batch(
        "
        CREATE TABLE turns(
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          session_id TEXT NOT NULL,
          role TEXT NOT NULL,
          content TEXT NOT NULL,
          ts INTEGER NOT NULL
        );
        CREATE TABLE session_terminal_outcomes(
          session_id TEXT PRIMARY KEY,
          status TEXT NOT NULL,
          payload_json TEXT NOT NULL,
          recorded_at INTEGER NOT NULL
        );
        PRAGMA user_version = 9;
        ",
    )
    .expect("create legacy terminal outcome schema");
    drop(conn);

    let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

    ensure_memory_db_ready(Some(db_path.clone()), &config).expect("repair sqlite db");

    let conn = Connection::open(&db_path).expect("open repaired sqlite db");
    let columns = sqlite_table_columns(&conn, "session_terminal_outcomes")
        .expect("session_terminal_outcomes columns");

    assert!(columns.iter().any(|column| column == "frozen_result_json"));

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn ensure_memory_db_ready_repairs_session_event_search_storage() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loongclaw-session-event-search-storage-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("session-event-search.sqlite3");
    let _ = fs::remove_file(&db_path);

    let conn = Connection::open(&db_path).expect("open legacy sqlite db");
    configure_sqlite_connection(&conn).expect("configure legacy sqlite db");
    conn.execute_batch(
        "
        CREATE TABLE turns(
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          session_id TEXT NOT NULL,
          role TEXT NOT NULL,
          content TEXT NOT NULL,
          ts INTEGER NOT NULL
        );
        CREATE INDEX idx_turns_session_id ON turns(session_id, id);
        CREATE TABLE session_events(
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          session_id TEXT NOT NULL,
          event_kind TEXT NOT NULL,
          actor_session_id TEXT NULL,
          payload_json TEXT NOT NULL,
          ts INTEGER NOT NULL
        );
        INSERT INTO session_events(session_id, event_kind, actor_session_id, payload_json, ts)
        VALUES ('root-session', 'memory_indexed', NULL, '{\"summary\":\"中文分词已经启用\"}', 100);
        PRAGMA user_version = 10;
        ",
    )
    .expect("create legacy session event schema");
    drop(conn);

    let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

    ensure_memory_db_ready(Some(db_path.clone()), &config).expect("repair sqlite db");

    let conn = Connection::open(&db_path).expect("open repaired sqlite db");
    let columns =
        sqlite_table_columns(&conn, "session_events").expect("session_events columns");
    assert!(columns.iter().any(|column| column == "search_text"));

    let search_text = conn
        .query_row(
            "SELECT search_text FROM session_events WHERE id = 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("load session event search_text");
    assert!(search_text.contains("中文"), "search_text={search_text}");
    assert!(search_text.contains("分词"), "search_text={search_text}");

    let match_query = crate::search_text::build_search_fts_query("中文 分词", 6)
        .expect("session event search query");
    let fts_count = conn
        .query_row(
            "SELECT COUNT(*) FROM session_events_fts WHERE session_events_fts MATCH ?1",
            rusqlite::params![match_query],
            |row| row.get::<_, i64>(0),
        )
        .expect("query session event FTS");
    assert_eq!(fts_count, 1);

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn ensure_memory_db_ready_repairs_session_tool_consent_mode_check_constraint() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-session-tool-consent-mode-check-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("session-tool-consent.sqlite3");
    let _ = fs::remove_file(&db_path);

    let conn = Connection::open(&db_path).expect("open legacy sqlite db");
    configure_sqlite_connection(&conn).expect("configure legacy sqlite db");
    conn.execute_batch(
        "
        CREATE TABLE turns(
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          session_id TEXT NOT NULL,
          role TEXT NOT NULL,
          content TEXT NOT NULL,
          ts INTEGER NOT NULL
        );
        CREATE INDEX idx_turns_session_id ON turns(session_id, id);
        CREATE TABLE session_tool_consent(
          scope_session_id TEXT PRIMARY KEY,
          mode TEXT NOT NULL,
          updated_by_session_id TEXT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL
        );
        PRAGMA user_version = 6;
        ",
    )
    .expect("create legacy schema");
    conn.execute(
        "INSERT INTO session_tool_consent(
            scope_session_id,
            mode,
            updated_by_session_id,
            created_at,
            updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            "root-session",
            "full",
            "root-session",
            unix_ts_now(),
            unix_ts_now(),
        ],
    )
    .expect("insert legacy session tool consent");
    drop(conn);

    let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

    ensure_memory_db_ready(Some(db_path.clone()), &config)
        .expect("migrate legacy sqlite memory db");

    let repaired_conn = Connection::open(&db_path).expect("open repaired sqlite db");
    let session_tool_consent_sql = repaired_conn
        .query_row(
            "SELECT sql
             FROM sqlite_master
             WHERE type = 'table' AND name = 'session_tool_consent'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("read repaired session_tool_consent sql");

    assert!(
        session_tool_consent_sql.contains(SESSION_TOOL_CONSENT_MODE_CHECK_SQL),
        "expected repaired DDL to contain mode check: {session_tool_consent_sql}"
    );

    let invalid_insert = repaired_conn.execute(
        "INSERT INTO session_tool_consent(
            scope_session_id,
            mode,
            updated_by_session_id,
            created_at,
            updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            "other-session",
            "bogus",
            "root-session",
            unix_ts_now(),
            unix_ts_now(),
        ],
    );
    assert!(
        invalid_insert.is_err(),
        "invalid mode should be rejected after repair"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn repeated_same_path_runtime_lookup_reuses_normalized_path_alias_cache() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-sqlite-runtime-alias-cache-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    let cwd = tmp.join("workspace").join("nested");
    fs::create_dir_all(&cwd).expect("create nested cwd dir");
    let db_path = tmp
        .join("workspace")
        .join("data")
        .join("alias-cache.sqlite3");
    let _cwd_guard = ScopedCurrentDir::new(&cwd);

    let config = sqlite_test_config_with_profile(
        PathBuf::from("../data/alias-cache.sqlite3"),
        MemoryProfile::WindowOnly,
        2,
    );

    ensure_memory_db_ready(None, &config).expect("ensure alias-cache db ready");

    let _metrics = begin_sqlite_metric_capture_for_tests();
    window_direct_with_options("alias-cache-session", 2, true, &config)
        .expect("first alias-cache window query should succeed");
    window_direct_with_options("alias-cache-session", 2, true, &config)
        .expect("second alias-cache window query should succeed");

    assert_eq!(
        runtime_path_normalization_full_count_for_tests(),
        0,
        "expected repeated same-path runtime lookups to reuse cached normalized path aliases instead of re-running full normalization"
    );
    assert!(
        runtime_path_normalization_alias_hit_count_for_tests() >= 2,
        "expected repeated same-path runtime lookups to hit the normalized path alias cache on each hot-path access"
    );
    assert_eq!(
        sqlite_bootstrap_count_for_tests(&db_path),
        1,
        "expected repeated same-path runtime lookups to reuse the existing cached sqlite runtime"
    );

    drop(_cwd_guard);
    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn reopening_current_schema_db_skips_metadata_repairs() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-sqlite-runtime-schema-repair-skip-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("schema-repair-skip.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

    ensure_memory_db_ready(Some(db_path.clone()), &config)
        .expect("bootstrap current schema db");
    drop_cached_sqlite_runtime_for_tests(&db_path);

    reset_sqlite_schema_repair_metrics_for_tests();
    ensure_memory_db_ready(Some(db_path.clone()), &config).expect("reopen current schema db");

    assert_eq!(
        sqlite_schema_repair_count_for_tests("turn_session_index"),
        0,
        "expected current-schema reopen to skip turn/session metadata repairs"
    );
    assert_eq!(
        sqlite_schema_repair_count_for_tests("summary_checkpoint_metadata"),
        0,
        "expected current-schema reopen to skip summary checkpoint metadata repairs"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn reopening_current_schema_db_skips_schema_init_batch() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-sqlite-runtime-schema-init-skip-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("schema-init-skip.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

    ensure_memory_db_ready(Some(db_path.clone()), &config)
        .expect("bootstrap current schema db");
    assert_eq!(
        sqlite_schema_init_count_for_tests(&db_path),
        1,
        "expected the initial bootstrap to execute schema initialization exactly once"
    );

    drop_cached_sqlite_runtime_for_tests(&db_path);
    ensure_memory_db_ready(Some(db_path.clone()), &config).expect("reopen current schema db");

    assert_eq!(
        sqlite_schema_init_count_for_tests(&db_path),
        1,
        "expected reopening a current-schema db to skip the unconditional schema initialization batch"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn ensure_memory_db_ready_diagnostics_distinguish_cache_miss_and_hit() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-sqlite-runtime-diagnostics-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("runtime-diagnostics.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

    let (_, cold_bootstrap) =
        ensure_memory_db_ready_with_diagnostics(Some(db_path.clone()), &config)
            .expect("cold bootstrap diagnostics");
    let (_, hot_bootstrap) =
        ensure_memory_db_ready_with_diagnostics(Some(db_path.clone()), &config)
            .expect("hot bootstrap diagnostics");

    assert!(!cold_bootstrap.cache_hit);
    assert!(hot_bootstrap.cache_hit);
    assert_eq!(hot_bootstrap.runtime_create_ms, 0.0);
    assert_eq!(hot_bootstrap.connection_open_ms, 0.0);
    assert_eq!(hot_bootstrap.configure_connection_ms, 0.0);
    assert_eq!(hot_bootstrap.schema_init_ms, 0.0);
    assert_eq!(hot_bootstrap.schema_upgrade_ms, 0.0);

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn window_reads_route_through_cached_statement_preparation() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_cached_prepare_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-sqlite-prepared-window-cache-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("prepared-window.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

    window_direct_with_options("prepared-window-session", 2, true, &config)
        .expect("window query should succeed");
    window_direct_with_options("prepared-window-session", 2, true, &config)
        .expect("second window query should succeed");

    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("FROM turns"),
        2,
        "expected hot window query to use cached statement preparation on both executions"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn window_only_context_snapshot_avoids_indexed_recent_turn_query() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_cached_prepare_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-window-only-snapshot-query-shape-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("window-only-snapshot.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

    append_turn_direct("window-only-snapshot-session", "user", "turn 1", &config)
        .expect("append turn 1 should succeed");
    append_turn_direct(
        "window-only-snapshot-session",
        "assistant",
        "turn 2",
        &config,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct("window-only-snapshot-session", "user", "turn 3", &config)
        .expect("append turn 3 should succeed");

    let snapshot = load_context_snapshot("window-only-snapshot-session", &config)
        .expect("load context snapshot");

    assert_eq!(snapshot.window_turns.len(), 2);
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT id, role, content\n             FROM turns"
        ),
        1,
        "expected window-only prompt snapshots to use the visible-turn prompt query that can skip internal persisted records without fetching timestamps"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT role, content\n             FROM turns"
        ),
        0,
        "expected window-only prompt snapshots to retire the older lean query shape now that internal records require visible-turn filtering by id"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("SELECT role, content, ts"),
        0,
        "expected window-only prompt snapshots to avoid the full window query shape with timestamps"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn summary_context_snapshot_avoids_indexed_window_materialization_query_shape() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_cached_prepare_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-snapshot-window-query-shape-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-snapshot-window.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "summary-snapshot-query-shape-session",
        "user",
        "turn 1",
        &config,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "summary-snapshot-query-shape-session",
        "assistant",
        "turn 2",
        &config,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "summary-snapshot-query-shape-session",
        "user",
        "turn 3",
        &config,
    )
    .expect("append turn 3 should succeed");

    reset_cached_prepare_metrics_for_tests();
    let snapshot = load_context_snapshot("summary-snapshot-query-shape-session", &config)
        .expect("load context snapshot");

    assert_eq!(snapshot.window_turns.len(), 2);
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT turn_count\n             FROM memory_session_state"
        ),
        1,
        "expected summary prompt snapshots to consult per-session turn-count metadata before selecting the active-window query shape"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "LEFT JOIN memory_summary_checkpoints checkpoint"
        ),
        1,
        "expected summary prompt snapshots to co-load the active window and checkpoint metadata once turn-count metadata proves overflow"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT id, role, content\n             FROM turns"
        ),
        0,
        "expected summary prompt snapshots to retire the older id-only active-window query now that checkpoint metadata is folded into the fast path"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("state.turn_count"),
        0,
        "expected summary prompt snapshots to retire the heavier joined turn-count query shape"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("session_turn_index"),
        0,
        "expected summary prompt snapshots to avoid session_turn_index metadata when turn-count metadata can derive the active window boundary"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("SELECT role, content, ts, id"),
        0,
        "expected summary prompt snapshots to avoid the window+boundary query shape that still fetches timestamps"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT role, content, id, session_turn_index"
        ),
        0,
        "expected summary prompt snapshots to retire the older query shape that depended on session_turn_index metadata"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn summary_append_path_routes_multiple_sqls_through_cached_preparation() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_cached_prepare_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-sqlite-prepared-summary-cache-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("prepared-summary.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct("prepared-summary-session", "user", "turn 1", &config)
        .expect("append turn 1 should succeed");
    append_turn_direct("prepared-summary-session", "assistant", "turn 2", &config)
        .expect("append turn 2 should succeed");
    append_turn_direct("prepared-summary-session", "user", "turn 3", &config)
        .expect("append turn 3 should succeed");
    append_turn_direct("prepared-summary-session", "assistant", "turn 4", &config)
        .expect("append turn 4 should succeed");

    assert!(
        cached_prepare_count_for_sql_fragment_for_tests("memory_summary_checkpoints") >= 2,
        "expected post-overflow summary append path to route repeated checkpoint SQL through cached preparation without touching summary maintenance before the window overflows"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("SELECT id, role, content, ts"),
        0,
        "expected summary append path to avoid the full indexed active-window query when only the summary boundary id is needed"
    );
    assert!(
        cached_prepare_count_for_sql_fragment_for_tests("session_turn_index <= 2") >= 1,
        "expected first overflow summary append to use the dedicated two-row initial checkpoint query"
    );
    assert!(
        cached_prepare_count_for_sql_fragment_for_tests("LIMIT 2") >= 1,
        "expected first overflow summary append to cap the dedicated initial checkpoint query at the two boundary rows"
    );
    assert!(
        cached_prepare_count_for_sql_fragment_for_tests("AS summary_before_turn_id") >= 1,
        "expected post-initial summary append maintenance to keep routing boundary and checkpoint metadata reads through one append-maintenance state query"
    );
    assert!(
        cached_prepare_count_for_sql_fragment_for_tests("summary_body_bytes") >= 2,
        "expected active summary append maintenance to read persisted summary body bytes metadata instead of recomputing text length inside SQLite"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("LENGTH(CAST(summary_body AS BLOB))"),
        0,
        "expected summary append path to avoid recomputing summary body length inside SQLite"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn summary_append_path_avoids_empty_checkpoint_delete_before_window_overflow() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_cached_prepare_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-append-empty-delete-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-append-empty-delete.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 4, 256);

    append_turn_direct(
        "summary-append-empty-delete-session",
        "user",
        "turn 1",
        &config,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "summary-append-empty-delete-session",
        "assistant",
        "turn 2",
        &config,
    )
    .expect("append turn 2 should succeed");

    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "DELETE FROM memory_summary_checkpoints"
        ),
        0,
        "expected append maintenance to avoid preparing checkpoint delete statements before the active window overflows"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn summary_append_path_skips_summary_maintenance_queries_before_window_overflow() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_cached_prepare_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-append-pre-overflow-maintenance-skip-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-append-pre-overflow-maintenance-skip.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 4, 256);

    append_turn_direct(
        "summary-append-pre-overflow-maintenance-skip-session",
        "user",
        "turn 1",
        &config,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "summary-append-pre-overflow-maintenance-skip-session",
        "assistant",
        "turn 2",
        &config,
    )
    .expect("append turn 2 should succeed");

    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("AS summary_before_turn_id"),
        0,
        "expected pre-overflow appends to skip summary maintenance state queries entirely"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "turns.session_turn_index = state.turn_count - ?2 + 1"
        ),
        0,
        "expected pre-overflow appends to skip summary boundary probes entirely"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn summary_append_hot_path_advances_boundary_without_window_offset_probe() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-append-hot-boundary-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-append-hot-boundary.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "summary-append-hot-boundary-session",
        "user",
        "turn 1",
        &config,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "summary-append-hot-boundary-session",
        "assistant",
        "turn 2",
        &config,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "summary-append-hot-boundary-session",
        "user",
        "turn 3",
        &config,
    )
    .expect("append turn 3 should succeed");
    append_turn_direct(
        "summary-append-hot-boundary-session",
        "assistant",
        "turn 4",
        &config,
    )
    .expect("append turn 4 should succeed");

    let boundary_before = read_summary_checkpoint_boundary_turn_id(
        &config,
        "summary-append-hot-boundary-session",
    )
    .expect("read summary checkpoint boundary after warmup");
    assert_eq!(boundary_before, Some(3));

    reset_cached_prepare_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();
    append_turn_direct(
        "summary-append-hot-boundary-session",
        "user",
        "turn 5",
        &config,
    )
    .expect("append turn 5 should succeed");

    assert!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "id > checkpoint.summary_before_turn_id"
        ) >= 1,
        "expected steady-state append to advance the summary boundary from checkpoint metadata instead of re-probing the full window"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("LIMIT 1 OFFSET ?2"),
        0,
        "expected steady-state append to avoid the window-offset boundary probe once checkpoint metadata is available"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT summarized_through_turn_id, summary_before_turn_id, summary_body_bytes, summary_budget_chars, summary_window_size, summary_format_version"
        ),
        0,
        "expected steady-state append to reuse checkpoint metadata already loaded by append maintenance instead of re-querying checkpoint meta"
    );

    let boundary_after = read_summary_checkpoint_boundary_turn_id(
        &config,
        "summary-append-hot-boundary-session",
    )
    .expect("read summary checkpoint boundary after hot append");
    assert_eq!(boundary_after, Some(4));

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn summary_append_cold_path_uses_dedicated_initial_checkpoint_query() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_summary_materialization_metrics_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-append-cold-boundary-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-append-cold-boundary.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "summary-append-cold-boundary-session",
        "user",
        "turn 1",
        &config,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "summary-append-cold-boundary-session",
        "assistant",
        "turn 2",
        &config,
    )
    .expect("append turn 2 should succeed");

    reset_cached_prepare_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();
    append_turn_direct(
        "summary-append-cold-boundary-session",
        "user",
        "turn 3",
        &config,
    )
    .expect("append turn 3 should succeed");

    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "turns.session_turn_index = state.turn_count - ?2 + 1"
        ),
        0,
        "expected first summary checkpoint materialization to bypass the separate boundary lookup by per-session turn-count metadata"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("LIMIT 1 OFFSET ?2"),
        0,
        "expected cold summary boundary lookup to avoid the window-offset probe"
    );
    assert!(
        cached_prepare_count_for_sql_fragment_for_tests("session_turn_index <= 2") >= 1,
        "expected first summary checkpoint materialization to use a dedicated two-row range query"
    );
    assert!(
        cached_prepare_count_for_sql_fragment_for_tests("LIMIT 2") >= 1,
        "expected first summary checkpoint materialization to cap the dedicated range query at the two boundary rows"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT summarized_through_turn_id, summary_before_turn_id, summary_body_bytes, summary_budget_chars, summary_window_size, summary_format_version"
        ),
        0,
        "expected first summary checkpoint materialization to avoid reloading a checkpoint row that append maintenance already knows does not exist"
    );
    assert_eq!(
        summary_streaming_query_count_for_tests("rebuild"),
        0,
        "expected first summary checkpoint materialization to avoid the generic rebuild streaming query"
    );
    assert_eq!(
        summary_row_observed_count_for_tests(),
        1,
        "expected first summary checkpoint materialization to observe exactly one payload row"
    );
    assert_eq!(
        summary_payload_decode_count_for_tests(),
        1,
        "expected first summary checkpoint materialization to decode exactly one payload row"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn summary_rebuild_routes_through_streaming_row_accumulation() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_summary_materialization_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-streaming-rebuild-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-streaming-rebuild.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config_window_two = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "summary-streaming-rebuild-session",
        "user",
        "turn 1",
        &config_window_two,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "summary-streaming-rebuild-session",
        "assistant",
        "turn 2",
        &config_window_two,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "summary-streaming-rebuild-session",
        "user",
        "turn 3",
        &config_window_two,
    )
    .expect("append turn 3 should succeed");
    append_turn_direct(
        "summary-streaming-rebuild-session",
        "assistant",
        "turn 4",
        &config_window_two,
    )
    .expect("append turn 4 should succeed");

    reset_summary_materialization_metrics_for_tests();
    let config_window_three = MemoryRuntimeConfig {
        sliding_window: 3,
        ..config_window_two
    };
    let _snapshot =
        load_context_snapshot("summary-streaming-rebuild-session", &config_window_three)
            .expect("load context snapshot after window change");

    assert_eq!(
        summary_streaming_query_count_for_tests("rebuild"),
        1,
        "expected rebuild path to route through streaming summary accumulation"
    );
    assert_eq!(
        summary_buffered_query_count_for_tests("rebuild"),
        0,
        "expected rebuild path to stop buffering full turn vectors"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn summary_catch_up_routes_through_streaming_row_accumulation() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_summary_materialization_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-streaming-catch-up-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-streaming-catch-up.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "summary-streaming-catch-up-session",
        "user",
        "turn 1",
        &config,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "summary-streaming-catch-up-session",
        "assistant",
        "turn 2",
        &config,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "summary-streaming-catch-up-session",
        "user",
        "turn 3",
        &config,
    )
    .expect("append turn 3 should succeed");

    reset_summary_materialization_metrics_for_tests();
    append_turn_direct(
        "summary-streaming-catch-up-session",
        "assistant",
        "turn 4",
        &config,
    )
    .expect("append turn 4 should succeed");

    assert_eq!(
        summary_streaming_query_count_for_tests("catch_up"),
        1,
        "expected catch-up path to route through streaming summary accumulation"
    );
    assert_eq!(
        summary_buffered_query_count_for_tests("catch_up"),
        0,
        "expected catch-up path to stop buffering delta turn vectors"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn summary_rebuild_skips_summary_formatting_after_budget_saturation() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_summary_materialization_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-saturation-rebuild-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-saturation-rebuild.sqlite3");
    let _ = fs::remove_file(&db_path);

    let first_turn = "FIRST-MARKER ".repeat(40);
    let second_turn = "SECOND-MARKER ".repeat(20);

    let config_window_two = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "summary-saturation-rebuild-session",
        "user",
        &first_turn,
        &config_window_two,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "summary-saturation-rebuild-session",
        "assistant",
        &second_turn,
        &config_window_two,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "summary-saturation-rebuild-session",
        "user",
        "turn 3",
        &config_window_two,
    )
    .expect("append turn 3 should succeed");
    append_turn_direct(
        "summary-saturation-rebuild-session",
        "assistant",
        "turn 4",
        &config_window_two,
    )
    .expect("append turn 4 should succeed");
    append_turn_direct(
        "summary-saturation-rebuild-session",
        "user",
        "turn 5",
        &config_window_two,
    )
    .expect("append turn 5 should succeed");

    reset_summary_materialization_metrics_for_tests();
    let config_window_three = MemoryRuntimeConfig {
        sliding_window: 3,
        ..config_window_two
    };
    let snapshot =
        load_context_snapshot("summary-saturation-rebuild-session", &config_window_three)
            .expect("load context snapshot after window change");
    let (summarized_through_turn_id, summary_body, _summary_budget, summary_window_size) =
        read_summary_checkpoint(&config_window_three, "summary-saturation-rebuild-session")
            .expect("summary checkpoint row should exist after rebuild");

    assert_eq!(summarized_through_turn_id, 2);
    assert_eq!(summary_window_size, 3);
    assert_eq!(snapshot.window_turns.len(), 3);
    assert!(summary_body.contains("FIRST-MARKER"));
    assert!(!summary_body.contains("SECOND-MARKER"));
    assert_eq!(
        summary_payload_decode_count_for_tests(),
        1,
        "expected rebuild path to stop decoding role/content after summary saturation"
    );
    assert_eq!(
        summary_normalization_count_for_tests(),
        0,
        "expected rebuild path to avoid scratch normalization before and after summary saturation"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn summary_rebuild_fast_forwards_frontier_after_budget_saturation() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_summary_materialization_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-frontier-fast-forward-rebuild-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-frontier-fast-forward-rebuild.sqlite3");
    let _ = fs::remove_file(&db_path);

    let first_turn = "FIRST-MARKER ".repeat(40);
    let config_window_two = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "summary-frontier-fast-forward-rebuild-session",
        "user",
        &first_turn,
        &config_window_two,
    )
    .expect("append turn 1 should succeed");
    for turn_index in 2..=32 {
        let role = if turn_index % 2 == 0 {
            "assistant"
        } else {
            "user"
        };
        append_turn_direct(
            "summary-frontier-fast-forward-rebuild-session",
            role,
            &format!("turn {turn_index}"),
            &config_window_two,
        )
        .expect("append tail turn should succeed");
    }

    reset_summary_materialization_metrics_for_tests();
    let config_window_four = MemoryRuntimeConfig {
        sliding_window: 4,
        ..config_window_two
    };
    let snapshot = load_context_snapshot(
        "summary-frontier-fast-forward-rebuild-session",
        &config_window_four,
    )
    .expect("load context snapshot after window change");
    let (summarized_through_turn_id, summary_body, _summary_budget, summary_window_size) =
        read_summary_checkpoint(
            &config_window_four,
            "summary-frontier-fast-forward-rebuild-session",
        )
        .expect("summary checkpoint row should exist after rebuild");

    assert_eq!(summarized_through_turn_id, 28);
    assert_eq!(summary_window_size, 4);
    assert_eq!(snapshot.window_turns.len(), 4);
    assert!(summary_body.contains("FIRST-MARKER"));
    assert_eq!(
        summary_payload_decode_count_for_tests(),
        1,
        "expected rebuild path to decode only the first saturating payload"
    );
    assert_eq!(
        summary_row_observed_count_for_tests(),
        1,
        "expected rebuild path to stop streaming rows once the summary budget saturates"
    );
    assert_eq!(
        summary_frontier_probe_count_for_tests("rebuild"),
        1,
        "expected rebuild path to perform one frontier lookup after summary saturation instead of carrying frontier metadata in every streamed row"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn summary_rebuild_load_diagnostics_split_stream_and_checkpoint_upsert_costs() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-rebuild-diagnostics-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-rebuild-diagnostics.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config_window_two = sqlite_test_summary_config(db_path.clone(), 2, 256);

    for (role, content) in [
        ("user", "turn 1"),
        ("assistant", "turn 2"),
        ("user", "turn 3"),
        ("assistant", "turn 4"),
    ] {
        append_turn_direct(
            "summary-rebuild-diagnostics-session",
            role,
            content,
            &config_window_two,
        )
        .expect("append turn should succeed");
    }

    let config_window_three = MemoryRuntimeConfig {
        sliding_window: 3,
        ..config_window_two
    };
    let (_snapshot, diagnostics) = load_context_snapshot_with_diagnostics(
        "summary-rebuild-diagnostics-session",
        &config_window_three,
    )
    .expect("load context snapshot after window change");

    assert!(
        diagnostics.summary_rebuild_ms > 0.0,
        "expected summary rebuild diagnostics to record total rebuild time"
    );
    assert!(
        diagnostics.summary_rebuild_stream_ms > 0.0,
        "expected summary rebuild diagnostics to split out stream accumulation time"
    );
    assert!(
        diagnostics.summary_rebuild_checkpoint_upsert_ms > 0.0,
        "expected summary rebuild diagnostics to split out checkpoint upsert time"
    );
    assert!(
        diagnostics.summary_rebuild_checkpoint_metadata_upsert_ms > 0.0,
        "expected summary rebuild diagnostics to split out checkpoint metadata upsert time"
    );
    assert!(
        diagnostics.summary_rebuild_checkpoint_body_upsert_ms > 0.0,
        "expected summary rebuild diagnostics to split out checkpoint body upsert time"
    );
    assert!(
        diagnostics.summary_rebuild_checkpoint_commit_ms > 0.0,
        "expected summary rebuild diagnostics to split out checkpoint commit time"
    );
    assert!(
        diagnostics.summary_rebuild_checkpoint_metadata_upsert_ms
            + diagnostics.summary_rebuild_checkpoint_body_upsert_ms
            + diagnostics.summary_rebuild_checkpoint_commit_ms
            <= diagnostics.summary_rebuild_checkpoint_upsert_ms + 1.0,
        "expected checkpoint upsert subphases to stay within the measured checkpoint upsert envelope"
    );
    assert!(
        diagnostics.summary_rebuild_stream_ms
            + diagnostics.summary_rebuild_checkpoint_upsert_ms
            <= diagnostics.summary_rebuild_ms + 1.0,
        "expected rebuild subphases to stay within the measured rebuild envelope"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn summary_catch_up_advances_frontier_after_budget_saturation_without_reformatting() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_summary_materialization_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-saturation-catch-up-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-saturation-catch-up.sqlite3");
    let _ = fs::remove_file(&db_path);

    let first_turn = "FIRST-MARKER ".repeat(40);
    let second_turn = "SECOND-MARKER ".repeat(20);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "summary-saturation-catch-up-session",
        "user",
        &first_turn,
        &config,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "summary-saturation-catch-up-session",
        "assistant",
        &second_turn,
        &config,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "summary-saturation-catch-up-session",
        "user",
        "turn 3",
        &config,
    )
    .expect("append turn 3 should succeed");

    let (through_before, summary_before, _budget_before, window_before) =
        read_summary_checkpoint(&config, "summary-saturation-catch-up-session")
            .expect("summary checkpoint should exist before catch-up");
    assert_eq!(through_before, 1);
    assert_eq!(window_before, 2);
    assert!(summary_before.contains("FIRST-MARKER"));
    assert!(!summary_before.contains("SECOND-MARKER"));

    reset_summary_materialization_metrics_for_tests();
    append_turn_direct(
        "summary-saturation-catch-up-session",
        "assistant",
        "turn 4",
        &config,
    )
    .expect("append turn 4 should succeed");

    let (through_after, summary_after, _budget_after, window_after) =
        read_summary_checkpoint(&config, "summary-saturation-catch-up-session")
            .expect("summary checkpoint should exist after catch-up");

    assert_eq!(through_after, 2);
    assert_eq!(window_after, 2);
    assert_eq!(summary_after, summary_before);
    assert_eq!(
        summary_payload_decode_count_for_tests(),
        0,
        "expected catch-up to advance the frontier without decoding saturated summary payloads"
    );
    assert_eq!(
        summary_normalization_count_for_tests(),
        0,
        "expected catch-up to advance the frontier without normalizing saturated summary payloads"
    );
    assert_eq!(
        summary_streaming_query_count_for_tests("catch_up"),
        0,
        "expected saturated append maintenance to skip catch-up streaming when the summary body is already at budget"
    );
    assert_eq!(
        summary_frontier_probe_count_for_tests("catch_up"),
        0,
        "expected catch-up to avoid a separate frontier lookup once streaming rows carry the session max id"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn summary_window_shrink_catch_up_avoids_scratch_normalization_buffer() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_summary_materialization_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-fused-append-rebuild-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-fused-append-rebuild.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config_window_two = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "summary-fused-append-rebuild-session",
        "user",
        "turn 1",
        &config_window_two,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "summary-fused-append-rebuild-session",
        "assistant",
        "turn 2",
        &config_window_two,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "summary-fused-append-rebuild-session",
        "user",
        "turn 3",
        &config_window_two,
    )
    .expect("append turn 3 should succeed");
    append_turn_direct(
        "summary-fused-append-rebuild-session",
        "assistant",
        "turn 4",
        &config_window_two,
    )
    .expect("append turn 4 should succeed");

    reset_summary_materialization_metrics_for_tests();
    let config_window_one = MemoryRuntimeConfig {
        sliding_window: 1,
        ..config_window_two
    };
    let snapshot =
        load_context_snapshot("summary-fused-append-rebuild-session", &config_window_one)
            .expect("load context snapshot after window change");

    assert_eq!(summary_streaming_query_count_for_tests("rebuild"), 0);
    assert_eq!(summary_streaming_query_count_for_tests("catch_up"), 1);
    assert_eq!(
        summary_normalization_count_for_tests(),
        0,
        "expected window shrink catch-up path to stop materializing scratch normalization buffers"
    );
    assert_eq!(snapshot.window_turns.len(), 1);

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn summary_catch_up_avoids_scratch_normalization_buffer() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_summary_materialization_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-fused-append-catch-up-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-fused-append-catch-up.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "summary-fused-append-catch-up-session",
        "user",
        "turn 1",
        &config,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "summary-fused-append-catch-up-session",
        "assistant",
        "turn 2",
        &config,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "summary-fused-append-catch-up-session",
        "user",
        "turn 3",
        &config,
    )
    .expect("append turn 3 should succeed");

    reset_summary_materialization_metrics_for_tests();
    append_turn_direct(
        "summary-fused-append-catch-up-session",
        "assistant",
        "turn 4",
        &config,
    )
    .expect("append turn 4 should succeed");

    assert_eq!(summary_streaming_query_count_for_tests("catch_up"), 1);
    assert_eq!(
        summary_normalization_count_for_tests(),
        0,
        "expected catch-up path to stop materializing scratch normalization buffers"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn append_summary_line_preserves_whitespace_collapse_and_utf8_safe_truncation() {
    let mut summary_body = String::new();

    append_summary_line(
        &mut summary_body,
        "user",
        " \n 你好\t世界  again \r\n next ",
        19,
    );

    assert_eq!(summary_body, "- user: 你好 世");
}

#[test]
fn context_snapshot_separates_materialized_summary_from_active_window() {
    let tmp =
        std::env::temp_dir().join(format!("loong-context-snapshot-{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("context-snapshot.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config =
        sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);

    append_turn_direct("snapshot-session", "user", "turn 1", &config)
        .expect("append turn 1 should succeed");
    append_turn_direct("snapshot-session", "assistant", "turn 2", &config)
        .expect("append turn 2 should succeed");
    append_turn_direct("snapshot-session", "user", "turn 3", &config)
        .expect("append turn 3 should succeed");
    append_turn_direct("snapshot-session", "assistant", "turn 4", &config)
        .expect("append turn 4 should succeed");

    let snapshot =
        load_context_snapshot("snapshot-session", &config).expect("load context snapshot");

    assert!(
        snapshot
            .summary_body
            .as_deref()
            .is_some_and(|summary| summary.contains("turn 1")),
        "expected summary body to include turn 1"
    );
    assert!(
        snapshot
            .summary_body
            .as_deref()
            .is_some_and(|summary| summary.contains("turn 2")),
        "expected summary body to include turn 2"
    );

    assert_eq!(snapshot.window_turns.len(), 2);
    assert_eq!(snapshot.window_turns[0].content, "turn 3");
    assert_eq!(snapshot.window_turns[1].content, "turn 4");

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[test]
fn append_turn_materializes_summary_checkpoint_once_window_overflows() {
    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-checkpoint-materialized-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("summary-checkpoint.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct("checkpoint-session", "user", "turn 1", &config)
        .expect("append turn 1 should succeed");
    append_turn_direct("checkpoint-session", "assistant", "turn 2", &config)
        .expect("append turn 2 should succeed");
    append_turn_direct("checkpoint-session", "user", "turn 3", &config)
        .expect("append turn 3 should succeed");
    append_turn_direct("checkpoint-session", "assistant", "turn 4", &config)
        .expect("append turn 4 should succeed");

    let (summarized_through_turn_id, summary_body, summary_budget_chars, summary_window_size) =
        read_summary_checkpoint(&config, "checkpoint-session")
            .expect("summary checkpoint row should exist");

    assert_eq!(summarized_through_turn_id, 2);
    assert!(summary_body.contains("turn 1"));
    assert!(summary_body.contains("turn 2"));
    assert_eq!(summary_budget_chars, 256);
    assert_eq!(summary_window_size, 2);

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[test]
fn initial_summary_checkpoint_waits_for_visible_overflow() {
    let tmp = std::env::temp_dir().join(format!(
        "loong-initial-summary-visible-overflow-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("initial-summary-visible-overflow.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);
    let hidden_turn = crate::memory::build_conversation_event_content(
        "provider_prompt_frame_snapshot",
        serde_json::json!({"phase": "initial"}),
    );

    append_turn_direct(
        "initial-summary-visible-overflow",
        "user",
        "visible 1",
        &config,
    )
    .expect("append visible turn 1 should succeed");
    append_turn_direct(
        "initial-summary-visible-overflow",
        "assistant",
        hidden_turn.as_str(),
        &config,
    )
    .expect("append hidden prompt-frame turn should succeed");
    append_turn_direct(
        "initial-summary-visible-overflow",
        "assistant",
        "visible 2",
        &config,
    )
    .expect("append visible turn 2 should succeed");

    assert_eq!(
        count_summary_checkpoints(&config, "initial-summary-visible-overflow")
            .expect("count checkpoints after raw overflow"),
        0,
        "raw overflow without visible overflow must not materialize a checkpoint",
    );

    let pre_overflow_snapshot =
        load_context_snapshot("initial-summary-visible-overflow", &config)
            .expect("load pre-overflow context snapshot");
    let pre_overflow_contents = pre_overflow_snapshot
        .window_turns
        .iter()
        .map(|turn| turn.content.as_str())
        .collect::<Vec<_>>();

    assert_eq!(pre_overflow_contents, vec!["visible 1", "visible 2"]);
    assert!(pre_overflow_snapshot.summary_body.is_none());

    append_turn_direct(
        "initial-summary-visible-overflow",
        "user",
        "visible 3",
        &config,
    )
    .expect("append visible turn 3 should succeed");

    assert_eq!(
        count_summary_checkpoints(&config, "initial-summary-visible-overflow")
            .expect("count checkpoints after visible overflow"),
        1,
        "checkpoint should materialize once the visible window actually overflows",
    );

    let post_overflow_snapshot =
        load_context_snapshot("initial-summary-visible-overflow", &config)
            .expect("load post-overflow context snapshot");
    let post_overflow_contents = post_overflow_snapshot
        .window_turns
        .iter()
        .map(|turn| turn.content.as_str())
        .collect::<Vec<_>>();

    assert_eq!(post_overflow_contents, vec!["visible 2", "visible 3"]);
    assert!(post_overflow_snapshot.summary_body.is_some());

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[test]
fn load_context_snapshot_rebuilds_materialized_summary_when_window_size_changes() {
    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-checkpoint-window-rebuild-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("summary-window-rebuild.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config_window_two = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "window-rebuild-session",
        "user",
        "turn 1",
        &config_window_two,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "window-rebuild-session",
        "assistant",
        "turn 2",
        &config_window_two,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "window-rebuild-session",
        "user",
        "turn 3",
        &config_window_two,
    )
    .expect("append turn 3 should succeed");
    append_turn_direct(
        "window-rebuild-session",
        "assistant",
        "turn 4",
        &config_window_two,
    )
    .expect("append turn 4 should succeed");

    let config_window_three = MemoryRuntimeConfig {
        sliding_window: 3,
        ..config_window_two
    };
    let snapshot = load_context_snapshot("window-rebuild-session", &config_window_three)
        .expect("load context snapshot after window change");
    let (summarized_through_turn_id, summary_body, _summary_budget_chars, summary_window_size) =
        read_summary_checkpoint(&config_window_three, "window-rebuild-session")
            .expect("summary checkpoint row should exist after rebuild");

    assert_eq!(summarized_through_turn_id, 1);
    assert!(summary_body.contains("turn 1"));
    assert!(!summary_body.contains("turn 2"));
    assert_eq!(summary_window_size, 3);
    assert_eq!(snapshot.window_turns.len(), 3);
    assert_eq!(snapshot.window_turns[0].content, "turn 2");
    assert_eq!(snapshot.window_turns[2].content, "turn 4");

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[test]
fn load_context_snapshot_updates_checkpoint_metadata_without_rewriting_body_when_frontier_is_stable()
 {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_cached_prepare_metrics_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-checkpoint-window-metadata-only-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-window-metadata-only.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config_window_two = sqlite_test_summary_config(db_path.clone(), 2, 256);
    let mut config_window_only =
        sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);
    config_window_only.summary_max_chars = 256;
    let config_window_three = MemoryRuntimeConfig {
        sliding_window: 3,
        ..config_window_two.clone()
    };

    for (role, content) in [
        ("user", "turn 1"),
        ("assistant", "turn 2"),
        ("user", "turn 3"),
        ("assistant", "turn 4"),
        ("user", "turn 5"),
    ] {
        append_turn_direct(
            "window-metadata-only-session",
            role,
            content,
            &config_window_two,
        )
        .expect("append turn under summary config should succeed");
    }

    let (through_before, summary_before, _budget_before, window_before) =
        read_summary_checkpoint(&config_window_two, "window-metadata-only-session")
            .expect("summary checkpoint should exist before metadata-only update");
    assert_eq!(through_before, 3);
    assert!(summary_before.contains("turn 1"));
    assert!(summary_before.contains("turn 3"));
    assert_eq!(window_before, 2);

    append_turn_direct(
        "window-metadata-only-session",
        "assistant",
        "turn 6",
        &config_window_only,
    )
    .expect("append turn under window-only config should succeed");

    reset_cached_prepare_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();
    let snapshot = load_context_snapshot("window-metadata-only-session", &config_window_three)
        .expect("load context snapshot after metadata-only window drift");
    let (through_after, summary_after, _budget_after, window_after) =
        read_summary_checkpoint(&config_window_three, "window-metadata-only-session")
            .expect("summary checkpoint should exist after metadata-only update");

    assert_eq!(through_after, 3);
    assert_eq!(summary_after, summary_before);
    assert_eq!(window_after, 3);
    assert_eq!(
        snapshot.summary_body.as_deref(),
        Some(summary_before.as_str())
    );
    assert_eq!(snapshot.window_turns.len(), 3);
    assert_eq!(snapshot.window_turns[0].content, "turn 4");
    assert_eq!(snapshot.window_turns[2].content, "turn 6");
    assert!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT summary_body\n             FROM memory_summary_checkpoint_bodies"
        ) >= 1,
        "expected metadata-only window drift to hydrate the persisted summary through the detached checkpoint body table"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("RETURNING summary_body"),
        0,
        "expected metadata-only window drift to avoid UPDATE ... RETURNING body hydration after splitting summary storage"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn load_context_snapshot_uses_compatible_checkpoint_body_fast_path() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-checkpoint-compatible-fast-path-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-compatible-fast-path.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

    for (role, content) in [
        ("user", "turn 1"),
        ("assistant", "turn 2"),
        ("user", "turn 3"),
        ("assistant", "turn 4"),
    ] {
        append_turn_direct(
            "summary-compatible-fast-path-session",
            role,
            content,
            &config,
        )
        .expect("append turn under summary config should succeed");
    }

    let first_snapshot = load_context_snapshot("summary-compatible-fast-path-session", &config)
        .expect("initial summary snapshot should succeed");
    assert!(first_snapshot.summary_body.is_some());
    assert_eq!(first_snapshot.window_turns.len(), 2);

    reset_cached_prepare_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();
    let second_snapshot =
        load_context_snapshot("summary-compatible-fast-path-session", &config)
            .expect("compatible summary snapshot should succeed");

    assert_eq!(second_snapshot.summary_body, first_snapshot.summary_body);
    assert_eq!(second_snapshot.window_turns.len(), 2);
    assert_eq!(second_snapshot.window_turns[0].content, "turn 3");
    assert_eq!(second_snapshot.window_turns[1].content, "turn 4");
    assert!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT summary_body\n             FROM memory_summary_checkpoint_bodies"
        ) >= 1,
        "expected a fully compatible summary snapshot to load the checkpoint body from the detached body table"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("RETURNING summary_body"),
        0,
        "expected a fully compatible summary snapshot to avoid metadata repair writes"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn load_context_snapshot_uses_catch_up_when_window_shrinks() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_summary_materialization_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-checkpoint-window-shrink-catch-up-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-window-shrink-catch-up.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config_window_three = sqlite_test_summary_config(db_path.clone(), 3, 256);

    append_turn_direct(
        "window-shrink-catch-up-session",
        "user",
        "turn 1",
        &config_window_three,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "window-shrink-catch-up-session",
        "assistant",
        "turn 2",
        &config_window_three,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "window-shrink-catch-up-session",
        "user",
        "turn 3",
        &config_window_three,
    )
    .expect("append turn 3 should succeed");
    append_turn_direct(
        "window-shrink-catch-up-session",
        "assistant",
        "turn 4",
        &config_window_three,
    )
    .expect("append turn 4 should succeed");
    append_turn_direct(
        "window-shrink-catch-up-session",
        "user",
        "turn 5",
        &config_window_three,
    )
    .expect("append turn 5 should succeed");

    let (through_before, summary_before, _budget_before, window_before) =
        read_summary_checkpoint(&config_window_three, "window-shrink-catch-up-session")
            .expect("summary checkpoint should exist before shrink");
    assert_eq!(through_before, 2);
    assert_eq!(window_before, 3);
    assert!(summary_before.contains("turn 1"));
    assert!(summary_before.contains("turn 2"));

    reset_summary_materialization_metrics_for_tests();
    let config_window_two = MemoryRuntimeConfig {
        sliding_window: 2,
        ..config_window_three
    };
    let snapshot = load_context_snapshot("window-shrink-catch-up-session", &config_window_two)
        .expect("load context snapshot after shrinking window");
    let (through_after, summary_after, _budget_after, window_after) =
        read_summary_checkpoint(&config_window_two, "window-shrink-catch-up-session")
            .expect("summary checkpoint should exist after shrink");

    assert_eq!(through_after, 3);
    assert_eq!(window_after, 2);
    assert!(summary_after.contains("turn 3"));
    assert_eq!(snapshot.window_turns.len(), 2);
    assert_eq!(snapshot.window_turns[0].content, "turn 4");
    assert_eq!(snapshot.window_turns[1].content, "turn 5");
    assert_eq!(
        summary_streaming_query_count_for_tests("rebuild"),
        0,
        "expected shrink path to avoid full rebuild when existing checkpoint can catch up"
    );
    assert_eq!(
        summary_streaming_query_count_for_tests("catch_up"),
        1,
        "expected shrink path to extend the existing checkpoint incrementally"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn load_context_snapshot_catch_up_probes_frontier_when_saturated() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_summary_materialization_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-window-shrink-saturated-catch-up-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("summary-window-shrink-saturated-catch-up.sqlite3");
    let _ = fs::remove_file(&db_path);

    let first_turn = "FIRST-MARKER ".repeat(40);
    let second_turn = "SECOND-MARKER ".repeat(20);
    let config_window_three = sqlite_test_summary_config(db_path.clone(), 3, 256);

    append_turn_direct(
        "window-shrink-saturated-catch-up-session",
        "user",
        &first_turn,
        &config_window_three,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "window-shrink-saturated-catch-up-session",
        "assistant",
        &second_turn,
        &config_window_three,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "window-shrink-saturated-catch-up-session",
        "user",
        "turn 3",
        &config_window_three,
    )
    .expect("append turn 3 should succeed");
    append_turn_direct(
        "window-shrink-saturated-catch-up-session",
        "assistant",
        "turn 4",
        &config_window_three,
    )
    .expect("append turn 4 should succeed");
    append_turn_direct(
        "window-shrink-saturated-catch-up-session",
        "user",
        "turn 5",
        &config_window_three,
    )
    .expect("append turn 5 should succeed");

    let (through_before, summary_before, _budget_before, window_before) =
        read_summary_checkpoint(
            &config_window_three,
            "window-shrink-saturated-catch-up-session",
        )
        .expect("summary checkpoint should exist before shrink");
    assert_eq!(through_before, 2);
    assert_eq!(window_before, 3);
    assert!(summary_before.contains("FIRST-MARKER"));
    assert!(!summary_before.contains("SECOND-MARKER"));

    reset_summary_materialization_metrics_for_tests();
    let config_window_two = MemoryRuntimeConfig {
        sliding_window: 2,
        ..config_window_three
    };
    let snapshot = load_context_snapshot(
        "window-shrink-saturated-catch-up-session",
        &config_window_two,
    )
    .expect("load context snapshot after shrinking saturated window");
    let (through_after, summary_after, _budget_after, window_after) = read_summary_checkpoint(
        &config_window_two,
        "window-shrink-saturated-catch-up-session",
    )
    .expect("summary checkpoint should exist after saturated shrink catch-up");

    assert_eq!(through_after, 3);
    assert_eq!(window_after, 2);
    assert_eq!(summary_after, summary_before);
    assert_eq!(snapshot.window_turns.len(), 2);
    assert_eq!(
        summary_streaming_query_count_for_tests("catch_up"),
        1,
        "expected saturated shrink path to run one catch-up stream"
    );
    assert_eq!(
        summary_payload_decode_count_for_tests(),
        0,
        "expected saturated shrink catch-up to avoid decoding additional payload fragments"
    );
    assert_eq!(
        summary_frontier_probe_count_for_tests("catch_up"),
        1,
        "expected saturated catch-up to use one frontier probe after summary saturation"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn recent_turn_query_with_boundary_id_returns_oldest_active_window_turn() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-window-boundary-query-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("window-boundary.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowOnly, 2);

    append_turn_direct("window-boundary-session", "user", "turn 1", &config)
        .expect("append turn 1 should succeed");
    append_turn_direct("window-boundary-session", "assistant", "turn 2", &config)
        .expect("append turn 2 should succeed");
    append_turn_direct("window-boundary-session", "user", "turn 3", &config)
        .expect("append turn 3 should succeed");

    let runtime = acquire_memory_runtime(&config).expect("acquire runtime");
    let recent_window = runtime
        .with_connection("test.query_recent_turns_with_boundary_id", |conn| {
            query_recent_turns_with_boundary_id(conn, "window-boundary-session", 2)
        })
        .expect("query turns with boundary id");

    assert_eq!(recent_window.turns.len(), 2);
    assert_eq!(recent_window.turns[0].content, "turn 2");
    assert_eq!(recent_window.turns[1].content, "turn 3");
    assert_eq!(recent_window.summary_before_turn_id, Some(2));
    assert!(
        !recent_window.window_starts_at_session_origin,
        "expected a three-turn session with a two-turn window to preserve older-turn context"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn load_context_snapshot_rebuilds_materialized_summary_when_budget_changes() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-checkpoint-budget-rebuild-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("summary-budget-rebuild.sqlite3");
    let _ = fs::remove_file(&db_path);

    let first_turn = "alpha ".repeat(40);
    let second_turn = "SECOND-MARKER ".repeat(8);

    let config_small_budget = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "budget-rebuild-session",
        "user",
        &first_turn,
        &config_small_budget,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "budget-rebuild-session",
        "assistant",
        &second_turn,
        &config_small_budget,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "budget-rebuild-session",
        "user",
        "turn 3",
        &config_small_budget,
    )
    .expect("append turn 3 should succeed");
    append_turn_direct(
        "budget-rebuild-session",
        "assistant",
        "turn 4",
        &config_small_budget,
    )
    .expect("append turn 4 should succeed");

    let (_through_small, small_summary_body, small_budget, _window_small) =
        read_summary_checkpoint(&config_small_budget, "budget-rebuild-session")
            .expect("small-budget checkpoint should exist");
    assert_eq!(small_budget, 256);

    let config_large_budget = MemoryRuntimeConfig {
        summary_max_chars: 512,
        ..config_small_budget
    };
    reset_cached_prepare_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();
    let snapshot = load_context_snapshot("budget-rebuild-session", &config_large_budget)
        .expect("load context snapshot after budget change");
    let (_through_large, large_summary_body, large_budget, _window_large) =
        read_summary_checkpoint(&config_large_budget, "budget-rebuild-session")
            .expect("large-budget checkpoint should exist");

    assert_eq!(large_budget, 512);
    assert!(small_summary_body.len() <= 256);
    assert!(!small_summary_body.contains("SECOND-MARKER"));
    assert!(large_summary_body.contains("SECOND-MARKER"));
    assert_eq!(snapshot.window_turns.len(), 2);
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT summarized_through_turn_id, summary_before_turn_id, summary_body_bytes, summary_budget_chars, summary_window_size, summary_format_version"
        ),
        0,
        "expected budget-change rebuild to avoid a second checkpoint metadata lookup after the known-overflow window query already loaded that metadata"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "LEFT JOIN memory_summary_checkpoints checkpoint"
        ),
        1,
        "expected budget-change rebuild to co-load checkpoint metadata with the known-overflow window query"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT summary_body\n             FROM memory_summary_checkpoint_bodies"
        ),
        0,
        "expected budget-change rebuild to avoid loading the existing summary body when metadata already proves a rebuild is required"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[test]
fn load_context_snapshot_skips_rebuild_when_budget_changes_but_summary_is_unsaturated() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_summary_materialization_metrics_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-checkpoint-budget-metadata-only-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("summary-budget-metadata-only.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config_small_budget = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "budget-metadata-only-session",
        "user",
        "turn 1",
        &config_small_budget,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "budget-metadata-only-session",
        "assistant",
        "turn 2",
        &config_small_budget,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "budget-metadata-only-session",
        "user",
        "turn 3",
        &config_small_budget,
    )
    .expect("append turn 3 should succeed");
    append_turn_direct(
        "budget-metadata-only-session",
        "assistant",
        "turn 4",
        &config_small_budget,
    )
    .expect("append turn 4 should succeed");

    let (_through_small, small_summary_body, small_budget, _window_small) =
        read_summary_checkpoint(&config_small_budget, "budget-metadata-only-session")
            .expect("small-budget checkpoint should exist");
    assert_eq!(small_budget, 256);
    assert!(small_summary_body.len() < 256);

    let config_large_budget = MemoryRuntimeConfig {
        summary_max_chars: 512,
        ..config_small_budget
    };
    reset_cached_prepare_metrics_for_tests();
    reset_summary_materialization_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();
    let snapshot = load_context_snapshot("budget-metadata-only-session", &config_large_budget)
        .expect("load context snapshot after unsaturated budget change");
    let (_through_large, large_summary_body, large_budget, _window_large) =
        read_summary_checkpoint(&config_large_budget, "budget-metadata-only-session")
            .expect("large-budget checkpoint should exist");

    assert_eq!(large_budget, 512);
    assert_eq!(large_summary_body, small_summary_body);
    assert_eq!(
        snapshot.summary_body.as_deref(),
        Some(small_summary_body.as_str())
    );
    assert_eq!(snapshot.window_turns.len(), 2);
    assert_eq!(
        summary_streaming_query_count_for_tests("rebuild"),
        0,
        "expected unsaturated budget changes to avoid a full summary rebuild when the existing checkpoint already covers all summarized turns"
    );
    assert!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT summary_body\n             FROM memory_summary_checkpoint_bodies"
        ) >= 1,
        "expected unsaturated budget changes to load the reusable summary body from the detached body table"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("RETURNING summary_body"),
        0,
        "expected unsaturated budget changes to avoid UPDATE ... RETURNING body hydration after splitting summary storage"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[test]
fn load_context_snapshot_diagnostics_identify_metadata_only_budget_change_fast_path() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-checkpoint-budget-load-diagnostics-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("summary-budget-load-diagnostics.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config_small_budget = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "budget-load-diagnostics-session",
        "user",
        "turn 1",
        &config_small_budget,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "budget-load-diagnostics-session",
        "assistant",
        "turn 2",
        &config_small_budget,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "budget-load-diagnostics-session",
        "user",
        "turn 3",
        &config_small_budget,
    )
    .expect("append turn 3 should succeed");
    append_turn_direct(
        "budget-load-diagnostics-session",
        "assistant",
        "turn 4",
        &config_small_budget,
    )
    .expect("append turn 4 should succeed");

    let (_through_small, small_summary_body, _small_budget, _window_small) =
        read_summary_checkpoint(&config_small_budget, "budget-load-diagnostics-session")
            .expect("small-budget checkpoint should exist");
    assert!(small_summary_body.len() < 256);

    let config_large_budget = MemoryRuntimeConfig {
        summary_max_chars: 512,
        ..config_small_budget
    };
    let (snapshot, diagnostics) = load_context_snapshot_with_diagnostics(
        "budget-load-diagnostics-session",
        &config_large_budget,
    )
    .expect("load context snapshot diagnostics after unsaturated budget change");

    assert_eq!(
        snapshot.summary_body.as_deref(),
        Some(small_summary_body.as_str())
    );
    assert_eq!(snapshot.window_turns.len(), 2);
    assert!(diagnostics.window_query_ms > 0.0);
    assert_eq!(
        diagnostics.summary_checkpoint_meta_query_ms, 0.0,
        "expected known-overflow diagnostics to fold checkpoint metadata into the window query instead of issuing a second metadata lookup"
    );
    assert!(
        diagnostics.summary_checkpoint_metadata_update_ms > 0.0,
        "expected metadata-only budget change to reuse the loaded checkpoint body and finish with a metadata-only UPDATE"
    );
    assert!(
        diagnostics.summary_checkpoint_body_load_ms > 0.0,
        "expected metadata-only budget change to load the reusable checkpoint body separately from the metadata query"
    );
    assert_eq!(
        diagnostics.summary_checkpoint_metadata_update_returning_body_ms,
        0.0
    );
    assert_eq!(diagnostics.summary_rebuild_ms, 0.0);
    assert_eq!(diagnostics.summary_catch_up_ms, 0.0);

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[test]
fn load_context_snapshot_skips_redundant_meta_query_when_window_probe_already_proves_checkpoint_absent()
 {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-known-absent-checkpoint-meta-query-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("known-absent-checkpoint-meta-query.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

    for (role, content) in [
        ("user", "turn 1"),
        ("assistant", "turn 2"),
        ("user", "turn 3"),
        ("assistant", "turn 4"),
    ] {
        append_turn_direct(
            "known-absent-checkpoint-meta-query-session",
            role,
            content,
            &config,
        )
        .expect("append turn should succeed");
    }

    assert_eq!(
        count_summary_checkpoints(&config, "known-absent-checkpoint-meta-query-session")
            .expect("count summary checkpoints before deletion"),
        1,
        "expected overflowing append path to materialize a checkpoint before the test deletes it"
    );

    let runtime = acquire_memory_runtime(&config).expect("acquire memory runtime");
    runtime
        .with_connection_mut("test.delete_summary_checkpoint_before_rebuild", |conn| {
            delete_summary_checkpoint(conn, "known-absent-checkpoint-meta-query-session")
        })
        .expect("delete summary checkpoint before rebuild");

    assert_eq!(
        count_summary_checkpoints(&config, "known-absent-checkpoint-meta-query-session")
            .expect("count summary checkpoints after deletion"),
        0
    );

    let (snapshot, diagnostics) = load_context_snapshot_with_diagnostics(
        "known-absent-checkpoint-meta-query-session",
        &config,
    )
    .expect("load context snapshot after deleting checkpoint");

    assert!(snapshot.summary_body.is_some());
    assert_eq!(snapshot.window_turns.len(), 2);
    assert!(diagnostics.window_query_ms > 0.0);
    assert_eq!(
        diagnostics.summary_checkpoint_meta_query_ms, 0.0,
        "expected known-overflow window probe to carry enough checkpoint absence state to avoid a second metadata lookup before rebuild"
    );
    assert!(
        diagnostics.summary_rebuild_ms > 0.0,
        "expected rebuild path to recreate the missing checkpoint"
    );
    assert_eq!(
        count_summary_checkpoints(&config, "known-absent-checkpoint-meta-query-session")
            .expect("count summary checkpoints after rebuild"),
        1
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[test]
fn load_context_snapshot_diagnostics_split_exact_window_query_costs() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-exact-window-query-diagnostics-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("exact-window-query-diagnostics.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "exact-window-query-diagnostics-session",
        "user",
        "turn 1",
        &config,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "exact-window-query-diagnostics-session",
        "assistant",
        "turn 2",
        &config,
    )
    .expect("append turn 2 should succeed");

    let (snapshot, diagnostics) = load_context_snapshot_with_diagnostics(
        "exact-window-query-diagnostics-session",
        &config,
    )
    .expect("load exact-window snapshot diagnostics");

    assert!(snapshot.summary_body.is_none());
    assert_eq!(snapshot.window_turns.len(), 2);
    assert!(diagnostics.window_query_ms > 0.0);
    assert!(diagnostics.window_turn_count_query_ms > 0.0);
    assert!(diagnostics.window_exact_rows_query_ms > 0.0);
    assert_eq!(diagnostics.window_known_overflow_rows_query_ms, 0.0);
    assert_eq!(diagnostics.window_fallback_rows_query_ms, 0.0);
    assert!(
        diagnostics.window_turn_count_query_ms + diagnostics.window_exact_rows_query_ms
            <= diagnostics.window_query_ms + 1.0
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn load_context_snapshot_diagnostics_split_known_overflow_window_query_costs() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-known-overflow-window-query-diagnostics-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("known-overflow-window-query-diagnostics.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "known-overflow-window-query-diagnostics-session",
        "user",
        "turn 1",
        &config,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "known-overflow-window-query-diagnostics-session",
        "assistant",
        "turn 2",
        &config,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "known-overflow-window-query-diagnostics-session",
        "user",
        "turn 3",
        &config,
    )
    .expect("append turn 3 should succeed");

    let (snapshot, diagnostics) = load_context_snapshot_with_diagnostics(
        "known-overflow-window-query-diagnostics-session",
        &config,
    )
    .expect("load known-overflow snapshot diagnostics");

    assert!(snapshot.summary_body.is_some());
    assert_eq!(snapshot.window_turns.len(), 2);
    assert!(diagnostics.window_query_ms > 0.0);
    assert!(diagnostics.window_turn_count_query_ms > 0.0);
    assert!(diagnostics.window_known_overflow_rows_query_ms > 0.0);
    assert_eq!(diagnostics.window_exact_rows_query_ms, 0.0);
    assert_eq!(diagnostics.window_fallback_rows_query_ms, 0.0);
    assert_eq!(diagnostics.summary_checkpoint_meta_query_ms, 0.0);
    assert!(
        diagnostics.window_turn_count_query_ms
            + diagnostics.window_known_overflow_rows_query_ms
            <= diagnostics.window_query_ms + 1.0
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn load_context_snapshot_diagnostics_split_fallback_window_query_costs_when_turn_count_is_missing()
 {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-fallback-window-query-diagnostics-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("fallback-window-query-diagnostics.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "fallback-window-query-diagnostics-session",
        "user",
        "turn 1",
        &config,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "fallback-window-query-diagnostics-session",
        "assistant",
        "turn 2",
        &config,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "fallback-window-query-diagnostics-session",
        "user",
        "turn 3",
        &config,
    )
    .expect("append turn 3 should succeed");

    let runtime = acquire_memory_runtime(&config).expect("acquire memory runtime");
    runtime
        .with_connection_mut(
            "test.delete_turn_count_before_fallback_diagnostics",
            |conn| {
                conn.execute(
                    "DELETE FROM memory_session_state
                 WHERE session_id = ?1",
                    rusqlite::params!["fallback-window-query-diagnostics-session"],
                )
                .map_err(|error| format!("delete session turn count failed: {error}"))?;
                Ok(())
            },
        )
        .expect("delete turn count before fallback diagnostics");

    let (snapshot, diagnostics) = load_context_snapshot_with_diagnostics(
        "fallback-window-query-diagnostics-session",
        &config,
    )
    .expect("load fallback snapshot diagnostics");

    assert!(snapshot.summary_body.is_some());
    assert_eq!(snapshot.window_turns.len(), 2);
    assert!(diagnostics.window_query_ms > 0.0);
    assert!(diagnostics.window_turn_count_query_ms > 0.0);
    assert!(diagnostics.window_fallback_rows_query_ms > 0.0);
    assert_eq!(diagnostics.window_exact_rows_query_ms, 0.0);
    assert_eq!(diagnostics.window_known_overflow_rows_query_ms, 0.0);
    assert!(
        diagnostics.summary_checkpoint_meta_query_ms > 0.0,
        "expected fallback window diagnostics to keep the checkpoint metadata lookup once the initial window probe lacks checkpoint state"
    );
    assert!(
        diagnostics.window_turn_count_query_ms + diagnostics.window_fallback_rows_query_ms
            <= diagnostics.window_query_ms + 1.0
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn clear_session_removes_materialized_summary_checkpoint() {
    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-checkpoint-clear-session-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("summary-clear-session.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct("clear-checkpoint-session", "user", "turn 1", &config)
        .expect("append turn 1 should succeed");
    append_turn_direct("clear-checkpoint-session", "assistant", "turn 2", &config)
        .expect("append turn 2 should succeed");
    append_turn_direct("clear-checkpoint-session", "user", "turn 3", &config)
        .expect("append turn 3 should succeed");

    let before_clear = count_summary_checkpoints(&config, "clear-checkpoint-session")
        .expect("count checkpoint rows before clear");
    assert_eq!(before_clear, 1);

    clear_session(
        MemoryCoreRequest {
            operation: MEMORY_OP_CLEAR_SESSION.to_owned(),
            payload: json!({
                "session_id": "clear-checkpoint-session",
            }),
        },
        &config,
    )
    .expect("clear session should succeed");

    let after_clear = count_summary_checkpoints(&config, "clear-checkpoint-session")
        .expect("count checkpoint rows after clear");
    assert_eq!(after_clear, 0);

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[test]
fn ensure_memory_db_ready_migrates_legacy_summary_checkpoint_body_bytes() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-summary-checkpoint-legacy-migration-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("legacy-summary-checkpoint.sqlite3");
    let _ = fs::remove_file(&db_path);

    let conn = Connection::open(&db_path).expect("open legacy sqlite db");
    configure_sqlite_connection(&conn).expect("configure legacy sqlite db");
    conn.execute_batch(
        "
        CREATE TABLE turns(
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          session_id TEXT NOT NULL,
          role TEXT NOT NULL,
          content TEXT NOT NULL,
          ts INTEGER NOT NULL
        );
        CREATE INDEX idx_turns_session_id ON turns(session_id, id);
        CREATE TABLE memory_summary_checkpoints(
          session_id TEXT PRIMARY KEY,
          summarized_through_turn_id INTEGER NOT NULL,
          summary_body TEXT NOT NULL,
          summary_budget_chars INTEGER NOT NULL,
          summary_window_size INTEGER NOT NULL,
          summary_format_version INTEGER NOT NULL,
          updated_at_ts INTEGER NOT NULL
        );
        ",
    )
    .expect("create legacy schema");
    for (role, content) in [
        ("user", "turn 1"),
        ("assistant", "turn 2"),
        ("user", "turn 3"),
        ("assistant", "turn 4"),
        ("user", "turn 5"),
        ("assistant", "turn 6"),
        ("user", "turn 7"),
        ("assistant", "turn 8"),
        ("user", "turn 9"),
        ("assistant", "turn 10"),
    ] {
        conn.execute(
            "INSERT INTO turns(session_id, role, content, ts) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["legacy-session", role, content, unix_ts_now()],
        )
        .expect("insert legacy turn");
    }
    conn.execute(
        "INSERT INTO memory_summary_checkpoints(
            session_id,
            summarized_through_turn_id,
            summary_body,
            summary_budget_chars,
            summary_window_size,
            summary_format_version,
            updated_at_ts
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            "legacy-session",
            7_i64,
            "legacy summary body",
            256_i64,
            4_i64,
            SUMMARY_FORMAT_VERSION,
            unix_ts_now(),
        ],
    )
    .expect("insert legacy checkpoint");
    drop(conn);

    let config = sqlite_test_summary_config(db_path.clone(), 4, 256);

    ensure_memory_db_ready(Some(db_path.clone()), &config)
        .expect("migrate legacy sqlite memory db");

    let runtime = acquire_memory_runtime(&config).expect("acquire migrated runtime");
    let (summary_body_bytes, summary_before_turn_id) = runtime
        .with_connection("test.read_summary_checkpoint_metadata", |conn| {
            conn.query_row(
                "SELECT summary_body_bytes, summary_before_turn_id
                 FROM memory_summary_checkpoints
                 WHERE session_id = ?1",
                rusqlite::params!["legacy-session"],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)),
            )
            .map_err(|error| {
                format!("read migrated summary checkpoint metadata failed: {error}")
            })
        })
        .expect("read migrated summary checkpoint metadata");
    let migrated_summary_body = runtime
        .with_connection("test.read_summary_checkpoint_body", |conn| {
            conn.query_row(
                "SELECT summary_body
                 FROM memory_summary_checkpoint_bodies
                 WHERE session_id = ?1",
                rusqlite::params!["legacy-session"],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| format!("read migrated summary checkpoint body failed: {error}"))
        })
        .expect("read migrated summary checkpoint body");
    let checkpoint_columns = runtime
        .with_connection("test.read_summary_checkpoint_column_order", |conn| {
            let mut stmt = conn
                .prepare("PRAGMA table_info(memory_summary_checkpoints)")
                .map_err(|error| {
                    format!("prepare summary checkpoint table info query failed: {error}")
                })?;
            let mut rows = stmt.query([]).map_err(|error| {
                format!("query summary checkpoint table info failed: {error}")
            })?;
            let mut names = Vec::new();
            while let Some(row) = rows.next().map_err(|error| {
                format!("read summary checkpoint table info row failed: {error}")
            })? {
                names.push(row.get::<_, String>(1).map_err(|error| {
                    format!("decode summary checkpoint table info column failed: {error}")
                })?);
            }
            Ok(names)
        })
        .expect("read summary checkpoint table info");

    assert_eq!(summary_body_bytes, "legacy summary body".len() as i64);
    assert_eq!(summary_before_turn_id, Some(8));
    assert_eq!(migrated_summary_body, "legacy summary body");
    assert_eq!(
        checkpoint_columns,
        vec![
            "session_id".to_owned(),
            "summarized_through_turn_id".to_owned(),
            "summary_before_turn_id".to_owned(),
            "summary_body_bytes".to_owned(),
            "summary_budget_chars".to_owned(),
            "summary_window_size".to_owned(),
            "summary_format_version".to_owned(),
            "updated_at_ts".to_owned(),
        ]
    );
    assert_eq!(
        read_session_turn_indices(&config, "legacy-session")
            .expect("read migrated session turn indices"),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    );
    assert_eq!(
        read_session_turn_count(&config, "legacy-session")
            .expect("read migrated session turn count"),
        Some(10)
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn context_snapshot_returns_no_materialized_summary_when_window_covers_session() {
    let tmp = std::env::temp_dir().join(format!(
        "loong-context-snapshot-short-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&tmp);
    let db_path = tmp.join("context-snapshot-short.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config =
        sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 4);

    append_turn_direct("snapshot-short-session", "user", "turn 1", &config)
        .expect("append turn 1 should succeed");
    append_turn_direct("snapshot-short-session", "assistant", "turn 2", &config)
        .expect("append turn 2 should succeed");

    let snapshot = load_context_snapshot("snapshot-short-session", &config)
        .expect("load short context snapshot");

    assert!(snapshot.summary_body.is_none());
    assert_eq!(snapshot.window_turns.len(), 2);
    assert_eq!(snapshot.window_turns[0].content, "turn 1");
    assert_eq!(snapshot.window_turns[1].content, "turn 2");

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir(&tmp);
}

#[test]
fn summary_context_snapshot_avoids_checkpoint_query_when_window_exactly_covers_session() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_cached_prepare_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-context-snapshot-exact-window-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("context-snapshot-exact-window.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config =
        sqlite_test_config_with_profile(db_path.clone(), MemoryProfile::WindowPlusSummary, 2);

    append_turn_direct("snapshot-exact-window-session", "user", "turn 1", &config)
        .expect("append turn 1 should succeed");
    append_turn_direct(
        "snapshot-exact-window-session",
        "assistant",
        "turn 2",
        &config,
    )
    .expect("append turn 2 should succeed");

    reset_cached_prepare_metrics_for_tests();
    let snapshot = load_context_snapshot("snapshot-exact-window-session", &config)
        .expect("load exact-window context snapshot");

    assert!(snapshot.summary_body.is_none());
    assert_eq!(snapshot.window_turns.len(), 2);
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT turn_count\n             FROM memory_session_state"
        ),
        1,
        "expected exact-window summary snapshots to consult session turn-count metadata before choosing the lighter prompt-window query shape"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT id, role, content\n             FROM turns"
        ),
        1,
        "expected exact-window summary snapshots to reuse the visible-turn prompt query once turn-count metadata proves there is no summarized prefix"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT role, content\n             FROM turns"
        ),
        0,
        "expected exact-window summary snapshots to retire the older lean prompt query shape now that internal records require visible-turn filtering by id"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("state.turn_count"),
        0,
        "expected exact-window summary snapshots to retire the heavier joined turn-count query shape"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("session_turn_index"),
        0,
        "expected exact-window summary snapshots to avoid session_turn_index metadata when turn-count metadata already rules out a summarized prefix"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("memory_summary_checkpoints"),
        0,
        "expected summary snapshot to avoid checkpoint queries when the active window already starts at the first session turn"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn summary_context_snapshot_falls_back_to_payload_overflow_probe_when_turn_count_metadata_is_missing()
 {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_cached_prepare_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-context-snapshot-missing-turn-count-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("context-snapshot-missing-turn-count.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct(
        "snapshot-missing-turn-count-session",
        "user",
        "turn 1",
        &config,
    )
    .expect("append turn 1 should succeed");
    append_turn_direct(
        "snapshot-missing-turn-count-session",
        "assistant",
        "turn 2",
        &config,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct(
        "snapshot-missing-turn-count-session",
        "user",
        "turn 3",
        &config,
    )
    .expect("append turn 3 should succeed");

    let runtime = acquire_memory_runtime(&config).expect("acquire memory runtime");
    runtime
        .with_connection_mut("test.delete_missing_turn_count_state", |conn| {
            conn.execute(
                "DELETE FROM memory_session_state
                 WHERE session_id = ?1",
                rusqlite::params!["snapshot-missing-turn-count-session"],
            )
            .map_err(|error| format!("delete session turn count failed: {error}"))?;
            Ok(())
        })
        .expect("delete session turn count");

    reset_cached_prepare_metrics_for_tests();
    let snapshot = load_context_snapshot("snapshot-missing-turn-count-session", &config)
        .expect("load context snapshot with missing turn-count metadata");

    assert!(snapshot.summary_body.is_some());
    assert_eq!(snapshot.window_turns.len(), 2);
    assert_eq!(snapshot.window_turns[0].content, "turn 2");
    assert_eq!(snapshot.window_turns[1].content, "turn 3");
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT turn_count\n             FROM memory_session_state"
        ),
        1,
        "expected summary snapshot to attempt the turn-count-aware fast path before deciding metadata is missing"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT id, role, content\n             FROM turns\n             WHERE session_id = ?1\n             ORDER BY id DESC\n             LIMIT ?2"
        ),
        1,
        "expected summary snapshot to fall back to the legacy overflow-probe query when turn-count metadata is unavailable"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("state.turn_count"),
        0,
        "expected missing turn-count metadata fallback to avoid the older joined turn-count query shape"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("session_turn_index"),
        0,
        "expected missing turn-count metadata fallback to avoid indexed session_turn_index probes"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn summary_context_snapshot_uses_turn_count_metadata_to_choose_known_overflow_query_shape() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();
    reset_cached_prepare_metrics_for_tests();
    let _metrics = begin_sqlite_metric_capture_for_tests();

    let tmp = std::env::temp_dir().join(format!(
        "loong-context-snapshot-known-overflow-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("context-snapshot-known-overflow.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_summary_config(db_path.clone(), 2, 256);

    append_turn_direct("snapshot-known-overflow-session", "user", "turn 1", &config)
        .expect("append turn 1 should succeed");
    append_turn_direct(
        "snapshot-known-overflow-session",
        "assistant",
        "turn 2",
        &config,
    )
    .expect("append turn 2 should succeed");
    append_turn_direct("snapshot-known-overflow-session", "user", "turn 3", &config)
        .expect("append turn 3 should succeed");

    reset_cached_prepare_metrics_for_tests();
    let snapshot = load_context_snapshot("snapshot-known-overflow-session", &config)
        .expect("load context snapshot with known overflow");

    assert!(snapshot.summary_body.is_some());
    assert_eq!(snapshot.window_turns.len(), 2);
    assert_eq!(snapshot.window_turns[0].content, "turn 2");
    assert_eq!(snapshot.window_turns[1].content, "turn 3");
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT turn_count\n             FROM memory_session_state"
        ),
        1,
        "expected overflowing summary snapshots to consult session turn-count metadata once"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "LEFT JOIN memory_summary_checkpoints checkpoint"
        ),
        1,
        "expected overflowing summary snapshots to co-load the active window and checkpoint metadata once turn-count metadata proves overflow"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT id, role, content\n             FROM turns"
        ),
        0,
        "expected known-overflow summary snapshots to retire the older id-only window query once checkpoint metadata is folded into the fast path"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("state.turn_count"),
        0,
        "expected known-overflow summary snapshots to retire the joined turn-count query shape"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests("session_turn_index"),
        0,
        "expected known-overflow summary snapshots to avoid indexed session_turn_index probes on the fast path"
    );
    assert_eq!(
        cached_prepare_count_for_sql_fragment_for_tests(
            "SELECT summarized_through_turn_id, summary_before_turn_id, summary_body_bytes, summary_budget_chars, summary_window_size, summary_format_version"
        ),
        0,
        "expected known-overflow summary snapshots to avoid a second checkpoint metadata lookup after the window query already proved overflow"
    );

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn canonical_memory_search_returns_prior_session_hits_and_excludes_current_session() {
    let tmp = std::env::temp_dir().join(format!(
        "loong-canonical-memory-search-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("canonical-search.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config(db_path.clone());

    append_turn_direct(
        "prior-session",
        "assistant",
        "Deployment cutoff is 17:00 Beijing time and requires a release note.",
        &config,
    )
    .expect("append prior session recall candidate");
    append_turn_direct(
        "active-session",
        "assistant",
        "Deployment cutoff draft that should not be recalled from the active session.",
        &config,
    )
    .expect("append active session recall candidate");
    append_turn_direct(
        "delegate-child",
        "assistant",
        "Delegate child turn that should stay out of root-session recall.",
        &config,
    )
    .expect("append delegate child recall candidate");
    append_turn_direct(
        "root-archived",
        "assistant",
        "Archived root turn that should stay out of resumable recall.",
        &config,
    )
    .expect("append archived root recall candidate");

    let runtime = acquire_memory_runtime(&config).expect("acquire memory runtime");
    runtime
        .with_connection_mut("test.seed_canonical_search_session_metadata", |conn| {
            conn.execute_batch(
                "
                INSERT INTO sessions(session_id, kind, parent_session_id, label, state, created_at, updated_at, last_error)
                VALUES
                  ('prior-session', 'root', NULL, NULL, 'ready', 100, 100, NULL),
                  ('delegate-child', 'delegate_child', 'prior-session', NULL, 'ready', 200, 200, NULL),
                  ('root-archived', 'root', NULL, NULL, 'ready', 300, 300, NULL);
                INSERT INTO session_events(session_id, event_kind, actor_session_id, payload_json, ts)
                VALUES ('root-archived', 'session_archived', NULL, '{}', 400);
                ",
            )
            .map_err(|error| format!("seed canonical search session metadata failed: {error}"))?;
            Ok(())
        })
        .expect("seed canonical search session metadata");

    let hits = search_canonical_records_for_recall(
        "deployment cutoff release note",
        4,
        Some("active-session"),
        &config,
    )
    .expect("search canonical memory");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].record.session_id, "prior-session");
    assert_eq!(hits[0].record.kind, CanonicalMemoryKind::AssistantTurn);
    assert_eq!(hits[0].record.scope, MemoryScope::Session);
    assert_eq!(hits[0].session_turn_index, Some(1));

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn canonical_memory_search_preserves_structured_scope_and_kind_metadata() {
    let tmp = std::env::temp_dir().join(format!(
        "loong-canonical-memory-structured-search-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("canonical-structured-search.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config(db_path.clone());

    let payload = json!({
        "type": crate::memory::CANONICAL_MEMORY_RECORD_TYPE,
        "_loong_internal": true,
        "scope": "workspace",
        "kind": "imported_profile",
        "content": "Workspace release checklist includes rollback and smoke test steps.",
        "metadata": {
            "source": "workspace-import"
        },
    })
    .to_string();

    append_turn_direct("workspace-session", "assistant", &payload, &config)
        .expect("append structured canonical payload");

    let hits = search_canonical_records_for_recall("rollback smoke test", 4, None, &config)
        .expect("search canonical memory");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].record.scope, MemoryScope::Workspace);
    assert_eq!(hits[0].record.kind, CanonicalMemoryKind::ImportedProfile);
    assert_eq!(hits[0].record.metadata["source"], "workspace-import");

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn canonical_memory_search_matches_segmented_chinese_queries() {
    let tmp = std::env::temp_dir().join(format!(
        "loongclaw-canonical-memory-chinese-search-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("canonical-chinese-search.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config(db_path.clone());

    append_turn_direct(
        "workspace-session",
        "assistant",
        "中文分词用于数据库搜索和记忆召回。",
        &config,
    )
    .expect("append chinese canonical payload");

    let hits = search_canonical_records_for_recall("中文 分词", 4, None, &config)
        .expect("search canonical memory by segmented chinese query");

    assert_eq!(hits.len(), 1, "hits={hits:?}");
    assert_eq!(hits[0].record.session_id, "workspace-session");
    assert!(hits[0].record.content.contains("数据库搜索"));

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn canonical_memory_search_matches_metadata_only_queries() {
    let tmp = std::env::temp_dir().join(format!(
        "loong-canonical-memory-metadata-only-search-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("canonical-metadata-only-search.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config(db_path.clone());

    let payload = json!({
        "type": crate::memory::CANONICAL_MEMORY_RECORD_TYPE,
        "_loong_internal": true,
        "scope": "workspace",
        "kind": "imported_profile",
        "content": "release checklist",
        "metadata": {
            "source": "workspace-import"
        },
    })
    .to_string();

    append_turn_direct("workspace-session", "assistant", &payload, &config)
        .expect("append structured canonical payload");

    let hits = search_canonical_records_for_recall("workspace-import", 4, None, &config)
        .expect("search canonical memory by metadata");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].record.scope, MemoryScope::Workspace);
    assert_eq!(hits[0].record.kind, CanonicalMemoryKind::ImportedProfile);
    assert_eq!(hits[0].record.metadata["source"], "workspace-import");

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn ensure_memory_db_ready_repairs_stale_canonical_fts_metadata_schema() {
    let tmp = std::env::temp_dir().join(format!(
        "loong-canonical-memory-stale-fts-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("stale-canonical-fts.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config(db_path.clone());
    ensure_memory_db_ready(None, &config).expect("initialize sqlite db");

    let conn = Connection::open(&db_path).expect("open sqlite db");
    conn.execute_batch(
        "
        DROP TRIGGER IF EXISTS memory_canonical_records_ai;
        DROP TRIGGER IF EXISTS memory_canonical_records_ad;
        DROP TRIGGER IF EXISTS memory_canonical_records_au;
        DROP TABLE IF EXISTS memory_canonical_records_fts;
        CREATE VIRTUAL TABLE memory_canonical_records_fts
          USING fts5(content, content='memory_canonical_records', content_rowid='record_id');
        CREATE TRIGGER memory_canonical_records_ai
          AFTER INSERT ON memory_canonical_records
        BEGIN
          INSERT INTO memory_canonical_records_fts(rowid, content)
          VALUES (new.record_id, new.content);
        END;
        CREATE TRIGGER memory_canonical_records_ad
          AFTER DELETE ON memory_canonical_records
        BEGIN
          INSERT INTO memory_canonical_records_fts(memory_canonical_records_fts, rowid, content)
          VALUES ('delete', old.record_id, old.content);
        END;
        CREATE TRIGGER memory_canonical_records_au
          AFTER UPDATE ON memory_canonical_records
        BEGIN
          INSERT INTO memory_canonical_records_fts(memory_canonical_records_fts, rowid, content)
          VALUES ('delete', old.record_id, old.content);
          INSERT INTO memory_canonical_records_fts(rowid, content)
          VALUES (new.record_id, new.content);
        END;
        PRAGMA user_version = 8;
        ",
    )
    .expect("degrade canonical FTS schema");
    drop(conn);

    let payload = json!({
        "type": crate::memory::CANONICAL_MEMORY_RECORD_TYPE,
        "_loong_internal": true,
        "scope": "workspace",
        "kind": "imported_profile",
        "content": "release checklist",
        "metadata": {
            "source": "workspace-import"
        },
    })
    .to_string();
    append_turn_direct("workspace-session", "assistant", &payload, &config)
        .expect("append structured canonical payload");

    ensure_memory_db_ready(None, &config).expect("repair stale canonical FTS schema");

    let hits = search_canonical_records_for_recall("workspace-import", 4, None, &config)
        .expect("search canonical memory after repair");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].record.metadata["source"], "workspace-import");

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn cached_runtime_repair_path_recovers_stale_canonical_fts_metadata_schema() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-cached-runtime-stale-fts-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("cached-runtime-stale-fts.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config(db_path.clone());
    ensure_memory_db_ready(None, &config).expect("initialize sqlite db");

    let runtime = acquire_memory_runtime(&config).expect("cached sqlite runtime");
    runtime
        .with_connection_mut("test.degrade_cached_runtime_canonical_fts", |conn| {
            conn.execute_batch(
                "
                DROP TRIGGER IF EXISTS memory_canonical_records_ai;
                DROP TRIGGER IF EXISTS memory_canonical_records_ad;
                DROP TRIGGER IF EXISTS memory_canonical_records_au;
                DROP TABLE IF EXISTS memory_canonical_records_fts;
                CREATE VIRTUAL TABLE memory_canonical_records_fts
                  USING fts5(content, content='memory_canonical_records', content_rowid='record_id');
                CREATE TRIGGER memory_canonical_records_ai
                  AFTER INSERT ON memory_canonical_records
                BEGIN
                  INSERT INTO memory_canonical_records_fts(rowid, content)
                  VALUES (new.record_id, new.content);
                END;
                CREATE TRIGGER memory_canonical_records_ad
                  AFTER DELETE ON memory_canonical_records
                BEGIN
                  INSERT INTO memory_canonical_records_fts(memory_canonical_records_fts, rowid, content)
                  VALUES ('delete', old.record_id, old.content);
                END;
                CREATE TRIGGER memory_canonical_records_au
                  AFTER UPDATE ON memory_canonical_records
                BEGIN
                  INSERT INTO memory_canonical_records_fts(memory_canonical_records_fts, rowid, content)
                  VALUES ('delete', old.record_id, old.content);
                  INSERT INTO memory_canonical_records_fts(rowid, content)
                  VALUES (new.record_id, new.content);
                END;
                PRAGMA user_version = 8;
                ",
            )
            .map_err(|error| format!("degrade cached canonical FTS schema failed: {error}"))?;
            Ok(())
        })
        .expect("degrade cached canonical FTS schema");

    let payload = json!({
        "type": crate::memory::CANONICAL_MEMORY_RECORD_TYPE,
        "_loong_internal": true,
        "scope": "workspace",
        "kind": "imported_profile",
        "content": "release checklist",
        "metadata": {
            "source": "workspace-import"
        },
    })
    .to_string();
    append_turn_direct("workspace-session", "assistant", &payload, &config)
        .expect("append structured canonical payload");

    reset_sqlite_schema_repair_metrics_for_tests();
    ensure_memory_db_ready_with_diagnostics(None, &config)
        .expect("repair cached stale canonical FTS schema");

    let canonical_record_repair_count =
        sqlite_schema_repair_count_for_tests("canonical_records");
    assert!(
        canonical_record_repair_count >= 1,
        "expected cached runtime repair path to trigger canonical record repair, got: {canonical_record_repair_count}"
    );

    let hits = search_canonical_records_for_recall("release checklist", 5, None, &config)
        .expect("search canonical memory after cached runtime repair");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].record.scope, MemoryScope::Workspace);
    assert_eq!(hits[0].record.kind, CanonicalMemoryKind::ImportedProfile);

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn cached_runtime_repair_path_restores_control_plane_pairing_tables() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-cached-runtime-pairing-schema-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("cached-runtime-pairing-schema.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config(db_path.clone());
    ensure_memory_db_ready(None, &config).expect("initialize sqlite db");

    let runtime = acquire_memory_runtime(&config).expect("cached sqlite runtime");
    runtime
        .with_connection_mut("test.drop_cached_pairing_tables", |conn| {
            conn.execute_batch(
                "
                DROP INDEX IF EXISTS idx_control_plane_pairing_requests_status_requested_at;
                DROP INDEX IF EXISTS idx_control_plane_device_tokens_device_id;
                DROP TABLE IF EXISTS control_plane_pairing_requests;
                DROP TABLE IF EXISTS control_plane_device_tokens;
                ",
            )
            .map_err(|error| format!("drop cached pairing tables failed: {error}"))?;
            Ok(())
        })
        .expect("drop cached pairing tables");

    ensure_memory_db_ready_with_diagnostics(None, &config)
        .expect("repair cached control-plane pairing schema");

    let pairing_requests_table_exists = runtime
        .with_connection("test.verify_cached_pairing_requests_table", |conn| {
            conn.query_row(
                "SELECT COUNT(*)
                 FROM sqlite_master
                 WHERE type = 'table' AND name = 'control_plane_pairing_requests'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count == 1)
            .map_err(|error| format!("query cached pairing requests table failed: {error}"))
        })
        .expect("query cached pairing requests table");
    let device_tokens_table_exists = runtime
        .with_connection("test.verify_cached_device_tokens_table", |conn| {
            conn.query_row(
                "SELECT COUNT(*)
                 FROM sqlite_master
                 WHERE type = 'table' AND name = 'control_plane_device_tokens'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count == 1)
            .map_err(|error| format!("query cached device tokens table failed: {error}"))
        })
        .expect("query cached device tokens table");

    assert!(pairing_requests_table_exists);
    assert!(device_tokens_table_exists);

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn ensure_memory_db_ready_preserves_newer_schema_versions_without_current_repairs() {
    let _guard = sqlite_runtime_test_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    reset_sqlite_runtime_test_state();

    let tmp = std::env::temp_dir().join(format!(
        "loong-future-sqlite-schema-version-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("future-schema-version.sqlite3");
    let _ = fs::remove_file(&db_path);

    let config = sqlite_test_config(db_path.clone());
    ensure_memory_db_ready(None, &config).expect("initialize sqlite db");

    let runtime = acquire_memory_runtime(&config).expect("cached sqlite runtime");
    let future_schema_version = SQLITE_MEMORY_SCHEMA_VERSION + 1;
    runtime
        .with_connection_mut("test.bump_sqlite_user_version", |conn| {
            write_sqlite_user_version(conn, future_schema_version)
        })
        .expect("bump sqlite user_version");

    reset_sqlite_schema_repair_metrics_for_tests();
    ensure_memory_db_ready_with_diagnostics(None, &config)
        .expect("recheck cached newer sqlite schema");

    let cached_user_version = runtime
        .with_connection("test.read_cached_future_user_version", |conn| {
            read_sqlite_user_version(conn)
        })
        .expect("read cached future sqlite user_version");
    assert_eq!(cached_user_version, future_schema_version);
    drop(runtime);

    reset_sqlite_runtime_test_state();
    ensure_memory_db_ready_with_diagnostics(Some(db_path.clone()), &config)
        .expect("reopen newer sqlite schema");

    let reopened_runtime =
        acquire_memory_runtime(&config).expect("reopen cached sqlite runtime");
    let reopened_user_version = reopened_runtime
        .with_connection("test.read_future_user_version", |conn| {
            read_sqlite_user_version(conn)
        })
        .expect("read future sqlite user_version");
    let schema_init_count = sqlite_schema_init_count_for_tests(&db_path);

    assert_eq!(reopened_user_version, future_schema_version);
    assert_eq!(schema_init_count, 0);

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn ensure_memory_db_ready_backfills_canonical_records_for_legacy_turns() {
    let tmp = std::env::temp_dir().join(format!(
        "loong-canonical-memory-migration-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp dir");
    let db_path = tmp.join("legacy-canonical.sqlite3");
    let _ = fs::remove_file(&db_path);

    let conn = Connection::open(&db_path).expect("open legacy sqlite db");
    conn.execute_batch(
        "
        PRAGMA user_version = 4;
        CREATE TABLE turns(
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          session_id TEXT NOT NULL,
          session_turn_index INTEGER,
          role TEXT NOT NULL,
          content TEXT NOT NULL,
          ts INTEGER NOT NULL
        );
        CREATE INDEX idx_turns_session_id ON turns(session_id, id);
        CREATE UNIQUE INDEX idx_turns_session_turn_index
          ON turns(session_id, session_turn_index);
        CREATE TABLE memory_session_state(
          session_id TEXT PRIMARY KEY,
          turn_count INTEGER NOT NULL
        );
        CREATE TABLE memory_summary_checkpoints(
          session_id TEXT PRIMARY KEY,
          summarized_through_turn_id INTEGER NOT NULL,
          summary_before_turn_id INTEGER,
          summary_body_bytes INTEGER NOT NULL DEFAULT 0,
          summary_budget_chars INTEGER NOT NULL,
          summary_window_size INTEGER NOT NULL,
          summary_format_version INTEGER NOT NULL,
          updated_at_ts INTEGER NOT NULL
        );
        CREATE TABLE memory_summary_checkpoint_bodies(
          session_id TEXT PRIMARY KEY
            REFERENCES memory_summary_checkpoints(session_id) ON DELETE CASCADE,
          summary_body TEXT NOT NULL
        );
        CREATE TABLE approval_requests(
          approval_request_id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          turn_id TEXT NOT NULL,
          tool_call_id TEXT NOT NULL,
          tool_name TEXT NOT NULL,
          approval_key TEXT NOT NULL,
          status TEXT NOT NULL,
          decision TEXT NULL,
          request_payload_json TEXT NOT NULL,
          governance_snapshot_json TEXT NOT NULL,
          requested_at INTEGER NOT NULL,
          resolved_at INTEGER NULL,
          resolved_by_session_id TEXT NULL,
          executed_at INTEGER NULL,
          last_error TEXT NULL
        );
        CREATE TABLE approval_grants(
          scope_session_id TEXT NOT NULL,
          approval_key TEXT NOT NULL,
          created_by_session_id TEXT NULL,
          created_at INTEGER NOT NULL,
          updated_at INTEGER NOT NULL,
          PRIMARY KEY(scope_session_id, approval_key)
        );
        CREATE INDEX idx_approval_requests_session_status_requested_at
          ON approval_requests(session_id, status, requested_at DESC, approval_request_id);
        ",
    )
    .expect("create legacy schema");
    conn.execute(
        "INSERT INTO turns(session_id, session_turn_index, role, content, ts)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            "legacy-session",
            1_i64,
            "assistant",
            "Legacy rollout fix includes rollback and smoke test verification.",
            1_717_000_000_i64
        ],
    )
    .expect("insert legacy turn");
    conn.execute(
        "INSERT INTO memory_session_state(session_id, turn_count)
         VALUES (?1, ?2)",
        rusqlite::params!["legacy-session", 1_i64],
    )
    .expect("insert session state");
    drop(conn);

    let config = sqlite_test_config(db_path.clone());
    let _ = ensure_memory_db_ready(None, &config).expect("upgrade legacy sqlite db");

    let hits = search_canonical_records_for_recall("rollback smoke test", 4, None, &config)
        .expect("search canonical memory after migration");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].record.session_id, "legacy-session");

    let _ = fs::remove_file(&db_path);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn default_window_size_prefers_injected_config() {
    let config = MemoryRuntimeConfig {
        sqlite_path: None,
        sliding_window: 24,
        ..MemoryRuntimeConfig::default()
    };

    assert_eq!(default_window_size(&config), 24);
}

#[test]
fn default_window_size_falls_back_to_default_without_config() {
    assert_eq!(default_window_size(&MemoryRuntimeConfig::default()), 12);
}
