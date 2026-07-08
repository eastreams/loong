use std::{
    borrow::Cow,
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use loong_contracts::{Capability, GrantId, PolicyEntry, PolicyOutcome, PolicyReport};
use loong_core::{
    kernel::Kernel,
    policy::{
        action::Action,
        context::{ContextFactory, PolicyContext},
        engine::PolicyEngine,
    },
};

use super::{
    access::{FsAccess, FsAccessContext, FsAccessError, read_granted_file},
    action::{FsAction, FsReadAction},
    error::FsActionError,
    path::CanonicalPath,
};

#[derive(Debug, Clone)]
struct FsAccessPolicyContext {
    resolution_root: PathBuf,
    allowed_roots: Vec<PathBuf>,
    capabilities: BTreeSet<Capability>,
}

impl FsAccessPolicyContext {
    fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            resolution_root: root.clone(),
            allowed_roots: vec![root],
            capabilities: BTreeSet::from([Capability::FilesystemRead]),
        }
    }
}

impl PolicyContext for FsAccessPolicyContext {
    fn capabilities(&self) -> BTreeSet<Capability> {
        self.capabilities.clone()
    }
}

impl FsAccessContext for FsAccessPolicyContext {
    fn fs_resolution_root(&self) -> &Path {
        &self.resolution_root
    }

    fn fs_allowed_roots(&self) -> &[PathBuf] {
        &self.allowed_roots
    }
}

struct FsAccessContextFactory;

impl ContextFactory for FsAccessContextFactory {
    type Cx<'a> = FsAccessPolicyContext;
}

struct FsAccessPolicyEngine {
    next_grant_id: AtomicU64,
    allow: bool,
}

impl Default for FsAccessPolicyEngine {
    fn default() -> Self {
        Self {
            next_grant_id: AtomicU64::new(0),
            allow: true,
        }
    }
}

#[async_trait]
impl PolicyEngine<FsAccessContextFactory> for FsAccessPolicyEngine {
    async fn decide<A: Action + 'static>(
        &self,
        _ctx: &<FsAccessContextFactory as ContextFactory>::Cx<'_>,
        _action: &A,
    ) -> PolicyReport {
        if self.allow {
            return PolicyReport {
                evaluations: Vec::new(),
                outcome: PolicyOutcome::Allow {
                    source: PolicyEntry {
                        policy_name: Cow::Borrowed("allow-all"),
                        policy_id: 1,
                    },
                    reason: Cow::Borrowed("allowed"),
                },
            };
        }

        PolicyReport {
            evaluations: Vec::new(),
            outcome: PolicyOutcome::Deny {
                grant_source: None,
                reason: Cow::Borrowed("denied by test policy"),
            },
        }
    }

    async fn next_grant_id(&self) -> GrantId {
        GrantId(self.next_grant_id.fetch_add(1, Ordering::Relaxed) + 1)
    }
}

#[derive(Default)]
struct FsAccessTestKernel {
    policy: FsAccessPolicyEngine,
}

impl FsAccessTestKernel {
    fn denying() -> Self {
        Self {
            policy: FsAccessPolicyEngine {
                next_grant_id: AtomicU64::new(0),
                allow: false,
            },
        }
    }
}

#[async_trait]
impl Kernel<FsAccessContextFactory> for FsAccessTestKernel {
    type PolicyEngine = FsAccessPolicyEngine;

    fn policy_engine(&self) -> &Self::PolicyEngine {
        &self.policy
    }
}

struct FsAccessToolCx<'a> {
    kernel: &'a FsAccessTestKernel,
    policy_context: FsAccessPolicyContext,
}

struct FsAccessTestCx<'a> {
    kernel: &'a FsAccessTestKernel,
    policy_context: FsAccessPolicyContext,
}

impl<'a> FsAccessToolCx<'a> {
    fn new(kernel: &'a FsAccessTestKernel, workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            kernel,
            policy_context: FsAccessPolicyContext::new(workspace_root),
        }
    }

    fn access(&self) -> FsAccessTestCx<'a> {
        FsAccessTestCx {
            kernel: self.kernel,
            policy_context: self.policy_context.clone(),
        }
    }
}

