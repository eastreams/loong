use super::*;
use crate::conversation::session_history;

#[cfg(feature = "memory-sqlite")]
pub(super) async fn repair_turn_checkpoint_tail_entry<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    ctx: &Context<'_>,
    runtime: &R,
    entry: &session_history::TurnCheckpointLatestEntry,
) -> CliResult<TurnCheckpointTailRepairOutcome> {
    let summary = &entry.summary;
    let (action, repair_plan, resume_input) =
        match load_turn_checkpoint_tail_runtime_eligibility(config, runtime, ctx, entry).await? {
            TurnCheckpointTailRuntimeEligibility::NotNeeded { action, reason } => {
                return Ok(TurnCheckpointTailRepairOutcome::from_summary(
                    TurnCheckpointTailRepairStatus::NotNeeded,
                    action,
                    Some(TurnCheckpointTailRepairSource::Summary),
                    reason,
                    summary,
                ));
            }
            TurnCheckpointTailRuntimeEligibility::Manual {
                action,
                reason,
                source,
            } => {
                return Ok(TurnCheckpointTailRepairOutcome::from_summary(
                    TurnCheckpointTailRepairStatus::ManualRequired,
                    action,
                    Some(source),
                    reason,
                    summary,
                ));
            }
            TurnCheckpointTailRuntimeEligibility::Runnable {
                action,
                plan,
                resume_input,
            } => (action, plan, resume_input),
        };

    let mut after_turn_status =
        restore_analytics_turn_checkpoint_progress_status(repair_plan.after_turn_status());
    let mut compaction_status =
        restore_analytics_turn_checkpoint_progress_status(repair_plan.compaction_status());

    if repair_plan.should_run_after_turn() {
        if !matches!(
            ctx.session().session_mode,
            GovernedSessionMode::MutatingCapable
        ) || !ctx.allowed_capabilities().contains(Capability::MemoryWrite)
        {
            after_turn_status = TurnCheckpointProgressStatus::Skipped;
            if repair_plan.should_run_compaction() {
                compaction_status = TurnCheckpointProgressStatus::Skipped;
            }
            persist_turn_checkpoint_event_value(
                runtime,
                &entry.checkpoint,
                TurnCheckpointStage::Finalized,
                TurnCheckpointFinalizationProgress {
                    after_turn: after_turn_status,
                    compaction: compaction_status,
                },
                None,
                ctx,
            )
            .await?;
            return Ok(TurnCheckpointTailRepairOutcome::repaired(
                action,
                summary,
                after_turn_status,
                compaction_status,
            ));
        }
        match runtime
            .after_turn(
                resume_input.user_input(),
                resume_input.assistant_reply(),
                resume_input.messages(),
                ctx,
            )
            .await
        {
            Ok(()) => {
                after_turn_status = TurnCheckpointProgressStatus::Completed;
            }
            Err(error) => {
                persist_turn_checkpoint_event_value(
                    runtime,
                    &entry.checkpoint,
                    TurnCheckpointStage::FinalizationFailed,
                    TurnCheckpointFinalizationProgress {
                        after_turn: TurnCheckpointProgressStatus::Failed,
                        compaction: if repair_plan.should_run_compaction() {
                            TurnCheckpointProgressStatus::Skipped
                        } else {
                            compaction_status
                        },
                    },
                    Some(TurnCheckpointFailure {
                        step: TurnCheckpointFailureStep::AfterTurn,
                        error: error.clone(),
                    }),
                    ctx,
                )
                .await?;
                return Err(error);
            }
        }
    }

    if repair_plan.should_run_compaction() {
        match maybe_compact_context(
            config,
            runtime,
            ctx,
            resume_input.messages(),
            resume_input.estimated_tokens(),
            None,
            false,
        )
        .await
        {
            Ok(outcome) => {
                compaction_status = outcome.checkpoint_status();
            }
            Err(error) => {
                persist_turn_checkpoint_event_value(
                    runtime,
                    &entry.checkpoint,
                    TurnCheckpointStage::FinalizationFailed,
                    TurnCheckpointFinalizationProgress {
                        after_turn: after_turn_status,
                        compaction: TurnCheckpointProgressStatus::Failed,
                    },
                    Some(TurnCheckpointFailure {
                        step: TurnCheckpointFailureStep::Compaction,
                        error: error.clone(),
                    }),
                    ctx,
                )
                .await?;
                return Err(error);
            }
        }
    }

    persist_turn_checkpoint_event_value(
        runtime,
        &entry.checkpoint,
        TurnCheckpointStage::Finalized,
        TurnCheckpointFinalizationProgress {
            after_turn: after_turn_status,
            compaction: compaction_status,
        },
        None,
        ctx,
    )
    .await?;

    Ok(TurnCheckpointTailRepairOutcome::repaired(
        action,
        summary,
        after_turn_status,
        compaction_status,
    ))
}

