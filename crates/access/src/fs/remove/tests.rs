use std::fs;

use crate::fs::{
    path::FsPathError,
    test_support::{FsAccessTestKernel, FsAccessToolCx, create_symlink, unique_temp_dir},
};

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

    assert!(matches!(
        error,
        FsRemoveFileError::Path(FsPathError::Authorization(_))
    ));
    assert!(workspace_root.join("target.txt").exists());

    fs::remove_dir_all(base).ok();
}
