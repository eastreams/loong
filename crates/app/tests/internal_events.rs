use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use loong_app::internal_events::{
    InternalEventJournalCursor, append_internal_event_to_journal,
    emit_internal_event_with_metadata, inspect_internal_event_journal_layout,
    internal_event_active_segment_id_path, internal_event_journal_cursor_from_line_cursor,
    internal_event_journal_path, internal_event_journal_state_path, internal_event_segment_path,
    probe_internal_event_journal_runtime_ready, prune_internal_event_journal_segments,
    read_internal_event_journal_after, read_internal_event_journal_since,
    repair_internal_event_journal_state, rotate_internal_event_journal_segment,
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

#[test]
fn read_internal_event_journal_after_continues_across_segment_boundary_without_replay() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    let active_segment_path = internal_event_segment_path("segment-000001");
    fs::create_dir_all(active_segment_path.parent().expect("segment parent"))
        .expect("create segment parent");
    fs::write(internal_event_active_segment_id_path(), "segment-000002\n")
        .expect("write active segment id");
    fs::write(
        &active_segment_path,
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"seg-a\"},\"recorded_at_ms\":1}\n",
    )
    .expect("write sealed segment");
    fs::write(
        internal_event_segment_path("segment-000002"),
        "{\"event_name\":\"session.archived\",\"payload\":{\"session_id\":\"seg-b\"},\"recorded_at_ms\":2}\n",
    )
    .expect("write active segment");

    let (events, next_cursor) =
        read_internal_event_journal_after(InternalEventJournalCursor::default())
            .expect("read across segment boundary");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].payload["session_id"], "seg-a");
    assert_eq!(events[1].payload["session_id"], "seg-b");
    assert_eq!(next_cursor.segment_id.as_deref(), Some("segment-000002"));

    let (follow_up_events, follow_up_cursor) =
        read_internal_event_journal_after(next_cursor.clone()).expect("read after final cursor");
    assert!(follow_up_events.is_empty());
    assert_eq!(follow_up_cursor, next_cursor);
}

#[test]
fn internal_event_journal_cursor_from_line_cursor_maps_legacy_numeric_cursor_across_segment_boundary()
 {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    let first_segment_path = internal_event_segment_path("segment-000001");
    fs::create_dir_all(first_segment_path.parent().expect("segment parent"))
        .expect("create segment parent");
    fs::write(internal_event_active_segment_id_path(), "segment-000002\n")
        .expect("write active segment id");
    fs::write(
        &first_segment_path,
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"legacy-a\"},\"recorded_at_ms\":1}\n",
    )
    .expect("write first segment");
    fs::write(
        internal_event_segment_path("segment-000002"),
        "{\"event_name\":\"session.archived\",\"payload\":{\"session_id\":\"legacy-b\"},\"recorded_at_ms\":2}\n",
    )
    .expect("write second segment");

    let cursor = internal_event_journal_cursor_from_line_cursor(1).expect("migrate numeric cursor");
    assert_eq!(cursor.segment_id.as_deref(), Some("segment-000001"));
    assert_eq!(cursor.line_cursor, 1);

    let (events, next_cursor) =
        read_internal_event_journal_after(cursor).expect("read after migrated cursor");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_name, "session.archived");
    assert_eq!(events[0].payload["session_id"], "legacy-b");
    assert_eq!(next_cursor.segment_id.as_deref(), Some("segment-000002"));
}

#[test]
fn rotate_internal_event_journal_segment_moves_legacy_journal_into_first_segment() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    let legacy_path = temp_home
        .path()
        .join("automation")
        .join("internal-events.jsonl");
    fs::create_dir_all(legacy_path.parent().expect("legacy parent")).expect("create legacy parent");
    fs::write(
        &legacy_path,
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"legacy\"},\"recorded_at_ms\":1}\n",
    )
    .expect("seed legacy journal row");

    let next_segment_id = rotate_internal_event_journal_segment().expect("rotate legacy segment");
    assert_eq!(next_segment_id, "segment-000002");
    assert_eq!(
        fs::read_to_string(internal_event_active_segment_id_path()).expect("read active segment"),
        "segment-000002\n"
    );
    let sealed_segment_path = internal_event_segment_path("segment-000001");
    assert!(sealed_segment_path.exists());
    let sealed_contents =
        fs::read_to_string(&sealed_segment_path).expect("read sealed legacy segment");
    assert!(
        sealed_contents.contains("\"session_id\":\"legacy\""),
        "sealed legacy segment should preserve the migrated row: {sealed_contents}"
    );

    append_internal_event_to_journal("session.archived", &json!({ "session_id": "active" }))
        .expect("append active segment row");
    let active_segment_path = internal_event_segment_path("segment-000002");
    assert!(active_segment_path.exists());
}

