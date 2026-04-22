use super::*;

#[test]
fn decide_provider_request_action_inlines_synthetic_reply_when_requested() {
    let decision = decide_provider_turn_request_action(
        Err("provider unavailable".to_owned()),
        ProviderErrorMode::InlineMessage,
    );

    if let ProviderTurnRequestAction::FinalizeInlineProviderError { reply } = decision {
        assert!(reply.contains("provider unavailable"));
    } else {
        panic!("unexpected decision: {decision:?}");
    }
}

#[test]
fn decide_provider_request_action_returns_error_in_propagate_mode() {
    let decision = decide_provider_turn_request_action(
        Err("provider unavailable".to_owned()),
        ProviderErrorMode::Propagate,
    );

    if let ProviderTurnRequestAction::ReturnError { error } = decision {
        assert_eq!(error, "provider unavailable");
    } else {
        panic!("unexpected decision: {decision:?}");
    }
}

#[test]
fn safe_lane_route_retryable_failure_replans_with_remaining_budget() {
    let failure = TurnFailure::retryable("safe_lane_plan_node_retryable_error", "transient");
    let route = SafeLaneFailureRoute::from_failure(&failure, SafeLaneReplanBudget::new(1));

    assert_eq!(route.decision, SafeLaneFailureRouteDecision::Replan);
    assert_eq!(route.reason, SafeLaneFailureRouteReason::RetryableFailure);
    assert_eq!(route.source, SafeLaneFailureRouteSource::BaseRouting);
    assert_eq!(route.reason.as_str(), "retryable_failure");
}

#[test]
fn safe_lane_route_retryable_failure_becomes_terminal_after_budget_exhaustion() {
    let failure = TurnFailure::retryable("safe_lane_plan_node_retryable_error", "transient");
    let route =
        SafeLaneFailureRoute::from_failure(&failure, SafeLaneReplanBudget::new(1).after_replan());

    assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
    assert_eq!(
        route.reason,
        SafeLaneFailureRouteReason::RoundBudgetExhausted
    );
    assert_eq!(route.source, SafeLaneFailureRouteSource::BaseRouting);
    assert!(route.is_base_round_budget_terminal());
}

#[test]
fn safe_lane_route_policy_denied_failure_is_terminal() {
    let failure = TurnFailure::policy_denied("safe_lane_plan_node_policy_denied", "denied");
    let route = SafeLaneFailureRoute::from_failure(&failure, SafeLaneReplanBudget::new(3));

    assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
    assert_eq!(route.reason, SafeLaneFailureRouteReason::PolicyDenied);
    assert_eq!(route.source, SafeLaneFailureRouteSource::BaseRouting);
}

#[test]
fn safe_lane_route_approval_required_failure_is_terminal_and_distinct() {
    let failure =
        TurnFailure::policy_denied("safe_lane_plan_node_approval_required", "approval required");
    let route = SafeLaneFailureRoute::from_failure(&failure, SafeLaneReplanBudget::new(3));

    assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
    assert_eq!(route.reason, SafeLaneFailureRouteReason::ApprovalRequired);
    assert_eq!(route.source, SafeLaneFailureRouteSource::BaseRouting);
}

#[test]
fn safe_lane_route_non_retryable_failure_is_terminal() {
    let failure = TurnFailure::non_retryable("safe_lane_plan_node_non_retryable_error", "bad");
    let route = SafeLaneFailureRoute::from_failure(&failure, SafeLaneReplanBudget::new(3));

    assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
    assert_eq!(
        route.reason,
        SafeLaneFailureRouteReason::NonRetryableFailure
    );
    assert_eq!(route.source, SafeLaneFailureRouteSource::BaseRouting);
}

#[test]
fn turn_failure_from_plan_failure_node_error_mapping_is_stable() {
    let cases = [
        (
            PlanNodeErrorKind::ApprovalRequired,
            TurnFailureKind::PolicyDenied,
            "safe_lane_plan_node_approval_required",
            false,
        ),
        (
            PlanNodeErrorKind::PolicyDenied,
            TurnFailureKind::PolicyDenied,
            "safe_lane_plan_node_policy_denied",
            false,
        ),
        (
            PlanNodeErrorKind::Retryable,
            TurnFailureKind::Retryable,
            "safe_lane_plan_node_retryable_error",
            true,
        ),
        (
            PlanNodeErrorKind::NonRetryable,
            TurnFailureKind::NonRetryable,
            "safe_lane_plan_node_non_retryable_error",
            false,
        ),
    ];

    for (node_kind, expected_kind, expected_code, expected_retryable) in cases {
        let failure = PlanRunFailure::NodeFailed {
            node_id: "tool-1".to_owned(),
            attempts_used: 1,
            last_error_kind: node_kind,
            last_error: "boom".to_owned(),
        };
        let mapped = turn_failure_from_plan_failure(&failure);
        assert_eq!(mapped.kind, expected_kind, "node_kind={node_kind:?}");
        assert_eq!(mapped.code, expected_code, "node_kind={node_kind:?}");
        assert_eq!(
            mapped.retryable, expected_retryable,
            "node_kind={node_kind:?}"
        );
    }
}

