use super::*;

#[cfg(feature = "memory-sqlite")]
#[test]
fn persist_runtime_self_continuity_for_compaction_merges_live_and_stored_delegate_continuity() {
    let workspace_root = unique_workspace_root("merged-runtime-self-continuity");
    let memory_config = sqlite_memory_config("merged-runtime-self-continuity");
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    let root_session_id = "root-session";
    let child_session_id = "delegate:child-session";
    let live_agents_text = "Keep standing instructions visible.";
    let stored_identity_text = "# Identity\n\n- Name: Stored continuity identity";
    let mut config = LoongConfig::default();

    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .display()
        .to_string();
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(workspace_root.join("AGENTS.md"), live_agents_text).expect("write AGENTS");
    config.memory.sqlite_path = sqlite_path;
    config.tools.file_root = Some(workspace_root.display().to_string());

    repo.create_session(NewSessionRecord {
        session_id: root_session_id.to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");
    repo.create_session(NewSessionRecord {
        session_id: child_session_id.to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some(root_session_id.to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child session");

    let stored_continuity = runtime_self_continuity::RuntimeSelfContinuity {
        workspace_guidance: crate::workspace_guidance::WorkspaceGuidanceModel::default(),
        runtime_self: crate::runtime_self::RuntimeSelfModel {
            identity_context: vec![stored_identity_text.to_owned()],
            ..Default::default()
        },
        resolved_identity: Some(crate::runtime_identity::ResolvedRuntimeIdentity {
            source: crate::runtime_identity::RuntimeIdentitySource::LegacyProfileNoteImport,
            content: stored_identity_text.to_owned(),
        }),
        session_profile_projection: None,
    };
    repo.append_event(NewSessionEvent {
        session_id: child_session_id.to_owned(),
        event_kind: "delegate_started".to_owned(),
        actor_session_id: Some(root_session_id.to_owned()),
        payload_json: json!({
            "runtime_self_continuity": stored_continuity,
        }),
    })
    .expect("append delegate event");

    persist_runtime_self_continuity_for_compaction(&config, child_session_id)
        .expect("persist merged runtime self continuity");

    let recent_events = repo
        .list_recent_events(child_session_id, 10)
        .expect("list recent events");
    let persisted_event = recent_events
        .iter()
        .rev()
        .find(|event| {
            event.event_kind == runtime_self_continuity::RUNTIME_SELF_CONTINUITY_EVENT_KIND
        })
        .expect("persisted continuity event");
    let persisted_continuity = runtime_self_continuity::runtime_self_continuity_from_event_payload(
        &persisted_event.payload_json,
    )
    .expect("decode persisted continuity payload");

    assert_eq!(
        persisted_continuity.workspace_guidance.entries,
        vec![live_agents_text.to_owned()]
    );
    assert!(
        persisted_continuity
            .runtime_self
            .standing_instructions
            .is_empty()
    );
    assert_eq!(
        persisted_continuity.runtime_self.identity_context,
        vec![stored_identity_text.to_owned()]
    );
    assert_eq!(
        persisted_continuity
            .resolved_identity
            .as_ref()
            .map(|value| value.content.as_str()),
        Some(stored_identity_text)
    );

    let _ = std::fs::remove_dir_all(&workspace_root);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn persist_runtime_self_continuity_for_compaction_reconstructs_legacy_delegate_session_row() {
    let workspace_root = unique_workspace_root("legacy-delegate-session-row");
    let memory_config = sqlite_memory_config("legacy-delegate-session-row");
    let repo = SessionRepository::new(&memory_config).expect("session repository");
    let root_session_id = "root-session";
    let child_session_id = "delegate:legacy-child";
    let mut config = LoongConfig::default();

    let sqlite_path = memory_config
        .sqlite_path
        .as_ref()
        .expect("sqlite path")
        .clone();
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(
        workspace_root.join("AGENTS.md"),
        "Keep continuity explicit.",
    )
    .expect("write AGENTS");
    config.memory.sqlite_path = sqlite_path.display().to_string();
    config.tools.file_root = Some(workspace_root.display().to_string());

    repo.create_session(NewSessionRecord {
        session_id: root_session_id.to_owned(),
        kind: SessionKind::Root,
        parent_session_id: None,
        label: Some("Root".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create root session");

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite connection");
    conn.execute(
        "INSERT INTO turns(session_id, session_turn_index, role, content, ts)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![child_session_id, 1_i64, "assistant", "legacy turn", 1_i64],
    )
    .expect("insert legacy turn");
    conn.execute(
        "INSERT INTO session_events(session_id, event_kind, actor_session_id, payload_json, ts)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            child_session_id,
            "delegate_started",
            root_session_id,
            json!({}).to_string(),
            2_i64
        ],
    )
    .expect("insert legacy delegate event");
    drop(conn);

    persist_runtime_self_continuity_for_compaction(&config, child_session_id)
        .expect("persist runtime self continuity");

    let reconstructed_session = repo
        .load_session(child_session_id)
        .expect("load reconstructed session")
        .expect("reconstructed session row");

    assert_eq!(reconstructed_session.kind, SessionKind::DelegateChild);
    assert_eq!(
        reconstructed_session.parent_session_id.as_deref(),
        Some(root_session_id)
    );

    let _ = std::fs::remove_dir_all(&workspace_root);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn maybe_compact_context_fails_open_when_runtime_self_continuity_persist_cannot_reconstruct_delegate_lineage()
 {
    let workspace_root = unique_workspace_root("compaction-fail-open");
    let sqlite_path = unique_sqlite_path("compaction-fail-open");
    let runtime = RecordingCompactRuntime::default();
    let mut config = LoongConfig::default();

    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(
        workspace_root.join("AGENTS.md"),
        "Keep continuity explicit.",
    )
    .expect("write AGENTS");
    config.memory.sqlite_path = sqlite_path.display().to_string();
    config.tools.file_root = Some(workspace_root.display().to_string());
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);
    config.conversation.compact_fail_open = true;

    let kernel_ctx = bootstrap_test_kernel_context("turn-coordinator-compaction", 3600)
        .expect("bootstrap kernel context");
    let binding = ConversationRuntimeBinding::from_optional_kernel_context(Some(&kernel_ctx));
    let runtime_handle = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    let messages = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "trigger compaction"}),
    ];
    let outcome = runtime_handle.block_on(maybe_compact_context(
        &config,
        &runtime,
        "delegate:missing-lineage",
        &messages,
        Some(16),
        binding,
        false,
    ));

    assert_eq!(
        outcome.expect("compaction should fail open"),
        ContextCompactionOutcome::FailedOpen
    );
    let compact_calls = runtime.compact_calls.lock().expect("compact lock");
    assert_eq!(*compact_calls, 0);

    let _ = std::fs::remove_dir_all(&workspace_root);
    let _ = std::fs::remove_file(&sqlite_path);
}

#[cfg(feature = "memory-sqlite")]
#[test]
fn maybe_compact_context_fails_open_when_durable_flush_cannot_write_workspace_export() {
    let workspace_root_parent = unique_workspace_root("compaction-durable-flush-fail-open");
    let workspace_root_file = workspace_root_parent.join("workspace-root-file");
    let sqlite_path = unique_sqlite_path("compaction-durable-flush-fail-open");
    let runtime = RecordingCompactRuntime::default();
    let mut config = LoongConfig::default();

    std::fs::create_dir_all(&workspace_root_parent).expect("create workspace root parent");
    std::fs::write(
        workspace_root_parent.join("AGENTS.md"),
        "Keep continuity explicit.",
    )
    .expect("write AGENTS");
    std::fs::write(&workspace_root_file, "not a workspace directory")
        .expect("write workspace root file");
    config.memory.sqlite_path = sqlite_path.display().to_string();
    config.tools.file_root = Some(workspace_root_file.display().to_string());
    config.memory.profile = crate::config::MemoryProfile::WindowPlusSummary;
    config.memory.sliding_window = 1;
    config.conversation.compact_min_messages = Some(1);
    config.conversation.compact_trigger_estimated_tokens = Some(1);
    config.conversation.compact_fail_open = true;

    let runtime_memory_config =
        crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
    crate::memory::append_turn_direct(
        "session-durable-flush-fail-open",
        "user",
        "remember the deployment cutoff",
        &runtime_memory_config,
    )
    .expect("append user turn");
    crate::memory::append_turn_direct(
        "session-durable-flush-fail-open",
        "assistant",
        "deployment cutoff is tonight",
        &runtime_memory_config,
    )
    .expect("append assistant turn");
    crate::memory::append_turn_direct(
        "session-durable-flush-fail-open",
        "user",
        "who is on call",
        &runtime_memory_config,
    )
    .expect("append second user turn");
    crate::memory::append_turn_direct(
        "session-durable-flush-fail-open",
        "assistant",
        "ops is on call",
        &runtime_memory_config,
    )
    .expect("append second assistant turn");

    let kernel_ctx = bootstrap_test_kernel_context("turn-coordinator-compaction", 3600)
        .expect("bootstrap kernel context");
    let binding = ConversationRuntimeBinding::from_optional_kernel_context(Some(&kernel_ctx));
    let runtime_handle = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");
    let messages = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "trigger compaction"}),
    ];
    let outcome = runtime_handle.block_on(maybe_compact_context(
        &config,
        &runtime,
        "session-durable-flush-fail-open",
        &messages,
        Some(16),
        binding,
        false,
    ));

    assert_eq!(
        outcome.expect("compaction should fail open"),
        ContextCompactionOutcome::FailedOpen
    );
    let compact_calls = runtime.compact_calls.lock().expect("compact lock");
    assert_eq!(*compact_calls, 0);

    let _ = std::fs::remove_dir_all(&workspace_root_parent);
    let _ = std::fs::remove_file(&sqlite_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn compact_session_uses_session_context_tool_view_and_turn_like_build_flags() {
    let mut config = LoongConfig::default();
    let sqlite_path = unique_sqlite_path("compact-session-build-messages");
    let _ = std::fs::remove_file(&sqlite_path);
    config.memory.sqlite_path = sqlite_path.display().to_string();

    let memory_config = store::session_store_config_from_memory_config(&config.memory);
    crate::session::store::append_session_turn_direct(
        "compact-session-build-messages",
        "user",
        "remember this detail",
        &memory_config,
    )
    .expect("append user turn");

    let expected_tool_view = crate::tools::ToolView::from_tool_names(["status.inspect"]);
    let runtime = CompactSessionBuildMessagesRuntime::new(expected_tool_view.clone(), false);
    let kernel_ctx = bootstrap_test_kernel_context("compact-session-build-messages", 3600)
        .expect("bootstrap kernel context");
    let binding = ConversationRuntimeBinding::from_optional_kernel_context(Some(&kernel_ctx));
    let coordinator = ConversationTurnCoordinator::new();

    let report = coordinator
        .compact_session_with_runtime(&config, "compact-session-build-messages", &runtime, binding)
        .await
        .expect("manual compaction should succeed");

    assert!(report.was_skipped());

    let build_messages_calls = runtime
        .build_messages_calls
        .lock()
        .expect("build_messages lock should not be poisoned");
    assert_eq!(build_messages_calls.len(), 2);
    assert!(
        build_messages_calls
            .iter()
            .all(|(include_system_prompt, _tool_view)| *include_system_prompt),
        "compact_session should mirror turn assembly by keeping the system prompt enabled"
    );
    assert!(
        build_messages_calls
            .iter()
            .all(|(_include_system_prompt, tool_view)| { *tool_view == expected_tool_view }),
        "compact_session should reuse the session-context tool view for both snapshots"
    );

    let _ = std::fs::remove_file(&sqlite_path);
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn compact_session_skips_when_post_compaction_readback_fails() {
    let mut config = LoongConfig::default();
    let sqlite_path = unique_sqlite_path("compact-session-readback-fail");
    let _ = std::fs::remove_file(&sqlite_path);
    config.memory.sqlite_path = sqlite_path.display().to_string();

    let memory_config = store::session_store_config_from_memory_config(&config.memory);
    crate::session::store::append_session_turn_direct(
        "compact-session-readback-fail",
        "user",
        "keep the context intact",
        &memory_config,
    )
    .expect("append user turn");

    let runtime = CompactSessionBuildMessagesRuntime::new(
        crate::tools::ToolView::from_tool_names(["status.inspect"]),
        true,
    );
    let kernel_ctx = bootstrap_test_kernel_context("compact-session-readback-fail", 3600)
        .expect("bootstrap kernel context");
    let binding = ConversationRuntimeBinding::from_optional_kernel_context(Some(&kernel_ctx));
    let coordinator = ConversationTurnCoordinator::new();

    let report = coordinator
        .compact_session_with_runtime(&config, "compact-session-readback-fail", &runtime, binding)
        .await
        .expect("manual compaction should degrade to skipped");

    assert!(report.was_skipped());
    assert_eq!(
        report.estimated_tokens_after,
        report.estimated_tokens_before
    );

    let build_messages_calls = runtime
        .build_messages_calls
        .lock()
        .expect("build_messages lock should not be poisoned");
    assert_eq!(build_messages_calls.len(), 2);

    let _ = std::fs::remove_file(&sqlite_path);
}
