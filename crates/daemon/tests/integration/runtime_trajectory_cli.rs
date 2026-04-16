use super::*;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}

fn write_runtime_trajectory_config(root: &Path) -> PathBuf {
    fs::create_dir_all(root).expect("create fixture root");

    let mut config = mvp::config::LoongClawConfig::default();
    let sqlite_path = root.join("memory.sqlite3");
    config.memory.sqlite_path = sqlite_path.display().to_string();
    config.tools.file_root = Some(root.display().to_string());

    let config_path = root.join("loongclaw.toml");
    let config_path_text = config_path.to_string_lossy();
    mvp::config::write(Some(config_path_text.as_ref()), &config, true)
        .expect("write config fixture");

    config_path
}

fn seed_runtime_trajectory_session(config_path: &Path, session_id: &str) {
    let config_path_text = config_path.to_string_lossy();
    let (_, config) =
        mvp::config::load(Some(config_path_text.as_ref())).expect("load config fixture");
    let memory_config =
        mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repository =
        mvp::session::repository::SessionRepository::new(&memory_config).expect("repository");

    let session_record = mvp::session::repository::NewSessionRecord {
        session_id: session_id.to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: mvp::session::repository::SessionState::Running,
    };
    repository
        .create_session(session_record)
        .expect("create root session");

    let turn_contents = ["step one", "step two", "step three"];
    for turn_content in turn_contents {
        mvp::memory::append_turn_direct(session_id, "assistant", turn_content, &memory_config)
            .expect("append turn");
    }

    let event_payload = serde_json::json!({
        "task": "summarize"
    });
    let session_event = mvp::session::repository::NewSessionEvent {
        session_id: session_id.to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("operator".to_owned()),
        payload_json: event_payload,
    };
    repository
        .append_event(session_event)
        .expect("append session event");

    let approval_payload = serde_json::json!({
        "tool_name": "delegate"
    });
    let governance_payload = serde_json::json!({
        "rule_id": "delegate_review"
    });
    let approval_request = mvp::session::repository::NewApprovalRequestRecord {
        approval_request_id: "approval-1".to_owned(),
        session_id: session_id.to_owned(),
        turn_id: "turn-1".to_owned(),
        tool_call_id: "tool-call-1".to_owned(),
        tool_name: "delegate".to_owned(),
        approval_key: "tool:delegate".to_owned(),
        request_payload_json: approval_payload,
        governance_snapshot_json: governance_payload,
    };
    repository
        .ensure_approval_request(approval_request)
        .expect("ensure approval request");

    let terminal_event_payload = serde_json::json!({
        "task": "summarize"
    });
    let terminal_outcome_payload = serde_json::json!({
        "summary": "done"
    });
    let finalize_request = mvp::session::repository::FinalizeSessionTerminalRequest {
        state: mvp::session::repository::SessionState::Completed,
        last_error: None,
        event_kind: "delegate_completed".to_owned(),
        actor_session_id: Some("operator".to_owned()),
        event_payload_json: terminal_event_payload,
        outcome_status: "ok".to_owned(),
        outcome_payload_json: terminal_outcome_payload,
        frozen_result: None,
    };
    repository
        .finalize_session_terminal(session_id, finalize_request)
        .expect("finalize session");
}

