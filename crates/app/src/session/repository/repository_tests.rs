use std::collections::BTreeSet;
use std::fs;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::json;

use crate::session::store::{SessionStoreConfig, append_session_turn_direct};
use crate::tools::runtime_config::ToolRuntimeNarrowing;

use super::*;

fn isolated_memory_config(test_name: &str) -> SessionStoreConfig {
    let base = std::env::temp_dir().join(format!(
        "loong-session-repository-{test_name}-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&base);
    let db_path = base.join("memory.sqlite3");
    let _ = fs::remove_file(&db_path);
    SessionStoreConfig {
        sqlite_path: Some(db_path),
        runtime_config: None,
    }
}

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

fn append_session_turn(config: &SessionStoreConfig, session_id: &str, role: &str, content: &str) {
    append_session_turn_direct(session_id, role, content, config).expect("append session turn");
}

fn set_session_updated_at(repo: &SessionRepository, session_id: &str, updated_at: i64) {
    let conn = repo.open_connection().expect("open connection");
    conn.execute(
        "UPDATE sessions
             SET updated_at = ?2
             WHERE session_id = ?1",
        params![session_id, updated_at],
    )
    .expect("set session updated_at");
}

fn set_turn_timestamps(repo: &SessionRepository, session_id: &str, ts: i64) {
    let conn = repo.open_connection().expect("open connection");
    conn.execute(
        "UPDATE turns
             SET ts = ?2
             WHERE session_id = ?1",
        params![session_id, ts],
    )
    .expect("set turn timestamps");
}

fn archive_session(repo: &SessionRepository, session_id: &str, archived_at: i64) {
    let conn = repo.open_connection().expect("open connection");
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
    .expect("insert archive event");
}

#[test]
fn session_repository_creates_and_loads_session_rows() {
    let config = isolated_memory_config("create-load");
    let repo = SessionRepository::new(&config).expect("repository");
    let created = repo
        .create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create session");

    assert_eq!(created.session_id, "root-session");
    assert_eq!(created.kind, SessionKind::Root);
    assert_eq!(created.state, SessionState::Ready);

    let loaded = repo
        .load_session("root-session")
        .expect("load session")
        .expect("session row");
    assert_eq!(loaded.session_id, "root-session");
    assert_eq!(loaded.label.as_deref(), Some("Root"));
    assert_eq!(loaded.parent_session_id, None);
}

#[test]
fn create_session_seeds_root_node_and_active_head() {
    let config = isolated_memory_config("seed-root-node");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create session");

    let root_node = repo
        .load_session_node("session-root:root-session")
        .expect("load root node")
        .expect("root node");
    let active_head = repo
        .load_session_head("root-session", ACTIVE_SESSION_HEAD_NAME)
        .expect("load active head")
        .expect("active head");

    assert_eq!(root_node.kind, SessionNodeKind::Root);
    assert_eq!(root_node.parent_node_id, None);
    assert_eq!(active_head.node_id, root_node.node_id);
    assert_eq!(active_head.mode, SessionHeadMode::Live);
}

#[test]
fn append_turn_dual_writes_linear_active_session_path() {
    let config = isolated_memory_config("append-turn-tree");
    let repo = SessionRepository::new(&config).expect("repository");
    create_root_session(&repo, "root-session");
    append_session_turn(&config, "root-session", "user", "hello");
    append_session_turn(&config, "root-session", "assistant", "world");

    let path = repo
        .load_active_session_path("root-session")
        .expect("load active path");

    assert_eq!(path.len(), 3);
    assert_eq!(path[0].kind, SessionNodeKind::Root);
    assert_eq!(path[1].kind, SessionNodeKind::UserTurn);
    assert_eq!(path[1].content.as_deref(), Some("hello"));
    assert_eq!(path[2].kind, SessionNodeKind::AssistantTurn);
    assert_eq!(path[2].content.as_deref(), Some("world"));
}

#[test]
fn fork_session_head_preserves_active_head_and_creates_named_head() {
    let config = isolated_memory_config("fork-head");
    let repo = SessionRepository::new(&config).expect("repository");
    create_root_session(&repo, "root-session");
    append_session_turn(&config, "root-session", "user", "hello");
    append_session_turn(&config, "root-session", "assistant", "world");

    let active_path = repo
        .load_active_session_path("root-session")
        .expect("load active path");
    let source_node_id = active_path[1].node_id.clone();
    let active_head_before = repo
        .load_session_head("root-session", ACTIVE_SESSION_HEAD_NAME)
        .expect("load active head")
        .expect("active head");

    let fork_head = repo
        .fork_session_head("root-session", &source_node_id, "thread/alpha")
        .expect("fork session head");
    let active_head_after = repo
        .load_session_head("root-session", ACTIVE_SESSION_HEAD_NAME)
        .expect("load active head")
        .expect("active head");
    let fork_path = repo
        .load_session_path_for_head("root-session", "thread/alpha")
        .expect("load fork path");

    assert_eq!(fork_head.node_id, source_node_id);
    assert_eq!(fork_head.mode, SessionHeadMode::Live);
    assert_eq!(active_head_before.node_id, active_head_after.node_id);
    assert_eq!(fork_path.len(), 2);
    assert_eq!(fork_path[1].content.as_deref(), Some("hello"));
}

#[test]
fn set_session_head_mode_pins_checkpoint_like_heads_without_touching_active() {
    let config = isolated_memory_config("session-head-mode");
    let repo = SessionRepository::new(&config).expect("repository");
    create_root_session(&repo, "root-session");
    append_session_turn(&config, "root-session", "user", "hello");

    repo.fork_session_head(
        "root-session",
        "session-turn:root-session:1",
        "thread/alpha",
    )
    .expect("fork session head");

    let pinned_head = repo
        .set_session_head_mode("root-session", "thread/alpha", SessionHeadMode::Pinned)
        .expect("pin session head");
    let unpinned_head = repo
        .set_session_head_mode("root-session", "thread/alpha", SessionHeadMode::Live)
        .expect("unpin session head");
    let active_error = repo
        .set_session_head_mode(
            "root-session",
            ACTIVE_SESSION_HEAD_NAME,
            SessionHeadMode::Pinned,
        )
        .expect_err("active head should refuse pinning");

    assert_eq!(pinned_head.mode, SessionHeadMode::Pinned);
    assert_eq!(unpinned_head.mode, SessionHeadMode::Live);
    assert!(
        active_error.contains("cannot be pinned"),
        "unexpected error: {active_error}"
    );
}

#[test]
fn replace_turns_rebuilds_linear_session_tree_path() {
    let config = isolated_memory_config("replace-turns-tree");
    let repo = SessionRepository::new(&config).expect("repository");
    create_root_session(&repo, "root-session");
    append_session_turn(&config, "root-session", "user", "hello");
    append_session_turn(&config, "root-session", "assistant", "world");

    store::replace_session_turns_direct(
        "root-session",
        &[
            store::SessionWindowTurn {
                role: "user".to_owned(),
                content: "replaced-user".to_owned(),
                ts: Some(100),
            },
            store::SessionWindowTurn {
                role: "assistant".to_owned(),
                content: "replaced-assistant".to_owned(),
                ts: Some(101),
            },
        ],
        &config,
    )
    .expect("replace session turns");

    let path = repo
        .load_active_session_path("root-session")
        .expect("load active path");

    assert_eq!(path.len(), 3);
    assert_eq!(path[1].content.as_deref(), Some("replaced-user"));
    assert_eq!(path[2].content.as_deref(), Some("replaced-assistant"));
}

#[test]
fn create_session_artifact_persists_and_lists_checkpoint_metadata() {
    let config = isolated_memory_config("session-artifact");
    let repo = SessionRepository::new(&config).expect("repository");
    create_root_session(&repo, "root-session");
    append_session_turn(&config, "root-session", "user", "hello");

    let active_path = repo
        .load_active_session_path("root-session")
        .expect("load active path");
    let anchor_node_id = active_path
        .last()
        .expect("active path node")
        .node_id
        .clone();

    let artifact = repo
        .create_session_artifact(NewSessionArtifactRecord {
            artifact_id: "artifact-1".to_owned(),
            session_id: "root-session".to_owned(),
            kind: SessionArtifactKind::Checkpoint,
            head_name: Some(ACTIVE_SESSION_HEAD_NAME.to_owned()),
            anchor_node_id: Some(anchor_node_id.clone()),
            source_start_node_id: Some(anchor_node_id.clone()),
            source_end_node_id: Some(anchor_node_id),
            payload_json: json!({"label": "checkpoint-a"}),
            summary_text: Some("Checkpoint A".to_owned()),
        })
        .expect("create session artifact");

    let artifacts = repo
        .list_session_artifacts("root-session")
        .expect("list artifacts");

    assert_eq!(artifact.kind, SessionArtifactKind::Checkpoint);
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].summary_text.as_deref(), Some("Checkpoint A"));
    assert_eq!(artifacts[0].payload_json["label"], "checkpoint-a");
}

#[test]
fn create_session_artifact_persists_and_lists_branch_summary_metadata() {
    let config = isolated_memory_config("session-branch-summary-artifact");
    let repo = SessionRepository::new(&config).expect("repository");
    create_root_session(&repo, "root-session");
    append_session_turn(&config, "root-session", "user", "hello");

    let artifact = repo
        .create_session_artifact(NewSessionArtifactRecord {
            artifact_id: "artifact-branch-summary-1".to_owned(),
            session_id: "root-session".to_owned(),
            kind: SessionArtifactKind::BranchSummary,
            head_name: Some(ACTIVE_SESSION_HEAD_NAME.to_owned()),
            anchor_node_id: Some("session-root:root-session".to_owned()),
            source_start_node_id: Some("session-turn:root-session:1".to_owned()),
            source_end_node_id: Some("session-turn:root-session:1".to_owned()),
            payload_json: json!({
                "head_name": "active",
                "exclusive_node_count": 1
            }),
            summary_text: Some("Branch Summary A".to_owned()),
        })
        .expect("create branch summary artifact");

    let artifacts = repo
        .list_session_artifacts("root-session")
        .expect("list artifacts");

    assert_eq!(artifact.kind, SessionArtifactKind::BranchSummary);
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].head_name.as_deref(), Some("active"));
    assert_eq!(
        artifacts[0].summary_text.as_deref(),
        Some("Branch Summary A")
    );
    assert_eq!(artifacts[0].payload_json["exclusive_node_count"], 1);
}

