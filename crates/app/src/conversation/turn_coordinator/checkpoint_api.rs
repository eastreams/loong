use super::*;
#[cfg(feature = "memory-sqlite")]
use crate::conversation::ContextCompactionDiagnostics;
#[cfg(feature = "memory-sqlite")]
use crate::conversation::compaction::compaction_policy_from_config;
#[cfg(feature = "memory-sqlite")]
use crate::conversation::compaction_preparation::prepare_compaction_window;
#[cfg(feature = "memory-sqlite")]
use crate::conversation::load_compaction_session_snapshot;

#[cfg(feature = "memory-sqlite")]
const COMPACTION_DIAGNOSTIC_TRANSCRIPT_PAGE_SIZE: usize = 256;

#[allow(dead_code)]
impl ConversationTurnCoordinator {
    pub async fn compact_production_session(
        &self,
        config: &LoongConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ContextCompactionReport> {
        let prepared = Self::build_default_runtime_with_production_binding(config, binding, None)?;
        let runtime = prepared.0;
        let production_binding = prepared.1;

        self.compact_session_with_runtime(config, session_id, &runtime, production_binding)
            .await
    }

    pub(crate) async fn compact_session_with_runtime<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongConfig,
        session_id: &str,
        runtime: &R,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ContextCompactionReport> {
        if let Some(kernel_ctx) = binding.kernel_context() {
            runtime.bootstrap(config, session_id, kernel_ctx).await?;
        }

        let session_context = runtime.session_context(config, session_id, binding)?;
        let tool_view = session_context.tool_view.clone();
        let before_messages = runtime
            .build_messages(config, session_id, true, &tool_view, binding)
            .await?;
        let estimated_tokens_before = estimate_tokens(&before_messages);
        let diagnostics = load_compaction_preparation_diagnostics(config, session_id, binding)
            .await
            .ok()
            .flatten();
        let compaction_outcome = maybe_compact_context(
            config,
            runtime,
            session_id,
            &before_messages,
            estimated_tokens_before,
            binding,
            true,
        )
        .await?;

        let mut status = compaction_outcome.checkpoint_status();
        let mut estimated_tokens_after = estimated_tokens_before;

        if compaction_outcome == ContextCompactionOutcome::Completed {
            match runtime
                .build_messages(config, session_id, true, &tool_view, binding)
                .await
            {
                Ok(after_messages) => {
                    let did_change = before_messages != after_messages;
                    let next_estimated_tokens = estimate_tokens(&after_messages);

                    estimated_tokens_after = next_estimated_tokens;

                    if !did_change {
                        status = TurnCheckpointProgressStatus::Skipped;
                    }
                }
                Err(_error) => {
                    status = TurnCheckpointProgressStatus::Skipped;
                    estimated_tokens_after = estimated_tokens_before;
                }
            }
        }

        let report = ContextCompactionReport {
            status: analytics_turn_checkpoint_progress_status(status),
            estimated_tokens_before,
            estimated_tokens_after,
            diagnostics,
        };

        Ok(report)
    }

    pub async fn repair_production_turn_checkpoint_tail(
        &self,
        config: &LoongConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<TurnCheckpointTailRepairOutcome> {
        let prepared = Self::build_default_runtime_with_production_binding(config, binding, None)?;
        let runtime = prepared.0;
        let production_binding = prepared.1;

        self.repair_turn_checkpoint_tail_with_runtime(
            config,
            session_id,
            &runtime,
            production_binding,
        )
        .await
    }

    pub(crate) async fn load_production_turn_checkpoint_diagnostics_with_limit(
        &self,
        config: &LoongConfig,
        session_id: &str,
        limit: usize,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<TurnCheckpointDiagnostics> {
        let prepared = Self::build_default_runtime_with_production_binding(config, binding, None)?;
        let runtime = prepared.0;
        let production_binding = prepared.1;
        self.load_turn_checkpoint_diagnostics_with_runtime_and_limit(
            config,
            session_id,
            limit,
            &runtime,
            production_binding,
        )
        .await
    }

    pub(crate) async fn repair_turn_checkpoint_tail_with_runtime<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongConfig,
        session_id: &str,
        runtime: &R,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<TurnCheckpointTailRepairOutcome> {
        #[cfg(feature = "memory-sqlite")]
        {
            let memory_config = store::session_store_config_from_memory_config(&config.memory);
            let Some(entry) = load_latest_turn_checkpoint_entry(
                session_id,
                config.memory.sliding_window,
                binding,
                &memory_config,
            )
            .await?
            else {
                return Ok(TurnCheckpointTailRepairOutcome::no_checkpoint());
            };

            repair_turn_checkpoint_tail_entry(config, runtime, session_id, &entry, binding).await
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (config, session_id, runtime, binding);
            Err("turn checkpoint repair unavailable: memory-sqlite feature disabled".to_owned())
        }
    }

    pub(crate) async fn load_turn_checkpoint_diagnostics_with_runtime_and_limit<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongConfig,
        session_id: &str,
        limit: usize,
        runtime: &R,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<TurnCheckpointDiagnostics> {
        #[cfg(feature = "memory-sqlite")]
        {
            let memory_config = store::session_store_config_from_memory_config(&config.memory);
            let (summary, latest_entry) =
                load_turn_checkpoint_history_snapshot(session_id, limit, binding, &memory_config)
                    .await?
                    .into_summary_and_latest_entry();
            let recovery = TurnCheckpointRecoveryAssessment::from_summary(&summary);
            let runtime_probe = match recovery.action() {
                TurnCheckpointRecoveryAction::None
                | TurnCheckpointRecoveryAction::InspectManually => None,
                TurnCheckpointRecoveryAction::RunAfterTurn
                | TurnCheckpointRecoveryAction::RunCompaction
                | TurnCheckpointRecoveryAction::RunAfterTurnAndCompaction => {
                    match latest_entry.as_ref() {
                        Some(entry) => {
                            probe_turn_checkpoint_tail_runtime_gate_entry(
                                config, runtime, session_id, entry, binding,
                            )
                            .await?
                        }
                        None => None,
                    }
                }
            };
            Ok(TurnCheckpointDiagnostics::new(
                summary,
                recovery,
                runtime_probe,
            ))
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (config, session_id, limit, runtime, binding);
            Err(
                "turn checkpoint diagnostics unavailable: memory-sqlite feature disabled"
                    .to_owned(),
            )
        }
    }

    pub(crate) async fn probe_turn_checkpoint_tail_runtime_gate_with_runtime_and_limit<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongConfig,
        session_id: &str,
        limit: usize,
        runtime: &R,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
        #[cfg(feature = "memory-sqlite")]
        {
            probe_turn_checkpoint_tail_runtime_gate_entry_with_limit(
                config, runtime, session_id, limit, binding,
            )
            .await
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (config, session_id, runtime, binding);
            Err(
                "turn checkpoint runtime probe unavailable: memory-sqlite feature disabled"
                    .to_owned(),
            )
        }
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) async fn load_compaction_preparation_diagnostics(
    config: &LoongConfig,
    session_id: &str,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<Option<ContextCompactionDiagnostics>> {
    let Some(compact_policy) = compaction_policy_from_config(config) else {
        return Ok(None);
    };
    let transcript_turns = match binding.kernel_context() {
        Some(kernel_ctx) => match load_compaction_session_snapshot(session_id, kernel_ctx).await {
            Ok(snapshot) if !snapshot.turns.is_empty() => snapshot.turns,
            Ok(_snapshot) => {
                load_compaction_preparation_turns_from_session_store(config, session_id)?
            }
            Err(_error) => {
                load_compaction_preparation_turns_from_session_store(config, session_id)?
            }
        },
        None => load_compaction_preparation_turns_from_session_store(config, session_id)?,
    };
    Ok(
        prepare_compaction_window(transcript_turns.as_slice(), compact_policy)
            .map(|preparation| preparation.diagnostics.into()),
    )
}

#[cfg(feature = "memory-sqlite")]
fn load_compaction_preparation_turns_from_session_store(
    config: &LoongConfig,
    session_id: &str,
) -> CliResult<Vec<crate::memory::WindowTurn>> {
    let memory_config = store::session_store_config_from_memory_config(&config.memory);
    store::transcript_session_turns_paged(
        session_id,
        COMPACTION_DIAGNOSTIC_TRANSCRIPT_PAGE_SIZE,
        &memory_config,
    )
    .map(|turns| {
        turns
            .into_iter()
            .map(|turn| crate::memory::WindowTurn {
                role: turn.role,
                content: turn.content,
                ts: Some(turn.ts),
            })
            .collect()
    })
}
