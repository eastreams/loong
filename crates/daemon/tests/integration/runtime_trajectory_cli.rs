#![allow(
    clippy::disallowed_methods,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks
)]

use super::*;
use serde_json::{Value, json};
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
    let temp_dir = std::env::temp_dir();
    let canonical_temp_dir = dunce::canonicalize(&temp_dir).unwrap_or(temp_dir);
    canonical_temp_dir.join(format!("{prefix}-{nanos}"))
}

fn write_runtime_trajectory_config(root: &Path) -> PathBuf {
    fs::create_dir_all(root).expect("create fixture root");

    let mut config = mvp::config::LoongClawConfig::default();
    let sqlite_path = root.join("memory.sqlite3");
    let sqlite_path_text = sqlite_path.display().to_string();
    config.memory.sqlite_path = sqlite_path_text;

    let config_path = root.join("loongclaw.toml");
    let config_path_text = config_path.to_string_lossy().to_string();
    mvp::config::write(Some(config_path_text.as_str()), &config, true)
        .expect("write config fixture");
    config_path
}

fn load_memory_runtime_config(
    config_path: &Path,
) -> mvp::memory::runtime_config::MemoryRuntimeConfig {
    let config_path_text = config_path
        .to_str()
        .expect("config path should be valid utf-8");
    let (_, config) = mvp::config::load(Some(config_path_text)).expect("load config fixture");
    mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory)
}

fn append_structured_conversation_event_turn(
    session_id: &str,
    event_name: &str,
    payload: Value,
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
) {
    let content = json!({
        "_loongclaw_internal": true,
        "type": "conversation_event",
        "event": event_name,
        "payload": payload,
    })
    .to_string();
    mvp::memory::append_turn_direct(session_id, "assistant", &content, memory_config)
        .expect("append structured conversation event turn");
}

#[test]
fn runtime_trajectory_export_session_only_keeps_requested_session_and_true_root_metadata() {
    let root = unique_temp_dir("runtime-trajectory-session-only");
    let config_path = write_runtime_trajectory_config(&root);
    let memory_config = load_memory_runtime_config(&config_path);
    let repo = mvp::session::repository::SessionRepository::new(&memory_config).expect("repo");

    repo.ensure_session(mvp::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("ensure root session");
    repo.ensure_session(mvp::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: mvp::session::repository::SessionState::Running,
    })
    .expect("ensure child session");
    mvp::memory::append_turn_direct("root-session", "user", "root turn", &memory_config)
        .expect("append root turn");
    mvp::memory::append_turn_direct("child-session", "assistant", "child turn", &memory_config)
        .expect("append child turn");

    let artifact =
        loongclaw_daemon::runtime_trajectory_cli::execute_runtime_trajectory_export_command(
            loongclaw_daemon::runtime_trajectory_cli::RuntimeTrajectoryExportCommandOptions {
                config: Some(config_path.display().to_string()),
                session: "child-session".to_owned(),
                output: None,
                lineage: false,
                json: false,
            },
        )
        .expect("session-only export should succeed");

    assert_eq!(artifact.requested_session_id, "child-session");
    assert_eq!(artifact.root_session_id, "root-session");
    assert_eq!(
        artifact.export_mode,
        loongclaw_daemon::runtime_trajectory_cli::RuntimeTrajectoryExportMode::SessionOnly
    );
    assert_eq!(artifact.sessions.len(), 1);
    assert_eq!(artifact.sessions[0].summary.session_id, "child-session");
}

#[test]
fn runtime_trajectory_export_lineage_includes_events_terminal_outcomes_and_approval_requests() {
    let root = unique_temp_dir("runtime-trajectory-lineage");
    let config_path = write_runtime_trajectory_config(&root);
    let memory_config = load_memory_runtime_config(&config_path);
    let repo = mvp::session::repository::SessionRepository::new(&memory_config).expect("repo");

    repo.ensure_session(mvp::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("ensure root session");
    repo.ensure_session(mvp::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: mvp::session::repository::SessionState::Completed,
    })
    .expect("ensure child session");

    mvp::memory::append_turn_direct("root-session", "user", "root turn", &memory_config)
        .expect("append root turn");
    append_structured_conversation_event_turn(
        "root-session",
        "delegate_completed",
        json!({
            "child_session_id": "child-session",
        }),
        &memory_config,
    );
    append_structured_conversation_event_turn(
        "root-session",
        "fast_lane_tool_batch",
        json!({
            "intent_outcomes": [
                {
                    "tool_call_id": "call-1",
                    "tool_name": "delegate_async",
                    "status": "needs_approval",
                    "detail": "approval required"
                },
                {
                    "tool_call_id": "call-2",
                    "tool_name": "file.read",
                    "status": "completed",
                    "detail": null
                }
            ]
        }),
        &memory_config,
    );
    mvp::memory::append_turn_direct("child-session", "assistant", "child turn", &memory_config)
        .expect("append child turn");
    repo.append_event(mvp::session::repository::NewSessionEvent {
        session_id: "root-session".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "child_session_id": "child-session",
        }),
    })
    .expect("append session event");
    repo.upsert_session_terminal_outcome("child-session", "completed", json!({"ok": true}))
        .expect("store terminal outcome");
    repo.ensure_approval_request(mvp::session::repository::NewApprovalRequestRecord {
        approval_request_id: "apr-runtime-trajectory".to_owned(),
        session_id: "child-session".to_owned(),
        turn_id: "turn-1".to_owned(),
        tool_call_id: "call-1".to_owned(),
        tool_name: "delegate_async".to_owned(),
        approval_key: "tool:delegate_async".to_owned(),
        request_payload_json: json!({
            "tool_name": "delegate_async",
            "args_json": {
                "task": "research"
            },
        }),
        governance_snapshot_json: json!({
            "reason": "governed_tool_requires_approval",
        }),
    })
    .expect("store approval request");

    let artifact =
        loongclaw_daemon::runtime_trajectory_cli::execute_runtime_trajectory_export_command(
            loongclaw_daemon::runtime_trajectory_cli::RuntimeTrajectoryExportCommandOptions {
                config: Some(config_path.display().to_string()),
                session: "child-session".to_owned(),
                output: None,
                lineage: true,
                json: false,
            },
        )
        .expect("lineage export should succeed");

    assert_eq!(artifact.root_session_id, "root-session");
    assert_eq!(artifact.sessions.len(), 2);
    assert_eq!(artifact.sessions[0].summary.session_id, "root-session");
    assert_eq!(artifact.sessions[1].summary.session_id, "child-session");
    assert_eq!(artifact.statistics.session_count, 2);
    assert_eq!(artifact.statistics.turn_count, 4);
    assert_eq!(artifact.statistics.session_event_count, 1);
    assert_eq!(artifact.statistics.approval_request_count, 1);
    assert_eq!(
        artifact.statistics.canonical_kind_counts["conversation_event"],
        2
    );
    assert_eq!(
        artifact.statistics.conversation_event_name_counts["delegate_completed"],
        1
    );
    assert_eq!(
        artifact.statistics.conversation_event_name_counts["fast_lane_tool_batch"],
        1
    );
    assert_eq!(
        artifact.statistics.tool_intent_status_counts["completed"],
        1
    );
    assert_eq!(
        artifact.statistics.tool_intent_status_counts["needs_approval"],
        1
    );
    assert!(artifact.sessions[1].terminal_outcome.is_some());
    assert_eq!(artifact.sessions[1].approval_requests.len(), 1);
}

