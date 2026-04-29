use super::*;
use crate::conversation::ConversationRuntimeBinding;
use crate::test_support::ScopedEnv;
use serde_json::json;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(feature = "memory-sqlite")]
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};

#[cfg(feature = "memory-sqlite")]
use async_trait::async_trait;
#[cfg(feature = "memory-sqlite")]
use loong_contracts::{Capability, ExecutionRoute, HarnessKind, MemoryPlaneError};
#[cfg(feature = "memory-sqlite")]
use loong_kernel::{
    CoreMemoryAdapter, FixedClock, InMemoryAuditSink, LoongKernel, MemoryCoreOutcome,
    MemoryCoreRequest, StaticPolicyEngine, VerticalPackManifest,
};
#[cfg(feature = "memory-sqlite")]
use serde_json::Value;

#[cfg(feature = "memory-sqlite")]
fn test_config() -> LoongConfig {
    let mut config = LoongConfig::default();
    config.provider = crate::config::ProviderConfig::default();
    config.audit.mode = crate::config::AuditMode::InMemory;
    config
}

#[cfg(feature = "memory-sqlite")]
fn test_kernel_context_with_memory(
    agent_id: &str,
    memory_config: &SessionStoreConfig,
) -> crate::KernelContext {
    let clock = Arc::new(FixedClock::new(1_700_000_000));
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut kernel = LoongKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);

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

    let adapter = crate::session::store::session_memory_adapter(memory_config);
    kernel.register_core_memory_adapter(adapter);

    kernel
        .set_default_core_memory_adapter("mvp-memory")
        .expect("set memory test adapter");

    let token = kernel
        .issue_token("test-pack-memory", agent_id, 60)
        .expect("issue memory test token");

    crate::KernelContext {
        kernel: Arc::new(kernel),
        token,
    }
}

#[test]
fn cli_chat_options_detect_explicit_acp_requests() {
    assert!(
        CliChatOptions {
            acp_requested: true,
            ..CliChatOptions::default()
        }
        .requests_explicit_acp()
    );

    assert!(
        CliChatOptions {
            acp_bootstrap_mcp_servers: vec!["filesystem".to_owned()],
            ..CliChatOptions::default()
        }
        .requests_explicit_acp()
    );

    assert!(
        CliChatOptions {
            acp_working_directory: Some(PathBuf::from("/workspace/project")),
            ..CliChatOptions::default()
        }
        .requests_explicit_acp()
    );
}

#[test]
fn cli_chat_options_keep_automatic_routing_without_explicit_acp_inputs() {
    assert!(!CliChatOptions::default().requests_explicit_acp());
}

#[test]
fn build_onboard_command_defaults_to_current_executable() {
    let expected_executable = std::env::current_exe().expect("current executable");
    let command =
        build_onboard_command(None, Path::new("/tmp/loong.toml")).expect("onboard command");

    assert_eq!(command.get_program(), expected_executable.as_os_str());
    assert_eq!(
        command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        vec!["onboard".to_owned()]
    );
}

#[test]
fn build_onboard_command_prefers_loong_test_override() {
    let mut env = ScopedEnv::new();
    env.remove(TEST_ONBOARD_EXECUTABLE_ENV);
    env.set(TEST_ONBOARD_EXECUTABLE_ENV, "/tmp/loong-onboard");

    let command =
        build_onboard_command(None, Path::new("/tmp/loong.toml")).expect("onboard command");

    assert_eq!(command.get_program(), OsStr::new("/tmp/loong-onboard"));
}

#[test]
fn build_onboard_command_forwards_explicit_config_path_to_output() {
    let command = build_onboard_command_for_executable(
        PathBuf::from("/tmp/loong"),
        Some("custom.toml"),
        Path::new("/tmp/custom.toml"),
    );

    assert_eq!(command.get_program(), OsStr::new("/tmp/loong"));
    assert_eq!(
        command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        vec![
            "onboard".to_owned(),
            "--output".to_owned(),
            "/tmp/custom.toml".to_owned()
        ]
    );
}

#[test]
fn onboard_command_hint_preserves_explicit_config_path() {
    let hint = format_onboard_command_hint(Some("custom.toml"), Path::new("/tmp/custom.toml"));

    assert_eq!(hint, "loong onboard --output /tmp/custom.toml");
}

