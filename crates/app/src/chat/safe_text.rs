use super::*;
#[cfg(test)]
use crate::conversation::load_safe_lane_event_summary;

#[cfg(test)]
pub(super) async fn load_safe_lane_summary_output(
    session_id: &str,
    limit: usize,
    conversation_config: &ConversationConfig,
    binding: ConversationRuntimeBinding<'_>,
    memory_config: &SessionStoreConfig,
) -> CliResult<String> {
    let summary = load_safe_lane_event_summary(session_id, limit, binding, memory_config).await?;

    Ok(format_safe_lane_summary(
        session_id,
        limit,
        conversation_config,
        &summary,
    ))
}

#[cfg(test)]
pub(super) fn format_safe_lane_summary(
    session_id: &str,
    limit: usize,
    conversation_config: &ConversationConfig,
    summary: &SafeLaneEventSummary,
) -> String {
    let final_status = match summary.final_status {
        Some(SafeLaneFinalStatus::Succeeded) => "succeeded",
        Some(SafeLaneFinalStatus::Failed) => "failed",
        None => "unknown",
    };
    let final_failure_code = summary.final_failure_code.as_deref().unwrap_or("-");
    let final_route = summary.final_route_decision.as_deref().unwrap_or("-");
    let final_route_reason = summary.final_route_reason.as_deref().unwrap_or("-");
    let metrics = summary.latest_metrics.as_ref();
    let rounds_started = metrics
        .map(|value| value.rounds_started as f64)
        .unwrap_or(summary.round_started_events as f64);
    let replan_rate = if rounds_started > 0.0 {
        summary.replan_triggered_events as f64 / rounds_started
    } else {
        0.0
    };
    let verify_failure_rate = if rounds_started > 0.0 {
        summary.verify_failed_events as f64 / rounds_started
    } else {
        0.0
    };
    let route_rollup = format_rollup_counts(&summary.route_decision_counts);
    let route_reason_rollup = format_rollup_counts(&summary.route_reason_counts);
    let failure_rollup = format_rollup_counts(&summary.failure_code_counts);
    let governor_trend_failure_ewma =
        format_milli_ratio(summary.session_governor_latest_trend_failure_ewma_milli);
    let governor_trend_backpressure_ewma =
        format_milli_ratio(summary.session_governor_latest_trend_backpressure_ewma_milli);
    let latest_tool_truncation_ratio = format_milli_ratio(
        summary
            .latest_tool_output
            .as_ref()
            .map(|snapshot| snapshot.truncation_ratio_milli),
    );
    let aggregate_tool_truncation_ratio_milli = summary
        .tool_output_aggregate_truncation_ratio_milli
        .or_else(|| {
            if summary.tool_output_result_lines_total == 0 {
                None
            } else {
                Some(
                    summary
                        .tool_output_truncated_result_lines_total
                        .saturating_mul(1000)
                        .saturating_div(summary.tool_output_result_lines_total)
                        .min(u32::MAX as u64) as u32,
                )
            }
        });
    let aggregate_tool_truncation_ratio =
        aggregate_tool_truncation_ratio_milli.map(|milli| (milli as f64) / 1000.0);
    let aggregate_tool_truncation_ratio_text = aggregate_tool_truncation_ratio
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| "-".to_owned());
    let health_signal = derive_safe_lane_health_signal(
        conversation_config,
        summary,
        replan_rate,
        verify_failure_rate,
        aggregate_tool_truncation_ratio,
    );
    let health_flags = if health_signal.flags.is_empty() {
        "-".to_owned()
    } else {
        health_signal.flags.join(",")
    };
    let health_payload = serde_json::json!({
        "severity": health_signal.severity,
        "flags": health_signal.flags,
    })
    .to_string();
    let latest_health_event_severity = summary
        .latest_health_signal
        .as_ref()
        .map(|snapshot| snapshot.severity.as_str())
        .unwrap_or("-");
    let latest_health_event_flags = summary
        .latest_health_signal
        .as_ref()
        .map(|snapshot| {
            if snapshot.flags.is_empty() {
                "-".to_owned()
            } else {
                snapshot.flags.join(",")
            }
        })
        .unwrap_or_else(|| "-".to_owned());

    let metrics_line = if let Some(metrics) = metrics {
        format!(
            "latest_metrics rounds_started={} rounds_succeeded={} rounds_failed={} verify_failures={} replans_triggered={} total_attempts_used={}",
            metrics.rounds_started,
            metrics.rounds_succeeded,
            metrics.rounds_failed,
            metrics.verify_failures,
            metrics.replans_triggered,
            metrics.total_attempts_used
        )
    } else {
        "latest_metrics unavailable".to_owned()
    };

    [
        format!("safe_lane_summary session={session_id} limit={limit}"),
        format!(
            "events lane_selected={} round_started={} round_completed_succeeded={} round_completed_failed={} verify_failed={} verify_policy_adjusted={} replan_triggered={} final_status={} governor_engaged={} governor_force_no_replan={}",
            summary.lane_selected_events,
            summary.round_started_events,
            summary.round_completed_succeeded_events,
            summary.round_completed_failed_events,
            summary.verify_failed_events,
            summary.verify_policy_adjusted_events,
            summary.replan_triggered_events,
            summary.final_status_events,
            summary.session_governor_engaged_events,
            summary.session_governor_force_no_replan_events
        ),
        format!(
            "terminal status={} failure_code={} route_decision={} route_reason={}",
            final_status, final_failure_code, final_route, final_route_reason
        ),
        format!(
            "governor trigger_failed_threshold={} trigger_backpressure_threshold={} trigger_trend_threshold={} trigger_recovery_threshold={}",
            summary.session_governor_failed_threshold_triggered_events,
            summary.session_governor_backpressure_threshold_triggered_events,
            summary.session_governor_trend_threshold_triggered_events,
            summary.session_governor_recovery_threshold_triggered_events
        ),
        format!(
            "governor_latest snapshots={} trend_samples={} trend_min_samples={} trend_failure_ewma={} trend_backpressure_ewma={} recovery_success_streak={} recovery_streak_threshold={}",
            summary.session_governor_metrics_snapshots_seen,
            summary
                .session_governor_latest_trend_samples
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            summary
                .session_governor_latest_trend_min_samples
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            governor_trend_failure_ewma,
            governor_trend_backpressure_ewma,
            summary
                .session_governor_latest_recovery_success_streak
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            summary
                .session_governor_latest_recovery_success_streak_threshold
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
        ),
        format!(
            "rates replan_per_round={:.3} verify_fail_per_round={:.3}",
            replan_rate, verify_failure_rate
        ),
        format!(
            "tool_output snapshots={} truncated_events={} result_lines_total={} truncated_result_lines_total={} latest_truncation_ratio={} aggregate_truncation_ratio={} aggregate_truncation_ratio_milli={} truncation_verify_failed_events={} truncation_replan_events={} truncation_final_failure_events={}",
            summary.tool_output_snapshots_seen,
            summary.tool_output_truncated_events,
            summary.tool_output_result_lines_total,
            summary.tool_output_truncated_result_lines_total,
            latest_tool_truncation_ratio,
            aggregate_tool_truncation_ratio_text,
            aggregate_tool_truncation_ratio_milli
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            summary.tool_output_truncation_verify_failed_events,
            summary.tool_output_truncation_replan_events,
            summary.tool_output_truncation_final_failure_events
        ),
        format!(
            "health severity={} flags={health_flags}",
            health_signal.severity
        ),
        format!("health_payload {health_payload}"),
        format!(
            "health_events snapshots={} warn={} critical={} latest_severity={} latest_flags={}",
            summary.health_signal_snapshots_seen,
            summary.health_signal_warn_events,
            summary.health_signal_critical_events,
            latest_health_event_severity,
            latest_health_event_flags
        ),
        metrics_line,
        format!("rollup route_decisions={route_rollup}"),
        format!("rollup route_reasons={route_reason_rollup}"),
        format!("rollup failure_codes={failure_rollup}"),
    ]
    .join("\n")
}

#[cfg(any(test, feature = "memory-sqlite"))]
pub(super) fn derive_safe_lane_health_signal(
    _conversation_config: &ConversationConfig,
    summary: &SafeLaneEventSummary,
    replan_rate: f64,
    verify_failure_rate: f64,
    aggregate_truncation_ratio: Option<f64>,
) -> SafeLaneHealthSignal {
    let mut flags = Vec::new();
    let mut has_critical = false;
    let truncation_warn_threshold = 0.25;
    let truncation_critical_threshold = 0.60;
    let verify_failure_warn_threshold = 0.40;
    let replan_warn_threshold = 0.50;

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
    let terminal_instability = summary.has_terminal_instability_final_failure();
    if terminal_instability {
        flags.push("terminal_instability".to_owned());
        has_critical = true;
    }

    SafeLaneHealthSignal {
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

#[cfg(any(test, feature = "memory-sqlite"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SafeLaneHealthSignal {
    pub(super) severity: &'static str,
    pub(super) flags: Vec<String>,
}
