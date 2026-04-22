use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct SafeLaneFailureRoute {
    pub(super) decision: SafeLaneFailureRouteDecision,
    pub(super) reason: SafeLaneFailureRouteReason,
    pub(super) source: SafeLaneFailureRouteSource,
}

impl SafeLaneFailureRoute {
    pub(super) fn from_failure(failure: &TurnFailure, replan_budget: SafeLaneReplanBudget) -> Self {
        if let SafeLaneContinuationBudgetDecision::Terminal { reason } =
            replan_budget.continuation_decision()
        {
            return Self::terminal(reason);
        }

        match failure.code.as_str() {
            "tool_not_found" if failure.supports_discovery_recovery => {
                return Self::replan(SafeLaneFailureRouteReason::RetryableFailure);
            }
            "kernel_policy_denied"
            | "tool_not_found"
            | "max_tool_steps_exceeded"
            | "no_kernel_context" => {
                return Self::terminal(SafeLaneFailureRouteReason::PolicyDenied);
            }
            "tool_execution_failed" => {
                if failure.retryable {
                    return Self::replan(SafeLaneFailureRouteReason::RetryableFailure);
                }
                return Self::terminal(SafeLaneFailureRouteReason::RetryableFlagFalse);
            }
            "kernel_execution_failed" => {
                return Self::terminal(SafeLaneFailureRouteReason::NonRetryableFailure);
            }
            _ => {}
        }

        if let Some(code) = SafeLaneFailureCode::parse(failure.code.as_str()) {
            match code {
                SafeLaneFailureCode::PlanNodePolicyDenied => {
                    return Self::terminal(SafeLaneFailureRouteReason::PolicyDenied);
                }
                SafeLaneFailureCode::VerifyFailed => {
                    if failure.retryable {
                        return Self::replan(SafeLaneFailureRouteReason::RetryableFailure);
                    }
                    return Self::terminal(SafeLaneFailureRouteReason::NonRetryableFailure);
                }
                SafeLaneFailureCode::PlanNodeRetryableError => {
                    if failure.retryable {
                        return Self::replan(SafeLaneFailureRouteReason::RetryableFailure);
                    }
                    return Self::terminal(SafeLaneFailureRouteReason::RetryableFlagFalse);
                }
                SafeLaneFailureCode::VerifyFailedBudgetExhausted => {
                    return Self::terminal(SafeLaneFailureRouteReason::RoundBudgetExhausted);
                }
                SafeLaneFailureCode::PlanValidationFailed
                | SafeLaneFailureCode::PlanTopologyResolutionFailed
                | SafeLaneFailureCode::PlanBudgetExceeded
                | SafeLaneFailureCode::PlanWallTimeExceeded
                | SafeLaneFailureCode::PlanNodeNonRetryableError
                | SafeLaneFailureCode::VerifyFailedBackpressureGuard
                | SafeLaneFailureCode::VerifyFailedSessionGovernor
                | SafeLaneFailureCode::PlanBackpressureGuard
                | SafeLaneFailureCode::PlanSessionGovernorNoReplan => {
                    return Self::terminal(SafeLaneFailureRouteReason::NonRetryableFailure);
                }
            }
        }

        match failure.kind {
            TurnFailureKind::Retryable if failure.retryable => {
                Self::replan(SafeLaneFailureRouteReason::RetryableFailure)
            }
            TurnFailureKind::Retryable => {
                Self::terminal(SafeLaneFailureRouteReason::RetryableFlagFalse)
            }
            TurnFailureKind::PolicyDenied => {
                Self::terminal(SafeLaneFailureRouteReason::PolicyDenied)
            }
            TurnFailureKind::NonRetryable => {
                Self::terminal(SafeLaneFailureRouteReason::NonRetryableFailure)
            }
            TurnFailureKind::Provider => {
                Self::terminal(SafeLaneFailureRouteReason::ProviderFailure)
            }
        }
    }

    pub(super) fn replan(reason: SafeLaneFailureRouteReason) -> Self {
        Self {
            decision: SafeLaneFailureRouteDecision::Replan,
            reason,
            source: SafeLaneFailureRouteSource::BaseRouting,
        }
    }

    fn terminal(reason: SafeLaneFailureRouteReason) -> Self {
        Self {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason,
            source: SafeLaneFailureRouteSource::BaseRouting,
        }
    }

    pub(super) fn terminal_with_source(
        reason: SafeLaneFailureRouteReason,
        source: SafeLaneFailureRouteSource,
    ) -> Self {
        Self {
            decision: SafeLaneFailureRouteDecision::Terminal,
            reason,
            source,
        }
    }