#[test]
fn turn_failure_from_plan_failure_static_failure_mapping_is_stable() {
    let failures = [
        PlanRunFailure::ValidationFailed("invalid".to_owned()),
        PlanRunFailure::TopologyResolutionFailed,
        PlanRunFailure::BudgetExceeded {
            attempts_used: 5,
            limit: 4,
        },
        PlanRunFailure::WallTimeExceeded {
            elapsed_ms: 1200,
            limit_ms: 1000,
        },
    ];

    for failure in failures {
        let mapped = turn_failure_from_plan_failure(&failure);
        assert_eq!(mapped.kind, TurnFailureKind::NonRetryable);
        assert!(!mapped.retryable);
        assert!(
            mapped.code.starts_with("safe_lane_plan_"),
            "unexpected code: {}",
            mapped.code
        );
    }
}

#[test]
fn safe_lane_event_sampling_keeps_critical_events() {
    let mut config = LoongConfig::default();
    config.conversation.safe_lane_emit_runtime_events = true;
    config.conversation.safe_lane_event_sample_every = 3;

    let emitted = should_emit_safe_lane_event(
        &config,
        "final_status",
        &json!({
            "round": 1
        }),
    );
    assert!(emitted, "critical final_status event must always emit");
}

#[test]
fn safe_lane_event_sampling_skips_non_critical_rounds() {
    let mut config = LoongConfig::default();
    config.conversation.safe_lane_emit_runtime_events = true;
    config.conversation.safe_lane_event_sample_every = 2;
    config.conversation.safe_lane_event_adaptive_sampling = false;

    let emit_round_0 = should_emit_safe_lane_event(
        &config,
        "plan_round_started",
        &json!({
            "round": 0
        }),
    );
    let emit_round_1 = should_emit_safe_lane_event(
        &config,
        "plan_round_started",
        &json!({
            "round": 1
        }),
    );

    assert!(emit_round_0, "round 0 should pass sampling gate");
    assert!(!emit_round_1, "round 1 should be sampled out");
}

#[test]
fn safe_lane_event_sampling_adaptive_mode_keeps_failure_pressure_events() {
    let mut config = LoongConfig::default();
    config.conversation.safe_lane_emit_runtime_events = true;
    config.conversation.safe_lane_event_sample_every = 4;
    config.conversation.safe_lane_event_adaptive_sampling = true;
    config
        .conversation
        .safe_lane_event_adaptive_failure_threshold = 1;

    let emitted = should_emit_safe_lane_event(
        &config,
        "plan_round_completed",
        &json!({
            "round": 1,
            "failure_code": "safe_lane_plan_node_retryable_error",
            "route_decision": "replan",
            "metrics": {
                "rounds_started": 2,
                "rounds_succeeded": 0,
                "rounds_failed": 1,
                "verify_failures": 0,
                "replans_triggered": 1,
                "total_attempts_used": 2
            }
        }),
    );

    assert!(
        emitted,
        "adaptive failure-pressure sampling should force emit for troubleshooting"
    );
}

#[test]
fn safe_lane_event_sampling_adaptive_mode_can_be_disabled() {
    let mut config = LoongConfig::default();
    config.conversation.safe_lane_emit_runtime_events = true;
    config.conversation.safe_lane_event_sample_every = 4;
    config.conversation.safe_lane_event_adaptive_sampling = false;
    config
        .conversation
        .safe_lane_event_adaptive_failure_threshold = 1;

    let emitted = should_emit_safe_lane_event(
        &config,
        "plan_round_completed",
        &json!({
            "round": 1,
            "failure_code": "safe_lane_plan_node_retryable_error",
            "route_decision": "replan",
            "metrics": {
                "rounds_started": 2,
                "rounds_succeeded": 0,
                "rounds_failed": 1,
                "verify_failures": 0,
                "replans_triggered": 1,
                "total_attempts_used": 2
            }
        }),
    );

    assert!(
        !emitted,
        "with adaptive sampling disabled, round-based sampling should still drop this event"
    );
}

