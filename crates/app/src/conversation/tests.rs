use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use loongclaw_contracts::{
    Capability, ExecutionRoute, HarnessKind, MemoryPlaneError, ToolCoreOutcome, ToolCoreRequest,
};
use loongclaw_kernel::{
    CoreMemoryAdapter, FixedClock, InMemoryAuditSink, LoongClawKernel, MemoryCoreOutcome,
    MemoryCoreRequest, StaticPolicyEngine, VerticalPackManifest,
};
#[cfg(feature = "memory-sqlite")]
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::super::config::{LoongClawConfig, MemoryProfile, MemorySystemKind, ProviderConfig};
use super::persistence::format_provider_error_reply;
use super::runtime::DefaultConversationRuntime;
use super::*;
use crate::CliResult;
use crate::KernelContext;
use crate::acp::{
    ACP_TURN_METADATA_ACK_CURSOR, ACP_TURN_METADATA_ROUTING_INTENT,
    ACP_TURN_METADATA_SOURCE_MESSAGE_ID, ACP_TURN_METADATA_TRACE_ID, AcpBackendMetadata,
    AcpCapability, AcpConversationTurnOptions, AcpRoutingIntent, AcpRuntimeBackend,
    AcpSessionBootstrap, AcpSessionHandle, AcpSessionState, AcpTurnEventSink, AcpTurnProvenance,
    AcpTurnRequest, AcpTurnResult, AcpTurnStopReason, register_acp_backend,
};
use crate::memory::MEMORY_OP_WINDOW;
#[cfg(feature = "memory-sqlite")]
use crate::memory::runtime_config::MemoryRuntimeConfig;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{NewSessionRecord, SessionKind, SessionRepository, SessionState};

struct FakeRuntime {
    seed_messages: Vec<Value>,
    assembled_context_with_system_prompt: Option<AssembledConversationContext>,
    assembled_context_without_system_prompt: Option<AssembledConversationContext>,
    tool_view_override: Option<crate::tools::ToolView>,
    completion_responses: Mutex<VecDeque<Result<String, String>>>,
    turn_responses: Mutex<VecDeque<Result<FakeTurnResponse, String>>>,
    after_turn_result: Result<(), String>,
    compact_result: Result<(), String>,
    #[cfg(feature = "memory-sqlite")]
    durable_memory_config: Option<MemoryRuntimeConfig>,
    #[cfg(feature = "memory-sqlite")]
    async_delegate_spawner_override: Option<Arc<dyn crate::conversation::AsyncDelegateSpawner>>,
    persisted: Mutex<Vec<(String, String, String)>>,
    bootstrap_calls: Mutex<Vec<String>>,
    ingested_messages: Mutex<Vec<(String, Value)>>,
    requested_messages: Mutex<Vec<Value>>,
    turn_requested_messages: Mutex<Vec<Vec<Value>>>,
    completion_requested_messages: Mutex<Vec<Vec<Value>>>,
    built_tool_views: Mutex<Vec<crate::tools::ToolView>>,
    turn_requested_tool_views: Mutex<Vec<crate::tools::ToolView>>,
    build_context_calls: Mutex<Vec<(String, bool)>>,
    turn_requested_provider_ids: Mutex<Vec<String>>,
    completion_requested_provider_ids: Mutex<Vec<String>>,
    completion_calls: Mutex<usize>,
    turn_calls: Mutex<usize>,
    after_turn_calls: Mutex<Vec<(String, String, String, usize)>>,
    compact_calls: Mutex<Vec<(String, usize)>>,
    prepare_subagent_spawn_result: Result<(), String>,
    on_subagent_ended_result: Result<(), String>,
    subagent_lifecycle_calls: Mutex<Vec<String>>,
}

enum FakeTurnResponse {
    Parsed(ProviderTurn),
    RawBody(Value),
}

struct TraitDefaultToolViewRuntime;

#[async_trait]
impl ConversationRuntime for TraitDefaultToolViewRuntime {
    async fn build_messages(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        Ok(Vec::new())
    }

    async fn request_completion(
        &self,
        _config: &LoongClawConfig,
        _messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        Err("trait default tool view test should not request a completion".to_owned())
    }

