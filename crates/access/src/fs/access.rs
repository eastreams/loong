use std::path::Path;

use loong_core::{
    error::ExecutionError,
    policy::{
        context::PolicyContextFor,
        engine::{HasPolicyEngine, PolicyEngine},
    },
};

use super::{CanonicalRoot, FsAction, FsBackend, FsReadAction, FsWriteAction, action};

pub trait HasFsAccess {
    type Host: FsBackend + HasPolicyEngine;

    fn fs(&self) -> FsAccess<'_, Self::Host>;
}

/// Filesystem access facade exposed as `ctx.fs()`.
///
/// Public methods remain natural and function-like. Internally they follow the
/// same pipeline: construct an action, ask policy to grant it, then execute the
/// granted action. Concrete runtimes can wrap grant / execute with audit.
pub struct FsAccess<'a, K>
where
    K: HasPolicyEngine + FsBackend + ?Sized,
{
    kernel: &'a K,
    workspace_root: CanonicalRoot,
    policy_context: PolicyContextFor<'a, K>,
}

impl<'a, K> FsAccess<'a, K>
where
    K: HasPolicyEngine + FsBackend,
{
    pub fn new(
        kernel: &'a K,
        workspace_root: &'a Path,
        policy_context: PolicyContextFor<'a, K>,
    ) -> Self {
        let workspace_root = CanonicalRoot::existing(workspace_root)
            .expect("tool context workspace root is canonical");
        Self {
            kernel,
            workspace_root,
            policy_context,
        }
    }

    pub async fn read_file(&self, path: &str) -> Result<String, ExecutionError> {
        let path = action::resolve_under_authorization(&self.workspace_root, Path::new(path))
            .map_err(ExecutionError::Authorization)?;

        let parent = FsAction::new(path);
        let grant = self
            .kernel
            .policy_engine()
            .grant(&self.policy_context, parent)
            .await
            .map_err(ExecutionError::Authorization)?;

        let action = FsReadAction::new(grant.granted);
        let grant = self
            .kernel
            .policy_engine()
            .grant(&self.policy_context, action)
            .await
            .map_err(ExecutionError::Authorization)?;

        let executor: &dyn FsBackend = self.kernel;
        self.kernel.execute_granted(grant.granted, executor).await
    }

    pub async fn write_file(&self, path: &str, content: &str) -> Result<(), ExecutionError> {
        let path = action::resolve_write_under_authorization(&self.workspace_root, Path::new(path))
            .map_err(ExecutionError::Authorization)?;

        let parent = FsAction::new(path);
        let grant = self
            .kernel
            .policy_engine()
            .grant(&self.policy_context, parent)
            .await
            .map_err(ExecutionError::Authorization)?;

        let action = FsWriteAction::new(grant.granted, content.to_owned());
        let grant = self
            .kernel
            .policy_engine()
            .grant(&self.policy_context, action)
            .await
            .map_err(ExecutionError::Authorization)?;

        let executor: &dyn FsBackend = self.kernel;
        self.kernel.execute_granted(grant.granted, executor).await
    }
}
