use super::*;

#[tokio::test]
async fn fs_action_wraps_read_action() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let path = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::resolve("notes.md", ctx.fs_resolution_root())
                .expect("path resolution should prepare action"),
        )
        .await
        .expect("policy should grant path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted path resolution should run");
    let action = FsAction::read_file(path);
    let metadata = action.metadata();

    assert_eq!(metadata.kind, "fs.read");
    assert_eq!(metadata.operation, "read_file");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemRead]
    );
    let expected_payload = serde_json::json!({"path": "/workspace/notes.md"});
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn tool_context_like_chain_grants_read_file_via_access_then_fs() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-read-output");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("notes")).expect("create notes dir");
    fs::write(workspace_root.join("notes/todo.md"), "hello").expect("write note");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .read_file("notes/todo.md")
        .await
        .expect("grant should succeed");

    let expected_path =
        dunce::canonicalize(workspace_root.join("notes/todo.md")).expect("canonical note path");
    assert_eq!(output.path, expected_path);
    assert_eq!(output.bytes, b"hello");

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_read_execution_boundary_consumes_granted_action() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-granted-boundary");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("notes")).expect("create notes dir");
    fs::write(workspace_root.join("notes/todo.md"), "hello").expect("write note");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let path = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::resolve("notes/todo.md", ctx.fs_resolution_root())
                .expect("path resolution should prepare action"),
        )
        .await
        .expect("policy should grant path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted path resolution should run");
    let action = FsReadAction::new(path);
    let grant = kernel
        .policy_engine()
        .grant(&ctx, action)
        .await
        .expect("policy should grant read");

    let output = grant
        .granted
        .run(&ctx)
        .await
        .expect("granted read should execute");

    let expected_path =
        dunce::canonicalize(workspace_root.join("notes/todo.md")).expect("canonical note path");
    assert_eq!(output.path, expected_path);
    assert_eq!(output.bytes, b"hello");

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_access_denies_before_reading_file() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-deny-before-read");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .read_file("missing.txt")
        .await
        .expect_err("policy denial should happen before file read");

    assert!(matches!(error, FsAccessError::Authorization(_)));

    fs::remove_dir_all(base).ok();
}
