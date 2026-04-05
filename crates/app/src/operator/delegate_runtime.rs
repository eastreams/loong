use serde_json::Value;

use crate::config::LoongClawConfig;
use crate::conversation::{
    ConstrainedSubagentContractView, ConstrainedSubagentExecution, ConstrainedSubagentIdentity,
    ConstrainedSubagentMode, ConstrainedSubagentProfile, ConstrainedSubagentTerminalReason,
    ConversationRuntimeBinding,
};
use crate::memory::runtime_config::MemoryRuntimeConfig;
use crate::runtime_self_continuity::RuntimeSelfContinuity;
use crate::session::recovery::{
    RECOVERY_EVENT_KIND, build_async_spawn_failure_recovery_payload,
    build_terminal_finalize_recovery_payload,
};
use crate::session::repository::{
    CreateSessionWithEventRequest, CreateSessionWithEventResult, FinalizeSessionTerminalRequest,
    NewSessionRecord, SessionKind, SessionRepository, SessionState,
    TransitionSessionWithEventIfCurrentRequest,
};
use crate::trust::{
    delegate_child_trust_event, embed_trust_event_payload, extract_trust_event_payload,
};

use super::session_graph::OperatorSessionGraph;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DelegateChildLifecycleSeed {
    pub execution: ConstrainedSubagentExecution,
    pub request: CreateSessionWithEventRequest,
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
    config: &LoongClawConfig,
    binding: ConversationRuntimeBinding<'_>,
    mode: ConstrainedSubagentMode,
    timeout_seconds: u64,
    next_child_depth: usize,
    active_children: usize,
    parent_session_id: &str,
    child_session_id: &str,
    child_label: Option<String>,
    task: &str,
    runtime_self_continuity: Option<&RuntimeSelfContinuity>,
    identity: Option<ConstrainedSubagentIdentity>,
) -> DelegateChildLifecycleSeed {
    let execution = build_delegate_child_execution(
        config,
        binding,
        mode,
        timeout_seconds,
        next_child_depth,
        active_children,
        identity,
    );
    let request = build_delegate_child_request(
        parent_session_id,
        child_session_id,
        child_label,
        task,
        runtime_self_continuity,
        &execution,
        mode,
    );

    DelegateChildLifecycleSeed { execution, request }
}

