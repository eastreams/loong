use super::*;
#[cfg(test)]
use crate::conversation::load_fast_lane_tool_batch_event_summary;

#[cfg(any(test, feature = "memory-sqlite"))]
pub(super) fn render_fast_lane_summary_lines_with_width(
    session_id: &str,
    limit: usize,
    summary: &FastLaneToolBatchEventSummary,
    width: usize,
) -> Vec<String> {
    let message_spec = build_fast_lane_summary_message_spec(session_id, limit, summary);
    render_cli_chat_message_spec_with_width(&message_spec, width)
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_fast_lane_summary_message_spec(
    session_id: &str,
    limit: usize,
    summary: &FastLaneToolBatchEventSummary,
) -> TuiMessageSpec {
    let parallel_safe_ratio = format_ratio(
        summary.total_parallel_safe_intents_seen,
        summary.total_intents_seen,
    );
    let serial_only_ratio = format_ratio(
        summary.total_serial_only_intents_seen,
        summary.total_intents_seen,
    );
    let configured_max_in_flight_avg = format_average(
        summary.parallel_execution_max_in_flight_sum,
        summary.parallel_execution_max_in_flight_samples,
    );
    let observed_peak_in_flight_avg = format_average(
        summary.observed_peak_in_flight_sum,
        summary.observed_peak_in_flight_samples,
    );
    let observed_wall_time_ms_avg = format_average(
        summary.observed_wall_time_ms_sum,
        summary.observed_wall_time_ms_samples,
    );
    let scheduling_class_values = collect_rollup_values(&summary.scheduling_class_counts);
    let execution_mode_values = collect_rollup_values(&summary.execution_mode_counts);
    let rollup_scheduling_classes = csv_values_or_dash(scheduling_class_values);
    let rollup_execution_modes = csv_values_or_dash(execution_mode_values);
    let latest_segment_lines = build_fast_lane_segment_lines(&summary.latest_segments);
    let caption = format!("session={session_id} limit={limit}");
    let sections = vec![
        TuiSectionSpec::KeyValues {
            title: Some("events".to_owned()),
            items: vec![
                tui_plain_item("batch events", summary.batch_events.to_string()),
                tui_plain_item(
                    "schema version",
                    format_fast_lane_summary_optional(summary.latest_schema_version),
                ),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("batch mix".to_owned()),
            items: vec![
                tui_plain_item(
                    "parallel enabled",
                    summary.parallel_execution_enabled_batches.to_string(),
                ),
                tui_plain_item("parallel only", summary.parallel_only_batches.to_string()),
                tui_plain_item("mixed", summary.mixed_execution_batches.to_string()),
                tui_plain_item(
                    "sequential only",
                    summary.sequential_only_batches.to_string(),
                ),
                tui_plain_item(
                    "without segments",
                    summary.batches_without_segments.to_string(),
                ),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("intent mix".to_owned()),
            items: vec![
                tui_plain_item("total intents", summary.total_intents_seen.to_string()),
                tui_plain_item(
                    "parallel-safe intents",
                    summary.total_parallel_safe_intents_seen.to_string(),
                ),
                tui_plain_item(
                    "serial-only intents",
                    summary.total_serial_only_intents_seen.to_string(),
                ),
                tui_plain_item("parallel-safe ratio", parallel_safe_ratio),
                tui_plain_item("serial-only ratio", serial_only_ratio),
                tui_plain_item(
                    "parallel segments",
                    summary.total_parallel_segments_seen.to_string(),
                ),
                tui_plain_item(
                    "sequential segments",
                    summary.total_sequential_segments_seen.to_string(),
                ),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("execution".to_owned()),
            items: vec![
                tui_plain_item("configured max in flight avg", configured_max_in_flight_avg),
                tui_plain_item(
                    "configured max in flight max",
                    format_fast_lane_summary_optional(summary.parallel_execution_max_in_flight_max),
                ),
                tui_plain_item(
                    "configured max in flight samples",
                    summary.parallel_execution_max_in_flight_samples.to_string(),
                ),
                tui_plain_item("observed peak avg", observed_peak_in_flight_avg),
                tui_plain_item(
                    "observed peak max",
                    format_fast_lane_summary_optional(summary.observed_peak_in_flight_max),
                ),
                tui_plain_item(
                    "observed peak samples",
                    summary.observed_peak_in_flight_samples.to_string(),
                ),
                tui_plain_item("wall time avg", observed_wall_time_ms_avg),
                tui_plain_item(
                    "wall time max",
                    format_fast_lane_summary_optional(summary.observed_wall_time_ms_max),
                ),
                tui_plain_item(
                    "wall time samples",
                    summary.observed_wall_time_ms_samples.to_string(),
                ),
                tui_plain_item(
                    "degraded parallel segments",
                    summary.degraded_parallel_segments.to_string(),
                ),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("latest batch".to_owned()),
            items: vec![
                tui_plain_item(
                    "total intents",
                    format_fast_lane_summary_optional(summary.latest_total_intents),
                ),
                tui_plain_item(
                    "parallel enabled",
                    format_fast_lane_summary_optional(summary.latest_parallel_execution_enabled),
                ),
                tui_plain_item(
                    "max in flight",
                    format_fast_lane_summary_optional(
                        summary.latest_parallel_execution_max_in_flight,
                    ),
                ),
                tui_plain_item(
                    "observed peak",
                    format_fast_lane_summary_optional(summary.latest_observed_peak_in_flight),
                ),
                tui_plain_item(
                    "wall time ms",
                    format_fast_lane_summary_optional(summary.latest_observed_wall_time_ms),
                ),
                tui_plain_item(
                    "parallel-safe intents",
                    format_fast_lane_summary_optional(summary.latest_parallel_safe_intents),
                ),
                tui_plain_item(
                    "serial-only intents",
                    format_fast_lane_summary_optional(summary.latest_serial_only_intents),
                ),
                tui_plain_item(
                    "parallel segments",
                    format_fast_lane_summary_optional(summary.latest_parallel_segments),
                ),
                tui_plain_item(
                    "sequential segments",
                    format_fast_lane_summary_optional(summary.latest_sequential_segments),
                ),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("rollups".to_owned()),
            items: vec![
                tui_csv_item("scheduling classes", rollup_scheduling_classes),
                tui_csv_item("execution modes", rollup_execution_modes),
            ],
        },
        TuiSectionSpec::Narrative {
            title: Some("latest segments".to_owned()),
            lines: latest_segment_lines,
        },
    ];

    TuiMessageSpec {
        role: "fast-lane".to_owned(),
        caption: Some(caption),
        sections,
        footer_lines: vec![
            "Use /fast_lane_summary after tool-heavy turns to inspect concurrency behavior."
                .to_owned(),
        ],
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_fast_lane_segment_lines(segments: &[FastLaneToolBatchSegmentSnapshot]) -> Vec<String> {
    if segments.is_empty() {
        return vec!["- no segment snapshot recorded".to_owned()];
    }

    let mut lines = Vec::new();

    for segment in segments {
        let peak_in_flight = format_fast_lane_summary_optional(segment.observed_peak_in_flight);
        let wall_time_ms = format_fast_lane_summary_optional(segment.observed_wall_time_ms);
        let line = format!(
            "- segment {}: class={} mode={} intents={} peak={} wall_ms={}",
            segment.segment_index,
            segment.scheduling_class,
            segment.execution_mode,
            segment.intent_count,
            peak_in_flight,
            wall_time_ms,
        );

        lines.push(line);
    }

    lines
}

#[cfg(test)]
pub(super) async fn load_fast_lane_summary_output(
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
    memory_config: &SessionStoreConfig,
) -> CliResult<String> {
    let summary =
        load_fast_lane_tool_batch_event_summary(session_id, limit, binding, memory_config).await?;

    Ok(format_fast_lane_summary(session_id, limit, &summary))
}

#[cfg(test)]
fn format_fast_lane_segments(segments: &[FastLaneToolBatchSegmentSnapshot]) -> String {
    if segments.is_empty() {
        return "-".to_owned();
    }

    segments
        .iter()
        .map(|segment| {
            let observed_suffix = match (
                segment.observed_peak_in_flight,
                segment.observed_wall_time_ms,
            ) {
                (None, None) => String::new(),
                (observed_peak_in_flight, observed_wall_time_ms) => format!(
                    "[peak={} wall_ms={}]",
                    format_fast_lane_summary_optional(observed_peak_in_flight),
                    format_fast_lane_summary_optional(observed_wall_time_ms)
                ),
            };
            format!(
                "{}:{}/{}/{}{}",
                segment.segment_index,
                segment.scheduling_class,
                segment.execution_mode,
                segment.intent_count,
                observed_suffix,
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
pub(super) fn format_fast_lane_summary(
    session_id: &str,
    limit: usize,
    summary: &FastLaneToolBatchEventSummary,
) -> String {
    let parallel_safe_ratio = format_ratio(
        summary.total_parallel_safe_intents_seen,
        summary.total_intents_seen,
    );
    let serial_only_ratio = format_ratio(
        summary.total_serial_only_intents_seen,
        summary.total_intents_seen,
    );
    let configured_max_in_flight_avg = format_average(
        summary.parallel_execution_max_in_flight_sum,
        summary.parallel_execution_max_in_flight_samples,
    );
    let observed_peak_in_flight_avg = format_average(
        summary.observed_peak_in_flight_sum,
        summary.observed_peak_in_flight_samples,
    );
    let observed_wall_time_ms_avg = format_average(
        summary.observed_wall_time_ms_sum,
        summary.observed_wall_time_ms_samples,
    );
    let scheduling_class_rollup = format_rollup_counts(&summary.scheduling_class_counts);
    let execution_mode_rollup = format_rollup_counts(&summary.execution_mode_counts);

    [
        format!("fast_lane_summary session={session_id} limit={limit}"),
        format!(
            "events batch_events={} schema_version={}",
            summary.batch_events,
            format_fast_lane_summary_optional(summary.latest_schema_version)
        ),
        format!(
            "aggregate_batches parallel_enabled={} parallel_only={} mixed={} sequential_only={} without_segments={}",
            summary.parallel_execution_enabled_batches,
            summary.parallel_only_batches,
            summary.mixed_execution_batches,
            summary.sequential_only_batches,
            summary.batches_without_segments,
        ),
        format!(
            "aggregate_intents total={} parallel_safe={} serial_only={} parallel_safe_ratio={} serial_only_ratio={}",
            summary.total_intents_seen,
            summary.total_parallel_safe_intents_seen,
            summary.total_serial_only_intents_seen,
            parallel_safe_ratio,
            serial_only_ratio,
        ),
        format!(
            "aggregate_segments parallel={} sequential={}",
            summary.total_parallel_segments_seen,
            summary.total_sequential_segments_seen,
        ),
        format!(
            "aggregate_execution configured_max_in_flight_avg={} configured_max_in_flight_max={} configured_max_in_flight_samples={} observed_peak_in_flight_avg={} observed_peak_in_flight_max={} observed_peak_in_flight_samples={} degraded_parallel_segments={}",
            configured_max_in_flight_avg,
            format_fast_lane_summary_optional(summary.parallel_execution_max_in_flight_max),
            summary.parallel_execution_max_in_flight_samples,
            observed_peak_in_flight_avg,
            format_fast_lane_summary_optional(summary.observed_peak_in_flight_max),
            summary.observed_peak_in_flight_samples,
            summary.degraded_parallel_segments,
        ),
        format!(
            "aggregate_latency observed_wall_time_ms_avg={} observed_wall_time_ms_max={} observed_wall_time_ms_samples={}",
            observed_wall_time_ms_avg,
            format_fast_lane_summary_optional(summary.observed_wall_time_ms_max),
            summary.observed_wall_time_ms_samples,
        ),
        format!("rollup scheduling_classes={scheduling_class_rollup}"),
        format!("rollup execution_modes={execution_mode_rollup}"),
        format!(
            "latest_batch total_intents={} parallel_enabled={} max_in_flight={} observed_peak_in_flight={} observed_wall_time_ms={} parallel_safe_intents={} serial_only_intents={} parallel_segments={} sequential_segments={}",
            format_fast_lane_summary_optional(summary.latest_total_intents),
            format_fast_lane_summary_optional(summary.latest_parallel_execution_enabled),
            format_fast_lane_summary_optional(summary.latest_parallel_execution_max_in_flight),
            format_fast_lane_summary_optional(summary.latest_observed_peak_in_flight),
            format_fast_lane_summary_optional(summary.latest_observed_wall_time_ms),
            format_fast_lane_summary_optional(summary.latest_parallel_safe_intents),
            format_fast_lane_summary_optional(summary.latest_serial_only_intents),
            format_fast_lane_summary_optional(summary.latest_parallel_segments),
            format_fast_lane_summary_optional(summary.latest_sequential_segments),
        ),
        format!(
            "latest_segments={}",
            format_fast_lane_segments(&summary.latest_segments)
        ),
    ]
    .join("\n")
}
