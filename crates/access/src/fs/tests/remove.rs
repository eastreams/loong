use super::*;

#[tokio::test]
async fn fs_remove_file_removes_file_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-remove-file");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::write(workspace_root.join("target.txt"), "hello").expect("write target");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .remove_file("target.txt")
        .await
        .expect("remove should execute after policy grants");

    let expected_path = dunce::canonicalize(&workspace_root)
        .expect("canonical workspace root")
        .join("target.txt");
    assert_eq!(output.path, expected_path);
    assert!(output.removed);
    assert_eq!(output.kind, Some(FsRemoveFileKind::File));
    assert!(!workspace_root.join("target.txt").exists());

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_remove_file_missing_path_is_noop_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-remove-file-missing");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .remove_file("missing.txt")
        .await
        .expect("missing remove should still be governed");

    assert!(!output.removed);
    assert_eq!(output.kind, None);

    fs::remove_dir_all(base).ok();
}

#[cfg(unix)]
#[tokio::test]
async fn fs_remove_file_removes_final_symlink_without_deleting_target() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-remove-final-symlink");
    let workspace_root = base.join("workspace");
    let outside_root = base.join("outside");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::create_dir_all(&outside_root).expect("create outside root");
    let outside_file = outside_root.join("secret.txt");
    fs::write(&outside_file, "secret").expect("write outside file");
    create_symlink(&outside_file, &workspace_root.join("secret-link")).expect("create symlink");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .remove_file("secret-link")
        .await
        .expect("final symlink remove should delete the link itself");

    assert!(output.removed);
    assert_eq!(output.kind, Some(FsRemoveFileKind::Symlink));
    assert!(!workspace_root.join("secret-link").exists());
    assert_eq!(
        fs::read_to_string(outside_file).expect("read outside target"),
        "secret"
    );

    fs::remove_dir_all(base).ok();
}

#[test]
fn fs_remove_file_action_uses_granted_entry_path() {
    let action = FsRemoveFileAction::new(GrantedEntryPath::new(PathBuf::from(
        "/workspace/logs/old.txt",
    )));
    let metadata = action.metadata();

    assert_eq!(action.path(), Path::new("/workspace/logs/old.txt"));
    assert_eq!(metadata.kind, "fs.remove_file");
    assert_eq!(metadata.operation, "remove_file");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemWrite]
    );
    let expected_payload = serde_json::json!({
        "path": "/workspace/logs/old.txt",
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);

    let action = FsAction::remove_file(action);
    assert_eq!(action.metadata().kind, "fs.remove_file");
}

#[tokio::test]
async fn fs_remove_file_denies_before_removing_file() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-remove-file-deny");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::write(workspace_root.join("target.txt"), "hello").expect("write target");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .remove_file("target.txt")
        .await
        .expect_err("policy denial should happen before file removal");

    assert!(matches!(error, FsAccessError::Authorization(_)));
    assert!(workspace_root.join("target.txt").exists());

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_remove_dir_all_removes_directory_tree_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-remove-dir-all");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("managed/demo/nested")).expect("create managed dir");
    fs::write(workspace_root.join("managed/demo/nested/file.txt"), "hello")
        .expect("write nested file");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .remove_dir_all("managed/demo")
        .await
        .expect("directory removal should execute after policy grants");

    assert!(output.removed);
    assert!(!workspace_root.join("managed/demo").exists());
    assert!(workspace_root.join("managed").exists());

    fs::remove_dir_all(base).ok();
}

#[test]
fn fs_remove_dir_all_action_uses_granted_entry_path() {
    let action = FsRemoveDirAllAction::new(GrantedEntryPath::new(PathBuf::from(
        "/workspace/managed/demo",
    )));
    let metadata = action.metadata();

    assert_eq!(action.path(), Path::new("/workspace/managed/demo"));
    assert_eq!(metadata.kind, "fs.remove_dir_all");
    assert_eq!(metadata.operation, "remove_dir_all");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemWrite]
    );
    let expected_payload = serde_json::json!({
        "path": "/workspace/managed/demo",
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);

    let action = FsAction::remove_dir_all(action);
    assert_eq!(action.metadata().kind, "fs.remove_dir_all");
}

#[tokio::test]
async fn fs_remove_dir_all_missing_path_is_noop_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-remove-dir-all-missing");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .remove_dir_all("managed/missing")
        .await
        .expect("missing directory removal should still be governed");

    assert!(!output.removed);

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_remove_dir_all_rejects_file_path() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-remove-dir-all-file");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::write(workspace_root.join("managed-file"), "hello").expect("write file");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .remove_dir_all("managed-file")
        .await
        .expect_err("file path should not be recursively removed");

    assert!(matches!(error, FsAccessError::PathIsNotDirectory { .. }));
    assert_eq!(
        fs::read_to_string(workspace_root.join("managed-file")).expect("read file"),
        "hello"
    );

    fs::remove_dir_all(base).ok();
}

#[cfg(unix)]
#[tokio::test]
async fn fs_remove_dir_all_rejects_final_symlink() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-remove-dir-all-symlink");
    let workspace_root = base.join("workspace");
    let outside_root = base.join("outside");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::create_dir_all(outside_root.join("target")).expect("create outside target");
    fs::write(outside_root.join("target/secret.txt"), "secret").expect("write outside file");
    create_symlink(
        &outside_root.join("target"),
        &workspace_root.join("managed-link"),
    )
    .expect("create final symlink");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .remove_dir_all("managed-link")
        .await
        .expect_err("recursive removal should refuse final symlink");

    assert!(matches!(error, FsAccessError::RefuseSymlink { .. }));
    assert!(workspace_root.join("managed-link").is_symlink());
    assert_eq!(
        fs::read_to_string(outside_root.join("target/secret.txt")).expect("read outside target"),
        "secret"
    );

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_remove_dir_all_denies_before_side_effect() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-remove-dir-all-deny");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("managed/demo")).expect("create managed dir");
    fs::write(workspace_root.join("managed/demo/SKILL.md"), "hello").expect("write file");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .remove_dir_all("managed/demo")
        .await
        .expect_err("policy denial should happen before recursive removal");

    assert!(matches!(error, FsAccessError::Authorization(_)));
    assert!(workspace_root.join("managed/demo/SKILL.md").exists());

    fs::remove_dir_all(base).ok();
}
