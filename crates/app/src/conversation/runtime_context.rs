use std::path::PathBuf;

use crate::runtime_self_continuity::RuntimeSelfContinuity;
use crate::tools::ToolView;
use crate::tools::runtime_config::ToolRuntimeNarrowing;

use super::super::subagent::{
    ConstrainedSubagentContractView, ConstrainedSubagentExecution, ConstrainedSubagentIdentity,
    ConstrainedSubagentProfile, DelegateBuiltinProfile,
};
use super::LoongConfig;
use super::mailbox_for_session;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContext {
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub profile: Option<DelegateBuiltinProfile>,
    pub tool_view: ToolView,
    pub workspace_root: Option<PathBuf>,
    pub active_skill_roots: Vec<PathBuf>,
    pub visible_skill_roots: Vec<PathBuf>,
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
            active_skill_roots: Vec::new(),
            visible_skill_roots: Vec::new(),
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
            active_skill_roots: Vec::new(),
            visible_skill_roots: Vec::new(),
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
    pub fn with_active_skill_roots(mut self, active_skill_roots: Vec<PathBuf>) -> Self {
        self.active_skill_roots = active_skill_roots
            .into_iter()
            .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
            .collect();
        self
    }

    #[must_use]
    pub fn with_visible_skill_roots(mut self, visible_skill_roots: Vec<PathBuf>) -> Self {
        self.visible_skill_roots = visible_skill_roots
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
            .filter(|runtime_narrowing: &ToolRuntimeNarrowing| !runtime_narrowing.is_empty());
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

pub(super) fn root_session_context_from_config(
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

pub(super) fn non_empty_runtime_narrowing_ref(
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

pub(super) fn model_visible_skill_roots_from_config(config: &LoongConfig) -> Vec<PathBuf> {
    let tool_runtime_config =
        crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
    crate::tools::model_visible_skill_roots_for_runtime_config(&tool_runtime_config)
}
