use super::*;

#[tokio::test]
async fn control_snapshot_returns_snapshot_payload() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    manager.set_session_count(7);
    let router = build_control_plane_router(manager);
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request("GET", "/control/snapshot", &token))
        .await
        .expect("snapshot response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let snapshot: ControlPlaneSnapshotResponse =
        serde_json::from_slice(&body).expect("snapshot json");
    assert_eq!(snapshot.snapshot.session_count, 7);
}

#[tokio::test]
async fn control_events_returns_recent_events_with_limit() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
    let _ = manager.record_health_changed(true, serde_json::json!({ "idx": 2 }));
    let _ = manager.record_session_message(serde_json::json!({ "idx": 3 }), true);
    let router = build_control_plane_router(manager);
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request("GET", "/control/events?limit=2", &token))
        .await
        .expect("events response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let events: ControlPlaneRecentEventsResponse =
        serde_json::from_slice(&body).expect("events json");
    assert_eq!(events.events.len(), 2);
    assert_eq!(events.events[0].seq, 1);
    assert_eq!(events.events[1].seq, 2);
    assert_eq!(events.events[0].payload["idx"], 1);
    assert_eq!(events.events[1].payload["idx"], 2);
}

#[tokio::test]
async fn control_events_can_include_targeted_records_when_requested() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let _ = manager.record_session_message(serde_json::json!({ "kind": "broadcast" }), false);
    let _ = manager.record_session_message(serde_json::json!({ "kind": "targeted" }), true);
    let router = build_control_plane_router(manager);
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request(
            "GET",
            "/control/events?limit=10&include_targeted=true",
            &token,
        ))
        .await
        .expect("events response");
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let events: ControlPlaneRecentEventsResponse =
        serde_json::from_slice(&body).expect("events json");
    assert_eq!(events.events.len(), 2);
    assert_eq!(events.events[1].payload["kind"], "targeted");
}

#[tokio::test]
async fn control_events_supports_after_seq_long_poll() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
    let router = build_control_plane_router(manager.clone());
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;

    let request_future = {
        let router = router.clone();
        let token = token.clone();
        tokio::spawn(async move {
            router
                .oneshot(bearer_request(
                    "GET",
                    "/control/events?after_seq=1&timeout_ms=1000",
                    &token,
                ))
                .await
        })
    };

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let _ = manager.record_health_changed(true, serde_json::json!({ "idx": 2 }));

    let response = request_future
        .await
        .expect("join")
        .expect("events response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let events: ControlPlaneRecentEventsResponse =
        serde_json::from_slice(&body).expect("events json");
    assert_eq!(events.events.len(), 1);
    assert_eq!(events.events[0].payload["idx"], 2);
    assert_eq!(events.events[0].seq, 2);
}

#[tokio::test]
async fn control_events_after_seq_returns_empty_on_timeout() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
    let router = build_control_plane_router(manager);
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;

    let response = router
        .oneshot(bearer_request(
            "GET",
            "/control/events?after_seq=1&timeout_ms=20",
            &token,
        ))
        .await
        .expect("events response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let events: ControlPlaneRecentEventsResponse =
        serde_json::from_slice(&body).expect("events json");
    assert!(events.events.is_empty());
}

#[tokio::test]
async fn control_subscribe_rejects_missing_token() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router(manager);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/control/subscribe")
                .method("GET")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("subscribe response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn control_subscribe_returns_sse_content_type() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
    let router = build_control_plane_router(manager);
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request(
            "GET",
            "/control/subscribe?after_seq=0",
            &token,
        ))
        .await
        .expect("subscribe response");
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(content_type.starts_with("text/event-stream"));
}

#[tokio::test]
async fn control_subscribe_stream_yields_backlog_event() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
    let _ = manager.record_health_changed(true, serde_json::json!({ "idx": 2 }));
    let stream = control_plane_subscribe_stream(manager, 1, true);
    let mut stream = Box::pin(stream);
    let next = stream.next().await.expect("stream item");
    let event = next.expect("event");
    let event_debug = format!("{event:?}");
    assert!(!event_debug.is_empty());
}

