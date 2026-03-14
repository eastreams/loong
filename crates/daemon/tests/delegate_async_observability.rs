#![allow(
    clippy::await_holding_lock,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use async_trait::async_trait;
use loongclaw_app as mvp;
use loongclaw_contracts::ToolCoreRequest;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{oneshot, Notify};

use mvp::conversation::turn_engine::{AsyncDelegateSpawnRequest, AsyncDelegateSpawner};
use mvp::conversation::{AppToolDispatcher, DefaultAppToolDispatcher, SessionContext};

fn integration_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn integration_test_dir(test_name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("loongclaw-daemon-integ-{test_name}-{unique}"));
    fs::create_dir_all(&dir).expect("create integration test dir");
    dir
}

fn spawn_openai_turn_server_once_with_delay(
    reply_text: &str,
    response_delay: Duration,
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

fn spawn_openai_turn_server_once_with_response_gate(
    reply_text: &str,
) -> (
    String,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    std::thread::JoinHandle<()>,
) {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider stub");
    let addr = listener.local_addr().expect("local provider addr");
    let reply_text = reply_text.to_owned();
    let (request_seen_tx, request_seen_rx) = oneshot::channel();
    let (release_response_tx, release_response_rx) = oneshot::channel();
    let server = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request_buf = [0_u8; 8192];
            let _ = stream.read(&mut request_buf);
            let _ = request_seen_tx.send(());
            release_response_rx
                .blocking_recv()
                .expect("release provider stub response");
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
    (
        format!("http://{addr}"),
        request_seen_rx,
        release_response_tx,
        server,
    )
}

fn spawn_openai_error_server_once(
    status_code: u16,
    body: Value,
) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider stub");
    let addr = listener.local_addr().expect("local provider addr");
    let server = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request_buf = [0_u8; 8192];
            let _ = stream.read(&mut request_buf);
            let body = serde_json::to_string(&body).expect("serialize provider stub error body");
            let response = format!(
                "HTTP/1.1 {status_code} Stub Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://{addr}"), server)
}

fn spawn_openai_error_server_once_with_response_gate(
    status_code: u16,
    body: Value,
) -> (
    String,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    std::thread::JoinHandle<()>,
) {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider stub");
    let addr = listener.local_addr().expect("local provider addr");
    let (request_seen_tx, request_seen_rx) = oneshot::channel();
    let (release_response_tx, release_response_rx) = oneshot::channel();
    let server = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request_buf = [0_u8; 8192];
            let _ = stream.read(&mut request_buf);
            let _ = request_seen_tx.send(());
            release_response_rx
                .blocking_recv()
                .expect("release provider stub error response");
            let body = serde_json::to_string(&body).expect("serialize provider stub error body");
            let response = format!(
                "HTTP/1.1 {status_code} Stub Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (
        format!("http://{addr}"),
        request_seen_rx,
        release_response_tx,
        server,
    )
}

fn write_delegate_async_config(
    test_name: &str,
    base_url: &str,
) -> (
    PathBuf,
    mvp::config::LoongClawConfig,
    mvp::memory::runtime_config::MemoryRuntimeConfig,
) {
    let dir = integration_test_dir(test_name);
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
        .expect("write integration config");

    let memory_config = mvp::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    };
    (config_path, config, memory_config)
}

fn resolve_daemon_binary_path() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_loongclawd") {
        let resolved = PathBuf::from(path);
        if resolved.is_file() {
            return resolved;
        }
    }

    let current_exe = std::env::current_exe().expect("resolve current integration test executable");
    let target_debug_dir = current_exe
        .parent()
        .and_then(|path| path.parent())
        .expect("integration test executable should live under target/debug/deps");
    let fallback = target_debug_dir.join("loongclawd");
    assert!(
        fallback.is_file(),
        "daemon binary not found via env or fallback path: {}",
        fallback.display()
    );
    fallback
}

#[derive(Debug, Clone)]
struct BinaryAsyncDelegateSpawner {
    daemon_bin: PathBuf,
    config_path: PathBuf,
}

#[async_trait]
impl AsyncDelegateSpawner for BinaryAsyncDelegateSpawner {
    async fn spawn(&self, request: AsyncDelegateSpawnRequest) -> Result<(), String> {
        let mut command = tokio::process::Command::new(&self.daemon_bin);
        command.args([
            "run-turn",
            "--config",
            self.config_path
                .to_str()
                .ok_or_else(|| "non-utf8 config path".to_owned())?,
            "--session",
            &request.child_session_id,
            "--input",
            &request.task,
            "--timeout-seconds",
            &request.timeout_seconds.to_string(),
            "--delegate-child",
        ]);
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        let child = command
            .spawn()
            .map_err(|error| format!("spawn daemon delegate subprocess failed: {error}"))?;
        drop(child);
        Ok(())
    }
}