    async fn request_turn(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _turn_id: &str,
        _messages: &[Value],
        _tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn> {
        Err("trait default tool view test should not request a provider turn".to_owned())
    }

    async fn persist_turn(
        &self,
        _session_id: &str,
        _role: &str,
        _content: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<()> {
        Ok(())
    }
}

#[cfg(feature = "memory-sqlite")]
#[derive(Default)]
struct FakeAsyncDelegateSpawner {
    requests: Arc<Mutex<Vec<crate::conversation::AsyncDelegateSpawnRequest>>>,
    spawn_error: Option<String>,
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl crate::conversation::AsyncDelegateSpawner for FakeAsyncDelegateSpawner {
    async fn spawn(
        &self,
        request: crate::conversation::AsyncDelegateSpawnRequest,
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
impl crate::conversation::AsyncDelegateSpawner for PanicAsyncDelegateSpawner {
    async fn spawn(
        &self,
        _request: crate::conversation::AsyncDelegateSpawnRequest,
    ) -> Result<(), String> {
        panic!("panic-async-spawn");
    }
}

#[cfg(feature = "memory-sqlite")]
struct LocalChildRuntimeAsyncDelegateSpawner {
    config: LoongClawConfig,
    runtime: Arc<OnceLock<Arc<FakeRuntime>>>,
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl crate::conversation::AsyncDelegateSpawner for LocalChildRuntimeAsyncDelegateSpawner {
    async fn spawn(
        &self,
        request: crate::conversation::AsyncDelegateSpawnRequest,
    ) -> Result<(), String> {
        let memory_config = MemoryRuntimeConfig::from_memory_config(&self.config.memory);
        let repo = crate::session::repository::SessionRepository::new(&memory_config)?;
        let started = repo.transition_session_with_event_if_current(
            &request.child_session_id,
            crate::session::repository::TransitionSessionWithEventIfCurrentRequest {
                expected_state: crate::session::repository::SessionState::Ready,
                next_state: crate::session::repository::SessionState::Running,
                last_error: None,
                event_kind: "delegate_started".to_owned(),
                actor_session_id: Some(request.parent_session_id.clone()),
                event_payload_json: json!({
                    "task": request.task,
                    "label": request.label,
                    "timeout_seconds": request.timeout_seconds,
                }),
            },
        )?;
        if started.is_none() {
            return Ok(());
        }

        let runtime = self
            .runtime
            .get()
            .ok_or_else(|| "test_local_delegate_runtime_missing".to_owned())?;
        let _ = super::turn_coordinator::run_started_delegate_child_turn_with_runtime(
            &self.config,
            runtime.as_ref(),
            &request.child_session_id,
            &request.parent_session_id,
            request.label,
            &request.task,
            request.execution,
            request.timeout_seconds,
            ConversationRuntimeBinding::from_optional_kernel_context(
                request.kernel_context.as_ref(),
            ),
        )
        .await;
        Ok(())
    }
}

#[cfg(feature = "memory-sqlite")]
struct GatedFakeAsyncDelegateSpawner {
    requests: Arc<Mutex<Vec<crate::conversation::AsyncDelegateSpawnRequest>>>,
    sender:
        Mutex<Option<tokio::sync::oneshot::Sender<crate::conversation::AsyncDelegateSpawnRequest>>>,
    release: Arc<tokio::sync::Notify>,
}

#[cfg(feature = "memory-sqlite")]
impl GatedFakeAsyncDelegateSpawner {
    fn new() -> (
        Self,
        tokio::sync::oneshot::Receiver<crate::conversation::AsyncDelegateSpawnRequest>,
        Arc<tokio::sync::Notify>,
    ) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let release = Arc::new(tokio::sync::Notify::new());
        (
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                sender: Mutex::new(Some(tx)),
                release: release.clone(),
            },
            rx,
            release,
        )
    }
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl crate::conversation::AsyncDelegateSpawner for GatedFakeAsyncDelegateSpawner {
    async fn spawn(
        &self,
        request: crate::conversation::AsyncDelegateSpawnRequest,
    ) -> Result<(), String> {
        self.requests
            .lock()
            .expect("async delegate requests lock")
            .push(request.clone());
        if let Some(sender) = self.sender.lock().expect("gated sender lock").take() {
            let _ = sender.send(request);
        }
        self.release.notified().await;
        Ok(())
    }
}

struct StubContextEngine;
struct StubEnvContextEngine;
struct StubSystemPromptAdditionEngine;
struct RecordingLifecycleContextEngine {
    calls: Arc<Mutex<Vec<String>>>,
}

#[derive(Default)]
struct RoutedAcpState {
    ensure_calls: usize,
    turn_calls: usize,
    last_bootstrap: Option<AcpSessionBootstrap>,
    last_request: Option<AcpTurnRequest>,
}

struct RoutedAcpBackend {
    id: &'static str,
    shared: Arc<Mutex<RoutedAcpState>>,
    fail_turn: bool,
    emitted_events: Vec<Value>,
}

#[derive(Default)]
struct RecordingAcpEventSink {
    events: Mutex<Vec<Value>>,
}

impl RecordingAcpEventSink {
    fn snapshot(&self) -> Vec<Value> {
        self.events
            .lock()
            .expect("recording ACP event sink lock")
            .clone()
    }
}

impl AcpTurnEventSink for RecordingAcpEventSink {
    fn on_event(&self, event: &Value) -> CliResult<()> {
        self.events
            .lock()
            .expect("recording ACP event sink lock")
            .push(event.clone());
        Ok(())
    }
}

#[async_trait]
impl ConversationContextEngine for StubContextEngine {
    fn id(&self) -> &'static str {
        "stub-context-engine"
    }

    async fn assemble_messages(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        Ok(vec![json!({
            "role": "system",
            "content": "stub-context-engine",
        })])
    }
}

#[async_trait]
impl ConversationContextEngine for StubEnvContextEngine {
    fn id(&self) -> &'static str {
        "stub-env-context-engine"
    }

    async fn assemble_messages(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        Ok(vec![json!({
            "role": "system",
            "content": "stub-env-context-engine",
        })])
    }
}

#[async_trait]
impl ConversationContextEngine for StubSystemPromptAdditionEngine {
    fn id(&self) -> &'static str {
        "stub-system-prompt-addition"
    }

    fn metadata(&self) -> ContextEngineMetadata {
        ContextEngineMetadata::new(self.id(), [ContextEngineCapability::SystemPromptAddition])
    }

    async fn assemble_context(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        Ok(AssembledConversationContext {
            messages: vec![json!({
                "role": "system",
                "content": "base-system-prompt",
            })],
            estimated_tokens: Some(42),
            system_prompt_addition: Some("runtime-policy-addition".to_owned()),
        })
    }

    async fn assemble_messages(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        Ok(vec![json!({
            "role": "system",
            "content": "base-system-prompt",
        })])
    }
}

#[async_trait]
impl ConversationContextEngine for RecordingLifecycleContextEngine {
    fn id(&self) -> &'static str {
        "recording-lifecycle-context-engine"
    }

    async fn bootstrap(
        &self,
        _config: &LoongClawConfig,
        session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineBootstrapResult> {
        self.calls
            .lock()
            .expect("recording context engine lock")
            .push(format!("bootstrap:{session_id}"));
        Ok(ContextEngineBootstrapResult {
            bootstrapped: true,
            imported_messages: Some(0),
            reason: None,
        })
    }

    async fn ingest(
        &self,
        session_id: &str,
        message: &Value,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineIngestResult> {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        self.calls
            .lock()
            .expect("recording context engine lock")
            .push(format!("ingest:{session_id}:{role}"));
        Ok(ContextEngineIngestResult { ingested: true })
    }

    async fn prepare_subagent_spawn(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.calls
            .lock()
            .expect("recording context engine lock")
            .push(format!(
                "prepare_subagent_spawn:{parent_session_id}:{subagent_session_id}"
            ));
        Ok(())
    }

    async fn on_subagent_ended(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.calls
            .lock()
            .expect("recording context engine lock")
            .push(format!(
                "on_subagent_ended:{parent_session_id}:{subagent_session_id}"
            ));
        Ok(())
    }

    async fn assemble_messages(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        Ok(Vec::new())
    }
}

fn context_engine_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct ScopedEnvVar {
    previous: Option<Option<String>>,
}

impl ScopedEnvVar {
    fn set(key: &'static str, value: &str) -> Self {
        assert_eq!(key, CONTEXT_ENGINE_ENV, "unexpected scoped env key");
        let previous = Some(super::context_engine_registry::context_engine_id_from_env());
        super::context_engine_registry::set_context_engine_env_override(Some(value));
        Self { previous }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            super::context_engine_registry::set_context_engine_env_override(previous.as_deref());
        } else {
            super::context_engine_registry::clear_context_engine_env_override();
        }
    }
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

    fn with_turns_and_completions(
        seed_messages: Vec<Value>,
        turns: Vec<Result<ProviderTurn, String>>,
        completions: Vec<Result<String, String>>,
    ) -> Self {
        let turn_responses = turns
            .into_iter()
            .map(|turn| turn.map(FakeTurnResponse::Parsed))
            .collect::<Vec<_>>();
        Self::with_fake_turn_responses(seed_messages, turn_responses, completions)
    }

    fn with_turn_bodies_and_completions(
        seed_messages: Vec<Value>,
        bodies: Vec<Result<Value, String>>,
        completions: Vec<Result<String, String>>,
    ) -> Self {
        let turn_responses = bodies
            .into_iter()
            .map(|body| body.map(FakeTurnResponse::RawBody))
            .collect::<Vec<_>>();
        Self::with_fake_turn_responses(seed_messages, turn_responses, completions)
    }

    fn with_fake_turn_responses(
        seed_messages: Vec<Value>,
        turn_responses: Vec<Result<FakeTurnResponse, String>>,
        completions: Vec<Result<String, String>>,
    ) -> Self {
        Self {
            seed_messages,
            assembled_context_with_system_prompt: None,
            assembled_context_without_system_prompt: None,
            tool_view_override: None,
            completion_responses: Mutex::new(VecDeque::from(completions)),
            turn_responses: Mutex::new(VecDeque::from(turn_responses)),
            after_turn_result: Ok(()),
            compact_result: Ok(()),
            #[cfg(feature = "memory-sqlite")]
            durable_memory_config: None,
            #[cfg(feature = "memory-sqlite")]
            async_delegate_spawner_override: None,
            persisted: Mutex::new(Vec::new()),
            bootstrap_calls: Mutex::new(Vec::new()),
            ingested_messages: Mutex::new(Vec::new()),
            requested_messages: Mutex::new(Vec::new()),
            turn_requested_messages: Mutex::new(Vec::new()),
            completion_requested_messages: Mutex::new(Vec::new()),
            built_tool_views: Mutex::new(Vec::new()),
            turn_requested_tool_views: Mutex::new(Vec::new()),
            build_context_calls: Mutex::new(Vec::new()),
            turn_requested_provider_ids: Mutex::new(Vec::new()),
            completion_requested_provider_ids: Mutex::new(Vec::new()),
            completion_calls: Mutex::new(0),
            turn_calls: Mutex::new(0),
            after_turn_calls: Mutex::new(Vec::new()),
            compact_calls: Mutex::new(Vec::new()),
            prepare_subagent_spawn_result: Ok(()),
            on_subagent_ended_result: Ok(()),
            subagent_lifecycle_calls: Mutex::new(Vec::new()),
        }
    }

    fn with_tool_view(mut self, tool_view: crate::tools::ToolView) -> Self {
        self.tool_view_override = Some(tool_view);
        self
    }

    fn with_assembled_context(mut self, assembled_context: AssembledConversationContext) -> Self {
        self.assembled_context_with_system_prompt = Some(assembled_context.clone());
        self.assembled_context_without_system_prompt = Some(assembled_context);
        self
    }

    fn with_assembled_context_variants(
        mut self,
        with_system_prompt: AssembledConversationContext,
        without_system_prompt: AssembledConversationContext,
    ) -> Self {
        self.assembled_context_with_system_prompt = Some(with_system_prompt);
        self.assembled_context_without_system_prompt = Some(without_system_prompt);
        self
    }

    fn with_after_turn_result(mut self, result: Result<(), String>) -> Self {
        self.after_turn_result = result;
        self
    }

    fn with_compact_result(mut self, result: Result<(), String>) -> Self {
        self.compact_result = result;
        self
    }

    fn with_prepare_subagent_spawn_result(mut self, result: Result<(), String>) -> Self {
        self.prepare_subagent_spawn_result = result;
        self
    }

    #[cfg(feature = "memory-sqlite")]
    fn with_durable_memory_config(mut self, config: MemoryRuntimeConfig) -> Self {
        self.durable_memory_config = Some(config);
        self
    }

    #[cfg(feature = "memory-sqlite")]
    fn with_async_delegate_spawner(
        mut self,
        spawner: Arc<dyn crate::conversation::AsyncDelegateSpawner>,
    ) -> Self {
        self.async_delegate_spawner_override = Some(spawner);
        self
    }
}

fn unique_acp_test_id(prefix: &str, suffix: &str) -> String {
    format!(
        "{prefix}-{suffix}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    )
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

fn register_routed_acp_backend(
    suffix: &str,
    fail_turn: bool,
) -> (&'static str, Arc<Mutex<RoutedAcpState>>) {
    register_routed_acp_backend_with_events(suffix, fail_turn, Vec::new())
}

fn register_routed_acp_backend_with_events(
    suffix: &str,
    fail_turn: bool,
    emitted_events: Vec<Value>,
) -> (&'static str, Arc<Mutex<RoutedAcpState>>) {
    let backend_id: &'static str =
        Box::leak(unique_acp_test_id("conversation-acp-backend", suffix).into_boxed_str());
    let shared = Arc::new(Mutex::new(RoutedAcpState::default()));
    register_acp_backend(backend_id, {
        let shared = shared.clone();
        move || {
            Box::new(RoutedAcpBackend {
                id: backend_id,
                shared: shared.clone(),
                fail_turn,
                emitted_events: emitted_events.clone(),
            })
        }
    })
    .expect("register routed ACP backend");
    (backend_id, shared)
}

fn unique_acp_sqlite_path(suffix: &str) -> String {
    std::env::temp_dir()
        .join(format!(
            "{}.sqlite3",
            unique_acp_test_id("conversation-acp", suffix)
        ))
        .display()
        .to_string()
}

fn persisted_conversation_event_payloads_by_name(
    persisted: &[(String, String, String)],
    event_name: &str,
) -> Vec<Value> {
    persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            (parsed.get("event")?.as_str()? == event_name)
                .then(|| parsed.get("payload").cloned().unwrap_or(Value::Null))
        })
        .collect()
}

fn assert_discovery_first_followup_summary(
    persisted: &[(String, String, String)],
    raw_tool_output_requested: bool,
    expected_target_tool_id: &str,
) {
    let summary = super::analytics::summarize_discovery_first_events(
        persisted
            .iter()
            .filter_map(|(_, role, content)| (role == "assistant").then_some(content.as_str())),
    );
    assert_eq!(summary.search_round_events, 1);
    assert_eq!(summary.followup_requested_events, 1);
    assert_eq!(summary.followup_result_events, 1);
    assert_eq!(
        summary.raw_output_followup_events,
        u32::from(raw_tool_output_requested)
    );
    assert_eq!(summary.search_to_invoke_hits, 1);
    assert_eq!(
        summary.latest_followup_outcome.as_deref(),
        Some("tool.invoke")
    );
    assert_eq!(
        summary.latest_followup_tool_name.as_deref(),
        Some("tool.invoke")
    );
    assert_eq!(
        summary.latest_followup_target_tool_id.as_deref(),
        Some(expected_target_tool_id)
    );
    assert!(
        summary.aggregate_added_estimated_tokens > 0,
        "follow-up telemetry should record a positive added token estimate: {summary:?}"
    );
}

async fn run_provider_shape_tool_search_followup(
    session_id: &str,
    user_input: &str,
    note_contents: &str,
    first_body: Value,
    second_body: Value,
    completion: Result<String, String>,
) -> (String, FakeRuntime) {
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(harness.temp_dir.join("note.md"), note_contents).expect("seed test note");

    let runtime = FakeRuntime::with_turn_bodies_and_completions(
        vec![],
        vec![Ok(first_body), Ok(second_body)],
        vec![completion],
    );

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &test_config(),
            session_id,
            user_input,
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::from_optional_kernel_context(Some(&harness.kernel_ctx)),
        )
        .await
        .expect("provider-shape discovery-first followup should succeed");

    (reply, runtime)
}

fn is_internal_assistant_record(content: &str) -> bool {
    serde_json::from_str::<Value>(content)
        .ok()
        .and_then(|parsed| {
            parsed
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .map(|event_type| {
            matches!(
                event_type.as_str(),
                "conversation_event" | "tool_decision" | "tool_outcome"
            )
        })
        .unwrap_or(false)
}

fn persisted_visible_turns(
    persisted: &[(String, String, String)],
) -> Vec<(String, String, String)> {
    persisted
        .iter()
        .filter(|(_, role, content)| *role != "assistant" || !is_internal_assistant_record(content))
        .cloned()
        .collect()
}

fn test_turn_checkpoint_identity(user_input: &str, assistant_reply: &str) -> Value {
    json!({
        "user_input_sha256": format!("{:x}", Sha256::digest(user_input.as_bytes())),
        "assistant_reply_sha256": format!("{:x}", Sha256::digest(assistant_reply.as_bytes())),
        "user_input_chars": user_input.chars().count(),
        "assistant_reply_chars": assistant_reply.chars().count(),
    })
}

fn test_turn_preparation_context_fingerprint(messages: &[Value]) -> String {
    let serialized =
        serde_json::to_vec(messages).expect("serializing test preparation messages should work");
    format!("{:x}", Sha256::digest(serialized))
}

fn unique_memory_sqlite_path(suffix: &str) -> String {
    std::env::temp_dir()
        .join(format!(
            "{}.sqlite3",
            unique_acp_test_id("conversation-memory", suffix)
        ))
        .display()
        .to_string()
}

fn persisted_canonical_records(
    persisted: &[(String, String, String)],
) -> Vec<crate::memory::CanonicalMemoryRecord> {
    persisted
        .iter()
        .map(|(session_id, role, content)| {
            crate::memory::canonical_memory_record_from_persisted_turn(
                session_id.as_str(),
                role.as_str(),
                content.as_str(),
            )
        })
        .collect()
}

#[async_trait]
impl AcpRuntimeBackend for RoutedAcpBackend {
    fn id(&self) -> &'static str {
        self.id
    }

    fn metadata(&self) -> AcpBackendMetadata {
        AcpBackendMetadata::new(
            self.id(),
            [
                AcpCapability::SessionLifecycle,
                AcpCapability::TurnExecution,
            ],
            "Conversation ACP routing test backend",
        )
    }

    async fn ensure_session(
        &self,
        _config: &LoongClawConfig,
        request: &AcpSessionBootstrap,
    ) -> CliResult<AcpSessionHandle> {
        let mut guard = self.shared.lock().expect("routed ACP state lock");
        guard.ensure_calls += 1;
        guard.last_bootstrap = Some(request.clone());
        Ok(AcpSessionHandle {
            session_key: request.session_key.clone(),
            backend_id: self.id().to_owned(),
            runtime_session_name: format!("routed-{}", request.session_key),
            working_directory: None,
            backend_session_id: Some(format!("backend-{}", request.session_key)),
            agent_session_id: Some(format!("agent-{}", request.session_key)),
            binding: request.binding.clone(),
        })
    }

    async fn run_turn(
        &self,
        _config: &LoongClawConfig,
        _session: &AcpSessionHandle,
        request: &AcpTurnRequest,
    ) -> CliResult<AcpTurnResult> {
        let mut guard = self.shared.lock().expect("routed ACP state lock");
        guard.turn_calls += 1;
        guard.last_request = Some(request.clone());
        if self.fail_turn {
            return Err("synthetic ACP routing failure".to_owned());
        }
        Ok(AcpTurnResult {
            output_text: format!("acp: {}", request.input),
            state: AcpSessionState::Ready,
            usage: None,
            events: self.emitted_events.clone(),
            stop_reason: Some(AcpTurnStopReason::Completed),
        })
    }

    async fn cancel(
        &self,
        _config: &LoongClawConfig,
        _session: &AcpSessionHandle,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn close(&self, _config: &LoongClawConfig, _session: &AcpSessionHandle) -> CliResult<()> {
        Ok(())
    }
}

#[async_trait]
impl ConversationRuntime for FakeRuntime {
    fn tool_view(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<crate::tools::ToolView> {
        match self.tool_view_override.clone() {
            Some(tool_view) => Ok(tool_view),
            None => DefaultConversationRuntime::default().tool_view(config, session_id, binding),
        }
    }

    #[cfg(feature = "memory-sqlite")]
    fn async_delegate_spawner(
        &self,
        _config: &LoongClawConfig,
    ) -> Option<Arc<dyn crate::conversation::AsyncDelegateSpawner>> {
        self.async_delegate_spawner_override.clone()
    }

    async fn bootstrap(
        &self,
        _config: &LoongClawConfig,
        session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineBootstrapResult> {
        self.bootstrap_calls
            .lock()
            .expect("bootstrap lock")
            .push(session_id.to_owned());
        Ok(ContextEngineBootstrapResult {
            bootstrapped: true,
            imported_messages: Some(0),
            reason: None,
        })
    }

    async fn ingest(
        &self,
        session_id: &str,
        message: &Value,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineIngestResult> {
        self.ingested_messages
            .lock()
            .expect("ingest lock")
            .push((session_id.to_owned(), message.clone()));
        Ok(ContextEngineIngestResult { ingested: true })
    }
    async fn build_messages(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        include_system_prompt: bool,
        tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        self.built_tool_views
            .lock()
            .expect("built tool views lock")
            .push(tool_view.clone());
        let assembled = if include_system_prompt {
            self.assembled_context_with_system_prompt.as_ref()
        } else {
            self.assembled_context_without_system_prompt.as_ref()
        };
        Ok(assembled
            .map(|context| context.messages.clone())
            .unwrap_or_else(|| self.seed_messages.clone()))
    }

    async fn build_context(
        &self,
        _config: &LoongClawConfig,
        session_id: &str,
        include_system_prompt: bool,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        self.build_context_calls
            .lock()
            .expect("build context lock")
            .push((session_id.to_owned(), include_system_prompt));
        let assembled = if include_system_prompt {
            self.assembled_context_with_system_prompt.clone()
        } else {
            self.assembled_context_without_system_prompt.clone()
        };
        Ok(assembled.unwrap_or_else(|| {
            AssembledConversationContext::from_messages(self.seed_messages.clone())
        }))
    }

    async fn request_completion(
        &self,
        config: &LoongClawConfig,
        messages: &[Value],
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let mut calls = self.completion_calls.lock().expect("completion calls lock");
        *calls += 1;
        *self.requested_messages.lock().expect("request lock") = messages.to_vec();
        self.completion_requested_messages
            .lock()
            .expect("completion request lock")
            .push(messages.to_vec());
        self.completion_requested_provider_ids
            .lock()
            .expect("completion provider ids lock")
            .push(config.active_provider_id().unwrap_or_default().to_owned());
        drop(calls);
        self.completion_responses
            .lock()
            .expect("completion response lock")
            .pop_front()
            .unwrap_or_else(|| Err("unexpected_completion_call".to_owned()))
    }

    async fn request_turn(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &crate::tools::ToolView,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn> {
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
        self.turn_requested_provider_ids
            .lock()
            .expect("turn provider ids lock")
            .push(config.active_provider_id().unwrap_or_default().to_owned());
        drop(calls);
        match self
            .turn_responses
            .lock()
            .expect("turn response lock")
            .pop_front()
            .unwrap_or_else(|| Err("unexpected_turn_call".to_owned()))
        {
            Ok(FakeTurnResponse::Parsed(turn)) => Ok(turn),
            Ok(FakeTurnResponse::RawBody(body)) => {
                crate::provider::extract_provider_turn_with_scope_and_messages(
                    &body,
                    Some(session_id),
                    Some(turn_id),
                    messages,
                )
                .ok_or_else(|| "fake_runtime_failed_to_parse_provider_body".to_owned())
            }
            Err(error) => Err(error),
        }
    }

    async fn persist_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<()> {
        #[cfg(feature = "memory-sqlite")]
        if let Some(config) = self.durable_memory_config.as_ref() {
            crate::memory::append_turn_direct(session_id, role, content, config)
                .map_err(|error| format!("persist {role} turn failed: {error}"))?;
        }
        self.persisted.lock().expect("persist lock").push((
            session_id.to_owned(),
            role.to_owned(),
            content.to_owned(),
        ));
        Ok(())
    }

    async fn after_turn(
        &self,
        session_id: &str,
        user_input: &str,
        assistant_reply: &str,
        messages: &[Value],
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.after_turn_calls
            .lock()
            .expect("after-turn lock")
            .push((
                session_id.to_owned(),
                user_input.to_owned(),
                assistant_reply.to_owned(),
                messages.len(),
            ));
        self.after_turn_result.clone()
    }

    async fn compact_context(
        &self,
        _config: &LoongClawConfig,
        session_id: &str,
        messages: &[Value],
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.compact_calls
            .lock()
            .expect("compact lock")
            .push((session_id.to_owned(), messages.len()));
        self.compact_result.clone()
    }

    async fn prepare_subagent_spawn(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.subagent_lifecycle_calls
            .lock()
            .expect("subagent lifecycle calls lock")
            .push(format!(
                "prepare_subagent_spawn:{parent_session_id}:{subagent_session_id}"
            ));
        self.prepare_subagent_spawn_result.clone()
    }

    async fn on_subagent_ended(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.subagent_lifecycle_calls
            .lock()
            .expect("subagent lifecycle calls lock")
            .push(format!(
                "on_subagent_ended:{parent_session_id}:{subagent_session_id}"
            ));
        self.on_subagent_ended_result.clone()
    }
}

fn test_config() -> LoongClawConfig {
    LoongClawConfig {
        provider: ProviderConfig::default(),
        ..LoongClawConfig::default()
    }
}

fn test_kernel_context(agent_id: &str) -> KernelContext {
    crate::context::bootstrap_kernel_context(agent_id, 60).expect("bootstrap test kernel context")
}

#[cfg(feature = "memory-sqlite")]
fn test_kernel_context_with_memory(
    agent_id: &str,
    memory_config: &MemoryRuntimeConfig,
) -> KernelContext {
    let clock = Arc::new(FixedClock::new(1_700_000_000));
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut kernel = LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);

    let pack = VerticalPackManifest {
        pack_id: "test-pack-memory".to_owned(),
        domain: "testing".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([Capability::MemoryRead, Capability::MemoryWrite]),
        metadata: BTreeMap::new(),
    };
    kernel
        .register_pack(pack)
        .expect("register memory test pack");
    kernel.register_core_memory_adapter(crate::memory::MvpMemoryAdapter::with_config(
        memory_config.clone(),
    ));
    kernel
        .set_default_core_memory_adapter("mvp-memory")
        .expect("set memory test adapter");

    let token = kernel
        .issue_token("test-pack-memory", agent_id, 60)
        .expect("issue memory test token");

    KernelContext {
        kernel: Arc::new(kernel),
        token,
    }
}

#[cfg(feature = "memory-sqlite")]
fn sample_delegate_runtime_narrowing() -> crate::tools::runtime_config::ToolRuntimeNarrowing {
    crate::tools::runtime_config::ToolRuntimeNarrowing {
        browser: crate::tools::runtime_config::BrowserRuntimeNarrowing {
            max_sessions: Some(1),
            max_links: Some(8),
            max_text_chars: Some(512),
        },
        web_fetch: crate::tools::runtime_config::WebFetchRuntimeNarrowing {
            allow_private_hosts: Some(false),
            allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
            blocked_domains: BTreeSet::from(["deny.example.com".to_owned()]),
            timeout_seconds: Some(5),
            max_bytes: Some(4_096),
            max_redirects: Some(2),
        },
    }
}

#[cfg(feature = "memory-sqlite")]
fn seed_delegate_child_session_with_runtime_narrowing(
    config: &mut LoongClawConfig,
    suffix: &str,
    runtime_narrowing: crate::tools::runtime_config::ToolRuntimeNarrowing,
) -> String {
    let sqlite_path = unique_memory_sqlite_path(&format!("delegate-runtime-contract-{suffix}"));
    config.memory.sqlite_path = sqlite_path;
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    let root_session_id = format!("root-session-{suffix}");
    let child_session_id = format!("child-session-{suffix}");

    repo.create_session(NewSessionRecord {
        session_id: root_session_id.clone(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(NewSessionRecord {
        session_id: child_session_id.clone(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some(root_session_id.clone()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child session");
    repo.append_event(crate::session::repository::NewSessionEvent {
        session_id: child_session_id.clone(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some(root_session_id),
        payload_json: json!({
            "task": "fetch docs",
            "label": "Child",
            "execution": {
                "mode": "inline",
                "depth": 1,
                "max_depth": 2,
                "active_children": 0,
                "max_active_children": 3,
                "timeout_seconds": 60,
                "allow_shell_in_child": false,
                "child_tool_allowlist": ["web.fetch"],
                "kernel_bound": true,
                "runtime_narrowing": runtime_narrowing,
            }
        }),
    })
    .expect("append delegate_started event");

    child_session_id
}

fn provider_tool_intent(
    tool_name: &str,
    args_json: Value,
    session_id: &str,
    turn_id: &str,
    tool_call_id: &str,
) -> ToolIntent {
    let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
        tool_name,
        args_json,
        Some(session_id),
        Some(turn_id),
    );
    ToolIntent {
        tool_name,
        args_json,
        source: "provider_tool_call".to_owned(),
        session_id: session_id.to_owned(),
        turn_id: turn_id.to_owned(),
        tool_call_id: tool_call_id.to_owned(),
    }
}

fn effective_tool_request(request: &ToolCoreRequest) -> (String, &Value) {
    let tool_name = crate::tools::canonical_tool_name(request.tool_name.as_str());
    if tool_name != "tool.invoke" {
        return (tool_name.to_owned(), &request.payload);
    }
    let invoked_tool = request
        .payload
        .get("tool_id")
        .and_then(Value::as_str)
        .map(crate::tools::canonical_tool_name)
        .unwrap_or(tool_name)
        .to_owned();
    let arguments = request.payload.get("arguments").unwrap_or(&request.payload);
    (invoked_tool, arguments)
}
#[tokio::test]
async fn default_runtime_supports_injected_context_engine() {
    let runtime = DefaultConversationRuntime::with_context_engine(StubContextEngine);
    let binding = crate::conversation::ConversationRuntimeBinding::direct();
    let tool_view = runtime
        .tool_view(&test_config(), "session-injected", binding)
        .expect("default runtime tool view");
    let messages = runtime
        .build_messages(
            &test_config(),
            "session-injected",
            true,
            &tool_view,
            binding,
        )
        .await
        .expect("build messages via injected context engine");

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "stub-context-engine");
}

#[tokio::test]
async fn default_runtime_can_resolve_context_engine_from_registry() {
    register_context_engine("stub-registry", || Box::new(StubContextEngine))
        .expect("register context engine");
    let runtime = DefaultConversationRuntime::from_engine_id(Some("stub-registry"))
        .expect("resolve context engine from registry");
    let binding = crate::conversation::ConversationRuntimeBinding::direct();
    let tool_view = runtime
        .tool_view(&test_config(), "session-registry", binding)
        .expect("default runtime tool view");
    let messages = runtime
        .build_messages(
            &test_config(),
            "session-registry",
            true,
            &tool_view,
            binding,
        )
        .await
        .expect("build messages via registry context engine");

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["content"], "stub-context-engine");
}

#[tokio::test]
#[allow(clippy::await_holding_lock)] // env var mutation is process-global; keep lock for full test body.
async fn default_runtime_prefers_configured_context_engine_when_env_not_set() {
    let _env_lock = context_engine_env_lock().lock().expect("env lock");
    register_context_engine("stub-config", || Box::new(StubContextEngine))
        .expect("register context engine");
    let _scoped_env = ScopedEnvVar::set(CONTEXT_ENGINE_ENV, "");
    let mut config = test_config();
    config.conversation.context_engine = Some("stub-config".to_owned());

    let runtime = DefaultConversationRuntime::from_config_or_env(&config)
        .expect("resolve context engine from config");
    let binding = ConversationRuntimeBinding::direct();
    let tool_view = runtime
        .tool_view(&config, "session-config", binding)
        .expect("configured runtime tool view");
    let messages = runtime
        .build_messages(&config, "session-config", true, &tool_view, binding)
        .await
        .expect("build messages via configured context engine");

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["content"], "stub-context-engine");
}

#[test]
fn default_runtime_exposes_context_engine_metadata() {
    let runtime = DefaultConversationRuntime::default();
    let metadata = runtime.context_engine_metadata();
    assert_eq!(metadata.id, DEFAULT_CONTEXT_ENGINE_ID);
    assert_eq!(metadata.api_version, CONTEXT_ENGINE_API_VERSION);
}

#[tokio::test]
async fn default_runtime_build_messages_respects_restricted_tool_view() {
    let runtime = DefaultConversationRuntime::default();
    let view = crate::tools::ToolView::from_tool_names(["file.read"]);
    let binding = ConversationRuntimeBinding::direct();

    let messages = runtime
        .build_messages(&test_config(), "noop-session", true, &view, binding)
        .await
        .expect("build messages");

    assert!(!messages.is_empty());
    let system_content = messages[0]["content"].as_str().expect("system content");
    assert!(system_content.contains("- tool.search: Discover non-core tools"));
    assert!(system_content.contains("- tool.invoke: Invoke a discovered non-core tool"));
    assert!(system_content.contains("Non-core tools are intentionally hidden"));
    assert!(!system_content.contains("- file.read:"));
    assert!(!system_content.contains("- file.write:"));
    assert!(!system_content.contains("- shell.exec:"));
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn default_runtime_tool_view_uses_persisted_delegate_child_restrictions() {
    let mut config = test_config();
    config.tools.delegate.allow_shell_in_child = false;
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-tool-view", "persisted-child")
    ));
    let _ = std::fs::remove_file(&db_path);
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config =
        crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child session");

    let runtime = DefaultConversationRuntime::default();
    let child_view = runtime
        .tool_view(
            &config,
            "child-session",
            ConversationRuntimeBinding::direct(),
        )
        .expect("child tool view");

    assert!(child_view.contains("file.read"));
    assert!(child_view.contains("file.write"));
    assert!(!child_view.contains("shell.exec"));
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn default_runtime_tool_view_denies_delegate_for_broken_lineage_child() {
    let mut config = test_config();
    config.tools.delegate.allow_shell_in_child = false;
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-tool-view", "broken-lineage")
    ));
    let _ = std::fs::remove_file(&db_path);
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config =
        crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("missing-parent".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child session");

    let runtime = DefaultConversationRuntime::default();
    let child_view = runtime
        .tool_view(
            &config,
            "child-session",
            ConversationRuntimeBinding::direct(),
        )
        .expect("child tool view");

    assert!(child_view.contains("file.read"));
    assert!(child_view.contains("file.write"));
    assert!(!child_view.contains("shell.exec"));
    assert!(!child_view.contains("delegate"));
    assert!(!child_view.contains("delegate_async"));
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn default_runtime_session_context_uses_persisted_parent_session_id() {
    let mut config = test_config();
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-session-context", "persisted-child")
    ));
    let _ = std::fs::remove_file(&db_path);
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config =
        crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child session");

    let runtime = DefaultConversationRuntime::default();
    let session_context = runtime
        .session_context(
            &config,
            "child-session",
            ConversationRuntimeBinding::direct(),
        )
        .expect("session context");

    assert_eq!(session_context.session_id, "child-session");
    assert_eq!(
        session_context.parent_session_id.as_deref(),
        Some("root-session")
    );
}

#[tokio::test]
async fn default_runtime_delegates_bootstrap_and_ingest_to_context_engine_with_kernel() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let runtime =
        DefaultConversationRuntime::with_context_engine(RecordingLifecycleContextEngine {
            calls: calls.clone(),
        });
    let kernel_ctx = crate::context::bootstrap_kernel_context("test-runtime-lifecycle", 60)
        .expect("bootstrap kernel context");

    let bootstrap = runtime
        .bootstrap(&test_config(), "session-lifecycle", &kernel_ctx)
        .await
        .expect("bootstrap should delegate to context engine");
    let ingest = runtime
        .ingest(
            "session-lifecycle",
            &json!({
                "role": "user",
                "content": "hello",
            }),
            &kernel_ctx,
        )
        .await
        .expect("ingest should delegate to context engine");

    assert!(bootstrap.bootstrapped);
    assert!(ingest.ingested);
    assert_eq!(
        calls.lock().expect("recording calls lock").clone(),
        vec![
            "bootstrap:session-lifecycle".to_owned(),
            "ingest:session-lifecycle:user".to_owned(),
        ]
    );
}

#[tokio::test]
async fn default_runtime_delegates_subagent_lifecycle_to_context_engine_with_kernel() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let runtime =
        DefaultConversationRuntime::with_context_engine(RecordingLifecycleContextEngine {
            calls: calls.clone(),
        });
    let kernel_ctx = crate::context::bootstrap_kernel_context("test-runtime-subagent", 60)
        .expect("bootstrap kernel context");

    runtime
        .prepare_subagent_spawn("session-parent", "session-child", &kernel_ctx)
        .await
        .expect("prepare_subagent_spawn should delegate to context engine");
    runtime
        .on_subagent_ended("session-parent", "session-child", &kernel_ctx)
        .await
        .expect("on_subagent_ended should delegate to context engine");

    assert_eq!(
        calls.lock().expect("recording calls lock").clone(),
        vec![
            "prepare_subagent_spawn:session-parent:session-child".to_owned(),
            "on_subagent_ended:session-parent:session-child".to_owned(),
        ]
    );
}

#[tokio::test]
async fn default_runtime_build_context_applies_system_prompt_addition() {
    let runtime = DefaultConversationRuntime::with_context_engine(StubSystemPromptAdditionEngine);
    let binding = crate::conversation::ConversationRuntimeBinding::direct();
    let assembled = runtime
        .build_context(&test_config(), "session-system-addition", true, binding)
        .await
        .expect("build context with system prompt addition");

    assert_eq!(assembled.estimated_tokens, Some(42));
    assert_eq!(
        assembled.system_prompt_addition.as_deref(),
        Some("runtime-policy-addition")
    );
    assert_eq!(assembled.messages.len(), 1);
    assert_eq!(assembled.messages[0]["role"], "system");
    let merged = assembled.messages[0]["content"]
        .as_str()
        .expect("system prompt should stay string");
    assert_eq!(
        merged, "runtime-policy-addition\n\nbase-system-prompt",
        "system prompt addition should be prepended"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn default_runtime_build_context_merges_delegate_runtime_contract_with_system_prompt_addition()
 {
    let mut config = test_config();
    let child_session_id = seed_delegate_child_session_with_runtime_narrowing(
        &mut config,
        "system-prompt-contract",
        sample_delegate_runtime_narrowing(),
    );
    let runtime = DefaultConversationRuntime::with_context_engine(StubSystemPromptAdditionEngine);
    let binding = crate::conversation::ConversationRuntimeBinding::direct();

    let assembled = runtime
        .build_context(&config, &child_session_id, true, binding)
        .await
        .expect("build context with delegate runtime contract");

    let merged = assembled.messages[0]["content"]
        .as_str()
        .expect("system prompt should stay string");
    assert!(
        merged.contains("runtime-policy-addition"),
        "expected existing addition to remain in merged prompt, got: {merged}"
    );
    assert!(
        merged.contains("[delegate_child_runtime_contract]"),
        "expected delegate runtime contract marker, got: {merged}"
    );
    assert!(merged.contains("Plan within these child-session runtime limits:"));
    assert!(merged.contains("- web.fetch private hosts: denied"));
    assert!(merged.contains("- web.fetch allowed domains: docs.example.com"));
    assert!(merged.contains("- web.fetch blocked domains: deny.example.com"));
    assert!(merged.contains("- web.fetch timeout seconds: 5"));
    assert!(merged.contains("- web.fetch max bytes: 4096"));
    assert!(merged.contains("- web.fetch max redirects: 2"));
    assert!(merged.contains("- browser max sessions: 1"));
    assert!(merged.contains("- browser max links: 8"));
    assert!(merged.contains("- browser max text chars: 512"));
    assert!(merged.ends_with("base-system-prompt"));
    assert!(
        merged.find("runtime-policy-addition") < merged.find("[delegate_child_runtime_contract]"),
        "expected the existing system prompt addition to remain ahead of the child contract block, got: {merged}"
    );
}

#[tokio::test]
async fn default_runtime_build_context_does_not_add_delegate_runtime_contract_for_root_session() {
    let runtime = DefaultConversationRuntime::default();
    let binding = crate::conversation::ConversationRuntimeBinding::direct();

    let assembled = runtime
        .build_context(&test_config(), "root-session-no-contract", true, binding)
        .await
        .expect("build context for root session");

    let system_content = assembled.messages[0]["content"]
        .as_str()
        .expect("system prompt should stay string");
    assert!(
        !system_content.contains("[delegate_child_runtime_contract]"),
        "root system prompt should not include the child runtime contract block: {system_content}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn default_runtime_build_context_skips_delegate_runtime_contract_for_empty_child_narrowing() {
    let mut config = test_config();
    let child_session_id = seed_delegate_child_session_with_runtime_narrowing(
        &mut config,
        "empty-child-narrowing",
        crate::tools::runtime_config::ToolRuntimeNarrowing::default(),
    );
    let runtime = DefaultConversationRuntime::default();
    let binding = crate::conversation::ConversationRuntimeBinding::direct();

    let assembled = runtime
        .build_context(&config, &child_session_id, true, binding)
        .await
        .expect("build context for child session");

    let system_content = assembled.messages[0]["content"]
        .as_str()
        .expect("system prompt should stay string");
    assert!(
        !system_content.contains("[delegate_child_runtime_contract]"),
        "empty child narrowing should not inject a contract block: {system_content}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn default_runtime_build_context_uses_effective_private_host_policy_in_delegate_contract() {
    let mut config = test_config();
    let child_session_id = seed_delegate_child_session_with_runtime_narrowing(
        &mut config,
        "effective-private-hosts",
        crate::tools::runtime_config::ToolRuntimeNarrowing {
            web_fetch: crate::tools::runtime_config::WebFetchRuntimeNarrowing {
                allow_private_hosts: Some(true),
                ..crate::tools::runtime_config::WebFetchRuntimeNarrowing::default()
            },
            ..crate::tools::runtime_config::ToolRuntimeNarrowing::default()
        },
    );
    let runtime = DefaultConversationRuntime::default();
    let binding = crate::conversation::ConversationRuntimeBinding::direct();

    let assembled = runtime
        .build_context(&config, &child_session_id, true, binding)
        .await
        .expect("build context for child session");

    let system_content = assembled.messages[0]["content"]
        .as_str()
        .expect("system prompt should stay string");
    assert!(
        system_content.contains("- web.fetch private hosts: denied"),
        "effective child contract should preserve the base private-host denial, got: {system_content}"
    );
    assert!(
        !system_content.contains("- web.fetch private hosts: allowed"),
        "child prompt must not widen private-host policy beyond the effective runtime contract: {system_content}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn default_runtime_build_context_surfaces_fail_closed_allowlist_intersection() {
    let mut config = test_config();
    config.tools.web.allowed_domains = vec!["api.example.com".to_owned()];
    let child_session_id = seed_delegate_child_session_with_runtime_narrowing(
        &mut config,
        "effective-allowlist-intersection",
        crate::tools::runtime_config::ToolRuntimeNarrowing {
            web_fetch: crate::tools::runtime_config::WebFetchRuntimeNarrowing {
                allowed_domains: BTreeSet::from(["docs.example.com".to_owned()]),
                ..crate::tools::runtime_config::WebFetchRuntimeNarrowing::default()
            },
            ..crate::tools::runtime_config::ToolRuntimeNarrowing::default()
        },
    );
    let runtime = DefaultConversationRuntime::default();
    let binding = crate::conversation::ConversationRuntimeBinding::direct();

    let assembled = runtime
        .build_context(&config, &child_session_id, true, binding)
        .await
        .expect("build context for child session");

    let system_content = assembled.messages[0]["content"]
        .as_str()
        .expect("system prompt should stay string");
    assert!(
        system_content
            .contains("- web.fetch allowed domains: none (effective intersection is empty)"),
        "child prompt should expose the fail-closed effective allowlist, got: {system_content}"
    );
    assert!(
        !system_content.contains("- web.fetch allowed domains: docs.example.com"),
        "child prompt must not show requested domains that the effective runtime will reject: {system_content}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn default_runtime_build_context_matches_builtin_summary_projection() {
    let runtime = DefaultConversationRuntime::default();
    let session_id = unique_acp_test_id("default-runtime-context", "summary");
    let sqlite_path = unique_memory_sqlite_path("summary");
    let mut config = test_config();
    config.memory.system = MemorySystemKind::Builtin;
    config.memory.profile = MemoryProfile::WindowPlusSummary;
    config.memory.sliding_window = 2;
    config.memory.sqlite_path = sqlite_path.clone();

    let runtime_config =
        crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    crate::memory::append_turn_direct(&session_id, "user", "turn 1", &runtime_config)
        .expect("append turn 1 should succeed");
    crate::memory::append_turn_direct(&session_id, "assistant", "turn 2", &runtime_config)
        .expect("append turn 2 should succeed");
    crate::memory::append_turn_direct(&session_id, "user", "turn 3", &runtime_config)
        .expect("append turn 3 should succeed");
    crate::memory::append_turn_direct(&session_id, "assistant", "turn 4", &runtime_config)
        .expect("append turn 4 should succeed");

    let assembled = runtime
        .build_context(
            &config,
            &session_id,
            true,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("build context from default runtime");
    let provider_messages = crate::provider::build_messages_for_session(&config, &session_id, true)
        .expect("build provider messages");

    assert_eq!(
        assembled.messages, provider_messages,
        "default runtime should match provider projection for builtin summary hydration"
    );
    assert!(
        assembled.messages.iter().any(|message| {
            message["role"] == "system"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("## Memory Summary"))
        }),
        "expected hydrated summary block in default runtime messages"
    );

    let _ = std::fs::remove_file(sqlite_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn default_runtime_build_context_explicit_builtin_system_preserves_profile_projection() {
    let runtime = DefaultConversationRuntime::default();
    let session_id = unique_acp_test_id("default-runtime-context", "profile");
    let sqlite_path = unique_memory_sqlite_path("profile");
    let mut config = test_config();
    config.memory.system = MemorySystemKind::Builtin;
    config.memory.profile = MemoryProfile::ProfilePlusWindow;
    config.memory.profile_note = Some("Imported ZeroClaw preferences".to_owned());
    config.memory.sliding_window = 2;
    config.memory.sqlite_path = sqlite_path.clone();

    let runtime_config =
        crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    crate::memory::append_turn_direct(&session_id, "assistant", "turn 1", &runtime_config)
        .expect("append turn should succeed");

    let assembled = runtime
        .build_context(
            &config,
            &session_id,
            true,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("build context from default runtime");
    let provider_messages = crate::provider::build_messages_for_session(&config, &session_id, true)
        .expect("build provider messages");

    assert_eq!(
        assembled.messages, provider_messages,
        "explicit builtin memory system should not change prompt projection"
    );
    assert!(
        assembled.messages.iter().any(|message| {
            message["role"] == "system"
                && message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("Imported ZeroClaw preferences"))
        }),
        "expected hydrated profile block in default runtime messages"
    );

    let _ = std::fs::remove_file(sqlite_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn default_runtime_build_context_fail_open_memory_derivation_preserves_recent_window_projection()
 {
    let _faults = crate::memory::ScopedMemoryOrchestratorTestFaults::set(
        crate::memory::MemoryOrchestratorTestFaults {
            derivation_error: Some("simulated derivation failure".to_owned()),
            ..crate::memory::MemoryOrchestratorTestFaults::default()
        },
    );
    let runtime = DefaultConversationRuntime::default();
    let session_id = unique_acp_test_id("default-runtime-context", "fail-open-memory");
    let sqlite_path = unique_memory_sqlite_path("fail-open-memory");
    let mut config = test_config();
    config.memory.system = MemorySystemKind::Builtin;
    config.memory.profile = MemoryProfile::WindowOnly;
    config.memory.sliding_window = 2;
    config.memory.sqlite_path = sqlite_path.clone();

    let runtime_config =
        crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    crate::memory::append_turn_direct(&session_id, "user", "turn 1", &runtime_config)
        .expect("append turn 1 should succeed");
    crate::memory::append_turn_direct(&session_id, "assistant", "turn 2", &runtime_config)
        .expect("append turn 2 should succeed");
    crate::memory::append_turn_direct(&session_id, "user", "turn 3", &runtime_config)
        .expect("append turn 3 should succeed");

    let assembled = runtime
        .build_context(
            &config,
            &session_id,
            true,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("build context should stay available when memory derivation degrades");

    let projected_turns = assembled
        .messages
        .iter()
        .filter_map(|message| {
            let role = message.get("role")?.as_str()?;
            let content = message.get("content")?.as_str()?;
            matches!(role, "user" | "assistant").then_some((role.to_owned(), content.to_owned()))
        })
        .collect::<Vec<_>>();

    assert_eq!(
        projected_turns,
        vec![
            ("assistant".to_owned(), "turn 2".to_owned()),
            ("user".to_owned(), "turn 3".to_owned()),
        ]
    );

    let _ = std::fs::remove_file(sqlite_path);
}

#[test]
fn resolve_context_engine_selection_uses_default_when_unset() {
    let _env_lock = context_engine_env_lock().lock().expect("env lock");
    let _scoped_env = ScopedEnvVar::set(CONTEXT_ENGINE_ENV, "");
    let config = test_config();
    let selection = resolve_context_engine_selection(&config);
    assert_eq!(selection.id, DEFAULT_CONTEXT_ENGINE_ID);
    assert_eq!(selection.source, ContextEngineSelectionSource::Default);
    assert_eq!(selection.source.as_str(), "default");
}

#[test]
fn resolve_context_engine_selection_prefers_env_over_config() {
    let _env_lock = context_engine_env_lock().lock().expect("env lock");
    let _scoped_env = ScopedEnvVar::set(CONTEXT_ENGINE_ENV, "stub-env-priority");
    let mut config = test_config();
    config.conversation.context_engine = Some("stub-config".to_owned());

    let selection = resolve_context_engine_selection(&config);
    assert_eq!(selection.id, "stub-env-priority");
    assert_eq!(selection.source, ContextEngineSelectionSource::Env);
}

#[test]
fn resolve_context_engine_selection_uses_config_when_env_missing() {
    let _env_lock = context_engine_env_lock().lock().expect("env lock");
    let _scoped_env = ScopedEnvVar::set(CONTEXT_ENGINE_ENV, "");
    let mut config = test_config();
    config.conversation.context_engine = Some("legacy".to_owned());

    let selection = resolve_context_engine_selection(&config);
    assert_eq!(selection.id, "legacy");
    assert_eq!(selection.source, ContextEngineSelectionSource::Config);
}

#[test]
fn collect_context_engine_runtime_snapshot_reports_compaction_and_selection() {
    let _env_lock = context_engine_env_lock().lock().expect("env lock");
    let _scoped_env = ScopedEnvVar::set(CONTEXT_ENGINE_ENV, "");
    let mut config = test_config();
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(7);
    config.conversation.compact_fail_open = false;

    let snapshot = collect_context_engine_runtime_snapshot(&config)
        .expect("collect context engine runtime snapshot");
    assert_eq!(snapshot.selected.id, DEFAULT_CONTEXT_ENGINE_ID);
    assert_eq!(
        snapshot.selected.source,
        ContextEngineSelectionSource::Default
    );
    assert_eq!(snapshot.selected_metadata.id, DEFAULT_CONTEXT_ENGINE_ID);
    assert!(
        snapshot
            .available
            .iter()
            .any(|metadata| metadata.id == DEFAULT_CONTEXT_ENGINE_ID)
    );
    assert_eq!(snapshot.compaction.min_messages, Some(7));
    assert_eq!(snapshot.compaction.trigger_estimated_tokens, None);
    assert!(!snapshot.compaction.fail_open);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)] // env var mutation is process-global; keep lock for full test body.
async fn default_runtime_prefers_env_context_engine_over_config() {
    let _env_lock = context_engine_env_lock().lock().expect("env lock");
    register_context_engine("stub-config-env-priority", || Box::new(StubContextEngine))
        .expect("register config context engine");
    register_context_engine("stub-env-priority", || Box::new(StubEnvContextEngine))
        .expect("register env context engine");
    let _scoped_env = ScopedEnvVar::set(CONTEXT_ENGINE_ENV, "stub-env-priority");

    let mut config = test_config();
    config.conversation.context_engine = Some("stub-config-env-priority".to_owned());

    let runtime = DefaultConversationRuntime::from_config_or_env(&config)
        .expect("resolve context engine from env override");
    let binding = ConversationRuntimeBinding::direct();
    let tool_view = runtime
        .tool_view(&config, "session-env-priority", binding)
        .expect("env-selected runtime tool view");
    let messages = runtime
        .build_messages(&config, "session-env-priority", true, &tool_view, binding)
        .await
        .expect("build messages via env-selected context engine");

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["content"], "stub-env-context-engine");
}

#[cfg(feature = "feishu-integration")]
#[test]
fn conversation_runtime_trait_default_tool_view_includes_runtime_discovered_feishu_tools() {
    let runtime = TraitDefaultToolViewRuntime;
    let config = LoongClawConfig {
        feishu: crate::config::FeishuChannelConfig {
            enabled: true,
            app_id: Some("test-feishu-app-id".to_owned()),
            app_secret: Some("test-feishu-app-secret".to_owned()),
            ..crate::config::FeishuChannelConfig::default()
        },
        ..test_config()
    };

    let tool_view = runtime
        .tool_view(
            &config,
            "session-feishu-runtime-tools",
            ConversationRuntimeBinding::direct(),
        )
        .expect("trait default tool view");

    assert!(tool_view.contains("feishu.whoami"));
    assert!(tool_view.contains("feishu.messages.send"));
}

#[tokio::test]
async fn handle_turn_with_runtime_success_with_kernel_runs_lifecycle_hooks() {
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let kernel_ctx = crate::context::bootstrap_kernel_context("test-handle-turn-success", 60)
        .expect("bootstrap kernel context");
    let reply = coordinator
        .handle_turn_with_runtime(
            &test_config(),
            "session-1",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect("handle turn success");

    assert_eq!(reply, "assistant-reply");
    assert_eq!(
        runtime
            .bootstrap_calls
            .lock()
            .expect("bootstrap lock")
            .as_slice(),
        ["session-1"]
    );

    let requested = runtime.requested_messages.lock().expect("requested lock");
    assert_eq!(requested.len(), 2);
    assert_eq!(requested[1]["role"], "user");
    assert_eq!(requested[1]["content"], "hello");

    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let visible_turns = persisted_visible_turns(&persisted);
    assert_eq!(visible_turns.len(), 2);
    assert_eq!(
        visible_turns[0],
        (
            "session-1".to_owned(),
            "user".to_owned(),
            "hello".to_owned()
        )
    );
    assert_eq!(
        visible_turns[1],
        (
            "session-1".to_owned(),
            "assistant".to_owned(),
            "assistant-reply".to_owned(),
        )
    );

    let ingested = runtime
        .ingested_messages
        .lock()
        .expect("ingest lock")
        .clone();
    assert_eq!(ingested.len(), 2);
    assert_eq!(ingested[0].0, "session-1");
    assert_eq!(ingested[0].1["role"], "user");
    assert_eq!(ingested[0].1["content"], "hello");
    assert_eq!(ingested[1].0, "session-1");
    assert_eq!(ingested[1].1["role"], "assistant");
    assert_eq!(ingested[1].1["content"], "assistant-reply");

    let after_turn = runtime
        .after_turn_calls
        .lock()
        .expect("after-turn lock")
        .clone();
    assert_eq!(after_turn.len(), 1);
    assert_eq!(after_turn[0].0, "session-1");
    assert_eq!(after_turn[0].1, "hello");
    assert_eq!(after_turn[0].2, "assistant-reply");
    assert_eq!(after_turn[0].3, 3);

    let compact = runtime.compact_calls.lock().expect("compact lock").clone();
    assert_eq!(compact.len(), 1);
    assert_eq!(compact[0].0, "session-1");
    assert_eq!(compact[0].1, 3);
}

#[tokio::test]
async fn handle_turn_with_runtime_success_without_kernel_skips_lifecycle_hooks() {
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &test_config(),
            "session-1-no-kernel",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("handle turn success without kernel");

    assert_eq!(reply, "assistant-reply");
    assert!(
        runtime
            .bootstrap_calls
            .lock()
            .expect("bootstrap lock")
            .is_empty()
    );
    assert!(
        runtime
            .ingested_messages
            .lock()
            .expect("ingest lock")
            .is_empty()
    );
    assert!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .is_empty()
    );
    assert!(
        runtime
            .compact_calls
            .lock()
            .expect("compact lock")
            .is_empty()
    );

    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let visible_turns = persisted_visible_turns(&persisted);
    assert_eq!(visible_turns.len(), 2);
    assert_eq!(
        visible_turns[0],
        (
            "session-1-no-kernel".to_owned(),
            "user".to_owned(),
            "hello".to_owned()
        )
    );
    assert_eq!(
        visible_turns[1],
        (
            "session-1-no-kernel".to_owned(),
            "assistant".to_owned(),
            "assistant-reply".to_owned(),
        )
    );
}

#[tokio::test]
async fn persist_turn_provider_turns_expose_typed_canonical_records() {
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    coordinator
        .handle_turn_with_runtime(
            &test_config(),
            "session-canonical-provider",
            "hello canonical memory",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("provider turn should succeed");

    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let records = persisted_canonical_records(&persisted_visible_turns(&persisted));

    assert_eq!(records.len(), 2);
    assert_eq!(records[0].scope, crate::memory::MemoryScope::Session);
    assert_eq!(
        records[0].kind,
        crate::memory::CanonicalMemoryKind::UserTurn
    );
    assert_eq!(records[0].role.as_deref(), Some("user"));
    assert_eq!(records[0].content, "hello canonical memory");
    assert_eq!(records[0].session_id, "session-canonical-provider");

    assert_eq!(records[1].scope, crate::memory::MemoryScope::Session);
    assert_eq!(
        records[1].kind,
        crate::memory::CanonicalMemoryKind::AssistantTurn
    );
    assert_eq!(records[1].role.as_deref(), Some("assistant"));
    assert_eq!(records[1].content, "assistant-reply");
}

#[tokio::test]
async fn handle_turn_with_runtime_keeps_provider_path_by_default_when_acp_enabled() {
    let (backend_id, shared) = register_routed_acp_backend("success", false);
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("provider-normal-path".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.default_agent = Some("claude".to_owned());
    config.acp.allowed_agents = vec!["claude".to_owned()];
    config.acp.backend = Some(backend_id.to_owned());
    config.acp.bindings_enabled = true;
    config.acp.dispatch.bootstrap_mcp_servers =
        vec![" Filesystem ".to_owned(), "filesystem".to_owned()];
    config.memory.sqlite_path = unique_acp_sqlite_path("success");

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "telegram:42",
            "hello from channel",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("default handle_turn should stay on provider path");

    assert_eq!(reply, "provider-normal-path");
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 1);
    assert_eq!(shared.lock().expect("ACP shared state").turn_calls, 0);
}

#[tokio::test]
async fn handle_turn_with_runtime_routes_explicit_acp_turns_through_acp() {
    let (backend_id, shared) = register_routed_acp_backend("success-explicit", false);
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("provider-should-not-run".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.default_agent = Some("claude".to_owned());
    config.acp.allowed_agents = vec!["claude".to_owned()];
    config.acp.backend = Some(backend_id.to_owned());
    config.acp.bindings_enabled = true;
    config.acp.dispatch.bootstrap_mcp_servers =
        vec![" Filesystem ".to_owned(), "filesystem".to_owned()];
    config.memory.sqlite_path = unique_acp_sqlite_path("success-explicit");

    let reply = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options(
            &config,
            &ConversationSessionAddress::from_session_id("telegram:42"),
            "hello from channel",
            ProviderErrorMode::Propagate,
            &runtime,
            &AcpConversationTurnOptions {
                routing_intent: AcpRoutingIntent::Explicit,
                ..AcpConversationTurnOptions::default()
            },
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("explicit ACP turn should route through ACP");

    assert_eq!(reply, "acp: hello from channel");
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 0);
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        0
    );
    assert!(
        runtime
            .requested_messages
            .lock()
            .expect("requested messages lock")
            .is_empty(),
        "provider path should not build or request provider messages for explicit ACP turns"
    );

    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    assert_eq!(persisted.len(), 2);
    assert_eq!(persisted[0].0, "telegram:42");
    assert_eq!(persisted[0].1, "user");
    assert_eq!(persisted[1].1, "assistant");
    assert_eq!(persisted[1].2, "acp: hello from channel");
    assert!(
        runtime
            .bootstrap_calls
            .lock()
            .expect("bootstrap lock")
            .is_empty()
    );
    assert!(
        runtime
            .ingested_messages
            .lock()
            .expect("ingest lock")
            .is_empty()
    );
    assert!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .is_empty()
    );
    assert!(
        runtime
            .compact_calls
            .lock()
            .expect("compact lock")
            .is_empty()
    );

    let state = shared.lock().expect("ACP shared state");
    assert_eq!(state.ensure_calls, 1);
    assert_eq!(state.turn_calls, 1);
    let bootstrap = state
        .last_bootstrap
        .clone()
        .expect("ACP bootstrap should be captured");
    assert_eq!(bootstrap.conversation_id.as_deref(), Some("telegram:42"));
    assert_eq!(
        bootstrap
            .binding
            .as_ref()
            .map(|binding| binding.route_session_id.as_str()),
        Some("telegram:42")
    );
    assert_eq!(bootstrap.session_key, "agent:claude:telegram:42");
    assert_eq!(bootstrap.mcp_servers, vec!["filesystem".to_owned()]);
    assert_eq!(
        bootstrap.metadata.get("acp_agent").map(String::as_str),
        Some("claude")
    );
    assert_eq!(
        bootstrap
            .metadata
            .get("loongclaw.acp.activation_origin")
            .map(String::as_str),
        Some("explicit_request")
    );
    let request = state
        .last_request
        .clone()
        .expect("ACP request should be captured");
    assert_eq!(request.session_key, "agent:claude:telegram:42");
    assert_eq!(request.input, "hello from channel");
    assert_eq!(
        request
            .metadata
            .get("loongclaw.acp.routing_origin")
            .map(String::as_str),
        Some("explicit_request")
    );
}

#[tokio::test]
async fn persist_turn_explicit_acp_routing_exposes_typed_canonical_records() {
    let (backend_id, _shared) = register_routed_acp_backend_with_events(
        "typed-explicit",
        false,
        vec![
            json!({
                "type": "text",
                "content": "partial hello"
            }),
            json!({
                "type": "done",
                "stopReason": "completed"
            }),
        ],
    );
    let runtime = FakeRuntime::new(Vec::new(), Ok("provider-should-not-run".to_owned()));
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.backend = Some(backend_id.to_owned());
    config.acp.emit_runtime_events = true;
    config.memory.sqlite_path = unique_acp_sqlite_path("typed-explicit");

    let reply = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options(
            &config,
            &ConversationSessionAddress::from_session_id("telegram:typed-explicit"),
            "hello explicit canonical",
            ProviderErrorMode::Propagate,
            &runtime,
            &AcpConversationTurnOptions {
                routing_intent: AcpRoutingIntent::Explicit,
                ..AcpConversationTurnOptions::default()
            },
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("explicit ACP turn should succeed");

    assert_eq!(reply, "acp: hello explicit canonical");
    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let records = persisted_canonical_records(&persisted);

    assert!(
        records
            .iter()
            .any(|record| record.kind == crate::memory::CanonicalMemoryKind::UserTurn)
    );
    assert!(
        records
            .iter()
            .any(|record| record.kind == crate::memory::CanonicalMemoryKind::AssistantTurn)
    );

    let runtime_event = records
        .iter()
        .find(|record| record.kind == crate::memory::CanonicalMemoryKind::AcpRuntimeEvent)
        .expect("expected ACP runtime event record");
    assert_eq!(runtime_event.scope, crate::memory::MemoryScope::Session);
    assert_eq!(runtime_event.metadata["event"], "acp_turn_event");
    assert_eq!(
        runtime_event.metadata["payload"]["routing_intent"],
        "explicit"
    );

    let final_event = records
        .iter()
        .find(|record| record.kind == crate::memory::CanonicalMemoryKind::AcpFinalEvent)
        .expect("expected ACP final event record");
    assert_eq!(final_event.metadata["event"], "acp_turn_final");
    assert_eq!(
        final_event.metadata["payload"]["routing_intent"],
        "explicit"
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_merges_additional_acp_bootstrap_mcp_servers_from_options() {
    let (backend_id, shared) = register_routed_acp_backend("bootstrap-mcp-options", false);
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("provider-should-not-run".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.default_agent = Some("claude".to_owned());
    config.acp.allowed_agents = vec!["claude".to_owned()];
    config.acp.backend = Some(backend_id.to_owned());
    config.acp.dispatch.bootstrap_mcp_servers = vec![" Filesystem ".to_owned()];
    config.memory.sqlite_path = unique_acp_sqlite_path("bootstrap-mcp-options");
    let extra_servers = vec![
        "search".to_owned(),
        "filesystem".to_owned(),
        " Search ".to_owned(),
    ];

    let reply = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options(
            &config,
            &ConversationSessionAddress::from_session_id("telegram:4242"),
            "hello with extra bootstrap mcp",
            ProviderErrorMode::Propagate,
            &runtime,
            &AcpConversationTurnOptions {
                routing_intent: AcpRoutingIntent::Explicit,
                additional_bootstrap_mcp_servers: Some(extra_servers.as_slice()),
                ..AcpConversationTurnOptions::default()
            },
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("ACP-routed turn with additional bootstrap MCP servers should succeed");

    assert_eq!(reply, "acp: hello with extra bootstrap mcp");
    let state = shared.lock().expect("ACP shared state");
    let bootstrap = state
        .last_bootstrap
        .clone()
        .expect("ACP bootstrap should be captured");
    assert_eq!(
        bootstrap.mcp_servers,
        vec!["filesystem".to_owned(), "search".to_owned()]
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_applies_acp_turn_provenance_metadata() {
    let (backend_id, shared) = register_routed_acp_backend("turn-provenance", false);
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("provider-should-not-run".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.default_agent = Some("claude".to_owned());
    config.acp.allowed_agents = vec!["claude".to_owned()];
    config.acp.backend = Some(backend_id.to_owned());
    config.memory.sqlite_path = unique_acp_sqlite_path("turn-provenance");

    let reply = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options(
            &config,
            &ConversationSessionAddress::from_session_id("telegram:4242"),
            "hello with provenance",
            ProviderErrorMode::Propagate,
            &runtime,
            &AcpConversationTurnOptions {
                routing_intent: AcpRoutingIntent::Explicit,
                provenance: AcpTurnProvenance {
                    trace_id: Some("trace-123"),
                    source_message_id: Some("message-42"),
                    ack_cursor: Some("cursor-9"),
                },
                ..AcpConversationTurnOptions::default()
            },
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("ACP-routed turn with provenance should succeed");

    assert_eq!(reply, "acp: hello with provenance");
    let state = shared.lock().expect("ACP shared state");
    let request = state
        .last_request
        .clone()
        .expect("ACP request should be captured");
    assert_eq!(
        request
            .metadata
            .get(ACP_TURN_METADATA_TRACE_ID)
            .map(String::as_str),
        Some("trace-123")
    );
    assert_eq!(
        request
            .metadata
            .get(ACP_TURN_METADATA_SOURCE_MESSAGE_ID)
            .map(String::as_str),
        Some("message-42")
    );
    assert_eq!(
        request
            .metadata
            .get(ACP_TURN_METADATA_ACK_CURSOR)
            .map(String::as_str),
        Some("cursor-9")
    );
    assert_eq!(
        request
            .metadata
            .get(ACP_TURN_METADATA_ROUTING_INTENT)
            .map(String::as_str),
        Some("explicit")
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_applies_acp_working_directory_from_options() {
    let (backend_id, shared) = register_routed_acp_backend("turn-working-directory", false);
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("provider-should-not-run".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.default_agent = Some("claude".to_owned());
    config.acp.allowed_agents = vec!["claude".to_owned()];
    config.acp.backend = Some(backend_id.to_owned());
    config.memory.sqlite_path = unique_acp_sqlite_path("turn-working-directory");
    let working_directory = PathBuf::from("/workspace/project");

    let reply = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options(
            &config,
            &ConversationSessionAddress::from_session_id("telegram:4242"),
            "hello with working directory",
            ProviderErrorMode::Propagate,
            &runtime,
            &AcpConversationTurnOptions {
                routing_intent: AcpRoutingIntent::Explicit,
                working_directory: Some(working_directory.as_path()),
                ..AcpConversationTurnOptions::default()
            },
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("ACP-routed turn with working directory should succeed");

    assert_eq!(reply, "acp: hello with working directory");
    let state = shared.lock().expect("ACP shared state");
    let bootstrap = state
        .last_bootstrap
        .clone()
        .expect("ACP bootstrap should be captured");
    let request = state
        .last_request
        .clone()
        .expect("ACP request should be captured");
    assert_eq!(
        bootstrap.working_directory.as_deref(),
        Some(working_directory.as_path())
    );
    assert_eq!(
        request.working_directory.as_deref(),
        Some(working_directory.as_path())
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_falls_back_to_dispatch_acp_working_directory() {
    let (backend_id, shared) = register_routed_acp_backend("dispatch-working-directory", false);
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("provider-should-not-run".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.default_agent = Some("claude".to_owned());
    config.acp.allowed_agents = vec!["claude".to_owned()];
    config.acp.backend = Some(backend_id.to_owned());
    config.acp.dispatch.working_directory = Some(" /workspace/dispatch ".to_owned());
    config.memory.sqlite_path = unique_acp_sqlite_path("dispatch-working-directory");

    let reply = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options(
            &config,
            &ConversationSessionAddress::from_session_id("telegram:4343"),
            "hello with dispatch working directory",
            ProviderErrorMode::Propagate,
            &runtime,
            &AcpConversationTurnOptions {
                routing_intent: AcpRoutingIntent::Explicit,
                ..AcpConversationTurnOptions::default()
            },
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("ACP-routed turn should inherit dispatch working directory");

    assert_eq!(reply, "acp: hello with dispatch working directory");
    let state = shared.lock().expect("ACP shared state");
    let bootstrap = state
        .last_bootstrap
        .clone()
        .expect("ACP bootstrap should be captured");
    let request = state
        .last_request
        .clone()
        .expect("ACP request should be captured");
    assert_eq!(
        bootstrap.working_directory.as_deref(),
        Some(std::path::Path::new("/workspace/dispatch"))
    );
    assert_eq!(
        request.working_directory.as_deref(),
        Some(std::path::Path::new("/workspace/dispatch"))
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_uses_provider_path_when_acp_dispatch_is_disabled() {
    let (backend_id, shared) = register_routed_acp_backend("dispatch-disabled", false);
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("provider-path-reply".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.backend = Some(backend_id.to_owned());
    config.acp.dispatch.enabled = false;
    config.memory.sqlite_path = unique_acp_sqlite_path("dispatch-disabled");

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "telegram:424242",
            "hello provider path",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("provider path should remain available when ACP dispatch is disabled");

    assert_eq!(reply, "provider-path-reply");
    assert_eq!(
        shared.lock().expect("ACP shared state").turn_calls,
        0,
        "ACP backend should not receive turns when dispatch is disabled"
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 1);
    assert!(
        runtime
            .bootstrap_calls
            .lock()
            .expect("bootstrap lock")
            .is_empty()
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_explicit_acp_request_bypasses_dispatch_gate() {
    let (backend_id, shared) = register_routed_acp_backend("dispatch-disabled-explicit", false);
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("provider-should-not-run".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.backend = Some(backend_id.to_owned());
    config.acp.dispatch.enabled = false;
    config.memory.sqlite_path = unique_acp_sqlite_path("dispatch-disabled-explicit");

    let reply = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options(
            &config,
            &ConversationSessionAddress::from_session_id("telegram:424242"),
            "hello explicit acp path",
            ProviderErrorMode::Propagate,
            &runtime,
            &AcpConversationTurnOptions {
                routing_intent: AcpRoutingIntent::Explicit,
                ..AcpConversationTurnOptions::default()
            },
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("explicit ACP requests should bypass automatic dispatch gating");

    assert_eq!(reply, "acp: hello explicit acp path");
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 0);
    assert_eq!(shared.lock().expect("ACP shared state").turn_calls, 1);
}

#[tokio::test]
async fn handle_turn_with_runtime_explicit_acp_request_fails_closed_when_acp_is_disabled() {
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("provider-should-not-run".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = false;

    let reply = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options(
            &config,
            &ConversationSessionAddress::from_session_id("telegram:424242"),
            "hello explicit disabled acp path",
            ProviderErrorMode::InlineMessage,
            &runtime,
            &AcpConversationTurnOptions {
                routing_intent: AcpRoutingIntent::Explicit,
                ..AcpConversationTurnOptions::default()
            },
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("inline mode should synthesize a clear ACP-disabled reply");

    assert_eq!(
        reply,
        format_provider_error_reply("ACP is disabled by policy (`acp.enabled=false`)")
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 0);
}

#[tokio::test]
async fn handle_turn_with_runtime_routes_only_agent_prefixed_sessions_when_configured() {
    let (backend_id, shared) = register_routed_acp_backend("prefixed-only", false);
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("provider-fallback".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.backend = Some(backend_id.to_owned());
    config.acp.dispatch.conversation_routing =
        crate::config::AcpConversationRoutingMode::AgentPrefixedOnly;
    config.memory.sqlite_path = unique_acp_sqlite_path("prefixed-only");

    let non_prefixed = coordinator
        .handle_turn_with_runtime(
            &config,
            "telegram:600",
            "should stay on provider path",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("non-prefixed session should stay on provider path");
    assert_eq!(non_prefixed, "provider-fallback");

    let prefixed = coordinator
        .handle_turn_with_runtime(
            &config,
            "agent:codex:review-thread",
            "should route through ACP",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("prefixed session should route through ACP");
    assert_eq!(prefixed, "acp: should route through ACP");

    let state = shared.lock().expect("ACP shared state");
    assert_eq!(state.turn_calls, 1);
    let bootstrap = state
        .last_bootstrap
        .clone()
        .expect("ACP bootstrap should be captured for prefixed session");
    assert_eq!(
        bootstrap
            .metadata
            .get("loongclaw.acp.activation_origin")
            .map(String::as_str),
        Some("automatic_agent_prefixed")
    );
    let request = state
        .last_request
        .clone()
        .expect("ACP request should be captured for prefixed session");
    assert_eq!(request.session_key, "agent:codex:review-thread");
    assert_eq!(
        request
            .metadata
            .get("loongclaw.acp.routing_origin")
            .map(String::as_str),
        Some("automatic_agent_prefixed")
    );
}

#[tokio::test]
async fn persist_turn_automatic_acp_routing_exposes_typed_canonical_records() {
    let (backend_id, _shared) = register_routed_acp_backend_with_events(
        "typed-automatic",
        false,
        vec![
            json!({
                "type": "text",
                "content": "partial hello"
            }),
            json!({
                "type": "done",
                "stopReason": "completed"
            }),
        ],
    );
    let runtime = FakeRuntime::new(Vec::new(), Ok("provider-should-not-run".to_owned()));
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.backend = Some(backend_id.to_owned());
    config.acp.emit_runtime_events = true;
    config.acp.dispatch.conversation_routing =
        crate::config::AcpConversationRoutingMode::AgentPrefixedOnly;
    config.memory.sqlite_path = unique_acp_sqlite_path("typed-automatic");

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "agent:codex:review-thread",
            "hello automatic canonical",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("automatic ACP turn should succeed");

    assert_eq!(reply, "acp: hello automatic canonical");
    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let records = persisted_canonical_records(&persisted);

    assert!(
        records
            .iter()
            .any(|record| record.kind == crate::memory::CanonicalMemoryKind::UserTurn)
    );
    assert!(
        records
            .iter()
            .any(|record| record.kind == crate::memory::CanonicalMemoryKind::AssistantTurn)
    );

    let runtime_event = records
        .iter()
        .find(|record| record.kind == crate::memory::CanonicalMemoryKind::AcpRuntimeEvent)
        .expect("expected ACP runtime event record");
    assert_eq!(runtime_event.metadata["event"], "acp_turn_event");
    assert_eq!(
        runtime_event.metadata["payload"]["routing_intent"],
        "automatic"
    );

    let final_event = records
        .iter()
        .find(|record| record.kind == crate::memory::CanonicalMemoryKind::AcpFinalEvent)
        .expect("expected ACP final event record");
    assert_eq!(final_event.metadata["event"], "acp_turn_final");
    assert_eq!(
        final_event.metadata["payload"]["routing_intent"],
        "automatic"
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_automatic_acp_routing_bypasses_context_engine_lifecycle_hooks() {
    let (backend_id, shared) = register_routed_acp_backend("automatic-lifecycle-bypass", false);
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("provider-should-not-run".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.backend = Some(backend_id.to_owned());
    config.acp.dispatch.conversation_routing =
        crate::config::AcpConversationRoutingMode::AgentPrefixedOnly;
    config.memory.sqlite_path = unique_acp_sqlite_path("automatic-lifecycle-bypass");

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "agent:codex:review-thread",
            "route automatically through ACP",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("automatic ACP turn should succeed");

    assert_eq!(reply, "acp: route automatically through ACP");
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 0);
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        0
    );
    assert!(
        runtime
            .requested_messages
            .lock()
            .expect("requested messages lock")
            .is_empty()
    );
    assert!(
        runtime
            .bootstrap_calls
            .lock()
            .expect("bootstrap lock")
            .is_empty()
    );
    assert!(
        runtime
            .ingested_messages
            .lock()
            .expect("ingest lock")
            .is_empty()
    );
    assert!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .is_empty()
    );
    assert!(
        runtime
            .compact_calls
            .lock()
            .expect("compact lock")
            .is_empty()
    );

    let state = shared.lock().expect("ACP shared state");
    assert_eq!(state.ensure_calls, 1);
    assert_eq!(state.turn_calls, 1);
}

#[tokio::test]
async fn handle_turn_with_runtime_routes_only_allowed_channels_into_acp() {
    let (backend_id, shared) = register_routed_acp_backend("channel-allowlist", false);
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("provider-feishu-reply".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.backend = Some(backend_id.to_owned());
    config.acp.dispatch.conversation_routing = crate::config::AcpConversationRoutingMode::All;
    config.acp.dispatch.allowed_channels = vec!["telegram".to_owned()];
    config.memory.sqlite_path = unique_acp_sqlite_path("channel-allowlist");

    let telegram = coordinator
        .handle_turn_with_runtime(
            &config,
            "telegram:100",
            "hello telegram",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("telegram session should route through ACP");
    assert_eq!(telegram, "acp: hello telegram");

    {
        let state = shared.lock().expect("ACP shared state");
        let bootstrap = state
            .last_bootstrap
            .clone()
            .expect("ACP bootstrap should be captured for allowlisted channel session");
        assert_eq!(
            bootstrap
                .metadata
                .get("loongclaw.acp.activation_origin")
                .map(String::as_str),
            Some("automatic_dispatch")
        );
        let request = state
            .last_request
            .clone()
            .expect("ACP request should be captured for allowlisted channel session");
        assert_eq!(
            request
                .metadata
                .get("loongclaw.acp.routing_origin")
                .map(String::as_str),
            Some("automatic_dispatch")
        );
    }

    let feishu = coordinator
        .handle_turn_with_runtime(
            &config,
            "feishu:oc_123",
            "hello feishu",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("feishu session should stay on provider path");
    assert_eq!(feishu, "provider-feishu-reply");

    let state = shared.lock().expect("ACP shared state");
    assert_eq!(state.turn_calls, 1);
    let request = state
        .last_request
        .clone()
        .expect("ACP request should exist for telegram session");
    assert_eq!(request.session_key, "agent:codex:telegram:100");
}

#[tokio::test]
async fn handle_turn_with_runtime_and_address_routes_structured_channel_scope_into_acp() {
    let (backend_id, shared) = register_routed_acp_backend("structured-channel-address", false);
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("provider-reply".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.backend = Some(backend_id.to_owned());
    config.acp.dispatch.conversation_routing = crate::config::AcpConversationRoutingMode::All;
    config.acp.dispatch.allowed_channels = vec!["telegram".to_owned()];
    config.memory.sqlite_path = unique_acp_sqlite_path("structured-channel-address");
    let address = ConversationSessionAddress::from_session_id("opaque-session")
        .with_channel_scope("telegram", "100");

    let reply = coordinator
        .handle_turn_with_runtime_and_address(
            &config,
            &address,
            "hello structured route",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("structured channel address should route through ACP");

    assert_eq!(reply, "acp: hello structured route");

    let state = shared.lock().expect("ACP shared state");
    assert_eq!(state.turn_calls, 1);
    let request = state
        .last_request
        .clone()
        .expect("ACP request should be captured");
    let bootstrap = state
        .last_bootstrap
        .clone()
        .expect("ACP bootstrap should be captured");
    assert_eq!(request.session_key, "agent:codex:opaque-session");
    assert_eq!(
        bootstrap
            .binding
            .as_ref()
            .map(|binding| binding.route_session_id.as_str()),
        Some("telegram:100")
    );
    assert_eq!(
        bootstrap
            .binding
            .as_ref()
            .and_then(|binding| binding.channel_id.as_deref()),
        Some("telegram")
    );
    assert_eq!(
        request.metadata.get("channel").map(String::as_str),
        Some("telegram")
    );
    assert_eq!(
        request
            .metadata
            .get("channel_conversation_id")
            .map(String::as_str),
        Some("100")
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_and_address_enforces_account_and_thread_dispatch_scope() {
    let (backend_id, shared) =
        register_routed_acp_backend("structured-account-thread-scope", false);
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("provider-reply".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.backend = Some(backend_id.to_owned());
    config.acp.dispatch.conversation_routing = crate::config::AcpConversationRoutingMode::All;
    config.acp.dispatch.allowed_channels = vec!["feishu".to_owned()];
    config.acp.dispatch.allowed_account_ids = vec!["lark-prod".to_owned()];
    config.acp.dispatch.thread_routing = crate::config::AcpDispatchThreadRoutingMode::ThreadOnly;
    config.memory.sqlite_path = unique_acp_sqlite_path("structured-account-thread-scope");

    let allowed = ConversationSessionAddress::from_session_id("opaque-session")
        .with_channel_scope("feishu", "oc_123")
        .with_account_id("lark-prod")
        .with_thread_id("om_thread_1");
    let blocked = ConversationSessionAddress::from_session_id("opaque-session-root")
        .with_channel_scope("feishu", "oc_123")
        .with_account_id("lark-prod");

    let allowed_reply = coordinator
        .handle_turn_with_runtime_and_address(
            &config,
            &allowed,
            "hello allowed",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("thread-bound allowed address should route through ACP");
    assert_eq!(allowed_reply, "acp: hello allowed");

    let blocked_reply = coordinator
        .handle_turn_with_runtime_and_address(
            &config,
            &blocked,
            "hello blocked",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("root conversation should stay on provider path");
    assert_eq!(blocked_reply, "provider-reply");

    let state = shared.lock().expect("ACP shared state");
    assert_eq!(state.turn_calls, 1);
    let bootstrap = state
        .last_bootstrap
        .clone()
        .expect("ACP bootstrap should be captured");
    assert_eq!(
        bootstrap
            .binding
            .as_ref()
            .map(|binding| binding.route_session_id.as_str()),
        Some("feishu:lark-prod:oc_123:om_thread_1")
    );
    assert_eq!(
        bootstrap
            .binding
            .as_ref()
            .and_then(|binding| binding.account_id.as_deref()),
        Some("lark-prod")
    );
    assert_eq!(
        bootstrap
            .binding
            .as_ref()
            .and_then(|binding| binding.thread_id.as_deref()),
        Some("om_thread_1")
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_formats_acp_errors_inline_when_requested() {
    let (backend_id, shared) = register_routed_acp_backend("inline-error", true);
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("provider-should-not-run".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.backend = Some(backend_id.to_owned());
    config.memory.sqlite_path = unique_acp_sqlite_path("inline-error");

    let reply = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options(
            &config,
            &ConversationSessionAddress::from_session_id("feishu:oc_123"),
            "hello from feishu",
            ProviderErrorMode::InlineMessage,
            &runtime,
            &AcpConversationTurnOptions {
                routing_intent: AcpRoutingIntent::Explicit,
                ..AcpConversationTurnOptions::default()
            },
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("ACP inline error mode should synthesize a reply");

    assert_eq!(
        reply,
        format_provider_error_reply("synthetic ACP routing failure")
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 0);
    assert_eq!(
        shared.lock().expect("ACP shared state").turn_calls,
        1,
        "ACP backend should have received the routed turn"
    );
    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    assert_eq!(persisted.len(), 2);
    assert_eq!(persisted[0].0, "feishu:oc_123");
    assert_eq!(
        persisted[1].2,
        format_provider_error_reply("synthetic ACP routing failure")
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_reuses_shared_acp_session_between_turns() {
    let (backend_id, shared) = register_routed_acp_backend("reuse", false);
    let runtime = FakeRuntime::new(Vec::new(), Ok("provider-should-not-run".to_owned()));
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.backend = Some(backend_id.to_owned());
    config.memory.sqlite_path = unique_acp_sqlite_path("reuse");

    let first = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options(
            &config,
            &ConversationSessionAddress::from_session_id("telegram:4242"),
            "first",
            ProviderErrorMode::Propagate,
            &runtime,
            &AcpConversationTurnOptions {
                routing_intent: AcpRoutingIntent::Explicit,
                ..AcpConversationTurnOptions::default()
            },
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("first ACP-routed turn");
    let second = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options(
            &config,
            &ConversationSessionAddress::from_session_id("telegram:4242"),
            "second",
            ProviderErrorMode::Propagate,
            &runtime,
            &AcpConversationTurnOptions {
                routing_intent: AcpRoutingIntent::Explicit,
                ..AcpConversationTurnOptions::default()
            },
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("second ACP-routed turn");

    assert_eq!(first, "acp: first");
    assert_eq!(second, "acp: second");
    let state = shared.lock().expect("ACP shared state");
    assert_eq!(
        state.ensure_calls, 1,
        "ACP session should be reused through the shared control-plane manager"
    );
    assert_eq!(state.turn_calls, 2);
}

#[tokio::test]
async fn handle_turn_with_runtime_persists_acp_runtime_events_when_enabled() {
    let (backend_id, _shared) = register_routed_acp_backend_with_events(
        "runtime-events",
        false,
        vec![
            json!({
                "type": "text",
                "content": "partial hello"
            }),
            json!({
                "type": "done",
                "stopReason": "completed"
            }),
        ],
    );
    let runtime = FakeRuntime::new(Vec::new(), Ok("provider-should-not-run".to_owned()));
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.backend = Some(backend_id.to_owned());
    config.acp.emit_runtime_events = true;
    config.memory.sqlite_path = unique_acp_sqlite_path("runtime-events");

    let reply = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options(
            &config,
            &ConversationSessionAddress::from_session_id("telegram:777"),
            "hello runtime events",
            ProviderErrorMode::Propagate,
            &runtime,
            &AcpConversationTurnOptions {
                routing_intent: AcpRoutingIntent::Explicit,
                ..AcpConversationTurnOptions::default()
            },
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("ACP-routed turn with runtime events should succeed");

    assert_eq!(reply, "acp: hello runtime events");
    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let event_records = persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            parsed.get("event")?.as_str().map(ToOwned::to_owned)
        })
        .collect::<Vec<_>>();
    assert!(
        event_records.iter().any(|event| event == "acp_turn_event"),
        "expected persisted ACP turn event records, got: {event_records:?}"
    );
    assert!(
        event_records.iter().any(|event| event == "acp_turn_final"),
        "expected persisted ACP turn final record, got: {event_records:?}"
    );
    let agent_ids = persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            parsed
                .get("payload")?
                .get("agent_id")?
                .as_str()
                .map(ToOwned::to_owned)
        })
        .collect::<Vec<_>>();
    assert!(
        agent_ids.iter().any(|agent| agent == "codex"),
        "expected persisted ACP runtime records to expose explicit agent_id, got: {agent_ids:?}"
    );
    let routing_intents = persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            parsed
                .get("payload")?
                .get("routing_intent")?
                .as_str()
                .map(ToOwned::to_owned)
        })
        .collect::<Vec<_>>();
    assert!(
        routing_intents.iter().any(|intent| intent == "explicit"),
        "expected persisted ACP runtime records to keep routing_intent, got: {routing_intents:?}"
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_streams_acp_runtime_events_to_external_sink_without_persisting() {
    let (backend_id, _shared) = register_routed_acp_backend_with_events(
        "external-runtime-events",
        false,
        vec![
            json!({
                "type": "text",
                "content": "partial hello"
            }),
            json!({
                "type": "done",
                "stopReason": "completed"
            }),
        ],
    );
    let runtime = FakeRuntime::new(Vec::new(), Ok("provider-should-not-run".to_owned()));
    let coordinator = ConversationTurnCoordinator::new();
    let sink = RecordingAcpEventSink::default();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.backend = Some(backend_id.to_owned());
    config.acp.emit_runtime_events = false;
    config.memory.sqlite_path = unique_acp_sqlite_path("external-runtime-events");

    let acp_options = AcpConversationTurnOptions::from_event_sink(Some(&sink));
    let reply = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options(
            &config,
            &ConversationSessionAddress::from_session_id("telegram:778"),
            "hello external runtime events",
            ProviderErrorMode::Propagate,
            &runtime,
            &acp_options,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("ACP-routed turn with external event sink should succeed");

    assert_eq!(reply, "acp: hello external runtime events");
    assert_eq!(
        sink.snapshot(),
        vec![
            json!({
                "type": "text",
                "content": "partial hello"
            }),
            json!({
                "type": "done",
                "stopReason": "completed"
            }),
        ]
    );
    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    assert_eq!(
        persisted.len(),
        2,
        "only user/assistant turns should persist"
    );
    assert!(persisted.iter().all(|(_, role, content)| {
        *role != "assistant"
            || serde_json::from_str::<Value>(content)
                .ok()
                .and_then(|value| value.get("type").and_then(Value::as_str).map(str::to_owned))
                .as_deref()
                != Some("conversation_event")
    }));
}

#[tokio::test]
async fn handle_turn_with_runtime_streams_and_persists_acp_runtime_events_when_both_enabled() {
    let (backend_id, _shared) = register_routed_acp_backend_with_events(
        "external-and-persisted-runtime-events",
        false,
        vec![
            json!({
                "type": "text",
                "content": "partial hello"
            }),
            json!({
                "type": "done",
                "stopReason": "completed"
            }),
        ],
    );
    let runtime = FakeRuntime::new(Vec::new(), Ok("provider-should-not-run".to_owned()));
    let coordinator = ConversationTurnCoordinator::new();
    let sink = RecordingAcpEventSink::default();
    let mut config = test_config();
    config.acp.enabled = true;
    config.acp.backend = Some(backend_id.to_owned());
    config.acp.emit_runtime_events = true;
    config.memory.sqlite_path = unique_acp_sqlite_path("external-and-persisted-runtime-events");

    let acp_options = AcpConversationTurnOptions::from_event_sink(Some(&sink));
    let reply = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options(
            &config,
            &ConversationSessionAddress::from_session_id("telegram:779"),
            "hello external and persisted runtime events",
            ProviderErrorMode::Propagate,
            &runtime,
            &acp_options,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("ACP-routed turn with external sink and persistence should succeed");

    assert_eq!(reply, "acp: hello external and persisted runtime events");
    assert_eq!(
        sink.snapshot(),
        vec![
            json!({
                "type": "text",
                "content": "partial hello"
            }),
            json!({
                "type": "done",
                "stopReason": "completed"
            }),
        ]
    );
    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let event_records = persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            parsed.get("event")?.as_str().map(ToOwned::to_owned)
        })
        .collect::<Vec<_>>();
    assert!(event_records.iter().any(|event| event == "acp_turn_event"));
    assert!(event_records.iter().any(|event| event == "acp_turn_final"));
}

#[tokio::test]
async fn handle_turn_with_runtime_skips_compaction_when_disabled() {
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    );
    let mut config = test_config();
    config.conversation.compact_enabled = false;

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-no-compact",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("handle turn success");

    assert_eq!(reply, "assistant-reply");
    assert!(
        runtime
            .compact_calls
            .lock()
            .expect("compact lock")
            .is_empty()
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_skips_compaction_below_min_messages() {
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    );
    let mut config = test_config();
    config.conversation.compact_min_messages = Some(10);

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-no-compact-threshold",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("handle turn success");

    assert_eq!(reply, "assistant-reply");
    assert!(
        runtime
            .compact_calls
            .lock()
            .expect("compact lock")
            .is_empty()
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_skips_compaction_below_token_threshold() {
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    );
    let mut config = test_config();
    config.conversation.compact_min_messages = None;
    config.conversation.compact_trigger_estimated_tokens = Some(100_000);

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-no-compact-token-threshold",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("handle turn success");

    assert_eq!(reply, "assistant-reply");
    assert!(
        runtime
            .compact_calls
            .lock()
            .expect("compact lock")
            .is_empty()
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_compacts_when_token_threshold_reached() {
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    );
    let mut config = test_config();
    config.conversation.compact_min_messages = Some(999);
    config.conversation.compact_trigger_estimated_tokens = Some(1);

    let coordinator = ConversationTurnCoordinator::new();
    let kernel_ctx = test_kernel_context("test-compaction-token-threshold");
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-compact-token-threshold",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect("handle turn success");

    assert_eq!(reply, "assistant-reply");
    let compact = runtime.compact_calls.lock().expect("compact lock").clone();
    assert_eq!(compact.len(), 1);
    assert_eq!(compact[0].0, "session-compact-token-threshold");
}

#[tokio::test]
async fn handle_turn_with_runtime_compaction_error_is_ignored_when_fail_open() {
    let mut runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    );
    runtime.compact_result = Err("compact failure".to_owned());
    let mut config = test_config();
    config.conversation.compact_fail_open = true;

    let coordinator = ConversationTurnCoordinator::new();
    let kernel_ctx = test_kernel_context("test-compaction-fail-open");
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-compact-fail-open",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect("fail-open mode should keep turn successful");

    assert_eq!(reply, "assistant-reply");
    let compact = runtime.compact_calls.lock().expect("compact lock").clone();
    assert_eq!(compact.len(), 1);
}

#[tokio::test]
async fn handle_turn_with_runtime_compaction_error_propagates_when_fail_closed() {
    let mut runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    );
    runtime.compact_result = Err("compact failure".to_owned());
    let mut config = test_config();
    config.conversation.compact_fail_open = false;

    let coordinator = ConversationTurnCoordinator::new();
    let kernel_ctx = test_kernel_context("test-compaction-fail-closed");
    let error = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-compact-fail-closed",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect_err("fail-closed mode should propagate compaction error");

    assert!(
        error.contains("compact failure"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_persists_turn_checkpoint_events_for_successful_provider_turn() {
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    );
    let mut config = test_config();
    config.conversation.compact_enabled = false;

    let coordinator = ConversationTurnCoordinator::new();
    let kernel_ctx = test_kernel_context("test-turn-checkpoint-success");
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-turn-checkpoint-success",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect("success path should persist checkpoint events");

    assert_eq!(reply, "assistant-reply");
    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert_eq!(payloads.len(), 2, "expected two checkpoint events");

    assert_eq!(payloads[0]["schema_version"], 1);
    assert_eq!(payloads[0]["stage"], "post_persist");
    assert_eq!(payloads[0]["checkpoint"]["request"]["kind"], "continue");
    assert_eq!(
        payloads[0]["checkpoint"]["identity"],
        test_turn_checkpoint_identity("hello", "assistant-reply")
    );
    assert_eq!(
        payloads[0]["checkpoint"]["preparation"]["context_fingerprint_sha256"],
        test_turn_preparation_context_fingerprint(&[
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hello"}),
        ])
    );
    assert_eq!(payloads[0]["checkpoint"]["lane"]["lane"], "fast");
    assert_eq!(
        payloads[0]["checkpoint"]["lane"]["result_kind"],
        "final_text"
    );
    assert_eq!(payloads[0]["checkpoint"]["reply"]["decision"], "direct");
    assert_eq!(
        payloads[0]["checkpoint"]["finalization"]["persistence_mode"],
        "success"
    );
    assert_eq!(
        payloads[0]["finalization_progress"]["after_turn"],
        "pending"
    );
    assert_eq!(
        payloads[0]["finalization_progress"]["compaction"],
        "pending"
    );

    assert_eq!(payloads[1]["schema_version"], 1);
    assert_eq!(payloads[1]["stage"], "finalized");
    assert_eq!(
        payloads[1]["finalization_progress"]["after_turn"],
        "completed"
    );
    assert_eq!(
        payloads[1]["finalization_progress"]["compaction"],
        "skipped"
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_persists_turn_checkpoint_events_for_inline_provider_error() {
    let runtime = FakeRuntime::new(vec![], Err("timeout".to_owned()));
    let mut config = test_config();
    config.conversation.compact_enabled = false;

    let coordinator = ConversationTurnCoordinator::new();
    let kernel_ctx = test_kernel_context("test-turn-checkpoint-inline-error");
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-turn-checkpoint-inline-error",
            "hello",
            ProviderErrorMode::InlineMessage,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect("inline provider error should persist checkpoint events");

    assert_eq!(reply, "[provider_error] timeout");
    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert_eq!(payloads.len(), 2, "expected two checkpoint events");

    assert_eq!(payloads[0]["stage"], "post_persist");
    assert_eq!(
        payloads[0]["checkpoint"]["request"]["kind"],
        "finalize_inline_provider_error"
    );
    assert_eq!(
        payloads[0]["checkpoint"]["identity"],
        test_turn_checkpoint_identity("hello", "[provider_error] timeout")
    );
    assert!(payloads[0]["checkpoint"]["lane"].is_null());
    assert!(payloads[0]["checkpoint"]["reply"].is_null());
    assert_eq!(
        payloads[0]["checkpoint"]["finalization"]["persistence_mode"],
        "inline_provider_error"
    );

    assert_eq!(payloads[1]["stage"], "finalized");
    assert_eq!(
        payloads[1]["finalization_progress"]["after_turn"],
        "completed"
    );
    assert_eq!(
        payloads[1]["finalization_progress"]["compaction"],
        "skipped"
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_persists_turn_checkpoint_event_for_propagated_provider_error() {
    let runtime = FakeRuntime::new(vec![], Err("timeout".to_owned()));
    let mut config = test_config();
    config.conversation.compact_enabled = false;

    let coordinator = ConversationTurnCoordinator::new();
    let error = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-turn-checkpoint-propagated-error",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect_err("propagated provider error should still persist checkpoint event");

    assert_eq!(error, "timeout");
    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert_eq!(payloads.len(), 1, "expected one finalized checkpoint event");

    assert_eq!(payloads[0]["stage"], "finalized");
    assert_eq!(payloads[0]["checkpoint"]["request"]["kind"], "return_error");
    assert!(payloads[0]["checkpoint"]["identity"].is_null());
    assert!(payloads[0]["checkpoint"]["lane"].is_null());
    assert!(payloads[0]["checkpoint"]["reply"].is_null());
    assert_eq!(
        payloads[0]["checkpoint"]["finalization"]["kind"],
        "return_error"
    );
    assert_eq!(
        payloads[0]["finalization_progress"]["after_turn"],
        "skipped"
    );
    assert_eq!(
        payloads[0]["finalization_progress"]["compaction"],
        "skipped"
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_persists_failed_turn_checkpoint_when_compaction_fails_closed() {
    let mut runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    );
    runtime.compact_result = Err("compact failure".to_owned());
    let mut config = test_config();
    config.conversation.compact_fail_open = false;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);

    let coordinator = ConversationTurnCoordinator::new();
    let kernel_ctx = test_kernel_context("test-turn-checkpoint-compaction-failure");
    let error = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-turn-checkpoint-compaction-failure",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect_err("compaction failure should still persist failed checkpoint event");

    assert!(
        error.contains("compact failure"),
        "unexpected error: {error}"
    );
    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert_eq!(
        payloads.len(),
        2,
        "expected pre-failure and failure checkpoints"
    );

    assert_eq!(payloads[0]["stage"], "post_persist");
    assert_eq!(payloads[1]["stage"], "finalization_failed");
    assert_eq!(
        payloads[1]["finalization_progress"]["after_turn"],
        "completed"
    );
    assert_eq!(payloads[1]["finalization_progress"]["compaction"], "failed");
    assert_eq!(payloads[1]["failure"]["step"], "compaction");
    assert_eq!(payloads[1]["failure"]["error"], "compact failure");
}

#[tokio::test]
async fn handle_turn_with_runtime_propagates_error_without_persisting_reply_turns() {
    let runtime = FakeRuntime::new(vec![], Err("timeout".to_owned()));
    let coordinator = ConversationTurnCoordinator::new();
    let error = coordinator
        .handle_turn_with_runtime(
            &test_config(),
            "session-2",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect_err("propagate mode should return error");

    assert!(error.contains("timeout"));
    assert!(
        runtime
            .bootstrap_calls
            .lock()
            .expect("bootstrap lock")
            .is_empty()
    );
    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0]["stage"], "finalized");
    assert_eq!(payloads[0]["checkpoint"]["request"]["kind"], "return_error");
    assert!(
        runtime
            .ingested_messages
            .lock()
            .expect("ingest lock")
            .is_empty()
    );
    assert!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .is_empty()
    );
    assert!(
        runtime
            .compact_calls
            .lock()
            .expect("compact lock")
            .is_empty()
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_inline_mode_returns_synthetic_reply_and_persists() {
    let runtime = FakeRuntime::new(vec![], Err("timeout".to_owned()));
    let coordinator = ConversationTurnCoordinator::new();
    let output = coordinator
        .handle_turn_with_runtime(
            &test_config(),
            "session-3",
            "hello",
            ProviderErrorMode::InlineMessage,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("inline mode should return synthetic reply");

    assert_eq!(output, "[provider_error] timeout");
    assert!(
        runtime
            .bootstrap_calls
            .lock()
            .expect("bootstrap lock")
            .is_empty()
    );

    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let visible_turns = persisted_visible_turns(&persisted);
    assert_eq!(visible_turns.len(), 2);
    assert_eq!(
        visible_turns[0],
        (
            "session-3".to_owned(),
            "user".to_owned(),
            "hello".to_owned()
        )
    );
    assert_eq!(
        visible_turns[1],
        (
            "session-3".to_owned(),
            "assistant".to_owned(),
            "[provider_error] timeout".to_owned(),
        )
    );

    let ingested = runtime
        .ingested_messages
        .lock()
        .expect("ingest lock")
        .clone();
    assert!(ingested.is_empty());

    let after_turn = runtime
        .after_turn_calls
        .lock()
        .expect("after-turn lock")
        .clone();
    assert!(after_turn.is_empty());

    let compact = runtime.compact_calls.lock().expect("compact lock").clone();
    assert!(compact.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_tool_turn_uses_natural_language_completion_by_default() {
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(
        harness.temp_dir.join("note.md"),
        "hello from coordinator test",
    )
    .expect("seed test note");

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Reading the file now.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!({"path": "note.md"}),
                "session-tool",
                "turn-tool",
                "call-tool",
            )],
            raw_meta: Value::Null,
        }),
        Ok("Summary: the note says hello from coordinator test.".to_owned()),
    );

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &test_config(),
            "session-tool",
            "read and summarize note.md",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
        )
        .await
        .expect("tool turn should succeed");

    assert_eq!(reply, "Summary: the note says hello from coordinator test.");
    assert!(
        !reply.contains("[ok]"),
        "default reply should not contain raw tool marker, got: {reply}"
    );
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        1
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 1);

    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let visible_turns = persisted_visible_turns(&persisted);
    assert_eq!(visible_turns.len(), 2);
    assert_eq!(visible_turns[0].1, "user");
    assert_eq!(visible_turns[1].1, "assistant");
    assert_eq!(visible_turns[1].2, reply);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_tool_search_requests_a_followup_provider_turn() {
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(
        harness.temp_dir.join("note.md"),
        "hello from coordinator search followup test",
    )
    .expect("seed test note");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Let me search for the right tool first.".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "tool.search",
                    json!({"query": "read note.md", "limit": 3}),
                    "session-tool-search",
                    "turn-tool-search",
                    "call-tool-search",
                )],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Now I'll read the file.".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "file.read",
                    json!({"path": "note.md"}),
                    "session-tool-search",
                    "turn-tool-search",
                    "call-tool-invoke",
                )],
                raw_meta: Value::Null,
            }),
        ],
        vec![Ok(
            "Summary: the note says hello from coordinator search followup test.".to_owned(),
        )],
    );

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &test_config(),
            "session-tool-search",
            "search for the right tool, then read and summarize note.md",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::from_optional_kernel_context(Some(&harness.kernel_ctx)),
        )
        .await
        .expect("tool search turn should succeed");

    assert_eq!(
        reply,
        "Summary: the note says hello from coordinator search followup test."
    );
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        1
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 2);

    let requested_turn_messages = runtime
        .turn_requested_messages
        .lock()
        .expect("turn request lock")
        .clone();
    assert_eq!(requested_turn_messages.len(), 2);
    assert!(
        requested_turn_messages[1].iter().any(|message| {
            message.get("role").and_then(Value::as_str) == Some("assistant")
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|content| content.starts_with("[tool_result]\n"))
        }),
        "second provider turn should receive tool-search followup context: {requested_turn_messages:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_tool_search_raw_request_still_uses_followup_provider_turn() {
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(
        harness.temp_dir.join("note.md"),
        "hello from coordinator raw search followup test",
    )
    .expect("seed test note");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Let me search for the right tool first.".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "tool.search",
                    json!({"query": "read note.md", "limit": 3}),
                    "session-tool-search-raw",
                    "turn-tool-search-raw",
                    "call-tool-search-raw",
                )],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Now I'll read the file.".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "file.read",
                    json!({"path": "note.md"}),
                    "session-tool-search-raw",
                    "turn-tool-search-raw",
                    "call-tool-invoke-raw",
                )],
                raw_meta: Value::Null,
            }),
        ],
        vec![Ok("this must not be used".to_owned())],
    );

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &test_config(),
            "session-tool-search-raw",
            "search for the right tool, then read note.md and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::from_optional_kernel_context(Some(&harness.kernel_ctx)),
        )
        .await
        .expect("tool search raw-output turn should succeed");

    assert!(
        reply.contains("[ok]"),
        "raw-request mode should return the invoked tool output, got: {reply}"
    );
    assert!(
        reply.contains("hello from coordinator raw search followup test"),
        "expected the second-round tool output, got: {reply}"
    );
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        0
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 2);

    let requested_turn_messages = runtime
        .turn_requested_messages
        .lock()
        .expect("turn request lock")
        .clone();
    assert_eq!(requested_turn_messages.len(), 2);
    assert!(
        requested_turn_messages[1].iter().any(|message| {
            message.get("role").and_then(Value::as_str) == Some("assistant")
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|content| content.starts_with("[tool_result]\n"))
        }),
        "second provider turn should still receive tool-search followup context in raw mode: {requested_turn_messages:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_provider_shape_tool_search_followup_openai_chat_completions() {
    let (reply, runtime) = run_provider_shape_tool_search_followup(
        "session-provider-shape-openai",
        "search for the right tool, then read and summarize note.md",
        "hello from openai provider-shape discovery followup test",
        json!({
            "choices": [{
                "message": {
                    "content": "Let me search for the right tool first.",
                    "tool_calls": [{
                        "id": "call-openai-search",
                        "type": "function",
                        "function": {
                            "name": "tool_search",
                            "arguments": "{\"query\":\"read note.md\",\"limit\":3}"
                        }
                    }]
                }
            }]
        }),
        json!({
            "choices": [{
                "message": {
                    "content": "Now I'll read the file.",
                    "tool_calls": [{
                        "id": "call-openai-read",
                        "type": "function",
                        "function": {
                            "name": "file_read",
                            "arguments": "{\"path\":\"note.md\"}"
                        }
                    }]
                }
            }]
        }),
        Ok(
            "Summary: the note says hello from openai provider-shape discovery followup test."
                .to_owned(),
        ),
    )
    .await;

    assert_eq!(
        reply,
        "Summary: the note says hello from openai provider-shape discovery followup test."
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 2);
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        1
    );

    let requested_turn_messages = runtime
        .turn_requested_messages
        .lock()
        .expect("turn request lock")
        .clone();
    assert_eq!(requested_turn_messages.len(), 2);
    assert!(
        requested_turn_messages[1].iter().any(|message| {
            message.get("role").and_then(Value::as_str) == Some("assistant")
                && message
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|content| content.starts_with("[tool_result]\n"))
        }),
        "second provider turn should receive tool-search followup context: {requested_turn_messages:?}"
    );

    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    assert_discovery_first_followup_summary(&persisted, false, "file.read");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_provider_shape_tool_search_followup_responses() {
    let (reply, runtime) = run_provider_shape_tool_search_followup(
        "session-provider-shape-responses",
        "search for the right tool, then read and summarize note.md",
        "hello from responses provider-shape discovery followup test",
        json!({
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {"type": "output_text", "text": "Let me search for the right tool first."}
                    ]
                },
                {
                    "type": "function_call",
                    "name": "tool_search",
                    "arguments": "{\"query\":\"read note.md\",\"limit\":3}",
                    "call_id": "call-responses-search"
                }
            ]
        }),
        json!({
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {"type": "output_text", "text": "Now I'll read the file."}
                    ]
                },
                {
                    "type": "function_call",
                    "name": "file_read",
                    "arguments": "{\"path\":\"note.md\"}",
                    "call_id": "call-responses-read"
                }
            ]
        }),
        Ok(
            "Summary: the note says hello from responses provider-shape discovery followup test."
                .to_owned(),
        ),
    )
    .await;

    assert_eq!(
        reply,
        "Summary: the note says hello from responses provider-shape discovery followup test."
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 2);

    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    assert_discovery_first_followup_summary(&persisted, false, "file.read");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_provider_shape_tool_search_followup_anthropic() {
    let (reply, runtime) = run_provider_shape_tool_search_followup(
        "session-provider-shape-anthropic",
        "search for the right tool, then read and summarize note.md",
        "hello from anthropic provider-shape discovery followup test",
        json!({
            "content": [
                {
                    "type": "text",
                    "text": "Let me search for the right tool first."
                },
                {
                    "type": "tool_use",
                    "id": "toolu-search",
                    "name": "tool_search",
                    "input": {
                        "query": "read note.md",
                        "limit": 3
                    }
                }
            ]
        }),
        json!({
            "content": [
                {
                    "type": "text",
                    "text": "Now I'll read the file."
                },
                {
                    "type": "tool_use",
                    "id": "toolu-read",
                    "name": "file_read",
                    "input": {
                        "path": "note.md"
                    }
                }
            ]
        }),
        Ok(
            "Summary: the note says hello from anthropic provider-shape discovery followup test."
                .to_owned(),
        ),
    )
    .await;

    assert_eq!(
        reply,
        "Summary: the note says hello from anthropic provider-shape discovery followup test."
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 2);

    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    assert_discovery_first_followup_summary(&persisted, false, "file.read");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_provider_shape_tool_search_followup_bedrock() {
    let (reply, runtime) = run_provider_shape_tool_search_followup(
        "session-provider-shape-bedrock",
        "search for the right tool, then read and summarize note.md",
        "hello from bedrock provider-shape discovery followup test",
        json!({
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [
                        {
                            "text": "Let me search for the right tool first."
                        },
                        {
                            "toolUse": {
                                "toolUseId": "toolu-search",
                                "name": "tool_search",
                                "input": {
                                    "query": "read note.md",
                                    "limit": 3
                                }
                            }
                        }
                    ]
                }
            },
            "stopReason": "tool_use"
        }),
        json!({
            "output": {
                "message": {
                    "role": "assistant",
                    "content": [
                        {
                            "text": "Now I'll read the file."
                        },
                        {
                            "toolUse": {
                                "toolUseId": "toolu-read",
                                "name": "file_read",
                                "input": {
                                    "path": "note.md"
                                }
                            }
                        }
                    ]
                }
            },
            "stopReason": "tool_use"
        }),
        Ok(
            "Summary: the note says hello from bedrock provider-shape discovery followup test."
                .to_owned(),
        ),
    )
    .await;

    assert_eq!(
        reply,
        "Summary: the note says hello from bedrock provider-shape discovery followup test."
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 2);

    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    assert_discovery_first_followup_summary(&persisted, false, "file.read");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_provider_shape_tool_search_followup_inline_raw_output() {
    let (reply, runtime) = run_provider_shape_tool_search_followup(
        "session-provider-shape-inline",
        "search for the right tool, then read note.md and show raw json tool output",
        "hello from inline provider-shape discovery followup raw test",
        json!({
            "choices": [{
                "message": {
                    "content": "Let me search for the right tool first.\n<function=tool_search><parameter=query>read note.md</parameter><parameter=limit>3</parameter></function>"
                }
            }]
        }),
        json!({
            "choices": [{
                "message": {
                    "content": "Now I'll read the file.\n<function=file_read><parameter=path>note.md</parameter></function>"
                }
            }]
        }),
        Ok("unused completion".to_owned()),
    )
    .await;

    assert!(
        reply.contains("[ok]"),
        "raw-request mode should return the invoked tool output, got: {reply}"
    );
    assert!(
        reply.contains("hello from inline provider-shape discovery followup raw test"),
        "expected second-round invoked tool output, got: {reply}"
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 2);
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        0
    );

    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    assert_discovery_first_followup_summary(&persisted, true, "file.read");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_tool_turn_raw_request_skips_second_pass_completion() {
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(
        harness.temp_dir.join("note.md"),
        "hello from coordinator test",
    )
    .expect("seed test note");

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Reading the file now.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!({"path": "note.md"}),
                "session-tool-raw",
                "turn-tool-raw",
                "call-tool-raw",
            )],
            raw_meta: Value::Null,
        }),
        Ok("this must not be used".to_owned()),
    );

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &test_config(),
            "session-tool-raw",
            "read note.md and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
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
async fn handle_turn_with_runtime_tool_search_followup_checkpoint_uses_visible_context() {
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(
        harness.temp_dir.join("note.md"),
        "hello from coordinator search checkpoint test",
    )
    .expect("seed test note");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Let me search for the right tool first.".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "tool.search",
                    json!({"query": "read note.md", "limit": 3}),
                    "session-tool-search-checkpoint",
                    "turn-tool-search-checkpoint",
                    "call-tool-search-checkpoint",
                )],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Now I'll read the file.".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "file.read",
                    json!({"path": "note.md"}),
                    "session-tool-search-checkpoint",
                    "turn-tool-search-checkpoint",
                    "call-tool-invoke-checkpoint",
                )],
                raw_meta: Value::Null,
            }),
        ],
        vec![Ok(
            "Summary: the note says hello from coordinator search checkpoint test.".to_owned(),
        )],
    );
    let mut config = test_config();
    config.conversation.compact_enabled = false;

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-tool-search-checkpoint",
            "search for the right tool, then read and summarize note.md",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::from_optional_kernel_context(Some(&harness.kernel_ctx)),
        )
        .await
        .expect("tool-search checkpoint turn should succeed");

    assert_eq!(
        reply,
        "Summary: the note says hello from coordinator search checkpoint test."
    );

    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert_eq!(
        payloads.len(),
        2,
        "expected post-persist and finalization-success events"
    );
    assert_eq!(payloads[0]["stage"], "post_persist");
    assert_eq!(
        payloads[0]["checkpoint"]["preparation"]["context_message_count"],
        1
    );
    assert_eq!(
        payloads[0]["checkpoint"]["preparation"]["context_fingerprint_sha256"],
        test_turn_preparation_context_fingerprint(&[json!({
            "role": "user",
            "content": "search for the right tool, then read and summarize note.md"
        })])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_provider_switch_tool_updates_provider_for_followup_round() {
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::new();
    let config_path = harness.temp_dir.join("loongclaw.toml");

    let mut config = test_config();
    let mut openai = ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
    openai.model = "gpt-5".to_owned();
    config.set_active_provider_profile(
        "openai-gpt-5",
        crate::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: openai.clone(),
        },
    );
    let mut deepseek = ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Deepseek);
    deepseek.model = "deepseek-chat".to_owned();
    config.providers.insert(
        "deepseek-chat".to_owned(),
        crate::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: deepseek.clone(),
        },
    );
    config.provider = openai;
    config.active_provider = Some("openai-gpt-5".to_owned());

    std::fs::write(
        &config_path,
        crate::config::render(&config).expect("render provider switch config"),
    )
    .expect("write provider switch config");
    let canonical_config_path =
        std::fs::canonicalize(&config_path).expect("canonicalize provider switch config path");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Switching provider.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "provider.switch",
                json!({
                    "selector": "deepseek",
                    "config_path": canonical_config_path.display().to_string()
                }),
                "session-provider-switch",
                "turn-provider-switch-1",
                "call-provider-switch",
            )],
            raw_meta: Value::Null,
        })],
        vec![Ok("DeepSeek is now active.".to_owned())],
    );

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-provider-switch",
            "switch to deepseek and continue",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
        )
        .await
        .expect("provider switch turn should succeed");

    assert_eq!(reply, "DeepSeek is now active.");
    assert_eq!(
        runtime
            .turn_requested_provider_ids
            .lock()
            .expect("turn provider ids lock")
            .clone(),
        vec!["openai-gpt-5".to_owned()]
    );
    assert_eq!(
        runtime
            .completion_requested_provider_ids
            .lock()
            .expect("completion provider ids lock")
            .clone(),
        vec!["deepseek-chat".to_owned()]
    );

    let (_, reloaded) =
        crate::config::load(Some(config_path.to_str().expect("utf8 config path"))).expect("reload");
    assert_eq!(reloaded.active_provider_id(), Some("deepseek-chat"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_honors_configured_tool_result_summary_limit_on_fast_lane() {
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(harness.temp_dir.join("large-note.md"), "x".repeat(8_000))
        .expect("seed large test note");

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Reading large note.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!({"path": "large-note.md"}),
                "session-fast-limit",
                "turn-fast-limit",
                "call-fast-limit",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );

    let mut config = test_config();
    config.conversation.tool_result_payload_summary_limit_chars = 256;

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-fast-limit",
            "read large-note.md and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
        )
        .await
        .expect("tool turn should succeed");

    let line = reply
        .lines()
        .find(|entry| entry.starts_with("[ok] "))
        .expect("reply should include tool envelope line");
    let envelope: Value = serde_json::from_str(
        line.strip_prefix("[ok] ")
            .expect("tool line should keep status prefix"),
    )
    .expect("tool envelope should be valid json");

    assert_eq!(envelope["payload_truncated"], true);
    assert!(
        envelope["payload_chars"]
            .as_u64()
            .expect("payload chars should exist")
            > 256
    );
    let summary = envelope["payload_summary"]
        .as_str()
        .expect("payload summary should be a string");
    assert!(
        summary.contains("...(truncated "),
        "summary should contain truncation marker, got: {summary}"
    );
    assert!(
        summary.chars().count() <= 420,
        "summary should respect configured bound, chars={}",
        summary.chars().count()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_honors_configured_tool_result_summary_limit_on_safe_lane_plan() {
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(harness.temp_dir.join("large-note.md"), "x".repeat(8_000))
        .expect("seed large test note");

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Running deployment read checks.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!({"path": "large-note.md"}),
                "session-safe-limit",
                "turn-safe-limit",
                "call-safe-limit",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );

    let mut config = test_config();
    config.conversation.safe_lane_plan_execution_enabled = true;
    config.conversation.tool_result_payload_summary_limit_chars = 256;

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-safe-limit",
            "deploy production safely and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
        )
        .await
        .expect("safe-lane plan turn should succeed");

    let line = reply
        .lines()
        .find(|entry| entry.starts_with("[ok] "))
        .expect("reply should include tool envelope line");
    let envelope: Value = serde_json::from_str(
        line.strip_prefix("[ok] ")
            .expect("tool line should keep status prefix"),
    )
    .expect("tool envelope should be valid json");

    assert_eq!(envelope["payload_truncated"], true);
    assert!(
        envelope["payload_chars"]
            .as_u64()
            .expect("payload chars should exist")
            > 256
    );
    let summary = envelope["payload_summary"]
        .as_str()
        .expect("payload summary should be a string");
    assert!(
        summary.contains("...(truncated "),
        "summary should contain truncation marker, got: {summary}"
    );
    assert!(
        summary.chars().count() <= 420,
        "summary should respect configured bound, chars={}",
        summary.chars().count()
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_safe_lane_honors_configured_tool_step_budget() {
    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Executing deployment checks.".to_owned(),
            tool_intents: vec![
                provider_tool_intent(
                    "file.read",
                    json!({"path": "note.md"}),
                    "session-safe-budget",
                    "turn-safe-budget",
                    "call-safe-budget-1",
                ),
                provider_tool_intent(
                    "file.read",
                    json!({"path": "checklist.md"}),
                    "session-safe-budget",
                    "turn-safe-budget",
                    "call-safe-budget-2",
                ),
            ],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );

    let mut config = test_config();
    config.conversation.safe_lane_max_tool_steps_per_turn = 2;

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-safe-budget",
            "deploy to production with secret token and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("safe lane should execute with configured step budget");

    assert!(
        reply.contains("no_kernel_context"),
        "expected kernel-context denial once tool-step budget is honored, got: {reply}"
    );
    assert!(
        !reply.contains("max_tool_steps_exceeded"),
        "safe lane should not hit max_tool_steps after config override, got: {reply}"
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_safe_lane_plan_path_bypasses_turn_step_limit() {
    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Executing deployment checks.".to_owned(),
            tool_intents: vec![
                provider_tool_intent(
                    "file.read",
                    json!({"path": "note.md"}),
                    "session-safe-plan",
                    "turn-safe-plan",
                    "call-safe-plan-1",
                ),
                provider_tool_intent(
                    "file.read",
                    json!({"path": "checklist.md"}),
                    "session-safe-plan",
                    "turn-safe-plan",
                    "call-safe-plan-2",
                ),
            ],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );

    let mut config = test_config();
    config.conversation.safe_lane_plan_execution_enabled = true;
    config.conversation.safe_lane_max_tool_steps_per_turn = 1;

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-safe-plan",
            "deploy to production with secret token and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("safe lane plan path should return inline tool error");

    assert!(
        reply.contains("no_kernel_context"),
        "expected kernel-context denial from plan execution path, got: {reply}"
    );
    assert!(
        !reply.contains("max_tool_steps_exceeded"),
        "plan path should not use TurnEngine max_tool_steps gate, got: {reply}"
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_safe_lane_plan_persists_runtime_events_when_enabled() {
    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Executing deployment checks.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!({"path": "note.md"}),
                "session-safe-events",
                "turn-safe-events",
                "call-safe-events-1",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );

    let mut config = test_config();
    config.conversation.safe_lane_plan_execution_enabled = true;
    config.conversation.safe_lane_emit_runtime_events = true;
    config.conversation.safe_lane_replan_max_rounds = 0;

    let coordinator = ConversationTurnCoordinator::new();
    let _reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-safe-events",
            "deploy to production with secret token and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("safe lane plan should produce a reply");

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let event_records = persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            Some((
                parsed.get("event")?.as_str()?.to_owned(),
                parsed.get("payload").cloned().unwrap_or(Value::Null),
            ))
        })
        .collect::<Vec<_>>();
    let event_names = event_records
        .iter()
        .map(|(event, _)| event.to_owned())
        .collect::<Vec<_>>();

    assert!(
        event_names.iter().any(|name| name == "lane_selected"),
        "expected lane_selected event, got: {event_names:?}"
    );
    assert!(
        event_names.iter().any(|name| name == "plan_round_started"),
        "expected plan_round_started event, got: {event_names:?}"
    );
    assert!(
        event_names
            .iter()
            .any(|name| name == "plan_round_completed"),
        "expected plan_round_completed event, got: {event_names:?}"
    );
    assert!(
        event_names.iter().any(|name| name == "final_status"),
        "expected final_status event, got: {event_names:?}"
    );

    let plan_round_completed_payload = event_records
        .iter()
        .find_map(|(event, payload)| (event == "plan_round_completed").then_some(payload))
        .expect("plan_round_completed payload should exist");
    let plan_stats = plan_round_completed_payload
        .get("tool_output_stats")
        .expect("plan_round_completed should include tool_output_stats");
    assert_eq!(
        plan_stats
            .get("truncated_result_lines")
            .and_then(Value::as_u64),
        Some(0)
    );
    let plan_health = plan_round_completed_payload
        .get("health_signal")
        .expect("plan_round_completed should include health_signal");
    assert_eq!(
        plan_health.get("severity").and_then(Value::as_str),
        Some("ok")
    );
    assert_eq!(
        plan_health
            .get("flags")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );

    let final_status_payload = event_records
        .iter()
        .find_map(|(event, payload)| (event == "final_status").then_some(payload))
        .expect("final_status payload should exist");
    let final_stats = final_status_payload
        .get("tool_output_stats")
        .expect("final_status should include tool_output_stats");
    assert_eq!(
        final_stats.get("result_lines").and_then(Value::as_u64),
        Some(0)
    );
    let final_health = final_status_payload
        .get("health_signal")
        .expect("final_status should include health_signal");
    assert_eq!(
        final_health.get("severity").and_then(Value::as_str),
        Some("ok")
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_safe_lane_plan_skips_runtime_events_when_disabled() {
    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Executing deployment checks.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!({"path": "note.md"}),
                "session-safe-events-off",
                "turn-safe-events-off",
                "call-safe-events-off-1",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );

    let mut config = test_config();
    config.conversation.safe_lane_plan_execution_enabled = true;
    config.conversation.safe_lane_emit_runtime_events = false;
    config.conversation.safe_lane_replan_max_rounds = 0;

    let coordinator = ConversationTurnCoordinator::new();
    let _reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-safe-events-off",
            "deploy to production with secret token and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("safe lane plan should produce a reply");

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let event_count = persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            (parsed.get("event")?.as_str()? != "turn_checkpoint").then_some(())
        })
        .count();
    assert_eq!(event_count, 0, "unexpected runtime events: {persisted:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_safe_lane_plan_emits_kernel_runtime_audit_events() {
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(harness.temp_dir.join("note.md"), "safe lane audit probe")
        .expect("write note fixture");
    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Executing deployment checks.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!({"path": "note.md"}),
                "session-safe-audit-on",
                "turn-safe-audit-on",
                "call-safe-audit-on-1",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );

    let mut config = test_config();
    config.conversation.safe_lane_plan_execution_enabled = true;
    config.conversation.safe_lane_emit_runtime_events = true;
    config.conversation.safe_lane_replan_max_rounds = 0;

    let coordinator = ConversationTurnCoordinator::new();
    let _reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-safe-audit-on",
            "deploy to production with secret token and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
        )
        .await
        .expect("safe lane plan should produce a reply");

    let events = harness.audit.snapshot();
    #[allow(clippy::wildcard_enum_match_arm)]
    let runtime_ops = events
        .iter()
        .filter_map(|event| match &event.kind {
            loongclaw_kernel::AuditEventKind::PlaneInvoked {
                pack_id,
                plane,
                tier,
                primary_adapter,
                operation,
                ..
            } if *plane == loongclaw_contracts::ExecutionPlane::Runtime
                && operation.starts_with("conversation.safe_lane.") =>
            {
                Some((
                    pack_id.to_owned(),
                    *tier,
                    primary_adapter.to_owned(),
                    operation.to_owned(),
                ))
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(
        runtime_ops
            .iter()
            .any(|(_, _, _, operation)| operation == "conversation.safe_lane.lane_selected"),
        "expected lane_selected runtime audit event, got: {runtime_ops:?}"
    );
    assert!(
        runtime_ops
            .iter()
            .any(|(_, _, _, operation)| operation == "conversation.safe_lane.final_status"),
        "expected final_status runtime audit event, got: {runtime_ops:?}"
    );
    assert!(
        runtime_ops.iter().all(|(pack_id, tier, adapter, _)| {
            pack_id == "test-pack"
                && *tier == loongclaw_contracts::PlaneTier::Core
                && adapter == "conversation.safe_lane"
        }),
        "unexpected runtime audit metadata: {runtime_ops:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_safe_lane_plan_does_not_emit_kernel_runtime_audit_when_disabled()
{
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(harness.temp_dir.join("note.md"), "safe lane audit disabled")
        .expect("write note fixture");
    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Executing deployment checks.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!({"path": "note.md"}),
                "session-safe-audit-off",
                "turn-safe-audit-off",
                "call-safe-audit-off-1",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );

    let mut config = test_config();
    config.conversation.safe_lane_plan_execution_enabled = true;
    config.conversation.safe_lane_emit_runtime_events = false;
    config.conversation.safe_lane_replan_max_rounds = 0;

    let coordinator = ConversationTurnCoordinator::new();
    let _reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-safe-audit-off",
            "deploy to production with secret token and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
        )
        .await
        .expect("safe lane plan should produce a reply");

    let has_safe_lane_runtime_event = harness.audit.snapshot().iter().any(|event| {
        matches!(
            &event.kind,
            loongclaw_kernel::AuditEventKind::PlaneInvoked {
                plane: loongclaw_contracts::ExecutionPlane::Runtime,
                operation,
                ..
            } if operation.starts_with("conversation.safe_lane.")
        )
    });

    assert!(
        !has_safe_lane_runtime_event,
        "safe-lane runtime audit events should be disabled"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_safe_lane_plan_replans_after_transient_tool_failure() {
    use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest, ToolPlaneError};
    use loongclaw_kernel::CoreToolAdapter;

    struct FlakyOnceToolAdapter {
        calls: Arc<Mutex<usize>>,
    }

    #[async_trait]
    impl CoreToolAdapter for FlakyOnceToolAdapter {
        fn name(&self) -> &str {
            "flaky-once-tools"
        }

        async fn execute_core_tool(
            &self,
            request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, ToolPlaneError> {
            let (tool_name, _arguments) = effective_tool_request(&request);
            let current_call = {
                let mut calls = self.calls.lock().expect("flaky calls lock");
                *calls = calls.saturating_add(1);
                *calls
            };
            if current_call == 1 {
                return Err(ToolPlaneError::Execution(
                    "transient tool failure".to_owned(),
                ));
            }
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "tool": tool_name,
                    "attempt": current_call
                }),
            })
        }
    }

    let call_counter = Arc::new(Mutex::new(0usize));
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
        granted_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");
    kernel.register_core_tool_adapter(FlakyOnceToolAdapter {
        calls: call_counter.clone(),
    });
    kernel
        .set_default_core_tool_adapter("flaky-once-tools")
        .expect("set default core tool adapter");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");
    let ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Running checks.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!({"path": "note.md"}),
                "session-safe-replan",
                "turn-safe-replan",
                "call-safe-replan-1",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );

    let mut config = test_config();
    config.conversation.safe_lane_plan_execution_enabled = true;
    config.conversation.safe_lane_node_max_attempts = 1;
    config.conversation.safe_lane_replan_max_rounds = 1;
    config.conversation.safe_lane_replan_max_node_attempts = 2;
    config.conversation.safe_lane_event_sample_every = 2;

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-safe-replan",
            "deploy to production with secret token and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&ctx),
        )
        .await
        .expect("safe lane plan should recover via bounded replan");

    assert!(
        reply.contains("[ok]"),
        "expected successful tool output after replan, got: {reply}"
    );
    assert!(
        !reply.contains("transient tool failure"),
        "final reply should not leak first transient failure after successful replan, got: {reply}"
    );
    let calls = *call_counter.lock().expect("call counter lock");
    assert_eq!(calls, 2, "expected one failure + one replan success");

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let failed_round_payload = persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            if parsed.get("event")?.as_str()? != "plan_round_completed" {
                return None;
            }
            let payload = parsed.get("payload")?;
            if payload.get("status")?.as_str()? != "failed" {
                return None;
            }
            Some(payload.clone())
        })
        .next()
        .expect("failed plan_round_completed payload");
    assert_eq!(failed_round_payload["failure_kind"], "retryable");
    assert_eq!(
        failed_round_payload["failure_code"],
        "safe_lane_plan_node_retryable_error"
    );
    assert_eq!(failed_round_payload["failure_retryable"], true);
    assert_eq!(failed_round_payload["route_decision"], "replan");
    assert_eq!(failed_round_payload["route_reason"], "retryable_failure");

    let has_sampled_out_success_round = !persisted.iter().any(|(_, role, content)| {
        if role != "assistant" {
            return false;
        }
        let parsed = match serde_json::from_str::<Value>(content) {
            Ok(value) => value,
            Err(_) => return false,
        };
        if parsed.get("type").and_then(Value::as_str) != Some("conversation_event") {
            return false;
        }
        if parsed.get("event").and_then(Value::as_str) != Some("plan_round_completed") {
            return false;
        }
        let payload = match parsed.get("payload") {
            Some(value) => value,
            None => return false,
        };
        payload.get("status").and_then(Value::as_str) == Some("succeeded")
            && payload.get("round").and_then(Value::as_u64) == Some(1)
    });
    assert!(
        has_sampled_out_success_round,
        "round-1 plan_round_completed should be sampled out"
    );

    let final_status_payload = persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            if parsed.get("event")?.as_str()? != "final_status" {
                return None;
            }
            parsed.get("payload").cloned()
        })
        .next_back()
        .expect("final_status payload");
    assert_eq!(final_status_payload["status"], "succeeded");
    assert_eq!(final_status_payload["metrics"]["rounds_started"], 2);
    assert_eq!(final_status_payload["metrics"]["rounds_succeeded"], 1);
    assert_eq!(final_status_payload["metrics"]["rounds_failed"], 1);
    assert_eq!(final_status_payload["metrics"]["verify_failures"], 0);
    assert_eq!(final_status_payload["metrics"]["replans_triggered"], 1);
    assert!(
        final_status_payload["metrics"]["total_attempts_used"]
            .as_u64()
            .unwrap_or_default()
            >= 2
    );

    let summary = summarize_safe_lane_events(
        persisted
            .iter()
            .filter_map(|(_, role, content)| (role == "assistant").then_some(content.as_str())),
    );
    assert_eq!(summary.final_status, Some(SafeLaneFinalStatus::Succeeded));
    assert_eq!(summary.replan_triggered_events, 1);
    assert_eq!(
        summary
            .latest_metrics
            .as_ref()
            .map(|metrics| metrics.rounds_started),
        Some(2)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_safe_lane_backpressure_guard_blocks_retry_storm() {
    use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest, ToolPlaneError};
    use loongclaw_kernel::CoreToolAdapter;

    struct FlakyAlwaysRetryableAdapter {
        calls: Arc<Mutex<usize>>,
    }

    #[async_trait]
    impl CoreToolAdapter for FlakyAlwaysRetryableAdapter {
        fn name(&self) -> &str {
            "flaky-always-retryable-tools"
        }

        async fn execute_core_tool(
            &self,
            _request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, ToolPlaneError> {
            {
                let mut calls = self.calls.lock().expect("flaky calls lock");
                *calls = calls.saturating_add(1);
            }
            Err(ToolPlaneError::Execution(
                "transient tool failure".to_owned(),
            ))
        }
    }

    let call_counter = Arc::new(Mutex::new(0usize));
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
        granted_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");
    kernel.register_core_tool_adapter(FlakyAlwaysRetryableAdapter {
        calls: call_counter.clone(),
    });
    kernel
        .set_default_core_tool_adapter("flaky-always-retryable-tools")
        .expect("set default core tool adapter");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");
    let ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Running checks.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!({"path": "note.md"}),
                "session-safe-backpressure",
                "turn-safe-backpressure",
                "call-safe-backpressure-1",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );

    let mut config = test_config();
    config.conversation.safe_lane_plan_execution_enabled = true;
    config.conversation.safe_lane_node_max_attempts = 1;
    config.conversation.safe_lane_replan_max_rounds = 3;
    config.conversation.safe_lane_replan_max_node_attempts = 4;
    config.conversation.safe_lane_backpressure_guard_enabled = true;
    config
        .conversation
        .safe_lane_backpressure_max_total_attempts = 1;
    config.conversation.safe_lane_backpressure_max_replans = 10;

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-safe-backpressure",
            "deploy to production with secret token and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&ctx),
        )
        .await
        .expect("safe lane should fail-fast under backpressure guard");

    assert!(
        reply.contains("safe_lane_plan_backpressure_guard"),
        "expected explicit backpressure guard reason, got: {reply}"
    );
    let calls = *call_counter.lock().expect("call counter lock");
    assert_eq!(
        calls, 1,
        "backpressure guard should block further replan retries"
    );

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let final_status_payload = persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            if parsed.get("event")?.as_str()? != "final_status" {
                return None;
            }
            parsed.get("payload").cloned()
        })
        .next_back()
        .expect("final_status payload");
    assert_eq!(
        final_status_payload["route_reason"],
        "backpressure_attempts_exhausted"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_safe_lane_verify_non_retryable_failure_skips_replan() {
    use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest, ToolPlaneError};
    use loongclaw_kernel::CoreToolAdapter;

    struct DenyMarkerAdapter {
        calls: Arc<Mutex<usize>>,
    }

    #[async_trait]
    impl CoreToolAdapter for DenyMarkerAdapter {
        fn name(&self) -> &str {
            "deny-marker-tools"
        }

        async fn execute_core_tool(
            &self,
            _request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, ToolPlaneError> {
            let current_call = {
                let mut calls = self.calls.lock().expect("anchor mismatch calls lock");
                *calls = calls.saturating_add(1);
                *calls
            };
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "attempt": current_call,
                    "message": "simulated tool_not_found marker"
                }),
            })
        }
    }

    let call_counter = Arc::new(Mutex::new(0usize));
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
        granted_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");
    kernel.register_core_tool_adapter(DenyMarkerAdapter {
        calls: call_counter.clone(),
    });
    kernel
        .set_default_core_tool_adapter("deny-marker-tools")
        .expect("set default core tool adapter");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");
    let ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Running checks.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!({"path": "note.md"}),
                "session-safe-verify-nonretryable",
                "turn-safe-verify-nonretryable",
                "call-safe-verify-nonretryable-1",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );

    let mut config = test_config();
    config.conversation.safe_lane_plan_execution_enabled = true;
    config.conversation.safe_lane_node_max_attempts = 1;
    config.conversation.safe_lane_replan_max_rounds = 3;
    config.conversation.safe_lane_replan_max_node_attempts = 4;

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-safe-verify-nonretryable",
            "deploy to production with secret token and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&ctx),
        )
        .await
        .expect("safe lane should return verify failure");

    assert!(
        reply.contains("safe_lane_plan_verify_failed"),
        "expected verify failure in reply, got: {reply}"
    );
    let calls = *call_counter.lock().expect("call counter lock");
    assert_eq!(
        calls, 1,
        "non-retryable verify failure should not trigger replan tool re-execution"
    );

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let verify_failed_payload = persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            if parsed.get("event")?.as_str()? != "verify_failed" {
                return None;
            }
            parsed.get("payload").cloned()
        })
        .next_back()
        .expect("verify_failed payload");
    assert_eq!(verify_failed_payload["failure_kind"], "non_retryable");
    assert_eq!(
        verify_failed_payload["failure_code"],
        "safe_lane_plan_verify_failed"
    );
    assert_eq!(verify_failed_payload["failure_retryable"], false);
    assert_eq!(verify_failed_payload["route_decision"], "terminal");
    assert_eq!(
        verify_failed_payload["route_reason"],
        "non_retryable_failure"
    );

    let final_status_payload = persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            if parsed.get("event")?.as_str()? != "final_status" {
                return None;
            }
            parsed.get("payload").cloned()
        })
        .next_back()
        .expect("final_status payload");
    assert_eq!(final_status_payload["failure_kind"], "non_retryable");
    assert_eq!(
        final_status_payload["failure_code"],
        "safe_lane_plan_verify_failed"
    );
    assert_eq!(final_status_payload["failure_retryable"], false);
    assert_eq!(final_status_payload["route_decision"], "terminal");
    assert_eq!(
        final_status_payload["route_reason"],
        "non_retryable_failure"
    );
    assert_eq!(final_status_payload["metrics"]["rounds_started"], 1);
    assert_eq!(final_status_payload["metrics"]["rounds_succeeded"], 1);
    assert_eq!(final_status_payload["metrics"]["rounds_failed"], 0);
    assert_eq!(final_status_payload["metrics"]["verify_failures"], 1);
    assert_eq!(final_status_payload["metrics"]["replans_triggered"], 0);

    let summary = summarize_safe_lane_events(
        persisted
            .iter()
            .filter_map(|(_, role, content)| (role == "assistant").then_some(content.as_str())),
    );
    assert_eq!(summary.final_status, Some(SafeLaneFinalStatus::Failed));
    assert_eq!(
        summary.final_failure_code.as_deref(),
        Some("safe_lane_plan_verify_failed")
    );
    assert_eq!(summary.verify_failed_events, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_safe_lane_session_governor_forces_no_replan() {
    use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest, ToolPlaneError};
    use loongclaw_kernel::{CoreMemoryAdapter, CoreToolAdapter};

    struct FlakyAlwaysRetryableAdapter {
        calls: Arc<Mutex<usize>>,
    }

    #[async_trait]
    impl CoreToolAdapter for FlakyAlwaysRetryableAdapter {
        fn name(&self) -> &str {
            "flaky-governor-tools"
        }

        async fn execute_core_tool(
            &self,
            _request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, ToolPlaneError> {
            {
                let mut calls = self.calls.lock().expect("flaky calls lock");
                *calls = calls.saturating_add(1);
            }
            Err(ToolPlaneError::Execution(
                "transient tool failure".to_owned(),
            ))
        }
    }

    struct ChronicFailureMemoryAdapter;

    #[async_trait]
    impl CoreMemoryAdapter for ChronicFailureMemoryAdapter {
        fn name(&self) -> &str {
            "chronic-failure-memory"
        }

        async fn execute_core_memory(
            &self,
            request: MemoryCoreRequest,
        ) -> Result<MemoryCoreOutcome, MemoryPlaneError> {
            if request.operation == MEMORY_OP_WINDOW {
                return Ok(MemoryCoreOutcome {
                    status: "ok".to_owned(),
                    payload: json!({
                        "turns": [
                            {
                                "role": "assistant",
                                "content": "{\"type\":\"conversation_event\",\"event\":\"final_status\",\"payload\":{\"status\":\"failed\",\"failure_code\":\"safe_lane_plan_node_retryable_error\",\"route_decision\":\"terminal\"}}",
                                "ts": 1
                            }
                        ]
                    }),
                });
            }
            Ok(MemoryCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({}),
            })
        }
    }

    let call_counter = Arc::new(Mutex::new(0usize));
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
        granted_capabilities: BTreeSet::from([
            Capability::InvokeTool,
            Capability::MemoryRead,
            Capability::FilesystemRead,
        ]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");
    kernel.register_core_memory_adapter(ChronicFailureMemoryAdapter);
    kernel
        .set_default_core_memory_adapter("chronic-failure-memory")
        .expect("set default core memory adapter");
    kernel.register_core_tool_adapter(FlakyAlwaysRetryableAdapter {
        calls: call_counter.clone(),
    });
    kernel
        .set_default_core_tool_adapter("flaky-governor-tools")
        .expect("set default core tool adapter");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");
    let ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Running checks.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!({"path": "note.md"}),
                "session-safe-governor",
                "turn-safe-governor",
                "call-safe-governor-1",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );

    let mut config = test_config();
    config.conversation.safe_lane_plan_execution_enabled = true;
    config.conversation.safe_lane_node_max_attempts = 1;
    config.conversation.safe_lane_replan_max_rounds = 3;
    config.conversation.safe_lane_replan_max_node_attempts = 4;
    config.conversation.safe_lane_session_governor_enabled = true;
    config
        .conversation
        .safe_lane_session_governor_failed_final_status_threshold = 1;
    config
        .conversation
        .safe_lane_session_governor_backpressure_failure_threshold = 9;
    config
        .conversation
        .safe_lane_session_governor_force_no_replan = true;
    config
        .conversation
        .safe_lane_session_governor_force_node_max_attempts = 1;

    let coordinator = ConversationTurnCoordinator::new();
    let _reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-safe-governor",
            "deploy to production with secret token and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&ctx),
        )
        .await
        .expect("safe lane should fail without replan under governor");

    let calls = *call_counter.lock().expect("call counter lock");
    assert_eq!(calls, 1, "governor should suppress replans");

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let lane_selected_payload = persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            if parsed.get("event")?.as_str()? != "lane_selected" {
                return None;
            }
            parsed.get("payload").cloned()
        })
        .next_back()
        .expect("lane_selected payload");
    assert_eq!(lane_selected_payload["session_governor"]["engaged"], true);
    assert_eq!(
        lane_selected_payload["session_governor"]["force_no_replan"],
        true
    );
    assert_eq!(
        lane_selected_payload["session_governor"]["failed_threshold_triggered"],
        true
    );
    assert_eq!(
        lane_selected_payload["session_governor"]["trend_enabled"],
        true
    );
    assert_eq!(
        lane_selected_payload["session_governor"]["trend_samples"],
        1
    );
    assert_eq!(
        lane_selected_payload["session_governor"]["trend_threshold_triggered"],
        false
    );
    assert_eq!(
        lane_selected_payload["session_governor"]["recovery_threshold_triggered"],
        false
    );
    assert_eq!(
        lane_selected_payload["session_governor"]["trend_failure_ewma"],
        Value::Null
    );

    let round_started_payload = persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            if parsed.get("event")?.as_str()? != "plan_round_started" {
                return None;
            }
            parsed.get("payload").cloned()
        })
        .next_back()
        .expect("plan_round_started payload");
    assert_eq!(round_started_payload["effective_max_rounds"], 0);
    assert_eq!(round_started_payload["effective_max_node_attempts"], 1);

    let final_status_payload = persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            if parsed.get("event")?.as_str()? != "final_status" {
                return None;
            }
            parsed.get("payload").cloned()
        })
        .next_back()
        .expect("final_status payload");
    assert_eq!(
        final_status_payload["route_reason"],
        "session_governor_no_replan"
    );
    assert_eq!(
        final_status_payload["failure_code"],
        "safe_lane_plan_session_governor_no_replan"
    );
    assert_eq!(final_status_payload["metrics"]["replans_triggered"], 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_safe_lane_session_governor_requests_extended_history_window() {
    use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};
    use loongclaw_kernel::{CoreMemoryAdapter, CoreToolAdapter};

    struct NoopToolAdapter;

    #[async_trait]
    impl CoreToolAdapter for NoopToolAdapter {
        fn name(&self) -> &str {
            "noop-governor-tool"
        }

        async fn execute_core_tool(
            &self,
            _request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, loongclaw_contracts::ToolPlaneError> {
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({"ok": true}),
            })
        }
    }

    struct CapturingMemoryAdapter {
        invocations: Arc<Mutex<Vec<MemoryCoreRequest>>>,
    }

    #[async_trait]
    impl CoreMemoryAdapter for CapturingMemoryAdapter {
        fn name(&self) -> &str {
            "capturing-governor-memory"
        }

        async fn execute_core_memory(
            &self,
            request: MemoryCoreRequest,
        ) -> Result<MemoryCoreOutcome, MemoryPlaneError> {
            self.invocations
                .lock()
                .expect("memory invocations lock")
                .push(request.clone());
            if request.operation == MEMORY_OP_WINDOW {
                return Ok(MemoryCoreOutcome {
                    status: "ok".to_owned(),
                    payload: json!({
                        "turns": []
                    }),
                });
            }
            Ok(MemoryCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({}),
            })
        }
    }

    let memory_invocations = Arc::new(Mutex::new(Vec::<MemoryCoreRequest>::new()));
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
        granted_capabilities: BTreeSet::from([
            Capability::InvokeTool,
            Capability::MemoryRead,
            Capability::MemoryWrite,
        ]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");
    kernel.register_core_tool_adapter(NoopToolAdapter);
    kernel
        .set_default_core_tool_adapter("noop-governor-tool")
        .expect("set default core tool adapter");
    kernel.register_core_memory_adapter(CapturingMemoryAdapter {
        invocations: memory_invocations.clone(),
    });
    kernel
        .set_default_core_memory_adapter("capturing-governor-memory")
        .expect("set default core memory adapter");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");
    let ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Running checks.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!({"path": "note.md"}),
                "session-safe-governor-window",
                "turn-safe-governor-window",
                "call-safe-governor-window-1",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );

    let mut config = test_config();
    config.conversation.safe_lane_plan_execution_enabled = true;
    config.conversation.safe_lane_session_governor_enabled = true;
    config.conversation.safe_lane_session_governor_window_turns = 200;

    let coordinator = ConversationTurnCoordinator::new();
    let _ = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-safe-governor-window",
            "deploy to production with secret token and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&ctx),
        )
        .await
        .expect("safe lane turn should complete");

    let captured = memory_invocations
        .lock()
        .expect("memory invocations lock")
        .clone();
    let window_request = captured
        .iter()
        .find(|request| request.operation == MEMORY_OP_WINDOW)
        .expect("window request should be issued");
    assert_eq!(
        window_request.payload["session_id"],
        "session-safe-governor-window"
    );
    assert_eq!(window_request.payload["limit"], 200);
    assert_eq!(window_request.payload["allow_extended_limit"], true);

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let lane_selected_payload = persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            if parsed.get("event")?.as_str()? != "lane_selected" {
                return None;
            }
            parsed.get("payload").cloned()
        })
        .next_back()
        .expect("lane_selected payload");
    assert_eq!(
        lane_selected_payload["session_governor"]["history_load_status"],
        "loaded"
    );
    assert_eq!(
        lane_selected_payload["session_governor"]["history_load_error"],
        Value::Null
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_safe_lane_session_governor_does_not_reuse_sqlite_history_when_kernel_window_is_non_ok()
 {
    use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest, ToolPlaneError};
    use loongclaw_kernel::{CoreMemoryAdapter, CoreToolAdapter};

    struct FlakyAlwaysRetryableAdapter {
        calls: Arc<Mutex<usize>>,
    }

    #[async_trait]
    impl CoreToolAdapter for FlakyAlwaysRetryableAdapter {
        fn name(&self) -> &str {
            "flaky-governor-fallback-tools"
        }

        async fn execute_core_tool(
            &self,
            _request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, ToolPlaneError> {
            {
                let mut calls = self.calls.lock().expect("flaky calls lock");
                *calls = calls.saturating_add(1);
            }
            Err(ToolPlaneError::Execution(
                "transient tool failure".to_owned(),
            ))
        }
    }

    struct NonOkWindowMemoryAdapter;

    #[async_trait]
    impl CoreMemoryAdapter for NonOkWindowMemoryAdapter {
        fn name(&self) -> &str {
            "non-ok-governor-memory"
        }

        async fn execute_core_memory(
            &self,
            request: MemoryCoreRequest,
        ) -> Result<MemoryCoreOutcome, MemoryPlaneError> {
            if request.operation == MEMORY_OP_WINDOW {
                return Ok(MemoryCoreOutcome {
                    status: "error".to_owned(),
                    payload: json!({
                        "reason": "kernel memory window unavailable"
                    }),
                });
            }
            Ok(MemoryCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({}),
            })
        }
    }

    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-safe-lane-governor", "sqlite-fallback")
    ));
    let _ = std::fs::remove_file(&db_path);

    let call_counter = Arc::new(Mutex::new(0usize));
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
        granted_capabilities: BTreeSet::from([
            Capability::InvokeTool,
            Capability::MemoryRead,
            Capability::FilesystemRead,
        ]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");
    kernel.register_core_memory_adapter(NonOkWindowMemoryAdapter);
    kernel
        .set_default_core_memory_adapter("non-ok-governor-memory")
        .expect("set default core memory adapter");
    kernel.register_core_tool_adapter(FlakyAlwaysRetryableAdapter {
        calls: call_counter.clone(),
    });
    kernel
        .set_default_core_tool_adapter("flaky-governor-fallback-tools")
        .expect("set default core tool adapter");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");
    let ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.conversation.safe_lane_plan_execution_enabled = true;
    config.conversation.safe_lane_node_max_attempts = 1;
    config.conversation.safe_lane_replan_max_rounds = 3;
    config.conversation.safe_lane_replan_max_node_attempts = 4;
    config.conversation.safe_lane_session_governor_enabled = true;
    config
        .conversation
        .safe_lane_session_governor_failed_final_status_threshold = 1;
    config
        .conversation
        .safe_lane_session_governor_backpressure_failure_threshold = 9;
    config
        .conversation
        .safe_lane_session_governor_force_no_replan = true;
    config
        .conversation
        .safe_lane_session_governor_force_node_max_attempts = 1;

    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    crate::memory::append_turn_direct(
        "session-safe-governor-fallback",
        "assistant",
        r#"{"type":"conversation_event","event":"final_status","payload":{"status":"failed","failure_code":"safe_lane_plan_node_retryable_error","route_decision":"terminal"}} "#.trim(),
        &mem_config,
    )
    .expect("persist governor history into configured sqlite db");

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Running checks.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!({"path": "note.md"}),
                "session-safe-governor-fallback",
                "turn-safe-governor-fallback",
                "call-safe-governor-fallback-1",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );

    let coordinator = ConversationTurnCoordinator::new();
    let _reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-safe-governor-fallback",
            "deploy to production with secret token and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&ctx),
        )
        .await
        .expect("safe lane turn should continue without governed history fallback");

    let calls = *call_counter.lock().expect("call counter lock");
    assert!(
        calls > 1,
        "governor should no longer suppress replans from configured sqlite history when kernel history is unavailable"
    );

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let lane_selected_payload = persisted
        .iter()
        .filter_map(|(_, role, content)| {
            if role != "assistant" {
                return None;
            }
            let parsed = serde_json::from_str::<Value>(content).ok()?;
            if parsed.get("type")?.as_str()? != "conversation_event" {
                return None;
            }
            if parsed.get("event")?.as_str()? != "lane_selected" {
                return None;
            }
            parsed.get("payload").cloned()
        })
        .next_back()
        .expect("lane_selected payload");
    assert_eq!(lane_selected_payload["session_governor"]["engaged"], false);
    assert_eq!(
        lane_selected_payload["session_governor"]["failed_threshold_triggered"],
        false
    );
    assert_eq!(
        lane_selected_payload["session_governor"]["history_load_status"],
        "unavailable"
    );
    assert_eq!(
        lane_selected_payload["session_governor"]["history_load_error"], "kernel_non_ok_status",
        "expected normalized governor history load error code: {lane_selected_payload:?}"
    );

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_safe_lane_replans_failed_subgraph_only() {
    use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest, ToolPlaneError};
    use loongclaw_kernel::CoreToolAdapter;

    #[derive(Default)]
    struct CallCounters {
        note: usize,
        checklist: usize,
    }

    struct FailChecklistOnceAdapter {
        counters: Arc<Mutex<CallCounters>>,
    }

    #[async_trait]
    impl CoreToolAdapter for FailChecklistOnceAdapter {
        fn name(&self) -> &str {
            "fail-checklist-once-tools"
        }

        async fn execute_core_tool(
            &self,
            request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, ToolPlaneError> {
            let (_tool_name, arguments) = effective_tool_request(&request);
            let path = arguments
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let (note_calls, checklist_calls) = {
                let mut counters = self.counters.lock().expect("counters lock");
                match path.as_str() {
                    "note.md" => counters.note = counters.note.saturating_add(1),
                    "checklist.md" => counters.checklist = counters.checklist.saturating_add(1),
                    _ => {}
                }
                (counters.note, counters.checklist)
            };

            if path == "checklist.md" && checklist_calls == 1 {
                return Err(ToolPlaneError::Execution(
                    "transient checklist failure".to_owned(),
                ));
            }

            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "path": path,
                    "note_calls": note_calls,
                    "checklist_calls": checklist_calls
                }),
            })
        }
    }

    let counters = Arc::new(Mutex::new(CallCounters::default()));
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
        granted_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");
    kernel.register_core_tool_adapter(FailChecklistOnceAdapter {
        counters: counters.clone(),
    });
    kernel
        .set_default_core_tool_adapter("fail-checklist-once-tools")
        .expect("set default core tool adapter");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");
    let ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Running checks.".to_owned(),
            tool_intents: vec![
                provider_tool_intent(
                    "file.read",
                    json!({"path": "note.md"}),
                    "session-safe-subgraph",
                    "turn-safe-subgraph",
                    "call-safe-subgraph-1",
                ),
                provider_tool_intent(
                    "file.read",
                    json!({"path": "checklist.md"}),
                    "session-safe-subgraph",
                    "turn-safe-subgraph",
                    "call-safe-subgraph-2",
                ),
            ],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );

    let mut config = test_config();
    config.conversation.safe_lane_plan_execution_enabled = true;
    config.conversation.safe_lane_node_max_attempts = 1;
    config.conversation.safe_lane_replan_max_rounds = 1;
    config.conversation.safe_lane_replan_max_node_attempts = 2;

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "session-safe-subgraph",
            "deploy to production with secret token and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&ctx),
        )
        .await
        .expect("safe lane should recover by replaying only failed subgraph");

    assert!(
        reply.contains("note.md"),
        "expected note output, got: {reply}"
    );
    assert!(
        reply.contains("checklist.md"),
        "expected checklist output, got: {reply}"
    );

    let counters = counters.lock().expect("counters lock");
    assert_eq!(counters.note, 1, "note.md should not be re-executed");
    assert_eq!(
        counters.checklist, 2,
        "checklist.md should execute once + one replan retry"
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_tool_denial_returns_inline_reply_even_in_propagate_mode() {
    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Reading the file now.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!({"path": "note.md"}),
                "session-denied",
                "turn-denied",
                "call-denied",
            )],
            raw_meta: Value::Null,
        }),
        Ok("MODEL_DENIED_REPLY".to_owned()),
    );

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &test_config(),
            "session-denied",
            "read note.md",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
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
        1,
        "tool-denied fallback should run a completion pass for language-aware output"
    );

    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let visible_turns = persisted_visible_turns(&persisted);
    assert_eq!(visible_turns.len(), 2);
    assert_eq!(visible_turns[1].2, reply);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_tool_error_returns_natural_language_fallback() {
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::new();
    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Reading the file now.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!("not an object"),
                "session-tool-error",
                "turn-tool-error",
                "call-tool-error",
            )],
            raw_meta: Value::Null,
        }),
        Ok("MODEL_ERROR_REPLY".to_owned()),
    );

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &test_config(),
            "session-tool-error",
            "read note.md",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
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
        1,
        "tool-error fallback should run a completion pass for language-aware output"
    );

    let persisted = runtime.persisted.lock().expect("persisted lock").clone();
    let visible_turns = persisted_visible_turns(&persisted);
    assert_eq!(visible_turns.len(), 2);
    assert_eq!(visible_turns[1].2, reply);
}

