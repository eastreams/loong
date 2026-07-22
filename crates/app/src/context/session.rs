//! Owned session identity, stable authority, and lifecycle state.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use loong_contracts::{Capabilities, Capability, GovernedSessionMode};
use loong_kernel::access::fs::normalize_path_lexically;
use loong_kernel::mailbox::AgentMailbox;
use loong_runtime::runtime::{Runtime, RuntimeId};

use super::RuntimeContextFactory;
use crate::conversation::{
    ConstrainedSubagentContractView, ConstrainedSubagentExecution, ConstrainedSubagentIdentity,
    ConstrainedSubagentProfile, DelegateBuiltinProfile,
};
use crate::runtime_self_continuity::RuntimeSelfContinuity;
use crate::tools::ToolView;
use crate::tools::runtime_config::ToolRuntimeNarrowing;

mod materialize;
#[cfg(feature = "memory-sqlite")]
pub(crate) use materialize::SessionToolPolicyProjection;

/// Stable app-owned identity and authority baseline for one live session.
///
/// A session deliberately owns neither Runtime nor legacy bearer evidence.
/// Long-lived surface owners retain Runtime and Session separately, then borrow
/// both into [`super::Context`] for one structured execution scope.
#[derive(Clone)]
pub struct Session {
    pub(super) runtime_id: RuntimeId,
    pub(crate) agent_id: String,
    pub(crate) session_id: String,
    pub(crate) parent_session_id: Option<String>,
    pub(crate) profile: Option<DelegateBuiltinProfile>,
    pub(crate) tool_view: ToolView,
    pub(crate) session_mode: GovernedSessionMode,
    // Clones of the same live Session share delivery state. Derived root and
    // child Sessions replace it so identity strings never become global keys.
    mailbox: AgentMailbox,
    /// Session-local working directory used by prompt and workspace UX.
    /// Filesystem authorization reads only `fs_allowed_roots` below.
    pub(crate) workspace_root: Option<PathBuf>,
    pub(crate) active_skill_roots: Vec<PathBuf>,
    pub(crate) visible_skill_roots: Vec<PathBuf>,
    pub(crate) subagent_execution: Option<ConstrainedSubagentExecution>,
    /// Cumulative runtime ceiling after ancestor, delegate, and Session policy.
    ///
    /// This is distinct from the delegate-local narrowing retained inside
    /// `subagent_execution`; root policy therefore cannot masquerade as a child
    /// execution contract.
    pub(crate) effective_runtime_narrowing: Option<ToolRuntimeNarrowing>,
    pub(crate) runtime_self_continuity: Option<RuntimeSelfContinuity>,
    pub(super) baseline_capabilities: Capabilities,
    // Legacy executors still consume this materialized snapshot. Typed fs
    // policy reads only the two explicit projections below.
    pub(super) tool_runtime_config: crate::tools::runtime_config::ToolRuntimeConfig,
    pub(super) fs_resolution_root: PathBuf,
    pub(super) fs_allowed_roots: Vec<PathBuf>,
    pub(super) fs_authority_ceiling_roots: Vec<PathBuf>,
    /// Concrete memory runtime selected once for this Session authority snapshot.
    /// Recursive Contexts borrow it; Kernel never acts as a backend locator.
    pub(super) memory_backend: Arc<
        dyn loong_kernel::access::memory::MemoryBackend<
                StageEnvelope = crate::memory::StageEnvelope,
                CompactOutput = crate::memory::StageDiagnostics,
            >,
    >,
}

/// Make one root absolute against an already-owned base and normalize it lexically.
///
/// This intentionally performs no existence, kind, or symlink observation.
/// Granted fs resolve actions later place effective roots, parent ceilings, and
/// requested paths into the same canonical path space for containment policy.
fn absolute_lexical(path: PathBuf, base: &std::path::Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    normalize_path_lexically(&absolute)
}

/// Materialize the roots that fs path policy may authorize.
fn fs_allowed_roots(
    config: &crate::tools::runtime_config::ToolRuntimeConfig,
    fallback_root: &std::path::Path,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(file_root) = config.file_root.as_ref() {
        roots.push(file_root.clone());
    }
    if let Some(workspace_root) = config.workspace_root.as_ref()
        && roots.iter().all(|root| root != workspace_root)
    {
        roots.push(workspace_root.clone());
    }
    if roots.is_empty() {
        roots.push(fallback_root.to_path_buf());
    }
    roots
}

