use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use loong_app::internal_events::{
    InternalEventJournalCursor, append_internal_event_to_journal,
    emit_internal_event_with_metadata, internal_event_journal_cursor_from_line_cursor,
    internal_event_journal_path, probe_internal_event_journal_runtime_ready,
    read_internal_event_journal_after, read_internal_event_journal_since,
};
use loong_app::test_support::ScopedEnv;
use serde_json::json;

#[test]
fn read_internal_event_journal_since_filters_by_cursor() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    emit_internal_event_with_metadata(
        "session.cancelled",
        "app.tools.session",
        json!({
            "session_id": "s1"
        }),
    );
    emit_internal_event_with_metadata(
        "session.archived",
        "app.tools.session",
        json!({
            "session_id": "s2"
        }),
    );

    let events = read_internal_event_journal_since(1).expect("read journal after cursor");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].line_cursor, 2);
    assert_eq!(events[0].event_name, "session.archived");
    assert_eq!(events[0].payload["session_id"], "s2");
}

#[test]
fn read_internal_event_journal_preserves_scalar_payloads_via_value_wrapper() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    emit_internal_event_with_metadata("session.cancelled", "app.tools.session", json!("hello"));

    let events = read_internal_event_journal_since(0).expect("read journal");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["value"], "hello");
    assert_eq!(
        events[0].payload["_automation"]["source_surface"],
        "app.tools.session"
    );
}

#[test]
fn read_internal_event_journal_accepts_legacy_rows_and_blank_lines() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());
    let journal_path = internal_event_journal_path();
    if let Some(parent) = journal_path.parent() {
        fs::create_dir_all(parent).expect("create journal parent");
    }
    fs::write(
        &journal_path,
        concat!(
            "{\"event_name\":\"legacy.one\"}\n",
            "\n",
            "{\"event_name\":\"legacy.two\",\"payload\":{\"ok\":true}}\n"
        ),
    )
    .expect("write legacy journal");

    let events = read_internal_event_journal_since(0).expect("read legacy journal");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].line_cursor, 1);
    assert_eq!(events[0].event_name, "legacy.one");
    assert_eq!(events[0].payload, serde_json::Value::Null);
    assert_eq!(events[0].recorded_at_ms, 0);
    assert_eq!(events[1].line_cursor, 3);
    assert_eq!(events[1].event_name, "legacy.two");
    assert_eq!(events[1].payload["ok"], true);
}

#[test]
fn append_internal_event_to_journal_writes_rows_in_readable_order() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    append_internal_event_to_journal(
        "session.cancelled",
        &json!({
            "session_id": "s1"
        }),
    )
    .expect("append first journal row");
    append_internal_event_to_journal(
        "session.archived",
        &json!({
            "session_id": "s2"
        }),
    )
    .expect("append second journal row");

    let events = read_internal_event_journal_since(0).expect("read appended journal");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].line_cursor, 1);
    assert_eq!(events[0].event_name, "session.cancelled");
    assert_eq!(events[0].payload["session_id"], "s1");
    assert!(events[0].recorded_at_ms > 0);
    assert_eq!(events[1].line_cursor, 2);
    assert_eq!(events[1].event_name, "session.archived");
    assert_eq!(events[1].payload["session_id"], "s2");
    assert!(events[1].recorded_at_ms > 0);
}

#[test]
fn read_internal_event_journal_after_uses_byte_offset_for_incremental_reads() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    append_internal_event_to_journal("session.cancelled", &json!({ "session_id": "s1" }))
        .expect("append first row");
    let (first_events, first_cursor) =
        read_internal_event_journal_after(InternalEventJournalCursor::default())
            .expect("read first batch");
    assert_eq!(first_events.len(), 1);
    assert_eq!(first_events[0].payload["session_id"], "s1");
    assert_eq!(first_cursor.line_cursor, 1);
    assert!(first_cursor.byte_offset > 0);

    append_internal_event_to_journal("session.archived", &json!({ "session_id": "s2" }))
        .expect("append second row");
    let (second_events, second_cursor) =
        read_internal_event_journal_after(first_cursor.clone()).expect("read second batch");
    assert_eq!(second_events.len(), 1);
    assert_eq!(second_events[0].payload["session_id"], "s2");
    assert_eq!(second_cursor.line_cursor, 2);
    assert!(second_cursor.byte_offset > first_cursor.byte_offset);
}

#[test]
fn internal_event_journal_cursor_from_line_cursor_preserves_legacy_numeric_position() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    append_internal_event_to_journal("session.cancelled", &json!({ "session_id": "s1" }))
        .expect("append first row");
    append_internal_event_to_journal("session.archived", &json!({ "session_id": "s2" }))
        .expect("append second row");

    let cursor =
        internal_event_journal_cursor_from_line_cursor(1).expect("migrate line cursor to offset");
    assert_eq!(cursor.line_cursor, 1);
    assert!(cursor.byte_offset > 0);

    let (events, next_cursor) =
        read_internal_event_journal_after(cursor.clone()).expect("read after migrated cursor");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_name, "session.archived");
    assert_eq!(events[0].payload["session_id"], "s2");
    assert_eq!(next_cursor.line_cursor, 2);
    assert!(next_cursor.byte_offset > cursor.byte_offset);
}

