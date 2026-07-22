use std::fs;

use crate::fs::{
    path::FsPathError,
    test_support::{
        FsAccessTestContext, FsAccessTestKernel, FsAccessToolCx, grant_target_path, unique_temp_dir,
    },
};

use super::*;

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
    let ctx = FsAccessTestContext::new(&workspace_root);
    let path = grant_target_path(&kernel, &ctx, "src").await;
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

    assert!(matches!(
        error,
        FsGlobError::Path(FsPathError::Authorization(_))
    ));

    fs::remove_dir_all(base).ok();
}
