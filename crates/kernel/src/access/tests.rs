use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use loong_contracts::Capability;
use loong_core::{
    AuthorizationError, PolicyGrantError,
    policy::context::{CapabilityContext, ContextFactory},
};

use super::AccessCx;
use crate::access::fs::{FsAccessError, FsPathPolicyContext, FsResolutionContext};
use crate::policy::{
    FsContentSearchAllowPolicy, FsGlobAllowPolicy, FsReadAllowPolicy, FsReadDirAllowPolicy,
    FsRemoveDirAllAllowPolicy, FsRemoveDirAllAllowedRootsPolicy, FsRemoveFileAllowPolicy,
    FsRemoveFileAllowedRootsPolicy, FsResolvePathAllowedRootsPolicy,
};

#[derive(Debug, Clone)]
struct AccessCxPolicyContext {
    resolution_root: PathBuf,
    allowed_roots: Vec<PathBuf>,
    capabilities: BTreeSet<Capability>,
}

impl AccessCxPolicyContext {
    fn new(workspace_root: impl Into<PathBuf>) -> Self {
        let workspace_root = workspace_root.into();
        let policy_root =
            std::fs::canonicalize(&workspace_root).unwrap_or_else(|_| workspace_root.clone());
        Self {
            resolution_root: workspace_root,
            allowed_roots: vec![policy_root],
            capabilities: BTreeSet::from([Capability::FilesystemRead, Capability::FilesystemWrite]),
        }
    }
}

impl CapabilityContext for AccessCxPolicyContext {
    fn allowed_capabilities(&self) -> BTreeSet<Capability> {
        self.capabilities.clone()
    }
}

impl FsResolutionContext for AccessCxPolicyContext {
    fn fs_resolution_root(&self) -> &Path {
        &self.resolution_root
    }
}

impl FsPathPolicyContext for AccessCxPolicyContext {
    fn fs_allowed_roots(&self) -> &[PathBuf] {
        &self.allowed_roots
    }
}

struct AccessCxContextFactory;

impl ContextFactory for AccessCxContextFactory {
    type Cx<'a> = AccessCxPolicyContext;
}

#[test]
fn loong_kernel_exposes_access_types_and_fs_surface_for_workspace_kernels() {
    fn assert_access_exported<T>() {}

    assert_access_exported::<AccessCx<'static, 'static, AccessCxContextFactory>>();
}

struct AccessToolCx<'a> {
    kernel: &'a crate::Kernel<AccessCxContextFactory>,
    ctx: AccessCxPolicyContext,
}

impl<'a> AccessToolCx<'a> {
    fn new(
        kernel: &'a crate::Kernel<AccessCxContextFactory>,
        workspace_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            kernel,
            ctx: AccessCxPolicyContext::new(workspace_root),
        }
    }

    fn access(&self) -> AccessCx<'_, '_, AccessCxContextFactory> {
        AccessCx::new(self.kernel, &self.ctx)
    }
}

