use super::*;
use crate::conversation::TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS;

const SAFE_LANE_VERIFY_OUTPUT_NON_EMPTY: bool = true;
const SAFE_LANE_VERIFY_MIN_OUTPUT_CHARS: usize = 8;
const SAFE_LANE_VERIFY_REQUIRE_STATUS_PREFIX: bool = true;
const SAFE_LANE_PLAN_MAX_WALL_TIME_MS: u64 = 30_000;
const SAFE_LANE_VERIFY_DENY_MARKERS: &[&str] = &[
    "tool_failure",
    "provider_error",
    "no_kernel_context",
    "tool_not_found",
];

pub(super) struct SafeLanePlanNodeExecutor<'a> {
    pub(super) tool_intents: &'a [ToolIntent],
    pub(super) session_context: &'a SessionContext,
    pub(super) app_dispatcher: &'a dyn AppToolDispatcher,
    pub(super) binding: ConversationRuntimeBinding<'a>,
    pub(super) ingress: Option<&'a ConversationIngressContext>,
    pub(super) tool_outputs: Mutex<Vec<String>>,
    pub(super) tool_result_payload_summary_limit_chars: usize,
}

impl<'a> SafeLanePlanNodeExecutor<'a> {
    pub(super) fn new(
        tool_intents: &'a [ToolIntent],
        session_context: &'a SessionContext,
        app_dispatcher: &'a dyn AppToolDispatcher,
        binding: ConversationRuntimeBinding<'a>,
        ingress: Option<&'a ConversationIngressContext>,
        seed_tool_outputs: Vec<String>,
        tool_result_payload_summary_limit_chars: usize,
    ) -> Self {
        Self {
            tool_intents,
            session_context,
            app_dispatcher,
            binding,
            ingress,
            tool_outputs: Mutex::new(seed_tool_outputs),
            tool_result_payload_summary_limit_chars,
        }
    }

    pub(super) async fn tool_outputs_snapshot(&self) -> Vec<String> {
        self.tool_outputs.lock().await.clone()
    }
}

#[async_trait]
impl PlanNodeExecutor for SafeLanePlanNodeExecutor<'_> {
    async fn execute(&self, node: &PlanNode, _attempt: u8) -> Result<(), PlanNodeError> {
        match node.kind {
            PlanNodeKind::Tool => {
                let index = parse_tool_node_index(node.id.as_str())?;
                let intent = self.tool_intents.get(index).ok_or_else(|| {
                    PlanNodeError::non_retryable(format!(
                        "missing tool intent for node `{}`",
                        node.id
                    ))
                })?;
                let output = execute_single_tool_intent(
                    intent,
                    self.session_context,
                    self.app_dispatcher,
                    self.binding,
                    self.ingress,
                    self.tool_result_payload_summary_limit_chars,
                )
                .await?;
                self.tool_outputs.lock().await.push(output);
                Ok(())
            }
            PlanNodeKind::Verify => {
                if !SAFE_LANE_VERIFY_OUTPUT_NON_EMPTY {
                    return Ok(());
                }
                let outputs = self.tool_outputs.lock().await;
                if outputs.is_empty() || outputs.iter().any(|line| line.trim().is_empty()) {
                    return Err(PlanNodeError::non_retryable(
                        "verify_failed:empty_tool_output".to_owned(),
                    ));
                }
                Ok(())
            }
            PlanNodeKind::Transform | PlanNodeKind::Respond => Ok(()),
        }
    }
}

pub(super) fn parse_tool_node_index(node_id: &str) -> Result<usize, PlanNodeError> {
    let suffix = node_id
        .strip_prefix("tool-")
        .ok_or_else(|| PlanNodeError::non_retryable(format!("invalid tool node id `{node_id}`")))?;
    let parsed = suffix.parse::<usize>().map_err(|error| {
        PlanNodeError::non_retryable(format!("invalid tool node id `{node_id}`: {error}"))
    })?;
    if parsed == 0 {
        return Err(PlanNodeError::non_retryable(format!(
            "invalid tool node ordinal in `{node_id}`"
        )));
    }
    Ok(parsed - 1)
}

