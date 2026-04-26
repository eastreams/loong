use super::*;

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn execute_delegate_tool<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_context: &SessionContext,
    payload: Value,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<loong_contracts::ToolCoreOutcome, String> {
    if !config.tools.delegate.enabled {
        return Err("app_tool_disabled: delegate is disabled by config".to_owned());
    }

    let delegate_request =
        crate::tools::delegate::parse_delegate_request_with_default_timeout(&payload)?;
    let delegate_policy =
        crate::tools::delegate::resolve_delegate_policy(&delegate_request, &config.tools.delegate);
    let child_session_id = crate::tools::delegate::next_delegate_session_id();
    let child_label = delegate_policy.label.clone();
    let subagent_identity =
        crate::tools::delegate::subagent_identity_for_delegate_request(&delegate_request);
    let repo = SessionRepository::new(&store::session_store_config_from_memory_config(
        &config.memory,
    ))?;
    let parent_task_id = resolve_canonical_task_id_for_session(&repo, &session_context.session_id)
        .unwrap_or_else(|| session_context.session_id.clone());
    let next_child_depth = next_delegate_child_depth_for_delegate(config, &repo, session_context)?;
    let runtime_self_continuity =
        effective_runtime_self_continuity_for_session(config, session_context);
    let workspace_root =
        prepare_delegate_workspace_root(config, &child_session_id, delegate_policy.isolation)?;
    let workspace_cleanup_owned_by_child =
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let workspace_cleanup_owned_by_child_for_work =
        std::sync::Arc::clone(&workspace_cleanup_owned_by_child);
    crate::conversation::delegate_support::with_prepared_subagent_spawn_cleanup_if_kernel_bound(
        runtime,
        &session_context.session_id,
        &child_session_id,
        binding,
        || async {
            let (_, execution) = repo.create_delegate_child_session_with_event_if_within_limit(
                &session_context.session_id,
                config.tools.delegate.max_active_children,
                |active_children| {
                    let execution_policy = DelegateChildExecutionPolicy {
                        isolation: delegate_policy.isolation,
                        profile: delegate_policy.profile,
                        owner_kind: None,
                        timeout_seconds: delegate_policy.timeout_seconds,
                        allow_shell_in_child: delegate_policy.allow_shell_in_child,
                        child_tool_allowlist: delegate_policy.child_tool_allowlist.clone(),
                        runtime_narrowing: delegate_policy.runtime_narrowing.clone(),
                        workspace_root: workspace_root.clone(),
                    };
                    let seed = build_delegate_child_lifecycle_seed(
                        config,
                        binding,
                        ConstrainedSubagentMode::Inline,
                        next_child_depth,
                        active_children,
                        &session_context.session_id,
                        &child_session_id,
                        child_label.clone(),
                        &delegate_request.task,
                        Some(parent_task_id.as_str()),
                        runtime_self_continuity.as_ref(),
                        subagent_identity.clone(),
                        execution_policy,
                    );
                    Ok((seed.request, seed.execution))
                },
            )?;
            let outcome = run_started_delegate_child_turn_with_runtime(
                config,
                runtime,
                &child_session_id,
                &session_context.session_id,
                child_label,
                &delegate_request.task,
                delegate_policy.profile,
                execution,
                delegate_policy.timeout_seconds,
                binding,
            )
            .await?;
            workspace_cleanup_owned_by_child_for_work
                .store(true, std::sync::atomic::Ordering::Release);

            Ok(outcome)
        },
    )
    .await
    .inspect_err(|_error| {
        let cleanup_owned_by_child =
            workspace_cleanup_owned_by_child.load(std::sync::atomic::Ordering::Acquire);
        if cleanup_owned_by_child {
            return;
        }

        let _ = cleanup_prepared_delegate_workspace_root(
            delegate_policy.isolation,
            workspace_root.as_deref(),
        );
    })
}

