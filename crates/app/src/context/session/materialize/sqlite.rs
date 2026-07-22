//! SQLite-backed durable projections for Session materialization.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use loong_contracts::{Capabilities, GovernedSessionMode};
use loong_runtime::runtime::Runtime;

use crate::CliResult;
use crate::RuntimeContextFactory;
use crate::config::LoongConfig;
use crate::conversation::{
    ConstrainedSubagentExecution, ConstrainedSubagentIdentity, DelegateBuiltinProfile,
    active_skills,
};
use crate::memory::runtime_config::MemoryRuntimeConfig;
use crate::runtime_self_continuity::RuntimeSelfContinuity;
use crate::session::repository::{
    SessionEventRecord, SessionKind, SessionMaterializationSnapshot, SessionRepository,
    SessionToolPolicyRecord,
};
use crate::tools::ToolView;
use crate::tools::runtime_config::{ToolRuntimeConfig, ToolRuntimeNarrowing};

use super::{Session, SessionToolPolicyProjection, runtime_authority_tool_view};

struct MaterializedSession {
    session: Session,
    tool_policy: SessionToolPolicyProjection,
}

pub(super) struct SessionRematerializationProjection {
    pub(super) tool_policy: SessionToolPolicyProjection,
    pub(super) active_skill_roots: Vec<PathBuf>,
}

/// Materialize the durable Session projection before generic authority ceilings are applied.
pub(super) fn materialize(
    runtime: &Runtime<RuntimeContextFactory>,
    config: &LoongConfig,
    session_id: String,
    agent_id: String,
    session_mode: GovernedSessionMode,
    baseline_capabilities: Capabilities,
    tool_runtime_config: ToolRuntimeConfig,
    memory_runtime_config: MemoryRuntimeConfig,
) -> CliResult<Session> {
    let repo = SessionRepository::from_memory_config_without_env_overrides(&config.memory)?;
    let lineage = load_persisted_session_lineage(&repo, &session_id)?;
    let snapshot = lineage.get(&session_id);

    match snapshot {
        Some(_) => build_session_from_lineage(
            runtime,
            config,
            &lineage,
            &session_id,
            None,
            0,
            agent_id.as_str(),
            session_mode,
            &baseline_capabilities,
            &tool_runtime_config,
            &memory_runtime_config,
        )
        .map(|materialized| materialized.session),
        None => Session::root(
            runtime,
            agent_id,
            session_id,
            session_mode,
            baseline_capabilities,
            tool_runtime_config,
            memory_runtime_config,
            runtime_authority_tool_view(runtime, config),
            None,
            None,
        ),
    }
}

/// Rebuild a persisted direct child from its live parent authority.
///
/// The parent instance, rather than a reconstructed root, supplies backend and
/// authority identity. Durable state may still narrow the derived child.
pub(super) fn materialize_child(
    runtime: &Runtime<RuntimeContextFactory>,
    config: &LoongConfig,
    parent: &Session,
    child_session_id: &str,
) -> CliResult<Session> {
    let repo = SessionRepository::from_memory_config_without_env_overrides(&config.memory)?;
    let lineage = load_persisted_session_lineage(&repo, child_session_id)?;
    let snapshot = lineage.get(child_session_id).ok_or_else(|| {
        format!("session materialization lineage is missing `{child_session_id}`")
    })?;
    if snapshot.parent_session_id.as_deref() != Some(parent.session_id()) {
        return Err(format!(
            "persisted session `{child_session_id}` is not a direct child of `{}`",
            parent.session_id()
        ));
    }
    let memory_runtime_config =
        MemoryRuntimeConfig::from_memory_config_without_env_overrides(&config.memory);

    build_session_from_lineage(
        runtime,
        config,
        &lineage,
        child_session_id,
        Some(parent),
        0,
        parent.agent_id(),
        parent.session_mode,
        parent.baseline_capabilities(),
        &parent.tool_runtime_config,
        &memory_runtime_config,
    )
    .map(|materialized| materialized.session)
}

