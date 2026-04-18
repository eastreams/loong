use super::*;

pub(super) async fn finalize_provider_turn_reply<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_id: &str,
    user_input: &str,
    tail_phase: &ProviderTurnReplyTailPhase,
    checkpoint: &TurnCheckpointSnapshot,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<String> {
    let Some(persistence_mode) = checkpoint.finalization.persistence_mode() else {
        return Ok(tail_phase.reply().to_owned());
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

    persist_turn_checkpoint_event(
        runtime,
        session_id,
        checkpoint,
        TurnCheckpointStage::PostPersist,
        TurnCheckpointFinalizationProgress::pending(checkpoint),
        None,
        binding,
    )
    .await?;

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
                    persist_turn_checkpoint_event(
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
                persist_turn_checkpoint_event(
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
                    binding,
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
        session_id,
        checkpoint,
        TurnCheckpointStage::Finalized,
        TurnCheckpointFinalizationProgress {
            after_turn: after_turn_status,
            compaction: compaction_status,
        },
        None,
        binding,
    )
    .await?;
    Ok(tail_phase.reply().to_owned())
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
) -> CliResult<String> {
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

pub(super) fn effective_tool_config_for_session(
    tool_config: &crate::config::ToolConfig,
    session_context: &SessionContext,
) -> crate::config::ToolConfig {
    let mut tool_config = tool_config.clone();
    if session_context.parent_session_id.is_some() {
        tool_config.sessions.visibility = crate::config::SessionVisibility::SelfOnly;
    }
    tool_config
}

pub(super) struct CoordinatorAppToolDispatcher<'a, R: ?Sized> {
    pub(super) config: &'a LoongConfig,
    pub(super) runtime: &'a R,
    pub(super) fallback: &'a DefaultAppToolDispatcher,
}

#[async_trait]
impl<R> AppToolDispatcher for CoordinatorAppToolDispatcher<'_, R>
where
    R: ConversationRuntime + ?Sized,
{
    async fn preflight_tool_intent_with_binding(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
        budget_state: &crate::conversation::autonomy_policy::AutonomyTurnBudgetState,
    ) -> Result<crate::conversation::turn_engine::ToolPreflightOutcome, String> {
        self.fallback
            .preflight_tool_intent_with_binding(
                session_context,
                intent,
                descriptor,
                binding,
                budget_state,
            )
            .await
    }

    async fn maybe_require_approval_with_binding(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<Option<crate::conversation::turn_engine::ApprovalRequirement>, String> {
        self.fallback
            .maybe_require_approval_with_binding(session_context, intent, descriptor, binding)
            .await
    }

    async fn preflight_tool_execution_with_binding(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        request: loong_contracts::ToolCoreRequest,
        descriptor: &crate::tools::ToolDescriptor,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<ToolExecutionPreflight, String> {
        self.fallback
            .preflight_tool_execution_with_binding(
                session_context,
                intent,
                request,
                descriptor,
                binding,
            )
            .await
    }

    async fn execute_app_tool(
        &self,
        session_context: &SessionContext,
        request: loong_contracts::ToolCoreRequest,
        binding: ConversationRuntimeBinding<'_>,
    ) -> Result<loong_contracts::ToolCoreOutcome, String> {
        match crate::tools::canonical_tool_name(request.tool_name.as_str()) {
            "approval_request_resolve" => {
                #[cfg(not(feature = "memory-sqlite"))]
                {
                    let _ = (session_context, binding);
                    Err("approval tools require sqlite memory support (enable feature `memory-sqlite`)"
                        .to_owned())
                }

                #[cfg(feature = "memory-sqlite")]
                {
                    let memory_config =
                        MemoryRuntimeConfig::from_memory_config(&self.config.memory);
                    let effective_tool_config =
                        effective_tool_config_for_session(&self.config.tools, session_context);
                    let approval_runtime = CoordinatorApprovalResolutionRuntime::new(
                        self.config,
                        self.runtime,
                        self.fallback,
                        binding,
                    );
                    crate::tools::approval::execute_approval_tool_with_runtime_support(
                        request,
                        &session_context.session_id,
                        &memory_config,
                        &effective_tool_config,
                        Some(&approval_runtime),
                    )
                    .await
                }
            }
            "delegate" => {
                execute_delegate_tool(
                    self.config,
                    self.runtime,
                    session_context,
                    request.payload,
                    binding,
                )
                .await
            }
            "delegate_async" => {
                execute_delegate_async_tool(
                    self.config,
                    self.runtime,
                    session_context,
                    request.payload,
                    binding,
                )
                .await
            }
            _ => {
                self.fallback
                    .execute_app_tool(session_context, request, binding)
                    .await
            }
        }
    }

    async fn after_tool_execution(
        &self,
        session_context: &SessionContext,
        intent: &ToolIntent,
        intent_sequence: usize,
        request: &loong_contracts::ToolCoreRequest,
        outcome: &loong_contracts::ToolCoreOutcome,
        binding: ConversationRuntimeBinding<'_>,
    ) {
        let tool_name = crate::tools::canonical_tool_name(request.tool_name.as_str());

        persist_tool_discovery_refresh_event_if_needed(
            self.runtime,
            &session_context.session_id,
            intent,
            intent_sequence,
            tool_name,
            outcome,
            binding,
        )
        .await;
    }
}

pub(super) async fn persist_tool_discovery_refresh_event_if_needed<
    R: ConversationRuntime + ?Sized,
>(
    runtime: &R,
    session_id: &str,
    intent: &ToolIntent,
    intent_sequence: usize,
    tool_name: &str,
    outcome: &loong_contracts::ToolCoreOutcome,
    binding: ConversationRuntimeBinding<'_>,
) {
    if tool_name != "tool.search" {
        return;
    }

    if outcome.status != "ok" {
        return;
    }

    let Some(discovery_state) = ToolDiscoveryState::from_tool_search_payload(&outcome.payload)
    else {
        return;
    };
    let Some(discovery_payload) =
        build_tool_discovery_refresh_event_payload(discovery_state, intent, intent_sequence)
    else {
        return;
    };
    let persist_result = persist_conversation_event(
        runtime,
        session_id,
        TOOL_DISCOVERY_REFRESHED_EVENT_NAME,
        discovery_payload,
        binding,
    )
    .await;

    if persist_result.is_ok() {
        return;
    }

    let Some(ctx) = binding.kernel_context() else {
        return;
    };

    let _ = ctx.kernel.record_audit_event(
        Some(ctx.agent_id()),
        AuditEventKind::PlaneInvoked {
            pack_id: ctx.pack_id().to_owned(),
            plane: ExecutionPlane::Runtime,
            tier: PlaneTier::Core,
            primary_adapter: "conversation.runtime".to_owned(),
            delegated_core_adapter: None,
            operation: "conversation.runtime.tool_discovery_persist_failed".to_owned(),
            required_capabilities: Vec::new(),
        },
    );
}

pub(super) fn build_tool_discovery_refresh_event_payload(
    discovery_state: ToolDiscoveryState,
    intent: &ToolIntent,
    intent_sequence: usize,
) -> Option<Value> {
    let discovery_payload = serde_json::to_value(discovery_state).ok()?;
    let Value::Object(mut discovery_payload) = discovery_payload else {
        return None;
    };
    let turn_id = intent.turn_id.trim();
    let tool_call_id = intent.tool_call_id.trim();

    if !turn_id.is_empty() {
        discovery_payload.insert("turn_id".to_owned(), Value::String(turn_id.to_owned()));
    }

    if !tool_call_id.is_empty() {
        discovery_payload.insert(
            "tool_call_id".to_owned(),
            Value::String(tool_call_id.to_owned()),
        );
    }

    discovery_payload.insert("intent_sequence".to_owned(), json!(intent_sequence));

    Some(Value::Object(discovery_payload))
}