#[test]
fn safe_lane_failure_pressure_counts_truncated_tool_output_stats() {
    let payload = json!({
        "tool_output_stats": {
            "output_lines": 1,
            "result_lines": 1,
            "truncated_result_lines": 1,
            "any_truncated": true,
            "truncation_ratio_milli": 1000
        }
    });
    assert_eq!(safe_lane_failure_pressure(&payload), 1);
}

#[test]
fn safe_lane_tool_output_stats_detect_truncated_result_lines() {
    let outputs = vec![
        "[ok] {\"payload_truncated\":true}".to_owned(),
        "[ok] {\"payload_truncated\":false}\n[tool_result_truncated] removed_chars=2".to_owned(),
        "plain diagnostic line".to_owned(),
    ];

    let stats = summarize_safe_lane_tool_output_stats(outputs.as_slice());
    assert_eq!(stats.output_lines, 4);
    assert_eq!(stats.result_lines, 3);
    assert_eq!(stats.truncated_result_lines, 2);
    assert_eq!(stats.truncation_ratio_milli(), 666);
    let encoded = stats.as_json();
    assert_eq!(encoded["any_truncated"], true);
    assert_eq!(encoded["truncation_ratio_milli"], 666);
}

#[test]
fn safe_lane_tool_output_stats_handles_mixed_multiline_blocks() {
    let outputs = vec![
        "\n[ok] {\"payload_truncated\":false}\nnot a result line\n[ok] {\"payload_truncated\":true}\n"
            .to_owned(),
        "[result] completed\n\n[ok] {\"payload_truncated\":false}".to_owned(),
    ];

    let stats = summarize_safe_lane_tool_output_stats(outputs.as_slice());
    assert_eq!(stats.output_lines, 5);
    assert_eq!(stats.result_lines, 4);
    assert_eq!(stats.truncated_result_lines, 1);
    assert_eq!(stats.truncation_ratio_milli(), 250);
    let encoded = stats.as_json();
    assert_eq!(encoded["any_truncated"], true);
    assert_eq!(encoded["truncation_ratio_milli"], 250);
}

#[test]
fn runtime_health_signal_marks_warn_on_truncation_pressure() {
    let mut config = LoongConfig::default();
    config
        .conversation
        .safe_lane_health_truncation_warn_threshold = 0.20;
    config
        .conversation
        .safe_lane_health_truncation_critical_threshold = 0.50;
    let metrics = SafeLaneExecutionMetrics {
        rounds_started: 2,
        tool_output_result_lines_total: 4,
        tool_output_truncated_result_lines_total: 1,
        ..SafeLaneExecutionMetrics::default()
    };

    let signal = derive_safe_lane_runtime_health_signal(&config, metrics, false, None);
    assert_eq!(signal.severity, "warn");
    assert!(
        signal
            .flags
            .iter()
            .any(|value| value.contains("truncation_pressure(0.250)"))
    );
}

#[test]
fn runtime_health_signal_marks_critical_on_terminal_instability() {
    let config = LoongConfig::default();
    let metrics = SafeLaneExecutionMetrics {
        rounds_started: 2,
        verify_failures: 1,
        replans_triggered: 1,
        tool_output_result_lines_total: 2,
        tool_output_truncated_result_lines_total: 1,
        ..SafeLaneExecutionMetrics::default()
    };

    let signal = derive_safe_lane_runtime_health_signal(
        &config,
        metrics,
        true,
        Some("safe_lane_plan_verify_failed_session_governor"),
    );
    assert_eq!(signal.severity, "critical");
    assert!(
        signal
            .flags
            .iter()
            .any(|value| value == "terminal_instability")
    );
}

#[test]
fn verify_anchor_policy_escalates_after_configured_failures() {
    let mut config = LoongConfig::default();
    config
        .conversation
        .safe_lane_verify_adaptive_anchor_escalation = true;
    config
        .conversation
        .safe_lane_verify_anchor_escalation_after_failures = 2;
    config
        .conversation
        .safe_lane_verify_anchor_escalation_min_matches = 1;

    assert_eq!(compute_safe_lane_verify_min_anchor_matches(&config, 0), 0);
    assert_eq!(compute_safe_lane_verify_min_anchor_matches(&config, 1), 0);
    assert_eq!(compute_safe_lane_verify_min_anchor_matches(&config, 2), 1);
    assert_eq!(compute_safe_lane_verify_min_anchor_matches(&config, 5), 1);
}

