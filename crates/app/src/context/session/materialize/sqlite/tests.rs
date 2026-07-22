use loong_contracts::{Capabilities, Capability, GovernedSessionMode, ToolPath};
use loong_runtime::tool_plane::ToolRegistration;

use super::*;
use crate::config::{AuditMode, LoongConfig};
use crate::conversation::{
    ConstrainedSubagentExecution, ConstrainedSubagentIsolation, ConstrainedSubagentMode,
    active_skills::{ACTIVE_SKILLS_EVENT_KIND, ActiveSkill, ActiveSkillsState},
};
use crate::session::repository::{
    NewSessionEvent, NewSessionRecord, NewSessionToolPolicyRecord, SessionKind, SessionState,
};

#[allow(clippy::expect_used)]
fn tool_path(segment: &str) -> ToolPath {
    ToolPath::new([segment]).expect("test tool path must be valid")
}

#[test]
fn tool_policy_filter_preserves_runtime_path_and_provider_identity() {
    let path = tool_path("internal.file.read");
    let mut base = ToolView::default();
    base.insert_registration(path.clone(), &ToolRegistration::direct("read"));
    let policy = SessionToolPolicyRecord {
        session_id: "session".to_owned(),
        requested_tool_ids: vec![path.to_string()],
        runtime_narrowing: ToolRuntimeNarrowing::default(),
        updated_at: 1,
    };

    let filtered = apply_session_tool_policy_to_tool_view(base, Some(&policy))
        .expect("canonical policy path should filter the view");

    assert!(filtered.contains_path(&path));
    assert!(!filtered.contains_path(&tool_path("read")));
    assert_eq!(filtered.tool_names().collect::<Vec<_>>(), vec!["read"]);
}

#[test]
fn tool_policy_filter_distinguishes_embedded_dot_from_path_separator() {
    let embedded_dot = tool_path("alpha.beta");
    let split_path = ToolPath::new(["alpha", "beta"]).expect("test tool path must be valid");
    let mut base = ToolView::default();
    base.insert_registration(
        embedded_dot.clone(),
        &ToolRegistration::discoverable("alpha.beta"),
    );
    base.insert_registration(
        split_path.clone(),
        &ToolRegistration::discoverable("alpha_beta_split"),
    );
    let policy = SessionToolPolicyRecord {
        session_id: "session".to_owned(),
        requested_tool_ids: vec![embedded_dot.to_string()],
        runtime_narrowing: ToolRuntimeNarrowing::default(),
        updated_at: 1,
    };

    let filtered = apply_session_tool_policy_to_tool_view(base, Some(&policy))
        .expect("exact canonical path should remain parseable");

    assert!(filtered.contains_path(&embedded_dot));
    assert!(!filtered.contains_path(&split_path));
}

#[test]
fn latest_delegate_anchor_does_not_inherit_fields_from_older_events() {
    let old_execution = ConstrainedSubagentExecution {
        mode: ConstrainedSubagentMode::Inline,
        isolation: ConstrainedSubagentIsolation::Shared,
        owner_kind: None,
        depth: 1,
        max_depth: 3,
        active_children: 0,
        max_active_children: 2,
        timeout_seconds: 60,
        allow_shell_in_child: false,
        child_tool_allowlist: vec!["read".to_owned()],
        capability_ceiling: Capabilities::from([Capability::InvokeTool]),
        workspace_root: Some(PathBuf::from("/old-workspace")),
        runtime_narrowing: ToolRuntimeNarrowing::default(),
        identity: None,
        profile: None,
    };
    let mut latest_execution = old_execution.clone();
    latest_execution.workspace_root = None;
    let events = vec![
        SessionEventRecord {
            id: 1,
            session_id: "child".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root".to_owned()),
            payload_json: old_execution.spawn_payload_with_profile(
                "old",
                None,
                Some(DelegateBuiltinProfile::Research),
            ),
            ts: 1,
        },
        SessionEventRecord {
            id: 2,
            session_id: "child".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root".to_owned()),
            payload_json: latest_execution.spawn_payload("latest", None),
            ts: 2,
        },
    ];

    let anchor = delegate_anchor_snapshot(&events).expect("latest anchor should parse");

    assert_eq!(anchor.execution, Some(latest_execution));
    assert_eq!(anchor.profile, None);
    assert_eq!(anchor.workspace_root, None);
}