#[cfg(feature = "memory-sqlite")]
fn unique_chat_sqlite_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "loong-chat-binding-{label}-{}.sqlite3",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn cleanup_chat_test_memory(sqlite_path: &Path) {
    let _ = std::fs::remove_file(sqlite_path);
    let _ = std::fs::remove_file(format!("{}-wal", sqlite_path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", sqlite_path.display()));
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn init_chat_test_memory(label: &str) -> (LoongConfig, SessionStoreConfig, PathBuf) {
    let sqlite_path = unique_chat_sqlite_path(label);
    cleanup_chat_test_memory(&sqlite_path);

    let mut config = LoongConfig::default();
    config.audit.mode = crate::config::AuditMode::InMemory;
    config.memory.sqlite_path = sqlite_path.display().to_string();
    let memory_config = SessionStoreConfig::from_memory_config(&config.memory);
    store::ensure_session_store_ready(Some(config.memory.resolved_sqlite_path()), &memory_config)
        .expect("initialize sqlite memory");

    (config, memory_config, sqlite_path)
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn init_chat_test_memory_uses_in_memory_audit_mode() {
    let (config, _memory_config, sqlite_path) = init_chat_test_memory("audit-mode");

    assert_eq!(config.audit.mode, crate::config::AuditMode::InMemory);

    cleanup_chat_test_memory(&sqlite_path);
}

#[cfg(feature = "memory-sqlite")]
struct SharedTestMemoryAdapter {
    invocations: Arc<Mutex<Vec<MemoryCoreRequest>>>,
    status: String,
    window_turns: Value,
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl CoreMemoryAdapter for SharedTestMemoryAdapter {
    fn name(&self) -> &str {
        "chat-binding-memory-shared"
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
            status: self.status.clone(),
            payload,
        })
    }
}

#[cfg(feature = "memory-sqlite")]
fn build_kernel_context_with_window_turns(
    window_turns: Value,
) -> (crate::KernelContext, Arc<Mutex<Vec<MemoryCoreRequest>>>) {
    build_kernel_context_with_window_outcome("ok", window_turns)
}

#[cfg(feature = "memory-sqlite")]
fn build_kernel_context_with_window_outcome(
    status: &str,
    window_turns: Value,
) -> (crate::KernelContext, Arc<Mutex<Vec<MemoryCoreRequest>>>) {
    let audit = Arc::new(InMemoryAuditSink::default());
    let clock = Arc::new(FixedClock::new(1_700_000_000));
    let mut kernel = LoongKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);

    let pack = VerticalPackManifest {
        pack_id: "chat-test-pack".to_owned(),
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
    kernel.register_pack(pack).expect("register pack");

    let invocations = Arc::new(Mutex::new(Vec::new()));
    kernel.register_core_memory_adapter(SharedTestMemoryAdapter {
        invocations: invocations.clone(),
        status: status.to_owned(),
        window_turns,
    });
    kernel
        .set_default_core_memory_adapter("chat-binding-memory-shared")
        .expect("set default memory adapter");

    let token = kernel
        .issue_token("chat-test-pack", "chat-test-agent", 3600)
        .expect("issue token");

    let ctx = crate::KernelContext {
        kernel: Arc::new(kernel),
        token,
    };
    (ctx, invocations)
}

#[cfg(feature = "memory-sqlite")]
fn append_assistant_payloads(
    session_id: &str,
    payloads: &[String],
    memory_config: &SessionStoreConfig,
) {
    for payload in payloads {
        store::append_session_turn_direct(session_id, "assistant", payload, memory_config)
            .expect("persist assistant payload");
    }
}

#[cfg(feature = "memory-sqlite")]
fn assistant_window_turns(payloads: &[String]) -> Value {
    json!(
        payloads
            .iter()
            .enumerate()
            .map(|(index, payload)| json!({
                "role": "assistant",
                "content": payload,
                "ts": index as i64 + 1
            }))
            .collect::<Vec<_>>()
    )
}

#[cfg(feature = "memory-sqlite")]
fn safe_lane_event_payloads() -> Vec<String> {
    vec![
        json!({
            "type": "conversation_event",
            "event": "plan_round_started",
            "payload": {
                "round": 0
            }
        })
        .to_string(),
        json!({
            "type": "conversation_event",
            "event": "verify_failed",
            "payload": {
                "failure_code": "safe_lane_plan_verify_failed"
            }
        })
        .to_string(),
        json!({
            "type": "conversation_event",
            "event": "final_status",
            "payload": {
                "status": "failed",
                "failure_code": "safe_lane_plan_verify_failed",
                "route_decision": "terminal"
            }
        })
        .to_string(),
    ]
}

#[cfg(feature = "memory-sqlite")]
fn turn_checkpoint_event_payloads() -> Vec<String> {
    vec![
        json!({
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
        json!({
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
    ]
}

#[cfg(feature = "memory-sqlite")]
fn fast_lane_tool_batch_event_payloads() -> Vec<String> {
    vec![
        json!({
            "type": "conversation_event",
            "event": "fast_lane_tool_batch",
            "payload": {
                "schema_version": 2,
                "total_intents": 5,
                "parallel_execution_enabled": true,
                "parallel_execution_max_in_flight": 2,
                "observed_peak_in_flight": 2,
                "observed_wall_time_ms": 34,
                "parallel_safe_intents": 4,
                "serial_only_intents": 1,
                "parallel_segments": 2,
                "sequential_segments": 1,
                "segments": [
                    {
                        "segment_index": 0,
                        "scheduling_class": "parallel_safe",
                        "execution_mode": "parallel",
                        "intent_count": 2,
                        "observed_peak_in_flight": 2,
                        "observed_wall_time_ms": 14
                    },
                    {
                        "segment_index": 1,
                        "scheduling_class": "serial_only",
                        "execution_mode": "sequential",
                        "intent_count": 1,
                        "observed_peak_in_flight": 1,
                        "observed_wall_time_ms": 8
                    },
                    {
                        "segment_index": 2,
                        "scheduling_class": "parallel_safe",
                        "execution_mode": "parallel",
                        "intent_count": 2,
                        "observed_peak_in_flight": 2,
                        "observed_wall_time_ms": 12
                    }
                ]
            }
        })
        .to_string(),
    ]
}

#[cfg(feature = "memory-sqlite")]
fn legacy_fast_lane_tool_batch_event_payloads() -> Vec<String> {
    vec![
        json!({
            "type": "conversation_event",
            "event": "fast_lane_tool_batch",
            "payload": {
                "schema_version": 1,
                "total_intents": 3,
                "parallel_execution_enabled": true,
                "parallel_execution_max_in_flight": 2,
                "parallel_safe_intents": 2,
                "serial_only_intents": 1,
                "parallel_segments": 1,
                "sequential_segments": 1,
                "segments": [
                    {
                        "segment_index": 0,
                        "scheduling_class": "parallel_safe",
                        "execution_mode": "parallel",
                        "intent_count": 2
                    },
                    {
                        "segment_index": 1,
                        "scheduling_class": "serial_only",
                        "execution_mode": "sequential",
                        "intent_count": 1
                    }
                ]
            }
        })
        .to_string(),
    ]
}

#[tokio::test]
async fn run_cli_ask_rejects_empty_message() {
    let error = run_cli_ask(None, None, "   ", &CliChatOptions::default())
        .await
        .expect_err("empty one-shot message should fail");

    assert!(error.contains("ask message must not be empty"));
}

#[test]
fn concurrent_cli_host_requires_explicit_session_id() {
    let shutdown = ConcurrentCliShutdown::new();
    let error = run_concurrent_cli_host(&ConcurrentCliHostOptions {
        resolved_path: PathBuf::from("/tmp/loong.toml"),
        config: LoongConfig::default(),
        session_id: "   ".to_owned(),
        shutdown,
        initialize_runtime_environment: false,
    })
    .expect_err("concurrent host should reject an implicit session id");

    assert!(
        error.contains("explicit session"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
#[cfg(feature = "memory-sqlite")]
async fn concurrent_cli_host_exits_when_shutdown_is_requested() {
    let (mut config, _memory_config, sqlite_path) = init_chat_test_memory("concurrent-host");
    config.audit.mode = crate::config::AuditMode::InMemory;
    let options = CliChatOptions::default();
    let runtime = initialize_cli_turn_runtime_with_loaded_config(
        PathBuf::from("/tmp/loong.toml"),
        config,
        Some("cli-supervisor"),
        &options,
        "cli-chat-concurrent-test",
        CliSessionRequirement::RequireExplicit,
        false,
    )
    .expect("concurrent host runtime");
    assert!(runtime.conversation_binding().is_kernel_bound());
    assert_eq!(
        runtime.runtime_kernel.kernel_context().agent_id(),
        "cli-chat-concurrent-test"
    );
    let shutdown = ConcurrentCliShutdown::new();
    shutdown.request_shutdown();

    run_concurrent_cli_host_loop(&runtime, &options, &shutdown)
        .await
        .expect("concurrent host should stop cleanly when shutdown is requested");

    cleanup_chat_test_memory(&sqlite_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn print_history_accepts_explicit_runtime_binding() {
    let (config, memory_config, sqlite_path) = init_chat_test_memory("diagnostics");

    let session_id = "chat-binding-history-direct";
    store::append_session_turn_direct(session_id, "user", "hello", &memory_config)
        .expect("persist user turn");
    store::append_session_turn_direct(session_id, "assistant", "world", &memory_config)
        .expect("persist assistant turn");

    let direct_lines = load_history_lines(
        session_id,
        config.memory.sliding_window,
        ConversationRuntimeBinding::direct(),
        &memory_config,
    )
    .await
    .expect("load history lines with explicit direct binding");
    assert_eq!(
        direct_lines,
        vec!["user: hello".to_owned(), "assistant: world".to_owned()]
    );

    let (kernel_ctx, invocations) = build_kernel_context_with_window_turns(json!([
        {
            "role": "user",
            "content": "kernel hello",
            "ts": 7
        },
        {
            "role": "assistant",
            "content": "kernel world",
            "ts": 8
        }
    ]));
    let kernel_lines = load_history_lines(
        "chat-binding-history-kernel",
        16,
        ConversationRuntimeBinding::kernel(&kernel_ctx),
        &memory_config,
    )
    .await
    .expect("load history lines with explicit kernel binding");
    assert_eq!(
        kernel_lines,
        vec![
            "[7] user: kernel hello".to_owned(),
            "[8] assistant: kernel world".to_owned()
        ]
    );

    let captured = invocations.lock().expect("invocations lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);
    assert_eq!(
        captured[0].payload["session_id"],
        "chat-binding-history-kernel"
    );
    assert_eq!(captured[0].payload["limit"], json!(16));

    cleanup_chat_test_memory(&sqlite_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn print_history_rejects_non_ok_kernel_memory_outcome() {
    let (_config, memory_config, sqlite_path) = init_chat_test_memory("diagnostics-non-ok");

    let (kernel_ctx, invocations) = build_kernel_context_with_window_outcome(
        "error",
        json!([
            {
                "role": "user",
                "content": "kernel hello",
                "ts": 7
            }
        ]),
    );
    let error = load_history_lines(
        "chat-binding-history-kernel-non-ok",
        16,
        ConversationRuntimeBinding::kernel(&kernel_ctx),
        &memory_config,
    )
    .await
    .expect_err("non-ok kernel memory outcome should fail closed");
    assert!(error.contains("non-ok status"), "unexpected error: {error}");
    assert!(error.contains("error"), "unexpected error: {error}");

    let captured = invocations.lock().expect("invocations lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);
    assert_eq!(
        captured[0].payload["session_id"],
        "chat-binding-history-kernel-non-ok"
    );
    assert_eq!(captured[0].payload["limit"], json!(16));

    cleanup_chat_test_memory(&sqlite_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn safe_lane_summary_output_accepts_explicit_runtime_binding() {
    let (config, memory_config, sqlite_path) = init_chat_test_memory("safe-lane-output");

    let direct_payloads = safe_lane_event_payloads();
    append_assistant_payloads(
        "chat-binding-safe-lane-direct",
        &direct_payloads,
        &memory_config,
    );
    let direct_output = load_safe_lane_summary_output(
        "chat-binding-safe-lane-direct",
        64,
        &config.conversation,
        ConversationRuntimeBinding::direct(),
        &memory_config,
    )
    .await
    .expect("load safe lane summary via direct binding");
    assert!(
        direct_output.contains("safe_lane_summary session=chat-binding-safe-lane-direct limit=64")
    );
    assert!(direct_output.contains("round_started=1"));
    assert!(direct_output.contains("verify_failed=1"));
    assert!(direct_output.contains("failure_code=safe_lane_plan_verify_failed"));

    let kernel_payloads = safe_lane_event_payloads();
    let (kernel_ctx, invocations) =
        build_kernel_context_with_window_turns(assistant_window_turns(&kernel_payloads));
    let kernel_output = load_safe_lane_summary_output(
        "chat-binding-safe-lane-kernel",
        80,
        &config.conversation,
        ConversationRuntimeBinding::kernel(&kernel_ctx),
        &memory_config,
    )
    .await
    .expect("load safe lane summary via kernel binding");
    assert!(
        kernel_output.contains("safe_lane_summary session=chat-binding-safe-lane-kernel limit=80")
    );
    assert!(kernel_output.contains("round_started=1"));
    assert!(kernel_output.contains("verify_failed=1"));
    assert!(kernel_output.contains("failure_code=safe_lane_plan_verify_failed"));

    let captured = invocations.lock().expect("invocations lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);
    assert_eq!(
        captured[0].payload["session_id"],
        "chat-binding-safe-lane-kernel"
    );
    assert_eq!(captured[0].payload["limit"], json!(80));
    assert_eq!(captured[0].payload["allow_extended_limit"], json!(true));

    cleanup_chat_test_memory(&sqlite_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fast_lane_summary_output_accepts_explicit_runtime_binding() {
    let (_config, memory_config, sqlite_path) = init_chat_test_memory("fast-lane-output");

    let direct_payloads = fast_lane_tool_batch_event_payloads();
    append_assistant_payloads(
        "chat-binding-fast-lane-direct",
        &direct_payloads,
        &memory_config,
    );
    let direct_output = load_fast_lane_summary_output(
        "chat-binding-fast-lane-direct",
        72,
        ConversationRuntimeBinding::direct(),
        &memory_config,
    )
    .await
    .expect("load fast lane summary via direct binding");
    assert!(
        direct_output.contains("fast_lane_summary session=chat-binding-fast-lane-direct limit=72")
    );
    assert!(direct_output.contains("batch_events=1"));
    assert!(direct_output.contains("total_intents=5"));
    assert!(direct_output.contains("parallel_safe_intents=4"));
    assert!(direct_output.contains(
            "aggregate_batches parallel_enabled=1 parallel_only=0 mixed=1 sequential_only=0 without_segments=0"
        ));
    assert!(direct_output.contains(
            "aggregate_execution configured_max_in_flight_avg=2.000 configured_max_in_flight_max=2 configured_max_in_flight_samples=1 observed_peak_in_flight_avg=2.000 observed_peak_in_flight_max=2 observed_peak_in_flight_samples=1 degraded_parallel_segments=0"
        ));
    assert!(direct_output.contains(
            "aggregate_latency observed_wall_time_ms_avg=34.000 observed_wall_time_ms_max=34 observed_wall_time_ms_samples=1"
        ));
    assert!(direct_output.contains(
            "latest_batch total_intents=5 parallel_enabled=true max_in_flight=2 observed_peak_in_flight=2 observed_wall_time_ms=34 parallel_safe_intents=4 serial_only_intents=1 parallel_segments=2 sequential_segments=1"
        ));
    assert!(direct_output.contains(
            "latest_segments=0:parallel_safe/parallel/2[peak=2 wall_ms=14],1:serial_only/sequential/1[peak=1 wall_ms=8],2:parallel_safe/parallel/2[peak=2 wall_ms=12]"
        ));

    let kernel_payloads = fast_lane_tool_batch_event_payloads();
    let (kernel_ctx, invocations) =
        build_kernel_context_with_window_turns(assistant_window_turns(&kernel_payloads));
    let kernel_output = load_fast_lane_summary_output(
        "chat-binding-fast-lane-kernel",
        88,
        ConversationRuntimeBinding::kernel(&kernel_ctx),
        &memory_config,
    )
    .await
    .expect("load fast lane summary via kernel binding");
    assert!(
        kernel_output.contains("fast_lane_summary session=chat-binding-fast-lane-kernel limit=88")
    );
    assert!(kernel_output.contains("batch_events=1"));
    assert!(kernel_output.contains("total_intents=5"));
    assert!(kernel_output.contains("parallel_safe_intents=4"));
    assert!(kernel_output.contains(
            "aggregate_batches parallel_enabled=1 parallel_only=0 mixed=1 sequential_only=0 without_segments=0"
        ));
    assert!(kernel_output.contains(
            "aggregate_execution configured_max_in_flight_avg=2.000 configured_max_in_flight_max=2 configured_max_in_flight_samples=1 observed_peak_in_flight_avg=2.000 observed_peak_in_flight_max=2 observed_peak_in_flight_samples=1 degraded_parallel_segments=0"
        ));
    assert!(kernel_output.contains(
            "aggregate_latency observed_wall_time_ms_avg=34.000 observed_wall_time_ms_max=34 observed_wall_time_ms_samples=1"
        ));
    assert!(kernel_output.contains(
            "latest_batch total_intents=5 parallel_enabled=true max_in_flight=2 observed_peak_in_flight=2 observed_wall_time_ms=34 parallel_safe_intents=4 serial_only_intents=1 parallel_segments=2 sequential_segments=1"
        ));
    assert!(kernel_output.contains(
            "latest_segments=0:parallel_safe/parallel/2[peak=2 wall_ms=14],1:serial_only/sequential/1[peak=1 wall_ms=8],2:parallel_safe/parallel/2[peak=2 wall_ms=12]"
        ));

    let captured = invocations.lock().expect("invocations lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);
    assert_eq!(
        captured[0].payload["session_id"],
        "chat-binding-fast-lane-kernel"
    );
    assert_eq!(captured[0].payload["limit"], json!(88));
    assert_eq!(captured[0].payload["allow_extended_limit"], json!(true));

    cleanup_chat_test_memory(&sqlite_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fast_lane_summary_output_accepts_legacy_schema_v1_events() {
    let (_config, memory_config, sqlite_path) = init_chat_test_memory("fast-lane-legacy");

    let payloads = legacy_fast_lane_tool_batch_event_payloads();
    append_assistant_payloads("chat-binding-fast-lane-legacy", &payloads, &memory_config);

    let output = load_fast_lane_summary_output(
        "chat-binding-fast-lane-legacy",
        32,
        ConversationRuntimeBinding::direct(),
        &memory_config,
    )
    .await
    .expect("load fast lane summary for legacy schema");

    assert!(output.contains("schema_version=1"));
    assert!(output.contains(
            "aggregate_execution configured_max_in_flight_avg=2.000 configured_max_in_flight_max=2 configured_max_in_flight_samples=1 observed_peak_in_flight_avg=- observed_peak_in_flight_max=- observed_peak_in_flight_samples=0 degraded_parallel_segments=0"
        ));
    assert!(output.contains(
            "aggregate_latency observed_wall_time_ms_avg=- observed_wall_time_ms_max=- observed_wall_time_ms_samples=0"
        ));
    assert!(output.contains(
            "latest_batch total_intents=3 parallel_enabled=true max_in_flight=2 observed_peak_in_flight=- observed_wall_time_ms=- parallel_safe_intents=2 serial_only_intents=1 parallel_segments=1 sequential_segments=1"
        ));
    assert!(
        output.contains("latest_segments=0:parallel_safe/parallel/2,1:serial_only/sequential/1")
    );

    cleanup_chat_test_memory(&sqlite_path);
}

#[test]
fn format_fast_lane_summary_includes_window_aggregates() {
    let summary = FastLaneToolBatchEventSummary {
        batch_events: 4,
        latest_schema_version: Some(2),
        latest_total_intents: Some(0),
        latest_parallel_execution_enabled: Some(false),
        latest_parallel_execution_max_in_flight: None,
        latest_observed_peak_in_flight: Some(1),
        latest_observed_wall_time_ms: Some(11),
        latest_parallel_safe_intents: Some(0),
        latest_serial_only_intents: Some(0),
        latest_parallel_segments: Some(0),
        latest_sequential_segments: Some(0),
        latest_segments: Vec::new(),
        parallel_execution_enabled_batches: 2,
        parallel_only_batches: 1,
        mixed_execution_batches: 1,
        sequential_only_batches: 1,
        batches_without_segments: 1,
        total_intents_seen: 8,
        total_parallel_safe_intents_seen: 5,
        total_serial_only_intents_seen: 3,
        total_parallel_segments_seen: 3,
        total_sequential_segments_seen: 3,
        parallel_execution_max_in_flight_samples: 3,
        parallel_execution_max_in_flight_sum: 6,
        parallel_execution_max_in_flight_max: Some(3),
        observed_peak_in_flight_samples: 3,
        observed_peak_in_flight_sum: 5,
        observed_peak_in_flight_max: Some(3),
        observed_wall_time_ms_samples: 3,
        observed_wall_time_ms_sum: 72,
        observed_wall_time_ms_max: Some(33),
        degraded_parallel_segments: 1,
        scheduling_class_counts: BTreeMap::from([
            ("parallel_safe".to_owned(), 3),
            ("serial_only".to_owned(), 3),
        ]),
        execution_mode_counts: BTreeMap::from([
            ("parallel".to_owned(), 3),
            ("sequential".to_owned(), 3),
        ]),
    };

    let output = format_fast_lane_summary("session-fast-lane", 64, &summary);

    assert!(output.contains("fast_lane_summary session=session-fast-lane limit=64"));
    assert!(output.contains(
            "aggregate_batches parallel_enabled=2 parallel_only=1 mixed=1 sequential_only=1 without_segments=1"
        ));
    assert!(output.contains(
            "aggregate_intents total=8 parallel_safe=5 serial_only=3 parallel_safe_ratio=0.625 serial_only_ratio=0.375"
        ));
    assert!(output.contains("aggregate_segments parallel=3 sequential=3"));
    assert!(output.contains(
            "aggregate_execution configured_max_in_flight_avg=2.000 configured_max_in_flight_max=3 configured_max_in_flight_samples=3 observed_peak_in_flight_avg=1.667 observed_peak_in_flight_max=3 observed_peak_in_flight_samples=3 degraded_parallel_segments=1"
        ));
    assert!(output.contains(
            "aggregate_latency observed_wall_time_ms_avg=24.000 observed_wall_time_ms_max=33 observed_wall_time_ms_samples=3"
        ));
    assert!(output.contains(
            "latest_batch total_intents=0 parallel_enabled=false max_in_flight=- observed_peak_in_flight=1 observed_wall_time_ms=11 parallel_safe_intents=0 serial_only_intents=0 parallel_segments=0 sequential_segments=0"
        ));
    assert!(output.contains("rollup scheduling_classes=parallel_safe:3,serial_only:3"));
    assert!(output.contains("rollup execution_modes=parallel:3,sequential:3"));
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turn_checkpoint_summary_output_accepts_explicit_runtime_binding() {
    let (config, memory_config, sqlite_path) = init_chat_test_memory("turn-checkpoint-output");

    let direct_payloads = turn_checkpoint_event_payloads();
    append_assistant_payloads(
        "chat-binding-turn-checkpoint-direct",
        &direct_payloads,
        &memory_config,
    );
    let coordinator = ConversationTurnCoordinator::new();
    let direct_output = load_turn_checkpoint_summary_output(
        &coordinator,
        &config,
        "chat-binding-turn-checkpoint-direct",
        96,
        ConversationRuntimeBinding::direct(),
    )
    .await
    .expect("load turn checkpoint summary via direct binding");
    assert!(direct_output.contains(
        "turn_checkpoint_summary session=chat-binding-turn-checkpoint-direct limit=96 checkpoints=2"
    ));
    assert!(direct_output.contains("state=finalized"));
    assert!(direct_output.contains("after_turn=completed"));
    assert!(direct_output.contains("compaction=skipped"));

    let kernel_payloads = turn_checkpoint_event_payloads();
    let (kernel_ctx, invocations) =
        build_kernel_context_with_window_turns(assistant_window_turns(&kernel_payloads));
    let kernel_output = load_turn_checkpoint_summary_output(
        &coordinator,
        &config,
        "chat-binding-turn-checkpoint-kernel",
        112,
        ConversationRuntimeBinding::kernel(&kernel_ctx),
    )
    .await
    .expect("load turn checkpoint summary via kernel binding");
    assert!(kernel_output.contains("turn_checkpoint_summary session=chat-binding-turn-checkpoint-kernel limit=112 checkpoints=2"));
    assert!(kernel_output.contains("state=finalized"));
    assert!(kernel_output.contains("after_turn=completed"));
    assert!(kernel_output.contains("compaction=skipped"));

    let captured = invocations.lock().expect("invocations lock");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].operation, crate::memory::MEMORY_OP_WINDOW);
    assert_eq!(
        captured[0].payload["session_id"],
        "chat-binding-turn-checkpoint-kernel"
    );
    assert_eq!(captured[0].payload["limit"], json!(112));
    assert_eq!(captured[0].payload["allow_extended_limit"], json!(true));

    cleanup_chat_test_memory(&sqlite_path);
}

#[test]
fn render_cli_chat_startup_lines_prioritize_first_turn_guidance() {
    let lines = render_cli_chat_startup_lines_with_width(
        &CliChatStartupSummary {
            config_path: "/tmp/loong.toml".to_owned(),
            memory_label: "/tmp/loong.db".to_owned(),
            session_id: "default".to_owned(),
            context_engine_id: "threaded".to_owned(),
            context_engine_source: "config".to_owned(),
            compaction_enabled: true,
            compaction_min_messages: Some(6),
            compaction_trigger_estimated_tokens: Some(120),
            compaction_preserve_recent_turns: 4,
            compaction_preserve_recent_estimated_tokens: Some(96),
            compaction_fail_open: false,
            acp_enabled: true,
            dispatch_enabled: true,
            conversation_routing: "automatic".to_owned(),
            allowed_channels: vec!["cli".to_owned()],
            acp_backend_id: "builtin".to_owned(),
            acp_backend_source: "default".to_owned(),
            explicit_acp_request: false,
            event_stream_enabled: false,
            bootstrap_mcp_servers: Vec::new(),
            working_directory: None,
        },
        80,
    );

    assert!(
        lines.first().is_some_and(|line| line.starts_with("LOONG")),
        "chat startup should now use the shared compact brand header: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line
                == "start here: Summarize this repository and suggest the best next step."),
        "chat startup should render the first answer handoff through the structured action group: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("command deck")),
        "chat startup should surface a command deck section beside the first answer handoff: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("note: how this surface works")),
        "chat startup should keep the usage guidance as a structured callout: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("session anchor")),
        "chat startup should keep session/config facts in a structured key-value section: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("runtime posture")),
        "chat startup should still preserve runtime context in a compact secondary section: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("continuity guardrails")),
        "chat startup should show compaction maintenance settings in a dedicated section: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("- session: default")),
        "chat startup should continue to show session identity after the handoff block: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("- compaction: true")),
        "chat startup should show whether automatic compaction is enabled: {lines:#?}"
    );
}

#[test]
fn render_cli_chat_startup_lines_surface_explicit_acp_overrides() {
    let lines = render_cli_chat_startup_lines_with_width(
        &CliChatStartupSummary {
            config_path: "/tmp/loong.toml".to_owned(),
            memory_label: "/tmp/loong.db".to_owned(),
            session_id: "thread-42".to_owned(),
            context_engine_id: "threaded".to_owned(),
            context_engine_source: "env".to_owned(),
            compaction_enabled: true,
            compaction_min_messages: Some(6),
            compaction_trigger_estimated_tokens: Some(120),
            compaction_preserve_recent_turns: 4,
            compaction_preserve_recent_estimated_tokens: Some(96),
            compaction_fail_open: false,
            acp_enabled: true,
            dispatch_enabled: true,
            conversation_routing: "manual".to_owned(),
            allowed_channels: vec!["cli".to_owned(), "telegram".to_owned()],
            acp_backend_id: "jsonrpc".to_owned(),
            acp_backend_source: "config".to_owned(),
            explicit_acp_request: true,
            event_stream_enabled: true,
            bootstrap_mcp_servers: vec!["filesystem".to_owned()],
            working_directory: Some("/workspace/project".to_owned()),
        },
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line.contains("note: acp overrides")),
        "chat startup should group ACP overrides under a dedicated callout heading: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- bootstrap MCP servers: filesystem")),
        "chat startup should still surface the bootstrap MCP override details: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- working directory: /workspace/project")),
        "chat startup should still surface the working directory override: {lines:#?}"
    );
}

#[test]
fn render_cli_chat_status_lines_focus_on_runtime_state_without_start_here() {
    let lines = render_cli_chat_status_lines_with_width(
        &CliChatStartupSummary {
            config_path: "/tmp/loong.toml".to_owned(),
            memory_label: "/tmp/loong.db".to_owned(),
            session_id: "default".to_owned(),
            context_engine_id: "threaded".to_owned(),
            context_engine_source: "config".to_owned(),
            compaction_enabled: true,
            compaction_min_messages: Some(6),
            compaction_trigger_estimated_tokens: Some(120),
            compaction_preserve_recent_turns: 4,
            compaction_preserve_recent_estimated_tokens: Some(96),
            compaction_fail_open: false,
            acp_enabled: true,
            dispatch_enabled: true,
            conversation_routing: "automatic".to_owned(),
            allowed_channels: vec!["cli".to_owned()],
            acp_backend_id: "builtin".to_owned(),
            acp_backend_source: "default".to_owned(),
            explicit_acp_request: false,
            event_stream_enabled: false,
            bootstrap_mcp_servers: Vec::new(),
            working_directory: None,
        },
        80,
    );

    assert_eq!(lines[0], "╭─ control deck · session=default");
    assert!(
        lines.iter().any(|line| line.contains("session anchor")),
        "status output should keep session facts grouped under a section: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("runtime posture")),
        "status output should keep runtime facts grouped under a section: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("continuity guardrails")),
        "status output should surface compaction maintenance settings: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("note: next moves")),
        "status output should include the operator control callout: {lines:#?}"
    );
    assert!(
        !lines.iter().any(|line| line.starts_with("start here:")),
        "status output should not re-render the first-turn guidance block: {lines:#?}"
    );
}

#[test]
fn should_run_missing_config_onboard_uses_default_yes_and_respects_decline() {
    assert!(should_run_missing_config_onboard(1, "\n"));
    assert!(should_run_missing_config_onboard(1, "yes\n"));
    assert!(!should_run_missing_config_onboard(1, "n\n"));
    assert!(!should_run_missing_config_onboard(0, ""));
}

#[test]
fn render_cli_chat_missing_config_lines_wrap_setup_prompt_in_surface() {
    let command = "loong onboard --output /tmp/loong.toml";
    let lines = render_cli_chat_missing_config_lines_with_width(command, 80);

    assert!(
        lines.first().is_some_and(|line| line.starts_with("LOONG")),
        "missing-config setup prompt should keep the shared compact header: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "setup required"),
        "missing-config setup prompt should promote the title into the shared screen surface: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "setup command: loong onboard --output /tmp/loong.toml"),
        "missing-config setup prompt should surface the setup command block: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line == "y) run setup wizard (recommended)"),
        "missing-config setup prompt should show the default acceptance choice explicitly: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line == "Press Enter to accept y."),
        "missing-config setup prompt should explain the default-enter behavior: {lines:#?}"
    );
}

#[test]
fn render_turn_checkpoint_startup_health_lines_surface_recovery_and_probe() {
    let summary = TurnCheckpointEventSummary {
        checkpoint_events: 1,
        latest_stage: Some(TurnCheckpointStage::PostPersist),
        latest_after_turn: Some(TurnCheckpointProgressStatus::Pending),
        latest_compaction: Some(TurnCheckpointProgressStatus::Pending),
        latest_lane: Some("safe".to_owned()),
        latest_result_kind: Some("tool_error".to_owned()),
        latest_persistence_mode: Some("success".to_owned()),
        latest_safe_lane_terminal_route: Some(crate::conversation::SafeLaneTerminalRouteSnapshot {
            decision: crate::conversation::SafeLaneFailureRouteDecision::Terminal,
            reason: crate::conversation::SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
            source: crate::conversation::SafeLaneFailureRouteSource::BackpressureGuard,
        }),
        latest_identity_present: Some(true),
        session_state: TurnCheckpointSessionState::PendingFinalization,
        checkpoint_durable: true,
        requires_recovery: true,
        reply_durable: true,
        ..TurnCheckpointEventSummary::default()
    };
    let probe = TurnCheckpointTailRepairRuntimeProbe::new(
            TurnCheckpointRecoveryAction::InspectManually,
            crate::conversation::TurnCheckpointTailRepairSource::Runtime,
            crate::conversation::TurnCheckpointTailRepairReason::CheckpointPreparationFingerprintMismatch,
        );
    let diagnostics = test_turn_checkpoint_diagnostics(summary, Some(probe));
    let lines =
        render_turn_checkpoint_startup_health_lines_with_width("session-health", &diagnostics, 80)
            .expect("startup health surface");

    assert_eq!(lines[0], "╭─ checkpoint · session=session-health");
    assert!(
        lines.iter().any(|line| line.contains("durability status")),
        "startup health should group durability facts under a shared key-value section: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("attention: recovery")),
        "startup health should surface pending recovery as a warning callout: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- action: inspect_manually")),
        "startup health should preserve the concrete recovery action in the callout: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("note: runtime probe")),
        "startup health should surface runtime probe context as a secondary structured callout: {lines:#?}"
    );
}

#[test]
fn render_turn_checkpoint_startup_health_lines_skip_non_durable_sessions() {
    let summary = TurnCheckpointEventSummary {
        session_state: TurnCheckpointSessionState::NotDurable,
        checkpoint_durable: false,
        reply_durable: false,
        ..TurnCheckpointEventSummary::default()
    };
    let diagnostics = test_turn_checkpoint_diagnostics(summary, None);

    let lines =
        render_turn_checkpoint_startup_health_lines_with_width("session-health", &diagnostics, 80);

    assert!(
        lines.is_none(),
        "startup health should stay quiet until a durable checkpoint exists"
    );
}

#[test]
fn render_turn_checkpoint_startup_health_lines_surface_non_durable_recovery() {
    let summary = TurnCheckpointEventSummary {
        session_state: TurnCheckpointSessionState::NotDurable,
        checkpoint_durable: false,
        reply_durable: false,
        requires_recovery: true,
        ..TurnCheckpointEventSummary::default()
    };
    let diagnostics = test_turn_checkpoint_diagnostics(summary, None);

    let lines =
        render_turn_checkpoint_startup_health_lines_with_width("session-health", &diagnostics, 80)
            .expect("non-durable recovery should still render");

    assert!(
        lines
            .iter()
            .any(|line| line.contains("recovery needed: yes")),
        "startup health should surface non-durable recovery cases: {lines:#?}"
    );
}

#[test]
fn render_turn_checkpoint_status_health_lines_surface_non_durable_sessions() {
    let summary = TurnCheckpointEventSummary {
        session_state: TurnCheckpointSessionState::NotDurable,
        checkpoint_durable: false,
        reply_durable: false,
        ..TurnCheckpointEventSummary::default()
    };
    let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
    let lines = ops::render_turn_checkpoint_status_health_lines_with_width(
        "session-health",
        &diagnostics,
        80,
    );

    assert_eq!(lines[0], "╭─ checkpoint · session=session-health");
    assert!(
        lines.iter().any(|line| line.contains("state: not_durable")),
        "status health should surface non-durable sessions explicitly: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("checkpoint durable: no")),
        "status health should surface checkpoint durability explicitly: {lines:#?}"
    );
}

#[test]
fn render_turn_checkpoint_status_health_lines_surface_compaction_diagnostics() {
    let summary = TurnCheckpointEventSummary {
        checkpoint_events: 1,
        latest_compaction_diagnostics: Some(crate::conversation::ContextCompactionDiagnostics {
            summary_turn_count: 4,
            retained_turn_count: 3,
            demoted_recent_turn_count: 1,
            total_turns: 7,
            assistant_turns: 3,
            low_signal_turns: 1,
            tool_result_line_prunes: 1,
            tool_outcome_record_prunes: 0,
        }),
        checkpoint_durable: true,
        reply_durable: true,
        ..TurnCheckpointEventSummary::default()
    };
    let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
    let lines = ops::render_turn_checkpoint_status_health_lines_with_width(
        "session-health",
        &diagnostics,
        80,
    );

    assert!(
        lines
            .iter()
            .any(|line| line.contains("compaction diagnostics")),
        "status health should surface compaction diagnostics as a dedicated section: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("summary turns: 4")),
        "status health should surface compaction rollup values: {lines:#?}"
    );
}

#[test]
fn render_fast_lane_summary_lines_surface_aggregates_and_segments() {
    let mut summary = FastLaneToolBatchEventSummary {
        batch_events: 2,
        total_intents_seen: 4,
        total_parallel_safe_intents_seen: 3,
        total_serial_only_intents_seen: 1,
        total_parallel_segments_seen: 2,
        total_sequential_segments_seen: 1,
        parallel_execution_max_in_flight_samples: 1,
        parallel_execution_max_in_flight_sum: 4,
        observed_peak_in_flight_samples: 1,
        observed_peak_in_flight_sum: 3,
        observed_wall_time_ms_samples: 1,
        observed_wall_time_ms_sum: 120,
        latest_schema_version: Some(3),
        latest_total_intents: Some(2),
        latest_parallel_execution_enabled: Some(true),
        latest_parallel_execution_max_in_flight: Some(4),
        latest_observed_peak_in_flight: Some(3),
        latest_observed_wall_time_ms: Some(120),
        latest_parallel_safe_intents: Some(2),
        latest_serial_only_intents: Some(0),
        latest_parallel_segments: Some(1),
        latest_sequential_segments: Some(0),
        latest_segments: vec![FastLaneToolBatchSegmentSnapshot {
            segment_index: 0,
            scheduling_class: "parallel_safe".to_owned(),
            execution_mode: "parallel".to_owned(),
            intent_count: 2,
            observed_peak_in_flight: Some(3),
            observed_wall_time_ms: Some(120),
        }],
        ..FastLaneToolBatchEventSummary::default()
    };
    summary
        .scheduling_class_counts
        .insert("parallel_safe".to_owned(), 2);
    summary
        .execution_mode_counts
        .insert("parallel".to_owned(), 2);

    let lines = render_fast_lane_summary_lines_with_width("session-fast", 64, &summary, 80);

    assert_eq!(lines[0], "╭─ fast-lane · session=session-fast limit=64");
    assert!(
        lines.iter().any(|line| line.contains("intent mix")),
        "fast-lane summary should promote aggregate intent counters into a titled section: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("latest segments")),
        "fast-lane summary should keep the latest segment narrative visible: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| {
            line.contains(
                "- segment 0: class=parallel_safe mode=parallel intents=2 peak=3 wall_ms=120",
            )
        }),
        "fast-lane summary should render latest segment details as readable surface lines: {lines:#?}"
    );
}

#[test]
fn render_safe_lane_summary_lines_surface_health_and_rollups() {
    let config = ConversationConfig::default();
    let mut summary = SafeLaneEventSummary {
        lane_selected_events: 1,
        round_started_events: 2,
        round_completed_succeeded_events: 1,
        round_completed_failed_events: 1,
        verify_failed_events: 1,
        replan_triggered_events: 1,
        final_status_events: 1,
        final_status: Some(SafeLaneFinalStatus::Failed),
        final_failure_code: Some("safe_lane_plan_verify_failed".to_owned()),
        final_route_decision: Some("terminal".to_owned()),
        final_route_reason: Some("session_governor_no_replan".to_owned()),
        latest_metrics: Some(crate::conversation::SafeLaneMetricsSnapshot {
            rounds_started: 2,
            rounds_succeeded: 1,
            rounds_failed: 1,
            verify_failures: 1,
            replans_triggered: 1,
            total_attempts_used: 3,
        }),
        tool_output_snapshots_seen: 2,
        tool_output_truncated_events: 1,
        tool_output_result_lines_total: 3,
        tool_output_truncated_result_lines_total: 1,
        tool_output_aggregate_truncation_ratio_milli: Some(333),
        ..SafeLaneEventSummary::default()
    };
    summary
        .route_decision_counts
        .insert("terminal".to_owned(), 1);
    summary
        .route_reason_counts
        .insert("session_governor_no_replan".to_owned(), 1);
    summary
        .failure_code_counts
        .insert("safe_lane_plan_verify_failed".to_owned(), 1);

    let lines =
        render_safe_lane_summary_lines_with_width("session-safe", 32, &config, &summary, 80);

    assert_eq!(lines[0], "╭─ safe-lane · session=session-safe limit=32");
    assert!(
        lines.iter().any(|line| line.contains("attention: health")),
        "safe-lane summary should surface warning health as a structured callout: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- severity: critical")),
        "safe-lane health callout should preserve the derived severity: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("rollups")),
        "safe-lane summary should keep the route and failure rollups in a dedicated section: {lines:#?}"
    );
}

#[test]
fn render_turn_checkpoint_summary_lines_surface_runtime_probe() {
    let summary = TurnCheckpointEventSummary {
        checkpoint_events: 2,
        post_persist_events: 1,
        finalized_events: 1,
        latest_stage: Some(TurnCheckpointStage::FinalizationFailed),
        latest_after_turn: Some(TurnCheckpointProgressStatus::Completed),
        latest_compaction: Some(TurnCheckpointProgressStatus::Failed),
        latest_lane: Some("fast".to_owned()),
        latest_result_kind: Some("final_text".to_owned()),
        latest_persistence_mode: Some("success".to_owned()),
        latest_identity_present: Some(true),
        session_state: TurnCheckpointSessionState::FinalizationFailed,
        checkpoint_durable: true,
        requires_recovery: true,
        reply_durable: true,
        ..TurnCheckpointEventSummary::default()
    };
    let probe = TurnCheckpointTailRepairRuntimeProbe::new(
            TurnCheckpointRecoveryAction::InspectManually,
            crate::conversation::TurnCheckpointTailRepairSource::Runtime,
            crate::conversation::TurnCheckpointTailRepairReason::CheckpointPreparationFingerprintMismatch,
        );
    let diagnostics = test_turn_checkpoint_diagnostics(summary, Some(probe));
    let lines =
        render_turn_checkpoint_summary_lines_with_width("session-summary", 64, &diagnostics, 80);

    assert_eq!(lines[0], "╭─ checkpoint · session=session-summary limit=64");
    assert!(
        lines.iter().any(|line| line.contains("summary")),
        "turn checkpoint summary should group the latest durability state in a titled section: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("note: runtime probe")),
        "turn checkpoint summary should append runtime probe context as a structured callout: {lines:#?}"
    );
}

#[test]
fn render_turn_checkpoint_repair_lines_surface_manual_result() {
    let summary = TurnCheckpointEventSummary {
        checkpoint_events: 1,
        latest_stage: Some(TurnCheckpointStage::PostPersist),
        session_state: TurnCheckpointSessionState::PendingFinalization,
        checkpoint_durable: true,
        requires_recovery: true,
        reply_durable: true,
        ..TurnCheckpointEventSummary::default()
    };
    let outcome = crate::conversation::TurnCheckpointTailRepairOutcome::from_summary(
        crate::conversation::TurnCheckpointTailRepairStatus::ManualRequired,
        TurnCheckpointRecoveryAction::InspectManually,
        Some(crate::conversation::TurnCheckpointTailRepairSource::Summary),
        crate::conversation::TurnCheckpointTailRepairReason::CheckpointIdentityMissing,
        &summary,
    );
    let lines = render_turn_checkpoint_repair_lines_with_width("session-repair", &outcome, 80);

    assert_eq!(lines[0], "╭─ repair · session=session-repair");
    assert!(
        lines.iter().any(|line| line.contains("repair status")),
        "turn checkpoint repair should group repair facts in a structured key-value section: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("attention: repair result")),
        "manual repair outcomes should surface a warning callout: {lines:#?}"
    );
}

#[test]
fn render_cli_chat_help_lines_promotes_commands_to_surface() {
    let lines = render_cli_chat_help_lines_with_width(72);

    assert_eq!(lines[0], "╭─ help · operator deck");
    assert!(
        lines.iter().any(|line| line.contains("slash commands")),
        "help output should keep a dedicated slash-command section: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("/history: print the current session sliding window")),
        "help output should render slash commands as readable key-value rows: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| { line.contains("/review: reopen the latest approval/review summary") }),
        "help output should surface the review command: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("/mission:") && line.contains("mission control"))
            || lines.iter().any(|line| line.contains("/mission:"))
                && lines
                    .iter()
                    .any(|line| line.contains("current session scope")),
        "help output should surface the mission-control command: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("/sessions: inspect visible sessions")),
        "help output should surface the session queue command: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| { line.contains("/workers: inspect visible worker/delegate sessions") }),
        "help output should surface the workers command: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line
            .contains("/status: show session, runtime, compaction, and durability status")),
        "help output should surface the status command: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line
            .contains("/compact: write a continuity-safe checkpoint into the active window")),
        "help output should surface the manual compaction command: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("note: surface controls")),
        "help output should surface the command-menu control deck: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("keyboard")),
        "help output should surface keyboard shortcuts as a first-class section: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("note: usage notes")),
        "help output should preserve operator guidance as a callout: {lines:#?}"
    );
}

#[test]
fn render_cli_chat_command_usage_lines_wrap_usage_in_warning_card() {
    let lines = render_cli_chat_command_usage_lines_with_width("usage: /history", 72);

    assert_eq!(lines[0], "╭─ chat · command");
    assert!(
        lines.iter().any(|line| line.contains("attention: usage")),
        "usage errors should render inside a warning pane: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("usage: /history")),
        "usage pane should preserve the concrete command usage: {lines:#?}"
    );
}

#[test]
fn render_cli_chat_status_lines_surface_runtime_and_compaction_controls() {
    let summary = CliChatStartupSummary {
        config_path: "/tmp/loong.toml".to_owned(),
        memory_label: "window_plus_summary".to_owned(),
        session_id: "session-status".to_owned(),
        context_engine_id: "default".to_owned(),
        context_engine_source: "config".to_owned(),
        compaction_enabled: true,
        compaction_min_messages: Some(6),
        compaction_trigger_estimated_tokens: Some(12_000),
        compaction_preserve_recent_turns: 4,
        compaction_preserve_recent_estimated_tokens: Some(4_096),
        compaction_fail_open: true,
        acp_enabled: true,
        dispatch_enabled: true,
        conversation_routing: "auto".to_owned(),
        allowed_channels: vec!["cli".to_owned()],
        acp_backend_id: "builtin".to_owned(),
        acp_backend_source: "config".to_owned(),
        explicit_acp_request: false,
        event_stream_enabled: false,
        bootstrap_mcp_servers: Vec::new(),
        working_directory: None,
    };

    let lines = render_cli_chat_status_lines_with_width(&summary, 80);

    assert_eq!(lines[0], "╭─ control deck · session=session-status");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("continuity guardrails")),
        "status output should expose compaction settings as a dedicated section: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("compaction: true")),
        "status output should expose compaction enablement: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("trigger tokens: 12000")),
        "status output should surface the compaction token trigger: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("preserve recent tokens: 4096")),
        "status output should surface the recent-tail token budget: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("note: next moves")),
        "status output should append the operator controls callout: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("checkpoint the active session window on demand")),
        "status output should direct operators toward manual compaction: {lines:#?}"
    );
}

#[test]
fn render_manual_compaction_lines_surface_structured_result() {
    let result = ManualCompactionResult {
        status: ManualCompactionStatus::Applied,
        before_turns: 8,
        after_turns: 3,
        estimated_tokens_before: Some(1200),
        estimated_tokens_after: Some(420),
        summary_headline: Some("Compacted 6 earlier turns".to_owned()),
        prune_summary: Some(
            "summary:6 retained:3 demoted:1 low_signal:2 tool_results:1 tool_outcomes:0"
                .to_owned(),
        ),
        detail: "Compacted 6 earlier turns. Session-local recall only. It does not replace Runtime Self Context.".to_owned(),
    };

    let lines = render_manual_compaction_lines_with_width("session-compact", &result, 80);

    assert_eq!(lines[0], "╭─ compact · session=session-compact");
    assert!(
        lines.iter().any(|line| line.contains("compaction result")),
        "manual compaction should render a dedicated result section: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("status: applied")),
        "manual compaction should surface the applied status: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("tokens after: 420")),
        "manual compaction should surface token estimates: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("prune: summary:6 retained:3 demoted:1")),
        "manual compaction should surface the prune summary rollup: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Runtime Self Context")),
        "manual compaction should preserve the continuity boundary detail: {lines:#?}"
    );
}

#[test]
fn render_cli_chat_history_lines_wrap_history_in_surface() {
    let history_lines = vec![
        "user: summarize the current repo".to_owned(),
        "assistant: start with the daemon crate".to_owned(),
    ];
    let lines = render_cli_chat_history_lines_with_width("session-7", 24, &history_lines, 72);

    assert_eq!(lines[0], "╭─ history · session=session-7 limit=24");
    assert!(
        lines.iter().any(|line| line.contains("sliding window")),
        "history output should keep a dedicated window section: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("user: summarize the current repo")),
        "history output should still surface the original transcript entries: {lines:#?}"
    );
}

#[test]
fn render_cli_chat_assistant_lines_promotes_markdown_to_structured_sections() {
    let assistant_text = "\
## Plan

- inspect the active config
* compare runtime state
> reuse current provider settings when safe

```rust
let value = input.trim();
println!(\"{value}\");
```";
    let lines = render_cli_chat_assistant_lines_with_width(assistant_text, 72);

    assert_eq!(lines[0], "╭─ loong · reply");
    assert!(
        lines.iter().any(|line| line.contains("Plan")),
        "markdown headings should become section titles: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- inspect the active config")),
        "markdown list items should remain visible in the narrative block: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- compare runtime state")),
        "markdown star bullets should normalize into wrapped display bullets: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("note: quoted context")),
        "markdown blockquotes should render as structured callouts: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("code [rust]")),
        "markdown fences should render as preformatted sections: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("let value = input.trim();")),
        "preformatted sections should keep code indentation intact: {lines:#?}"
    );
}