/// Produce policy status through the same lineage projection as execution.
pub(super) fn materialize_tool_policy_projection(
    runtime: &Runtime<RuntimeContextFactory>,
    config: &LoongConfig,
    session_id: &str,
) -> CliResult<SessionToolPolicyProjection> {
    let repo = SessionRepository::from_memory_config_without_env_overrides(&config.memory)?;
    let lineage = load_persisted_session_lineage(&repo, session_id)?;
    project_session_from_lineage(runtime, config, &lineage, session_id, 0)
}

/// Recompute mutable authority through the complete persisted Session lineage.
///
/// Rematerialization cannot evaluate only the target row: a changed ancestor
/// policy must immediately narrow every live descendant continuation.
pub(super) fn load_rematerialization_projection(
    runtime: &Runtime<RuntimeContextFactory>,
    config: &LoongConfig,
    session_id: &str,
) -> CliResult<SessionRematerializationProjection> {
    let repo = SessionRepository::from_memory_config_without_env_overrides(&config.memory)?;
    let lineage = load_persisted_session_lineage(&repo, session_id)?;
    if lineage.is_empty() {
        // A newly created live root may precede its durable row. With no
        // persisted authority request, only current runtime availability is
        // projected; Session::rematerialize still intersects the live ceiling.
        let tool_view = runtime_authority_tool_view(runtime, config);
        return Ok(SessionRematerializationProjection {
            tool_policy: SessionToolPolicyProjection {
                base_tool_view: tool_view.clone(),
                effective_tool_view: tool_view,
                session_tool_policy: None,
                delegate_runtime_narrowing: None,
                effective_runtime_narrowing: None,
            },
            active_skill_roots: Vec::new(),
        });
    }
    let snapshot = lineage
        .get(session_id)
        .ok_or_else(|| format!("session materialization lineage is missing `{session_id}`"))?;
    let tool_policy = project_session_from_lineage(runtime, config, &lineage, session_id, 0)?;
    Ok(SessionRematerializationProjection {
        tool_policy,
        active_skill_roots: snapshot.active_skill_roots.clone(),
    })
}

/// Project one lineage without constructing Session owners or backend state.
fn project_session_from_lineage(
    runtime: &Runtime<RuntimeContextFactory>,
    config: &LoongConfig,
    lineage: &BTreeMap<String, PersistedSessionSnapshot>,
    session_id: &str,
    depth: usize,
) -> CliResult<SessionToolPolicyProjection> {
    let snapshot = lineage
        .get(session_id)
        .ok_or_else(|| format!("session materialization lineage is missing `{session_id}`"))?;
    if depth > config.tools.delegate.max_depth {
        return Err(format!(
            "persisted session lineage for {session_id} exceeds delegate max depth {}",
            config.tools.delegate.max_depth
        ));
    }

    match snapshot.parent_session_id.as_deref() {
        Some(parent_session_id) => {
            let parent = project_session_from_lineage(
                runtime,
                config,
                lineage,
                parent_session_id,
                depth.saturating_add(1),
            )?;
            project_snapshot(
                runtime,
                config,
                snapshot,
                Some(&parent.effective_tool_view),
                parent.effective_runtime_narrowing,
            )
        }
        None => project_snapshot(runtime, config, snapshot, None, None),
    }
}

