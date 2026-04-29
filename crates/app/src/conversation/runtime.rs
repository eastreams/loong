use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use loong_contracts::Capability;
use serde_json::Value;

use crate::CliResult;
use crate::KernelContext;
#[cfg(feature = "memory-sqlite")]
use crate::operator::delegate_runtime::{
    derive_subagent_profile_from_lineage, resolve_delegate_child_contract,
};
use crate::runtime_self_continuity::{self, RuntimeSelfContinuity};
use crate::tools::runtime_config::ToolRuntimeNarrowing;
use crate::tools::{ToolView, delegate_child_tool_view_for_contract};

use super::super::memory;
use super::super::{config::LoongConfig, provider};
#[cfg(feature = "memory-sqlite")]
use super::active_external_skills;
use super::context_engine::ContextArtifactKind;
use super::context_engine::{
    AssembledConversationContext, ContextEngineBootstrapResult, ContextEngineIngestResult,
    ContextEngineMetadata, ConversationContextEngine, DefaultContextEngine,
};
use super::context_engine_registry::{
    DEFAULT_CONTEXT_ENGINE_ID, context_engine_id_from_env, describe_context_engine,
    list_context_engine_metadata, resolve_context_engine,
};
use super::mailbox_for_session;
use super::prompt_orchestrator::seed_prompt_fragments_from_context;
use super::prompt_orchestrator::sync_prompt_fragments_into_context;
use super::runtime_binding::{ConversationRuntimeBinding, OwnedConversationRuntimeBinding};
use super::subagent::{
    ConstrainedSubagentContractView, ConstrainedSubagentExecution, ConstrainedSubagentIdentity,
    ConstrainedSubagentProfile, DelegateBuiltinProfile,
};
use super::turn_engine::ProviderTurn;
use super::turn_middleware::{
    ConversationTurnMiddleware, TurnMiddlewareMetadata, builtin_turn_middlewares,
};
use super::turn_middleware_registry::{
    default_turn_middleware_ids, describe_turn_middlewares, list_turn_middleware_metadata,
    resolve_turn_middlewares, turn_middleware_ids_from_env,
};
use super::{PromptFragment, PromptFrameAuthority, PromptLane};

#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    SessionKind, SessionRepository, SessionState, SessionToolPolicyRecord,
    TransitionSessionWithEventIfCurrentRequest,
};
#[cfg(feature = "memory-sqlite")]
use crate::session::store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContext {
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub profile: Option<DelegateBuiltinProfile>,
    pub tool_view: ToolView,
    pub workspace_root: Option<PathBuf>,
    pub active_external_skill_roots: Vec<PathBuf>,
    pub visible_external_skill_roots: Vec<PathBuf>,
    pub runtime_narrowing: Option<ToolRuntimeNarrowing>,
    pub subagent_execution: Option<ConstrainedSubagentExecution>,
    pub subagent_contract: Option<ConstrainedSubagentContractView>,
    pub(crate) runtime_self_continuity: Option<RuntimeSelfContinuity>,
}

impl SessionContext {
    pub fn root_with_tool_view(session_id: impl Into<String>, tool_view: ToolView) -> Self {
        let session_id = normalize_session_id(session_id.into());
        let _ = mailbox_for_session(&session_id);
        Self {
            session_id,
            parent_session_id: None,
            profile: None,
            tool_view,
            workspace_root: None,
            active_external_skill_roots: Vec::new(),
            visible_external_skill_roots: Vec::new(),
            runtime_narrowing: None,
            subagent_execution: None,
            subagent_contract: None,
            runtime_self_continuity: None,
        }
    }

    pub fn child(
        session_id: impl Into<String>,
        parent_session_id: impl Into<String>,
        tool_view: ToolView,
    ) -> Self {
        let session_id = normalize_session_id(session_id.into());
        let parent_session_id = normalize_session_id(parent_session_id.into());
        let _ = mailbox_for_session(&session_id);
        let _ = mailbox_for_session(&parent_session_id);
        Self {
            session_id,
            parent_session_id: Some(parent_session_id),
            profile: None,
            tool_view,
            workspace_root: None,
            active_external_skill_roots: Vec::new(),
            visible_external_skill_roots: Vec::new(),
            runtime_narrowing: None,
            subagent_execution: None,
            subagent_contract: None,
            runtime_self_continuity: None,
        }
    }

    #[must_use]
    pub fn with_workspace_root(mut self, workspace_root: PathBuf) -> Self {
        self.workspace_root = Some(workspace_root);
        self
    }

    #[must_use]
    pub fn with_active_external_skill_roots(
        mut self,
        active_external_skill_roots: Vec<PathBuf>,
    ) -> Self {
        self.active_external_skill_roots = active_external_skill_roots
            .into_iter()
            .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
            .collect();
        self
    }

    #[must_use]
    pub fn with_visible_external_skill_roots(
        mut self,
        visible_external_skill_roots: Vec<PathBuf>,
    ) -> Self {
        self.visible_external_skill_roots = visible_external_skill_roots
            .into_iter()
            .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
            .collect();
        self
    }

    #[must_use]
    pub fn with_profile(mut self, profile: DelegateBuiltinProfile) -> Self {
        self.profile = Some(profile);
        self
    }

    #[must_use]
    pub fn with_runtime_narrowing(mut self, runtime_narrowing: ToolRuntimeNarrowing) -> Self {
        if !runtime_narrowing.is_empty() {
            self.runtime_narrowing = Some(runtime_narrowing.clone());
            let contract = self.subagent_contract.take().unwrap_or_default();
            self.subagent_contract = Some(contract.with_runtime_narrowing(runtime_narrowing));
            self.synchronize_runtime_narrowing_views();
        }
        self
    }

    #[must_use]
    pub fn with_subagent_execution(
        mut self,
        subagent_execution: ConstrainedSubagentExecution,
    ) -> Self {
        let existing_contract = self.subagent_contract.take();
        let existing_workspace_root = self.workspace_root.clone();
        let existing_identity = existing_contract
            .as_ref()
            .and_then(ConstrainedSubagentContractView::resolved_identity)
            .cloned();
        let existing_profile = existing_contract
            .as_ref()
            .and_then(|contract| contract.profile);
        let existing_runtime_narrowing = existing_contract
            .as_ref()
            .map(|contract| contract.runtime_narrowing.clone())
            .filter(|runtime_narrowing| !runtime_narrowing.is_empty());
        let mut subagent_execution = subagent_execution.with_resolved_profile();
        if subagent_execution.identity.is_none()
            && let Some(identity) = existing_identity
        {
            subagent_execution.identity = Some(identity);
        }
        let mut merged_contract = subagent_execution.contract_view();
        if merged_contract.profile.is_none()
            && let Some(profile) = existing_profile
        {
            merged_contract = merged_contract.with_profile(profile);
        }
        if merged_contract.runtime_narrowing.is_empty()
            && let Some(runtime_narrowing) = existing_runtime_narrowing
        {
            merged_contract = merged_contract.with_runtime_narrowing(runtime_narrowing);
        }
        if self.workspace_root.is_none() {
            let execution_workspace_root = subagent_execution.workspace_root.clone();
            self.workspace_root = execution_workspace_root.or(existing_workspace_root);
        }
        self.subagent_contract = Some(merged_contract);
        self.subagent_execution = Some(subagent_execution);
        self.synchronize_runtime_narrowing_views();
        self
    }

    #[must_use]
    pub fn with_subagent_profile(mut self, subagent_profile: ConstrainedSubagentProfile) -> Self {
        if let Some(subagent_execution) = self.subagent_execution.as_mut() {
            subagent_execution.profile = Some(subagent_profile);
        }
        let contract = self.subagent_contract.take().unwrap_or_default();
        self.subagent_contract = Some(contract.with_profile(subagent_profile));
        self.synchronize_runtime_narrowing_views();
        self
    }

    #[must_use]
    pub fn with_subagent_identity(
        mut self,
        subagent_identity: ConstrainedSubagentIdentity,
    ) -> Self {
        if subagent_identity.is_empty() {
            return self;
        }
        if let Some(subagent_execution) = self.subagent_execution.as_mut() {
            subagent_execution.identity = Some(subagent_identity.clone());
        }
        let contract = self.subagent_contract.take().unwrap_or_default();
        self.subagent_contract = Some(contract.with_identity(subagent_identity));
        self.synchronize_runtime_narrowing_views();
        self
    }

    pub fn resolved_runtime_narrowing(&self) -> Option<&ToolRuntimeNarrowing> {
        self.resolve_runtime_narrowing_ref()
    }

    pub fn resolved_subagent_profile(&self) -> Option<ConstrainedSubagentProfile> {
        self.subagent_execution
            .as_ref()
            .map(ConstrainedSubagentExecution::resolved_profile)
            .or_else(|| {
                self.subagent_contract
                    .as_ref()
                    .and_then(ConstrainedSubagentContractView::resolved_profile)
            })
    }

