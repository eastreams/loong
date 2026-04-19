use std::path::PathBuf;

use serde_json::{Value, json};

use crate::config::LoongConfig;
use crate::conversation::{
    ConstrainedSubagentContractView, ConstrainedSubagentExecution, ConstrainedSubagentIdentity,
    ConstrainedSubagentIsolation, ConstrainedSubagentMode, ConstrainedSubagentOwnerKind,
    ConstrainedSubagentProfile, ConstrainedSubagentTerminalReason, ConversationRuntimeBinding,
    DelegateBuiltinProfile,
};
use crate::runtime_self_continuity::RuntimeSelfContinuity;
use crate::session::frozen_result::capture_frozen_result;
use crate::session::recovery::{
    RECOVERY_EVENT_KIND, build_async_spawn_failure_recovery_payload,
    build_terminal_finalize_recovery_payload,
};
use crate::session::repository::{
    CreateSessionWithEventRequest, FinalizeSessionTerminalRequest, NewSessionRecord, SessionKind,
    SessionRepository, SessionState,
};
use crate::session::store::SessionStoreConfig;
use crate::tools::runtime_config::ToolRuntimeNarrowing;
use crate::trust::{
    delegate_child_trust_event, embed_trust_event_payload, extract_trust_event_payload,
};

use super::session_graph::OperatorSessionGraph;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DelegateChildLifecycleSeed {
    pub execution: ConstrainedSubagentExecution,
    pub request: CreateSessionWithEventRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DelegateChildExecutionPolicy {
    pub isolation: ConstrainedSubagentIsolation,
    pub profile: Option<DelegateBuiltinProfile>,
    pub owner_kind: Option<ConstrainedSubagentOwnerKind>,
    pub timeout_seconds: u64,
    pub allow_shell_in_child: bool,
    pub child_tool_allowlist: Vec<String>,
    pub runtime_narrowing: ToolRuntimeNarrowing,
    pub workspace_root: Option<PathBuf>,
}

pub(crate) fn load_delegate_execution(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<ConstrainedSubagentExecution>, String> {
    let events = repo.list_delegate_lifecycle_events(session_id)?;
    let execution = events.into_iter().rev().find_map(|event| {
        let event_kind = event.event_kind.as_str();
        let is_delegate_lifecycle_event =
            matches!(event_kind, "delegate_queued" | "delegate_started");
        if !is_delegate_lifecycle_event {
            return None;
        }

        ConstrainedSubagentExecution::from_event_payload(&event.payload_json)
    });
    Ok(execution)
}

pub(crate) fn derive_subagent_profile_from_lineage(
    repo: &SessionRepository,
    session_id: &str,
    max_depth: usize,
) -> Result<Option<ConstrainedSubagentProfile>, String> {
    let session_graph = OperatorSessionGraph::new(repo);
    let depth_result = session_graph.lineage_depth(session_id);
    let depth = match depth_result {
        Ok(depth) => depth,
        Err(error)
            if error.starts_with("session_lineage_broken:")
                || error.starts_with("session_lineage_cycle_detected:") =>
        {
            return Ok(None);
        }
        Err(error) => {
            let message = format!(
                "compute session lineage depth for delegate runtime contract failed: {error}"
            );
            return Err(message);
        }
    };

    let profile = ConstrainedSubagentProfile::for_child_depth(depth, max_depth);
    Ok(Some(profile))
}

pub(crate) fn resolve_delegate_child_contract(
    repo: &SessionRepository,
    session_id: &str,
    max_depth: usize,
) -> Result<Option<ConstrainedSubagentContractView>, String> {
    let execution = load_delegate_execution(repo, session_id)?;
    if let Some(execution) = execution {
        let contract = execution.contract_view();
        return Ok(Some(contract));
    }

    let profile = derive_subagent_profile_from_lineage(repo, session_id, max_depth)?;
    let contract = profile.map(ConstrainedSubagentContractView::from_profile);
    Ok(contract)
}

pub(crate) fn next_delegate_child_depth(
    repo: &SessionRepository,
    session_id: &str,
    max_depth: usize,
) -> Result<usize, String> {
    let session_graph = OperatorSessionGraph::new(repo);
    session_graph.next_delegate_child_depth(session_id, max_depth)
}

pub(crate) fn build_delegate_child_lifecycle_seed(
    config: &LoongConfig,
    binding: ConversationRuntimeBinding<'_>,
    mode: ConstrainedSubagentMode,
    next_child_depth: usize,
    active_children: usize,
    parent_session_id: &str,
    child_session_id: &str,
    child_label: Option<String>,
    task: &str,
    runtime_self_continuity: Option<&RuntimeSelfContinuity>,
    identity: Option<ConstrainedSubagentIdentity>,
    execution_policy: DelegateChildExecutionPolicy,
) -> DelegateChildLifecycleSeed {
    let execution = build_delegate_child_execution(
        config,
        binding,
        mode,
        next_child_depth,
        active_children,
        identity,
        &execution_policy,
    );
    let request = build_delegate_child_request(
        parent_session_id,
        child_session_id,
        child_label,
        task,
        runtime_self_continuity,
        &execution,
        mode,
        execution_policy.profile,
    );

    DelegateChildLifecycleSeed { execution, request }
}

fn build_delegate_child_execution(
    config: &LoongConfig,
    binding: ConversationRuntimeBinding<'_>,
    mode: ConstrainedSubagentMode,
    next_child_depth: usize,
    active_children: usize,
    identity: Option<ConstrainedSubagentIdentity>,
    execution_policy: &DelegateChildExecutionPolicy,
) -> ConstrainedSubagentExecution {
    let kernel_bound = binding.is_kernel_bound();
    let profile = ConstrainedSubagentProfile::for_child_depth(
        next_child_depth,
        config.tools.delegate.max_depth,
    );

    ConstrainedSubagentExecution {
        mode,
        isolation: execution_policy.isolation,
        owner_kind: execution_policy.owner_kind,
        depth: next_child_depth,
        max_depth: config.tools.delegate.max_depth,
        active_children,
        max_active_children: config.tools.delegate.max_active_children,
        timeout_seconds: execution_policy.timeout_seconds,
        allow_shell_in_child: execution_policy.allow_shell_in_child,
        child_tool_allowlist: execution_policy.child_tool_allowlist.clone(),
        workspace_root: execution_policy.workspace_root.clone(),
        runtime_narrowing: execution_policy.runtime_narrowing.clone(),
        kernel_bound,
        identity,
        profile: Some(profile),
    }
}

fn build_delegate_child_request(
    parent_session_id: &str,
    child_session_id: &str,
    child_label: Option<String>,
    task: &str,
    runtime_self_continuity: Option<&RuntimeSelfContinuity>,
    execution: &ConstrainedSubagentExecution,
    mode: ConstrainedSubagentMode,
    profile: Option<DelegateBuiltinProfile>,
) -> CreateSessionWithEventRequest {
    let session_state = delegate_child_session_state(mode);
    let event_kind = delegate_child_event_kind(mode);
    let source_surface = delegate_child_source_surface(mode);
    let event_payload_json = build_delegate_child_event_payload(
        parent_session_id,
        child_session_id,
        task,
        child_label.as_deref(),
        runtime_self_continuity,
        execution,
        profile,
        source_surface,
    );
    let session = NewSessionRecord {
        session_id: child_session_id.to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some(parent_session_id.to_owned()),
        label: child_label,
        state: session_state,
    };

    CreateSessionWithEventRequest {
        session,
        event_kind: event_kind.to_owned(),
        actor_session_id: Some(parent_session_id.to_owned()),
        event_payload_json,
    }
}

fn delegate_child_session_state(mode: ConstrainedSubagentMode) -> SessionState {
    match mode {
        ConstrainedSubagentMode::Inline => SessionState::Running,
        ConstrainedSubagentMode::Async => SessionState::Ready,
    }
}

fn delegate_child_event_kind(mode: ConstrainedSubagentMode) -> &'static str {
    match mode {
        ConstrainedSubagentMode::Inline => "delegate_started",
        ConstrainedSubagentMode::Async => "delegate_queued",
    }
}

fn delegate_child_source_surface(mode: ConstrainedSubagentMode) -> &'static str {
    match mode {
        ConstrainedSubagentMode::Inline => "delegate.inline",
        ConstrainedSubagentMode::Async => "delegate.async",
    }
}