#[tokio::test]
async fn handle_turn_with_runtime_tool_failure_completion_error_uses_raw_reason_without_markers() {
    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Reading the file now.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "file.read",
                json!({"path": "note.md"}),
                "session-denied-fallback",
                "turn-denied-fallback",
                "call-denied-fallback",
            )],
            raw_meta: Value::Null,
        }),
        Err("completion_unavailable".to_owned()),
    );

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &test_config(),
            "session-denied-fallback",
            "read note.md",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
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
    use crate::conversation::turn_engine::{ProviderTurn, TurnEngine, TurnValidation};
    let engine = TurnEngine::new(1); // max_tool_steps = 1
    let turn = ProviderTurn {
        assistant_text: "Hello!".to_owned(),
        tool_intents: vec![],
        raw_meta: serde_json::Value::Null,
    };
    let result = engine.validate_turn(&turn);
    match result {
        Ok(TurnValidation::FinalText(text)) => assert_eq!(text, "Hello!"),
        other => panic!("expected FinalText, got {:?}", other),
    }
}

#[test]
fn provider_direct_discoverable_alias_without_search_is_rejected() {
    use crate::conversation::turn_engine::{TurnEngine, TurnFailureKind};
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
    let result = engine.validate_turn(&turn);
    match result {
        Err(failure) => {
            assert_eq!(failure.kind, TurnFailureKind::PolicyDenied);
            assert_eq!(failure.code, "tool_not_found");
            assert!(
                !failure.reason.contains("file.read"),
                "provider denial should not confirm guessed discoverable tool names: {failure:?}"
            );
        }
        other => panic!("expected provider denial, got {:?}", other),
    }
}

