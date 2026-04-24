use std::collections::BTreeSet;
use std::path::PathBuf;

use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::json;

use super::*;

fn test_tool_runtime_config(root: PathBuf) -> runtime_config::ToolRuntimeConfig {
    runtime_config::ToolRuntimeConfig {
        shell_allow: BTreeSet::from(["echo".to_owned(), "cat".to_owned(), "ls".to_owned()]),
        file_root: Some(root),
        messages_enabled: true,
        external_skills: runtime_config::ExternalSkillsRuntimePolicy {
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

fn execute_tool_core_with_test_context(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    if payload_uses_reserved_internal_tool_context(&request.payload) {
        with_trusted_internal_tool_payload(|| super::execute_tool_core_with_config(request, config))
    } else {
        super::execute_tool_core_with_config(request, config)
    }
}

#[cfg(feature = "tool-file")]
#[test]
fn file_read_uses_runtime_workspace_root_from_runtime_config() {
    let outer_root = std::env::temp_dir().join(format!(
        "loong-file-read-runtime-workspace-root-outer-{}",
        std::process::id()
    ));
    let runtime_root = std::env::temp_dir().join(format!(
        "loong-file-read-runtime-workspace-root-runtime-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&outer_root).expect("create outer root");
    std::fs::create_dir_all(&runtime_root).expect("create runtime root");
    std::fs::write(outer_root.join("note.txt"), "outer").expect("write outer note");
    std::fs::write(runtime_root.join("note.txt"), "runtime").expect("write runtime note");

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
    .expect("runtime workspace root should be used for default resolution");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["content"], "runtime");
    let expected_path =
        dunce::canonicalize(runtime_root.join("note.txt")).expect("canonicalize runtime note");
    assert_eq!(outcome.payload["path"], expected_path.display().to_string());

    std::fs::remove_dir_all(&outer_root).ok();
    std::fs::remove_dir_all(&runtime_root).ok();
}

#[cfg(feature = "tool-file")]
#[test]
fn file_read_uses_workspace_root_from_trusted_internal_payload() {
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
#[test]
fn file_read_rejects_relative_workspace_root_from_trusted_internal_payload() {
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
    .expect_err("relative workspace root override should be rejected");

    assert!(
        error.contains("path must be absolute"),
        "expected absolute-path rejection, got: {error}"
    );

    std::fs::remove_dir_all(&outer_root).ok();
}

#[cfg(feature = "tool-file")]
#[test]
fn tool_invoke_preserves_combined_trusted_internal_context_for_inner_execution() {
    let _env = crate::test_support::ScopedEnv::new();
    let child_root = std::env::temp_dir().join(format!(
        "loong-tool-invoke-workspace-root-child-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&child_root).expect("create child fixture root");
    std::fs::write(child_root.join("note.txt"), "child").expect("write child note");

    let (tool_name, mut payload) = bridge_provider_tool_call_with_scope(
        "file.read",
        json!({
            "path": "note.txt"
        }),
        None,
        None,
    );
    let payload_object = payload.as_object_mut().expect("tool.invoke payload object");
    payload_object.insert(
        LOONG_INTERNAL_TOOL_CONTEXT_KEY.to_owned(),
        json!({
            LOONG_INTERNAL_WORKSPACE_ROOT_KEY: child_root.display().to_string(),
            LOONG_INTERNAL_RUNTIME_NARROWING_KEY: {
                "web_fetch": {
                    "allowed_domains": ["docs.example.com"],
                    "allow_private_hosts": false
                }
            }
        }),
    );

    let request = ToolCoreRequest { tool_name, payload };
    let (_, effective_request) =
        resolve_tool_invoke_request(&request).expect("tool.invoke should preserve trusted context");

    let internal_context = effective_request.payload[LOONG_INTERNAL_TOOL_CONTEXT_KEY]
        .as_object()
        .expect("inner arguments should keep trusted internal context");
    assert_eq!(
        internal_context[LOONG_INTERNAL_WORKSPACE_ROOT_KEY],
        child_root.display().to_string()
    );
    assert_eq!(
        internal_context[LOONG_INTERNAL_RUNTIME_NARROWING_KEY]["web_fetch"]["allowed_domains"][0],
        "docs.example.com"
    );
    assert_eq!(effective_request.payload["path"], "note.txt");

    std::fs::remove_dir_all(&child_root).ok();
}