#[tokio::test]
async fn access_context_preserves_workspace_context_for_fs_access() {
    let kernel = kernel_with_fs_path_policy();
    let base = tempfile_dir("loong-kernel-access-context");
    let workspace_root = base.join("workspace");
    std::fs::create_dir_all(workspace_root.join("notes")).expect("create notes dir");
    std::fs::write(workspace_root.join("notes/todo.md"), "hello").expect("write note");
    let ctx = AccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
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

#[tokio::test]
async fn access_context_grants_glob_paths_with_explicit_fs_policy() {
    let kernel = kernel_with_fs_path_policy();
    let base = tempfile_dir("loong-kernel-access-glob-policy");
    let workspace_root = base.join("workspace");
    std::fs::create_dir_all(workspace_root.join("src")).expect("create source dir");
    std::fs::write(workspace_root.join("src/lib.rs"), "pub fn lib() {}").expect("write lib");
    let ctx = AccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .glob_paths(".", "**/*.rs", false, 10)
        .await
        .expect("glob should succeed with explicit fs policy");

    assert_eq!(
        output.matches.first().expect("first match").relative_path,
        "src/lib.rs"
    );

    std::fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn access_context_grants_content_search_with_explicit_fs_policy() {
    let kernel = kernel_with_fs_path_policy();
    let base = tempfile_dir("loong-kernel-access-content-policy");
    let workspace_root = base.join("workspace");
    std::fs::create_dir_all(workspace_root.join("src")).expect("create source dir");
    std::fs::write(workspace_root.join("src/lib.rs"), "pub fn lib() {}\n").expect("write lib");
    let ctx = AccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .search_content(
            ".",
            "lib",
            loong_access::fs::FsContentSearchOptions {
                glob: Some("**/*.rs".to_owned()),
                max_results: 10,
                max_bytes_per_file: 262_144,
                case_sensitive: false,
            },
        )
        .await
        .expect("content search should succeed with explicit fs policy");

    assert_eq!(
        output.matches.first().expect("first match").relative_path,
        "src/lib.rs"
    );

    std::fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_path_escape_is_reported_as_path_resolution_policy_denial() {
    let kernel = kernel_with_fs_path_policy();
    let base = tempfile_dir("loong-kernel-access-path-policy");
    let workspace_root = base.join("workspace");
    let outside_root = base.join("outside");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::create_dir_all(&outside_root).expect("create outside root");
    std::fs::write(outside_root.join("secret.txt"), "secret").expect("write outside file");
    let ctx = AccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .read_file("../outside/secret.txt")
        .await
        .expect_err("path escape should be denied by policy");

    let FsAccessError::Authorization(AuthorizationError::PolicyGrant(PolicyGrantError::Denied {
        report,
        reason,
    })) = error
    else {
        panic!("expected path policy denial, got {error:?}");
    };

    assert!(
        reason.contains("escapes allowed filesystem roots"),
        "unexpected denial reason: {reason}"
    );
    assert!(
        report.evaluations.iter().any(|evaluation| {
            evaluation.policy_stage == "action"
                && evaluation.source.policy_name == "fs-resolve-path-allowed-roots"
        }),
        "expected fs resolve path policy evaluation in report: {report:?}"
    );

    std::fs::remove_dir_all(base).ok();
}

#[cfg(unix)]
#[tokio::test]
async fn fs_remove_file_denies_ancestor_symlink_escape() {
    let kernel = kernel_with_fs_path_policy();
    let base = tempfile_dir("loong-kernel-access-remove-symlink-policy");
    let workspace_root = base.join("workspace");
    let outside_root = base.join("outside");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::create_dir_all(&outside_root).expect("create outside root");
    let outside_file = outside_root.join("secret.txt");
    std::fs::write(&outside_file, "secret").expect("write outside file");
    create_symlink(&outside_root, &workspace_root.join("outside-link")).expect("create symlink");
    let ctx = AccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .remove_file("outside-link/secret.txt")
        .await
        .expect_err("ancestor symlink escape should be denied by policy");

    let FsAccessError::Authorization(AuthorizationError::PolicyGrant(PolicyGrantError::Denied {
        report,
        reason,
    })) = error
    else {
        panic!("expected remove path policy denial, got {error:?}");
    };

    assert!(
        reason.contains("escapes allowed filesystem roots"),
        "unexpected denial reason: {reason}"
    );
    assert!(
        report.evaluations.iter().any(|evaluation| {
            evaluation.policy_stage == "action"
                && evaluation.source.policy_name == "fs-remove-file-allowed-roots"
        }),
        "expected fs remove path policy evaluation in report: {report:?}"
    );
    assert_eq!(
        std::fs::read_to_string(outside_file).expect("read outside file"),
        "secret"
    );

    std::fs::remove_dir_all(base).ok();
}

#[cfg(unix)]
#[tokio::test]
async fn fs_remove_dir_all_denies_ancestor_symlink_escape() {
    let kernel = kernel_with_fs_path_policy();
    let base = tempfile_dir("loong-kernel-access-remove-dir-symlink-policy");
    let workspace_root = base.join("workspace");
    let outside_root = base.join("outside");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::create_dir_all(outside_root.join("tree")).expect("create outside tree");
    let outside_file = outside_root.join("tree/secret.txt");
    std::fs::write(&outside_file, "secret").expect("write outside file");
    create_symlink(&outside_root, &workspace_root.join("outside-link")).expect("create symlink");
    let ctx = AccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .remove_dir_all("outside-link/tree")
        .await
        .expect_err("ancestor symlink escape should be denied by recursive removal policy");

    let FsAccessError::Authorization(AuthorizationError::PolicyGrant(PolicyGrantError::Denied {
        report,
        reason,
    })) = error
    else {
        panic!("expected remove-dir path policy denial, got {error:?}");
    };

    assert!(
        reason.contains("escapes allowed filesystem roots"),
        "unexpected denial reason: {reason}"
    );
    assert!(
        report.evaluations.iter().any(|evaluation| {
            evaluation.policy_stage == "action"
                && evaluation.source.policy_name == "fs-remove-dir-all-allowed-roots"
        }),
        "expected fs recursive remove path policy evaluation in report: {report:?}"
    );
    assert_eq!(
        std::fs::read_to_string(outside_file).expect("read outside file"),
        "secret"
    );

    std::fs::remove_dir_all(base).ok();
}

fn kernel_with_fs_path_policy() -> crate::Kernel<AccessCxContextFactory> {
    let policy = crate::PolicyPipeline::<AccessCxContextFactory>::new()
        .with_policy(FsResolvePathAllowedRootsPolicy)
        .with_policy(FsRemoveFileAllowedRootsPolicy)
        .with_policy(FsRemoveDirAllAllowedRootsPolicy)
        .with_policy(FsReadAllowPolicy)
        .with_policy(FsRemoveFileAllowPolicy)
        .with_policy(FsRemoveDirAllAllowPolicy)
        .with_policy(FsGlobAllowPolicy)
        .with_policy(FsReadDirAllowPolicy)
        .with_policy(FsContentSearchAllowPolicy);
    crate::Kernel::with_policy_runtime(
        policy,
        Arc::new(crate::SystemClock),
        Arc::new(crate::NoopAuditSink),
    )
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

fn tempfile_dir(prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}