#[tokio::test]
async fn control_subscribe_stream_yields_live_event_after_wait() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let _ = manager.record_presence_changed(1, serde_json::json!({ "idx": 1 }));
    let stream = control_plane_subscribe_stream(manager.clone(), 1, true);
    let mut stream = Box::pin(stream);

    let waiter = tokio::spawn(async move { stream.next().await });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let _ = manager.record_health_changed(true, serde_json::json!({ "idx": 2 }));

    let next = waiter.await.expect("join").expect("stream item");
    let event = next.expect("event");
    let event_debug = format!("{event:?}");
    assert!(!event_debug.is_empty());
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn control_snapshot_uses_repository_backed_session_counts_when_available() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    manager.set_runtime_ready(true);
    manager.set_session_count(99);
    let (repository_view, acp_view) = seeded_control_plane_views("snapshot-repo");
    let router =
        build_control_plane_router_with_views(manager, Some(repository_view), Some(acp_view));
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request("GET", "/control/snapshot", &token))
        .await
        .expect("snapshot response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let snapshot: ControlPlaneSnapshotResponse =
        serde_json::from_slice(&body).expect("snapshot json");
    assert_eq!(snapshot.snapshot.session_count, 2);
    assert_eq!(snapshot.snapshot.pending_approval_count, 1);
    assert_eq!(snapshot.snapshot.acp_session_count, 1);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn session_list_returns_visible_repository_sessions() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router_with_views(
        manager,
        Some(seeded_repository_view("session-list")),
        None,
    );
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request("GET", "/session/list?limit=10", &token))
        .await
        .expect("session list response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let sessions: ControlPlaneSessionListResponse =
        serde_json::from_slice(&body).expect("session list json");
    assert_eq!(sessions.current_session_id, "root-session");
    assert_eq!(sessions.matched_count, 2);
    assert_eq!(sessions.returned_count, 2);
    assert!(
        sessions
            .sessions
            .iter()
            .any(|session| session.session_id == "root-session")
    );
    let child = sessions
        .sessions
        .iter()
        .find(|session| session.session_id == "child-session")
        .expect("child session");
    assert_eq!(child.workflow.workflow_id, "root-session");
    assert_eq!(
        child.workflow.task.as_deref(),
        Some("research control plane parity")
    );
    assert_eq!(child.workflow.phase.as_deref(), Some("execute"));
    assert_eq!(
        child
            .workflow
            .binding
            .as_ref()
            .expect("workflow binding")
            .mode,
        "mutating_capable"
    );
    let continuity = child
        .workflow
        .runtime_self_continuity
        .as_ref()
        .expect("runtime self continuity");
    assert!(continuity.present);
    assert!(continuity.resolved_identity_present);
    assert!(continuity.session_profile_projection_present);
    assert!(
        !sessions
            .sessions
            .iter()
            .any(|session| session.session_id == "hidden-root")
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn session_read_returns_repository_observation_for_visible_session() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router_with_views(
        manager,
        Some(seeded_repository_view("session-read")),
        None,
    );
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request(
            "GET",
            "/session/read?session_id=child-session&recent_event_limit=10",
            &token,
        ))
        .await
        .expect("session read response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let session: ControlPlaneSessionReadResponse =
        serde_json::from_slice(&body).expect("session read json");
    assert_eq!(session.current_session_id, "root-session");
    assert_eq!(session.observation.session.session_id, "child-session");
    assert_eq!(
        session.observation.session.workflow.workflow_id,
        "root-session"
    );
    assert_eq!(
        session.observation.session.workflow.task.as_deref(),
        Some("research control plane parity")
    );
    assert_eq!(
        session.observation.session.workflow.phase.as_deref(),
        Some("execute")
    );
    assert_eq!(
        session
            .observation
            .session
            .workflow
            .binding
            .as_ref()
            .expect("workflow binding")
            .execution_surface,
        "delegate.async"
    );
    let continuity = session
        .observation
        .session
        .workflow
        .runtime_self_continuity
        .as_ref()
        .expect("runtime self continuity");
    assert!(continuity.present);
    assert!(continuity.resolved_identity_present);
    assert!(continuity.session_profile_projection_present);
    assert_eq!(session.observation.recent_events.len(), 1);
    assert_eq!(
        session.observation.recent_events[0].event_kind,
        "delegate_started"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn task_list_returns_visible_background_tasks() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router_with_views(
        manager,
        Some(seeded_repository_view("task-list")),
        None,
    );
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request("GET", "/task/list?limit=10", &token))
        .await
        .expect("task list response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let tasks: ControlPlaneTaskListResponse =
        serde_json::from_slice(&body).expect("task list json");
    assert_eq!(tasks.current_session_id, "root-session");
    assert_eq!(tasks.matched_count, 1);
    assert_eq!(tasks.returned_count, 1);
    let task = tasks.tasks.first().expect("task summary");
    assert_eq!(task.task_id, "child-session");
    assert_eq!(task.task_session_id, "child-session");
    assert_eq!(task.owner_session_id, "child-session");
    assert_eq!(task.workflow.workflow_id, "root-session");
    assert_eq!(
        task.workflow.task.as_deref(),
        Some("research control plane parity")
    );
    assert_eq!(task.workflow.phase.as_deref(), Some("execute"));
    assert_eq!(
        task.workflow
            .binding
            .as_ref()
            .expect("workflow binding")
            .task_id,
        "child-session"
    );
    assert_eq!(task.delegate_mode.as_deref(), Some("async"));
    assert_eq!(task.requested_tool_ids, vec!["/read".to_owned()]);
    assert_eq!(task.visible_requested_tool_ids, vec!["read".to_owned()]);
    assert_eq!(task.effective_tool_ids, vec!["/read".to_owned()]);
    assert_eq!(task.visible_effective_tool_ids, vec!["read".to_owned()]);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn task_read_returns_visible_background_task_detail() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router_with_views(
        manager,
        Some(seeded_repository_view("task-read")),
        None,
    );
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorRead]),
    )
    .await;
    let response = router
        .oneshot(bearer_request(
            "GET",
            "/task/read?task_id=child-session",
            &token,
        ))
        .await
        .expect("task read response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let task: ControlPlaneTaskReadResponse = serde_json::from_slice(&body).expect("task read json");
    assert_eq!(task.current_session_id, "root-session");
    assert_eq!(task.task.task_id, "child-session");
    assert_eq!(task.task.task_session_id, "child-session");
    assert_eq!(task.task.owner_session_id, "child-session");
    assert_eq!(task.task.workflow.workflow_id, "root-session");
    assert_eq!(
        task.task
            .workflow
            .binding
            .as_ref()
            .expect("workflow binding")
            .worktree
            .as_ref()
            .expect("worktree binding")
            .worktree_id,
        "child-session"
    );
    assert_eq!(task.task.delegate_phase.as_deref(), Some("running"));
    assert_eq!(task.task.approval_request_count, 1);
    assert_eq!(task.task.requested_tool_ids, vec!["/read".to_owned()]);
    assert_eq!(
        task.task.visible_requested_tool_ids,
        vec!["read".to_owned()]
    );
    assert_eq!(task.task.effective_tool_ids, vec!["/read".to_owned()]);
    assert_eq!(
        task.task.visible_effective_tool_ids,
        vec!["read".to_owned()]
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn task_routes_reject_insufficient_scope() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router_with_views(
        manager,
        Some(seeded_repository_view("task-scope")),
        None,
    );
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorPairing]),
    )
    .await;

    let list_response = router
        .clone()
        .oneshot(bearer_request("GET", "/task/list?limit=10", &token))
        .await
        .expect("task list response");
    assert_eq!(list_response.status(), StatusCode::FORBIDDEN);

    let read_response = router
        .oneshot(bearer_request(
            "GET",
            "/task/read?task_id=child-session",
            &token,
        ))
        .await
        .expect("task read response");
    assert_eq!(read_response.status(), StatusCode::FORBIDDEN);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn approval_list_returns_only_visible_requests() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router_with_views(
        manager,
        Some(seeded_repository_view("approval-list")),
        None,
    );
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorApprovals]),
    )
    .await;
    let response = router
        .oneshot(bearer_request(
            "GET",
            "/approval/list?status=pending&limit=10",
            &token,
        ))
        .await
        .expect("approval list response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let approvals: ControlPlaneApprovalListResponse =
        serde_json::from_slice(&body).expect("approval list json");
    assert_eq!(approvals.current_session_id, "root-session");
    assert_eq!(approvals.matched_count, 1);
    assert_eq!(approvals.returned_count, 1);
    assert_eq!(approvals.approvals[0].approval_request_id, "apr-visible");
    assert_eq!(
        approvals.approvals[0].status,
        ControlPlaneApprovalRequestStatus::Pending
    );
    assert_eq!(
        approvals.approvals[0].reason.as_deref(),
        Some("governed_tool_requires_approval")
    );
    assert_eq!(
        approvals.approvals[0].visible_tool_name.as_deref(),
        Some("delegate")
    );
    assert_eq!(
        approvals.approvals[0].request_summary.as_ref(),
        Some(&serde_json::json!({
            "tool": "delegate",
            "request": {}
        }))
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn acp_session_list_returns_only_visible_sessions() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let (_repository_view, acp_view) = seeded_control_plane_views("acp-list");
    let router = build_control_plane_router_with_views(manager, None, Some(acp_view));
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorAcp]),
    )
    .await;
    let response = router
        .oneshot(bearer_request("GET", "/acp/session/list?limit=10", &token))
        .await
        .expect("ACP session list response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let sessions: ControlPlaneAcpSessionListResponse =
        serde_json::from_slice(&body).expect("ACP session list json");
    assert_eq!(sessions.current_session_id, "root-session");
    assert_eq!(sessions.matched_count, 1);
    assert_eq!(sessions.returned_count, 1);
    assert_eq!(
        sessions.sessions[0].session_key,
        "agent:codex:child-session"
    );
    assert_eq!(
        sessions.sessions[0]
            .binding
            .as_ref()
            .expect("binding")
            .route_session_id,
        "child-session"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn acp_session_read_returns_live_status_for_visible_session() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let (_repository_view, acp_view) = seeded_control_plane_views("acp-read");
    let router = build_control_plane_router_with_views(manager, None, Some(acp_view));
    let token = connect_token(
        &router,
        std::collections::BTreeSet::from([ControlPlaneScope::OperatorAcp]),
    )
    .await;
    let response = router
        .oneshot(bearer_request(
            "GET",
            "/acp/session/read?session_key=agent:codex:child-session",
            &token,
        ))
        .await
        .expect("ACP session read response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body bytes");
    let session: ControlPlaneAcpSessionReadResponse =
        serde_json::from_slice(&body).expect("ACP session read json");
    assert_eq!(session.current_session_id, "root-session");
    assert_eq!(session.metadata.session_key, "agent:codex:child-session");
    assert_eq!(session.status.session_key, "agent:codex:child-session");
    assert_eq!(session.status.state, ControlPlaneAcpSessionState::Ready);
    assert_eq!(
        session.status.mode,
        Some(ControlPlaneAcpSessionMode::Interactive)
    );
    assert!(
        session
            .status
            .last_error
            .as_deref()
            .is_some_and(|error| error.starts_with("status_unavailable:")),
        "expected ACP session read to degrade with status_unavailable when backend is absent"
    );
}

#[tokio::test]
async fn control_snapshot_rejects_missing_token() {
    let manager = Arc::new(mvp::control_plane::ControlPlaneManager::new());
    let router = build_control_plane_router(manager);
    let response = router
        .oneshot(
            Request::builder()
                .uri("/control/snapshot")
                .method("GET")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("snapshot response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