#[test]
fn runtime_trajectory_show_round_trips_exported_artifact() {
    let root = unique_temp_dir("runtime-trajectory-show");
    let config_path = write_runtime_trajectory_config(&root);
    let memory_config = load_memory_runtime_config(&config_path);
    let repo = mvp::session::repository::SessionRepository::new(&memory_config).expect("repo");

    repo.ensure_session(mvp::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("ensure root session");
    mvp::memory::append_turn_direct("root-session", "user", "root turn", &memory_config)
        .expect("append root turn");

    let artifact_path = root.join("artifacts/runtime-trajectory.json");
    let exported =
        loongclaw_daemon::runtime_trajectory_cli::execute_runtime_trajectory_export_command(
            loongclaw_daemon::runtime_trajectory_cli::RuntimeTrajectoryExportCommandOptions {
                config: Some(config_path.display().to_string()),
                session: "root-session".to_owned(),
                output: Some(artifact_path.display().to_string()),
                lineage: false,
                json: false,
            },
        )
        .expect("export artifact");
    let shown = loongclaw_daemon::runtime_trajectory_cli::execute_runtime_trajectory_show_command(
        loongclaw_daemon::runtime_trajectory_cli::RuntimeTrajectoryShowCommandOptions {
            artifact: artifact_path.display().to_string(),
            json: false,
        },
    )
    .expect("show artifact");

    assert_eq!(shown.requested_session_id, "root-session");
    assert_eq!(shown.statistics.turn_count, exported.statistics.turn_count);
    assert_eq!(
        shown.statistics.tool_intent_status_counts,
        exported.statistics.tool_intent_status_counts
    );
    assert_eq!(shown.sessions[0].summary.session_id, "root-session");
}

#[test]
fn runtime_trajectory_render_text_surfaces_rollups() {
    let root = unique_temp_dir("runtime-trajectory-render");
    let config_path = write_runtime_trajectory_config(&root);
    let memory_config = load_memory_runtime_config(&config_path);
    let repo = mvp::session::repository::SessionRepository::new(&memory_config).expect("repo");

    repo.ensure_session(mvp::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("ensure root session");
    mvp::memory::append_turn_direct("root-session", "user", "root turn", &memory_config)
        .expect("append root turn");
    append_structured_conversation_event_turn(
        "root-session",
        "fast_lane_tool_batch",
        json!({
            "intent_outcomes": [
                {
                    "tool_call_id": "call-1",
                    "tool_name": "delegate_async",
                    "status": "needs_approval",
                    "detail": "approval required"
                }
            ]
        }),
        &memory_config,
    );

    let artifact =
        loongclaw_daemon::runtime_trajectory_cli::execute_runtime_trajectory_export_command(
            loongclaw_daemon::runtime_trajectory_cli::RuntimeTrajectoryExportCommandOptions {
                config: Some(config_path.display().to_string()),
                session: "root-session".to_owned(),
                output: None,
                lineage: false,
                json: false,
            },
        )
        .expect("render export should succeed");

    let rendered =
        loongclaw_daemon::runtime_trajectory_cli::render_runtime_trajectory_text(&artifact);

    assert!(
        rendered.contains("runtime trajectory export requested_session_id=root-session"),
        "rendered text should start with the export headline: {rendered}"
    );
    assert!(
        rendered.contains("canonical_kind_counts=conversation_event=1,user_turn=1"),
        "rendered text should include canonical kind rollups: {rendered}"
    );
    assert!(
        rendered.contains("conversation_event_name_counts=fast_lane_tool_batch=1"),
        "rendered text should include conversation event rollups: {rendered}"
    );
    assert!(
        rendered.contains("tool_intent_status_counts=needs_approval=1"),
        "rendered text should include tool intent status rollups: {rendered}"
    );
}
