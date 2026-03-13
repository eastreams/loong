use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[cfg(feature = "memory-sqlite")]
use std::{fs, path::PathBuf};

use async_trait::async_trait;
use loongclaw_contracts::{Capability, ExecutionRoute, HarnessKind, MemoryPlaneError};
use loongclaw_kernel::{
    CoreMemoryAdapter, FixedClock, InMemoryAuditSink, LoongClawKernel, MemoryCoreOutcome,
    MemoryCoreRequest, StaticPolicyEngine, VerticalPackManifest,
};
#[cfg(feature = "memory-sqlite")]
use rusqlite::Connection;
use serde_json::{json, Value};
use tokio::sync::{oneshot, Notify};
use tokio::time::sleep;

use super::super::config::{
    CliChannelConfig, ConversationConfig, FeishuChannelConfig, LoongClawConfig, MemoryConfig,
    ProviderConfig, TelegramChannelConfig, ToolConfig,
};
use super::persistence::format_provider_error_reply;
use super::runtime::DefaultConversationRuntime;
use super::*;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::SessionRepository;
use crate::CliResult;
use crate::KernelContext;

#[cfg(feature = "memory-sqlite")]
#[derive(Default)]
struct FakeAsyncDelegateSpawner {
    requests: Arc<Mutex<Vec<crate::conversation::turn_engine::AsyncDelegateSpawnRequest>>>,
    spawn_error: Option<String>,
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl crate::conversation::turn_engine::AsyncDelegateSpawner for FakeAsyncDelegateSpawner {
    async fn spawn(
        &self,
        request: crate::conversation::turn_engine::AsyncDelegateSpawnRequest,
    ) -> Result<(), String> {
        self.requests
            .lock()
            .expect("async delegate requests lock")
            .push(request);
        match &self.spawn_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}

#[cfg(feature = "memory-sqlite")]
struct PanicAsyncDelegateSpawner;

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl crate::conversation::turn_engine::AsyncDelegateSpawner for PanicAsyncDelegateSpawner {
    async fn spawn(
        &self,
        _request: crate::conversation::turn_engine::AsyncDelegateSpawnRequest,
    ) -> Result<(), String> {
        panic!("panic-async-spawn");
    }
}

#[cfg(feature = "memory-sqlite")]
struct GatedFakeAsyncDelegateSpawner {
    requests: Arc<Mutex<Vec<crate::conversation::turn_engine::AsyncDelegateSpawnRequest>>>,
    request_tx:
        Mutex<Option<oneshot::Sender<crate::conversation::turn_engine::AsyncDelegateSpawnRequest>>>,
    release_notify: Arc<Notify>,
}

#[cfg(feature = "memory-sqlite")]
impl GatedFakeAsyncDelegateSpawner {
    fn new() -> (
        Self,
        oneshot::Receiver<crate::conversation::turn_engine::AsyncDelegateSpawnRequest>,
        Arc<Notify>,
    ) {
        let (request_tx, request_rx) = oneshot::channel();
        let release_notify = Arc::new(Notify::new());
        (
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                request_tx: Mutex::new(Some(request_tx)),
                release_notify: release_notify.clone(),
            },
            request_rx,
            release_notify,
        )
    }
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl crate::conversation::turn_engine::AsyncDelegateSpawner for GatedFakeAsyncDelegateSpawner {
    async fn spawn(
        &self,
        request: crate::conversation::turn_engine::AsyncDelegateSpawnRequest,
    ) -> Result<(), String> {
        self.requests
            .lock()
            .expect("async delegate requests lock")
            .push(request.clone());
        let request_tx = self
            .request_tx
            .lock()
            .expect("async delegate request sender lock")
            .take()
            .expect("single gated async delegate request sender");
        request_tx
            .send(request)
            .expect("gated async delegate request receiver");
        self.release_notify.notified().await;
        Ok(())
    }
}

struct FakeRuntime {
    seed_messages: Vec<Value>,
    tool_view: crate::tools::ToolView,
    completion_responses: Mutex<VecDeque<Result<String, String>>>,
    turn_responses: Mutex<VecDeque<Result<ProviderTurn, String>>>,
    persisted: Mutex<Vec<(String, String, String)>>,
    requested_messages: Mutex<Vec<Value>>,
    turn_requested_messages: Mutex<Vec<Vec<Value>>>,
    built_tool_views: Mutex<Vec<crate::tools::ToolView>>,
    turn_requested_tool_views: Mutex<Vec<crate::tools::ToolView>>,
    completion_requested_messages: Mutex<Vec<Vec<Value>>>,
    turn_delays: Mutex<VecDeque<Duration>>,
    completion_calls: Mutex<usize>,
    turn_calls: Mutex<usize>,
}

impl FakeRuntime {
    fn new(seed_messages: Vec<Value>, completion: Result<String, String>) -> Self {
        let turn = completion.as_ref().map_or_else(
            |error| Err(error.to_owned()),
            |content| {
                Ok(ProviderTurn {
                    assistant_text: content.to_owned(),
                    tool_intents: Vec::new(),
                    raw_meta: Value::Null,
                })
            },
        );
        Self::with_turns_and_completions(seed_messages, vec![turn], vec![completion])
    }

    fn with_turn_and_completion(
        seed_messages: Vec<Value>,
        turn: Result<ProviderTurn, String>,
        completion: Result<String, String>,
    ) -> Self {
        Self::with_turns_and_completions(seed_messages, vec![turn], vec![completion])
    }

    fn with_turns(seed_messages: Vec<Value>, turns: Vec<Result<ProviderTurn, String>>) -> Self {
        Self::with_turns_and_completions(seed_messages, turns, Vec::new())
    }

    fn with_turns_and_completions(
        seed_messages: Vec<Value>,
        turns: Vec<Result<ProviderTurn, String>>,
        completions: Vec<Result<String, String>>,
    ) -> Self {
        Self {
            seed_messages,
            tool_view: crate::tools::runtime_tool_view(),
            completion_responses: Mutex::new(VecDeque::from(completions)),
            turn_responses: Mutex::new(VecDeque::from(turns)),
            persisted: Mutex::new(Vec::new()),
            requested_messages: Mutex::new(Vec::new()),
            turn_requested_messages: Mutex::new(Vec::new()),
            built_tool_views: Mutex::new(Vec::new()),
            turn_requested_tool_views: Mutex::new(Vec::new()),
            completion_requested_messages: Mutex::new(Vec::new()),
            turn_delays: Mutex::new(VecDeque::new()),
            completion_calls: Mutex::new(0),
            turn_calls: Mutex::new(0),
        }
    }

    fn with_tool_view(mut self, tool_view: crate::tools::ToolView) -> Self {
        self.tool_view = tool_view;
        self
    }

    fn with_turn_delays(self, delays: Vec<Duration>) -> Self {
        let runtime = self;
        *runtime.turn_delays.lock().expect("turn delays lock") = VecDeque::from(delays);
        runtime
    }
}

struct PanicRuntime {
    tool_view: crate::tools::ToolView,
}

impl PanicRuntime {
    fn new() -> Self {
        Self {
            tool_view: crate::tools::runtime_tool_view(),
        }
    }
}

#[async_trait]
impl ConversationRuntime for FakeRuntime {
    fn tool_view(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<crate::tools::ToolView> {
        Ok(self.tool_view.clone())
    }

    fn build_messages(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        tool_view: &crate::tools::ToolView,
        _kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<Vec<Value>> {
        self.built_tool_views
            .lock()
            .expect("built tool views lock")
            .push(tool_view.clone());
        Ok(self.seed_messages.clone())
    }

    async fn request_completion(
        &self,
        _config: &LoongClawConfig,
        messages: &[Value],
    ) -> CliResult<String> {
        let mut calls = self.completion_calls.lock().expect("completion calls lock");
        *calls += 1;
        *self.requested_messages.lock().expect("request lock") = messages.to_vec();
        self.completion_requested_messages
            .lock()
            .expect("completion request lock")
            .push(messages.to_vec());
        self.completion_responses
            .lock()
            .expect("completion response lock")
            .pop_front()
            .unwrap_or_else(|| Err("unexpected_completion_call".to_owned()))
            .map_err(|error| error.to_owned())
    }

    async fn request_turn(
        &self,
        _config: &LoongClawConfig,
        messages: &[Value],
        tool_view: &crate::tools::ToolView,
    ) -> CliResult<ProviderTurn> {
        let delay = {
            self.turn_delays
                .lock()
                .expect("turn delays lock")
                .pop_front()
        };
        if let Some(delay) = delay {
            sleep(delay).await;
        }
        let mut calls = self.turn_calls.lock().expect("turn calls lock");
        *calls += 1;
        *self.requested_messages.lock().expect("request lock") = messages.to_vec();
        self.turn_requested_messages
            .lock()
            .expect("turn request lock")
            .push(messages.to_vec());
        self.turn_requested_tool_views
            .lock()
            .expect("turn request tool views lock")
            .push(tool_view.clone());
        self.turn_responses
            .lock()
            .expect("turn response lock")
            .pop_front()
            .unwrap_or_else(|| Err("unexpected_turn_call".to_owned()))
            .map_err(|error| error.to_owned())
    }

    async fn persist_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        _kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<()> {
        self.persisted.lock().expect("persist lock").push((
            session_id.to_owned(),
            role.to_owned(),
            content.to_owned(),
        ));
        Ok(())
    }
}

#[async_trait]
impl ConversationRuntime for PanicRuntime {
    fn tool_view(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<crate::tools::ToolView> {
        Ok(self.tool_view.clone())
    }

    fn build_messages(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _tool_view: &crate::tools::ToolView,
        _kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<Vec<Value>> {
        Ok(Vec::new())
    }

    async fn request_completion(
        &self,
        _config: &LoongClawConfig,
        _messages: &[Value],
    ) -> CliResult<String> {
        Err("unexpected_completion_call".to_owned())
    }

    async fn request_turn(
        &self,
        _config: &LoongClawConfig,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
    ) -> CliResult<ProviderTurn> {
        panic!("panic-runtime-request-turn");
    }

    async fn persist_turn(
        &self,
        _session_id: &str,
        _role: &str,
        _content: &str,
        _kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<()> {
        Ok(())
    }
}

fn test_config() -> LoongClawConfig {
    LoongClawConfig {
        provider: ProviderConfig::default(),
        cli: CliChannelConfig::default(),
        telegram: TelegramChannelConfig::default(),
        feishu: FeishuChannelConfig::default(),
        tools: ToolConfig::default(),
        memory: MemoryConfig::default(),
        conversation: ConversationConfig::default(),
    }
}

#[cfg(feature = "memory-sqlite")]
fn isolated_sqlite_path(test_name: &str) -> String {
    let base = std::env::temp_dir().join(format!(
        "loongclaw-conversation-tests-{test_name}-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&base);
    let db_path = base.join("memory.sqlite3");
    let _ = fs::remove_file(&db_path);
    db_path.display().to_string()
}

#[cfg(feature = "memory-sqlite")]
fn isolated_test_config(test_name: &str) -> (LoongClawConfig, PathBuf) {
    let base = std::env::temp_dir().join(format!(
        "loongclaw-conversation-tests-{test_name}-{}",
        std::process::id()
    ));
    let _ = fs::create_dir_all(&base);
    let db_path = base.join("memory.sqlite3");
    let _ = fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    (config, db_path)
}

#[cfg(all(feature = "memory-sqlite", feature = "channel-telegram"))]
fn spawn_telegram_send_server_once() -> (
    String,
    std::sync::mpsc::Receiver<String>,
    std::thread::JoinHandle<()>,
) {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind telegram stub");
    let addr = listener.local_addr().expect("telegram stub addr");
    let (request_tx, request_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request_buf = [0_u8; 8192];
            let read = stream
                .read(&mut request_buf)
                .expect("read telegram request");
            request_tx
                .send(String::from_utf8_lossy(&request_buf[..read]).into_owned())
                .expect("send telegram request capture");
            let body = serde_json::to_string(&json!({
                "ok": true,
                "result": {
                    "message_id": 1
                }
            }))
            .expect("serialize telegram stub body");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write telegram response");
        }
    });
    (format!("http://{addr}"), request_rx, server)
}

#[tokio::test]
async fn handle_turn_with_runtime_success_persists_user_and_assistant_turns() {
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    );
    let turn_loop = ConversationTurnLoop::new();
    let reply = turn_loop
        .handle_turn_with_runtime(
            &test_config(),
            "session-1",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("handle turn success");

    assert_eq!(reply, "assistant-reply");

    let requested = runtime.requested_messages.lock().expect("requested lock");
    assert_eq!(requested.len(), 2);
    assert_eq!(requested[1]["role"], "user");
    assert_eq!(requested[1]["content"], "hello");

    let persisted = runtime.persisted.lock().expect("persisted lock");
    assert_eq!(persisted.len(), 2);
    assert_eq!(
        persisted[0],
        (
            "session-1".to_owned(),
            "user".to_owned(),
            "hello".to_owned()
        )
    );
    assert_eq!(
        persisted[1],
        (
            "session-1".to_owned(),
            "assistant".to_owned(),
            "assistant-reply".to_owned(),
        )
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_registers_root_session_metadata() {
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    );
    let mut config = test_config();
    config.memory.sqlite_path = isolated_sqlite_path("register-root-session");

    let reply = ConversationTurnLoop::new()
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("handle turn success");

    assert_eq!(reply, "assistant-reply");

    let repo = crate::session::repository::SessionRepository::new(
        &crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
    )
    .expect("session repository");
    let session = repo
        .load_session("root-session")
        .expect("load session")
        .expect("session row");
    assert_eq!(session.kind, crate::session::repository::SessionKind::Root);
    assert_eq!(session.parent_session_id, None);
    assert_eq!(
        session.state,
        crate::session::repository::SessionState::Ready
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_and_context_registers_child_session_metadata() {
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    );
    let mut config = test_config();
    config.memory.sqlite_path = isolated_sqlite_path("register-child-session");
    let session_context = SessionContext::child(
        "child-session",
        "root-session",
        crate::tools::planned_delegate_child_tool_view(),
    );

    let reply = ConversationTurnLoop::new()
        .handle_turn_with_runtime_and_context(
            &config,
            &session_context,
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            &NoopAppToolDispatcher,
            None,
        )
        .await
        .expect("handle turn success");

    assert_eq!(reply, "assistant-reply");

    let repo = crate::session::repository::SessionRepository::new(
        &crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
    )
    .expect("session repository");
    let session = repo
        .load_session("child-session")
        .expect("load session")
        .expect("session row");
    assert_eq!(
        session.kind,
        crate::session::repository::SessionKind::DelegateChild
    );
    assert_eq!(session.parent_session_id.as_deref(), Some("root-session"));
    assert_eq!(
        session.state,
        crate::session::repository::SessionState::Ready
    );
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn default_runtime_tool_view_for_resumed_child_session_respects_remaining_depth() {
    let (mut config, db_path) = isolated_test_config("resumed-child-tool-view");
    config.tools.delegate.max_depth = 2;
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child");

    let runtime = DefaultConversationRuntime;
    let child_view = runtime
        .tool_view(&config, "child-session", None)
        .expect("child tool view");
    assert!(child_view.contains("delegate"));
    assert!(child_view.contains("session_status"));
    assert!(child_view.contains("sessions_history"));
    assert!(!child_view.contains("sessions_list"));

    config.tools.delegate.max_depth = 1;
    let exhausted_view = runtime
        .tool_view(&config, "child-session", None)
        .expect("exhausted child tool view");
    assert!(!exhausted_view.contains("delegate"));
    assert!(exhausted_view.contains("session_status"));
    assert!(exhausted_view.contains("sessions_history"));
    assert!(!exhausted_view.contains("sessions_list"));
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn child_session_self_only_can_read_own_status_via_default_dispatcher() {
    let (config, db_path) = isolated_test_config("child-self-status");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Completed,
    })
    .expect("create child");

    let dispatcher = DefaultAppToolDispatcher::new(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
    );
    let session_context = SessionContext::child(
        "child-session",
        "root-session",
        crate::tools::planned_delegate_child_tool_view(),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_id": "child-session"
                }),
            },
            None,
        )
        .await
        .expect("child self status outcome");

    assert_eq!(outcome.payload["session"]["session_id"], "child-session");
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn child_session_self_only_can_read_own_history_via_default_dispatcher() {
    let (config, db_path) = isolated_test_config("child-self-history");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Completed,
    })
    .expect("create child");
    crate::memory::append_turn_direct(
        "child-session",
        "user",
        "hello from child",
        &crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
    )
    .expect("append child turn");

    let dispatcher = DefaultAppToolDispatcher::new(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
    );
    let session_context = SessionContext::child(
        "child-session",
        "root-session",
        crate::tools::planned_delegate_child_tool_view(),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "sessions_history".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "limit": 10
                }),
            },
            None,
        )
        .await
        .expect("child self history outcome");

    let turns = outcome.payload["turns"].as_array().expect("turns array");
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0]["content"], "hello from child");
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn child_session_hidden_sessions_list_is_rejected_by_default_dispatcher() {
    let (config, db_path) = isolated_test_config("child-hidden-sessions-list");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child");

    let dispatcher = DefaultAppToolDispatcher::new(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
    );
    let session_context = SessionContext::child(
        "child-session",
        "root-session",
        crate::tools::planned_delegate_child_tool_view(),
    );

    let error = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "sessions_list".to_owned(),
                payload: json!({}),
            },
            None,
        )
        .await
        .expect_err("child should not execute hidden sessions_list");

    assert!(
        error.contains("tool_not_visible: sessions_list"),
        "expected tool_not_visible for sessions_list, got: {error}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn child_session_hidden_session_wait_is_rejected_by_default_dispatcher() {
    let (config, db_path) = isolated_test_config("child-hidden-session-wait");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Completed,
    })
    .expect("create child");

    let dispatcher = DefaultAppToolDispatcher::new(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
    );
    let session_context = SessionContext::child(
        "child-session",
        "root-session",
        crate::tools::planned_delegate_child_tool_view(),
    );

    let error = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_wait".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "timeout_ms": 10
                }),
            },
            None,
        )
        .await
        .expect_err("child should not execute hidden session_wait");

    assert!(
        error.contains("tool_not_visible: session_wait"),
        "expected tool_not_visible for session_wait, got: {error}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn child_session_hidden_session_recover_is_rejected_by_default_dispatcher() {
    let (config, db_path) = isolated_test_config("child-hidden-session-recover");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child");

    let dispatcher = DefaultAppToolDispatcher::new(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
    );
    let session_context = SessionContext::child(
        "child-session",
        "root-session",
        crate::tools::planned_delegate_child_tool_view(),
    );

    let error = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_recover".to_owned(),
                payload: json!({
                    "session_id": "child-session"
                }),
            },
            None,
        )
        .await
        .expect_err("child should not execute hidden session_recover");

    assert!(
        error.contains("tool_not_visible: session_recover"),
        "expected tool_not_visible for session_recover, got: {error}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn child_session_hidden_session_cancel_is_rejected_by_default_dispatcher() {
    let (config, db_path) = isolated_test_config("child-hidden-session-cancel");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Running,
    })
    .expect("create child");

    let dispatcher = DefaultAppToolDispatcher::new(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
    );
    let session_context = SessionContext::child(
        "child-session",
        "root-session",
        crate::tools::planned_delegate_child_tool_view(),
    );

    let error = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_cancel".to_owned(),
                payload: json!({
                    "session_id": "child-session"
                }),
            },
            None,
        )
        .await
        .expect_err("child should not execute hidden session_cancel");

    assert!(
        error.contains("tool_not_visible: session_cancel"),
        "expected tool_not_visible for session_cancel, got: {error}"
    );
}

