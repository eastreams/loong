use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::CliResult;
use crate::runtime_self_continuity::{self, RuntimeSelfContinuity};
use crate::tools::runtime_config::ToolRuntimeNarrowing;
use crate::tools::{ToolView, delegate_child_tool_view_for_contract};

use super::super::super::config::LoongConfig;
use super::super::subagent::{
    ConstrainedSubagentExecution, ConstrainedSubagentIdentity, ConstrainedSubagentProfile,
    DelegateBuiltinProfile,
};
use super::SessionContext;
#[cfg(feature = "memory-sqlite")]
use super::active_skills;
#[cfg(feature = "memory-sqlite")]
use crate::operator::delegate_runtime::{
    derive_subagent_profile_from_lineage, resolve_delegate_child_contract,
};
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{SessionKind, SessionRepository, SessionToolPolicyRecord};
#[cfg(feature = "memory-sqlite")]
use crate::session::store;

#[cfg(feature = "memory-sqlite")]
pub(super) fn load_session_tool_policy(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<SessionToolPolicyRecord>, String> {
    repo.load_session_tool_policy(session_id)
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn apply_session_tool_policy_to_tool_view(
    base_tool_view: ToolView,
    session_tool_policy: Option<&SessionToolPolicyRecord>,
) -> ToolView {
    let Some(session_tool_policy) = session_tool_policy else {
        return base_tool_view;
    };
    if session_tool_policy.requested_tool_ids.is_empty() {
        return base_tool_view;
    }

    let policy_tool_view = ToolView::from_tool_names(session_tool_policy.requested_tool_ids.iter());
    base_tool_view.intersect(&policy_tool_view)
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn merge_effective_runtime_narrowing(
    delegate_runtime_narrowing: Option<ToolRuntimeNarrowing>,
    session_tool_policy: Option<&SessionToolPolicyRecord>,
) -> Option<ToolRuntimeNarrowing> {
    let policy_runtime_narrowing = session_tool_policy.and_then(|policy| {
        (!policy.runtime_narrowing.is_empty()).then_some(policy.runtime_narrowing.clone())
    });
    crate::tools::runtime_config::merge_runtime_narrowing_sources(
        delegate_runtime_narrowing,
        policy_runtime_narrowing,
    )
}

#[cfg(feature = "memory-sqlite")]
pub(super) struct DelegateAnchorSnapshot {
    execution: Option<ConstrainedSubagentExecution>,
    profile: Option<DelegateBuiltinProfile>,
    workspace_root: Option<PathBuf>,
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn load_delegate_anchor_snapshot(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<DelegateAnchorSnapshot, String> {
    let events = repo.list_delegate_lifecycle_events(session_id)?;
    let mut execution = None;
    let mut profile = None;
    let mut workspace_root = None;

    for event in events.into_iter().rev() {
        let is_delegate_anchor = matches!(
            event.event_kind.as_str(),
            "delegate_queued" | "delegate_started"
        );
        if !is_delegate_anchor {
            continue;
        }

        if execution.is_none() {
            execution = ConstrainedSubagentExecution::from_event_payload(&event.payload_json);
        }
        if profile.is_none() {
            profile = ConstrainedSubagentExecution::profile_from_event_payload(&event.payload_json);
        }
        if workspace_root.is_none() {
            let event_workspace_root =
                ConstrainedSubagentExecution::from_event_payload(&event.payload_json)
                    .and_then(|execution| execution.workspace_root);
            workspace_root = event_workspace_root;
        }
        if execution.is_some() && profile.is_some() && workspace_root.is_some() {
            break;
        }
    }

    Ok(DelegateAnchorSnapshot {
        execution,
        profile,
        workspace_root,
    })
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn load_delegate_execution_contract(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<ConstrainedSubagentExecution>, String> {
    let snapshot = load_delegate_anchor_snapshot(repo, session_id)?;
    Ok(snapshot.execution)
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn load_delegate_profile(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<DelegateBuiltinProfile>, String> {
    let snapshot = load_delegate_anchor_snapshot(repo, session_id)?;
    Ok(snapshot.profile)
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn load_delegate_workspace_root(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<PathBuf>, String> {
    let snapshot = load_delegate_anchor_snapshot(repo, session_id)?;
    Ok(snapshot.workspace_root)
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn load_session_runtime_self_continuity(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<RuntimeSelfContinuity>, String> {
    runtime_self_continuity::load_persisted_runtime_self_continuity(repo, session_id)
}

#[cfg(feature = "memory-sqlite")]
#[derive(Clone)]
pub(super) struct PersistedSessionSnapshot {
    pub(super) session_id: String,
    pub(super) parent_session_id: Option<String>,
    pub(super) label: Option<String>,
    pub(super) is_delegate_child: bool,
    pub(super) subagent_execution: Option<ConstrainedSubagentExecution>,
    pub(super) session_tool_policy: Option<SessionToolPolicyRecord>,
    pub(super) delegate_runtime_narrowing: Option<ToolRuntimeNarrowing>,
    pub(super) delegate_profile: Option<DelegateBuiltinProfile>,
    pub(super) workspace_root: Option<PathBuf>,
    pub(super) active_skills: Option<active_skills::ActiveSkillsState>,
    pub(super) active_skill_roots: Vec<PathBuf>,
    pub(super) runtime_self_continuity: Option<RuntimeSelfContinuity>,
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn open_session_repository(config: &LoongConfig) -> CliResult<SessionRepository> {
    let memory_config =
        store::session_store_config_from_memory_config_without_env_overrides(&config.memory);
    SessionRepository::new(&memory_config)
        .map_err(|error| format!("open session repository failed: {error}"))
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn load_persisted_session_snapshot(
    repo: &SessionRepository,
    session_id: &str,
) -> CliResult<Option<PersistedSessionSnapshot>> {
    let session_tool_policy = load_session_tool_policy(repo, session_id)?;
    let session = repo
        .load_session(session_id)
        .map_err(|error| format!("load session context failed: {error}"))?;

    if let Some(session) = session {
        let parent_session_id = session.parent_session_id;
        let is_delegate_child = parent_session_id.is_some();
        let label = session.label;
        let subagent_execution = if is_delegate_child {
            load_delegate_execution_contract(repo, session_id)?
        } else {
            None
        };
        let delegate_runtime_narrowing = if is_delegate_child {
            subagent_execution.as_ref().and_then(|execution| {
                (!execution.runtime_narrowing.is_empty())
                    .then_some(execution.runtime_narrowing.clone())
            })
        } else {
            None
        };
        let delegate_profile = if is_delegate_child {
            load_delegate_profile(repo, session_id)?
        } else {
            None
        };
        let workspace_root = if is_delegate_child {
            load_delegate_workspace_root(repo, session_id)?
        } else {
            None
        };
        let runtime_self_continuity = load_session_runtime_self_continuity(repo, session_id)?;
        let active_skills = load_active_skills_state(repo, session_id).unwrap_or_default();
        let active_skill_roots = active_skill_roots_from_state(active_skills.as_ref());
        let snapshot = PersistedSessionSnapshot {
            session_id: session.session_id,
            parent_session_id,
            label,
            is_delegate_child,
            subagent_execution,
            session_tool_policy,
            delegate_runtime_narrowing,
            delegate_profile,
            workspace_root,
            active_skills,
            active_skill_roots,
            runtime_self_continuity,
        };
        return Ok(Some(snapshot));
    }

    let summary = repo
        .load_session_summary_with_legacy_fallback(session_id)
        .map_err(|error| format!("load legacy session context failed: {error}"))?;
    let Some(summary) = summary else {
        return Ok(None);
    };

    let is_delegate_child = summary.kind == SessionKind::DelegateChild;
    let subagent_execution = if is_delegate_child {
        load_delegate_execution_contract(repo, session_id)?
    } else {
        None
    };
    let delegate_runtime_narrowing = if is_delegate_child {
        subagent_execution.as_ref().and_then(|execution| {
            (!execution.runtime_narrowing.is_empty()).then_some(execution.runtime_narrowing.clone())
        })
    } else {
        None
    };
    let delegate_profile = if is_delegate_child {
        load_delegate_profile(repo, session_id)?
    } else {
        None
    };
    let workspace_root = if is_delegate_child {
        load_delegate_workspace_root(repo, session_id)?
    } else {
        None
    };
    let runtime_self_continuity = load_session_runtime_self_continuity(repo, session_id)?;
    let active_skills = load_active_skills_state(repo, session_id).unwrap_or_default();
    let active_skill_roots = active_skill_roots_from_state(active_skills.as_ref());
    let snapshot = PersistedSessionSnapshot {
        session_id: summary.session_id,
        parent_session_id: summary.parent_session_id,
        label: summary.label,
        is_delegate_child,
        subagent_execution,
        session_tool_policy,
        delegate_runtime_narrowing,
        delegate_profile,
        workspace_root,
        active_skills,
        active_skill_roots,
        runtime_self_continuity,
    };
    Ok(Some(snapshot))
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn load_active_skills_state(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<active_skills::ActiveSkillsState>, String> {
    active_skills::load_persisted_active_skills(repo, session_id)
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn active_skill_roots_from_state(
    active_skills: Option<&active_skills::ActiveSkillsState>,
) -> Vec<PathBuf> {
    let Some(active_skills) = active_skills else {
        return Vec::new();
    };
    let mut roots = Vec::new();
    for skill in &active_skills.skills {
        let Some(skill_root) = skill.skill_root.as_deref() else {
            continue;
        };
        let trimmed = skill_root.trim();
        if trimmed.is_empty() {
            continue;
        }
        let path = PathBuf::from(trimmed);
        let canonical = std::fs::canonicalize(&path).unwrap_or(path);
        if !roots.contains(&canonical) {
            roots.push(canonical);
        }
    }
    roots
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn apply_active_skill_blocked_tools_to_tool_view(
    base_tool_view: ToolView,
    active_skills: Option<&active_skills::ActiveSkillsState>,
) -> ToolView {
    let Some(active_skills) = active_skills else {
        return base_tool_view;
    };

    let mut blocked_names = BTreeSet::new();
    for skill in &active_skills.skills {
        for blocked_tool in &skill.blocked_tools {
            let blocked_tool = blocked_tool.trim();
            if blocked_tool.is_empty() {
                continue;
            }
            let canonical_name = crate::tools::canonical_tool_name(blocked_tool);
            blocked_names.insert(canonical_name.to_owned());
            if let Some(direct_tool_name) =
                crate::tools::direct_tool_name_for_hidden_tool(blocked_tool)
            {
                blocked_names.insert(direct_tool_name.to_owned());
            }
        }
    }

    if blocked_names.is_empty() {
        return base_tool_view;
    }

    ToolView::from_tool_names(
        base_tool_view
            .tool_names()
            .filter(|tool_name| !blocked_names.contains(*tool_name)),
    )
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn build_base_tool_view_from_snapshot(
    config: &LoongConfig,
    repo: &SessionRepository,
    session_id: &str,
    snapshot: Option<&PersistedSessionSnapshot>,
) -> CliResult<ToolView> {
    let Some(snapshot) = snapshot else {
        return Ok(crate::tools::runtime_tool_view_from_loong_config(config));
    };

    let is_delegate_child = snapshot.parent_session_id.is_some() || snapshot.is_delegate_child;
    if is_delegate_child {
        if snapshot.subagent_execution.is_none() {
            let derived_profile = derive_subagent_profile_from_lineage(
                repo,
                session_id,
                config.tools.delegate.max_depth,
            )?;
            let allow_delegate = derived_profile
                .map(ConstrainedSubagentProfile::allows_child_delegation)
                .unwrap_or(false);
            return Ok(
                crate::tools::delegate_child_tool_view_for_config_with_delegate(
                    &config.tools,
                    allow_delegate,
                ),
            );
        }
        let derived_contract =
            resolve_delegate_child_contract(repo, session_id, config.tools.delegate.max_depth)?;
        return Ok(delegate_child_tool_view_for_contract(
            &config.tools,
            derived_contract.as_ref(),
        ));
    }

    Ok(crate::tools::runtime_tool_view_from_loong_config(config))
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn build_session_context_from_snapshot(
    config: &LoongConfig,
    repo: &SessionRepository,
    session_id: &str,
    base_tool_view: ToolView,
    snapshot: PersistedSessionSnapshot,
) -> CliResult<SessionContext> {
    let visible_skill_roots = super::model_visible_skill_roots_from_config(config);
    let tool_view = apply_active_skill_blocked_tools_to_tool_view(
        apply_session_tool_policy_to_tool_view(
            base_tool_view,
            snapshot.session_tool_policy.as_ref(),
        ),
        snapshot.active_skills.as_ref(),
    );
    let runtime_narrowing = merge_effective_runtime_narrowing(
        snapshot.delegate_runtime_narrowing.clone(),
        snapshot.session_tool_policy.as_ref(),
    );
    let mut session_context = match snapshot.parent_session_id.clone() {
        Some(parent_session_id) => {
            SessionContext::child(snapshot.session_id.clone(), parent_session_id, tool_view)
        }
        None => {
            super::root_session_context_from_config(config, snapshot.session_id.clone(), tool_view)
        }
    };
    if let Some(profile) = snapshot.delegate_profile {
        session_context = session_context.with_profile(profile);
    }
    if let Some(workspace_root) = snapshot.workspace_root {
        session_context = session_context.with_workspace_root(workspace_root);
    }
    if !snapshot.active_skill_roots.is_empty() {
        session_context = session_context.with_active_skill_roots(snapshot.active_skill_roots);
    }
    if !visible_skill_roots.is_empty() {
        session_context = session_context.with_visible_skill_roots(visible_skill_roots);
    }
    if snapshot.is_delegate_child {
        if let Some(label) = snapshot.label {
            session_context = session_context.with_subagent_identity(ConstrainedSubagentIdentity {
                nickname: Some(label),
                specialization: None,
            });
        }
        if let Some(subagent_execution) = snapshot.subagent_execution {
            session_context = session_context.with_subagent_execution(subagent_execution);
        } else if let Some(subagent_profile) =
            derive_subagent_profile_from_lineage(repo, session_id, config.tools.delegate.max_depth)?
        {
            session_context = session_context.with_subagent_profile(subagent_profile);
        }
    }
    if let Some(runtime_narrowing) = runtime_narrowing {
        session_context = session_context.with_runtime_narrowing(runtime_narrowing);
    }
    if let Some(runtime_self_continuity) = snapshot.runtime_self_continuity {
        session_context = session_context.with_runtime_self_continuity(runtime_self_continuity);
    }
    Ok(session_context)
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn load_persisted_session_context(
    config: &LoongConfig,
    session_id: &str,
    tool_view: &ToolView,
) -> CliResult<Option<SessionContext>> {
    let repo = open_session_repository(config)?;
    let snapshot = load_persisted_session_snapshot(&repo, session_id)?;
    let Some(snapshot) = snapshot else {
        return Ok(None);
    };
    let session_context = build_session_context_from_snapshot(
        config,
        &repo,
        session_id,
        tool_view.clone(),
        snapshot,
    )?;
    Ok(Some(session_context))
}