/// Apply one Session's requests beneath an already-materialized parent ceiling.
///
/// This is shared by owner construction and owner-preserving rematerialization,
/// so tool visibility and runtime limits cannot drift between those paths.
fn project_snapshot(
    runtime: &Runtime<RuntimeContextFactory>,
    config: &LoongConfig,
    snapshot: &PersistedSessionSnapshot,
    parent_tool_view: Option<&ToolView>,
    parent_runtime_narrowing: Option<ToolRuntimeNarrowing>,
) -> CliResult<SessionToolPolicyProjection> {
    let requested_tool_view = if snapshot.parent_session_id.is_some() || snapshot.is_delegate_child
    {
        let execution = snapshot.subagent_execution.as_ref().ok_or_else(|| {
            format!(
                "delegate session `{}` has no typed execution authority anchor",
                snapshot.session_id
            )
        })?;
        crate::tools::runtime_delegate_child_tool_view(
            runtime,
            &config.tools,
            Some(&execution.contract_view()),
        )
    } else {
        runtime_authority_tool_view(runtime, config)
    };
    let candidate_base_tool_view = apply_active_skill_blocked_tools_to_tool_view(
        requested_tool_view,
        snapshot.active_skills.as_ref(),
    );
    let base_tool_view = match parent_tool_view {
        Some(parent_tool_view) => candidate_base_tool_view.intersect(parent_tool_view),
        None => candidate_base_tool_view.intersect(&runtime_authority_tool_view(runtime, config)),
    };
    let effective_tool_view = apply_session_tool_policy_to_tool_view(
        base_tool_view.clone(),
        snapshot.session_tool_policy.as_ref(),
    )?;
    let delegate_runtime_narrowing = snapshot.delegate_runtime_narrowing.clone();
    let policy_runtime_narrowing = snapshot.session_tool_policy.as_ref().and_then(|policy| {
        (!policy.runtime_narrowing.is_empty()).then_some(policy.runtime_narrowing.clone())
    });
    let local_runtime_narrowing = crate::tools::runtime_config::merge_runtime_narrowing_sources(
        delegate_runtime_narrowing.clone(),
        policy_runtime_narrowing,
    );
    let effective_runtime_narrowing = crate::tools::runtime_config::merge_runtime_narrowing_sources(
        parent_runtime_narrowing,
        local_runtime_narrowing,
    );

    Ok(SessionToolPolicyProjection {
        base_tool_view,
        effective_tool_view,
        session_tool_policy: snapshot.session_tool_policy.clone(),
        delegate_runtime_narrowing,
        effective_runtime_narrowing,
    })
}