#[test]
fn invalid_latest_delegate_anchor_does_not_fall_back() {
    let valid_execution = ConstrainedSubagentExecution {
        mode: ConstrainedSubagentMode::Inline,
        isolation: ConstrainedSubagentIsolation::Shared,
        owner_kind: None,
        depth: 1,
        max_depth: 3,
        active_children: 0,
        max_active_children: 2,
        timeout_seconds: 60,
        allow_shell_in_child: false,
        child_tool_allowlist: vec!["read".to_owned()],
        capability_ceiling: Capabilities::from([Capability::InvokeTool]),
        workspace_root: None,
        runtime_narrowing: ToolRuntimeNarrowing::default(),
        identity: None,
        profile: None,
    };
    let events = vec![
        SessionEventRecord {
            id: 1,
            session_id: "child".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root".to_owned()),
            payload_json: valid_execution.spawn_payload("old", None),
            ts: 1,
        },
        SessionEventRecord {
            id: 2,
            session_id: "child".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root".to_owned()),
            payload_json: serde_json::json!({ "execution": "invalid" }),
            ts: 2,
        },
    ];

    let error = match delegate_anchor_snapshot(&events) {
        Ok(_) => panic!("latest invalid anchor must fail"),
        Err(error) => error,
    };

    assert!(error.contains("event 2 contains invalid execution"));
}

