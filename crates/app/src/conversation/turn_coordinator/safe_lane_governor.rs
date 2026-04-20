use super::*;
use crate::memory::runtime_config::MemoryRuntimeConfig;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct SafeLaneAdaptiveVerifyPolicyState {
    pub(super) min_anchor_matches: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct SafeLaneToolOutputStats {
    pub(super) output_lines: usize,
    pub(super) result_lines: usize,
    pub(super) truncated_result_lines: usize,
}

impl SafeLaneToolOutputStats {
    pub(super) fn truncation_ratio_milli(self) -> usize {
        if self.result_lines == 0 {
            return 0;
        }
        self.truncated_result_lines
            .saturating_mul(1000)
            .saturating_div(self.result_lines)
    }

    pub(super) fn as_json(self) -> Value {
        json!({
            "output_lines": self.output_lines,
            "result_lines": self.result_lines,
            "truncated_result_lines": self.truncated_result_lines,
            "any_truncated": self.truncated_result_lines > 0,
            "truncation_ratio_milli": self.truncation_ratio_milli(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SafeLaneRuntimeHealthSignal {
    pub(super) severity: &'static str,
    pub(super) flags: Vec<String>,
}

impl SafeLaneRuntimeHealthSignal {
    pub(super) fn as_json(&self) -> Value {
        json!({
            "severity": self.severity,
            "flags": self.flags,
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum SafeLaneGovernorHistoryLoadStatus {
    #[default]
    Disabled,
    Loaded,
    Unavailable,
}

impl SafeLaneGovernorHistoryLoadStatus {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Loaded => "loaded",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct SafeLaneSessionGovernorDecision {
    pub(super) engaged: bool,
    pub(super) history_window_turns: usize,
    pub(super) history_load_status: SafeLaneGovernorHistoryLoadStatus,
    pub(super) history_load_error: Option<AssistantHistoryLoadErrorCode>,
    pub(super) failed_final_status_events: u32,
    pub(super) failed_final_status_threshold: u32,
    pub(super) failed_threshold_triggered: bool,
    pub(super) backpressure_failure_events: u32,
    pub(super) backpressure_failure_threshold: u32,
    pub(super) backpressure_threshold_triggered: bool,
    pub(super) trend_enabled: bool,
    pub(super) trend_samples: usize,
    pub(super) trend_min_samples: usize,
    pub(super) trend_failure_ewma: Option<f64>,
    pub(super) trend_failure_ewma_threshold: f64,
    pub(super) trend_backpressure_ewma: Option<f64>,
    pub(super) trend_backpressure_ewma_threshold: f64,
    pub(super) trend_threshold_triggered: bool,
    pub(super) recovery_success_streak: u32,
    pub(super) recovery_success_streak_threshold: u32,
    pub(super) recovery_failure_ewma_threshold: f64,
    pub(super) recovery_backpressure_ewma_threshold: f64,
    pub(super) recovery_threshold_triggered: bool,
    pub(super) force_no_replan: bool,
    pub(super) forced_node_max_attempts: Option<u8>,
}

impl SafeLaneSessionGovernorDecision {
    pub(super) fn as_json(&self) -> Value {
        json!({
            "engaged": self.engaged,
            "history_window_turns": self.history_window_turns,
            "history_load_status": self.history_load_status.as_str(),
            "history_load_error": self.history_load_error.map(|error| error.as_str()),
            "failed_final_status_events": self.failed_final_status_events,
            "failed_final_status_threshold": self.failed_final_status_threshold,
            "failed_threshold_triggered": self.failed_threshold_triggered,
            "backpressure_failure_events": self.backpressure_failure_events,
            "backpressure_failure_threshold": self.backpressure_failure_threshold,
            "backpressure_threshold_triggered": self.backpressure_threshold_triggered,
            "trend_enabled": self.trend_enabled,
            "trend_samples": self.trend_samples,
            "trend_min_samples": self.trend_min_samples,
            "trend_failure_ewma": self.trend_failure_ewma,
            "trend_failure_ewma_threshold": self.trend_failure_ewma_threshold,
            "trend_backpressure_ewma": self.trend_backpressure_ewma,
            "trend_backpressure_ewma_threshold": self.trend_backpressure_ewma_threshold,
            "trend_threshold_triggered": self.trend_threshold_triggered,
            "recovery_success_streak": self.recovery_success_streak,
            "recovery_success_streak_threshold": self.recovery_success_streak_threshold,
            "recovery_failure_ewma_threshold": self.recovery_failure_ewma_threshold,
            "recovery_backpressure_ewma_threshold": self.recovery_backpressure_ewma_threshold,
            "recovery_threshold_triggered": self.recovery_threshold_triggered,
            "force_no_replan": self.force_no_replan,
            "forced_node_max_attempts": self.forced_node_max_attempts,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct SafeLaneGovernorHistorySignals {
    pub(super) history_load_status: SafeLaneGovernorHistoryLoadStatus,
    pub(super) history_load_error: Option<AssistantHistoryLoadErrorCode>,
    pub(super) summary: SafeLaneEventSummary,
    pub(super) final_status_failed_samples: Vec<bool>,
    pub(super) backpressure_failure_samples: Vec<bool>,
}

pub(super) fn summarize_safe_lane_tool_output_stats(outputs: &[String]) -> SafeLaneToolOutputStats {
    let mut stats = SafeLaneToolOutputStats::default();
    for output in outputs {
        for line in output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            stats.output_lines = stats.output_lines.saturating_add(1);
            if !line.starts_with('[') {
                continue;
            }
            stats.result_lines = stats.result_lines.saturating_add(1);
            if tool_result_contains_truncation_signal(line) {
                stats.truncated_result_lines = stats.truncated_result_lines.saturating_add(1);
            }
        }
    }
    stats
}

pub(super) fn derive_safe_lane_runtime_health_signal(
    config: &LoongConfig,
    metrics: SafeLaneExecutionMetrics,
    final_status_failed: bool,
    final_failure_code: Option<&str>,
) -> SafeLaneRuntimeHealthSignal {
    let rounds_started = metrics.rounds_started as f64;
    let replan_rate = if rounds_started > 0.0 {
        metrics.replans_triggered as f64 / rounds_started
    } else {
        0.0
    };
    let verify_failure_rate = if rounds_started > 0.0 {
        metrics.verify_failures as f64 / rounds_started
    } else {
        0.0
    };
    let aggregate_truncation_ratio = metrics
        .aggregate_tool_truncation_ratio_milli()
        .map(|milli| (milli as f64) / 1000.0);
    let truncation_warn_threshold = config
        .conversation
        .safe_lane_health_truncation_warn_threshold();
    let truncation_critical_threshold = config
        .conversation
        .safe_lane_health_truncation_critical_threshold();
    let verify_failure_warn_threshold = config
        .conversation
        .safe_lane_health_verify_failure_warn_threshold();
    let replan_warn_threshold = config.conversation.safe_lane_health_replan_warn_threshold();

    let mut flags = Vec::new();
    let mut has_critical = false;

    if let Some(ratio) = aggregate_truncation_ratio {
        if ratio >= truncation_critical_threshold {
            flags.push(format!("truncation_severe({ratio:.3})"));
            has_critical = true;
        } else if ratio >= truncation_warn_threshold {
            flags.push(format!("truncation_pressure({ratio:.3})"));
        }
    }

    if verify_failure_rate >= verify_failure_warn_threshold {
        flags.push(format!("verify_failure_pressure({verify_failure_rate:.3})"));
    }
    if replan_rate >= replan_warn_threshold {
        flags.push(format!("replan_pressure({replan_rate:.3})"));
    }

    let terminal_instability = final_status_failed
        && final_failure_code
            .map(|code| {
                code.contains("verify_failed")
                    || code.contains("backpressure")
                    || code.contains("session_governor")
            })
            .unwrap_or(false);
    if terminal_instability {
        flags.push("terminal_instability".to_owned());
        has_critical = true;
    }

    SafeLaneRuntimeHealthSignal {
        severity: if has_critical {
            "critical"
        } else if flags.is_empty() {
            "ok"
        } else {
            "warn"
        },
        flags,
    }
}

pub(super) fn select_safe_lane_risk_tier(
    config: &LoongConfig,
    lane_decision: &LaneDecision,
) -> RiskTier {
    let high_risk_bar = config
        .conversation
        .safe_lane_risk_threshold
        .saturating_mul(2);
    let high_complexity_bar = config
        .conversation
        .safe_lane_complexity_threshold
        .saturating_mul(2);
    if lane_decision.risk_score >= high_risk_bar
        || lane_decision.complexity_score >= high_complexity_bar
    {
        RiskTier::High
    } else if lane_decision.risk_score > 0 || lane_decision.complexity_score > 0 {
        RiskTier::Medium
    } else {
        RiskTier::Low
    }
}

pub(super) fn compute_safe_lane_verify_min_anchor_matches(
    config: &LoongConfig,
    verify_failures: u32,
) -> usize {
    if !config
        .conversation
        .safe_lane_verify_adaptive_anchor_escalation
    {
        return 0;
    }
    if verify_failures
        < config
            .conversation
            .safe_lane_verify_anchor_escalation_after_failures()
    {
        return 0;
    }
    config
        .conversation
        .safe_lane_verify_anchor_escalation_min_matches()
}

pub(super) fn decide_safe_lane_session_governor(
    config: &LoongConfig,
    history: &SafeLaneGovernorHistorySignals,
) -> SafeLaneSessionGovernorDecision {
    let summary = &history.summary;
    let history_window_turns = config
        .conversation
        .safe_lane_session_governor_window_turns();
    let failed_final_status_events = summary.failed_final_status_events();
    let backpressure_failure_events = summary.backpressure_failure_events();
    let failed_final_status_threshold = config
        .conversation
        .safe_lane_session_governor_failed_final_status_threshold();
    let backpressure_failure_threshold = config
        .conversation
        .safe_lane_session_governor_backpressure_failure_threshold();
    let failed_threshold_triggered = failed_final_status_events >= failed_final_status_threshold;
    let backpressure_threshold_triggered =
        backpressure_failure_events >= backpressure_failure_threshold;
    let trend_enabled = config.conversation.safe_lane_session_governor_trend_enabled;
    let trend_samples = history.final_status_failed_samples.len();
    let trend_min_samples = config
        .conversation
        .safe_lane_session_governor_trend_min_samples();
    let trend_failure_ewma_threshold = config
        .conversation
        .safe_lane_session_governor_trend_failure_ewma_threshold();
    let trend_backpressure_ewma_threshold = config
        .conversation
        .safe_lane_session_governor_trend_backpressure_ewma_threshold();
    let trend_ewma_alpha = config
        .conversation
        .safe_lane_session_governor_trend_ewma_alpha();
    let trend_ready = trend_enabled && trend_samples >= trend_min_samples;
    let trend_failure_ewma = if trend_ready {
        compute_ewma_bool(
            history.final_status_failed_samples.as_slice(),
            trend_ewma_alpha,
        )
    } else {
        None
    };
    let trend_backpressure_ewma = if trend_ready {
        compute_ewma_bool(
            history.backpressure_failure_samples.as_slice(),
            trend_ewma_alpha,
        )
    } else {
        None
    };
    let trend_threshold_triggered = trend_failure_ewma
        .map(|value| value >= trend_failure_ewma_threshold)
        .unwrap_or(false)
        || trend_backpressure_ewma
            .map(|value| value >= trend_backpressure_ewma_threshold)
            .unwrap_or(false);

    let recovery_success_streak = if trend_ready {
        trailing_success_streak(history.final_status_failed_samples.as_slice())
    } else {
        0
    };
    let recovery_success_streak_threshold = config
        .conversation
        .safe_lane_session_governor_recovery_success_streak();
    let recovery_failure_ewma_threshold = config
        .conversation
        .safe_lane_session_governor_recovery_max_failure_ewma();
    let recovery_backpressure_ewma_threshold = config
        .conversation
        .safe_lane_session_governor_recovery_max_backpressure_ewma();
    let recovery_threshold_triggered = trend_ready
        && recovery_success_streak >= recovery_success_streak_threshold
        && trend_failure_ewma
            .map(|value| value <= recovery_failure_ewma_threshold)
            .unwrap_or(false)
        && trend_backpressure_ewma
            .map(|value| value <= recovery_backpressure_ewma_threshold)
            .unwrap_or(false);

    let engaged = config.conversation.safe_lane_session_governor_enabled
        && (failed_threshold_triggered
            || backpressure_threshold_triggered
            || trend_threshold_triggered)
        && !recovery_threshold_triggered;

    SafeLaneSessionGovernorDecision {
        engaged,
        history_window_turns,
        history_load_status: history.history_load_status,
        history_load_error: history.history_load_error,
        failed_final_status_events,
        failed_final_status_threshold,
        failed_threshold_triggered,
        backpressure_failure_events,
        backpressure_failure_threshold,
        backpressure_threshold_triggered,
        trend_enabled,
        trend_samples,
        trend_min_samples,
        trend_failure_ewma,
        trend_failure_ewma_threshold,
        trend_backpressure_ewma,
        trend_backpressure_ewma_threshold,
        trend_threshold_triggered,
        recovery_success_streak,
        recovery_success_streak_threshold,
        recovery_failure_ewma_threshold,
        recovery_backpressure_ewma_threshold,
        recovery_threshold_triggered,
        force_no_replan: engaged
            && config
                .conversation
                .safe_lane_session_governor_force_no_replan,
        forced_node_max_attempts: engaged.then(|| {
            config
                .conversation
                .safe_lane_session_governor_force_node_max_attempts()
        }),
    }
}

pub(super) async fn load_safe_lane_history_signals_for_governor(
    config: &LoongConfig,
    session_id: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> SafeLaneGovernorHistorySignals {
    if !config.conversation.safe_lane_session_governor_enabled {
        return SafeLaneGovernorHistorySignals::default();
    }

    let window_turns = config
        .conversation
        .safe_lane_session_governor_window_turns();
    #[cfg(feature = "memory-sqlite")]
    {
        let memory_config = MemoryRuntimeConfig::from_memory_config(&config.memory);
        return match load_assistant_contents_from_session_window_detailed(
            session_id,
            window_turns,
            binding,
            &memory_config,
        )
        .await
        {
            Ok(assistant_contents) => {
                summarize_governor_history_signals(assistant_contents.iter().map(String::as_str))
            }
            Err(error) => SafeLaneGovernorHistorySignals {
                history_load_status: SafeLaneGovernorHistoryLoadStatus::Unavailable,
                history_load_error: Some(error.code()),
                ..SafeLaneGovernorHistorySignals::default()
            },
        };
    }

    #[cfg(not(feature = "memory-sqlite"))]
    {
        SafeLaneGovernorHistorySignals::default()
    }
}

pub(super) fn summarize_governor_history_signals<'a, I>(
    assistant_contents: I,
) -> SafeLaneGovernorHistorySignals
where
    I: IntoIterator<Item = &'a str>,
{
    let projection = summarize_safe_lane_history(assistant_contents);
    SafeLaneGovernorHistorySignals {
        history_load_status: SafeLaneGovernorHistoryLoadStatus::Loaded,
        history_load_error: None,
        summary: projection.summary,
        final_status_failed_samples: projection.final_status_failed_samples,
        backpressure_failure_samples: projection.backpressure_failure_samples,
    }
}

fn compute_ewma_bool(samples: &[bool], alpha: f64) -> Option<f64> {
    let mut iter = samples.iter();
    let first = iter.next().copied()?;
    let mut ewma = if first { 1.0 } else { 0.0 };
    for sample in iter {
        let value = if *sample { 1.0 } else { 0.0 };
        ewma = (alpha * value) + ((1.0 - alpha) * ewma);
    }
    Some(ewma)
}

fn trailing_success_streak(failed_samples: &[bool]) -> u32 {
    let mut streak = 0u32;
    for failed in failed_samples.iter().rev() {
        if *failed {
            break;
        }
        streak = streak.saturating_add(1);
    }
    streak
}

pub(super) fn safe_lane_backpressure_budget(
    config: &LoongConfig,
) -> Option<SafeLaneBackpressureBudget> {
    config
        .conversation
        .safe_lane_backpressure_guard_enabled
        .then(|| {
            SafeLaneBackpressureBudget::new(
                config
                    .conversation
                    .safe_lane_backpressure_max_total_attempts(),
                config.conversation.safe_lane_backpressure_max_replans(),
            )
        })
}