#[test]
fn render_cli_chat_assistant_lines_preserve_heading_before_quotes_and_at_eof() {
    let assistant_text = "\
## Risks
> keep credentials in env vars

## Next";
    let lines = render_cli_chat_assistant_lines_with_width(assistant_text, 72);

    assert!(
        lines.iter().any(|line| line.contains("note: Risks")),
        "headings should stay attached to quoted sections instead of falling back to a generic title: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("- keep credentials in env vars")),
        "quoted content should stay visible after preserving the heading: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("Next")),
        "a trailing heading should still render even when it has no body lines yet: {lines:#?}"
    );
}

#[test]
fn parse_cli_chat_markdown_sections_promotes_reasoning_heading_to_callout() {
    let sections = parse_cli_chat_markdown_sections(
        "## Reasoning\nThe provider compared two options before choosing one.",
    );
    assert!(matches!(
        sections.first(),
        Some(TuiSectionSpec::Callout { title, .. }) if title.as_deref() == Some("reasoning")
    ));
}

#[test]
fn parse_cli_chat_markdown_sections_promotes_tool_activity_heading_to_callout() {
    let sections =
        parse_cli_chat_markdown_sections("## Tool Activity\nread completed with 1 result line.");
    assert!(matches!(
        sections.first(),
        Some(TuiSectionSpec::Callout { title, .. }) if title.as_deref() == Some("tool activity")
    ));
}

