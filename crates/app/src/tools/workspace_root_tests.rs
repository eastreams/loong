use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use loong_contracts::{Capability, ExecutionRoute, HarnessKind, ToolCoreOutcome, ToolCoreRequest};
use loong_kernel::{
    InMemoryAuditSink, Kernel, SystemClock, VerticalPackManifest,
    access::fs::{FsPathAllowedRootsPolicy, FsResolvePathAllowPolicy},
    policy::{
        FsContentSearchAllowPolicy, FsGlobAllowPolicy, FsReadAllowPolicy, FsReadFilenameDenyPolicy,
        FsWriteAllowPolicy, PolicyPipelineBuilder,
    },
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

async fn execute_tool_core_with_test_context(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let trusted_internal_payload = payload_uses_reserved_internal_tool_context(&request.payload);
    let mut policy =
        PolicyPipelineBuilder::<crate::context::AppContextFactory>::new_legacy_allow_fallback()
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
    let mut kernel = Kernel::with_policy_runtime(
        policy,
        Arc::new(SystemClock),
        Arc::new(InMemoryAuditSink::default()),
    );
    let pack = VerticalPackManifest {
        pack_id: "test-pack".to_owned(),
        domain: "test".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
        ]),
        metadata: Default::default(),
    };
    kernel
        .register_pack(pack)
        .map_err(|error| format!("kernel pack registration failed: {error}"))?;
    crate::tools::register_kernel_tools(
        &mut kernel,
        config.clone(),
        crate::config::ObservabilityConfig::runtime_default(),
    )
    .map_err(|error| format!("kernel tool registration failed: {error}"))?;
    let token = kernel
        .issue_token("test-pack", "test-agent", 60)
        .map_err(|error| format!("kernel token issue failed: {error}"))?;
    let app_ctx = crate::AppContext::new(
        Arc::new(loong_runtime::runtime::Runtime::new(
            kernel,
            crate::tools::plane::test_builtin_tool_plane(),
        )),
        token,
        config.clone(),
        "test-session",
        crate::tools::runtime_tool_view(),
        loong_contracts::GovernedSessionMode::MutatingCapable,
    )?;

    execute_kernel_tool_request(&app_ctx, request, trusted_internal_payload)
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

    let outcome = execute_tool_core_with_test_context(
        ToolCoreRequest {
            tool_name: "file.read".to_owned(),
            payload: json!({
                "path": "note.txt"
            }),
        },
        &config,
    )
    .await
    .expect("runtime workspace root should be used for default resolution");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["content"], "runtime");
    assert_eq!(outcome.payload["path"], expected_path.display().to_string());

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

    let error = execute_tool_core_with_test_context(
        ToolCoreRequest {
            tool_name: "file.read".to_owned(),
            payload: json!({
                "path": "clippy.toml"
            }),
        },
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

    let relative_outcome = execute_tool_core_with_test_context(
        ToolCoreRequest {
            tool_name: "file.read".to_owned(),
            payload: json!({
                "path": "inner.txt"
            }),
        },
        &config,
    )
    .await
    .expect("relative path should resolve from workspace root");
    assert_eq!(relative_outcome.payload["content"], "inner");

    let absolute_outcome = execute_tool_core_with_test_context(
        ToolCoreRequest {
            tool_name: "file.read".to_owned(),
            payload: json!({
                "path": outer_root.join("outer.txt").display().to_string()
            }),
        },
        &config,
    )
    .await
    .expect("absolute path inside file_root should still be allowed");
    assert_eq!(absolute_outcome.payload["content"], "outer");

    std::fs::remove_dir_all(&outer_root).ok();
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn file_read_uses_workspace_root_from_trusted_internal_payload() {
    let outer_root = std::env::temp_dir().join(format!(
        "loong-file-read-workspace-root-outer-{}",
        std::process::id()
    ));
    let child_root = std::env::temp_dir().join(format!(
        "loong-file-read-workspace-root-child-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&outer_root).expect("create outer root");
    std::fs::create_dir_all(&child_root).expect("create child root");
    std::fs::write(outer_root.join("note.txt"), "outer").expect("write outer note");
    std::fs::write(child_root.join("note.txt"), "child").expect("write child note");

    let config = test_tool_runtime_config(outer_root.clone());
    let outcome = execute_tool_core_with_test_context(
        ToolCoreRequest {
            tool_name: "file.read".to_owned(),
            payload: json!({
                "path": "note.txt",
                "_loong": {
                    "workspace_root": child_root.display().to_string()
                }
            }),
        },
        &config,
    )
    .await
    .expect("trusted workspace root override should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["content"], "child");
    let expected_path =
        dunce::canonicalize(child_root.join("note.txt")).expect("canonicalize child note");
    assert_eq!(outcome.payload["path"], expected_path.display().to_string());

    std::fs::remove_dir_all(&outer_root).ok();
    std::fs::remove_dir_all(&child_root).ok();
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn file_read_rejects_relative_workspace_root_from_trusted_internal_payload() {
    let outer_root = std::env::temp_dir().join(format!(
        "loong-file-read-relative-workspace-root-outer-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&outer_root).expect("create outer root");
    std::fs::write(outer_root.join("note.txt"), "outer").expect("write outer note");

    let config = test_tool_runtime_config(outer_root.clone());
    let error = execute_tool_core_with_test_context(
        ToolCoreRequest {
            tool_name: "file.read".to_owned(),
            payload: json!({
                "path": "note.txt",
                "_loong": {
                    "workspace_root": "relative/path"
                }
            }),
        },
        &config,
    )
    .await
    .expect_err("relative workspace root override should be rejected");

    assert!(
        error.contains("path must be absolute"),
        "expected absolute-path rejection, got: {error}"
    );

    std::fs::remove_dir_all(&outer_root).ok();
}