struct GatedAsyncDelegateSpawner {
    inner: BinaryAsyncDelegateSpawner,
    request_tx: Mutex<Option<oneshot::Sender<AsyncDelegateSpawnRequest>>>,
    launch_notify: Arc<Notify>,
}

impl GatedAsyncDelegateSpawner {
    fn new(
        inner: BinaryAsyncDelegateSpawner,
    ) -> (
        Self,
        oneshot::Receiver<AsyncDelegateSpawnRequest>,
        Arc<Notify>,
    ) {
        let (request_tx, request_rx) = oneshot::channel();
        let launch_notify = Arc::new(Notify::new());
        (
            Self {
                inner,
                request_tx: Mutex::new(Some(request_tx)),
                launch_notify: launch_notify.clone(),
            },
            request_rx,
            launch_notify,
        )
    }
}

#[async_trait]
impl AsyncDelegateSpawner for GatedAsyncDelegateSpawner {
    async fn spawn(&self, request: AsyncDelegateSpawnRequest) -> Result<(), String> {
        let request_tx = self
            .request_tx
            .lock()
            .expect("gated request sender lock")
            .take()
            .ok_or_else(|| "gated async delegate spawn request already captured".to_owned())?;
        request_tx
            .send(request.clone())
            .map_err(|_send_error| "gated async delegate spawn receiver dropped".to_owned())?;
        self.launch_notify.notified().await;
        self.inner.spawn(request).await
    }
}

async fn execute_root_tool(
    dispatcher: &DefaultAppToolDispatcher,
    session_context: &SessionContext,
    tool_name: &str,
    payload: Value,
) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
    dispatcher
        .execute_app_tool(
            session_context,
            ToolCoreRequest {
                tool_name: tool_name.to_owned(),
                payload,
            },
            None,
        )
        .await
}

