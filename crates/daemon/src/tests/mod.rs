#![allow(clippy::await_holding_lock, clippy::expect_used)]

use super::*;
use clap::Parser;
use std::path::PathBuf;

fn run_turn_env_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn run_turn_test_dir(test_name: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("loongclaw-run-turn-{test_name}-{unique}"));
    fs::create_dir_all(&dir).expect("create run-turn test dir");
    dir
}

fn spawn_openai_turn_server_once(reply_text: &str) -> (String, std::thread::JoinHandle<()>) {
    spawn_openai_turn_server_once_with_delay(reply_text, std::time::Duration::ZERO)
}

fn spawn_openai_turn_server_once_with_delay(
    reply_text: &str,
    response_delay: std::time::Duration,
) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider stub");
    let addr = listener.local_addr().expect("local provider addr");
    let reply_text = reply_text.to_owned();
    let server = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request_buf = [0_u8; 8192];
            let _ = stream.read(&mut request_buf);
            if !response_delay.is_zero() {
                std::thread::sleep(response_delay);
            }
            let body = serde_json::to_string(&json!({
                "choices": [{
                    "message": {
                        "content": reply_text
                    }
                }]
            }))
            .expect("serialize provider stub body");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://{addr}"), server)
}

fn write_run_turn_config(
    test_name: &str,
    base_url: &str,
) -> (PathBuf, mvp::memory::runtime_config::MemoryRuntimeConfig) {
    let dir = run_turn_test_dir(test_name);
    let config_path = dir.join("loongclaw.toml");
    let db_path = dir.join("memory.sqlite3");

    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.model = "stub-model".to_owned();
    config.provider.base_url = base_url.to_owned();
    config.provider.endpoint = Some(format!("{base_url}/v1/chat/completions"));
    config.provider.models_endpoint = Some(format!("{base_url}/v1/models"));
    config.provider.api_key = Some("test-api-key".to_owned());
    config.provider.api_key_env = None;
    config.provider.retry_max_attempts = 1;
    config.memory.sqlite_path = db_path.display().to_string();

    mvp::config::write(Some(config_path.to_string_lossy().as_ref()), &config, true)
        .expect("write run-turn config");

    (
        config_path,
        mvp::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(db_path),
        },
    )
}

fn approval_test_operation(tool_name: &str, payload: Value) -> OperationSpec {
    OperationSpec::ToolCore {
        tool_name: tool_name.to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload,
        core: None,
    }
}

fn write_temp_risk_profile(path: &Path, body: &str) {
    fs::create_dir_all(
        path.parent()
            .expect("temp risk profile path should have parent directory"),
    )
    .expect("create temp risk profile directory");
    fs::write(path, body).expect("write temp risk profile");
}

fn sign_security_scan_profile_for_test(profile: &SecurityScanProfile) -> (String, String) {
    use ed25519_dalek::{Signer, SigningKey};

    let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
    let signature = signing_key.sign(&security_scan_profile_message(profile));
    let public_key_base64 = BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes());
    let signature_base64 = BASE64_STANDARD.encode(signature.to_bytes());
    (public_key_base64, signature_base64)
}

mod architecture;
mod onboard_cli;
mod programmatic;
mod spec_runtime;
mod spec_runtime_bridge;

#[test]
fn resolve_validate_output_defaults_to_text() {
    let resolved = resolve_validate_output(false, None).expect("resolve default output");
    assert_eq!(resolved, ValidateConfigOutput::Text);
}

#[test]
fn resolve_validate_output_uses_json_flag_legacy_alias() {
    let resolved = resolve_validate_output(true, None).expect("resolve json output");
    assert_eq!(resolved, ValidateConfigOutput::Json);
}

#[test]
fn resolve_validate_output_accepts_explicit_problem_json() {
    let resolved = resolve_validate_output(false, Some(ValidateConfigOutput::ProblemJson))
        .expect("resolve problem-json output");
    assert_eq!(resolved, ValidateConfigOutput::ProblemJson);
}

#[test]
fn resolve_validate_output_rejects_conflicting_json_and_output_flags() {
    let error = resolve_validate_output(true, Some(ValidateConfigOutput::Json))
        .expect_err("conflicting flags should fail");
    assert!(error.contains("conflicts"));
}