#[test]
fn rotate_internal_event_journal_segment_advances_active_segment_for_new_appends() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    fs::create_dir_all(
        internal_event_active_segment_id_path()
            .parent()
            .expect("active segment parent"),
    )
    .expect("create active segment parent");
    fs::write(internal_event_active_segment_id_path(), "segment-000001\n")
        .expect("seed active segment id");
    append_internal_event_to_journal("session.cancelled", &json!({ "session_id": "first" }))
        .expect("append first active row");

    let next_segment_id = rotate_internal_event_journal_segment().expect("rotate active segment");
    assert_eq!(next_segment_id, "segment-000002");

    append_internal_event_to_journal("session.archived", &json!({ "session_id": "second" }))
        .expect("append second active row");
    assert!(internal_event_segment_path("segment-000001").exists());
    assert!(internal_event_segment_path("segment-000002").exists());
}

#[test]
fn prune_internal_event_journal_segments_removes_only_fully_consumed_sealed_segments() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    fs::create_dir_all(
        internal_event_segment_path("segment-000001")
            .parent()
            .expect("segment parent"),
    )
    .expect("create segment parent");
    fs::write(internal_event_active_segment_id_path(), "segment-000003\n")
        .expect("write active segment");
    fs::write(
        internal_event_segment_path("segment-000001"),
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"oldest\"},\"recorded_at_ms\":1}\n",
    )
    .expect("write first sealed segment");
    fs::write(
        internal_event_segment_path("segment-000002"),
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"cursor\"},\"recorded_at_ms\":2}\n",
    )
    .expect("write cursor segment");
    fs::write(
        internal_event_segment_path("segment-000003"),
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"active\"},\"recorded_at_ms\":3}\n",
    )
    .expect("write active segment");

    let pruned = prune_internal_event_journal_segments(&InternalEventJournalCursor {
        segment_id: Some("segment-000002".to_owned()),
        line_cursor: 1,
        byte_offset: 1,
        journal_fingerprint: None,
    })
    .expect("prune consumed segments");
    assert_eq!(pruned, vec!["segment-000001".to_owned()]);
    assert!(!internal_event_segment_path("segment-000001").exists());
    assert!(internal_event_segment_path("segment-000002").exists());
    assert!(internal_event_segment_path("segment-000003").exists());
}

#[test]
fn read_internal_event_journal_after_missing_segment_cursor_skips_older_surviving_segments() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    fs::create_dir_all(
        internal_event_segment_path("segment-000001")
            .parent()
            .expect("segment parent"),
    )
    .expect("create segment parent");
    fs::write(internal_event_active_segment_id_path(), "segment-000003\n")
        .expect("write active segment id");
    fs::write(
        internal_event_segment_path("segment-000001"),
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"older\"},\"recorded_at_ms\":1}\n",
    )
    .expect("write older surviving segment");
    fs::write(
        internal_event_segment_path("segment-000003"),
        "{\"event_name\":\"session.archived\",\"payload\":{\"session_id\":\"newer\"},\"recorded_at_ms\":3}\n",
    )
    .expect("write newer surviving segment");

    let (events, next_cursor) = read_internal_event_journal_after(InternalEventJournalCursor {
        segment_id: Some("segment-000002".to_owned()),
        line_cursor: 5,
        byte_offset: 99,
        journal_fingerprint: Some("stale".to_owned()),
    })
    .expect("read after missing segment cursor");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_name, "session.archived");
    assert_eq!(events[0].payload["session_id"], "newer");
    assert_eq!(next_cursor.segment_id.as_deref(), Some("segment-000003"));
}

