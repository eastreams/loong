use std::{
    borrow::Cow,
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use loong_contracts::{Capability, GrantId, PolicyEntry, PolicyOutcome};
use loong_core::policy::{
    action::Action,
    context::{PolicyContext, WorkspacePolicyContext},
    engine::{HasPolicyEngine, PolicyEngine},
};

use super::AccessCx;
use crate::{HasFsAccess, LoongKernel};

#[derive(Debug, Clone)]
struct TestPolicyContext {
    workspace_root: PathBuf,
    capabilities: BTreeSet<Capability>,
}

impl TestPolicyContext {
    fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            capabilities: BTreeSet::from([Capability::FilesystemRead]),
        }
    }
}

impl PolicyContext for TestPolicyContext {
    fn capabilities(&self) -> BTreeSet<Capability> {
        self.capabilities.clone()
    }
}

impl WorkspacePolicyContext for TestPolicyContext {
    fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }
}

#[derive(Default)]
struct AllowAllPolicyEngine {
    next_grant_id: AtomicU64,
}

#[async_trait]
impl PolicyEngine for AllowAllPolicyEngine {
    type Cx<'a> = TestPolicyContext;

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
struct TestKernel {
    policy: AllowAllPolicyEngine,
}

#[async_trait]
impl HasPolicyEngine for TestKernel {
    type PolicyEngine<'a>
        = AllowAllPolicyEngine
    where
        Self: 'a;

    fn policy_engine(&self) -> &Self::PolicyEngine<'_> {
        &self.policy
    }
}

#[test]
fn loong_kernel_exposes_access_types_and_fs_surface_for_workspace_kernels() {
    fn assert_has_policy_engine<T: HasPolicyEngine>() {}
    fn assert_access_exported<T>() {}
    fn assert_has_fs_access<'a, T: HasFsAccess<'a, TestKernel>>() {}

    assert_has_policy_engine::<LoongKernel>();
    assert_access_exported::<AccessCx<'static, LoongKernel>>();

    assert_access_exported::<AccessCx<'static, TestKernel>>();
    assert_has_fs_access::<AccessCx<'static, TestKernel>>();
}

#[tokio::test]
async fn access_context_preserves_workspace_policy_context_for_fs_access() {
    let kernel = TestKernel::default();
    let access = AccessCx::new(&kernel, TestPolicyContext::new("/workspace"));

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
