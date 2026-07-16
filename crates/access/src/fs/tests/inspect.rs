use super::*;

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
    let path = grant_target_path(&kernel, &ctx, "notes.md").await;
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
