use super::*;

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
    let source = grant_target_path(&kernel, &ctx, "source.txt").await;
    let destination = grant_target_path(&kernel, &ctx, "backup/source.txt").await;
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
    let source = grant_target_path(&kernel, &ctx, "source.txt").await;
    let destination = grant_target_path(&kernel, &ctx, "backup/source.txt").await;
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
