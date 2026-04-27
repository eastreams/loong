use super::*;

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn pending_approval_control_turn_bootstraps_once_and_emits_terminal_phases_once() {
    let runtime = ApprovalControlRuntime::default();
    let observer = Arc::new(RecordingTurnObserver::default());
    let observer_handle: ConversationTurnObserverHandle = observer.clone();
    let coordinator = ConversationTurnCoordinator::new();
    let mut config = LoongConfig::default();
    let memory_config = sqlite_memory_config("approval-control-bootstrap");
    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .display()
        .to_string();
    config.memory.sqlite_path = sqlite_path;
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");
    seed_pending_approval_request(&repo, "root-session", "apr-deny-1", "delegate_async", "app");

    let acp_options = AcpConversationTurnOptions::automatic();
    let address = ConversationSessionAddress::from_session_id("root-session");
    let kernel_ctx = crate::context::bootstrap_test_kernel_context("approval-control-observer", 60)
        .expect("kernel context");

    let reply = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            &config,
            &address,
            "esc",
            ProviderErrorMode::Propagate,
            &runtime,
            &acp_options,
            ConversationRuntimeBinding::kernel(&kernel_ctx),
            None,
            Some(observer_handle),
            None,
            None,
        )
        .await
        .expect("approval control turn should succeed");

    assert_eq!(reply, "approval handled");

    let bootstrap_calls = runtime
        .bootstrap_calls
        .lock()
        .expect("bootstrap call lock should not be poisoned");
    assert_eq!(*bootstrap_calls, 1);

    let phase_events = observer
        .phase_events
        .lock()
        .expect("phase event lock should not be poisoned");
    let phase_names = phase_events
        .iter()
        .map(|event| event.phase)
        .collect::<Vec<_>>();

    assert_eq!(
        phase_names,
        vec![
            ConversationTurnPhase::Preparing,
            ConversationTurnPhase::ContextReady,
            ConversationTurnPhase::RunningTools,
            ConversationTurnPhase::FinalizingReply,
            ConversationTurnPhase::Completed,
        ]
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn pending_approval_control_turn_does_not_persist_session_mode_when_resolution_fails() {
    let coordinator = ConversationTurnCoordinator::new();
    let runtime = ApprovalControlRuntime::default();
    let mut config = LoongConfig::default();
    let memory_config = sqlite_memory_config("approval-control-session-mode");
    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .display()
        .to_string();
    config.memory.sqlite_path = sqlite_path;
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");
    seed_pending_approval_request(
        &repo,
        "root-session",
        "apr-auto-failure",
        "delegate_async",
        "app",
    );

    let db_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .to_path_buf();
    let conn = rusqlite::Connection::open(db_path).expect("open sqlite connection");
    conn.execute_batch(
        "
        CREATE TRIGGER fail_pending_approval_update
        BEFORE UPDATE ON approval_requests
        BEGIN
            SELECT RAISE(FAIL, 'forced approval request update failure');
        END;
        ",
    )
    .expect("create approval failure trigger");

    let acp_options = AcpConversationTurnOptions::automatic();
    let address = ConversationSessionAddress::from_session_id("root-session");
    let result = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer_with_manager(
            &config,
            &address,
            "auto",
            ProviderErrorMode::Propagate,
            &runtime,
            &acp_options,
            ConversationRuntimeBinding::direct(),
            None,
            None,
            None,
            None,
        )
        .await;
    assert!(
        result.is_ok(),
        "approval control turn should stay user-visible"
    );

    let stored = repo
        .load_session_tool_consent("root-session")
        .expect("load session tool consent");
    assert!(
        stored.is_none(),
        "session mode should not persist on failure"
    );

    let approval_request = repo
        .load_approval_request("apr-auto-failure")
        .expect("load approval request")
        .expect("approval request row");
    assert_eq!(approval_request.status, ApprovalRequestStatus::Pending);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn pending_approval_control_turn_resolves_delegate_request_after_yes_confirmation() {
    let coordinator = ConversationTurnCoordinator::new();
    let runtime = ApprovalControlRuntime::default();
    let mut config = LoongConfig::default();
    let memory_config = sqlite_memory_config("approval-control-run-once-success");
    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .display()
        .to_string();
    config.memory.sqlite_path = sqlite_path;
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");
    seed_pending_approval_request(&repo, "root-session", "apr-delegate-yes", "delegate", "app");

    let acp_options = AcpConversationTurnOptions::automatic();
    let address = ConversationSessionAddress::from_session_id("root-session");
    let reply = coordinator
        .handle_turn_with_runtime_and_address_and_acp_options_and_ingress_and_observer(
            &config,
            &address,
            "yes",
            ProviderErrorMode::Propagate,
            &runtime,
            &acp_options,
            ConversationRuntimeBinding::direct(),
            None,
            None,
            None,
        )
        .await
        .expect("approval control turn should succeed");

    assert_eq!(reply, "approval handled");

    let approval_request = repo
        .load_approval_request("apr-delegate-yes")
        .expect("load approval request")
        .expect("approval request row");
    assert_eq!(approval_request.status, ApprovalRequestStatus::Approved);
    assert_eq!(
        approval_request.decision,
        Some(ApprovalDecision::ApproveOnce)
    );
    assert_eq!(
        approval_request.resolved_by_session_id.as_deref(),
        Some("root-session")
    );
    assert!(
        approval_request.last_error.is_none(),
        "request={approval_request:?}"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn approval_request_resolve_persists_session_mode_on_success() {
    let runtime = ApprovalControlRuntime::default();
    let mut config = LoongConfig::default();
    let memory_config = sqlite_memory_config("approval-control-session-mode-success");
    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .display()
        .to_string();
    config.memory.sqlite_path = sqlite_path;
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");
    seed_pending_approval_request(
        &repo,
        "root-session",
        "apr-auto-success",
        "sessions_list",
        "app",
    );
    let fallback = DefaultAppToolDispatcher::new(memory_config.clone(), ToolConfig::default());
    let approval_runtime = CoordinatorApprovalResolutionRuntime::new(
        &config,
        &runtime,
        &fallback,
        ConversationRuntimeBinding::direct(),
    );
    let outcome = crate::tools::approval::execute_approval_tool_with_runtime_support(
        loong_contracts::ToolCoreRequest {
            tool_name: "approval_request_resolve".to_owned(),
            payload: json!({
                "approval_request_id": "apr-auto-success",
                "decision": "approve_once",
                "session_consent_mode": "auto",
            }),
        },
        "root-session",
        &memory_config,
        &ToolConfig::default(),
        Some(&approval_runtime),
    )
    .await
    .expect("approval request resolve should succeed");
    assert_eq!(outcome.payload["approval_request"]["status"], "approved");

    let stored = repo
        .load_session_tool_consent("root-session")
        .expect("load session tool consent")
        .expect("session tool consent row");
    assert_eq!(stored.mode, ToolConsentMode::Auto);
    assert_eq!(
        stored.updated_by_session_id.as_deref(),
        Some("root-session")
    );

    let approval_request = repo
        .load_approval_request("apr-auto-success")
        .expect("load approval request")
        .expect("approval request row");
    assert_eq!(
        approval_request.decision,
        Some(ApprovalDecision::ApproveOnce)
    );
    assert_eq!(approval_request.status, ApprovalRequestStatus::Approved);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn approval_request_resolve_retries_missing_session_mode_after_approval() {
    let runtime = ApprovalControlRuntime::default();
    let mut config = LoongConfig::default();
    let memory_config = sqlite_memory_config("approval-control-session-mode-retry");
    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .display()
        .to_string();
    config.memory.sqlite_path = sqlite_path;
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");
    seed_pending_approval_request(
        &repo,
        "root-session",
        "apr-auto-retry",
        "sessions_list",
        "app",
    );
    repo.transition_approval_request_if_current(
        "apr-auto-retry",
        TransitionApprovalRequestIfCurrentRequest {
            expected_status: ApprovalRequestStatus::Pending,
            next_status: ApprovalRequestStatus::Approved,
            decision: Some(ApprovalDecision::ApproveOnce),
            resolved_by_session_id: Some("root-session".to_owned()),
            executed_at: None,
            last_error: None,
        },
    )
    .expect("transition approval request")
    .expect("approval request should be pending");

    let fallback = DefaultAppToolDispatcher::new(memory_config.clone(), ToolConfig::default());
    let approval_runtime = CoordinatorApprovalResolutionRuntime::new(
        &config,
        &runtime,
        &fallback,
        ConversationRuntimeBinding::direct(),
    );
    let outcome = crate::tools::approval::execute_approval_tool_with_runtime_support(
        loong_contracts::ToolCoreRequest {
            tool_name: "approval_request_resolve".to_owned(),
            payload: json!({
                "approval_request_id": "apr-auto-retry",
                "decision": "approve_once",
                "session_consent_mode": "auto",
            }),
        },
        "root-session",
        &memory_config,
        &ToolConfig::default(),
        Some(&approval_runtime),
    )
    .await
    .expect("approval request retry should succeed");

    assert_eq!(outcome.payload["approval_request"]["status"], "approved");

    let stored = repo
        .load_session_tool_consent("root-session")
        .expect("load session tool consent")
        .expect("session tool consent row");
    assert_eq!(stored.mode, ToolConsentMode::Auto);
    assert_eq!(
        stored.updated_by_session_id.as_deref(),
        Some("root-session")
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn core_approval_replay_skips_app_session_context_loading() {
    let runtime = CoreReplayRuntime;
    let mut config = LoongConfig::default();
    let memory_config = sqlite_memory_config("approval-core-replay");
    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .display()
        .to_string();
    config.memory.sqlite_path = sqlite_path;
    let repo = SessionRepository::new(&memory_config).expect("repository");
    repo.ensure_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("ensure root session");
    repo.ensure_approval_request(crate::session::repository::NewApprovalRequestRecord {
        approval_request_id: "apr-core-replay".to_owned(),
        session_id: "root-session".to_owned(),
        turn_id: "turn-core-replay".to_owned(),
        tool_call_id: "call-core-replay".to_owned(),
        tool_name: "provider.switch".to_owned(),
        approval_key: "tool:provider.switch".to_owned(),
        request_payload_json: json!({
            "session_id": "root-session",
            "turn_id": "turn-core-replay",
            "tool_call_id": "call-core-replay",
            "tool_name": "provider.switch",
            "args_json": {
                "selector": "openai"
            },
            "source": "test",
            "execution_kind": "core",
        }),
        governance_snapshot_json: json!({
            "rule_id": "session_tool_consent_auto_blocked",
        }),
    })
    .expect("seed core approval request");
    let approval_request = repo
        .load_approval_request("apr-core-replay")
        .expect("load approval request")
        .expect("approval request row");
    let kernel_ctx =
        bootstrap_test_kernel_context("approval-core-replay", 60).expect("kernel context");
    let fallback = DefaultAppToolDispatcher::new(memory_config.clone(), ToolConfig::default());
    let approval_runtime = CoordinatorApprovalResolutionRuntime::new(
        &config,
        &runtime,
        &fallback,
        ConversationRuntimeBinding::kernel(&kernel_ctx),
    );

    let error = approval_runtime
        .replay_approved_request(&approval_request)
        .await
        .expect_err("provider.switch should fail at core execution");
    assert!(
        error.contains("provider.switch requires a resolved runtime config path"),
        "expected core execution failure, got: {error}"
    );
}