#[test]
fn provider_hidden_tool_denial_does_not_leak_name() {
    use crate::conversation::turn_engine::{
        ProviderTurn, ToolIntent, TurnEngine, TurnFailureKind, TurnResult,
    };

    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![ToolIntent {
            tool_name: "tool.invoke".to_owned(),
            args_json: serde_json::json!({
                "tool_id": "sessions_send",
                "lease": "guessed.invalid",
                "arguments": {"session_id": "child", "text": "hi"}
            }),
            source: "provider_tool_call".to_owned(),
            session_id: "child-session".to_owned(),
            turn_id: "turn-hidden".to_owned(),
            tool_call_id: "call-hidden".to_owned(),
        }],
        raw_meta: Value::Null,
    };

    let engine = TurnEngine::new(1);
    let result = engine.evaluate_turn_in_view(
        &turn,
        &crate::tools::ToolView::from_tool_names(["tool.search", "tool.invoke", "file.read"]),
    );

    match result {
        TurnResult::ToolDenied(failure) => {
            assert_eq!(failure.kind, TurnFailureKind::PolicyDenied);
            assert_eq!(failure.code, "tool_not_found");
            assert!(
                !failure.reason.contains("sessions_send"),
                "provider denial should not leak hidden tool ids: {failure:?}"
            );
        }
        other @ TurnResult::FinalText(_)
        | other @ TurnResult::NeedsApproval(_)
        | other @ TurnResult::ToolError(_)
        | other @ TurnResult::ProviderError(_) => {
            panic!("expected ToolDenied, got {:?}", other)
        }
    }
}