#[test]
fn verify_anchor_policy_escalation_can_be_disabled() {
    let mut config = LoongConfig::default();
    config
        .conversation
        .safe_lane_verify_adaptive_anchor_escalation = false;
    config
        .conversation
        .safe_lane_verify_anchor_escalation_after_failures = 1;
    config
        .conversation
        .safe_lane_verify_anchor_escalation_min_matches = 3;

    assert_eq!(compute_safe_lane_verify_min_anchor_matches(&config, 5), 0);
}

#[test]
fn backpressure_guard_blocks_replan_when_attempt_budget_exhausted() {
    let mut config = LoongConfig::default();
    config.conversation.safe_lane_backpressure_guard_enabled = true;
    config
        .conversation
        .safe_lane_backpressure_max_total_attempts = 2;
    config.conversation.safe_lane_backpressure_max_replans = 10;

    let route = SafeLaneFailureRoute {
        decision: SafeLaneFailureRouteDecision::Replan,
        reason: SafeLaneFailureRouteReason::RetryableFailure,
        source: SafeLaneFailureRouteSource::BaseRouting,
    };
    let metrics = SafeLaneExecutionMetrics {
        total_attempts_used: 2,
        ..SafeLaneExecutionMetrics::default()
    };
    let guarded = route.with_backpressure_guard(safe_lane_backpressure_budget(&config), metrics);
    assert_eq!(guarded.decision, SafeLaneFailureRouteDecision::Terminal);
    assert_eq!(
        guarded.reason,
        SafeLaneFailureRouteReason::BackpressureAttemptsExhausted
    );
    assert_eq!(
        guarded.source,
        SafeLaneFailureRouteSource::BackpressureGuard
    );
}

#[test]
fn backpressure_guard_blocks_replan_when_replan_budget_exhausted() {
    let mut config = LoongConfig::default();
    config.conversation.safe_lane_backpressure_guard_enabled = true;
    config
        .conversation
        .safe_lane_backpressure_max_total_attempts = 10;
    config.conversation.safe_lane_backpressure_max_replans = 1;

    let route = SafeLaneFailureRoute {
        decision: SafeLaneFailureRouteDecision::Replan,
        reason: SafeLaneFailureRouteReason::RetryableFailure,
        source: SafeLaneFailureRouteSource::BaseRouting,
    };
    let metrics = SafeLaneExecutionMetrics {
        replans_triggered: 1,
        ..SafeLaneExecutionMetrics::default()
    };
    let guarded = route.with_backpressure_guard(safe_lane_backpressure_budget(&config), metrics);
    assert_eq!(guarded.decision, SafeLaneFailureRouteDecision::Terminal);
    assert_eq!(
        guarded.reason,
        SafeLaneFailureRouteReason::BackpressureReplansExhausted
    );
    assert_eq!(
        guarded.source,
        SafeLaneFailureRouteSource::BackpressureGuard
    );
}

fn governor_history_with_summary(summary: SafeLaneEventSummary) -> SafeLaneGovernorHistorySignals {
    SafeLaneGovernorHistorySignals {
        summary,
        ..SafeLaneGovernorHistorySignals::default()
    }
}

#[test]
fn safe_lane_backpressure_budget_detects_attempt_exhaustion() {
    let budget = SafeLaneBackpressureBudget::new(2, 10);
    let metrics = SafeLaneExecutionMetrics {
        total_attempts_used: 2,
        ..SafeLaneExecutionMetrics::default()
    };

    assert_eq!(
        budget.continuation_decision(metrics.total_attempts_used, metrics.replans_triggered),
        SafeLaneContinuationBudgetDecision::Terminal {
            reason: SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
        }
    );
}

#[test]
fn decide_safe_lane_failure_route_applies_backpressure_after_retryable_base_route() {
    let mut config = LoongConfig::default();
    config.conversation.safe_lane_backpressure_guard_enabled = true;
    config
        .conversation
        .safe_lane_backpressure_max_total_attempts = 2;
    config.conversation.safe_lane_backpressure_max_replans = 10;

    let route = decide_safe_lane_failure_route(
        &config,
        &TurnFailure::retryable("safe_lane_plan_node_retryable_error", "transient"),
        SafeLaneReplanBudget::new(3),
        SafeLaneExecutionMetrics {
            total_attempts_used: 2,
            ..SafeLaneExecutionMetrics::default()
        },
        &SafeLaneSessionGovernorDecision::default(),
    );

    assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
    assert_eq!(
        route.reason,
        SafeLaneFailureRouteReason::BackpressureAttemptsExhausted
    );
    assert_eq!(route.source, SafeLaneFailureRouteSource::BackpressureGuard);
}

