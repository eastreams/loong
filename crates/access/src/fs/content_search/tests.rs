use std::fs;

use crate::fs::{
    path::FsPathError,
    test_support::{
        FsAccessTestContext, FsAccessTestKernel, FsAccessToolCx, grant_target_path, unique_temp_dir,
    },
};

use super::*;

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
    let ctx = FsAccessTestContext::new(&workspace_root);
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

    assert!(matches!(
        error,
        FsContentSearchError::Path(FsPathError::Authorization(_))
    ));

    fs::remove_dir_all(base).ok();
}