#[test]
fn read_internal_event_journal_after_resets_stale_offset_after_truncation() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    append_internal_event_to_journal("session.cancelled", &json!({ "session_id": "s1" }))
        .expect("append first row");
    append_internal_event_to_journal("session.archived", &json!({ "session_id": "s2" }))
        .expect("append second row");

    let (_, stale_cursor) =
        read_internal_event_journal_after(InternalEventJournalCursor::default())
            .expect("read initial journal");
    let journal_path = internal_event_journal_path();
    fs::write(
        &journal_path,
        "{\"event_name\":\"session.recovered\",\"payload\":{\"session_id\":\"s3\"},\"recorded_at_ms\":1}\n",
    )
    .expect("truncate journal to retained subset");

    let (events, recovered_cursor) =
        read_internal_event_journal_after(stale_cursor).expect("read after stale cursor");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_name, "session.recovered");
    assert_eq!(events[0].line_cursor, 1);
    assert_eq!(events[0].payload["session_id"], "s3");
    assert_eq!(recovered_cursor.line_cursor, 1);
    assert!(recovered_cursor.byte_offset > 0);
}

#[test]
fn read_internal_event_journal_after_resets_stale_offset_after_same_size_rotation() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    append_internal_event_to_journal(
        "session.cancelled",
        &json!({ "session_id": "first-rotation-source" }),
    )
    .expect("append first source row");
    append_internal_event_to_journal(
        "session.archived",
        &json!({ "session_id": "second-rotation-source" }),
    )
    .expect("append second source row");

    let (_, stale_cursor) =
        read_internal_event_journal_after(InternalEventJournalCursor::default())
            .expect("read initial journal");
    let stale_fingerprint = stale_cursor.journal_fingerprint.clone();
    let journal_path = internal_event_journal_path();
    fs::write(
        &journal_path,
        concat!(
            "{\"event_name\":\"session.recovered\",\"payload\":{\"session_id\":\"rotation-target-a\"},\"recorded_at_ms\":2}\n",
            "{\"event_name\":\"session.recovered\",\"payload\":{\"session_id\":\"rotation-target-b\"},\"recorded_at_ms\":3}\n"
        ),
    )
    .expect("replace journal with same-size-or-larger rotated file");

    let (events, recovered_cursor) =
        read_internal_event_journal_after(stale_cursor).expect("read after rotated journal");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].line_cursor, 1);
    assert_eq!(events[0].event_name, "session.recovered");
    assert_eq!(events[0].payload["session_id"], "rotation-target-a");
    assert_eq!(events[1].line_cursor, 2);
    assert_eq!(events[1].payload["session_id"], "rotation-target-b");
    assert_eq!(recovered_cursor.line_cursor, 2);
    assert!(recovered_cursor.byte_offset > 0);
    assert_ne!(
        recovered_cursor.journal_fingerprint, stale_fingerprint,
        "rotation should produce a different journal fingerprint"
    );
}

#[test]
fn probe_internal_event_journal_runtime_ready_accepts_fresh_path() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    probe_internal_event_journal_runtime_ready().expect("probe runtime readiness");
    assert!(
        internal_event_journal_path().exists(),
        "probe should create the journal path when it does not exist yet"
    );
}

#[test]
fn append_internal_event_to_journal_waits_for_existing_file_lock() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    probe_internal_event_journal_runtime_ready().expect("probe runtime readiness");
    let journal_path = internal_event_journal_path();
    let external_lock = fs::OpenOptions::new()
        .read(true)
        .append(true)
        .open(&journal_path)
        .expect("open external journal handle");
    external_lock.lock().expect("hold external journal lock");

    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let result = append_internal_event_to_journal(
            "session.cancelled",
            &json!({
                "session_id": "locked"
            }),
        );
        tx.send(result).expect("send append result");
    });

    match rx.recv_timeout(Duration::from_millis(100)) {
        Err(mpsc::RecvTimeoutError::Timeout) => {}
        Ok(result) => panic!("journal append should block on external file lock, got {result:?}"),
        Err(error) => panic!("journal append channel closed unexpectedly: {error:?}"),
    }

    external_lock
        .unlock()
        .expect("release external journal lock");
    rx.recv_timeout(Duration::from_secs(1))
        .expect("append should complete after lock release")
        .expect("append should succeed after lock release");
    handle.join().expect("join journal writer thread");

    let contents = fs::read_to_string(&journal_path).expect("read journal contents");
    assert_eq!(contents.lines().count(), 1);
}