#[cfg(feature = "tool-file")]
#[test]
fn policy_projection_applies_every_ancestor_as_the_next_session_ceiling() {
    let root = crate::test_utils::unique_temp_dir("session-policy-lineage");
    std::fs::create_dir_all(&root).expect("create projection test root");
    let mut config = LoongConfig::default();
    config.audit.mode = AuditMode::InMemory;
    config.memory.sqlite_path = root.join("memory.sqlite3").display().to_string();
    config.tools.delegate.max_depth = 3;
    let repo = SessionRepository::from_memory_config_without_env_overrides(&config.memory)
        .expect("session repository");

    for (session_id, kind, parent_session_id) in [
        ("root", SessionKind::Root, None),
        ("child", SessionKind::DelegateChild, Some("root")),
        ("grandchild", SessionKind::DelegateChild, Some("child")),
    ] {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind,
            parent_session_id: parent_session_id.map(str::to_owned),
            label: Some(session_id.to_owned()),
            state: SessionState::Ready,
        })
        .expect("create persisted session");
    }

    let capability_ceiling = Capabilities::from([
        Capability::InvokeTool,
        Capability::NetworkEgress,
        Capability::MemoryRead,
        Capability::MemoryWrite,
        Capability::FilesystemRead,
        Capability::FilesystemWrite,
    ]);
    for (session_id, parent_session_id, depth) in [("child", "root", 1), ("grandchild", "child", 2)]
    {
        let execution = ConstrainedSubagentExecution {
            mode: ConstrainedSubagentMode::Inline,
            isolation: ConstrainedSubagentIsolation::Shared,
            owner_kind: None,
            depth,
            max_depth: 3,
            active_children: 0,
            max_active_children: 2,
            timeout_seconds: 60,
            allow_shell_in_child: false,
            child_tool_allowlist: vec!["read".to_owned(), "write".to_owned(), "edit".to_owned()],
            capability_ceiling: capability_ceiling.clone(),
            workspace_root: None,
            runtime_narrowing: ToolRuntimeNarrowing::default(),
            identity: None,
            profile: None,
        };
        repo.append_event(NewSessionEvent {
            session_id: session_id.to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some(parent_session_id.to_owned()),
            payload_json: execution.spawn_payload("test policy lineage", Some(session_id)),
        })
        .expect("append typed delegate authority");
    }

    for (session_id, requested_tool_ids, max_browser_sessions) in [
        ("root", vec!["/read", "/write", "/edit"], 4),
        ("child", vec!["/read", "/write"], 3),
        ("grandchild", vec!["/write", "/edit"], 2),
    ] {
        repo.upsert_session_tool_policy(NewSessionToolPolicyRecord {
            session_id: session_id.to_owned(),
            requested_tool_ids: requested_tool_ids.into_iter().map(str::to_owned).collect(),
            runtime_narrowing: ToolRuntimeNarrowing {
                browser: crate::tools::runtime_config::BrowserRuntimeNarrowing {
                    max_sessions: Some(max_browser_sessions),
                    ..Default::default()
                },
                ..Default::default()
            },
        })
        .expect("persist session tool policy");
    }

    let runtime = crate::runtime::bootstrap_runtime_with_config(&config).expect("runtime");
    let projection = materialize_tool_policy_projection(runtime.as_ref(), &config, "grandchild")
        .expect("materialize policy projection");

    assert!(projection.base_tool_view.contains("read"));
    assert!(projection.base_tool_view.contains("write"));
    assert!(!projection.base_tool_view.contains("edit"));
    assert!(!projection.effective_tool_view.contains("read"));
    assert!(projection.effective_tool_view.contains("write"));
    assert!(!projection.effective_tool_view.contains("edit"));

    let executable = Session::from_config(
        runtime.as_ref(),
        &config,
        "grandchild",
        "test-agent",
        GovernedSessionMode::MutatingCapable,
    )
    .expect("materialize executable session");
    assert_eq!(projection.effective_tool_view, executable.tool_view);
    assert_eq!(executable.tool_runtime_config.browser.max_sessions, 2);
    assert_eq!(
        executable
            .resolved_runtime_narrowing()
            .and_then(|narrowing| narrowing.browser.max_sessions),
        Some(2)
    );

    repo.append_event(NewSessionEvent {
        session_id: "root".to_owned(),
        event_kind: ACTIVE_SKILLS_EVENT_KIND.to_owned(),
        actor_session_id: Some("root".to_owned()),
        payload_json: serde_json::json!({
            "active_skills": ActiveSkillsState {
                skills: vec![ActiveSkill {
                    skill_id: "lineage-guard".to_owned(),
                    display_name: "Lineage Guard".to_owned(),
                    instructions: "protect descendant authority".to_owned(),
                    skill_root: None,
                    allowed_tools: Vec::new(),
                    blocked_tools: vec!["write".to_owned()],
                }],
            },
        }),
    })
    .expect("persist ancestor active-skill restriction");

    let skill_narrowed = executable
        .rematerialize(runtime.as_ref(), &config)
        .expect("apply ancestor active-skill restriction");
    assert!(!skill_narrowed.tool_view.contains("write"));

    repo.append_event(NewSessionEvent {
        session_id: "root".to_owned(),
        event_kind: ACTIVE_SKILLS_EVENT_KIND.to_owned(),
        actor_session_id: Some("root".to_owned()),
        payload_json: serde_json::json!({
            "active_skills": ActiveSkillsState { skills: Vec::new() },
        }),
    })
    .expect("persist cleared ancestor active-skill state");
    let skill_removal_attempt = skill_narrowed
        .rematerialize(runtime.as_ref(), &config)
        .expect("rematerialize after ancestor active-skill removal");
    assert!(
        !skill_removal_attempt.tool_view.contains("write"),
        "removing durable policy cannot restore live Session authority"
    );

    repo.upsert_session_tool_policy(NewSessionToolPolicyRecord {
        session_id: "root".to_owned(),
        requested_tool_ids: vec!["/read".to_owned()],
        runtime_narrowing: ToolRuntimeNarrowing {
            browser: crate::tools::runtime_config::BrowserRuntimeNarrowing {
                max_sessions: Some(1),
                ..Default::default()
            },
            ..Default::default()
        },
    })
    .expect("tighten root session policy");

    let rematerialized = executable
        .rematerialize(runtime.as_ref(), &config)
        .expect("rematerialize grandchild through updated lineage");

    assert!(!rematerialized.tool_view.contains("write"));
    assert_eq!(rematerialized.tool_runtime_config.browser.max_sessions, 1);
    assert_eq!(
        rematerialized
            .resolved_runtime_narrowing()
            .and_then(|narrowing| narrowing.browser.max_sessions),
        Some(1)
    );
    assert!(std::sync::Arc::ptr_eq(
        &executable.memory_backend,
        &rematerialized.memory_backend
    ));

    repo.delete_session_tool_policy("root")
        .expect("delete ancestor policy");
    let policy_removal_attempt = rematerialized
        .rematerialize(runtime.as_ref(), &config)
        .expect("rematerialize after ancestor policy removal");
    assert!(!policy_removal_attempt.tool_view.contains("write"));
    assert_eq!(
        policy_removal_attempt
            .resolved_runtime_narrowing()
            .and_then(|narrowing| narrowing.browser.max_sessions),
        Some(1),
        "removing durable policy cannot widen the live runtime contract"
    );
}