#[cfg(feature = "memory-sqlite")]
async fn enqueue_delegate_async_with_runtime<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_context: &SessionContext,
    delegate_request: crate::tools::delegate::DelegateRequest,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<loong_contracts::ToolCoreOutcome, String> {
    if !config.tools.delegate.enabled {
        return Err("app_tool_disabled: delegate is disabled by config".to_owned());
    }

    let runtime_handle = Handle::try_current()
        .map_err(|error| format!("delegate_async_runtime_unavailable: {error}"))?;
    let spawner = runtime
        .async_delegate_spawner(config)
        .ok_or_else(|| "delegate_async_not_configured".to_owned())?;
    let delegate_request = build_delegate_async_enqueue_request(
        config,
        runtime,
        session_context,
        delegate_request,
        binding,
        Some(crate::conversation::ConstrainedSubagentOwnerKind::AsyncDelegateSpawner),
    )
    .await?;
    let detached_config = std::sync::Arc::new(config.clone());
    spawn_async_delegate_detached(
        runtime_handle,
        detached_config,
        delegate_request.memory_config,
        spawner,
        delegate_request.request,
        config.tools.delegate.max_frozen_bytes,
        DelegateAnnounceSettings::from_config(config),
    );

    Ok(delegate_request.outcome)
}

#[cfg(feature = "memory-sqlite")]
async fn enqueue_background_task_with_runtime<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_context: &SessionContext,
    delegate_request: crate::tools::delegate::DelegateRequest,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<loong_contracts::ToolCoreOutcome, String> {
    if !config.tools.delegate.enabled {
        return Err("app_tool_disabled: delegate is disabled by config".to_owned());
    }

    let spawner = runtime
        .background_task_spawner(config)
        .ok_or_else(|| {
            "background_task_host_unavailable: current runtime does not provide a durable async background-task host"
                .to_owned()
        })?;
    let delegate_request = build_delegate_async_enqueue_request(
        config,
        runtime,
        session_context,
        delegate_request,
        binding,
        Some(crate::conversation::ConstrainedSubagentOwnerKind::BackgroundTaskHost),
    )
    .await?;
    let spawn_result = spawner.spawn(delegate_request.request.clone()).await;

    if let Err(error) = spawn_result {
        finalize_async_delegate_spawn_failure_with_recovery(
            &delegate_request.memory_config,
            &delegate_request.request.child_session_id,
            &delegate_request.request.parent_session_id,
            delegate_request.request.label.clone(),
            delegate_request.request.profile,
            &delegate_request.request.execution,
            config.tools.delegate.max_frozen_bytes,
            error.clone(),
        )?;
        enqueue_delegate_result_announce_with_memory_config(
            delegate_request.memory_config.clone(),
            delegate_request.request.parent_session_id.clone(),
            delegate_request.request.child_session_id.clone(),
            DelegateAnnounceSettings::from_config(config),
        );
        emit_async_delegate_child_terminal_event(
            runtime,
            &delegate_request.request.parent_session_id,
            &delegate_request.request.child_session_id,
            delegate_request.request.label.as_deref(),
            delegate_request.request.profile,
            "failed",
            delegate_request.request.execution.isolation,
            0,
            None,
            Some(error.as_str()),
            None,
            delegate_request.request.execution.workspace_root.as_deref(),
            None,
            binding,
        )
        .await;
        return Err(error);
    }

    Ok(delegate_request.outcome)
}

#[cfg(feature = "memory-sqlite")]
struct PreparedAsyncDelegateEnqueue {
    memory_config: SessionStoreConfig,
    request: AsyncDelegateSpawnRequest,
    outcome: loong_contracts::ToolCoreOutcome,
}

