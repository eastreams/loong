use super::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct SafeLaneExecutionMetrics {
    pub(super) rounds_started: u32,
    pub(super) rounds_succeeded: u32,
    pub(super) rounds_failed: u32,
    pub(super) verify_failures: u32,
    pub(super) replans_triggered: u32,
    pub(super) total_attempts_used: u64,
    pub(super) tool_output_result_lines_total: u64,
    pub(super) tool_output_truncated_result_lines_total: u64,
}

impl SafeLaneExecutionMetrics {
    pub(super) fn record_tool_output_stats(&mut self, stats: SafeLaneToolOutputStats) {
        self.tool_output_result_lines_total = self
            .tool_output_result_lines_total
            .saturating_add(stats.result_lines as u64);
        self.tool_output_truncated_result_lines_total = self
            .tool_output_truncated_result_lines_total
            .saturating_add(stats.truncated_result_lines as u64);
    }

    pub(super) fn aggregate_tool_truncation_ratio_milli(self) -> Option<u32> {
        if self.tool_output_result_lines_total == 0 {
            return None;
        }
        Some(
            self.tool_output_truncated_result_lines_total
                .saturating_mul(1000)
                .saturating_div(self.tool_output_result_lines_total)
                .min(u32::MAX as u64) as u32,
        )
    }

    pub(super) fn as_json(self) -> Value {
        json!({
            "rounds_started": self.rounds_started,
            "rounds_succeeded": self.rounds_succeeded,
            "rounds_failed": self.rounds_failed,
            "verify_failures": self.verify_failures,
            "replans_triggered": self.replans_triggered,
            "total_attempts_used": self.total_attempts_used,
            "tool_output_result_lines_total": self.tool_output_result_lines_total,
            "tool_output_truncated_result_lines_total": self.tool_output_truncated_result_lines_total,
            "tool_output_aggregate_truncation_ratio_milli": self.aggregate_tool_truncation_ratio_milli(),
        })
    }
}

#[derive(Debug, Clone)]
pub(super) struct SafeLanePlanLoopState {
    pub(super) governor: SafeLaneSessionGovernorDecision,
    pub(super) replan_budget: SafeLaneReplanBudget,
    pub(super) tool_node_attempt_budget: EscalatingAttemptBudget,
    pub(super) plan_start_tool_index: usize,
    pub(super) seed_tool_outputs: Vec<String>,
    pub(super) metrics: SafeLaneExecutionMetrics,
    pub(super) adaptive_verify_policy: SafeLaneAdaptiveVerifyPolicyState,
}

impl SafeLanePlanLoopState {
    pub(super) fn new(config: &LoongConfig, governor: SafeLaneSessionGovernorDecision) -> Self {
        let force_no_replan = governor.force_no_replan;
        let mut tool_node_max_attempts = config.conversation.safe_lane_node_max_attempts.max(1);
        if let Some(forced_node_max_attempts) = governor.forced_node_max_attempts {
            tool_node_max_attempts = tool_node_max_attempts.min(forced_node_max_attempts.max(1));
        }
        let mut max_node_attempts = config
            .conversation
            .safe_lane_replan_max_node_attempts
            .max(tool_node_max_attempts);
        if let Some(forced_node_max_attempts) = governor.forced_node_max_attempts {
            max_node_attempts = max_node_attempts.min(forced_node_max_attempts.max(1));
        }

        Self {
            governor,
            replan_budget: SafeLaneReplanBudget::new(if force_no_replan {
                0
            } else {
                config.conversation.safe_lane_replan_max_rounds
            }),
            tool_node_attempt_budget: EscalatingAttemptBudget::new(
                tool_node_max_attempts,
                max_node_attempts,
            ),
            plan_start_tool_index: 0,
            seed_tool_outputs: Vec::new(),
            metrics: SafeLaneExecutionMetrics::default(),
            adaptive_verify_policy: SafeLaneAdaptiveVerifyPolicyState::default(),
        }
    }

    pub(super) fn refresh_verify_policy(&mut self, config: &LoongConfig) -> Option<usize> {
        let next_min_anchor_matches =
            compute_safe_lane_verify_min_anchor_matches(config, self.metrics.verify_failures);
        if next_min_anchor_matches == self.adaptive_verify_policy.min_anchor_matches {
            return None;
        }
        self.adaptive_verify_policy.min_anchor_matches = next_min_anchor_matches;
        (next_min_anchor_matches > 0).then_some(next_min_anchor_matches)
    }

