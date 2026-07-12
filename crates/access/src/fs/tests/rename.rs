use super::*;

#[tokio::test]
async fn fs_rename_path_moves_directory_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-rename-path");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("incoming/demo-skill")).expect("create source dir");
    fs::write(
        workspace_root.join("incoming/demo-skill/SKILL.md"),
        "# Demo Skill\n",
    )
    .expect("write source file");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .rename_path(
            "incoming/demo-skill",
            "managed/demo-skill",
            FsWriteOptions {
                create_dirs: true,
                overwrite: false,
            },
        )
        .await
        .expect("rename should execute after policy grants");

    assert!(!output.overwritten);
    assert!(!workspace_root.join("incoming/demo-skill").exists());
    assert_eq!(
        fs::read_to_string(workspace_root.join("managed/demo-skill/SKILL.md"))
            .expect("read renamed file"),
        "# Demo Skill\n"
    );

    fs::remove_dir_all(base).ok();
}

#[test]
fn fs_rename_action_uses_granted_entry_paths() {
    let action = FsRenameAction::new(
        GrantedEntryPath::new(PathBuf::from("/workspace/incoming/demo-skill")),
        GrantedEntryPath::new(PathBuf::from("/workspace/managed/demo-skill")),
        FsWriteOptions {
            create_dirs: true,
            overwrite: false,
        },
    );
    let metadata = action.metadata();

    assert_eq!(
        action.source_path(),
        Path::new("/workspace/incoming/demo-skill")
    );
    assert_eq!(
        action.destination_path(),
        Path::new("/workspace/managed/demo-skill")
    );
    assert_eq!(metadata.kind, "fs.rename");
    assert_eq!(metadata.operation, "rename_path");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemWrite]
    );
    let expected_payload = serde_json::json!({
        "source": "/workspace/incoming/demo-skill",
        "destination": "/workspace/managed/demo-skill",
        "create_dirs": true,
        "overwrite": false,
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);

    let action = FsAction::rename_path(action);
    assert_eq!(action.metadata().kind, "fs.rename");
}

#[tokio::test]
async fn fs_rename_path_rejects_existing_destination_without_overwrite() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-rename-path-overwrite");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("incoming/demo")).expect("create source dir");
    fs::create_dir_all(workspace_root.join("managed/demo")).expect("create destination dir");
    fs::write(workspace_root.join("incoming/demo/SKILL.md"), "new").expect("write source");
    fs::write(workspace_root.join("managed/demo/SKILL.md"), "old").expect("write destination");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .rename_path(
            "incoming/demo",
            "managed/demo",
            FsWriteOptions {
                create_dirs: false,
                overwrite: false,
            },
        )
        .await
        .expect_err("existing destination should require overwrite");

    assert!(matches!(
        error,
        FsAccessError::PathExistsRequiresOverwrite { .. }
    ));
    assert!(workspace_root.join("incoming/demo/SKILL.md").exists());
    assert_eq!(
        fs::read_to_string(workspace_root.join("managed/demo/SKILL.md"))
            .expect("read original destination"),
        "old"
    );

    fs::remove_dir_all(base).ok();
}

#[cfg(unix)]
#[tokio::test]
async fn fs_rename_path_treats_dangling_destination_symlink_as_existing() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-rename-dangling-destination");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("incoming/demo")).expect("create source dir");
    fs::write(workspace_root.join("incoming/demo/SKILL.md"), "new").expect("write source");
    create_symlink(
        Path::new("missing-target"),
        &workspace_root.join("managed-link"),
    )
    .expect("create dangling symlink destination");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .rename_path(
            "incoming/demo",
            "managed-link",
            FsWriteOptions {
                create_dirs: false,
                overwrite: false,
            },
        )
        .await
        .expect_err("dangling destination symlink should require overwrite");

    assert!(matches!(
        error,
        FsAccessError::PathExistsRequiresOverwrite { .. }
    ));
    assert!(workspace_root.join("incoming/demo/SKILL.md").exists());
    assert!(workspace_root.join("managed-link").is_symlink());

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_rename_path_denies_before_side_effect() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-rename-path-deny");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("incoming/demo")).expect("create source dir");
    fs::write(workspace_root.join("incoming/demo/SKILL.md"), "new").expect("write source");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .rename_path(
            "incoming/demo",
            "managed/demo",
            FsWriteOptions {
                create_dirs: true,
                overwrite: false,
            },
        )
        .await
        .expect_err("policy denial should happen before rename");

    assert!(matches!(error, FsAccessError::Authorization(_)));
    assert!(workspace_root.join("incoming/demo/SKILL.md").exists());
    assert!(!workspace_root.join("managed/demo").exists());

    fs::remove_dir_all(base).ok();
}
