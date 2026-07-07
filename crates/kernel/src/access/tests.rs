use std::{
    borrow::Cow,
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use loong_contracts::{Capability, GrantId, PolicyEntry, PolicyOutcome, PolicyReport};
use loong_core::{
    kernel::Kernel as CoreKernel,
    policy::{
        action::Action,
        context::{ActionContext, PolicyContext},
        engine::PolicyEngine,
    },
};

use super::AccessCx;
use crate::Kernel as RuntimeKernel;
use loong_access::fs::access::FsAccessContext;

#[derive(Debug, Clone)]
struct AccessCxPolicyContext {
    resolution_root: PathBuf,
    allowed_roots: Vec<PathBuf>,
    capabilities: BTreeSet<Capability>,
}

impl AccessCxPolicyContext {
    fn new(workspace_root: impl Into<PathBuf>) -> Self {
        let workspace_root = workspace_root.into();
        Self {
            resolution_root: workspace_root.clone(),
            allowed_roots: vec![workspace_root],
            capabilities: BTreeSet::from([Capability::FilesystemRead]),
        }
    }
}

impl PolicyContext for AccessCxPolicyContext {
    fn capabilities(&self) -> BTreeSet<Capability> {
        self.capabilities.clone()
    }
}

impl FsAccessContext for AccessCxPolicyContext {
    fn fs_resolution_root(&self) -> &Path {
        &self.resolution_root
    }

    fn fs_allowed_roots(&self) -> &[PathBuf] {
        &self.allowed_roots
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

    async fn decide<A: Action + 'static>(&self, _ctx: &Self::Cx<'_>, _action: &A) -> PolicyReport {
        PolicyReport {
            evaluations: Vec::new(),
            outcome: PolicyOutcome::Allow {
                source: PolicyEntry {
                    policy_name: Cow::Borrowed("allow-all"),
                    policy_id: 1,
                },
                reason: Cow::Borrowed("allowed"),
            },
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

    assert_access_exported::<AccessCx<'static, RuntimeKernel>>();
    assert_access_exported::<AccessCx<'static, AccessCxTestKernel>>();
}

#[tokio::test]
async fn access_context_preserves_workspace_policy_context_for_fs_access() {
    let kernel = AccessCxTestKernel::default();
    let base = tempfile_dir("loong-kernel-access-context");
    let workspace_root = base.join("workspace");
    std::fs::create_dir_all(workspace_root.join("notes")).expect("create notes dir");
    std::fs::write(workspace_root.join("notes/todo.md"), "hello").expect("write note");
    let access = AccessCx::new(&kernel, AccessCxPolicyContext::new(&workspace_root));

    let output = access
        .fs()
        .read_file("notes/todo.md")
        .await
        .expect("read should succeed");

    let expected_path =
        std::fs::canonicalize(workspace_root.join("notes/todo.md")).expect("canonicalize note");
    assert_eq!(output.path, expected_path);
    assert_eq!(output.bytes, b"hello");

    std::fs::remove_dir_all(base).ok();
}

fn tempfile_dir(prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}