#[cfg(feature = "memory-sqlite")]
pub(super) async fn probe_turn_checkpoint_tail_runtime_gate_entry<
    R: ConversationRuntime + ?Sized,
>(
    config: &LoongConfig,
    ctx: &Context<'_>,
    runtime: &R,
    entry: &session_history::TurnCheckpointLatestEntry,
) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
    match load_turn_checkpoint_tail_runtime_eligibility(config, runtime, ctx, entry).await? {
        TurnCheckpointTailRuntimeEligibility::Manual {
            action,
            reason,
            source: TurnCheckpointTailRepairSource::Runtime,
        } => Ok(Some(TurnCheckpointTailRepairRuntimeProbe::new(
            action,
            TurnCheckpointTailRepairSource::Runtime,
            reason,
        ))),
        TurnCheckpointTailRuntimeEligibility::NotNeeded { .. }
        | TurnCheckpointTailRuntimeEligibility::Manual { .. }
        | TurnCheckpointTailRuntimeEligibility::Runnable { .. } => Ok(None),
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) async fn load_turn_checkpoint_tail_runtime_eligibility<
    R: ConversationRuntime + ?Sized,
>(
    config: &LoongConfig,
    runtime: &R,
    session_context: &Context<'_>,
    entry: &session_history::TurnCheckpointLatestEntry,
) -> CliResult<TurnCheckpointTailRuntimeEligibility> {
    let session_id = session_context.session().session_id.as_str();
    let summary = &entry.summary;
    let recovery = TurnCheckpointRecoveryAssessment::from_summary(summary);
    let action = recovery.action();
    if matches!(action, TurnCheckpointRecoveryAction::None) {
        return Ok(TurnCheckpointTailRuntimeEligibility::NotNeeded {
            action,
            reason: TurnCheckpointTailRepairReason::NotNeeded,
        });
    }
    if matches!(action, TurnCheckpointRecoveryAction::InspectManually) {
        return Ok(TurnCheckpointTailRuntimeEligibility::Manual {
            action,
            reason: recovery
                .reason()
                .unwrap_or(TurnCheckpointTailRepairReason::CheckpointStateRequiresManualInspection),
            source: recovery.source(),
        });
    }

    let repair_plan = build_turn_checkpoint_repair_plan(summary);
    let assembled = match runtime.build_context(config, session_context, true).await {
        Ok(assembled) => assembled,
        Err(error) => {
            tracing::warn!(
                target: "loong.conversation",
                session_id = %session_id,
                error = %error,
                "failed to assemble runtime context for turn checkpoint tail repair; degrading to manual inspection"
            );
            return Ok(TurnCheckpointTailRuntimeEligibility::Manual {
                action: TurnCheckpointRecoveryAction::InspectManually,
                reason: TurnCheckpointTailRepairReason::CheckpointStateRequiresManualInspection,
                source: TurnCheckpointTailRepairSource::Runtime,
            });
        }
    };
    match TurnCheckpointRepairResumeInput::from_assembled_context(assembled, &entry.checkpoint) {
        Ok(resume_input) => Ok(TurnCheckpointTailRuntimeEligibility::Runnable {
            action,
            plan: repair_plan,
            resume_input,
        }),
        Err(reason) => Ok(TurnCheckpointTailRuntimeEligibility::Manual {
            action: TurnCheckpointRecoveryAction::InspectManually,
            reason,
            source: TurnCheckpointTailRepairSource::Runtime,
        }),
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) async fn probe_turn_checkpoint_tail_runtime_gate_entry_with_limit<
    R: ConversationRuntime + ?Sized,
>(
    config: &LoongConfig,
    ctx: &Context<'_>,
    runtime: &R,
    limit: usize,
) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
    let Some(entry) = load_latest_turn_checkpoint_entry(limit, ctx, runtime).await? else {
        return Ok(None);
    };
    probe_turn_checkpoint_tail_runtime_gate_entry(config, ctx, runtime, &entry).await
}