#[test]
fn create_session_artifact_enforces_overlay_retention_to_latest_records() {
    let config = isolated_memory_config("session-artifact-overlay-retention");
    let repo = SessionRepository::new(&config)
        .expect("repository")
        .with_max_total_artifacts(Some(2));
    create_root_session(&repo, "root-session");
    append_session_turn(&config, "root-session", "user", "hello");

    for (artifact_id, label) in [
        ("artifact-1", "Checkpoint A"),
        ("artifact-2", "Checkpoint B"),
        ("artifact-3", "Checkpoint C"),
    ] {
        repo.create_session_artifact(NewSessionArtifactRecord {
            artifact_id: artifact_id.to_owned(),
            session_id: "root-session".to_owned(),
            kind: SessionArtifactKind::Checkpoint,
            head_name: Some(ACTIVE_SESSION_HEAD_NAME.to_owned()),
            anchor_node_id: Some("session-turn:root-session:1".to_owned()),
            source_start_node_id: Some("session-turn:root-session:1".to_owned()),
            source_end_node_id: Some("session-turn:root-session:1".to_owned()),
            payload_json: json!({ "label": label }),
            summary_text: Some(label.to_owned()),
        })
        .expect("create session artifact");
    }

    let retained_ids = repo
        .list_session_artifacts("root-session")
        .expect("list artifacts")
        .into_iter()
        .map(|artifact| artifact.artifact_id)
        .collect::<Vec<_>>();
    assert_eq!(retained_ids, vec!["artifact-2", "artifact-3"]);
}

#[test]
fn replace_turns_preserves_named_heads_and_artifacts_across_rewrites() {
    let config = isolated_memory_config("replace-turns-preserves-tree-metadata");
    let repo = SessionRepository::new(&config).expect("repository");
    create_root_session(&repo, "root-session");
    append_session_turn(&config, "root-session", "user", "hello");
    append_session_turn(&config, "root-session", "assistant", "world");

    repo.fork_session_head(
        "root-session",
        "session-turn:root-session:1",
        "thread/alpha",
    )
    .expect("fork thread head");
    repo.create_session_artifact(NewSessionArtifactRecord {
        artifact_id: "artifact-branch-summary-1".to_owned(),
        session_id: "root-session".to_owned(),
        kind: SessionArtifactKind::BranchSummary,
        head_name: Some("thread/alpha".to_owned()),
        anchor_node_id: Some("session-root:root-session".to_owned()),
        source_start_node_id: Some("session-turn:root-session:1".to_owned()),
        source_end_node_id: Some("session-turn:root-session:1".to_owned()),
        payload_json: json!({
            "head_name": "thread/alpha",
            "exclusive_node_count": 1
        }),
        summary_text: Some("Branch Summary A".to_owned()),
    })
    .expect("create branch summary artifact");

    store::replace_session_turns_direct(
        "root-session",
        &[
            store::SessionWindowTurn {
                role: "user".to_owned(),
                content: "rewritten-user".to_owned(),
                ts: Some(100),
            },
            store::SessionWindowTurn {
                role: "assistant".to_owned(),
                content: "rewritten-assistant".to_owned(),
                ts: Some(101),
            },
        ],
        &config,
    )
    .expect("replace session turns");

    let heads = repo
        .list_session_heads("root-session")
        .expect("list session heads");
    let artifacts = repo
        .list_session_artifacts("root-session")
        .expect("list session artifacts");
    let active_path = repo
        .load_active_session_path("root-session")
        .expect("load active session path");

    assert_eq!(heads.len(), 2);
    assert!(
        heads
            .iter()
            .any(|head| head.head_name == ACTIVE_SESSION_HEAD_NAME)
    );
    assert!(heads.iter().any(|head| head.head_name == "thread/alpha"));
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].head_name.as_deref(), Some("thread/alpha"));
    assert_eq!(active_path.len(), 3);
    assert_eq!(active_path[1].content.as_deref(), Some("rewritten-user"));
    assert_eq!(
        active_path[2].content.as_deref(),
        Some("rewritten-assistant")
    );
}

// Regression guard for the shortening-rewrite case — the sibling to the
// test above, which only exercises same-length rewrite.
//
// Scenario: 5-turn session, fork `checkpoint/foo` at turn 5, attach a
// checkpoint artifact whose source range is turn 5, then rewrite the
// transcript down to 2 turns. After rebuild, turn 5's node is gone. The
// session-tree layer must not leave `checkpoint/foo` pointing at the
// vanished node, and must not leave the artifact's `source_*_node_id`
// columns dangling — that is silent data corruption. Expected behaviour
// after the fix: the stale head is dropped with a `session_events` audit
// record capturing the pre-rewrite content, and the artifact's
// out-of-range node-id columns are nulled (payload_json / summary_text
// stay intact, so the artifact's own content is preserved).
#[test]
fn replace_turns_shorter_drops_stale_head_and_nulls_artifact_refs() {
    let config = isolated_memory_config("replace-turns-shorter-preserves");
    let repo = SessionRepository::new(&config).expect("repository");
    create_root_session(&repo, "root-session");
    for i in 1..=5 {
        let role = if i % 2 == 1 { "user" } else { "assistant" };
        append_session_turn(&config, "root-session", role, &format!("turn-{i}"));
    }

    repo.fork_session_head(
        "root-session",
        "session-turn:root-session:5",
        "checkpoint/foo",
    )
    .expect("fork checkpoint head");

    repo.create_session_artifact(NewSessionArtifactRecord {
        artifact_id: "artifact-checkpoint-1".to_owned(),
        session_id: "root-session".to_owned(),
        kind: SessionArtifactKind::Checkpoint,
        head_name: Some("checkpoint/foo".to_owned()),
        anchor_node_id: Some("session-turn:root-session:5".to_owned()),
        source_start_node_id: Some("session-turn:root-session:5".to_owned()),
        source_end_node_id: Some("session-turn:root-session:5".to_owned()),
        payload_json: json!({ "exclusive_node_count": 1 }),
        summary_text: Some("Checkpoint at turn 5".to_owned()),
    })
    .expect("create checkpoint artifact");

    let rewrite_started_at = unix_ts_now();
    store::replace_session_turns_direct(
        "root-session",
        &[
            store::SessionWindowTurn {
                role: "user".to_owned(),
                content: "rewritten-1".to_owned(),
                ts: Some(100),
            },
            store::SessionWindowTurn {
                role: "assistant".to_owned(),
                content: "rewritten-2".to_owned(),
                ts: Some(101),
            },
        ],
        &config,
    )
    .expect("replace session turns");
    let rewrite_finished_at = unix_ts_now();

    // (1) The stale head must be dropped -- it pointed at a node that no
    //     longer exists, so leaving it in the table is the corruption.
    let heads = repo.list_session_heads("root-session").expect("list heads");
    assert!(
        heads.iter().all(|h| h.head_name != "checkpoint/foo"),
        "checkpoint/foo head was not dropped; pointer would dangle: {heads:?}"
    );

    // (2) An audit event records the drop, preserving the original head
    //     name, the stale node id, and a snapshot of the original
    //     content so operators can see what was lost.
    let events = repo
        .list_all_events("root-session", 128)
        .expect("list events");
    let drop_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_kind == "session_tree_rewrite_dropped_head")
        .collect();
    assert_eq!(
        drop_events.len(),
        1,
        "expected 1 drop event, got {drop_events:?}"
    );
    let payload = &drop_events[0].payload_json;
    assert_eq!(payload["head_name"].as_str(), Some("checkpoint/foo"));
    assert_eq!(
        payload["stale_node_id"].as_str(),
        Some("session-turn:root-session:5"),
    );
    assert_eq!(payload["content_snapshot"].as_str(), Some("turn-5"));
    assert!(
        drop_events[0].ts >= rewrite_started_at && drop_events[0].ts <= rewrite_finished_at,
        "drop event ts should reflect rewrite time, got {} outside [{rewrite_started_at}, {rewrite_finished_at}]",
        drop_events[0].ts
    );
    let conn = repo.open_connection().expect("open conn");
    let drop_event_search_text = conn
        .query_row(
            "SELECT search_text FROM session_events WHERE id = ?1",
            params![drop_events[0].id],
            |row| row.get::<_, String>(0),
        )
        .expect("load drop event search_text");
    assert!(
        !drop_event_search_text.is_empty(),
        "drop event search_text should be indexed"
    );

    // (3) The artifact row survives — its own payload_json /
    //     summary_text is not node-referential, so we keep it.
    let artifacts = repo
        .list_session_artifacts("root-session")
        .expect("list artifacts");
    let artifact = artifacts
        .iter()
        .find(|a| a.artifact_id == "artifact-checkpoint-1")
        .expect("artifact survives");
    assert_eq!(
        artifact.summary_text.as_deref(),
        Some("Checkpoint at turn 5")
    );

    // (4) But its out-of-range *_node_id columns must be nulled so
    //     nothing downstream can follow a dangling pointer.
    assert!(
        artifact.anchor_node_id.is_none(),
        "anchor_node_id still dangles: {:?}",
        artifact.anchor_node_id
    );
    assert!(
        artifact.source_start_node_id.is_none(),
        "source_start_node_id still dangles: {:?}",
        artifact.source_start_node_id
    );
    assert!(
        artifact.source_end_node_id.is_none(),
        "source_end_node_id still dangles: {:?}",
        artifact.source_end_node_id
    );

    // (5) artifact.head_name must be nulled in lockstep with the dropped
    //     head, otherwise session_artifacts.head_name points at a row
    //     that no longer exists in session_heads -- the same shape of
    //     dangling pointer the *_node_id nulling guards against.
    assert!(
        artifact.head_name.is_none(),
        "artifact.head_name still references the dropped head: {:?}",
        artifact.head_name
    );
}

