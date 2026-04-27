use super::*;
use crate::conversation::PromptFrame;
use crate::conversation::PromptFrameSummary;

#[derive(Debug, Clone)]
pub(super) struct ProviderTurnSessionState {
    pub(super) messages: Vec<Value>,
    pub(super) estimated_tokens: Option<usize>,
    pub(super) prompt_frame: PromptFrame,
}

impl ProviderTurnSessionState {
    pub(super) fn from_assembled_context(
        assembled_context: AssembledConversationContext,
        user_input: &str,
        ingress: Option<&ConversationIngressContext>,
    ) -> Self {
        let AssembledConversationContext {
            messages,
            artifacts,
            estimated_tokens,
            prompt_fragments,
            system_prompt_addition,
        } = assembled_context;
        let mut messages = messages;
        let turn_ephemeral_start_index = messages.len();
        if let Some(ingress) = ingress.filter(|value| value.has_contextual_hints()) {
            messages.push(ingress.as_system_message());
        }
        messages.push(json!({
            "role": "user",
            "content": user_input,
        }));
        let prompt_frame = PromptFrame::from_context_parts(
            prompt_fragments.as_slice(),
            messages.as_slice(),
            artifacts.as_slice(),
            estimated_tokens,
            Some(turn_ephemeral_start_index),
        );
        let assembled_context = AssembledConversationContext {
            messages,
            artifacts,
            estimated_tokens,
            prompt_fragments,
            system_prompt_addition,
        };
        Self {
            messages: assembled_context.messages,
            estimated_tokens,
            prompt_frame,
        }
    }

    pub(super) fn after_turn_messages(&self, reply: &str) -> Vec<Value> {
        let mut messages = self.messages.clone();
        messages.push(json!({
            "role": "assistant",
            "content": reply,
        }));
        messages
    }

    pub(super) fn prompt_frame_summary(&self) -> &PromptFrameSummary {
        &self.prompt_frame.summary
    }
}

#[derive(Debug, Clone)]
pub(super) struct ProviderTurnReplyTailPhase {
    reply: String,
    after_turn_messages: Vec<Value>,
    estimated_tokens: Option<usize>,
}

impl ProviderTurnReplyTailPhase {
    pub(super) fn from_session(session: &ProviderTurnSessionState, reply: &str) -> Self {
        Self {
            reply: reply.to_owned(),
            after_turn_messages: session.after_turn_messages(reply),
            estimated_tokens: session.estimated_tokens,
        }
    }

    pub(super) fn reply(&self) -> &str {
        self.reply.as_str()
    }

    pub(super) fn after_turn_messages(&self) -> &[Value] {
        &self.after_turn_messages
    }

    pub(super) fn estimated_tokens(&self) -> Option<usize> {
        self.estimated_tokens
    }
}

#[derive(Debug, Clone)]
pub(super) struct ProviderTurnPreparation {
    pub(super) session: ProviderTurnSessionState,
    pub(super) lane_plan: ProviderTurnLanePlan,
    pub(super) raw_tool_output_requested: bool,
    pub(super) turn_id: String,
}

impl ProviderTurnPreparation {
    #[cfg(test)]
    pub(super) fn from_assembled_context(
        config: &LoongConfig,
        assembled_context: AssembledConversationContext,
        user_input: &str,
        ingress: Option<&ConversationIngressContext>,
    ) -> Self {
        let turn_id = next_conversation_turn_id();
        Self::from_assembled_context_with_turn_id(
            config,
            assembled_context,
            user_input,
            turn_id.as_str(),
            ingress,
        )
    }

    pub(super) fn from_assembled_context_with_turn_id(
        config: &LoongConfig,
        assembled_context: AssembledConversationContext,
        user_input: &str,
        turn_id: &str,
        ingress: Option<&ConversationIngressContext>,
    ) -> Self {
        Self {
            session: ProviderTurnSessionState::from_assembled_context(
                assembled_context,
                user_input,
                ingress,
            ),
            lane_plan: ProviderTurnLanePlan::from_user_input(config, user_input),
            raw_tool_output_requested: user_requested_raw_tool_output(user_input),
            turn_id: turn_id.to_owned(),
        }
    }

    pub(super) fn for_followup_messages(&self, messages: Vec<Value>) -> Self {
        let default_tail_start_index = self.session.messages.len();
        let tail_start_index = self
            .session
            .prompt_frame
            .turn_ephemeral_start_index()
            .unwrap_or(default_tail_start_index);
        let followup_tail_slice = messages.get(tail_start_index..);
        let followup_tail_messages = followup_tail_slice.unwrap_or(&[]).to_vec();
        let prompt_frame = self
            .session
            .prompt_frame
            .with_turn_ephemeral_messages(followup_tail_messages.as_slice(), None);
        Self {
            session: ProviderTurnSessionState {
                messages,
                estimated_tokens: None,
                prompt_frame,
            },
            lane_plan: self.lane_plan.clone(),
            raw_tool_output_requested: self.raw_tool_output_requested,
            turn_id: self.turn_id.clone(),
        }
    }

