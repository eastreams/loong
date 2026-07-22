use std::borrow::Cow;
use std::fmt;
use std::path::{Path, PathBuf};

use loong_contracts::{
    AuthorizationScope, AuthorizationSubject, Capabilities, GovernedSessionMode, ToolPath,
};
use loong_core::policy::context::{ContextFactory, PolicyContext};
use loong_kernel::access::fs::{FsPathPolicyContext, FsResolutionContext};
use loong_kernel::{AccessCx, KernelAccess};
use loong_runtime::{
    runtime::Runtime,
    tool_plane::{
        ToolInvocation, ToolInvocationContext,
        error::{CapabilityNarrowingError, LookupError},
    },
};

mod memory;
mod session;

pub use session::Session;
#[cfg(feature = "memory-sqlite")]
pub(crate) use session::SessionToolPolicyProjection;

/// Borrowed recursive execution scope shared by Tool, Access, Action, and Policy.
///
/// Runtime and Session remain the only long-lived owners. Base fields borrow the
/// Session baseline; child scopes allocate only the views they actually narrow.
#[derive(Clone)]
pub struct Context<'a> {
    runtime: &'a Runtime<RuntimeContextFactory>,
    session: &'a Session,
    effective_capabilities: Cow<'a, Capabilities>,
}

#[derive(Debug, thiserror::Error)]
pub enum ContextSessionError {
    #[error("cannot construct Context from a Session owned by a different Runtime")]
    RuntimeMismatch,
    #[error(
        "cannot derive Context for unrelated Session `{target}` from `{current}` (parent: {parent:?})"
    )]
    Unrelated {
        current: String,
        target: String,
        parent: Option<String>,
    },
    #[error("cannot derive Context because target Session expands {dimension} authority")]
    AuthorityExpanded { dimension: &'static str },
    #[error("cannot derive Context because target Session replaces the memory backend")]
    MemoryBackendChanged,
}

impl fmt::Debug for Context<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Context")
            .field("session_id", &self.session.session_id)
            .field("parent_session_id", &self.session.parent_session_id)
            .field("session_mode", &self.session.session_mode)
            .finish_non_exhaustive()
    }
}

impl<'a> Context<'a> {
    /// Borrow the two long-lived owners into one recursive execution scope.
    ///
    /// Construction does not create authority: Session construction has already
    /// fixed the stable baseline, and recursive children may only narrow it.
    pub fn new(
        runtime: &'a Runtime<RuntimeContextFactory>,
        session: &'a Session,
    ) -> Result<Self, ContextSessionError> {
        if runtime.id() != session.runtime_id {
            return Err(ContextSessionError::RuntimeMismatch);
        }
        Ok(Self {
            runtime,
            session,
            effective_capabilities: Cow::Borrowed(&session.baseline_capabilities),
        })
    }