    pub(super) fn is_base_round_budget_terminal(self) -> bool {
        self.decision == SafeLaneFailureRouteDecision::Terminal
            && self.source == SafeLaneFailureRouteSource::BaseRouting
            && self.reason == SafeLaneFailureRouteReason::RoundBudgetExhausted
    }

    pub(super) fn should_replan(self) -> bool {
        self.decision == SafeLaneFailureRouteDecision::Replan
    }

    pub(super) fn decision_label(self) -> &'static str {
        self.decision.as_str()
    }

    pub(super) fn source_label(self) -> &'static str {
        self.source.as_str()
    }

    pub(super) fn verify_terminal_summary_label(self) -> &'static str {
        match (self.source, self.reason) {
            (SafeLaneFailureRouteSource::BackpressureGuard, _) => {
                "verify_failed_backpressure_guard"
            }
            (SafeLaneFailureRouteSource::SessionGovernor, _) => "verify_failed_session_governor",
            (
                SafeLaneFailureRouteSource::BaseRouting,
                SafeLaneFailureRouteReason::RoundBudgetExhausted,
            ) => "verify_failed_budget_exhausted",
            (SafeLaneFailureRouteSource::BaseRouting, _) => "verify_failed_non_retryable",
        }
    }

    pub(super) fn terminal_verify_failure_code(
        self,
        retryable_signal: bool,
    ) -> SafeLaneFailureCode {
        match (self.source, self.reason, retryable_signal) {
            (SafeLaneFailureRouteSource::BackpressureGuard, _, _) => {
                SafeLaneFailureCode::VerifyFailedBackpressureGuard
            }
            (SafeLaneFailureRouteSource::SessionGovernor, _, _) => {
                SafeLaneFailureCode::VerifyFailedSessionGovernor
            }
            (
                SafeLaneFailureRouteSource::BaseRouting,
                SafeLaneFailureRouteReason::RoundBudgetExhausted,
                true,
            ) => SafeLaneFailureCode::VerifyFailedBudgetExhausted,
            (SafeLaneFailureRouteSource::BaseRouting, _, _) => SafeLaneFailureCode::VerifyFailed,
        }
    }

    pub(super) fn terminal_plan_failure_code(self) -> Option<SafeLaneFailureCode> {
        match self.source {
            SafeLaneFailureRouteSource::BackpressureGuard => {
                Some(SafeLaneFailureCode::PlanBackpressureGuard)
            }
            SafeLaneFailureRouteSource::SessionGovernor => {
                Some(SafeLaneFailureCode::PlanSessionGovernorNoReplan)
            }
            SafeLaneFailureRouteSource::BaseRouting => None,
        }
    }

    pub(super) fn with_backpressure_guard(
        self,
        backpressure_budget: Option<SafeLaneBackpressureBudget>,
        metrics: SafeLaneExecutionMetrics,
    ) -> Self {
        if !self.should_replan() {
            return self;
        }

        let Some(reason) = backpressure_budget.and_then(|budget| {
            match budget
                .continuation_decision(metrics.total_attempts_used, metrics.replans_triggered)
            {
                SafeLaneContinuationBudgetDecision::Continue => None,
                SafeLaneContinuationBudgetDecision::Terminal { reason } => Some(reason),
            }
        }) else {
            return self;
        };

        Self::terminal_with_source(reason, SafeLaneFailureRouteSource::BackpressureGuard)
    }

    pub(super) fn with_session_governor_override(
        self,
        governor: &SafeLaneSessionGovernorDecision,
    ) -> Self {
        if governor.force_no_replan && self.is_base_round_budget_terminal() {
            return Self::terminal_with_source(
                SafeLaneFailureRouteReason::SessionGovernorNoReplan,
                SafeLaneFailureRouteSource::SessionGovernor,
            );
        }
        self
    }
}

#[derive(Debug, Clone)]
pub(super) enum SafeLaneRoundDecision {
    Finalize {
        result: TurnResult,
    },
    Replan {
        reason: String,
        next_plan_start_tool_index: usize,
        next_seed_tool_outputs: Vec<String>,
    },
}

pub(super) fn decide_safe_lane_failure_route(
    config: &LoongConfig,
    failure: &TurnFailure,
    replan_budget: SafeLaneReplanBudget,
    metrics: SafeLaneExecutionMetrics,
    governor: &SafeLaneSessionGovernorDecision,
) -> SafeLaneFailureRoute {
    SafeLaneFailureRoute::from_failure(failure, replan_budget)
        .with_backpressure_guard(safe_lane_backpressure_budget(config), metrics)
        .with_session_governor_override(governor)
}