impl<'a> FsAccessTestCx<'a> {
    fn fs(self) -> FsAccess<'a, FsAccessContextFactory, FsAccessPolicyEngine> {
        FsAccess::new(self.kernel.policy_engine(), self.policy_context)
    }
}

#[test]
fn canonical_path_resolves_relative_path_inside_workspace() {
    let workspace_root = PathBuf::from("/workspace");
    let path = CanonicalPath::resolve(
        "docs/../notes/todo.md",
        &workspace_root,
        std::slice::from_ref(&workspace_root),
    )
    .expect("path inside workspace should normalize");

    assert_eq!(path.as_path(), Path::new("/workspace/notes/todo.md"));
}

#[test]
fn fs_read_action_uses_canonical_path() {
    let workspace_root = PathBuf::from("/workspace");
    let path = CanonicalPath::resolve(
        "docs/../notes/todo.md",
        &workspace_root,
        std::slice::from_ref(&workspace_root),
    )
    .expect("path inside workspace should normalize");
    let action = FsReadAction::new(path);

    assert_eq!(action.path(), Path::new("/workspace/notes/todo.md"));
    assert_eq!(action.operation(), Cow::Borrowed("read_file"));
    assert_eq!(
        action.required_capabilities(),
        BTreeSet::from([Capability::FilesystemRead])
    );
}

#[test]
fn canonical_path_rejects_workspace_escape() {
    let workspace_root = PathBuf::from("/workspace");
    let error = CanonicalPath::resolve(
        "../secrets.txt",
        &workspace_root,
        std::slice::from_ref(&workspace_root),
    )
    .expect_err("path escape should be denied");

    assert!(matches!(
        error,
        FsActionError::PathEscapesAllowedRoot { .. }
    ));
}

#[test]
fn fs_action_wraps_read_action() {
    let workspace_root = PathBuf::from("/workspace");
    let path = CanonicalPath::resolve(
        "notes.md",
        &workspace_root,
        std::slice::from_ref(&workspace_root),
    )
    .expect("path inside workspace");
    let action = FsAction::read_file(path);

    assert_eq!(action.kind(), "fs.read");
    assert_eq!(action.operation(), Cow::Borrowed("read_file"));
    assert_eq!(
        action.required_capabilities(),
        BTreeSet::from([Capability::FilesystemRead])
    );
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

    let error = CanonicalPath::resolve(
        "secret-link",
        &workspace_root,
        std::slice::from_ref(&workspace_root),
    )
    .expect_err("symlink escape should be denied");

    assert!(matches!(
        error,
        FsActionError::PathEscapesAllowedRoot { .. }
    ));
}

#[tokio::test]
async fn tool_context_like_chain_grants_read_file_via_access_then_fs() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-read-output");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("notes")).expect("create notes dir");
    fs::write(workspace_root.join("notes/todo.md"), "hello").expect("write note");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .read_file("notes/todo.md")
        .await
        .expect("grant should succeed");

    let expected_path =
        dunce::canonicalize(workspace_root.join("notes/todo.md")).expect("canonical note path");
    assert_eq!(output.path, expected_path);
    assert_eq!(output.bytes, b"hello");

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_read_execution_boundary_consumes_granted_action() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-granted-boundary");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("notes")).expect("create notes dir");
    fs::write(workspace_root.join("notes/todo.md"), "hello").expect("write note");
    let policy_context = FsAccessPolicyContext::new(&workspace_root);
    let path = CanonicalPath::resolve(
        "notes/todo.md",
        policy_context.fs_resolution_root(),
        policy_context.fs_allowed_roots(),
    )
    .expect("path inside workspace should resolve");
    let action = FsReadAction::new(path);
    let grant = kernel
        .policy_engine()
        .grant(&policy_context, action)
        .await
        .expect("policy should grant read");

    let output = read_granted_file(grant.granted).expect("granted read should execute");

    let expected_path =
        dunce::canonicalize(workspace_root.join("notes/todo.md")).expect("canonical note path");
    assert_eq!(output.path, expected_path);
    assert_eq!(output.bytes, b"hello");

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_access_denies_before_reading_file() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-deny-before-read");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .read_file("missing.txt")
        .await
        .expect_err("policy denial should happen before file read");

    assert!(matches!(error, FsAccessError::Authorization(_)));

    fs::remove_dir_all(base).ok();
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