async fn wait_for_session_event(
    dispatcher: &DefaultAppToolDispatcher,
    session_context: &SessionContext,
    child_session_id: &str,
    expected_event_kind: &str,
    timeout: Duration,
) -> loongclaw_contracts::ToolCoreOutcome {
    let started = tokio::time::Instant::now();
    loop {
        let outcome = execute_root_tool(
            dispatcher,
            session_context,
            "session_events",
            json!({
                "session_id": child_session_id,
                "limit": 10
            }),
        )
        .await
        .expect("session_events outcome");
        let found = outcome.payload["events"]
            .as_array()
            .expect("events array")
            .iter()
            .any(|event| event["event_kind"] == expected_event_kind);
        if found {
            return outcome;
        }
        assert!(
            started.elapsed() < timeout,
            "timed out waiting for event `{expected_event_kind}` for child session `{child_session_id}`"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn assert_queued_tail_and_get_after_id(
    dispatcher: &DefaultAppToolDispatcher,
    session_context: &SessionContext,
    child_session_id: &str,
) -> i64 {
    let queued_tail = execute_root_tool(
        dispatcher,
        session_context,
        "session_events",
        json!({
            "session_id": child_session_id,
            "after_id": 0,
            "limit": 10
        }),
    )
    .await
    .expect("queued tail session_events outcome");
    assert_eq!(queued_tail.status, "ok");
    assert_eq!(
        queued_tail.payload["after_id"]
            .as_i64()
            .expect("queued after_id"),
        0
    );
    let queued_events = queued_tail.payload["events"]
        .as_array()
        .expect("queued events array");
    assert_eq!(queued_events.len(), 1);
    assert_eq!(queued_events[0]["event_kind"], "delegate_queued");
    let queued_after_id = queued_events[0]
        .as_object()
        .and_then(|_| queued_events[0]["id"].as_i64())
        .expect("queued event id");
    assert_eq!(
        queued_tail.payload["next_after_id"]
            .as_i64()
            .expect("queued next_after_id"),
        queued_after_id
    );
    queued_after_id
}

async fn assert_empty_tail(
    dispatcher: &DefaultAppToolDispatcher,
    session_context: &SessionContext,
    child_session_id: &str,
    after_id: i64,
) {
    let empty_tail = execute_root_tool(
        dispatcher,
        session_context,
        "session_events",
        json!({
            "session_id": child_session_id,
            "after_id": after_id,
            "limit": 10
        }),
    )
    .await
    .expect("empty tail session_events outcome");
    assert_eq!(empty_tail.status, "ok");
    assert!(empty_tail.payload["events"]
        .as_array()
        .expect("empty events array")
        .is_empty());
    assert_eq!(
        empty_tail.payload["next_after_id"]
            .as_i64()
            .expect("empty next_after_id"),
        after_id
    );
}

async fn assert_incremental_tail_contains_only(
    dispatcher: &DefaultAppToolDispatcher,
    session_context: &SessionContext,
    child_session_id: &str,
    after_id: i64,
    expected_event_kind: &str,
) -> i64 {
    let tail = execute_root_tool(
        dispatcher,
        session_context,
        "session_events",
        json!({
            "session_id": child_session_id,
            "after_id": after_id,
            "limit": 10
        }),
    )
    .await
    .expect("incremental tail session_events outcome");
    assert_eq!(tail.status, "ok");
    assert_eq!(
        tail.payload["after_id"]
            .as_i64()
            .expect("incremental after_id"),
        after_id
    );
    let events = tail.payload["events"]
        .as_array()
        .expect("incremental events array");
    assert!(!events.is_empty());
    assert!(events
        .iter()
        .all(|event| event["id"].as_i64().expect("incremental event id") > after_id));
    assert!(events
        .iter()
        .all(|event| event["event_kind"] == expected_event_kind));
    let next_after_id = tail.payload["next_after_id"]
        .as_i64()
        .expect("incremental next_after_id");
    assert!(next_after_id > after_id);
    next_after_id
}

#[tokio::test]
async fn delegate_async_real_subprocess_is_observable_via_session_tools() {
    let _guard = integration_env_lock().lock().expect("env lock");
    let (base_url, server) = spawn_openai_turn_server_once_with_delay(
        "Async child final output",
        Duration::from_millis(400),
    );
    let (config_path, config, memory_config) =
        write_delegate_async_config("delegate-async-observability", &base_url);
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

    let daemon_bin = resolve_daemon_binary_path();
    let spawner = std::sync::Arc::new(BinaryAsyncDelegateSpawner {
        daemon_bin,
        config_path: config_path.clone(),
    });
    let dispatcher = DefaultAppToolDispatcher::with_async_delegate_spawner(
        memory_config.clone(),
        config.tools.clone(),
        spawner,
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        mvp::tools::runtime_tool_view_for_config(&config.tools),
    );

    let queued = execute_root_tool(
        &dispatcher,
        &session_context,
        "delegate_async",
        json!({
            "task": "child task",
            "label": "async-child",
            "timeout_seconds": 5
        }),
    )
    .await
    .expect("delegate_async queued outcome");

    assert_eq!(queued.status, "ok");
    assert_eq!(queued.payload["mode"], "async");
    assert_eq!(queued.payload["state"], "queued");
    let child_session_id = queued.payload["child_session_id"]
        .as_str()
        .expect("child session id")
        .to_owned();

    let initial_status = execute_root_tool(
        &dispatcher,
        &session_context,
        "session_status",
        json!({
            "session_id": child_session_id
        }),
    )
    .await
    .expect("initial session_status outcome");
    assert_eq!(initial_status.status, "ok");
    assert_eq!(
        initial_status.payload["session"]["session_id"],
        child_session_id
    );
    assert!(matches!(
        initial_status.payload["session"]["state"].as_str(),
        Some("ready") | Some("running") | Some("completed")
    ));

    let started_events = wait_for_session_event(
        &dispatcher,
        &session_context,
        &child_session_id,
        "delegate_started",
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(started_events.status, "ok");

    let running_status = execute_root_tool(
        &dispatcher,
        &session_context,
        "session_status",
        json!({
            "session_id": child_session_id
        }),
    )
    .await
    .expect("running session_status outcome");
    assert_eq!(running_status.status, "ok");
    assert_eq!(
        running_status.payload["session"]["session_id"],
        child_session_id
    );
    assert!(matches!(
        running_status.payload["session"]["state"].as_str(),
        Some("running") | Some("completed")
    ));
    assert!(running_status.payload["recent_events"]
        .as_array()
        .expect("recent_events array")
        .iter()
        .any(|event| event["event_kind"] == "delegate_started"));

    let waited = execute_root_tool(
        &dispatcher,
        &session_context,
        "session_wait",
        json!({
            "session_id": child_session_id,
            "timeout_ms": 5_000
        }),
    )
    .await
    .expect("session_wait outcome");
    assert_eq!(waited.status, "ok");
    assert_eq!(waited.payload["wait_status"], "completed");
    assert_eq!(waited.payload["session"]["state"], "completed");
    assert_eq!(waited.payload["terminal_outcome"]["status"], "ok");
    assert_eq!(
        waited.payload["terminal_outcome"]["payload"]["final_output"],
        "Async child final output"
    );

    let final_events = execute_root_tool(
        &dispatcher,
        &session_context,
        "session_events",
        json!({
            "session_id": child_session_id,
            "limit": 10
        }),
    )
    .await
    .expect("final session_events outcome");
    assert_eq!(final_events.status, "ok");
    let event_kinds: Vec<&str> = final_events.payload["events"]
        .as_array()
        .expect("events array")
        .iter()
        .map(|event| event["event_kind"].as_str().expect("event kind"))
        .collect();
    assert!(event_kinds.contains(&"delegate_queued"));
    assert!(event_kinds.contains(&"delegate_started"));
    assert!(event_kinds.contains(&"delegate_completed"));

    server.join().expect("join provider stub");
}

#[tokio::test]
async fn delegate_async_real_subprocess_session_events_after_id_is_incremental() {
    let _guard = integration_env_lock().lock().expect("env lock");
    let (base_url, provider_request_rx, release_response_tx, server) =
        spawn_openai_turn_server_once_with_response_gate("Async child tail output");
    let (config_path, config, memory_config) =
        write_delegate_async_config("delegate-async-observability-tail", &base_url);
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

    let daemon_bin = resolve_daemon_binary_path();
    let (spawner, request_rx, launch_notify) =
        GatedAsyncDelegateSpawner::new(BinaryAsyncDelegateSpawner {
            daemon_bin,
            config_path: config_path.clone(),
        });
    let dispatcher = DefaultAppToolDispatcher::with_async_delegate_spawner(
        memory_config.clone(),
        config.tools.clone(),
        Arc::new(spawner),
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        mvp::tools::runtime_tool_view_for_config(&config.tools),
    );

    let queued_dispatcher = dispatcher.clone();
    let queued_session_context = session_context.clone();
    let queued_call = tokio::spawn(async move {
        execute_root_tool(
            &queued_dispatcher,
            &queued_session_context,
            "delegate_async",
            json!({
                "task": "child task tail",
                "label": "async-child-tail",
                "timeout_seconds": 5
            }),
        )
        .await
    });

    let queued = tokio::time::timeout(Duration::from_millis(250), queued_call)
        .await
        .expect("delegate_async should return queued handle before child launch")
        .expect("join delegate_async queued task")
        .expect("delegate_async queued outcome");
    assert_eq!(queued.status, "ok");
    assert_eq!(queued.payload["mode"], "async");
    assert_eq!(queued.payload["state"], "queued");

    let spawn_request = tokio::time::timeout(Duration::from_secs(5), request_rx)
        .await
        .expect("timed out waiting for gated spawn request")
        .expect("gated spawn request");
    let child_session_id = spawn_request.child_session_id.clone();
    assert_eq!(spawn_request.parent_session_id, "root-session");
    assert_eq!(spawn_request.task, "child task tail");
    assert_eq!(spawn_request.label.as_deref(), Some("async-child-tail"));
    assert_eq!(spawn_request.timeout_seconds, 5);
    assert_eq!(queued.payload["child_session_id"], child_session_id);

    let queued_after_id =
        assert_queued_tail_and_get_after_id(&dispatcher, &session_context, &child_session_id).await;
    assert_empty_tail(
        &dispatcher,
        &session_context,
        &child_session_id,
        queued_after_id,
    )
    .await;

    launch_notify.notify_one();

    tokio::time::timeout(Duration::from_secs(5), provider_request_rx)
        .await
        .expect("timed out waiting for provider request")
        .expect("provider request signal");

    let started_after_id = assert_incremental_tail_contains_only(
        &dispatcher,
        &session_context,
        &child_session_id,
        queued_after_id,
        "delegate_started",
    )
    .await;

    release_response_tx
        .send(())
        .expect("release provider response");

    let waited = execute_root_tool(
        &dispatcher,
        &session_context,
        "session_wait",
        json!({
            "session_id": child_session_id,
            "timeout_ms": 5_000
        }),
    )
    .await
    .expect("tail session_wait outcome");
    assert_eq!(waited.status, "ok");
    assert_eq!(waited.payload["wait_status"], "completed");
    assert_eq!(waited.payload["session"]["state"], "completed");
    assert_eq!(
        waited.payload["terminal_outcome"]["payload"]["final_output"],
        "Async child tail output"
    );

    let completed_after_id = assert_incremental_tail_contains_only(
        &dispatcher,
        &session_context,
        &child_session_id,
        started_after_id,
        "delegate_completed",
    )
    .await;

    assert_empty_tail(
        &dispatcher,
        &session_context,
        &child_session_id,
        completed_after_id,
    )
    .await;

    server.join().expect("join provider stub");
}

#[tokio::test]
async fn delegate_async_real_subprocess_session_wait_returns_incremental_events_after_id() {
    let _guard = integration_env_lock().lock().expect("env lock");
    let (base_url, provider_request_rx, release_response_tx, server) =
        spawn_openai_turn_server_once_with_response_gate("Async child wait tail output");
    let (config_path, config, memory_config) =
        write_delegate_async_config("delegate-async-observability-wait-tail", &base_url);
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

    let daemon_bin = resolve_daemon_binary_path();
    let (spawner, request_rx, launch_notify) =
        GatedAsyncDelegateSpawner::new(BinaryAsyncDelegateSpawner {
            daemon_bin,
            config_path: config_path.clone(),
        });
    let dispatcher = DefaultAppToolDispatcher::with_async_delegate_spawner(
        memory_config.clone(),
        config.tools.clone(),
        Arc::new(spawner),
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        mvp::tools::runtime_tool_view_for_config(&config.tools),
    );

    let queued_dispatcher = dispatcher.clone();
    let queued_session_context = session_context.clone();
    let queued_call = tokio::spawn(async move {
        execute_root_tool(
            &queued_dispatcher,
            &queued_session_context,
            "delegate_async",
            json!({
                "task": "child task wait tail",
                "label": "async-child-wait-tail",
                "timeout_seconds": 5
            }),
        )
        .await
    });

    let queued = tokio::time::timeout(Duration::from_millis(250), queued_call)
        .await
        .expect("delegate_async should return queued handle before child launch")
        .expect("join delegate_async queued task")
        .expect("delegate_async queued outcome");
    assert_eq!(queued.status, "ok");
    assert_eq!(queued.payload["mode"], "async");
    assert_eq!(queued.payload["state"], "queued");

    let spawn_request = tokio::time::timeout(Duration::from_secs(5), request_rx)
        .await
        .expect("timed out waiting for gated spawn request")
        .expect("gated spawn request");
    let child_session_id = spawn_request.child_session_id.clone();
    assert_eq!(spawn_request.parent_session_id, "root-session");
    assert_eq!(spawn_request.task, "child task wait tail");
    assert_eq!(
        spawn_request.label.as_deref(),
        Some("async-child-wait-tail")
    );
    assert_eq!(spawn_request.timeout_seconds, 5);
    assert_eq!(queued.payload["child_session_id"], child_session_id);

    let queued_after_id =
        assert_queued_tail_and_get_after_id(&dispatcher, &session_context, &child_session_id).await;
    assert_empty_tail(
        &dispatcher,
        &session_context,
        &child_session_id,
        queued_after_id,
    )
    .await;

    launch_notify.notify_one();

    tokio::time::timeout(Duration::from_secs(5), provider_request_rx)
        .await
        .expect("timed out waiting for provider request")
        .expect("provider request signal");

    let wait_dispatcher = dispatcher.clone();
    let wait_session_context = session_context.clone();
    let wait_child_session_id = child_session_id.clone();
    let wait_task = tokio::spawn(async move {
        execute_root_tool(
            &wait_dispatcher,
            &wait_session_context,
            "session_wait",
            json!({
                "session_id": wait_child_session_id,
                "timeout_ms": 5_000,
                "after_id": queued_after_id
            }),
        )
        .await
    });

    release_response_tx
        .send(())
        .expect("release provider response");

    let waited = wait_task
        .await
        .expect("join session_wait task")
        .expect("session_wait outcome");
    assert_eq!(waited.status, "ok");
    assert_eq!(waited.payload["wait_status"], "completed");
    assert_eq!(waited.payload["after_id"], queued_after_id);
    assert_eq!(waited.payload["session"]["state"], "completed");
    assert_eq!(waited.payload["terminal_outcome"]["status"], "ok");
    assert_eq!(
        waited.payload["terminal_outcome"]["payload"]["final_output"],
        "Async child wait tail output"
    );
    let events = waited.payload["events"]
        .as_array()
        .expect("session_wait events array");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event_kind"], "delegate_started");
    assert_eq!(events[1]["event_kind"], "delegate_completed");
    let wait_next_after_id = waited.payload["next_after_id"]
        .as_i64()
        .expect("session_wait next_after_id");
    assert_eq!(
        wait_next_after_id,
        events[1]["id"]
            .as_i64()
            .expect("completed event id from session_wait")
    );
    assert!(wait_next_after_id > queued_after_id);

    assert_empty_tail(
        &dispatcher,
        &session_context,
        &child_session_id,
        wait_next_after_id,
    )
    .await;

    server.join().expect("join provider stub");
}

#[tokio::test]
async fn delegate_async_real_subprocess_failure_session_events_after_id_is_incremental() {
    let _guard = integration_env_lock().lock().expect("env lock");
    let (base_url, provider_request_rx, release_response_tx, server) =
        spawn_openai_error_server_once_with_response_gate(
            500,
            json!({
                "error": {
                    "message": "stub failure"
                }
            }),
        );
    let (config_path, config, memory_config) =
        write_delegate_async_config("delegate-async-observability-failure-tail", &base_url);
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

    let daemon_bin = resolve_daemon_binary_path();
    let (spawner, request_rx, launch_notify) =
        GatedAsyncDelegateSpawner::new(BinaryAsyncDelegateSpawner {
            daemon_bin,
            config_path: config_path.clone(),
        });
    let dispatcher = DefaultAppToolDispatcher::with_async_delegate_spawner(
        memory_config.clone(),
        config.tools.clone(),
        Arc::new(spawner),
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        mvp::tools::runtime_tool_view_for_config(&config.tools),
    );

    let queued_dispatcher = dispatcher.clone();
    let queued_session_context = session_context.clone();
    let queued_call = tokio::spawn(async move {
        execute_root_tool(
            &queued_dispatcher,
            &queued_session_context,
            "delegate_async",
            json!({
                "task": "child task failure tail",
                "label": "async-child-failure-tail",
                "timeout_seconds": 5
            }),
        )
        .await
    });

    let queued = tokio::time::timeout(Duration::from_millis(250), queued_call)
        .await
        .expect("delegate_async should return queued handle before child launch")
        .expect("join delegate_async queued task")
        .expect("delegate_async queued outcome");
    assert_eq!(queued.status, "ok");
    assert_eq!(queued.payload["mode"], "async");
    assert_eq!(queued.payload["state"], "queued");

    let spawn_request = tokio::time::timeout(Duration::from_secs(5), request_rx)
        .await
        .expect("timed out waiting for gated spawn request")
        .expect("gated spawn request");
    let child_session_id = spawn_request.child_session_id.clone();
    assert_eq!(spawn_request.parent_session_id, "root-session");
    assert_eq!(spawn_request.task, "child task failure tail");
    assert_eq!(
        spawn_request.label.as_deref(),
        Some("async-child-failure-tail")
    );
    assert_eq!(spawn_request.timeout_seconds, 5);
    assert_eq!(queued.payload["child_session_id"], child_session_id);

    let queued_after_id =
        assert_queued_tail_and_get_after_id(&dispatcher, &session_context, &child_session_id).await;
    assert_empty_tail(
        &dispatcher,
        &session_context,
        &child_session_id,
        queued_after_id,
    )
    .await;

    launch_notify.notify_one();

    tokio::time::timeout(Duration::from_secs(5), provider_request_rx)
        .await
        .expect("timed out waiting for provider request")
        .expect("provider request signal");

    let started_after_id = assert_incremental_tail_contains_only(
        &dispatcher,
        &session_context,
        &child_session_id,
        queued_after_id,
        "delegate_started",
    )
    .await;

    release_response_tx
        .send(())
        .expect("release provider error response");

    let waited = execute_root_tool(
        &dispatcher,
        &session_context,
        "session_wait",
        json!({
            "session_id": child_session_id,
            "timeout_ms": 5_000
        }),
    )
    .await
    .expect("failure tail session_wait outcome");
    assert_eq!(waited.status, "ok");
    assert_eq!(waited.payload["wait_status"], "completed");
    assert_eq!(waited.payload["session"]["state"], "failed");
    assert_eq!(waited.payload["terminal_outcome"]["status"], "error");
    assert!(waited.payload["terminal_outcome"]["payload"]["error"]
        .as_str()
        .expect("failure tail error")
        .contains("provider returned status 500"));

    let failed_after_id = assert_incremental_tail_contains_only(
        &dispatcher,
        &session_context,
        &child_session_id,
        started_after_id,
        "delegate_failed",
    )
    .await;

    assert_empty_tail(
        &dispatcher,
        &session_context,
        &child_session_id,
        failed_after_id,
    )
    .await;

    server.join().expect("join provider stub");
}

#[tokio::test]
async fn delegate_async_real_subprocess_failure_is_observable_via_session_tools() {
    let _guard = integration_env_lock().lock().expect("env lock");
    let (base_url, server) = spawn_openai_error_server_once(
        500,
        json!({
            "error": {
                "message": "stub failure"
            }
        }),
    );
    let (config_path, config, memory_config) =
        write_delegate_async_config("delegate-async-observability-failure", &base_url);
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

    let daemon_bin = resolve_daemon_binary_path();
    let spawner = std::sync::Arc::new(BinaryAsyncDelegateSpawner {
        daemon_bin,
        config_path: config_path.clone(),
    });
    let dispatcher = DefaultAppToolDispatcher::with_async_delegate_spawner(
        memory_config.clone(),
        config.tools.clone(),
        spawner,
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        mvp::tools::runtime_tool_view_for_config(&config.tools),
    );

    let queued = execute_root_tool(
        &dispatcher,
        &session_context,
        "delegate_async",
        json!({
            "task": "child task failure",
            "label": "async-child-failure",
            "timeout_seconds": 5
        }),
    )
    .await
    .expect("delegate_async queued outcome");

    assert_eq!(queued.status, "ok");
    let child_session_id = queued.payload["child_session_id"]
        .as_str()
        .expect("child session id")
        .to_owned();

    let failed_events = wait_for_session_event(
        &dispatcher,
        &session_context,
        &child_session_id,
        "delegate_failed",
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(failed_events.status, "ok");

    let status = execute_root_tool(
        &dispatcher,
        &session_context,
        "session_status",
        json!({
            "session_id": child_session_id
        }),
    )
    .await
    .expect("failure session_status outcome");
    assert_eq!(status.status, "ok");
    assert_eq!(status.payload["session"]["state"], "failed");
    assert_eq!(status.payload["terminal_outcome"]["status"], "error");
    assert!(status.payload["terminal_outcome"]["payload"]["error"]
        .as_str()
        .expect("terminal error")
        .contains("provider returned status 500"));

    let waited = execute_root_tool(
        &dispatcher,
        &session_context,
        "session_wait",
        json!({
            "session_id": child_session_id,
            "timeout_ms": 5_000
        }),
    )
    .await
    .expect("failure session_wait outcome");
    assert_eq!(waited.status, "ok");
    assert_eq!(waited.payload["wait_status"], "completed");
    assert_eq!(waited.payload["session"]["state"], "failed");
    assert_eq!(waited.payload["terminal_outcome"]["status"], "error");
    assert!(waited.payload["terminal_outcome"]["payload"]["error"]
        .as_str()
        .expect("waited terminal error")
        .contains("provider returned status 500"));

    let final_events = execute_root_tool(
        &dispatcher,
        &session_context,
        "session_events",
        json!({
            "session_id": child_session_id,
            "limit": 10
        }),
    )
    .await
    .expect("final failure session_events outcome");
    assert_eq!(final_events.status, "ok");
    let event_kinds: Vec<&str> = final_events.payload["events"]
        .as_array()
        .expect("events array")
        .iter()
        .map(|event| event["event_kind"].as_str().expect("event kind"))
        .collect();
    assert!(event_kinds.contains(&"delegate_queued"));
    assert!(event_kinds.contains(&"delegate_started"));
    assert!(event_kinds.contains(&"delegate_failed"));

    server.join().expect("join provider stub");
}

#[tokio::test]
async fn delegate_async_real_subprocess_timeout_session_events_after_id_is_incremental() {
    let _guard = integration_env_lock().lock().expect("env lock");
    let (base_url, provider_request_rx, release_response_tx, server) =
        spawn_openai_turn_server_once_with_response_gate("Too slow");
    let (config_path, config, memory_config) =
        write_delegate_async_config("delegate-async-observability-timeout-tail", &base_url);
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

    let daemon_bin = resolve_daemon_binary_path();
    let (spawner, request_rx, launch_notify) =
        GatedAsyncDelegateSpawner::new(BinaryAsyncDelegateSpawner {
            daemon_bin,
            config_path: config_path.clone(),
        });
    let dispatcher = DefaultAppToolDispatcher::with_async_delegate_spawner(
        memory_config.clone(),
        config.tools.clone(),
        Arc::new(spawner),
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        mvp::tools::runtime_tool_view_for_config(&config.tools),
    );

    let queued_dispatcher = dispatcher.clone();
    let queued_session_context = session_context.clone();
    let queued_call = tokio::spawn(async move {
        execute_root_tool(
            &queued_dispatcher,
            &queued_session_context,
            "delegate_async",
            json!({
                "task": "child task timeout tail",
                "label": "async-child-timeout-tail",
                "timeout_seconds": 1
            }),
        )
        .await
    });

    let queued = tokio::time::timeout(Duration::from_millis(250), queued_call)
        .await
        .expect("delegate_async should return queued handle before child launch")
        .expect("join delegate_async queued task")
        .expect("delegate_async queued outcome");
    assert_eq!(queued.status, "ok");
    assert_eq!(queued.payload["mode"], "async");
    assert_eq!(queued.payload["state"], "queued");

    let spawn_request = tokio::time::timeout(Duration::from_secs(5), request_rx)
        .await
        .expect("timed out waiting for gated spawn request")
        .expect("gated spawn request");
    let child_session_id = spawn_request.child_session_id.clone();
    assert_eq!(spawn_request.parent_session_id, "root-session");
    assert_eq!(spawn_request.task, "child task timeout tail");
    assert_eq!(
        spawn_request.label.as_deref(),
        Some("async-child-timeout-tail")
    );
    assert_eq!(spawn_request.timeout_seconds, 1);
    assert_eq!(queued.payload["child_session_id"], child_session_id);

    let queued_after_id =
        assert_queued_tail_and_get_after_id(&dispatcher, &session_context, &child_session_id).await;
    assert_empty_tail(
        &dispatcher,
        &session_context,
        &child_session_id,
        queued_after_id,
    )
    .await;

    launch_notify.notify_one();

    tokio::time::timeout(Duration::from_secs(5), provider_request_rx)
        .await
        .expect("timed out waiting for provider request")
        .expect("provider request signal");

    let started_after_id = assert_incremental_tail_contains_only(
        &dispatcher,
        &session_context,
        &child_session_id,
        queued_after_id,
        "delegate_started",
    )
    .await;

    let waited = execute_root_tool(
        &dispatcher,
        &session_context,
        "session_wait",
        json!({
            "session_id": child_session_id,
            "timeout_ms": 5_000
        }),
    )
    .await
    .expect("timeout tail session_wait outcome");
    assert_eq!(waited.status, "ok");
    assert_eq!(waited.payload["wait_status"], "completed");
    assert_eq!(waited.payload["session"]["state"], "timed_out");
    assert_eq!(waited.payload["terminal_outcome"]["status"], "timeout");
    assert_eq!(
        waited.payload["terminal_outcome"]["payload"]["error"],
        "delegate_timeout"
    );

    let timed_out_after_id = assert_incremental_tail_contains_only(
        &dispatcher,
        &session_context,
        &child_session_id,
        started_after_id,
        "delegate_timed_out",
    )
    .await;

    assert_empty_tail(
        &dispatcher,
        &session_context,
        &child_session_id,
        timed_out_after_id,
    )
    .await;

    release_response_tx
        .send(())
        .expect("release provider response after timeout");
    server.join().expect("join provider stub");
}

#[tokio::test]
async fn delegate_async_real_subprocess_timeout_is_observable_via_session_tools() {
    let _guard = integration_env_lock().lock().expect("env lock");
    let (base_url, server) =
        spawn_openai_turn_server_once_with_delay("Too slow", Duration::from_millis(1_200));
    let (config_path, config, memory_config) =
        write_delegate_async_config("delegate-async-observability-timeout", &base_url);
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

    let daemon_bin = resolve_daemon_binary_path();
    let spawner = std::sync::Arc::new(BinaryAsyncDelegateSpawner {
        daemon_bin,
        config_path: config_path.clone(),
    });
    let dispatcher = DefaultAppToolDispatcher::with_async_delegate_spawner(
        memory_config.clone(),
        config.tools.clone(),
        spawner,
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        mvp::tools::runtime_tool_view_for_config(&config.tools),
    );

    let queued = execute_root_tool(
        &dispatcher,
        &session_context,
        "delegate_async",
        json!({
            "task": "child task timeout",
            "label": "async-child-timeout",
            "timeout_seconds": 1
        }),
    )
    .await
    .expect("delegate_async queued outcome");

    assert_eq!(queued.status, "ok");
    let child_session_id = queued.payload["child_session_id"]
        .as_str()
        .expect("child session id")
        .to_owned();

    let timed_out_events = wait_for_session_event(
        &dispatcher,
        &session_context,
        &child_session_id,
        "delegate_timed_out",
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(timed_out_events.status, "ok");

    let status = execute_root_tool(
        &dispatcher,
        &session_context,
        "session_status",
        json!({
            "session_id": child_session_id
        }),
    )
    .await
    .expect("timeout session_status outcome");
    assert_eq!(status.status, "ok");
    assert_eq!(status.payload["session"]["state"], "timed_out");
    assert_eq!(status.payload["terminal_outcome"]["status"], "timeout");
    assert_eq!(
        status.payload["terminal_outcome"]["payload"]["error"],
        "delegate_timeout"
    );

    let waited = execute_root_tool(
        &dispatcher,
        &session_context,
        "session_wait",
        json!({
            "session_id": child_session_id,
            "timeout_ms": 5_000
        }),
    )
    .await
    .expect("timeout session_wait outcome");
    assert_eq!(waited.status, "ok");
    assert_eq!(waited.payload["wait_status"], "completed");
    assert_eq!(waited.payload["session"]["state"], "timed_out");
    assert_eq!(waited.payload["terminal_outcome"]["status"], "timeout");
    assert_eq!(
        waited.payload["terminal_outcome"]["payload"]["error"],
        "delegate_timeout"
    );

    let final_events = execute_root_tool(
        &dispatcher,
        &session_context,
        "session_events",
        json!({
            "session_id": child_session_id,
            "limit": 10
        }),
    )
    .await
    .expect("final timeout session_events outcome");
    assert_eq!(final_events.status, "ok");
    let event_kinds: Vec<&str> = final_events.payload["events"]
        .as_array()
        .expect("events array")
        .iter()
        .map(|event| event["event_kind"].as_str().expect("event kind"))
        .collect();
    assert!(event_kinds.contains(&"delegate_queued"));
    assert!(event_kinds.contains(&"delegate_started"));
    assert!(event_kinds.contains(&"delegate_timed_out"));

    server.join().expect("join provider stub");
}
