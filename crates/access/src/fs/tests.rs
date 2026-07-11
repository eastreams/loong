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
        action::ActionMeta,
        context::{CapabilityContext, ContextFactory},
        engine::PolicyEngine,
    },
};

use super::{
    FsPathPolicyContext, FsResolutionContext,
    access::{FsAccess, FsAccessError},
    action::{FsAction, FsReadAction, FsResolvePathAction},
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

impl CapabilityContext for FsAccessPolicyContext {
    fn allowed_capabilities(&self) -> BTreeSet<Capability> {
        self.capabilities.clone()
    }
}

impl FsResolutionContext for FsAccessPolicyContext {
    fn fs_resolution_root(&self) -> &Path {
        &self.resolution_root
    }
}

impl FsPathPolicyContext for FsAccessPolicyContext {
    fn fs_allowed_roots(&self) -> &[PathBuf] {
        &self.allowed_roots
    }
}

struct FsAccessTestContextFactory;

impl ContextFactory for FsAccessTestContextFactory {
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
impl PolicyEngine<FsAccessTestContextFactory> for FsAccessPolicyEngine {
    async fn decide<A: ActionMeta + 'static>(
        &self,
        _ctx: &<FsAccessTestContextFactory as ContextFactory>::Cx<'_>,
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
impl Kernel<FsAccessTestContextFactory> for FsAccessTestKernel {
    type PolicyEngine = FsAccessPolicyEngine;

    fn policy_engine(&self) -> &Self::PolicyEngine {
        &self.policy
    }
}

struct FsAccessToolCx<'a> {
    kernel: &'a FsAccessTestKernel,
    ctx: FsAccessPolicyContext,
}

struct FsAccessTestCx<'a> {
    kernel: &'a FsAccessTestKernel,
    ctx: &'a FsAccessPolicyContext,
}

impl<'a> FsAccessToolCx<'a> {
    fn new(kernel: &'a FsAccessTestKernel, workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            kernel,
            ctx: FsAccessPolicyContext::new(workspace_root),
        }
    }

    fn access(&self) -> FsAccessTestCx<'_> {
        FsAccessTestCx {
            kernel: self.kernel,
            ctx: &self.ctx,
        }
    }
}

impl<'a> FsAccessTestCx<'a> {
    fn fs(self) -> FsAccess<'a, 'a, FsAccessTestContextFactory, FsAccessPolicyEngine> {
        FsAccess::new(self.kernel.policy_engine(), self.ctx)
    }
}

#[test]
fn fs_resolve_path_action_resolves_relative_path_inside_workspace() {
    let workspace_root = PathBuf::from("/workspace");
    let action = FsResolvePathAction::resolve("docs/../notes/todo.md", &workspace_root)
        .expect("path inside workspace should normalize");

    assert_eq!(
        action.resolved_path(),
        Path::new("/workspace/notes/todo.md")
    );
}

#[tokio::test]
async fn fs_resolve_path_action_outputs_granted_path_for_read_action() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let resolve_action =
        FsResolvePathAction::resolve("docs/../notes/todo.md", ctx.fs_resolution_root())
            .expect("path resolution should prepare action");
    let resolve_metadata = resolve_action.metadata();

    assert_eq!(resolve_metadata.kind, "fs.resolve_path");
    assert_eq!(resolve_metadata.operation, "resolve_path");
    assert!(resolve_metadata.required_capabilities.is_empty());
    let expected_resolve_payload = serde_json::json!({
        "path": "docs/../notes/todo.md",
        "resolved_path": "/workspace/notes/todo.md",
    });
    assert_eq!(resolve_action.payload().as_ref(), &expected_resolve_payload);

    let path = kernel
        .policy_engine()
        .grant(&ctx, resolve_action)
        .await
        .expect("policy should grant path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted path resolution should run");
    let action = FsReadAction::new(path);
    let metadata = action.metadata();

    assert_eq!(action.path(), Path::new("/workspace/notes/todo.md"));
    assert_eq!(metadata.operation, "read_file");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemRead]
    );
    let expected_payload = serde_json::json!({"path": "/workspace/notes/todo.md"});
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[test]
fn fs_resolve_path_action_marks_workspace_escape_for_policy() {
    let workspace_root = PathBuf::from("/workspace");
    let action = FsResolvePathAction::resolve("../secrets.txt", &workspace_root)
        .expect("path resolution should prepare escaped action for policy");

    assert_eq!(action.resolved_path(), Path::new("/secrets.txt"));
}

#[tokio::test]
async fn fs_action_wraps_read_action() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let path = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::resolve("notes.md", ctx.fs_resolution_root())
                .expect("path resolution should prepare action"),
        )
        .await
        .expect("policy should grant path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted path resolution should run");
    let action = FsAction::read_file(path);
    let metadata = action.metadata();

    assert_eq!(metadata.kind, "fs.read");
    assert_eq!(metadata.operation, "read_file");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemRead]
    );
    let expected_payload = serde_json::json!({"path": "/workspace/notes.md"});
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[cfg(unix)]
#[test]
fn fs_resolve_path_action_marks_symlink_escape_for_policy() {
    let base = unique_temp_dir("loong-access-fs-read");
    let workspace_root = base.join("workspace");
    let outside_root = base.join("outside");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::create_dir_all(&outside_root).expect("create outside root");

    let outside_file = outside_root.join("secret.txt");
    fs::write(&outside_file, "secret").expect("write outside file");

    let symlink_path = workspace_root.join("secret-link");
    create_symlink(&outside_file, &symlink_path).expect("create symlink");

    let action = FsResolvePathAction::resolve("secret-link", &workspace_root)
        .expect("path resolution should prepare symlink escape for policy");

    assert_eq!(
        action.resolved_path(),
        dunce::canonicalize(&outside_file).expect("canonical outside file")
    );
}

#[cfg(unix)]
#[test]
fn fs_resolve_path_action_resolves_missing_allowed_root_through_symlink_ancestor() {
    let base = unique_temp_dir("loong-access-fs-missing-root-symlink");
    let workspace_root = base.join("workspace");
    let outside_root = base.join("outside");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::create_dir_all(&outside_root).expect("create outside root");

    let link_path = workspace_root.join("linked-root-parent");
    create_symlink(&outside_root, &link_path).expect("create symlink");

    let allowed_root = link_path.join("missing-root");
    let action = FsResolvePathAction::resolve("notes.txt", &allowed_root)
        .expect("missing allowed root under symlink ancestor should resolve");

    let expected_root = dunce::canonicalize(&outside_root).expect("canonical outside root");
    assert_eq!(
        action.resolved_path(),
        expected_root.join("missing-root/notes.txt")
    );
    fs::remove_dir_all(base).ok();
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
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let path = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::resolve("notes/todo.md", ctx.fs_resolution_root())
                .expect("path resolution should prepare action"),
        )
        .await
        .expect("policy should grant path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted path resolution should run");
    let action = FsReadAction::new(path);
    let grant = kernel
        .policy_engine()
        .grant(&ctx, action)
        .await
        .expect("policy should grant read");

    let output = grant
        .granted
        .run(&ctx)
        .await
        .expect("granted read should execute");

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