pub(super) async fn execute_single_tool_intent(
    intent: &ToolIntent,
    session_context: &SessionContext,
    app_dispatcher: &dyn AppToolDispatcher,
    binding: ConversationRuntimeBinding<'_>,
    ingress: Option<&ConversationIngressContext>,
    payload_summary_limit_chars: usize,
) -> Result<String, PlanNodeError> {
    let engine = TurnEngine::with_tool_result_payload_summary_limit(1, payload_summary_limit_chars);
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![intent.clone()],
        raw_meta: Value::Null,
    };

    match engine
        .execute_turn_in_context(&turn, session_context, app_dispatcher, binding, ingress)
        .await
    {
        TurnResult::FinalText(output) => Ok(output),
        TurnResult::StreamingText(text) => Ok(text),
        TurnResult::StreamingDone(text) => Ok(text),
        TurnResult::NeedsApproval(requirement) => Err(PlanNodeError::policy_denied(
            format_approval_required_reply("", &requirement),
        )),
        TurnResult::ToolDenied(failure) => Err(PlanNodeError::policy_denied(failure.reason)),
        TurnResult::ToolError(failure) => Err(PlanNodeError {
            kind: match failure.kind {
                TurnFailureKind::Retryable => PlanNodeErrorKind::Retryable,
                TurnFailureKind::PolicyDenied
                | TurnFailureKind::NonRetryable
                | TurnFailureKind::Provider => PlanNodeErrorKind::NonRetryable,
            },
            message: failure.reason,
        }),
        TurnResult::ProviderError(failure) => Err(PlanNodeError {
            kind: PlanNodeErrorKind::NonRetryable,
            message: failure.reason,
        }),
    }
}

#[derive(Debug, Clone)]
pub(super) struct SafeLaneRoundExecution {
    pub(super) report: PlanRunReport,
    pub(super) tool_outputs: Vec<String>,
    pub(super) tool_output_stats: SafeLaneToolOutputStats,
}