    pub(super) fn note_round_started(&mut self) {
        self.metrics.rounds_started = self.metrics.rounds_started.saturating_add(1);
    }

    pub(super) fn record_round_execution(
        &mut self,
        report: &PlanRunReport,
        stats: SafeLaneToolOutputStats,
    ) {
        self.metrics.total_attempts_used = self
            .metrics
            .total_attempts_used
            .saturating_add(report.attempts_used as u64);
        self.metrics.record_tool_output_stats(stats);
    }

    pub(super) fn note_round_succeeded(&mut self) {
        self.metrics.rounds_succeeded = self.metrics.rounds_succeeded.saturating_add(1);
    }

    pub(super) fn note_round_failed(&mut self) {
        self.metrics.rounds_failed = self.metrics.rounds_failed.saturating_add(1);
    }

    pub(super) fn note_verify_failure(&mut self) {
        self.metrics.verify_failures = self.metrics.verify_failures.saturating_add(1);
    }

    pub(super) fn note_replan(
        &mut self,
        next_plan_start_tool_index: usize,
        next_seed_tool_outputs: Vec<String>,
    ) {
        self.plan_start_tool_index = next_plan_start_tool_index;
        self.seed_tool_outputs = next_seed_tool_outputs;
        self.metrics.replans_triggered = self.metrics.replans_triggered.saturating_add(1);
    }

    pub(super) fn advance_round(&mut self) {
        self.replan_budget = self.replan_budget.after_replan();
        self.tool_node_attempt_budget = self.tool_node_attempt_budget.after_retry();
    }

    pub(super) fn round(&self) -> u8 {
        self.replan_budget.current_round()
    }

    pub(super) fn max_rounds(&self) -> u8 {
        self.replan_budget.max_replans()
    }

    pub(super) fn tool_node_max_attempts(&self) -> u8 {
        self.tool_node_attempt_budget.current_limit()
    }

    pub(super) fn max_node_attempts(&self) -> u8 {
        self.tool_node_attempt_budget.max_limit()
    }
}

#[derive(Debug, Clone)]
pub(super) struct SafeLaneRoundExecution {
    pub(super) report: PlanRunReport,
    pub(super) tool_outputs: Vec<String>,
    pub(super) tool_output_stats: SafeLaneToolOutputStats,
}

pub(super) struct SafeLanePlanNodeExecutor<'a> {
    pub(super) tool_intents: &'a [ToolIntent],
    pub(super) session_context: &'a SessionContext,
    pub(super) app_dispatcher: &'a dyn AppToolDispatcher,
    pub(super) binding: ConversationRuntimeBinding<'a>,
    pub(super) ingress: Option<&'a ConversationIngressContext>,
    pub(super) verify_output_non_empty: bool,
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
        verify_output_non_empty: bool,
        seed_tool_outputs: Vec<String>,
        tool_result_payload_summary_limit_chars: usize,
    ) -> Self {
        Self {
            tool_intents,
            session_context,
            app_dispatcher,
            binding,
            ingress,
            verify_output_non_empty,
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
                if !self.verify_output_non_empty {
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
        config.conversation.safe_lane_verify_output_non_empty,
        state.seed_tool_outputs.clone(),
        config
            .conversation
            .tool_result_payload_summary_limit_chars(),
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

    if config.conversation.safe_lane_verify_output_non_empty {
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
            max_wall_time_ms: config.conversation.safe_lane_plan_max_wall_time_ms.max(1),
        },
    }
}

pub(super) fn verify_safe_lane_final_output(
    config: &LoongConfig,
    output: &str,
    tool_intents: &[ToolIntent],
    adaptive_policy: SafeLaneAdaptiveVerifyPolicyState,
) -> PlanVerificationReport {
    let policy = PlanVerificationPolicy {
        require_non_empty: config.conversation.safe_lane_verify_output_non_empty,
        min_output_chars: config.conversation.safe_lane_verify_min_output_chars,
        require_status_prefix: config.conversation.safe_lane_verify_require_status_prefix,
        deny_markers: config
            .conversation
            .safe_lane_verify_deny_markers
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