// Edge: only some of the artifact's *_node_id columns dangle; the in-range
// columns must be preserved verbatim. Exercises the per-column null logic
// in `preserve_session_tree_before_rebuild`.
#[test]
fn replace_turns_shorter_only_nulls_dangling_artifact_columns() {
    let config = isolated_memory_config("replace-turns-partial-dangle");
    let repo = SessionRepository::new(&config).expect("repository");
    create_root_session(&repo, "root-session");
    for i in 1..=5 {
        let role = if i % 2 == 1 { "user" } else { "assistant" };
        append_session_turn(&config, "root-session", role, &format!("turn-{i}"));
    }

    // anchor + source_start at turn 1 (in-range when truncating to 2 turns);
    // source_end at turn 5 (out-of-range).
    repo.create_session_artifact(NewSessionArtifactRecord {
        artifact_id: "artifact-partial-1".to_owned(),
        session_id: "root-session".to_owned(),
        kind: SessionArtifactKind::BranchSummary,
        head_name: None,
        anchor_node_id: Some("session-turn:root-session:1".to_owned()),
        source_start_node_id: Some("session-turn:root-session:1".to_owned()),
        source_end_node_id: Some("session-turn:root-session:5".to_owned()),
        payload_json: json!({ "exclusive_node_count": 5 }),
        summary_text: Some("Span turns 1-5".to_owned()),
    })
    .expect("create artifact");

    let rewrite_started_at = unix_ts_now();
    store::replace_session_turns_direct(
        "root-session",
        &[
            store::SessionWindowTurn {
                role: "user".to_owned(),
                content: "rewritten-1".to_owned(),
                ts: Some(100),
            },
            store::SessionWindowTurn {
                role: "assistant".to_owned(),
                content: "rewritten-2".to_owned(),
                ts: Some(101),
            },
        ],
        &config,
    )
    .expect("replace session turns");
    let rewrite_finished_at = unix_ts_now();

    let artifacts = repo
        .list_session_artifacts("root-session")
        .expect("list artifacts");
    let artifact = artifacts
        .iter()
        .find(|a| a.artifact_id == "artifact-partial-1")
        .expect("artifact survives");

    // Preserved (in-range)
    assert_eq!(
        artifact.anchor_node_id.as_deref(),
        Some("session-turn:root-session:1")
    );
    assert_eq!(
        artifact.source_start_node_id.as_deref(),
        Some("session-turn:root-session:1")
    );
    // Nulled (out-of-range)
    assert!(
        artifact.source_end_node_id.is_none(),
        "source_end_node_id should be nulled, got {:?}",
        artifact.source_end_node_id
    );
    // Self-contained content untouched
    assert_eq!(artifact.summary_text.as_deref(), Some("Span turns 1-5"));
    assert_eq!(artifact.payload_json["exclusive_node_count"], 5);

    // Audit event: only `source_end` listed in `original_*` payload.
    let events = repo
        .list_all_events("root-session", 128)
        .expect("list events");
    let null_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_kind == "session_tree_rewrite_nulled_artifact_refs")
        .collect();
    assert_eq!(null_events.len(), 1);
    let payload = &null_events[0].payload_json;
    assert!(payload["original_head_name"].is_null());
    assert!(payload["original_anchor_node_id"].is_null());
    assert!(payload["original_source_start_node_id"].is_null());
    assert_eq!(
        payload["original_source_end_node_id"].as_str(),
        Some("session-turn:root-session:5")
    );
    assert!(
        null_events[0].ts >= rewrite_started_at && null_events[0].ts <= rewrite_finished_at,
        "null-ref event ts should reflect rewrite time, got {} outside [{rewrite_started_at}, {rewrite_finished_at}]",
        null_events[0].ts
    );
    let conn = repo.open_connection().expect("open conn");
    let null_event_search_text = conn
        .query_row(
            "SELECT search_text FROM session_events WHERE id = ?1",
            params![null_events[0].id],
            |row| row.get::<_, String>(0),
        )
        .expect("load null event search_text");
    assert!(
        !null_event_search_text.is_empty(),
        "null-ref event search_text should be indexed"
    );
}

// Edge: a dropped head can still be referenced by an artifact whose node
// refs remain in-range. That `artifact.head_name` cleanup must not be
// silent; the preservation event should record the head-name nulling even
// when the node-id columns survive untouched.
#[test]
fn replace_turns_shorter_nulls_artifact_head_name_when_nodes_survive() {
    let config = isolated_memory_config("replace-turns-head-name-only-dangle");
    let repo = SessionRepository::new(&config).expect("repository");
    create_root_session(&repo, "root-session");
    for i in 1..=5 {
        let role = if i % 2 == 1 { "user" } else { "assistant" };
        append_session_turn(&config, "root-session", role, &format!("turn-{i}"));
    }

    repo.fork_session_head(
        "root-session",
        "session-turn:root-session:5",
        "checkpoint/foo",
    )
    .expect("fork checkpoint head");

    repo.create_session_artifact(NewSessionArtifactRecord {
        artifact_id: "artifact-head-name-1".to_owned(),
        session_id: "root-session".to_owned(),
        kind: SessionArtifactKind::BranchSummary,
        head_name: Some("checkpoint/foo".to_owned()),
        anchor_node_id: Some("session-turn:root-session:1".to_owned()),
        source_start_node_id: Some("session-turn:root-session:1".to_owned()),
        source_end_node_id: Some("session-turn:root-session:1".to_owned()),
        payload_json: json!({ "exclusive_node_count": 1 }),
        summary_text: Some("Summary still anchored at turn 1".to_owned()),
    })
    .expect("create branch summary artifact");

    store::replace_session_turns_direct(
        "root-session",
        &[
            store::SessionWindowTurn {
                role: "user".to_owned(),
                content: "rewritten-1".to_owned(),
                ts: Some(100),
            },
            store::SessionWindowTurn {
                role: "assistant".to_owned(),
                content: "rewritten-2".to_owned(),
                ts: Some(101),
            },
        ],
        &config,
    )
    .expect("replace session turns");

    let heads = repo.list_session_heads("root-session").expect("list heads");
    assert!(
        heads.iter().all(|head| head.head_name != "checkpoint/foo"),
        "checkpoint/foo head should be dropped after the rewrite: {heads:?}"
    );

    let artifacts = repo
        .list_session_artifacts("root-session")
        .expect("list artifacts");
    let artifact = artifacts
        .iter()
        .find(|current_artifact| current_artifact.artifact_id == "artifact-head-name-1")
        .expect("artifact survives");
    assert!(
        artifact.head_name.is_none(),
        "artifact.head_name should be nulled when its head is dropped: {:?}",
        artifact.head_name
    );
    assert_eq!(
        artifact.anchor_node_id.as_deref(),
        Some("session-turn:root-session:1")
    );
    assert_eq!(
        artifact.source_start_node_id.as_deref(),
        Some("session-turn:root-session:1")
    );
    assert_eq!(
        artifact.source_end_node_id.as_deref(),
        Some("session-turn:root-session:1")
    );

    let events = repo
        .list_all_events("root-session", 128)
        .expect("list events");
    let null_events: Vec<_> = events
        .iter()
        .filter(|event| event.event_kind == "session_tree_rewrite_nulled_artifact_refs")
        .collect();
    assert_eq!(null_events.len(), 1);
    let payload = &null_events[0].payload_json;
    assert_eq!(
        payload["original_head_name"].as_str(),
        Some("checkpoint/foo")
    );
    assert!(payload["original_anchor_node_id"].is_null());
    assert!(payload["original_source_start_node_id"].is_null());
    assert!(payload["original_source_end_node_id"].is_null());
}

// Edge: two heads pointing past new tail get separate audit events; one
// head still in-range survives untouched.
#[test]
fn replace_turns_shorter_drops_multiple_stale_heads_with_separate_events() {
    let config = isolated_memory_config("replace-turns-multi-heads");
    let repo = SessionRepository::new(&config).expect("repository");
    create_root_session(&repo, "root-session");
    for i in 1..=5 {
        let role = if i % 2 == 1 { "user" } else { "assistant" };
        append_session_turn(&config, "root-session", role, &format!("turn-{i}"));
    }

    repo.fork_session_head(
        "root-session",
        "session-turn:root-session:5",
        "checkpoint/foo",
    )
    .expect("fork foo");
    repo.fork_session_head(
        "root-session",
        "session-turn:root-session:4",
        "thread/alpha",
    )
    .expect("fork alpha");
    repo.fork_session_head(
        "root-session",
        "session-turn:root-session:1",
        "checkpoint/early",
    )
    .expect("fork early");

    store::replace_session_turns_direct(
        "root-session",
        &[
            store::SessionWindowTurn {
                role: "user".to_owned(),
                content: "r1".to_owned(),
                ts: Some(100),
            },
            store::SessionWindowTurn {
                role: "assistant".to_owned(),
                content: "r2".to_owned(),
                ts: Some(101),
            },
        ],
        &config,
    )
    .expect("replace session turns");

    let heads = repo.list_session_heads("root-session").expect("list heads");
    let head_names: Vec<&str> = heads.iter().map(|h| h.head_name.as_str()).collect();
    assert!(head_names.contains(&ACTIVE_SESSION_HEAD_NAME));
    assert!(
        head_names.contains(&"checkpoint/early"),
        "in-range head survives"
    );
    assert!(
        !head_names.contains(&"checkpoint/foo"),
        "foo @ turn 5 dropped"
    );
    assert!(
        !head_names.contains(&"thread/alpha"),
        "alpha @ turn 4 dropped"
    );

    let events = repo
        .list_all_events("root-session", 128)
        .expect("list events");
    let drop_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_kind == "session_tree_rewrite_dropped_head")
        .collect();
    assert_eq!(drop_events.len(), 2);
    let dropped_names: Vec<&str> = drop_events
        .iter()
        .map(|e| e.payload_json["head_name"].as_str().unwrap_or(""))
        .collect();
    assert!(dropped_names.contains(&"checkpoint/foo"));
    assert!(dropped_names.contains(&"thread/alpha"));
}