fn apply_session_tool_policy_to_tool_view(
    base_tool_view: ToolView,
    session_tool_policy: Option<&SessionToolPolicyRecord>,
) -> Result<ToolView, String> {
    let Some(session_tool_policy) = session_tool_policy else {
        return Ok(base_tool_view);
    };
    if session_tool_policy.requested_tool_ids.is_empty() {
        return Ok(base_tool_view);
    }

    let requested_paths =
        session_tool_policy
            .requested_tool_ids
            .iter()
            .map(|tool_id| {
                tool_id.parse::<loong_contracts::ToolPath>().map_err(|error| {
                format!(
                    "session `{}` has invalid canonical tool policy id `{tool_id}`: {error}",
                    session_tool_policy.session_id
                )
            })
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(base_tool_view.filter(|path, _| requested_paths.contains(path)))
}

struct DelegateAnchorSnapshot {
    execution: Option<ConstrainedSubagentExecution>,
    profile: Option<DelegateBuiltinProfile>,
    workspace_root: Option<PathBuf>,
}

fn delegate_anchor_snapshot(
    events: &[SessionEventRecord],
) -> Result<DelegateAnchorSnapshot, String> {
    let Some(event) = events.iter().rev().find(|event| {
        matches!(
            event.event_kind.as_str(),
            "delegate_queued" | "delegate_started"
        )
    }) else {
        return Ok(DelegateAnchorSnapshot {
            execution: None,
            profile: None,
            workspace_root: None,
        });
    };

    // One lifecycle event is one authority snapshot. Filling missing fields
    // from older events could combine permissions that never existed together.
    let execution_value = event.payload_json.get("execution").ok_or_else(|| {
        format!(
            "latest delegate authority event {} contains no execution",
            event.id
        )
    })?;
    let execution: ConstrainedSubagentExecution = serde_json::from_value(execution_value.clone())
        .map_err(|error| {
        format!(
            "latest delegate authority event {} contains invalid execution: {error}",
            event.id
        )
    })?;
    let profile = match event.payload_json.get("profile") {
        None | Some(serde_json::Value::Null) => None,
        Some(profile) => Some(serde_json::from_value(profile.clone()).map_err(|error| {
            format!(
                "latest delegate authority event {} contains invalid profile: {error}",
                event.id
            )
        })?),
    };
    let workspace_root = execution.workspace_root.clone();

    Ok(DelegateAnchorSnapshot {
        execution: Some(execution),
        profile,
        workspace_root,
    })
}

#[derive(Clone)]
struct PersistedSessionSnapshot {
    session_id: String,
    parent_session_id: Option<String>,
    label: Option<String>,
    is_delegate_child: bool,
    subagent_execution: Option<ConstrainedSubagentExecution>,
    session_tool_policy: Option<SessionToolPolicyRecord>,
    delegate_runtime_narrowing: Option<ToolRuntimeNarrowing>,
    delegate_profile: Option<DelegateBuiltinProfile>,
    workspace_root: Option<PathBuf>,
    active_skills: Option<active_skills::ActiveSkillsState>,
    active_skill_roots: Vec<PathBuf>,
    runtime_self_continuity: Option<RuntimeSelfContinuity>,
}

impl TryFrom<SessionMaterializationSnapshot> for PersistedSessionSnapshot {
    type Error = String;

    fn try_from(snapshot: SessionMaterializationSnapshot) -> Result<Self, Self::Error> {
        let SessionMaterializationSnapshot {
            session,
            tool_policy: session_tool_policy,
            latest_events,
            delegate_events,
        } = snapshot;
        let is_delegate_child =
            session.kind == SessionKind::DelegateChild || session.parent_session_id.is_some();
        let session_id = session.session_id;
        let parent_session_id = session.parent_session_id;
        let label = session.label;

        let DelegateAnchorSnapshot {
            execution,
            profile,
            workspace_root,
        } = delegate_anchor_snapshot(&delegate_events)?;
        let subagent_execution = is_delegate_child.then_some(execution).flatten();
        let delegate_runtime_narrowing = subagent_execution.as_ref().and_then(|execution| {
            (!execution.runtime_narrowing.is_empty()).then_some(execution.runtime_narrowing.clone())
        });
        let delegate_profile = is_delegate_child.then_some(profile).flatten();
        let workspace_root = is_delegate_child.then_some(workspace_root).flatten();
        let runtime_self_continuity = match latest_events.iter().find(|event| {
            event.event_kind == crate::runtime_self_continuity::RUNTIME_SELF_CONTINUITY_EVENT_KIND
        }) {
            Some(event) => Some(
                crate::runtime_self_continuity::runtime_self_continuity_from_event_payload(
                    &event.payload_json,
                )
                .ok_or_else(|| "runtime-self continuity event contains invalid state".to_owned())?,
            ),
            None => delegate_events.iter().rev().find_map(|event| {
                crate::runtime_self_continuity::runtime_self_continuity_from_event_payload(
                    &event.payload_json,
                )
            }),
        };
        let active_skills = latest_events
            .iter()
            .find(|event| event.event_kind == active_skills::ACTIVE_SKILLS_EVENT_KIND)
            .map(|event| active_skills::active_skills_from_event_payload(&event.payload_json))
            .transpose()?;
        let active_skill_roots = active_skill_roots_from_state(active_skills.as_ref());

        Ok(Self {
            session_id,
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
        })
    }
}

fn load_persisted_session_lineage(
    repo: &SessionRepository,
    session_id: &str,
) -> CliResult<BTreeMap<String, PersistedSessionSnapshot>> {
    let raw_lineage = repo
        .load_session_materialization_lineage(session_id)
        .map_err(|error| format!("load session materialization lineage failed: {error}"))?;
    let mut lineage = BTreeMap::new();
    for raw_snapshot in raw_lineage {
        let snapshot = PersistedSessionSnapshot::try_from(raw_snapshot)?;
        let snapshot_id = snapshot.session_id.clone();
        if lineage.insert(snapshot_id.clone(), snapshot).is_some() {
            return Err(format!(
                "session materialization lineage contains duplicate `{snapshot_id}`"
            ));
        }
    }
    Ok(lineage)
}

fn active_skill_roots_from_state(
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
        if !roots.contains(&path) {
            roots.push(path);
        }
    }
    roots
}

fn apply_active_skill_blocked_tools_to_tool_view(
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

    base_tool_view.filter(|path, provider_name| {
        !blocked_names.contains(provider_name) && !blocked_names.contains(&path.to_string())
    })
}

/// Rebuild the real parent chain before deriving child authority.
///
/// Persisted parent ids are evidence for lookup, not permission to choose an
/// arbitrary parent. Repository materialization has already captured the whole
/// lineage in one SQLite snapshot, and every child is derived from its parent.
fn build_session_from_lineage(
    runtime: &Runtime<RuntimeContextFactory>,
    config: &LoongConfig,
    lineage: &BTreeMap<String, PersistedSessionSnapshot>,
    session_id: &str,
    live_parent: Option<&Session>,
    depth: usize,
    agent_id: &str,
    session_mode: GovernedSessionMode,
    baseline_capabilities: &Capabilities,
    tool_runtime_config: &ToolRuntimeConfig,
    memory_runtime_config: &MemoryRuntimeConfig,
) -> CliResult<MaterializedSession> {
    let snapshot = lineage
        .get(session_id)
        .ok_or_else(|| format!("session materialization lineage is missing `{session_id}`"))?;
    if snapshot.session_id != session_id {
        return Err(format!(
            "persisted session identity mismatch: requested {session_id}, loaded {}",
            snapshot.session_id
        ));
    }
    if depth > config.tools.delegate.max_depth {
        return Err(format!(
            "persisted session lineage for {session_id} exceeds delegate max depth {}",
            config.tools.delegate.max_depth
        ));
    }

    let (mut session, tool_policy) = match snapshot.parent_session_id.clone() {
        Some(parent_session_id) => {
            let (parent, parent_runtime_narrowing) = match live_parent {
                Some(parent) if parent.session_id() == parent_session_id => {
                    (parent.clone(), parent.resolved_runtime_narrowing().cloned())
                }
                Some(parent) => {
                    return Err(format!(
                        "persisted session `{session_id}` is not a direct child of `{}`",
                        parent.session_id()
                    ));
                }
                None => {
                    let parent = build_session_from_lineage(
                        runtime,
                        config,
                        lineage,
                        &parent_session_id,
                        None,
                        depth.saturating_add(1),
                        agent_id,
                        session_mode,
                        baseline_capabilities,
                        tool_runtime_config,
                        memory_runtime_config,
                    )?;
                    let parent_runtime_narrowing =
                        parent.tool_policy.effective_runtime_narrowing.clone();
                    (parent.session, parent_runtime_narrowing)
                }
            };
            let tool_policy = project_snapshot(
                runtime,
                config,
                snapshot,
                Some(&parent.tool_view),
                parent_runtime_narrowing,
            )?;
            let mut execution = snapshot.subagent_execution.clone().ok_or_else(|| {
                format!("delegate session `{session_id}` has no typed execution authority anchor")
            })?;
            if execution.identity.is_none()
                && let Some(label) = snapshot.label.clone()
            {
                execution.identity = Some(ConstrainedSubagentIdentity {
                    nickname: Some(label),
                    specialization: None,
                });
            }
            let session = parent.delegate_child(
                snapshot.session_id.clone(),
                tool_policy.effective_tool_view.clone(),
                execution,
                tool_policy.effective_runtime_narrowing.as_ref(),
            )?;
            (session, tool_policy)
        }
        None => {
            if live_parent.is_some() {
                return Err(format!(
                    "persisted session `{session_id}` is a root, not a child"
                ));
            }
            let tool_policy = project_snapshot(runtime, config, snapshot, None, None)?;
            let session = Session::root(
                runtime,
                agent_id,
                snapshot.session_id.clone(),
                session_mode,
                baseline_capabilities.clone(),
                tool_runtime_config.clone(),
                memory_runtime_config.clone(),
                tool_policy.effective_tool_view.clone(),
                snapshot.workspace_root.clone(),
                tool_policy.effective_runtime_narrowing.as_ref(),
            )?;
            (session, tool_policy)
        }
    };
    if let Some(profile) = snapshot.delegate_profile {
        session = session.with_profile(profile);
    }
    if !snapshot.active_skill_roots.is_empty() {
        session = session.with_active_skill_roots(snapshot.active_skill_roots.clone());
    }
    if let Some(runtime_self_continuity) = snapshot.runtime_self_continuity.clone() {
        session = session.with_runtime_self_continuity(runtime_self_continuity);
    }
    Ok(MaterializedSession {
        session,
        tool_policy,
    })
}

#[cfg(test)]
#[path = "sqlite/tests.rs"]
mod tests;
