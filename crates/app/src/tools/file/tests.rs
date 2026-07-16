use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use loong_contracts::{Capability, ExecutionRoute, HarnessKind, ToolCoreOutcome, ToolCoreRequest};
use loong_core::tool::{RegisteredTool, RegisteredToolError, ToolProvenance};
use loong_kernel::{
    InMemoryAuditSink, Kernel, SystemClock, VerticalPackManifest,
    access::fs::{
        FsPathAllowedRootsPolicy, FsReadAllowPolicy, FsReadFilenameDenyPolicy,
        FsResolvePathAllowPolicy,
    },
    policy::{
        FsContentSearchAllowPolicy, FsGlobAllowPolicy, FsWriteAllowPolicy, PolicyPipelineBuilder,
    },
};
use loong_tools::file::ReadTool;
use serde_json::json;

use super::*;
use crate::context::AppContextFactory;
use crate::tools::file_path::resolve_safe_file_path_with_config;
use crate::tools::runtime_config::ToolRuntimeConfig;
use crate::tools::runtime_events::{
    ToolFileChangeKind, ToolRuntimeEvent, ToolRuntimeEventSink, with_tool_runtime_event_sink,
};

#[derive(Default)]
struct RecordingRuntimeSink {
    events: Mutex<Vec<ToolRuntimeEvent>>,
}

fn lock_runtime_events(
    sink: &RecordingRuntimeSink,
) -> std::sync::MutexGuard<'_, Vec<ToolRuntimeEvent>> {
    match sink.events.lock() {
        Ok(events) => events,
        Err(poisoned_events) => poisoned_events.into_inner(),
    }
}

impl ToolRuntimeEventSink for RecordingRuntimeSink {
    fn emit(&self, event: ToolRuntimeEvent) {
        let mut events = lock_runtime_events(self);
        events.push(event);
    }
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}

fn test_pack_with_capabilities(granted_capabilities: BTreeSet<Capability>) -> VerticalPackManifest {
    VerticalPackManifest {
        pack_id: "test-pack".to_owned(),
        domain: "test".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities,
        metadata: Default::default(),
    }
}

fn test_pack() -> VerticalPackManifest {
    test_pack_with_capabilities(BTreeSet::from([
        Capability::InvokeTool,
        Capability::FilesystemRead,
        Capability::FilesystemWrite,
    ]))
}

async fn execute_file_read_with_test_context(
    request: ToolCoreRequest,
    config: &ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let mut policy = PolicyPipelineBuilder::<AppContextFactory>::new_legacy_allow_fallback()
        .with_policy(crate::tools::plane::ToolInvocationAllowPolicy)
        .with_policy(FsResolvePathAllowPolicy::target())
        .with_policy(FsPathAllowedRootsPolicy::target());
    if !config.fs.deny_read_filenames.is_empty() {
        policy.push_policy(FsReadFilenameDenyPolicy::new(
            config.fs.deny_read_filenames.clone(),
        ));
    }
    policy.push_policy(FsReadAllowPolicy);
    policy.push_policy(FsWriteAllowPolicy);
    policy.push_policy(FsGlobAllowPolicy);
    policy.push_policy(FsContentSearchAllowPolicy);
    let mut kernel = Kernel::<AppContextFactory>::with_policy_runtime(
        policy,
        Arc::new(SystemClock),
        Arc::new(InMemoryAuditSink::default()),
    );
    let pack = test_pack();
    kernel
        .register_pack(pack)
        .map_err(|error| format!("register pack failed: {error}"))?;
    let token = kernel
        .issue_token("test-pack", "test-agent", 60)
        .map_err(|error| format!("issue token failed: {error}"))?;
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
    let execution_context = app_ctx.for_invocation(config)?;
    let _ = config;
    let tool = RegisteredTool::<AppContextFactory>::from_tool(
        ToolProvenance::Compatibility,
        ReadTool::new("read"),
    );
    let outcome = tool
        .invoke(&execution_context, request.payload)
        .await
        .map_err(|error| match error {
            RegisteredToolError::Input(loong_contracts::ToolInputError::InvalidPayload {
                reason,
            }) => reason,
            RegisteredToolError::Input(input_error) => input_error.to_string(),
            RegisteredToolError::Execution { source } => source.to_string(),
            unknown => unknown.to_string(),
        })?;
    Ok(ToolCoreOutcome {
        status: "ok".to_owned(),
        payload: outcome,
    })
}