// Edge: empty rewrite (turns.len() == 0). All non-active heads must be
// dropped; active must repoint at root; the root node must survive
// (recreated by the rebuild phase via deterministic id).
#[test]
fn replace_turns_empty_drops_non_active_heads_and_keeps_root() {
    let config = isolated_memory_config("replace-turns-empty");
    let repo = SessionRepository::new(&config).expect("repository");
    create_root_session(&repo, "root-session");
    append_session_turn(&config, "root-session", "user", "turn-1");
    append_session_turn(&config, "root-session", "assistant", "turn-2");

    repo.fork_session_head(
        "root-session",
        "session-turn:root-session:2",
        "checkpoint/foo",
    )
    .expect("fork");

    store::replace_session_turns_direct("root-session", &[], &config)
        .expect("replace with empty turns");

    let heads = repo.list_session_heads("root-session").expect("list heads");
    let head_names: Vec<&str> = heads.iter().map(|h| h.head_name.as_str()).collect();
    assert_eq!(
        head_names,
        vec![ACTIVE_SESSION_HEAD_NAME],
        "only active head remains after empty rewrite"
    );

    let active = heads
        .iter()
        .find(|h| h.head_name == ACTIVE_SESSION_HEAD_NAME)
        .expect("active head present");
    assert_eq!(
        active.node_id, "session-root:root-session",
        "active head repoints at root when no turns remain"
    );

    let nodes = repo.list_session_nodes("root-session").expect("list nodes");
    assert_eq!(nodes.len(), 1, "only root node survives");
    assert!(
        nodes[0].session_turn_index.is_none(),
        "root node has no turn_index"
    );
}

// Edge: legacy dangling head (target node was already missing before this
// rewrite). Preservation phase cleans it up too -- exercises the
// `n.node_id IS NULL` branch in the stale-head SELECT.
#[test]
fn replace_turns_cleans_up_legacy_dangling_head() {
    let config = isolated_memory_config("replace-turns-legacy-dangle");
    let repo = SessionRepository::new(&config).expect("repository");
    create_root_session(&repo, "root-session");
    append_session_turn(&config, "root-session", "user", "turn-1");

    // Inject a head that points at a node id that does not exist.
    {
        let conn = repo.open_connection().expect("open conn");
        conn.execute(
            "INSERT INTO session_heads(session_id, head_name, node_id, head_mode, updated_at)
                 VALUES (?1, ?2, ?3, 'live', ?4)",
            params![
                "root-session",
                "checkpoint/legacy",
                "session-turn:root-session:99",
                0_i64
            ],
        )
        .expect("inject legacy head");
    }
    let heads_pre = repo
        .list_session_heads("root-session")
        .expect("pre-list heads");
    assert!(
        heads_pre.iter().any(|h| h.head_name == "checkpoint/legacy"),
        "legacy head injected"
    );

    // Trigger any rewrite to fire the preservation phase.
    store::replace_session_turns_direct(
        "root-session",
        &[store::SessionWindowTurn {
            role: "user".to_owned(),
            content: "r1".to_owned(),
            ts: Some(100),
        }],
        &config,
    )
    .expect("replace");

    let heads_post = repo
        .list_session_heads("root-session")
        .expect("post-list heads");
    assert!(
        !heads_post
            .iter()
            .any(|h| h.head_name == "checkpoint/legacy"),
        "legacy dangling head was cleaned up"
    );

    let events = repo
        .list_all_events("root-session", 128)
        .expect("list events");
    let drop_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_kind == "session_tree_rewrite_dropped_head")
        .collect();
    let legacy_drop = drop_events
        .iter()
        .find(|e| e.payload_json["head_name"].as_str() == Some("checkpoint/legacy"))
        .expect("legacy drop event recorded");
    assert_eq!(
        legacy_drop.payload_json["stale_node_id"].as_str(),
        Some("session-turn:root-session:99")
    );
    // Content snapshot is null since the target never existed.
    assert!(legacy_drop.payload_json["content_snapshot"].is_null());
}

// Edge: same-length rewrite must NOT emit any preservation events; the
// existing test only verifies that heads + artifacts survive, not that
// the preservation phase stays a no-op.
#[test]
fn replace_turns_same_length_emits_no_preservation_events() {
    let config = isolated_memory_config("replace-turns-equal-length-noop");
    let repo = SessionRepository::new(&config).expect("repository");
    create_root_session(&repo, "root-session");
    append_session_turn(&config, "root-session", "user", "turn-1");
    append_session_turn(&config, "root-session", "assistant", "turn-2");

    repo.fork_session_head(
        "root-session",
        "session-turn:root-session:1",
        "thread/alpha",
    )
    .expect("fork");
    repo.create_session_artifact(NewSessionArtifactRecord {
        artifact_id: "art-1".to_owned(),
        session_id: "root-session".to_owned(),
        kind: SessionArtifactKind::BranchSummary,
        head_name: Some("thread/alpha".to_owned()),
        anchor_node_id: Some("session-turn:root-session:1".to_owned()),
        source_start_node_id: Some("session-turn:root-session:1".to_owned()),
        source_end_node_id: Some("session-turn:root-session:1".to_owned()),
        payload_json: json!({}),
        summary_text: Some("s".to_owned()),
    })
    .expect("create artifact");

    store::replace_session_turns_direct(
        "root-session",
        &[
            store::SessionWindowTurn {
                role: "user".to_owned(),
                content: "r1".to_owned(),
                ts: Some(100),
            },
            store::SessionWindowTurn {
                role: "assistant".to_owned(),
                content: "r2".to_owned(),
                ts: Some(101),
            },
        ],
        &config,
    )
    .expect("replace");

    let events = repo
        .list_all_events("root-session", 128)
        .expect("list events");
    let preserve_events: Vec<_> = events
        .iter()
        .filter(|e| {
            e.event_kind == "session_tree_rewrite_dropped_head"
                || e.event_kind == "session_tree_rewrite_nulled_artifact_refs"
        })
        .collect();
    assert!(
        preserve_events.is_empty(),
        "expected zero preservation events for same-length rewrite, got {preserve_events:?}"
    );

    // Belt-and-suspenders: confirm head + artifact survive untouched.
    let heads = repo.list_session_heads("root-session").expect("list heads");
    assert!(heads.iter().any(|h| h.head_name == "thread/alpha"));
    let artifacts = repo
        .list_session_artifacts("root-session")
        .expect("list artifacts");
    let art = artifacts
        .iter()
        .find(|a| a.artifact_id == "art-1")
        .expect("survives");
    assert_eq!(
        art.anchor_node_id.as_deref(),
        Some("session-turn:root-session:1")
    );
}

#[test]
fn session_repository_updates_state_and_last_error() {
    let config = isolated_memory_config("update-state");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create session");

    let updated = repo
        .update_session_state(
            "child-session",
            SessionState::Failed,
            Some("tool timeout".to_owned()),
        )
        .expect("update session state");
    assert_eq!(updated.state, SessionState::Failed);
    assert_eq!(updated.last_error.as_deref(), Some("tool timeout"));
}

#[test]
fn session_repository_conditional_state_update_requires_expected_state() {
    let config = isolated_memory_config("update-state-if-current");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Completed,
    })
    .expect("create session");

    let updated = repo
        .update_session_state_if_current(
            "child-session",
            SessionState::Ready,
            SessionState::Running,
            None,
        )
        .expect("conditional update should succeed");
    assert!(updated.is_none());

    let loaded = repo
        .load_session("child-session")
        .expect("load session")
        .expect("session row");
    assert_eq!(loaded.state, SessionState::Completed);
}

#[test]
fn transition_session_with_event_if_current_writes_state_and_event_together() {
    let config = isolated_memory_config("transition-session-with-event");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child");

    let transitioned = repo
        .transition_session_with_event_if_current(
            "child-session",
            TransitionSessionWithEventIfCurrentRequest {
                expected_state: SessionState::Ready,
                next_state: SessionState::Running,
                last_error: None,
                event_kind: "delegate_started".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "task": "child task",
                    "timeout_seconds": 60
                }),
            },
        )
        .expect("transition session with event")
        .expect("transition result");

    assert_eq!(transitioned.session.state, SessionState::Running);
    assert_eq!(transitioned.session.last_error, None);
    assert_eq!(transitioned.event.event_kind, "delegate_started");
    assert_eq!(
        transitioned.event.actor_session_id.as_deref(),
        Some("root-session")
    );

    let child = repo
        .load_session("child-session")
        .expect("load child")
        .expect("child row");
    assert_eq!(child.state, SessionState::Running);

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_kind, "delegate_started");
}

#[test]
fn transition_session_with_event_if_current_rolls_back_state_when_event_insert_fails() {
    let config = isolated_memory_config("transition-session-with-event-rollback");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child");

    let conn = repo.open_connection().expect("open connection");
    conn.execute("DROP TABLE session_events", [])
        .expect("drop session_events table");

    let error = repo
        .transition_session_with_event_if_current(
            "child-session",
            TransitionSessionWithEventIfCurrentRequest {
                expected_state: SessionState::Ready,
                next_state: SessionState::Running,
                last_error: None,
                event_kind: "delegate_started".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "task": "child task",
                    "timeout_seconds": 60
                }),
            },
        )
        .expect_err("transition should fail when event insert fails");
    assert!(error.contains("insert session transition event failed"));

    let child = repo
        .load_session("child-session")
        .expect("load child")
        .expect("child row");
    assert_eq!(child.state, SessionState::Ready);

    let events_error = repo
        .list_recent_events("child-session", 10)
        .expect_err("list events should fail after dropping table");
    assert!(events_error.contains("prepare session event query failed"));
}

#[test]
fn transition_session_with_event_and_clear_terminal_outcome_clears_existing_terminal_row() {
    let config = isolated_memory_config("transition-session-clear-terminal");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Completed,
    })
    .expect("create child");
    repo.upsert_terminal_outcome(
        "child-session",
        "ok",
        json!({
            "child_session_id": "child-session",
            "final_output": "old"
        }),
    )
    .expect("upsert terminal outcome");

    let transitioned = repo
        .transition_session_with_event_and_clear_terminal_outcome_if_current(
            "child-session",
            TransitionSessionWithEventIfCurrentRequest {
                expected_state: SessionState::Completed,
                next_state: SessionState::Running,
                last_error: None,
                event_kind: "delegate_started".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "task": "continued child task",
                    "timeout_seconds": 60
                }),
            },
        )
        .expect("transition should succeed")
        .expect("transition result");

    assert_eq!(transitioned.session.state, SessionState::Running);
    assert!(
        repo.load_terminal_outcome("child-session")
            .expect("load cleared terminal outcome")
            .is_none(),
        "terminal outcome should be cleared before the next continued run"
    );

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_kind, "delegate_started");
}