pub(super) fn decide_safe_lane_verify_failure_action(
    verify_error: &str,
    retryable_signal: bool,
    route: SafeLaneFailureRoute,
) -> SafeLaneRoundDecision {
    if !route.should_replan() {
        return SafeLaneRoundDecision::Finalize {
            result: TurnResult::ToolError(terminal_turn_failure_from_verify_failure(
                verify_error,
                retryable_signal,
                route,
            )),
        };
    }

    SafeLaneRoundDecision::Replan {
        reason: "verify_failed".to_owned(),
        next_plan_start_tool_index: 0,
        next_seed_tool_outputs: Vec::new(),
    }
}

pub(super) fn should_replan_for_verification_failure(report: &PlanVerificationReport) -> bool {
    !report.failure_codes.iter().any(|code| {
        matches!(
            code,
            PlanVerificationFailureCode::DenyMarkerDetected
                | PlanVerificationFailureCode::MissingStatusPrefix
                | PlanVerificationFailureCode::MissingSemanticAnchors
        )
    })
}

pub(super) fn format_verification_failure_code(code: &PlanVerificationFailureCode) -> &'static str {
    match code {
        PlanVerificationFailureCode::EmptyOutput => "empty_output",
        PlanVerificationFailureCode::OutputTooShort => "output_too_short",
        PlanVerificationFailureCode::DenyMarkerDetected => "deny_marker_detected",
        PlanVerificationFailureCode::InsufficientResultLines => "insufficient_result_lines",
        PlanVerificationFailureCode::MissingStatusPrefix => "missing_status_prefix",
        PlanVerificationFailureCode::FailureStatusDetected => "failure_status_detected",
        PlanVerificationFailureCode::MissingSemanticAnchors => "missing_semantic_anchors",
    }
}

pub(super) fn collect_semantic_anchors(tool_intents: &[ToolIntent]) -> BTreeSet<String> {
    let mut anchors = BTreeSet::new();
    for intent in tool_intents {
        collect_value_anchors(None, &intent.args_json, &mut anchors);
    }
    anchors
}

fn collect_value_anchors(parent_key: Option<&str>, value: &Value, anchors: &mut BTreeSet<String>) {
    #[allow(clippy::wildcard_enum_match_arm)]
    match value {
        Value::String(text) => {
            if parent_key.map(is_anchor_key_allowed).unwrap_or(false) {
                push_anchor_candidate(text.as_str(), anchors);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_value_anchors(parent_key, item, anchors);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                if is_sensitive_key(key.as_str()) {
                    continue;
                }
                collect_value_anchors(Some(key.as_str()), item, anchors);
            }
        }
        _ => {}
    }
}

fn is_anchor_key_allowed(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "path"
            | "file"
            | "filename"
            | "url"
            | "endpoint"
            | "target"
            | "query"
            | "operation"
            | "command"
            | "cwd"
            | "dir"
            | "directory"
    )
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase();
    [
        "token",
        "secret",
        "password",
        "credential",
        "api_key",
        "apikey",
        "auth",
        "authorization",
        "cookie",
        "session",
        "bearer",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn push_anchor_candidate(text: &str, anchors: &mut BTreeSet<String>) {
    let normalized = text.trim().to_ascii_lowercase();
    if normalized.len() < 3 || normalized.len() > 96 {
        return;
    }
    if normalized.contains(' ') {
        return;
    }
    anchors.insert(normalized.clone());
    if let Some(last_segment) = normalized.rsplit('/').next()
        && last_segment.len() >= 3
    {
        anchors.insert(last_segment.to_owned());
    }
}

pub(super) fn derive_replan_cursor(
    failure: &PlanRunFailure,
    round_tool_outputs: &[String],
    tool_count: usize,
) -> (usize, Vec<String>) {
    #[allow(clippy::wildcard_enum_match_arm)]
    match failure {
        PlanRunFailure::NodeFailed { node_id, .. } => {
            if let Ok(index) = parse_tool_node_index(node_id.as_str())
                && index < tool_count
            {
                return (index, round_tool_outputs.to_vec());
            }
            (0, Vec::new())
        }
        _ => (0, Vec::new()),
    }
}

pub(super) fn summarize_plan_failure(failure: &PlanRunFailure) -> String {
    match failure {
        PlanRunFailure::ValidationFailed(error) => {
            format!("validation_failed:{error}")
        }
        PlanRunFailure::TopologyResolutionFailed => "topology_resolution_failed".to_owned(),
        PlanRunFailure::BudgetExceeded {
            attempts_used,
            limit,
        } => {
            format!("budget_exceeded attempts_used={attempts_used} limit={limit}")
        }
        PlanRunFailure::WallTimeExceeded {
            elapsed_ms,
            limit_ms,
        } => {
            format!("wall_time_exceeded elapsed_ms={elapsed_ms} limit_ms={limit_ms}")
        }
        PlanRunFailure::NodeFailed {
            node_id,
            last_error_kind,
            last_error,
            ..
        } => {
            format!("node_failed node={node_id} error_kind={last_error_kind:?} reason={last_error}")
        }
    }
}

pub(super) fn format_turn_failure_kind(kind: TurnFailureKind) -> &'static str {
    match kind {
        TurnFailureKind::PolicyDenied => "policy_denied",
        TurnFailureKind::Retryable => "retryable",
        TurnFailureKind::NonRetryable => "non_retryable",
        TurnFailureKind::Provider => "provider",
    }
}