#[test]
fn inspect_internal_event_journal_layout_reports_active_and_legacy_segments() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    let legacy_path = temp_home
        .path()
        .join("automation")
        .join("internal-events.jsonl");
    fs::create_dir_all(legacy_path.parent().expect("legacy parent")).expect("create legacy parent");
    fs::write(
        &legacy_path,
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"legacy\"},\"recorded_at_ms\":1}\n",
    )
    .expect("write legacy journal");
    fs::write(internal_event_active_segment_id_path(), "segment-000002\n")
        .expect("write active segment id");
    fs::create_dir_all(
        internal_event_segment_path("segment-000001")
            .parent()
            .expect("segment parent"),
    )
    .expect("create segment parent");
    fs::write(
        internal_event_segment_path("segment-000001"),
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"sealed\"},\"recorded_at_ms\":2}\n",
    )
    .expect("write sealed segment");

    let layout = inspect_internal_event_journal_layout().expect("inspect journal layout");
    assert_eq!(layout.active_segment_id, "segment-000002");
    assert_eq!(layout.segments.len(), 3);
    assert_eq!(layout.segments[0].segment_id, "legacy");
    assert!(!layout.segments[0].is_active);
    assert_eq!(layout.segments[1].segment_id, "segment-000001");
    assert!(!layout.segments[1].is_active);
    assert_eq!(layout.segments[2].segment_id, "segment-000002");
    assert!(layout.segments[2].is_active);
}

#[test]
fn rotate_internal_event_journal_segment_persists_layout_state_and_active_marker() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    let next_segment_id = rotate_internal_event_journal_segment().expect("rotate segment");
    assert_eq!(next_segment_id, "segment-000002");

    let state_payload: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(internal_event_journal_state_path()).expect("read journal state"),
    )
    .expect("parse journal state");
    assert_eq!(state_payload["schema_version"], 1);
    assert_eq!(state_payload["active_segment_id"], "segment-000002");
    assert_eq!(
        state_payload["segments"]
            .as_array()
            .expect("segments array")
            .len(),
        2
    );
    assert_eq!(state_payload["segments"][0]["segment_id"], "segment-000001");
    assert_eq!(state_payload["segments"][0]["status"], "sealed");
    assert_eq!(state_payload["segments"][1]["segment_id"], "segment-000002");
    assert_eq!(state_payload["segments"][1]["status"], "active");
    assert_eq!(
        fs::read_to_string(internal_event_active_segment_id_path()).expect("read active marker"),
        "segment-000002\n"
    );
}

#[test]
fn append_internal_event_to_journal_auto_rotates_when_segment_exceeds_threshold() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());
    env.set("LOONG_INTERNAL_EVENT_SEGMENT_MAX_BYTES", "1");

    append_internal_event_to_journal("session.cancelled", &json!({ "session_id": "first" }))
        .expect("append first row");
    append_internal_event_to_journal("session.archived", &json!({ "session_id": "second" }))
        .expect("append second row");

    let first_segment = internal_event_segment_path("segment-000001");
    let second_segment = internal_event_segment_path("segment-000002");
    let first_contents = fs::read_to_string(&first_segment).expect("read first segment");
    let second_contents = fs::read_to_string(&second_segment).expect("read second segment");
    assert!(first_contents.contains("\"session_id\":\"first\""));
    assert!(second_contents.contains("\"session_id\":\"second\""));

    let layout = inspect_internal_event_journal_layout().expect("inspect journal layout");
    assert_eq!(layout.active_segment_id, "segment-000002");
}

#[test]
fn inspect_internal_event_journal_layout_prefers_state_file_over_stale_active_marker() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    fs::create_dir_all(
        internal_event_active_segment_id_path()
            .parent()
            .expect("automation parent"),
    )
    .expect("create automation parent");
    fs::write(internal_event_active_segment_id_path(), "segment-000001\n")
        .expect("write stale active marker");
    fs::write(
        internal_event_journal_state_path(),
        "{\n  \"active_segment_id\": \"segment-000003\"\n}\n",
    )
    .expect("write journal state");
    fs::create_dir_all(
        internal_event_segment_path("segment-000001")
            .parent()
            .expect("segment parent"),
    )
    .expect("create segment parent");
    fs::write(
        internal_event_segment_path("segment-000001"),
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"sealed\"},\"recorded_at_ms\":1}\n",
    )
    .expect("write sealed segment");

    let layout = inspect_internal_event_journal_layout().expect("inspect journal layout");
    assert_eq!(layout.active_segment_id, "segment-000003");
    assert_eq!(
        layout
            .segments
            .last()
            .expect("active segment entry")
            .segment_id,
        "segment-000003"
    );
    assert!(
        layout
            .segments
            .last()
            .expect("active segment entry")
            .is_active
    );
}