#[test]
fn turn_engine_unknown_tool_returns_tool_denied() {
    use crate::conversation::turn_engine::{ProviderTurn, ToolIntent, TurnEngine};
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
    let result = engine.validate_turn(&turn);
    match result {
        Err(reason) => assert!(reason.contains("tool_not_found"), "reason: {reason}"),
        other => panic!("expected ToolDenied, got {:?}", other),
    }
}

#[test]
fn turn_engine_unknown_tool_exposes_structured_policy_denial() {
    use crate::conversation::turn_engine::{ProviderTurn, ToolIntent, TurnEngine, TurnFailureKind};
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

    let result = engine.validate_turn(&turn);
    match result {
        Err(failure) => {
            assert_eq!(failure.kind, TurnFailureKind::PolicyDenied);
            assert_eq!(failure.code, "tool_not_found");
            assert!(!failure.retryable);
            assert!(
                failure.reason.contains("tool_not_found"),
                "failure={failure:?}"
            );
        }
        other => panic!("expected ToolDenied, got {:?}", other),
    }
}

#[test]
fn turn_engine_exceeding_max_steps_returns_denied() {
    use crate::conversation::turn_engine::{ProviderTurn, TurnEngine};
    let engine = TurnEngine::new(1);
    let intent = provider_tool_intent("file.read", serde_json::json!({}), "s1", "t1", "c1");
    let turn = ProviderTurn {
        assistant_text: "".to_owned(),
        tool_intents: vec![intent.clone(), intent],
        raw_meta: serde_json::Value::Null,
    };
    let result = engine.validate_turn(&turn);
    match result {
        Err(reason) => assert!(
            reason.contains("max_tool_steps_exceeded"),
            "reason: {reason}"
        ),
        other => panic!("expected ToolDenied for max steps, got {:?}", other),
    }
}

#[test]
fn turn_engine_known_tool_validates_to_execution_required() {
    use crate::conversation::turn_engine::{ProviderTurn, TurnEngine, TurnValidation};
    let engine = TurnEngine::new(1);
    let turn = ProviderTurn {
        assistant_text: "".to_owned(),
        tool_intents: vec![provider_tool_intent(
            "file.read",
            serde_json::json!({"path": "test.txt"}),
            "s1",
            "t1",
            "c1",
        )],
        raw_meta: serde_json::Value::Null,
    };
    let result = engine.validate_turn(&turn);
    match result {
        Ok(TurnValidation::ToolExecutionRequired) => {}
        other => panic!("expected ToolExecutionRequired, got {:?}", other),
    }
}