    pub(super) fn checkpoint(&self) -> TurnPreparationSnapshot {
        TurnPreparationSnapshot {
            lane: self.lane_plan.decision.lane,
            max_tool_steps: self.lane_plan.max_tool_steps,
            raw_tool_output_requested: self.raw_tool_output_requested,
            context_message_count: self.session.messages.len(),
            context_fingerprint_sha256: checkpoint_context_fingerprint_sha256(
                &self.session.messages,
            ),
            estimated_tokens: self.session.estimated_tokens,
        }
    }
}

#[derive(Debug)]
pub(super) struct DiscoveryFirstFollowupTurnSummary {
    pub(super) outcome: String,
    pub(super) followup_tool_name: Option<String>,
    pub(super) followup_target_tool_id: Option<String>,
    pub(super) resolved_to_tool_invoke: bool,
}

pub(super) fn summarize_discovery_first_followup_turn(
    turn: &ProviderTurn,
) -> DiscoveryFirstFollowupTurnSummary {
    let Some(first) = turn.tool_intents.first() else {
        return DiscoveryFirstFollowupTurnSummary {
            outcome: "final_reply".to_owned(),
            followup_tool_name: None,
            followup_target_tool_id: None,
            resolved_to_tool_invoke: false,
        };
    };

    let intent = turn
        .tool_intents
        .iter()
        .find(|intent| {
            crate::tools::canonical_tool_name(intent.tool_name.as_str()) == "tool.invoke"
        })
        .unwrap_or(first);
    let canonical_tool_name =
        crate::tools::canonical_tool_name(intent.tool_name.as_str()).to_owned();
    let resolved_to_tool_invoke = canonical_tool_name == "tool.invoke";
    let followup_target_tool_id = resolved_to_tool_invoke
        .then(|| {
            intent
                .args_json
                .get("tool_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .flatten();

    DiscoveryFirstFollowupTurnSummary {
        outcome: canonical_tool_name.clone(),
        followup_tool_name: Some(canonical_tool_name),
        followup_target_tool_id,
        resolved_to_tool_invoke,
    }
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
fn resolve_canonical_task_id(config: &LoongConfig, session_id: &str) -> String {
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

pub(super) fn estimate_tokens_for_messages(
    estimated_tokens: Option<usize>,
    messages: &[Value],
) -> Option<usize> {
    estimated_tokens.or_else(|| estimate_tokens(messages))
}

const DELEGATE_CHILD_OUTPUT_PREVIEW_CHARS: usize = 200;

pub(super) async fn emit_discovery_first_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    event_name: &str,
    payload: Value,
    binding: ConversationRuntimeBinding<'_>,
) {
    let _ = persist_conversation_event(runtime, session_id, event_name, payload, binding).await;
    if let Some(ctx) = binding.kernel_context() {
        let _ = ctx.kernel.record_audit_event(
            Some(ctx.agent_id()),
            AuditEventKind::PlaneInvoked {
                pack_id: ctx.pack_id().to_owned(),
                plane: ExecutionPlane::Runtime,
                tier: PlaneTier::Core,
                primary_adapter: "conversation.discovery_first".to_owned(),
                delegated_core_adapter: None,
                operation: format!("conversation.discovery_first.{event_name}"),
                required_capabilities: Vec::new(),
            },
        );
    }
}

pub(super) async fn emit_prompt_frame_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    provider_round: usize,
    phase: &str,
    summary: &PromptFrameSummary,
    binding: ConversationRuntimeBinding<'_>,
) {
    let payload = json!({
        "provider_round": provider_round,
        "phase": phase,
        "prompt_frame": summary.to_event_payload(),
    });
    let _ = persist_conversation_event(
        runtime,
        session_id,
        "provider_prompt_frame_snapshot",
        payload,
        binding,
    )
    .await;
}

#[cfg(feature = "memory-sqlite")]
pub(super) async fn emit_async_delegate_child_queued_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    parent_session_id: &str,
    child_session_id: &str,
    child_label: Option<&str>,
    profile: Option<crate::conversation::DelegateBuiltinProfile>,
    isolation: crate::conversation::ConstrainedSubagentIsolation,
    timeout_seconds: u64,
    workspace_root: Option<&std::path::Path>,
    binding: ConversationRuntimeBinding<'_>,
) {
    emit_delegate_child_projection_event(
        runtime,
        parent_session_id,
        "delegate_child_queued",
        json!({
            "child_session_id": child_session_id,
            "label": child_label,
            "profile": profile.map(crate::conversation::DelegateBuiltinProfile::as_str),
            "mode": "async",
            "phase": "queued",
            "isolation": isolation.as_str(),
            "timeout_seconds": timeout_seconds,
            "workspace_root": workspace_root.map(|workspace_root| workspace_root.display().to_string()),
        }),
        binding,
    )
    .await;
}

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn emit_async_delegate_child_terminal_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    parent_session_id: &str,
    child_session_id: &str,
    child_label: Option<&str>,
    profile: Option<crate::conversation::DelegateBuiltinProfile>,
    phase: &'static str,
    isolation: crate::conversation::ConstrainedSubagentIsolation,
    duration_ms: u64,
    turn_count: Option<usize>,
    error: Option<&str>,
    final_output: Option<&str>,
    workspace_root: Option<&std::path::Path>,
    workspace_retained: Option<bool>,
    binding: ConversationRuntimeBinding<'_>,
) {
    emit_delegate_child_projection_event(
        runtime,
        parent_session_id,
        "delegate_child_terminal",
        json!({
            "child_session_id": child_session_id,
            "label": child_label,
            "profile": profile.map(crate::conversation::DelegateBuiltinProfile::as_str),
            "mode": "async",
            "phase": phase,
            "isolation": isolation.as_str(),
            "duration_ms": duration_ms,
            "turn_count": turn_count,
            "error": error,
            "final_output_preview": final_output.map(truncate_delegate_child_output_preview),
            "workspace_root": workspace_root.map(|workspace_root| workspace_root.display().to_string()),
            "workspace_retained": workspace_retained,
        }),
        binding,
    )
    .await;
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn inject_delegate_workspace_metadata(
    outcome: &mut loong_contracts::ToolCoreOutcome,
    execution: &ConstrainedSubagentExecution,
    cleanup: Option<&DelegateWorkspaceCleanupResult>,
    cleanup_error: Option<String>,
) {
    let Some(object) = outcome.payload.as_object_mut() else {
        return;
    };

    object.insert("isolation".to_owned(), json!(execution.isolation.as_str()));
    if let Some(workspace_root) = execution.workspace_root.as_ref() {
        let display_path = workspace_root.display().to_string();
        object.insert("workspace_root".to_owned(), json!(display_path));
    }
    if let Some(cleanup) = cleanup {
        object.insert("workspace_retained".to_owned(), json!(cleanup.retained));
        object.insert("workspace_dirty".to_owned(), json!(cleanup.dirty));
    }
    if let Some(cleanup_error) = cleanup_error {
        object.insert("workspace_cleanup_error".to_owned(), json!(cleanup_error));
    }
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn split_delegate_workspace_cleanup(
    cleanup: Result<Option<DelegateWorkspaceCleanupResult>, String>,
) -> (Option<DelegateWorkspaceCleanupResult>, Option<String>) {
    match cleanup {
        Ok(metadata) => (metadata, None),
        Err(error) => (None, Some(error)),
    }
}

#[cfg(feature = "memory-sqlite")]
async fn emit_delegate_child_projection_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    parent_session_id: &str,
    event_name: &str,
    payload: Value,
    binding: ConversationRuntimeBinding<'_>,
) {
    let _ =
        persist_conversation_event(runtime, parent_session_id, event_name, payload, binding).await;
    if let Some(ctx) = binding.kernel_context() {
        let _ = ctx.kernel.record_audit_event(
            Some(ctx.agent_id()),
            AuditEventKind::PlaneInvoked {
                pack_id: ctx.pack_id().to_owned(),
                plane: ExecutionPlane::Runtime,
                tier: PlaneTier::Core,
                primary_adapter: "conversation.delegate_child".to_owned(),
                delegated_core_adapter: None,
                operation: format!("conversation.delegate_child.{event_name}"),
                required_capabilities: Vec::new(),
            },
        );
    }
}

#[cfg(feature = "memory-sqlite")]
fn truncate_delegate_child_output_preview(value: &str) -> String {
    let total_chars = value.chars().count();
    if total_chars <= DELEGATE_CHILD_OUTPUT_PREVIEW_CHARS {
        return value.to_owned();
    }

    let mut truncated = String::new();
    for ch in value.chars().take(DELEGATE_CHILD_OUTPUT_PREVIEW_CHARS) {
        truncated.push(ch);
    }
    let omitted = total_chars.saturating_sub(DELEGATE_CHILD_OUTPUT_PREVIEW_CHARS);
    truncated.push_str(&format!("...(truncated {omitted} chars)"));
    truncated
}
