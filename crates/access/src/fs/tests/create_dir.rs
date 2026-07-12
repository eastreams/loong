use super::*;

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
    let path = grant_target_path(&kernel, &ctx, "state").await;
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
    let path = grant_target_path(&kernel, &ctx, "state").await;
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