#[test]
fn render_cli_chat_assistant_lines_promotes_tool_approval_to_choice_screen() {
    let assistant_text = "\
我准备调用 provider.switch 来切换后续会话的 provider。
[tool_approval_required]
tool: provider.switch
request_id: apr_provider_switch
rule_id: session_tool_consent_auto_blocked
reason: `provider.switch` is not eligible for auto mode and needs operator confirmation
allowed_decisions: yes / auto / full / esc";
    let lines = render_cli_chat_assistant_lines_with_width(assistant_text, 72);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("准备调用 provider.switch")),
        "approval replies should render as a dedicated screen title: {lines:#?}"
    );
    let first_choice_visible = lines.iter().any(|line| line.trim_start().starts_with("1)"));
    let second_choice_visible = lines.iter().any(|line| line.trim_start().starts_with("2)"));

    assert!(
        first_choice_visible && second_choice_visible,
        "approval choice screen should expose numbered choices in order: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("yes / auto / full / esc")),
        "approval choice screen should keep the raw keyword controls visible: {lines:#?}"
    );
}

#[test]
fn render_cli_chat_live_surface_lines_show_pipeline_status_and_preview() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RequestingProvider,
        provider_round: Some(1),
        lane: None,
        tool_call_count: 0,
        message_count: Some(4),
        estimated_tokens: Some(128),
        first_token_latency_ms: Some(123),
        draft_preview: Some("Inspecting the repo layout...".to_owned()),
        tools: Vec::new(),
    };
    let lines = render_cli_chat_live_surface_lines_with_width(&snapshot, 72);

    assert_eq!(
        lines[0],
        "╭─ loong · live · round 1 · 4 msgs · ~128 tok · ttft 123ms"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("note: querying model")),
        "live surface should explain the active phase through a callout: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("turn pipeline")),
        "live surface should keep the pipeline checklist visible: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| {
            line.contains("[WARN] call model") && line.contains("first token in 123 ms")
        }),
        "live surface should keep the model step actively highlighted: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("first token") && line.contains("123 ms")),
        "live surface should surface first-token latency in status rows: {lines:#?}"
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("streaming provider round 1")),
        "live surface should avoid claiming streaming when the snapshot does not encode that capability: {lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("draft preview")),
        "live surface should surface partial text as a dedicated preview block: {lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Inspecting the repo layout...")),
        "live surface should preserve the partial preview text: {lines:#?}"
    );
}

