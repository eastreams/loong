use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnRoundBudgetDecision {
    ContinueWithFollowup,
    FinalizeWithCompletionPass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnRoundBudget {
    round_index: usize,
    max_rounds: usize,
}

impl TurnRoundBudget {
    pub fn for_round_index(round_index: usize, max_rounds: usize) -> Self {
        Self {
            round_index,
            max_rounds: max_rounds.max(1),
        }
    }

    pub fn followup_decision(self) -> TurnRoundBudgetDecision {
        if self.round_index.saturating_add(1) < self.max_rounds {
            TurnRoundBudgetDecision::ContinueWithFollowup
        } else {
            TurnRoundBudgetDecision::FinalizeWithCompletionPass
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafeLaneReplanBudget {
    current_round: u8,
    max_replans: u8,
}

impl SafeLaneReplanBudget {
    pub fn new(max_replans: u8) -> Self {
        Self {
            current_round: 0,
            max_replans,
        }
    }

    pub fn current_round(self) -> u8 {
        self.current_round
    }

    pub fn max_replans(self) -> u8 {
        self.max_replans
    }

    pub fn continuation_decision(self) -> SafeLaneContinuationBudgetDecision {
        if self.current_round < self.max_replans {
            SafeLaneContinuationBudgetDecision::Continue
        } else {
            SafeLaneContinuationBudgetDecision::Terminal {
                reason: SafeLaneFailureRouteReason::RoundBudgetExhausted,
            }
        }
    }

    pub fn after_replan(self) -> Self {
        Self {
            current_round: self.current_round.saturating_add(1),
            max_replans: self.max_replans,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EscalatingAttemptBudget {
    current_limit: u8,
    max_limit: u8,
}

impl EscalatingAttemptBudget {
    pub fn new(initial_limit: u8, max_limit: u8) -> Self {
        let current_limit = initial_limit.max(1);
        let max_limit = max_limit.max(current_limit).max(1);
        Self {
            current_limit: current_limit.min(max_limit),
            max_limit,
        }
    }

    pub fn current_limit(self) -> u8 {
        self.current_limit
    }

    pub fn max_limit(self) -> u8 {
        self.max_limit
    }

    pub fn after_retry(self) -> Self {
        Self {
            current_limit: self
                .current_limit
                .saturating_add(1)
                .min(self.max_limit)
                .max(1),
            max_limit: self.max_limit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafeLaneBackpressureBudget {
    max_total_attempts: u64,
    max_replans: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeLaneContinuationBudgetDecision {
    Continue,
    Terminal { reason: SafeLaneFailureRouteReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeLaneFailureRouteReason {
    RetryableFailure,
    RoundBudgetExhausted,
    ApprovalRequired,
    PolicyDenied,
    NonRetryableFailure,
    RetryableFlagFalse,
    ProviderFailure,
    BackpressureAttemptsExhausted,
    BackpressureReplansExhausted,
    SessionGovernorNoReplan,
}

impl SafeLaneFailureRouteReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RetryableFailure => "retryable_failure",
            Self::RoundBudgetExhausted => "round_budget_exhausted",
            Self::ApprovalRequired => "approval_required",
            Self::PolicyDenied => "policy_denied",
            Self::NonRetryableFailure => "non_retryable_failure",
            Self::RetryableFlagFalse => "retryable_flag_false",
            Self::ProviderFailure => "provider_failure",
            Self::BackpressureAttemptsExhausted => "backpressure_attempts_exhausted",
            Self::BackpressureReplansExhausted => "backpressure_replans_exhausted",
            Self::SessionGovernorNoReplan => "session_governor_no_replan",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "retryable_failure" => Some(Self::RetryableFailure),
            "round_budget_exhausted" => Some(Self::RoundBudgetExhausted),
            "approval_required" => Some(Self::ApprovalRequired),
            "policy_denied" => Some(Self::PolicyDenied),
            "non_retryable_failure" => Some(Self::NonRetryableFailure),
            "retryable_flag_false" => Some(Self::RetryableFlagFalse),
            "provider_failure" => Some(Self::ProviderFailure),
            "backpressure_attempts_exhausted" => Some(Self::BackpressureAttemptsExhausted),
            "backpressure_replans_exhausted" => Some(Self::BackpressureReplansExhausted),
            "session_governor_no_replan" => Some(Self::SessionGovernorNoReplan),
            _ => None,
        }
    }

    pub fn is_backpressure(self) -> bool {
        matches!(
            self,
            Self::BackpressureAttemptsExhausted | Self::BackpressureReplansExhausted
        )
    }
}

impl FromStr for SafeLaneFailureRouteReason {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value).ok_or(())
    }
}

impl SafeLaneBackpressureBudget {
    pub fn new(max_total_attempts: u64, max_replans: u32) -> Self {
        Self {
            max_total_attempts: max_total_attempts.max(1),
            max_replans: max_replans.max(1),
        }
    }

    pub fn continuation_decision(
        self,
        total_attempts_used: u64,
        replans_triggered: u32,
    ) -> SafeLaneContinuationBudgetDecision {
        if total_attempts_used >= self.max_total_attempts {
            return SafeLaneContinuationBudgetDecision::Terminal {
                reason: SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
            };
        }
        if replans_triggered >= self.max_replans {
            return SafeLaneContinuationBudgetDecision::Terminal {
                reason: SafeLaneFailureRouteReason::BackpressureReplansExhausted,
            };
        }
        SafeLaneContinuationBudgetDecision::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SafeLaneBackpressureBudget, SafeLaneContinuationBudgetDecision, SafeLaneFailureRouteReason,
        SafeLaneReplanBudget, TurnRoundBudget, TurnRoundBudgetDecision,
    };

    #[test]
    fn turn_round_budget_followup_decision_reports_remaining_capacity() {
        let budget = TurnRoundBudget::for_round_index(0, 2);
        assert_eq!(
            budget.followup_decision(),
            TurnRoundBudgetDecision::ContinueWithFollowup
        );
    }

    #[test]
    fn turn_round_budget_followup_decision_reports_round_limit_reached() {
        let budget = TurnRoundBudget::for_round_index(1, 2);
        assert_eq!(
            budget.followup_decision(),
            TurnRoundBudgetDecision::FinalizeWithCompletionPass
        );
    }

    #[test]
    fn safe_lane_replan_budget_continuation_decision_reports_budget_exhaustion() {
        let budget = SafeLaneReplanBudget::new(1).after_replan();
        assert_eq!(
            budget.continuation_decision(),
            SafeLaneContinuationBudgetDecision::Terminal {
                reason: SafeLaneFailureRouteReason::RoundBudgetExhausted,
            }
        );
    }

    #[test]
    fn safe_lane_backpressure_budget_continuation_decision_reports_attempt_exhaustion() {
        let budget = SafeLaneBackpressureBudget::new(2, 10);
        assert_eq!(
            budget.continuation_decision(2, 0),
            SafeLaneContinuationBudgetDecision::Terminal {
                reason: SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
            }
        );
    }

    #[test]
    fn safe_lane_backpressure_budget_continuation_decision_reports_continue_below_limits() {
        let budget = SafeLaneBackpressureBudget::new(3, 2);
        assert_eq!(
            budget.continuation_decision(1, 0),
            SafeLaneContinuationBudgetDecision::Continue
        );
    }

    #[test]
    fn safe_lane_failure_route_reason_round_trips_approval_required() {
        let reason = SafeLaneFailureRouteReason::parse("approval_required")
            .expect("approval_required should parse");
        assert_eq!(reason, SafeLaneFailureRouteReason::ApprovalRequired);
        assert_eq!(reason.as_str(), "approval_required");
    }
}
