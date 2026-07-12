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
    FsPathKind, FsPathPolicyContext, FsResolutionContext,
    access::{FsAccess, FsAccessError},
    action::{
        FsAction, FsContentSearchAction, FsContentSearchOptions, FsCopyFileAction,
        FsCreateDirAllAction, FsGlobAction, FsInspectPathAction, FsReadAction, FsResolvePathAction,
        FsWriteAction, FsWriteOptions,
    },
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
            capabilities: BTreeSet::from([Capability::FilesystemRead, Capability::FilesystemWrite]),
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

#[tokio::test]
async fn fs_write_action_writes_file_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-write");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .write_file(
            "notes/todo.md",
            b"hello".to_vec(),
            FsWriteOptions {
                create_dirs: true,
                overwrite: false,
            },
        )
        .await
        .expect("write should execute after policy grants");

    assert_eq!(output.bytes_written, 5);
    assert!(!output.overwritten);
    assert_eq!(
        fs::read_to_string(workspace_root.join("notes/todo.md")).expect("read written file"),
        "hello"
    );

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_write_action_uses_granted_path() {
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
    let action = FsWriteAction::new(
        path,
        b"hello".to_vec(),
        FsWriteOptions {
            create_dirs: true,
            overwrite: false,
        },
    );
    let metadata = action.metadata();

    assert_eq!(action.path(), Path::new("/workspace/notes.md"));
    assert_eq!(metadata.kind, "fs.write");
    assert_eq!(metadata.operation, "write_file");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemWrite]
    );
    let expected_payload = serde_json::json!({
        "path": "/workspace/notes.md",
        "byte_count": 5,
        "create_dirs": true,
        "overwrite": false,
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn fs_action_wraps_write_action() {
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
    let action = FsAction::write_file(
        path,
        b"hello".to_vec(),
        FsWriteOptions {
            create_dirs: false,
            overwrite: true,
        },
    );
    let metadata = action.metadata();

    assert_eq!(metadata.kind, "fs.write");
    assert_eq!(metadata.operation, "write_file");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemWrite]
    );
    let expected_payload = serde_json::json!({
        "path": "/workspace/notes.md",
        "byte_count": 5,
        "create_dirs": false,
        "overwrite": true,
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn fs_copy_file_copies_file_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-copy-file");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::write(workspace_root.join("source.txt"), "hello").expect("write source");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .copy_file(
            "source.txt",
            "backup/source.txt",
            FsWriteOptions {
                create_dirs: true,
                overwrite: false,
            },
        )
        .await
        .expect("copy should execute after policy grants");

    assert_eq!(output.bytes_copied, 5);
    assert!(!output.overwritten);
    assert_eq!(
        fs::read_to_string(workspace_root.join("backup/source.txt")).expect("read copied file"),
        "hello"
    );

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_copy_file_action_uses_granted_paths() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let source = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::resolve("source.txt", ctx.fs_resolution_root())
                .expect("source path resolution should prepare action"),
        )
        .await
        .expect("policy should grant source path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted source path resolution should run");
    let destination = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::resolve("backup/source.txt", ctx.fs_resolution_root())
                .expect("destination path resolution should prepare action"),
        )
        .await
        .expect("policy should grant destination path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted destination path resolution should run");
    let action = FsCopyFileAction::new(
        source,
        destination,
        FsWriteOptions {
            create_dirs: true,
            overwrite: false,
        },
    );
    let metadata = action.metadata();

    assert_eq!(action.source_path(), Path::new("/workspace/source.txt"));
    assert_eq!(
        action.destination_path(),
        Path::new("/workspace/backup/source.txt")
    );
    assert_eq!(metadata.kind, "fs.copy_file");
    assert_eq!(metadata.operation, "copy_file");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemRead, Capability::FilesystemWrite]
    );
    let expected_payload = serde_json::json!({
        "source": "/workspace/source.txt",
        "destination": "/workspace/backup/source.txt",
        "create_dirs": true,
        "overwrite": false,
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn fs_action_wraps_copy_file_action() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let source = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::resolve("source.txt", ctx.fs_resolution_root())
                .expect("source path resolution should prepare action"),
        )
        .await
        .expect("policy should grant source path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted source path resolution should run");
    let destination = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::resolve("backup/source.txt", ctx.fs_resolution_root())
                .expect("destination path resolution should prepare action"),
        )
        .await
        .expect("policy should grant destination path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted destination path resolution should run");
    let action = FsAction::copy_file(
        source,
        destination,
        FsWriteOptions {
            create_dirs: false,
            overwrite: true,
        },
    );
    let metadata = action.metadata();

    assert_eq!(metadata.kind, "fs.copy_file");
    assert_eq!(metadata.operation, "copy_file");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemRead, Capability::FilesystemWrite]
    );
    let expected_payload = serde_json::json!({
        "source": "/workspace/source.txt",
        "destination": "/workspace/backup/source.txt",
        "create_dirs": false,
        "overwrite": true,
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn fs_copy_file_denies_before_copying_file() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-copy-file-deny");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::write(workspace_root.join("source.txt"), "hello").expect("write source");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .copy_file(
            "source.txt",
            "backup/source.txt",
            FsWriteOptions {
                create_dirs: true,
                overwrite: false,
            },
        )
        .await
        .expect_err("policy denial should happen before file copy");

    assert!(matches!(error, FsAccessError::Authorization(_)));
    assert!(!workspace_root.join("backup/source.txt").exists());

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_copy_file_rejects_existing_destination_without_overwrite() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-copy-file-overwrite");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::write(workspace_root.join("source.txt"), "hello").expect("write source");
    let destination = workspace_root.join("destination.txt");
    fs::write(&destination, "old").expect("write destination");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .copy_file(
            "source.txt",
            "destination.txt",
            FsWriteOptions {
                create_dirs: false,
                overwrite: false,
            },
        )
        .await
        .expect_err("existing destination should require overwrite");

    assert!(matches!(
        error,
        FsAccessError::FileExistsRequiresOverwrite { .. }
    ));
    assert_eq!(
        fs::read_to_string(destination).expect("read original destination"),
        "old"
    );

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_create_dir_all_creates_nested_directory_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-create-dir-all");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .create_dir_all("state/imports/latest")
        .await
        .expect("create_dir_all should execute after policy grants");

    let expected_path = dunce::canonicalize(workspace_root.join("state/imports/latest"))
        .expect("canonical created directory");
    assert_eq!(output.path, expected_path);
    assert!(!output.already_exists);
    assert!(workspace_root.join("state/imports/latest").is_dir());

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_create_dir_all_reports_existing_directory() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-create-dir-existing");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("state")).expect("create existing directory");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .create_dir_all("state")
        .await
        .expect("existing directory should be accepted");

    assert!(output.already_exists);

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_create_dir_all_action_uses_granted_path() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let path = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::resolve("state", ctx.fs_resolution_root())
                .expect("path resolution should prepare action"),
        )
        .await
        .expect("policy should grant path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted path resolution should run");
    let action = FsCreateDirAllAction::new(path);
    let metadata = action.metadata();

    assert_eq!(action.path(), Path::new("/workspace/state"));
    assert_eq!(metadata.kind, "fs.create_dir_all");
    assert_eq!(metadata.operation, "create_dir_all");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemWrite]
    );
    let expected_payload = serde_json::json!({
        "path": "/workspace/state",
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn fs_action_wraps_create_dir_all_action() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let path = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::resolve("state", ctx.fs_resolution_root())
                .expect("path resolution should prepare action"),
        )
        .await
        .expect("policy should grant path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted path resolution should run");
    let action = FsAction::create_dir_all(path);
    let metadata = action.metadata();

    assert_eq!(metadata.kind, "fs.create_dir_all");
    assert_eq!(metadata.operation, "create_dir_all");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemWrite]
    );
    let expected_payload = serde_json::json!({
        "path": "/workspace/state",
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn fs_create_dir_all_denies_before_creating_directory() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-create-dir-deny");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .create_dir_all("state")
        .await
        .expect_err("policy denial should happen before directory creation");

    assert!(matches!(error, FsAccessError::Authorization(_)));
    assert!(!workspace_root.join("state").exists());

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_inspect_path_reports_file_kind_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-inspect");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::write(workspace_root.join("notes.md"), "hello").expect("write note");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .inspect_path("notes.md")
        .await
        .expect("inspect should execute after policy grants");

    let expected_path =
        dunce::canonicalize(workspace_root.join("notes.md")).expect("canonical note path");
    assert_eq!(output.path, expected_path);
    assert_eq!(output.kind, Some(FsPathKind::File));

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_inspect_path_reports_missing_kind_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-inspect-missing");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .inspect_path("missing.md")
        .await
        .expect("missing path inspect should still be governed");

    let expected_path = dunce::canonicalize(&workspace_root)
        .expect("canonical workspace root")
        .join("missing.md");
    assert_eq!(output.path, expected_path);
    assert_eq!(output.kind, None);

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_inspect_path_action_uses_granted_path() {
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
    let action = FsInspectPathAction::new(path);
    let metadata = action.metadata();

    assert_eq!(action.path(), Path::new("/workspace/notes.md"));
    assert_eq!(metadata.kind, "fs.inspect_path");
    assert_eq!(metadata.operation, "inspect_path");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemRead]
    );
    let expected_payload = serde_json::json!({
        "path": "/workspace/notes.md",
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn fs_action_wraps_inspect_path_action() {
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
    let action = FsAction::inspect_path(path);
    let metadata = action.metadata();

    assert_eq!(metadata.kind, "fs.inspect_path");
    assert_eq!(metadata.operation, "inspect_path");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemRead]
    );
    let expected_payload = serde_json::json!({
        "path": "/workspace/notes.md",
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn fs_inspect_path_denies_before_inspecting_path() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-inspect-deny");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .inspect_path("notes.md")
        .await
        .expect_err("policy denial should happen before path inspect");

    assert!(matches!(error, FsAccessError::Authorization(_)));

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_write_denies_before_creating_file() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-write-deny");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .write_file(
            "notes/todo.md",
            b"hello".to_vec(),
            FsWriteOptions {
                create_dirs: true,
                overwrite: false,
            },
        )
        .await
        .expect_err("policy denial should happen before file creation");

    assert!(matches!(error, FsAccessError::Authorization(_)));
    assert!(!workspace_root.join("notes/todo.md").exists());

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_write_rejects_existing_file_without_overwrite() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-write-overwrite");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let target = workspace_root.join("notes.md");
    fs::write(&target, "old").expect("seed existing file");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .write_file(
            "notes.md",
            b"new".to_vec(),
            FsWriteOptions {
                create_dirs: false,
                overwrite: false,
            },
        )
        .await
        .expect_err("existing file should require overwrite");

    assert!(matches!(
        error,
        FsAccessError::FileExistsRequiresOverwrite { .. }
    ));
    assert_eq!(
        fs::read_to_string(target).expect("read original file"),
        "old"
    );

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_glob_action_lists_matching_paths_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-glob-paths");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("src/nested")).expect("create source dirs");
    fs::write(workspace_root.join("src/lib.rs"), "pub fn lib() {}").expect("write lib");
    fs::write(workspace_root.join("src/main.rs"), "fn main() {}").expect("write main");
    fs::write(workspace_root.join("src/nested/mod.rs"), "pub mod nested;").expect("write nested");
    fs::write(workspace_root.join("README.md"), "readme").expect("write readme");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .glob_paths(".", "**/*.rs", false, 10)
        .await
        .expect("glob should execute after policy grants");

    assert_eq!(
        output
            .matches
            .iter()
            .map(|entry| (entry.relative_path.as_str(), entry.kind))
            .collect::<Vec<_>>(),
        vec![
            ("src/lib.rs", FsPathKind::File),
            ("src/main.rs", FsPathKind::File),
            ("src/nested/mod.rs", FsPathKind::File),
        ]
    );
    assert!(!output.truncated);

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_glob_action_uses_granted_path_root() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let path = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::resolve("src", ctx.fs_resolution_root())
                .expect("path resolution should prepare action"),
        )
        .await
        .expect("policy should grant path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted path resolution should run");
    let action = FsGlobAction::new(path, "**/*.rs", true, 50);
    let metadata = action.metadata();

    assert_eq!(metadata.kind, "fs.glob");
    assert_eq!(metadata.operation, "glob_paths");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemRead]
    );
    let expected_payload = serde_json::json!({
        "root": "/workspace/src",
        "pattern": "**/*.rs",
        "include_directories": true,
        "max_results": 50,
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn fs_glob_denies_before_reading_directory() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-glob-deny-before-read");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .glob_paths(".", "**/*.rs", false, 10)
        .await
        .expect_err("policy denial should happen before directory read");

    assert!(matches!(error, FsAccessError::Authorization(_)));

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_content_search_returns_match_metadata_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-content-search");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("src")).expect("create source dir");
    fs::write(
        workspace_root.join("src/main.rs"),
        "fn main() {\n    println!(\"hello world\");\n}\n",
    )
    .expect("write main");
    fs::write(workspace_root.join("notes.txt"), "hello from notes").expect("write notes");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .search_content(
            ".",
            "hello world",
            FsContentSearchOptions {
                glob: Some("src/**/*.rs".to_owned()),
                max_results: 5,
                max_bytes_per_file: 262_144,
                case_sensitive: false,
            },
        )
        .await
        .expect("content search should execute after policy grants");

    let first = output.matches.first().expect("first match");
    assert_eq!(output.matches.len(), 1);
    assert_eq!(first.relative_path, "src/main.rs");
    assert_eq!(first.line, 2);
    assert_eq!(first.column, 15);
    assert_eq!(first.match_text, "hello world");
    assert_eq!(first.snippet, "println!(\"hello world\");");
    assert!(!first.truncated_file);

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_content_search_action_uses_granted_path_root() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let path = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::resolve("src", ctx.fs_resolution_root())
                .expect("path resolution should prepare action"),
        )
        .await
        .expect("policy should grant path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted path resolution should run");
    let action = FsContentSearchAction::new(
        path,
        "needle",
        FsContentSearchOptions {
            glob: Some("**/*.rs".to_owned()),
            max_results: 20,
            max_bytes_per_file: 1024,
            case_sensitive: true,
        },
    );
    let metadata = action.metadata();

    assert_eq!(metadata.kind, "fs.content_search");
    assert_eq!(metadata.operation, "search_content");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemRead]
    );
    let expected_payload = serde_json::json!({
        "root": "/workspace/src",
        "query": "needle",
        "glob": "**/*.rs",
        "max_results": 20,
        "max_bytes_per_file": 1024,
        "case_sensitive": true,
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn fs_content_search_denies_before_reading_directory() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-content-deny-before-read");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .search_content(
            ".",
            "needle",
            FsContentSearchOptions {
                glob: None,
                max_results: 10,
                max_bytes_per_file: 262_144,
                case_sensitive: false,
            },
        )
        .await
        .expect_err("policy denial should happen before directory read");

    assert!(matches!(error, FsAccessError::Authorization(_)));

    fs::remove_dir_all(base).ok();
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