#[test]
fn runtime_trajectory_export_writes_bounded_artifact_with_lineage_and_canonical_records() {
    let root = unique_temp_dir("loongclaw-runtime-trajectory-export");
    let config_path = write_runtime_trajectory_config(root.as_path());
    seed_runtime_trajectory_session(config_path.as_path(), "root-session");

    let artifact_path = root.join("artifacts").join("root-session.json");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_loong"))
        .args([
            "runtime-trajectory",
            "export",
            "--config",
            config_path.to_str().expect("config path should be utf-8"),
            "--session",
            "root-session",
            "--output",
            artifact_path
                .to_str()
                .expect("artifact path should be utf-8"),
            "--turn-limit",
            "2",
            "--json",
        ])
        .output()
        .expect("run runtime-trajectory export");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "export should succeed: {stderr}");

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    let artifact = serde_json::from_str::<Value>(&stdout).expect("decode export json");

    assert_eq!(artifact["session"]["session_id"], "root-session");
    assert_eq!(artifact["lineage"]["root_session_id"], "root-session");
    assert_eq!(artifact["lineage"]["depth"], 0);
    assert_eq!(artifact["exported_turn_count"], 2);
    assert_eq!(artifact["turns_truncated"], true);
    assert_eq!(artifact["canonical_record_count"], 2);
    assert_eq!(artifact["event_count"], 2);
    assert_eq!(artifact["approval_request_count"], 1);
    assert_eq!(artifact["turns"][0]["sequence"], 2);
    assert_eq!(artifact["turns"][1]["sequence"], 3);
    assert_eq!(artifact["canonical_records"][0]["kind"], "assistant_turn");
    assert_eq!(artifact["terminal_outcome"]["status"], "ok");

    let persisted = fs::read_to_string(&artifact_path).expect("read persisted artifact");
    let persisted_artifact = serde_json::from_str::<Value>(&persisted).expect("decode persisted");
    assert_eq!(persisted_artifact, artifact);
}

#[test]
fn runtime_trajectory_show_round_trips_persisted_artifact_in_text_mode() {
    let root = unique_temp_dir("loongclaw-runtime-trajectory-show");
    let config_path = write_runtime_trajectory_config(root.as_path());
    seed_runtime_trajectory_session(config_path.as_path(), "root-session");

    let artifact_path = root.join("artifacts").join("root-session.json");
    let export_status = std::process::Command::new(env!("CARGO_BIN_EXE_loong"))
        .args([
            "runtime-trajectory",
            "export",
            "--config",
            config_path.to_str().expect("config path should be utf-8"),
            "--session",
            "root-session",
            "--output",
            artifact_path
                .to_str()
                .expect("artifact path should be utf-8"),
        ])
        .status()
        .expect("run runtime-trajectory export");

    assert!(export_status.success(), "export should succeed");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_loong"))
        .args([
            "runtime-trajectory",
            "show",
            "--artifact",
            artifact_path
                .to_str()
                .expect("artifact path should be utf-8"),
        ])
        .output()
        .expect("run runtime-trajectory show");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "show should succeed: {stderr}");

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(
        stdout.lines().any(|line| {
            line.starts_with("artifact_path=")
                || line.starts_with("LOONGCLAW")
                || line.contains(" loongclaw ")
        }),
        "runtime trajectory show should print the wrapped operator surface (optionally after artifact_path): {stdout}"
    );
    assert!(stdout.contains("runtime trajectory"));
    assert!(stdout.contains("runtime_trajectory session=root-session"));
    assert!(stdout.contains("lineage_root=root-session"));
    assert!(stdout.contains("canonical_records=3"));
    assert!(stdout.contains("approvals=1"));
    assert!(stdout.contains("terminal_status=ok"));
}

#[test]
fn runtime_trajectory_export_accepts_bare_output_file_in_current_directory() {
    let root = unique_temp_dir("loongclaw-runtime-trajectory-bare-output");
    let config_path = write_runtime_trajectory_config(root.as_path());
    seed_runtime_trajectory_session(config_path.as_path(), "root-session");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_loong"))
        .current_dir(root.as_path())
        .args([
            "runtime-trajectory",
            "export",
            "--config",
            config_path.to_str().expect("config path should be utf-8"),
            "--session",
            "root-session",
            "--output",
            "trajectory.json",
        ])
        .output()
        .expect("run runtime-trajectory export with bare output path");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "export should succeed: {stderr}");

    let artifact_path = root.join("trajectory.json");
    let persisted = fs::read_to_string(artifact_path).expect("read persisted artifact");
    let persisted_artifact = serde_json::from_str::<Value>(&persisted).expect("decode artifact");

    assert_eq!(persisted_artifact["session"]["session_id"], "root-session");
}
