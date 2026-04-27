use super::*;

#[cfg(any(test, feature = "memory-sqlite"))]
#[allow(dead_code)]
pub(super) fn render_safe_lane_summary_lines_with_width(
    session_id: &str,
    limit: usize,
    conversation_config: &ConversationConfig,
    summary: &SafeLaneEventSummary,
    width: usize,
) -> Vec<String> {
    let message_spec =
        build_safe_lane_summary_message_spec(session_id, limit, conversation_config, summary);
    render_cli_chat_message_spec_with_width(&message_spec, width)
}

#[cfg(any(test, feature = "memory-sqlite"))]
#[allow(dead_code)]
fn build_safe_lane_summary_message_spec(
    session_id: &str,
    limit: usize,
    conversation_config: &ConversationConfig,
    summary: &SafeLaneEventSummary,
) -> TuiMessageSpec {
    let final_status = match summary.final_status {
        Some(SafeLaneFinalStatus::Succeeded) => "succeeded",
        Some(SafeLaneFinalStatus::Failed) => "failed",
        None => "unknown",
    };
    let final_failure_code = summary.final_failure_code.as_deref().unwrap_or("-");
    let final_route_decision = summary.final_route_decision.as_deref().unwrap_or("-");
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
                return None;
            }

            let truncated_lines = summary.tool_output_truncated_result_lines_total;
            let total_lines = summary.tool_output_result_lines_total;
            let ratio_milli = truncated_lines
                .saturating_mul(1000)
                .saturating_div(total_lines)
                .min(u32::MAX as u64) as u32;

            Some(ratio_milli)
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
        "none".to_owned()
    } else {
        health_signal.flags.join(", ")
    };
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
                return "none".to_owned();
            }

            snapshot.flags.join(", ")
        })
        .unwrap_or_else(|| "-".to_owned());
    let route_decision_values = collect_rollup_values(&summary.route_decision_counts);
    let route_reason_values = collect_rollup_values(&summary.route_reason_counts);
    let failure_code_values = collect_rollup_values(&summary.failure_code_counts);
    let rollup_route_decisions = csv_values_or_dash(route_decision_values);
    let rollup_route_reasons = csv_values_or_dash(route_reason_values);
    let rollup_failure_codes = csv_values_or_dash(failure_code_values);
    let health_tone = safe_lane_health_tone(health_signal.severity);
    let latest_metrics_section = match metrics {
        Some(metrics) => TuiSectionSpec::KeyValues {
            title: Some("latest metrics".to_owned()),
            items: vec![
                tui_plain_item("rounds started", metrics.rounds_started.to_string()),
                tui_plain_item("rounds succeeded", metrics.rounds_succeeded.to_string()),
                tui_plain_item("rounds failed", metrics.rounds_failed.to_string()),
                tui_plain_item("verify failures", metrics.verify_failures.to_string()),
                tui_plain_item("replans triggered", metrics.replans_triggered.to_string()),
                tui_plain_item("attempts used", metrics.total_attempts_used.to_string()),
            ],
        },
        None => TuiSectionSpec::KeyValues {
            title: Some("latest metrics".to_owned()),
            items: vec![tui_plain_item("status", "unavailable".to_owned())],
        },
    };
    let caption = format!("session={session_id} limit={limit}");
    let sections = vec![
        TuiSectionSpec::KeyValues {
            title: Some("terminal status".to_owned()),
            items: vec![
                tui_plain_item("status", final_status.to_owned()),
                tui_plain_item("failure code", final_failure_code.to_owned()),
                tui_plain_item("route decision", final_route_decision.to_owned()),
                tui_plain_item("route reason", final_route_reason.to_owned()),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("events".to_owned()),
            items: vec![
                tui_plain_item("lane selected", summary.lane_selected_events.to_string()),
                tui_plain_item("round started", summary.round_started_events.to_string()),
                tui_plain_item(
                    "round succeeded",
                    summary.round_completed_succeeded_events.to_string(),
                ),
                tui_plain_item(
                    "round failed",
                    summary.round_completed_failed_events.to_string(),
                ),
                tui_plain_item("verify failed", summary.verify_failed_events.to_string()),
                tui_plain_item(
                    "verify policy adjusted",
                    summary.verify_policy_adjusted_events.to_string(),
                ),
                tui_plain_item(
                    "replan triggered",
                    summary.replan_triggered_events.to_string(),
                ),
                tui_plain_item("final status", summary.final_status_events.to_string()),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("rates".to_owned()),
            items: vec![
                tui_plain_item("replan per round", format!("{replan_rate:.3}")),
                tui_plain_item("verify fail per round", format!("{verify_failure_rate:.3}")),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("governor".to_owned()),
            items: vec![
                tui_plain_item(
                    "engaged events",
                    summary.session_governor_engaged_events.to_string(),
                ),
                tui_plain_item(
                    "force no replan",
                    summary.session_governor_force_no_replan_events.to_string(),
                ),
                tui_plain_item(
                    "failed threshold triggers",
                    summary
                        .session_governor_failed_threshold_triggered_events
                        .to_string(),
                ),
                tui_plain_item(
                    "backpressure threshold triggers",
                    summary
                        .session_governor_backpressure_threshold_triggered_events
                        .to_string(),
                ),
                tui_plain_item(
                    "trend threshold triggers",
                    summary
                        .session_governor_trend_threshold_triggered_events
                        .to_string(),
                ),
                tui_plain_item(
                    "recovery threshold triggers",
                    summary
                        .session_governor_recovery_threshold_triggered_events
                        .to_string(),
                ),
                tui_plain_item(
                    "metric snapshots",
                    summary.session_governor_metrics_snapshots_seen.to_string(),
                ),
                tui_plain_item(
                    "trend samples",
                    format_fast_lane_summary_optional(
                        summary.session_governor_latest_trend_samples,
                    ),
                ),
                tui_plain_item(
                    "trend min samples",
                    format_fast_lane_summary_optional(
                        summary.session_governor_latest_trend_min_samples,
                    ),
                ),
                tui_plain_item("trend failure ewma", governor_trend_failure_ewma),
                tui_plain_item("trend backpressure ewma", governor_trend_backpressure_ewma),
                tui_plain_item(
                    "recovery success streak",
                    format_fast_lane_summary_optional(
                        summary.session_governor_latest_recovery_success_streak,
                    ),
                ),
                tui_plain_item(
                    "recovery streak threshold",
                    format_fast_lane_summary_optional(
                        summary.session_governor_latest_recovery_success_streak_threshold,
                    ),
                ),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("tool output".to_owned()),
            items: vec![
                tui_plain_item("snapshots", summary.tool_output_snapshots_seen.to_string()),
                tui_plain_item(
                    "truncated events",
                    summary.tool_output_truncated_events.to_string(),
                ),
                tui_plain_item(
                    "result lines total",
                    summary.tool_output_result_lines_total.to_string(),
                ),
                tui_plain_item(
                    "truncated result lines",
                    summary.tool_output_truncated_result_lines_total.to_string(),
                ),
                tui_plain_item("latest truncation ratio", latest_tool_truncation_ratio),
                tui_plain_item(
                    "aggregate truncation ratio",
                    aggregate_tool_truncation_ratio_text,
                ),
                tui_plain_item(
                    "aggregate truncation ratio milli",
                    format_fast_lane_summary_optional(aggregate_tool_truncation_ratio_milli),
                ),
                tui_plain_item(
                    "truncation verify failed",
                    summary
                        .tool_output_truncation_verify_failed_events
                        .to_string(),
                ),
                tui_plain_item(
                    "truncation replan",
                    summary.tool_output_truncation_replan_events.to_string(),
                ),
                tui_plain_item(
                    "truncation final failure",
                    summary
                        .tool_output_truncation_final_failure_events
                        .to_string(),
                ),
            ],
        },
        TuiSectionSpec::Callout {
            tone: health_tone,
            title: Some("health".to_owned()),
            lines: vec![
                format!("severity: {}", health_signal.severity),
                format!("flags: {health_flags}"),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("health events".to_owned()),
            items: vec![
                tui_plain_item(
                    "snapshots",
                    summary.health_signal_snapshots_seen.to_string(),
                ),
                tui_plain_item("warn events", summary.health_signal_warn_events.to_string()),
                tui_plain_item(
                    "critical events",
                    summary.health_signal_critical_events.to_string(),
                ),
                tui_plain_item("latest severity", latest_health_event_severity.to_owned()),
                tui_plain_item("latest flags", latest_health_event_flags),
            ],
        },
        latest_metrics_section,
        TuiSectionSpec::KeyValues {
            title: Some("rollups".to_owned()),
            items: vec![
                tui_csv_item("route decisions", rollup_route_decisions),
                tui_csv_item("route reasons", rollup_route_reasons),
                tui_csv_item("failure codes", rollup_failure_codes),
            ],
        },
    ];

    TuiMessageSpec {
        role: "safe-lane".to_owned(),
        caption: Some(caption),
        sections,
        footer_lines: vec![
            "Use /safe_lane_summary when verify/replan behavior needs inspection.".to_owned(),
        ],
    }
}