#[test]
fn session_repository_ensure_session_is_idempotent() {
    let config = isolated_memory_config("ensure-session");
    let repo = SessionRepository::new(&config).expect("repository");

    let first = repo
        .ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("ensure root session");
    let second = repo
        .ensure_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("other-parent".to_owned()),
            label: Some("Ignored".to_owned()),
            state: SessionState::Failed,
        })
        .expect("ensure existing session");

    assert_eq!(first.session_id, second.session_id);
    assert_eq!(second.kind, SessionKind::Root);
    assert_eq!(second.parent_session_id, None);
    assert_eq!(second.label.as_deref(), Some("Root"));
    assert_eq!(repo.list_sessions().expect("list sessions").len(), 1);
}

#[test]
fn create_session_with_event_writes_session_and_event_together() {
    let config = isolated_memory_config("create-session-with-event");
    let repo = SessionRepository::new(&config).expect("repository");

    let created = repo
        .create_session_with_event(CreateSessionWithEventRequest {
            session: NewSessionRecord {
                session_id: "child-session".to_owned(),
                kind: SessionKind::DelegateChild,
                parent_session_id: Some("root-session".to_owned()),
                label: Some("Child".to_owned()),
                state: SessionState::Ready,
            },
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            event_payload_json: json!({
                "task": "child task",
                "timeout_seconds": 60
            }),
        })
        .expect("create session with queued event");

    assert_eq!(created.session.state, SessionState::Ready);
    assert_eq!(
        created.session.parent_session_id.as_deref(),
        Some("root-session")
    );
    assert_eq!(created.event.event_kind, "delegate_queued");
    assert_eq!(
        created.event.actor_session_id.as_deref(),
        Some("root-session")
    );

    let sessions = repo.list_sessions().expect("list sessions");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "child-session");

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list child events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_kind, "delegate_queued");
}

#[test]
fn create_session_with_event_rolls_back_session_when_event_insert_fails() {
    let config = isolated_memory_config("create-session-with-event-rollback");
    let repo = SessionRepository::new(&config).expect("repository");
    let conn = repo.open_connection().expect("open connection");
    conn.execute(
        "CREATE TRIGGER fail_create_session_event
             BEFORE INSERT ON session_events
             BEGIN
                SELECT RAISE(FAIL, 'forced create session event failure');
             END;",
        [],
    )
    .expect("create session event failure trigger");

    let error = repo
        .create_session_with_event(CreateSessionWithEventRequest {
            session: NewSessionRecord {
                session_id: "child-session".to_owned(),
                kind: SessionKind::DelegateChild,
                parent_session_id: Some("root-session".to_owned()),
                label: Some("Child".to_owned()),
                state: SessionState::Ready,
            },
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            event_payload_json: json!({
                "task": "child task",
                "timeout_seconds": 60
            }),
        })
        .expect_err("create session with event should fail when event insert fails");
    assert!(error.contains("insert session event failed"));

    assert!(
        repo.load_session("child-session")
            .expect("load child after rollback")
            .is_none()
    );
}

#[test]
fn create_delegate_child_session_with_event_if_within_limit_serializes_capacity() {
    let config = isolated_memory_config("delegate-child-limit-serialized");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");

    let config = Arc::new(config);
    let handles = ["child-session-a", "child-session-b"]
        .into_iter()
        .map(|child_session_id| {
            let config = Arc::clone(&config);
            thread::spawn(move || {
                let repo = SessionRepository::new(&config).expect("repository");
                repo.create_delegate_child_session_with_event_if_within_limit(
                    "root-session",
                    1,
                    |active_children| {
                        thread::park_timeout(Duration::from_millis(100));
                        Ok((
                            CreateSessionWithEventRequest {
                                session: NewSessionRecord {
                                    session_id: child_session_id.to_owned(),
                                    kind: SessionKind::DelegateChild,
                                    parent_session_id: Some("root-session".to_owned()),
                                    label: Some(child_session_id.to_owned()),
                                    state: SessionState::Ready,
                                },
                                event_kind: "delegate_queued".to_owned(),
                                actor_session_id: Some("root-session".to_owned()),
                                event_payload_json: json!({
                                    "task": child_session_id,
                                    "active_children": active_children
                                }),
                            },
                            active_children,
                        ))
                    },
                )
            })
        })
        .collect::<Vec<_>>();

    let mut active_children_values = Vec::new();
    let mut limit_errors = Vec::new();
    for handle in handles {
        match handle.join().expect("thread join") {
            Ok((created, active_children)) => {
                active_children_values.push(active_children);
                assert_eq!(
                    created.session.parent_session_id.as_deref(),
                    Some("root-session")
                );
            }
            Err(error) => limit_errors.push(error),
        }
    }

    assert_eq!(
        active_children_values,
        vec![0],
        "only one child should be admitted before capacity is exhausted"
    );
    assert_eq!(
        limit_errors.len(),
        1,
        "one concurrent admission should be rejected"
    );
    assert!(
        limit_errors[0].contains("delegate_active_children_exceeded"),
        "unexpected error: {}",
        limit_errors[0]
    );
    assert_eq!(
        repo.count_active_direct_children("root-session")
            .expect("count active direct children after concurrent admissions"),
        1
    );
}

#[test]
fn session_repository_lists_parent_child_relationships() {
    let config = isolated_memory_config("list-relationships");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");

    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({"depth": 1}),
    })
    .expect("append event");

    let sessions = repo.list_sessions().expect("list sessions");
    assert_eq!(sessions.len(), 2);
    let child = sessions
        .iter()
        .find(|session| session.session_id == "child-session")
        .expect("child session");
    assert_eq!(child.parent_session_id.as_deref(), Some("root-session"));
    assert_eq!(child.kind, SessionKind::DelegateChild);
}

#[test]
fn list_visible_sessions_infers_legacy_rows_from_turn_history_without_backfill() {
    let config = isolated_memory_config("legacy-visible-sessions");
    append_session_turn_direct("telegram:123", "user", "hello", &config).expect("append user turn");
    append_session_turn_direct("telegram:123", "assistant", "world", &config)
        .expect("append assistant turn");

    let repo = SessionRepository::new(&config).expect("repository");
    let sessions = repo
        .list_visible_sessions("telegram:123")
        .expect("list visible sessions");

    assert_eq!(sessions.len(), 1);
    let session = &sessions[0];
    assert_eq!(session.session_id, "telegram:123");
    assert_eq!(session.kind, SessionKind::Root);
    assert_eq!(session.parent_session_id, None);
    assert_eq!(session.turn_count, 2);
    assert!(session.last_turn_at.is_some());

    assert!(
        repo.load_session("telegram:123")
            .expect("load legacy session")
            .is_none()
    );
    assert!(
        repo.list_sessions()
            .expect("list concrete sessions")
            .is_empty()
    );
}

#[test]
fn inferred_legacy_session_kind_uses_known_prefixes() {
    let config = isolated_memory_config("legacy-kind-prefixes");
    append_session_turn_direct("delegate:legacy-child", "assistant", "done", &config)
        .expect("append delegate turn");
    append_session_turn_direct("telegram:456", "user", "ping", &config)
        .expect("append telegram turn");

    let repo = SessionRepository::new(&config).expect("repository");
    let delegate_session = repo
        .list_visible_sessions("delegate:legacy-child")
        .expect("list delegate legacy session")
        .into_iter()
        .find(|session| session.session_id == "delegate:legacy-child")
        .expect("delegate legacy session");
    assert_eq!(delegate_session.kind, SessionKind::DelegateChild);

    let telegram_session = repo
        .list_visible_sessions("telegram:456")
        .expect("list telegram legacy session")
        .into_iter()
        .find(|session| session.session_id == "telegram:456")
        .expect("telegram legacy session");
    assert_eq!(telegram_session.kind, SessionKind::Root);
}

#[test]
fn latest_resumable_root_session_prefers_newest_eligible_root() {
    let config = isolated_memory_config("latest-resumable-root");
    let repo = SessionRepository::new(&config).expect("repository");

    create_root_session(&repo, "root-old");
    append_session_turn(&config, "root-old", "user", "old");
    set_session_updated_at(&repo, "root-old", 100);
    set_turn_timestamps(&repo, "root-old", 100);

    create_root_session(&repo, "root-new");
    append_session_turn(&config, "root-new", "user", "new");
    set_session_updated_at(&repo, "root-new", 200);
    set_turn_timestamps(&repo, "root-new", 200);

    create_delegate_child_session(&repo, "delegate-child", "root-new");
    append_session_turn(&config, "delegate-child", "assistant", "child");
    set_session_updated_at(&repo, "delegate-child", 400);
    set_turn_timestamps(&repo, "delegate-child", 400);

    create_root_session(&repo, "root-archived");
    append_session_turn(&config, "root-archived", "assistant", "archived");
    set_session_updated_at(&repo, "root-archived", 500);
    set_turn_timestamps(&repo, "root-archived", 500);
    archive_session(&repo, "root-archived", 600);

    create_root_session(&repo, "root-empty");
    set_session_updated_at(&repo, "root-empty", 700);

    let latest = repo
        .latest_resumable_root_session_summary()
        .expect("load latest resumable root session")
        .expect("eligible root session");

    assert_eq!(latest.session_id, "root-new");
    assert_eq!(latest.kind, SessionKind::Root);
    assert_eq!(latest.archived_at, None);
    assert_eq!(latest.turn_count, 1);
}