#[test]
fn decide_safe_lane_failure_route_applies_session_governor_override_to_exhausted_budget() {
    let config = LoongConfig::default();
    let route = decide_safe_lane_failure_route(
        &config,
        &TurnFailure::retryable("safe_lane_plan_node_retryable_error", "transient"),
        SafeLaneReplanBudget::new(1).after_replan(),
        SafeLaneExecutionMetrics::default(),
        &SafeLaneSessionGovernorDecision {
            force_no_replan: true,
            ..SafeLaneSessionGovernorDecision::default()
        },
    );

    assert_eq!(route.decision, SafeLaneFailureRouteDecision::Terminal);
    assert_eq!(
        route.reason,
        SafeLaneFailureRouteReason::SessionGovernorNoReplan
    );
    assert_eq!(route.source, SafeLaneFailureRouteSource::SessionGovernor);
}

#[test]
fn summarize_governor_history_signals_extracts_failure_samples() {
    let contents = [
        r#"{"type":"conversation_event","event":"final_status","payload":{"status":"failed","failure_code":"safe_lane_plan_backpressure_guard","route_reason":"backpressure_attempts_exhausted"}}"#,
        r#"{"type":"conversation_event","event":"final_status","payload":{"status":"succeeded"}}"#,
    ];

    let signals = summarize_governor_history_signals(contents.iter().copied());
    assert_eq!(signals.final_status_failed_samples, vec![true, false]);
    assert_eq!(signals.backpressure_failure_samples, vec![true, false]);
    assert_eq!(
        signals
            .summary
            .failure_code_counts
            .get("safe_lane_plan_backpressure_guard")
            .copied(),
        Some(1)
    );
}

#[test]
fn summarize_governor_history_signals_ignores_unknown_backpressure_like_strings() {
    let contents = [
        r#"{"type":"conversation_event","event":"final_status","payload":{"status":"failed","failure_code":"unknown_backpressure_hint","route_reason":"backpressure_noise"}}"#,
    ];

    let signals = summarize_governor_history_signals(contents.iter().copied());
    assert_eq!(signals.final_status_failed_samples, vec![true]);
    assert_eq!(signals.backpressure_failure_samples, vec![false]);
}

#[test]
fn session_governor_engages_on_failed_final_status_threshold() {
    let mut config = LoongConfig::default();
    config.conversation.safe_lane_session_governor_enabled = true;
    config
        .conversation
        .safe_lane_session_governor_failed_final_status_threshold = 2;
    config
        .conversation
        .safe_lane_session_governor_backpressure_failure_threshold = 9;
    config
        .conversation
        .safe_lane_session_governor_force_no_replan = true;
    config
        .conversation
        .safe_lane_session_governor_force_node_max_attempts = 1;

    let mut summary = SafeLaneEventSummary::default();
    summary.final_status_counts.insert("failed".to_owned(), 2);

    let history = governor_history_with_summary(summary);
    let decision = decide_safe_lane_session_governor(&config, &history);
    assert!(decision.engaged);
    assert!(decision.failed_threshold_triggered);
    assert!(!decision.backpressure_threshold_triggered);
    assert!(decision.force_no_replan);
    assert_eq!(decision.forced_node_max_attempts, Some(1));
}

#[test]
fn session_governor_engages_on_backpressure_threshold() {
    let mut config = LoongConfig::default();
    config.conversation.safe_lane_session_governor_enabled = true;
    config
        .conversation
        .safe_lane_session_governor_failed_final_status_threshold = 9;
    config
        .conversation
        .safe_lane_session_governor_backpressure_failure_threshold = 2;
    config
        .conversation
        .safe_lane_session_governor_force_node_max_attempts = 2;

    let mut summary = SafeLaneEventSummary::default();
    summary
        .failure_code_counts
        .insert("safe_lane_plan_backpressure_guard".to_owned(), 1);
    summary.failure_code_counts.insert(
        "safe_lane_plan_verify_failed_backpressure_guard".to_owned(),
        1,
    );

    let history = governor_history_with_summary(summary);
    let decision = decide_safe_lane_session_governor(&config, &history);
    assert!(decision.engaged);
    assert!(!decision.failed_threshold_triggered);
    assert!(decision.backpressure_threshold_triggered);
    assert_eq!(decision.backpressure_failure_events, 2);
    assert_eq!(decision.forced_node_max_attempts, Some(2));
}