/// Materialize relative-path resolution independently from path authorization.
fn fs_resolution_root(
    config: &crate::tools::runtime_config::ToolRuntimeConfig,
    allowed_roots: &[PathBuf],
) -> Result<PathBuf, String> {
    let resolution_root = match config.path_resolution_root() {
        Some(path) => path.to_path_buf(),
        None => allowed_roots
            .first()
            .cloned()
            .ok_or_else(|| "filesystem access requires at least one allowed root".to_owned())?,
    };
    if !allowed_roots
        .iter()
        .any(|allowed_root| resolution_root.starts_with(allowed_root))
    {
        return Err(format!(
            "filesystem resolution root escapes allowed authority: {}",
            resolution_root.display()
        ));
    }
    Ok(resolution_root)
}

impl fmt::Debug for Session {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // ToolRuntimeConfig may contain resolved credentials. Session diagnostics
        // expose identity and authority shape without formatting executor config.
        formatter
            .debug_struct("Session")
            .field("agent_id", &self.agent_id)
            .field("session_id", &self.session_id)
            .field("parent_session_id", &self.parent_session_id)
            .field("profile", &self.profile)
            .field("tool_view", &self.tool_view)
            .field("session_mode", &self.session_mode)
            .field("workspace_root", &self.workspace_root)
            .field("active_skill_roots", &self.active_skill_roots)
            .field("visible_skill_roots", &self.visible_skill_roots)
            .field("subagent_execution", &self.subagent_execution)
            .field(
                "effective_runtime_narrowing",
                &self.effective_runtime_narrowing,
            )
            .field("runtime_self_continuity", &self.runtime_self_continuity)
            .field("baseline_capabilities", &self.baseline_capabilities)
            .field("fs_resolution_root", &self.fs_resolution_root)
            .field("fs_allowed_roots", &self.fs_allowed_roots)
            .field(
                "fs_authority_ceiling_roots",
                &self.fs_authority_ceiling_roots,
            )
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Construct one final root Session from host-approved authority.
    ///
    /// This is an associated constructor rather than a derivation from another
    /// Session: an existing child must never be able to erase its lineage and
    /// become a root. Only [`Session::delegate_child`] derives from a live Session.
    pub(crate) fn root(
        runtime: &Runtime<RuntimeContextFactory>,
        agent_id: impl Into<String>,
        session_id: impl Into<String>,
        session_mode: GovernedSessionMode,
        baseline_capabilities: Capabilities,
        mut tool_runtime_config: crate::tools::runtime_config::ToolRuntimeConfig,
        memory_runtime_config: crate::memory::runtime_config::MemoryRuntimeConfig,
        tool_view: ToolView,
        workspace_root: Option<PathBuf>,
        runtime_narrowing: Option<&ToolRuntimeNarrowing>,
    ) -> Result<Self, String> {
        if matches!(session_mode, GovernedSessionMode::AdvisoryOnly)
            && baseline_capabilities.iter().any(|capability| {
                matches!(
                    capability,
                    Capability::InvokeTool
                        | Capability::InvokeConnector
                        | Capability::MemoryWrite
                        | Capability::FilesystemWrite
                        | Capability::ControlWrite
                        | Capability::ControlApprovals
                        | Capability::ControlPairing
                        | Capability::ControlAcp
                )
            })
        {
            return Err(
                "advisory session cannot carry invocation or mutation authority".to_owned(),
            );
        }
        let host_base = std::env::current_dir()
            .map_err(|error| format!("failed to resolve current directory: {error}"))?;
        tool_runtime_config.file_root = tool_runtime_config
            .file_root
            .take()
            .map(|path| absolute_lexical(path, &host_base));
        tool_runtime_config.workspace_root = tool_runtime_config
            .workspace_root
            .take()
            .map(|path| absolute_lexical(path, &host_base));
        let authority_roots = fs_allowed_roots(&tool_runtime_config, &host_base);
        let workspace_root = workspace_root
            .or_else(|| {
                tool_runtime_config
                    .path_resolution_root()
                    .map(std::path::Path::to_path_buf)
            })
            .map(|path| absolute_lexical(path, &host_base));
        if let Some(workspace_root) = workspace_root.as_ref()
            && !authority_roots
                .iter()
                .any(|allowed_root| workspace_root.starts_with(allowed_root))
        {
            return Err(format!(
                "session workspace root escapes configured authority: {}",
                workspace_root.display()
            ));
        }

        if let Some(runtime_narrowing) = runtime_narrowing {
            tool_runtime_config = tool_runtime_config.narrowed(runtime_narrowing);
        }
        if let Some(workspace_root) = workspace_root.as_ref() {
            // A root may select a narrower resolution root while retaining the
            // host-granted file root for authorized absolute paths.
            tool_runtime_config.workspace_root = Some(workspace_root.clone());
        }
        let fs_allowed_roots = fs_allowed_roots(&tool_runtime_config, &host_base);
        if fs_allowed_roots.iter().any(|derived_root| {
            !authority_roots
                .iter()
                .any(|authority_root| derived_root.starts_with(authority_root))
        }) {
            return Err("root filesystem authority exceeds host configuration".to_owned());
        }
        let fs_resolution_root = fs_resolution_root(&tool_runtime_config, &fs_allowed_roots)?;
        let memory_backend = Arc::new(crate::memory::MemorySystemBackend::new(
            crate::memory::resolve_memory_system_runtime(&memory_runtime_config)?,
        ));
        let session_id = validated_session_id(session_id.into())?;
        Ok(Self {
            runtime_id: runtime.id(),
            agent_id: agent_id.into(),
            session_id,
            parent_session_id: None,
            profile: None,
            tool_view,
            session_mode,
            mailbox: AgentMailbox::new(),
            workspace_root,
            active_skill_roots: Vec::new(),
            visible_skill_roots: Vec::new(),
            subagent_execution: None,
            effective_runtime_narrowing: runtime_narrowing
                .filter(|narrowing| !narrowing.is_empty())
                .cloned(),
            runtime_self_continuity: None,
            baseline_capabilities,
            tool_runtime_config,
            fs_resolution_root,
            fs_allowed_roots,
            fs_authority_ceiling_roots: authority_roots,
            memory_backend,
        })
    }