#[test]
fn run_turn_command_parses_delegate_child_and_timeout() {
    let cli = Cli::try_parse_from([
        "loongclawd",
        "run-turn",
        "--config",
        "/tmp/worker.toml",
        "--session",
        "delegate:child-1",
        "--input",
        "child task",
        "--timeout-seconds",
        "17",
        "--delegate-child",
    ])
    .expect("parse run-turn command");

    match cli.command.expect("command") {
        Commands::RunTurn {
            config,
            session,
            input,
            timeout_seconds,
            delegate_child,
        } => {
            assert_eq!(config.as_deref(), Some("/tmp/worker.toml"));
            assert_eq!(session, "delegate:child-1");
            assert_eq!(input, "child task");
            assert_eq!(timeout_seconds, Some(17));
            assert!(delegate_child);
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn resolve_run_turn_config_path_prefers_explicit_arg_over_env() {
    let _guard = run_turn_env_lock().lock().expect("env lock");
    std::env::set_var("LOONGCLAW_CONFIG_PATH", "/tmp/from-env.toml");

    let resolved = resolve_run_turn_config_path(Some("/tmp/from-arg.toml"));

    assert_eq!(resolved.as_deref(), Some("/tmp/from-arg.toml"));
    std::env::remove_var("LOONGCLAW_CONFIG_PATH");
}

#[test]
fn resolve_run_turn_config_path_uses_env_when_arg_missing() {
    let _guard = run_turn_env_lock().lock().expect("env lock");
    std::env::set_var("LOONGCLAW_CONFIG_PATH", "/tmp/from-env.toml");

    let resolved = resolve_run_turn_config_path(None);

    assert_eq!(resolved.as_deref(), Some("/tmp/from-env.toml"));
    std::env::remove_var("LOONGCLAW_CONFIG_PATH");
}

#[tokio::test]
async fn execute_run_turn_returns_root_reply_and_persists_turns() {
    let _guard = run_turn_env_lock().lock().expect("env lock");
    let (base_url, server) = spawn_openai_turn_server_once("Root daemon reply");
    let (config_path, memory_config) = write_run_turn_config("root", &base_url);

    let result = execute_run_turn(
        Some(config_path.to_string_lossy().as_ref()),
        "root-session",
        "hello from root",
        None,
        false,
    )
    .await
    .expect("execute run-turn root");

    server.join().expect("join provider stub");

    match result {
        RunTurnResult::Reply(reply) => assert_eq!(reply, "Root daemon reply"),
        other => panic!("unexpected run-turn result: {other:?}"),
    }

    let turns = mvp::memory::window_direct("root-session", 10, &memory_config)
        .expect("load root session turns");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].role, "user");
    assert_eq!(turns[0].content, "hello from root");
    assert_eq!(turns[1].role, "assistant");
    assert_eq!(turns[1].content, "Root daemon reply");

    let repo = mvp::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    let root = repo
        .load_session("root-session")
        .expect("load root session")
        .expect("root session row");
    assert_eq!(root.kind, mvp::session::repository::SessionKind::Root);
}

#[tokio::test]
async fn execute_run_turn_uses_current_memory_runtime_for_each_config() {
    let _guard = run_turn_env_lock().lock().expect("env lock");
    let (base_url_a, server_a) = spawn_openai_turn_server_once("First config reply");
    let (config_path_a, memory_config_a) = write_run_turn_config("root-config-a", &base_url_a);

    let result_a = execute_run_turn(
        Some(config_path_a.to_string_lossy().as_ref()),
        "root-session-a",
        "hello from config a",
        None,
        false,
    )
    .await
    .expect("execute run-turn root config a");

    server_a.join().expect("join provider stub a");

    match result_a {
        RunTurnResult::Reply(reply) => assert_eq!(reply, "First config reply"),
        other => panic!("unexpected first run-turn result: {other:?}"),
    }

    let (base_url_b, server_b) = spawn_openai_turn_server_once("Second config reply");
    let (config_path_b, memory_config_b) = write_run_turn_config("root-config-b", &base_url_b);

    let result_b = execute_run_turn(
        Some(config_path_b.to_string_lossy().as_ref()),
        "root-session-b",
        "hello from config b",
        None,
        false,
    )
    .await
    .expect("execute run-turn root config b");

    server_b.join().expect("join provider stub b");

    match result_b {
        RunTurnResult::Reply(reply) => assert_eq!(reply, "Second config reply"),
        other => panic!("unexpected second run-turn result: {other:?}"),
    }

    let turns_a = mvp::memory::window_direct("root-session-a", 10, &memory_config_a)
        .expect("load root session turns for config a");
    assert_eq!(turns_a.len(), 2);
    assert_eq!(turns_a[0].content, "hello from config a");
    assert_eq!(turns_a[1].content, "First config reply");

    let turns_b = mvp::memory::window_direct("root-session-b", 10, &memory_config_b)
        .expect("load root session turns for config b");
    assert_eq!(turns_b.len(), 2);
    assert_eq!(turns_b[0].content, "hello from config b");
    assert_eq!(turns_b[1].content, "Second config reply");
}

#[tokio::test]
async fn execute_run_turn_completes_delegate_child_session() {
    let _guard = run_turn_env_lock().lock().expect("env lock");
    let (base_url, server) = spawn_openai_turn_server_once("Child daemon reply");
    let (config_path, memory_config) = write_run_turn_config("delegate-child", &base_url);
    let repo = mvp::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: None,
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("async-child".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create child session");

    let result = execute_run_turn(
        Some(config_path.to_string_lossy().as_ref()),
        "child-session",
        "child task",
        Some(5),
        true,
    )
    .await
    .expect("execute run-turn delegate child");

    server.join().expect("join provider stub");

    match result {
        RunTurnResult::DelegateOutcome(outcome) => {
            assert_eq!(outcome.status, "ok");
            assert_eq!(outcome.payload["child_session_id"], "child-session");
            assert_eq!(outcome.payload["final_output"], "Child daemon reply");
        }
        other => panic!("unexpected run-turn result: {other:?}"),
    }

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(
        child.state,
        mvp::session::repository::SessionState::Completed
    );

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list child events");
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert!(event_kinds.contains(&"delegate_started"));
    assert!(event_kinds.contains(&"delegate_completed"));

    let terminal_outcome = repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(terminal_outcome.status, "ok");
    assert_eq!(
        terminal_outcome.payload_json["final_output"],
        "Child daemon reply"
    );
}

#[tokio::test]
async fn execute_run_turn_fails_delegate_child_session_on_provider_error() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let _guard = run_turn_env_lock().lock().expect("env lock");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider stub");
    let addr = listener.local_addr().expect("local provider addr");
    let server = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request_buf = [0_u8; 8192];
            let _ = stream.read(&mut request_buf);
            let body = serde_json::to_string(&json!({
                "error": {
                    "message": "stub failure"
                }
            }))
            .expect("serialize provider stub error body");
            let response = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let (config_path, memory_config) =
        write_run_turn_config("delegate-child-failure", &format!("http://{addr}"));
    let repo = mvp::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: None,
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("async-child".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create child session");

    let result = execute_run_turn(
        Some(config_path.to_string_lossy().as_ref()),
        "child-session",
        "failing child task",
        Some(5),
        true,
    )
    .await
    .expect("execute run-turn delegate child failure");

    server.join().expect("join provider stub");

    match result {
        RunTurnResult::DelegateOutcome(outcome) => {
            assert_eq!(outcome.status, "error");
            assert_eq!(outcome.payload["child_session_id"], "child-session");
            let error = outcome.payload["error"]
                .as_str()
                .expect("delegate failure error string");
            assert!(
                error.contains("provider returned status 500"),
                "error: {error}"
            );
        }
        other => panic!("unexpected run-turn result: {other:?}"),
    }

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(child.state, mvp::session::repository::SessionState::Failed);
    assert!(
        child
            .last_error
            .as_deref()
            .expect("child last_error")
            .contains("provider returned status 500"),
        "last_error: {:?}",
        child.last_error
    );

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list child events");
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert!(event_kinds.contains(&"delegate_started"));
    assert!(event_kinds.contains(&"delegate_failed"));

    let terminal_outcome = repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(terminal_outcome.status, "error");
    assert!(
        terminal_outcome.payload_json["error"]
            .as_str()
            .expect("terminal outcome error")
            .contains("provider returned status 500"),
        "terminal_error: {}",
        terminal_outcome.payload_json["error"]
    );
}

#[tokio::test]
async fn execute_run_turn_times_out_delegate_child_session() {
    let _guard = run_turn_env_lock().lock().expect("env lock");
    let (base_url, server) = spawn_openai_turn_server_once_with_delay(
        "Too slow",
        std::time::Duration::from_millis(1_200),
    );
    let (config_path, memory_config) = write_run_turn_config("delegate-child-timeout", &base_url);
    let repo = mvp::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: mvp::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: None,
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(mvp::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: mvp::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("async-child".to_owned()),
        state: mvp::session::repository::SessionState::Ready,
    })
    .expect("create child session");

    let result = execute_run_turn(
        Some(config_path.to_string_lossy().as_ref()),
        "child-session",
        "slow child task",
        Some(1),
        true,
    )
    .await
    .expect("execute run-turn delegate child timeout");

    server.join().expect("join provider stub");

    match result {
        RunTurnResult::DelegateOutcome(outcome) => {
            assert_eq!(outcome.status, "timeout");
            assert_eq!(outcome.payload["child_session_id"], "child-session");
            assert_eq!(outcome.payload["error"], "delegate_timeout");
        }
        other => panic!("unexpected run-turn result: {other:?}"),
    }

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(
        child.state,
        mvp::session::repository::SessionState::TimedOut
    );
    assert_eq!(child.last_error.as_deref(), Some("delegate_timeout"));

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list child events");
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert!(event_kinds.contains(&"delegate_started"));
    assert!(event_kinds.contains(&"delegate_timed_out"));

    let terminal_outcome = repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(terminal_outcome.status, "timeout");
    assert_eq!(terminal_outcome.payload_json["error"], "delegate_timeout");
}