#[test]
fn turn_engine_denies_known_tool_outside_restricted_view() {
    use crate::conversation::turn_engine::{
        ProviderTurn, ToolIntent, TurnEngine, TurnFailureKind, TurnResult,
    };

    let engine = TurnEngine::new(1);
    let turn = ProviderTurn {
        assistant_text: "".to_owned(),
        tool_intents: vec![ToolIntent {
            tool_name: "shell.exec".to_owned(),
            args_json: serde_json::json!({"command": "echo", "args": ["hidden"]}),
            source: "provider_tool_call".to_owned(),
            session_id: "delegate-child".to_owned(),
            turn_id: "t1".to_owned(),
            tool_call_id: "c-hidden".to_owned(),
        }],
        raw_meta: serde_json::Value::Null,
    };

    let result = engine.evaluate_turn_in_view(
        &turn,
        &crate::tools::ToolView::from_tool_names(["file.read"]),
    );

    match result {
        TurnResult::ToolDenied(failure) => {
            assert_eq!(failure.kind, TurnFailureKind::PolicyDenied);
            assert_eq!(failure.code, "tool_not_found");
            assert!(
                failure.reason.contains("tool_not_found"),
                "failure={failure:?}"
            );
            assert!(
                !failure.reason.contains("shell.exec"),
                "provider denial should not leak hidden tool names: {failure:?}"
            );
        }
        other @ TurnResult::FinalText(_)
        | other @ TurnResult::NeedsApproval(_)
        | other @ TurnResult::ToolError(_)
        | other @ TurnResult::ProviderError(_) => {
            panic!("expected ToolDenied, got {:?}", other)
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
            _binding: crate::conversation::ConversationRuntimeBinding<'_>,
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
        tool_intents: vec![provider_tool_intent(
            "sessions_list",
            json!({}),
            "root-session",
            "turn-app-1",
            "call-app-1",
        )],
        raw_meta: Value::Null,
    };
    let session_context = crate::conversation::SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::planned_root_tool_view(),
    );
    let (kernel_ctx, _invocations) = build_kernel_context(Arc::new(InMemoryAuditSink::default()));

    let result = engine
        .execute_turn_in_context(
            &turn,
            &session_context,
            &dispatcher,
            crate::conversation::ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
        )
        .await;

    match result {
        TurnResult::FinalText(text) => {
            let line = text.lines().next().expect("tool result line should exist");
            let payload = line
                .strip_prefix("[ok] ")
                .expect("tool result line should keep [ok] prefix");
            let envelope: Value =
                serde_json::from_str(payload).expect("tool result envelope should be json");
            assert_eq!(envelope["tool"], "sessions_list");
            assert!(
                envelope["payload_summary"]
                    .as_str()
                    .expect("payload summary should be text")
                    .contains("\"tool_name\":\"sessions_list\""),
                "expected dispatcher payload in output, got: {text}"
            );
        }
        other @ TurnResult::ToolDenied(_)
        | other @ TurnResult::NeedsApproval(_)
        | other @ TurnResult::ToolError(_)
        | other @ TurnResult::ProviderError(_) => {
            panic!("expected FinalText, got: {other:?}")
        }
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
async fn turn_engine_routes_direct_binding_to_app_dispatcher() {
    use async_trait::async_trait;
    use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest};

    #[derive(Default)]
    struct BindingRecordingAppDispatcher {
        bindings: Mutex<Vec<bool>>,
    }

    #[async_trait]
    impl crate::conversation::AppToolDispatcher for BindingRecordingAppDispatcher {
        async fn execute_app_tool(
            &self,
            _session_context: &crate::conversation::SessionContext,
            request: ToolCoreRequest,
            binding: crate::conversation::ConversationRuntimeBinding<'_>,
        ) -> Result<ToolCoreOutcome, String> {
            self.bindings
                .lock()
                .expect("dispatcher bindings lock")
                .push(binding.is_kernel_bound());
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "tool_name": request.tool_name,
                    "binding_scope": if binding.is_kernel_bound() {
                        "kernel"
                    } else {
                        "direct"
                    },
                }),
            })
        }
    }

    let dispatcher = BindingRecordingAppDispatcher::default();
    let engine = TurnEngine::new(1);
    let turn = ProviderTurn {
        assistant_text: "".to_owned(),
        tool_intents: vec![provider_tool_intent(
            "sessions_list",
            json!({}),
            "root-session",
            "turn-app-direct",
            "call-app-direct",
        )],
        raw_meta: Value::Null,
    };
    let session_context = crate::conversation::SessionContext::root_with_tool_view(
        "root-session",
        crate::tools::planned_root_tool_view(),
    );

    let result = engine
        .execute_turn_in_context(
            &turn,
            &session_context,
            &dispatcher,
            crate::conversation::ConversationRuntimeBinding::direct(),
            None,
        )
        .await;

    match result {
        TurnResult::FinalText(text) => {
            let line = text.lines().next().expect("tool result line should exist");
            let payload = line
                .strip_prefix("[ok] ")
                .expect("tool result line should keep [ok] prefix");
            let envelope: Value =
                serde_json::from_str(payload).expect("tool result envelope should be json");
            assert_eq!(envelope["tool"], "sessions_list");
            assert!(
                envelope["payload_summary"]
                    .as_str()
                    .expect("payload summary should be text")
                    .contains("\"binding_scope\":\"direct\""),
                "expected direct binding payload in output, got: {text}"
            );
        }
        other @ TurnResult::NeedsApproval(_)
        | other @ TurnResult::ToolDenied(_)
        | other @ TurnResult::ToolError(_)
        | other @ TurnResult::ProviderError(_) => {
            panic!("expected FinalText, got: {other:?}")
        }
    }

    assert_eq!(
        dispatcher
            .bindings
            .lock()
            .expect("dispatcher bindings lock")
            .as_slice(),
        &[false]
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn default_app_tool_dispatcher_executes_session_wait_for_visible_terminal_child_session() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-session-wait", "dispatcher")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Completed,
    })
    .expect("create child session");
    repo.upsert_terminal_outcome(
        "child-session",
        "ok",
        json!({
            "child_session_id": "child-session",
            "final_output": "done"
        }),
    )
    .expect("upsert terminal outcome");

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
                    "session_id": "child-session",
                    "timeout_ms": 50
                }),
            },
            crate::conversation::ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("session_wait outcome");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["wait_status"], "completed");
    assert_eq!(outcome.payload["session"]["session_id"], "child-session");
    assert_eq!(outcome.payload["terminal_outcome"]["status"], "ok");
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn child_session_hidden_session_wait_is_rejected_by_default_dispatcher() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-session-wait", "hidden-child")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Completed,
    })
    .expect("create child session");

    let dispatcher = DefaultAppToolDispatcher::new(memory_config, config.tools.clone());
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
            crate::conversation::ConversationRuntimeBinding::direct(),
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
async fn child_session_hidden_sessions_send_is_rejected_by_default_dispatcher() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-sessions-send", "hidden-child")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.tools.messages.enabled = true;
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create child session");

    let dispatcher = DefaultAppToolDispatcher::new(memory_config, config.tools.clone());
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
            crate::conversation::ConversationRuntimeBinding::direct(),
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
async fn sessions_send_rejects_unknown_target_session() {
    let mut config = test_config();
    config.tools.messages.enabled = true;
    config.memory.sqlite_path = unique_acp_sqlite_path("sessions-send-unknown-target");

    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
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
            crate::conversation::ConversationRuntimeBinding::direct(),
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
    let mut config = test_config();
    config.tools.messages.enabled = true;
    config.memory.sqlite_path = unique_acp_sqlite_path("sessions-send-child-target");

    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
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
            crate::conversation::ConversationRuntimeBinding::direct(),
        )
        .await
        .expect_err("delegate child target must be rejected");

    assert!(
        error.contains("sessions_send_not_supported") && error.contains("not a root session"),
        "expected root-session rejection, got: {error}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_engine_tool_execution_error_is_marked_retryable() {
    use crate::conversation::turn_engine::{ProviderTurn, TurnEngine, TurnFailureKind, TurnResult};
    use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest, ToolPlaneError};
    use loongclaw_kernel::CoreToolAdapter;

    struct RetryableErrorToolAdapter;

    #[async_trait]
    impl CoreToolAdapter for RetryableErrorToolAdapter {
        fn name(&self) -> &str {
            "retryable-error-tools"
        }

        async fn execute_core_tool(
            &self,
            _request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, ToolPlaneError> {
            Err(ToolPlaneError::Execution("transient failure".to_owned()))
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
        granted_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");
    kernel.register_core_tool_adapter(RetryableErrorToolAdapter);
    kernel
        .set_default_core_tool_adapter("retryable-error-tools")
        .expect("set default");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");

    let ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };

    let engine = TurnEngine::new(1);
    let turn = ProviderTurn {
        assistant_text: "".to_owned(),
        tool_intents: vec![provider_tool_intent(
            "file.read",
            json!({"path": "test.txt"}),
            "s1",
            "t1",
            "c1",
        )],
        raw_meta: serde_json::Value::Null,
    };

    let result = engine.execute_turn(&turn, &ctx).await;
    #[allow(clippy::wildcard_enum_match_arm)]
    match result {
        TurnResult::ToolError(failure) => {
            assert_eq!(failure.kind, TurnFailureKind::Retryable);
            assert_eq!(failure.code, "tool_execution_failed");
            assert!(failure.retryable);
            assert!(
                failure.reason.contains("transient failure"),
                "failure={failure:?}"
            );
        }
        other => panic!("expected ToolError, got {:?}", other),
    }
}

#[test]
fn kernel_error_classification_table_is_stable() {
    use crate::conversation::turn_engine::{KernelFailureClass, classify_kernel_error};
    use loongclaw_contracts::{KernelError, PolicyError, RuntimePlaneError, ToolPlaneError};

    let policy_error = KernelError::Policy(PolicyError::ToolCallDenied {
        tool_name: "file.read".to_owned(),
        reason: "blocked".to_owned(),
    });
    assert_eq!(
        classify_kernel_error(&policy_error),
        KernelFailureClass::PolicyDenied
    );

    let boundary_error = KernelError::PackCapabilityBoundary {
        pack_id: "test-pack".to_owned(),
        capability: Capability::InvokeTool,
    };
    assert_eq!(
        classify_kernel_error(&boundary_error),
        KernelFailureClass::PolicyDenied
    );

    let connector_error = KernelError::ConnectorNotAllowed {
        connector: "shell".to_owned(),
        pack_id: "test-pack".to_owned(),
    };
    assert_eq!(
        classify_kernel_error(&connector_error),
        KernelFailureClass::PolicyDenied
    );

    let retryable_tool_error =
        KernelError::ToolPlane(ToolPlaneError::Execution("temporary outage".to_owned()));
    assert_eq!(
        classify_kernel_error(&retryable_tool_error),
        KernelFailureClass::RetryableExecution
    );

    let policy_wrapped_tool_error = KernelError::ToolPlane(ToolPlaneError::Execution(
        "policy_denied: blocked".to_owned(),
    ));
    assert_eq!(
        classify_kernel_error(&policy_wrapped_tool_error),
        KernelFailureClass::PolicyDenied
    );

    let non_retryable_tool_error = KernelError::ToolPlane(ToolPlaneError::NoDefaultCoreAdapter);
    assert_eq!(
        classify_kernel_error(&non_retryable_tool_error),
        KernelFailureClass::NonRetryable
    );

    let runtime_error =
        KernelError::RuntimePlane(RuntimePlaneError::Execution("runtime failure".to_owned()));
    assert_eq!(
        classify_kernel_error(&runtime_error),
        KernelFailureClass::NonRetryable
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_engine_executes_known_tool_with_kernel() {
    use crate::conversation::turn_engine::{ProviderTurn, TurnEngine, TurnResult};
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
            let (tool_name, arguments) = effective_tool_request(&request);
            // Echo back the tool name and payload
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({"tool": tool_name, "input": arguments}),
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
        granted_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead]),
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
        tool_intents: vec![provider_tool_intent(
            "file.read",
            json!({"path": "test.txt"}),
            "s1",
            "t1",
            "c1",
        )],
        raw_meta: serde_json::Value::Null,
    };

    let result = engine.execute_turn(&turn, &ctx).await;
    #[allow(clippy::wildcard_enum_match_arm)]
    match result {
        TurnResult::FinalText(text) => {
            let line = text.lines().next().expect("tool result line should exist");
            let payload = line
                .strip_prefix("[ok] ")
                .expect("tool result line should keep [ok] prefix");
            let envelope: Value =
                serde_json::from_str(payload).expect("tool result envelope should be json");
            assert!(
                payload.contains("\"tool\":\"file.read\""),
                "expected echoed tool payload in output, got: {text}"
            );
            assert_eq!(envelope["status"], "ok");
            assert_eq!(envelope["tool"], "file.read");
            assert_eq!(envelope["tool_call_id"], "c1");
            assert_eq!(envelope["payload_truncated"], false);
            assert!(
                envelope["payload_summary"]
                    .as_str()
                    .expect("payload summary should be string")
                    .contains("\"path\":\"test.txt\""),
                "expected payload summary to include original args, got: {envelope:?}"
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
async fn turn_engine_truncates_oversized_tool_payload_summary() {
    use crate::conversation::turn_engine::{ProviderTurn, TurnEngine, TurnResult};
    use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest, ToolPlaneError};
    use loongclaw_kernel::CoreToolAdapter;

    struct LargePayloadToolAdapter;

    #[async_trait]
    impl CoreToolAdapter for LargePayloadToolAdapter {
        fn name(&self) -> &str {
            "large-payload-tools"
        }

        async fn execute_core_tool(
            &self,
            request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, ToolPlaneError> {
            let (tool_name, _arguments) = effective_tool_request(&request);
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "tool": tool_name,
                    "blob": "x".repeat(10_000)
                }),
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
        granted_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");
    kernel.register_core_tool_adapter(LargePayloadToolAdapter);
    kernel
        .set_default_core_tool_adapter("large-payload-tools")
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
        tool_intents: vec![provider_tool_intent(
            "file.read",
            json!({"path": "test.txt"}),
            "s1",
            "t1",
            "c-large",
        )],
        raw_meta: serde_json::Value::Null,
    };

    let result = engine.execute_turn(&turn, &ctx).await;
    #[allow(clippy::wildcard_enum_match_arm)]
    match result {
        TurnResult::FinalText(text) => {
            let line = text.lines().next().expect("tool result line should exist");
            let payload = line
                .strip_prefix("[ok] ")
                .expect("tool result line should keep [ok] prefix");
            let envelope: Value =
                serde_json::from_str(payload).expect("tool result envelope should be json");

            assert_eq!(envelope["tool"], "file.read");
            assert_eq!(envelope["tool_call_id"], "c-large");
            assert_eq!(envelope["payload_truncated"], true);
            assert!(
                envelope["payload_chars"]
                    .as_u64()
                    .expect("payload chars should exist")
                    > 2048
            );
            let summary = envelope["payload_summary"]
                .as_str()
                .expect("payload summary should be string");
            assert!(
                summary.contains("...(truncated "),
                "expected truncated marker, got: {summary}"
            );
            assert!(
                summary.chars().count() <= 2200,
                "truncated summary should stay bounded, chars={}",
                summary.chars().count()
            );
        }
        other => panic!("expected FinalText, got {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_engine_keeps_external_skill_invoke_payloads_intact() {
    use crate::conversation::turn_engine::{ProviderTurn, TurnEngine, TurnResult};
    use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest, ToolPlaneError};
    use loongclaw_kernel::CoreToolAdapter;

    struct ExternalSkillInvokeAdapter;

    #[async_trait]
    impl CoreToolAdapter for ExternalSkillInvokeAdapter {
        fn name(&self) -> &str {
            "external-skill-invoke-adapter"
        }

        async fn execute_core_tool(
            &self,
            request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, ToolPlaneError> {
            let (tool_name, _arguments) = effective_tool_request(&request);
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "tool": tool_name,
                    "instructions": "Follow the managed skill instruction. ".repeat(200),
                    "invocation_summary": "Loaded managed external skill instructions."
                }),
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
        granted_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead]),
        metadata: BTreeMap::new(),
    };
    kernel.register_pack(pack).expect("register pack");
    kernel.register_core_tool_adapter(ExternalSkillInvokeAdapter);
    kernel
        .set_default_core_tool_adapter("external-skill-invoke-adapter")
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
        tool_intents: vec![provider_tool_intent(
            "external_skills.invoke",
            json!({"skill_id": "demo-skill"}),
            "s1",
            "t1",
            "c-skill",
        )],
        raw_meta: serde_json::Value::Null,
    };

    let tool_view = crate::tools::runtime_tool_view_for_runtime_config(
        &crate::tools::runtime_config::ToolRuntimeConfig {
            external_skills: crate::tools::runtime_config::ExternalSkillsRuntimePolicy {
                enabled: true,
                ..crate::tools::runtime_config::ExternalSkillsRuntimePolicy::default()
            },
            ..crate::tools::runtime_config::ToolRuntimeConfig::default()
        },
    );
    let result = engine
        .execute_turn_in_view(
            &turn,
            &tool_view,
            super::runtime_binding::ConversationRuntimeBinding::kernel(&ctx),
        )
        .await;
    match result {
        TurnResult::FinalText(text) => {
            let line = text.lines().next().expect("tool result line should exist");
            let payload = line
                .strip_prefix("[ok] ")
                .expect("tool result line should keep [ok] prefix");
            let envelope: Value =
                serde_json::from_str(payload).expect("tool result envelope should be valid json");
            assert_eq!(envelope["tool"], "external_skills.invoke");
            assert_eq!(envelope["payload_truncated"], json!(false));
            assert!(
                envelope["payload_summary"]
                    .as_str()
                    .expect("payload summary should be text")
                    .contains("Follow the managed skill instruction."),
                "payload summary should keep invoke instructions intact: {envelope:?}"
            );
        }
        other @ TurnResult::ToolDenied(_)
        | other @ TurnResult::NeedsApproval(_)
        | other @ TurnResult::ToolError(_)
        | other @ TurnResult::ProviderError(_) => panic!("unexpected result: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_engine_injects_browser_scope_into_kernel_request() {
    use crate::conversation::turn_engine::{ProviderTurn, ToolIntent, TurnEngine, TurnResult};
    use loongclaw_contracts::{ToolCoreOutcome, ToolCoreRequest, ToolPlaneError};
    use loongclaw_kernel::CoreToolAdapter;

    // In the discovery-first model, browser.open is a discoverable Core tool
    // and must be invoked through tool.invoke.  The adapter receives the outer
    // tool.invoke request; we extract the inner arguments to verify browser
    // scope injection.
    struct BrowserScopeEchoAdapter;

    #[async_trait]
    impl CoreToolAdapter for BrowserScopeEchoAdapter {
        fn name(&self) -> &str {
            "browser-scope-echo"
        }

        async fn execute_core_tool(
            &self,
            request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, ToolPlaneError> {
            // tool.invoke: extract browser scope from nested arguments.
            let (tool_name, scope) =
                if crate::tools::canonical_tool_name(&request.tool_name) == "tool.invoke" {
                    let tool_id = request
                        .payload
                        .get("tool_id")
                        .and_then(Value::as_str)
                        .unwrap_or(&request.tool_name)
                        .to_owned();
                    let arguments = request
                        .payload
                        .get("arguments")
                        .cloned()
                        .unwrap_or(Value::Null);
                    let scope = arguments[crate::tools::BROWSER_SESSION_SCOPE_FIELD].clone();
                    (tool_id, scope)
                } else {
                    let scope = request.payload[crate::tools::BROWSER_SESSION_SCOPE_FIELD].clone();
                    (request.tool_name, scope)
                };
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "tool": tool_name,
                    "scope": scope,
                }),
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
    kernel.register_core_tool_adapter(BrowserScopeEchoAdapter);
    kernel
        .set_default_core_tool_adapter("browser-scope-echo")
        .expect("set default");

    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");

    let ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };

    let engine = TurnEngine::new(5);
    let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
        "browser.open",
        json!({"url": "https://example.com"}),
        Some("root-session"),
        Some("t-browser"),
    );
    let turn = ProviderTurn {
        assistant_text: "".to_owned(),
        tool_intents: vec![ToolIntent {
            tool_name,
            args_json,
            source: "provider_tool_call".to_owned(),
            session_id: "root-session".to_owned(),
            turn_id: "t-browser".to_owned(),
            tool_call_id: "c-browser".to_owned(),
        }],
        raw_meta: serde_json::Value::Null,
    };

    let result = engine
        .execute_turn_in_view(
            &turn,
            &crate::tools::ToolView::from_tool_names(["tool.invoke", "browser.open"]),
            super::runtime_binding::ConversationRuntimeBinding::kernel(&ctx),
        )
        .await;

    match result {
        TurnResult::FinalText(text) => {
            let line = text.lines().next().expect("tool result line should exist");
            let payload = line
                .strip_prefix("[ok] ")
                .expect("tool result line should keep [ok] prefix");
            let envelope: Value =
                serde_json::from_str(payload).expect("tool result envelope should be valid json");
            assert_eq!(envelope["tool"], "browser.open");
            assert!(
                envelope["payload_summary"]
                    .as_str()
                    .expect("payload summary should be text")
                    .contains("\"scope\":\"root-session\""),
                "expected injected browser scope in payload summary, got: {envelope:?}"
            );
        }
        other @ (TurnResult::NeedsApproval(_)
        | TurnResult::ToolDenied(_)
        | TurnResult::ToolError(_)
        | TurnResult::ProviderError(_)) => panic!("unexpected result: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_engine_execute_turn_denied_without_capability() {
    use crate::conversation::turn_engine::{ProviderTurn, TurnEngine, TurnResult};
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
        tool_intents: vec![provider_tool_intent(
            "file.read",
            json!({"path": "test.txt"}),
            "s1",
            "t1",
            "c1",
        )],
        raw_meta: serde_json::Value::Null,
    };

    let result = engine.execute_turn(&turn, &ctx).await;
    #[allow(clippy::wildcard_enum_match_arm)]
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

    persist_tool_decision(
        &runtime,
        "sess-1",
        "turn-1",
        "call-1",
        &decision,
        ConversationRuntimeBinding::direct(),
    )
    .await
    .expect("persist decision");

    persist_tool_outcome(
        &runtime,
        "sess-1",
        "turn-1",
        "call-1",
        &outcome,
        ConversationRuntimeBinding::direct(),
    )
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
    build_kernel_context_with_window_turns(
        audit,
        json!([
            {
                "role": "assistant",
                "content": "kernel-memory-window",
                "ts": 1
            }
        ]),
    )
}

fn build_kernel_context_with_window_turns(
    audit: Arc<InMemoryAuditSink>,
    window_turns: Value,
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
        window_turns,
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

#[test]
fn conversation_runtime_binding_direct_reports_no_kernel_context() {
    let binding = crate::conversation::ConversationRuntimeBinding::direct();

    assert!(!binding.is_kernel_bound());
    assert!(binding.kernel_context().is_none());
}

#[test]
fn conversation_runtime_binding_kernel_exposes_bound_context() {
    let (kernel_ctx, _invocations) = build_kernel_context(Arc::new(InMemoryAuditSink::default()));
    let binding = crate::conversation::ConversationRuntimeBinding::kernel(&kernel_ctx);

    assert!(binding.is_kernel_bound());
    assert!(binding.kernel_context().is_some());
}

fn build_kernel_context_with_window_turn_sequence(
    audit: Arc<InMemoryAuditSink>,
    window_turn_sequence: Vec<Value>,
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
    let adapter = SequencedTestMemoryAdapter {
        invocations: invocations.clone(),
        window_turns: Mutex::new(VecDeque::from(window_turn_sequence)),
    };
    kernel.register_core_memory_adapter(adapter);
    kernel
        .set_default_core_memory_adapter("test-memory-sequenced")
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

fn build_kernel_context_with_window_error(
    audit: Arc<InMemoryAuditSink>,
    error: &str,
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
    kernel.register_core_memory_adapter(FailingWindowMemoryAdapter {
        invocations: invocations.clone(),
        error: error.to_owned(),
    });
    kernel
        .set_default_core_memory_adapter("test-memory-failing-window")
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

fn build_kernel_context_with_raw_window_payload(
    audit: Arc<InMemoryAuditSink>,
    payload: Value,
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
    kernel.register_core_memory_adapter(RawWindowPayloadMemoryAdapter {
        invocations: invocations.clone(),
        payload,
    });
    kernel
        .set_default_core_memory_adapter("test-memory-raw-window-payload")
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

#[cfg(feature = "memory-sqlite")]
fn prepare_discovery_first_summary_test(
    db_scope: &str,
    direct_session_id: &str,
    payloads: &[String],
) -> (PathBuf, MemoryRuntimeConfig) {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id(db_scope, "binding")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    for payload in payloads {
        crate::memory::append_turn_direct(direct_session_id, "assistant", payload, &mem_config)
            .expect("persist discovery-first payload");
    }

    (db_path, mem_config)
}

#[cfg(feature = "memory-sqlite")]
fn discovery_first_window_turns(payloads: &[String]) -> Value {
    json!(
        payloads
            .iter()
            .enumerate()
            .map(|(index, payload)| json!({
                "role": "assistant",
                "content": payload,
                "ts": index + 1
            }))
            .collect::<Vec<_>>()
    )
}

struct SharedTestMemoryAdapter {
    invocations: Arc<Mutex<Vec<MemoryCoreRequest>>>,
    window_turns: Value,
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
        let payload = if request.operation == crate::memory::MEMORY_OP_WINDOW {
            json!({
                "turns": self.window_turns.clone()
            })
        } else {
            json!({})
        };
        self.invocations
            .lock()
            .expect("invocations lock")
            .push(request);
        Ok(MemoryCoreOutcome {
            status: "ok".to_owned(),
            payload,
        })
    }
}

struct SequencedTestMemoryAdapter {
    invocations: Arc<Mutex<Vec<MemoryCoreRequest>>>,
    window_turns: Mutex<VecDeque<Value>>,
}

#[async_trait]
impl CoreMemoryAdapter for SequencedTestMemoryAdapter {
    fn name(&self) -> &str {
        "test-memory-sequenced"
    }

    async fn execute_core_memory(
        &self,
        request: MemoryCoreRequest,
    ) -> Result<MemoryCoreOutcome, MemoryPlaneError> {
        let payload = if request.operation == crate::memory::MEMORY_OP_WINDOW {
            let turns = {
                let mut queued_turns = self.window_turns.lock().expect("window turns lock");
                queued_turns.pop_front().unwrap_or_else(|| json!([]))
            };
            json!({
                "turns": turns
            })
        } else {
            json!({})
        };
        self.invocations
            .lock()
            .expect("invocations lock")
            .push(request);
        Ok(MemoryCoreOutcome {
            status: "ok".to_owned(),
            payload,
        })
    }
}

struct FailingWindowMemoryAdapter {
    invocations: Arc<Mutex<Vec<MemoryCoreRequest>>>,
    error: String,
}

#[async_trait]
impl CoreMemoryAdapter for FailingWindowMemoryAdapter {
    fn name(&self) -> &str {
        "test-memory-failing-window"
    }

    async fn execute_core_memory(
        &self,
        request: MemoryCoreRequest,
    ) -> Result<MemoryCoreOutcome, MemoryPlaneError> {
        self.invocations
            .lock()
            .expect("invocations lock")
            .push(request.clone());
        if request.operation == crate::memory::MEMORY_OP_WINDOW {
            return Err(MemoryPlaneError::Execution(self.error.clone()));
        }
        Ok(MemoryCoreOutcome {
            status: "ok".to_owned(),
            payload: json!({}),
        })
    }
}

struct RawWindowPayloadMemoryAdapter {
    invocations: Arc<Mutex<Vec<MemoryCoreRequest>>>,
    payload: Value,
}

#[async_trait]
impl CoreMemoryAdapter for RawWindowPayloadMemoryAdapter {
    fn name(&self) -> &str {
        "test-memory-raw-window-payload"
    }

    async fn execute_core_memory(
        &self,
        request: MemoryCoreRequest,
    ) -> Result<MemoryCoreOutcome, MemoryPlaneError> {
        self.invocations
            .lock()
            .expect("invocations lock")
            .push(request.clone());
        if request.operation == crate::memory::MEMORY_OP_WINDOW {
            return Ok(MemoryCoreOutcome {
                status: "ok".to_owned(),
                payload: self.payload.clone(),
            });
        }
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
    let binding = crate::conversation::ConversationRuntimeBinding::kernel(&ctx);

    let runtime = DefaultConversationRuntime::default();
    runtime
        .persist_turn("session-k1", "user", "kernel-hello", binding)
        .await
        .expect("persist via kernel");

    // Verify the memory adapter received the request.
    let captured = invocations.lock().expect("invocations lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_APPEND_TURN);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn build_messages_routes_memory_window_through_kernel_when_context_provided() {
    let audit = Arc::new(InMemoryAuditSink::default());
    let (ctx, invocations) = build_kernel_context(audit.clone());
    let runtime = DefaultConversationRuntime::default();
    let config = test_config();
    let binding = crate::conversation::ConversationRuntimeBinding::kernel(&ctx);
    let tool_view = runtime
        .tool_view(&config, "session-k-window", binding)
        .expect("kernel window tool view");
    let messages = runtime
        .build_messages(&config, "session-k-window", true, &tool_view, binding)
        .await
        .expect("build messages via kernel");

    assert!(
        !messages.is_empty(),
        "expected at least system prompt message, got: {messages:?}"
    );
    assert_eq!(messages[0]["role"], "system");
    assert!(
        messages
            .iter()
            .any(|message| message["content"] == "kernel-memory-window"),
        "messages should include history loaded from kernel window payload"
    );

    let captured = invocations.lock().expect("invocations lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);
    assert_eq!(captured[0].payload["session_id"], "session-k-window");
    assert_eq!(
        captured[0].payload["limit"],
        json!(config.memory.sliding_window)
    );

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn load_turn_checkpoint_event_summary_prefers_kernel_memory_window_when_context_provided() {
    let checkpoint_turns = json!([
        {
            "role": "assistant",
            "content": json!({
                "type": "conversation_event",
                "event": "turn_checkpoint",
                "payload": {
                    "schema_version": 1,
                    "stage": "post_persist",
                    "checkpoint": {
                        "lane": {
                            "lane": "safe",
                            "result_kind": "tool_call"
                        },
                        "finalization": {
                            "persistence_mode": "success"
                        }
                    },
                    "finalization_progress": {
                        "after_turn": "pending",
                        "compaction": "pending"
                    },
                    "failure": null
                }
            })
            .to_string(),
            "ts": 1
        },
        {
            "role": "assistant",
            "content": json!({
                "type": "conversation_event",
                "event": "turn_checkpoint",
                "payload": {
                    "schema_version": 1,
                    "stage": "finalized",
                    "checkpoint": {
                        "lane": {
                            "lane": "safe",
                            "result_kind": "tool_call"
                        },
                        "finalization": {
                            "persistence_mode": "success"
                        }
                    },
                    "finalization_progress": {
                        "after_turn": "completed",
                        "compaction": "skipped"
                    },
                    "failure": null
                }
            })
            .to_string(),
            "ts": 2
        }
    ]);
    let audit = Arc::new(InMemoryAuditSink::default());
    let (ctx, invocations) = build_kernel_context_with_window_turns(audit, checkpoint_turns);
    let config = test_config();
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    let summary = load_turn_checkpoint_event_summary(
        "session-k-turn-checkpoint",
        96,
        ConversationRuntimeBinding::kernel(&ctx),
        &mem_config,
    )
    .await
    .expect("load checkpoint summary via kernel");

    assert_eq!(summary.checkpoint_events, 2);
    assert_eq!(summary.session_state, TurnCheckpointSessionState::Finalized);
    assert!(summary.checkpoint_durable);
    assert_eq!(summary.latest_stage, Some(TurnCheckpointStage::Finalized));
    assert_eq!(
        summary.latest_after_turn,
        Some(TurnCheckpointProgressStatus::Completed)
    );
    assert_eq!(
        summary.latest_compaction,
        Some(TurnCheckpointProgressStatus::Skipped)
    );
    assert!(!summary.requires_recovery);
    assert!(summary.reply_durable);

    let captured = invocations.lock().expect("invocations lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);
    assert_eq!(
        captured[0].payload["session_id"],
        "session-k-turn-checkpoint"
    );
    assert_eq!(captured[0].payload["limit"], json!(96));
    assert_eq!(captured[0].payload["allow_extended_limit"], json!(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn load_turn_checkpoint_event_summary_fails_closed_when_kernel_window_errors() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "kernel-window-error")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 8;
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(
        "session-kernel-window-error",
        "assistant",
        r#"{"type":"conversation_event","event":"turn_checkpoint","payload":{"schema_version":1,"stage":"finalized","checkpoint":{"lane":{"lane":"safe","result_kind":"tool_call"},"finalization":{"persistence_mode":"success"}},"finalization_progress":{"after_turn":"completed","compaction":"skipped"},"failure":null}}"#,
        &mem_config,
    )
    .expect("seed sqlite fallback history");

    let audit = Arc::new(InMemoryAuditSink::default());
    let (ctx, invocations) =
        build_kernel_context_with_window_error(audit, "forced kernel window failure");

    let error = load_turn_checkpoint_event_summary(
        "session-kernel-window-error",
        8,
        ConversationRuntimeBinding::kernel(&ctx),
        &mem_config,
    )
    .await
    .expect_err("kernel-bound history should fail closed");

    assert!(
        error.contains("load assistant history via kernel failed"),
        "unexpected error: {error}"
    );

    let captured = invocations.lock().expect("invocations lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn load_turn_checkpoint_event_summary_fails_closed_when_kernel_window_payload_is_malformed() {
    let audit = Arc::new(InMemoryAuditSink::default());
    let (ctx, invocations) =
        build_kernel_context_with_raw_window_payload(audit, json!({"unexpected": "shape"}));
    let config = test_config();
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    let error = load_turn_checkpoint_event_summary(
        "session-kernel-window-malformed",
        8,
        ConversationRuntimeBinding::kernel(&ctx),
        &mem_config,
    )
    .await
    .expect_err("kernel-bound history should fail closed on malformed payload");

    assert!(error.contains("malformed"), "unexpected error: {error}");

    let captured = invocations.lock().expect("invocations lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn load_turn_checkpoint_event_summary_fails_closed_when_kernel_window_assistant_content_is_malformed()
 {
    let audit = Arc::new(InMemoryAuditSink::default());
    let (ctx, invocations) = build_kernel_context_with_raw_window_payload(
        audit,
        json!({
            "turns": [
                {
                    "role": "assistant",
                    "content": {
                        "unexpected": "shape"
                    }
                }
            ]
        }),
    );
    let config = test_config();
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    let error = load_turn_checkpoint_event_summary(
        "session-kernel-window-malformed-assistant-content",
        8,
        ConversationRuntimeBinding::kernel(&ctx),
        &mem_config,
    )
    .await
    .expect_err("kernel-bound history should fail closed on malformed assistant content");

    assert!(error.contains("malformed"), "unexpected error: {error}");

    let captured = invocations.lock().expect("invocations lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn load_turn_checkpoint_event_summary_direct_read_failure_uses_neutral_error_message() {
    let sqlite_dir = std::env::temp_dir().join(unique_acp_test_id(
        "conversation-turn-checkpoint",
        "direct-read-error",
    ));
    let _ = std::fs::remove_dir_all(&sqlite_dir);
    std::fs::create_dir_all(&sqlite_dir).expect("create sqlite placeholder directory");

    let mut config = test_config();
    config.memory.sqlite_path = sqlite_dir.display().to_string();
    config.memory.sliding_window = 8;
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    let error = load_turn_checkpoint_event_summary(
        "session-direct-read-error",
        8,
        ConversationRuntimeBinding::direct(),
        &mem_config,
    )
    .await
    .expect_err("direct history load should fail when sqlite path is a directory");

    assert!(
        error.contains("direct read failed"),
        "unexpected error: {error}"
    );
    assert!(
        !error.contains("load turn checkpoint summary failed"),
        "error should use context-agnostic wording: {error}"
    );

    let _ = std::fs::remove_dir_all(&sqlite_dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn load_discovery_first_event_summary_accepts_explicit_runtime_binding() {
    let payloads = [
        json!({
            "type": "conversation_event",
            "event": "discovery_first_search_round",
            "payload": {
                "provider_round": 0,
                "search_tool_calls": 1,
                "raw_tool_output_requested": false,
                "initial_estimated_tokens": 12
            }
        })
        .to_string(),
        json!({
            "type": "conversation_event",
            "event": "discovery_first_followup_requested",
            "payload": {
                "provider_round": 1,
                "raw_tool_output_requested": true,
                "initial_estimated_tokens": 12,
                "followup_estimated_tokens": 21,
                "followup_added_estimated_tokens": 9
            }
        })
        .to_string(),
        json!({
            "type": "conversation_event",
            "event": "discovery_first_followup_result",
            "payload": {
                "provider_round": 1,
                "outcome": "tool.invoke",
                "followup_tool_name": "tool.invoke",
                "followup_target_tool_id": "file.read",
                "resolved_to_tool_invoke": true,
                "raw_tool_output_requested": true
            }
        })
        .to_string(),
    ];

    let (db_path, mem_config) = prepare_discovery_first_summary_test(
        "conversation-discovery-first",
        "session-discovery-first-direct",
        &payloads,
    );

    let direct_summary = super::session_history::load_discovery_first_event_summary_with_binding(
        "session-discovery-first-direct",
        32,
        ConversationRuntimeBinding::direct(),
        &mem_config,
    )
    .await
    .expect("load discovery-first summary via direct binding");
    assert_eq!(direct_summary.search_round_events, 1);
    assert_eq!(direct_summary.followup_requested_events, 1);
    assert_eq!(direct_summary.followup_result_events, 1);
    assert_eq!(
        direct_summary.latest_followup_target_tool_id.as_deref(),
        Some("file.read")
    );

    let audit = Arc::new(InMemoryAuditSink::default());
    let (kernel_ctx, invocations) =
        build_kernel_context_with_window_turns(audit, discovery_first_window_turns(&payloads));

    let kernel_summary = super::session_history::load_discovery_first_event_summary_with_binding(
        "session-discovery-first-kernel",
        48,
        ConversationRuntimeBinding::kernel(&kernel_ctx),
        &mem_config,
    )
    .await
    .expect("load discovery-first summary via kernel binding");
    assert_eq!(kernel_summary.search_round_events, 1);
    assert_eq!(kernel_summary.followup_requested_events, 1);
    assert_eq!(kernel_summary.followup_result_events, 1);
    assert_eq!(
        kernel_summary.latest_followup_target_tool_id.as_deref(),
        Some("file.read")
    );

    let captured = invocations.lock().expect("invocations lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);
    assert_eq!(
        captured[0].payload["session_id"],
        "session-discovery-first-kernel"
    );
    assert_eq!(captured[0].payload["limit"], json!(48));
    assert_eq!(captured[0].payload["allow_extended_limit"], json!(true));

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn load_discovery_first_event_summary_preserves_public_kernel_context_signature() {
    let payloads = [
        json!({
            "type": "conversation_event",
            "event": "discovery_first_search_round",
            "payload": {
                "provider_round": 0,
                "search_tool_calls": 2,
                "raw_tool_output_requested": false,
                "initial_estimated_tokens": 8
            }
        })
        .to_string(),
        json!({
            "type": "conversation_event",
            "event": "discovery_first_followup_result",
            "payload": {
                "provider_round": 0,
                "outcome": "tool.invoke",
                "followup_tool_name": "tool.invoke",
                "followup_target_tool_id": "file.read",
                "resolved_to_tool_invoke": true,
                "raw_tool_output_requested": false
            }
        })
        .to_string(),
    ];

    let (db_path, mem_config) = prepare_discovery_first_summary_test(
        "conversation-discovery-first-compat",
        "session-discovery-first-compat-direct",
        &payloads,
    );

    let direct_summary = load_discovery_first_event_summary(
        "session-discovery-first-compat-direct",
        16,
        None,
        &mem_config,
    )
    .await
    .expect("load discovery-first summary via legacy direct signature");
    assert_eq!(direct_summary.search_round_events, 1);
    assert_eq!(direct_summary.followup_result_events, 1);
    assert_eq!(
        direct_summary.latest_followup_target_tool_id.as_deref(),
        Some("file.read")
    );

    let audit = Arc::new(InMemoryAuditSink::default());
    let (kernel_ctx, invocations) =
        build_kernel_context_with_window_turns(audit, discovery_first_window_turns(&payloads));

    let kernel_summary = load_discovery_first_event_summary(
        "session-discovery-first-compat-kernel",
        24,
        Some(&kernel_ctx),
        &mem_config,
    )
    .await
    .expect("load discovery-first summary via legacy kernel signature");
    assert_eq!(kernel_summary.search_round_events, 1);
    assert_eq!(kernel_summary.followup_result_events, 1);
    assert_eq!(
        kernel_summary.latest_followup_target_tool_id.as_deref(),
        Some("file.read")
    );

    let captured = invocations.lock().expect("invocations lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);
    assert_eq!(
        captured[0].payload["session_id"],
        "session-discovery-first-compat-kernel"
    );
    assert_eq!(captured[0].payload["limit"], json!(24));
    assert_eq!(captured[0].payload["allow_extended_limit"], json!(true));

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(not(feature = "memory-sqlite"))]
#[tokio::test]
async fn persist_turn_without_memory_sqlite_is_noop_with_kernel_context() {
    let ctx = crate::context::bootstrap_kernel_context("test-agent-no-memory", 60)
        .expect("bootstrap kernel context without memory-sqlite");
    let runtime = DefaultConversationRuntime::default();
    runtime
        .persist_turn(
            "session-k0",
            "user",
            "no-memory",
            ConversationRuntimeBinding::kernel(&ctx),
        )
        .await
        .expect("persist should be no-op when memory-sqlite is disabled");
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn persisted_turn_checkpoint_events_survive_reload_without_polluting_prompt_history() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "reload")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 16;

    let runtime = DefaultConversationRuntime::default();
    let session_id = "session-turn-checkpoint-reload";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "post_persist",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success"
                    }
                },
                "finalization_progress": {
                    "after_turn": "pending",
                    "compaction": "pending"
                },
                "failure": null
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist post_persist checkpoint");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "finalized",
                "checkpoint": {
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success"
                    }
                },
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": "skipped"
                },
                "failure": null
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist finalized checkpoint");

    let messages = runtime
        .build_messages(
            &config,
            session_id,
            true,
            &crate::tools::runtime_tool_view_for_config(&config.tools),
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("reload prompt history");
    assert!(
        messages.iter().any(
            |message| message["role"] == "assistant" && message["content"] == "assistant-reply"
        ),
        "assistant reply should survive reload: {messages:?}"
    );
    assert!(
        !messages.iter().any(|message| {
            message["content"]
                .as_str()
                .map(|content| content.contains("\"event\":\"turn_checkpoint\""))
                .unwrap_or(false)
        }),
        "checkpoint events must not pollute provider prompt history: {messages:?}"
    );

    let turns = crate::memory::window_direct(session_id, 16, &mem_config)
        .expect("load raw turns from sqlite");
    let assistant_contents = turns
        .iter()
        .filter_map(|turn| (turn.role == "assistant").then_some(turn.content.as_str()))
        .collect::<Vec<_>>();
    let summary = summarize_turn_checkpoint_events(assistant_contents.iter().copied());
    assert_eq!(summary.checkpoint_events, 2);
    assert_eq!(summary.latest_stage, Some(TurnCheckpointStage::Finalized));
    assert_eq!(
        summary.latest_after_turn,
        Some(TurnCheckpointProgressStatus::Completed)
    );
    assert_eq!(
        summary.latest_compaction,
        Some(TurnCheckpointProgressStatus::Skipped)
    );
    assert_eq!(summary.latest_lane.as_deref(), Some("fast"));
    assert_eq!(summary.latest_result_kind.as_deref(), Some("final_text"));
    assert_eq!(summary.session_state, TurnCheckpointSessionState::Finalized);
    assert!(summary.checkpoint_durable);
    assert!(summary.reply_durable);
    assert!(!summary.requires_recovery);

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn load_turn_checkpoint_event_summary_reads_recovery_state_from_sqlite_history() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "reader")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 8;

    let session_id = "session-turn-checkpoint-reader";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "post_persist",
                "checkpoint": {
                    "lane": {
                        "lane": "safe",
                        "result_kind": "tool_call"
                    },
                    "finalization": {
                        "persistence_mode": "error"
                    }
                },
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": "pending"
                },
                "failure": null
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist post_persist checkpoint");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "finalization_failed",
                "checkpoint": {
                    "lane": {
                        "lane": "safe",
                        "result_kind": "tool_call"
                    },
                    "finalization": {
                        "persistence_mode": "error"
                    }
                },
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": "failed"
                },
                "failure": {
                    "step": "compaction",
                    "error": "context compaction failed"
                }
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist failed checkpoint");

    let summary = load_turn_checkpoint_event_summary(
        session_id,
        32,
        ConversationRuntimeBinding::direct(),
        &mem_config,
    )
    .await
    .expect("load checkpoint event summary");

    assert_eq!(summary.checkpoint_events, 2);
    assert_eq!(
        summary.session_state,
        TurnCheckpointSessionState::FinalizationFailed
    );
    assert!(summary.checkpoint_durable);
    assert_eq!(
        summary.latest_stage,
        Some(TurnCheckpointStage::FinalizationFailed)
    );
    assert_eq!(
        summary.latest_after_turn,
        Some(TurnCheckpointProgressStatus::Completed)
    );
    assert_eq!(
        summary.latest_compaction,
        Some(TurnCheckpointProgressStatus::Failed)
    );
    assert_eq!(
        summary.latest_failure_step,
        Some(TurnCheckpointFailureStep::Compaction)
    );
    assert_eq!(
        summary.latest_failure_error.as_deref(),
        Some("context compaction failed")
    );
    assert_eq!(summary.latest_lane.as_deref(), Some("safe"));
    assert_eq!(summary.latest_result_kind.as_deref(), Some("tool_call"));
    assert_eq!(summary.latest_persistence_mode.as_deref(), Some("error"));
    assert!(summary.reply_durable);
    assert!(summary.requires_recovery);

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn repair_turn_checkpoint_tail_with_runtime_finalizes_pending_checkpoint() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "repair-pending")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);
    config.conversation.compact_fail_open = false;

    let session_id = "session-turn-checkpoint-repair-pending";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "post_persist",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "pending",
                    "compaction": "pending"
                },
                "failure": null
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist post_persist checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "assistant-reply"}),
        ],
        vec![],
        vec![],
    );
    let coordinator = ConversationTurnCoordinator::new();
    let kernel_ctx =
        test_kernel_context_with_memory("test-turn-checkpoint-repair-pending", &mem_config);

    let outcome = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect("repair pending checkpoint");

    assert_eq!(outcome.status().as_str(), "repaired");
    assert_eq!(
        outcome.source().map(|source| source.as_str()),
        Some("runtime")
    );
    assert_eq!(outcome.action().as_str(), "run_after_turn_and_compaction");
    assert_eq!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        1
    );
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 1);

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert_eq!(payloads.len(), 1, "expected one repair checkpoint event");
    assert_eq!(payloads[0]["stage"], "finalized");
    assert_eq!(
        payloads[0]["finalization_progress"]["after_turn"],
        "completed"
    );
    assert_eq!(
        payloads[0]["finalization_progress"]["compaction"],
        "completed"
    );

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn repair_turn_checkpoint_tail_with_runtime_requires_manual_repair_without_identity() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "repair-missing-identity")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);

    let session_id = "session-turn-checkpoint-repair-missing-identity";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "post_persist",
                "checkpoint": {
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "pending",
                    "compaction": "pending"
                },
                "failure": null
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist post_persist checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "assistant-reply"}),
        ],
        vec![],
        vec![],
    );
    let coordinator = ConversationTurnCoordinator::new();

    let outcome = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("repair should fail closed when identity is missing");

    assert_eq!(outcome.status().as_str(), "manual_required");
    assert_eq!(
        outcome.source().map(|source| source.as_str()),
        Some("summary")
    );
    assert_eq!(outcome.action().as_str(), "inspect_manually");
    assert_eq!(
        outcome.reason(),
        TurnCheckpointTailRepairReason::CheckpointIdentityMissing
    );
    assert_eq!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        0
    );
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 0);
    let persisted = runtime.persisted.lock().expect("persisted lock");
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert!(
        payloads.is_empty(),
        "manual downgrade should not persist a new checkpoint event"
    );

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn repair_turn_checkpoint_tail_with_runtime_preserves_safe_lane_override_reason_when_tail_is_not_runnable()
 {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id(
            "conversation-turn-checkpoint",
            "repair-safe-lane-override-manual-reason"
        )
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;

    let session_id = "session-turn-checkpoint-repair-safe-lane-override-manual-reason";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "post_persist",
                "checkpoint": {
                    "identity": {
                        "user_input_sha256": "u1",
                        "assistant_reply_sha256": "a1",
                        "user_input_chars": 5,
                        "assistant_reply_chars": 15
                    },
                    "lane": {
                        "lane": "safe",
                        "result_kind": "tool_error",
                        "safe_lane_terminal_route": {
                            "decision": "terminal",
                            "reason": "backpressure_attempts_exhausted",
                            "source": "backpressure_guard"
                        }
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": false,
                        "attempts_context_compaction": false
                    }
                },
                "finalization_progress": {
                    "after_turn": "skipped",
                    "compaction": "skipped"
                },
                "failure": null
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist post_persist checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "assistant-reply"}),
        ],
        vec![],
        vec![],
    );
    let coordinator = ConversationTurnCoordinator::new();

    let outcome = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("repair should downgrade to manual inspection");

    assert_eq!(outcome.status().as_str(), "manual_required");
    assert_eq!(outcome.action().as_str(), "inspect_manually");
    assert_eq!(
        outcome.reason().as_str(),
        "safe_lane_backpressure_terminal_requires_manual_inspection"
    );
    assert_eq!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        0
    );
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 0);

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn repair_turn_checkpoint_tail_with_runtime_requires_manual_repair_on_identity_mismatch() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "repair-identity-mismatch")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);

    let session_id = "session-turn-checkpoint-repair-identity-mismatch";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "post_persist",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "pending",
                    "compaction": "pending"
                },
                "failure": null
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist post_persist checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "assistant-reply-mutated"}),
        ],
        vec![],
        vec![],
    );
    let coordinator = ConversationTurnCoordinator::new();

    let outcome = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("repair should fail closed on mismatched visible tail");

    assert_eq!(outcome.status().as_str(), "manual_required");
    assert_eq!(outcome.action().as_str(), "inspect_manually");
    assert_eq!(
        outcome.reason(),
        TurnCheckpointTailRepairReason::CheckpointIdentityMismatch
    );
    assert_eq!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        0
    );
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 0);
    let persisted = runtime.persisted.lock().expect("persisted lock");
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert!(
        payloads.is_empty(),
        "mismatch downgrade should not persist a new checkpoint event"
    );

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn repair_turn_checkpoint_tail_with_runtime_retries_failed_compaction_only() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "repair-compaction")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);

    let session_id = "session-turn-checkpoint-repair-compaction";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "finalization_failed",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": "failed"
                },
                "failure": {
                    "step": "compaction",
                    "error": "compact failed"
                }
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist failed checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "assistant-reply"}),
        ],
        vec![],
        vec![],
    );
    let coordinator = ConversationTurnCoordinator::new();
    let kernel_ctx =
        test_kernel_context_with_memory("test-turn-checkpoint-repair-compaction", &mem_config);

    let outcome = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect("repair failed compaction checkpoint");

    assert_eq!(outcome.status().as_str(), "repaired");
    assert_eq!(outcome.action().as_str(), "run_compaction");
    assert_eq!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        0
    );
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 1);

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert_eq!(payloads.len(), 1, "expected one repair checkpoint event");
    assert_eq!(payloads[0]["stage"], "finalized");
    assert_eq!(
        payloads[0]["finalization_progress"]["after_turn"],
        "completed"
    );
    assert_eq!(
        payloads[0]["finalization_progress"]["compaction"],
        "completed"
    );

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn repair_turn_checkpoint_tail_rebuilds_original_finalization_context_for_compaction_retry() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "repair-compaction-context")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(3);
    config.conversation.compact_trigger_estimated_tokens = None;

    let session_id = "session-turn-checkpoint-repair-compaction-context";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "finalization_failed",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": "failed"
                },
                "failure": {
                    "step": "compaction",
                    "error": "compact failed"
                }
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist failed checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(vec![], vec![], vec![])
        .with_assembled_context_variants(
            AssembledConversationContext {
                messages: vec![
                    json!({"role": "system", "content": "sys"}),
                    json!({"role": "user", "content": "hello"}),
                    json!({"role": "assistant", "content": "assistant-reply"}),
                ],
                estimated_tokens: Some(3),
                system_prompt_addition: None,
            },
            AssembledConversationContext {
                messages: vec![
                    json!({"role": "user", "content": "hello"}),
                    json!({"role": "assistant", "content": "assistant-reply"}),
                ],
                estimated_tokens: Some(2),
                system_prompt_addition: None,
            },
        );
    let coordinator = ConversationTurnCoordinator::new();
    let kernel_ctx = test_kernel_context_with_memory(
        "test-turn-checkpoint-repair-compaction-context",
        &mem_config,
    );

    let outcome = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect("repair should replay compaction against original finalization context");

    assert_eq!(outcome.status().as_str(), "repaired");
    assert_eq!(outcome.action().as_str(), "run_compaction");
    assert_eq!(
        runtime
            .build_context_calls
            .lock()
            .expect("build context lock")
            .clone(),
        vec![(session_id.to_owned(), true)]
    );
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 1);

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert_eq!(payloads.len(), 1, "expected one repair checkpoint event");
    assert_eq!(payloads[0]["stage"], "finalized");
    assert_eq!(
        payloads[0]["finalization_progress"]["compaction"],
        "completed"
    );

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn repair_turn_checkpoint_tail_prefers_checkpoint_estimate_for_compaction_retry() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "repair-compaction-estimate")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(999);
    config.conversation.compact_trigger_estimated_tokens = Some(50);

    let session_id = "session-turn-checkpoint-repair-compaction-estimate";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "finalization_failed",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "preparation": {
                        "estimated_tokens": 60
                    },
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": "failed"
                },
                "failure": {
                    "step": "compaction",
                    "error": "compact failed"
                }
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist failed checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(vec![], vec![], vec![])
        .with_assembled_context(AssembledConversationContext {
            messages: vec![
                json!({"role": "system", "content": "sys"}),
                json!({"role": "user", "content": "hello"}),
                json!({"role": "assistant", "content": "assistant-reply"}),
            ],
            estimated_tokens: Some(1),
            system_prompt_addition: None,
        });
    let coordinator = ConversationTurnCoordinator::new();
    let kernel_ctx = test_kernel_context_with_memory(
        "test-turn-checkpoint-repair-compaction-estimate",
        &mem_config,
    );

    let outcome = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect("repair should reuse checkpoint estimate for compaction retry");

    assert_eq!(outcome.status().as_str(), "repaired");
    assert_eq!(outcome.action().as_str(), "run_compaction");
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 1);

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert_eq!(payloads.len(), 1, "expected one repair checkpoint event");
    assert_eq!(payloads[0]["stage"], "finalized");
    assert_eq!(
        payloads[0]["finalization_progress"]["compaction"],
        "completed"
    );

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn probe_turn_checkpoint_tail_runtime_gate_reports_preparation_content_mismatch() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id(
            "conversation-turn-checkpoint",
            "probe-context-fingerprint-mismatch"
        )
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);

    let session_id = "session-turn-checkpoint-probe-context-fingerprint-mismatch";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "finalization_failed",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "preparation": {
                        "context_message_count": 2,
                        "context_fingerprint_sha256": test_turn_preparation_context_fingerprint(&[
                            json!({"role": "system", "content": "sys"}),
                            json!({"role": "user", "content": "hello"}),
                        ]),
                        "estimated_tokens": 16
                    },
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": "failed"
                },
                "failure": {
                    "step": "compaction",
                    "error": "compact failed"
                }
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist failed checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(vec![], vec![], vec![])
        .with_assembled_context(AssembledConversationContext {
            messages: vec![
                json!({"role": "system", "content": "summary drift"}),
                json!({"role": "user", "content": "hello"}),
                json!({"role": "assistant", "content": "assistant-reply"}),
            ],
            estimated_tokens: Some(99),
            system_prompt_addition: None,
        });
    let coordinator = ConversationTurnCoordinator::new();

    let probe = coordinator
        .probe_turn_checkpoint_tail_runtime_gate_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("runtime probe should succeed")
        .expect("fingerprint drift should produce a runtime probe");

    assert_eq!(probe.action().as_str(), "inspect_manually");
    assert_eq!(probe.source().as_str(), "runtime");
    assert_eq!(
        probe.reason().as_str(),
        "checkpoint_preparation_fingerprint_mismatch"
    );
    assert_eq!(
        runtime
            .build_context_calls
            .lock()
            .expect("build context lock")
            .as_slice(),
        &[(session_id.to_owned(), true)]
    );
    assert_eq!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        0
    );
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 0);

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn probe_turn_checkpoint_tail_runtime_gate_returns_none_when_repair_not_needed() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "probe-not-needed")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;

    let session_id = "session-turn-checkpoint-probe-not-needed";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "finalized",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": "completed"
                },
                "failure": null
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist finalized checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "assistant-reply"}),
        ],
        vec![],
        vec![],
    );
    let coordinator = ConversationTurnCoordinator::new();

    let probe = coordinator
        .probe_turn_checkpoint_tail_runtime_gate_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("not-needed probe should succeed");

    assert!(probe.is_none());
    assert!(
        runtime
            .build_context_calls
            .lock()
            .expect("build context lock")
            .is_empty()
    );
    assert!(runtime.persisted.lock().expect("persisted lock").is_empty());
    assert_eq!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        0
    );
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 0);

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn probe_turn_checkpoint_tail_runtime_gate_returns_none_for_summary_manual_repair() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "probe-summary-manual")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;

    let session_id = "session-turn-checkpoint-probe-summary-manual";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "post_persist",
                "checkpoint": {
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "pending",
                    "compaction": "pending"
                },
                "failure": null
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist summary-manual checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "assistant-reply"}),
        ],
        vec![],
        vec![],
    );
    let coordinator = ConversationTurnCoordinator::new();

    let probe = coordinator
        .probe_turn_checkpoint_tail_runtime_gate_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("summary-manual probe should succeed");

    assert!(probe.is_none());
    assert!(
        runtime
            .build_context_calls
            .lock()
            .expect("build context lock")
            .is_empty(),
        "summary-derived manual downgrade must stop before runtime context assembly"
    );
    assert!(runtime.persisted.lock().expect("persisted lock").is_empty());
    assert_eq!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        0
    );
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 0);

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn probe_turn_checkpoint_tail_runtime_gate_returns_none_for_runnable_repair() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "probe-runnable")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;

    let session_id = "session-turn-checkpoint-probe-runnable";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "post_persist",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "pending",
                    "compaction": "pending"
                },
                "failure": null
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist runnable checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "assistant-reply"}),
        ],
        vec![],
        vec![],
    );
    let coordinator = ConversationTurnCoordinator::new();

    let probe = coordinator
        .probe_turn_checkpoint_tail_runtime_gate_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("runnable probe should succeed");

    assert!(probe.is_none());
    assert_eq!(
        runtime
            .build_context_calls
            .lock()
            .expect("build context lock")
            .as_slice(),
        &[(session_id.to_owned(), true)],
        "runnable repair should validate runtime context but remain read-only"
    );
    assert!(runtime.persisted.lock().expect("persisted lock").is_empty());
    assert_eq!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        0
    );
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 0);

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn load_turn_checkpoint_diagnostics_with_runtime_preserves_summary_manual_assessment() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "diagnostics-summary-manual")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;

    let session_id = "session-turn-checkpoint-diagnostics-summary-manual";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "post_persist",
                "checkpoint": {
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "pending",
                    "compaction": "pending"
                },
                "failure": null
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist summary-manual checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "assistant-reply"}),
        ],
        vec![],
        vec![],
    );
    let coordinator = ConversationTurnCoordinator::new();

    let diagnostics = coordinator
        .load_turn_checkpoint_diagnostics_with_runtime_and_limit(
            &config,
            session_id,
            12,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("diagnostics should load");

    assert_eq!(
        diagnostics.summary().session_state,
        TurnCheckpointSessionState::PendingFinalization
    );
    assert_eq!(
        diagnostics.recovery().action(),
        TurnCheckpointRecoveryAction::InspectManually
    );
    assert_eq!(
        diagnostics.recovery().source(),
        TurnCheckpointTailRepairSource::Summary
    );
    assert_eq!(
        diagnostics.recovery().reason(),
        Some(TurnCheckpointTailRepairReason::CheckpointIdentityMissing)
    );
    assert!(diagnostics.runtime_probe().is_none());
    assert!(
        runtime
            .build_context_calls
            .lock()
            .expect("build context lock")
            .is_empty(),
        "summary-derived manual assessment must not assemble runtime context"
    );

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn load_turn_checkpoint_diagnostics_with_runtime_preserves_summary_assessment_and_runtime_probe()
 {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "diagnostics-runtime-drift")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);

    let session_id = "session-turn-checkpoint-diagnostics-runtime-drift";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "finalization_failed",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "preparation": {
                        "context_message_count": 2,
                        "context_fingerprint_sha256": test_turn_preparation_context_fingerprint(&[
                            json!({"role": "system", "content": "sys"}),
                            json!({"role": "user", "content": "hello"}),
                        ]),
                        "estimated_tokens": 16
                    },
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": "failed"
                },
                "failure": {
                    "step": "compaction",
                    "error": "compact failed"
                }
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist failed checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(vec![], vec![], vec![])
        .with_assembled_context(AssembledConversationContext {
            messages: vec![
                json!({"role": "system", "content": "summary drift"}),
                json!({"role": "user", "content": "hello"}),
                json!({"role": "assistant", "content": "assistant-reply"}),
            ],
            estimated_tokens: Some(99),
            system_prompt_addition: None,
        });
    let coordinator = ConversationTurnCoordinator::new();

    let diagnostics = coordinator
        .load_turn_checkpoint_diagnostics_with_runtime_and_limit(
            &config,
            session_id,
            12,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("diagnostics should load");

    assert_eq!(
        diagnostics.summary().session_state,
        TurnCheckpointSessionState::FinalizationFailed
    );
    assert_eq!(
        diagnostics.recovery().action(),
        TurnCheckpointRecoveryAction::RunCompaction
    );
    assert_eq!(
        diagnostics.recovery().source(),
        TurnCheckpointTailRepairSource::Summary
    );
    assert_eq!(diagnostics.recovery().reason(), None);

    let runtime_probe = diagnostics
        .runtime_probe()
        .expect("runtime drift should surface a probe");
    assert_eq!(runtime_probe.action().as_str(), "inspect_manually");
    assert_eq!(runtime_probe.source().as_str(), "runtime");
    assert_eq!(
        runtime_probe.reason(),
        TurnCheckpointTailRepairReason::CheckpointPreparationFingerprintMismatch
    );
    assert_eq!(
        runtime
            .build_context_calls
            .lock()
            .expect("build context lock")
            .as_slice(),
        &[(session_id.to_owned(), true)]
    );

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn load_turn_checkpoint_diagnostics_uses_single_kernel_window_snapshot_for_summary_and_runtime_probe()
 {
    let first_window_turns = json!([
        {
            "role": "assistant",
            "content": json!({
                "type": "conversation_event",
                "event": "turn_checkpoint",
                "payload": {
                    "schema_version": 1,
                    "stage": "finalization_failed",
                    "checkpoint": {
                        "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                        "preparation": {
                            "context_message_count": 2,
                            "context_fingerprint_sha256": test_turn_preparation_context_fingerprint(&[
                                json!({"role": "system", "content": "different-system"}),
                                json!({"role": "user", "content": "hello"}),
                            ]),
                            "estimated_tokens": 16
                        },
                        "lane": {
                            "lane": "fast",
                            "result_kind": "final_text"
                        },
                        "finalization": {
                            "persistence_mode": "success",
                            "runs_after_turn": true,
                            "attempts_context_compaction": true
                        }
                    },
                    "finalization_progress": {
                        "after_turn": "completed",
                        "compaction": "failed"
                    },
                    "failure": {
                        "step": "compaction",
                        "error": "compact failed"
                    }
                }
            })
            .to_string(),
            "ts": 1
        }
    ]);
    let second_window_turns = json!([
        {
            "role": "assistant",
            "content": "stale-drift-without-checkpoint",
            "ts": 2
        }
    ]);
    let audit = Arc::new(InMemoryAuditSink::default());
    let (ctx, invocations) = build_kernel_context_with_window_turn_sequence(
        audit,
        vec![first_window_turns, second_window_turns],
    );

    let mut config = test_config();
    config.memory.sliding_window = 12;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);

    let session_id = "session-turn-checkpoint-diagnostics-kernel-single-window";
    let runtime = FakeRuntime::with_turns_and_completions(vec![], vec![], vec![])
        .with_assembled_context(AssembledConversationContext {
            messages: vec![
                json!({"role": "system", "content": "summary drift"}),
                json!({"role": "user", "content": "hello"}),
                json!({"role": "assistant", "content": "assistant-reply"}),
            ],
            estimated_tokens: Some(99),
            system_prompt_addition: None,
        });
    let coordinator = ConversationTurnCoordinator::new();

    let diagnostics = coordinator
        .load_turn_checkpoint_diagnostics_with_runtime_and_limit(
            &config,
            session_id,
            12,
            &runtime,
            ConversationRuntimeBinding::kernel(&ctx),
        )
        .await
        .expect("diagnostics should load from one kernel window snapshot");

    assert_eq!(
        diagnostics.summary().session_state,
        TurnCheckpointSessionState::FinalizationFailed
    );
    assert_eq!(
        diagnostics.recovery().action(),
        TurnCheckpointRecoveryAction::RunCompaction
    );
    assert_eq!(
        diagnostics.recovery().source(),
        TurnCheckpointTailRepairSource::Summary
    );
    assert_eq!(diagnostics.recovery().reason(), None);

    let runtime_probe = diagnostics
        .runtime_probe()
        .expect("runtime probe should use the same checkpoint snapshot");
    assert_eq!(runtime_probe.action().as_str(), "inspect_manually");
    assert_eq!(runtime_probe.source().as_str(), "runtime");
    assert_eq!(
        runtime_probe.reason(),
        TurnCheckpointTailRepairReason::CheckpointPreparationFingerprintMismatch
    );

    let captured = invocations.lock().expect("invocations lock");
    let window_calls = captured
        .iter()
        .filter(|request| request.operation == MEMORY_OP_WINDOW)
        .count();
    assert_eq!(
        window_calls, 1,
        "diagnostics should reuse one kernel window snapshot for summary and runtime probe"
    );
    assert_eq!(
        runtime
            .build_context_calls
            .lock()
            .expect("build context lock")
            .as_slice(),
        &[(session_id.to_owned(), true)]
    );
}

