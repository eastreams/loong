use super::*;

#[tokio::test]
async fn fs_glob_action_lists_matching_paths_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-glob-paths");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("src/nested")).expect("create source dirs");
    fs::write(workspace_root.join("src/lib.rs"), "pub fn lib() {}").expect("write lib");
    fs::write(workspace_root.join("src/main.rs"), "fn main() {}").expect("write main");
    fs::write(workspace_root.join("src/nested/mod.rs"), "pub mod nested;").expect("write nested");
    fs::write(workspace_root.join("README.md"), "readme").expect("write readme");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .glob_paths(".", "**/*.rs", false, 10)
        .await
        .expect("glob should execute after policy grants");

    assert_eq!(
        output
            .matches
            .iter()
            .map(|entry| (entry.relative_path.as_str(), entry.kind))
            .collect::<Vec<_>>(),
        vec![
            ("src/lib.rs", FsPathKind::File),
            ("src/main.rs", FsPathKind::File),
            ("src/nested/mod.rs", FsPathKind::File),
        ]
    );
    assert!(!output.truncated);

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_glob_action_uses_granted_path_root() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let path = grant_target_path(&kernel, &ctx, "src").await;
    let action = FsGlobAction::new(path, "**/*.rs", true, 50);
    let metadata = action.metadata();

    assert_eq!(metadata.kind, "fs.glob");
    assert_eq!(metadata.operation, "glob_paths");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemRead]
    );
    let expected_payload = serde_json::json!({
        "root": "/workspace/src",
        "pattern": "**/*.rs",
        "include_directories": true,
        "max_results": 50,
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn fs_glob_denies_before_reading_directory() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-glob-deny-before-read");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .glob_paths(".", "**/*.rs", false, 10)
        .await
        .expect_err("policy denial should happen before directory read");

    assert!(matches!(error, FsAccessError::Authorization(_)));

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_read_dir_lists_immediate_entries_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-read-dir");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("src/nested")).expect("create source dirs");
    fs::write(workspace_root.join("README.md"), "readme").expect("write readme");
    fs::write(workspace_root.join("src/main.rs"), "fn main() {}").expect("write main");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .read_dir(".", 10)
        .await
        .expect("read_dir should execute after policy grants");

    assert_eq!(
        output
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.kind))
            .collect::<Vec<_>>(),
        vec![
            ("README.md", FsPathKind::File),
            ("src", FsPathKind::Directory)
        ]
    );
    assert!(!output.truncated);

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_read_dir_truncates_at_max_entries() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-read-dir-truncated");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    fs::write(workspace_root.join("a.txt"), "a").expect("write a");
    fs::write(workspace_root.join("b.txt"), "b").expect("write b");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .read_dir(".", 1)
        .await
        .expect("read_dir should execute after policy grants");

    assert_eq!(output.entries.len(), 1);
    assert!(output.truncated);

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_read_dir_action_uses_granted_path_root() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let path = grant_target_path(&kernel, &ctx, "src").await;
    let action = FsReadDirAction::new(path, 25);
    let metadata = action.metadata();

    assert_eq!(metadata.kind, "fs.read_dir");
    assert_eq!(metadata.operation, "read_dir");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemRead]
    );
    let expected_payload = serde_json::json!({
        "root": "/workspace/src",
        "max_entries": 25,
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);

    let action = FsAction::read_dir(GrantedPath::new(PathBuf::from("/workspace/src")), 25);
    assert_eq!(action.metadata().kind, "fs.read_dir");
}

#[tokio::test]
async fn fs_read_dir_denies_before_reading_directory() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-read-dir-deny-before-read");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .read_dir(".", 10)
        .await
        .expect_err("policy denial should happen before directory read");

    assert!(matches!(error, FsAccessError::Authorization(_)));

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_content_search_returns_match_metadata_after_grants() {
    let kernel = FsAccessTestKernel::default();
    let base = unique_temp_dir("loong-access-fs-content-search");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(workspace_root.join("src")).expect("create source dir");
    fs::write(
        workspace_root.join("src/main.rs"),
        "fn main() {\n    println!(\"hello world\");\n}\n",
    )
    .expect("write main");
    fs::write(workspace_root.join("notes.txt"), "hello from notes").expect("write notes");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let output = ctx
        .access()
        .fs()
        .search_content(
            ".",
            "hello world",
            FsContentSearchOptions {
                glob: Some("src/**/*.rs".to_owned()),
                max_results: 5,
                max_bytes_per_file: 262_144,
                case_sensitive: false,
            },
        )
        .await
        .expect("content search should execute after policy grants");

    let first = output.matches.first().expect("first match");
    assert_eq!(output.matches.len(), 1);
    assert_eq!(first.relative_path, "src/main.rs");
    assert_eq!(first.line, 2);
    assert_eq!(first.column, 15);
    assert_eq!(first.match_text, "hello world");
    assert_eq!(first.snippet, "println!(\"hello world\");");
    assert!(!first.truncated_file);

    fs::remove_dir_all(base).ok();
}

#[tokio::test]
async fn fs_content_search_action_uses_granted_path_root() {
    let kernel = FsAccessTestKernel::default();
    let workspace_root = PathBuf::from("/workspace");
    let ctx = FsAccessPolicyContext::new(&workspace_root);
    let path = grant_target_path(&kernel, &ctx, "src").await;
    let action = FsContentSearchAction::new(
        path,
        "needle",
        FsContentSearchOptions {
            glob: Some("**/*.rs".to_owned()),
            max_results: 20,
            max_bytes_per_file: 1024,
            case_sensitive: true,
        },
    );
    let metadata = action.metadata();

    assert_eq!(metadata.kind, "fs.content_search");
    assert_eq!(metadata.operation, "search_content");
    assert_eq!(
        metadata.required_capabilities.as_ref(),
        [Capability::FilesystemRead]
    );
    let expected_payload = serde_json::json!({
        "root": "/workspace/src",
        "query": "needle",
        "glob": "**/*.rs",
        "max_results": 20,
        "max_bytes_per_file": 1024,
        "case_sensitive": true,
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn fs_content_search_denies_before_reading_directory() {
    let kernel = FsAccessTestKernel::denying();
    let base = unique_temp_dir("loong-access-fs-content-deny-before-read");
    let workspace_root = base.join("workspace");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    let ctx = FsAccessToolCx::new(&kernel, &workspace_root);

    let error = ctx
        .access()
        .fs()
        .search_content(
            ".",
            "needle",
            FsContentSearchOptions {
                glob: None,
                max_results: 10,
                max_bytes_per_file: 262_144,
                case_sensitive: false,
            },
        )
        .await
        .expect_err("policy denial should happen before directory read");

    assert!(matches!(error, FsAccessError::Authorization(_)));

    fs::remove_dir_all(base).ok();
}