#[test]
fn latest_resumable_root_session_includes_legacy_root_when_newest() {
    let config = isolated_memory_config("latest-legacy-root");
    let repo = SessionRepository::new(&config).expect("repository");

    create_root_session(&repo, "root-session");
    append_session_turn(&config, "root-session", "user", "root");
    set_session_updated_at(&repo, "root-session", 100);
    set_turn_timestamps(&repo, "root-session", 100);

    append_session_turn(&config, "telegram:latest", "assistant", "legacy");
    set_turn_timestamps(&repo, "telegram:latest", 200);

    let latest = repo
        .latest_resumable_root_session_summary()
        .expect("load latest resumable root session")
        .expect("latest session");

    assert_eq!(latest.session_id, "telegram:latest");
    assert_eq!(latest.kind, SessionKind::Root);
    assert_eq!(latest.turn_count, 1);
    assert_eq!(latest.last_turn_at, Some(200));
    assert!(
        repo.load_session("telegram:latest")
            .expect("load legacy session")
            .is_none()
    );
}

#[test]
fn latest_resumable_root_session_returns_none_when_no_root_is_resumable() {
    let config = isolated_memory_config("latest-no-resumable-root");
    let repo = SessionRepository::new(&config).expect("repository");

    create_root_session(&repo, "root-empty");
    set_session_updated_at(&repo, "root-empty", 300);

    create_root_session(&repo, "root-archived");
    append_session_turn(&config, "root-archived", "assistant", "archived");
    set_session_updated_at(&repo, "root-archived", 400);
    set_turn_timestamps(&repo, "root-archived", 400);
    archive_session(&repo, "root-archived", 500);

    create_delegate_child_session(&repo, "delegate-child", "root-archived");
    append_session_turn(&config, "delegate-child", "assistant", "delegate");
    set_session_updated_at(&repo, "delegate-child", 600);
    set_turn_timestamps(&repo, "delegate-child", 600);

    append_session_turn(
        &config,
        "delegate:legacy-child",
        "assistant",
        "legacy delegate",
    );
    set_turn_timestamps(&repo, "delegate:legacy-child", 700);

    let latest = repo
        .latest_resumable_root_session_summary()
        .expect("load latest resumable root session");

    assert!(latest.is_none());
}

#[test]
fn session_lineage_depth_counts_root_child_and_grandchild() {
    let config = isolated_memory_config("lineage-depth");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child");
    repo.create_session(NewSessionRecord {
        session_id: "grandchild-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("child-session".to_owned()),
        label: Some("Grandchild".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create grandchild");

    assert_eq!(
        repo.session_lineage_depth("root-session")
            .expect("root depth"),
        0
    );
    assert_eq!(
        repo.session_lineage_depth("child-session")
            .expect("child depth"),
        1
    );
    assert_eq!(
        repo.session_lineage_depth("grandchild-session")
            .expect("grandchild depth"),
        2
    );
}

#[test]
fn lineage_root_session_id_returns_root_for_delegate_descendants() {
    let config = isolated_memory_config("lineage-root");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child");
    repo.create_session(NewSessionRecord {
        session_id: "grandchild-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("child-session".to_owned()),
        label: Some("Grandchild".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create grandchild");

    assert_eq!(
        repo.lineage_root_session_id("root-session")
            .expect("root lineage root"),
        Some("root-session".to_owned())
    );
    assert_eq!(
        repo.lineage_root_session_id("grandchild-session")
            .expect("grandchild lineage root"),
        Some("root-session".to_owned())
    );
    assert_eq!(
        repo.lineage_root_session_id("missing-session")
            .expect("missing lineage root"),
        None
    );
}

#[test]
fn list_visible_sessions_includes_descendant_delegate_chain() {
    let config = isolated_memory_config("descendant-visibility");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child");
    repo.create_session(NewSessionRecord {
        session_id: "grandchild-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("child-session".to_owned()),
        label: Some("Grandchild".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create grandchild");

    let visible = repo
        .list_visible_sessions("root-session")
        .expect("visible sessions");
    let ids: Vec<&str> = visible
        .iter()
        .map(|session| session.session_id.as_str())
        .collect();
    assert!(ids.contains(&"root-session"));
    assert!(ids.contains(&"child-session"));
    assert!(ids.contains(&"grandchild-session"));
    assert!(
        repo.is_session_visible("root-session", "grandchild-session")
            .expect("root should see grandchild")
    );
}

#[test]
fn search_session_content_returns_turn_and_event_hits_for_session_scope() {
    let config = isolated_memory_config("search-session-content");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Completed,
    })
    .expect("create child");

    append_session_turn_direct(
        "child-session",
        "assistant",
        "Deploy freeze window is Friday and migration starts Saturday.",
        &config,
    )
    .expect("append assistant turn");
    repo.append_event(NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_completed".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "summary": "deploy freeze checklist completed"
        }),
    })
    .expect("append child event");

    let hits = repo
        .search_session_content("child-session", "deploy freeze", 8)
        .expect("search session content");

    assert!(
        hits.iter().any(|hit| {
            hit.source_kind == SessionSearchSourceKind::Turn
                && hit.content_text.contains("Deploy freeze window")
        }),
        "expected a turn hit, got: {hits:?}"
    );
    assert!(
        hits.iter().any(|hit| {
            hit.source_kind == SessionSearchSourceKind::Event
                && hit
                    .content_text
                    .contains("deploy freeze checklist completed")
        }),
        "expected an event hit, got: {hits:?}"
    );
}

#[test]
fn session_terminal_outcome_round_trips_payload_and_frozen_result() {
    let config = isolated_memory_config("terminal-outcome-round-trip");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Completed,
    })
    .expect("create child");

    let frozen_result = crate::session::frozen_result::FrozenResult {
        content: crate::session::frozen_result::FrozenContent::Text("done".to_owned()),
        captured_at: SystemTime::now(),
        byte_len: "done".len(),
        truncated: false,
    };

    repo.upsert_terminal_outcome_with_frozen_result(
        "child-session",
        "ok",
        json!({
            "child_session_id": "child-session",
            "final_output": "done",
            "duration_ms": 12
        }),
        Some(frozen_result.clone()),
    )
    .expect("upsert terminal outcome");

    let outcome = repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome")
        .expect("terminal outcome row");

    assert_eq!(outcome.session_id, "child-session");
    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload_json["final_output"], "done");
    assert_eq!(outcome.frozen_result, Some(frozen_result));
    assert!(outcome.recorded_at > 0);
}

#[test]
fn session_terminal_outcome_upsert_replaces_existing_row() {
    let config = isolated_memory_config("terminal-outcome-upsert");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Failed,
    })
    .expect("create child");

    repo.upsert_terminal_outcome(
        "child-session",
        "error",
        json!({
            "error": "first"
        }),
    )
    .expect("upsert first terminal outcome");
    repo.upsert_terminal_outcome(
        "child-session",
        "timeout",
        json!({
            "error": "delegate_timeout"
        }),
    )
    .expect("upsert second terminal outcome");

    let outcome = repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(outcome.status, "timeout");
    assert_eq!(outcome.payload_json["error"], "delegate_timeout");
}

#[test]
fn session_terminal_outcome_upsert_preserves_existing_frozen_result_when_none_is_supplied() {
    let config = isolated_memory_config("terminal-outcome-preserve-frozen-result");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Failed,
    })
    .expect("create child");

    let frozen_result = crate::session::frozen_result::FrozenResult {
        content: crate::session::frozen_result::FrozenContent::Text("done".to_owned()),
        captured_at: SystemTime::now(),
        byte_len: "done".len(),
        truncated: false,
    };

    repo.upsert_terminal_outcome_with_frozen_result(
        "child-session",
        "error",
        json!({
            "error": "first"
        }),
        Some(frozen_result.clone()),
    )
    .expect("upsert terminal outcome with frozen result");

    let outcome = repo
        .upsert_terminal_outcome(
            "child-session",
            "timeout",
            json!({
                "error": "delegate_timeout"
            }),
        )
        .expect("upsert terminal outcome without frozen result");

    assert_eq!(outcome.status, "timeout");
    assert_eq!(outcome.payload_json["error"], "delegate_timeout");
    assert_eq!(outcome.frozen_result, Some(frozen_result));
}

#[test]
fn finalize_session_terminal_writes_state_event_and_outcome_together() {
    let config = isolated_memory_config("finalize-session-terminal");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");

    let finalized = repo
        .finalize_session_terminal(
            "child-session",
            FinalizeSessionTerminalRequest {
                state: SessionState::Completed,
                last_error: None,
                event_kind: "delegate_completed".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "turn_count": 2,
                    "duration_ms": 15
                }),
                outcome_status: "ok".to_owned(),
                outcome_payload_json: json!({
                    "child_session_id": "child-session",
                    "final_output": "done",
                    "turn_count": 2,
                    "duration_ms": 15
                }),
                frozen_result: None,
            },
        )
        .expect("finalize session");

    assert_eq!(finalized.session.state, SessionState::Completed);
    assert_eq!(finalized.session.last_error, None);
    assert_eq!(finalized.event.event_kind, "delegate_completed");
    assert_eq!(
        finalized.event.actor_session_id.as_deref(),
        Some("root-session")
    );
    assert_eq!(finalized.terminal_outcome.status, "ok");
    assert_eq!(finalized.session.updated_at, finalized.event.ts);
    assert_eq!(finalized.event.ts, finalized.terminal_outcome.recorded_at);

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(child.state, SessionState::Completed);

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list child events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_kind, "delegate_completed");

    let outcome = repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload_json["final_output"], "done");
}

#[test]
fn finalize_session_terminal_replaces_previous_outcome_payload() {
    let config = isolated_memory_config("finalize-session-terminal-upsert");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");

    repo.finalize_session_terminal(
        "child-session",
        FinalizeSessionTerminalRequest {
            state: SessionState::Failed,
            last_error: Some("first".to_owned()),
            event_kind: "delegate_failed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            event_payload_json: json!({
                "error": "first"
            }),
            outcome_status: "error".to_owned(),
            outcome_payload_json: json!({
                "error": "first"
            }),
            frozen_result: None,
        },
    )
    .expect("finalize first terminal state");

    let finalized = repo
        .finalize_session_terminal(
            "child-session",
            FinalizeSessionTerminalRequest {
                state: SessionState::TimedOut,
                last_error: Some("delegate_timeout".to_owned()),
                event_kind: "delegate_timed_out".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "error": "delegate_timeout"
                }),
                outcome_status: "timeout".to_owned(),
                outcome_payload_json: json!({
                    "error": "delegate_timeout"
                }),
                frozen_result: None,
            },
        )
        .expect("finalize second terminal state");

    assert_eq!(finalized.session.state, SessionState::TimedOut);
    assert_eq!(
        finalized.session.last_error.as_deref(),
        Some("delegate_timeout")
    );
    assert_eq!(finalized.terminal_outcome.status, "timeout");
    assert_eq!(
        finalized.terminal_outcome.payload_json["error"],
        "delegate_timeout"
    );

    let outcome = repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(outcome.status, "timeout");
    assert_eq!(outcome.payload_json["error"], "delegate_timeout");
}