fn build_delegate_child_event_payload(
    parent_session_id: &str,
    child_session_id: &str,
    task: &str,
    child_label: Option<&str>,
    runtime_self_continuity: Option<&RuntimeSelfContinuity>,
    execution: &ConstrainedSubagentExecution,
    profile: Option<DelegateBuiltinProfile>,
    source_surface: &str,
) -> Value {
    let trust_event =
        delegate_child_trust_event(parent_session_id, child_session_id, source_surface);
    let event_payload_json = execution.spawn_payload_with_profile_and_runtime_self_continuity(
        task,
        child_label,
        profile,
        runtime_self_continuity,
    );
    let payload_with_trust =
        embed_trust_event_payload(event_payload_json.clone(), trust_event.clone());
    let extracted_trust_event = extract_trust_event_payload(&payload_with_trust);
    if extracted_trust_event.as_ref() != Some(&trust_event) {
        return event_payload_json;
    }

    payload_with_trust
}

#[cfg(test)]
pub(crate) fn finalize_async_delegate_spawn_failure(
    memory_config: &SessionStoreConfig,
    child_session_id: &str,
    parent_session_id: &str,
    label: Option<String>,
    profile: Option<DelegateBuiltinProfile>,
    execution: &ConstrainedSubagentExecution,
    max_frozen_bytes: usize,
    error: String,
) -> Result<(), String> {
    let repo = SessionRepository::new(memory_config)?;
    let request = build_async_delegate_spawn_failure_request(
        child_session_id,
        parent_session_id,
        label,
        profile,
        execution,
        max_frozen_bytes,
        error,
    );
    finalize_terminal_if_current_allowing_stale_state(
        &repo,
        child_session_id,
        SessionState::Ready,
        request,
    )?;

    Ok(())
}