async fn execute_request_via_kernel_tool_registry(
    request: ToolCoreRequest,
    config: &ToolRuntimeConfig,
) -> Result<(ToolCoreOutcome, Arc<InMemoryAuditSink>), loong_kernel::KernelError> {
    execute_request_via_kernel_tool_registry_with_capabilities(
        request,
        config,
        BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
        ]),
    )
    .await
}

async fn execute_request_via_kernel_tool_registry_result(
    request: ToolCoreRequest,
    config: &ToolRuntimeConfig,
) -> (
    Result<ToolCoreOutcome, loong_kernel::KernelError>,
    Arc<InMemoryAuditSink>,
) {
    execute_request_via_kernel_tool_registry_with_capabilities_result(
        request,
        config,
        BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
        ]),
    )
    .await
}

async fn execute_request_via_kernel_tool_registry_with_capabilities(
    request: ToolCoreRequest,
    config: &ToolRuntimeConfig,
    capabilities: BTreeSet<Capability>,
) -> Result<(ToolCoreOutcome, Arc<InMemoryAuditSink>), loong_kernel::KernelError> {
    let (outcome, audit) = execute_request_via_kernel_tool_registry_with_capabilities_result(
        request,
        config,
        capabilities,
    )
    .await;
    outcome.map(|outcome| (outcome, audit))
}

// Invalid fixture setup may panic; production bootstrap propagates every
// registration and token error instead.
#[allow(clippy::expect_used)]
async fn execute_request_via_kernel_tool_registry_with_capabilities_result(
    request: ToolCoreRequest,
    config: &ToolRuntimeConfig,
    capabilities: BTreeSet<Capability>,
) -> (
    Result<ToolCoreOutcome, loong_kernel::KernelError>,
    Arc<InMemoryAuditSink>,
) {
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut policy = PolicyPipelineBuilder::<AppContextFactory>::new_legacy_allow_fallback()
        .with_policy(crate::tools::plane::ToolInvocationAllowPolicy)
        .with_policy(FsResolvePathAllowPolicy::target())
        .with_policy(FsPathAllowedRootsPolicy::target());
    if !config.fs.deny_read_filenames.is_empty() {
        policy.push_policy(FsReadFilenameDenyPolicy::new(
            config.fs.deny_read_filenames.clone(),
        ));
    }
    policy.push_policy(FsReadAllowPolicy);
    policy.push_policy(FsWriteAllowPolicy);
    policy.push_policy(FsGlobAllowPolicy);
    policy.push_policy(FsContentSearchAllowPolicy);
    let mut kernel = Kernel::<AppContextFactory>::with_policy_runtime(
        policy,
        Arc::new(SystemClock),
        audit.clone(),
    );
    let pack = test_pack_with_capabilities(capabilities);
    kernel.register_pack(pack).expect("register test pack");
    crate::tools::register_kernel_tools(
        &mut kernel,
        config.clone(),
        crate::config::ObservabilityConfig::runtime_default(),
    )
    .expect("register test kernel tools");
    let token = kernel
        .issue_token("test-pack", "test-agent", 60)
        .expect("issue test token");
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
    )
    .expect("test app context");
    let outcome = crate::tools::execute_kernel_tool_request(&app_ctx, request, false).await;
    (outcome, audit)
}