#[test]
fn cli_chat_live_surface_observer_emits_phase_and_stream_preview_batches() {
    let captured_batches = Arc::new(StdMutex::new(Vec::<Vec<String>>::new()));
    let render_sink: CliChatLiveSurfaceSink = {
        let captured_batches = Arc::clone(&captured_batches);
        Arc::new(move |lines| {
            let mut batches = captured_batches
                .lock()
                .expect("captured batches lock should not be poisoned");
            batches.push(lines);
        })
    };
    let observer = CliChatLiveSurfaceObserver::new(72, render_sink);

    observer.on_phase(ConversationTurnPhaseEvent::preparing());
    observer.on_phase(ConversationTurnPhaseEvent::requesting_provider(
        1,
        3,
        Some(96),
    ));
    observer.on_streaming_token(crate::acp::StreamingTokenEvent {
        event_type: "text_delta".to_owned(),
        delta: crate::acp::TokenDelta {
            text: Some("Draft response".to_owned()),
            tool_call: None,
        },
        index: None,
        elapsed_ms: Some(42),
    });

    let batches = captured_batches
        .lock()
        .expect("captured batches lock should not be poisoned");
    assert!(
        batches.len() >= 3,
        "observer should emit both phase updates and the first preview update: {batches:#?}"
    );

    let preview_batch = batches
        .iter()
        .find(|lines| lines.iter().any(|line| line.contains("draft preview")))
        .expect("preview batch");
    assert!(
        preview_batch
            .iter()
            .any(|line| line.contains("Draft response")),
        "preview batch should include the streamed text: {preview_batch:#?}"
    );
    assert!(
        preview_batch.iter().any(|line| line.contains("ttft 42ms")),
        "preview batch should include the first-token latency in the title: {preview_batch:#?}"
    );
}

#[test]
fn cli_chat_live_surface_observer_renders_tool_lifecycle_updates() {
    let captured_batches = Arc::new(StdMutex::new(Vec::<Vec<String>>::new()));
    let render_sink: CliChatLiveSurfaceSink = {
        let captured_batches = Arc::clone(&captured_batches);
        Arc::new(move |lines| {
            let mut batches = captured_batches
                .lock()
                .expect("captured batches lock should not be poisoned");
            batches.push(lines);
        })
    };
    let observer = CliChatLiveSurfaceObserver::new(72, render_sink);

    observer.on_phase(ConversationTurnPhaseEvent::running_tools(
        1,
        ExecutionLane::Fast,
        1,
    ));
    observer.on_streaming_token(crate::acp::StreamingTokenEvent {
        event_type: "tool_call_start".to_owned(),
        delta: crate::acp::TokenDelta {
            text: None,
            tool_call: Some(crate::acp::ToolCallDelta {
                name: Some("file.read".to_owned()),
                args: None,
                id: Some("call-tool-1".to_owned()),
            }),
        },
        index: Some(0),
        elapsed_ms: None,
    });
    observer.on_streaming_token(crate::acp::StreamingTokenEvent {
        event_type: "tool_call_input_delta".to_owned(),
        delta: crate::acp::TokenDelta {
            text: None,
            tool_call: Some(crate::acp::ToolCallDelta {
                name: None,
                args: Some("{\"path\":\"README.md\"}".to_owned()),
                id: None,
            }),
        },
        index: Some(0),
        elapsed_ms: None,
    });
    observer.on_tool(ConversationTurnToolEvent::completed(
        "call-tool-1",
        "file.read",
        Some("ok".to_owned()),
    ));

    let batches = captured_batches
        .lock()
        .expect("captured batches lock should not be poisoned");
    let running_batch = batches
        .iter()
        .find(|lines| lines.iter().any(|line| line.contains("tool activity")))
        .expect("running tool batch");
    let completed_batch = batches
        .iter()
        .rev()
        .find(|lines| {
            lines
                .iter()
                .any(|line| line.contains("[completed] read (id=call-tool-1) - ok"))
        })
        .expect("completed tool batch");

    assert!(
        running_batch
            .iter()
            .any(|line| line.contains("[running] read (id=call-tool-1)")),
        "tool batch should surface the running tool state: {running_batch:#?}"
    );

    assert!(
        completed_batch
            .iter()
            .any(|line| line.contains("[completed] read (id=call-tool-1) - ok")),
        "tool batch should surface the completed tool state: {completed_batch:#?}"
    );
    assert!(
        completed_batch
            .iter()
            .any(|line| line.contains("args: {\"path\":\"README.md\"}")),
        "tool batch should preserve streamed tool args: {completed_batch:#?}"
    );
}

#[test]
fn cli_chat_live_surface_observer_renders_runtime_output_and_file_change_updates() {
    let captured_batches = Arc::new(StdMutex::new(Vec::<Vec<String>>::new()));
    let render_sink: CliChatLiveSurfaceSink = {
        let captured_batches = Arc::clone(&captured_batches);
        Arc::new(move |lines| {
            let mut batches = captured_batches
                .lock()
                .expect("captured batches lock should not be poisoned");
            batches.push(lines);
        })
    };
    let observer = CliChatLiveSurfaceObserver::new(72, render_sink);

    observer.on_phase(ConversationTurnPhaseEvent::running_tools(
        1,
        ExecutionLane::Fast,
        1,
    ));
    observer.on_tool(
        ConversationTurnToolEvent::running("call-tool-2", "shell.exec")
            .with_request_summary(Some("{\"command\":\"printf\"}".to_owned())),
    );
    observer.on_runtime(ConversationTurnRuntimeEvent::new(
        "call-tool-2",
        ToolRuntimeEvent::OutputDelta(ToolOutputDelta {
            stream: ToolRuntimeStream::Stdout,
            chunk: "first line\nsecond line".to_owned(),
            total_bytes: 22,
            total_lines: 2,
            truncated: false,
        }),
    ));
    observer.on_runtime(ConversationTurnRuntimeEvent::new(
        "call-tool-2",
        ToolRuntimeEvent::FileChangePreview(ToolFileChangePreview {
            path: "src/lib.rs".to_owned(),
            kind: ToolFileChangeKind::Edit,
            added_lines: 2,
            removed_lines: 1,
            preview: Some("@@ -1,1 +1,2 @@\n-old\n+new\n+line".to_owned()),
        }),
    ));
    observer.on_runtime(ConversationTurnRuntimeEvent::new(
        "call-tool-2",
        ToolRuntimeEvent::CommandMetrics(ToolCommandMetrics {
            exit_code: Some(0),
            duration_ms: 42,
        }),
    ));
    observer.on_tool(ConversationTurnToolEvent::completed(
        "call-tool-2",
        "shell.exec",
        Some("ok".to_owned()),
    ));

    let batches = captured_batches
        .lock()
        .expect("captured batches lock should not be poisoned");
    let final_batch = batches.last().expect("final runtime batch");

    assert!(
        final_batch
            .iter()
            .any(|line| line.contains("[completed] exec (id=call-tool-2) - ok")),
        "runtime output should surface the visible tool name: {final_batch:#?}"
    );
    assert!(
        final_batch
            .iter()
            .any(|line| line.contains("stdout: 2 lines · 22 bytes")),
        "runtime output should surface stdout counters: {final_batch:#?}"
    );
    assert!(
        final_batch.iter().any(|line| line.contains("first line")),
        "runtime output should retain stdout preview lines: {final_batch:#?}"
    );
    assert!(
        final_batch
            .iter()
            .any(|line| line.contains("file: edit src/lib.rs (+2 / -1)")),
        "runtime output should surface file change summaries: {final_batch:#?}"
    );
    assert!(
        final_batch
            .iter()
            .any(|line| line.contains("metrics: 42ms · exit=0")),
        "runtime output should surface command metrics: {final_batch:#?}"
    );
}

#[test]
fn build_cli_chat_live_surface_snapshot_preserves_structured_tool_state() {
    let mut state = CliChatLiveSurfaceState {
        latest_phase_event: Some(ConversationTurnPhaseEvent::running_tools(
            1,
            ExecutionLane::Fast,
            1,
        )),
        first_token_latency_ms: Some(88),
        ..CliChatLiveSurfaceState::default()
    };

    let tool_state = ensure_cli_chat_live_tool_state(&mut state, "call-structured");
    tool_state.name = Some("shell.exec".to_owned());
    tool_state.request_summary = Some("{\"command\":\"printf\"}".to_owned());
    tool_state.args = "{\"command\":\"printf\"}".to_owned();
    tool_state.stdout = CliChatLiveOutputView {
        text: "hello".to_owned(),
        total_bytes: 5,
        total_lines: 1,
        truncated: false,
    };
    tool_state.duration_ms = Some(12);
    tool_state.exit_code = Some(0);

    let snapshot =
        build_cli_chat_live_surface_snapshot(&state).expect("snapshot should be available");
    let tool = snapshot
        .tools
        .first()
        .expect("snapshot should include one tool");

    assert_eq!(snapshot.tools.len(), 1);
    assert_eq!(tool.tool_call_id, "call-structured");
    assert_eq!(tool.name.as_deref(), Some("exec"));
    assert_eq!(
        tool.request_summary.as_deref(),
        Some("{\"command\":\"printf\"}")
    );
    assert_eq!(tool.args, "{\"command\":\"printf\"}");
    assert_eq!(tool.stdout.text, "hello");
    assert_eq!(tool.duration_ms, Some(12));
    assert_eq!(tool.exit_code, Some(0));
    assert_eq!(snapshot.first_token_latency_ms, Some(88));
}

#[test]
fn build_cli_chat_live_surface_snapshot_keeps_precise_hidden_tool_names() {
    let mut state = CliChatLiveSurfaceState {
        latest_phase_event: Some(ConversationTurnPhaseEvent::running_tools(
            1,
            ExecutionLane::Fast,
            1,
        )),
        ..CliChatLiveSurfaceState::default()
    };

    let tool_state = ensure_cli_chat_live_tool_state(&mut state, "call-hidden");
    tool_state.name = Some("delegate_async".to_owned());

    let snapshot =
        build_cli_chat_live_surface_snapshot(&state).expect("snapshot should be available");
    let tool = snapshot
        .tools
        .first()
        .expect("snapshot should include one tool");

    assert_eq!(tool.name.as_deref(), Some("delegate_async"));
}

#[test]
fn parse_markdown_heading_follows_commonmark_atx_rules() {
    assert_eq!(parse_markdown_heading("## Plan"), Some("Plan"));
    assert_eq!(parse_markdown_heading("### Plan ###"), Some("Plan"));
    assert_eq!(parse_markdown_heading("## C#"), Some("C#"));
    assert_eq!(parse_markdown_heading("#NoSpace"), None);
    assert_eq!(parse_markdown_heading("#!/bin/bash"), None);
    assert_eq!(parse_markdown_heading("####### too many"), None);
}