pub(super) async fn execute_turn_with_safe_lane_plan<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_id: &str,
    lane_decision: &LaneDecision,
    turn: &ProviderTurn,
    session_context: &SessionContext,
    app_dispatcher: &dyn AppToolDispatcher,
    binding: ConversationRuntimeBinding<'_>,
    ingress: Option<&ConversationIngressContext>,
) -> SafeLaneTurnOutcome {
    let governor_history_signals =
        load_safe_lane_history_signals_for_governor(config, session_id, binding).await;
    let governor = decide_safe_lane_session_governor(config, &governor_history_signals);

    emit_safe_lane_event(
        config,
        runtime,
        session_id,
        "lane_selected",
        json!({
            "lane": "safe",
            "risk_score": lane_decision.risk_score,
            "complexity_score": lane_decision.complexity_score,
            "reasons": lane_decision.reasons.clone(),
            "tool_intents": turn.tool_intents.len(),
            "session_governor": governor.as_json(),
        }),
        binding,
    )
    .await;

    let mut state = SafeLanePlanLoopState::new(config, governor);

    loop {
        if let Some(min_anchor_matches) = state.refresh_verify_policy(config) {
            emit_safe_lane_event(
                config,
                runtime,
                session_id,
                "verify_policy_adjusted",
                json!({
                    "round": state.round(),
                    "policy": "adaptive_anchor_escalation",
                    "min_anchor_matches": min_anchor_matches,
                    "verify_failures": state.metrics.verify_failures,
                    "escalation_after_failures": 2,
                    "metrics": state.metrics.as_json(),
                }),
                binding,
            )
            .await;
        }

        state.note_round_started();
        emit_safe_lane_event(
            config,
            runtime,
            session_id,
            "plan_round_started",
            json!({
                "round": state.round(),
                "start_tool_index": state.plan_start_tool_index,
                "tool_node_max_attempts": state.tool_node_max_attempts(),
                "effective_max_rounds": state.max_rounds(),
                "effective_max_node_attempts": state.max_node_attempts(),
                "verify_min_anchor_matches": state.adaptive_verify_policy.min_anchor_matches,
                "session_governor": state.governor.as_json(),
                "metrics": state.metrics.as_json(),
            }),
            binding,
        )
        .await;

        let round_execution = evaluate_safe_lane_round(
            config,
            lane_decision,
            turn,
            session_context,
            app_dispatcher,
            binding,
            ingress,
            &state,
        )
        .await;
        state.record_round_execution(&round_execution.report, round_execution.tool_output_stats);

        match round_execution.report.status.clone() {
            PlanRunStatus::Succeeded => {
                state.note_round_succeeded();
                emit_safe_lane_event(
                    config,
                    runtime,
                    session_id,
                    "plan_round_completed",
                    json!({
                        "round": state.round(),
                        "status": "succeeded",
                        "attempts_used": round_execution.report.attempts_used,
                        "elapsed_ms": round_execution.report.elapsed_ms,
                        "tool_output_stats": round_execution.tool_output_stats.as_json(),
                        "health_signal": derive_safe_lane_runtime_health_signal(
                            config,
                            state.metrics,
                            false,
                            None,
                        )
                        .as_json(),
                        "metrics": state.metrics.as_json(),
                    }),
                    binding,
                )
                .await;
                let tool_output = round_execution.tool_outputs.join("\n");
                let verify_report = verify_safe_lane_final_output(
                    config,
                    tool_output.as_str(),
                    turn.tool_intents.as_slice(),
                    state.adaptive_verify_policy,
                );
                if verify_report.passed {
                    emit_safe_lane_event(
                        config,
                        runtime,
                        session_id,
                        "final_status",
                        json!({
                            "status": "succeeded",
                            "round": state.round(),
                            "tool_output_stats": round_execution.tool_output_stats.as_json(),
                            "health_signal": derive_safe_lane_runtime_health_signal(
                                config,
                                state.metrics,
                                false,
                                None,
                            )
                            .as_json(),
                            "metrics": state.metrics.as_json(),
                        }),
                        binding,
                    )
                    .await;
                    return SafeLaneTurnOutcome::without_terminal_route(TurnResult::FinalText(
                        tool_output,
                    ));
                }

                let verify_error = verify_report.failure_reasons.join(",");
                let failure_codes = verify_report
                    .failure_codes
                    .iter()
                    .map(format_verification_failure_code)
                    .collect::<Vec<_>>();
                let retryable_verify_failure =
                    should_replan_for_verification_failure(&verify_report);
                let verify_failure = turn_failure_from_verify_failure(
                    verify_error.as_str(),
                    retryable_verify_failure,
                );
                state.note_verify_failure();
                let verify_route = decide_safe_lane_failure_route(
                    config,
                    &verify_failure,
                    state.replan_budget,
                    state.metrics,
                    &state.governor,
                );
                emit_safe_lane_event(
                    config,
                    runtime,
                    session_id,
                    "verify_failed",
                    json!({
                        "round": state.round(),
                        "error": verify_error.clone(),
                        "failure_codes": failure_codes,
                        "retryable": retryable_verify_failure,
                        "failure_kind": format_turn_failure_kind(verify_failure.kind),
                        "failure_code": verify_failure.code.clone(),
                        "failure_retryable": verify_failure.retryable,
                        "route_decision": verify_route.decision_label(),
                        "route_reason": verify_route.reason.as_str(),
                        "route_source": verify_route.source_label(),
                        "tool_output_stats": round_execution.tool_output_stats.as_json(),
                        "health_signal": derive_safe_lane_runtime_health_signal(
                            config,
                            state.metrics,
                            false,
                            None,
                        )
                        .as_json(),
                        "metrics": state.metrics.as_json(),
                    }),
                    binding,
                )
                .await;

                match decide_safe_lane_verify_failure_action(
                    verify_error.as_str(),
                    retryable_verify_failure,
                    verify_route,
                ) {
                    SafeLaneRoundDecision::Finalize { result } => {
                        let failure_meta = result.failure();
                        emit_safe_lane_event(
                            config,
                            runtime,
                            session_id,
                            "final_status",
                            json!({
                                "status": "failed",
                                "round": state.round(),
                                "failure": verify_route.verify_terminal_summary_label(),
                                "failure_kind": failure_meta
                                    .map(|failure| format_turn_failure_kind(failure.kind)),
                                "failure_code": failure_meta.map(|failure| failure.code.clone()),
                                "failure_retryable": failure_meta.map(|failure| failure.retryable),
                                "route_decision": verify_route.decision_label(),
                                "route_reason": verify_route.reason.as_str(),
                                "route_source": verify_route.source_label(),
                                "tool_output_stats": round_execution.tool_output_stats.as_json(),
                                "health_signal": derive_safe_lane_runtime_health_signal(
                                    config,
                                    state.metrics,
                                    true,
                                    failure_meta.map(|failure| failure.code.as_str()),
                                )
                                .as_json(),
                                "metrics": state.metrics.as_json(),
                            }),
                            binding,
                        )
                        .await;
                        return SafeLaneTurnOutcome::with_terminal_route(result, verify_route);
                    }
                    SafeLaneRoundDecision::Replan {
                        reason,
                        next_plan_start_tool_index,
                        next_seed_tool_outputs,
                    } => {
                        state.note_replan(next_plan_start_tool_index, next_seed_tool_outputs);
                        emit_safe_lane_event(
                            config,
                            runtime,
                            session_id,
                            "replan_triggered",
                            json!({
                                "round": state.round(),
                                "reason": reason,
                                "detail": verify_error,
                                "route_decision": verify_route.decision_label(),
                                "route_reason": verify_route.reason.as_str(),
                                "route_source": verify_route.source_label(),
                                "tool_output_stats": round_execution.tool_output_stats.as_json(),
                                "health_signal": derive_safe_lane_runtime_health_signal(
                                    config,
                                    state.metrics,
                                    false,
                                    None,
                                )
                                .as_json(),
                                "metrics": state.metrics.as_json(),
                            }),
                            binding,
                        )
                        .await;
                    }
                }
            }
            PlanRunStatus::Failed(failure) => {
                state.note_round_failed();
                let round_failure_meta = turn_failure_from_plan_failure(&failure);
                let route = decide_safe_lane_failure_route(
                    config,
                    &round_failure_meta,
                    state.replan_budget,
                    state.metrics,
                    &state.governor,
                );
                let failure_summary = summarize_plan_failure(&failure);
                emit_safe_lane_event(
                    config,
                    runtime,
                    session_id,
                    "plan_round_completed",
                    json!({
                        "round": state.round(),
                        "status": "failed",
                        "attempts_used": round_execution.report.attempts_used,
                        "elapsed_ms": round_execution.report.elapsed_ms,
                        "failure": failure_summary.clone(),
                        "failure_kind": format_turn_failure_kind(round_failure_meta.kind),
                        "failure_code": round_failure_meta.code.clone(),
                        "failure_retryable": round_failure_meta.retryable,
                        "route_decision": route.decision_label(),
                        "route_reason": route.reason.as_str(),
                        "route_source": route.source_label(),
                        "tool_output_stats": round_execution.tool_output_stats.as_json(),
                        "health_signal": derive_safe_lane_runtime_health_signal(
                            config,
                            state.metrics,
                            false,
                            None,
                        )
                        .as_json(),
                        "metrics": state.metrics.as_json(),
                    }),
                    binding,
                )
                .await;
                let (next_start_tool_index, next_seed_outputs) = if route.should_replan() {
                    let (next_start_tool_index, next_seed_outputs) = derive_replan_cursor(
                        &failure,
                        round_execution.tool_outputs.as_slice(),
                        turn.tool_intents.len(),
                    );
                    (next_start_tool_index, next_seed_outputs)
                } else {
                    (0, Vec::new())
                };
                match decide_safe_lane_plan_failure_action(
                    failure.clone(),
                    route,
                    next_start_tool_index,
                    next_seed_outputs,
                ) {
                    SafeLaneRoundDecision::Finalize { result } => {
                        let failure_meta = result.failure();
                        emit_safe_lane_event(
                            config,
                            runtime,
                            session_id,
                            "final_status",
                            json!({
                                "status": "failed",
                                "round": state.round(),
                                "failure": failure_summary,
                                "failure_kind": failure_meta
                                    .map(|failure| format_turn_failure_kind(failure.kind)),
                                "failure_code": failure_meta.map(|failure| failure.code.clone()),
                                "failure_retryable": failure_meta.map(|failure| failure.retryable),
                                "route_decision": route.decision_label(),
                                "route_reason": route.reason.as_str(),
                                "route_source": route.source_label(),
                                "tool_output_stats": round_execution.tool_output_stats.as_json(),
                                "health_signal": derive_safe_lane_runtime_health_signal(
                                    config,
                                    state.metrics,
                                    true,
                                    failure_meta.map(|failure| failure.code.as_str()),
                                )
                                .as_json(),
                                "metrics": state.metrics.as_json(),
                            }),
                            binding,
                        )
                        .await;
                        return SafeLaneTurnOutcome::with_terminal_route(result, route);
                    }
                    SafeLaneRoundDecision::Replan {
                        reason,
                        next_plan_start_tool_index,
                        next_seed_tool_outputs,
                    } => {
                        let seeded_outputs_count = next_seed_tool_outputs.len();
                        state.note_replan(next_plan_start_tool_index, next_seed_tool_outputs);
                        emit_safe_lane_event(
                            config,
                            runtime,
                            session_id,
                            "replan_triggered",
                            json!({
                                "round": state.round(),
                                "reason": reason,
                                "restart_tool_index": state.plan_start_tool_index,
                                "seeded_outputs": seeded_outputs_count,
                                "route_decision": route.decision_label(),
                                "route_reason": route.reason.as_str(),
                                "route_source": route.source_label(),
                                "tool_output_stats": round_execution.tool_output_stats.as_json(),
                                "health_signal": derive_safe_lane_runtime_health_signal(
                                    config,
                                    state.metrics,
                                    false,
                                    None,
                                )
                                .as_json(),
                                "metrics": state.metrics.as_json(),
                            }),
                            binding,
                        )
                        .await;
                    }
                }
            }
        }

        state.advance_round();
    }
}