pub(crate) fn finalize_async_delegate_spawn_failure_with_recovery(
    memory_config: &SessionStoreConfig,
    child_session_id: &str,
    parent_session_id: &str,
    label: Option<String>,
    profile: Option<DelegateBuiltinProfile>,
    execution: &ConstrainedSubagentExecution,
    max_frozen_bytes: usize,
    error: String,
) -> Result<(), String> {
    let recovery_label = label.clone();
    let request = build_async_delegate_spawn_failure_request(
        child_session_id,
        parent_session_id,
        label,
        profile,
        execution,
        max_frozen_bytes,
        error.clone(),
    );
    let request_frozen_result = request.frozen_result.clone();
    let repo = SessionRepository::new(memory_config)?;
    let finalize_result = finalize_terminal_if_current_allowing_stale_state(
        &repo,
        child_session_id,
        SessionState::Ready,
        request,
    );
    match finalize_result {
        Ok(()) => Ok(()),
        Err(finalize_error) => {
            let recovery_error = format!(
                "delegate_async_spawn_failure_persist_failed: {finalize_error}; original spawn error: {error}"
            );
            let recovery_frozen_result = request_frozen_result.clone();
            let fallback_frozen_result = request_frozen_result;
            let recovery_request = FinalizeSessionTerminalRequest {
                state: SessionState::Failed,
                last_error: Some(recovery_error.clone()),
                event_kind: RECOVERY_EVENT_KIND.to_owned(),
                actor_session_id: Some(parent_session_id.to_owned()),
                event_payload_json: build_async_spawn_failure_recovery_payload(
                    recovery_label.as_deref(),
                    &error,
                    &recovery_error,
                ),
                outcome_status: "error".to_owned(),
                outcome_payload_json: json!({
                    "error": recovery_error.as_str(),
                }),
                frozen_result: recovery_frozen_result,
            };
            let transition_result = repo.finalize_session_terminal_if_current(
                child_session_id,
                SessionState::Ready,
                recovery_request,
            );
            match transition_result {
                Ok(Some(_)) => Ok(()),
                Ok(None) => {
                    let current_state = repo
                        .load_session(child_session_id)?
                        .map(|session| session.state.as_str().to_owned())
                        .unwrap_or_else(|| "missing".to_owned());
                    let message = format!(
                        "{recovery_error}; delegate_async_spawn_recovery_skipped_from_state: {current_state}"
                    );
                    Err(message)
                }
                Err(recovery_event_error) => {
                    let state_result = repo.update_session_state_if_current(
                        child_session_id,
                        SessionState::Ready,
                        SessionState::Failed,
                        Some(recovery_error.clone()),
                    );
                    match state_result {
                        Ok(Some(_)) => {
                            persist_recovery_terminal_outcome(
                                &repo,
                                child_session_id,
                                &recovery_error,
                                fallback_frozen_result,
                            )?;
                            Ok(())
                        }
                        Ok(None) => {
                            let current_state = repo
                                .load_session(child_session_id)?
                                .map(|session| session.state.as_str().to_owned())
                                .unwrap_or_else(|| "missing".to_owned());
                            let message = format!(
                                "{recovery_error}; delegate_async_spawn_recovery_skipped_from_state: {current_state}"
                            );
                            Err(message)
                        }
                        Err(mark_error) => {
                            let message = format!(
                                "{recovery_error}; delegate_async_spawn_recovery_failed: {mark_error}; delegate_async_spawn_recovery_event_failed: {recovery_event_error}"
                            );
                            Err(message)
                        }
                    }
                }
            }
        }
    }
}

