use std::{
    borrow::Cow,
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use loong_contracts::{Capability, GrantId, PolicyEntry, PolicyOutcome};
use loong_core::{
    kernel::Kernel as CoreKernel,
    policy::{
        action::Action,
        context::{ActionContext, PolicyContext, WorkspacePolicyContext},
        engine::PolicyEngine,
    },
};

use super::AccessCx;
use crate::{HasFsAccess, Kernel as RuntimeKernel};

#[derive(Debug, Clone)]
struct AccessCxPolicyContext {
    workspace_root: PathBuf,
    capabilities: BTreeSet<Capability>,
}

impl AccessCxPolicyContext {
    fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            capabilities: BTreeSet::from([Capability::FilesystemRead]),
        }
    }
}

impl PolicyContext for AccessCxPolicyContext {
    fn capabilities(&self) -> BTreeSet<Capability> {
        self.capabilities.clone()
    }
}

impl WorkspacePolicyContext for AccessCxPolicyContext {
    fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }
}

impl ActionContext for AccessCxPolicyContext {
    fn execution_plane(&self) -> loong_contracts::ExecutionPlane {
        loong_contracts::ExecutionPlane::Tool
    }

    fn plane_tier(&self) -> loong_contracts::PlaneTier {
        loong_contracts::PlaneTier::Core
    }
}

#[derive(Default)]
struct AccessCxPolicyEngine {
    next_grant_id: AtomicU64,
}

#[async_trait]
impl PolicyEngine for AccessCxPolicyEngine {
    type Cx<'a> = AccessCxPolicyContext;

    async fn decide<A: Action>(&self, _ctx: &Self::Cx<'_>, _action: &A) -> PolicyOutcome {
        PolicyOutcome::Allow {
            source: PolicyEntry {
                policy_name: Cow::Borrowed("allow-all"),
                policy_id: 1,
            },
            reason: Cow::Borrowed("allowed"),
        }
    }

    async fn next_grant_id(&self) -> GrantId {
        GrantId(self.next_grant_id.fetch_add(1, Ordering::Relaxed) + 1)
    }
}

#[derive(Default)]
struct AccessCxTestKernel {
    policy: AccessCxPolicyEngine,
}

#[async_trait]
impl CoreKernel for AccessCxTestKernel {
    type Cx<'a> = AccessCxPolicyContext;
    type PolicyEngine = AccessCxPolicyEngine;

    fn policy_engine(&self) -> &Self::PolicyEngine {
        &self.policy
    }
}

#[test]
fn loong_kernel_exposes_access_types_and_fs_surface_for_workspace_kernels() {
    fn assert_access_exported<T>() {}
    fn assert_has_fs_access<'a, T: HasFsAccess<'a, AccessCxTestKernel>>() {}

    assert_access_exported::<AccessCx<'static, RuntimeKernel>>();

    assert_access_exported::<AccessCx<'static, AccessCxTestKernel>>();
    assert_has_fs_access::<AccessCx<'static, AccessCxTestKernel>>();
}

#[tokio::test]
async fn access_context_preserves_workspace_policy_context_for_fs_access() {
    let kernel = AccessCxTestKernel::default();
    let access = AccessCx::new(&kernel, AccessCxPolicyContext::new("/workspace"));

    let grant = access
        .fs()
        .read_file("notes/todo.md")
        .await
        .expect("grant should succeed");

    assert_eq!(grant.id, GrantId(1));
    assert_eq!(
        grant.granted.into_action().path(),
        Path::new("/workspace/notes/todo.md")
    );
}
