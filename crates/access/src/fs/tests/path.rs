use super::*;

#[test]
fn fs_resolve_path_action_resolves_relative_path_inside_workspace() {
    let workspace_root = PathBuf::from("/workspace");
    let action = FsResolvePathAction::resolve("docs/../notes/todo.md", &workspace_root)
        .expect("path inside workspace should normalize");

    assert_eq!(
        action.resolved_path(),
        Path::new("/workspace/notes/todo.md")
    );
}

#[tokio::test]
async fn fs_resolve_path_action_outputs_granted_path_for_read_action() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let resolve_action =
        FsResolvePathAction::resolve("docs/../notes/todo.md", ctx.fs_resolution_root())
            .expect("path resolution should prepare action");
    let resolve_metadata = resolve_action.metadata();

    assert_eq!(resolve_metadata.kind, "fs.resolve_path");
    assert_eq!(resolve_metadata.operation, "resolve_path");
    assert!(resolve_metadata.required_capabilities.is_empty());
    let expected_resolve_payload = serde_json::json!({
        "path": "docs/../notes/todo.md",
        "resolved_path": "/workspace/notes/todo.md",
    });
    assert_eq!(resolve_action.payload().as_ref(), &expected_resolve_payload);

    let path = kernel
        .policy_engine()
        .grant(&ctx, resolve_action)
        .await
        .expect("policy should grant path resolution")
        .granted
        .run(&ctx)
        .await
        .expect("granted path resolution should run");
    let action = FsReadAction::new(path);
    let metadata = action.metadata();

    assert_eq!(action.path(), Path::new("/workspace/notes/todo.md"));
    assert_eq!(metadata.operation, "read_file");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemRead]
    );
    let expected_payload = serde_json::json!({"path": "/workspace/notes/todo.md"});
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[test]
fn fs_resolve_path_action_marks_workspace_escape_for_policy() {
    let workspace_root = PathBuf::from("/workspace");
    let action = FsResolvePathAction::resolve("../secrets.txt", &workspace_root)
        .expect("path resolution should prepare escaped action for policy");

    assert_eq!(action.resolved_path(), Path::new("/secrets.txt"));
}

#[cfg(unix)]
#[test]
fn fs_resolve_path_action_marks_symlink_escape_for_policy() {
    let base = unique_temp_dir("loong-access-fs-read");
    let workspace_root = base.join("workspace");
    let outside_root = base.join("outside");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::create_dir_all(&outside_root).expect("create outside root");

    let outside_file = outside_root.join("secret.txt");
    fs::write(&outside_file, "secret").expect("write outside file");

    let symlink_path = workspace_root.join("secret-link");
    create_symlink(&outside_file, &symlink_path).expect("create symlink");

    let action = FsResolvePathAction::resolve("secret-link", &workspace_root)
        .expect("path resolution should prepare symlink escape for policy");

    assert_eq!(
        action.resolved_path(),
        dunce::canonicalize(&outside_file).expect("canonical outside file")
    );
}

#[cfg(unix)]
#[test]
fn fs_resolve_path_action_resolves_missing_allowed_root_through_symlink_ancestor() {
    let base = unique_temp_dir("loong-access-fs-missing-root-symlink");
    let workspace_root = base.join("workspace");
    let outside_root = base.join("outside");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::create_dir_all(&outside_root).expect("create outside root");

    let link_path = workspace_root.join("linked-root-parent");
    create_symlink(&outside_root, &link_path).expect("create symlink");

    let allowed_root = link_path.join("missing-root");
    let action = FsResolvePathAction::resolve("notes.txt", &allowed_root)
        .expect("missing allowed root under symlink ancestor should resolve");

    let expected_root = dunce::canonicalize(&outside_root).expect("canonical outside root");
    assert_eq!(
        action.resolved_path(),
        expected_root.join("missing-root/notes.txt")
    );
    fs::remove_dir_all(base).ok();
}