#[cfg(feature = "memory-sqlite")]
async fn build_delegate_async_enqueue_request<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_context: &SessionContext,
    delegate_request: crate::tools::delegate::DelegateRequest,
    binding: ConversationRuntimeBinding<'_>,
    owner_kind: Option<crate::conversation::ConstrainedSubagentOwnerKind>,
) -> Result<PreparedAsyncDelegateEnqueue, String> {
    let delegate_policy =
        crate::tools::delegate::resolve_delegate_policy(&delegate_request, &config.tools.delegate);
    let child_session_id = crate::tools::delegate::next_delegate_session_id();
    let child_label = delegate_policy.label.clone();
    let subagent_identity =
        crate::tools::delegate::subagent_identity_for_delegate_request(&delegate_request);
    let memory_config = store::session_store_config_from_memory_config(&config.memory);
    let repo = SessionRepository::new(&memory_config)?;
    let parent_task_id = resolve_canonical_task_id_for_session(&repo, &session_context.session_id)
        .unwrap_or_else(|| session_context.session_id.clone());

    ensure_session_exists_for_runtime_self_continuity(&repo, &session_context.session_id)?;

    let next_child_depth = next_delegate_child_depth_for_delegate(config, &repo, session_context)?;
    let runtime_self_continuity =
        effective_runtime_self_continuity_for_session(config, session_context);
    let workspace_root =
        prepare_delegate_workspace_root(config, &child_session_id, delegate_policy.isolation)?;
    let (_, execution) = repo
        .create_delegate_child_session_with_event_if_within_limit(
            &session_context.session_id,
            config.tools.delegate.max_active_children,
            |active_children| {
                let execution_policy = DelegateChildExecutionPolicy {
                    isolation: delegate_policy.isolation,
                    profile: delegate_policy.profile,
                    owner_kind,
                    timeout_seconds: delegate_policy.timeout_seconds,
                    allow_shell_in_child: delegate_policy.allow_shell_in_child,
                    child_tool_allowlist: delegate_policy.child_tool_allowlist.clone(),
                    runtime_narrowing: delegate_policy.runtime_narrowing.clone(),
                    workspace_root: workspace_root.clone(),
                };
                let seed = build_delegate_child_lifecycle_seed(
                    config,
                    binding,
                    ConstrainedSubagentMode::Async,
                    next_child_depth,
                    active_children,
                    &session_context.session_id,
                    &child_session_id,
                    child_label.clone(),
                    &delegate_request.task,
                    Some(parent_task_id.as_str()),
                    runtime_self_continuity.as_ref(),
                    subagent_identity.clone(),
                    execution_policy,
                );
                Ok((seed.request, seed.execution))
            },
        )
        .inspect_err(|_error| {
            let _ = cleanup_prepared_delegate_workspace_root(
                delegate_policy.isolation,
                workspace_root.as_deref(),
            );
        })?;

    let queued_execution = execution.clone();
    let queued_workspace_root = execution.workspace_root.clone();
    emit_async_delegate_child_queued_event(
        runtime,
        &session_context.session_id,
        &child_session_id,
        delegate_policy.label.as_deref(),
        delegate_policy.profile,
        delegate_policy.isolation,
        delegate_policy.timeout_seconds,
        queued_workspace_root.as_deref(),
        binding,
    )
    .await;
    let request = AsyncDelegateSpawnRequest {
        child_session_id: child_session_id.clone(),
        parent_session_id: session_context.session_id.clone(),
        task: delegate_request.task,
        canonical_task_id: Some(parent_task_id),
        label: child_label,
        profile: delegate_policy.profile,
        execution: queued_execution,
        runtime_self_continuity,
        timeout_seconds: delegate_policy.timeout_seconds,
        binding: OwnedConversationRuntimeBinding::from_borrowed(binding),
    };

    let mut outcome = crate::tools::delegate::delegate_async_queued_outcome(
        child_session_id,
        Some(session_context.session_id.clone()),
        delegate_policy.label,
        delegate_policy.profile,
        delegate_policy.timeout_seconds,
    );
    inject_delegate_workspace_metadata(&mut outcome, &execution, None, None);

    Ok(PreparedAsyncDelegateEnqueue {
        memory_config,
        request,
        outcome,
    })
}

