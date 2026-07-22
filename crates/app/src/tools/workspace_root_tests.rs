use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use loong_contracts::{Capabilities, Capability, GovernedSessionMode};
use loong_kernel::{
    InMemoryAuditSink, Kernel, SystemClock,
    access::fs::{
        FsContentSearchAllowPolicy, FsGlobAllowPolicy, FsPathAllowedRootsPolicy, FsReadAllowPolicy,
        FsReadFilenameDenyPolicy, FsResolvePathAllowPolicy, FsWriteAllowPolicy,
    },
    policy::PolicyPipelineBuilder,
};
use serde_json::json;

use super::*;

fn test_tool_runtime_config(root: PathBuf) -> runtime_config::ToolRuntimeConfig {
    runtime_config::ToolRuntimeConfig {
        shell_allow: BTreeSet::from(["echo".to_owned(), "cat".to_owned(), "ls".to_owned()]),
        file_root: Some(root),
        messages_enabled: true,
        skills: runtime_config::SkillsRuntimePolicy {
            enabled: true,
            require_download_approval: true,
            allowed_domains: BTreeSet::new(),
            blocked_domains: BTreeSet::new(),
            install_root: None,
            auto_expose_installed: false,
        },
        ..Default::default()
    }
}

/// Build the real policy/runtime boundary once per case while keeping the tests
/// focused on Context-owned filesystem authority rather than legacy envelopes.
async fn invoke_read_with_test_context(
    payload: serde_json::Value,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<serde_json::Value, String> {
    let mut policy =
        PolicyPipelineBuilder::<crate::context::RuntimeContextFactory>::new_legacy_allow_fallback()
            .with_pre_policy(crate::tools::plane::ToolVisibilityPolicy)
            .with_policy(crate::tools::plane::ToolInvocationAllowPolicy)
            .with_policy(FsResolvePathAllowPolicy::target())
            .with_policy(FsResolvePathAllowPolicy::entry())
            .with_policy(FsPathAllowedRootsPolicy::target())
            .with_policy(FsPathAllowedRootsPolicy::entry());
    if !config.fs.deny_read_filenames.is_empty() {
        policy.push_policy(FsReadFilenameDenyPolicy::new(
            config.fs.deny_read_filenames.clone(),
        ));
    }
    policy.push_policy(FsReadAllowPolicy);
    policy.push_policy(FsWriteAllowPolicy);
    policy.push_policy(FsGlobAllowPolicy);
    policy.push_policy(FsContentSearchAllowPolicy);
    let kernel = Kernel::with_policy_runtime(
        policy,
        Arc::new(SystemClock),
        Arc::new(InMemoryAuditSink::default()),
    );
    let runtime = Arc::new(loong_runtime::runtime::Runtime::new(
        kernel,
        crate::tools::plane::test_builtin_tool_plane(),
    ));
    let session = crate::Session::root(
        runtime.as_ref(),
        "test-agent",
        "test-session",
        GovernedSessionMode::MutatingCapable,
        Capabilities::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
        ]),
        config.clone(),
        crate::memory::runtime_config::MemoryRuntimeConfig::default(),
        crate::tools::runtime_visible_tool_view(runtime.as_ref(), config, None),
        None,
        None,
    )?;
    let context = crate::Context::new(runtime.as_ref(), &session)
        .expect("workspace test Session must remain bound to its construction Runtime");
    context
        .tool(loong_contracts::ToolPath::new(["read"]).expect("test tool path must be valid"))
        .map_err(|error| error.to_string())?
        .invoke(payload)
        .await
        .map_err(|error| format!("{error}"))
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn file_read_uses_runtime_workspace_root_from_runtime_config() {
    let outer_root = std::env::temp_dir().join(format!(
        "loongclaw-file-read-runtime-workspace-root-outer-{}",
        std::process::id()
    ));
    let runtime_root = std::env::temp_dir().join(format!(
        "loongclaw-file-read-runtime-workspace-root-runtime-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&outer_root).expect("create outer root");
    std::fs::create_dir_all(&runtime_root).expect("create runtime root");
    std::fs::write(outer_root.join("note.txt"), "outer").expect("write outer note");
    std::fs::write(runtime_root.join("note.txt"), "runtime").expect("write runtime note");
    let expected_path =
        dunce::canonicalize(runtime_root.join("note.txt")).expect("canonicalize runtime note");

    let mut config = test_tool_runtime_config(outer_root.clone());
    config.workspace_root = Some(runtime_root.clone());

    let outcome = invoke_read_with_test_context(
        json!({
            "path": "note.txt"
        }),
        &config,
    )
    .await
    .expect("runtime workspace root should be used for default resolution");

    assert_eq!(outcome["content"], "runtime");
    assert_eq!(outcome["path"], expected_path.display().to_string());

    std::fs::remove_dir_all(&outer_root).ok();
    std::fs::remove_dir_all(&runtime_root).ok();
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn file_read_rejects_configured_denied_filename() {
    let root = std::env::temp_dir().join(format!(
        "loong-file-read-configured-denied-filename-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(root.join("clippy.toml"), "warns = []").expect("write clippy fixture");

    let mut config = test_tool_runtime_config(root.clone());
    config
        .fs
        .deny_read_filenames
        .insert("clippy.toml".to_owned());

    let error = invoke_read_with_test_context(
        json!({
            "path": "clippy.toml"
        }),
        &config,
    )
    .await
    .expect_err("configured filename policy should reject clippy.toml reads");

    assert!(
        error.contains("clippy.toml"),
        "expected clippy.toml denial, got: {error}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn file_read_relative_resolution_uses_workspace_root_without_shrinking_file_root_access() {
    let outer_root = std::env::temp_dir().join(format!(
        "loong-file-read-relative-resolution-outer-{}",
        std::process::id()
    ));
    let runtime_root = outer_root.join("workspace");
    std::fs::create_dir_all(&runtime_root).expect("create runtime root");
    std::fs::write(outer_root.join("outer.txt"), "outer").expect("write outer note");
    std::fs::write(runtime_root.join("inner.txt"), "inner").expect("write runtime note");

    let mut config = test_tool_runtime_config(outer_root.clone());
    config.workspace_root = Some(runtime_root);

    let relative_outcome = invoke_read_with_test_context(
        json!({
            "path": "inner.txt"
        }),
        &config,
    )
    .await
    .expect("relative path should resolve from workspace root");
    assert_eq!(relative_outcome["content"], "inner");

    let absolute_outcome = invoke_read_with_test_context(
        json!({
            "path": outer_root.join("outer.txt").display().to_string()
        }),
        &config,
    )
    .await
    .expect("absolute path inside file_root should still be allowed");
    assert_eq!(absolute_outcome["content"], "outer");

    std::fs::remove_dir_all(&outer_root).ok();
}
