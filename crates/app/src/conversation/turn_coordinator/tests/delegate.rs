use super::*;

#[cfg(feature = "memory-sqlite")]
fn finalize_recovered_child(
    repo: &SessionRepository,
    expected_state: SessionState,
) -> FinalizeSessionTerminalResult {
    let frozen_result = crate::session::frozen_result::FrozenResult {
        content: crate::session::frozen_result::FrozenContent::Text(
            "delegate_recovered".to_owned(),
        ),
        captured_at: std::time::SystemTime::now(),
        byte_len: "delegate_recovered".len(),
        truncated: false,
    };
    repo.finalize_session_terminal_if_current(
        "child-session",
        expected_state,
        FinalizeSessionTerminalRequest {
            state: SessionState::Failed,
            last_error: Some("delegate_recovered".to_owned()),
            event_kind: RECOVERY_EVENT_KIND.to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            event_payload_json: json!({
                "recovery_kind": "forced_recovery",
                "recovered_state": "failed",
            }),
            outcome_status: "error".to_owned(),
            outcome_payload_json: json!({
                "error": "delegate_recovered"
            }),
            frozen_result: Some(frozen_result),
        },
    )
    .expect("recover child terminal state")
    .expect("recovery should transition child")
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn finalize_delegate_child_terminal_with_recovery_does_not_overwrite_recovered_failure() {
    let memory_config = sqlite_memory_config("recovered-running-child");
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Running,
    })
    .expect("create child session");

    let recovered = finalize_recovered_child(&repo, SessionState::Running);
    assert_eq!(recovered.session.state, SessionState::Failed);
    assert_eq!(recovered.terminal_outcome.status, "error");

    finalize_delegate_child_terminal_with_recovery(
        &repo,
        "child-session",
        FinalizeSessionTerminalRequest {
            state: SessionState::Completed,
            last_error: None,
            event_kind: "delegate_completed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            event_payload_json: json!({
                "turn_count": 1,
                "duration_ms": 12,
            }),
            outcome_status: "ok".to_owned(),
            outcome_payload_json: json!({
                "child_session_id": "child-session",
                "final_output": "late success",
            }),
            frozen_result: None,
        },
    )
    .expect("stale running finalizer should no-op");

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(child.state, SessionState::Failed);
    assert_eq!(child.last_error.as_deref(), Some("delegate_recovered"));

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list child events");
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert!(event_kinds.contains(&RECOVERY_EVENT_KIND));
    assert!(!event_kinds.contains(&"delegate_completed"));

    let terminal_outcome = repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(terminal_outcome.status, "error");
    assert_eq!(terminal_outcome.payload_json["error"], "delegate_recovered");
    assert_eq!(
        terminal_outcome
            .frozen_result
            .expect("frozen result")
            .content,
        crate::session::frozen_result::FrozenContent::Text("delegate_recovered".to_owned())
    );
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn finalize_async_delegate_spawn_failure_does_not_overwrite_recovered_failure() {
    let memory_config = sqlite_memory_config("recovered-ready-child");
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    let execution = ConstrainedSubagentExecution {
        mode: ConstrainedSubagentMode::Async,
        isolation: crate::conversation::ConstrainedSubagentIsolation::Shared,
        owner_kind: None,
        depth: 1,
        max_depth: 1,
        active_children: 0,
        max_active_children: 1,
        timeout_seconds: 60,
        allow_shell_in_child: false,
        child_tool_allowlist: vec![
            "file.read".to_owned(),
            "file.write".to_owned(),
            "file.edit".to_owned(),
        ],
        workspace_root: None,
        runtime_narrowing: crate::tools::runtime_config::ToolRuntimeNarrowing::default(),
        kernel_bound: false,
        identity: None,
        profile: Some(crate::conversation::ConstrainedSubagentProfile::for_child_depth(1, 1)),
    };
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(NewSessionRecord {
        session_id: "child-session".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("root-session".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child session");

    let recovered = finalize_recovered_child(&repo, SessionState::Ready);
    assert_eq!(recovered.session.state, SessionState::Failed);
    assert_eq!(recovered.terminal_outcome.status, "error");

    finalize_async_delegate_spawn_failure(
        &memory_config,
        "child-session",
        "root-session",
        Some("Child".to_owned()),
        None,
        &execution,
        crate::config::ToolConfig::default()
            .delegate
            .max_frozen_bytes,
        "spawn unavailable".to_owned(),
    )
    .expect("stale queued spawn failure finalizer should no-op");

    let child = repo
        .load_session("child-session")
        .expect("load child session")
        .expect("child session row");
    assert_eq!(child.state, SessionState::Failed);
    assert_eq!(child.last_error.as_deref(), Some("delegate_recovered"));

    let events = repo
        .list_recent_events("child-session", 10)
        .expect("list child events");
    let event_kinds: Vec<&str> = events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect();
    assert!(event_kinds.contains(&RECOVERY_EVENT_KIND));
    assert!(!event_kinds.contains(&"delegate_spawn_failed"));

    let terminal_outcome = repo
        .load_terminal_outcome("child-session")
        .expect("load terminal outcome")
        .expect("terminal outcome row");
    assert_eq!(terminal_outcome.status, "error");
    assert_eq!(terminal_outcome.payload_json["error"], "delegate_recovered");
    assert_eq!(
        terminal_outcome
            .frozen_result
            .expect("frozen result")
            .content,
        crate::session::frozen_result::FrozenContent::Text("delegate_recovered".to_owned())
    );
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn finalize_delegate_child_terminal_with_recovery_errors_when_child_session_missing() {
    let memory_config = sqlite_memory_config("missing-running-child");
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");

    let error = finalize_delegate_child_terminal_with_recovery(
        &repo,
        "child-session",
        FinalizeSessionTerminalRequest {
            state: SessionState::Completed,
            last_error: None,
            event_kind: "delegate_completed".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            event_payload_json: json!({
                "turn_count": 1,
                "duration_ms": 12,
            }),
            outcome_status: "ok".to_owned(),
            outcome_payload_json: json!({
                "child_session_id": "child-session",
                "final_output": "late success",
            }),
            frozen_result: None,
        },
    )
    .expect_err("missing child session should not be treated as stale");

    assert!(error.contains("session `child-session` not found"));
    assert!(error.contains("delegate_terminal_recovery_skipped_from_state: missing"));
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn finalize_async_delegate_spawn_failure_with_recovery_errors_when_child_session_missing() {
    let memory_config = sqlite_memory_config("missing-ready-child");
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    let execution = ConstrainedSubagentExecution {
        mode: ConstrainedSubagentMode::Async,
        isolation: crate::conversation::ConstrainedSubagentIsolation::Shared,
        owner_kind: None,
        depth: 1,
        max_depth: 1,
        active_children: 0,
        max_active_children: 1,
        timeout_seconds: 60,
        allow_shell_in_child: false,
        child_tool_allowlist: vec![
            "file.read".to_owned(),
            "file.write".to_owned(),
            "file.edit".to_owned(),
        ],
        workspace_root: None,
        runtime_narrowing: crate::tools::runtime_config::ToolRuntimeNarrowing::default(),
        kernel_bound: false,
        identity: None,
        profile: Some(crate::conversation::ConstrainedSubagentProfile::for_child_depth(1, 1)),
    };
    repo.create_session(NewSessionRecord {
        session_id: "root-session".to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");

    let error = finalize_async_delegate_spawn_failure_with_recovery(
        &memory_config,
        "child-session",
        "root-session",
        Some("Child".to_owned()),
        None,
        &execution,
        crate::config::ToolConfig::default()
            .delegate
            .max_frozen_bytes,
        "spawn unavailable".to_owned(),
    )
    .expect_err("missing child session should not bypass spawn failure recovery");

    assert!(error.contains("session `child-session` not found"));
    assert!(error.contains("delegate_async_spawn_recovery_skipped_from_state: missing"));
    assert_eq!(
        repo.load_session("child-session")
            .expect("load child session"),
        None
    );
}