#[cfg(feature = "memory-sqlite")]
pub async fn spawn_background_delegate_with_runtime<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_id: &str,
    task: &str,
    label: Option<String>,
    _specialization: Option<String>,
    timeout_seconds: Option<u64>,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<loong_contracts::ToolCoreOutcome, String> {
    let session_context = runtime.session_context(config, session_id, binding)?;
    let mut delegate_payload = json!({
        "task": task,
    });
    if let Some(label) = label
        && let Some(payload_object) = delegate_payload.as_object_mut()
    {
        payload_object.insert("label".to_owned(), json!(label));
    }
    if let Some(specialization) = _specialization
        && let Some(payload_object) = delegate_payload.as_object_mut()
    {
        payload_object.insert("specialization".to_owned(), json!(specialization));
    }
    if let Some(timeout_seconds) = timeout_seconds
        && let Some(payload_object) = delegate_payload.as_object_mut()
    {
        payload_object.insert("timeout_seconds".to_owned(), json!(timeout_seconds));
    }
    let delegate_request =
        crate::tools::delegate::parse_delegate_request_with_default_timeout(&delegate_payload)?;
    enqueue_background_task_with_runtime(
        config,
        runtime,
        &session_context,
        delegate_request,
        binding,
    )
    .await
}

#[cfg(not(feature = "memory-sqlite"))]
pub async fn spawn_background_delegate_with_runtime<R: ConversationRuntime + ?Sized>(
    _config: &LoongConfig,
    _runtime: &R,
    _session_id: &str,
    _task: &str,
    _label: Option<String>,
    _specialization: Option<String>,
    _timeout_seconds: Option<u64>,
    _binding: ConversationRuntimeBinding<'_>,
) -> Result<loong_contracts::ToolCoreOutcome, String> {
    Err("delegate_async requires sqlite memory support (enable feature `memory-sqlite`)".to_owned())
}

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn execute_delegate_async_tool<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_context: &SessionContext,
    payload: Value,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<loong_contracts::ToolCoreOutcome, String> {
    if !binding.allows_mutation() {
        return Err("app_tool_denied: governed_runtime_binding_required".to_owned());
    }

    let delegate_request =
        crate::tools::delegate::parse_delegate_request_with_default_timeout(&payload)?;
    enqueue_delegate_async_with_runtime(config, runtime, session_context, delegate_request, binding)
        .await
}

#[cfg(not(feature = "memory-sqlite"))]
pub(crate) async fn execute_delegate_tool<R: ConversationRuntime + ?Sized>(
    _config: &LoongConfig,
    _runtime: &R,
    _session_context: &SessionContext,
    _payload: Value,
    _binding: ConversationRuntimeBinding<'_>,
) -> Result<loong_contracts::ToolCoreOutcome, String> {
    Err("delegate requires sqlite memory support (enable feature `memory-sqlite`)".to_owned())
}

#[cfg(not(feature = "memory-sqlite"))]
pub(crate) async fn execute_delegate_async_tool<R: ConversationRuntime + ?Sized>(
    _config: &LoongConfig,
    _runtime: &R,
    _session_context: &SessionContext,
    _payload: Value,
    _binding: ConversationRuntimeBinding<'_>,
) -> Result<loong_contracts::ToolCoreOutcome, String> {
    Err("delegate_async requires sqlite memory support (enable feature `memory-sqlite`)".to_owned())
}

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn run_started_delegate_child_turn_with_runtime<
    R: ConversationRuntime + ?Sized,