#[cfg(feature = "memory-sqlite")]
struct CancelRequestingAppToolDispatcher {
    memory_config: crate::memory::runtime_config::MemoryRuntimeConfig,
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl AppToolDispatcher for CancelRequestingAppToolDispatcher {
    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: loongclaw_contracts::ToolCoreRequest,
        _kernel_ctx: Option<&KernelContext>,
    ) -> Result<loongclaw_contracts::ToolCoreOutcome, String> {
        if request.tool_name != "session_status" {
            return Err(format!("unexpected app tool: {}", request.tool_name));
        }
        let repo = SessionRepository::new(&self.memory_config)?;
        repo.transition_session_with_event_if_current(
            &session_context.session_id,
            crate::session::repository::TransitionSessionWithEventIfCurrentRequest {
                expected_state: crate::session::repository::SessionState::Running,
                next_state: crate::session::repository::SessionState::Running,
                last_error: None,
                event_kind: "delegate_cancel_requested".to_owned(),
                actor_session_id: session_context.parent_session_id.clone(),
                event_payload_json: json!({
                    "reference": "running",
                    "cancel_reason": "operator_requested"
                }),
            },
        )?;
        Ok(loongclaw_contracts::ToolCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "cancel_requested": true
            }),
        })
    }
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_child_background_cancel_request_stops_before_next_round() {
    let (config, db_path) = isolated_test_config("delegate-child-background-cancel");
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: None,
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("bg-child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child session");

    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Inspecting child session".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name: "session_status".to_owned(),
                args_json: json!({
                    "session_id": "child-session"
                }),
                source: "provider_tool_call".to_owned(),
                session_id: "child-session".to_owned(),
                turn_id: "turn-1".to_owned(),
                tool_call_id: "call-1".to_owned(),
            }],
            raw_meta: Value::Null,
        })],
    );

    let outcome = super::run_delegate_child_turn_with_runtime(
        &ConversationTurnLoop::new(),
        &config,
        &runtime,
        &CancelRequestingAppToolDispatcher {
            memory_config: memory_config.clone(),
        },
        "child-session",
        "child task",
        60,
        None,
    )
    .await
    .expect("delegate child cancellation outcome");

    assert_eq!(outcome.status, "error");
    assert_eq!(
        outcome.payload["error"],
        "delegate_cancelled: operator_requested"
    );
    assert_eq!(
        *runtime.turn_calls.lock().expect("turn calls lock"),
        1,
        "cancel should stop before a second provider round"
    );

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(
        child.state,
        crate::session::repository::SessionState::Failed
    );
    assert_eq!(
        child.last_error.as_deref(),
        Some("delegate_cancelled: operator_requested")
    );

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list child events");
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert!(event_kinds.contains(&"delegate_started"));
    assert!(event_kinds.contains(&"delegate_cancel_requested"));
    assert!(event_kinds.contains(&"delegate_cancelled"));

    let terminal_outcome = repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(terminal_outcome.status, "error");
    assert_eq!(
        terminal_outcome.payload_json["error"],
        "delegate_cancelled: operator_requested"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn child_session_forged_root_tool_view_still_rejects_hidden_sessions_list() {
    let (config, db_path) = isolated_test_config("child-forged-root-view-sessions-list");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child");

    let dispatcher = DefaultAppToolDispatcher::new(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
    );
    let session_context = SessionContext::child(
        "child-session",
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let error = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "sessions_list".to_owned(),
                payload: json!({}),
            },
            None,
        )
        .await
        .expect_err("forged child root tool view should not expose sessions_list");

    assert!(
        error.contains("tool_not_visible: sessions_list"),
        "expected tool_not_visible for sessions_list, got: {error}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn depth_exhausted_child_forged_root_tool_view_still_rejects_delegate_async() {
    let (mut config, db_path) = isolated_test_config("child-forged-root-view-delegate-async");
    config.tools.delegate.max_depth = 1;
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child");

    let dispatcher = DefaultAppToolDispatcher::new(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
    );
    let session_context = SessionContext::child(
        "child-session",
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let error = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "delegate_async".to_owned(),
                payload: json!({
                    "task": "nested child task",
                    "timeout_seconds": 5
                }),
            },
            None,
        )
        .await
        .expect_err("depth exhausted child should not execute forged delegate_async");

    assert!(
        error.contains("tool_not_visible: delegate_async"),
        "expected tool_not_visible for delegate_async, got: {error}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn child_session_self_only_cannot_inspect_descendant_status_via_default_dispatcher() {
    let (mut config, db_path) = isolated_test_config("child-descendant-denied");
    config.tools.delegate.max_depth = 2;
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Completed,
    })
    .expect("create child");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "grandchild-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("child-session".to_owned()),
        label: Some("Grandchild".to_owned()),
        state: crate::session::repository::SessionState::Completed,
    })
    .expect("create grandchild");

    let dispatcher = DefaultAppToolDispatcher::new(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
    );
    let session_context = SessionContext::child(
        "child-session",
        "root-session",
        crate::tools::delegate_child_tool_view_for_config_with_delegate(&config.tools, true),
    );

    let error = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_status".to_owned(),
                payload: json!({
                    "session_id": "grandchild-session"
                }),
            },
            None,
        )
        .await
        .expect_err("child should not inspect descendant status");

    assert!(
        error.contains("visibility_denied"),
        "expected visibility_denied, got: {error}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn session_wait_returns_completed_for_terminal_visible_session() {
    let (config, db_path) = isolated_test_config("session-wait-completed");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Completed,
    })
    .expect("create child");
    repo.upsert_terminal_outcome(
        "child-session",
        "ok",
        json!({
            "child_session_id": "child-session",
            "final_output": "done"
        }),
    )
    .expect("upsert terminal outcome");

    let dispatcher = DefaultAppToolDispatcher::new(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_wait".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "timeout_ms": 50
                }),
            },
            None,
        )
        .await
        .expect("session_wait outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["wait_status"], "completed");
    assert_eq!(outcome.payload["session"]["session_id"], "child-session");
    assert_eq!(outcome.payload["terminal_outcome_state"], "present");
    assert!(outcome.payload["terminal_outcome_missing_reason"].is_null());
    assert_eq!(outcome.payload["terminal_outcome"]["status"], "ok");
}

#[cfg(all(feature = "memory-sqlite", feature = "channel-telegram"))]
#[tokio::test]
async fn sessions_send_delivers_to_known_root_channel_session_without_mutating_transcript() {
    let (base_url, request_rx, server) = spawn_telegram_send_server_once();
    let (mut config, db_path) = isolated_test_config("sessions-send-telegram-success");
    config.tools.messages.enabled = true;
    config.telegram.enabled = true;
    config.telegram.bot_token = Some("123456:telegram-test-token".to_owned());
    config.telegram.bot_token_env = None;
    config.telegram.base_url = base_url;
    config.telegram.allowed_chat_ids = vec![123];

    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "controller-root".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Controller".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create controller root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "telegram:123".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Telegram Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create telegram root");
    crate::memory::append_turn_direct("telegram:123", "user", "previous inbound", &memory_config)
        .expect("append prior transcript turn");
    let before_turns =
        crate::memory::window_direct("telegram:123", 10, &memory_config).expect("window turns");
    let before_summary = repo
        .load_session_summary("telegram:123")
        .expect("load target summary before send")
        .expect("target summary before send");

    let dispatcher = DefaultAppToolDispatcher::with_config(memory_config.clone(), config.clone());
    let session_context = SessionContext::root_with_tool_view(
        "controller-root",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "sessions_send".to_owned(),
                payload: json!({
                    "session_id": "telegram:123",
                    "text": "hello root channel"
                }),
            },
            None,
        )
        .await
        .expect("sessions_send outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool"], "sessions_send");
    assert_eq!(outcome.payload["session_id"], "telegram:123");
    assert_eq!(outcome.payload["channel"], "telegram");
    assert_eq!(outcome.payload["target"], "123");
    assert_eq!(outcome.payload["delivery"], "sent");
    assert_eq!(outcome.payload["text_length"], 18);

    let request = request_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("telegram request should be captured");
    assert!(request.starts_with("POST /bot123456:telegram-test-token/sendMessage "));
    assert!(request.contains("\"chat_id\":123"));
    assert!(request.contains("\"text\":\"hello root channel\""));

    let after_turns =
        crate::memory::window_direct("telegram:123", 10, &memory_config).expect("window turns");
    assert_eq!(after_turns.len(), before_turns.len());
    assert_eq!(after_turns[0].role, before_turns[0].role);
    assert_eq!(after_turns[0].content, before_turns[0].content);

    let after_summary = repo
        .load_session_summary("telegram:123")
        .expect("load target summary after send")
        .expect("target summary after send");
    assert_eq!(after_summary.turn_count, before_summary.turn_count);
    assert_eq!(after_summary.state, before_summary.state);

    let events = repo
        .list_recent_events("telegram:123", 10)
        .expect("list target events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_kind, "session_message_sent");
    assert_eq!(
        events[0].actor_session_id.as_deref(),
        Some("controller-root")
    );
    assert_eq!(events[0].payload_json["channel"], "telegram");
    assert_eq!(events[0].payload_json["target"], "123");
    assert_eq!(events[0].payload_json["text_length"], 18);
    assert_eq!(events[0].payload_json["delivery"], "sent");
    assert!(events[0].payload_json.get("text").is_none());

    server.join().expect("telegram stub join");
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn sessions_send_rejects_unknown_target_session() {
    let (mut config, db_path) = isolated_test_config("sessions-send-unknown-target");
    config.tools.messages.enabled = true;
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "controller-root".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Controller".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create controller root");

    let dispatcher = DefaultAppToolDispatcher::with_config(memory_config, config.clone());
    let session_context = SessionContext::root_with_tool_view(
        "controller-root",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let error = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "sessions_send".to_owned(),
                payload: json!({
                    "session_id": "telegram:999",
                    "text": "hello"
                }),
            },
            None,
        )
        .await
        .expect_err("unknown session target must be rejected");

    assert!(
        error.contains("session_not_found: `telegram:999`"),
        "expected session_not_found error, got: {error}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn sessions_send_rejects_delegate_child_target() {
    let (mut config, db_path) = isolated_test_config("sessions-send-child-target");
    config.tools.messages.enabled = true;
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "controller-root".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Controller".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create controller root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "telegram:123".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("controller-root".to_owned()),
        label: Some("Pretend Child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child target");

    let dispatcher = DefaultAppToolDispatcher::with_config(memory_config, config.clone());
    let session_context = SessionContext::root_with_tool_view(
        "controller-root",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let error = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "sessions_send".to_owned(),
                payload: json!({
                    "session_id": "telegram:123",
                    "text": "hello"
                }),
            },
            None,
        )
        .await
        .expect_err("delegate child target must be rejected");

    assert!(
        error.contains("sessions_send_not_supported") && error.contains("not a root session"),
        "expected root-session rejection, got: {error}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn sessions_send_rejects_target_not_in_channel_allowlist() {
    let (mut config, db_path) = isolated_test_config("sessions-send-disallowed-target");
    config.tools.messages.enabled = true;
    config.telegram.enabled = true;
    config.telegram.bot_token = Some("123456:telegram-test-token".to_owned());
    config.telegram.bot_token_env = None;
    config.telegram.allowed_chat_ids = vec![456];

    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "controller-root".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Controller".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create controller root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "telegram:123".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Telegram Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create telegram root");

    let dispatcher = DefaultAppToolDispatcher::with_config(memory_config, config.clone());
    let session_context = SessionContext::root_with_tool_view(
        "controller-root",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let error = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "sessions_send".to_owned(),
                payload: json!({
                    "session_id": "telegram:123",
                    "text": "hello"
                }),
            },
            None,
        )
        .await
        .expect_err("disallowed target must be rejected");

    assert!(
        error.contains("sessions_send_target_not_allowed"),
        "expected allowlist rejection, got: {error}"
    );
}

