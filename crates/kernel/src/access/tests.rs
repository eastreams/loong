use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use loong_contracts::Capability;
use loong_core::policy::context::{ContextFactory, FsAccessContext, PolicyContext};

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

    assert_access_exported::<AccessCx<'static, AccessCxContextFactory>>();
}

#[tokio::test]
async fn access_context_preserves_workspace_policy_context_for_fs_access() {
    let kernel = crate::Kernel::<AccessCxContextFactory>::new_without_audit();
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
