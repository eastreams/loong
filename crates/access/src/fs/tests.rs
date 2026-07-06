use std::{
    borrow::Cow,
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use loong_contracts::{Capability, GrantId, PolicyEntry, PolicyOutcome};
use loong_core::policy::{
    action::Action,
    context::{PolicyContext, WorkspacePolicyContext},
    engine::{HasPolicyEngine, PolicyEngine},
};

use super::{CanonicalPath, FsAccess, FsAction, FsActionError, FsReadAction, HasFsAccess};

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

struct TestToolCx<'a> {
    kernel: &'a TestKernel,
    policy_context: TestPolicyContext,
}

struct TestAccessCx<'a> {
    kernel: &'a TestKernel,
    policy_context: TestPolicyContext,
}

impl<'a> TestToolCx<'a> {
    fn new(kernel: &'a TestKernel, workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            kernel,
            policy_context: TestPolicyContext::new(workspace_root),
        }
    }

    fn access(&self) -> TestAccessCx<'a> {
        TestAccessCx {
            kernel: self.kernel,
            policy_context: self.policy_context.clone(),
        }
    }
}

impl<'a> HasFsAccess<'a, TestKernel> for TestAccessCx<'a> {
    fn fs(self) -> FsAccess<'a, TestKernel> {
        FsAccess::new(self.kernel, self.policy_context)
    }
}

#[test]
fn canonical_path_resolves_relative_path_inside_workspace() {
    let path = CanonicalPath::resolve("docs/../notes/todo.md", Path::new("/workspace"))
        .expect("path inside workspace should normalize");

    assert_eq!(path.as_path(), Path::new("/workspace/notes/todo.md"));
}

#[test]
fn fs_read_action_uses_canonical_path() {
    let path = CanonicalPath::resolve("docs/../notes/todo.md", Path::new("/workspace"))
        .expect("path inside workspace should normalize");
    let action = FsReadAction::new(path);

    assert_eq!(action.path(), Path::new("/workspace/notes/todo.md"));
    assert_eq!(
        action.required_capabilities(),
        BTreeSet::from([Capability::FilesystemRead])
    );
}

#[test]
fn canonical_path_rejects_workspace_escape() {
    let error = CanonicalPath::resolve("../secrets.txt", Path::new("/workspace"))
        .expect_err("path escape should be denied");

    assert!(matches!(error, FsActionError::PathEscapesWorkspace { .. }));
}

#[test]
fn fs_action_wraps_read_action() {
    let path =
        CanonicalPath::resolve("notes.md", Path::new("/workspace")).expect("path inside workspace");
    let action = FsAction::read_file(path);

    assert_eq!(action.kind(), "fs.read");
    assert_eq!(action.operation(), Cow::Borrowed("read_file"));
}

#[cfg(unix)]
#[test]
fn canonical_path_rejects_symlink_escape() {
    let base = unique_temp_dir("loong-access-fs-read");
    let workspace_root = base.join("workspace");
    let outside_root = base.join("outside");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::create_dir_all(&outside_root).expect("create outside root");

    let outside_file = outside_root.join("secret.txt");
    fs::write(&outside_file, "secret").expect("write outside file");

    let symlink_path = workspace_root.join("secret-link");
    create_symlink(&outside_file, &symlink_path).expect("create symlink");

    let error = CanonicalPath::resolve("secret-link", &workspace_root)
        .expect_err("symlink escape should be denied");

    assert!(matches!(error, FsActionError::PathEscapesWorkspace { .. }));
}

#[tokio::test]
async fn tool_context_like_chain_grants_read_file_via_access_then_fs() {
    let kernel = TestKernel::default();
    let ctx = TestToolCx::new(&kernel, "/workspace");

    let grant = ctx
        .access()
        .fs()
        .read_file("notes/todo.md")
        .await
        .expect("grant should succeed");

    let granted = grant.granted.into_action();
    assert_eq!(grant.id, GrantId(1));
    assert_eq!(granted.path(), Path::new("/workspace/notes/todo.md"));
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}