#[cfg(all(feature = "memory-sqlite", feature = "channel-telegram"))]
#[tokio::test]
async fn sessions_send_materializes_legacy_root_session_before_recording_event() {
    let (base_url, request_rx, server) = spawn_telegram_send_server_once();
    let (mut config, db_path) = isolated_test_config("sessions-send-legacy-target");
    config.tools.messages.enabled = true;
    config.telegram.enabled = true;
    config.telegram.bot_token = Some("123456:telegram-test-token".to_owned());
    config.telegram.bot_token_env = None;
    config.telegram.base_url = base_url;
    config.telegram.allowed_chat_ids = vec![123];

    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "controller-root".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Controller".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create controller root");
    crate::memory::append_turn_direct("telegram:123", "user", "legacy inbound", &memory_config)
        .expect("append legacy transcript turn");
    assert!(repo
        .load_session("telegram:123")
        .expect("load target row before send")
        .is_none());
    assert!(repo
        .load_session_summary_with_legacy_fallback("telegram:123")
        .expect("load legacy summary")
        .is_some());

    let dispatcher = DefaultAppToolDispatcher::with_config(memory_config.clone(), config.clone());
    let session_context = SessionContext::root_with_tool_view(
        "controller-root",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "sessions_send".to_owned(),
                payload: json!({
                    "session_id": "telegram:123",
                    "text": "hello legacy"
                }),
            },
            None,
        )
        .await
        .expect("legacy target sessions_send outcome");

    assert_eq!(outcome.status, "ok");
    let target_row = repo
        .load_session("telegram:123")
        .expect("load target row after send")
        .expect("target row should be materialized");
    assert_eq!(
        target_row.kind,
        crate::session::repository::SessionKind::Root
    );

    let events = repo
        .list_recent_events("telegram:123", 10)
        .expect("list target events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_kind, "session_message_sent");

    let request = request_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("telegram request should be captured");
    assert!(request.contains("\"text\":\"hello legacy\""));

    server.join().expect("telegram stub join");
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn child_session_hidden_sessions_send_is_rejected_by_default_dispatcher() {
    let (mut config, db_path) = isolated_test_config("child-hidden-sessions-send");
    config.tools.messages.enabled = true;
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child");

    let dispatcher = DefaultAppToolDispatcher::with_config(memory_config, config.clone());
    let session_context = SessionContext::child(
        "child-session",
        "root-session",
        crate::tools::delegate_child_tool_view_for_config(&config.tools),
    );

    let error = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "sessions_send".to_owned(),
                payload: json!({
                    "session_id": "telegram:123",
                    "text": "hello"
                }),
            },
            None,
        )
        .await
        .expect_err("child should not execute hidden sessions_send");

    assert!(
        error.contains("tool_not_visible: sessions_send"),
        "expected tool_not_visible for sessions_send, got: {error}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn session_wait_times_out_for_non_terminal_session() {
    let (config, db_path) = isolated_test_config("session-wait-timeout");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Running,
    })
    .expect("create child");

    let dispatcher = DefaultAppToolDispatcher::new(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_wait".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "timeout_ms": 10
                }),
            },
            None,
        )
        .await
        .expect("session_wait outcome");

    assert_eq!(outcome.status, "timeout");
    assert_eq!(outcome.payload["wait_status"], "timeout");
    assert_eq!(outcome.payload["session"]["state"], "running");
    assert_eq!(outcome.payload["terminal_outcome_state"], "not_terminal");
    assert!(outcome.payload["terminal_outcome_missing_reason"].is_null());
    assert!(outcome.payload["terminal_outcome"].is_null());
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn session_wait_batch_returns_mixed_completed_timeout_and_hidden_results() {
    let (config, db_path) = isolated_test_config("session-wait-batch");
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "completed-child".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Completed".to_owned()),
        state: crate::session::repository::SessionState::Completed,
    })
    .expect("create completed child");
    repo.upsert_terminal_outcome(
        "completed-child",
        "ok",
        json!({
            "child_session_id": "completed-child",
            "final_output": "done"
        }),
    )
    .expect("upsert terminal outcome");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "running-child".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Running".to_owned()),
        state: crate::session::repository::SessionState::Running,
    })
    .expect("create running child");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "hidden-root".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Hidden".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create hidden root");

    let dispatcher = DefaultAppToolDispatcher::new(memory_config, config.tools.clone());
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_wait".to_owned(),
                payload: json!({
                    "session_ids": ["hidden-root", "completed-child", "running-child"],
                    "timeout_ms": 10
                }),
            },
            None,
        )
        .await
        .expect("session_wait batch outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool"], "session_wait");
    assert_eq!(outcome.payload["current_session_id"], "root-session");
    assert_eq!(outcome.payload["requested_count"], 3);
    assert_eq!(outcome.payload["timeout_ms"], 10);
    assert!(outcome.payload["after_id"].is_null());
    assert_eq!(outcome.payload["result_counts"]["ok"], 1);
    assert_eq!(outcome.payload["result_counts"]["timeout"], 1);
    assert_eq!(outcome.payload["result_counts"]["skipped_not_visible"], 1);

    let results = outcome.payload["results"]
        .as_array()
        .expect("batch results array");
    let ids: Vec<&str> = results
        .iter()
        .filter_map(|item| item.get("session_id"))
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(ids, vec!["hidden-root", "completed-child", "running-child"]);

    let hidden = results
        .iter()
        .find(|item| item["session_id"] == "hidden-root")
        .expect("hidden batch result");
    assert_eq!(hidden["result"], "skipped_not_visible");
    assert!(hidden["inspection"].is_null());
    assert!(hidden["message"]
        .as_str()
        .expect("hidden message")
        .contains("visibility_denied"));

    let completed = results
        .iter()
        .find(|item| item["session_id"] == "completed-child")
        .expect("completed batch result");
    assert_eq!(completed["result"], "ok");
    assert_eq!(completed["inspection"]["wait_status"], "completed");
    assert_eq!(completed["inspection"]["session"]["state"], "completed");
    assert_eq!(completed["inspection"]["terminal_outcome"]["status"], "ok");

    let running = results
        .iter()
        .find(|item| item["session_id"] == "running-child")
        .expect("running batch result");
    assert_eq!(running["result"], "timeout");
    assert_eq!(running["inspection"]["wait_status"], "timeout");
    assert_eq!(running["inspection"]["session"]["state"], "running");
    assert!(running["inspection"]["terminal_outcome"].is_null());
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn session_wait_reports_delegate_lifecycle_for_queued_child() {
    let (config, db_path) = isolated_test_config("session-wait-delegate-lifecycle");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child");
    repo.append_event(crate::session::repository::NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "label": "Child",
            "timeout_seconds": 30
        }),
    })
    .expect("append queued event");

    let dispatcher = DefaultAppToolDispatcher::new(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_wait".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "timeout_ms": 10
                }),
            },
            None,
        )
        .await
        .expect("session_wait outcome");

    assert_eq!(outcome.status, "timeout");
    assert_eq!(outcome.payload["wait_status"], "timeout");
    assert_eq!(outcome.payload["session"]["state"], "ready");
    assert_eq!(outcome.payload["delegate_lifecycle"]["mode"], "async");
    assert_eq!(outcome.payload["delegate_lifecycle"]["phase"], "queued");
    assert_eq!(outcome.payload["delegate_lifecycle"]["timeout_seconds"], 30);
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["staleness"]["reference"],
        "queued"
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["staleness"]["state"],
        "fresh"
    );
    assert!(outcome.payload["delegate_lifecycle"]["queued_at"].is_number());
    assert!(outcome.payload["delegate_lifecycle"]["started_at"].is_null());
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn session_wait_reports_pending_cancel_request_for_running_child() {
    let (config, db_path) = isolated_test_config("session-wait-cancel-requested");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Running,
    })
    .expect("create child");
    repo.append_event(crate::session::repository::NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_queued".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "timeout_seconds": 60
        }),
    })
    .expect("append queued event");
    repo.append_event(crate::session::repository::NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "research",
            "timeout_seconds": 60
        }),
    })
    .expect("append started event");
    repo.append_event(crate::session::repository::NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_cancel_requested".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "reference": "running",
            "cancel_reason": "operator_requested"
        }),
    })
    .expect("append cancel requested event");

    let dispatcher = DefaultAppToolDispatcher::new(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_wait".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "timeout_ms": 10
                }),
            },
            None,
        )
        .await
        .expect("session_wait outcome");

    assert_eq!(outcome.status, "timeout");
    assert_eq!(outcome.payload["wait_status"], "timeout");
    assert_eq!(outcome.payload["delegate_lifecycle"]["phase"], "running");
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["cancellation"]["state"],
        "requested"
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["cancellation"]["reference"],
        "running"
    );
    assert_eq!(
        outcome.payload["delegate_lifecycle"]["cancellation"]["reason"],
        "operator_requested"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn session_wait_reports_missing_terminal_outcome_for_recovered_failed_session() {
    let (config, db_path) = isolated_test_config("session-wait-recovered-failed");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Failed,
    })
    .expect("create child");
    repo.update_session_state(
        "child-session",
        crate::session::repository::SessionState::Failed,
        Some("opaque_recovery_failure".to_owned()),
    )
    .expect("update child status");
    repo.append_event(crate::session::repository::NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_recovery_applied".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "recovery_kind": "async_spawn_failure_persist_failed",
            "recovered_state": "failed",
            "recovery_error": "delegate_async_spawn_failure_persist_failed: sqlite_busy; original spawn error: boom",
            "original_error": "boom"
        }),
    })
    .expect("append failed event");

    let dispatcher = DefaultAppToolDispatcher::new(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_wait".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "timeout_ms": 50
                }),
            },
            None,
        )
        .await
        .expect("session_wait outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["wait_status"], "completed");
    assert_eq!(outcome.payload["session"]["state"], "failed");
    assert_eq!(outcome.payload["terminal_outcome_state"], "missing");
    assert_eq!(
        outcome.payload["terminal_outcome_missing_reason"],
        "async_spawn_failure_persist_failed"
    );
    assert_eq!(
        outcome.payload["recovery"]["kind"],
        "async_spawn_failure_persist_failed"
    );
    assert_eq!(
        outcome.payload["recovery"]["event_kind"],
        "delegate_recovery_applied"
    );
    assert_eq!(outcome.payload["recovery"]["original_error"], "boom");
    assert_eq!(outcome.payload["recovery"]["source"], "event");
    assert!(outcome.payload["terminal_outcome"].is_null());
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn session_wait_synthesizes_recovery_from_last_error_when_event_missing() {
    let (config, db_path) = isolated_test_config("session-wait-recovery-fallback");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Failed,
    })
    .expect("create child");
    repo.update_session_state(
        "child-session",
        crate::session::repository::SessionState::Failed,
        Some(
            "delegate_async_spawn_failure_persist_failed: sqlite_busy; original spawn error: boom"
                .to_owned(),
        ),
    )
    .expect("update child status");

    let dispatcher = DefaultAppToolDispatcher::new(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_wait".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "timeout_ms": 50
                }),
            },
            None,
        )
        .await
        .expect("session_wait outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["wait_status"], "completed");
    assert_eq!(outcome.payload["terminal_outcome_state"], "missing");
    assert_eq!(
        outcome.payload["terminal_outcome_missing_reason"],
        "async_spawn_failure_persist_failed"
    );
    assert_eq!(
        outcome.payload["recovery"]["kind"],
        "async_spawn_failure_persist_failed"
    );
    assert_eq!(outcome.payload["recovery"]["source"], "last_error");
    assert_eq!(
        outcome.payload["recovery"]["recovery_error"],
        "delegate_async_spawn_failure_persist_failed: sqlite_busy; original spawn error: boom"
    );
    assert!(outcome.payload["recovery"]["event_kind"].is_null());
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn session_wait_synthesizes_unknown_recovery_when_metadata_missing() {
    let (config, db_path) = isolated_test_config("session-wait-recovery-unknown");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Failed,
    })
    .expect("create child");

    let dispatcher = DefaultAppToolDispatcher::new(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_wait".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "timeout_ms": 50
                }),
            },
            None,
        )
        .await
        .expect("session_wait outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["wait_status"], "completed");
    assert_eq!(outcome.payload["terminal_outcome_state"], "missing");
    assert_eq!(
        outcome.payload["terminal_outcome_missing_reason"],
        "unknown"
    );
    assert_eq!(outcome.payload["recovery"]["kind"], "unknown");
    assert_eq!(outcome.payload["recovery"]["source"], "none");
    assert!(outcome.payload["recovery"]["recovery_error"].is_null());
    assert!(outcome.payload["recovery"]["event_kind"].is_null());
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn session_wait_returns_incremental_events_after_after_id_when_session_completes() {
    let (config, db_path) = isolated_test_config("session-wait-after-id-completed");
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Running,
    })
    .expect("create child");
    let started_event = repo
        .append_event(crate::session::repository::NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "child task"
            }),
        })
        .expect("append started event");

    let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), config.tools.clone());
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let finalize_memory_config = memory_config.clone();
    let finalize_task = tokio::spawn(async move {
        sleep(Duration::from_millis(10)).await;
        let finalize_repo =
            SessionRepository::new(&finalize_memory_config).expect("finalize session repository");
        finalize_repo
            .finalize_session_terminal(
                "child-session",
                crate::session::repository::FinalizeSessionTerminalRequest {
                    state: crate::session::repository::SessionState::Completed,
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
                },
            )
            .expect("finalize child session")
    });

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_wait".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "timeout_ms": 100,
                    "after_id": started_event.id
                }),
            },
            None,
        )
        .await
        .expect("session_wait outcome");

    finalize_task.await.expect("join finalize task");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["wait_status"], "completed");
    assert_eq!(outcome.payload["after_id"], started_event.id);
    assert_eq!(outcome.payload["session"]["state"], "completed");
    assert_eq!(outcome.payload["terminal_outcome"]["status"], "ok");
    let events = outcome.payload["events"]
        .as_array()
        .expect("session_wait events array");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event_kind"], "delegate_completed");
    assert!(
        outcome.payload["next_after_id"]
            .as_i64()
            .expect("next_after_id")
            > started_event.id
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn session_wait_timeout_returns_incremental_events_observed_while_waiting() {
    let (config, db_path) = isolated_test_config("session-wait-after-id-timeout");
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Running,
    })
    .expect("create child");

    let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), config.tools.clone());
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let append_memory_config = memory_config.clone();
    let append_task = tokio::spawn(async move {
        sleep(Duration::from_millis(10)).await;
        let append_repo =
            SessionRepository::new(&append_memory_config).expect("append session repository");
        append_repo
            .append_event(crate::session::repository::NewSessionEvent {
                session_id: "child-session".to_owned(),
                event_kind: "delegate_started".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                payload_json: json!({
                    "task": "child task"
                }),
            })
            .expect("append started event")
    });

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_wait".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "timeout_ms": 100,
                    "after_id": 0
                }),
            },
            None,
        )
        .await
        .expect("session_wait outcome");

    let started_event = append_task.await.expect("join append task");

    assert_eq!(outcome.status, "timeout");
    assert_eq!(outcome.payload["wait_status"], "timeout");
    assert_eq!(outcome.payload["after_id"], 0);
    assert_eq!(outcome.payload["session"]["state"], "running");
    assert!(outcome.payload["terminal_outcome"].is_null());
    let events = outcome.payload["events"]
        .as_array()
        .expect("session_wait events array");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event_kind"], "delegate_started");
    assert_eq!(events[0]["id"], started_event.id);
    assert_eq!(outcome.payload["next_after_id"], started_event.id);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn session_wait_after_id_on_terminal_session_drains_until_terminal_event() {
    let (config, db_path) = isolated_test_config("session-wait-terminal-drain");
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Running,
    })
    .expect("create child");

    let mut last_event_id = 0_i64;
    for index in 0..60 {
        last_event_id = repo
            .append_event(crate::session::repository::NewSessionEvent {
                session_id: "child-session".to_owned(),
                event_kind: format!("delegate_progress_{index}"),
                actor_session_id: Some("root-session".to_owned()),
                payload_json: json!({
                    "step": index
                }),
            })
            .expect("append progress event")
            .id;
    }
    let finalized = repo
        .finalize_session_terminal(
            "child-session",
            crate::session::repository::FinalizeSessionTerminalRequest {
                state: crate::session::repository::SessionState::Completed,
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
            },
        )
        .expect("finalize child session");

    let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), config.tools.clone());
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_wait".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "timeout_ms": 100,
                    "after_id": 0
                }),
            },
            None,
        )
        .await
        .expect("session_wait outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["wait_status"], "completed");
    assert_eq!(outcome.payload["session"]["state"], "completed");
    let events = outcome.payload["events"]
        .as_array()
        .expect("session_wait events array");
    assert_eq!(events.len(), 61);
    assert_eq!(events[0]["id"], 1);
    assert_eq!(
        events.last().expect("last session_wait event")["event_kind"],
        "delegate_completed"
    );
    assert_eq!(
        outcome.payload["next_after_id"],
        finalized.event.id.max(last_event_id)
    );
    assert_eq!(outcome.payload["terminal_outcome"]["status"], "ok");
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_async_queue_returns_handle_and_records_queued_event() {
    let (config, db_path) = isolated_test_config("delegate-async-queued");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");

    let spawner = Arc::new(FakeAsyncDelegateSpawner::default());
    let dispatcher = DefaultAppToolDispatcher::with_async_delegate_spawner(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
        spawner.clone(),
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "delegate_async".to_owned(),
                payload: json!({
                    "task": "child task",
                    "label": "async-child",
                    "timeout_seconds": 9
                }),
            },
            None,
        )
        .await
        .expect("delegate_async outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["mode"], "async");
    assert_eq!(outcome.payload["state"], "queued");
    assert_eq!(outcome.payload["label"], "async-child");
    let child_session_id = outcome.payload["child_session_id"]
        .as_str()
        .expect("child_session_id")
        .to_owned();
    assert!(child_session_id.starts_with("delegate:"));

    let requests = tokio::time::timeout(Duration::from_millis(250), async {
        loop {
            let maybe_requests = {
                let requests = spawner
                    .requests
                    .lock()
                    .expect("async delegate requests lock");
                (requests.len() == 1).then(|| requests.clone())
            };
            if let Some(requests) = maybe_requests {
                break requests;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("async delegate request should be dispatched");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].child_session_id, child_session_id);
    assert_eq!(requests[0].parent_session_id, "root-session");
    assert_eq!(requests[0].task, "child task");
    assert_eq!(requests[0].label.as_deref(), Some("async-child"));
    assert_eq!(requests[0].timeout_seconds, 9);

    let child = repo
        .load_session_summary(&child_session_id)
        .expect("load child summary")
        .expect("child summary");
    assert_eq!(child.state, crate::session::repository::SessionState::Ready);
    assert_eq!(child.label.as_deref(), Some("async-child"));
    let events = repo
        .list_recent_events(&child_session_id, 10)
        .expect("list child events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_kind, "delegate_queued");
    assert!(repo
        .load_terminal_outcome(&child_session_id)
        .expect("load terminal outcome")
        .is_none());
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_async_returns_handle_without_waiting_for_spawn_completion() {
    let (config, db_path) = isolated_test_config("delegate-async-immediate-return");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");

    let (spawner, request_rx, release_notify) = GatedFakeAsyncDelegateSpawner::new();
    let dispatcher = DefaultAppToolDispatcher::with_async_delegate_spawner(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
        Arc::new(spawner),
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let queued_dispatcher = dispatcher.clone();
    let queued_session_context = session_context.clone();
    let queued_call = tokio::spawn(async move {
        queued_dispatcher
            .execute_app_tool(
                &queued_session_context,
                loongclaw_contracts::ToolCoreRequest {
                    tool_name: "delegate_async".to_owned(),
                    payload: json!({
                        "task": "child task",
                        "label": "async-child",
                        "timeout_seconds": 9
                    }),
                },
                None,
            )
            .await
    });

    let spawn_request = tokio::time::timeout(Duration::from_millis(250), request_rx)
        .await
        .expect("delegate_async should dispatch spawn quickly")
        .expect("gated async delegate spawn request");
    let queued = tokio::time::timeout(Duration::from_millis(250), queued_call)
        .await
        .expect("delegate_async should return handle without waiting for spawn gate")
        .expect("join queued task")
        .expect("delegate_async outcome");

    assert_eq!(queued.status, "ok");
    assert_eq!(queued.payload["mode"], "async");
    assert_eq!(queued.payload["state"], "queued");
    assert_eq!(queued.payload["label"], "async-child");
    let child_session_id = queued.payload["child_session_id"]
        .as_str()
        .expect("child_session_id")
        .to_owned();
    assert_eq!(spawn_request.child_session_id, child_session_id);
    assert_eq!(spawn_request.parent_session_id, "root-session");
    assert_eq!(spawn_request.task, "child task");
    assert_eq!(spawn_request.label.as_deref(), Some("async-child"));
    assert_eq!(spawn_request.timeout_seconds, 9);

    let child = repo
        .load_session_summary(&child_session_id)
        .expect("load child summary")
        .expect("child summary");
    assert_eq!(child.state, crate::session::repository::SessionState::Ready);
    assert_eq!(child.label.as_deref(), Some("async-child"));
    let events = repo
        .list_recent_events(&child_session_id, 10)
        .expect("list child events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_kind, "delegate_queued");
    assert!(repo
        .load_terminal_outcome(&child_session_id)
        .expect("load terminal outcome")
        .is_none());

    release_notify.notify_waiters();
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_async_queue_failure_does_not_leave_orphan_child_session() {
    let (config, db_path) = isolated_test_config("delegate-async-queue-rollback");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");

    let spawner = Arc::new(FakeAsyncDelegateSpawner::default());
    let dispatcher = DefaultAppToolDispatcher::with_async_delegate_spawner(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
        spawner.clone(),
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let conn = Connection::open(&db_path).expect("open sqlite connection");
    conn.execute(
        "CREATE TRIGGER fail_delegate_queue_event
         BEFORE INSERT ON session_events
         BEGIN
            SELECT RAISE(FAIL, 'forced delegate queue failure');
         END;",
        [],
    )
    .expect("create session_events failure trigger");

    let error = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "delegate_async".to_owned(),
                payload: json!({
                    "task": "child task",
                    "label": "async-child"
                }),
            },
            None,
        )
        .await
        .expect_err("delegate_async should fail when queued event cannot be written");
    assert!(error.contains("insert session event failed"));

    let sessions = repo
        .list_sessions()
        .expect("list sessions after queue failure");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "root-session");
    assert_eq!(
        spawner
            .requests
            .lock()
            .expect("async delegate requests lock")
            .len(),
        0
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_async_spawn_failure_is_observable_after_queued_handle_returns() {
    let (config, db_path) = isolated_test_config("delegate-async-spawn-failed");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");

    let spawner = Arc::new(FakeAsyncDelegateSpawner {
        requests: Arc::new(Mutex::new(Vec::new())),
        spawn_error: Some("spawn unavailable".to_owned()),
    });
    let dispatcher = DefaultAppToolDispatcher::with_async_delegate_spawner(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
        spawner,
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "delegate_async".to_owned(),
                payload: json!({
                    "task": "child task",
                    "label": "async-child"
                }),
            },
            None,
        )
        .await
        .expect("delegate_async outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["mode"], "async");
    assert_eq!(outcome.payload["state"], "queued");
    let child_session_id = outcome.payload["child_session_id"]
        .as_str()
        .expect("child_session_id");

    let waited = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_wait".to_owned(),
                payload: json!({
                    "session_id": child_session_id,
                    "timeout_ms": 500
                }),
            },
            None,
        )
        .await
        .expect("session_wait outcome");

    assert_eq!(waited.status, "ok");
    assert_eq!(waited.payload["wait_status"], "completed");
    assert_eq!(waited.payload["session"]["state"], "failed");
    assert_eq!(waited.payload["terminal_outcome"]["status"], "error");
    assert_eq!(
        waited.payload["terminal_outcome"]["payload"]["error"],
        "spawn unavailable"
    );

    let child = repo
        .load_session_summary(child_session_id)
        .expect("load child summary")
        .expect("child summary");
    assert_eq!(
        child.state,
        crate::session::repository::SessionState::Failed
    );
    assert_eq!(child.last_error.as_deref(), Some("spawn unavailable"));
    let events = repo
        .list_recent_events(child_session_id, 10)
        .expect("list child events");
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert!(event_kinds.contains(&"delegate_queued"));
    assert!(event_kinds.contains(&"delegate_spawn_failed"));
    let terminal_outcome = repo
        .load_terminal_outcome(child_session_id)
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(terminal_outcome.status, "error");
    assert_eq!(terminal_outcome.payload_json["error"], "spawn unavailable");
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_async_spawn_panic_is_observable_after_queued_handle_returns() {
    let (config, db_path) = isolated_test_config("delegate-async-spawn-panic");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");

    let dispatcher = DefaultAppToolDispatcher::with_async_delegate_spawner(
        crate::memory::runtime_config::MemoryRuntimeConfig {
            sqlite_path: Some(config.memory.resolved_sqlite_path()),
        },
        config.tools.clone(),
        Arc::new(PanicAsyncDelegateSpawner),
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "delegate_async".to_owned(),
                payload: json!({
                    "task": "child task",
                    "label": "async-child"
                }),
            },
            None,
        )
        .await
        .expect("delegate_async outcome");

    assert_eq!(outcome.status, "ok");
    let child_session_id = outcome.payload["child_session_id"]
        .as_str()
        .expect("child_session_id");

    let waited = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_wait".to_owned(),
                payload: json!({
                    "session_id": child_session_id,
                    "timeout_ms": 500
                }),
            },
            None,
        )
        .await
        .expect("session_wait outcome");

    assert_eq!(waited.status, "ok");
    assert_eq!(waited.payload["wait_status"], "completed");
    assert_eq!(waited.payload["session"]["state"], "failed");
    assert_eq!(waited.payload["terminal_outcome"]["status"], "error");
    assert_eq!(
        waited.payload["terminal_outcome"]["payload"]["error"],
        "delegate_async_spawn_panic: panic-async-spawn"
    );

    let child = repo
        .load_session_summary(child_session_id)
        .expect("load child summary")
        .expect("child summary");
    assert_eq!(
        child.state,
        crate::session::repository::SessionState::Failed
    );
    assert_eq!(
        child.last_error.as_deref(),
        Some("delegate_async_spawn_panic: panic-async-spawn")
    );
    let events = repo
        .list_recent_events(child_session_id, 10)
        .expect("list child events");
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert!(event_kinds.contains(&"delegate_queued"));
    assert!(event_kinds.contains(&"delegate_spawn_failed"));
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_async_spawn_failure_persistence_failure_recovers_to_failed_state() {
    let (config, db_path) = isolated_test_config("delegate-async-spawn-failure-persistence");
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root");

    let conn = Connection::open(&db_path).expect("open sqlite connection");
    conn.execute(
        "CREATE TRIGGER fail_async_spawn_terminal_outcome
         BEFORE INSERT ON session_terminal_outcomes
         BEGIN
            SELECT RAISE(FAIL, 'forced async spawn terminal outcome failure');
         END;",
        [],
    )
    .expect("create terminal outcome failure trigger");

    let spawner = Arc::new(FakeAsyncDelegateSpawner {
        requests: Arc::new(Mutex::new(Vec::new())),
        spawn_error: Some("spawn unavailable".to_owned()),
    });
    let dispatcher = DefaultAppToolDispatcher::with_async_delegate_spawner(
        memory_config.clone(),
        config.tools.clone(),
        spawner.clone(),
    );
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );

    let outcome = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "delegate_async".to_owned(),
                payload: json!({
                    "task": "child task",
                    "label": "async-child"
                }),
            },
            None,
        )
        .await
        .expect("delegate_async outcome");
    let child_session_id = outcome.payload["child_session_id"]
        .as_str()
        .expect("child_session_id")
        .to_owned();

    let child = tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            let child = repo
                .load_session_summary(&child_session_id)
                .expect("load child summary")
                .expect("child summary");
            if child.state == crate::session::repository::SessionState::Failed {
                break child;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("child should recover to failed state");

    assert!(child
        .last_error
        .as_deref()
        .expect("child last_error")
        .contains("delegate_async_spawn_failure_persist_failed"));
    let events = repo
        .list_recent_events(&child_session_id, 10)
        .expect("list child events");
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert!(event_kinds.contains(&"delegate_queued"));
    assert!(!event_kinds.contains(&"delegate_spawn_failed"));
    let recovery_event = events
        .iter()
        .find(|event| event.event_kind == "delegate_recovery_applied")
        .expect("delegate recovery event");
    assert_eq!(
        recovery_event.payload_json["recovery_kind"],
        "async_spawn_failure_persist_failed"
    );
    assert_eq!(recovery_event.payload_json["recovered_state"], "failed");
    assert_eq!(
        recovery_event.payload_json["original_error"],
        "spawn unavailable"
    );
    assert!(repo
        .load_terminal_outcome(&child_session_id)
        .expect("load terminal outcome")
        .is_none());
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_child_background_success_persists_terminal_outcome() {
    let (config, db_path) = isolated_test_config("delegate-child-background-success");
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: None,
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("bg-child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child session");

    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Child final output".to_owned(),
            tool_intents: vec![],
            raw_meta: Value::Null,
        })],
    );

    let outcome = super::run_delegate_child_turn_with_runtime(
        &ConversationTurnLoop::new(),
        &config,
        &runtime,
        &NoopAppToolDispatcher,
        "child-session",
        "child task",
        60,
        None,
    )
    .await
    .expect("delegate child background outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["child_session_id"], "child-session");
    assert_eq!(outcome.payload["final_output"], "Child final output");

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(
        child.state,
        crate::session::repository::SessionState::Completed
    );
    assert_eq!(child.last_error, None);

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
        "Child final output"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_child_background_terminal_finalize_failure_recovers_to_failed_state() {
    let (config, db_path) = isolated_test_config("delegate-child-background-finalize-failure");
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: None,
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("bg-child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child session");

    let conn = Connection::open(&db_path).expect("open sqlite connection");
    conn.execute(
        "CREATE TRIGGER fail_child_terminal_outcome
         BEFORE INSERT ON session_terminal_outcomes
         BEGIN
            SELECT RAISE(FAIL, 'forced child terminal outcome failure');
         END;",
        [],
    )
    .expect("create terminal outcome failure trigger");

    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Child final output".to_owned(),
            tool_intents: vec![],
            raw_meta: Value::Null,
        })],
    );

    let error = super::run_delegate_child_turn_with_runtime(
        &ConversationTurnLoop::new(),
        &config,
        &runtime,
        &NoopAppToolDispatcher,
        "child-session",
        "child task",
        60,
        None,
    )
    .await
    .expect_err("terminal finalize failure should surface as error");

    assert!(
        error.contains("delegate_terminal_finalize_failed"),
        "error: {error}"
    );

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(
        child.state,
        crate::session::repository::SessionState::Failed
    );
    assert!(child
        .last_error
        .as_deref()
        .expect("child last_error")
        .contains("delegate_terminal_finalize_failed"));

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list child events");
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert!(event_kinds.contains(&"delegate_started"));
    assert!(!event_kinds.contains(&"delegate_completed"));
    let recovery_event = events
        .iter()
        .find(|event| event.event_kind == "delegate_recovery_applied")
        .expect("delegate recovery event");
    assert_eq!(
        recovery_event.payload_json["recovery_kind"],
        "terminal_finalize_persist_failed"
    );
    assert_eq!(recovery_event.payload_json["recovered_state"], "failed");
    assert_eq!(
        recovery_event.payload_json["attempted_terminal_event_kind"],
        "delegate_completed"
    );

    assert!(repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome")
        .is_none());
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_child_background_terminal_finalize_failure_falls_back_when_recovery_event_persist_fails(
) {
    let (config, db_path) =
        isolated_test_config("delegate-child-background-finalize-recovery-fallback");
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: None,
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("bg-child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child session");

    let conn = Connection::open(&db_path).expect("open sqlite connection");
    conn.execute(
        "CREATE TRIGGER fail_child_terminal_outcome
         BEFORE INSERT ON session_terminal_outcomes
         BEGIN
            SELECT RAISE(FAIL, 'forced child terminal outcome failure');
         END;",
        [],
    )
    .expect("create terminal outcome failure trigger");
    conn.execute(
        "CREATE TRIGGER fail_delegate_recovery_event
         BEFORE INSERT ON session_events
         WHEN NEW.event_kind = 'delegate_recovery_applied'
         BEGIN
            SELECT RAISE(FAIL, 'forced delegate recovery event failure');
         END;",
        [],
    )
    .expect("create recovery event failure trigger");

    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Child final output".to_owned(),
            tool_intents: vec![],
            raw_meta: Value::Null,
        })],
    );

    let error = super::run_delegate_child_turn_with_runtime(
        &ConversationTurnLoop::new(),
        &config,
        &runtime,
        &NoopAppToolDispatcher,
        "child-session",
        "child task",
        60,
        None,
    )
    .await
    .expect_err("terminal finalize failure should surface as error");

    assert!(
        error.contains("delegate_terminal_recovery_event_failed"),
        "error: {error}"
    );

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(
        child.state,
        crate::session::repository::SessionState::Failed
    );
    assert!(child
        .last_error
        .as_deref()
        .expect("child last_error")
        .contains("delegate_terminal_finalize_failed"));

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list child events");
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert!(event_kinds.contains(&"delegate_started"));
    assert!(!event_kinds.contains(&"delegate_completed"));
    assert!(!event_kinds.contains(&"delegate_recovery_applied"));

    let dispatcher = DefaultAppToolDispatcher::new(memory_config.clone(), config.tools.clone());
    let session_context = SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::runtime_tool_view_for_config(&config.tools),
    );
    let waited = dispatcher
        .execute_app_tool(
            &session_context,
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "session_wait".to_owned(),
                payload: json!({
                    "session_id": "child-session",
                    "timeout_ms": 50
                }),
            },
            None,
        )
        .await
        .expect("session_wait outcome");

    assert_eq!(waited.payload["terminal_outcome_state"], "missing");
    assert_eq!(
        waited.payload["terminal_outcome_missing_reason"],
        "terminal_finalize_persist_failed"
    );
    assert_eq!(
        waited.payload["recovery"]["kind"],
        "terminal_finalize_persist_failed"
    );
    assert_eq!(waited.payload["recovery"]["source"], "last_error");
    assert!(waited.payload["recovery"]["event_kind"].is_null());

    assert!(repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome")
        .is_none());
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_child_background_rerun_does_not_overwrite_terminal_outcome() {
    let (config, db_path) = isolated_test_config("delegate-child-background-rerun");
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: None,
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("bg-child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child session");

    let first_runtime = FakeRuntime::with_turns(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Child final output".to_owned(),
            tool_intents: vec![],
            raw_meta: Value::Null,
        })],
    );

    let first_outcome = super::run_delegate_child_turn_with_runtime(
        &ConversationTurnLoop::new(),
        &config,
        &first_runtime,
        &NoopAppToolDispatcher,
        "child-session",
        "child task",
        60,
        None,
    )
    .await
    .expect("first delegate child background outcome");
    assert_eq!(first_outcome.status, "ok");

    let second_runtime =
        FakeRuntime::with_turns(vec![], vec![Err("child runtime failed".to_owned())]);

    let second_error = super::run_delegate_child_turn_with_runtime(
        &ConversationTurnLoop::new(),
        &config,
        &second_runtime,
        &NoopAppToolDispatcher,
        "child-session",
        "child task retry",
        60,
        None,
    )
    .await
    .expect_err("rerun should be rejected after terminal completion");
    assert!(second_error.contains("not runnable"));
    assert!(second_error.contains("completed"));

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(
        child.state,
        crate::session::repository::SessionState::Completed
    );
    assert_eq!(child.last_error, None);

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list child events");
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert_eq!(
        event_kinds
            .iter()
            .filter(|kind| **kind == "delegate_started")
            .count(),
        1
    );
    assert_eq!(
        event_kinds
            .iter()
            .filter(|kind| **kind == "delegate_completed")
            .count(),
        1
    );
    assert!(!event_kinds.contains(&"delegate_failed"));

    let terminal_outcome = repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(terminal_outcome.status, "ok");
    assert_eq!(
        terminal_outcome.payload_json["final_output"],
        "Child final output"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_child_background_failure_persists_terminal_outcome() {
    let (config, db_path) = isolated_test_config("delegate-child-background-failure");
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: None,
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("bg-child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child session");

    let runtime = FakeRuntime::with_turns(vec![], vec![Err("child runtime failed".to_owned())]);

    let outcome = super::run_delegate_child_turn_with_runtime(
        &ConversationTurnLoop::new(),
        &config,
        &runtime,
        &NoopAppToolDispatcher,
        "child-session",
        "child task",
        60,
        None,
    )
    .await
    .expect("delegate child background outcome");

    assert_eq!(outcome.status, "error");
    assert_eq!(outcome.payload["child_session_id"], "child-session");
    assert_eq!(outcome.payload["error"], "child runtime failed");

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(
        child.state,
        crate::session::repository::SessionState::Failed
    );
    assert_eq!(child.last_error.as_deref(), Some("child runtime failed"));

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
    assert_eq!(
        terminal_outcome.payload_json["error"],
        "child runtime failed"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_child_background_timeout_persists_terminal_outcome() {
    let (config, db_path) = isolated_test_config("delegate-child-background-timeout");
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: None,
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("bg-child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child session");

    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Too slow".to_owned(),
            tool_intents: vec![],
            raw_meta: Value::Null,
        })],
    )
    .with_turn_delays(vec![Duration::from_millis(1_100)]);

    let outcome = super::run_delegate_child_turn_with_runtime(
        &ConversationTurnLoop::new(),
        &config,
        &runtime,
        &NoopAppToolDispatcher,
        "child-session",
        "slow child task",
        1,
        None,
    )
    .await
    .expect("delegate child background outcome");

    assert_eq!(outcome.status, "timeout");
    assert_eq!(outcome.payload["child_session_id"], "child-session");
    assert_eq!(outcome.payload["error"], "delegate_timeout");

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(
        child.state,
        crate::session::repository::SessionState::TimedOut
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

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_child_background_panic_persists_failed_terminal_outcome() {
    let (config, db_path) = isolated_test_config("delegate-child-background-panic");
    let memory_config = crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
    };
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: None,
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("bg-child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child session");

    let outcome = super::run_delegate_child_turn_with_runtime(
        &ConversationTurnLoop::new(),
        &config,
        &PanicRuntime::new(),
        &NoopAppToolDispatcher,
        "child-session",
        "panic child task",
        60,
        None,
    )
    .await
    .expect("delegate child panic outcome");

    assert_eq!(outcome.status, "error");
    assert_eq!(outcome.payload["child_session_id"], "child-session");
    assert_eq!(
        outcome.payload["error"],
        "delegate_child_panic: panic-runtime-request-turn"
    );

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(
        child.state,
        crate::session::repository::SessionState::Failed
    );
    assert_eq!(
        child.last_error.as_deref(),
        Some("delegate_child_panic: panic-runtime-request-turn")
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
    assert_eq!(
        terminal_outcome.payload_json["error"],
        "delegate_child_panic: panic-runtime-request-turn"
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_propagates_error_without_persisting() {
    let runtime = FakeRuntime::new(vec![], Err("timeout".to_owned()));
    let turn_loop = ConversationTurnLoop::new();
    let error = turn_loop
        .handle_turn_with_runtime(
            &test_config(),
            "session-2",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect_err("propagate mode should return error");

    assert!(error.contains("timeout"));
    assert!(runtime.persisted.lock().expect("persisted lock").is_empty());
}

#[tokio::test]
async fn handle_turn_with_runtime_inline_mode_returns_synthetic_reply_and_persists() {
    let runtime = FakeRuntime::new(vec![], Err("timeout".to_owned()));
    let turn_loop = ConversationTurnLoop::new();
    let output = turn_loop
        .handle_turn_with_runtime(
            &test_config(),
            "session-3",
            "hello",
            ProviderErrorMode::InlineMessage,
            &runtime,
            None,
        )
        .await
        .expect("inline mode should return synthetic reply");

    assert_eq!(output, "[provider_error] timeout");

    let persisted = runtime.persisted.lock().expect("persisted lock");
    assert_eq!(persisted.len(), 2);
    assert_eq!(
        persisted[0],
        (
            "session-3".to_owned(),
            "user".to_owned(),
            "hello".to_owned()
        )
    );
    assert_eq!(
        persisted[1],
        (
            "session-3".to_owned(),
            "assistant".to_owned(),
            "[provider_error] timeout".to_owned(),
        )
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_uses_session_tool_view_from_runtime() {
    let child_view = crate::tools::planned_delegate_child_tool_view();
    let runtime = FakeRuntime::new(vec![], Ok("assistant-reply".to_owned()))
        .with_tool_view(child_view.clone());
    let turn_loop = ConversationTurnLoop::new();

    let reply = turn_loop
        .handle_turn_with_runtime(
            &test_config(),
            "session-child",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("handle turn success");

    assert_eq!(reply, "assistant-reply");
    assert_eq!(
        runtime
            .built_tool_views
            .lock()
            .expect("built tool views lock")
            .as_slice(),
        &[child_view.clone()]
    );
    assert_eq!(
        runtime
            .turn_requested_tool_views
            .lock()
            .expect("turn request tool views lock")
            .as_slice(),
        &[child_view]
    );
}

#[tokio::test]
async fn conversation_turn_uses_tool_view_from_session_context() {
    let runtime = FakeRuntime::new(vec![], Ok("assistant-reply".to_owned()));
    let turn_loop = ConversationTurnLoop::new();
    let child_context = crate::conversation::SessionContext::child(
        "delegate:child-1",
        "root-session",
        crate::tools::planned_delegate_child_tool_view(),
    );

    let reply = turn_loop
        .handle_turn_with_runtime_and_context(
            &test_config(),
            &child_context,
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            &crate::conversation::NoopAppToolDispatcher,
            None,
        )
        .await
        .expect("handle turn success");

    assert_eq!(reply, "assistant-reply");
    assert_eq!(
        runtime
            .built_tool_views
            .lock()
            .expect("built tool views lock")
            .as_slice(),
        &[crate::tools::planned_delegate_child_tool_view()]
    );
    assert_eq!(
        runtime
            .turn_requested_tool_views
            .lock()
            .expect("turn request tool views lock")
            .as_slice(),
        &[crate::tools::planned_delegate_child_tool_view()]
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_executes_session_tools_via_default_dispatcher() {
    let (config, db_path) = isolated_test_config("default-session-tools");
    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Listing sessions.".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name: "sessions_list".to_owned(),
                args_json: json!({}),
                source: "provider_tool_call".to_owned(),
                session_id: "root-session".to_owned(),
                turn_id: "turn-session-tools".to_owned(),
                tool_call_id: "call-session-tools".to_owned(),
            }],
            raw_meta: Value::Null,
        })],
    );
    let turn_loop = ConversationTurnLoop::new();

    let reply = turn_loop
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("handle turn success");

    assert!(
        reply.contains("\"current_session_id\":\"root-session\""),
        "reply should contain session tool payload, got: {reply}"
    );
    assert!(
        reply.contains("\"session_id\":\"root-session\""),
        "reply should list the registered root session, got: {reply}"
    );
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        0
    );

    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    let session = repo
        .load_session("root-session")
        .expect("load session")
        .expect("root session row");
    assert_eq!(session.session_id, "root-session");
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_creates_child_session_and_returns_structured_result() {
    let (config, db_path) = isolated_test_config("delegate-happy-path");
    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Delegating.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "delegate".to_owned(),
                    args_json: json!({
                        "task": "child task",
                        "label": "research-subtask"
                    }),
                    source: "provider_tool_call".to_owned(),
                    session_id: "root-session".to_owned(),
                    turn_id: "turn-delegate-parent".to_owned(),
                    tool_call_id: "call-delegate-parent".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Child final output".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
    );

    let reply = ConversationTurnLoop::new()
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("handle turn success");

    assert!(
        reply.contains("\"label\":\"research-subtask\""),
        "reply: {reply}"
    );
    assert!(
        reply.contains("\"final_output\":\"Child final output\""),
        "reply: {reply}"
    );
    assert!(
        reply.contains("\"child_session_id\":\"delegate:"),
        "reply: {reply}"
    );

    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    let child = repo
        .list_visible_sessions("root-session")
        .expect("list visible sessions")
        .into_iter()
        .find(|session| session.parent_session_id.as_deref() == Some("root-session"))
        .expect("child session summary");
    assert_eq!(
        child.kind,
        crate::session::repository::SessionKind::DelegateChild
    );
    assert_eq!(
        child.state,
        crate::session::repository::SessionState::Completed
    );
    assert_eq!(child.label.as_deref(), Some("research-subtask"));

    let events = repo
        .list_recent_events(&child.session_id, 10)
        .expect("list child events");
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert!(event_kinds.contains(&"delegate_started"));
    assert!(event_kinds.contains(&"delegate_completed"));

    let terminal_outcome = repo
        .load_terminal_outcome(&child.session_id)
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(terminal_outcome.status, "ok");
    assert_eq!(
        terminal_outcome.payload_json["final_output"],
        "Child final output"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_start_failure_does_not_leave_orphan_child_session() {
    let (config, db_path) = isolated_test_config("delegate-start-rollback");
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path.clone()),
    })
    .expect("session repository");
    let conn = Connection::open(&db_path).expect("open sqlite connection");
    conn.execute(
        "CREATE TRIGGER fail_delegate_started_event
         BEFORE INSERT ON session_events
         WHEN NEW.event_kind = 'delegate_started'
         BEGIN
            SELECT RAISE(FAIL, 'forced delegate started failure');
         END;",
        [],
    )
    .expect("create delegate_started failure trigger");

    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Delegating.".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name: "delegate".to_owned(),
                args_json: json!({
                    "task": "child task",
                    "label": "research-subtask"
                }),
                source: "provider_tool_call".to_owned(),
                session_id: "root-session".to_owned(),
                turn_id: "turn-delegate-parent".to_owned(),
                tool_call_id: "call-delegate-parent".to_owned(),
            }],
            raw_meta: Value::Null,
        })],
    );

    let reply = ConversationTurnLoop::new()
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("handle turn should surface tool error");

    assert!(
        reply.contains("insert session event failed"),
        "reply: {reply}"
    );

    let sessions = repo
        .list_sessions()
        .expect("list sessions after delegate start failure");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "root-session");
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_child_uses_restricted_tool_view() {
    let (config, _db_path) = isolated_test_config("delegate-child-tool-view");
    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Delegating.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "delegate".to_owned(),
                    args_json: json!({
                        "task": "child task",
                        "label": "child-view"
                    }),
                    source: "provider_tool_call".to_owned(),
                    session_id: "root-session".to_owned(),
                    turn_id: "turn-delegate-parent".to_owned(),
                    tool_call_id: "call-delegate-parent".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Child output".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
    );

    ConversationTurnLoop::new()
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("handle turn success");

    assert_eq!(
        runtime
            .turn_requested_tool_views
            .lock()
            .expect("turn request tool views lock")
            .as_slice(),
        &[
            crate::tools::runtime_tool_view(),
            crate::tools::planned_delegate_child_tool_view(),
        ]
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_child_tool_view_allows_shell_when_config_enabled() {
    let (mut config, _db_path) = isolated_test_config("delegate-child-shell");
    config.tools.delegate.allow_shell_in_child = true;
    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Delegating.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "delegate".to_owned(),
                    args_json: json!({
                        "task": "child task",
                        "label": "child-shell"
                    }),
                    source: "provider_tool_call".to_owned(),
                    session_id: "root-session".to_owned(),
                    turn_id: "turn-delegate-parent".to_owned(),
                    tool_call_id: "call-delegate-parent".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Child output".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
    );

    ConversationTurnLoop::new()
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("handle turn success");

    let requested = runtime
        .turn_requested_tool_views
        .lock()
        .expect("turn request tool views lock");
    assert_eq!(requested.len(), 2);
    assert!(requested[1].contains("shell.exec"));
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_timeout_sets_child_session_to_timed_out() {
    let (config, db_path) = isolated_test_config("delegate-timeout");
    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Delegating.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "delegate".to_owned(),
                    args_json: json!({
                        "task": "slow child task",
                        "label": "timeout-child",
                        "timeout_seconds": 1
                    }),
                    source: "provider_tool_call".to_owned(),
                    session_id: "root-session".to_owned(),
                    turn_id: "turn-delegate-parent".to_owned(),
                    tool_call_id: "call-delegate-parent".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Too slow".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
    )
    .with_turn_delays(vec![Duration::ZERO, Duration::from_millis(1_100)]);

    let reply = ConversationTurnLoop::new()
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("handle turn success");

    assert!(reply.contains("[timeout]"), "reply: {reply}");
    assert!(
        reply.contains("\"error\":\"delegate_timeout\""),
        "reply: {reply}"
    );

    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    let child = repo
        .list_visible_sessions("root-session")
        .expect("list visible sessions")
        .into_iter()
        .find(|session| session.parent_session_id.as_deref() == Some("root-session"))
        .expect("child session summary");
    assert_eq!(
        child.state,
        crate::session::repository::SessionState::TimedOut
    );
    assert_eq!(child.last_error.as_deref(), Some("delegate_timeout"));

    let events = repo
        .list_recent_events(&child.session_id, 10)
        .expect("list child events");
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert!(event_kinds.contains(&"delegate_started"));
    assert!(event_kinds.contains(&"delegate_timed_out"));

    let terminal_outcome = repo
        .load_terminal_outcome(&child.session_id)
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(terminal_outcome.status, "timeout");
    assert_eq!(terminal_outcome.payload_json["error"], "delegate_timeout");
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_terminal_outcome_persists_for_failed_child() {
    let (config, db_path) = isolated_test_config("delegate-failed-terminal-outcome");
    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Delegating.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "delegate".to_owned(),
                    args_json: json!({
                        "task": "failing child task",
                        "label": "failed-child"
                    }),
                    source: "provider_tool_call".to_owned(),
                    session_id: "root-session".to_owned(),
                    turn_id: "turn-delegate-parent".to_owned(),
                    tool_call_id: "call-delegate-parent".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Err("child runtime failed".to_owned()),
        ],
    );

    let reply = ConversationTurnLoop::new()
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("handle turn success");

    assert!(reply.contains("[error]"), "reply: {reply}");
    assert!(
        reply.contains("\"error\":\"child runtime failed\""),
        "reply: {reply}"
    );

    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    let child = repo
        .list_visible_sessions("root-session")
        .expect("list visible sessions")
        .into_iter()
        .find(|session| session.parent_session_id.as_deref() == Some("root-session"))
        .expect("child session summary");
    assert_eq!(
        child.state,
        crate::session::repository::SessionState::Failed
    );
    assert_eq!(child.label.as_deref(), Some("failed-child"));

    let terminal_outcome = repo
        .load_terminal_outcome(&child.session_id)
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(terminal_outcome.status, "error");
    assert_eq!(
        terminal_outcome.payload_json["error"],
        "child runtime failed"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_child_cannot_reenter_delegate() {
    let (config, _db_path) = isolated_test_config("delegate-reenter");
    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Delegating.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "delegate".to_owned(),
                    args_json: json!({
                        "task": "show raw json tool output",
                        "label": "nested-child"
                    }),
                    source: "provider_tool_call".to_owned(),
                    session_id: "root-session".to_owned(),
                    turn_id: "turn-delegate-parent".to_owned(),
                    tool_call_id: "call-delegate-parent".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Trying nested delegate.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "delegate".to_owned(),
                    args_json: json!({"task": "nested"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "delegate:child".to_owned(),
                    turn_id: "turn-delegate-child".to_owned(),
                    tool_call_id: "call-delegate-child".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
        ],
    );

    let reply = ConversationTurnLoop::new()
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("handle turn success");

    assert!(
        reply.contains("tool_not_visible: delegate"),
        "reply should surface nested delegate denial, got: {reply}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_child_can_reenter_when_max_depth_allows() {
    let (mut config, db_path) = isolated_test_config("delegate-nested-allowed");
    config.tools.delegate.max_depth = 2;
    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Delegating from root.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "delegate".to_owned(),
                    args_json: json!({
                        "task": "show raw json tool output",
                        "label": "child"
                    }),
                    source: "provider_tool_call".to_owned(),
                    session_id: "root-session".to_owned(),
                    turn_id: "turn-root".to_owned(),
                    tool_call_id: "call-root".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Delegating from child.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "delegate".to_owned(),
                    args_json: json!({
                        "task": "final grandchild task",
                        "label": "grandchild"
                    }),
                    source: "provider_tool_call".to_owned(),
                    session_id: "delegate:child-runtime".to_owned(),
                    turn_id: "turn-child".to_owned(),
                    tool_call_id: "call-child".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Grandchild final output".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
    );

    let reply = ConversationTurnLoop::new()
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("nested delegate success");

    assert!(
        reply.contains("Grandchild final output"),
        "reply should include nested delegate final output, got: {reply}"
    );

    let requested = runtime
        .turn_requested_tool_views
        .lock()
        .expect("turn request tool views lock");
    assert_eq!(requested.len(), 3);
    assert!(requested[1].contains("delegate"));
    assert!(!requested[2].contains("delegate"));

    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    let visible = repo
        .list_visible_sessions("root-session")
        .expect("visible sessions");
    let descendant_ids: Vec<&str> = visible
        .iter()
        .map(|session| session.session_id.as_str())
        .collect();
    assert!(descendant_ids.contains(&"root-session"));
    assert!(
        visible.len() >= 3,
        "expected root, child, grandchild; got: {visible:?}"
    );
    assert!(
        visible
            .iter()
            .any(|session| session.parent_session_id.as_deref() == Some("root-session")),
        "expected direct child session in visible set: {visible:?}"
    );
    assert!(
        visible
            .iter()
            .any(|session| session.parent_session_id.is_some()
                && session.parent_session_id.as_deref() != Some("root-session")),
        "expected descendant grandchild session in visible set: {visible:?}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn delegate_returns_depth_exceeded_when_nested_delegate_exceeds_max_depth() {
    let (mut config, db_path) = isolated_test_config("delegate-depth-exceeded");
    config.tools.delegate.max_depth = 1;
    let repo = SessionRepository::new(&crate::memory::runtime_config::MemoryRuntimeConfig {
        sqlite_path: Some(db_path),
    })
    .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");

    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Trying nested delegate despite depth.".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name: "delegate".to_owned(),
                args_json: json!({
                    "task": "nested",
                    "label": "too-deep"
                }),
                source: "provider_tool_call".to_owned(),
                session_id: "delegate:stale-child".to_owned(),
                turn_id: "turn-child".to_owned(),
                tool_call_id: "call-child".to_owned(),
            }],
            raw_meta: Value::Null,
        })],
    );
    let session_context = SessionContext::child(
        "delegate:stale-child",
        "root-session",
        crate::tools::delegate_child_tool_view_for_config_with_delegate(&config.tools, true),
    );

    let reply = ConversationTurnLoop::new()
        .handle_turn_with_runtime_and_context(
            &config,
            &session_context,
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            &DefaultAppToolDispatcher::new(
                crate::memory::runtime_config::MemoryRuntimeConfig {
                    sqlite_path: Some(config.memory.resolved_sqlite_path()),
                },
                config.tools.clone(),
            ),
            None,
        )
        .await
        .expect("depth exceeded reply");

    assert!(
        reply.contains("delegate_depth_exceeded"),
        "reply should surface depth enforcement, got: {reply}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_denies_tool_outside_session_tool_view() {
    use super::integration_tests::TurnTestHarness;

    let harness = TurnTestHarness::new();
    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Trying a shell command.".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name: "shell.exec".to_owned(),
                args_json: json!({"command": "echo", "args": ["blocked by tool view"]}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-child".to_owned(),
                turn_id: "turn-child".to_owned(),
                tool_call_id: "call-child".to_owned(),
            }],
            raw_meta: Value::Null,
        })],
        Vec::new(),
    )
    .with_tool_view(crate::tools::planned_delegate_child_tool_view());
    let turn_loop = ConversationTurnLoop::new();

    let reply = turn_loop
        .handle_turn_with_runtime(
            &test_config(),
            "session-child",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            Some(&harness.kernel_ctx),
        )
        .await
        .expect("turn should return denied raw reply");

    assert!(
        reply.contains("tool_not_visible: shell.exec"),
        "reply should surface tool-view denial, got: {reply}"
    );
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        0
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_tool_turn_runs_second_turn_for_natural_language_reply() {
    use super::integration_tests::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(
        harness.temp_dir.join("note.md"),
        "hello from orchestrator test",
    )
    .expect("seed test note");

    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Reading the file now.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "note.md"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-tool".to_owned(),
                    turn_id: "turn-tool".to_owned(),
                    tool_call_id: "call-tool".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Summary: the note says hello from orchestrator test.".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
    );

    let turn_loop = ConversationTurnLoop::new();
    let reply = turn_loop
        .handle_turn_with_runtime(
            &test_config(),
            "session-tool",
            "read and summarize note.md",
            ProviderErrorMode::Propagate,
            &runtime,
            Some(&harness.kernel_ctx),
        )
        .await
        .expect("tool turn should succeed");

    assert_eq!(
        reply,
        "Summary: the note says hello from orchestrator test."
    );
    assert!(
        !reply.contains("[ok]"),
        "default reply should not contain raw tool marker, got: {reply}"
    );
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        0
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 2);

    let requested_turns = runtime
        .turn_requested_messages
        .lock()
        .expect("turn request lock");
    assert_eq!(requested_turns.len(), 2);
    let second_turn_payload = serde_json::to_string(&requested_turns[1]).expect("serialize turns");
    assert!(
        second_turn_payload.contains("[tool_result]"),
        "second turn should include tool result context, got: {second_turn_payload}"
    );
    assert!(
        second_turn_payload.contains("Original request"),
        "second turn should include followup prompt, got: {second_turn_payload}"
    );

    let persisted = runtime.persisted.lock().expect("persisted lock");
    assert_eq!(persisted.len(), 2);
    assert_eq!(persisted[0].1, "user");
    assert_eq!(persisted[1].1, "assistant");
    assert_eq!(persisted[1].2, reply);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_tool_turn_raw_request_skips_second_pass_completion() {
    use super::integration_tests::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(
        harness.temp_dir.join("note.md"),
        "hello from orchestrator test",
    )
    .expect("seed test note");

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Reading the file now.".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({"path": "note.md"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-tool-raw".to_owned(),
                turn_id: "turn-tool-raw".to_owned(),
                tool_call_id: "call-tool-raw".to_owned(),
            }],
            raw_meta: Value::Null,
        }),
        Ok("this must not be used".to_owned()),
    );

    let turn_loop = ConversationTurnLoop::new();
    let reply = turn_loop
        .handle_turn_with_runtime(
            &test_config(),
            "session-tool-raw",
            "read note.md and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            Some(&harness.kernel_ctx),
        )
        .await
        .expect("tool turn should succeed");

    assert!(
        reply.contains("[ok]"),
        "raw-request mode should keep tool marker, got: {reply}"
    );
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        0
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_supports_multiple_tool_rounds_before_final_answer() {
    use super::integration_tests::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(harness.temp_dir.join("note_a.md"), "first note").expect("seed note_a");
    std::fs::write(harness.temp_dir.join("note_b.md"), "second note").expect("seed note_b");

    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Reading note_a.md.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "note_a.md"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-multi-tool".to_owned(),
                    turn_id: "turn-1".to_owned(),
                    tool_call_id: "call-1".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Need note_b.md as well.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "note_b.md"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-multi-tool".to_owned(),
                    turn_id: "turn-2".to_owned(),
                    tool_call_id: "call-2".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Summary: note_a says first note; note_b says second note."
                    .to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
    );

    let turn_loop = ConversationTurnLoop::new();
    let reply = turn_loop
        .handle_turn_with_runtime(
            &test_config(),
            "session-multi-tool",
            "read note_a.md and note_b.md then summarize",
            ProviderErrorMode::Propagate,
            &runtime,
            Some(&harness.kernel_ctx),
        )
        .await
        .expect("multi-tool turn should succeed");

    assert_eq!(
        reply,
        "Summary: note_a says first note; note_b says second note."
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 3);
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        0
    );

    let requested_turns = runtime
        .turn_requested_messages
        .lock()
        .expect("turn request lock");
    assert_eq!(requested_turns.len(), 3);
    let third_turn_payload = serde_json::to_string(&requested_turns[2]).expect("serialize turns");
    let tool_result_mentions = third_turn_payload.matches("[tool_result]").count();
    assert!(
        tool_result_mentions >= 2,
        "third turn should include at least two tool_result entries, got: {third_turn_payload}"
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_repeated_tool_signature_guard_warns_then_triggers_completion() {
    let repeated_tool_turn = || {
        Ok(ProviderTurn {
            assistant_text: "Reading the file again.".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({"path": "note.md"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-loop-guard".to_owned(),
                turn_id: "turn-loop-guard".to_owned(),
                tool_call_id: "call-loop-guard".to_owned(),
            }],
            raw_meta: Value::Null,
        })
    };

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            repeated_tool_turn(),
            repeated_tool_turn(),
            repeated_tool_turn(),
            repeated_tool_turn(),
        ],
        vec![Ok(
            "I cannot access additional context, but here's what I found.".to_owned(),
        )],
    );

    let turn_loop = ConversationTurnLoop::new();
    let reply = turn_loop
        .handle_turn_with_runtime(
            &test_config(),
            "session-loop-guard",
            "read note.md",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("loop guard fallback should succeed");

    assert_eq!(
        reply,
        "I cannot access additional context, but here's what I found."
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 4);
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        1
    );

    let completion_payloads = runtime
        .completion_requested_messages
        .lock()
        .expect("completion requests lock");
    assert_eq!(completion_payloads.len(), 1);
    let serialized = serde_json::to_string(&completion_payloads[0]).expect("serialize completion");
    assert!(
        serialized.contains("[tool_loop_guard]"),
        "completion fallback payload should include loop guard marker, got: {serialized}"
    );
    assert!(
        serialized.contains("Detected tool-loop behavior across rounds."),
        "completion fallback should include generic tool-loop guard prompt, got: {serialized}"
    );
    assert!(
        serialized.contains("Loop guard reason:"),
        "completion fallback should include loop guard reason section, got: {serialized}"
    );
    assert!(
        serialized.matches("[tool_failure]").count() == 4,
        "completion fallback should preserve the latest tool failure context before guard fallback, got: {serialized}"
    );

    let turn_payloads = runtime
        .turn_requested_messages
        .lock()
        .expect("turn requests lock");
    assert_eq!(turn_payloads.len(), 4);
    let warning_turn_payload =
        serde_json::to_string(&turn_payloads[3]).expect("serialize warning turn");
    assert!(
        warning_turn_payload.contains("[tool_loop_warning]"),
        "warning turn payload should include loop warning marker before hard stop, got: {warning_turn_payload}"
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_ping_pong_loop_guard_triggers_completion() {
    let turn_for = |path: &str, call_id: &str| {
        Ok(ProviderTurn {
            assistant_text: format!("Trying to read {path}."),
            tool_intents: vec![ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({ "path": path }),
                source: "provider_tool_call".to_owned(),
                session_id: "session-ping-pong-guard".to_owned(),
                turn_id: format!("turn-ping-pong-{path}"),
                tool_call_id: call_id.to_owned(),
            }],
            raw_meta: Value::Null,
        })
    };

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            turn_for("note_a.md", "call-ping-a-1"),
            turn_for("note_b.md", "call-ping-b-1"),
            turn_for("note_a.md", "call-ping-a-2"),
            turn_for("note_b.md", "call-ping-b-2"),
            turn_for("note_a.md", "call-ping-a-3"),
        ],
        vec![Ok("Switching strategy after loop warning.".to_owned())],
    );

    let mut config = test_config();
    config.conversation.turn_loop.max_rounds = 6;
    config.conversation.turn_loop.max_repeated_tool_call_rounds = 8;
    config.conversation.turn_loop.max_ping_pong_cycles = 2;
    config.conversation.turn_loop.max_same_tool_failure_rounds = 8;

    let turn_loop = ConversationTurnLoop::new();
    let reply = turn_loop
        .handle_turn_with_runtime(
            &config,
            "session-ping-pong-guard",
            "read note_a then note_b",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("ping-pong loop guard fallback should succeed");

    assert_eq!(reply, "Switching strategy after loop warning.");
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 5);
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        1
    );

    let completion_payloads = runtime
        .completion_requested_messages
        .lock()
        .expect("completion requests lock");
    assert_eq!(completion_payloads.len(), 1);
    let completion_payload =
        serde_json::to_string(&completion_payloads[0]).expect("serialize completion");
    assert!(
        completion_payload.contains("[tool_loop_guard]"),
        "completion payload should include loop guard marker, got: {completion_payload}"
    );
    assert!(
        completion_payload.contains("Loop guard reason:"),
        "completion payload should include loop guard reason section, got: {completion_payload}"
    );
    assert!(
        completion_payload.matches("[tool_failure]").count() == 5,
        "completion payload should include the latest tool failure payload before hard stop, got: {completion_payload}"
    );
    assert!(
        completion_payload.contains("ping_pong_tool_patterns"),
        "completion payload should include ping-pong reason, got: {completion_payload}"
    );

    let turn_payloads = runtime
        .turn_requested_messages
        .lock()
        .expect("turn requests lock");
    assert_eq!(turn_payloads.len(), 5);
    let warning_turn_payload =
        serde_json::to_string(&turn_payloads[4]).expect("serialize warning turn");
    assert!(
        warning_turn_payload.contains("[tool_loop_warning]"),
        "warning turn payload should include loop warning marker, got: {warning_turn_payload}"
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_failure_streak_guard_triggers_completion() {
    let turn_for = |path: &str, call_id: &str| {
        Ok(ProviderTurn {
            assistant_text: format!("Attempting read for {path}."),
            tool_intents: vec![ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({ "path": path }),
                source: "provider_tool_call".to_owned(),
                session_id: "session-failure-streak-guard".to_owned(),
                turn_id: format!("turn-failure-streak-{path}"),
                tool_call_id: call_id.to_owned(),
            }],
            raw_meta: Value::Null,
        })
    };

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            turn_for("note_1.md", "call-failure-1"),
            turn_for("note_2.md", "call-failure-2"),
            turn_for("note_3.md", "call-failure-3"),
            turn_for("note_4.md", "call-failure-4"),
        ],
        vec![Ok("Stopping after repeated tool failures.".to_owned())],
    );

    let mut config = test_config();
    config.conversation.turn_loop.max_rounds = 5;
    config.conversation.turn_loop.max_repeated_tool_call_rounds = 8;
    config.conversation.turn_loop.max_ping_pong_cycles = 8;
    config.conversation.turn_loop.max_same_tool_failure_rounds = 3;

    let turn_loop = ConversationTurnLoop::new();
    let reply = turn_loop
        .handle_turn_with_runtime(
            &config,
            "session-failure-streak-guard",
            "read those notes",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("failure-streak loop guard fallback should succeed");

    assert_eq!(reply, "Stopping after repeated tool failures.");
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 4);
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        1
    );

    let completion_payloads = runtime
        .completion_requested_messages
        .lock()
        .expect("completion requests lock");
    assert_eq!(completion_payloads.len(), 1);
    let completion_payload =
        serde_json::to_string(&completion_payloads[0]).expect("serialize completion");
    assert!(
        completion_payload.contains("[tool_loop_guard]"),
        "completion payload should include loop guard marker, got: {completion_payload}"
    );
    assert!(
        completion_payload.contains("Loop guard reason:"),
        "completion payload should include loop guard reason section, got: {completion_payload}"
    );
    assert!(
        completion_payload.matches("[tool_failure]").count() == 4,
        "completion payload should include the latest tool failure payload before hard stop, got: {completion_payload}"
    );
    assert!(
        completion_payload.contains("tool_failure_streak"),
        "completion payload should include failure-streak reason, got: {completion_payload}"
    );

    let turn_payloads = runtime
        .turn_requested_messages
        .lock()
        .expect("turn requests lock");
    assert_eq!(turn_payloads.len(), 4);
    let warning_turn_payload =
        serde_json::to_string(&turn_payloads[3]).expect("serialize warning turn");
    assert!(
        warning_turn_payload.contains("[tool_loop_warning]"),
        "warning turn payload should include loop warning marker, got: {warning_turn_payload}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_truncates_large_tool_result_in_followup_payload() {
    use super::integration_tests::TurnTestHarness;

    let harness = TurnTestHarness::new();
    let large_note = format!("BEGIN-UNIQUE-{}-END-UNIQUE", "x".repeat(1_600));
    std::fs::write(harness.temp_dir.join("large_note.md"), large_note).expect("seed large note");

    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Reading large note.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "large_note.md"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-truncate-tool-result".to_owned(),
                    turn_id: "turn-truncate-tool-result-1".to_owned(),
                    tool_call_id: "call-truncate-tool-result-1".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Summary completed.".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
    );

    let mut config = test_config();
    config
        .conversation
        .turn_loop
        .max_followup_tool_payload_chars = 220;

    let turn_loop = ConversationTurnLoop::new();
    let reply = turn_loop
        .handle_turn_with_runtime(
            &config,
            "session-truncate-tool-result",
            "read large_note.md and summarize",
            ProviderErrorMode::Propagate,
            &runtime,
            Some(&harness.kernel_ctx),
        )
        .await
        .expect("tool-result truncation path should succeed");

    assert_eq!(reply, "Summary completed.");
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 2);

    let requested_turns = runtime
        .turn_requested_messages
        .lock()
        .expect("turn request lock");
    assert_eq!(requested_turns.len(), 2);
    let second_turn_payload = serde_json::to_string(&requested_turns[1]).expect("serialize turns");
    assert!(
        second_turn_payload.contains("[tool_result_truncated]"),
        "followup payload should include tool-result truncation marker, got: {second_turn_payload}"
    );
    assert!(
        second_turn_payload.contains("BEGIN-UNIQUE-"),
        "followup payload should retain leading tool context, got: {second_turn_payload}"
    );
    assert!(
        !second_turn_payload.contains("-END-UNIQUE"),
        "followup payload should trim tail content when truncated, got: {second_turn_payload}"
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_truncates_large_tool_failure_in_followup_payload() {
    let oversized_tool_name = format!("tool_{}", "z".repeat(900));
    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Attempting unknown tool.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: oversized_tool_name.clone(),
                    args_json: json!({}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-truncate-tool-failure".to_owned(),
                    turn_id: "turn-truncate-tool-failure-1".to_owned(),
                    tool_call_id: "call-truncate-tool-failure-1".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Fallback answer after tool failure.".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
    );

    let mut config = test_config();
    config
        .conversation
        .turn_loop
        .max_followup_tool_payload_chars = 180;
    config.conversation.turn_loop.max_repeated_tool_call_rounds = 5;

    let turn_loop = ConversationTurnLoop::new();
    let reply = turn_loop
        .handle_turn_with_runtime(
            &config,
            "session-truncate-tool-failure",
            "run this tool",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("tool-failure truncation path should succeed");

    assert_eq!(reply, "Fallback answer after tool failure.");
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 2);

    let requested_turns = runtime
        .turn_requested_messages
        .lock()
        .expect("turn request lock");
    assert_eq!(requested_turns.len(), 2);
    let second_turn_payload = serde_json::to_string(&requested_turns[1]).expect("serialize turns");
    assert!(
        second_turn_payload.contains("[tool_failure_truncated]"),
        "followup payload should include tool-failure truncation marker, got: {second_turn_payload}"
    );
    assert!(
        second_turn_payload.contains("tool_not_found"),
        "followup payload should retain failure type, got: {second_turn_payload}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_enforces_total_followup_payload_budget_across_rounds() {
    use super::integration_tests::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(
        harness.temp_dir.join("note_a.md"),
        format!("NOTE-A-BEGIN-{}-NOTE-A-END", "a".repeat(1_200)),
    )
    .expect("seed note_a");
    std::fs::write(
        harness.temp_dir.join("note_b.md"),
        format!("NOTE-B-BEGIN-{}-NOTE-B-END", "b".repeat(1_200)),
    )
    .expect("seed note_b");
    std::fs::write(
        harness.temp_dir.join("note_c.md"),
        format!("NOTE-C-BEGIN-{}-NOTE-C-END", "c".repeat(1_200)),
    )
    .expect("seed note_c");

    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Reading note A.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "note_a.md"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-total-budget".to_owned(),
                    turn_id: "turn-total-budget-1".to_owned(),
                    tool_call_id: "call-total-budget-1".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Reading note B.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "note_b.md"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-total-budget".to_owned(),
                    turn_id: "turn-total-budget-2".to_owned(),
                    tool_call_id: "call-total-budget-2".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Reading note C.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "note_c.md"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-total-budget".to_owned(),
                    turn_id: "turn-total-budget-3".to_owned(),
                    tool_call_id: "call-total-budget-3".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Final synthesis after bounded context.".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
    );

    let mut config = test_config();
    config.conversation.turn_loop.max_rounds = 4;
    config.conversation.turn_loop.max_repeated_tool_call_rounds = 8;
    config.conversation.turn_loop.max_ping_pong_cycles = 8;
    config.conversation.turn_loop.max_same_tool_failure_rounds = 8;
    config
        .conversation
        .turn_loop
        .max_followup_tool_payload_chars = 600;
    config
        .conversation
        .turn_loop
        .max_followup_tool_payload_chars_total = 120;

    let turn_loop = ConversationTurnLoop::new();
    let reply = turn_loop
        .handle_turn_with_runtime(
            &config,
            "session-total-budget",
            "read all notes then summarize",
            ProviderErrorMode::Propagate,
            &runtime,
            Some(&harness.kernel_ctx),
        )
        .await
        .expect("total followup payload budget path should succeed");

    assert_eq!(reply, "Final synthesis after bounded context.");
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 4);

    let requested_turns = runtime
        .turn_requested_messages
        .lock()
        .expect("turn request lock");
    assert_eq!(requested_turns.len(), 4);
    let fourth_turn_payload = serde_json::to_string(&requested_turns[3]).expect("serialize turns");
    assert!(
        fourth_turn_payload.contains("[tool_result_truncated]"),
        "fourth turn should still include truncation marker, got: {fourth_turn_payload}"
    );
    assert!(
        fourth_turn_payload.contains("budget_exhausted=true"),
        "fourth turn should report exhausted total payload budget, got: {fourth_turn_payload}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_turn_loop_policy_override_allows_multiple_tool_steps() {
    use super::integration_tests::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(harness.temp_dir.join("note_a.md"), "first note").expect("seed note_a");
    std::fs::write(harness.temp_dir.join("note_b.md"), "second note").expect("seed note_b");

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Reading both notes.".to_owned(),
            tool_intents: vec![
                ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "note_a.md"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-step-override".to_owned(),
                    turn_id: "turn-step-override".to_owned(),
                    tool_call_id: "call-step-1".to_owned(),
                },
                ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "note_b.md"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-step-override".to_owned(),
                    turn_id: "turn-step-override".to_owned(),
                    tool_call_id: "call-step-2".to_owned(),
                },
            ],
            raw_meta: Value::Null,
        }),
        Ok("this must not be used".to_owned()),
    );

    let mut config = test_config();
    config.conversation.turn_loop.max_tool_steps_per_round = 2;

    let turn_loop = ConversationTurnLoop::new();
    let reply = turn_loop
        .handle_turn_with_runtime(
            &config,
            "session-step-override",
            "read note_a.md and note_b.md, return raw tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            Some(&harness.kernel_ctx),
        )
        .await
        .expect("multiple tool steps should be allowed by override");

    assert!(
        reply.matches("[ok]").count() >= 2,
        "expected at least two tool outputs, got: {reply}"
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 1);
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        0
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_turn_loop_policy_override_allows_more_repeated_rounds() {
    let repeated_tool_turn = || {
        Ok(ProviderTurn {
            assistant_text: "Trying file.read again.".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({"path": "note.md"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-loop-override".to_owned(),
                turn_id: "turn-loop-override".to_owned(),
                tool_call_id: "call-loop-override".to_owned(),
            }],
            raw_meta: Value::Null,
        })
    };

    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![
            repeated_tool_turn(),
            repeated_tool_turn(),
            repeated_tool_turn(),
            repeated_tool_turn(),
            Ok(ProviderTurn {
                assistant_text: "Final answer after four retries.".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
    );

    let mut config = test_config();
    config.conversation.turn_loop.max_rounds = 5;
    config.conversation.turn_loop.max_repeated_tool_call_rounds = 4;
    config.conversation.turn_loop.max_ping_pong_cycles = 8;
    config.conversation.turn_loop.max_same_tool_failure_rounds = 8;

    let turn_loop = ConversationTurnLoop::new();
    let reply = turn_loop
        .handle_turn_with_runtime(
            &config,
            "session-loop-override",
            "read note.md",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("policy override should permit extra repeated rounds");

    assert_eq!(reply, "Final answer after four retries.");
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 5);
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        0
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_tool_denial_returns_inline_reply_even_in_propagate_mode() {
    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Reading the file now.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "note.md"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-denied".to_owned(),
                    turn_id: "turn-denied".to_owned(),
                    tool_call_id: "call-denied".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "MODEL_DENIED_REPLY".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
    );

    let turn_loop = ConversationTurnLoop::new();
    let reply = turn_loop
        .handle_turn_with_runtime(
            &test_config(),
            "session-denied",
            "read note.md",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("tool denial should still return inline assistant text");

    assert_eq!(reply, "MODEL_DENIED_REPLY");
    assert!(
        !reply.contains("[tool_denied]"),
        "reply should not expose raw tool_denied marker, got: {reply}"
    );
    assert!(
        !reply.contains("[tool_error]"),
        "reply should not expose raw tool_error marker, got: {reply}"
    );
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        0,
        "tool-denied loop should continue with request_turn without completion fallback"
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 2);

    let persisted = runtime.persisted.lock().expect("persisted lock");
    assert_eq!(persisted.len(), 2);
    assert_eq!(persisted[1].2, reply);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_tool_error_returns_natural_language_fallback() {
    use super::integration_tests::TurnTestHarness;

    let harness = TurnTestHarness::new();
    let runtime = FakeRuntime::with_turns(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Reading the file now.".to_owned(),
                tool_intents: vec![ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!("not an object"),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-tool-error".to_owned(),
                    turn_id: "turn-tool-error".to_owned(),
                    tool_call_id: "call-tool-error".to_owned(),
                }],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "MODEL_ERROR_REPLY".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
    );

    let turn_loop = ConversationTurnLoop::new();
    let reply = turn_loop
        .handle_turn_with_runtime(
            &test_config(),
            "session-tool-error",
            "read note.md",
            ProviderErrorMode::Propagate,
            &runtime,
            Some(&harness.kernel_ctx),
        )
        .await
        .expect("tool error should still return inline assistant text");

    assert_eq!(reply, "MODEL_ERROR_REPLY");
    assert!(
        !reply.contains("[tool_error]"),
        "reply should not expose raw tool_error marker, got: {reply}"
    );
    assert!(
        !reply.contains("[tool_denied]"),
        "reply should not expose raw tool_denied marker, got: {reply}"
    );

    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        0,
        "tool-error loop should continue with request_turn without completion fallback"
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 2);

    let persisted = runtime.persisted.lock().expect("persisted lock");
    assert_eq!(persisted.len(), 2);
    assert_eq!(persisted[1].2, reply);
}

#[tokio::test]
async fn handle_turn_with_runtime_tool_failure_completion_error_uses_raw_reason_without_markers() {
    let repeated_tool_turn = || {
        Ok(ProviderTurn {
            assistant_text: "Reading the file now.".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({"path": "note.md"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-denied-fallback".to_owned(),
                turn_id: "turn-denied-fallback".to_owned(),
                tool_call_id: "call-denied-fallback".to_owned(),
            }],
            raw_meta: Value::Null,
        })
    };

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            repeated_tool_turn(),
            repeated_tool_turn(),
            repeated_tool_turn(),
            repeated_tool_turn(),
        ],
        vec![Err("completion_unavailable".to_owned())],
    );

    let mut config = test_config();
    config.conversation.turn_loop.max_repeated_tool_call_rounds = 8;

    let turn_loop = ConversationTurnLoop::new();
    let reply = turn_loop
        .handle_turn_with_runtime(
            &config,
            "session-denied-fallback",
            "read note.md",
            ProviderErrorMode::Propagate,
            &runtime,
            None,
        )
        .await
        .expect("fallback should still return assistant text");

    assert!(
        reply.contains("Reading the file now."),
        "expected assistant preface, got: {reply}"
    );
    assert!(
        reply.contains("no_kernel_context"),
        "expected raw denial reason when completion fails, got: {reply}"
    );
    assert!(
        !reply.contains("[tool_denied]"),
        "reply should not expose raw tool_denied marker, got: {reply}"
    );
    assert!(
        !reply.contains("[tool_error]"),
        "reply should not expose raw tool_error marker, got: {reply}"
    );
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        1
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 4);
}

