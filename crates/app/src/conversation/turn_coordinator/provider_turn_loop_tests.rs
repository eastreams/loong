use super::*;
use crate::conversation::turn_engine::ApprovalRequirement;

fn provider_loop_policy(
    max_repeated_tool_call_rounds: usize,
    max_ping_pong_cycles: usize,
    max_same_tool_failure_rounds: usize,
    max_consecutive_same_tool: usize,
) -> ProviderTurnLoopPolicy {
    let mut config = LoongConfig::default();
    config.conversation.turn_loop.max_repeated_tool_call_rounds = max_repeated_tool_call_rounds;
    config.conversation.turn_loop.max_ping_pong_cycles = max_ping_pong_cycles;
    config.conversation.turn_loop.max_same_tool_failure_rounds = max_same_tool_failure_rounds;
    config.conversation.turn_loop.max_consecutive_same_tool = max_consecutive_same_tool;
    ProviderTurnLoopPolicy::from_config(&config)
}

fn provider_loop_turn(tool_name: &str, args_json: Value, call_id: &str) -> ProviderTurn {
    let intent = ToolIntent {
        tool_name: tool_name.to_owned(),
        args_json,
        source: "provider_tool_call".to_owned(),
        session_id: "session-provider-loop".to_owned(),
        turn_id: "turn-provider-loop".to_owned(),
        tool_call_id: call_id.to_owned(),
    };

    ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![intent],
        raw_meta: Value::Null,
    }
}

fn provider_loop_execution(turn_result: TurnResult) -> ProviderTurnLaneExecution {
    ProviderTurnLaneExecution {
        lane: ExecutionLane::Fast,
        assistant_preface: String::new(),
        provider_usage: None,
        had_tool_intents: true,
        tool_request_summary: None,
        discovery_search_turn: false,
        search_tool_intents: 0,
        malformed_parse_followup_turn: false,
        supports_provider_turn_followup: true,
        raw_tool_output_requested: false,
        turn_result,
        safe_lane_terminal_route: None,
        tool_events: Vec::new(),
    }
}

fn provider_loop_result(content: &str, call_id: &str, lease: &str) -> TurnResult {
    let payload_summary = json!({
        "results": [
            {
                "tool_id": "file.read",
                "content": content,
                "lease": lease,
            }
        ],
    });
    let envelope = json!({
        "status": "ok",
        "tool": "file.read",
        "tool_call_id": call_id,
        "lease": lease,
        "payload_summary": payload_summary.to_string(),
    });
    TurnResult::FinalText(envelope.to_string())
}

fn assert_warns_then_stops(
    state: &mut ProviderTurnLoopState,
    policy: &ProviderTurnLoopPolicy,
    first_turn: &ProviderTurn,
    first_execution: &ProviderTurnLaneExecution,
    second_turn: &ProviderTurn,
    second_execution: &ProviderTurnLaneExecution,
    third_turn: &ProviderTurn,
    third_execution: &ProviderTurnLaneExecution,
) {
    assert!(matches!(
        state.observe_turn(policy, first_turn, first_execution),
        Some(ProviderTurnLoopVerdict::Continue)
    ));
    assert!(matches!(
        state.observe_turn(policy, second_turn, second_execution),
        Some(ProviderTurnLoopVerdict::InjectWarning { .. })
    ));
    assert!(matches!(
        state.observe_turn(policy, third_turn, third_execution),
        Some(ProviderTurnLoopVerdict::HardStop { .. })
    ));
}

#[test]
fn provider_turn_loop_warns_then_stops_on_no_progress_pattern() {
    let policy = provider_loop_policy(1, 100, 100, 100);
    let mut state = ProviderTurnLoopState::default();
    let first_turn = provider_loop_turn("file.read", json!({"path": "README.md"}), "call-1");
    let second_turn = provider_loop_turn("file.read", json!({"path": "README.md"}), "call-2");
    let third_turn = provider_loop_turn("file.read", json!({"path": "README.md"}), "call-3");
    let first_execution =
        provider_loop_execution(provider_loop_result("same content", "call-1", "lease-1"));
    let second_execution =
        provider_loop_execution(provider_loop_result("same content", "call-2", "lease-2"));
    let third_execution =
        provider_loop_execution(provider_loop_result("same content", "call-3", "lease-3"));

    assert_warns_then_stops(
        &mut state,
        &policy,
        &first_turn,
        &first_execution,
        &second_turn,
        &second_execution,
        &third_turn,
        &third_execution,
    );
}