#[test]
fn policy_projection_rejects_broken_lineage() {
    let root = crate::test_utils::unique_temp_dir("session-policy-broken-lineage");
    std::fs::create_dir_all(&root).expect("create projection test root");
    let mut config = LoongConfig::default();
    config.audit.mode = AuditMode::InMemory;
    config.memory.sqlite_path = root.join("memory.sqlite3").display().to_string();
    let repo = SessionRepository::from_memory_config_without_env_overrides(&config.memory)
        .expect("session repository");
    repo.create_session(NewSessionRecord {
        session_id: "child".to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some("missing-parent".to_owned()),
        label: Some("Child".to_owned()),
        state: SessionState::Ready,
    })
    .expect("create child with missing parent");

    let runtime = crate::runtime::bootstrap_runtime_with_config(&config).expect("runtime");
    let error = match materialize_tool_policy_projection(runtime.as_ref(), &config, "child") {
        Ok(_) => panic!("policy projection must fail closed on broken lineage"),
        Err(error) => error,
    };

    assert!(
        error.contains("references missing session `missing-parent`"),
        "error={error}"
    );
}

#[test]
fn typed_session_materialization_rejects_legacy_only_identity() {
    let root = crate::test_utils::unique_temp_dir("session-legacy-only-materialization");
    std::fs::create_dir_all(&root).expect("create materialization test root");
    let mut config = LoongConfig::default();
    config.audit.mode = AuditMode::InMemory;
    config.memory.sqlite_path = root.join("memory.sqlite3").display().to_string();
    let memory_config =
        MemoryRuntimeConfig::from_memory_config_without_env_overrides(&config.memory);
    crate::memory::append_turn_direct("legacy-only", "user", "old turn", &memory_config)
        .expect("seed legacy turn without canonical Session row");
    let runtime = crate::runtime::bootstrap_runtime_with_config(&config).expect("runtime");

    let error = Session::from_config(
        runtime.as_ref(),
        &config,
        "legacy-only",
        "test-agent",
        GovernedSessionMode::MutatingCapable,
    )
    .expect_err("legacy summary must not become typed Session authority");

    assert!(
        error.contains("requires a canonical session row for existing identity `legacy-only`"),
        "error={error}"
    );
}

#[test]
fn typed_session_materialization_distinguishes_new_roots_from_orphan_evidence() {
    let root = crate::test_utils::unique_temp_dir("session-orphan-materialization");
    std::fs::create_dir_all(&root).expect("create materialization test root");
    let sqlite_path = root.join("memory.sqlite3");
    let mut config = LoongConfig::default();
    config.audit.mode = AuditMode::InMemory;
    config.memory.sqlite_path = sqlite_path.display().to_string();
    let repo = SessionRepository::from_memory_config_without_env_overrides(&config.memory)
        .expect("session repository");

    for session_id in ["orphan-policy", "orphan-event"] {
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: None,
            state: SessionState::Ready,
        })
        .expect("create canonical test Session");
    }
    repo.upsert_session_tool_policy(NewSessionToolPolicyRecord {
        session_id: "orphan-policy".to_owned(),
        requested_tool_ids: vec!["/read".to_owned()],
        runtime_narrowing: ToolRuntimeNarrowing::default(),
    })
    .expect("persist orphaned policy evidence");
    repo.append_event(NewSessionEvent {
        session_id: "orphan-event".to_owned(),
        event_kind: "test_authority_event".to_owned(),
        actor_session_id: None,
        payload_json: serde_json::json!({"authority": "existing"}),
    })
    .expect("persist orphaned event evidence");

    let conn = rusqlite::Connection::open(&sqlite_path).expect("open test database");
    conn.execute_batch(
        "DELETE FROM session_heads WHERE session_id IN ('orphan-policy', 'orphan-event');
         DELETE FROM session_nodes WHERE session_id IN ('orphan-policy', 'orphan-event');
         DELETE FROM sessions WHERE session_id IN ('orphan-policy', 'orphan-event');",
    )
    .expect("remove canonical identity while retaining orphan evidence");

    let runtime = crate::runtime::bootstrap_runtime_with_config(&config).expect("runtime");
    Session::from_config(
        runtime.as_ref(),
        &config,
        "fresh-root",
        "test-agent",
        GovernedSessionMode::MutatingCapable,
    )
    .expect("an identity with no durable evidence may become a new root");

    for session_id in ["orphan-policy", "orphan-event"] {
        let error = Session::from_config(
            runtime.as_ref(),
            &config,
            session_id,
            "test-agent",
            GovernedSessionMode::MutatingCapable,
        )
        .expect_err("orphan authority evidence must not become a fresh root");
        assert!(
            error.contains(&format!(
                "requires a canonical session row for existing identity `{session_id}`"
            )),
            "error={error}"
        );
    }
}