#[test]
fn format_provider_error_reply_is_stable() {
    let output = format_provider_error_reply("timeout");
    assert_eq!(output, "[provider_error] timeout");
}

#[test]
fn turn_contracts_have_stable_defaults() {
    use crate::conversation::{ProviderTurn, ToolIntent, TurnResult};
    let turn = ProviderTurn::default();
    assert!(turn.assistant_text.is_empty());
    assert!(turn.tool_intents.is_empty());
    let _intent = ToolIntent {
        tool_name: "file.read".to_owned(),
        args_json: serde_json::json!({"path":"README.md"}),
        source: "provider_tool_call".to_owned(),
        session_id: "s1".to_owned(),
        turn_id: "t1".to_owned(),
        tool_call_id: "c1".to_owned(),
    };
    let _result = TurnResult::FinalText("ok".to_owned());
}

#[test]
fn turn_engine_no_tool_intents_returns_final_text() {
    use crate::conversation::turn_engine::{ProviderTurn, TurnEngine, TurnResult};
    let engine = TurnEngine::new(1); // max_tool_steps = 1
    let turn = ProviderTurn {
        assistant_text: "Hello!".to_owned(),
        tool_intents: vec![],
        raw_meta: serde_json::Value::Null,
    };
    let result = engine.evaluate_turn(&turn);
    match result {
        TurnResult::FinalText(text) => assert_eq!(text, "Hello!"),
        other => panic!("expected FinalText, got {:?}", other),
    }
}