pub(super) async fn evaluate_safe_lane_round(
    config: &LoongConfig,
    lane_decision: &LaneDecision,
    turn: &ProviderTurn,
    session_context: &SessionContext,
    app_dispatcher: &dyn AppToolDispatcher,
    binding: ConversationRuntimeBinding<'_>,
    ingress: Option<&ConversationIngressContext>,
    state: &SafeLanePlanLoopState,
) -> SafeLaneRoundExecution {
    let plan = build_safe_lane_plan_graph(
        config,
        lane_decision,
        turn,
        state.tool_node_max_attempts(),
        state.plan_start_tool_index,
    );
    let executor = SafeLanePlanNodeExecutor::new(
        turn.tool_intents.as_slice(),
        session_context,
        app_dispatcher,
        binding,
        ingress,
        state.seed_tool_outputs.clone(),
        TOOL_RESULT_PAYLOAD_SUMMARY_LIMIT_CHARS,
    );
    let report = PlanExecutor::execute(&plan, &executor).await;
    let tool_outputs = executor.tool_outputs_snapshot().await;
    let tool_output_stats = summarize_safe_lane_tool_output_stats(tool_outputs.as_slice());

    SafeLaneRoundExecution {
        report,
        tool_outputs,
        tool_output_stats,
    }
}