    pub fn resolved_subagent_identity(&self) -> Option<&ConstrainedSubagentIdentity> {
        self.subagent_execution
            .as_ref()
            .and_then(|execution| execution.identity.as_ref())
            .or_else(|| {
                self.subagent_contract
                    .as_ref()
                    .and_then(ConstrainedSubagentContractView::resolved_identity)
            })
    }

    pub fn resolved_subagent_contract(&self) -> Option<ConstrainedSubagentContractView> {
        let mut contract = self
            .subagent_execution
            .as_ref()
            .map(ConstrainedSubagentExecution::contract_view)
            .or(self.subagent_contract.clone())?;
        if let Some(stored_contract) = self.subagent_contract.as_ref()
            && contract.profile.is_none()
            && let Some(profile) = stored_contract.profile
        {
            contract = contract.with_profile(profile);
        }
        let resolved_runtime_narrowing = self.resolved_runtime_narrowing().cloned();
        if let Some(runtime_narrowing) = resolved_runtime_narrowing {
            contract = contract.with_runtime_narrowing(runtime_narrowing);
        }
        (!contract.is_empty()).then_some(contract)
    }

    pub fn subagent_runtime_narrowing(&self) -> Option<&ToolRuntimeNarrowing> {
        self.resolved_runtime_narrowing()
    }

    #[must_use]
    pub(crate) fn with_runtime_self_continuity(
        mut self,
        runtime_self_continuity: RuntimeSelfContinuity,
    ) -> Self {
        if !runtime_self_continuity.is_empty() {
            self.runtime_self_continuity = Some(runtime_self_continuity);
        }
        self
    }

    fn resolve_runtime_narrowing_owned(&self) -> Option<ToolRuntimeNarrowing> {
        let resolved_runtime_narrowing = self.resolve_runtime_narrowing_ref();
        resolved_runtime_narrowing.cloned()
    }

    fn synchronize_runtime_narrowing_views(&mut self) {
        let resolved_runtime_narrowing = self.resolve_runtime_narrowing_owned();
        let execution_runtime_narrowing = resolved_runtime_narrowing.clone().unwrap_or_default();
        let contract_runtime_narrowing = execution_runtime_narrowing.clone();

        self.runtime_narrowing = resolved_runtime_narrowing;
        if let Some(subagent_execution) = self.subagent_execution.as_mut() {
            subagent_execution.runtime_narrowing = execution_runtime_narrowing;
        }
        if let Some(subagent_contract) = self.subagent_contract.as_mut() {
            subagent_contract.runtime_narrowing = contract_runtime_narrowing;
        }
    }

    fn resolve_runtime_narrowing_ref(&self) -> Option<&ToolRuntimeNarrowing> {
        let session_runtime_narrowing =
            non_empty_runtime_narrowing_ref(self.runtime_narrowing.as_ref());
        if let Some(session_runtime_narrowing) = session_runtime_narrowing {
            return Some(session_runtime_narrowing);
        }

        let execution_runtime_narrowing_source = self
            .subagent_execution
            .as_ref()
            .map(|execution| &execution.runtime_narrowing);
        let execution_runtime_narrowing =
            non_empty_runtime_narrowing_ref(execution_runtime_narrowing_source);
        if let Some(execution_runtime_narrowing) = execution_runtime_narrowing {
            return Some(execution_runtime_narrowing);
        }

        let contract_runtime_narrowing_source = self
            .subagent_contract
            .as_ref()
            .map(|contract| &contract.runtime_narrowing);
        non_empty_runtime_narrowing_ref(contract_runtime_narrowing_source)
    }
}

fn configured_root_session_workspace_root(config: &LoongConfig) -> Option<PathBuf> {
    config
        .tools
        .configured_runtime_workspace_root()
        .or_else(|| config.tools.configured_file_root())
        .and_then(|workspace_root| {
            let canonical_workspace_root = dunce::canonicalize(&workspace_root).ok()?;
            canonical_workspace_root
                .is_dir()
                .then_some(canonical_workspace_root)
        })
}

fn root_session_context_from_config(
    config: &LoongConfig,
    session_id: impl Into<String>,
    tool_view: ToolView,
) -> SessionContext {
    let mut session_context = SessionContext::root_with_tool_view(session_id, tool_view);
    if let Some(workspace_root) = configured_root_session_workspace_root(config) {
        session_context = session_context.with_workspace_root(workspace_root);
    }
    session_context
}

fn non_empty_runtime_narrowing_ref(
    runtime_narrowing: Option<&ToolRuntimeNarrowing>,
) -> Option<&ToolRuntimeNarrowing> {
    runtime_narrowing.filter(|runtime_narrowing| !runtime_narrowing.is_empty())
}