#[test]
fn finalize_session_terminal_if_current_writes_state_event_and_outcome_when_state_matches() {
    let config = isolated_memory_config("finalize-session-terminal-if-current");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child");

    let finalized = repo
        .finalize_session_terminal_if_current(
            "child-session",
            SessionState::Ready,
            FinalizeSessionTerminalRequest {
                state: SessionState::Failed,
                last_error: Some("delegate_timeout".to_owned()),
                event_kind: "delegate_recovery_applied".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "kind": "queued_async_overdue_marked_failed",
                    "reference": "queued"
                }),
                outcome_status: "error".to_owned(),
                outcome_payload_json: json!({
                    "error": "delegate_timeout"
                }),
                frozen_result: None,
            },
        )
        .expect("conditionally finalize session")
        .expect("conditional finalize result");

    assert_eq!(finalized.session.state, SessionState::Failed);
    assert_eq!(
        finalized.session.last_error.as_deref(),
        Some("delegate_timeout")
    );
    assert_eq!(finalized.event.event_kind, "delegate_recovery_applied");
    assert_eq!(finalized.terminal_outcome.status, "error");

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(child.state, SessionState::Failed);
    assert_eq!(child.last_error.as_deref(), Some("delegate_timeout"));

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list child events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_kind, "delegate_recovery_applied");

    let outcome = repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(outcome.status, "error");
    assert_eq!(outcome.payload_json["error"], "delegate_timeout");
}

#[test]
fn finalize_session_terminal_if_current_preserves_existing_frozen_result_when_none_is_supplied() {
    let config = isolated_memory_config("finalize-if-current-preserve-frozen-result");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child");

    let frozen_result = crate::session::frozen_result::FrozenResult {
        content: crate::session::frozen_result::FrozenContent::Text("done".to_owned()),
        captured_at: SystemTime::now(),
        byte_len: "done".len(),
        truncated: false,
    };
    repo.upsert_terminal_outcome_with_frozen_result(
        "child-session",
        "ok",
        json!({
            "final_output": "done"
        }),
        Some(frozen_result.clone()),
    )
    .expect("seed terminal outcome");

    let finalized = repo
        .finalize_session_terminal_if_current(
            "child-session",
            SessionState::Ready,
            FinalizeSessionTerminalRequest {
                state: SessionState::Failed,
                last_error: Some("delegate_timeout".to_owned()),
                event_kind: "delegate_recovery_applied".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "kind": "queued_async_overdue_marked_failed",
                    "reference": "queued"
                }),
                outcome_status: "error".to_owned(),
                outcome_payload_json: json!({
                    "error": "delegate_timeout"
                }),
                frozen_result: None,
            },
        )
        .expect("conditionally finalize session")
        .expect("conditional finalize result");

    assert_eq!(
        finalized.terminal_outcome.frozen_result,
        Some(frozen_result)
    );
}

#[test]
fn finalize_session_terminal_if_current_writes_nothing_when_state_does_not_match() {
    let config = isolated_memory_config("finalize-session-terminal-if-current-noop");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");

    let finalized = repo
        .finalize_session_terminal_if_current(
            "child-session",
            SessionState::Ready,
            FinalizeSessionTerminalRequest {
                state: SessionState::Failed,
                last_error: Some("delegate_timeout".to_owned()),
                event_kind: "delegate_recovery_applied".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                event_payload_json: json!({
                    "kind": "queued_async_overdue_marked_failed",
                    "reference": "queued"
                }),
                outcome_status: "error".to_owned(),
                outcome_payload_json: json!({
                    "error": "delegate_timeout"
                }),
                frozen_result: None,
            },
        )
        .expect("conditionally finalize session");

    assert!(finalized.is_none());

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(child.state, SessionState::Running);
    assert!(child.last_error.is_none());

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list child events");
    assert!(events.is_empty());

    let outcome = repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome");
    assert!(outcome.is_none());
}

#[test]
fn load_session_observation_drains_tail_after_cursor_through_terminal_event() {
    let config = isolated_memory_config("session-observation-tail-drain");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child");

    for index in 0..60 {
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: format!("delegate_progress_{index}"),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "step": index
            }),
        })
        .expect("append progress event");
    }
    repo.finalize_session_terminal(
        "child-session",
        FinalizeSessionTerminalRequest {
            state: SessionState::Completed,
            last_error: None,
            event_kind: "delegate_completed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            event_payload_json: json!({
                "turn_count": 1
            }),
            outcome_status: "ok".to_owned(),
            outcome_payload_json: json!({
                "child_session_id": "child-session",
                "final_output": "done"
            }),
            frozen_result: None,
        },
    )
    .expect("finalize child");

    let observation = repo
        .load_session_observation("child-session", 5, Some(0), 50)
        .expect("load session observation")
        .expect("session observation");

    assert_eq!(observation.session.state, SessionState::Completed);
    assert_eq!(
        observation
            .terminal_outcome
            .as_ref()
            .expect("terminal outcome")
            .status,
        "ok"
    );
    assert_eq!(observation.tail_events.len(), 61);
    assert_eq!(
        observation
            .tail_events
            .first()
            .expect("first tail event")
            .id,
        1
    );
    assert_eq!(
        observation
            .tail_events
            .last()
            .expect("last tail event")
            .event_kind,
        "delegate_completed"
    );
    assert_eq!(observation.recent_events.len(), 5);
    assert_eq!(
        observation
            .recent_events
            .last()
            .expect("last recent event")
            .event_kind,
        "delegate_completed"
    );
}

#[test]
fn approval_request_repository_persists_pending_request() {
    let config = isolated_memory_config("approval-request-create");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");

    let created = repo
        .ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id: "apr_123".to_owned(),
            session_id: "root-session".to_owned(),
            turn_id: "turn-123".to_owned(),
            tool_call_id: "call-123".to_owned(),
            tool_name: "delegate_async".to_owned(),
            approval_key: "tool:delegate_async".to_owned(),
            request_payload_json: json!({
                "tool_name": "delegate_async",
                "payload": {
                    "task": "inspect child issue"
                }
            }),
            governance_snapshot_json: json!({
                "reason": "governed_tool_requires_approval",
                "rule_id": "medium_balanced_delegate_async"
            }),
        })
        .expect("persist approval request");

    assert_eq!(created.approval_request_id, "apr_123");
    assert_eq!(created.session_id, "root-session");
    assert_eq!(created.tool_name, "delegate_async");
    assert_eq!(created.approval_key, "tool:delegate_async");
    assert_eq!(created.status, ApprovalRequestStatus::Pending);
    assert_eq!(created.decision, None);
    assert_eq!(
        created.request_payload_json["payload"]["task"],
        "inspect child issue"
    );
    assert_eq!(
        created.governance_snapshot_json["rule_id"],
        "medium_balanced_delegate_async"
    );
    assert!(created.resolved_at.is_none());
    assert!(created.executed_at.is_none());
    assert!(created.last_error.is_none());

    let loaded = repo
        .load_approval_request("apr_123")
        .expect("load approval request")
        .expect("approval request row");
    assert_eq!(loaded, created);
}

#[test]
fn approval_request_repository_duplicate_create_returns_existing_row() {
    let config = isolated_memory_config("approval-request-idempotent");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");

    let first = repo
        .ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id: "apr_duplicate".to_owned(),
            session_id: "root-session".to_owned(),
            turn_id: "turn-1".to_owned(),
            tool_call_id: "call-1".to_owned(),
            tool_name: "delegate".to_owned(),
            approval_key: "tool:delegate".to_owned(),
            request_payload_json: json!({
                "tool_name": "delegate",
                "payload": {
                    "task": "original"
                }
            }),
            governance_snapshot_json: json!({
                "reason": "first_reason",
                "rule_id": "first_rule"
            }),
        })
        .expect("persist first approval request");
    let second = repo
        .ensure_approval_request(NewApprovalRequestRecord {
            approval_request_id: "apr_duplicate".to_owned(),
            session_id: "root-session".to_owned(),
            turn_id: "turn-2".to_owned(),
            tool_call_id: "call-2".to_owned(),
            tool_name: "delegate_async".to_owned(),
            approval_key: "tool:delegate_async".to_owned(),
            request_payload_json: json!({
                "tool_name": "delegate_async",
                "payload": {
                    "task": "should_be_ignored"
                }
            }),
            governance_snapshot_json: json!({
                "reason": "second_reason",
                "rule_id": "second_rule"
            }),
        })
        .expect("persist second approval request");

    assert_eq!(second.approval_request_id, first.approval_request_id);
    assert_eq!(second.turn_id, first.turn_id);
    assert_eq!(second.tool_call_id, first.tool_call_id);
    assert_eq!(second.tool_name, first.tool_name);
    assert_eq!(second.approval_key, first.approval_key);
    assert_eq!(second.request_payload_json, first.request_payload_json);
    assert_eq!(
        second.governance_snapshot_json,
        first.governance_snapshot_json
    );
}

