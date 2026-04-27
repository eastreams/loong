use super::*;

#[test]
fn build_provider_turn_tool_terminal_events_prefers_trace_outcomes_over_generic_fallbacks() {
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![
            ToolIntent {
                tool_name: "sessions_list".to_owned(),
                args_json: json!({}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-a".to_owned(),
                turn_id: "turn-a".to_owned(),
                tool_call_id: "call-1".to_owned(),
            },
            ToolIntent {
                tool_name: "session_status".to_owned(),
                args_json: json!({"session_id": "session-a"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-a".to_owned(),
                turn_id: "turn-a".to_owned(),
                tool_call_id: "call-2".to_owned(),
            },
        ],
        raw_meta: Value::Null,
    };
    let turn_result = TurnResult::ToolError(TurnFailure::retryable(
        "tool_execution_failed",
        "second tool failed",
    ));
    let trace = ToolBatchExecutionTrace {
        total_intents: 2,
        parallel_execution_enabled: false,
        parallel_execution_max_in_flight: 1,
        observed_peak_in_flight: 1,
        observed_wall_time_ms: 10,
        segments: Vec::new(),
        decision_records: Vec::new(),
        outcome_records: Vec::new(),
        intent_outcomes: vec![
            ToolBatchExecutionIntentTrace {
                tool_call_id: "call-1".to_owned(),
                tool_name: "sessions_list".to_owned(),
                status: ToolBatchExecutionIntentStatus::Completed,
                detail: None,
            },
            ToolBatchExecutionIntentTrace {
                tool_call_id: "call-2".to_owned(),
                tool_name: "session_status".to_owned(),
                status: ToolBatchExecutionIntentStatus::Failed,
                detail: Some("second tool failed".to_owned()),
            },
        ],
    };

    let events = build_provider_turn_tool_terminal_events(&turn, &turn_result, Some(&trace));

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].tool_call_id, "call-1");
    assert_eq!(events[0].state, ConversationTurnToolState::Completed);
    assert_eq!(events[1].tool_call_id, "call-2");
    assert_eq!(events[1].state, ConversationTurnToolState::Failed);
    assert_eq!(events[1].detail.as_deref(), Some("second tool failed"));
}

#[test]
fn build_provider_turn_tool_terminal_events_attach_visible_shell_request_summary() {
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![ToolIntent {
            tool_name: "shell.exec".to_owned(),
            args_json: json!({"command": "ls /root"}),
            source: "provider_tool_call".to_owned(),
            session_id: "session-a".to_owned(),
            turn_id: "turn-a".to_owned(),
            tool_call_id: "call-shell".to_owned(),
        }],
        raw_meta: Value::Null,
    };
    let turn_result = TurnResult::ToolDenied(TurnFailure::policy_denied(
        "shell_policy_denied",
        "policy_denied: command contains embedded whitespace",
    ));

    let events = build_provider_turn_tool_terminal_events(&turn, &turn_result, None);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].tool_call_id, "call-shell");
    assert_eq!(events[0].state, ConversationTurnToolState::Denied);
    let request_summary =
        summarize_tool_event_request(&turn.tool_intents[0]).expect("request summary");
    let request_summary_json: Value =
        serde_json::from_str(&request_summary).expect("request summary should be valid json");
    assert_eq!(
        request_summary_json,
        json!({
            "tool": "exec",
            "request": {"command": "ls", "args_redacted": 1}
        })
    );
}

#[test]
fn summarize_failed_provider_lane_tool_request_preserves_multi_intent_context_without_trace() {
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![
            ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({"path": "Cargo.toml"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-a".to_owned(),
                turn_id: "turn-a".to_owned(),
                tool_call_id: "call-1".to_owned(),
            },
            ToolIntent {
                tool_name: "shell.exec".to_owned(),
                args_json: json!({"command": "ls /root"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-a".to_owned(),
                turn_id: "turn-a".to_owned(),
                tool_call_id: "call-2".to_owned(),
            },
        ],
        raw_meta: Value::Null,
    };

    let request_summary = summarize_provider_lane_tool_request(
        &turn,
        &TurnResult::ToolError(TurnFailure::retryable("tool_error", "temporary failure")),
        None,
    )
    .expect("multi-intent failures should retain a request summary");
    let request_summary_json: Value =
        serde_json::from_str(&request_summary).expect("request summary should be valid json");
    let request_entries = request_summary_json
        .as_array()
        .expect("multi-intent request summary should be an array");

    assert_eq!(request_entries.len(), 2);
    assert_eq!(request_entries[0]["tool"], "read");
    assert_eq!(request_entries[1]["tool"], "exec");
    assert_eq!(request_entries[1]["request"]["command"], "ls");
    assert_eq!(request_entries[1]["request"]["args_redacted"], 1);
}

#[test]
fn safe_lane_replan_budget_allows_one_retry_then_exhausts() {
    let initial = SafeLaneReplanBudget::new(1);

    assert_eq!(
        initial.continuation_decision(),
        SafeLaneContinuationBudgetDecision::Continue
    );
    assert_eq!(initial.current_round(), 0);

    let exhausted = initial.after_replan();
    assert_eq!(
        exhausted.continuation_decision(),
        SafeLaneContinuationBudgetDecision::Terminal {
            reason: SafeLaneFailureRouteReason::RoundBudgetExhausted,
        }
    );
    assert_eq!(exhausted.current_round(), 1);
}

#[test]
fn escalating_attempt_budget_caps_growth_at_maximum() {
    let budget = EscalatingAttemptBudget::new(2, 4);

    assert_eq!(budget.current_limit(), 2);
    assert_eq!(budget.after_retry().current_limit(), 3);
    assert_eq!(budget.after_retry().after_retry().current_limit(), 4);
    assert_eq!(
        budget
            .after_retry()
            .after_retry()
            .after_retry()
            .current_limit(),
        4
    );
}

#[test]
fn decide_provider_request_action_continues_on_success() {
    let decision = decide_provider_turn_request_action(
        Ok(ProviderTurn {
            assistant_text: "preface".to_owned(),
            tool_intents: Vec::new(),
            raw_meta: Value::Null,
        }),
        ProviderErrorMode::Propagate,
    );

    if let ProviderTurnRequestAction::Continue { turn } = decision {
        assert_eq!(turn.assistant_text, "preface");
        assert!(turn.tool_intents.is_empty());
    } else {
        panic!("unexpected decision: {decision:?}");
    }
}