pub(crate) fn finalize_delegate_child_terminal_with_recovery(
    repo: &SessionRepository,
    child_session_id: &str,
    request: FinalizeSessionTerminalRequest,
) -> Result<(), String> {
    let recovery_request = request.clone();
    let finalize_result = finalize_terminal_if_current_allowing_stale_state(
        repo,
        child_session_id,
        SessionState::Running,
        request,
    );
    match finalize_result {
        Ok(()) => Ok(()),
        Err(finalize_error) => {
            let recovery_error = format!("delegate_terminal_finalize_failed: {finalize_error}");
            let fallback_frozen_result = recovery_request.frozen_result.clone();
            let recovery_request = FinalizeSessionTerminalRequest {
                state: SessionState::Failed,
                last_error: Some(recovery_error.clone()),
                event_kind: RECOVERY_EVENT_KIND.to_owned(),
                actor_session_id: recovery_request.actor_session_id.clone(),
                event_payload_json: build_terminal_finalize_recovery_payload(
                    &recovery_request,
                    &recovery_error,
                ),
                outcome_status: "error".to_owned(),
                outcome_payload_json: json!({
                    "error": recovery_error.as_str(),
                }),
                frozen_result: recovery_request.frozen_result,
            };
            let transition_result = repo.finalize_session_terminal_if_current(
                child_session_id,
                SessionState::Running,
                recovery_request,
            );
            match transition_result {
                Ok(Some(_)) => Err(recovery_error),
                Ok(None) => {
                    delegate_terminal_recovery_skipped_error(repo, child_session_id, recovery_error)
                }
                Err(recovery_event_error) => {
                    let state_result = repo.update_session_state_if_current(
                        child_session_id,
                        SessionState::Running,
                        SessionState::Failed,
                        Some(recovery_error.clone()),
                    );
                    match state_result {
                        Ok(Some(_)) => {
                            if let Err(persist_error) = persist_recovery_terminal_outcome(
                                repo,
                                child_session_id,
                                &recovery_error,
                                fallback_frozen_result,
                            ) {
                                let message = format!(
                                    "{recovery_error}; delegate_terminal_recovery_outcome_persist_failed: {persist_error}; delegate_terminal_recovery_event_failed: {recovery_event_error}"
                                );
                                return Err(message);
                            }
                            let message = format!(
                                "{recovery_error}; delegate_terminal_recovery_event_failed: {recovery_event_error}"
                            );
                            Err(message)
                        }
                        Ok(None) => delegate_terminal_recovery_skipped_error(
                            repo,
                            child_session_id,
                            recovery_error,
                        ),
                        Err(mark_error) => {
                            let message = format!(
                                "{recovery_error}; delegate_terminal_recovery_failed: {mark_error}"
                            );
                            Err(message)
                        }
                    }
                }
            }
        }
    }
}

