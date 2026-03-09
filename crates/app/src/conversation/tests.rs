use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use loongclaw_contracts::{Capability, ExecutionRoute, HarnessKind, MemoryPlaneError};
use loongclaw_kernel::{
    CoreMemoryAdapter, FixedClock, InMemoryAuditSink, LoongClawKernel, MemoryCoreOutcome,
    MemoryCoreRequest, StaticPolicyEngine, VerticalPackManifest,
};
use serde_json::{json, Value};

use super::super::config::{
    CliChannelConfig, ConversationConfig, FeishuChannelConfig, LoongClawConfig, MemoryConfig,
    ProviderConfig, TelegramChannelConfig, ToolConfig,
};
use super::persistence::format_provider_error_reply;
use super::runtime::DefaultConversationRuntime;
use super::*;
use crate::CliResult;
use crate::KernelContext;

struct FakeRuntime {
    seed_messages: Vec<Value>,
    completion_responses: Mutex<VecDeque<Result<String, String>>>,
    turn_responses: Mutex<VecDeque<Result<ProviderTurn, String>>>,
    persisted: Mutex<Vec<(String, String, String)>>,
    requested_messages: Mutex<Vec<Value>>,
    turn_requested_messages: Mutex<Vec<Vec<Value>>>,
    completion_requested_messages: Mutex<Vec<Vec<Value>>>,
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
            completion_responses: Mutex::new(VecDeque::from(completions)),
            turn_responses: Mutex::new(VecDeque::from(turns)),
            persisted: Mutex::new(Vec::new()),
            requested_messages: Mutex::new(Vec::new()),
            turn_requested_messages: Mutex::new(Vec::new()),
            completion_requested_messages: Mutex::new(Vec::new()),
            completion_calls: Mutex::new(0),
            turn_calls: Mutex::new(0),
        }
    }
}

#[async_trait]
impl ConversationRuntime for FakeRuntime {
    fn build_messages(
        &self,
        _config: &LoongClawConfig,
        _session_id: &str,
        _include_system_prompt: bool,
        _kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<Vec<Value>> {
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
    ) -> CliResult<ProviderTurn> {
        let mut calls = self.turn_calls.lock().expect("turn calls lock");
        *calls += 1;
        *self.requested_messages.lock().expect("request lock") = messages.to_vec();
        self.turn_requested_messages
            .lock()
            .expect("turn request lock")
            .push(messages.to_vec());
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
