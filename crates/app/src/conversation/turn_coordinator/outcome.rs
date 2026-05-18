use super::*;

#[allow(dead_code)]
impl ConversationTurnCoordinator {
    pub(crate) async fn handle_turn_with_runtime_and_address_and_ingress_and_observer_outcome<
        R: ConversationRuntime + ?Sized,
    >(
        &self,
        config: &LoongConfig,
        address: &ConversationSessionAddress,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<ConversationTurnObserverHandle>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ConversationTurnOutcome> {
        let turn_result: CliResult<(ConversationTurnOutcome, bool)> = async {
            let session_id = address.session_id.as_str();
            #[cfg(feature = "memory-sqlite")]
            if let Some(reply) = self
                .maybe_handle_pending_approval_control_turn(
                    config,
                    runtime,
                    session_id,
                    user_input,
                    error_mode,
                    binding,
                    observer.as_ref(),
                )
                .await?
            {
                return Ok((ConversationTurnOutcome { reply, usage: None }, false));
            }
            if let Some(reply) = self
                .maybe_handle_explicit_skill_activation_control_turn(
                    config,
                    runtime,
                    session_id,
                    user_input,
                    error_mode,
                    binding,
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

            if let Some(kernel_ctx) = binding.kernel_context() {
                runtime.bootstrap(config, session_id, kernel_ctx).await?;
            }

            let session_context = runtime.session_context(config, session_id, binding)?;
            let tool_view = session_context.tool_view.clone();
            let visible_ingress = ingress.filter(|value| value.has_contextual_hints());
            emit_turn_ingress_event(runtime, session_id, visible_ingress, binding).await;

            let turn_id = next_conversation_turn_id();
            let assembled_context = runtime
                .build_context(config, session_id, true, binding)
                .await?;
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
                session_id,
                1,
                "initial",
                preparation.session.prompt_frame_summary(),
                binding,
            )
            .await;

            let provider_turn_result = request_provider_turn_with_observer(
                config,
                runtime,
                session_id,
                preparation.turn_id.as_str(),
                &preparation.session.messages,
                &tool_view,
                binding,
                observer.as_ref(),
                retry_progress.clone(),
            )
            .await;
            let resolved_turn = resolve_provider_turn(
                config,
                runtime,
                session_id,
                user_input,
                &preparation,
                provider_turn_result,
                error_mode,
                binding,
                ingress,
                observer.as_ref(),
                retry_progress,
            )
            .await;

            apply_resolved_provider_turn(
                config,
                runtime,
                session_id,
                user_input,
                &preparation,
                &resolved_turn,
                binding,
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
                    address.session_id.as_str(),
                    "turn_failed",
                    failed_task_progress_record(config, address.session_id.as_str(), user_input),
                );
                Err(error)
            }
        }
    }
}