#[test]
fn approval_request_repository_transitions_status_if_current() {
    let config = isolated_memory_config("approval-request-transition");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");

    repo.ensure_approval_request(NewApprovalRequestRecord {
        approval_request_id: "apr-transition".to_owned(),
        session_id: "root-session".to_owned(),
        turn_id: "turn-1".to_owned(),
        tool_call_id: "call-1".to_owned(),
        tool_name: "delegate".to_owned(),
        approval_key: "tool:delegate".to_owned(),
        request_payload_json: json!({
            "tool_name": "delegate"
        }),
        governance_snapshot_json: json!({
            "reason": "requires_review",
            "rule_id": "delegate_review"
        }),
    })
    .expect("persist approval request");

    let approved = repo
        .transition_approval_request_if_current(
            "apr-transition",
            TransitionApprovalRequestIfCurrentRequest {
                expected_status: ApprovalRequestStatus::Pending,
                next_status: ApprovalRequestStatus::Approved,
                decision: Some(ApprovalDecision::ApproveOnce),
                resolved_by_session_id: Some("root-session".to_owned()),
                executed_at: None,
                last_error: None,
            },
        )
        .expect("transition approval request")
        .expect("transition result");
    assert_eq!(approved.status, ApprovalRequestStatus::Approved);
    assert_eq!(approved.decision, Some(ApprovalDecision::ApproveOnce));
    assert_eq!(
        approved.resolved_by_session_id.as_deref(),
        Some("root-session")
    );
    assert!(approved.resolved_at.is_some());
    assert!(approved.executed_at.is_none());
    assert!(approved.last_error.is_none());

    let noop = repo
        .transition_approval_request_if_current(
            "apr-transition",
            TransitionApprovalRequestIfCurrentRequest {
                expected_status: ApprovalRequestStatus::Pending,
                next_status: ApprovalRequestStatus::Denied,
                decision: Some(ApprovalDecision::Deny),
                resolved_by_session_id: Some("root-session".to_owned()),
                executed_at: None,
                last_error: Some("should not apply".to_owned()),
            },
        )
        .expect("stale transition should not error");
    assert!(noop.is_none());
}

#[test]
fn approval_request_repository_persists_session_scoped_runtime_grant() {
    let config = isolated_memory_config("approval-grant-upsert");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");

    let created = repo
        .upsert_approval_grant(NewApprovalGrantRecord {
            scope_session_id: "root-session".to_owned(),
            approval_key: "tool:delegate_async".to_owned(),
            created_by_session_id: Some("operator-session".to_owned()),
        })
        .expect("upsert approval grant");
    assert_eq!(created.scope_session_id, "root-session");
    assert_eq!(created.approval_key, "tool:delegate_async");
    assert_eq!(
        created.created_by_session_id.as_deref(),
        Some("operator-session")
    );

    let loaded = repo
        .load_approval_grant("root-session", "tool:delegate_async")
        .expect("load approval grant")
        .expect("approval grant row");
    assert_eq!(loaded, created);
}

#[test]
fn session_tool_consent_repository_round_trips_root_mode() {
    let config = isolated_memory_config("session-tool-consent");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");

    let created = repo
        .upsert_session_tool_consent(NewSessionToolConsentRecord {
            scope_session_id: "root-session".to_owned(),
            mode: ToolConsentMode::Full,
            updated_by_session_id: Some("root-session".to_owned()),
        })
        .expect("upsert session tool consent");
    assert_eq!(created.scope_session_id, "root-session");
    assert_eq!(created.mode, ToolConsentMode::Full);
    assert_eq!(
        created.updated_by_session_id.as_deref(),
        Some("root-session")
    );

    let loaded = repo
        .load_session_tool_consent("root-session")
        .expect("load session tool consent")
        .expect("session tool consent row");
    assert_eq!(loaded, created);
}

#[test]
fn session_tool_consent_repository_normalizes_delegate_scope_to_root() {
    let config = isolated_memory_config("session-tool-consent-delegate-root");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child session");

    let created = repo
        .upsert_session_tool_consent(NewSessionToolConsentRecord {
            scope_session_id: "child-session".to_owned(),
            mode: ToolConsentMode::Auto,
            updated_by_session_id: Some("child-session".to_owned()),
        })
        .expect("upsert session tool consent");

    assert_eq!(created.scope_session_id, "root-session");
    assert_eq!(created.mode, ToolConsentMode::Auto);

    let loaded = repo
        .load_session_tool_consent("root-session")
        .expect("load session tool consent")
        .expect("session tool consent row");
    assert_eq!(loaded, created);

    let loaded_via_child = repo
        .load_session_tool_consent("child-session")
        .expect("load child session tool consent")
        .expect("child session tool consent row");
    assert_eq!(loaded_via_child, created);
}

#[test]
fn session_tool_policy_repository_round_trips_and_deletes_policy() {
    let config = isolated_memory_config("session-tool-policy");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");

    let created = repo
        .upsert_session_tool_policy(NewSessionToolPolicyRecord {
            session_id: "root-session".to_owned(),
            requested_tool_ids: vec![
                "read".to_owned(),
                "session_status".to_owned(),
                "read".to_owned(),
            ],
            runtime_narrowing: ToolRuntimeNarrowing {
                browser: crate::tools::runtime_config::BrowserRuntimeNarrowing {
                    max_sessions: Some(1),
                    ..crate::tools::runtime_config::BrowserRuntimeNarrowing::default()
                },
                web_fetch: crate::tools::runtime_config::WebFetchRuntimeNarrowing {
                    allow_private_hosts: Some(false),
                    enforce_allowed_domains: false,
                    allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
                    blocked_domains: BTreeSet::from(["deny.example.com".to_owned()]),
                    timeout_seconds: Some(5),
                    max_bytes: Some(4_096),
                    max_redirects: Some(2),
                },
            },
        })
        .expect("upsert session tool policy");

    assert_eq!(created.session_id, "root-session");
    assert_eq!(
        created.requested_tool_ids,
        vec!["read".to_owned(), "session_status".to_owned()]
    );
    assert_eq!(created.runtime_narrowing.browser.max_sessions, Some(1));
    assert_eq!(
        created.runtime_narrowing.web_fetch.allowed_domains,
        BTreeSet::from(["docs.example.com".to_owned()])
    );

    let loaded = repo
        .load_session_tool_policy("root-session")
        .expect("load session tool policy")
        .expect("session tool policy");
    assert_eq!(loaded, created);

    let deleted = repo
        .delete_session_tool_policy("root-session")
        .expect("delete session tool policy");
    assert!(deleted);
    assert!(
        repo.load_session_tool_policy("root-session")
            .expect("load session tool policy after delete")
            .is_none()
    );
}

#[test]
fn approval_request_repository_lists_requests_for_session_and_status() {
    let config = isolated_memory_config("approval-request-list");
    let repo = SessionRepository::new(&config).expect("repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child session");

    repo.ensure_approval_request(NewApprovalRequestRecord {
        approval_request_id: "apr-root-pending".to_owned(),
        session_id: "root-session".to_owned(),
        turn_id: "turn-root-pending".to_owned(),
        tool_call_id: "call-root-pending".to_owned(),
        tool_name: "delegate".to_owned(),
        approval_key: "tool:delegate".to_owned(),
        request_payload_json: json!({
            "tool_name": "delegate"
        }),
        governance_snapshot_json: json!({
            "rule_id": "root_pending"
        }),
    })
    .expect("persist root pending request");
    repo.ensure_approval_request(NewApprovalRequestRecord {
        approval_request_id: "apr-root-approved".to_owned(),
        session_id: "root-session".to_owned(),
        turn_id: "turn-root-approved".to_owned(),
        tool_call_id: "call-root-approved".to_owned(),
        tool_name: "delegate_async".to_owned(),
        approval_key: "tool:delegate_async".to_owned(),
        request_payload_json: json!({
            "tool_name": "delegate_async"
        }),
        governance_snapshot_json: json!({
            "rule_id": "root_approved"
        }),
    })
    .expect("persist root approved request");
    repo.transition_approval_request_if_current(
        "apr-root-approved",
        TransitionApprovalRequestIfCurrentRequest {
            expected_status: ApprovalRequestStatus::Pending,
            next_status: ApprovalRequestStatus::Approved,
            decision: Some(ApprovalDecision::ApproveAlways),
            resolved_by_session_id: Some("root-session".to_owned()),
            executed_at: None,
            last_error: None,
        },
    )
    .expect("transition root approved request")
    .expect("approved root request");
    repo.ensure_approval_request(NewApprovalRequestRecord {
        approval_request_id: "apr-child-pending".to_owned(),
        session_id: "child-session".to_owned(),
        turn_id: "turn-child-pending".to_owned(),
        tool_call_id: "call-child-pending".to_owned(),
        tool_name: "delegate".to_owned(),
        approval_key: "tool:delegate".to_owned(),
        request_payload_json: json!({
            "tool_name": "delegate"
        }),
        governance_snapshot_json: json!({
            "rule_id": "child_pending"
        }),
    })
    .expect("persist child pending request");

    let all_root_requests = repo
        .list_approval_requests_for_session("root-session", None)
        .expect("list root approval requests");
    assert_eq!(all_root_requests.len(), 2);
    let root_ids = all_root_requests
        .iter()
        .map(|record| record.approval_request_id.as_str())
        .collect::<Vec<_>>();
    assert!(root_ids.contains(&"apr-root-pending"));
    assert!(root_ids.contains(&"apr-root-approved"));

    let pending_root_requests = repo
        .list_approval_requests_for_session("root-session", Some(ApprovalRequestStatus::Pending))
        .expect("list pending root approval requests");
    assert_eq!(pending_root_requests.len(), 1);
    assert_eq!(
        pending_root_requests[0].approval_request_id,
        "apr-root-pending"
    );
}

#[test]
fn session_route_binding_upsert_and_reload_round_trip() {
    let config = isolated_memory_config("session-route-binding");
    let repo = SessionRepository::new(&config).expect("repository");

    let created = repo
        .upsert_session_route_binding("feishu:lark_cli_a1b2c3:oc_123", "im:session:1")
        .expect("create route binding");
    assert_eq!(created.route_session_id, "feishu:lark_cli_a1b2c3:oc_123");
    assert_eq!(created.active_session_id, "im:session:1");

    let updated = repo
        .upsert_session_route_binding("feishu:lark_cli_a1b2c3:oc_123", "im:session:2")
        .expect("update route binding");
    assert_eq!(updated.route_session_id, "feishu:lark_cli_a1b2c3:oc_123");
    assert_eq!(updated.active_session_id, "im:session:2");
    assert!(updated.updated_at >= updated.created_at);

    let loaded = repo
        .load_session_route_binding("feishu:lark_cli_a1b2c3:oc_123")
        .expect("load route binding")
        .expect("route binding exists");
    assert_eq!(loaded, updated);
}
