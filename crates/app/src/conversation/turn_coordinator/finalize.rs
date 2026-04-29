use super::checkpoint_api::load_compaction_preparation_diagnostics;
use super::*;

pub(super) async fn finalize_provider_turn_reply<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_id: &str,
    user_input: &str,
    tail_phase: &ProviderTurnReplyTailPhase,
    usage: Option<Value>,
    checkpoint: &TurnCheckpointSnapshot,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<ConversationTurnOutcome> {
    let Some(persistence_mode) = checkpoint.finalization.persistence_mode() else {
        return Ok(ConversationTurnOutcome {
            reply: tail_phase.reply().to_owned(),
            usage,
        });
    };
    persist_reply_turns_with_mode(
        runtime,
        session_id,
        user_input,
        tail_phase.reply(),
        persistence_mode,
        binding,
    )
    .await?;

    let compaction_diagnostics = if checkpoint.finalization.attempts_context_compaction() {
        load_compaction_preparation_diagnostics(config, session_id, binding)
            .await
            .ok()
            .flatten()
    } else {
        None
    };

    persist_turn_checkpoint_event_with_compaction_diagnostics(
        runtime,
        session_id,
        checkpoint,
        TurnCheckpointStage::PostPersist,
        TurnCheckpointFinalizationProgress::pending(checkpoint),
        None,
        compaction_diagnostics.as_ref(),
        binding,
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
        if let Some(kernel_ctx) = binding.kernel_context() {
            match runtime
                .after_turn(
                    session_id,
                    user_input,
                    tail_phase.reply(),
                    tail_phase.after_turn_messages(),
                    kernel_ctx,
                )
                .await
            {
                Ok(()) => TurnCheckpointProgressStatus::Completed,
                Err(error) => {
                    persist_turn_checkpoint_event_with_compaction_diagnostics(
                        runtime,
                        session_id,
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
                        compaction_diagnostics.as_ref(),
                        binding,
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
            session_id,
            tail_phase.after_turn_messages(),
            tail_phase.estimated_tokens(),
            binding,
            false,
        )
        .await
        {
            Ok(outcome) => outcome.checkpoint_status(),
            Err(error) => {
                persist_turn_checkpoint_event_with_compaction_diagnostics(
                    runtime,
                    session_id,
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
                    compaction_diagnostics.as_ref(),
                    binding,
                )
                .await?;
                return Err(error);
            }
        }
    } else {
        TurnCheckpointProgressStatus::Skipped
    };
    persist_turn_checkpoint_event_with_compaction_diagnostics(
        runtime,
        session_id,
        checkpoint,
        TurnCheckpointStage::Finalized,
        TurnCheckpointFinalizationProgress {
            after_turn: after_turn_status,
            compaction: compaction_status,
        },
        None,
        compaction_diagnostics.as_ref(),
        binding,
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
    session_id: &str,
    checkpoint: &TurnCheckpointSnapshot,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    persist_turn_checkpoint_event(
        runtime,
        session_id,
        checkpoint,
        TurnCheckpointStage::Finalized,
        TurnCheckpointFinalizationProgress::pending(checkpoint),
        None,
        binding,
    )
    .await
}

pub(super) async fn apply_resolved_provider_turn<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_id: &str,
    user_input: &str,
    preparation: &ProviderTurnPreparation,
    resolved: &ResolvedProviderTurn,
    binding: ConversationRuntimeBinding<'_>,
    observer: Option<&ConversationTurnObserverHandle>,
) -> CliResult<ConversationTurnOutcome> {
    if let Some(error_text) = resolved.provider_error_text() {
        emit_provider_failover_trust_event_if_needed(
            config, runtime, session_id, error_text, binding,
        )
        .await;
    }
    let terminal_phase = resolved.terminal_phase(&preparation.session);
    let completion_event = match &terminal_phase {
        ProviderTurnTerminalPhase::PersistReply(phase) => {
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
    let apply_result = terminal_phase
        .apply(config, runtime, session_id, user_input, binding)
        .await;

    let completion_observation = match (completion_event, apply_result.is_ok()) {
        (Some(event), true) => Some(event),
        (Some(_), false) | (None, true) | (None, false) => None,
    };

    if let Some(event) = completion_observation {
        observe_turn_phase(observer, event);
    }

    apply_result
}