pub(super) fn build_safe_lane_plan_graph(
    config: &LoongConfig,
    lane_decision: &LaneDecision,
    turn: &ProviderTurn,
    tool_node_max_attempts: u8,
    start_tool_index: usize,
) -> PlanGraph {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let node_risk_tier = select_safe_lane_risk_tier(config, lane_decision);
    let normalized_start = start_tool_index.min(turn.tool_intents.len());
    for (index, intent) in turn.tool_intents.iter().enumerate().skip(normalized_start) {
        let visible_tool_name = effective_followup_visible_tool_name(intent);
        nodes.push(PlanNode {
            id: format!("tool-{}", index + 1),
            kind: PlanNodeKind::Tool,
            label: format!("invoke `{visible_tool_name}`"),
            tool_name: Some(visible_tool_name),
            timeout_ms: 3_000,
            max_attempts: tool_node_max_attempts,
            risk_tier: node_risk_tier,
        });
    }

    if SAFE_LANE_VERIFY_OUTPUT_NON_EMPTY {
        nodes.push(PlanNode {
            id: "verify-1".to_owned(),
            kind: PlanNodeKind::Verify,
            label: "verify non-empty tool outputs".to_owned(),
            tool_name: None,
            timeout_ms: 500,
            max_attempts: 1,
            risk_tier: RiskTier::Medium,
        });
    }

    nodes.push(PlanNode {
        id: "respond-1".to_owned(),
        kind: PlanNodeKind::Respond,
        label: "compose final response".to_owned(),
        tool_name: None,
        timeout_ms: 500,
        max_attempts: 1,
        risk_tier: RiskTier::Low,
    });

    for pair in nodes.windows(2) {
        let [from, to] = pair else {
            continue;
        };
        edges.push(PlanEdge {
            from: from.id.clone(),
            to: to.id.clone(),
        });
    }

    let max_total_attempts = nodes
        .iter()
        .map(|node| node.max_attempts as usize)
        .sum::<usize>()
        .max(1);
    PlanGraph {
        version: PLAN_GRAPH_VERSION.to_owned(),
        nodes,
        edges,
        budget: PlanBudget {
            max_nodes: 16,
            max_total_attempts,
            max_wall_time_ms: SAFE_LANE_PLAN_MAX_WALL_TIME_MS,
        },
    }
}

pub(super) fn verify_safe_lane_final_output(
    _config: &LoongConfig,
    output: &str,
    tool_intents: &[ToolIntent],
    adaptive_policy: SafeLaneAdaptiveVerifyPolicyState,
) -> PlanVerificationReport {
    let policy = PlanVerificationPolicy {
        require_non_empty: SAFE_LANE_VERIFY_OUTPUT_NON_EMPTY,
        min_output_chars: SAFE_LANE_VERIFY_MIN_OUTPUT_CHARS,
        require_status_prefix: SAFE_LANE_VERIFY_REQUIRE_STATUS_PREFIX,
        deny_markers: SAFE_LANE_VERIFY_DENY_MARKERS
            .iter()
            .map(|marker| marker.trim().to_ascii_lowercase())
            .filter(|marker| !marker.is_empty())
            .collect(),
    };
    let semantic_anchors = collect_semantic_anchors(tool_intents);
    let context = PlanVerificationContext {
        expected_result_lines: tool_intents.len().max(1),
        semantic_anchors,
        min_anchor_matches: adaptive_policy.min_anchor_matches,
    };
    verify_output(output, &context, &policy)
}
