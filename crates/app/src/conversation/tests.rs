use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use loongclaw_contracts::{Capability, ExecutionRoute, HarnessKind, MemoryPlaneError};
use loongclaw_kernel::{
    CoreMemoryAdapter, FixedClock, InMemoryAuditSink, LoongClawKernel, MemoryCoreOutcome,
    MemoryCoreRequest, StaticPolicyEngine, VerticalPackManifest,
};
use serde_json::{Value, json};

use super::super::config::{
    CliChannelConfig, ConversationConfig, ExternalSkillsConfig, FeishuChannelConfig,
    LoongClawConfig, MemoryConfig, ProviderConfig, TelegramChannelConfig, ToolConfig,
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
    async fn build_messages(
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
        drop(calls);
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
        drop(calls);
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
        conversation: ConversationConfig::default(),
        tools: ToolConfig::default(),
        external_skills: ExternalSkillsConfig::default(),
        memory: MemoryConfig::default(),
    }
}

#[tokio::test]
async fn handle_turn_with_runtime_success_persists_user_and_assistant_turns() {
    let runtime = FakeRuntime::new(
        vec![json!({"role": "system", "content": "sys"})],
        Ok("assistant-reply".to_owned()),
    );
    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
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
    let coordinator = ConversationTurnCoordinator::new();
    let error = coordinator
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
    let coordinator = ConversationTurnCoordinator::new();
    let output = coordinator
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
async fn handle_turn_with_runtime_tool_turn_uses_natural_language_completion_by_default() {
    use super::integration_tests::TurnTestHarness;

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
            Some(&harness.kernel_ctx),
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
        "hello from coordinator test",
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

    let coordinator = ConversationTurnCoordinator::new();
    let reply = coordinator
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

#[tokio::test]
async fn handle_turn_with_runtime_safe_lane_honors_configured_tool_step_budget() {
    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Executing deployment checks.".to_owned(),
            tool_intents: vec![
                ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "note.md"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-safe-budget".to_owned(),
                    turn_id: "turn-safe-budget".to_owned(),
                    tool_call_id: "call-safe-budget-1".to_owned(),
                },
                ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "checklist.md"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-safe-budget".to_owned(),
                    turn_id: "turn-safe-budget".to_owned(),
                    tool_call_id: "call-safe-budget-2".to_owned(),
                },
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
            None,
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
                ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "note.md"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-safe-plan".to_owned(),
                    turn_id: "turn-safe-plan".to_owned(),
                    tool_call_id: "call-safe-plan-1".to_owned(),
                },
                ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "checklist.md"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-safe-plan".to_owned(),
                    turn_id: "turn-safe-plan".to_owned(),
                    tool_call_id: "call-safe-plan-2".to_owned(),
                },
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
            None,
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
            tool_intents: vec![ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({"path": "note.md"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-safe-events".to_owned(),
                turn_id: "turn-safe-events".to_owned(),
                tool_call_id: "call-safe-events-1".to_owned(),
            }],
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
            None,
        )
        .await
        .expect("safe lane plan should produce a reply");

    let persisted = runtime.persisted.lock().expect("persisted lock");
    let event_names = persisted
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
}