#[test]
fn provider_tool_aliases_flow_through_parse_and_turn_validation() {
    use crate::conversation::turn_engine::{TurnEngine, TurnResult};
    use crate::provider::extract_provider_turn;

    let response_body = serde_json::json!({
        "choices": [{
            "message": {
                "content": "reading",
                "tool_calls": [{
                    "id": "call_underscore",
                    "type": "function",
                    "function": {
                        "name": "file_read",
                        "arguments": "{\"path\":\"README.md\"}"
                    }
                }]
            }
        }]
    });

    let turn = extract_provider_turn(&response_body).expect("provider turn");
    assert_eq!(turn.tool_intents.len(), 1);
    assert_eq!(turn.tool_intents[0].tool_name, "file.read");

    let engine = TurnEngine::new(1);
    let result = engine.evaluate_turn(&turn);
    match result {
        TurnResult::NeedsApproval(reason) => {
            assert!(
                reason.contains("kernel_context_required"),
                "reason: {reason}"
            );
        }
        other => panic!("expected NeedsApproval, got {:?}", other),
    }
}

#[test]
fn turn_engine_unknown_tool_returns_tool_denied() {
    use crate::conversation::turn_engine::{ProviderTurn, ToolIntent, TurnEngine, TurnResult};
    let engine = TurnEngine::new(1);
    let turn = ProviderTurn {
        assistant_text: "".to_owned(),
        tool_intents: vec![ToolIntent {
            tool_name: "nonexistent.tool".to_owned(),
            args_json: serde_json::json!({}),
            source: "provider_tool_call".to_owned(),
            session_id: "s1".to_owned(),
            turn_id: "t1".to_owned(),
            tool_call_id: "c1".to_owned(),
        }],
        raw_meta: serde_json::Value::Null,
    };
    let result = engine.evaluate_turn(&turn);
    match result {
        TurnResult::ToolDenied(reason) => {
            assert!(reason.contains("tool_not_found"), "reason: {reason}")
        }
        other => panic!("expected ToolDenied, got {:?}", other),
    }
}