#[test]
fn provider_turn_loop_warns_then_stops_on_repeated_approval_pattern() {
    let policy = provider_loop_policy(1, 100, 100, 100);
    let mut state = ProviderTurnLoopState::default();
    let first_turn = provider_loop_turn("shell.exec", json!({"command": "ls"}), "call-1");
    let second_turn = provider_loop_turn("shell.exec", json!({"command": "ls"}), "call-2");
    let third_turn = provider_loop_turn("shell.exec", json!({"command": "ls"}), "call-3");
    let first_requirement = ApprovalRequirement::governed_tool(
        "shell.exec",
        "tool:shell.exec:ls",
        "approval required",
        "shell_exec_requires_approval",
        Some("apr-first".to_owned()),
    );
    let second_requirement = ApprovalRequirement::governed_tool(
        "shell.exec",
        "tool:shell.exec:ls",
        "approval required",
        "shell_exec_requires_approval",
        Some("apr-second".to_owned()),
    );
    let third_requirement = ApprovalRequirement::governed_tool(
        "shell.exec",
        "tool:shell.exec:ls",
        "approval required",
        "shell_exec_requires_approval",
        Some("apr-third".to_owned()),
    );
    let first_execution = provider_loop_execution(TurnResult::NeedsApproval(first_requirement));
    let second_execution = provider_loop_execution(TurnResult::NeedsApproval(second_requirement));
    let third_execution = provider_loop_execution(TurnResult::NeedsApproval(third_requirement));

    assert!(matches!(
        state.observe_turn(&policy, &first_turn, &first_execution),
        Some(ProviderTurnLoopVerdict::Continue)
    ));
    assert!(matches!(
        state.observe_turn(&policy, &second_turn, &second_execution),
        Some(ProviderTurnLoopVerdict::InjectWarning { .. })
    ));
    assert!(matches!(
        state.observe_turn(&policy, &third_turn, &third_execution),
        Some(ProviderTurnLoopVerdict::HardStop { .. })
    ));
}

#[test]
fn provider_turn_loop_clears_pending_warning_after_toolless_turn() {
    let policy = provider_loop_policy(1, 100, 100, 100);
    let mut state = ProviderTurnLoopState::default();
    let repeated_turn = provider_loop_turn("file.read", json!({"path": "README.md"}), "call-1");
    let toolless_turn = ProviderTurn {
        assistant_text: "done".to_owned(),
        tool_intents: Vec::new(),
        raw_meta: Value::Null,
    };
    let first_execution =
        provider_loop_execution(provider_loop_result("same content", "call-1", "lease-1"));
    let second_execution =
        provider_loop_execution(provider_loop_result("same content", "call-2", "lease-2"));
    let final_execution = provider_loop_execution(TurnResult::FinalText("done".to_owned()));

    assert!(matches!(
        state.observe_turn(&policy, &repeated_turn, &first_execution),
        Some(ProviderTurnLoopVerdict::Continue)
    ));
    assert!(matches!(
        state.observe_turn(&policy, &repeated_turn, &second_execution),
        Some(ProviderTurnLoopVerdict::InjectWarning { .. })
    ));
    assert!(
        state
            .observe_turn(&policy, &toolless_turn, &final_execution)
            .is_none()
    );
    assert!(matches!(
        state.observe_turn(&policy, &repeated_turn, &second_execution),
        Some(ProviderTurnLoopVerdict::InjectWarning { .. })
    ));
}

#[test]
fn provider_turn_loop_clears_pending_warning_after_provider_error_outcome() {
    let policy = provider_loop_policy(1, 100, 100, 100);
    let mut state = ProviderTurnLoopState::default();
    let repeated_turn = provider_loop_turn("file.read", json!({"path": "README.md"}), "call-1");
    let first_execution =
        provider_loop_execution(provider_loop_result("same content", "call-1", "lease-1"));
    let second_execution =
        provider_loop_execution(provider_loop_result("same content", "call-2", "lease-2"));
    let provider_error_execution =
        provider_loop_execution(TurnResult::provider_error("provider", "transient"));

    assert!(matches!(
        state.observe_turn(&policy, &repeated_turn, &first_execution),
        Some(ProviderTurnLoopVerdict::Continue)
    ));
    assert!(matches!(
        state.observe_turn(&policy, &repeated_turn, &second_execution),
        Some(ProviderTurnLoopVerdict::InjectWarning { .. })
    ));
    assert!(matches!(
        state.observe_turn(&policy, &repeated_turn, &provider_error_execution),
        Some(ProviderTurnLoopVerdict::Continue)
    ));
    assert!(matches!(
        state.observe_turn(&policy, &repeated_turn, &second_execution),
        Some(ProviderTurnLoopVerdict::InjectWarning { .. })
    ));
}