#[test]
fn session_governor_stays_disabled_when_thresholds_not_reached() {
    let mut config = LoongConfig::default();
    config.conversation.safe_lane_session_governor_enabled = true;
    config
        .conversation
        .safe_lane_session_governor_failed_final_status_threshold = 3;
    config
        .conversation
        .safe_lane_session_governor_backpressure_failure_threshold = 2;

    let mut summary = SafeLaneEventSummary::default();
    summary.final_status_counts.insert("failed".to_owned(), 1);
    summary
        .failure_code_counts
        .insert("safe_lane_plan_backpressure_guard".to_owned(), 1);

    let history = governor_history_with_summary(summary);
    let decision = decide_safe_lane_session_governor(&config, &history);
    assert!(!decision.engaged);
    assert!(!decision.force_no_replan);
    assert_eq!(decision.forced_node_max_attempts, None);
}

#[test]
fn session_governor_engages_on_trend_threshold_when_counts_are_low() {
    let mut config = LoongConfig::default();
    config.conversation.safe_lane_session_governor_enabled = true;
    config
        .conversation
        .safe_lane_session_governor_failed_final_status_threshold = 9;
    config
        .conversation
        .safe_lane_session_governor_backpressure_failure_threshold = 9;
    config.conversation.safe_lane_session_governor_trend_enabled = true;
    config
        .conversation
        .safe_lane_session_governor_trend_min_samples = 4;
    config
        .conversation
        .safe_lane_session_governor_trend_ewma_alpha = 0.5;
    config
        .conversation
        .safe_lane_session_governor_trend_failure_ewma_threshold = 0.60;
    config
        .conversation
        .safe_lane_session_governor_trend_backpressure_ewma_threshold = 0.70;

    let mut summary = SafeLaneEventSummary::default();
    summary.final_status_counts.insert("failed".to_owned(), 1);
    let history = SafeLaneGovernorHistorySignals {
        history_load_status: SafeLaneGovernorHistoryLoadStatus::Loaded,
        history_load_error: None,
        summary,
        final_status_failed_samples: vec![false, true, true, true],
        backpressure_failure_samples: vec![false, false, false, false],
    };

    let decision = decide_safe_lane_session_governor(&config, &history);
    assert!(decision.engaged);
    assert!(!decision.failed_threshold_triggered);
    assert!(!decision.backpressure_threshold_triggered);
    assert!(decision.trend_threshold_triggered);
    assert!(
        decision
            .trend_failure_ewma
            .map(|value| value > 0.60)
            .unwrap_or(false)
    );
}

#[test]
fn session_governor_recovery_threshold_can_suppress_engagement() {
    let mut config = LoongConfig::default();
    config.conversation.safe_lane_session_governor_enabled = true;
    config
        .conversation
        .safe_lane_session_governor_failed_final_status_threshold = 1;
    config
        .conversation
        .safe_lane_session_governor_backpressure_failure_threshold = 9;
    config.conversation.safe_lane_session_governor_trend_enabled = true;
    config
        .conversation
        .safe_lane_session_governor_trend_min_samples = 4;
    config
        .conversation
        .safe_lane_session_governor_trend_ewma_alpha = 0.5;
    config
        .conversation
        .safe_lane_session_governor_trend_failure_ewma_threshold = 0.70;
    config
        .conversation
        .safe_lane_session_governor_recovery_success_streak = 3;
    config
        .conversation
        .safe_lane_session_governor_recovery_max_failure_ewma = 0.30;
    config
        .conversation
        .safe_lane_session_governor_recovery_max_backpressure_ewma = 0.10;

    let mut summary = SafeLaneEventSummary::default();
    summary.final_status_counts.insert("failed".to_owned(), 1);
    let history = SafeLaneGovernorHistorySignals {
        history_load_status: SafeLaneGovernorHistoryLoadStatus::Loaded,
        history_load_error: None,
        summary,
        final_status_failed_samples: vec![true, false, false, false, false],
        backpressure_failure_samples: vec![true, false, false, false, false],
    };

    let decision = decide_safe_lane_session_governor(&config, &history);
    assert!(decision.failed_threshold_triggered);
    assert!(!decision.trend_threshold_triggered);
    assert!(decision.recovery_threshold_triggered);
    assert_eq!(decision.recovery_success_streak, 4);
    assert!(!decision.engaged);
}