#[tokio::test]
async fn handle_turn_with_runtime_passes_restricted_tool_view_into_provider_request() {
    let child_view = crate::tools::ToolView::from_tool_names(["file.read"]);
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    )
    .with_tool_view(child_view.clone());
    let coordinator = ConversationTurnCoordinator::new();

    let reply = coordinator
        .handle_turn_with_runtime(
            &test_config(),
            "delegate-child-session",
            "hello",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("handle turn success");

    assert_eq!(reply, "assistant-reply");
    assert_eq!(
        runtime
            .turn_requested_tool_views
            .lock()
            .expect("turn request tool views lock")
            .as_slice(),
        &[child_view]
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_child_session_injects_runtime_narrowing_into_kernel_payload() {
    struct EchoToolAdapter;

    #[async_trait::async_trait]
    impl loongclaw_kernel::CoreToolAdapter for EchoToolAdapter {
        fn name(&self) -> &str {
            "echo-tools"
        }

        async fn execute_core_tool(
            &self,
            request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, loongclaw_contracts::ToolPlaneError> {
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({
                    "tool": request.tool_name,
                    "payload": request.payload,
                }),
            })
        }
    }

    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-delegate-runtime", "payload-context")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.tools.delegate.child_tool_allowlist = vec!["web.fetch".to_owned()];

    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = SessionRepository::new(&memory_config).expect("session repository");
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
        state: SessionState::Running,
    })
    .expect("create child session");
    repo.append_event(crate::session::repository::NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "fetch docs",
            "label": "Child",
            "execution": {
                "mode": "inline",
                "depth": 1,
                "max_depth": 2,
                "active_children": 0,
                "max_active_children": 3,
                "timeout_seconds": 60,
                "allow_shell_in_child": false,
                "child_tool_allowlist": ["web.fetch"],
                "kernel_bound": true,
                "runtime_narrowing": {
                    "web_fetch": {
                        "allowed_domains": ["docs.example.com"],
                        "allow_private_hosts": false
                    },
                    "browser": {
                        "max_sessions": 1,
                        "max_links": 8,
                        "max_text_chars": 512
                    }
                }
            }
        }),
    })
    .expect("append delegate_started event");

    let runtime = DefaultConversationRuntime::default();
    let session_context = runtime
        .session_context(
            &config,
            "child-session",
            ConversationRuntimeBinding::direct(),
        )
        .expect("load child session context");
    assert_eq!(
        session_context
            .runtime_narrowing
            .as_ref()
            .and_then(|narrowing| { narrowing.web_fetch.allowed_domains.iter().next().cloned() }),
        Some("docs.example.com".to_owned())
    );

    let clock = Arc::new(FixedClock::new(1_700_000_000));
    let audit = Arc::new(InMemoryAuditSink::default());
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
        .expect("set default tool adapter");
    let token = kernel
        .issue_token("test-pack", "test-agent", 3600)
        .expect("issue token");
    let kernel_ctx = KernelContext {
        kernel: Arc::new(kernel),
        token,
    };

    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![provider_tool_intent(
            "web.fetch",
            json!({
                "url": "https://outside.invalid/docs"
            }),
            "child-session",
            "turn-child-runtime",
            "call-child-runtime",
        )],
        raw_meta: Value::Null,
    };

    let result = TurnEngine::new(5)
        .execute_turn_in_context(
            &turn,
            &session_context,
            &DefaultAppToolDispatcher::runtime(),
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
        )
        .await;

    match result {
        TurnResult::FinalText(text) => {
            let line = text.lines().next().expect("tool result line should exist");
            let payload = line
                .strip_prefix("[ok] ")
                .expect("tool result line should keep [ok] prefix");
            let envelope: Value =
                serde_json::from_str(payload).expect("tool result envelope should be json");
            let payload_summary = envelope["payload_summary"]
                .as_str()
                .expect("payload summary should be text");
            assert!(
                payload_summary.contains("\"runtime_narrowing\""),
                "expected runtime_narrowing in payload summary, got: {payload_summary}"
            );
            assert!(
                payload_summary.contains("docs.example.com"),
                "expected narrowed domain in payload summary, got: {payload_summary}"
            );
            assert!(
                payload_summary.contains("\"max_sessions\":1"),
                "expected browser max_sessions narrowing in payload summary, got: {payload_summary}"
            );
        }
        other @ TurnResult::NeedsApproval(_)
        | other @ TurnResult::ToolDenied(_)
        | other @ TurnResult::ToolError(_)
        | other @ TurnResult::ProviderError(_) => {
            panic!("expected final text tool output, got: {other:?}")
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn session_context_preserves_child_runtime_narrowing_after_many_later_events() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-delegate-runtime", "history-retention")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();

    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = SessionRepository::new(&memory_config).expect("session repository");
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
        state: SessionState::Running,
    })
    .expect("create child session");
    repo.append_event(crate::session::repository::NewSessionEvent {
        session_id: "child-session".to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some("root-session".to_owned()),
        payload_json: json!({
            "task": "fetch docs",
            "label": "Child",
            "execution": {
                "mode": "inline",
                "depth": 1,
                "max_depth": 2,
                "active_children": 0,
                "max_active_children": 3,
                "timeout_seconds": 60,
                "allow_shell_in_child": false,
                "child_tool_allowlist": ["web.fetch"],
                "kernel_bound": true,
                "runtime_narrowing": {
                    "web_fetch": {
                        "allowed_domains": ["docs.example.com"],
                        "allow_private_hosts": false
                    },
                    "browser": {
                        "max_sessions": 1
                    }
                }
            }
        }),
    })
    .expect("append delegate_started event");

    for index in 0..24 {
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: format!("child_progress_{index}"),
            actor_session_id: Some("child-session".to_owned()),
            payload_json: json!({
                "index": index,
            }),
        })
        .expect("append later child event");
    }

    let runtime = DefaultConversationRuntime::default();
    let session_context = runtime
        .session_context(
            &config,
            "child-session",
            ConversationRuntimeBinding::direct(),
        )
        .expect("load child session context");

    let runtime_narrowing = session_context
        .runtime_narrowing
        .expect("child runtime narrowing should survive later events");
    assert_eq!(
        runtime_narrowing.web_fetch.allowed_domains,
        BTreeSet::from(["docs.example.com".to_owned()])
    );
    assert_eq!(runtime_narrowing.browser.max_sessions, Some(1));
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_executes_session_tools_via_default_dispatcher() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-session-tools", "normal-lane")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Completed,
    })
    .expect("create child session");

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Listing sessions.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "sessions_list",
                json!({}),
                "root-session",
                "turn-session-tools",
                "call-session-tools",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("handle turn success");

    assert!(
        reply.contains("\"tool\":\"sessions_list\""),
        "expected raw session tool output, got: {reply}"
    );
    assert!(
        reply.contains("child-session"),
        "expected listed child session in output, got: {reply}"
    );
}

#[cfg(all(feature = "memory-sqlite", feature = "channel-telegram"))]
#[tokio::test]
async fn handle_turn_with_runtime_executes_sessions_send_via_default_dispatcher() {
    let (base_url, request_rx, server) = spawn_telegram_send_server_once();
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-sessions-send", "normal-lane")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.tools.messages.enabled = true;
    config.telegram.enabled = true;
    config.telegram.bot_token = Some("123456:telegram-test-token".to_owned());
    config.telegram.bot_token_env = None;
    config.telegram.base_url = base_url;
    config.telegram.allowed_chat_ids = vec![123];

    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
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

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Sending to known session.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "sessions_send",
                json!({
                    "session_id": "telegram:123",
                    "text": "hello root channel"
                }),
                "controller-root",
                "turn-sessions-send",
                "call-sessions-send",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "controller-root",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("handle turn success");

    assert!(
        reply.contains("\"tool\":\"sessions_send\""),
        "expected raw sessions_send tool output, got: {reply}"
    );
    let line = reply.lines().last().expect("tool result line should exist");
    let payload = line
        .strip_prefix("[ok] ")
        .expect("tool result line should keep [ok] prefix");
    let envelope: Value =
        serde_json::from_str(payload).expect("tool result envelope should be json");
    assert!(
        envelope["payload_summary"]
            .as_str()
            .expect("payload summary should be text")
            .contains("\"delivery\":\"sent\""),
        "expected send receipt in output, got: {reply}"
    );

    let request = request_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("telegram request should be captured");
    assert!(request.starts_with("POST /bot123456:telegram-test-token/sendMessage "));
    assert!(request.contains("\"chat_id\":123"));
    assert!(request.contains("\"text\":\"hello root channel\""));

    let after_turns =
        crate::memory::window_direct("telegram:123", 10, &memory_config).expect("window turns");
    assert_eq!(after_turns.len(), before_turns.len());
    assert_eq!(after_turns[0].role, before_turns[0].role);
    assert_eq!(after_turns[0].content, before_turns[0].content);

    let events = repo
        .list_recent_events("telegram:123", 10)
        .expect("list target events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_kind, "session_message_sent");
    assert_eq!(
        events[0].actor_session_id.as_deref(),
        Some("controller-root")
    );

    server.join().expect("telegram stub join");
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_requires_approval_before_delegate_execution() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-delegate-approval", "normal-lane")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.tools.approval.mode = crate::config::GovernedToolApprovalMode::Strict;
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Delegating.".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "delegate",
                    json!({
                        "task": "child task",
                        "label": "research-subtask"
                    }),
                    "root-session",
                    "turn-delegate-parent",
                    "call-delegate-parent",
                )],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Child final output".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
        vec![],
    )
    .with_durable_memory_config(memory_config.clone());
    let coordinator = ConversationTurnCoordinator::new();

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "delegate this task",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("approval reply");

    assert!(
        reply.contains("[tool_approval_required]"),
        "expected approval marker, got: {reply}"
    );
    assert!(
        reply.contains("tool: delegate"),
        "expected delegate tool detail, got: {reply}"
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 1);
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        0
    );

    let requests = repo
        .list_approval_requests_for_session("root-session", None)
        .expect("list approval requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].tool_name, "delegate");
    assert!(
        reply.contains(requests[0].approval_request_id.as_str()),
        "reply should surface approval request id, got: {reply}"
    );
    assert!(
        repo.list_visible_sessions("root-session")
            .expect("list sessions")
            .into_iter()
            .all(|session| session.parent_session_id.as_deref() != Some("root-session")),
        "delegate child session should not be created before approval"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_executes_delegate_via_coordinator() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-delegate", "normal-lane")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Delegating.".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "delegate",
                    json!({
                        "task": "child task",
                        "label": "research-subtask"
                    }),
                    "root-session",
                    "turn-delegate-parent",
                    "call-delegate-parent",
                )],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Child final output".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
        vec![],
    )
    .with_durable_memory_config(memory_config.clone());
    let coordinator = ConversationTurnCoordinator::new();

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("delegate handle turn success");

    assert!(
        reply.contains("\"tool\":\"delegate\""),
        "expected raw delegate tool output, got: {reply}"
    );
    let line = reply.lines().last().expect("tool result line should exist");
    let payload = line
        .strip_prefix("[ok] ")
        .expect("tool result line should keep [ok] prefix");
    let envelope: Value =
        serde_json::from_str(payload).expect("tool result envelope should be json");
    let payload_summary = envelope["payload_summary"]
        .as_str()
        .expect("payload summary should be text");
    assert!(
        payload_summary.contains("\"label\":\"research-subtask\""),
        "expected child label in payload summary, got: {reply}"
    );
    assert!(
        payload_summary.contains("\"final_output\":\"Child final output\""),
        "expected child final output in payload summary, got: {reply}"
    );
    assert!(
        payload_summary.contains("\"child_session_id\":\"delegate:"),
        "expected child session id in payload summary, got: {reply}"
    );

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

    let requested = runtime
        .turn_requested_tool_views
        .lock()
        .expect("turn request tool views lock");
    assert_eq!(requested.len(), 2);
    assert!(requested[0].contains("delegate"));
    assert!(!requested[1].contains("delegate"));
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_kernel_delegate_calls_subagent_lifecycle_hooks() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-delegate", "kernel-lifecycle")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Delegating.".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "delegate",
                    json!({
                        "task": "child task",
                        "label": "research-subtask"
                    }),
                    "root-session",
                    "turn-delegate-parent",
                    "call-delegate-parent",
                )],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Child final output".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
        vec![],
    )
    .with_durable_memory_config(memory_config.clone());
    let kernel_ctx = test_kernel_context("conversation-delegate-kernel-lifecycle");

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect("delegate handle turn success");

    assert!(
        reply.contains("\"tool\":\"delegate\""),
        "expected raw delegate tool output, got: {reply}"
    );

    let child = repo
        .list_visible_sessions("root-session")
        .expect("list visible sessions")
        .into_iter()
        .find(|session| session.parent_session_id.as_deref() == Some("root-session"))
        .expect("child session summary");
    assert_eq!(
        child.state,
        crate::session::repository::SessionState::Completed
    );

    assert_eq!(
        runtime
            .subagent_lifecycle_calls
            .lock()
            .expect("subagent lifecycle calls lock")
            .clone(),
        vec![
            format!("prepare_subagent_spawn:root-session:{}", child.session_id),
            format!("on_subagent_ended:root-session:{}", child.session_id),
        ]
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_delegate_rejects_spawn_when_prepare_subagent_spawn_fails() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-delegate", "prepare-failure")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Delegating.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "delegate",
                json!({
                    "task": "child task",
                    "label": "research-subtask"
                }),
                "root-session",
                "turn-delegate-parent",
                "call-delegate-parent",
            )],
            raw_meta: Value::Null,
        })],
        vec![],
    )
    .with_prepare_subagent_spawn_result(Err("synthetic_prepare_subagent_spawn_failure".to_owned()))
    .with_durable_memory_config(memory_config.clone());
    let kernel_ctx = test_kernel_context("conversation-delegate-prepare-failure");

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect("delegate handle turn reply");

    assert!(
        reply.contains("synthetic_prepare_subagent_spawn_failure"),
        "expected prepare_subagent_spawn failure in reply, got: {reply}"
    );
    assert_eq!(
        repo.list_sessions().expect("list sessions").len(),
        1,
        "failed prepare_subagent_spawn should not create a child session"
    );

    let calls = runtime
        .subagent_lifecycle_calls
        .lock()
        .expect("subagent lifecycle calls lock")
        .clone();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0].starts_with("prepare_subagent_spawn:root-session:delegate:"),
        "unexpected lifecycle calls: {calls:?}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_approval_request_resolve_replays_delegate_for_approve_once() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-approval-resolve", "approve-once")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.ensure_approval_request(crate::session::repository::NewApprovalRequestRecord {
        approval_request_id: "apr-delegate-1".to_owned(),
        session_id: "root-session".to_owned(),
        turn_id: "turn-delegate-parent".to_owned(),
        tool_call_id: "call-delegate-parent".to_owned(),
        tool_name: "delegate".to_owned(),
        approval_key: "tool:delegate".to_owned(),
        request_payload_json: json!({
            "session_id": "root-session",
            "parent_session_id": Value::Null,
            "turn_id": "turn-delegate-parent",
            "tool_call_id": "call-delegate-parent",
            "tool_name": "delegate",
            "args_json": {
                "task": "child task",
                "label": "research-subtask"
            },
            "source": "provider_tool_call",
            "execution_kind": "app"
        }),
        governance_snapshot_json: json!({
            "governance_scope": "topology_mutation",
            "risk_class": "high",
            "approval_mode": "policy_driven",
            "rule_id": "governed_tool_requires_approval",
            "reason": "operator approval required before running `delegate`"
        }),
    })
    .expect("seed approval request");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "resolving approval".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "approval_request_resolve",
                    json!({
                        "approval_request_id": "apr-delegate-1",
                        "decision": "approve_once"
                    }),
                    "root-session",
                    "turn-approval-resolve",
                    "call-approval-resolve",
                )],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Child final output".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
        vec![],
    )
    .with_durable_memory_config(memory_config.clone());
    let coordinator = ConversationTurnCoordinator::new();

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("approval resolve reply");

    assert!(
        reply.contains("\"tool\":\"approval_request_resolve\""),
        "expected raw approval resolve tool output, got: {reply}"
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 2);
    assert_eq!(
        *runtime
            .completion_calls
            .lock()
            .expect("completion calls lock"),
        0
    );

    let request = repo
        .load_approval_request("apr-delegate-1")
        .expect("load approval request")
        .expect("approval request row");
    assert_eq!(
        request.status,
        crate::session::repository::ApprovalRequestStatus::Executed
    );
    assert_eq!(
        request.decision,
        Some(crate::session::repository::ApprovalDecision::ApproveOnce)
    );
    assert_eq!(
        request.resolved_by_session_id.as_deref(),
        Some("root-session")
    );
    assert!(request.executed_at.is_some());
    assert!(request.last_error.is_none(), "request={request:?}");
    assert!(
        repo.load_approval_grant("root-session", "tool:delegate")
            .expect("load grant")
            .is_none()
    );

    let child = repo
        .list_visible_sessions("root-session")
        .expect("list visible sessions")
        .into_iter()
        .find(|session| session.parent_session_id.as_deref() == Some("root-session"))
        .expect("child session summary");
    assert_eq!(
        child.state,
        crate::session::repository::SessionState::Completed
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_approval_request_resolve_approve_always_reuses_root_grant() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-approval-resolve", "approve-always")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.tools.approval.mode = crate::config::GovernedToolApprovalMode::Strict;
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.ensure_approval_request(crate::session::repository::NewApprovalRequestRecord {
        approval_request_id: "apr-delegate-always".to_owned(),
        session_id: "root-session".to_owned(),
        turn_id: "turn-delegate-parent".to_owned(),
        tool_call_id: "call-delegate-parent".to_owned(),
        tool_name: "delegate".to_owned(),
        approval_key: "tool:delegate".to_owned(),
        request_payload_json: json!({
            "session_id": "root-session",
            "parent_session_id": Value::Null,
            "turn_id": "turn-delegate-parent",
            "tool_call_id": "call-delegate-parent",
            "tool_name": "delegate",
            "args_json": {
                "task": "child task",
                "label": "research-subtask"
            },
            "source": "provider_tool_call",
            "execution_kind": "app"
        }),
        governance_snapshot_json: json!({
            "governance_scope": "topology_mutation",
            "risk_class": "high",
            "approval_mode": "policy_driven",
            "rule_id": "governed_tool_requires_approval",
            "reason": "operator approval required before running `delegate`"
        }),
    })
    .expect("seed approval request");

    let approval_runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "resolving approval".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "approval_request_resolve",
                    json!({
                        "approval_request_id": "apr-delegate-always",
                        "decision": "approve_always"
                    }),
                    "root-session",
                    "turn-approval-resolve",
                    "call-approval-resolve",
                )],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "approval resolved".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
        vec![],
    )
    .with_durable_memory_config(memory_config.clone());
    let coordinator = ConversationTurnCoordinator::new();

    let approval_reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &approval_runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("approval resolve reply");

    assert!(
        approval_reply.contains("\"tool\":\"approval_request_resolve\""),
        "expected raw approval resolve tool output, got: {approval_reply}"
    );

    let resolved = repo
        .load_approval_request("apr-delegate-always")
        .expect("load approval request")
        .expect("approval request row");
    assert_eq!(
        resolved.status,
        crate::session::repository::ApprovalRequestStatus::Executed
    );
    assert_eq!(
        resolved.decision,
        Some(crate::session::repository::ApprovalDecision::ApproveAlways)
    );
    assert!(
        repo.load_approval_grant("root-session", "tool:delegate")
            .expect("load grant")
            .is_some()
    );

    let granted_runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "delegating with grant".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "delegate",
                    json!({
                        "task": "second child task",
                        "label": "granted-subtask"
                    }),
                    "root-session",
                    "turn-after-grant",
                    "call-after-grant",
                )],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "granted child output".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
        vec![],
    )
    .with_durable_memory_config(memory_config.clone());

    let granted_reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &granted_runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("granted delegate reply");

    assert!(
        granted_reply.contains("\"tool\":\"delegate\""),
        "expected raw delegate tool output after stored grant, got: {granted_reply}"
    );
    assert!(
        !granted_reply.contains("[tool_approval_required]"),
        "grant-backed delegate call should not request approval again, got: {granted_reply}"
    );

    let requests = repo
        .list_approval_requests_for_session("root-session", None)
        .expect("list approval requests");
    assert_eq!(
        requests.len(),
        1,
        "grant-backed delegate call should not create a second approval request"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_approval_request_resolve_deny_does_not_replay_delegate() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-approval-resolve", "deny")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.ensure_approval_request(crate::session::repository::NewApprovalRequestRecord {
        approval_request_id: "apr-delegate-deny".to_owned(),
        session_id: "root-session".to_owned(),
        turn_id: "turn-delegate-parent".to_owned(),
        tool_call_id: "call-delegate-parent".to_owned(),
        tool_name: "delegate".to_owned(),
        approval_key: "tool:delegate".to_owned(),
        request_payload_json: json!({
            "session_id": "root-session",
            "parent_session_id": Value::Null,
            "turn_id": "turn-delegate-parent",
            "tool_call_id": "call-delegate-parent",
            "tool_name": "delegate",
            "args_json": {
                "task": "child task",
                "label": "research-subtask"
            },
            "source": "provider_tool_call",
            "execution_kind": "app"
        }),
        governance_snapshot_json: json!({
            "governance_scope": "topology_mutation",
            "risk_class": "high",
            "approval_mode": "policy_driven",
            "rule_id": "governed_tool_requires_approval",
            "reason": "operator approval required before running `delegate`"
        }),
    })
    .expect("seed approval request");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "denying approval".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "approval_request_resolve",
                    json!({
                        "approval_request_id": "apr-delegate-deny",
                        "decision": "deny"
                    }),
                    "root-session",
                    "turn-approval-deny",
                    "call-approval-deny",
                )],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "denied".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
        vec![],
    )
    .with_durable_memory_config(memory_config.clone());
    let coordinator = ConversationTurnCoordinator::new();

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("approval deny reply");

    assert!(
        reply.contains("\"tool\":\"approval_request_resolve\""),
        "expected raw approval resolve tool output, got: {reply}"
    );
    assert_eq!(*runtime.turn_calls.lock().expect("turn calls lock"), 1);

    let request = repo
        .load_approval_request("apr-delegate-deny")
        .expect("load approval request")
        .expect("approval request row");
    assert_eq!(
        request.status,
        crate::session::repository::ApprovalRequestStatus::Denied
    );
    assert_eq!(
        request.decision,
        Some(crate::session::repository::ApprovalDecision::Deny)
    );
    assert_eq!(
        request.resolved_by_session_id.as_deref(),
        Some("root-session")
    );
    assert!(request.executed_at.is_none(), "request={request:?}");
    assert!(request.last_error.is_none(), "request={request:?}");
    assert!(
        repo.load_approval_grant("root-session", "tool:delegate")
            .expect("load grant")
            .is_none()
    );
    assert!(
        repo.list_visible_sessions("root-session")
            .expect("list visible sessions")
            .into_iter()
            .all(|session| session.parent_session_id.as_deref() != Some("root-session")),
        "deny should not replay the stored delegate call"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_delegate_async_queue_failure_rolls_back_child_creation() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-delegate-async", "queue-rollback")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");

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

    let spawner = Arc::new(FakeAsyncDelegateSpawner::default());
    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Delegating async.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "delegate_async",
                json!({
                    "task": "child async task",
                    "label": "async-child"
                }),
                "root-session",
                "turn-delegate-async-parent",
                "call-delegate-async-parent",
            )],
            raw_meta: Value::Null,
        })],
        vec![],
    )
    .with_async_delegate_spawner(spawner.clone())
    .with_durable_memory_config(memory_config.clone());

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("delegate_async queue failure reply");

    assert!(
        reply.contains("insert session event failed"),
        "reply should surface delegate_async queue failure, got: {reply}"
    );

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
async fn handle_turn_with_runtime_delegate_async_rejects_when_active_child_limit_is_exhausted() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-delegate-async", "active-child-limit")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.tools.delegate.max_active_children = 1;
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session-existing".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Existing".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create existing child session");

    let spawner = Arc::new(FakeAsyncDelegateSpawner::default());
    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Delegating async.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "delegate_async",
                json!({
                    "task": "child async task",
                    "label": "async-child"
                }),
                "root-session",
                "turn-delegate-async-parent",
                "call-delegate-async-parent",
            )],
            raw_meta: Value::Null,
        })],
        vec![],
    )
    .with_async_delegate_spawner(spawner.clone())
    .with_durable_memory_config(memory_config.clone());

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("delegate_async reply");

    assert!(
        reply.contains("delegate_active_children_exceeded"),
        "expected active-child limit rejection, got: {reply}"
    );
    assert_eq!(
        repo.list_visible_sessions("root-session")
            .expect("list visible sessions")
            .into_iter()
            .filter(|session| session.parent_session_id.as_deref() == Some("root-session"))
            .count(),
        1,
        "active-child limit rejection should not create another child session"
    );
    assert_eq!(
        spawner
            .requests
            .lock()
            .expect("async delegate requests lock")
            .len(),
        0,
        "active-child limit rejection should happen before dispatching the async spawner"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_executes_delegate_async_via_coordinator_without_waiting() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-delegate-async", "queued")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");

    let (gated_spawner, request_rx, release_notify) = GatedFakeAsyncDelegateSpawner::new();
    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Delegating async.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "delegate_async",
                json!({
                    "task": "child async task",
                    "label": "async-child",
                    "timeout_seconds": 9
                }),
                "root-session",
                "turn-delegate-async-parent",
                "call-delegate-async-parent",
            )],
            raw_meta: Value::Null,
        })],
        vec![],
    )
    .with_async_delegate_spawner(Arc::new(gated_spawner))
    .with_durable_memory_config(memory_config.clone());

    let coordinator = ConversationTurnCoordinator::new();
    let queued_call = tokio::spawn(async move {
        coordinator
            .handle_turn_with_runtime(
                &config,
                "root-session",
                "show raw json tool output",
                ProviderErrorMode::Propagate,
                &runtime,
                ConversationRuntimeBinding::direct(),
            )
            .await
    });

    let spawn_request = tokio::time::timeout(std::time::Duration::from_millis(250), request_rx)
        .await
        .expect("delegate_async should dispatch spawn quickly")
        .expect("gated async delegate spawn request");
    let reply = tokio::time::timeout(std::time::Duration::from_millis(250), queued_call)
        .await
        .expect("delegate_async should return queued handle without waiting")
        .expect("join queued delegate_async task")
        .expect("delegate_async reply");

    assert!(
        reply.contains("\"tool\":\"delegate_async\""),
        "expected raw delegate_async tool output, got: {reply}"
    );
    let line = reply.lines().last().expect("tool result line should exist");
    let payload = line
        .strip_prefix("[ok] ")
        .expect("tool result line should keep [ok] prefix");
    let envelope: Value =
        serde_json::from_str(payload).expect("tool result envelope should be json");
    let payload_summary = envelope["payload_summary"]
        .as_str()
        .expect("payload summary should be text");
    assert!(
        payload_summary.contains("\"mode\":\"async\""),
        "expected async mode in payload summary, got: {reply}"
    );
    assert!(
        payload_summary.contains("\"state\":\"queued\""),
        "expected queued state in payload summary, got: {reply}"
    );
    assert!(
        payload_summary.contains("\"label\":\"async-child\""),
        "expected child label in payload summary, got: {reply}"
    );

    let child = repo
        .list_visible_sessions("root-session")
        .expect("list visible sessions")
        .into_iter()
        .find(|session| session.parent_session_id.as_deref() == Some("root-session"))
        .expect("queued child session summary");
    assert_eq!(spawn_request.child_session_id, child.session_id);
    assert_eq!(spawn_request.parent_session_id, "root-session");
    assert_eq!(spawn_request.task, "child async task");
    assert_eq!(spawn_request.label.as_deref(), Some("async-child"));
    assert_eq!(spawn_request.timeout_seconds, 9);
    assert!(
        spawn_request.kernel_context.is_none(),
        "direct parent turns should keep async delegate children in direct mode"
    );
    assert_eq!(child.state, crate::session::repository::SessionState::Ready);
    assert_eq!(child.label.as_deref(), Some("async-child"));

    let events = repo
        .list_recent_events(&child.session_id, 10)
        .expect("list child events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_kind, "delegate_queued");
    assert!(
        repo.load_terminal_outcome(&child.session_id)
            .expect("load terminal outcome")
            .is_none()
    );

    let requested = repo
        .load_session_summary(&child.session_id)
        .expect("load child summary")
        .expect("child summary");
    assert_eq!(
        requested.state,
        crate::session::repository::SessionState::Ready
    );

    release_notify.notify_waiters();
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_delegate_async_preserves_kernel_binding_in_spawn_request() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-delegate-async", "kernel-binding")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");

    let (gated_spawner, request_rx, release_notify) = GatedFakeAsyncDelegateSpawner::new();
    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Delegating async.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "delegate_async",
                json!({
                    "task": "child async task",
                    "label": "async-child",
                    "timeout_seconds": 9
                }),
                "root-session",
                "turn-delegate-async-parent",
                "call-delegate-async-parent",
            )],
            raw_meta: Value::Null,
        })],
        vec![],
    )
    .with_async_delegate_spawner(Arc::new(gated_spawner))
    .with_durable_memory_config(memory_config.clone());

    let (_audit, kernel_ctx) = {
        let audit = Arc::new(InMemoryAuditSink::default());
        let (kernel_ctx, _invocations) = build_kernel_context(audit.clone());
        (audit, kernel_ctx)
    };
    let expected_kernel_ctx = kernel_ctx.clone();

    let coordinator = ConversationTurnCoordinator::new();
    let queued_call = tokio::spawn(async move {
        coordinator
            .handle_turn_with_runtime(
                &config,
                "root-session",
                "show raw json tool output",
                ProviderErrorMode::Propagate,
                &runtime,
                ConversationRuntimeBinding::kernel(&kernel_ctx),
            )
            .await
    });

    let spawn_request = tokio::time::timeout(std::time::Duration::from_millis(250), request_rx)
        .await
        .expect("delegate_async should dispatch spawn quickly")
        .expect("gated async delegate spawn request");
    let reply = tokio::time::timeout(std::time::Duration::from_millis(250), queued_call)
        .await
        .expect("delegate_async should return queued handle without waiting")
        .expect("join queued delegate_async task")
        .expect("delegate_async reply");

    assert!(
        reply.contains("\"tool\":\"delegate_async\""),
        "expected raw delegate_async tool output, got: {reply}"
    );
    assert!(
        spawn_request.kernel_context.is_some(),
        "kernel-bound parent turns should preserve kernel context for async delegate children"
    );
    let child_kernel_ctx = spawn_request
        .kernel_context
        .as_ref()
        .expect("spawn request should carry kernel context");
    assert_eq!(child_kernel_ctx.token, expected_kernel_ctx.token);
    assert!(
        Arc::ptr_eq(&child_kernel_ctx.kernel, &expected_kernel_ctx.kernel),
        "spawned child should inherit the same kernel instance"
    );

    release_notify.notify_waiters();
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_delegate_async_spawn_failure_is_observable_after_queueing() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-delegate-async", "spawn-failed")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Delegating async.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "delegate_async",
                json!({
                    "task": "child async task",
                    "label": "async-child"
                }),
                "root-session",
                "turn-delegate-async-parent",
                "call-delegate-async-parent",
            )],
            raw_meta: Value::Null,
        })],
        vec![],
    )
    .with_async_delegate_spawner(Arc::new(FakeAsyncDelegateSpawner {
        requests: Arc::new(Mutex::new(Vec::new())),
        spawn_error: Some("spawn unavailable".to_owned()),
    }))
    .with_durable_memory_config(memory_config.clone());

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("delegate_async reply");

    assert!(
        reply.contains("\"tool\":\"delegate_async\""),
        "expected raw delegate_async tool output, got: {reply}"
    );

    let child = tokio::time::timeout(std::time::Duration::from_millis(500), async {
        loop {
            let maybe_child = repo
                .list_visible_sessions("root-session")
                .expect("list visible sessions")
                .into_iter()
                .find(|session| session.parent_session_id.as_deref() == Some("root-session"));
            if let Some(child) = maybe_child
                && child.state == crate::session::repository::SessionState::Failed
            {
                break child;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("queued delegate child should fail after spawn failure");

    let waited = crate::tools::wait_for_session_with_config(
        json!({
            "session_id": child.session_id,
            "timeout_ms": 500
        }),
        "root-session",
        &memory_config,
        &config.tools,
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

    let events = repo
        .list_recent_events(&child.session_id, 10)
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
async fn handle_turn_with_runtime_delegate_async_spawn_panic_is_observable_after_queueing() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-delegate-async", "spawn-panic")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Delegating async.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "delegate_async",
                json!({
                    "task": "child async task",
                    "label": "async-child"
                }),
                "root-session",
                "turn-delegate-async-parent",
                "call-delegate-async-parent",
            )],
            raw_meta: Value::Null,
        })],
        vec![],
    )
    .with_async_delegate_spawner(Arc::new(PanicAsyncDelegateSpawner))
    .with_durable_memory_config(memory_config.clone());

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("delegate_async reply");

    assert!(
        reply.contains("\"tool\":\"delegate_async\""),
        "expected raw delegate_async tool output, got: {reply}"
    );

    let child = tokio::time::timeout(std::time::Duration::from_millis(500), async {
        loop {
            let maybe_child = repo
                .list_visible_sessions("root-session")
                .expect("list visible sessions")
                .into_iter()
                .find(|session| session.parent_session_id.as_deref() == Some("root-session"));
            if let Some(child) = maybe_child
                && child.state == crate::session::repository::SessionState::Failed
            {
                break child;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("queued delegate child should fail after spawn panic");

    let waited = crate::tools::wait_for_session_with_config(
        json!({
            "session_id": child.session_id,
            "timeout_ms": 500
        }),
        "root-session",
        &memory_config,
        &config.tools,
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

    let events = repo
        .list_recent_events(&child.session_id, 10)
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
async fn handle_turn_with_runtime_delegate_async_spawn_failure_persistence_recovers() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-delegate-async", "spawn-persist-recovery")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");

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

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![Ok(ProviderTurn {
            assistant_text: "Delegating async.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "delegate_async",
                json!({
                    "task": "child async task",
                    "label": "async-child"
                }),
                "root-session",
                "turn-delegate-async-parent",
                "call-delegate-async-parent",
            )],
            raw_meta: Value::Null,
        })],
        vec![],
    )
    .with_async_delegate_spawner(Arc::new(FakeAsyncDelegateSpawner {
        requests: Arc::new(Mutex::new(Vec::new())),
        spawn_error: Some("spawn unavailable".to_owned()),
    }))
    .with_durable_memory_config(memory_config.clone());

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("delegate_async reply");

    assert!(
        reply.contains("\"tool\":\"delegate_async\""),
        "expected raw delegate_async tool output, got: {reply}"
    );

    let child = tokio::time::timeout(std::time::Duration::from_millis(500), async {
        loop {
            let maybe_child = repo
                .list_visible_sessions("root-session")
                .expect("list visible sessions")
                .into_iter()
                .find(|session| session.parent_session_id.as_deref() == Some("root-session"));
            if let Some(child) = maybe_child
                && child.state == crate::session::repository::SessionState::Failed
            {
                break child;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("queued delegate child should recover to failed state");

    assert!(
        child
            .last_error
            .as_deref()
            .expect("child last_error")
            .contains("delegate_async_spawn_failure_persist_failed")
    );

    let events = repo
        .list_recent_events(&child.session_id, 10)
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

    assert!(
        repo.load_terminal_outcome(&child.session_id)
            .expect("load terminal outcome")
            .is_none()
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_delegate_child_cannot_reenter_delegate_by_default() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-delegate", "nested-denied")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Delegating.".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "delegate",
                    json!({
                        "task": "show raw json tool output",
                        "label": "nested-child"
                    }),
                    "root-session",
                    "turn-delegate-parent",
                    "call-delegate-parent",
                )],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Trying nested delegate.".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "delegate",
                    json!({
                        "task": "nested"
                    }),
                    "delegate:child",
                    "turn-delegate-child",
                    "call-delegate-child",
                )],
                raw_meta: Value::Null,
            }),
        ],
        vec![],
    )
    .with_durable_memory_config(memory_config.clone());
    let coordinator = ConversationTurnCoordinator::new();

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("nested delegate denial reply");

    assert!(
        reply.contains("tool_not_found: requested tool is not available"),
        "reply should surface generic nested delegate denial, got: {reply}"
    );
    assert!(
        !reply.contains("tool_not_visible: delegate"),
        "reply should not leak the nested delegate tool id in the denial reason, got: {reply}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_delegate_child_cannot_reenter_delegate_async_by_default() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-delegate-async", "nested-denied")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");

    let runtime_ref = Arc::new(OnceLock::new());
    let spawner = Arc::new(LocalChildRuntimeAsyncDelegateSpawner {
        config: config.clone(),
        runtime: runtime_ref.clone(),
    });
    let runtime = Arc::new(
        FakeRuntime::with_turns_and_completions(
            vec![],
            vec![
                Ok(ProviderTurn {
                    assistant_text: "Delegating async.".to_owned(),
                    tool_intents: vec![provider_tool_intent(
                        "delegate_async",
                        json!({
                            "task": "show raw json tool output",
                            "label": "nested-child"
                        }),
                        "root-session",
                        "turn-delegate-async-parent",
                        "call-delegate-async-parent",
                    )],
                    raw_meta: Value::Null,
                }),
                Ok(ProviderTurn {
                    assistant_text: "Trying nested async delegate.".to_owned(),
                    tool_intents: vec![provider_tool_intent(
                        "delegate_async",
                        json!({
                            "task": "nested"
                        }),
                        "delegate:child",
                        "turn-delegate-async-child",
                        "call-delegate-async-child",
                    )],
                    raw_meta: Value::Null,
                }),
            ],
            vec![],
        )
        .with_async_delegate_spawner(spawner)
        .with_durable_memory_config(memory_config.clone()),
    );
    assert!(
        runtime_ref.set(runtime.clone()).is_ok(),
        "install local async delegate runtime"
    );
    let coordinator = ConversationTurnCoordinator::new();

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            runtime.as_ref(),
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("nested delegate_async denial reply");

    assert!(
        reply.contains("\"tool\":\"delegate_async\""),
        "reply should surface queued delegate_async handle, got: {reply}"
    );

    let child = tokio::time::timeout(std::time::Duration::from_millis(500), async {
        loop {
            let maybe_child = repo
                .list_visible_sessions("root-session")
                .expect("list visible sessions")
                .into_iter()
                .find(|session| session.parent_session_id.as_deref() == Some("root-session"));
            if let Some(child) = maybe_child
                && child.state == crate::session::repository::SessionState::Completed
            {
                break child;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("async delegate child should complete after nested denial");

    let waited = crate::tools::wait_for_session_with_config(
        json!({
            "session_id": child.session_id,
            "timeout_ms": 500
        }),
        "root-session",
        &memory_config,
        &config.tools,
    )
    .await
    .expect("session_wait outcome");

    assert_eq!(waited.status, "ok");
    assert_eq!(waited.payload["wait_status"], "completed");
    assert_eq!(waited.payload["session"]["state"], "completed");
    assert_eq!(waited.payload["terminal_outcome"]["status"], "ok");
    let final_output = waited.payload["terminal_outcome"]["payload"]["final_output"]
        .as_str()
        .expect("delegate child final output");
    assert!(
        final_output.contains("tool_not_found: requested tool is not available"),
        "child terminal output should surface generic nested delegate_async denial, got: {waited:?}"
    );
    assert!(
        !final_output.contains("delegate_async"),
        "child terminal output should not leak hidden delegate_async by name, got: {waited:?}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_delegate_child_can_reenter_when_max_depth_allows() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-delegate", "nested-allowed")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.tools.delegate.max_depth = 2;
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Delegating from root.".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "delegate",
                    json!({
                        "task": "show raw json tool output",
                        "label": "child"
                    }),
                    "root-session",
                    "turn-root",
                    "call-root",
                )],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Delegating from child.".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "delegate",
                    json!({
                        "task": "final grandchild task",
                        "label": "grandchild"
                    }),
                    "delegate:child-runtime",
                    "turn-child",
                    "call-child",
                )],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Grandchild final output".to_owned(),
                tool_intents: vec![],
                raw_meta: Value::Null,
            }),
        ],
        vec![],
    )
    .with_durable_memory_config(memory_config.clone());
    let coordinator = ConversationTurnCoordinator::new();

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
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

    let visible = repo
        .list_visible_sessions("root-session")
        .expect("visible sessions");
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
async fn handle_turn_with_runtime_executes_session_wait_via_default_dispatcher() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-session-wait", "normal-lane")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Completed,
    })
    .expect("create child session");
    repo.upsert_terminal_outcome(
        "child-session",
        "ok",
        json!({
            "child_session_id": "child-session",
            "final_output": "done"
        }),
    )
    .expect("upsert terminal outcome");

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Waiting for session completion.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "session_wait",
                json!({
                    "session_id": "child-session",
                    "timeout_ms": 50
                }),
                "root-session",
                "turn-session-wait",
                "call-session-wait",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("handle turn success");

    assert!(
        reply.contains("\"tool\":\"session_wait\""),
        "expected raw session_wait tool output, got: {reply}"
    );
    assert!(
        reply.contains("child-session"),
        "expected waited child session in output, got: {reply}"
    );
    assert!(
        !reply.contains("tool_not_visible"),
        "expected dispatcher to execute session_wait, got: {reply}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_safe_lane_executes_session_tools_via_default_dispatcher() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-session-tools", "safe-lane")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.conversation.safe_lane_plan_execution_enabled = true;
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Completed,
    })
    .expect("create child session");

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Listing sessions safely.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "sessions_list",
                json!({}),
                "root-session",
                "turn-safe-session-tools",
                "call-safe-session-tools",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "deploy to production with secret token and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("safe-lane handle turn success");

    assert!(
        reply.contains("\"tool\":\"sessions_list\""),
        "expected raw session tool output, got: {reply}"
    );
    assert!(
        reply.contains("child-session"),
        "expected listed child session in output, got: {reply}"
    );
}

