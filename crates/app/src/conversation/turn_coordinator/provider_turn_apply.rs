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

#[cfg(feature = "memory-sqlite")]
pub(super) fn persist_task_progress_event_best_effort(
    config: &LoongConfig,
    session_id: &str,
    source: &str,
    task_progress: TaskProgressRecord,
) {
    let memory_config = store::session_store_config_from_memory_config(&config.memory);
    let Ok(repo) = SessionRepository::new(&memory_config) else {
        return;
    };

    let _ = repo.append_event(NewSessionEvent {
        session_id: session_id.to_owned(),
        event_kind: TASK_PROGRESS_EVENT_KIND.to_owned(),
        actor_session_id: Some(session_id.to_owned()),
        payload_json: task_progress_event_payload(source, &task_progress),
    });
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn resolve_canonical_task_id(config: &LoongConfig, session_id: &str) -> String {
    let memory_config = store::session_store_config_from_memory_config(&config.memory);
    let Ok(repo) = SessionRepository::new(&memory_config) else {
        return session_id.to_owned();
    };

    resolve_canonical_task_id_for_session(&repo, session_id)
        .unwrap_or_else(|| session_id.to_owned())
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn active_task_progress_record(
    config: &LoongConfig,
    session_id: &str,
    user_input: &str,
) -> TaskProgressRecord {
    let updated_at = unix_ts_now();
    let task_id = resolve_canonical_task_id(config, session_id);

    TaskProgressRecord {
        task_id,
        owner_kind: "conversation_turn".to_owned(),
        status: TaskProgressStatus::Active,
        intent_summary: summarize_task_progress_intent(user_input),
        verification_state: Some(TaskVerificationState::NotStarted),
        active_handles: vec![TaskActiveHandleRecord {
            handle_kind: "conversation_turn".to_owned(),
            handle_id: session_id.to_owned(),
            state: "running".to_owned(),
            last_event_at: Some(updated_at),
            stop_condition: "terminal_reply_or_error".to_owned(),
        }],
        resume_recipe: Some(TaskResumeRecipeRecord {
            recommended_tool: "task_status".to_owned(),
            session_id: session_id.to_owned(),
            note: Some(
                "Inspect task_status, task_wait, or task_history for durable task progress."
                    .to_owned(),
            ),
        }),
        updated_at,
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn verifying_task_progress_record(
    config: &LoongConfig,
    session_id: &str,
    user_input: &str,
) -> TaskProgressRecord {
    let updated_at = unix_ts_now();
    let task_id = resolve_canonical_task_id(config, session_id);

    TaskProgressRecord {
        task_id,
        owner_kind: "conversation_turn".to_owned(),
        status: TaskProgressStatus::Verifying,
        intent_summary: summarize_task_progress_intent(user_input),
        verification_state: Some(TaskVerificationState::Pending),
        active_handles: vec![TaskActiveHandleRecord {
            handle_kind: "turn_finalization".to_owned(),
            handle_id: session_id.to_owned(),
            state: "verifying".to_owned(),
            last_event_at: Some(updated_at),
            stop_condition: "after_turn_and_compaction_complete".to_owned(),
        }],
        resume_recipe: Some(TaskResumeRecipeRecord {
            recommended_tool: "task_status".to_owned(),
            session_id: session_id.to_owned(),
            note: Some(
                "Check task_status to see whether finalization verification has completed."
                    .to_owned(),
            ),
        }),
        updated_at,
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn waiting_task_progress_record(
    config: &LoongConfig,
    session_id: &str,
    user_input: &str,
) -> TaskProgressRecord {
    let updated_at = unix_ts_now();
    let task_id = resolve_canonical_task_id(config, session_id);

    TaskProgressRecord {
        task_id,
        owner_kind: "conversation_turn".to_owned(),
        status: TaskProgressStatus::Waiting,
        intent_summary: summarize_task_progress_intent(user_input),
        verification_state: Some(TaskVerificationState::Pending),
        active_handles: vec![TaskActiveHandleRecord {
            handle_kind: "approval_gate".to_owned(),
            handle_id: session_id.to_owned(),
            state: "waiting".to_owned(),
            last_event_at: Some(updated_at),
            stop_condition: "approval_decision".to_owned(),
        }],
        resume_recipe: Some(TaskResumeRecipeRecord {
            recommended_tool: "task_status".to_owned(),
            session_id: session_id.to_owned(),
            note: Some(
                "Use task_status or the approval control path to resolve the waiting task."
                    .to_owned(),
            ),
        }),
        updated_at,
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn completed_task_progress_record(
    config: &LoongConfig,
    session_id: &str,
    user_input: &str,
) -> TaskProgressRecord {
    TaskProgressRecord {
        task_id: resolve_canonical_task_id(config, session_id),
        owner_kind: "conversation_turn".to_owned(),
        status: TaskProgressStatus::Completed,
        intent_summary: summarize_task_progress_intent(user_input),
        verification_state: Some(TaskVerificationState::Passed),
        active_handles: Vec::new(),
        resume_recipe: None,
        updated_at: unix_ts_now(),
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn failed_task_progress_record(
    config: &LoongConfig,
    session_id: &str,
    user_input: &str,
) -> TaskProgressRecord {
    TaskProgressRecord {
        task_id: resolve_canonical_task_id(config, session_id),
        owner_kind: "conversation_turn".to_owned(),
        status: TaskProgressStatus::Failed,
        intent_summary: summarize_task_progress_intent(user_input),
        verification_state: Some(TaskVerificationState::Failed),
        active_handles: Vec::new(),
        resume_recipe: Some(TaskResumeRecipeRecord {
            recommended_tool: "task_history".to_owned(),
            session_id: session_id.to_owned(),
            note: Some(
                "Inspect recent task_history and task_status to diagnose the failed task."
                    .to_owned(),
            ),
        }),
        updated_at: unix_ts_now(),
    }
}

#[cfg(feature = "memory-sqlite")]
fn summarize_task_progress_intent(user_input: &str) -> Option<String> {
    let normalized = user_input.trim();
    if normalized.is_empty() {
        return None;
    }

    const MAX_CHARS: usize = 160;
    if normalized.chars().count() <= MAX_CHARS {
        return Some(normalized.to_owned());
    }

    let mut truncated = normalized
        .chars()
        .take(MAX_CHARS.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    Some(truncated)
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn checkpoint_waits_for_external_resolution(
    checkpoint: &TurnCheckpointSnapshot,
) -> bool {
    matches!(
        checkpoint.lane.as_ref().map(|lane| lane.result_kind),
        Some(TurnCheckpointResultKind::NeedsApproval)
    )
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn checkpoint_requires_verification_phase(checkpoint: &TurnCheckpointSnapshot) -> bool {
    !checkpoint_waits_for_external_resolution(checkpoint)
        && (checkpoint.finalization.runs_after_turn()
            || checkpoint.finalization.attempts_context_compaction())
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
    fn memory_config(&self) -> Option<&crate::session::store::SessionStoreConfig> {
        self.fallback.memory_config()
    }

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
                        store::session_store_config_from_memory_config(&self.config.memory);
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