#[tokio::test]
async fn handle_turn_with_runtime_safe_lane_plan_skips_runtime_events_when_disabled() {
    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Executing deployment checks.".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({"path": "note.md"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-safe-events-off".to_owned(),
                turn_id: "turn-safe-events-off".to_owned(),
                tool_call_id: "call-safe-events-off-1".to_owned(),
            }],
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
            None,
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
            (parsed.get("type")?.as_str()? == "conversation_event").then_some(())
        })
        .count();
    assert_eq!(event_count, 0, "unexpected runtime events: {persisted:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_safe_lane_plan_emits_kernel_runtime_audit_events() {
    use super::integration_tests::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(harness.temp_dir.join("note.md"), "safe lane audit probe")
        .expect("write note fixture");
    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Executing deployment checks.".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({"path": "note.md"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-safe-audit-on".to_owned(),
                turn_id: "turn-safe-audit-on".to_owned(),
                tool_call_id: "call-safe-audit-on-1".to_owned(),
            }],
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
            Some(&harness.kernel_ctx),
        )
        .await
        .expect("safe lane plan should produce a reply");

    let events = harness.audit.snapshot();
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
    use super::integration_tests::TurnTestHarness;

    let harness = TurnTestHarness::new();
    std::fs::write(harness.temp_dir.join("note.md"), "safe lane audit disabled")
        .expect("write note fixture");
    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
        Ok(ProviderTurn {
            assistant_text: "Executing deployment checks.".to_owned(),
            tool_intents: vec![ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({"path": "note.md"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-safe-audit-off".to_owned(),
                turn_id: "turn-safe-audit-off".to_owned(),
                tool_call_id: "call-safe-audit-off-1".to_owned(),
            }],
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
            Some(&harness.kernel_ctx),
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
                    "tool": request.tool_name,
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
        granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
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
            tool_intents: vec![ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({"path": "note.md"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-safe-replan".to_owned(),
                turn_id: "turn-safe-replan".to_owned(),
                tool_call_id: "call-safe-replan-1".to_owned(),
            }],
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
            Some(&ctx),
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
        granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
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
            tool_intents: vec![ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({"path": "note.md"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-safe-backpressure".to_owned(),
                turn_id: "turn-safe-backpressure".to_owned(),
                tool_call_id: "call-safe-backpressure-1".to_owned(),
            }],
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
            Some(&ctx),
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
        granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
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
            tool_intents: vec![ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({"path": "note.md"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-safe-verify-nonretryable".to_owned(),
                turn_id: "turn-safe-verify-nonretryable".to_owned(),
                tool_call_id: "call-safe-verify-nonretryable-1".to_owned(),
            }],
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
            Some(&ctx),
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
            if request.operation == "window" {
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
        granted_capabilities: BTreeSet::from([Capability::InvokeTool, Capability::MemoryRead]),
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
            tool_intents: vec![ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({"path": "note.md"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-safe-governor".to_owned(),
                turn_id: "turn-safe-governor".to_owned(),
                tool_call_id: "call-safe-governor-1".to_owned(),
            }],
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
            Some(&ctx),
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
            if request.operation == "window" {
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
            tool_intents: vec![ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({"path": "note.md"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-safe-governor-window".to_owned(),
                turn_id: "turn-safe-governor-window".to_owned(),
                tool_call_id: "call-safe-governor-window-1".to_owned(),
            }],
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
            Some(&ctx),
        )
        .await
        .expect("safe lane turn should complete");

    let captured = memory_invocations
        .lock()
        .expect("memory invocations lock")
        .clone();
    let window_request = captured
        .iter()
        .find(|request| request.operation == "window")
        .expect("window request should be issued");
    assert_eq!(
        window_request.payload["session_id"],
        "session-safe-governor-window"
    );
    assert_eq!(window_request.payload["limit"], 200);
    assert_eq!(window_request.payload["allow_extended_limit"], true);
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
            let path = request
                .payload
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
        granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
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
                ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "note.md"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-safe-subgraph".to_owned(),
                    turn_id: "turn-safe-subgraph".to_owned(),
                    tool_call_id: "call-safe-subgraph-1".to_owned(),
                },
                ToolIntent {
                    tool_name: "file.read".to_owned(),
                    args_json: json!({"path": "checklist.md"}),
                    source: "provider_tool_call".to_owned(),
                    session_id: "session-safe-subgraph".to_owned(),
                    turn_id: "turn-safe-subgraph".to_owned(),
                    tool_call_id: "call-safe-subgraph-2".to_owned(),
                },
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
            Some(&ctx),
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
        1,
        "tool-denied fallback should run a completion pass for language-aware output"
    );

    let persisted = runtime.persisted.lock().expect("persisted lock");
    assert_eq!(persisted.len(), 2);
    assert_eq!(persisted[1].2, reply);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_turn_with_runtime_tool_error_returns_natural_language_fallback() {
    use super::integration_tests::TurnTestHarness;

    let harness = TurnTestHarness::new();
    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
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
        1,
        "tool-error fallback should run a completion pass for language-aware output"
    );

    let persisted = runtime.persisted.lock().expect("persisted lock");
    assert_eq!(persisted.len(), 2);
    assert_eq!(persisted[1].2, reply);
}

#[tokio::test]
async fn handle_turn_with_runtime_tool_failure_completion_error_uses_raw_reason_without_markers() {
    let runtime = FakeRuntime::with_turn_and_completion(
        vec![],
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
fn turn_engine_unknown_tool_exposes_structured_policy_denial() {
    use crate::conversation::turn_engine::{
        ProviderTurn, ToolIntent, TurnEngine, TurnFailureKind, TurnResult,
    };
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
        TurnResult::ToolDenied(failure) => {
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
async fn turn_engine_tool_execution_error_is_marked_retryable() {
    use crate::conversation::turn_engine::{
        ProviderTurn, ToolIntent, TurnEngine, TurnFailureKind, TurnResult,
    };
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
        granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
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
        let payload = if request.operation == crate::memory::MEMORY_OP_WINDOW {
            json!({
                "turns": [
                    {
                        "role": "assistant",
                        "content": "history-from-kernel",
                        "ts": 1
                    }
                ]
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
async fn build_messages_routes_window_through_kernel_when_context_provided() {
    let audit = Arc::new(InMemoryAuditSink::default());
    let (ctx, invocations) = build_kernel_context(audit.clone());

    let runtime = DefaultConversationRuntime;
    let config = test_config();
    let messages = runtime
        .build_messages(&config, "session-k-window", true, Some(&ctx))
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
            .any(|message| message["content"] == "history-from-kernel"),
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