>(
    config: &LoongConfig,
    runtime: &R,
    child_session_id: &str,
    parent_session_id: &str,
    child_label: Option<String>,
    user_input: &str,
    profile: Option<crate::conversation::DelegateBuiltinProfile>,
    execution: ConstrainedSubagentExecution,
    timeout_seconds: u64,
    binding: ConversationRuntimeBinding<'_>,
) -> Result<loong_contracts::ToolCoreOutcome, String> {
    // This resumes execution after the child session row/workspace have already
    // been created. The remaining job is to run the child turn with the shared
    // runtime/binding, enforce timeout + unwind containment, and then finalize
    // the persisted delegate session/announcement state from the outcome.
    let repo = SessionRepository::new(&store::session_store_config_from_memory_config(
        &config.memory,
    ))?;
    let start = Instant::now();
    let child_coordinator = ConversationTurnCoordinator::new();
    let child_turn_future = child_coordinator.handle_turn_with_runtime(
        config,
        child_session_id,
        user_input,
        ProviderErrorMode::Propagate,
        runtime,
        binding,
    );
    // Heap-allocate nested delegate turn futures so each child turn does not inline the next
    // delegate layer into the parent future on small-stack platforms like Windows.
    let child_turn_future = Box::pin(AssertUnwindSafe(child_turn_future).catch_unwind());
    let child_timeout = Duration::from_secs(timeout_seconds);
    let child_result = timeout(child_timeout, child_turn_future).await;
    let duration_ms = start.elapsed().as_millis() as u64;

    match child_result {
        Ok(Ok(Ok(final_output))) => {
            let turn_count = repo
                .load_session_summary(child_session_id)?
                .map(|session| session.turn_count)
                .unwrap_or_default();
            let final_output_preview = final_output.clone();
            let mut outcome = crate::tools::delegate::delegate_success_outcome(
                child_session_id.to_owned(),
                Some(parent_session_id.to_owned()),
                child_label.clone(),
                profile,
                final_output,
                turn_count,
                duration_ms,
            );
            let workspace_cleanup = cleanup_delegate_workspace_root(&execution);
            let (cleanup_metadata, cleanup_error) =
                split_delegate_workspace_cleanup(workspace_cleanup);
            inject_delegate_workspace_metadata(
                &mut outcome,
                &execution,
                cleanup_metadata.as_ref(),
                cleanup_error,
            );
            let event_payload_json = execution.terminal_payload(
                ConstrainedSubagentTerminalReason::Completed,
                duration_ms,
                Some(turn_count),
                None,
            );
            finalize_and_announce_delegate_child_terminal(
                config,
                &repo,
                child_session_id,
                parent_session_id,
                &outcome,
                SessionState::Completed,
                None,
                "delegate_completed",
                event_payload_json,
            )?;
            if execution.mode == ConstrainedSubagentMode::Async {
                emit_async_delegate_child_terminal_event(
                    runtime,
                    parent_session_id,
                    child_session_id,
                    child_label.as_deref(),
                    profile,
                    "completed",
                    execution.isolation,
                    duration_ms,
                    Some(turn_count),
                    None,
                    Some(final_output_preview.as_str()),
                    execution.workspace_root.as_deref(),
                    cleanup_metadata.as_ref().map(|cleanup| cleanup.retained),
                    binding,
                )
                .await;
            }
            notify_parent_delegate_result(parent_session_id, child_session_id, &outcome);
            Ok(outcome)
        }
        Ok(Ok(Err(error))) => {
            let mut outcome = crate::tools::delegate::delegate_error_outcome(
                child_session_id.to_owned(),
                Some(parent_session_id.to_owned()),
                child_label.clone(),
                profile,
                error.clone(),
                duration_ms,
            );
            let workspace_cleanup = cleanup_delegate_workspace_root(&execution);
            let (cleanup_metadata, cleanup_error) =
                split_delegate_workspace_cleanup(workspace_cleanup);
            inject_delegate_workspace_metadata(
                &mut outcome,
                &execution,
                cleanup_metadata.as_ref(),
                cleanup_error,
            );
            let event_payload_json = execution.terminal_payload(
                ConstrainedSubagentTerminalReason::Failed,
                duration_ms,
                None,
                Some(error.as_str()),
            );
            finalize_and_announce_delegate_child_terminal(
                config,
                &repo,
                child_session_id,
                parent_session_id,
                &outcome,
                SessionState::Failed,
                Some(error.clone()),
                "delegate_failed",
                event_payload_json,
            )?;
            if execution.mode == ConstrainedSubagentMode::Async {
                emit_async_delegate_child_terminal_event(
                    runtime,
                    parent_session_id,
                    child_session_id,
                    child_label.as_deref(),
                    profile,
                    "failed",
                    execution.isolation,
                    duration_ms,
                    None,
                    Some(error.as_str()),
                    None,
                    execution.workspace_root.as_deref(),
                    cleanup_metadata.as_ref().map(|cleanup| cleanup.retained),
                    binding,
                )
                .await;
            }
            notify_parent_delegate_result(parent_session_id, child_session_id, &outcome);
            Ok(outcome)
        }
        Ok(Err(panic_payload)) => {
            let panic_error = format_delegate_child_panic(panic_payload);
            let mut outcome = crate::tools::delegate::delegate_error_outcome(
                child_session_id.to_owned(),
                Some(parent_session_id.to_owned()),
                child_label.clone(),
                profile,
                panic_error.clone(),
                duration_ms,
            );
            let workspace_cleanup = cleanup_delegate_workspace_root(&execution);
            let (cleanup_metadata, cleanup_error) =
                split_delegate_workspace_cleanup(workspace_cleanup);
            inject_delegate_workspace_metadata(
                &mut outcome,
                &execution,
                cleanup_metadata.as_ref(),
                cleanup_error,
            );
            let event_payload_json = execution.terminal_payload(
                ConstrainedSubagentTerminalReason::Failed,
                duration_ms,
                None,
                Some(panic_error.as_str()),
            );
            finalize_and_announce_delegate_child_terminal(
                config,
                &repo,
                child_session_id,
                parent_session_id,
                &outcome,
                SessionState::Failed,
                Some(panic_error.clone()),
                "delegate_failed",
                event_payload_json,
            )?;
            if execution.mode == ConstrainedSubagentMode::Async {
                emit_async_delegate_child_terminal_event(
                    runtime,
                    parent_session_id,
                    child_session_id,
                    child_label.as_deref(),
                    profile,
                    "failed",
                    execution.isolation,
                    duration_ms,
                    None,
                    Some(panic_error.as_str()),
                    None,
                    execution.workspace_root.as_deref(),
                    cleanup_metadata.as_ref().map(|cleanup| cleanup.retained),
                    binding,
                )
                .await;
            }
            notify_parent_delegate_result(parent_session_id, child_session_id, &outcome);
            Ok(outcome)
        }
        Err(_) => {
            let timeout_error = "delegate_timeout".to_owned();
            let mut outcome = crate::tools::delegate::delegate_timeout_outcome(
                child_session_id.to_owned(),
                Some(parent_session_id.to_owned()),
                child_label.clone(),
                profile,
                duration_ms,
            );
            let workspace_cleanup = cleanup_delegate_workspace_root(&execution);
            let (cleanup_metadata, cleanup_error) =
                split_delegate_workspace_cleanup(workspace_cleanup);
            inject_delegate_workspace_metadata(
                &mut outcome,
                &execution,
                cleanup_metadata.as_ref(),
                cleanup_error,
            );
            let event_payload_json = execution.terminal_payload(
                ConstrainedSubagentTerminalReason::TimedOut,
                duration_ms,
                None,
                Some(timeout_error.as_str()),
            );
            finalize_and_announce_delegate_child_terminal(
                config,
                &repo,
                child_session_id,
                parent_session_id,
                &outcome,
                SessionState::TimedOut,
                Some(timeout_error.clone()),
                "delegate_timed_out",
                event_payload_json,
            )?;
            if execution.mode == ConstrainedSubagentMode::Async {
                emit_async_delegate_child_terminal_event(
                    runtime,
                    parent_session_id,
                    child_session_id,
                    child_label.as_deref(),
                    profile,
                    "timed_out",
                    execution.isolation,
                    duration_ms,
                    None,
                    Some(timeout_error.as_str()),
                    None,
                    execution.workspace_root.as_deref(),
                    cleanup_metadata.as_ref().map(|cleanup| cleanup.retained),
                    binding,
                )
                .await;
            }
            notify_parent_delegate_result(parent_session_id, child_session_id, &outcome);
            Ok(outcome)
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn notify_parent_delegate_result(
    parent_session_id: &str,
    child_session_id: &str,
    outcome: &loong_contracts::ToolCoreOutcome,
) {
    let mailbox = mailbox_for_session(parent_session_id);
    let author = AgentPath::root()
        .join(child_session_id)
        .unwrap_or_else(|_| AgentPath::root());
    let message = InterAgentMessage {
        author,
        recipient: AgentPath::root(),
        content: MailboxContent::DelegateResult {
            session_id: child_session_id.to_owned(),
            frozen_result: json!({
                "status": outcome.status,
                "payload": outcome.payload,
            }),
        },
        trigger_turn: true,
    };
    let _ = mailbox.send(message);
}
