#[cfg(feature = "memory-sqlite")]
use crate::config::LoongClawConfig;
#[cfg(feature = "memory-sqlite")]
use crate::conversation::{
    ConstrainedSubagentContractView, ConstrainedSubagentExecution, ConstrainedSubagentIdentity,
    ConstrainedSubagentIsolation, ConstrainedSubagentMode, ConstrainedSubagentProfile,
    ConversationRuntimeBinding, DelegateBuiltinProfile,
};
#[cfg(feature = "memory-sqlite")]
use crate::runtime_self_continuity::RuntimeSelfContinuity;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    CreateSessionWithEventRequest, NewSessionRecord, SessionKind, SessionRepository, SessionState,
};
#[cfg(feature = "memory-sqlite")]
use crate::tools::runtime_config::ToolRuntimeNarrowing;
#[cfg(feature = "memory-sqlite")]
use crate::trust::{
    delegate_child_trust_event, embed_trust_event_payload, extract_trust_event_payload,
};

#[cfg(feature = "memory-sqlite")]
use super::session_graph::OperatorSessionGraph;

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DelegateChildExecutionPolicy {
    pub isolation: ConstrainedSubagentIsolation,
    pub profile: Option<DelegateBuiltinProfile>,
    pub timeout_seconds: u64,
    pub allow_shell_in_child: bool,
    pub child_tool_allowlist: Vec<String>,
    pub runtime_narrowing: ToolRuntimeNarrowing,
    pub workspace_root: Option<std::path::PathBuf>,
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DelegateChildLifecycleSeed {
    pub request: CreateSessionWithEventRequest,
    pub execution: ConstrainedSubagentExecution,
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn next_delegate_child_depth(
    repo: &SessionRepository,
    session_id: &str,
    max_depth: usize,
) -> Result<usize, String> {
    let session_graph = OperatorSessionGraph::new(repo);
    session_graph.next_delegate_child_depth(session_id, max_depth)
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn derive_subagent_profile_from_lineage(
    repo: &SessionRepository,
    session_id: &str,
    max_depth: usize,
) -> Result<Option<ConstrainedSubagentProfile>, String> {
    let session_graph = OperatorSessionGraph::new(repo);
    let depth_result = session_graph.lineage_depth(session_id);

    let depth = match depth_result {
        Ok(value) => value,
        Err(error)
            if error.starts_with("session_lineage_broken:")
                || error.starts_with("session_lineage_cycle_detected:") =>
        {
            return Ok(None);
        }
        Err(error) => {
            let error_message =
                format!("compute session lineage depth for subagent profile failed: {error}");
            return Err(error_message);
        }
    };

    let profile = ConstrainedSubagentProfile::for_child_depth(depth, max_depth);
    Ok(Some(profile))
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn load_delegate_execution(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<ConstrainedSubagentExecution>, String> {
    let contract = load_delegate_lifecycle_contract(repo, session_id)?;
    let execution = contract.map(|(execution, _profile)| execution);
    Ok(execution)
}

#[cfg(feature = "memory-sqlite")]
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

    let derived_profile = derive_subagent_profile_from_lineage(repo, session_id, max_depth)?;
    let contract = derived_profile.map(ConstrainedSubagentContractView::from_profile);
    Ok(contract)
}

#[cfg(feature = "memory-sqlite")]
fn load_delegate_lifecycle_contract(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<(ConstrainedSubagentExecution, Option<DelegateBuiltinProfile>)>, String> {
    let events = repo.list_delegate_lifecycle_events(session_id)?;
    let contract = events.into_iter().rev().find_map(|event| {
        let is_delegate_anchor = matches!(
            event.event_kind.as_str(),
            "delegate_queued" | "delegate_started"
        );
        if !is_delegate_anchor {
            return None;
        }

        let execution = ConstrainedSubagentExecution::from_event_payload(&event.payload_json)?;
        let profile = ConstrainedSubagentExecution::profile_from_event_payload(&event.payload_json);
        Some((execution, profile))
    });
    Ok(contract)
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn build_delegate_child_lifecycle_seed(
    config: &LoongClawConfig,
    binding: ConversationRuntimeBinding<'_>,
    mode: ConstrainedSubagentMode,
    next_child_depth: usize,
    active_children: usize,
    parent_session_id: &str,
    child_session_id: &str,
    child_label: Option<String>,
    task: &str,
    runtime_self_continuity: Option<&RuntimeSelfContinuity>,
    execution_policy: DelegateChildExecutionPolicy,
    subagent_identity: Option<ConstrainedSubagentIdentity>,
) -> DelegateChildLifecycleSeed {
    let max_depth = config.tools.delegate.max_depth;
    let max_active_children = config.tools.delegate.max_active_children;
    let internal_profile = ConstrainedSubagentProfile::for_child_depth(next_child_depth, max_depth);
    let session_state = match mode {
        ConstrainedSubagentMode::Inline => SessionState::Running,
        ConstrainedSubagentMode::Async => SessionState::Ready,
    };
    let event_kind = match mode {
        ConstrainedSubagentMode::Inline => "delegate_started",
        ConstrainedSubagentMode::Async => "delegate_queued",
    };
    let source_surface = delegate_child_source_surface(mode);

    let execution = ConstrainedSubagentExecution {
        mode,
        isolation: execution_policy.isolation,
        depth: next_child_depth,
        max_depth,
        active_children,
        max_active_children,
        timeout_seconds: execution_policy.timeout_seconds,
        allow_shell_in_child: execution_policy.allow_shell_in_child,
        child_tool_allowlist: execution_policy.child_tool_allowlist,
        workspace_root: execution_policy.workspace_root,
        runtime_narrowing: execution_policy.runtime_narrowing,
        kernel_bound: binding.is_kernel_bound(),
        identity: subagent_identity,
        profile: Some(internal_profile),
    };

    let request = CreateSessionWithEventRequest {
        session: NewSessionRecord {
            session_id: child_session_id.to_owned(),
            kind: SessionKind::DelegateChild,
            parent_session_id: Some(parent_session_id.to_owned()),
            label: child_label.clone(),
            state: session_state,
        },
        event_kind: event_kind.to_owned(),
        actor_session_id: Some(parent_session_id.to_owned()),
        event_payload_json: build_delegate_child_event_payload(
            parent_session_id,
            child_session_id,
            task,
            child_label.as_deref(),
            execution_policy.profile,
            runtime_self_continuity,
            &execution,
            source_surface,
        ),
    };

    DelegateChildLifecycleSeed { request, execution }
}

#[cfg(feature = "memory-sqlite")]
fn delegate_child_source_surface(mode: ConstrainedSubagentMode) -> &'static str {
    match mode {
        ConstrainedSubagentMode::Inline => "delegate.inline",
        ConstrainedSubagentMode::Async => "delegate.async",
    }
}

#[cfg(feature = "memory-sqlite")]
fn build_delegate_child_event_payload(
    parent_session_id: &str,
    child_session_id: &str,
    task: &str,
    child_label: Option<&str>,
    profile: Option<DelegateBuiltinProfile>,
    runtime_self_continuity: Option<&RuntimeSelfContinuity>,
    execution: &ConstrainedSubagentExecution,
    source_surface: &str,
) -> serde_json::Value {
    let trust_event =
        delegate_child_trust_event(parent_session_id, child_session_id, source_surface);
    let event_payload = execution.spawn_payload_with_profile_and_runtime_self_continuity(
        task,
        child_label,
        profile,
        runtime_self_continuity,
    );
    let payload_with_trust = embed_trust_event_payload(event_payload.clone(), trust_event.clone());
    let extracted_trust_event = extract_trust_event_payload(&payload_with_trust);
    if extracted_trust_event.as_ref() != Some(&trust_event) {
        return event_payload;
    }

    payload_with_trust
}