#[test]
fn turn_engine_known_tool_outside_tool_view_returns_tool_denied() {
    use crate::conversation::turn_engine::{ProviderTurn, ToolIntent, TurnEngine, TurnResult};

    let engine = TurnEngine::new(1);
    let turn = ProviderTurn {
        assistant_text: "".to_owned(),
        tool_intents: vec![ToolIntent {
            tool_name: "shell.exec".to_owned(),
            args_json: serde_json::json!({"command": "echo", "args": ["hello"]}),
            source: "provider_tool_call".to_owned(),
            session_id: "s-child".to_owned(),
            turn_id: "t-child".to_owned(),
            tool_call_id: "c-child".to_owned(),
        }],
        raw_meta: serde_json::Value::Null,
    };

    let result =
        engine.evaluate_turn_in_view(&turn, &crate::tools::planned_delegate_child_tool_view());
    match result {
        TurnResult::ToolDenied(reason) => {
            assert!(reason.contains("tool_not_visible"), "reason: {reason}")
        }
        other => panic!("expected ToolDenied, got {:?}", other),
    }
}

#[test]
fn turn_engine_exceeding_max_steps_returns_denied() {
    use crate::conversation::turn_engine::{ProviderTurn, ToolIntent, TurnEngine, TurnResult};
    let engine = TurnEngine::new(1);
    let intent = ToolIntent {
        tool_name: "file.read".to_owned(),
        args_json: serde_json::json!({}),
        source: "provider_tool_call".to_owned(),
        session_id: "s1".to_owned(),
        turn_id: "t1".to_owned(),
        tool_call_id: "c1".to_owned(),
    };
    let turn = ProviderTurn {
        assistant_text: "".to_owned(),
        tool_intents: vec![intent.clone(), intent],
        raw_meta: serde_json::Value::Null,
    };
    let result = engine.evaluate_turn(&turn);
    match result {
        TurnResult::ToolDenied(reason) => assert!(
            reason.contains("max_tool_steps_exceeded"),
            "reason: {reason}"
        ),
        other => panic!("expected ToolDenied for max steps, got {:?}", other),
    }
}