#[test]
fn session_governor_route_override_marks_no_replan_terminal_reason() {
    let route = SafeLaneFailureRoute {
        decision: SafeLaneFailureRouteDecision::Terminal,
        reason: SafeLaneFailureRouteReason::RoundBudgetExhausted,
        source: SafeLaneFailureRouteSource::BaseRouting,
    };
    let governor = SafeLaneSessionGovernorDecision {
        force_no_replan: true,
        ..SafeLaneSessionGovernorDecision::default()
    };
    let overridden = route.with_session_governor_override(&governor);
    assert_eq!(
        overridden.reason,
        SafeLaneFailureRouteReason::SessionGovernorNoReplan
    );
    assert_eq!(
        overridden.source,
        SafeLaneFailureRouteSource::SessionGovernor
    );
}

#[test]
fn terminal_verify_failure_uses_backpressure_error_code() {
    let failure = terminal_turn_failure_from_verify_failure(
        "retryable verify failure",
        true,
        SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
            source: SafeLaneFailureRouteSource::BackpressureGuard,
        },
    );
    assert_eq!(
        failure.code,
        "safe_lane_plan_verify_failed_backpressure_guard"
    );
    assert_eq!(failure.kind, TurnFailureKind::NonRetryable);
}

#[test]
fn safe_lane_terminal_verify_failure_code_prefers_budget_exhaustion_for_retryable_base_route() {
    let code = SafeLaneFailureRoute {
        decision: SafeLaneFailureRouteDecision::Terminal,
        reason: SafeLaneFailureRouteReason::RoundBudgetExhausted,
        source: SafeLaneFailureRouteSource::BaseRouting,
    }
    .terminal_verify_failure_code(true);
    assert_eq!(code, SafeLaneFailureCode::VerifyFailedBudgetExhausted);
}

#[test]
fn safe_lane_route_verify_summary_label_marks_backpressure_guard() {
    let label = SafeLaneFailureRoute {
        decision: SafeLaneFailureRouteDecision::Terminal,
        reason: SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
        source: SafeLaneFailureRouteSource::BackpressureGuard,
    }
    .verify_terminal_summary_label();
    assert_eq!(label, "verify_failed_backpressure_guard");
}

#[test]
fn safe_lane_route_profile_methods_encode_decision_and_source_labels() {
    let route = SafeLaneFailureRoute::replan(SafeLaneFailureRouteReason::RetryableFailure);
    assert!(route.should_replan());
    assert_eq!(route.decision_label(), "replan");
    assert_eq!(route.source_label(), "base_routing");

    let terminal = SafeLaneFailureRoute::terminal_with_source(
        SafeLaneFailureRouteReason::SessionGovernorNoReplan,
        SafeLaneFailureRouteSource::SessionGovernor,
    );
    assert!(!terminal.should_replan());
    assert_eq!(terminal.decision_label(), "terminal");
    assert_eq!(terminal.source_label(), "session_governor");
}

#[test]
fn safe_lane_route_backpressure_transition_is_localized_on_route() {
    let route = SafeLaneFailureRoute::replan(SafeLaneFailureRouteReason::RetryableFailure)
        .with_backpressure_guard(
            Some(SafeLaneBackpressureBudget::new(2, 10)),
            SafeLaneExecutionMetrics {
                total_attempts_used: 2,
                ..SafeLaneExecutionMetrics::default()
            },
        );
    assert!(!route.should_replan());
    assert_eq!(
        route.reason,
        SafeLaneFailureRouteReason::BackpressureAttemptsExhausted
    );
    assert_eq!(route.source, SafeLaneFailureRouteSource::BackpressureGuard);
}

#[test]
fn terminal_verify_failure_uses_budget_exhaustion_error_code() {
    let failure = terminal_turn_failure_from_verify_failure(
        "retryable verify failure",
        true,
        SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::RoundBudgetExhausted,
            source: SafeLaneFailureRouteSource::BaseRouting,
        },
    );
    assert_eq!(
        failure.code,
        "safe_lane_plan_verify_failed_budget_exhausted"
    );
    assert_eq!(failure.kind, TurnFailureKind::NonRetryable);
}

#[test]
fn terminal_verify_failure_uses_session_governor_error_code() {
    let failure = terminal_turn_failure_from_verify_failure(
        "retryable verify failure",
        true,
        SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
            source: SafeLaneFailureRouteSource::SessionGovernor,
        },
    );
    assert_eq!(
        failure.code,
        "safe_lane_plan_verify_failed_session_governor"
    );
    assert_eq!(failure.kind, TurnFailureKind::NonRetryable);
}

