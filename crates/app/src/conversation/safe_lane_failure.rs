use serde::{Deserialize, Serialize};

use super::plan_executor::{PlanNodeErrorKind, PlanRunFailure};
use super::turn_budget::SafeLaneFailureRouteReason;
use super::turn_engine::{TurnFailure, TurnFailureKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeLaneFailureRouteDecision {
    Replan,
    Terminal,
}

impl SafeLaneFailureRouteDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Replan => "replan",
            Self::Terminal => "terminal",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "replan" => Some(Self::Replan),
            "terminal" => Some(Self::Terminal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeLaneFailureRouteSource {
    BaseRouting,
    BackpressureGuard,
    SessionGovernor,
}

impl SafeLaneFailureRouteSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BaseRouting => "base_routing",
            Self::BackpressureGuard => "backpressure_guard",
            Self::SessionGovernor => "session_governor",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "base_routing" => Some(Self::BaseRouting),
            "backpressure_guard" => Some(Self::BackpressureGuard),
            "session_governor" => Some(Self::SessionGovernor),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeLaneTerminalRouteSnapshot {
    pub decision: SafeLaneFailureRouteDecision,
    pub reason: SafeLaneFailureRouteReason,
    pub source: SafeLaneFailureRouteSource,
}

impl SafeLaneTerminalRouteSnapshot {
    pub fn is_terminal(self) -> bool {
        self.decision == SafeLaneFailureRouteDecision::Terminal
    }

    pub fn is_backpressure_override_terminal(self) -> bool {
        self.is_terminal()
            && self.source == SafeLaneFailureRouteSource::BackpressureGuard
            && self.reason.is_backpressure()
    }

    pub fn is_session_governor_override_terminal(self) -> bool {
        self.is_terminal()
            && self.source == SafeLaneFailureRouteSource::SessionGovernor
            && self.reason == SafeLaneFailureRouteReason::SessionGovernorNoReplan
    }

    pub fn decision_label(self) -> &'static str {
        self.decision.as_str()
    }

    pub fn reason_label(self) -> &'static str {
        self.reason.as_str()
    }

    pub fn source_label(self) -> &'static str {
        self.source.as_str()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeLaneFailureCode {
    PlanValidationFailed,
    PlanTopologyResolutionFailed,
    PlanBudgetExceeded,
    PlanWallTimeExceeded,
    PlanNodePolicyDenied,
    PlanNodeRetryableError,
    PlanNodeNonRetryableError,
    VerifyFailed,
    VerifyFailedBackpressureGuard,
    VerifyFailedSessionGovernor,
    VerifyFailedBudgetExhausted,
    PlanBackpressureGuard,
    PlanSessionGovernorNoReplan,
}

impl SafeLaneFailureCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlanValidationFailed => "safe_lane_plan_validation_failed",
            Self::PlanTopologyResolutionFailed => "safe_lane_plan_topology_resolution_failed",
            Self::PlanBudgetExceeded => "safe_lane_plan_budget_exceeded",
            Self::PlanWallTimeExceeded => "safe_lane_plan_wall_time_exceeded",
            Self::PlanNodePolicyDenied => "safe_lane_plan_node_policy_denied",
            Self::PlanNodeRetryableError => "safe_lane_plan_node_retryable_error",
            Self::PlanNodeNonRetryableError => "safe_lane_plan_node_non_retryable_error",
            Self::VerifyFailed => "safe_lane_plan_verify_failed",
            Self::VerifyFailedBackpressureGuard => {
                "safe_lane_plan_verify_failed_backpressure_guard"
            }
            Self::VerifyFailedSessionGovernor => "safe_lane_plan_verify_failed_session_governor",
            Self::VerifyFailedBudgetExhausted => "safe_lane_plan_verify_failed_budget_exhausted",
            Self::PlanBackpressureGuard => "safe_lane_plan_backpressure_guard",
            Self::PlanSessionGovernorNoReplan => "safe_lane_plan_session_governor_no_replan",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "safe_lane_plan_validation_failed" => Some(Self::PlanValidationFailed),
            "safe_lane_plan_topology_resolution_failed" => Some(Self::PlanTopologyResolutionFailed),
            "safe_lane_plan_budget_exceeded" => Some(Self::PlanBudgetExceeded),
            "safe_lane_plan_wall_time_exceeded" => Some(Self::PlanWallTimeExceeded),
            "safe_lane_plan_node_policy_denied" => Some(Self::PlanNodePolicyDenied),
            "safe_lane_plan_node_retryable_error" => Some(Self::PlanNodeRetryableError),
            "safe_lane_plan_node_non_retryable_error" => Some(Self::PlanNodeNonRetryableError),
            "safe_lane_plan_verify_failed" => Some(Self::VerifyFailed),
            "safe_lane_plan_verify_failed_backpressure_guard" => {
                Some(Self::VerifyFailedBackpressureGuard)
            }
            "safe_lane_plan_verify_failed_session_governor" => {
                Some(Self::VerifyFailedSessionGovernor)
            }
            "safe_lane_plan_verify_failed_budget_exhausted" => {
                Some(Self::VerifyFailedBudgetExhausted)
            }
            "safe_lane_plan_backpressure_guard" => Some(Self::PlanBackpressureGuard),
            "safe_lane_plan_session_governor_no_replan" => Some(Self::PlanSessionGovernorNoReplan),
            _ => None,
        }
    }

    pub fn is_backpressure(self) -> bool {
        matches!(
            self,
            Self::VerifyFailedBackpressureGuard | Self::PlanBackpressureGuard
        )
    }

    pub fn is_terminal_instability(self) -> bool {
        matches!(
            self,
            Self::VerifyFailed
                | Self::VerifyFailedBackpressureGuard
                | Self::VerifyFailedSessionGovernor
                | Self::VerifyFailedBudgetExhausted
                | Self::PlanBackpressureGuard
                | Self::PlanSessionGovernorNoReplan
        )
    }

    pub fn into_turn_failure(
        self,
        kind: TurnFailureKind,
        reason: impl Into<String>,
    ) -> TurnFailure {
        match kind {
            TurnFailureKind::PolicyDenied => TurnFailure::policy_denied(self.as_str(), reason),
            TurnFailureKind::Retryable => TurnFailure::retryable(self.as_str(), reason),
            TurnFailureKind::NonRetryable => TurnFailure::non_retryable(self.as_str(), reason),
            TurnFailureKind::Provider => TurnFailure::provider(self.as_str(), reason),
        }
    }
}