#[test]
fn cli_chat_live_surface_observer_resets_request_scoped_buffers_between_rounds() {
    let captured_batches = Arc::new(StdMutex::new(Vec::<Vec<String>>::new()));
    let render_sink: CliChatLiveSurfaceSink = {
        let captured_batches = Arc::clone(&captured_batches);
        Arc::new(move |lines| {
            let mut batches = captured_batches
                .lock()
                .expect("captured batches lock should not be poisoned");
            batches.push(lines);
        })
    };
    let observer = CliChatLiveSurfaceObserver::new(72, render_sink);

    observer.on_phase(ConversationTurnPhaseEvent::requesting_provider(
        1,
        3,
        Some(96),
    ));
    observer.on_streaming_token(crate::acp::StreamingTokenEvent {
        event_type: "text_delta".to_owned(),
        delta: crate::acp::TokenDelta {
            text: Some("Draft response".to_owned()),
            tool_call: None,
        },
        index: None,
        elapsed_ms: Some(55),
    });
    observer.on_streaming_token(crate::acp::StreamingTokenEvent {
        event_type: "tool_call_input_delta".to_owned(),
        delta: crate::acp::TokenDelta {
            text: None,
            tool_call: Some(crate::acp::ToolCallDelta {
                name: None,
                args: Some("{\"query\":\"rust\"}".to_owned()),
                id: None,
            }),
        },
        index: Some(0),
        elapsed_ms: None,
    });
    observer.on_phase(ConversationTurnPhaseEvent::requesting_followup_provider(
        2,
        ExecutionLane::Fast,
        1,
        5,
        Some(128),
    ));

    let batches = captured_batches
        .lock()
        .expect("captured batches lock should not be poisoned");
    let last_batch = batches.last().expect("follow-up request batch");

    assert!(
        !last_batch.iter().any(|line| line.contains("draft preview")),
        "follow-up provider requests should reset the previous draft preview: {last_batch:#?}"
    );
    assert!(
        !last_batch.iter().any(|line| line.contains("tool activity")),
        "follow-up provider requests should not reuse prior tool activity lines: {last_batch:#?}"
    );
    assert!(
        !last_batch.iter().any(|line| line.contains("ttft 55ms")),
        "follow-up provider requests should reset prior first-token latency: {last_batch:#?}"
    );
    assert!(
        !last_batch
            .iter()
            .any(|line| line.contains("Draft response")),
        "follow-up provider requests should not carry the previous request preview text: {last_batch:#?}"
    );
}

#[test]
fn cli_chat_live_surface_observer_waits_for_tools_phase_before_rendering_tool_activity() {
    let captured_batches = Arc::new(StdMutex::new(Vec::<Vec<String>>::new()));
    let render_sink: CliChatLiveSurfaceSink = {
        let captured_batches = Arc::clone(&captured_batches);
        Arc::new(move |lines| {
            let mut batches = captured_batches
                .lock()
                .expect("captured batches lock should not be poisoned");
            batches.push(lines);
        })
    };
    let observer = CliChatLiveSurfaceObserver::new(72, render_sink);

    observer.on_phase(ConversationTurnPhaseEvent::requesting_provider(
        1,
        3,
        Some(96),
    ));

    let batch_count_before_tool_delta = captured_batches
        .lock()
        .expect("captured batches lock should not be poisoned")
        .len();

    observer.on_streaming_token(crate::acp::StreamingTokenEvent {
        event_type: "tool_call_start".to_owned(),
        delta: crate::acp::TokenDelta {
            text: None,
            tool_call: Some(crate::acp::ToolCallDelta {
                name: Some("search".to_owned()),
                args: None,
                id: Some("call_123".to_owned()),
            }),
        },
        index: Some(0),
        elapsed_ms: None,
    });

    let batch_count_after_tool_delta = captured_batches
        .lock()
        .expect("captured batches lock should not be poisoned")
        .len();
    assert_eq!(
        batch_count_after_tool_delta, batch_count_before_tool_delta,
        "tool-call deltas should wait for the tools phase before re-rendering"
    );

    observer.on_phase(ConversationTurnPhaseEvent::running_tools(
        1,
        ExecutionLane::Fast,
        1,
    ));

    let batches = captured_batches
        .lock()
        .expect("captured batches lock should not be poisoned");
    let last_batch = batches.last().expect("running-tools batch");

    assert!(
        last_batch.iter().any(|line| line.contains("tool activity")),
        "the tools phase should render the accumulated tool activity: {last_batch:#?}"
    );
    assert!(
        last_batch
            .iter()
            .any(|line| line.contains("[running] search (id=call_123)")),
        "the tools phase should surface the streamed tool metadata: {last_batch:#?}"
    );
}

#[test]
#[cfg(feature = "config-toml")]
fn reload_cli_turn_config_refreshes_provider_state_without_mutating_cli_settings() {
    let path = std::env::temp_dir().join(format!(
        "loong-chat-provider-reload-{}.toml",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let path_string = path.display().to_string();

    let mut in_memory = LoongConfig::default();
    in_memory.cli.exit_commands = vec!["/bye".to_owned()];
    let mut openai =
        crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
    openai.model = "gpt-5".to_owned();
    in_memory.set_active_provider_profile(
        "openai-gpt-5",
        crate::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: openai,
        },
    );

    let mut on_disk = in_memory.clone();
    on_disk.cli.exit_commands = vec!["/different".to_owned()];
    let mut deepseek =
        crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Deepseek);
    deepseek.model = "deepseek-chat".to_owned();
    on_disk.providers.insert(
        "deepseek-chat".to_owned(),
        crate::config::ProviderProfileConfig {
            default_for_kind: true,
            provider: deepseek.clone(),
        },
    );
    on_disk.provider = deepseek;
    on_disk.active_provider = Some("deepseek-chat".to_owned());
    crate::config::write(Some(&path_string), &on_disk, true).expect("write config fixture");

    let reloaded = reload_cli_turn_config(&in_memory, path.as_path()).expect("reload");
    assert_eq!(reloaded.active_provider_id(), Some("deepseek-chat"));
    assert_eq!(reloaded.provider.model, "deepseek-chat");
    assert_eq!(reloaded.cli.exit_commands, vec!["/bye".to_owned()]);

    let _ = std::fs::remove_file(path);
}

#[test]
fn parse_summary_limit_accepts_aliases_and_preserves_usage_text() {
    assert_eq!(
        parse_summary_limit(
            "/fast_lane_summary",
            20,
            &["/fast_lane_summary", "/fast-lane-summary"],
        )
        .expect("parse"),
        Some(80)
    );
    assert_eq!(
        parse_summary_limit(
            "/fast-lane-summary 144",
            20,
            &["/fast_lane_summary", "/fast-lane-summary"],
        )
        .expect("parse"),
        Some(144)
    );
    assert_eq!(
        parse_summary_limit(
            "/other_summary",
            20,
            &["/fast_lane_summary", "/fast-lane-summary"],
        )
        .expect("parse"),
        None
    );

    let error = parse_summary_limit(
        "/fast_lane_summary 0",
        20,
        &["/fast_lane_summary", "/fast-lane-summary"],
    )
    .expect_err("zero limit should be rejected");
    assert_eq!(
        error,
        "invalid /fast_lane_summary limit `0`; usage: /fast_lane_summary [limit]"
    );

    let error = parse_summary_limit(
        "/fast_lane_summary nope",
        20,
        &["/fast_lane_summary", "/fast-lane-summary"],
    )
    .expect_err("non-number limit should be rejected");
    assert!(error.contains("invalid /fast_lane_summary limit `nope`"));
    assert!(error.contains("usage: /fast_lane_summary [limit]"));

    let error = parse_summary_limit(
        "/fast-lane-summary 12 extra",
        20,
        &["/fast_lane_summary", "/fast-lane-summary"],
    )
    .expect_err("extra args should be rejected");
    assert_eq!(error, "usage: /fast_lane_summary [limit]");
}

#[test]
fn parse_safe_lane_summary_limit_accepts_default_and_explicit_limit() {
    assert_eq!(
        parse_safe_lane_summary_limit("/safe_lane_summary", 20).expect("parse"),
        Some(80)
    );
    assert_eq!(
        parse_safe_lane_summary_limit("/safe-lane-summary 120", 20).expect("parse"),
        Some(120)
    );
}

#[test]
fn parse_safe_lane_summary_limit_rejects_invalid_input() {
    let error = parse_safe_lane_summary_limit("/safe_lane_summary 0", 20)
        .expect_err("zero limit should be rejected");
    assert!(error.contains("usage"));

    let error = parse_safe_lane_summary_limit("/safe_lane_summary abc", 20)
        .expect_err("non-number limit should be rejected");
    assert!(error.contains("invalid"));
}

#[test]
fn parse_fast_lane_summary_limit_accepts_default_and_explicit_limit() {
    assert_eq!(
        parse_fast_lane_summary_limit("/fast_lane_summary", 20).expect("parse"),
        Some(80)
    );
    assert_eq!(
        parse_fast_lane_summary_limit("/fast-lane-summary 144", 20).expect("parse"),
        Some(144)
    );
}

#[test]
fn parse_fast_lane_summary_limit_rejects_invalid_input() {
    let error = parse_fast_lane_summary_limit("/fast_lane_summary 0", 20)
        .expect_err("zero limit should be rejected");
    assert!(error.contains("usage"));

    let error = parse_fast_lane_summary_limit("/fast_lane_summary nope", 20)
        .expect_err("non-number limit should be rejected");
    assert!(error.contains("invalid"));
}

#[test]
fn parse_turn_checkpoint_summary_limit_accepts_default_and_explicit_limit() {
    assert_eq!(
        parse_turn_checkpoint_summary_limit("/turn_checkpoint_summary", 20).expect("parse"),
        Some(80)
    );
    assert_eq!(
        parse_turn_checkpoint_summary_limit("/turn-checkpoint-summary 96", 20).expect("parse"),
        Some(96)
    );
}

#[test]
fn parse_turn_checkpoint_summary_limit_rejects_invalid_input() {
    let error = parse_turn_checkpoint_summary_limit("/turn_checkpoint_summary 0", 20)
        .expect_err("zero limit should be rejected");
    assert!(error.contains("usage"));

    let error = parse_turn_checkpoint_summary_limit("/turn_checkpoint_summary nope", 20)
        .expect_err("non-number limit should be rejected");
    assert!(error.contains("invalid"));
}

#[test]
fn is_turn_checkpoint_repair_command_accepts_aliases_and_rejects_extra_args() {
    assert!(is_turn_checkpoint_repair_command("/turn_checkpoint_repair").expect("parse"));
    assert!(is_turn_checkpoint_repair_command("/turn-checkpoint-repair").expect("parse"));
    assert!(!is_turn_checkpoint_repair_command("/turn_checkpoint_summary").expect("parse"));

    let error = is_turn_checkpoint_repair_command("/turn_checkpoint_repair now")
        .expect_err("extra args should be rejected");
    assert!(error.contains("usage"));
}

#[test]
fn is_cli_chat_status_command_accepts_exact_match_and_rejects_extra_args() {
    assert!(is_cli_chat_status_command("/status").expect("parse"));
    assert!(!is_cli_chat_status_command("/history").expect("parse"));

    let error =
        is_cli_chat_status_command("/status now").expect_err("extra args should be rejected");
    assert_eq!(error, "usage: /status");
}

#[test]
fn is_manual_compaction_command_accepts_exact_match_and_rejects_extra_args() {
    assert!(is_manual_compaction_command("/compact").expect("parse"));
    assert!(!is_manual_compaction_command("/history").expect("parse"));

    let error =
        is_manual_compaction_command("/compact now").expect_err("extra args should be rejected");
    assert_eq!(error, "usage: /compact");
}

#[test]
fn help_and_history_commands_reject_extra_args() {
    let help_error =
        parse_exact_chat_command("/help now", &[CLI_CHAT_HELP_COMMAND], "usage: /help")
            .expect_err("help should reject extra args");
    assert_eq!(help_error, "usage: /help");

    let history_error = parse_exact_chat_command(
        "/history now",
        &[CLI_CHAT_HISTORY_COMMAND],
        "usage: /history",
    )
    .expect_err("history should reject extra args");
    assert_eq!(history_error, "usage: /history");

    let sessions_error = parse_exact_chat_command(
        "/sessions now",
        &[CLI_CHAT_SESSIONS_COMMAND],
        "usage: /sessions",
    )
    .expect_err("sessions should reject extra args");
    assert_eq!(sessions_error, "usage: /sessions");

    let mission_error = parse_exact_chat_command(
        "/mission now",
        &[CLI_CHAT_MISSION_COMMAND],
        "usage: /mission",
    )
    .expect_err("mission should reject extra args");
    assert_eq!(mission_error, "usage: /mission");

    let review_error =
        parse_exact_chat_command("/review now", &[CLI_CHAT_REVIEW_COMMAND], "usage: /review")
            .expect_err("review should reject extra args");
    assert_eq!(review_error, "usage: /review");

    let workers_error = parse_exact_chat_command(
        "/workers now",
        &[CLI_CHAT_WORKERS_COMMAND],
        "usage: /workers",
    )
    .expect_err("workers should reject extra args");
    assert_eq!(workers_error, "usage: /workers");
}

#[test]
fn classify_chat_command_match_result_treats_usage_as_non_fatal() {
    let usage_result =
        classify_chat_command_match_result(Err("usage: /help".to_owned())).expect("classify");
    assert_eq!(
        usage_result,
        ChatCommandMatchResult::UsageError("usage: /help".to_owned())
    );

    let matched_result = classify_chat_command_match_result(Ok(true)).expect("classify matched");
    assert_eq!(matched_result, ChatCommandMatchResult::Matched);

    let not_matched_result =
        classify_chat_command_match_result(Ok(false)).expect("classify non-match");
    assert_eq!(not_matched_result, ChatCommandMatchResult::NotMatched);
}