fn build_async_delegate_spawn_failure_request(
    child_session_id: &str,
    parent_session_id: &str,
    label: Option<String>,
    profile: Option<DelegateBuiltinProfile>,
    execution: &ConstrainedSubagentExecution,
    max_frozen_bytes: usize,
    error: String,
) -> FinalizeSessionTerminalRequest {
    let outcome = crate::tools::delegate::delegate_error_outcome(
        child_session_id.to_owned(),
        Some(parent_session_id.to_owned()),
        label,
        profile,
        error.clone(),
        0,
    );
    let frozen_result = capture_frozen_result(&outcome, max_frozen_bytes);

    FinalizeSessionTerminalRequest {
        state: SessionState::Failed,
        last_error: Some(error.clone()),
        event_kind: "delegate_spawn_failed".to_owned(),
        actor_session_id: Some(parent_session_id.to_owned()),
        event_payload_json: execution.terminal_payload(
            ConstrainedSubagentTerminalReason::SpawnFailed,
            0,
            None,
            Some(error.as_str()),
        ),
        outcome_status: outcome.status,
        outcome_payload_json: outcome.payload,
        frozen_result: Some(frozen_result),
    }
}

fn persist_recovery_terminal_outcome(
    repo: &SessionRepository,
    child_session_id: &str,
    recovery_error: &str,
    frozen_result: Option<crate::session::frozen_result::FrozenResult>,
) -> Result<(), String> {
    let recovery_outcome_payload = json!({
        "error": recovery_error,
    });
    repo.upsert_terminal_outcome_with_frozen_result(
        child_session_id,
        "error",
        recovery_outcome_payload,
        frozen_result,
    )?;

    Ok(())
}

fn finalize_terminal_if_current_allowing_stale_state(
    repo: &SessionRepository,
    session_id: &str,
    expected_state: SessionState,
    request: FinalizeSessionTerminalRequest,
) -> Result<(), String> {
    let finalize_result =
        repo.finalize_session_terminal_if_current(session_id, expected_state, request)?;
    match finalize_result {
        Some(_) => Ok(()),
        None => {
            let session = repo.load_session(session_id)?;
            if session.is_some() {
                return Ok(());
            }

            let message = format!("session `{session_id}` not found");
            Err(message)
        }
    }
}

