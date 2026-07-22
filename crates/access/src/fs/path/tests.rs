use std::fs;

use loong_core::{kernel::Kernel, policy::policy::Policy};

use crate::fs::test_support::{
    FsAccessTestContext, FsAccessTestContextFactory, FsAccessTestKernel, create_symlink,
    unique_temp_dir,
};
use crate::fs::{read::FsReadAction, remove::FsRemoveFileAction};

use super::*;

#[test]
fn lexical_normalization_preserves_relative_parent_segments() {
    let normalized = normalize_path_lexically(Path::new("../../workspace/./src/../README.md"));

    assert_eq!(normalized, PathBuf::from("../../workspace/README.md"));
}

#[tokio::test]
async fn fs_resolve_path_action_runs_relative_target_resolution_after_grant() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessTestContext::new(&workspace_root);
    let resolve = FsResolvePathAction::target("docs/../notes/todo.md", &ctx);
    let metadata = resolve.metadata();

    assert_eq!(metadata.kind, "fs.resolve_path");
    assert_eq!(metadata.operation, "resolve_target_path");
    assert!(metadata.required_capabilities.is_empty());
    assert_eq!(
        resolve.payload().as_ref(),
        &serde_json::json!({
            "path": "docs/../notes/todo.md",
            "resolution_root": "/workspace",
            "allowed_roots": ["/workspace"],
            "authority_ceiling_roots": ["/workspace"],
            "semantics": "target",
        })
    );

    let resolved = kernel
        .policy_engine()
        .grant(&ctx, resolve)
        .await
        .expect("policy should grant path resolution")
        .into_granted()
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

    let ctx = FsAccessTestContext::new(&workspace_root);
    let link = workspace_root.join("late-link");
    let resolve = FsResolvePathAction::target("late-link", &ctx);
    create_symlink(&outside_root, &link).expect("create symlink after action construction");

    let resolved = kernel
        .policy_engine()
        .grant(&ctx, resolve)
        .await
        .expect("policy should grant path resolution")
        .into_granted()
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
    let ctx = FsAccessTestContext::new(&workspace_root);

    let error = FsAccess::new(kernel.policy_engine(), &ctx)
        .read_file("")
        .await
        .expect_err("policy denial must happen before empty-path resolution runs");

    assert!(matches!(
        error,
        crate::fs::FsReadError::Path(FsPathError::Authorization(_))
    ));
}

#[tokio::test]
async fn fs_path_action_outputs_granted_target_path_for_read_action() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessTestContext::new(&workspace_root);
    let resolved = kernel
        .policy_engine()
        .grant(
            &ctx,
            FsResolvePathAction::target("docs/../notes/todo.md", &ctx),
        )
        .await
        .expect("policy should grant path resolution")
        .into_granted()
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
            "resolved_allowed_roots": ["/workspace"],
            "resolved_authority_ceiling_roots": ["/workspace"],
            "semantics": "target",
        })
    );

    let path = kernel
        .policy_engine()
        .grant(&ctx, path_action)
        .await
        .expect("policy should grant resolved path")
        .into_granted()
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
    let ctx = FsAccessTestContext::new(&workspace_root);
    let resolved = kernel
        .policy_engine()
        .grant(&ctx, FsResolvePathAction::entry("logs/old.txt", &ctx))
        .await
        .expect("policy should grant entry resolution")
        .into_granted()
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
            "resolved_allowed_roots": ["/workspace"],
            "resolved_authority_ceiling_roots": ["/workspace"],
            "semantics": "entry",
        })
    );

    let path = kernel
        .policy_engine()
        .grant(&ctx, path_action)
        .await
        .expect("policy should grant entry path")
        .into_granted()
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
    let ctx = FsAccessTestContext::new(&workspace_root);
    let resolved = kernel
        .policy_engine()
        .grant(&ctx, FsResolvePathAction::target("../secrets.txt", &ctx))
        .await
        .expect("policy should grant path resolution")
        .into_granted()
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

    let ctx = FsAccessTestContext::new(&workspace_root);
    let resolved = kernel
        .policy_engine()
        .grant(&ctx, FsResolvePathAction::target("secret-link", &ctx))
        .await
        .expect("policy should grant path resolution")
        .into_granted()
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
    let ctx = FsAccessTestContext::new(&allowed_root);
    let resolved = kernel
        .policy_engine()
        .grant(&ctx, FsResolvePathAction::target("notes.txt", &ctx))
        .await
        .expect("policy should grant path resolution")
        .into_granted()
        .run(&ctx)
        .await
        .expect("missing root under symlink ancestor should resolve");

    let expected_root = dunce::canonicalize(&outside_root).expect("canonical outside root");
    assert_eq!(
        resolved.path(),
        expected_root.join("missing-root/notes.txt")
    );
    assert_eq!(
        resolved.allowed_roots(),
        [expected_root.join("missing-root")]
    );
    let granted_path = kernel
        .policy_engine()
        .grant(&ctx, FsPathAction::new(resolved))
        .await
        .expect("path policy should compare resolved roots and path in one path space")
        .into_granted()
        .run(&ctx)
        .await
        .expect("resolved lexical root should authorize its child");
    assert_eq!(
        granted_path.as_path(),
        expected_root.join("missing-root/notes.txt")
    );
    fs::remove_dir_all(base).ok();
}

#[cfg(unix)]
#[tokio::test]
async fn fs_path_policy_rejects_child_root_that_resolves_outside_parent_authority() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-child-root-symlink-escape");
    let parent_root = base.join("parent");
    let outside_root = base.join("outside");
    fs::create_dir_all(&parent_root).expect("create parent root");
    fs::create_dir_all(&outside_root).expect("create outside root");
    let link = parent_root.join("linked-child-parent");
    create_symlink(&outside_root, &link).expect("create child root symlink");
    let child_root = link.join("child");
    let ctx = FsAccessTestContext::with_authority_ceiling(&child_root, &parent_root);
    let resolved = kernel
        .policy_engine()
        .grant(&ctx, FsResolvePathAction::target("notes.txt", &ctx))
        .await
        .expect("resolution policy should allow filesystem observation")
        .into_granted()
        .run(&ctx)
        .await
        .expect("resolve action should expose the symlink escape to path policy");

    let path_action = FsPathAction::new(resolved);
    let path_policy: &dyn Policy<FsAccessTestContextFactory, FsPathAction> =
        &FsPathAllowedRootsPolicy::target();
    let policy_grant = path_policy.grant(&ctx, &path_action).await;

    assert_eq!(policy_grant.decision, PolicyDecision::Deny);
    assert!(
        policy_grant
            .reason
            .contains("filesystem root narrowing escapes")
    );
    fs::remove_dir_all(base).ok();
}