pub(super) fn turn_failure_from_plan_failure(failure: &PlanRunFailure) -> TurnFailure {
    let (code, kind) = classify_safe_lane_plan_failure(failure);
    match failure {
        PlanRunFailure::ValidationFailed(error) => {
            code.into_turn_failure(kind, format!("{}: {error}", code.as_str()))
        }
        PlanRunFailure::TopologyResolutionFailed => code.into_turn_failure(kind, code.as_str()),
        PlanRunFailure::BudgetExceeded {
            attempts_used,
            limit,
        } => code.into_turn_failure(
            kind,
            format!(
                "{} attempts_used={attempts_used} limit={limit}",
                code.as_str()
            ),
        ),
        PlanRunFailure::WallTimeExceeded {
            elapsed_ms,
            limit_ms,
        } => code.into_turn_failure(
            kind,
            format!(
                "{} elapsed_ms={elapsed_ms} limit_ms={limit_ms}",
                code.as_str()
            ),
        ),
        PlanRunFailure::NodeFailed { last_error, .. } => {
            code.into_turn_failure(kind, last_error.clone())
        }
    }
}

pub(super) fn turn_failure_from_verify_failure(verify_error: &str, retryable: bool) -> TurnFailure {
    SafeLaneFailureCode::VerifyFailed.into_turn_failure(
        if retryable {
            TurnFailureKind::Retryable
        } else {
            TurnFailureKind::NonRetryable
        },
        format!(
            "{}: {verify_error}",
            SafeLaneFailureCode::VerifyFailed.as_str()
        ),
    )
}

pub(super) fn terminal_turn_failure_from_verify_failure(
    verify_error: &str,
    retryable_signal: bool,
    route: SafeLaneFailureRoute,
) -> TurnFailure {
    route
        .terminal_verify_failure_code(retryable_signal)
        .into_turn_failure(
            TurnFailureKind::NonRetryable,
            format!(
                "{}: {verify_error}",
                SafeLaneFailureCode::VerifyFailed.as_str()
            ),
        )
}

pub(super) fn turn_result_from_plan_failure(failure: PlanRunFailure) -> TurnResult {
    let failure_meta = turn_failure_from_plan_failure(&failure);
    if matches!(failure_meta.kind, TurnFailureKind::PolicyDenied) {
        TurnResult::ToolDenied(failure_meta)
    } else if matches!(failure_meta.kind, TurnFailureKind::Provider) {
        TurnResult::ProviderError(failure_meta)
    } else {
        TurnResult::ToolError(failure_meta)
    }
}

pub(super) fn terminal_turn_result_from_plan_failure_with_route(
    failure: PlanRunFailure,
    route: SafeLaneFailureRoute,
) -> TurnResult {
    if let Some(code) = route.terminal_plan_failure_code() {
        let summary = summarize_plan_failure(&failure);
        return TurnResult::ToolError(code.into_turn_failure(
            TurnFailureKind::NonRetryable,
            format!("{}: {summary}", code.as_str()),
        ));
    }
    turn_result_from_plan_failure(failure)
}

pub(super) fn decide_safe_lane_plan_failure_action(
    failure: PlanRunFailure,
    route: SafeLaneFailureRoute,
    next_plan_start_tool_index: usize,
    next_seed_tool_outputs: Vec<String>,
) -> SafeLaneRoundDecision {
    if route.should_replan() {
        return SafeLaneRoundDecision::Replan {
            reason: summarize_plan_failure(&failure),
            next_plan_start_tool_index,
            next_seed_tool_outputs,
        };
    }

    SafeLaneRoundDecision::Finalize {
        result: terminal_turn_result_from_plan_failure_with_route(failure, route),
    }
}