    #[must_use]
    pub(crate) fn runtime(&self) -> &'a Runtime<RuntimeContextFactory> {
        self.runtime
    }

    /// Rebind recursive execution to a rematerialized Session or direct child.
    ///
    /// Unlike [`Context::new`], this path inherits the current capability ceiling.
    /// It is the only valid constructor when execution already owns a Context.
    pub(crate) fn rebind_session<'b>(
        &'b self,
        session: &'b Session,
    ) -> Result<Context<'b>, ContextSessionError> {
        if self.runtime.id() != session.runtime_id {
            return Err(ContextSessionError::RuntimeMismatch);
        }
        let same_session = session.session_id() == self.session.session_id();
        let direct_child = session.parent_session_id() == Some(self.session.session_id());
        if !same_session && !direct_child {
            return Err(ContextSessionError::Unrelated {
                current: self.session.session_id().to_owned(),
                target: session.session_id().to_owned(),
                parent: session.parent_session_id().map(str::to_owned),
            });
        }

        if session.agent_id != self.session.agent_id {
            return Err(ContextSessionError::AuthorityExpanded {
                dimension: "agent identity",
            });
        }
        if same_session && session.parent_session_id != self.session.parent_session_id {
            return Err(ContextSessionError::AuthorityExpanded {
                dimension: "session lineage",
            });
        }
        if self.session.session_mode == GovernedSessionMode::AdvisoryOnly
            && session.session_mode == GovernedSessionMode::MutatingCapable
        {
            return Err(ContextSessionError::AuthorityExpanded {
                dimension: "session mode",
            });
        }
        if !session
            .baseline_capabilities
            .is_subset(&self.session.baseline_capabilities)
        {
            return Err(ContextSessionError::AuthorityExpanded {
                dimension: "capability",
            });
        }
        if !session.tool_view.is_subset(&self.session.tool_view) {
            return Err(ContextSessionError::AuthorityExpanded {
                dimension: "tool visibility",
            });
        }
        if session.fs_allowed_roots.iter().any(|target_root| {
            !self
                .session
                .fs_allowed_roots
                .iter()
                .any(|current_root| target_root.starts_with(current_root))
        }) {
            return Err(ContextSessionError::AuthorityExpanded {
                dimension: "filesystem root",
            });
        }
        let authority_ceiling = if direct_child {
            &self.session.fs_allowed_roots
        } else {
            &self.session.fs_authority_ceiling_roots
        };
        if session
            .fs_authority_ceiling_roots
            .iter()
            .any(|target_root| {
                !authority_ceiling
                    .iter()
                    .any(|current_root| target_root.starts_with(current_root))
            })
        {
            return Err(ContextSessionError::AuthorityExpanded {
                dimension: "filesystem authority ceiling",
            });
        }
        if session.fs_allowed_roots.iter().any(|allowed_root| {
            !session
                .fs_authority_ceiling_roots
                .iter()
                .any(|ceiling_root| allowed_root.starts_with(ceiling_root))
        }) {
            return Err(ContextSessionError::AuthorityExpanded {
                dimension: "filesystem root",
            });
        }
        if !session
            .fs_allowed_roots
            .iter()
            .any(|root| session.fs_resolution_root.starts_with(root))
        {
            return Err(ContextSessionError::AuthorityExpanded {
                dimension: "filesystem resolution",
            });
        }
        if session.visible_skill_roots.iter().any(|target_root| {
            !self
                .session
                .visible_skill_roots
                .iter()
                .any(|current_root| target_root.starts_with(current_root))
        }) {
            return Err(ContextSessionError::AuthorityExpanded {
                dimension: "visible skill root",
            });
        }
        if !std::sync::Arc::ptr_eq(&session.memory_backend, &self.session.memory_backend) {
            return Err(ContextSessionError::MemoryBackendChanged);
        }

        let current_runtime_narrowing = self
            .session
            .resolved_runtime_narrowing()
            .cloned()
            .unwrap_or_default();
        let target_runtime_narrowing = session
            .resolved_runtime_narrowing()
            .cloned()
            .unwrap_or_default();
        if !target_runtime_narrowing.is_no_wider_than(&current_runtime_narrowing) {
            return Err(ContextSessionError::AuthorityExpanded {
                dimension: "tool runtime narrowing",
            });
        }

        let mut expected_tool_runtime_config = self.session.tool_runtime_config.clone();
        if let Some(runtime_narrowing) = session.resolved_runtime_narrowing() {
            expected_tool_runtime_config = expected_tool_runtime_config.narrowed(runtime_narrowing);
        }
        if direct_child && let Some(workspace_root) = session.workspace_root.as_ref() {
            expected_tool_runtime_config.file_root = Some(workspace_root.clone());
            expected_tool_runtime_config.workspace_root = Some(workspace_root.clone());
        }
        if session.tool_runtime_config != expected_tool_runtime_config {
            return Err(ContextSessionError::AuthorityExpanded {
                dimension: "tool runtime configuration",
            });
        }

        let current = self.effective_capabilities.as_ref();
        let effective_capabilities = if current.is_subset(&session.baseline_capabilities) {
            Cow::Borrowed(current)
        } else {
            Cow::Owned(
                current
                    .intersection(&session.baseline_capabilities)
                    .collect(),
            )
        };
        Ok(Context {
            runtime: self.runtime,
            session,
            effective_capabilities,
        })
    }

    #[must_use]
    pub fn session(&self) -> &'a Session {
        self.session
    }

    #[must_use]
    pub fn agent_id(&self) -> &str {
        self.session.agent_id()
    }

    #[must_use]
    pub fn tool_runtime_config(&self) -> &crate::tools::runtime_config::ToolRuntimeConfig {
        &self.session.tool_runtime_config
    }

    #[must_use]
    pub fn access(&self) -> AccessCx<'_, 'a, RuntimeContextFactory> {
        // AccessCx construction is localized at the concrete context boundary.
        // Tool/action code should call ctx.access() rather than rethreading the
        // kernel reference or recreating access facades by hand.
        self.runtime.access(self)
    }

    pub fn tool(
        &self,
        path: ToolPath,
    ) -> Result<ToolInvocation<'a, 'a, RuntimeContextFactory>, LookupError> {
        self.runtime.tool(self, path)
    }
}

impl ToolInvocationContext for Context<'_> {
    fn derive_tool_child(
        &self,
        capabilities: Capabilities,
    ) -> Result<Self, CapabilityNarrowingError> {
        if !capabilities.is_subset(self.effective_capabilities.as_ref()) {
            return Err(CapabilityNarrowingError {
                allowed: self.effective_capabilities.clone().into_owned(),
                derived: capabilities,
            });
        }

        // Recursive invocations borrow the same owners and allocate only their
        // narrowed capability view.
        Ok(Self {
            runtime: self.runtime,
            session: self.session,
            effective_capabilities: Cow::Owned(capabilities),
        })
    }
}

impl crate::tools::plane::ToolVisibilityContext for Context<'_> {
    fn tool_is_visible(&self, path: &ToolPath) -> bool {
        self.session.tool_view.contains_path(path)
    }
}

impl KernelAccess<RuntimeContextFactory> for Context<'_> {
    fn access(&self) -> AccessCx<'_, '_, RuntimeContextFactory> {
        // Concrete tools depend on this narrow requirement instead of the app
        // context type. Delegate to the inherent accessor so this concrete
        // context has one AccessCx construction point.
        Context::access(self)
    }
}

impl PolicyContext for Context<'_> {
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        Cow::Borrowed(self.effective_capabilities.as_ref())
    }

    fn authorization_subject(&self) -> AuthorizationSubject {
        AuthorizationSubject {
            actor_id: self.agent_id().to_owned(),
            scope: AuthorizationScope::Session {
                session_id: self.session.session_id.clone(),
            },
        }
    }
}

impl FsResolutionContext for Context<'_> {
    fn fs_resolution_root(&self) -> &Path {
        self.session.fs_resolution_root.as_path()
    }
}

impl FsPathPolicyContext for Context<'_> {
    fn fs_allowed_roots(&self) -> &[PathBuf] {
        self.session.fs_allowed_roots.as_slice()
    }

    fn fs_authority_ceiling_roots(&self) -> &[PathBuf] {
        self.session.fs_authority_ceiling_roots.as_slice()
    }
}

/// Selects the app's borrowed Context for generic policy/runtime integration.
///
/// This marker constructs no values and owns no runtime state.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeContextFactory;

impl ContextFactory for RuntimeContextFactory {
    type Cx<'a> = Context<'a>;
}

#[cfg(test)]
#[path = "context/tests.rs"]
mod tests;