#[test]
fn maybe_render_nonfatal_usage_error_accepts_embedded_usage_text() {
    let error = "invalid fast lane summary limit `nope`; usage: /fast_lane_summary [limit]";
    let usage_lines =
        maybe_render_nonfatal_usage_error(error).expect("usage should render non-fatally");

    assert!(
        usage_lines
            .iter()
            .any(|line| line.contains("/fast_lane_summary [limit]")),
        "embedded usage text should still render the usage card: {usage_lines:#?}"
    );
}

#[test]
fn manual_compaction_status_from_report_maps_failed_open() {
    let report = ContextCompactionReport {
        status: TurnCheckpointProgressStatus::FailedOpen,
        estimated_tokens_before: Some(420),
        estimated_tokens_after: Some(420),
        diagnostics: None,
    };

    let status =
        manual_compaction_status_from_report(&report).expect("failed_open should map cleanly");

    assert_eq!(status, ManualCompactionStatus::FailedOpen);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn manual_compaction_result_applies_and_surfaces_continuity_checkpoint() {
    let mut config = test_config();
    let sqlite_path = unique_chat_sqlite_path("chat-manual-compaction");
    cleanup_chat_test_memory(&sqlite_path);
    config.memory.sqlite_path = sqlite_path.display().to_string();
    config.memory.sliding_window = 32;
    config.conversation.compact_enabled = false;
    config.conversation.compact_preserve_recent_turns = 2;

    let memory_config = SessionStoreConfig::from_memory_config(&config.memory);
    let kernel_ctx = test_kernel_context_with_memory("chat-manual-compaction", &memory_config);
    let session_id = "chat-manual-compaction";

    for (role, content) in [
        ("user", "ask 1"),
        ("assistant", "reply 1"),
        ("user", "ask 2"),
        ("assistant", "reply 2"),
        ("user", "ask 3"),
        ("assistant", "reply 3"),
        ("user", "recent ask"),
        ("assistant", "recent reply"),
    ] {
        store::append_session_turn_direct(session_id, role, content, &memory_config)
            .expect("seed turns should succeed");
    }

    let binding = ConversationRuntimeBinding::kernel(&kernel_ctx);
    let turn_coordinator = ConversationTurnCoordinator::new();
    let result = load_manual_compaction_result(&config, session_id, &turn_coordinator, binding)
        .await
        .expect("manual compaction should succeed");

    assert_eq!(result.status, ManualCompactionStatus::Applied);
    assert_eq!(result.before_turns, 8);
    assert_eq!(result.after_turns, 3);
    assert!(
        result.estimated_tokens_before.is_some(),
        "manual compaction should surface a before-token estimate"
    );
    assert!(
        result.estimated_tokens_after.is_some(),
        "manual compaction should surface an after-token estimate"
    );
    assert!(
        result
            .summary_headline
            .as_deref()
            .is_some_and(|headline| headline.contains("Compacted 6 earlier turns"))
    );
    assert!(
        result
            .prune_summary
            .as_deref()
            .is_some_and(|summary| summary.contains("summary:6"))
    );
    assert!(
        result.detail.contains("Runtime Self Context"),
        "manual compaction detail should reuse the continuity boundary note"
    );

    let turns = store::window_session_turns(session_id, 32, &memory_config)
        .expect("window load should succeed");
    assert!(
        turns[0]
            .content
            .contains("Does not replace Runtime Self Context"),
        "manual compaction should persist the continuity-aware checkpoint"
    );

    cleanup_chat_test_memory(&sqlite_path);
}

fn test_turn_checkpoint_diagnostics(
    summary: TurnCheckpointEventSummary,
    runtime_probe: Option<TurnCheckpointTailRepairRuntimeProbe>,
) -> crate::conversation::TurnCheckpointDiagnostics {
    let recovery = crate::conversation::TurnCheckpointRecoveryAssessment::from_summary(&summary);
    crate::conversation::TurnCheckpointDiagnostics::new(summary, recovery, runtime_probe)
}

#[test]
fn format_turn_checkpoint_summary_reports_recovery_state_and_failure() {
    let summary = TurnCheckpointEventSummary {
        checkpoint_events: 2,
        post_persist_events: 1,
        finalization_failed_events: 1,
        latest_stage: Some(TurnCheckpointStage::FinalizationFailed),
        latest_after_turn: Some(TurnCheckpointProgressStatus::Completed),
        latest_compaction: Some(TurnCheckpointProgressStatus::Failed),
        latest_failure_step: Some(TurnCheckpointFailureStep::Compaction),
        latest_failure_error: Some("context compaction failed".to_owned()),
        latest_lane: Some("safe".to_owned()),
        latest_result_kind: Some("tool_call".to_owned()),
        latest_persistence_mode: Some("error".to_owned()),
        latest_safe_lane_terminal_route: Some(crate::conversation::SafeLaneTerminalRouteSnapshot {
            decision: crate::conversation::SafeLaneFailureRouteDecision::Terminal,
            reason: crate::conversation::SafeLaneFailureRouteReason::SessionGovernorNoReplan,
            source: crate::conversation::SafeLaneFailureRouteSource::SessionGovernor,
        }),
        latest_identity_present: Some(false),
        latest_runs_after_turn: Some(true),
        latest_attempts_context_compaction: Some(true),
        session_state: TurnCheckpointSessionState::FinalizationFailed,
        checkpoint_durable: true,
        requires_recovery: true,
        reply_durable: true,
        ..TurnCheckpointEventSummary::default()
    };

    let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
    let formatted = format_turn_checkpoint_summary("session-checkpoint", 128, &diagnostics);

    assert!(formatted.contains("turn_checkpoint_summary session=session-checkpoint limit=128"));
    assert!(formatted.contains("state=finalization_failed"));
    assert!(formatted.contains("durable=1"));
    assert!(formatted.contains("requires_recovery=1"));
    assert!(formatted.contains("stage=finalization_failed"));
    assert!(formatted.contains("after_turn=completed"));
    assert!(formatted.contains("compaction=failed"));
    assert!(formatted.contains("lane=safe"));
    assert!(formatted.contains("result_kind=tool_call"));
    assert!(formatted.contains("persistence_mode=error"));
    assert!(formatted.contains("safe_lane_route_decision=terminal"));
    assert!(formatted.contains("safe_lane_route_reason=session_governor_no_replan"));
    assert!(formatted.contains("safe_lane_route_source=session_governor"));
    assert!(formatted.contains("identity=missing"));
    assert!(formatted.contains("failure_step=compaction"));
    assert!(formatted.contains("failure_error=context compaction failed"));
    assert!(formatted.contains("recovery_action=inspect_manually"));
    assert!(formatted.contains("recovery_source=summary"));
    assert!(formatted.contains("recovery_reason=checkpoint_identity_missing"));
}

#[test]
fn format_turn_checkpoint_summary_marks_checkpoint_only_durability_for_return_error_sessions() {
    let summary = TurnCheckpointEventSummary {
        checkpoint_events: 1,
        finalized_events: 1,
        latest_stage: Some(TurnCheckpointStage::Finalized),
        latest_after_turn: Some(TurnCheckpointProgressStatus::Skipped),
        latest_compaction: Some(TurnCheckpointProgressStatus::Skipped),
        latest_lane: None,
        latest_result_kind: None,
        latest_persistence_mode: None,
        latest_identity_present: Some(false),
        latest_runs_after_turn: Some(false),
        latest_attempts_context_compaction: Some(false),
        session_state: TurnCheckpointSessionState::Finalized,
        checkpoint_durable: true,
        requires_recovery: false,
        reply_durable: false,
        ..TurnCheckpointEventSummary::default()
    };

    let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
    let formatted = format_turn_checkpoint_summary("session-checkpoint", 64, &diagnostics);

    assert!(formatted.contains("durable=0"));
    assert!(formatted.contains("checkpoint_durable=1"));
    assert!(formatted.contains("durability=checkpoint_only"));
    assert!(formatted.contains("state=finalized"));
}

#[test]
fn format_turn_checkpoint_summary_uses_typed_checkpoint_durability() {
    let summary = TurnCheckpointEventSummary {
        checkpoint_events: 1,
        latest_stage: Some(TurnCheckpointStage::Finalized),
        session_state: TurnCheckpointSessionState::Finalized,
        checkpoint_durable: false,
        reply_durable: false,
        ..TurnCheckpointEventSummary::default()
    };

    let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
    let formatted = format_turn_checkpoint_summary("session-checkpoint", 32, &diagnostics);

    assert!(formatted.contains("state=finalized"));
    assert!(formatted.contains("checkpoint_durable=0"));
    assert!(formatted.contains("durability=not_durable"));
}

#[test]
fn format_turn_checkpoint_startup_health_reports_recovery_action() {
    let summary = TurnCheckpointEventSummary {
        checkpoint_events: 1,
        post_persist_events: 1,
        latest_stage: Some(TurnCheckpointStage::PostPersist),
        latest_after_turn: Some(TurnCheckpointProgressStatus::Pending),
        latest_compaction: Some(TurnCheckpointProgressStatus::Pending),
        latest_lane: Some("safe".to_owned()),
        latest_result_kind: Some("tool_error".to_owned()),
        latest_persistence_mode: Some("success".to_owned()),
        latest_safe_lane_terminal_route: Some(crate::conversation::SafeLaneTerminalRouteSnapshot {
            decision: crate::conversation::SafeLaneFailureRouteDecision::Terminal,
            reason: crate::conversation::SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
            source: crate::conversation::SafeLaneFailureRouteSource::BackpressureGuard,
        }),
        latest_identity_present: Some(true),
        latest_runs_after_turn: Some(true),
        latest_attempts_context_compaction: Some(true),
        session_state: TurnCheckpointSessionState::PendingFinalization,
        checkpoint_durable: true,
        requires_recovery: true,
        reply_durable: true,
        ..TurnCheckpointEventSummary::default()
    };

    let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
    let formatted =
        format_turn_checkpoint_startup_health("session-health", &diagnostics).expect("health");

    assert!(formatted.contains("turn_checkpoint_health session=session-health"));
    assert!(formatted.contains("state=pending_finalization"));
    assert!(formatted.contains("recovery_needed=1"));
    assert!(formatted.contains("action=run_after_turn_and_compaction"));
    assert!(formatted.contains("source=summary"));
    assert!(formatted.contains("reason=-"));
    assert!(formatted.contains("lane=safe"));
    assert!(formatted.contains("result_kind=tool_error"));
    assert!(formatted.contains("safe_lane_route_decision=terminal"));
    assert!(formatted.contains("safe_lane_route_reason=backpressure_attempts_exhausted"));
    assert!(formatted.contains("safe_lane_route_source=backpressure_guard"));
    assert!(formatted.contains("identity=present"));
}

#[test]
fn format_turn_checkpoint_startup_health_reports_route_aware_manual_reason() {
    let summary = TurnCheckpointEventSummary {
        checkpoint_events: 1,
        post_persist_events: 1,
        latest_stage: Some(TurnCheckpointStage::PostPersist),
        latest_after_turn: Some(TurnCheckpointProgressStatus::Skipped),
        latest_compaction: Some(TurnCheckpointProgressStatus::Skipped),
        latest_lane: Some("safe".to_owned()),
        latest_result_kind: Some("tool_error".to_owned()),
        latest_persistence_mode: Some("success".to_owned()),
        latest_safe_lane_terminal_route: Some(crate::conversation::SafeLaneTerminalRouteSnapshot {
            decision: crate::conversation::SafeLaneFailureRouteDecision::Terminal,
            reason: crate::conversation::SafeLaneFailureRouteReason::SessionGovernorNoReplan,
            source: crate::conversation::SafeLaneFailureRouteSource::SessionGovernor,
        }),
        latest_identity_present: Some(true),
        latest_runs_after_turn: Some(false),
        latest_attempts_context_compaction: Some(false),
        session_state: TurnCheckpointSessionState::PendingFinalization,
        checkpoint_durable: true,
        requires_recovery: true,
        reply_durable: true,
        ..TurnCheckpointEventSummary::default()
    };

    let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
    let formatted =
        format_turn_checkpoint_startup_health("session-health", &diagnostics).expect("health");

    assert!(formatted.contains("turn_checkpoint_health session=session-health"));
    assert!(formatted.contains("action=inspect_manually"));
    assert!(formatted.contains("source=summary"));
    assert!(
        formatted.contains("reason=safe_lane_session_governor_terminal_requires_manual_inspection")
    );
    assert!(formatted.contains("safe_lane_route_reason=session_governor_no_replan"));
    assert!(formatted.contains("safe_lane_route_source=session_governor"));
}

#[test]
fn format_turn_checkpoint_startup_health_marks_checkpoint_only_durability() {
    let summary = TurnCheckpointEventSummary {
        checkpoint_events: 1,
        finalized_events: 1,
        latest_stage: Some(TurnCheckpointStage::Finalized),
        latest_after_turn: Some(TurnCheckpointProgressStatus::Skipped),
        latest_compaction: Some(TurnCheckpointProgressStatus::Skipped),
        latest_identity_present: Some(false),
        latest_runs_after_turn: Some(false),
        latest_attempts_context_compaction: Some(false),
        session_state: TurnCheckpointSessionState::Finalized,
        checkpoint_durable: true,
        requires_recovery: false,
        reply_durable: false,
        ..TurnCheckpointEventSummary::default()
    };

    let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
    let formatted =
        format_turn_checkpoint_startup_health("session-health", &diagnostics).expect("health");

    assert!(formatted.contains("reply_durable=0"));
    assert!(formatted.contains("checkpoint_durable=1"));
    assert!(formatted.contains("durability=checkpoint_only"));
}

#[test]
fn format_turn_checkpoint_startup_health_uses_typed_checkpoint_durability_gate() {
    let summary = TurnCheckpointEventSummary {
        checkpoint_events: 1,
        latest_stage: Some(TurnCheckpointStage::Finalized),
        session_state: TurnCheckpointSessionState::Finalized,
        checkpoint_durable: false,
        reply_durable: false,
        ..TurnCheckpointEventSummary::default()
    };

    let diagnostics = test_turn_checkpoint_diagnostics(summary, None);

    assert!(format_turn_checkpoint_startup_health("session-health", &diagnostics).is_none());
}

#[test]
fn format_turn_checkpoint_startup_health_skips_non_durable_sessions() {
    let diagnostics = test_turn_checkpoint_diagnostics(TurnCheckpointEventSummary::default(), None);
    assert!(format_turn_checkpoint_startup_health("session-empty", &diagnostics).is_none());
}

#[test]
fn format_turn_checkpoint_runtime_probe_reports_runtime_only_manual_reason() {
    let probe = TurnCheckpointTailRepairRuntimeProbe::new(
            TurnCheckpointRecoveryAction::InspectManually,
            crate::conversation::TurnCheckpointTailRepairSource::Runtime,
            crate::conversation::TurnCheckpointTailRepairReason::CheckpointPreparationFingerprintMismatch,
        );

    let formatted = format_turn_checkpoint_runtime_probe("session-probe", &probe);

    assert!(formatted.contains("turn_checkpoint_probe session=session-probe"));
    assert!(formatted.contains("action=inspect_manually"));
    assert!(formatted.contains("source=runtime"));
    assert!(formatted.contains("reason=checkpoint_preparation_fingerprint_mismatch"));
}

#[test]
fn format_turn_checkpoint_summary_output_appends_runtime_probe_line() {
    let summary = TurnCheckpointEventSummary {
        checkpoint_events: 1,
        post_persist_events: 1,
        latest_stage: Some(TurnCheckpointStage::FinalizationFailed),
        latest_after_turn: Some(TurnCheckpointProgressStatus::Completed),
        latest_compaction: Some(TurnCheckpointProgressStatus::Failed),
        latest_lane: Some("fast".to_owned()),
        latest_result_kind: Some("final_text".to_owned()),
        latest_persistence_mode: Some("success".to_owned()),
        latest_identity_present: Some(true),
        latest_runs_after_turn: Some(true),
        latest_attempts_context_compaction: Some(true),
        session_state: TurnCheckpointSessionState::FinalizationFailed,
        checkpoint_durable: true,
        requires_recovery: true,
        reply_durable: true,
        ..TurnCheckpointEventSummary::default()
    };
    let probe = TurnCheckpointTailRepairRuntimeProbe::new(
            TurnCheckpointRecoveryAction::InspectManually,
            crate::conversation::TurnCheckpointTailRepairSource::Runtime,
            crate::conversation::TurnCheckpointTailRepairReason::CheckpointPreparationFingerprintMismatch,
        );

    let diagnostics = test_turn_checkpoint_diagnostics(summary, Some(probe));
    let formatted = format_turn_checkpoint_summary_output("session-summary", 64, &diagnostics);

    assert!(formatted.contains("turn_checkpoint_summary session=session-summary limit=64"));
    assert!(formatted.contains("turn_checkpoint_probe session=session-summary"));
    assert!(formatted.contains("source=runtime"));
    assert!(formatted.contains("reason=checkpoint_preparation_fingerprint_mismatch"));
}

#[test]
fn format_turn_checkpoint_repair_reports_summary_source() {
    let summary = TurnCheckpointEventSummary {
        checkpoint_events: 1,
        latest_stage: Some(TurnCheckpointStage::PostPersist),
        session_state: TurnCheckpointSessionState::PendingFinalization,
        checkpoint_durable: true,
        requires_recovery: true,
        reply_durable: true,
        ..TurnCheckpointEventSummary::default()
    };
    let outcome = crate::conversation::TurnCheckpointTailRepairOutcome::from_summary(
        crate::conversation::TurnCheckpointTailRepairStatus::ManualRequired,
        TurnCheckpointRecoveryAction::InspectManually,
        Some(crate::conversation::TurnCheckpointTailRepairSource::Summary),
        crate::conversation::TurnCheckpointTailRepairReason::CheckpointIdentityMissing,
        &summary,
    );

    let formatted = format_turn_checkpoint_repair("session-repair", &outcome);

    assert!(formatted.contains("turn_checkpoint_repair session=session-repair"));
    assert!(formatted.contains("status=manual_required"));
    assert!(formatted.contains("source=summary"));
    assert!(formatted.contains("reason=checkpoint_identity_missing"));
}

#[test]
fn format_turn_checkpoint_summary_output_omits_runtime_probe_line_without_probe() {
    let summary = TurnCheckpointEventSummary {
        checkpoint_events: 1,
        post_persist_events: 1,
        latest_stage: Some(TurnCheckpointStage::PostPersist),
        latest_after_turn: Some(TurnCheckpointProgressStatus::Pending),
        latest_compaction: Some(TurnCheckpointProgressStatus::Pending),
        latest_lane: Some("fast".to_owned()),
        latest_result_kind: Some("final_text".to_owned()),
        latest_persistence_mode: Some("success".to_owned()),
        latest_identity_present: Some(true),
        latest_runs_after_turn: Some(true),
        latest_attempts_context_compaction: Some(true),
        session_state: TurnCheckpointSessionState::PendingFinalization,
        requires_recovery: true,
        reply_durable: true,
        ..TurnCheckpointEventSummary::default()
    };

    let diagnostics = test_turn_checkpoint_diagnostics(summary, None);
    let formatted = format_turn_checkpoint_summary_output("session-summary", 64, &diagnostics);

    assert!(formatted.contains("turn_checkpoint_summary session=session-summary limit=64"));
    assert!(!formatted.contains("turn_checkpoint_probe"));
    assert!(!formatted.ends_with('\n'));
}

#[test]
fn format_safe_lane_summary_includes_rollups_and_rates() {
    let config = ConversationConfig::default();
    let mut summary = SafeLaneEventSummary {
        lane_selected_events: 1,
        round_started_events: 2,
        round_completed_succeeded_events: 1,
        round_completed_failed_events: 1,
        verify_failed_events: 1,
        replan_triggered_events: 1,
        final_status_events: 1,
        session_governor_engaged_events: 1,
        session_governor_force_no_replan_events: 1,
        session_governor_failed_threshold_triggered_events: 1,
        session_governor_backpressure_threshold_triggered_events: 0,
        session_governor_trend_threshold_triggered_events: 1,
        session_governor_recovery_threshold_triggered_events: 0,
        session_governor_metrics_snapshots_seen: 2,
        session_governor_latest_trend_samples: Some(5),
        session_governor_latest_trend_min_samples: Some(4),
        session_governor_latest_trend_failure_ewma_milli: Some(250),
        session_governor_latest_trend_backpressure_ewma_milli: Some(63),
        session_governor_latest_recovery_success_streak: Some(4),
        session_governor_latest_recovery_success_streak_threshold: Some(3),
        final_status: Some(SafeLaneFinalStatus::Failed),
        final_failure_code: Some("safe_lane_plan_verify_failed".to_owned()),
        final_route_decision: Some("terminal".to_owned()),
        final_route_reason: Some("session_governor_no_replan".to_owned()),
        latest_metrics: Some(crate::conversation::SafeLaneMetricsSnapshot {
            rounds_started: 2,
            rounds_succeeded: 1,
            rounds_failed: 1,
            verify_failures: 1,
            replans_triggered: 1,
            total_attempts_used: 3,
        }),
        latest_tool_output: Some(crate::conversation::SafeLaneToolOutputSnapshot {
            output_lines: 2,
            result_lines: 2,
            truncated_result_lines: 1,
            any_truncated: true,
            truncation_ratio_milli: 500,
        }),
        tool_output_snapshots_seen: 2,
        tool_output_truncated_events: 1,
        tool_output_result_lines_total: 3,
        tool_output_truncated_result_lines_total: 1,
        tool_output_aggregate_truncation_ratio_milli: Some(333),
        tool_output_truncation_verify_failed_events: 1,
        tool_output_truncation_replan_events: 1,
        tool_output_truncation_final_failure_events: 1,
        latest_health_signal: Some(crate::conversation::SafeLaneHealthSignalSnapshot {
            severity: "critical".to_owned(),
            flags: vec!["terminal_instability".to_owned()],
        }),
        health_signal_snapshots_seen: 2,
        health_signal_warn_events: 1,
        health_signal_critical_events: 1,
        ..SafeLaneEventSummary::default()
    };
    summary
        .route_decision_counts
        .insert("terminal".to_owned(), 1);
    summary
        .route_reason_counts
        .insert("session_governor_no_replan".to_owned(), 1);
    summary
        .failure_code_counts
        .insert("safe_lane_plan_verify_failed".to_owned(), 1);
    let formatted = format_safe_lane_summary("session-a", 128, &config, &summary);

    assert!(formatted.contains("safe_lane_summary session=session-a limit=128"));
    assert!(formatted.contains("status=failed"));
    assert!(formatted.contains("route_decision=terminal"));
    assert!(formatted.contains("route_reason=session_governor_no_replan"));
    assert!(formatted.contains("replan_per_round=0.500"));
    assert!(formatted.contains("governor_engaged=1"));
    assert!(formatted.contains("governor_force_no_replan=1"));
    assert!(formatted.contains("trigger_failed_threshold=1"));
    assert!(formatted.contains("trigger_trend_threshold=1"));
    assert!(formatted.contains("governor_latest snapshots=2"));
    assert!(formatted.contains("trend_failure_ewma=0.250"));
    assert!(formatted.contains(
            "tool_output snapshots=2 truncated_events=1 result_lines_total=3 truncated_result_lines_total=1"
        ));
    assert!(formatted.contains("latest_truncation_ratio=0.500"));
    assert!(formatted.contains("aggregate_truncation_ratio=0.333"));
    assert!(formatted.contains("aggregate_truncation_ratio_milli=333"));
    assert!(formatted.contains("truncation_verify_failed_events=1"));
    assert!(formatted.contains("truncation_replan_events=1"));
    assert!(formatted.contains("truncation_final_failure_events=1"));
    assert!(formatted.contains("health severity=critical"));
    assert!(formatted.contains("health_payload {\"flags\":"));
    assert!(formatted.contains("\"severity\":\"critical\""));
    assert!(formatted.contains(
            "health_events snapshots=2 warn=1 critical=1 latest_severity=critical latest_flags=terminal_instability"
        ));
    assert!(formatted.contains("truncation_pressure(0.333)"));
    assert!(formatted.contains("verify_failure_pressure(0.500)"));
    assert!(formatted.contains("replan_pressure(0.500)"));
    assert!(formatted.contains("terminal_instability"));
    assert!(formatted.contains("rollup route_decisions=terminal:1"));
    assert!(formatted.contains("rollup route_reasons=session_governor_no_replan:1"));
    assert!(formatted.contains("rollup failure_codes=safe_lane_plan_verify_failed:1"));
}

#[test]
fn format_safe_lane_summary_health_is_ok_when_no_risk_signals() {
    let config = ConversationConfig::default();
    let summary = SafeLaneEventSummary {
        lane_selected_events: 1,
        round_started_events: 3,
        final_status_events: 1,
        final_status: Some(SafeLaneFinalStatus::Succeeded),
        latest_metrics: Some(crate::conversation::SafeLaneMetricsSnapshot {
            rounds_started: 3,
            rounds_succeeded: 3,
            rounds_failed: 0,
            verify_failures: 0,
            replans_triggered: 0,
            total_attempts_used: 3,
        }),
        tool_output_snapshots_seen: 1,
        tool_output_result_lines_total: 2,
        tool_output_truncated_result_lines_total: 0,
        latest_tool_output: Some(crate::conversation::SafeLaneToolOutputSnapshot {
            output_lines: 2,
            result_lines: 2,
            truncated_result_lines: 0,
            any_truncated: false,
            truncation_ratio_milli: 0,
        }),
        ..SafeLaneEventSummary::default()
    };
    let formatted = format_safe_lane_summary("session-ok", 64, &config, &summary);
    assert!(formatted.contains("health severity=ok flags=-"));
    assert!(formatted.contains("health_payload {\"flags\":[],\"severity\":\"ok\"}"));
    assert!(
        formatted.contains(
            "health_events snapshots=0 warn=0 critical=0 latest_severity=- latest_flags=-"
        )
    );
}

#[test]
fn format_safe_lane_summary_uses_internal_health_thresholds() {
    let config = ConversationConfig::default();
    let summary = SafeLaneEventSummary {
        round_started_events: 4,
        verify_failed_events: 1,
        replan_triggered_events: 1,
        tool_output_snapshots_seen: 1,
        tool_output_result_lines_total: 4,
        tool_output_truncated_result_lines_total: 1,
        tool_output_aggregate_truncation_ratio_milli: Some(250),
        latest_tool_output: Some(crate::conversation::SafeLaneToolOutputSnapshot {
            output_lines: 4,
            result_lines: 4,
            truncated_result_lines: 1,
            any_truncated: true,
            truncation_ratio_milli: 250,
        }),
        ..SafeLaneEventSummary::default()
    };

    let formatted = format_safe_lane_summary("session-threshold", 32, &config, &summary);
    assert!(formatted.contains("health severity=warn"));
    assert!(formatted.contains("truncation_pressure(0.250)"));
    assert!(!formatted.contains("verify_failure_pressure"));
    assert!(!formatted.contains("replan_pressure"));
}

#[test]
fn format_safe_lane_summary_does_not_mark_unknown_failure_code_substrings_as_instability() {
    let config = ConversationConfig::default();
    let summary = SafeLaneEventSummary {
        final_status: Some(SafeLaneFinalStatus::Failed),
        final_failure_code: Some("unknown_session_governor_hint".to_owned()),
        ..SafeLaneEventSummary::default()
    };

    let formatted = format_safe_lane_summary("session-unknown-code", 16, &config, &summary);
    assert!(formatted.contains("health severity=ok"));
    assert!(!formatted.contains("terminal_instability"));
}