#[test]
fn terminal_plan_failure_uses_session_governor_error_code() {
    let failure = PlanRunFailure::NodeFailed {
        node_id: "tool-1".to_owned(),
        attempts_used: 1,
        last_error_kind: PlanNodeErrorKind::Retryable,
        last_error: "transient".to_owned(),
    };
    let route = SafeLaneFailureRoute {
        decision: SafeLaneFailureRouteDecision::Terminal,
        reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
        source: SafeLaneFailureRouteSource::SessionGovernor,
    };
    assert_eq!(
        route.terminal_plan_failure_code(),
        Some(SafeLaneFailureCode::PlanSessionGovernorNoReplan)
    );
    let result = terminal_turn_result_from_plan_failure_with_route(failure, route);
    let meta = result.failure().expect("failure metadata");
    assert_eq!(meta.code, "safe_lane_plan_session_governor_no_replan");
    assert_eq!(meta.kind, TurnFailureKind::NonRetryable);
}

#[test]
fn decide_safe_lane_verify_failure_action_replans_with_remaining_budget() {
    let decision = decide_safe_lane_verify_failure_action(
        "missing anchors",
        true,
        SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Replan,
            reason: SafeLaneFailureRouteReason::RetryableFailure,
            source: SafeLaneFailureRouteSource::BaseRouting,
        },
    );

    if let SafeLaneRoundDecision::Replan {
        reason,
        next_plan_start_tool_index,
        next_seed_tool_outputs,
    } = decision
    {
        assert_eq!(reason, "verify_failed");
        assert_eq!(next_plan_start_tool_index, 0);
        assert!(next_seed_tool_outputs.is_empty());
    } else {
        panic!("unexpected decision: {decision:?}");
    }
}

#[test]
fn decide_safe_lane_verify_failure_action_terminalizes_with_governor_code() {
    let decision = decide_safe_lane_verify_failure_action(
        "missing anchors",
        true,
        SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
            source: SafeLaneFailureRouteSource::SessionGovernor,
        },
    );

    if let SafeLaneRoundDecision::Finalize {
        result: TurnResult::ToolError(failure),
    } = decision
    {
        assert_eq!(
            failure.code,
            "safe_lane_plan_verify_failed_session_governor"
        );
        assert_eq!(failure.kind, TurnFailureKind::NonRetryable);
    } else {
        panic!("unexpected decision: {decision:?}");
    }
}

#[test]
fn decide_safe_lane_plan_failure_action_replans_with_failed_subgraph_cursor() {
    let decision = decide_safe_lane_plan_failure_action(
        PlanRunFailure::NodeFailed {
            node_id: "tool-2".to_owned(),
            attempts_used: 1,
            last_error_kind: PlanNodeErrorKind::Retryable,
            last_error: "transient".to_owned(),
        },
        SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Replan,
            reason: SafeLaneFailureRouteReason::RetryableFailure,
            source: SafeLaneFailureRouteSource::BaseRouting,
        },
        1,
        vec!["[ok] {\"path\":\"note.md\"}".to_owned()],
    );

    if let SafeLaneRoundDecision::Replan {
        reason,
        next_plan_start_tool_index,
        next_seed_tool_outputs,
    } = decision
    {
        assert_eq!(
            reason,
            "node_failed node=tool-2 error_kind=Retryable reason=transient"
        );
        assert_eq!(next_plan_start_tool_index, 1);
        assert_eq!(next_seed_tool_outputs.len(), 1);
        assert!(next_seed_tool_outputs[0].contains("note.md"));
    } else {
        panic!("unexpected decision: {decision:?}");
    }
}

#[test]
fn decide_safe_lane_plan_failure_action_terminalizes_with_backpressure_code() {
    let decision = decide_safe_lane_plan_failure_action(
        PlanRunFailure::NodeFailed {
            node_id: "tool-1".to_owned(),
            attempts_used: 2,
            last_error_kind: PlanNodeErrorKind::Retryable,
            last_error: "transient".to_owned(),
        },
        SafeLaneFailureRoute {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
            source: SafeLaneFailureRouteSource::BackpressureGuard,
        },
        0,
        Vec::new(),
    );

    if let SafeLaneRoundDecision::Finalize {
        result: TurnResult::ToolError(failure),
    } = decision
    {
        assert_eq!(failure.code, "safe_lane_plan_backpressure_guard");
        assert_eq!(failure.kind, TurnFailureKind::NonRetryable);
    } else {
        panic!("unexpected decision: {decision:?}");
    }
}
