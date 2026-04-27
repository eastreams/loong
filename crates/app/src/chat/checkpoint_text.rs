use super::*;

#[cfg(test)]
pub(super) fn format_turn_checkpoint_summary(
    session_id: &str,
    limit: usize,
    diagnostics: &TurnCheckpointDiagnostics,
) -> String {
    let summary = diagnostics.summary();
    let render_labels = TurnCheckpointSummaryRenderLabels::from_summary(summary);
    let durability_labels = TurnCheckpointDurabilityRenderLabels::from_summary(summary);
    let recovery_labels =
        TurnCheckpointRecoveryRenderLabels::from_assessment(diagnostics.recovery());
    let failure_step = format_turn_checkpoint_failure_step(summary.latest_failure_step);
    let requires_recovery = if summary.requires_recovery { 1 } else { 0 };
    let failure_error = summary.latest_failure_error.as_deref().unwrap_or("-");

    let mut lines = vec![format!(
        "turn_checkpoint_summary session={session_id} limit={limit} checkpoints={} state={} durable={} checkpoint_durable={} durability={} requires_recovery={requires_recovery} recovery_action={} recovery_source={} recovery_reason={} stage={} after_turn={} compaction={} lane={} result_kind={} persistence_mode={} safe_lane_route_decision={} safe_lane_route_reason={} safe_lane_route_source={} identity={} failure_step={failure_step} failure_error={failure_error}",
        summary.checkpoint_events,
        render_labels.session_state,
        durability_labels.reply_durable,
        durability_labels.checkpoint_durable,
        durability_labels.durability,
        recovery_labels.action,
        recovery_labels.source,
        recovery_labels.reason,
        render_labels.stage,
        render_labels.after_turn,
        render_labels.compaction,
        render_labels.lane,
        render_labels.result_kind,
        render_labels.persistence_mode,
        render_labels.safe_lane_route_decision,
        render_labels.safe_lane_route_reason,
        render_labels.safe_lane_route_source,
        render_labels.identity,
    )];
    lines.push(format!(
        "events post_persist={} finalized={} finalization_failed={}",
        summary.post_persist_events, summary.finalized_events, summary.finalization_failed_events
    ));
    if !summary.stage_counts.is_empty() {
        let stage_rollup = summary
            .stage_counts
            .iter()
            .map(|(stage_name, count)| format!("{stage_name}:{count}"))
            .collect::<Vec<_>>()
            .join(",");
        lines.push(format!("rollup stages={stage_rollup}"));
    }
    lines.join("\n")
}

#[cfg(test)]
pub(super) fn format_turn_checkpoint_summary_output(
    session_id: &str,
    limit: usize,
    diagnostics: &TurnCheckpointDiagnostics,
) -> String {
    let mut rendered = format_turn_checkpoint_summary(session_id, limit, diagnostics);
    if let Some(probe) = diagnostics.runtime_probe() {
        rendered.push('\n');
        rendered.push_str(&format_turn_checkpoint_runtime_probe(session_id, probe));
    }
    rendered
}

#[cfg(test)]
pub(super) fn format_turn_checkpoint_startup_health(
    session_id: &str,
    diagnostics: &TurnCheckpointDiagnostics,
) -> Option<String> {
    let summary = diagnostics.summary();
    if !summary.checkpoint_durable {
        return None;
    }

    let render_labels = TurnCheckpointSummaryRenderLabels::from_summary(summary);
    let durability_labels = TurnCheckpointDurabilityRenderLabels::from_summary(summary);
    let recovery_labels =
        TurnCheckpointRecoveryRenderLabels::from_assessment(diagnostics.recovery());
    let recovery_needed = if summary.requires_recovery { 1 } else { 0 };

    Some(format!(
        "turn_checkpoint_health session={session_id} state={} reply_durable={} checkpoint_durable={} durability={} recovery_needed={recovery_needed} action={} source={} reason={} stage={} after_turn={} compaction={} lane={} result_kind={} persistence_mode={} safe_lane_route_decision={} safe_lane_route_reason={} safe_lane_route_source={} identity={}",
        render_labels.session_state,
        durability_labels.reply_durable,
        durability_labels.checkpoint_durable,
        durability_labels.durability,
        recovery_labels.action,
        recovery_labels.source,
        recovery_labels.reason,
        render_labels.stage,
        render_labels.after_turn,
        render_labels.compaction,
        render_labels.lane,
        render_labels.result_kind,
        render_labels.persistence_mode,
        render_labels.safe_lane_route_decision,
        render_labels.safe_lane_route_reason,
        render_labels.safe_lane_route_source,
        render_labels.identity,
    ))
}

#[cfg(test)]
pub(super) fn format_turn_checkpoint_repair(
    session_id: &str,
    outcome: &TurnCheckpointTailRepairOutcome,
) -> String {
    let after_turn = outcome.after_turn_status().unwrap_or("-");
    let compaction = outcome.compaction_status().unwrap_or("-");
    let render_labels = TurnCheckpointRecoveryRenderLabels::from_outcome(outcome);
    format!(
        "turn_checkpoint_repair session={session_id} status={} action={} source={} reason={} state={} checkpoints={} after_turn={after_turn} compaction={compaction}",
        outcome.status().as_str(),
        render_labels.action,
        render_labels.source,
        render_labels.reason,
        outcome.session_state().as_str(),
        outcome.checkpoint_events(),
    )
}

#[cfg(test)]
pub(super) fn format_turn_checkpoint_runtime_probe(
    session_id: &str,
    probe: &TurnCheckpointTailRepairRuntimeProbe,
) -> String {
    let render_labels = TurnCheckpointRecoveryRenderLabels::from_probe(probe);
    format!(
        "turn_checkpoint_probe session={session_id} action={} source={} reason={}",
        render_labels.action, render_labels.source, render_labels.reason,
    )
}

#[cfg(test)]
pub(super) async fn load_turn_checkpoint_summary_output(
    turn_coordinator: &ConversationTurnCoordinator,
    config: &LoongConfig,
    session_id: &str,
    limit: usize,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<String> {
    let runtime = DefaultConversationRuntime::from_config_or_env(config)?;
    let runtime_ref = &runtime;
    let diagnostics_future = turn_coordinator
        .load_turn_checkpoint_diagnostics_with_runtime_and_limit(
            config,
            session_id,
            limit,
            runtime_ref,
            binding,
        );
    let diagnostics = diagnostics_future.await?;

    Ok(format_turn_checkpoint_summary_output(
        session_id,
        limit,
        &diagnostics,
    ))
}