#[test]
fn provider_turn_loop_allows_same_tool_with_different_arguments() {
    let policy = provider_loop_policy(100, 100, 100, 2);
    let mut state = ProviderTurnLoopState::default();
    let first_turn = provider_loop_turn("file.read", json!({"path": "a.txt"}), "call-1");
    let second_turn = provider_loop_turn("file.read", json!({"path": "b.txt"}), "call-2");
    let third_turn = provider_loop_turn("file.read", json!({"path": "c.txt"}), "call-3");
    let first_execution =
        provider_loop_execution(provider_loop_result("alpha", "call-1", "lease-1"));
    let second_execution =
        provider_loop_execution(provider_loop_result("beta", "call-2", "lease-2"));
    let third_execution =
        provider_loop_execution(provider_loop_result("gamma", "call-3", "lease-3"));

    assert!(matches!(
        state.observe_turn(&policy, &first_turn, &first_execution),
        Some(ProviderTurnLoopVerdict::Continue)
    ));
    assert!(matches!(
        state.observe_turn(&policy, &second_turn, &second_execution),
        Some(ProviderTurnLoopVerdict::Continue)
    ));
    assert!(matches!(
        state.observe_turn(&policy, &third_turn, &third_execution),
        Some(ProviderTurnLoopVerdict::Continue)
    ));
}

#[test]
fn provider_turn_loop_warns_then_stops_on_ping_pong_patterns() {
    let policy = provider_loop_policy(100, 2, 100, 100);
    let mut state = ProviderTurnLoopState::default();
    let first_turn = provider_loop_turn("file.read", json!({"path": "a.txt"}), "call-1");
    let second_turn = provider_loop_turn("file.read", json!({"path": "b.txt"}), "call-2");
    let third_turn = provider_loop_turn("file.read", json!({"path": "a.txt"}), "call-3");
    let fourth_turn = provider_loop_turn("file.read", json!({"path": "b.txt"}), "call-4");
    let fifth_turn = provider_loop_turn("file.read", json!({"path": "a.txt"}), "call-5");
    let first_execution =
        provider_loop_execution(provider_loop_result("alpha", "call-1", "lease-1"));
    let second_execution =
        provider_loop_execution(provider_loop_result("beta", "call-2", "lease-2"));
    let third_execution =
        provider_loop_execution(provider_loop_result("alpha", "call-3", "lease-3"));
    let fourth_execution =
        provider_loop_execution(provider_loop_result("beta", "call-4", "lease-4"));
    let fifth_execution =
        provider_loop_execution(provider_loop_result("alpha", "call-5", "lease-5"));

    assert!(matches!(
        state.observe_turn(&policy, &first_turn, &first_execution),
        Some(ProviderTurnLoopVerdict::Continue)
    ));
    assert!(matches!(
        state.observe_turn(&policy, &second_turn, &second_execution),
        Some(ProviderTurnLoopVerdict::Continue)
    ));
    assert!(matches!(
        state.observe_turn(&policy, &third_turn, &third_execution),
        Some(ProviderTurnLoopVerdict::Continue)
    ));
    assert!(matches!(
        state.observe_turn(&policy, &fourth_turn, &fourth_execution),
        Some(ProviderTurnLoopVerdict::InjectWarning { .. })
    ));
    assert!(matches!(
        state.observe_turn(&policy, &fifth_turn, &fifth_execution),
        Some(ProviderTurnLoopVerdict::HardStop { .. })
    ));
}

#[test]
fn provider_turn_loop_warns_then_stops_on_same_tool_failure_streak() {
    let policy = provider_loop_policy(100, 100, 2, 100);
    let mut state = ProviderTurnLoopState::default();
    let first_turn = provider_loop_turn("file.read", json!({"path": "a.txt"}), "call-1");
    let second_turn = provider_loop_turn("file.read", json!({"path": "b.txt"}), "call-2");
    let third_turn = provider_loop_turn("file.read", json!({"path": "c.txt"}), "call-3");
    let first_error = TurnFailure::retryable("io_error", "could not read a.txt");
    let second_error = TurnFailure::retryable("io_error", "could not read b.txt");
    let third_error = TurnFailure::retryable("io_error", "could not read c.txt");
    let first_execution = provider_loop_execution(TurnResult::ToolError(first_error));
    let second_execution = provider_loop_execution(TurnResult::ToolError(second_error));
    let third_execution = provider_loop_execution(TurnResult::ToolError(third_error));

    assert!(matches!(
        state.observe_turn(&policy, &first_turn, &first_execution),
        Some(ProviderTurnLoopVerdict::Continue)
    ));
    assert!(matches!(
        state.observe_turn(&policy, &second_turn, &second_execution),
        Some(ProviderTurnLoopVerdict::InjectWarning { .. })
    ));
    assert!(matches!(
        state.observe_turn(&policy, &third_turn, &third_execution),
        Some(ProviderTurnLoopVerdict::HardStop { .. })
    ));
}