pub fn classify_safe_lane_plan_failure(
    failure: &PlanRunFailure,
) -> (SafeLaneFailureCode, TurnFailureKind) {
    match failure {
        PlanRunFailure::ValidationFailed(_) => (
            SafeLaneFailureCode::PlanValidationFailed,
            TurnFailureKind::NonRetryable,
        ),
        PlanRunFailure::TopologyResolutionFailed => (
            SafeLaneFailureCode::PlanTopologyResolutionFailed,
            TurnFailureKind::NonRetryable,
        ),
        PlanRunFailure::BudgetExceeded { .. } => (
            SafeLaneFailureCode::PlanBudgetExceeded,
            TurnFailureKind::NonRetryable,
        ),
        PlanRunFailure::WallTimeExceeded { .. } => (
            SafeLaneFailureCode::PlanWallTimeExceeded,
            TurnFailureKind::NonRetryable,
        ),
        PlanRunFailure::NodeFailed {
            last_error_kind, ..
        } => match last_error_kind {
            PlanNodeErrorKind::ApprovalRequired => (
                SafeLaneFailureCode::PlanNodePolicyDenied,
                TurnFailureKind::PolicyDenied,
            ),
            PlanNodeErrorKind::PolicyDenied => (
                SafeLaneFailureCode::PlanNodePolicyDenied,
                TurnFailureKind::PolicyDenied,
            ),
            PlanNodeErrorKind::Retryable => (
                SafeLaneFailureCode::PlanNodeRetryableError,
                TurnFailureKind::Retryable,
            ),
            PlanNodeErrorKind::NonRetryable => (
                SafeLaneFailureCode::PlanNodeNonRetryableError,
                TurnFailureKind::NonRetryable,
            ),
        },
    }
}

