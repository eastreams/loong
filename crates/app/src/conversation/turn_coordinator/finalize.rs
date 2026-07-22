use super::*;

pub(super) async fn finalize_provider_turn_reply<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    ctx: &Context<'_>,
    user_input: &str,
    tail_phase: &ProviderTurnReplyTailPhase,
    usage: Option<Value>,
    checkpoint: &TurnCheckpointSnapshot,
) -> CliResult<ConversationTurnOutcome> {
    let session_id = ctx.session().session_id();
    if checkpoint.finalization.persistence_mode().is_none() {
        return Ok(ConversationTurnOutcome {
            reply: tail_phase.reply().to_owned(),
            usage,
        });
    }
    persist_reply_turns(runtime, user_input, tail_phase.reply(), ctx).await?;

    persist_turn_checkpoint_event(
        runtime,
        checkpoint,
        TurnCheckpointStage::PostPersist,
        TurnCheckpointFinalizationProgress::pending(checkpoint),
        None,
        ctx,
    )
    .await?;

    #[cfg(feature = "memory-sqlite")]
    if checkpoint_requires_verification_phase(checkpoint) {
        persist_task_progress_event_best_effort(
            config,
            session_id,
            "turn_verifying",
            verifying_task_progress_record(config, session_id, user_input),
        );
    }

    let after_turn_status = if checkpoint.finalization.runs_after_turn() {
        if matches!(
            ctx.session().session_mode,
            GovernedSessionMode::MutatingCapable
        ) && ctx.allowed_capabilities().contains(Capability::MemoryWrite)
        {
            match runtime
                .after_turn(
                    user_input,
                    tail_phase.reply(),
                    tail_phase.after_turn_messages(),
                    ctx,
                )
                .await
            {
                Ok(()) => TurnCheckpointProgressStatus::Completed,
                Err(error) => {
                    persist_turn_checkpoint_event(
                        runtime,
                        checkpoint,
                        TurnCheckpointStage::FinalizationFailed,
                        TurnCheckpointFinalizationProgress {
                            after_turn: TurnCheckpointProgressStatus::Failed,
                            compaction: TurnCheckpointProgressStatus::Skipped,
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
        } else {
            TurnCheckpointProgressStatus::Skipped
        }
    } else {
        TurnCheckpointProgressStatus::Skipped
    };
    let compaction_status = if checkpoint.finalization.attempts_context_compaction() {
        match maybe_compact_context(
            config,
            runtime,
            ctx,
            tail_phase.after_turn_messages(),
            tail_phase.estimated_tokens(),
            tail_phase.runtime_self_continuity(),
            false,
        )
        .await
        {
            Ok(outcome) => outcome.checkpoint_status(),
            Err(error) => {
                persist_turn_checkpoint_event(
                    runtime,
                    checkpoint,
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
    } else {
        TurnCheckpointProgressStatus::Skipped
    };
    persist_turn_checkpoint_event(
        runtime,
        checkpoint,
        TurnCheckpointStage::Finalized,
        TurnCheckpointFinalizationProgress {
            after_turn: after_turn_status,
            compaction: compaction_status,
        },
        None,
        ctx,
    )
    .await?;

    #[cfg(feature = "memory-sqlite")]
    persist_task_progress_event_best_effort(
        config,
        session_id,
        if checkpoint_waits_for_external_resolution(checkpoint) {
            "turn_waiting"
        } else {
            "turn_completed"
        },
        if checkpoint_waits_for_external_resolution(checkpoint) {
            waiting_task_progress_record(config, session_id, user_input)
        } else {
            completed_task_progress_record(config, session_id, user_input)
        },
    );

    Ok(ConversationTurnOutcome {
        reply: tail_phase.reply().to_owned(),
        usage,
    })
}

pub(super) async fn persist_resolved_provider_error_checkpoint<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    checkpoint: &TurnCheckpointSnapshot,
    ctx: &Context<'_>,
) -> CliResult<()> {
    persist_turn_checkpoint_event(
        runtime,
        checkpoint,
        TurnCheckpointStage::Finalized,
        TurnCheckpointFinalizationProgress::pending(checkpoint),
        None,
        ctx,
    )
    .await
}

pub(super) async fn apply_resolved_provider_turn<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    ctx: &Context<'_>,
    user_input: &str,
    preparation: &ProviderTurnPreparation,
    resolved: &ResolvedProviderTurn,
    observer: Option<&ConversationTurnObserverHandle>,
) -> CliResult<ConversationTurnOutcome> {
    if let Some(error_text) = resolved.provider_error_text() {
        emit_provider_failover_trust_event_if_needed(config, runtime, error_text, ctx).await;
    }
    let terminal_phase = resolved.terminal_phase(&preparation.session);
    let completion_event = match &terminal_phase {
        ProviderTurnTerminalPhase::PersistReply(phase) => {
            let phase = phase.as_ref();
            let message_count = phase.tail_phase.after_turn_messages().len();
            let estimated_tokens = phase.tail_phase.estimated_tokens();
            let finalizing_event =
                ConversationTurnPhaseEvent::finalizing_reply(message_count, estimated_tokens);
            observe_turn_phase(observer, finalizing_event);
            Some(ConversationTurnPhaseEvent::completed(
                message_count,
                estimated_tokens,
            ))
        }
        ProviderTurnTerminalPhase::ReturnError(_) => None,
    };
    let apply_result = terminal_phase.apply(config, runtime, ctx, user_input).await;

    let completion_observation = match (completion_event, apply_result.is_ok()) {
        (Some(event), true) => Some(event),
        (Some(_), false) | (None, true) | (None, false) => None,
    };

    if let Some(event) = completion_observation {
        observe_turn_phase(observer, event);
    }

    apply_result
}