    /// Atomically derive one delegate Session from its complete execution anchor.
    ///
    /// Capability, workspace, local contract, and cumulative runtime narrowing
    /// enter together. No caller can observe or detach a child that has not yet
    /// received the execution evidence used to constrain it.
    pub(crate) fn delegate_child(
        &self,
        session_id: impl Into<String>,
        tool_view: ToolView,
        execution: ConstrainedSubagentExecution,
        effective_runtime_narrowing: Option<&ToolRuntimeNarrowing>,
    ) -> Result<Self, String> {
        let mut execution = execution.with_resolved_profile();
        if !tool_view.is_subset(&self.tool_view) {
            return Err("child tool view exceeds its parent session".to_owned());
        }
        // Delegate authority has three explicit sources: the ordinary tool
        // allowlist, the shell bit, and the profile's nested-delegation right.
        // The derived ToolView must be covered by one of those sources.
        if tool_view.tool_names().any(|tool_name| {
            let explicitly_allowed = execution
                .child_tool_allowlist
                .iter()
                .any(|allowed| allowed == tool_name);
            let shell_allowed = tool_name == "shell.exec" && execution.allow_shell_in_child;
            let delegation_allowed = matches!(tool_name, "delegate" | "delegate_async")
                && execution.allows_nested_delegate_children();
            !explicitly_allowed && !shell_allowed && !delegation_allowed
        }) {
            return Err("child tool view exceeds its execution authority".to_owned());
        }
        if !execution
            .capability_ceiling
            .is_subset(&self.baseline_capabilities)
        {
            return Err("child capabilities exceed its parent session".to_owned());
        }
        let capability_ceiling = execution.capability_ceiling.clone();
        let session_id = validated_session_id(session_id.into())?;
        let parent_session_id = self.session_id.clone();
        let workspace_root = execution
            .workspace_root
            .take()
            .map(|path| absolute_lexical(path, &self.fs_resolution_root));
        execution.workspace_root = workspace_root.clone();
        if let Some(workspace_root) = workspace_root.as_ref()
            && !self
                .fs_allowed_roots
                .iter()
                .any(|allowed_root| workspace_root.starts_with(allowed_root))
        {
            return Err(format!(
                "child workspace root escapes parent session authority: {}",
                workspace_root.display()
            ));
        }

        let required_runtime_narrowing =
            crate::tools::runtime_config::merge_runtime_narrowing_sources(
                self.effective_runtime_narrowing.clone(),
                Some(execution.runtime_narrowing.clone()),
            );
        let requested_runtime_narrowing = effective_runtime_narrowing
            .filter(|narrowing| !narrowing.is_empty())
            .cloned();
        if let (Some(requested), Some(required)) = (
            requested_runtime_narrowing.as_ref(),
            required_runtime_narrowing.as_ref(),
        ) && !requested.is_no_wider_than(required)
        {
            return Err(
                "child effective runtime narrowing expands inherited or delegate authority"
                    .to_owned(),
            );
        }
        let effective_runtime_narrowing =
            requested_runtime_narrowing.or(required_runtime_narrowing);
        let mut tool_runtime_config = effective_runtime_narrowing.as_ref().map_or_else(
            || self.tool_runtime_config.clone(),
            |narrowing| self.tool_runtime_config.narrowed(narrowing),
        );
        if let Some(workspace_root) = workspace_root.as_ref() {
            // A delegated workspace replaces both fs roots. Retaining the
            // parent's file root here would silently preserve parent access.
            tool_runtime_config.file_root = Some(workspace_root.clone());
            tool_runtime_config.workspace_root = Some(workspace_root.clone());
        }
        let fs_allowed_roots = fs_allowed_roots(&tool_runtime_config, &self.fs_resolution_root);
        if fs_allowed_roots.iter().any(|derived_root| {
            !self
                .fs_allowed_roots
                .iter()
                .any(|allowed_root| derived_root.starts_with(allowed_root))
        }) {
            return Err("child filesystem authority exceeds its parent session".to_owned());
        }
        let fs_resolution_root = fs_resolution_root(&tool_runtime_config, &fs_allowed_roots)?;

        Ok(Self {
            runtime_id: self.runtime_id,
            agent_id: self.agent_id.clone(),
            session_id,
            parent_session_id: Some(parent_session_id),
            profile: None,
            tool_view,
            session_mode: self.session_mode,
            mailbox: AgentMailbox::new(),
            workspace_root,
            active_skill_roots: Vec::new(),
            visible_skill_roots: Vec::new(),
            subagent_execution: Some(execution),
            effective_runtime_narrowing,
            runtime_self_continuity: None,
            baseline_capabilities: capability_ceiling,
            tool_runtime_config,
            fs_resolution_root,
            fs_allowed_roots,
            fs_authority_ceiling_roots: self.fs_allowed_roots.clone(),
            memory_backend: Arc::clone(&self.memory_backend),
        })
    }