pub fn is_safe_lane_backpressure_failure_code(value: &str) -> bool {
    SafeLaneFailureCode::parse(value)
        .map(SafeLaneFailureCode::is_backpressure)
        .unwrap_or(false)
}

pub fn is_safe_lane_terminal_instability_failure_code(value: Option<&str>) -> bool {
    value
        .and_then(SafeLaneFailureCode::parse)
        .map(SafeLaneFailureCode::is_terminal_instability)
        .unwrap_or(false)
}

pub fn is_safe_lane_backpressure_route_reason(value: &str) -> bool {
    SafeLaneFailureRouteReason::parse(value)
        .map(SafeLaneFailureRouteReason::is_backpressure)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{
        SafeLaneFailureCode, SafeLaneFailureRouteDecision, SafeLaneFailureRouteSource,
        SafeLaneTerminalRouteSnapshot, classify_safe_lane_plan_failure,
        is_safe_lane_backpressure_failure_code, is_safe_lane_backpressure_route_reason,
        is_safe_lane_terminal_instability_failure_code,
    };
    use crate::conversation::plan_executor::{PlanNodeErrorKind, PlanRunFailure};
    use crate::conversation::turn_budget::SafeLaneFailureRouteReason;
    use crate::conversation::turn_engine::TurnFailureKind;

    #[test]
    fn parse_safe_lane_failure_code_accepts_known_codes_only() {
        assert_eq!(
            SafeLaneFailureCode::parse("safe_lane_plan_verify_failed_session_governor"),
            Some(SafeLaneFailureCode::VerifyFailedSessionGovernor)
        );
        assert_eq!(
            SafeLaneFailureCode::parse("safe_lane_plan_backpressure_guard"),
            Some(SafeLaneFailureCode::PlanBackpressureGuard)
        );
        assert_eq!(
            SafeLaneFailureCode::parse("unknown_session_governor_hint"),
            None
        );
    }

    #[test]
    fn safe_lane_failure_code_classifiers_match_known_terminal_semantics() {
        assert!(is_safe_lane_terminal_instability_failure_code(Some(
            "safe_lane_plan_verify_failed"
        )));
        assert!(is_safe_lane_terminal_instability_failure_code(Some(
            "safe_lane_plan_session_governor_no_replan"
        )));
        assert!(!is_safe_lane_terminal_instability_failure_code(Some(
            "safe_lane_plan_node_policy_denied"
        )));
        assert!(!is_safe_lane_terminal_instability_failure_code(Some(
            "unknown_verify_failed_hint"
        )));
    }

    #[test]
    fn safe_lane_backpressure_classifiers_ignore_unknown_lookalikes() {
        assert!(is_safe_lane_backpressure_failure_code(
            "safe_lane_plan_backpressure_guard"
        ));
        assert!(!is_safe_lane_backpressure_failure_code(
            "unknown_backpressure_hint"
        ));
        assert!(is_safe_lane_backpressure_route_reason(
            "backpressure_attempts_exhausted"
        ));
        assert!(!is_safe_lane_backpressure_route_reason(
            "backpressure_noise"
        ));
    }

    #[test]
    fn safe_lane_route_vocabulary_parses_known_labels_only() {
        assert_eq!(
            SafeLaneFailureRouteDecision::parse("terminal"),
            Some(SafeLaneFailureRouteDecision::Terminal)
        );
        assert_eq!(
            SafeLaneFailureRouteSource::parse("session_governor"),
            Some(SafeLaneFailureRouteSource::SessionGovernor)
        );
        assert_eq!(SafeLaneFailureRouteDecision::parse("terminalish"), None);
        assert_eq!(
            SafeLaneFailureRouteSource::parse("route_source_noise"),
            None
        );
    }

    #[test]
    fn safe_lane_terminal_route_snapshot_reports_labels_from_typed_fields() {
        let snapshot = SafeLaneTerminalRouteSnapshot {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
            source: SafeLaneFailureRouteSource::SessionGovernor,
        };

        assert_eq!(snapshot.decision_label(), "terminal");
        assert_eq!(snapshot.reason_label(), "session_governor_no_replan");
        assert_eq!(snapshot.source_label(), "session_governor");
    }

    #[test]
    fn safe_lane_terminal_route_snapshot_terminality_depends_on_decision() {
        let terminal = SafeLaneTerminalRouteSnapshot {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
            source: SafeLaneFailureRouteSource::SessionGovernor,
        };
        let replan = SafeLaneTerminalRouteSnapshot {
            decision: SafeLaneFailureRouteDecision::Replan,
            reason: SafeLaneFailureRouteReason::RetryableFailure,
            source: SafeLaneFailureRouteSource::BaseRouting,
        };

        assert!(terminal.is_terminal());
        assert!(!replan.is_terminal());
    }

    #[test]
    fn safe_lane_terminal_route_snapshot_override_classifiers_require_consistent_pairs() {
        let backpressure_terminal = SafeLaneTerminalRouteSnapshot {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::BackpressureAttemptsExhausted,
            source: SafeLaneFailureRouteSource::BackpressureGuard,
        };
        let malformed_backpressure = SafeLaneTerminalRouteSnapshot {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::RetryableFailure,
            source: SafeLaneFailureRouteSource::BackpressureGuard,
        };
        let governor_terminal = SafeLaneTerminalRouteSnapshot {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::SessionGovernorNoReplan,
            source: SafeLaneFailureRouteSource::SessionGovernor,
        };
        let malformed_governor = SafeLaneTerminalRouteSnapshot {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason: SafeLaneFailureRouteReason::NonRetryableFailure,
            source: SafeLaneFailureRouteSource::SessionGovernor,
        };

        assert!(backpressure_terminal.is_backpressure_override_terminal());
        assert!(!malformed_backpressure.is_backpressure_override_terminal());
        assert!(governor_terminal.is_session_governor_override_terminal());
        assert!(!malformed_governor.is_session_governor_override_terminal());
    }

    #[test]
    fn classify_safe_lane_plan_failure_maps_node_and_static_failures() {
        let approval_node = PlanRunFailure::NodeFailed {
            node_id: "tool-approval".to_owned(),
            attempts_used: 1,
            last_error_kind: PlanNodeErrorKind::ApprovalRequired,
            last_error: "approval required".to_owned(),
        };
        assert_eq!(
            classify_safe_lane_plan_failure(&approval_node),
            (
                SafeLaneFailureCode::PlanNodePolicyDenied,
                TurnFailureKind::PolicyDenied
            )
        );

        let retryable_node = PlanRunFailure::NodeFailed {
            node_id: "tool-1".to_owned(),
            attempts_used: 1,
            last_error_kind: PlanNodeErrorKind::Retryable,
            last_error: "transient".to_owned(),
        };
        assert_eq!(
            classify_safe_lane_plan_failure(&retryable_node),
            (
                SafeLaneFailureCode::PlanNodeRetryableError,
                TurnFailureKind::Retryable
            )
        );

        let validation = PlanRunFailure::ValidationFailed("invalid".to_owned());
        assert_eq!(
            classify_safe_lane_plan_failure(&validation),
            (
                SafeLaneFailureCode::PlanValidationFailed,
                TurnFailureKind::NonRetryable
            )
        );
    }
}
