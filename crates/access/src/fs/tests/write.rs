use super::*;

#[tokio::test]
async fn fs_write_action_writes_file_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-write");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .write_file(
            "notes/todo.md",
            b"hello".to_vec(),
            FsWriteOptions {
                create_dirs: true,
                overwrite: false,
            },
        )
        .await
        .expect("write should execute after policy grants");

    assert_eq!(output.bytes_written, 5);
    assert!(!output.overwritten);
    assert_eq!(
        fs::read_to_string(workspace_root.join("notes/todo.md")).expect("read written file"),
        "hello"
    );

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_write_action_uses_granted_path() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let path = grant_target_path(&kernel, &ctx, "notes.md").await;
    let action = FsWriteAction::new(
        path,
        b"hello".to_vec(),
        FsWriteOptions {
            create_dirs: true,
            overwrite: false,
        },
    );
    let metadata = action.metadata();

    assert_eq!(action.path(), Path::new("/workspace/notes.md"));
    assert_eq!(metadata.kind, "fs.write");
    assert_eq!(metadata.operation, "write_file");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemWrite]
    );
    let expected_payload = serde_json::json!({
        "path": "/workspace/notes.md",
        "byte_count": 5,
        "create_dirs": true,
        "overwrite": false,
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn fs_atomic_write_file_writes_file_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-atomic-write");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::write(workspace_root.join("manifest.json"), b"{\"old\":true}")
        .expect("write existing manifest");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .write_file_atomically(
            "manifest.json",
            b"{\"new\":true}".to_vec(),
            FsWriteOptions {
                create_dirs: false,
                overwrite: true,
            },
        )
        .await
        .expect("atomic write should execute after policy grants");

    assert_eq!(output.bytes_written, 12);
    assert!(output.overwritten);
    assert_eq!(
        fs::read_to_string(workspace_root.join("manifest.json"))
            .expect("read atomically written file"),
        "{\"new\":true}"
    );

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_atomic_write_file_denies_before_replacing_existing_file() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-atomic-write-denied");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::write(workspace_root.join("manifest.json"), "old").expect("write existing manifest");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    ctx.access()
        .fs()
        .write_file_atomically(
            "manifest.json",
            b"new".to_vec(),
            FsWriteOptions {
                create_dirs: false,
                overwrite: true,
            },
        )
        .await
        .expect_err("policy denial should stop before atomic replacement");

    assert_eq!(
        fs::read_to_string(workspace_root.join("manifest.json")).expect("read preserved manifest"),
        "old"
    );

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_atomic_write_action_uses_granted_path() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let path = grant_target_path(&kernel, &ctx, "manifest.json").await;
    let action = FsAtomicWriteAction::new(
        path,
        b"hello".to_vec(),
        FsWriteOptions {
            create_dirs: true,
            overwrite: false,
        },
    );
    let metadata = action.metadata();

    assert_eq!(action.path(), Path::new("/workspace/manifest.json"));
    assert_eq!(metadata.kind, "fs.atomic_write");
    assert_eq!(metadata.operation, "write_file_atomically");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemWrite]
    );
    let expected_payload = serde_json::json!({
        "path": "/workspace/manifest.json",
        "byte_count": 5,
        "create_dirs": true,
        "overwrite": false,
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn fs_write_denies_before_creating_file() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-write-deny");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .write_file(
            "notes/todo.md",
            b"hello".to_vec(),
            FsWriteOptions {
                create_dirs: true,
                overwrite: false,
            },
        )
        .await
        .expect_err("policy denial should happen before file creation");

    assert!(matches!(error, FsAccessError::Authorization(_)));
    assert!(!workspace_root.join("notes/todo.md").exists());

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_write_rejects_existing_file_without_overwrite() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-write-overwrite");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let target = workspace_root.join("notes.md");
    fs::write(&target, "old").expect("seed existing file");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .write_file(
            "notes.md",
            b"new".to_vec(),
            FsWriteOptions {
                create_dirs: false,
                overwrite: false,
            },
        )
        .await
        .expect_err("existing file should require overwrite");

    assert!(matches!(
        error,
        FsAccessError::FileExistsRequiresOverwrite { .. }
    ));
    assert_eq!(
        fs::read_to_string(target).expect("read original file"),
        "old"
    );

    fs::remove_dir_all(base).ok();
}