fn normalize_session_id(session_id: String) -> String {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        "default".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(feature = "memory-sqlite")]
fn load_session_tool_policy(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<SessionToolPolicyRecord>, String> {
    repo.load_session_tool_policy(session_id)
}

#[cfg(feature = "memory-sqlite")]
fn apply_session_tool_policy_to_tool_view(
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
fn merge_effective_runtime_narrowing(
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
struct DelegateAnchorSnapshot {
    execution: Option<ConstrainedSubagentExecution>,
    profile: Option<DelegateBuiltinProfile>,
    workspace_root: Option<PathBuf>,
}

#[cfg(feature = "memory-sqlite")]
fn load_delegate_anchor_snapshot(
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
            execution = super::subagent::ConstrainedSubagentExecution::from_event_payload(
                &event.payload_json,
            );
        }
        if profile.is_none() {
            profile = super::subagent::ConstrainedSubagentExecution::profile_from_event_payload(
                &event.payload_json,
            );
        }
        if workspace_root.is_none() {
            let event_workspace_root =
                super::subagent::ConstrainedSubagentExecution::from_event_payload(
                    &event.payload_json,
                )
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
fn load_delegate_execution_contract(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<ConstrainedSubagentExecution>, String> {
    let snapshot = load_delegate_anchor_snapshot(repo, session_id)?;
    Ok(snapshot.execution)
}

#[cfg(feature = "memory-sqlite")]
fn load_delegate_profile(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<DelegateBuiltinProfile>, String> {
    let snapshot = load_delegate_anchor_snapshot(repo, session_id)?;
    Ok(snapshot.profile)
}

#[cfg(feature = "memory-sqlite")]
fn load_delegate_workspace_root(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<PathBuf>, String> {
    let snapshot = load_delegate_anchor_snapshot(repo, session_id)?;
    Ok(snapshot.workspace_root)
}

#[cfg(feature = "memory-sqlite")]
fn load_session_runtime_self_continuity(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<RuntimeSelfContinuity>, String> {
    runtime_self_continuity::load_persisted_runtime_self_continuity(repo, session_id)
}

#[cfg(feature = "memory-sqlite")]
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
    active_external_skills: Option<active_external_skills::ActiveExternalSkillsState>,
    active_external_skill_roots: Vec<PathBuf>,
    runtime_self_continuity: Option<RuntimeSelfContinuity>,
}

#[cfg(feature = "memory-sqlite")]
fn open_session_repository(config: &LoongConfig) -> CliResult<SessionRepository> {
    let memory_config =
        store::session_store_config_from_memory_config_without_env_overrides(&config.memory);
    SessionRepository::new(&memory_config)
        .map_err(|error| format!("open session repository failed: {error}"))
}

#[cfg(feature = "memory-sqlite")]
fn load_persisted_session_snapshot(
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
        let active_external_skills =
            load_active_external_skills_state(repo, session_id).unwrap_or_default();
        let active_external_skill_roots =
            active_external_skill_roots_from_state(active_external_skills.as_ref());
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
            active_external_skills,
            active_external_skill_roots,
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
    let active_external_skills =
        load_active_external_skills_state(repo, session_id).unwrap_or_default();
    let active_external_skill_roots =
        active_external_skill_roots_from_state(active_external_skills.as_ref());
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
        active_external_skills,
        active_external_skill_roots,
        runtime_self_continuity,
    };
    Ok(Some(snapshot))
}

#[cfg(feature = "memory-sqlite")]
fn load_active_external_skills_state(
    repo: &SessionRepository,
    session_id: &str,
) -> Result<Option<active_external_skills::ActiveExternalSkillsState>, String> {
    active_external_skills::load_persisted_active_external_skills(repo, session_id)
}

#[cfg(feature = "memory-sqlite")]
fn active_external_skill_roots_from_state(
    active_skills: Option<&active_external_skills::ActiveExternalSkillsState>,
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

fn model_visible_external_skill_roots_from_config(config: &LoongConfig) -> Vec<PathBuf> {
    let tool_runtime_config =
        crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
    crate::tools::model_visible_external_skill_roots_for_runtime_config(&tool_runtime_config)
}

#[cfg(feature = "memory-sqlite")]
fn apply_active_external_skill_blocked_tools_to_tool_view(
    base_tool_view: ToolView,
    active_skills: Option<&active_external_skills::ActiveExternalSkillsState>,
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
            if let Some(hidden_facade_tool_name) =
                crate::tools::hidden_facade_tool_name_for_hidden_tool(blocked_tool)
            {
                blocked_names.insert(hidden_facade_tool_name.to_owned());
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
fn build_base_tool_view_from_snapshot(
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
fn build_session_context_from_snapshot(
    config: &LoongConfig,
    repo: &SessionRepository,
    session_id: &str,
    base_tool_view: ToolView,
    snapshot: PersistedSessionSnapshot,
) -> CliResult<SessionContext> {
    let visible_external_skill_roots = model_visible_external_skill_roots_from_config(config);
    let tool_view = apply_active_external_skill_blocked_tools_to_tool_view(
        apply_session_tool_policy_to_tool_view(
            base_tool_view,
            snapshot.session_tool_policy.as_ref(),
        ),
        snapshot.active_external_skills.as_ref(),
    );
    let runtime_narrowing = merge_effective_runtime_narrowing(
        snapshot.delegate_runtime_narrowing.clone(),
        snapshot.session_tool_policy.as_ref(),
    );
    let mut session_context = match snapshot.parent_session_id.clone() {
        Some(parent_session_id) => {
            SessionContext::child(snapshot.session_id.clone(), parent_session_id, tool_view)
        }
        None => root_session_context_from_config(config, snapshot.session_id.clone(), tool_view),
    };
    if let Some(profile) = snapshot.delegate_profile {
        session_context = session_context.with_profile(profile);
    }
    if let Some(workspace_root) = snapshot.workspace_root {
        session_context = session_context.with_workspace_root(workspace_root);
    }
    if !snapshot.active_external_skill_roots.is_empty() {
        session_context =
            session_context.with_active_external_skill_roots(snapshot.active_external_skill_roots);
    }
    if !visible_external_skill_roots.is_empty() {
        session_context =
            session_context.with_visible_external_skill_roots(visible_external_skill_roots);
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
fn load_persisted_session_context(
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

#[derive(Clone)]
pub struct AsyncDelegateSpawnRequest {
    pub child_session_id: String,
    pub parent_session_id: String,
    pub task: String,
    pub canonical_task_id: Option<String>,
    pub label: Option<String>,
    pub profile: Option<DelegateBuiltinProfile>,
    pub execution: ConstrainedSubagentExecution,
    pub(crate) runtime_self_continuity: Option<RuntimeSelfContinuity>,
    pub timeout_seconds: u64,
    pub binding: OwnedConversationRuntimeBinding,
}

impl AsyncDelegateSpawnRequest {
    pub fn runtime_self_continuity_json(&self) -> Result<Option<Value>, String> {
        let continuity = self.runtime_self_continuity.as_ref();
        let encoded_continuity =
            continuity
                .map(serde_json::to_value)
                .transpose()
                .map_err(|error| {
                    format!("serialize async delegate runtime-self continuity failed: {error}")
                })?;

        Ok(encoded_continuity)
    }
}

pub fn async_delegate_spawn_request_from_serialized_parts(
    child_session_id: String,
    parent_session_id: String,
    task: String,
    canonical_task_id: Option<String>,
    label: Option<String>,
    profile: Option<DelegateBuiltinProfile>,
    execution: ConstrainedSubagentExecution,
    runtime_self_continuity_json: Option<Value>,
    timeout_seconds: u64,
    binding: OwnedConversationRuntimeBinding,
) -> Result<AsyncDelegateSpawnRequest, String> {
    let runtime_self_continuity = runtime_self_continuity_json
        .map(serde_json::from_value::<RuntimeSelfContinuity>)
        .transpose()
        .map_err(|error| format!("parse async delegate runtime-self continuity failed: {error}"))?;
    let request = AsyncDelegateSpawnRequest {
        child_session_id,
        parent_session_id,
        task,
        canonical_task_id,
        label,
        profile,
        execution,
        runtime_self_continuity,
        timeout_seconds,
        binding,
    };

    Ok(request)
}

#[async_trait]
pub trait AsyncDelegateSpawner: Send + Sync {
    async fn spawn(&self, request: AsyncDelegateSpawnRequest) -> Result<(), String>;
}

#[cfg(feature = "memory-sqlite")]
#[derive(Clone)]
struct DefaultAsyncDelegateSpawner {
    config: Arc<LoongConfig>,
}

#[cfg(feature = "memory-sqlite")]
impl DefaultAsyncDelegateSpawner {
    fn new(config: &LoongConfig) -> Self {
        Self {
            config: Arc::new(config.clone()),
        }
    }
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl AsyncDelegateSpawner for DefaultAsyncDelegateSpawner {
    async fn spawn(&self, request: AsyncDelegateSpawnRequest) -> Result<(), String> {
        execute_async_delegate_spawn_request(self.config.as_ref(), request).await?;
        Ok(())
    }
}

#[cfg(feature = "memory-sqlite")]
pub async fn execute_async_delegate_spawn_request(
    config: &LoongConfig,
    request: AsyncDelegateSpawnRequest,
) -> Result<(), String> {
    let AsyncDelegateSpawnRequest {
        child_session_id,
        parent_session_id,
        task,
        canonical_task_id,
        label,
        profile,
        execution,
        runtime_self_continuity,
        timeout_seconds,
        binding,
    } = request;

    let execution_timeout_seconds = execution.timeout_seconds;

    if timeout_seconds != execution_timeout_seconds {
        return Err(format!(
            "async_delegate_timeout_mismatch: request timeout {} != execution timeout {}",
            timeout_seconds, execution_timeout_seconds
        ));
    }

    let memory_config =
        store::session_store_config_from_memory_config_without_env_overrides(&config.memory);
    let repo = SessionRepository::new(&memory_config)?;
    let runtime = load_default_conversation_runtime(config)?;
    let runtime_ref = &runtime;
    let child_session_id_for_spawn = child_session_id.clone();
    let parent_session_id_for_spawn = parent_session_id.clone();
    let borrowed_binding = binding.as_borrowed();
    let child_binding = binding.clone();

    super::delegate_support::with_prepared_subagent_spawn_cleanup_if_kernel_bound(
        runtime_ref,
        &parent_session_id,
        &child_session_id,
        borrowed_binding,
        move || async move {
            let event_payload_json = execution
                .spawn_payload_with_profile_and_runtime_self_continuity(
                    &task,
                    label.as_deref(),
                    profile,
                    runtime_self_continuity.as_ref(),
                    canonical_task_id.as_deref(),
                    Some(child_session_id_for_spawn.as_str()),
                );
            let transition_request = TransitionSessionWithEventIfCurrentRequest {
                expected_state: SessionState::Ready,
                next_state: SessionState::Running,
                last_error: None,
                event_kind: "delegate_started".to_owned(),
                actor_session_id: Some(parent_session_id_for_spawn.clone()),
                event_payload_json,
            };
            let started = repo.transition_session_with_event_if_current(
                &child_session_id_for_spawn,
                transition_request,
            )?;

            if started.is_none() {
                return Err(format!(
                    "async_delegate_spawn_skipped: session `{}` was not in Ready state",
                    child_session_id_for_spawn
                ));
            }

            let _ = super::turn_coordinator::run_started_delegate_child_turn_with_runtime(
                config,
                runtime_ref,
                &child_session_id_for_spawn,
                &parent_session_id_for_spawn,
                label,
                &task,
                profile,
                execution,
                execution_timeout_seconds,
                child_binding.as_borrowed(),
            )
            .await;

            Ok(())
        },
    )
    .await?;

    Ok(())
}

pub struct DefaultConversationRuntime<E = DefaultContextEngine> {
    context_engine: E,
    turn_middlewares: Vec<Box<dyn ConversationTurnMiddleware>>,
}

pub type BoxedDefaultConversationRuntime =
    DefaultConversationRuntime<Box<dyn ConversationContextEngine>>;

#[cfg(feature = "memory-sqlite")]
#[derive(Clone)]
pub struct HostedConversationRuntime<R> {
    inner: R,
    memory_config: store::SessionStoreConfig,
    async_delegate_spawner_override: Option<Arc<dyn AsyncDelegateSpawner>>,
    background_task_spawner_override: Option<Arc<dyn AsyncDelegateSpawner>>,
}

#[cfg(feature = "memory-sqlite")]
impl<R> HostedConversationRuntime<R> {
    pub fn new(inner: R) -> Self {
        let memory_config = store::current_session_store_config().clone();
        Self::new_with_memory_config(inner, memory_config)
    }

    pub fn new_with_memory_config(inner: R, memory_config: store::SessionStoreConfig) -> Self {
        Self {
            inner,
            memory_config,
            async_delegate_spawner_override: None,
            background_task_spawner_override: None,
        }
    }

    #[must_use]
    pub fn with_async_delegate_spawner(
        mut self,
        async_delegate_spawner: Arc<dyn AsyncDelegateSpawner>,
    ) -> Self {
        self.async_delegate_spawner_override = Some(async_delegate_spawner);
        self
    }

    #[must_use]
    pub fn with_background_task_spawner(
        mut self,
        background_task_spawner: Arc<dyn AsyncDelegateSpawner>,
    ) -> Self {
        self.background_task_spawner_override = Some(background_task_spawner);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextEngineSelectionSource {
    Env,
    Config,
    Default,
}

impl ContextEngineSelectionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ContextEngineSelectionSource::Env => "env",
            ContextEngineSelectionSource::Config => "config",
            ContextEngineSelectionSource::Default => "default",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnMiddlewareSelectionSource {
    Env,
    Config,
    Default,
}

impl TurnMiddlewareSelectionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            TurnMiddlewareSelectionSource::Env => "env",
            TurnMiddlewareSelectionSource::Config => "config",
            TurnMiddlewareSelectionSource::Default => "default",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEngineSelection {
    pub id: String,
    pub source: ContextEngineSelectionSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnMiddlewareSelection {
    pub ids: Vec<String>,
    pub source: TurnMiddlewareSelectionSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCompactionPolicySnapshot {
    pub enabled: bool,
    pub min_messages: Option<usize>,
    pub trigger_estimated_tokens: Option<usize>,
    pub preserve_recent_turns: usize,
    pub preserve_recent_estimated_tokens: Option<usize>,
    pub fail_open: bool,
}

impl ContextCompactionPolicySnapshot {
    pub fn hygiene_strategy(&self) -> &'static str {
        if !self.enabled {
            return "disabled";
        }
        if self.preserve_recent_estimated_tokens.is_some() {
            return "turn_floor_plus_token_budget";
        }
        "turn_floor_only"
    }

    pub fn diagnostics_surface(&self) -> &'static str {
        "turn_checkpoint"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEngineRuntimeSnapshot {
    pub selected: ContextEngineSelection,
    pub selected_metadata: ContextEngineMetadata,
    pub available: Vec<ContextEngineMetadata>,
    pub turn_middlewares: TurnMiddlewareRuntimeSnapshot,
    pub compaction: ContextCompactionPolicySnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnMiddlewareRuntimeSnapshot {
    pub selected: TurnMiddlewareSelection,
    pub selected_metadata: Vec<TurnMiddlewareMetadata>,
    pub available: Vec<TurnMiddlewareMetadata>,
}

pub fn resolve_context_engine_selection(config: &LoongConfig) -> ContextEngineSelection {
    if let Some(id) = context_engine_id_from_env() {
        return ContextEngineSelection {
            id,
            source: ContextEngineSelectionSource::Env,
        };
    }

    if let Some(id) = config.conversation.context_engine_id() {
        return ContextEngineSelection {
            id,
            source: ContextEngineSelectionSource::Config,
        };
    }

    ContextEngineSelection {
        id: DEFAULT_CONTEXT_ENGINE_ID.to_owned(),
        source: ContextEngineSelectionSource::Default,
    }
}

pub fn resolve_turn_middleware_selection(
    config: &LoongConfig,
) -> CliResult<TurnMiddlewareSelection> {
    let mut ids = default_turn_middleware_ids()?;
    if let Some(env_ids) = turn_middleware_ids_from_env() {
        ids.extend(env_ids);
        return Ok(TurnMiddlewareSelection {
            ids: normalize_turn_middleware_ids(ids),
            source: TurnMiddlewareSelectionSource::Env,
        });
    }

    let configured_ids = config.conversation.turn_middleware_ids();
    if !configured_ids.is_empty() {
        ids.extend(configured_ids);
        return Ok(TurnMiddlewareSelection {
            ids: normalize_turn_middleware_ids(ids),
            source: TurnMiddlewareSelectionSource::Config,
        });
    }

    Ok(TurnMiddlewareSelection {
        ids: normalize_turn_middleware_ids(ids),
        source: TurnMiddlewareSelectionSource::Default,
    })
}

pub fn collect_context_engine_runtime_snapshot(
    config: &LoongConfig,
) -> CliResult<ContextEngineRuntimeSnapshot> {
    let selected = resolve_context_engine_selection(config);
    let selected_metadata = describe_context_engine(Some(selected.id.as_str()))?;
    let available = list_context_engine_metadata()?;
    let turn_middleware_selection = resolve_turn_middleware_selection(config)?;
    let turn_middlewares = TurnMiddlewareRuntimeSnapshot {
        selected_metadata: describe_turn_middlewares(turn_middleware_selection.ids.as_slice())?,
        available: list_turn_middleware_metadata()?,
        selected: turn_middleware_selection,
    };
    let compaction = ContextCompactionPolicySnapshot {
        enabled: config.conversation.compact_enabled,
        min_messages: config.conversation.compact_min_messages(),
        trigger_estimated_tokens: config.conversation.compact_trigger_estimated_tokens(),
        preserve_recent_turns: config.conversation.compact_preserve_recent_turns(),
        preserve_recent_estimated_tokens: config
            .conversation
            .compact_preserve_recent_estimated_tokens(),
        fail_open: config.conversation.compaction_fail_open(),
    };

    Ok(ContextEngineRuntimeSnapshot {
        selected,
        selected_metadata,
        available,
        turn_middlewares,
        compaction,
    })
}

impl Default for DefaultConversationRuntime<DefaultContextEngine> {
    fn default() -> Self {
        Self::with_context_engine(DefaultContextEngine)
    }
}

impl DefaultConversationRuntime<DefaultContextEngine> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_turn_middlewares(
        turn_middlewares: Vec<Box<dyn ConversationTurnMiddleware>>,
    ) -> Self {
        Self::with_context_engine_and_turn_middlewares(DefaultContextEngine, turn_middlewares)
    }
}

impl<E> DefaultConversationRuntime<E> {
    pub fn with_context_engine(context_engine: E) -> Self {
        Self {
            context_engine,
            turn_middlewares: builtin_turn_middlewares(),
        }
    }

    pub fn with_context_engine_and_turn_middlewares(
        context_engine: E,
        turn_middlewares: Vec<Box<dyn ConversationTurnMiddleware>>,
    ) -> Self {
        let mut combined_turn_middlewares = builtin_turn_middlewares();
        combined_turn_middlewares.extend(turn_middlewares);
        Self {
            context_engine,
            turn_middlewares: combined_turn_middlewares,
        }
    }
}

impl<E> DefaultConversationRuntime<E>
where
    E: ConversationContextEngine,
{
    async fn build_context_for_tool_view(
        &self,
        config: &LoongConfig,
        session_context: &SessionContext,
        include_system_prompt: bool,
        requested_tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        let effective_config_storage;
        let effective_config = match session_context.workspace_root.as_ref() {
            Some(workspace_root) => {
                let mut overridden_config = config.clone();
                overridden_config.tools.file_root = Some(workspace_root.display().to_string());
                effective_config_storage = overridden_config;
                &effective_config_storage
            }
            None => config,
        };
        let runtime_tool_view = crate::tools::runtime_tool_view_from_loong_config(effective_config);
        let mut assembled = self
            .context_engine
            .assemble_context(
                effective_config,
                session_context.session_id.as_str(),
                include_system_prompt,
                binding,
            )
            .await?;
        let runtime_self_continuity = include_system_prompt
            .then(|| runtime_self_continuity_prompt_summary(effective_config, session_context))
            .flatten();
        #[cfg(feature = "memory-sqlite")]
        let active_external_skills = include_system_prompt
            .then(|| {
                active_external_skills_prompt_summary(
                    effective_config,
                    session_context.session_id.as_str(),
                )
            })
            .flatten();
        #[cfg(not(feature = "memory-sqlite"))]
        let active_external_skills: Option<String> = None;
        let delegate_runtime_contract = include_system_prompt
            .then(|| {
                delegate_child_runtime_contract_prompt_summary(effective_config, session_context)
            })
            .flatten();
        let delegate_profile_contract = include_system_prompt
            .then(|| delegate_child_profile_prompt_summary(session_context))
            .flatten();

        seed_prompt_fragments_from_context(&mut assembled);
        append_runtime_prompt_fragment(
            &mut assembled,
            "runtime-self-continuity",
            runtime_self_continuity,
            PromptFrameAuthority::RuntimeSelf,
        );
        append_runtime_prompt_fragment(
            &mut assembled,
            "active-external-skills",
            active_external_skills,
            PromptFrameAuthority::SessionLocalRecall,
        );
        append_runtime_prompt_fragment(
            &mut assembled,
            "delegate-child-profile",
            delegate_profile_contract,
            PromptFrameAuthority::AdvisoryProfile,
        );
        append_runtime_prompt_fragment(
            &mut assembled,
            "delegate-child-runtime-contract",
            delegate_runtime_contract,
            PromptFrameAuthority::CapabilityContract,
        );
        sync_prompt_fragments_into_context(&mut assembled);

        self.apply_turn_middlewares_to_context(
            effective_config,
            session_context.session_id.as_str(),
            include_system_prompt,
            assembled,
            &runtime_tool_view,
            requested_tool_view,
            binding,
        )
        .await
    }

    pub fn context_engine_metadata(&self) -> ContextEngineMetadata {
        self.context_engine.metadata()
    }

    pub fn turn_middleware_metadata(&self) -> Vec<TurnMiddlewareMetadata> {
        self.turn_middlewares
            .iter()
            .map(|middleware| middleware.metadata())
            .collect()
    }

    async fn run_turn_middlewares_bootstrap(
        &self,
        config: &LoongConfig,
        session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware.bootstrap(config, session_id, kernel_ctx).await?;
        }
        Ok(())
    }

    async fn run_turn_middlewares_ingest(
        &self,
        session_id: &str,
        message: &Value,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware.ingest(session_id, message, kernel_ctx).await?;
        }
        Ok(())
    }

    async fn apply_turn_middlewares_to_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        mut assembled: AssembledConversationContext,
        runtime_tool_view: &ToolView,
        requested_tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        for middleware in &self.turn_middlewares {
            assembled = middleware
                .transform_context(
                    config,
                    session_id,
                    include_system_prompt,
                    assembled,
                    runtime_tool_view,
                    requested_tool_view,
                    binding,
                )
                .await?;
        }
        Ok(assembled)
    }

    async fn run_turn_middlewares_after_turn(
        &self,
        session_id: &str,
        user_input: &str,
        assistant_reply: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware
                .after_turn(
                    session_id,
                    user_input,
                    assistant_reply,
                    messages,
                    kernel_ctx,
                )
                .await?;
        }
        Ok(())
    }

    async fn run_turn_middlewares_compact_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware
                .compact_context(config, session_id, messages, kernel_ctx)
                .await?;
        }
        Ok(())
    }

    async fn run_turn_middlewares_prepare_subagent_spawn(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware
                .prepare_subagent_spawn(parent_session_id, subagent_session_id, kernel_ctx)
                .await?;
        }
        Ok(())
    }

    async fn run_turn_middlewares_on_subagent_ended(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        for middleware in &self.turn_middlewares {
            middleware
                .on_subagent_ended(parent_session_id, subagent_session_id, kernel_ctx)
                .await?;
        }
        Ok(())
    }
}

impl DefaultConversationRuntime<Box<dyn ConversationContextEngine>> {
    pub fn from_engine_id(engine_id: Option<&str>) -> CliResult<Self> {
        let context_engine = resolve_context_engine(engine_id)?;
        Ok(Self::with_context_engine(context_engine))
    }

    pub fn from_config_or_env(config: &LoongConfig) -> CliResult<Self> {
        let selection = resolve_context_engine_selection(config);
        let turn_middleware_selection = resolve_turn_middleware_selection(config)?;
        let context_engine = resolve_context_engine(Some(selection.id.as_str()))?;
        let turn_middlewares = resolve_turn_middlewares(turn_middleware_selection.ids.as_slice())?;
        Ok(Self {
            context_engine,
            turn_middlewares,
        })
    }
}

pub fn load_default_conversation_runtime(
    config: &LoongConfig,
) -> CliResult<BoxedDefaultConversationRuntime> {
    BoxedDefaultConversationRuntime::from_config_or_env(config)
}

#[cfg(feature = "memory-sqlite")]
pub fn load_hosted_default_conversation_runtime(
    config: &LoongConfig,
) -> CliResult<HostedConversationRuntime<BoxedDefaultConversationRuntime>> {
    let inner_runtime = load_default_conversation_runtime(config)?;
    let memory_config =
        store::session_store_config_from_memory_config_without_env_overrides(&config.memory);
    let runtime = HostedConversationRuntime::new_with_memory_config(inner_runtime, memory_config);
    Ok(runtime)
}

#[async_trait]
pub trait ConversationRuntime: Send + Sync {
    fn session_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<SessionContext> {
        let tool_view = self.tool_view(config, session_id, binding)?;

        #[cfg(feature = "memory-sqlite")]
        if let Some(session_context) =
            load_persisted_session_context(config, session_id, &tool_view)?
        {
            return Ok(session_context);
        }

        Ok(root_session_context_from_config(
            config, session_id, tool_view,
        ))
    }

    fn tool_view(
        &self,
        config: &LoongConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ToolView> {
        let _ = (session_id, binding);
        Ok(crate::tools::runtime_tool_view_from_loong_config(config))
    }

    #[cfg(feature = "memory-sqlite")]
    fn async_delegate_spawner(
        &self,
        config: &LoongConfig,
    ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
        Some(Arc::new(DefaultAsyncDelegateSpawner::new(config)))
    }

    #[cfg(feature = "memory-sqlite")]
    fn background_task_spawner(
        &self,
        _config: &LoongConfig,
    ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
        None
    }

    async fn bootstrap(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineBootstrapResult> {
        Ok(ContextEngineBootstrapResult::default())
    }

    async fn ingest(
        &self,
        _session_id: &str,
        _message: &Value,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineIngestResult> {
        Ok(ContextEngineIngestResult::default())
    }

    async fn build_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        let session_context = self.session_context(config, session_id, binding)?;
        self.build_messages(
            config,
            session_id,
            include_system_prompt,
            &session_context.tool_view,
            binding,
        )
        .await
        .map(AssembledConversationContext::from_messages)
    }
    async fn build_messages(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>>;

    async fn request_completion(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String>;

    async fn request_completion_with_retry_progress(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
        _retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<String> {
        self.request_completion(config, messages, binding).await
    }

    async fn request_turn(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn>;

    async fn request_turn_with_retry_progress(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        _retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ProviderTurn> {
        self.request_turn(config, session_id, turn_id, messages, tool_view, binding)
            .await
    }

    async fn request_turn_streaming(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn>;

    async fn request_turn_streaming_with_retry_progress(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        on_token: crate::provider::StreamingTokenCallback,
        _retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ProviderTurn> {
        self.request_turn_streaming(
            config, session_id, turn_id, messages, tool_view, binding, on_token,
        )
        .await
    }

    async fn persist_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<()>;

    async fn after_turn(
        &self,
        _session_id: &str,
        _user_input: &str,
        _assistant_reply: &str,
        _messages: &[Value],
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn compact_context(
        &self,
        _config: &LoongConfig,
        _session_id: &str,
        _messages: &[Value],
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn prepare_subagent_spawn(
        &self,
        _parent_session_id: &str,
        _subagent_session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }

    async fn on_subagent_ended(
        &self,
        _parent_session_id: &str,
        _subagent_session_id: &str,
        _kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        Ok(())
    }
}

#[cfg(feature = "memory-sqlite")]
#[async_trait]
impl<R> ConversationRuntime for HostedConversationRuntime<R>
where
    R: ConversationRuntime,
{
    fn session_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<SessionContext> {
        self.inner.session_context(config, session_id, binding)
    }

    fn tool_view(
        &self,
        config: &LoongConfig,
        session_id: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ToolView> {
        self.inner.tool_view(config, session_id, binding)
    }

    fn async_delegate_spawner(
        &self,
        config: &LoongConfig,
    ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
        let override_spawner = self.async_delegate_spawner_override.clone();
        match override_spawner {
            Some(override_spawner) => Some(override_spawner),
            None => self.inner.async_delegate_spawner(config),
        }
    }

    fn background_task_spawner(
        &self,
        config: &LoongConfig,
    ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
        let override_spawner = self.background_task_spawner_override.clone();
        match override_spawner {
            Some(override_spawner) => Some(override_spawner),
            None => self.inner.background_task_spawner(config),
        }
    }

    async fn bootstrap(
        &self,
        config: &LoongConfig,
        session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineBootstrapResult> {
        self.inner.bootstrap(config, session_id, kernel_ctx).await
    }

    async fn ingest(
        &self,
        session_id: &str,
        message: &Value,
        kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineIngestResult> {
        self.inner.ingest(session_id, message, kernel_ctx).await
    }

    async fn build_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        self.inner
            .build_context(config, session_id, include_system_prompt, binding)
            .await
    }

    async fn build_messages(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        self.inner
            .build_messages(
                config,
                session_id,
                include_system_prompt,
                tool_view,
                binding,
            )
            .await
    }

    async fn request_completion(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        self.inner
            .request_completion(config, messages, binding)
            .await
    }

    async fn request_turn(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn> {
        self.inner
            .request_turn(config, session_id, turn_id, messages, tool_view, binding)
            .await
    }

    async fn request_turn_streaming(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn> {
        self.inner
            .request_turn_streaming(
                config, session_id, turn_id, messages, tool_view, binding, on_token,
            )
            .await
    }

    async fn persist_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<()> {
        if binding.kernel_context().is_some() {
            return self
                .inner
                .persist_turn(session_id, role, content, binding)
                .await;
        }

        store::append_session_turn_direct(session_id, role, content, &self.memory_config)
            .map_err(|error| format!("persist {role} turn failed: {error}"))?;

        Ok(())
    }

    async fn after_turn(
        &self,
        session_id: &str,
        user_input: &str,
        assistant_reply: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.inner
            .after_turn(
                session_id,
                user_input,
                assistant_reply,
                messages,
                kernel_ctx,
            )
            .await
    }

    async fn compact_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.inner
            .compact_context(config, session_id, messages, kernel_ctx)
            .await
    }

    async fn prepare_subagent_spawn(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.inner
            .prepare_subagent_spawn(parent_session_id, subagent_session_id, kernel_ctx)
            .await
    }

    async fn on_subagent_ended(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.inner
            .on_subagent_ended(parent_session_id, subagent_session_id, kernel_ctx)
            .await
    }
}

#[async_trait]
impl<E> ConversationRuntime for DefaultConversationRuntime<E>
where
    E: ConversationContextEngine,
{
    fn session_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<SessionContext> {
        #[cfg(feature = "memory-sqlite")]
        {
            let repo = open_session_repository(config)?;
            let snapshot = load_persisted_session_snapshot(&repo, session_id)?;
            let base_tool_view =
                build_base_tool_view_from_snapshot(config, &repo, session_id, snapshot.as_ref())?;

            if let Some(snapshot) = snapshot {
                return build_session_context_from_snapshot(
                    config,
                    &repo,
                    session_id,
                    base_tool_view,
                    snapshot,
                );
            }

            Ok(root_session_context_from_config(
                config,
                session_id,
                base_tool_view,
            ))
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let tool_view = self.tool_view(config, session_id, _binding)?;
            Ok(root_session_context_from_config(
                config, session_id, tool_view,
            ))
        }
    }

    fn tool_view(
        &self,
        config: &LoongConfig,
        session_id: &str,
        _binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ToolView> {
        #[cfg(feature = "memory-sqlite")]
        {
            let repo = open_session_repository(config)?;
            let snapshot = load_persisted_session_snapshot(&repo, session_id)?;
            let base_tool_view =
                build_base_tool_view_from_snapshot(config, &repo, session_id, snapshot.as_ref())?;
            let tool_view = apply_session_tool_policy_to_tool_view(
                base_tool_view,
                snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.session_tool_policy.as_ref()),
            );
            Ok(apply_active_external_skill_blocked_tools_to_tool_view(
                tool_view,
                snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.active_external_skills.as_ref()),
            ))
        }

        #[cfg(not(feature = "memory-sqlite"))]
        Ok(crate::tools::runtime_tool_view_from_loong_config(config))
    }

    async fn bootstrap(
        &self,
        config: &LoongConfig,
        session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineBootstrapResult> {
        let result = self
            .context_engine
            .bootstrap(config, session_id, kernel_ctx)
            .await?;
        self.run_turn_middlewares_bootstrap(config, session_id, kernel_ctx)
            .await?;
        Ok(result)
    }

    async fn ingest(
        &self,
        session_id: &str,
        message: &Value,
        kernel_ctx: &KernelContext,
    ) -> CliResult<ContextEngineIngestResult> {
        let result = self
            .context_engine
            .ingest(session_id, message, kernel_ctx)
            .await?;
        self.run_turn_middlewares_ingest(session_id, message, kernel_ctx)
            .await?;
        Ok(result)
    }

    async fn build_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<AssembledConversationContext> {
        let session_context = self.session_context(config, session_id, binding)?;
        self.build_context_for_tool_view(
            config,
            &session_context,
            include_system_prompt,
            &session_context.tool_view,
            binding,
        )
        .await
    }

    async fn build_messages(
        &self,
        config: &LoongConfig,
        session_id: &str,
        include_system_prompt: bool,
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<Vec<Value>> {
        let session_context = self.session_context(config, session_id, binding)?;
        self.build_context_for_tool_view(
            config,
            &session_context,
            include_system_prompt,
            tool_view,
            binding,
        )
        .await
        .map(|assembled| assembled.messages)
    }

    async fn request_completion(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        provider::request_completion(config, messages, provider_runtime_binding(binding)).await
    }

    async fn request_completion_with_retry_progress(
        &self,
        config: &LoongConfig,
        messages: &[Value],
        binding: ConversationRuntimeBinding<'_>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<String> {
        provider::request_completion_with_retry_progress(
            config,
            messages,
            provider_runtime_binding(binding),
            retry_progress,
        )
        .await
    }

    async fn request_turn(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<ProviderTurn> {
        provider::request_turn_in_view(
            config,
            session_id,
            turn_id,
            messages,
            tool_view,
            provider_runtime_binding(binding),
        )
        .await
    }

    async fn request_turn_with_retry_progress(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ProviderTurn> {
        provider::request_turn_in_view_with_retry_progress(
            config,
            session_id,
            turn_id,
            messages,
            tool_view,
            provider_runtime_binding(binding),
            retry_progress,
        )
        .await
    }

    async fn request_turn_streaming(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        on_token: crate::provider::StreamingTokenCallback,
    ) -> CliResult<ProviderTurn> {
        provider::request_turn_streaming_in_view(
            config,
            session_id,
            turn_id,
            messages,
            tool_view,
            provider_runtime_binding(binding),
            on_token,
        )
        .await
    }

    async fn request_turn_streaming_with_retry_progress(
        &self,
        config: &LoongConfig,
        session_id: &str,
        turn_id: &str,
        messages: &[Value],
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
        on_token: crate::provider::StreamingTokenCallback,
        retry_progress: crate::provider::ProviderRetryProgressCallback,
    ) -> CliResult<ProviderTurn> {
        provider::request_turn_streaming_in_view_with_retry_progress(
            config,
            session_id,
            turn_id,
            messages,
            tool_view,
            provider_runtime_binding(binding),
            on_token,
            retry_progress,
        )
        .await
    }

    async fn persist_turn(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<()> {
        if let Some(ctx) = binding.kernel_context() {
            let request = memory::build_append_turn_request(session_id, role, content);
            let caps = BTreeSet::from([Capability::MemoryWrite]);
            ctx.kernel
                .execute_memory_core(ctx.pack_id(), &ctx.token, &caps, None, request)
                .await
                .map_err(|error| format!("persist {role} turn via kernel failed: {error}"))?;
            return Ok(());
        }

        #[cfg(feature = "memory-sqlite")]
        {
            store::append_session_turn_direct(
                session_id,
                role,
                content,
                store::current_session_store_config(),
            )
            .map_err(|error| format!("persist {role} turn failed: {error}"))?;
        }

        #[cfg(not(feature = "memory-sqlite"))]
        {
            let _ = (session_id, role, content);
        }

        Ok(())
    }

    async fn after_turn(
        &self,
        session_id: &str,
        user_input: &str,
        assistant_reply: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.context_engine
            .after_turn(
                session_id,
                user_input,
                assistant_reply,
                messages,
                kernel_ctx,
            )
            .await?;
        self.run_turn_middlewares_after_turn(
            session_id,
            user_input,
            assistant_reply,
            messages,
            kernel_ctx,
        )
        .await
    }

    async fn compact_context(
        &self,
        config: &LoongConfig,
        session_id: &str,
        messages: &[Value],
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.context_engine
            .compact_context(config, session_id, messages, kernel_ctx)
            .await?;
        self.run_turn_middlewares_compact_context(config, session_id, messages, kernel_ctx)
            .await
    }

    async fn prepare_subagent_spawn(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.context_engine
            .prepare_subagent_spawn(parent_session_id, subagent_session_id, kernel_ctx)
            .await?;
        self.run_turn_middlewares_prepare_subagent_spawn(
            parent_session_id,
            subagent_session_id,
            kernel_ctx,
        )
        .await
    }

    async fn on_subagent_ended(
        &self,
        parent_session_id: &str,
        subagent_session_id: &str,
        kernel_ctx: &KernelContext,
    ) -> CliResult<()> {
        self.context_engine
            .on_subagent_ended(parent_session_id, subagent_session_id, kernel_ctx)
            .await?;
        self.run_turn_middlewares_on_subagent_ended(
            parent_session_id,
            subagent_session_id,
            kernel_ctx,
        )
        .await
    }
}

fn provider_runtime_binding(
    binding: ConversationRuntimeBinding<'_>,
) -> provider::ProviderRuntimeBinding<'_> {
    match binding {
        ConversationRuntimeBinding::Kernel(kernel_ctx) => {
            provider::ProviderRuntimeBinding::kernel(kernel_ctx)
        }
        ConversationRuntimeBinding::Direct => provider::ProviderRuntimeBinding::advisory_only(),
    }
}

fn delegate_child_runtime_contract_prompt_summary(
    config: &LoongConfig,
    session_context: &SessionContext,
) -> Option<String> {
    session_context.parent_session_id.as_ref()?;
    session_context.subagent_runtime_narrowing()?;
    let subagent_contract = session_context.resolved_subagent_contract();
    crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None)
        .delegate_child_prompt_summary(subagent_contract.as_ref())
}

fn delegate_child_profile_prompt_summary(session_context: &SessionContext) -> Option<String> {
    let _parent_session_id = session_context.parent_session_id.as_ref()?;
    let profile = session_context.profile?;
    let summary = match profile {
        DelegateBuiltinProfile::Research => concat!(
            "[delegate_child_profile]\n",
            "You are running with the `research` delegate profile.\n",
            "- Gather evidence before conclusions.\n",
            "- Prefer reading files, web sources, and browser extraction over proposing edits.\n",
            "- Return concise findings, concrete references, and unresolved risks."
        ),
        DelegateBuiltinProfile::Plan => concat!(
            "[delegate_child_profile]\n",
            "You are running with the `plan` delegate profile.\n",
            "- Turn findings into an execution plan.\n",
            "- Prefer ordered steps, explicit assumptions, and acceptance criteria.\n",
            "- Do not claim implementation is complete when you only have a proposal."
        ),
        DelegateBuiltinProfile::Verify => concat!(
            "[delegate_child_profile]\n",
            "You are running with the `verify` delegate profile.\n",
            "- Try to falsify success claims before accepting them.\n",
            "- Prefer concrete checks, observed failures, and residual risk notes.\n",
            "- Report a clear verdict with evidence."
        ),
    };
    let rendered = summary.to_owned();
    Some(rendered)
}

fn runtime_self_continuity_prompt_summary(
    config: &LoongConfig,
    session_context: &SessionContext,
) -> Option<String> {
    let stored_continuity = session_context.runtime_self_continuity.as_ref()?;
    let live_continuity =
        runtime_self_continuity::resolve_runtime_self_continuity_for_config(config);
    let missing_continuity = runtime_self_continuity::missing_runtime_self_continuity(
        stored_continuity,
        live_continuity.as_ref(),
    )?;
    let inherited = session_context.parent_session_id.is_some();
    runtime_self_continuity::render_runtime_self_continuity_section(&missing_continuity, inherited)
}

#[cfg(feature = "memory-sqlite")]
fn active_external_skills_prompt_summary(config: &LoongConfig, session_id: &str) -> Option<String> {
    let repo = open_session_repository(config).ok()?;
    let active_skills =
        active_external_skills::load_persisted_active_external_skills(&repo, session_id)
            .ok()
            .flatten()?;
    active_external_skills::render_active_external_skills_section(&active_skills)
}

fn append_runtime_prompt_fragment(
    assembled: &mut AssembledConversationContext,
    source_id: &'static str,
    content: Option<String>,
    frame_authority: PromptFrameAuthority,
) {
    let Some(content) = content else {
        return;
    };

    let fragment = PromptFragment::new(
        source_id,
        PromptLane::Continuity,
        source_id,
        content,
        ContextArtifactKind::RuntimeContract,
    )
    .with_dedupe_key(source_id)
    .with_cacheable(true)
    .with_frame_authority(frame_authority);

    assembled.prompt_fragments.push(fragment);
}

fn normalize_turn_middleware_ids(ids: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for id in ids {
        if seen.insert(id.clone()) {
            normalized.push(id);
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "memory-sqlite")]
    use crate::conversation::active_external_skills::{
        ACTIVE_EXTERNAL_SKILLS_EVENT_KIND, ActiveExternalSkill, ActiveExternalSkillsState,
    };
    #[cfg(feature = "memory-sqlite")]
    use crate::session::repository::{
        NewSessionEvent, NewSessionRecord, SessionKind, SessionRepository, SessionState,
    };
    use crate::test_support::TurnTestHarness;
    use crate::test_support::unique_temp_dir;
    #[cfg(feature = "memory-sqlite")]
    use serde_json::json;
    #[cfg(feature = "memory-sqlite")]
    use std::sync::Arc;

    #[cfg(feature = "memory-sqlite")]
    #[derive(Clone)]
    struct NoopTestSpawner;

    #[cfg(feature = "memory-sqlite")]
    #[async_trait]
    impl AsyncDelegateSpawner for NoopTestSpawner {
        async fn spawn(&self, _request: AsyncDelegateSpawnRequest) -> Result<(), String> {
            Ok(())
        }
    }

    #[cfg(feature = "memory-sqlite")]
    struct SpawnerAwareRuntime {
        async_delegate_spawner: Option<Arc<dyn AsyncDelegateSpawner>>,
        background_task_spawner: Option<Arc<dyn AsyncDelegateSpawner>>,
    }

    #[cfg(feature = "memory-sqlite")]
    #[async_trait]
    impl ConversationRuntime for SpawnerAwareRuntime {
        fn tool_view(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ToolView> {
            Ok(crate::tools::runtime_tool_view())
        }

        fn async_delegate_spawner(
            &self,
            _config: &LoongConfig,
        ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
            self.async_delegate_spawner.clone()
        }

        fn background_task_spawner(
            &self,
            _config: &LoongConfig,
        ) -> Option<Arc<dyn AsyncDelegateSpawner>> {
            self.background_task_spawner.clone()
        }

        async fn build_messages(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _include_system_prompt: bool,
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<Vec<Value>> {
            Ok(Vec::new())
        }

        async fn request_completion(
            &self,
            _config: &LoongConfig,
            _messages: &[Value],
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<String> {
            Ok(String::new())
        }

        async fn request_turn(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ProviderTurn> {
            Ok(ProviderTurn::default())
        }

        async fn request_turn_streaming(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
            _on_token: crate::provider::StreamingTokenCallback,
        ) -> CliResult<ProviderTurn> {
            Ok(ProviderTurn::default())
        }

        async fn persist_turn(
            &self,
            _session_id: &str,
            _role: &str,
            _content: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    #[test]
    fn provider_runtime_binding_maps_direct_conversation_binding_to_advisory_only() {
        assert!(matches!(
            provider_runtime_binding(ConversationRuntimeBinding::direct()),
            provider::ProviderRuntimeBinding::AdvisoryOnly
        ));
    }

    #[test]
    fn provider_runtime_binding_maps_kernel_conversation_binding_to_kernel() {
        let harness = TurnTestHarness::new();

        assert!(matches!(
            provider_runtime_binding(ConversationRuntimeBinding::kernel(&harness.kernel_ctx)),
            provider::ProviderRuntimeBinding::Kernel(kernel_ctx)
                if std::ptr::eq(kernel_ctx, &harness.kernel_ctx)
        ));
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hosted_runtime_overrides_background_task_spawner_without_changing_async_delegate_spawner() {
        let config = LoongConfig::default();
        let inner_async_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
        let inner_background_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
        let override_background_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
        let inner_runtime = SpawnerAwareRuntime {
            async_delegate_spawner: Some(inner_async_spawner.clone()),
            background_task_spawner: Some(inner_background_spawner),
        };

        let hosted_runtime = HostedConversationRuntime::new(inner_runtime)
            .with_background_task_spawner(override_background_spawner.clone());

        let resolved_async_spawner = hosted_runtime
            .async_delegate_spawner(&config)
            .expect("async delegate spawner");
        let resolved_background_spawner = hosted_runtime
            .background_task_spawner(&config)
            .expect("background task spawner");

        assert!(Arc::ptr_eq(&resolved_async_spawner, &inner_async_spawner));
        assert!(Arc::ptr_eq(
            &resolved_background_spawner,
            &override_background_spawner
        ));
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn hosted_runtime_overrides_async_delegate_spawner_without_changing_background_task_spawner() {
        let config = LoongConfig::default();
        let inner_async_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
        let inner_background_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
        let override_async_spawner: Arc<dyn AsyncDelegateSpawner> = Arc::new(NoopTestSpawner);
        let inner_runtime = SpawnerAwareRuntime {
            async_delegate_spawner: Some(inner_async_spawner),
            background_task_spawner: Some(inner_background_spawner.clone()),
        };

        let hosted_runtime = HostedConversationRuntime::new(inner_runtime)
            .with_async_delegate_spawner(override_async_spawner.clone());

        let resolved_async_spawner = hosted_runtime
            .async_delegate_spawner(&config)
            .expect("async delegate spawner");
        let resolved_background_spawner = hosted_runtime
            .background_task_spawner(&config)
            .expect("background task spawner");

        assert!(Arc::ptr_eq(
            &resolved_async_spawner,
            &override_async_spawner
        ));
        assert!(Arc::ptr_eq(
            &resolved_background_spawner,
            &inner_background_spawner
        ));
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn hosted_runtime_build_context_delegates_to_inner_runtime() {
        #[derive(Clone)]
        struct BuildContextAwareRuntime;

        #[async_trait]
        impl ConversationRuntime for BuildContextAwareRuntime {
            fn tool_view(
                &self,
                _config: &LoongConfig,
                _session_id: &str,
                _binding: ConversationRuntimeBinding<'_>,
            ) -> CliResult<ToolView> {
                Ok(crate::tools::runtime_tool_view())
            }

            async fn build_context(
                &self,
                _config: &LoongConfig,
                _session_id: &str,
                _include_system_prompt: bool,
                _binding: ConversationRuntimeBinding<'_>,
            ) -> CliResult<AssembledConversationContext> {
                let messages = vec![serde_json::json!({
                    "role": "system",
                    "content": "delegated"
                })];
                let prompt_fragment = PromptFragment::new(
                    "fragment",
                    PromptLane::RuntimeSelf,
                    "runtime-self",
                    "delegated fragment",
                    ContextArtifactKind::RuntimeContract,
                );
                let assembled = AssembledConversationContext {
                    messages,
                    artifacts: Vec::new(),
                    estimated_tokens: Some(7),
                    prompt_fragments: vec![prompt_fragment],
                    system_prompt_addition: Some("addition".to_owned()),
                };

                Ok(assembled)
            }

            async fn build_messages(
                &self,
                _config: &LoongConfig,
                _session_id: &str,
                _include_system_prompt: bool,
                _tool_view: &ToolView,
                _binding: ConversationRuntimeBinding<'_>,
            ) -> CliResult<Vec<Value>> {
                Err("build_messages should not be used when build_context is delegated".to_owned())
            }

            async fn request_completion(
                &self,
                _config: &LoongConfig,
                _messages: &[Value],
                _binding: ConversationRuntimeBinding<'_>,
            ) -> CliResult<String> {
                Ok(String::new())
            }

            async fn request_turn(
                &self,
                _config: &LoongConfig,
                _session_id: &str,
                _turn_id: &str,
                _messages: &[Value],
                _tool_view: &ToolView,
                _binding: ConversationRuntimeBinding<'_>,
            ) -> CliResult<ProviderTurn> {
                Err("unused".to_owned())
            }

            async fn request_turn_streaming(
                &self,
                _config: &LoongConfig,
                _session_id: &str,
                _turn_id: &str,
                _messages: &[Value],
                _tool_view: &ToolView,
                _binding: ConversationRuntimeBinding<'_>,
                _on_token: crate::provider::StreamingTokenCallback,
            ) -> CliResult<ProviderTurn> {
                Err("unused".to_owned())
            }

            async fn persist_turn(
                &self,
                _session_id: &str,
                _role: &str,
                _content: &str,
                _binding: ConversationRuntimeBinding<'_>,
            ) -> CliResult<()> {
                Ok(())
            }
        }

        let config = LoongConfig::default();
        let hosted_runtime = HostedConversationRuntime::new(BuildContextAwareRuntime);

        let assembled = hosted_runtime
            .build_context(
                &config,
                "session-1",
                true,
                ConversationRuntimeBinding::Direct,
            )
            .await
            .expect("delegated build_context");

        assert_eq!(assembled.messages.len(), 1);
        assert_eq!(assembled.estimated_tokens, Some(7));
        assert_eq!(assembled.prompt_fragments.len(), 1);
        assert_eq!(
            assembled.system_prompt_addition.as_deref(),
            Some("addition")
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn load_hosted_default_conversation_runtime_keeps_default_async_spawner_only() {
        let config = LoongConfig::default();
        let runtime = load_hosted_default_conversation_runtime(&config)
            .expect("load hosted default conversation runtime");

        let async_delegate_spawner = runtime.async_delegate_spawner(&config);
        let background_task_spawner = runtime.background_task_spawner(&config);

        assert!(async_delegate_spawner.is_some());
        assert!(background_task_spawner.is_none());
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn hosted_runtime_persist_turn_uses_explicit_memory_config_for_direct_binding() {
        let root = unique_temp_dir("hosted-runtime-explicit-memory");
        let sqlite_path = root.join("explicit-memory.db");
        let memory_config =
            crate::session::store::SessionStoreConfig::for_sqlite_path(sqlite_path.clone());
        let session_id = "hosted-runtime-explicit-session";
        let runtime = HostedConversationRuntime::new_with_memory_config(
            SpawnerAwareRuntime {
                async_delegate_spawner: None,
                background_task_spawner: None,
            },
            memory_config.clone(),
        );

        crate::session::store::ensure_session_store_ready(Some(sqlite_path), &memory_config)
            .expect("initialize explicit session store");

        runtime
            .persist_turn(
                session_id,
                "assistant",
                "persist via explicit memory config",
                ConversationRuntimeBinding::Direct,
            )
            .await
            .expect("persist hosted runtime turn");

        let turns = crate::session::store::window_session_turns(session_id, 8, &memory_config)
            .expect("load persisted turns");
        let persisted_turn = turns
            .iter()
            .find(|turn| {
                turn.role == "assistant" && turn.content == "persist via explicit memory config"
            })
            .expect("persisted assistant turn");

        assert_eq!(persisted_turn.role, "assistant");
        assert_eq!(persisted_turn.content, "persist via explicit memory config");
    }

    #[tokio::test]
    async fn default_runtime_build_context_rehydrates_active_external_skills() {
        let runtime = DefaultConversationRuntime::default();
        let session_id = "session-active-external-skills";
        let root = unique_temp_dir("active-external-skills-runtime");
        let sqlite_path = root.join("memory.db");
        let workspace_root = root.join("workspace");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");

        let mut config = LoongConfig::default();
        config.memory.sqlite_path = sqlite_path.display().to_string();
        config.tools.file_root = Some(workspace_root.display().to_string());

        let memory_config =
            crate::session::store::session_store_config_from_memory_config(&config.memory);
        let repo = SessionRepository::new(&memory_config).expect("session repository");
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");
        repo.append_event(NewSessionEvent {
            session_id: session_id.to_owned(),
            event_kind: ACTIVE_EXTERNAL_SKILLS_EVENT_KIND.to_owned(),
            actor_session_id: Some(session_id.to_owned()),
            payload_json: json!({
                "source": "test",
                "active_external_skills": ActiveExternalSkillsState {
                    skills: vec![ActiveExternalSkill {
                        skill_id: "release-guard".to_owned(),
                        display_name: "Release Guard".to_owned(),
                        instructions: "<skill_content name=\"Release Guard\">protect releases</skill_content>".to_owned(),
                        skill_root: Some("/tmp/release-guard".to_owned()),
                        allowed_tools: vec!["shell.exec".to_owned()],
                        blocked_tools: vec!["web.fetch".to_owned()],
                    }],
                },
            }),
        })
        .expect("append active skills event");

        let assembled = runtime
            .build_context(
                &config,
                session_id,
                true,
                ConversationRuntimeBinding::direct(),
            )
            .await
            .expect("build context");
        let system_content = assembled.messages[0]["content"]
            .as_str()
            .expect("system prompt should be text");

        assert!(
            system_content.contains("[active_external_skills]"),
            "expected active external skills marker, got: {system_content}"
        );
        assert!(
            system_content.contains("release-guard"),
            "expected skill id in system prompt, got: {system_content}"
        );
        assert!(
            system_content.contains("Release Guard"),
            "expected skill display name in system prompt, got: {system_content}"
        );
        assert!(
            system_content.contains("protect releases"),
            "expected skill instructions in system prompt, got: {system_content}"
        );
        assert!(
            system_content.contains("Allowed tools: shell.exec"),
            "expected allowed tool summary in system prompt, got: {system_content}"
        );
        assert!(
            system_content.contains("Blocked tools: web.fetch"),
            "expected blocked tool summary in system prompt, got: {system_content}"
        );
    }

    #[tokio::test]
    async fn default_runtime_tool_view_excludes_active_external_skill_blocked_tools() {
        let runtime = DefaultConversationRuntime::default();
        let session_id = "session-active-external-skill-tool-block";
        let root = unique_temp_dir("active-external-skill-tool-block");
        let sqlite_path = root.join("memory.db");

        let mut config = LoongConfig::default();
        config.memory.sqlite_path = sqlite_path.display().to_string();

        let base_tool_view = crate::tools::runtime_tool_view_from_loong_config(&config);
        assert!(
            base_tool_view.contains("web"),
            "default runtime should expose the direct web surface"
        );

        let memory_config =
            crate::session::store::session_store_config_from_memory_config(&config.memory);
        let repo = SessionRepository::new(&memory_config).expect("session repository");
        repo.create_session(NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Ready,
        })
        .expect("create root session");
        repo.append_event(NewSessionEvent {
            session_id: session_id.to_owned(),
            event_kind: ACTIVE_EXTERNAL_SKILLS_EVENT_KIND.to_owned(),
            actor_session_id: Some(session_id.to_owned()),
            payload_json: json!({
                "source": "test",
                "active_external_skills": ActiveExternalSkillsState {
                    skills: vec![ActiveExternalSkill {
                        skill_id: "release-guard".to_owned(),
                        display_name: "Release Guard".to_owned(),
                        instructions: "<skill_content name=\"Release Guard\">protect releases</skill_content>".to_owned(),
                        skill_root: Some("/tmp/release-guard".to_owned()),
                        allowed_tools: Vec::new(),
                        blocked_tools: vec!["web.fetch".to_owned()],
                    }],
                },
            }),
        })
        .expect("append active skills event");

        let tool_view = runtime
            .tool_view(&config, session_id, ConversationRuntimeBinding::direct())
            .expect("runtime tool view");

        assert!(
            !tool_view.contains("web"),
            "blocked hidden tool should also remove its direct surface"
        );
        assert!(
            tool_view.contains("read"),
            "unrelated direct tools should remain visible"
        );
    }
}