fn delegate_terminal_recovery_skipped_error(
    repo: &SessionRepository,
    child_session_id: &str,
    recovery_error: String,
) -> Result<(), String> {
    let current_state = repo
        .load_session(child_session_id)?
        .map(|session| session.state.as_str().to_owned())
        .unwrap_or_else(|| "missing".to_owned());
    let message =
        format!("{recovery_error}; delegate_terminal_recovery_skipped_from_state: {current_state}");
    Err(message)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use serde_json::json;

    use super::*;
    use crate::config::LoongConfig;
    use crate::session::repository::{NewSessionEvent, NewSessionRecord};
    use crate::session::store::SessionStoreConfig;
    use crate::trust::extract_trust_event_payload;

    fn isolated_repo(test_name: &str) -> SessionRepository {
        let (repo, _sqlite_path) = isolated_repo_with_path(test_name);
        repo
    }

    fn isolated_repo_with_path(test_name: &str) -> (SessionRepository, std::path::PathBuf) {
        let sqlite_path = std::env::temp_dir().join(format!(
            "loong-operator-delegate-runtime-{test_name}-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&sqlite_path);
        let config = SessionStoreConfig {
            sqlite_path: Some(sqlite_path),
            ..SessionStoreConfig::default()
        };
        let repo = SessionRepository::new(&config).expect("session repository");
        let sqlite_path = config.sqlite_path.expect("sqlite path");

        (repo, sqlite_path)
    }

    #[test]
    fn resolve_delegate_child_contract_falls_back_to_lineage_profile() {
        let repo = isolated_repo("lineage-fallback");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: None,
            state: SessionState::Ready,
        })
        .expect("create root session");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: None,
            state: SessionState::Ready,
        })
        .expect("create child session");

        let contract =
            resolve_delegate_child_contract(&repo, "child-session", 2).expect("resolve contract");
        let profile = contract.and_then(|contract| contract.profile);

        assert_eq!(
            profile,
            Some(ConstrainedSubagentProfile::for_child_depth(1, 2))
        );
    }

    #[test]
    fn resolve_delegate_child_contract_prefers_persisted_execution() {
        let repo = isolated_repo("persisted-execution");
        repo.create_session(NewSessionRecord {
            session_id: "root-session".to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: None,
            state: SessionState::Ready,
        })
        .expect("create root session");
        repo.create_session(NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: None,
            state: SessionState::Ready,
        })
        .expect("create child session");
        repo.append_event(NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "research",
                "execution": {
                    "mode": "inline",
                    "depth": 1,
                    "max_depth": 3,
                    "active_children": 0,
                    "max_active_children": 2,
                    "timeout_seconds": 60,
                    "allow_shell_in_child": false,
                    "child_tool_allowlist": ["file.read"],
                    "kernel_bound": false,
                    "runtime_narrowing": {
                        "browser": {
                            "max_sessions": 1
                        }
                    }
                }
            }),
        })
        .expect("append event");

        let contract =
            resolve_delegate_child_contract(&repo, "child-session", 3).expect("resolve contract");
        let contract = contract.expect("resolved contract");
        let profile = contract.profile;

        assert_eq!(
            profile,
            Some(ConstrainedSubagentProfile::for_child_depth(1, 3))
        );
        assert_eq!(contract.runtime_narrowing.browser.max_sessions, Some(1));
    }

    #[test]
    fn build_delegate_child_lifecycle_seed_uses_mode_specific_state_and_event_kind() {
        let config = LoongConfig::default();
        let execution_policy = DelegateChildExecutionPolicy {
            isolation: ConstrainedSubagentIsolation::Shared,
            profile: None,
            owner_kind: None,
            timeout_seconds: 42,
            allow_shell_in_child: false,
            child_tool_allowlist: config.tools.delegate.child_tool_allowlist.clone(),
            runtime_narrowing: config.tools.delegate.child_runtime.runtime_narrowing(),
            workspace_root: None,
        };
        let seed = build_delegate_child_lifecycle_seed(
            &config,
            ConversationRuntimeBinding::direct(),
            ConstrainedSubagentMode::Async,
            1,
            0,
            "parent-session",
            "child-session",
            Some("worker".to_owned()),
            "research",
            None,
            None,
            execution_policy,
        );

        assert_eq!(seed.request.session.state, SessionState::Ready);
        assert_eq!(seed.request.event_kind, "delegate_queued");
        assert_eq!(seed.execution.mode, ConstrainedSubagentMode::Async);
        assert_eq!(seed.execution.depth, 1);
        assert_eq!(seed.execution.timeout_seconds, 42);
    }

    #[test]
    fn build_delegate_child_lifecycle_seed_embeds_delegate_trust_event() {
        let config = LoongConfig::default();
        let execution_policy = DelegateChildExecutionPolicy {
            isolation: ConstrainedSubagentIsolation::Shared,
            profile: None,
            owner_kind: None,
            timeout_seconds: 60,
            allow_shell_in_child: false,
            child_tool_allowlist: config.tools.delegate.child_tool_allowlist.clone(),
            runtime_narrowing: config.tools.delegate.child_runtime.runtime_narrowing(),
            workspace_root: None,
        };
        let seed = build_delegate_child_lifecycle_seed(
            &config,
            ConversationRuntimeBinding::direct(),
            ConstrainedSubagentMode::Inline,
            1,
            0,
            "parent-session",
            "child-session",
            Some("worker".to_owned()),
            "research",
            None,
            None,
            execution_policy,
        );

        let trust_event = extract_trust_event_payload(&seed.request.event_payload_json);
        assert!(trust_event.is_some(), "expected trust event payload");
    }

    #[test]
    fn finalize_delegate_child_terminal_with_recovery_persists_frozen_result_after_recovery_event()
    {
        let (repo, sqlite_path) = isolated_repo_with_path("terminal-recovery-frozen-result");
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

        let conn = Connection::open(&sqlite_path).expect("open sqlite connection");
        conn.execute(
            "CREATE TRIGGER fail_delegate_completed_event
             BEFORE INSERT ON session_events
             WHEN NEW.event_kind = 'delegate_completed'
             BEGIN
                SELECT RAISE(FAIL, 'forced delegate_completed event failure');
             END;",
            [],
        )
        .expect("create event failure trigger");
        drop(conn);

        let frozen_result = crate::session::frozen_result::FrozenResult {
            content: crate::session::frozen_result::FrozenContent::Text("done".to_owned()),
            captured_at: std::time::SystemTime::now(),
            byte_len: "done".len(),
            truncated: false,
        };
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
                }),
                outcome_status: "ok".to_owned(),
                outcome_payload_json: json!({
                    "child_session_id": "child-session",
                    "final_output": "done",
                }),
                frozen_result: Some(frozen_result.clone()),
            },
        )
        .expect_err("forced finalize failure should surface recovery error");

        assert!(error.contains("delegate_terminal_finalize_failed"));

        let child = repo
            .load_session("child-session")
            .expect("load child session")
            .expect("child session row");
        assert_eq!(child.state, SessionState::Failed);

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
        assert_eq!(terminal_outcome.frozen_result, Some(frozen_result));
    }

    #[test]
    fn finalize_async_delegate_spawn_failure_with_recovery_persists_frozen_result() {
        let (repo, sqlite_path) = isolated_repo_with_path("spawn-failure-recovery-frozen-result");
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

        let execution = ConstrainedSubagentExecution {
            mode: ConstrainedSubagentMode::Async,
            isolation: ConstrainedSubagentIsolation::default(),
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

        let conn = Connection::open(&sqlite_path).expect("open sqlite connection");
        conn.execute(
            "CREATE TRIGGER fail_delegate_spawn_failed_event
             BEFORE INSERT ON session_events
             WHEN NEW.event_kind = 'delegate_spawn_failed'
             BEGIN
                SELECT RAISE(FAIL, 'forced delegate_spawn_failed event failure');
             END;",
            [],
        )
        .expect("create spawn failure trigger");
        drop(conn);

        finalize_async_delegate_spawn_failure_with_recovery(
            &SessionStoreConfig {
                sqlite_path: Some(sqlite_path),
                ..SessionStoreConfig::default()
            },
            "child-session",
            "root-session",
            Some("Child".to_owned()),
            None,
            &execution,
            256 * 1024,
            "spawn unavailable".to_owned(),
        )
        .expect("recovery should persist terminal outcome");

        let child = repo
            .load_session("child-session")
            .expect("load child session")
            .expect("child session row");
        assert_eq!(child.state, SessionState::Failed);

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
        assert!(terminal_outcome.frozen_result.is_some());
        assert_eq!(
            terminal_outcome
                .frozen_result
                .expect("frozen result")
                .content,
            crate::session::frozen_result::FrozenContent::Error {
                code: "spawn unavailable".to_owned(),
                message: "spawn unavailable".to_owned(),
            }
        );
    }
}