fn tool_invoke_request(
    tool_id: &str,
    arguments: serde_json::Value,
) -> Result<ToolCoreRequest, String> {
    let lease_payload = serde_json::Map::new();
    let lease =
        crate::tools::issue_tool_lease(crate::tools::canonical_tool_name(tool_id), &lease_payload)?;

    Ok(ToolCoreRequest {
        tool_name: "tool.invoke".to_owned(),
        payload: json!({
            "tool_id": tool_id,
            "lease": lease,
            "arguments": arguments,
        }),
    })
}

#[cfg(unix)]
#[test]
fn resolve_safe_file_path_rejects_symlink_escape_on_read() {
    let base = unique_temp_dir("loong-file-read");
    let root = base.join("root");
    let outside = base.join("outside");
    fs::create_dir_all(&root).expect("create root");
    fs::create_dir_all(&outside).expect("create outside");

    let outside_file = outside.join("secret.txt");
    fs::write(&outside_file, "secret").expect("write outside file");
    let link = root.join("secret-link");
    assert!(create_symlink(&outside_file, &link).is_ok());

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let error =
        resolve_safe_file_path_with_config("secret-link", &config).expect_err("escape denied");

    assert!(error.starts_with("policy_denied: "));
    assert!(error.contains("escapes configured file root"));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn file_read_supports_line_window_pagination() {
    let base = unique_temp_dir("loongclaw-file-read-window");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("notes.txt"), "alpha\nbeta\ngamma\ndelta").expect("write fixture");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.read".to_owned(),
        payload: json!({
            "path": "notes.txt",
            "offset": 2,
            "limit": 2
        }),
    };

    let outcome = execute_file_read_with_test_context(request, &config)
        .await
        .expect("file.read window should succeed");

    assert_eq!(outcome.payload["content"], json!("beta\ngamma"));
    assert_eq!(outcome.payload["line_start"], json!(2));
    assert_eq!(outcome.payload["line_end"], json!(3));
    assert_eq!(outcome.payload["total_lines"], json!(4));
    assert_eq!(outcome.payload["next_offset"], json!(4));
    assert_eq!(outcome.payload["truncated"], json!(false));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_file_read_uses_typed_tool_registry() {
    let base = unique_temp_dir("loong-file-read-typed-registry");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("notes.txt"), "alpha\nbeta\ngamma").expect("write fixture");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.read".to_owned(),
        payload: json!({
            "path": "notes.txt",
            "offset": 2,
            "limit": 1
        }),
    };

    let (outcome, audit) = execute_request_via_kernel_tool_registry(request, &config)
        .await
        .expect("file.read should execute through typed registry");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["content"], json!("beta"));
    assert_eq!(outcome.payload["line_start"], json!(2));
    assert_eq!(outcome.payload["line_end"], json!(2));
    let events = audit.snapshot();
    assert!(events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::ToolInvocation {
                path_display,
                outcome: loong_kernel::InvocationOutcome::Completed,
                ..
            } if path_display == "read"
        )
    }));
    assert!(!events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::PlaneInvoked {
                primary_adapter,
                ..
            } if primary_adapter.starts_with("legacy:")
        )
    }));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_tool_invoke_file_read_uses_typed_tool_registry() {
    let base = unique_temp_dir("loong-tool-invoke-read-typed-registry");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("notes.txt"), "alpha\nbeta").expect("write fixture");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = tool_invoke_request(
        "file.read",
        json!({
            "path": "notes.txt",
            "offset": 2,
            "limit": 1
        }),
    )
    .unwrap_or_else(|error| panic!("issue test tool lease: {error}"));

    let (outcome, audit) = execute_request_via_kernel_tool_registry(request, &config)
        .await
        .expect("tool.invoke file.read should execute through typed registry");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["content"], json!("beta"));
    let events = audit.snapshot();
    assert!(events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::ToolInvocation {
                path_display,
                outcome: loong_kernel::InvocationOutcome::Completed,
                ..
            } if path_display == "read"
        )
    }));
    assert!(!events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::PlaneInvoked {
                primary_adapter,
                ..
            } if primary_adapter.starts_with("legacy:")
        )
    }));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_tool_invoke_capability_override_narrows_child_access_caps() {
    let base = unique_temp_dir("loong-tool-invoke-read-capability-override");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("notes.txt"), "alpha").expect("write fixture");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let mut request = tool_invoke_request(
        "file.read",
        json!({
            "path": "notes.txt",
        }),
    )
    .unwrap_or_else(|error| panic!("issue test tool lease: {error}"));
    request
        .payload
        .as_object_mut()
        .expect("tool.invoke payload object")
        .insert("capabilities_override".to_owned(), json!([]));

    let error = execute_request_via_kernel_tool_registry(request, &config)
        .await
        .expect_err("empty override should remove filesystem read from child context");

    assert!(
        error.to_string().contains("FilesystemRead")
            || error.to_string().contains("filesystem_read"),
        "expected filesystem read capability denial, got: {error}"
    );
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_tool_invoke_capability_override_rejects_added_capabilities() {
    let base = unique_temp_dir("loong-tool-invoke-read-capability-escalation");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let mut request = tool_invoke_request(
        "file.read",
        json!({
            "path": "notes.txt",
        }),
    )
    .unwrap_or_else(|error| panic!("issue test tool lease: {error}"));
    request
        .payload
        .as_object_mut()
        .expect("tool.invoke payload object")
        .insert(
            "capabilities_override".to_owned(),
            json!(["filesystem_write"]),
        );

    let error = execute_request_via_kernel_tool_registry(request, &config)
        .await
        .expect_err("override must not add capabilities beyond read descriptor");

    assert!(
        error
            .to_string()
            .contains("tool capability override cannot add capabilities"),
        "expected capability override rejection, got: {error}"
    );
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_direct_read_glob_uses_typed_tool_registry() {
    let base = unique_temp_dir("loong-read-glob-typed-registry");
    let root = base.join("root");
    fs::create_dir_all(root.join("src/nested")).expect("create root");
    fs::write(root.join("src/lib.rs"), "pub fn alpha() {}").expect("write lib");
    fs::write(root.join("src/nested/mod.rs"), "pub fn beta() {}").expect("write mod");
    fs::write(root.join("README.md"), "hello").expect("write readme");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "read".to_owned(),
        payload: json!({
            "pattern": "src/**/*.rs",
            "max_results": 10
        }),
    };

    let (outcome, audit) = execute_request_via_kernel_tool_registry(request, &config)
        .await
        .expect("read glob should execute through typed registry");

    assert_eq!(outcome.status, "ok");
    let matches = outcome.payload["matches"]
        .as_array()
        .expect("matches array");
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0]["path"], "src/lib.rs");
    assert_eq!(matches[1]["path"], "src/nested/mod.rs");
    let events = audit.snapshot();
    assert!(events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::ToolInvocation {
                path_display,
                outcome: loong_kernel::InvocationOutcome::Completed,
                ..
            } if path_display == "read"
        )
    }));
    assert!(!events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::PlaneInvoked {
                primary_adapter,
                ..
            } if primary_adapter.starts_with("legacy:")
        )
    }));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_direct_read_query_uses_typed_tool_registry() {
    let base = unique_temp_dir("loong-read-query-typed-registry");
    let root = base.join("root");
    fs::create_dir_all(root.join("src")).expect("create root");
    fs::write(
        root.join("src/main.rs"),
        "fn main() {\n    println!(\"hello world\");\n}\n",
    )
    .expect("write main");
    fs::write(root.join("notes.txt"), "hello from notes").expect("write notes");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "read".to_owned(),
        payload: json!({
            "query": "hello world",
            "glob": "src/**/*.rs",
            "max_results": 5
        }),
    };

    let (outcome, audit) = execute_request_via_kernel_tool_registry(request, &config)
        .await
        .expect("read query should execute through typed registry");

    assert_eq!(outcome.status, "ok");
    let matches = outcome.payload["matches"]
        .as_array()
        .expect("matches array");
    let first = matches.first().expect("first match");
    assert_eq!(matches.len(), 1);
    assert_eq!(first["path"], "src/main.rs");
    assert_eq!(first["line"], 2);
    assert_eq!(first["column"], 15);
    assert_eq!(first["snippet"], "println!(\"hello world\");");
    let events = audit.snapshot();
    assert!(events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::ToolInvocation {
                path_display,
                outcome: loong_kernel::InvocationOutcome::Completed,
                ..
            } if path_display == "read"
        )
    }));
    assert!(!events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::PlaneInvoked {
                primary_adapter,
                ..
            } if primary_adapter.starts_with("legacy:")
        )
    }));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_file_read_rejects_reserved_internal_payload_by_default() {
    let base = unique_temp_dir("loong-file-read-reserved-internal-context");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("notes.txt"), "alpha").expect("write fixture");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.read".to_owned(),
        payload: json!({
            "path": "notes.txt",
            "_loong": {
                "workspace_root": base.display().to_string()
            }
        }),
    };

    let error = execute_request_via_kernel_tool_registry(request, &config)
        .await
        .expect_err("untrusted reserved internal context should be rejected");

    assert!(
        error
            .to_string()
            .contains("payload._loong is reserved for trusted internal tool context"),
        "expected reserved internal context rejection, got: {error}"
    );
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_file_read_rejects_path_escape_through_typed_policy() {
    let base = unique_temp_dir("loong-file-read-typed-path-policy");
    let root = base.join("root");
    let outside = base.join("outside");
    fs::create_dir_all(&root).expect("create root");
    fs::create_dir_all(&outside).expect("create outside");
    fs::write(outside.join("secret.txt"), "secret").expect("write outside fixture");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.read".to_owned(),
        payload: json!({
            "path": "../outside/secret.txt"
        }),
    };

    let error = execute_request_via_kernel_tool_registry(request, &config)
        .await
        .expect_err("path escape should be denied by typed fs policy");

    let rendered = error.to_string();
    assert!(
        rendered.contains("policy_denied") || rendered.contains("escapes allowed filesystem roots"),
        "expected fs path policy denial, got: {rendered}"
    );
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_file_read_reports_typed_input_error() {
    let base = unique_temp_dir("loong-file-read-typed-error");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.read".to_owned(),
        payload: json!({
            "path": "notes.txt",
            "offset": 0
        }),
    };

    let (outcome, audit) = execute_request_via_kernel_tool_registry_result(request, &config).await;
    let error = outcome.expect_err("typed read input error should fail before execution");

    assert!(
        format!("{error}").contains("read payload.offset must be a positive integer"),
        "expected file read input error, got: {error}"
    );
    let events = audit.snapshot();
    assert!(events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::ToolInvocation {
                path_display,
                outcome:
                    loong_kernel::InvocationOutcome::Failed {
                        error_kind,
                        reason,
                    },
                ..
            } if path_display == "read"
                && error_kind == "input_error"
                && reason.contains("read payload.offset must be a positive integer")
        )
    }));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_glob_search_uses_typed_tool_registry() {
    let base = unique_temp_dir("loong-glob-search-typed-registry");
    let root = base.join("root");
    fs::create_dir_all(root.join("src/nested")).expect("create fixture dirs");
    fs::write(root.join("src/lib.rs"), "pub fn alpha() {}").expect("write lib");
    fs::write(root.join("src/nested/mod.rs"), "pub fn beta() {}").expect("write mod");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "glob.search".to_owned(),
        payload: json!({
            "pattern": "src/**/*.rs",
            "max_results": 10
        }),
    };

    let (outcome, audit) = execute_request_via_kernel_tool_registry(request, &config)
        .await
        .expect("glob.search should execute through typed registry");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool_name"], json!("glob.search"));
    assert_eq!(outcome.payload["match_count"], json!(2));
    assert_eq!(
        outcome.payload["continuation"]["recommended_tool"],
        json!("read")
    );
    let events = audit.snapshot();
    assert!(events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::ToolInvocation {
                path_display,
                outcome: loong_kernel::InvocationOutcome::Completed,
                ..
            } if path_display == "glob.search"
        )
    }));
    assert!(!events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::PlaneInvoked {
                primary_adapter,
                ..
            } if primary_adapter.starts_with("legacy:")
        )
    }));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_content_search_uses_typed_tool_registry() {
    let base = unique_temp_dir("loong-content-search-typed-registry");
    let root = base.join("root");
    fs::create_dir_all(root.join("src")).expect("create fixture dirs");
    fs::write(
        root.join("src/main.rs"),
        "fn main() {\n    println!(\"hello world\");\n}\n",
    )
    .expect("write main");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "content.search".to_owned(),
        payload: json!({
            "query": "hello world",
            "glob": "src/**/*.rs",
            "max_results": 5
        }),
    };

    let (outcome, audit) = execute_request_via_kernel_tool_registry(request, &config)
        .await
        .expect("content.search should execute through typed registry");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool_name"], json!("content.search"));
    assert_eq!(outcome.payload["match_count"], json!(1));
    assert_eq!(outcome.payload["matches"][0]["path"], json!("src/main.rs"));
    let events = audit.snapshot();
    assert!(events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::ToolInvocation {
                path_display,
                outcome: loong_kernel::InvocationOutcome::Completed,
                ..
            } if path_display == "content.search"
        )
    }));
    assert!(!events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::PlaneInvoked {
                primary_adapter,
                ..
            } if primary_adapter.starts_with("legacy:")
        )
    }));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_file_write_uses_typed_tool_registry() {
    let base = unique_temp_dir("loong-file-write-typed-registry");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");

    let config = ToolRuntimeConfig {
        file_root: Some(root.clone()),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.write".to_owned(),
        payload: json!({
            "path": "nested/notes.txt",
            "content": "alpha\nbeta\n",
        }),
    };

    let (outcome, audit) = execute_request_via_kernel_tool_registry(request, &config)
        .await
        .expect("file.write should execute through typed registry");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool_name"], json!("write"));
    let response_path = outcome.payload["path"]
        .as_str()
        .expect("response path should be a string");
    assert!(
        response_path.ends_with("/nested/notes.txt"),
        "unexpected response path: {response_path}"
    );
    assert_eq!(outcome.payload["bytes_written"], json!(11));
    assert_eq!(
        fs::read_to_string(root.join("nested/notes.txt")).expect("read written file"),
        "alpha\nbeta\n"
    );
    let events = audit.snapshot();
    assert!(events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::ToolInvocation {
                path_display,
                outcome: loong_kernel::InvocationOutcome::Completed,
                ..
            } if path_display == "write"
        )
    }));
    assert!(!events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::PlaneInvoked {
                primary_adapter,
                ..
            } if primary_adapter.starts_with("legacy:")
        )
    }));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn context_direct_write_uses_typed_tool_registry() {
    let base = unique_temp_dir("loong-context-direct-write-typed-registry");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");

    let config = ToolRuntimeConfig {
        file_root: Some(root.clone()),
        ..ToolRuntimeConfig::default()
    };
    let mut policy = PolicyPipelineBuilder::<AppContextFactory>::new_legacy_allow_fallback()
        .with_policy(crate::tools::plane::ToolInvocationAllowPolicy)
        .with_policy(FsResolvePathAllowPolicy::target())
        .with_policy(FsPathAllowedRootsPolicy::target());
    policy.push_policy(FsWriteAllowPolicy);
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut kernel = Kernel::<AppContextFactory>::with_policy_runtime(
        policy,
        Arc::new(SystemClock),
        audit.clone(),
    );
    let pack = test_pack();
    kernel.register_pack(pack).expect("register pack");
    let token = kernel
        .issue_token("test-pack", "test-agent", 60)
        .expect("issue token");
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
    )
    .expect("build app context");
    let execution_context = app_ctx
        .for_invocation(&config)
        .expect("build execution context");
    let outcome = execution_context
        .tool(loong_runtime::tool_plane::ToolPath::from("write"))
        .expect("lookup typed write")
        .invoke(json!({
            "path": "typed.txt",
            "content": "typed"
        }))
        .await
        .expect("context direct write should execute");

    assert_eq!(outcome["tool_name"], json!("write"));
    assert_eq!(outcome["bytes_written"], json!(5));
    assert_eq!(
        fs::read_to_string(root.join("typed.txt")).expect("read written file"),
        "typed"
    );
    let events = audit.snapshot();
    assert!(events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::ToolInvocation {
                path_display,
                outcome: loong_kernel::InvocationOutcome::Completed,
                ..
            } if path_display == "write"
        )
    }));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_tool_invoke_file_write_uses_typed_tool_registry() {
    let base = unique_temp_dir("loong-tool-invoke-write-typed-registry");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");

    let config = ToolRuntimeConfig {
        file_root: Some(root.clone()),
        ..ToolRuntimeConfig::default()
    };
    let request = tool_invoke_request(
        "file.write",
        json!({
            "path": "notes.txt",
            "content": "alpha",
        }),
    )
    .unwrap_or_else(|error| panic!("issue test tool lease: {error}"));

    let (outcome, audit) = execute_request_via_kernel_tool_registry(request, &config)
        .await
        .expect("tool.invoke file.write should execute through typed registry");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool_name"], json!("write"));
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).expect("read written file"),
        "alpha"
    );
    let events = audit.snapshot();
    assert!(events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::ToolInvocation {
                path_display,
                outcome: loong_kernel::InvocationOutcome::Completed,
                ..
            } if path_display == "write"
        )
    }));
    assert!(!events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::PlaneInvoked {
                primary_adapter,
                ..
            } if primary_adapter.starts_with("legacy:")
        )
    }));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_file_edit_uses_typed_tool_registry_and_preview_observer() {
    let base = unique_temp_dir("loong-file-edit-typed-registry");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    let target = root.join("notes.txt");
    fs::write(&target, "old line\nshared\n").expect("write fixture");

    let config = ToolRuntimeConfig {
        file_root: Some(root.clone()),
        ..ToolRuntimeConfig::default()
    };
    let request = make_edit_blocks_request("notes.txt", &[("old line", "new line")]);
    let sink = Arc::new(RecordingRuntimeSink::default());
    let runtime_sink: Arc<dyn ToolRuntimeEventSink> = sink.clone();

    let (outcome, audit) = with_tool_runtime_event_sink(
        runtime_sink,
        execute_request_via_kernel_tool_registry(request, &config),
    )
    .await
    .expect("file.edit should execute through typed registry");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["tool_name"], json!("edit"));
    assert_eq!(outcome.payload["replacements_made"], json!(1));
    assert_eq!(outcome.payload["edit_blocks_applied"], json!(1));
    assert_eq!(
        fs::read_to_string(&target).expect("read edited file"),
        "new line\nshared\n"
    );

    let events = lock_runtime_events(&sink);
    let preview = events.iter().find_map(|event| {
        if let ToolRuntimeEvent::FileChangePreview(preview) = event {
            return Some(preview);
        }

        None
    });
    let preview = preview.expect("typed edit should emit preview event");
    let preview_text = preview.preview.as_deref().unwrap_or_default();
    assert_eq!(preview.kind, ToolFileChangeKind::Edit);
    assert_eq!(preview.added_lines, 1);
    assert_eq!(preview.removed_lines, 1);
    assert!(preview_text.contains("-old line"));
    assert!(preview_text.contains("+new line"));

    let events = audit.snapshot();
    assert!(events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::ToolInvocation {
                path_display,
                outcome: loong_kernel::InvocationOutcome::Completed,
                ..
            } if path_display == "edit"
        )
    }));
    assert!(!events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::PlaneInvoked {
                primary_adapter,
                ..
            } if primary_adapter.starts_with("legacy:")
        )
    }));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_file_write_rejects_path_escape_through_typed_policy() {
    let base = unique_temp_dir("loong-file-write-typed-path-policy");
    let root = base.join("root");
    let outside = base.join("outside");
    fs::create_dir_all(&root).expect("create root");
    fs::create_dir_all(&outside).expect("create outside");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.write".to_owned(),
        payload: json!({
            "path": "../outside/secret.txt",
            "content": "secret"
        }),
    };

    let error = execute_request_via_kernel_tool_registry(request, &config)
        .await
        .expect_err("path escape should be denied by typed fs policy");

    let rendered = error.to_string();
    assert!(
        rendered.contains("policy_denied") || rendered.contains("escapes allowed filesystem roots"),
        "expected fs path policy denial, got: {rendered}"
    );
    assert!(
        !outside.join("secret.txt").exists(),
        "denied write must not create escaped file"
    );
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_file_write_requires_filesystem_write_capability() {
    let base = unique_temp_dir("loong-file-write-capability");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");

    let config = ToolRuntimeConfig {
        file_root: Some(root.clone()),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "write".to_owned(),
        payload: json!({
            "path": "notes.txt",
            "content": "alpha"
        }),
    };

    let error = execute_request_via_kernel_tool_registry_with_capabilities(
        request,
        &config,
        BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead]),
    )
    .await
    .expect_err("filesystem write capability should be required");

    assert!(
        error.to_string().contains("filesystem_write"),
        "expected write capability denial, got: {error}"
    );
    assert!(
        !root.join("notes.txt").exists(),
        "denied write must not create a file"
    );
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn file_read_rejects_line_offset_beyond_end_of_file() {
    let base = unique_temp_dir("loongclaw-file-read-window-bounds");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("notes.txt"), "alpha\nbeta").expect("write fixture");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.read".to_owned(),
        payload: json!({
            "path": "notes.txt",
            "offset": 3
        }),
    };

    let error = execute_file_read_with_test_context(request, &config)
        .await
        .expect_err("out-of-bounds file.read window should fail");

    assert!(error.contains("offset 3 is beyond end of file (2 lines total)"));
    let _ = fs::remove_dir_all(base);
}

