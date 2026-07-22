use std::fs;

use crate::fs::{
    path::FsPathError,
    test_support::{
        FsAccessTestContext, FsAccessTestKernel, FsAccessToolCx, grant_target_path, unique_temp_dir,
    },
};

use super::*;

#[tokio::test]
async fn fs_read_dir_lists_immediate_entries_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-read-dir");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("src/nested")).expect("create source dirs");
    fs::write(workspace_root.join("README.md"), "readme").expect("write readme");
    fs::write(workspace_root.join("src/main.rs"), "fn main() {}").expect("write main");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .read_dir(".", 10)
        .await
        .expect("read_dir should execute after policy grants");

    assert_eq!(
        output
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.kind))
            .collect::<Vec<_>>(),
        vec![
            ("README.md", FsPathKind::File),
            ("src", FsPathKind::Directory)
        ]
    );
    assert!(!output.truncated);

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_read_dir_truncates_at_max_entries() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-read-dir-truncated");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::write(workspace_root.join("a.txt"), "a").expect("write a");
    fs::write(workspace_root.join("b.txt"), "b").expect("write b");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .read_dir(".", 1)
        .await
        .expect("read_dir should execute after policy grants");

    assert_eq!(output.entries.len(), 1);
    assert!(output.truncated);

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_read_dir_action_uses_granted_path_root() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessTestContext::new(&workspace_root);
    let path = grant_target_path(&kernel, &ctx, "src").await;
    let action = FsReadDirAction::new(path, 25);
    let metadata = action.metadata();

    assert_eq!(metadata.kind, "fs.read_dir");
    assert_eq!(metadata.operation, "read_dir");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemRead]
    );
    let expected_payload = serde_json::json!({
        "root": "/workspace/src",
        "max_entries": 25,
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn fs_read_dir_denies_before_reading_directory() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-read-dir-deny-before-read");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .read_dir(".", 10)
        .await
        .expect_err("policy denial should happen before directory read");

    assert!(matches!(
        error,
        FsReadDirError::Path(FsPathError::Authorization(_))
    ));

    fs::remove_dir_all(base).ok();
}