#[cfg(all(feature = "memory-sqlite", feature = "channel-telegram"))]
#[tokio::test]
async fn handle_turn_with_runtime_safe_lane_executes_sessions_send_via_default_dispatcher() {
    let (base_url, request_rx, server) = spawn_telegram_send_server_once();
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-sessions-send", "safe-lane")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.conversation.safe_lane_plan_execution_enabled = true;
    config.tools.messages.enabled = true;
    config.telegram.enabled = true;
    config.telegram.bot_token = Some("123456:telegram-test-token".to_owned());
    config.telegram.bot_token_env = None;
    config.telegram.base_url = base_url;
    config.telegram.allowed_chat_ids = vec![123];

    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
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

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Sending to known session safely.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "sessions_send",
                json!({
                    "session_id": "telegram:123",
                    "text": "hello safe lane"
                }),
                "controller-root",
                "turn-safe-sessions-send",
                "call-safe-sessions-send",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "controller-root",
            "deploy to production with secret token and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("safe-lane handle turn success");

    assert!(
        reply.contains("\"tool\":\"sessions_send\""),
        "expected raw sessions_send tool output, got: {reply}"
    );
    let line = reply.lines().last().expect("tool result line should exist");
    let payload = line
        .strip_prefix("[ok] ")
        .expect("tool result line should keep [ok] prefix");
    let envelope: Value =
        serde_json::from_str(payload).expect("tool result envelope should be json");
    assert!(
        envelope["payload_summary"]
            .as_str()
            .expect("payload summary should be text")
            .contains("\"delivery\":\"sent\""),
        "expected send receipt in output, got: {reply}"
    );

    let request = request_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("telegram request should be captured");
    assert!(request.contains("\"text\":\"hello safe lane\""));

    server.join().expect("telegram stub join");
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn handle_turn_with_runtime_safe_lane_executes_session_wait_via_default_dispatcher() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-session-wait", "safe-lane")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.conversation.safe_lane_plan_execution_enabled = true;
    let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
    let repo = crate::session::repository::SessionRepository::new(&memory_config)
        .expect("session repository");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: crate::session::repository::SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: crate::session::repository::SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(crate::session::repository::NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: crate::session::repository::SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: crate::session::repository::SessionState::Completed,
    })
    .expect("create child session");
    repo.upsert_terminal_outcome(
        "child-session",
        "ok",
        json!({
            "child_session_id": "child-session",
            "final_output": "done"
        }),
    )
    .expect("upsert terminal outcome");

    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Waiting for session completion safely.".to_owned(),
            tool_intents: vec![provider_tool_intent(
                "session_wait",
                json!({
                    "session_id": "child-session",
                    "timeout_ms": 50
                }),
                "root-session",
                "turn-safe-session-wait",
                "call-safe-session-wait",
            )],
            raw_meta: Value::Null,
        }),
        Ok("unused".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();

    let reply = coordinator
        .handle_turn_with_runtime(
            &config,
            "root-session",
            "deploy to production with secret token and show raw json tool output",
            ProviderErrorMode::Propagate,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("safe-lane handle turn success");

    assert!(
        reply.contains("\"tool\":\"session_wait\""),
        "expected raw session_wait tool output, got: {reply}"
    );
    assert!(
        reply.contains("child-session"),
        "expected waited child session in output, got: {reply}"
    );
    assert!(
        !reply.contains("tool_not_visible"),
        "expected safe lane dispatcher to execute session_wait, got: {reply}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn repair_turn_checkpoint_tail_requires_manual_repair_on_preparation_context_mismatch() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "repair-context-mismatch")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);

    let session_id = "session-turn-checkpoint-repair-context-mismatch";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "finalization_failed",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "preparation": {
                        "context_message_count": 2,
                        "estimated_tokens": 16
                    },
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": "failed"
                },
                "failure": {
                    "step": "compaction",
                    "error": "compact failed"
                }
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist failed checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(vec![], vec![], vec![])
        .with_assembled_context(AssembledConversationContext {
            messages: vec![
                json!({"role": "system", "content": "sys"}),
                json!({"role": "system", "content": "summary drift"}),
                json!({"role": "user", "content": "hello"}),
                json!({"role": "assistant", "content": "assistant-reply"}),
            ],
            estimated_tokens: Some(99),
            system_prompt_addition: None,
        });
    let coordinator = ConversationTurnCoordinator::new();

    let outcome = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("context drift should downgrade to manual repair");

    assert_eq!(outcome.status().as_str(), "manual_required");
    assert_eq!(outcome.action().as_str(), "inspect_manually");
    assert_eq!(
        outcome.reason(),
        TurnCheckpointTailRepairReason::CheckpointPreparationMismatch
    );
    assert_eq!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        0
    );
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 0);

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert!(
        payloads.is_empty(),
        "preparation mismatch downgrade should not persist a new checkpoint event"
    );

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn repair_turn_checkpoint_tail_requires_manual_repair_on_preparation_content_mismatch() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id(
            "conversation-turn-checkpoint",
            "repair-context-fingerprint-mismatch"
        )
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);

    let session_id = "session-turn-checkpoint-repair-context-fingerprint-mismatch";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "finalization_failed",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "preparation": {
                        "context_message_count": 2,
                        "context_fingerprint_sha256": test_turn_preparation_context_fingerprint(&[
                            json!({"role": "system", "content": "sys"}),
                            json!({"role": "user", "content": "hello"}),
                        ]),
                        "estimated_tokens": 16
                    },
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": "failed"
                },
                "failure": {
                    "step": "compaction",
                    "error": "compact failed"
                }
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist failed checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(vec![], vec![], vec![])
        .with_assembled_context(AssembledConversationContext {
            messages: vec![
                json!({"role": "system", "content": "summary drift"}),
                json!({"role": "user", "content": "hello"}),
                json!({"role": "assistant", "content": "assistant-reply"}),
            ],
            estimated_tokens: Some(99),
            system_prompt_addition: None,
        });
    let coordinator = ConversationTurnCoordinator::new();

    let outcome = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("content drift should downgrade to manual repair");

    assert_eq!(outcome.status().as_str(), "manual_required");
    assert_eq!(
        outcome.source().map(|source| source.as_str()),
        Some("runtime")
    );
    assert_eq!(outcome.action().as_str(), "inspect_manually");
    assert_eq!(
        outcome.reason().as_str(),
        "checkpoint_preparation_fingerprint_mismatch"
    );
    assert_eq!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        0
    );
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 0);

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert!(
        payloads.is_empty(),
        "preparation fingerprint mismatch downgrade should not persist a new checkpoint event"
    );

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn repair_turn_checkpoint_tail_requires_manual_repair_on_malformed_preparation_snapshot() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id(
            "conversation-turn-checkpoint",
            "repair-preparation-malformed"
        )
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);

    let session_id = "session-turn-checkpoint-repair-preparation-malformed";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "finalization_failed",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "preparation": {
                        "context_message_count": "two",
                        "estimated_tokens": 16
                    },
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": "failed"
                },
                "failure": {
                    "step": "compaction",
                    "error": "compact failed"
                }
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist failed checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(vec![], vec![], vec![])
        .with_assembled_context(AssembledConversationContext {
            messages: vec![
                json!({"role": "system", "content": "sys"}),
                json!({"role": "user", "content": "hello"}),
                json!({"role": "assistant", "content": "assistant-reply"}),
            ],
            estimated_tokens: Some(99),
            system_prompt_addition: None,
        });
    let coordinator = ConversationTurnCoordinator::new();

    let outcome = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::direct(),
        )
        .await
        .expect("malformed preparation should downgrade to manual repair");

    assert_eq!(outcome.status().as_str(), "manual_required");
    assert_eq!(outcome.action().as_str(), "inspect_manually");
    assert_eq!(
        outcome.reason(),
        TurnCheckpointTailRepairReason::CheckpointPreparationMalformed
    );
    assert_eq!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        0
    );
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 0);

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert!(
        payloads.is_empty(),
        "malformed preparation downgrade should not persist a new checkpoint event"
    );

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn repair_turn_checkpoint_tail_with_runtime_persists_failed_after_turn_repair() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "repair-after-turn-fail")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);

    let session_id = "session-turn-checkpoint-repair-after-turn-fail";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "post_persist",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "pending",
                    "compaction": "pending"
                },
                "failure": null
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist post_persist checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "assistant-reply"}),
        ],
        vec![],
        vec![],
    )
    .with_after_turn_result(Err("repair after_turn failed".to_owned()));
    let coordinator = ConversationTurnCoordinator::new();
    let kernel_ctx = test_kernel_context_with_memory(
        "test-turn-checkpoint-repair-after-turn-failure",
        &mem_config,
    );

    let error = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect_err("after_turn repair should fail closed");
    assert!(error.contains("repair after_turn failed"));
    assert_eq!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        1
    );
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 0);

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert_eq!(
        payloads.len(),
        1,
        "expected one failed repair checkpoint event"
    );
    assert_eq!(payloads[0]["stage"], "finalization_failed");
    assert_eq!(payloads[0]["finalization_progress"]["after_turn"], "failed");
    assert_eq!(
        payloads[0]["finalization_progress"]["compaction"],
        "skipped"
    );
    assert_eq!(payloads[0]["failure"]["step"], "after_turn");
    assert_eq!(payloads[0]["failure"]["error"], "repair after_turn failed");

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn repair_turn_checkpoint_tail_with_runtime_persists_failed_compaction_repair() {
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "repair-compaction-fail")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 12;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);
    config.conversation.compact_fail_open = false;

    let session_id = "session-turn-checkpoint-repair-compaction-fail";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "finalization_failed",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "completed",
                    "compaction": "failed"
                },
                "failure": {
                    "step": "compaction",
                    "error": "compact failed"
                }
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist failed checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "assistant-reply"}),
        ],
        vec![],
        vec![],
    )
    .with_compact_result(Err("repair compaction failed".to_owned()));
    let coordinator = ConversationTurnCoordinator::new();
    let kernel_ctx = test_kernel_context_with_memory(
        "test-turn-checkpoint-repair-compaction-failure",
        &mem_config,
    );

    let error = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect_err("compaction repair should fail closed");
    assert!(error.contains("repair compaction failed"));
    assert_eq!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        0
    );
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 1);

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let payloads = persisted_conversation_event_payloads_by_name(&persisted, "turn_checkpoint");
    assert_eq!(
        payloads.len(),
        1,
        "expected one failed repair checkpoint event"
    );
    assert_eq!(payloads[0]["stage"], "finalization_failed");
    assert_eq!(
        payloads[0]["finalization_progress"]["after_turn"],
        "completed"
    );
    assert_eq!(payloads[0]["finalization_progress"]["compaction"], "failed");
    assert_eq!(payloads[0]["failure"]["step"], "compaction");
    assert_eq!(payloads[0]["failure"]["error"], "repair compaction failed");

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn durable_turn_checkpoint_repair_persists_finalized_checkpoint_and_repeated_repair_is_noop()
{
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "durable-repair-idempotent")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 16;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);
    config.conversation.compact_fail_open = false;

    let session_id = "session-turn-checkpoint-durable-repair-idempotent";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "post_persist",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "pending",
                    "compaction": "pending"
                },
                "failure": null
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist pending checkpoint");

    let runtime = FakeRuntime::with_turns_and_completions(
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "assistant-reply"}),
        ],
        vec![],
        vec![],
    )
    .with_durable_memory_config(mem_config.clone());
    let coordinator = ConversationTurnCoordinator::new();
    let kernel_ctx = test_kernel_context_with_memory(
        "test-turn-checkpoint-durable-repair-idempotent",
        &mem_config,
    );

    let first = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect("first durable repair should succeed");
    assert_eq!(first.status().as_str(), "repaired");
    assert_eq!(first.action().as_str(), "run_after_turn_and_compaction");
    assert_eq!(first.reason(), TurnCheckpointTailRepairReason::Repaired);
    assert_eq!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        1
    );
    assert_eq!(runtime.compact_calls.lock().expect("compact lock").len(), 1);

    let summary_after_first = load_turn_checkpoint_event_summary(
        session_id,
        32,
        ConversationRuntimeBinding::direct(),
        &mem_config,
    )
    .await
    .expect("load summary after first durable repair");
    assert_eq!(summary_after_first.checkpoint_events, 2);
    assert_eq!(
        summary_after_first.latest_stage,
        Some(TurnCheckpointStage::Finalized)
    );
    assert_eq!(
        summary_after_first.latest_after_turn,
        Some(TurnCheckpointProgressStatus::Completed)
    );
    assert_eq!(
        summary_after_first.latest_compaction,
        Some(TurnCheckpointProgressStatus::Completed)
    );
    assert_eq!(summary_after_first.latest_identity_present, Some(true));
    assert!(!summary_after_first.requires_recovery);

    let second = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect("second durable repair should be a noop");
    assert_eq!(second.status().as_str(), "not_needed");
    assert_eq!(second.action().as_str(), "none");
    assert_eq!(second.reason(), TurnCheckpointTailRepairReason::NotNeeded);
    assert_eq!(
        runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        1,
        "repeated repair must not rerun after_turn"
    );
    assert_eq!(
        runtime.compact_calls.lock().expect("compact lock").len(),
        1,
        "repeated repair must not rerun compaction"
    );

    let summary_after_second = load_turn_checkpoint_event_summary(
        session_id,
        32,
        ConversationRuntimeBinding::direct(),
        &mem_config,
    )
    .await
    .expect("load summary after second durable repair");
    assert_eq!(summary_after_second.checkpoint_events, 2);
    assert_eq!(
        summary_after_second.latest_stage,
        Some(TurnCheckpointStage::Finalized)
    );
    assert!(!summary_after_second.requires_recovery);

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn repair_turn_checkpoint_tail_with_runtime_recovers_discovery_followup_checkpoint() {
    use crate::test_support::TurnTestHarness;

    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "discovery-followup-repair")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 16;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);
    config.conversation.compact_fail_open = false;

    let session_id = "session-turn-checkpoint-discovery-followup-repair";
    let user_input = "search for the right tool, then read and summarize note.md";
    let final_reply = "Summary: the note says hello from discovery followup repair.";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    let harness = TurnTestHarness::new();
    std::fs::write(
        harness.temp_dir.join("note.md"),
        "hello from discovery followup repair",
    )
    .expect("seed discovery followup repair note");

    let failing_runtime = FakeRuntime::with_turns_and_completions(
        vec![],
        vec![
            Ok(ProviderTurn {
                assistant_text: "Let me search first.".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "tool.search",
                    json!({"query": "read note.md", "limit": 3}),
                    session_id,
                    "turn-discovery-followup-repair",
                    "call-search-repair",
                )],
                raw_meta: Value::Null,
            }),
            Ok(ProviderTurn {
                assistant_text: "Now I'll read the file.".to_owned(),
                tool_intents: vec![provider_tool_intent(
                    "file.read",
                    json!({"path": "note.md"}),
                    session_id,
                    "turn-discovery-followup-repair",
                    "call-invoke-repair",
                )],
                raw_meta: Value::Null,
            }),
        ],
        vec![Ok(final_reply.to_owned())],
    )
    .with_durable_memory_config(mem_config.clone())
    .with_compact_result(Err("discovery followup compaction failed".to_owned()));
    let coordinator = ConversationTurnCoordinator::new();

    let error = coordinator
        .handle_turn_with_runtime(
            &config,
            session_id,
            user_input,
            ProviderErrorMode::Propagate,
            &failing_runtime,
            ConversationRuntimeBinding::from_optional_kernel_context(Some(&harness.kernel_ctx)),
        )
        .await
        .expect_err("initial run should persist a failed checkpoint when compaction fails");
    assert!(error.contains("discovery followup compaction failed"));

    let summary_after_failure = load_turn_checkpoint_event_summary(
        session_id,
        32,
        ConversationRuntimeBinding::direct(),
        &mem_config,
    )
    .await
    .expect("load summary after failed discovery followup run");
    assert!(summary_after_failure.requires_recovery);
    assert_eq!(
        plan_turn_checkpoint_recovery(&summary_after_failure),
        TurnCheckpointRecoveryAction::RunCompaction
    );

    let retry_runtime = FakeRuntime::with_turns_and_completions(
        vec![
            json!({"role": "user", "content": user_input}),
            json!({"role": "assistant", "content": final_reply}),
        ],
        vec![],
        vec![],
    )
    .with_durable_memory_config(mem_config.clone());
    let kernel_ctx = test_kernel_context_with_memory(
        "test-turn-checkpoint-discovery-followup-repair",
        &mem_config,
    );

    let repair = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &retry_runtime,
            ConversationRuntimeBinding::from_optional_kernel_context(Some(&kernel_ctx)),
        )
        .await
        .expect("discovery followup checkpoint should remain repairable");
    assert_eq!(repair.status().as_str(), "repaired");
    assert_eq!(repair.action().as_str(), "run_compaction");
    assert_eq!(repair.reason(), TurnCheckpointTailRepairReason::Repaired);
    assert_eq!(
        retry_runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        0,
        "compaction-only retry should not rerun after_turn"
    );
    assert_eq!(
        retry_runtime
            .compact_calls
            .lock()
            .expect("compact lock")
            .len(),
        1
    );

    let _ = std::fs::remove_file(&db_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn durable_turn_checkpoint_repair_persists_failed_terminal_checkpoint_then_recovers_on_retry()
{
    let db_path = std::env::temp_dir().join(format!(
        "{}.sqlite3",
        unique_acp_test_id("conversation-turn-checkpoint", "durable-repair-retry")
    ));
    let _ = std::fs::remove_file(&db_path);

    let mut config = test_config();
    config.memory.sqlite_path = db_path.display().to_string();
    config.memory.sliding_window = 16;
    config.conversation.compact_enabled = true;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);
    config.conversation.compact_fail_open = false;

    let session_id = "session-turn-checkpoint-durable-repair-retry";
    let mem_config = MemoryRuntimeConfig::from_memory_config(&config.memory);

    crate::memory::append_turn_direct(session_id, "user", "hello", &mem_config)
        .expect("persist user turn");
    crate::memory::append_turn_direct(session_id, "assistant", "assistant-reply", &mem_config)
        .expect("persist assistant turn");
    crate::memory::append_turn_direct(
        session_id,
        "assistant",
        &json!({
            "type": "conversation_event",
            "event": "turn_checkpoint",
            "payload": {
                "schema_version": 1,
                "stage": "post_persist",
                "checkpoint": {
                    "identity": test_turn_checkpoint_identity("hello", "assistant-reply"),
                    "lane": {
                        "lane": "fast",
                        "result_kind": "final_text"
                    },
                    "finalization": {
                        "persistence_mode": "success",
                        "runs_after_turn": true,
                        "attempts_context_compaction": true
                    }
                },
                "finalization_progress": {
                    "after_turn": "pending",
                    "compaction": "pending"
                },
                "failure": null
            }
        })
        .to_string(),
        &mem_config,
    )
    .expect("persist pending checkpoint");

    let failing_runtime = FakeRuntime::with_turns_and_completions(
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "assistant-reply"}),
        ],
        vec![],
        vec![],
    )
    .with_durable_memory_config(mem_config.clone())
    .with_compact_result(Err("durable repair compaction failed".to_owned()));
    let coordinator = ConversationTurnCoordinator::new();
    let kernel_ctx =
        test_kernel_context_with_memory("test-turn-checkpoint-durable-repair-retry", &mem_config);

    let error = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &failing_runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect_err("first durable repair should persist failure and return error");
    assert!(error.contains("durable repair compaction failed"));
    assert_eq!(
        failing_runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        1
    );
    assert_eq!(
        failing_runtime
            .compact_calls
            .lock()
            .expect("compact lock")
            .len(),
        1
    );

    let summary_after_failure = load_turn_checkpoint_event_summary(
        session_id,
        32,
        ConversationRuntimeBinding::direct(),
        &mem_config,
    )
    .await
    .expect("load summary after durable failure");
    assert_eq!(summary_after_failure.checkpoint_events, 2);
    assert_eq!(
        summary_after_failure.latest_stage,
        Some(TurnCheckpointStage::FinalizationFailed)
    );
    assert_eq!(
        summary_after_failure.latest_after_turn,
        Some(TurnCheckpointProgressStatus::Completed)
    );
    assert_eq!(
        summary_after_failure.latest_compaction,
        Some(TurnCheckpointProgressStatus::Failed)
    );
    assert_eq!(summary_after_failure.latest_identity_present, Some(true));
    assert!(summary_after_failure.requires_recovery);
    assert_eq!(
        plan_turn_checkpoint_recovery(&summary_after_failure),
        TurnCheckpointRecoveryAction::RunCompaction
    );

    let retry_runtime = FakeRuntime::with_turns_and_completions(
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "hello"}),
            json!({"role": "assistant", "content": "assistant-reply"}),
        ],
        vec![],
        vec![],
    )
    .with_durable_memory_config(mem_config.clone());

    let retry = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &retry_runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect("second durable repair should recover");
    assert_eq!(retry.status().as_str(), "repaired");
    assert_eq!(retry.action().as_str(), "run_compaction");
    assert_eq!(retry.reason(), TurnCheckpointTailRepairReason::Repaired);
    assert_eq!(
        retry_runtime
            .after_turn_calls
            .lock()
            .expect("after-turn lock")
            .len(),
        0,
        "compaction-only retry must not rerun after_turn"
    );
    assert_eq!(
        retry_runtime
            .compact_calls
            .lock()
            .expect("compact lock")
            .len(),
        1
    );

    let summary_after_retry = load_turn_checkpoint_event_summary(
        session_id,
        32,
        ConversationRuntimeBinding::direct(),
        &mem_config,
    )
    .await
    .expect("load summary after durable retry");
    assert_eq!(summary_after_retry.checkpoint_events, 3);
    assert_eq!(
        summary_after_retry.latest_stage,
        Some(TurnCheckpointStage::Finalized)
    );
    assert_eq!(
        summary_after_retry.latest_after_turn,
        Some(TurnCheckpointProgressStatus::Completed)
    );
    assert_eq!(
        summary_after_retry.latest_compaction,
        Some(TurnCheckpointProgressStatus::Completed)
    );
    assert_eq!(summary_after_retry.latest_identity_present, Some(true));
    assert!(!summary_after_retry.requires_recovery);

    let third = coordinator
        .repair_turn_checkpoint_tail_with_runtime(
            &config,
            session_id,
            &retry_runtime,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
        )
        .await
        .expect("finalized durable repair should stay noop");
    assert_eq!(third.status().as_str(), "not_needed");
    assert_eq!(third.reason(), TurnCheckpointTailRepairReason::NotNeeded);
    assert_eq!(
        retry_runtime
            .compact_calls
            .lock()
            .expect("compact lock")
            .len(),
        1,
        "finalized durable checkpoint must not trigger another compaction"
    );

    let _ = std::fs::remove_file(&db_path);
}