#[test]
fn turn_engine_known_tool_with_no_kernel_returns_tool_denied() {
    use crate::conversation::turn_engine::{ProviderTurn, ToolIntent, TurnEngine, TurnResult};
    let engine = TurnEngine::new(1);
    let turn = ProviderTurn {
        assistant_text: "".to_owned(),
        tool_intents: vec![ToolIntent {
            tool_name: "file.read".to_owned(),
            args_json: serde_json::json!({"path": "test.txt"}),
            source: "provider_tool_call".to_owned(),
            session_id: "s1".to_owned(),
            turn_id: "t1".to_owned(),
            tool_call_id: "c1".to_owned(),
        }],
        raw_meta: serde_json::Value::Null,
    };
    // Without kernel context, known tools should be validated but flagged as needing execution
    let result = engine.evaluate_turn(&turn);
    match result {
        TurnResult::NeedsApproval(reason) => {
            assert!(
                reason.contains("kernel_context_required"),
                "reason: {reason}"
            );
        }
        other => panic!("expected NeedsApproval, got {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_engine_execute_turn_no_kernel_returns_denied() {
    use crate::conversation::turn_engine::{ProviderTurn, ToolIntent, TurnEngine, TurnResult};
    let engine = TurnEngine::new(1);
    let turn = ProviderTurn {
        assistant_text: "".to_owned(),
        tool_intents: vec![ToolIntent {
            tool_name: "file.read".to_owned(),
            args_json: serde_json::json!({"path": "test.txt"}),
            source: "provider_tool_call".to_owned(),
            session_id: "s1".to_owned(),
            turn_id: "t1".to_owned(),
            tool_call_id: "c1".to_owned(),
        }],
        raw_meta: serde_json::Value::Null,
    };
    let result = engine.execute_turn(&turn, None).await;
    match result {
        TurnResult::ToolDenied(reason) => {
            assert!(reason.contains("no_kernel_context"), "reason: {reason}");
        }
        other => panic!("expected ToolDenied, got {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_engine_executes_known_tool_with_kernel() {
    use crate::conversation::turn_engine::{ProviderTurn, ToolIntent, TurnEngine, TurnResult};
    use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest, ToolPlaneError};
    use loongclaw_kernel::CoreToolAdapter;

    struct EchoToolAdapter;

    #[async_trait]
    impl CoreToolAdapter for EchoToolAdapter {
        fn name(&self) -> &str {
            "echo-tools"
        }

        async fn execute_core_tool(
            &self,
            request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, ToolPlaneError> {
            // Echo back the tool name and payload
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({"tool": request.tool_name, "input": request.payload}),
            })
        }
    }

    let audit = Arc::new(InMemoryAuditSink::default());
    let clock = Arc::new(FixedClock::new(1_700_000_000));
    let mut kernel = LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);

    let pack = VerticalPackManifest {
        pack_id: "test-pack".to_owned(),
        domain: "testing".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");
    kernel.register_core_tool_adapter(EchoToolAdapter);
    kernel
        .set_default_core_tool_adapter("echo-tools")
        .expect("set default");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");

    let ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };

    let engine = TurnEngine::new(5);
    let turn = ProviderTurn {
        assistant_text: "".to_owned(),
        tool_intents: vec![ToolIntent {
            tool_name: "file.read".to_owned(),
            args_json: json!({"path": "test.txt"}),
            source: "provider_tool_call".to_owned(),
            session_id: "s1".to_owned(),
            turn_id: "t1".to_owned(),
            tool_call_id: "c1".to_owned(),
        }],
        raw_meta: serde_json::Value::Null,
    };

    let result = engine.execute_turn(&turn, Some(&ctx)).await;
    match result {
        TurnResult::FinalText(text) => {
            assert!(
                text.contains("\"tool\":\"file.read\""),
                "expected echoed tool payload in output, got: {text}"
            );
        }
        TurnResult::ToolDenied(reason) => {
            // Must NOT be "execution_not_wired" or "no_kernel_context"
            assert!(
                !reason.contains("execution_not_wired") && !reason.contains("no_kernel_context"),
                "should not get execution_not_wired or no_kernel_context with kernel, got: {reason}"
            );
        }
        other => {
            // ToolError is also acceptable (e.g. file doesn't exist) as long
            // as it went through kernel execution
            if let TurnResult::ToolError(ref err) = other {
                assert!(
                    !err.contains("execution_not_wired"),
                    "should not get execution_not_wired, got: {err}"
                );
            } else {
                panic!("unexpected result: {:?}", other);
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_engine_routes_app_tools_through_dispatcher() {
    use async_trait::async_trait;
    use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};

    #[derive(Default)]
    struct RecordingAppDispatcher {
        calls: Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl crate::conversation::AppToolDispatcher for RecordingAppDispatcher {
        async fn execute_app_tool(
            &self,
            session_context: &crate::conversation::SessionContext,
            request: ToolCoreRequest,
            _kernel_ctx: Option<&KernelContext>,
        ) -> Result<ToolCoreOutcome, String> {
            self.calls.lock().expect("dispatcher calls lock").push((
                session_context.session_id.clone(),
                request.tool_name.clone(),
            ));
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "session_id": session_context.session_id,
                    "tool_name": request.tool_name,
                }),
            })
        }
    }

    let dispatcher = RecordingAppDispatcher::default();
    let engine = TurnEngine::new(1);
    let turn = ProviderTurn {
        assistant_text: "".to_owned(),
        tool_intents: vec![ToolIntent {
            tool_name: "sessions_list".to_owned(),
            args_json: json!({}),
            source: "provider_tool_call".to_owned(),
            session_id: "root-session".to_owned(),
            turn_id: "turn-app-1".to_owned(),
            tool_call_id: "call-app-1".to_owned(),
        }],
        raw_meta: Value::Null,
    };
    let session_context = crate::conversation::SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::planned_root_tool_view(),
    );

    let result = engine
        .execute_turn_in_context(&turn, &session_context, &dispatcher, None)
        .await;

    match result {
        TurnResult::FinalText(text) => {
            assert!(
                text.contains("\"tool_name\":\"sessions_list\""),
                "expected dispatcher payload in output, got: {text}"
            );
        }
        other => panic!("expected FinalText, got: {other:?}"),
    }

    assert_eq!(
        dispatcher
            .calls
            .lock()
            .expect("dispatcher calls lock")
            .as_slice(),
        &[("root-session".to_owned(), "sessions_list".to_owned())]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_engine_execute_turn_denied_without_capability() {
    use crate::conversation::turn_engine::{ProviderTurn, ToolIntent, TurnEngine, TurnResult};
    use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest, ToolPlaneError};
    use loongclaw_kernel::CoreToolAdapter;

    struct NoopToolAdapter;

    #[async_trait]
    impl CoreToolAdapter for NoopToolAdapter {
        fn name(&self) -> &str {
            "noop-tools"
        }

        async fn execute_core_tool(
            &self,
            _request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, ToolPlaneError> {
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({}),
            })
        }
    }

    let audit = Arc::new(InMemoryAuditSink::default());
    let clock = Arc::new(FixedClock::new(1_700_000_000));
    let mut kernel = LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);

    // Grant only MemoryRead — InvokeTool is missing
    let pack = VerticalPackManifest {
        pack_id: "test-pack".to_owned(),
        domain: "testing".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([Capability::MemoryRead]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");
    kernel.register_core_tool_adapter(NoopToolAdapter);
    kernel
        .set_default_core_tool_adapter("noop-tools")
        .expect("set default");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");

    let ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };

    let engine = TurnEngine::new(5);
    let turn = ProviderTurn {
        assistant_text: "".to_owned(),
        tool_intents: vec![ToolIntent {
            tool_name: "file.read".to_owned(),
            args_json: json!({"path": "test.txt"}),
            source: "provider_tool_call".to_owned(),
            session_id: "s1".to_owned(),
            turn_id: "t1".to_owned(),
            tool_call_id: "c1".to_owned(),
        }],
        raw_meta: serde_json::Value::Null,
    };

    let result = engine.execute_turn(&turn, Some(&ctx)).await;
    match result {
        TurnResult::ToolDenied(reason) => {
            assert!(
                reason.contains("apability") || reason.contains("denied"),
                "expected capability/denial reason, got: {reason}"
            );
        }
        other => panic!(
            "expected ToolDenied for missing capability, got {:?}",
            other
        ),
    }
}

