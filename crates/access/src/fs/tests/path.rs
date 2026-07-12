use super::*;

#[tokio::test]
async fn fs_resolve_path_action_runs_relative_target_resolution_after_grant() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let resolve = FsResolvePathAction::target("docs/../notes/todo.md", &workspace_root);
    let metadata = resolve.metadata();

    assert_eq!(metadata.kind, "fs.resolve_path");
    assert_eq!(metadata.operation, "resolve_target_path");
    assert!(metadata.required_capabilities.is_empty());
    assert_eq!(
        resolve.payload().as_ref(),
        &serde_json::json!({
            "path": "docs/../notes/todo.md",
            "resolution_root": "/workspace",
            "semantics": "target",
        })
    );

    let resolved = kernel
        .policy_engine()
        .grant(&ctx, resolve)
        .await
        .expect("policy should grant path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted resolve action should resolve the path");

    assert_eq!(
        resolved.requested_path(),
        Path::new("docs/../notes/todo.md")
    );
    assert_eq!(resolved.path(), Path::new("/workspace/notes/todo.md"));
}

#[cfg(unix)]
#[tokio::test]
async fn fs_resolve_path_action_does_not_observe_filesystem_until_run() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-resolve-run-boundary");
    let workspace_root = base.join("workspace");
    let outside_root = base.join("outside");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::create_dir_all(&outside_root).expect("create outside root");

    let link = workspace_root.join("late-link");
    let resolve = FsResolvePathAction::target("late-link", &workspace_root);
    create_symlink(&outside_root, &link).expect("create symlink after action construction");

    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let resolved = kernel
        .policy_engine()
        .grant(&ctx, resolve)
        .await
        .expect("policy should grant path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted resolve action should observe the late symlink");

    assert_eq!(
        resolved.path(),
        dunce::canonicalize(&outside_root).expect("canonical outside root")
    );
    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_resolve_path_policy_denial_prevents_resolution() {
    let kernel = FsAccessTestKernel::denying();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);

    let error = FsAccess::new(kernel.policy_engine(), &ctx)
        .read_file("")
        .await
        .expect_err("policy denial must happen before empty-path resolution runs");

    assert!(matches!(error, FsAccessError::Authorization(_)));
}

#[tokio::test]
async fn fs_path_action_outputs_granted_target_path_for_read_action() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let resolved = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::target("docs/../notes/todo.md", ctx.fs_resolution_root()),
        )
        .await
        .expect("policy should grant path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted resolve action should run");
    let path_action = FsPathAction::new(resolved);
    let path_metadata = path_action.metadata();

    assert_eq!(path_metadata.kind, "fs.path");
    assert_eq!(path_metadata.operation, "authorize_target_path");
    assert!(path_metadata.required_capabilities.is_empty());
    assert_eq!(
        path_action.payload().as_ref(),
        &serde_json::json!({
            "path": "docs/../notes/todo.md",
            "resolved_path": "/workspace/notes/todo.md",
            "semantics": "target",
        })
    );

    let path = kernel
        .policy_engine()
        .grant(&ctx, path_action)
        .await
        .expect("policy should grant resolved path")
        .granted
        .run(&ctx)
        .await
        .expect("granted path action should mint a target path");
    let action = FsReadAction::new(path);
    let metadata = action.metadata();

    assert_eq!(action.path(), Path::new("/workspace/notes/todo.md"));
    assert_eq!(metadata.operation, "read_file");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemRead]
    );
    assert_eq!(
        action.payload().as_ref(),
        &serde_json::json!({"path": "/workspace/notes/todo.md"})
    );
}

#[tokio::test]
async fn fs_path_action_outputs_granted_entry_path_for_remove_action() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let resolved = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::entry("logs/old.txt", ctx.fs_resolution_root()),
        )
        .await
        .expect("policy should grant entry resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted entry resolve action should run");
    let path_action = FsPathAction::new(resolved);

    assert_eq!(path_action.metadata().kind, "fs.path");
    assert_eq!(path_action.metadata().operation, "authorize_entry_path");
    assert_eq!(
        path_action.payload().as_ref(),
        &serde_json::json!({
            "path": "logs/old.txt",
            "resolved_path": "/workspace/logs/old.txt",
            "semantics": "entry",
        })
    );

    let path = kernel
        .policy_engine()
        .grant(&ctx, path_action)
        .await
        .expect("policy should grant entry path")
        .granted
        .run(&ctx)
        .await
        .expect("granted path action should mint an entry path");
    let action = FsRemoveFileAction::new(path);

    assert_eq!(action.path(), Path::new("/workspace/logs/old.txt"));
    assert_eq!(action.metadata().kind, "fs.remove_file");
}

#[tokio::test]
async fn fs_resolve_path_action_preserves_workspace_escape_for_path_policy() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let resolved = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::target("../secrets.txt", &workspace_root),
        )
        .await
        .expect("policy should grant path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("path resolution should retain escaped path facts");

    assert_eq!(resolved.path(), Path::new("/secrets.txt"));
}

#[cfg(unix)]
#[tokio::test]
async fn fs_resolve_path_action_preserves_symlink_escape_for_path_policy() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-read");
    let workspace_root = base.join("workspace");
    let outside_root = base.join("outside");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::create_dir_all(&outside_root).expect("create outside root");

    let outside_file = outside_root.join("secret.txt");
    fs::write(&outside_file, "secret").expect("write outside file");

    let symlink_path = workspace_root.join("secret-link");
    create_symlink(&outside_file, &symlink_path).expect("create symlink");

    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let resolved = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::target("secret-link", &workspace_root),
        )
        .await
        .expect("policy should grant path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("path resolution should prepare symlink escape for policy");

    assert_eq!(
        resolved.path(),
        dunce::canonicalize(&outside_file).expect("canonical outside file")
    );
    fs::remove_dir_all(base).ok();
}

#[cfg(unix)]
#[tokio::test]
async fn fs_resolve_path_action_resolves_missing_root_through_symlink_ancestor() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-missing-root-symlink");
    let workspace_root = base.join("workspace");
    let outside_root = base.join("outside");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::create_dir_all(&outside_root).expect("create outside root");

    let link_path = workspace_root.join("linked-root-parent");
    create_symlink(&outside_root, &link_path).expect("create symlink");

    let allowed_root = link_path.join("missing-root");
    let ctx = FsAccessPolicyContext::new(&allowed_root);
    let resolved = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::target("notes.txt", &allowed_root),
        )
        .await
        .expect("policy should grant path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("missing root under symlink ancestor should resolve");

    let expected_root = dunce::canonicalize(&outside_root).expect("canonical outside root");
    assert_eq!(
        resolved.path(),
        expected_root.join("missing-root/notes.txt")
    );
    fs::remove_dir_all(base).ok();
}
