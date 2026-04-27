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
    pub(super) fn new(_config: &LoongConfig, governor: SafeLaneSessionGovernorDecision) -> Self {
        let force_no_replan = governor.force_no_replan;
        let mut tool_node_max_attempts = crate::conversation::SAFE_LANE_NODE_MAX_ATTEMPTS;
        if let Some(forced_node_max_attempts) = governor.forced_node_max_attempts {
            tool_node_max_attempts = tool_node_max_attempts.min(forced_node_max_attempts.max(1));
        }
        let mut max_node_attempts =
            crate::conversation::SAFE_LANE_REPLAN_MAX_NODE_ATTEMPTS.max(tool_node_max_attempts);
        if let Some(forced_node_max_attempts) = governor.forced_node_max_attempts {
            max_node_attempts = max_node_attempts.min(forced_node_max_attempts.max(1));
        }

        Self {
            governor,
            replan_budget: SafeLaneReplanBudget::new(if force_no_replan {
                0
            } else {
                crate::conversation::SAFE_LANE_REPLAN_MAX_ROUNDS
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
