use super::*;

#[allow(dead_code)]
impl ConversationTurnCoordinator {
    pub async fn compact_production_session(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
    ) -> CliResult<ContextCompactionReport> {
        let runtime = DefaultConversationRuntime::from_config_or_env(config)?;

        self.compact_session_with_runtime(config, ctx, &runtime)
            .await
    }

    pub(crate) async fn compact_session_with_runtime<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        runtime: &R,
    ) -> CliResult<ContextCompactionReport> {
        runtime.bootstrap(config, ctx).await?;

        let before_messages = runtime.build_messages(config, ctx, true).await?;
        let estimated_tokens_before = estimate_tokens(&before_messages);
        let compaction_outcome = maybe_compact_context(
            config,
            runtime,
            ctx,
            &before_messages,
            estimated_tokens_before,
            None,
            true,
        )
        .await?;

        let mut status = compaction_outcome.checkpoint_status();
        let mut estimated_tokens_after = estimated_tokens_before;

        if compaction_outcome == ContextCompactionOutcome::Completed {
            match runtime.build_messages(config, ctx, true).await {
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
        };

        Ok(report)
    }

    pub async fn repair_production_turn_checkpoint_tail(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
    ) -> CliResult<TurnCheckpointTailRepairOutcome> {
        let runtime = DefaultConversationRuntime::from_config_or_env(config)?;

        self.repair_turn_checkpoint_tail_with_runtime(config, ctx, &runtime)
            .await
    }

    pub(crate) async fn load_production_turn_checkpoint_diagnostics_with_limit(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        limit: usize,
    ) -> CliResult<TurnCheckpointDiagnostics> {
        let runtime = DefaultConversationRuntime::from_config_or_env(config)?;
        self.load_turn_checkpoint_diagnostics_with_runtime_and_limit(config, ctx, limit, &runtime)
            .await
    }

    pub(crate) async fn repair_turn_checkpoint_tail_with_runtime<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        runtime: &R,
    ) -> CliResult<TurnCheckpointTailRepairOutcome> {
        #[cfg(feature = "memory-sqlite")]
        {
            let Some(entry) =
                load_latest_turn_checkpoint_entry(config.memory.sliding_window, ctx, runtime)
                    .await?
            else {
                return Ok(TurnCheckpointTailRepairOutcome::no_checkpoint());
            };

            repair_turn_checkpoint_tail_entry(config, ctx, runtime, &entry).await
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (config, ctx, runtime);
            Err("turn checkpoint repair unavailable: memory-sqlite feature disabled".to_owned())
        }
    }

    pub(crate) async fn load_turn_checkpoint_diagnostics_with_runtime_and_limit<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        limit: usize,
        runtime: &R,
    ) -> CliResult<TurnCheckpointDiagnostics> {
        #[cfg(feature = "memory-sqlite")]
        {
            let (summary, latest_entry) =
                load_turn_checkpoint_history_snapshot(limit, ctx, runtime)
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
                                config, ctx, runtime, entry,
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
            let _ = (config, ctx, limit, runtime);
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
        ctx: &Context<'_>,
        limit: usize,
        runtime: &R,
    ) -> CliResult<Option<TurnCheckpointTailRepairRuntimeProbe>> {
        #[cfg(feature = "memory-sqlite")]
        {
            probe_turn_checkpoint_tail_runtime_gate_entry_with_limit(config, ctx, runtime, limit)
                .await
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (config, ctx, runtime);
            Err(
                "turn checkpoint runtime probe unavailable: memory-sqlite feature disabled"
                    .to_owned(),
            )
        }
    }
}
