use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use loong_contracts::Capability;
use loong_core::{
    AuthorizationError, PolicyGrantError,
    policy::context::{ContextFactory, FsAccessContext, PolicyContext},
};

use super::AccessCx;

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
    policy_context: AccessCxPolicyContext,
}

impl<'a> AccessToolCx<'a> {
    fn new(
        kernel: &'a crate::Kernel<AccessCxContextFactory>,
        workspace_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            kernel,
            policy_context: AccessCxPolicyContext::new(workspace_root),
        }
    }

    fn access(&self) -> AccessCx<'_, '_, AccessCxContextFactory> {
        AccessCx::new(self.kernel, &self.policy_context)
    }
}

#[tokio::test]
async fn access_context_preserves_workspace_policy_context_for_fs_access() {
    let kernel = crate::Kernel::<AccessCxContextFactory>::new_without_audit();
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
async fn fs_path_escape_is_reported_as_path_resolution_policy_denial() {
    let kernel = crate::Kernel::<AccessCxContextFactory>::new_without_audit();
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

    let loong_access::fs::FsAccessError::Authorization(AuthorizationError::PolicyGrant(
        PolicyGrantError::Denied { report, reason },
    )) = error
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

fn tempfile_dir(prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}