#[test]
fn resolve_safe_file_path_accepts_private_var_alias_inside_root() {
    let base = unique_temp_dir("loong-file-private-var-alias");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    let child = root.join("nested.txt");
    fs::write(&child, "ok").expect("write child");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let raw = child.display().to_string();
    let normalized_raw = if raw.starts_with("/private/var/") {
        raw.replacen("/private/var/", "/var/", 1)
    } else {
        raw
    };

    let resolved = resolve_safe_file_path_with_config(&normalized_raw, &config)
        .expect("alias path under root should resolve");

    assert_eq!(
        resolved,
        dunce::canonicalize(&child).expect("canonicalize child path")
    );
    let _ = fs::remove_dir_all(base);
}

fn make_edit_blocks_request(path: &str, edits: &[(&str, &str)]) -> ToolCoreRequest {
    let edit_blocks = edits
        .iter()
        .map(|(old, new)| {
            json!({
                "old_text": old,
                "new_text": new,
            })
        })
        .collect::<Vec<_>>();
    ToolCoreRequest {
        tool_name: "file.edit".to_owned(),
        payload: json!({
            "path": path,
            "edits": edit_blocks,
        }),
    }
}

#[test]
fn summarize_file_change_preview_preserves_shared_middle_lines_when_appending_tail() {
    let before_lines = vec!["old line".to_owned(), "shared".to_owned()];
    let after_lines = vec![
        "new line".to_owned(),
        "shared".to_owned(),
        "extra".to_owned(),
    ];

    let (added_lines, removed_lines, preview) =
        summarize_file_change_preview(before_lines.as_slice(), after_lines.as_slice());
    let preview = preview.expect("preview should exist");

    assert_eq!(added_lines, 2);
    assert_eq!(removed_lines, 1);
    assert!(preview.contains("-old line"), "preview: {preview}");
    assert!(preview.contains("+new line"), "preview: {preview}");
    assert!(preview.contains("+extra"), "preview: {preview}");
}