#[test]
fn repair_internal_event_journal_state_reconciles_disk_layout_and_preserves_known_metadata() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    fs::create_dir_all(
        internal_event_segment_path("segment-000001")
            .parent()
            .expect("segment parent"),
    )
    .expect("create segment parent");
    fs::write(
        internal_event_journal_state_path(),
        concat!(
            "{\n",
            "  \"schema_version\": 1,\n",
            "  \"active_segment_id\": \"segment-000003\",\n",
            "  \"segments\": [\n",
            "    {\"segment_id\":\"segment-000001\",\"status\":\"sealed\",\"created_at_ms\":10,\"sealed_at_ms\":20},\n",
            "    {\"segment_id\":\"segment-000003\",\"status\":\"active\",\"created_at_ms\":30}\n",
            "  ]\n",
            "}\n"
        ),
    )
    .expect("write journal state");
    fs::write(
        internal_event_segment_path("segment-000001"),
        "{\"event_name\":\"session.cancelled\",\"payload\":{\"session_id\":\"sealed\"},\"recorded_at_ms\":1}\n",
    )
    .expect("write sealed segment");
    fs::write(
        internal_event_segment_path("segment-000004"),
        "{\"event_name\":\"session.archived\",\"payload\":{\"session_id\":\"new-active\"},\"recorded_at_ms\":2}\n",
    )
    .expect("write recovered active segment");
    fs::write(internal_event_active_segment_id_path(), "segment-000004\n")
        .expect("write newer active marker");

    let repaired_layout =
        repair_internal_event_journal_state().expect("repair internal event journal state");
    assert_eq!(repaired_layout.active_segment_id, "segment-000004");
    assert_eq!(repaired_layout.segments.len(), 2);
    assert_eq!(repaired_layout.segments[0].segment_id, "segment-000001");
    assert_eq!(repaired_layout.segments[0].status, "sealed");
    assert_eq!(repaired_layout.segments[0].created_at_ms, Some(10));
    assert_eq!(repaired_layout.segments[0].sealed_at_ms, Some(20));
    assert_eq!(repaired_layout.segments[1].segment_id, "segment-000004");
    assert_eq!(repaired_layout.segments[1].status, "active");
}

#[test]
fn inspect_internal_event_journal_layout_reads_manifest_statuses() {
    let temp_home = tempfile::tempdir().expect("create temp home");
    let mut env = ScopedEnv::new();
    env.set("LOONG_HOME", temp_home.path().as_os_str());

    fs::create_dir_all(
        internal_event_journal_state_path()
            .parent()
            .expect("automation parent"),
    )
    .expect("create automation parent");
    fs::write(
        internal_event_journal_state_path(),
        concat!(
            "{\n",
            "  \"schema_version\": 1,\n",
            "  \"active_segment_id\": \"segment-000003\",\n",
            "  \"segments\": [\n",
            "    {\"segment_id\":\"legacy\",\"status\":\"legacy\"},\n",
            "    {\"segment_id\":\"segment-000002\",\"status\":\"sealed\",\"created_at_ms\":10,\"sealed_at_ms\":20},\n",
            "    {\"segment_id\":\"segment-000003\",\"status\":\"active\",\"created_at_ms\":30}\n",
            "  ]\n",
            "}\n"
        ),
    )
    .expect("write manifest state");

    let layout = inspect_internal_event_journal_layout().expect("inspect journal layout");
    assert_eq!(layout.active_segment_id, "segment-000003");
    assert_eq!(layout.segments[0].status, "legacy");
    assert_eq!(layout.segments[1].status, "sealed");
    assert_eq!(layout.segments[1].created_at_ms, Some(10));
    assert_eq!(layout.segments[1].sealed_at_ms, Some(20));
    assert_eq!(layout.segments[2].status, "active");
    assert_eq!(layout.segments[2].created_at_ms, Some(30));
    assert_eq!(layout.segments[2].sealed_at_ms, None);
}
