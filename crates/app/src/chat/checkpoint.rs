use super::*;

#[cfg(any(test, feature = "memory-sqlite"))]
pub(super) fn render_turn_checkpoint_health_error_lines_with_width(
    session_id: &str,
    error: &str,
    width: usize,
) -> Vec<String> {
    let message_spec = build_turn_checkpoint_health_error_message_spec(session_id, error);
    render_cli_chat_message_spec_with_width(&message_spec, width)
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_turn_checkpoint_health_error_message_spec(
    session_id: &str,
    error: &str,
) -> TuiMessageSpec {
    let caption = format!("session={session_id}");
    let sections = vec![
        TuiSectionSpec::KeyValues {
            title: Some("durability status".to_owned()),
            items: vec![
                tui_plain_item("state", "unavailable".to_owned()),
                tui_plain_item("session", session_id.to_owned()),
            ],
        },
        TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Warning,
            title: Some("durability unavailable".to_owned()),
            lines: vec![format!("error: {error}")],
        },
    ];

    TuiMessageSpec {
        role: "checkpoint".to_owned(),
        caption: Some(caption),
        sections,
        footer_lines: vec![
            "Durability state is unavailable until the next successful checkpoint sample."
                .to_owned(),
        ],
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
pub(super) fn render_turn_checkpoint_summary_lines_with_width(
    session_id: &str,
    limit: usize,
    diagnostics: &TurnCheckpointDiagnostics,
    width: usize,
) -> Vec<String> {
    let message_spec = build_turn_checkpoint_summary_message_spec(session_id, limit, diagnostics);
    render_cli_chat_message_spec_with_width(&message_spec, width)
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_turn_checkpoint_summary_message_spec(
    session_id: &str,
    limit: usize,
    diagnostics: &TurnCheckpointDiagnostics,
) -> TuiMessageSpec {
    let summary = diagnostics.summary();
    let render_labels = TurnCheckpointSummaryRenderLabels::from_summary(summary);
    let durability_labels = TurnCheckpointDurabilityRenderLabels::from_summary(summary);
    let recovery_labels =
        TurnCheckpointRecoveryRenderLabels::from_assessment(diagnostics.recovery());
    let failure_step = format_turn_checkpoint_failure_step(summary.latest_failure_step);
    let failure_error = summary.latest_failure_error.as_deref().unwrap_or("-");
    let reply_durable = bool_yes_no_value(summary.reply_durable);
    let checkpoint_durable = bool_yes_no_value(summary.checkpoint_durable);
    let recovery_needed = bool_yes_no_value(summary.requires_recovery);
    let recovery_tone = recovery_callout_tone(summary.requires_recovery);
    let stage_rollup_values = collect_rollup_values(&summary.stage_counts);
    let stage_rollups = csv_values_or_dash(stage_rollup_values);
    let caption = format!("session={session_id} limit={limit}");
    let mut sections = vec![
        TuiSectionSpec::KeyValues {
            title: Some("summary".to_owned()),
            items: vec![
                tui_plain_item("checkpoints", summary.checkpoint_events.to_string()),
                tui_plain_item("state", render_labels.session_state.to_owned()),
                tui_plain_item("durability", durability_labels.durability.to_owned()),
                tui_plain_item("reply durable", reply_durable),
                tui_plain_item("checkpoint durable", checkpoint_durable),
                tui_plain_item("requires recovery", recovery_needed),
            ],
        },
        TuiSectionSpec::Callout {
            tone: recovery_tone,
            title: Some("recovery".to_owned()),
            lines: vec![
                format!("action: {}", recovery_labels.action),
                format!("source: {}", recovery_labels.source),
                format!("reason: {}", recovery_labels.reason),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("latest checkpoint".to_owned()),
            items: vec![
                tui_plain_item("stage", render_labels.stage.to_owned()),
                tui_plain_item("after turn", render_labels.after_turn.to_owned()),
                tui_plain_item("compaction", render_labels.compaction.to_owned()),
                tui_plain_item("lane", render_labels.lane.to_owned()),
                tui_plain_item("result kind", render_labels.result_kind.to_owned()),
                tui_plain_item(
                    "persistence mode",
                    render_labels.persistence_mode.to_owned(),
                ),
                tui_plain_item("identity", render_labels.identity.to_owned()),
                tui_plain_item("failure step", failure_step.to_owned()),
                tui_plain_item("failure error", failure_error.to_owned()),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("events".to_owned()),
            items: vec![
                tui_plain_item("post persist", summary.post_persist_events.to_string()),
                tui_plain_item("finalized", summary.finalized_events.to_string()),
                tui_plain_item(
                    "finalization failed",
                    summary.finalization_failed_events.to_string(),
                ),
                tui_plain_item(
                    "schema version",
                    format_fast_lane_summary_optional(summary.latest_schema_version),
                ),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("rollups".to_owned()),
            items: vec![tui_csv_item("stages", stage_rollups)],
        },
    ];

    if render_labels.safe_lane_route_decision != "-"
        || render_labels.safe_lane_route_reason != "-"
        || render_labels.safe_lane_route_source != "-"
    {
        sections.insert(
            3,
            TuiSectionSpec::KeyValues {
                title: Some("safe-lane route".to_owned()),
                items: vec![
                    tui_plain_item(
                        "decision",
                        render_labels.safe_lane_route_decision.to_owned(),
                    ),
                    tui_plain_item("reason", render_labels.safe_lane_route_reason.to_owned()),
                    tui_plain_item("source", render_labels.safe_lane_route_source.to_owned()),
                ],
            },
        );
    }

    if let Some(probe) = diagnostics.runtime_probe() {
        let probe_lines = vec![
            format!("action: {}", probe.action().as_str()),
            format!("source: {}", probe.source().as_str()),
            format!("reason: {}", probe.reason().as_str()),
        ];

        sections.push(TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Info,
            title: Some("runtime probe".to_owned()),
            lines: probe_lines,
        });
    }

    TuiMessageSpec {
        role: "checkpoint".to_owned(),
        caption: Some(caption),
        sections,
        footer_lines: vec![
            "Use /turn_checkpoint_repair when the latest durable state needs repair.".to_owned(),
        ],
    }
}

#[cfg(any(test, feature = "memory-sqlite"))]
pub(super) fn render_turn_checkpoint_repair_lines_with_width(
    session_id: &str,
    outcome: &TurnCheckpointTailRepairOutcome,
    width: usize,
) -> Vec<String> {
    let message_spec = build_turn_checkpoint_repair_message_spec(session_id, outcome);
    render_cli_chat_message_spec_with_width(&message_spec, width)
}

#[cfg(any(test, feature = "memory-sqlite"))]
fn build_turn_checkpoint_repair_message_spec(
    session_id: &str,
    outcome: &TurnCheckpointTailRepairOutcome,
) -> TuiMessageSpec {
    let after_turn = outcome.after_turn_status().unwrap_or("-");
    let compaction = outcome.compaction_status().unwrap_or("-");
    let source = outcome.source().map(|value| value.as_str()).unwrap_or("-");
    let status = outcome.status();
    let (callout_tone, callout_lines) = match status {
        TurnCheckpointTailRepairStatus::Repaired => (
            TuiCalloutTone::Success,
            vec!["Repair completed and durable checkpoint state was updated.".to_owned()],
        ),
        TurnCheckpointTailRepairStatus::ManualRequired => (
            TuiCalloutTone::Warning,
            vec!["Manual inspection is still required before replaying the session.".to_owned()],
        ),
        TurnCheckpointTailRepairStatus::NotNeeded => (
            TuiCalloutTone::Success,
            vec!["No repair action was required for the latest durable checkpoint.".to_owned()],
        ),
        TurnCheckpointTailRepairStatus::NoCheckpoint => (
            TuiCalloutTone::Info,
            vec!["No durable checkpoint was available to repair.".to_owned()],
        ),
    };
    let caption = format!("session={session_id}");
    let sections = vec![
        TuiSectionSpec::KeyValues {
            title: Some("repair status".to_owned()),
            items: vec![
                tui_plain_item("status", status.as_str().to_owned()),
                tui_plain_item("action", outcome.action().as_str().to_owned()),
                tui_plain_item("source", source.to_owned()),
                tui_plain_item("reason", outcome.reason().as_str().to_owned()),
            ],
        },
        TuiSectionSpec::KeyValues {
            title: Some("checkpoint state".to_owned()),
            items: vec![
                tui_plain_item("session state", outcome.session_state().as_str().to_owned()),
                tui_plain_item("checkpoints", outcome.checkpoint_events().to_string()),
                tui_plain_item("after turn", after_turn.to_owned()),
                tui_plain_item("compaction", compaction.to_owned()),
            ],
        },
        TuiSectionSpec::Callout {
            tone: callout_tone,
            title: Some("repair result".to_owned()),
            lines: callout_lines,
        },
    ];

    TuiMessageSpec {
        role: "repair".to_owned(),
        caption: Some(caption),
        sections,
        footer_lines: vec![
            "Re-run /status after repair to confirm the checkpoint state.".to_owned(),
        ],
    }
}