fn build_delegate_child_execution(
    config: &LoongClawConfig,
    binding: ConversationRuntimeBinding<'_>,
    mode: ConstrainedSubagentMode,
    timeout_seconds: u64,
    next_child_depth: usize,
    active_children: usize,
    identity: Option<ConstrainedSubagentIdentity>,
) -> ConstrainedSubagentExecution {
    let runtime_narrowing = config.tools.delegate.child_runtime.runtime_narrowing();
    let kernel_bound = binding.is_kernel_bound();
    let profile = ConstrainedSubagentProfile::for_child_depth(
        next_child_depth,
        config.tools.delegate.max_depth,
    );

    ConstrainedSubagentExecution {
        mode,
        depth: next_child_depth,
        max_depth: config.tools.delegate.max_depth,
        active_children,
        max_active_children: config.tools.delegate.max_active_children,
        timeout_seconds,
        allow_shell_in_child: config.tools.delegate.allow_shell_in_child,
        child_tool_allowlist: config.tools.delegate.child_tool_allowlist.clone(),
        runtime_narrowing,
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
    source_surface: &str,
) -> Value {
    let trust_event =
        delegate_child_trust_event(parent_session_id, child_session_id, source_surface);
    let event_payload_json = execution.spawn_payload_with_runtime_self_continuity(
        task,
        child_label,
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

pub(crate) fn finalize_async_delegate_spawn_failure(
    memory_config: &MemoryRuntimeConfig,
    child_session_id: &str,
    parent_session_id: &str,
    label: Option<String>,
    execution: &ConstrainedSubagentExecution,
    error: String,
) -> Result<(), String> {
    let repo = SessionRepository::new(memory_config)?;
    let outcome = crate::tools::delegate::delegate_error_outcome(
        child_session_id.to_owned(),
        Some(parent_session_id.to_owned()),
        label,
        Some(&execution.contract_view()),
        error.clone(),
        0,
    );
    let request = FinalizeSessionTerminalRequest {
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
    };
    finalize_terminal_if_current_allowing_stale_state(
        &repo,
        child_session_id,
        SessionState::Ready,
        request,
    )?;

    Ok(())
}

pub(crate) fn finalize_async_delegate_spawn_failure_with_recovery(
    memory_config: &MemoryRuntimeConfig,
    child_session_id: &str,
    parent_session_id: &str,
    label: Option<String>,
    execution: &ConstrainedSubagentExecution,
    error: String,
) -> Result<(), String> {
    let recovery_label = label.clone();
    let finalize_result = finalize_async_delegate_spawn_failure(
        memory_config,
        child_session_id,
        parent_session_id,
        label,
        execution,
        error.clone(),
    );
    match finalize_result {
        Ok(()) => Ok(()),
        Err(finalize_error) => {
            let repo = SessionRepository::new(memory_config)?;
            let recovery_error = format!(
                "delegate_async_spawn_failure_persist_failed: {finalize_error}; original spawn error: {error}"
            );
            let transition_result = repo.transition_session_with_event_if_current(
                child_session_id,
                TransitionSessionWithEventIfCurrentRequest {
                    expected_state: SessionState::Ready,
                    next_state: SessionState::Failed,
                    last_error: Some(recovery_error.clone()),
                    event_kind: RECOVERY_EVENT_KIND.to_owned(),
                    actor_session_id: Some(parent_session_id.to_owned()),
                    event_payload_json: build_async_spawn_failure_recovery_payload(
                        recovery_label.as_deref(),
                        &error,
                        &recovery_error,
                    ),
                },
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
            let transition_result = repo.transition_session_with_event_if_current(
                child_session_id,
                TransitionSessionWithEventIfCurrentRequest {
                    expected_state: SessionState::Running,
                    next_state: SessionState::Failed,
                    last_error: Some(recovery_error.clone()),
                    event_kind: RECOVERY_EVENT_KIND.to_owned(),
                    actor_session_id: recovery_request.actor_session_id.clone(),
                    event_payload_json: build_terminal_finalize_recovery_payload(
                        &recovery_request,
                        &recovery_error,
                    ),
                },
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
    use serde_json::json;

    use super::*;
    use crate::config::LoongClawConfig;
    use crate::memory::runtime_config::MemoryRuntimeConfig;
    use crate::session::repository::{NewSessionEvent, NewSessionRecord};
    use crate::trust::extract_trust_event_payload;

    fn isolated_repo(test_name: &str) -> SessionRepository {
        let sqlite_path = std::env::temp_dir().join(format!(
            "loongclaw-operator-delegate-runtime-{test_name}-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&sqlite_path);
        let config = MemoryRuntimeConfig {
            sqlite_path: Some(sqlite_path),
            ..MemoryRuntimeConfig::default()
        };

        SessionRepository::new(&config).expect("session repository")
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
        let config = LoongClawConfig::default();
        let seed = build_delegate_child_lifecycle_seed(
            &config,
            ConversationRuntimeBinding::direct(),
            ConstrainedSubagentMode::Async,
            42,
            1,
            0,
            "parent-session",
            "child-session",
            Some("worker".to_owned()),
            "research",
            None,
            None,
        );

        assert_eq!(seed.request.session.state, SessionState::Ready);
        assert_eq!(seed.request.event_kind, "delegate_queued");
        assert_eq!(seed.execution.mode, ConstrainedSubagentMode::Async);
        assert_eq!(seed.execution.depth, 1);
        assert_eq!(seed.execution.timeout_seconds, 42);
    }

    #[test]
    fn build_delegate_child_lifecycle_seed_embeds_delegate_trust_event() {
        let config = LoongClawConfig::default();
        let seed = build_delegate_child_lifecycle_seed(
            &config,
            ConversationRuntimeBinding::direct(),
            ConstrainedSubagentMode::Inline,
            60,
            1,
            0,
            "parent-session",
            "child-session",
            Some("worker".to_owned()),
            "research",
            None,
            None,
        );

        let trust_event = extract_trust_event_payload(&seed.request.event_payload_json);
        assert!(trust_event.is_some(), "expected trust event payload");
    }
}