// --- Tool lifecycle persistence tests ---

#[tokio::test]
async fn turn_engine_persists_tool_lifecycle_events() {
    use super::persistence::{persist_tool_decision, persist_tool_outcome};
    use crate::conversation::turn_engine::{ToolDecision, ToolOutcome};

    let runtime = FakeRuntime::new(vec![], Ok(String::new()));

    let decision = ToolDecision {
        allow: true,
        deny: false,
        approval_required: false,
        reason: "policy_ok".to_owned(),
        rule_id: "rule-42".to_owned(),
    };

    let outcome = ToolOutcome {
        status: "ok".to_owned(),
        payload: json!({"result": "file contents"}),
        error_code: None,
        human_reason: None,
        audit_event_id: Some("audit-001".to_owned()),
    };

    persist_tool_decision(&runtime, "sess-1", "turn-1", "call-1", &decision, None)
        .await
        .expect("persist decision");

    persist_tool_outcome(&runtime, "sess-1", "turn-1", "call-1", &outcome, None)
        .await
        .expect("persist outcome");

    let persisted = runtime.persisted.lock().expect("persisted lock");
    assert_eq!(persisted.len(), 2, "expected two persisted records");

    // Both should be assistant-role messages for session sess-1
    assert_eq!(persisted[0].0, "sess-1");
    assert_eq!(persisted[0].1, "assistant");
    assert_eq!(persisted[1].0, "sess-1");
    assert_eq!(persisted[1].1, "assistant");

    // Verify decision content has correct correlation IDs and type
    let decision_json: serde_json::Value =
        serde_json::from_str(&persisted[0].2).expect("decision json parse");
    assert_eq!(decision_json["type"], "tool_decision");
    assert_eq!(decision_json["turn_id"], "turn-1");
    assert_eq!(decision_json["tool_call_id"], "call-1");
    assert_eq!(decision_json["decision"]["allow"], true);
    assert_eq!(decision_json["decision"]["rule_id"], "rule-42");

    // Verify outcome content has correct correlation IDs and type
    let outcome_json: serde_json::Value =
        serde_json::from_str(&persisted[1].2).expect("outcome json parse");
    assert_eq!(outcome_json["type"], "tool_outcome");
    assert_eq!(outcome_json["turn_id"], "turn-1");
    assert_eq!(outcome_json["tool_call_id"], "call-1");
    assert_eq!(outcome_json["outcome"]["status"], "ok");
    assert_eq!(outcome_json["outcome"]["audit_event_id"], "audit-001");
}

// --- Kernel-routed memory tests ---

fn build_kernel_context(
    audit: Arc<InMemoryAuditSink>,
) -> (KernelContext, Arc<Mutex<Vec<MemoryCoreRequest>>>) {
    let clock = Arc::new(FixedClock::new(1_700_000_000));
    let mut kernel = LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);

    let pack = VerticalPackManifest {
        pack_id: "test-pack".to_owned(),
        domain: "testing".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([Capability::MemoryWrite, Capability::MemoryRead]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");

    let invocations = Arc::new(Mutex::new(Vec::new()));
    let adapter = SharedTestMemoryAdapter {
        invocations: invocations.clone(),
    };
    kernel.register_core_memory_adapter(adapter);
    kernel
        .set_default_core_memory_adapter("test-memory-shared")
        .expect("set default memory adapter");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");

    let ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };

    (ctx, invocations)
}

struct SharedTestMemoryAdapter {
    invocations: Arc<Mutex<Vec<MemoryCoreRequest>>>,
}

#[async_trait]
impl CoreMemoryAdapter for SharedTestMemoryAdapter {
    fn name(&self) -> &str {
        "test-memory-shared"
    }

    async fn execute_core_memory(
        &self,
        request: MemoryCoreRequest,
    ) -> Result<MemoryCoreOutcome, MemoryPlaneError> {
        self.invocations
            .lock()
            .expect("invocations lock")
            .push(request);
        Ok(MemoryCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({}),
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persist_turn_routes_through_kernel_when_context_provided() {
    let audit = Arc::new(InMemoryAuditSink::default());
    let (ctx, invocations) = build_kernel_context(audit.clone());

    let runtime = DefaultConversationRuntime;
    runtime
        .persist_turn("session-k1", "user", "kernel-hello", Some(&ctx))
        .await
        .expect("persist via kernel");

    // Verify the memory adapter received the request.
    let captured = invocations.lock().expect("invocations lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].operation, "append_turn");
    assert_eq!(captured[0].payload["session_id"], "session-k1");
    assert_eq!(captured[0].payload["role"], "user");
    assert_eq!(captured[0].payload["content"], "kernel-hello");

    // Verify audit events contain a memory plane invocation.
    let events = audit.snapshot();
    let has_memory_plane = events.iter().any(|event| {
        matches!(
            &event.kind,
            loongclaw_kernel::AuditEventKind::PlaneInvoked {
                plane: loongclaw_contracts::ExecutionPlane::Memory,
                ..
            }
        )
    });
    assert!(
        has_memory_plane,
        "audit should contain memory plane invocation"
    );
}

#[test]
fn default_runtime_build_messages_respects_restricted_tool_view() {
    let runtime = DefaultConversationRuntime;
    let view = crate::tools::ToolView::from_tool_names(["file.read"]);

    let messages = runtime
        .build_messages(&test_config(), "noop-session", true, &view, None)
        .expect("build messages");

    assert!(!messages.is_empty());
    let system_content = messages[0]["content"].as_str().expect("system content");
    assert!(system_content.contains("- file.read: Read file contents"));
    assert!(!system_content.contains("- file.write: Write file contents"));
    assert!(!system_content.contains("- shell.exec: Execute shell commands"));
}

#[cfg(not(feature = "memory-sqlite"))]
#[tokio::test]
async fn persist_turn_without_memory_sqlite_is_noop_with_kernel_context() {
    let ctx = crate::context::bootstrap_kernel_context("test-agent-no-memory", 60)
        .expect("bootstrap kernel context without memory-sqlite");
    let runtime = DefaultConversationRuntime;
    runtime
        .persist_turn("session-k0", "user", "no-memory", Some(&ctx))
        .await
        .expect("persist should be no-op when memory-sqlite is disabled");
}
