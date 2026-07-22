use super::*;

#[allow(dead_code)]
impl ConversationTurnCoordinator {
    pub(crate) async fn handle_turn_with_runtime_and_address_and_ingress_and_observer_outcome<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongConfig,
        ctx: &Context<'_>,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        legacy_tools: &DefaultLegacyToolDispatcher,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ConversationTurnOutcome> {
        let session_id = ctx.session().session_id();
        let turn_result: CliResult<(ConversationTurnOutcome, bool)> = async {
            #[cfg(feature = "memory-sqlite")]
            if let Some(reply) = self
                .maybe_handle_pending_approval_control_turn(
                    config,
                    ctx,
                    runtime,
                    user_input,
                    error_mode,
                    legacy_tools,
                    observer.as_ref(),
                )
                .await?
            {
                return Ok((ConversationTurnOutcome { reply, usage: None }, false));
            }
            if let Some(reply) = self
                .maybe_handle_explicit_skill_activation_control_turn(
                    config,
                    ctx,
                    runtime,
                    user_input,
                    error_mode,
                    legacy_tools,
                    observer.as_ref(),
                )
                .await?
            {
                return Ok((reply, false));
            }
            #[cfg(feature = "memory-sqlite")]
            persist_task_progress_event_best_effort(
                config,
                session_id,
                "turn_started",
                active_task_progress_record(config, session_id, user_input),
            );
            let preparing_event = ConversationTurnPhaseEvent::preparing();
            observe_turn_phase(observer.as_ref(), preparing_event);

            runtime.bootstrap(config, ctx).await?;

            let visible_ingress = ingress.filter(|value| value.has_contextual_hints());
            emit_turn_ingress_event(runtime, visible_ingress, ctx).await;

            let turn_id = next_conversation_turn_id();
            let assembled_context = runtime.build_context(config, ctx, true).await?;
            let preparation = ProviderTurnPreparation::from_assembled_context_with_turn_id(
                config,
                assembled_context,
                user_input,
                turn_id.as_str(),
                visible_ingress,
            );
            let context_message_count = preparation.session.messages.len();
            let context_estimated_tokens = preparation.session.estimated_tokens;
            let initial_request_event = ConversationTurnPhaseEvent::requesting_provider(
                1,
                context_message_count,
                context_estimated_tokens,
            );
            observe_turn_phase(
                observer.as_ref(),
                ConversationTurnPhaseEvent::context_ready(
                    context_message_count,
                    context_estimated_tokens,
                ),
            );
            observe_turn_phase(observer.as_ref(), initial_request_event);
            emit_prompt_frame_event(
                runtime,
                1,
                "initial",
                preparation.session.prompt_frame_summary(),
                ctx,
            )
            .await;

            let provider_turn_result = request_provider_turn_with_observer(
                config,
                runtime,
                preparation.turn_id.as_str(),
                &preparation.session.messages,
                ctx,
                observer.as_ref(),
                retry_progress.clone(),
            )
            .await;
            let resolved_turn = resolve_provider_turn(
                config,
                runtime,
                ctx,
                user_input,
                &preparation,
                provider_turn_result,
                error_mode,
                legacy_tools,
                ingress,
                observer.as_ref(),
                retry_progress,
            )
            .await;

            apply_resolved_provider_turn(
                config,
                runtime,
                ctx,
                user_input,
                &preparation,
                &resolved_turn,
                observer.as_ref(),
            )
            .await
            .map(|reply| (reply, false))
        }
        .await;

        match turn_result {
            Ok((outcome, true)) => {
                observe_non_provider_turn_terminal_success_phases(observer.as_ref());
                Ok(outcome)
            }
            Ok((outcome, false)) => Ok(outcome),
            Err(error) => {
                let failed_event = ConversationTurnPhaseEvent::failed();
                observe_turn_phase(observer.as_ref(), failed_event);
                #[cfg(feature = "memory-sqlite")]
                persist_task_progress_event_best_effort(
                    config,
                    session_id,
                    "turn_failed",
                    failed_task_progress_record(config, session_id, user_input),
                );
                Err(error)
            }
        }
    }
}