    /// Persist a recursive Context ceiling into an owned Session clone.
    ///
    /// Detached execution cannot retain a borrowed Context. Narrowing the
    /// cloned Session baseline keeps reconstruction from recovering authority
    /// that the originating recursive scope had already removed.
    pub(crate) fn narrow_capabilities(
        mut self,
        capability_ceiling: &Capabilities,
    ) -> Result<Self, String> {
        if !capability_ceiling.is_subset(&self.baseline_capabilities) {
            return Err("detached session capabilities exceed their source Session".to_owned());
        }
        self.baseline_capabilities = capability_ceiling.clone();
        Ok(self)
    }

    #[must_use]
    pub(crate) fn with_active_skill_roots(mut self, active_skill_roots: Vec<PathBuf>) -> Self {
        self.active_skill_roots = active_skill_roots
            .into_iter()
            .map(|path| absolute_lexical(path, &self.fs_resolution_root))
            .collect();
        self
    }

    #[must_use]
    pub(crate) fn with_visible_skill_roots(mut self, visible_skill_roots: Vec<PathBuf>) -> Self {
        self.visible_skill_roots = visible_skill_roots
            .into_iter()
            .map(|path| absolute_lexical(path, &self.fs_resolution_root))
            .collect();
        // Active roots select from the host-visible skill authority; durable
        // state cannot turn a removed or relocated skill into a read root.
        self.active_skill_roots.retain(|active_root| {
            self.visible_skill_roots
                .iter()
                .any(|visible_root| active_root.starts_with(visible_root))
        });
        self
    }

    #[must_use]
    pub(crate) fn with_profile(mut self, profile: DelegateBuiltinProfile) -> Self {
        self.profile = Some(profile);
        self
    }

    pub fn resolved_runtime_narrowing(&self) -> Option<&ToolRuntimeNarrowing> {
        self.effective_runtime_narrowing.as_ref()
    }

    pub fn resolved_subagent_profile(&self) -> Option<ConstrainedSubagentProfile> {
        self.subagent_execution
            .as_ref()
            .map(ConstrainedSubagentExecution::resolved_profile)
    }

    pub fn resolved_subagent_identity(&self) -> Option<&ConstrainedSubagentIdentity> {
        self.subagent_execution
            .as_ref()
            .and_then(|execution| execution.identity.as_ref())
    }

    pub fn resolved_subagent_contract(&self) -> Option<ConstrainedSubagentContractView> {
        self.subagent_execution
            .as_ref()
            .map(ConstrainedSubagentExecution::contract_view)
    }

    #[must_use]
    pub(crate) fn subagent_execution(&self) -> Option<&ConstrainedSubagentExecution> {
        self.subagent_execution.as_ref()
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

    #[must_use]
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[must_use]
    pub fn parent_session_id(&self) -> Option<&str> {
        self.parent_session_id.as_deref()
    }

    #[must_use]
    pub(crate) fn mailbox(&self) -> &AgentMailbox {
        &self.mailbox
    }

    #[must_use]
    pub(crate) fn baseline_capabilities(&self) -> &Capabilities {
        &self.baseline_capabilities
    }
}

/// Apply the same owned identity invariant at both root and child construction.
/// A workspace-wide SessionId newtype is a separate contract migration; until
/// then this boundary must reject missing identity instead of inventing one.
fn validated_session_id(session_id: String) -> Result<String, String> {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        Err("session id must not be empty".to_owned())
    } else {
        Ok(trimmed.to_owned())
    }
}

#[cfg(test)]
#[path = "session/tests.rs"]
mod tests;
