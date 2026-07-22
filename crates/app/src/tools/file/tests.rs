use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use loong_contracts::{
    ActionExecutionEvent, AuditEvent, AuditEventKind, AuthorizationAttempt,
    AuthorizationAttemptEvent, AuthorizationPolicyEvent, AuthorizationTerminalOutcome, Capability,
    ExecutionRoute, HarnessKind, ToolPath,
};
use loong_kernel::{
    InMemoryAuditSink, Kernel, SystemClock, VerticalPackManifest,
    access::fs::{
        FsAtomicWriteAllowPolicy, FsContentSearchAllowPolicy, FsCopyFileAllowPolicy,
        FsCreateDirAllAllowPolicy, FsGlobAllowPolicy, FsInspectPathAllowPolicy,
        FsPathAllowedRootsPolicy, FsReadAllowPolicy, FsReadDirAllowPolicy,
        FsReadFilenameDenyPolicy, FsRemoveDirAllAllowPolicy, FsRemoveFileAllowPolicy,
        FsRenameAllowPolicy, FsResolvePathAllowPolicy, FsWriteAllowPolicy,
    },
    policy::PolicyPipelineBuilder,
};
use loong_runtime::tool_plane::ToolRegistration;
use serde_json::json;

use super::*;
use crate::context::RuntimeContextFactory;
use crate::tools::file_path::resolve_safe_file_path_with_config;
use crate::tools::runtime_config::ToolRuntimeConfig;
use crate::tools::runtime_events::{
    ToolFileChangeKind, ToolRuntimeEvent, ToolRuntimeEventSink, with_tool_runtime_event_sink,
};

// Contracts tests path validation; file fixtures use valid one-segment tool
// identities and focus on access, grant, and fallback behavior.
#[allow(clippy::expect_used)]
fn tool_path(segment: &str) -> ToolPath {
    ToolPath::new([segment]).expect("test tool path must be valid")
}

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

/// Join execution evidence to the authorization action that owns tool identity.
fn terminal_action_execution<'a>(
    events: &'a [AuditEvent],
    path: &ToolPath,
) -> Option<&'a ActionExecutionEvent> {
    let operation = path.to_string();
    let grant_id = events.iter().find_map(|event| {
        let AuditEventKind::Authorization { evidence } = &event.kind else {
            return None;
        };
        if evidence.action.kind != "tool.invoke" || evidence.action.operation != operation {
            return None;
        }
        let AuthorizationAttempt::Started {
            event:
                AuthorizationAttemptEvent::Policy {
                    event:
                        AuthorizationPolicyEvent::Terminal(AuthorizationTerminalOutcome::Allow {
                            grant_id,
                        }),
                    ..
                },
            ..
        } = &evidence.attempt
        else {
            return None;
        };
        Some(*grant_id)
    })?;

    events.iter().rev().find_map(|event| {
        let AuditEventKind::ActionExecution {
            grant_id: event_grant_id,
            event,
        } = &event.kind
        else {
            return None;
        };
        (*event_grant_id == grant_id && !matches!(event, ActionExecutionEvent::Started))
            .then_some(event)
    })
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

/// Owned test boundary for direct typed Tool invocation.
///
/// It centralizes policy/bootstrap setup only; tests still spell out
/// `context().tool(path).invoke(payload)` so legacy envelopes cannot creep back
/// into the asserted execution path.
struct TypedFileTestRuntime {
    runtime: Arc<loong_runtime::runtime::Runtime<RuntimeContextFactory>>,
    session: crate::context::Session,
    audit: Arc<InMemoryAuditSink>,
}

impl TypedFileTestRuntime {
    fn new(config: &ToolRuntimeConfig) -> Result<Self, String> {
        Self::with_capabilities(
            config,
            loong_contracts::Capabilities::from([
                Capability::InvokeTool,
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
            ]),
        )
    }

    fn with_capabilities(
        config: &ToolRuntimeConfig,
        capabilities: loong_contracts::Capabilities,
    ) -> Result<Self, String> {
        let audit = Arc::new(InMemoryAuditSink::default());
        let mut policy = PolicyPipelineBuilder::<RuntimeContextFactory>::new()
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
        policy.push_policy(FsAtomicWriteAllowPolicy);
        policy.push_policy(FsCopyFileAllowPolicy);
        policy.push_policy(FsCreateDirAllAllowPolicy);
        policy.push_policy(FsRemoveFileAllowPolicy);
        policy.push_policy(FsRemoveDirAllAllowPolicy);
        policy.push_policy(FsRenameAllowPolicy);
        policy.push_policy(FsInspectPathAllowPolicy);
        policy.push_policy(FsGlobAllowPolicy);
        policy.push_policy(FsReadDirAllowPolicy);
        policy.push_policy(FsContentSearchAllowPolicy);
        let kernel = Kernel::<RuntimeContextFactory>::with_policy_runtime(
            policy,
            Arc::new(SystemClock),
            audit.clone(),
        );
        let runtime = Arc::new(loong_runtime::runtime::Runtime::new(
            kernel,
            crate::tools::plane::test_builtin_tool_plane(),
        ));
        let tool_view = crate::tools::runtime_visible_tool_view(runtime.as_ref(), config, None);
        let session = crate::context::Session::root(
            runtime.as_ref(),
            "test-agent",
            "test-session",
            loong_contracts::GovernedSessionMode::MutatingCapable,
            capabilities,
            config.clone(),
            crate::memory::runtime_config::MemoryRuntimeConfig::default(),
            tool_view,
            None,
            None,
        )?;

        Ok(Self {
            runtime,
            session,
            audit,
        })
    }

    fn context(&self) -> Result<crate::Context<'_>, crate::context::ContextSessionError> {
        crate::Context::new(self.runtime.as_ref(), &self.session)
    }
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
    let fixture = TypedFileTestRuntime::new(&config).expect("typed file fixture");
    let outcome = fixture
        .context()
        .expect("typed file context")
        .tool(tool_path("read"))
        .expect("read should be registered")
        .invoke(json!({
            "path": "notes.txt",
            "offset": 2,
            "limit": 2
        }))
        .await
        .expect("file.read window should succeed");

    assert_eq!(outcome["content"], json!("beta\ngamma"));
    assert_eq!(outcome["line_start"], json!(2));
    assert_eq!(outcome["line_end"], json!(3));
    assert_eq!(outcome["total_lines"], json!(4));
    assert_eq!(outcome["next_offset"], json!(4));
    assert_eq!(outcome["truncated"], json!(false));
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
    let fixture = TypedFileTestRuntime::new(&config).expect("typed file fixture");
    let outcome = fixture
        .context()
        .expect("typed file context")
        .tool(tool_path("read"))
        .expect("read should be registered")
        .invoke(json!({
            "path": "notes.txt",
            "offset": 2,
            "limit": 1
        }))
        .await
        .expect("file.read should execute through typed registry");

    assert_eq!(outcome["content"], json!("beta"));
    assert_eq!(outcome["line_start"], json!(2));
    assert_eq!(outcome["line_end"], json!(2));
    assert!(outcome.get("adapter").is_none());
    assert!(outcome.get("tool_name").is_none());
    let events = fixture.audit.snapshot();
    assert!(matches!(
        terminal_action_execution(&events, &tool_path("read")),
        Some(ActionExecutionEvent::Completed)
    ));
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
async fn capability_override_narrows_child_access_caps() {
    let base = unique_temp_dir("loong-tool-invoke-read-capability-override");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("notes.txt"), "alpha").expect("write fixture");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let fixture = TypedFileTestRuntime::new(&config).expect("typed file fixture");
    let error = fixture
        .context()
        .expect("typed file context")
        .tool(tool_path("read"))
        .expect("read should be registered")
        .with_capabilities_override(loong_contracts::Capabilities::new())
        .invoke(json!({
            "path": "notes.txt",
        }))
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
async fn capability_override_rejects_added_capabilities() {
    let base = unique_temp_dir("loong-tool-invoke-read-capability-escalation");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let fixture = TypedFileTestRuntime::new(&config).expect("typed file fixture");
    let error = fixture
        .context()
        .expect("typed file context")
        .tool(tool_path("read"))
        .expect("read should be registered")
        .with_capabilities_override(loong_contracts::Capabilities::from([
            Capability::FilesystemWrite,
        ]))
        .invoke(json!({
            "path": "notes.txt",
        }))
        .await
        .expect_err("override must not add capabilities beyond read descriptor");

    assert!(matches!(
        error,
        loong_runtime::tool_plane::error::ToolInvocationError::CapabilityOverride(
            loong_runtime::tool_plane::error::CapabilityOverrideError {
                path,
                requested,
                declared,
            }
        ) if path == tool_path("read")
            && requested == loong_contracts::Capabilities::from([Capability::FilesystemWrite])
            && declared == loong_contracts::Capabilities::from([Capability::FilesystemRead])
    ));
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
    let fixture = TypedFileTestRuntime::new(&config).expect("typed file fixture");
    let outcome = fixture
        .context()
        .expect("typed file context")
        .tool(tool_path("read"))
        .expect("read should be registered")
        .invoke(json!({
            "pattern": "src/**/*.rs",
            "max_results": 10
        }))
        .await
        .expect("read glob should execute through typed registry");

    let matches = outcome["matches"].as_array().expect("matches array");
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0]["path"], "src/lib.rs");
    assert_eq!(matches[1]["path"], "src/nested/mod.rs");
    let events = fixture.audit.snapshot();
    assert!(matches!(
        terminal_action_execution(&events, &tool_path("read")),
        Some(ActionExecutionEvent::Completed)
    ));
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
    let fixture = TypedFileTestRuntime::new(&config).expect("typed file fixture");
    let outcome = fixture
        .context()
        .expect("typed file context")
        .tool(tool_path("read"))
        .expect("read should be registered")
        .invoke(json!({
            "query": "hello world",
            "glob": "src/**/*.rs",
            "max_results": 5
        }))
        .await
        .expect("read query should execute through typed registry");

    let matches = outcome["matches"].as_array().expect("matches array");
    let first = matches.first().expect("first match");
    assert_eq!(matches.len(), 1);
    assert_eq!(first["path"], "src/main.rs");
    assert_eq!(first["line"], 2);
    assert_eq!(first["column"], 15);
    assert_eq!(first["snippet"], "println!(\"hello world\");");
    let events = fixture.audit.snapshot();
    assert!(matches!(
        terminal_action_execution(&events, &tool_path("read")),
        Some(ActionExecutionEvent::Completed)
    ));
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
    let fixture = TypedFileTestRuntime::new(&config).expect("typed file fixture");
    let error = fixture
        .context()
        .expect("typed file context")
        .tool(tool_path("read"))
        .expect("read should be registered")
        .invoke(json!({
            "path": "../outside/secret.txt"
        }))
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
    let fixture = TypedFileTestRuntime::new(&config).expect("typed file fixture");
    let error = fixture
        .context()
        .expect("typed file context")
        .tool(tool_path("read"))
        .expect("read should be registered")
        .invoke(json!({
            "path": "notes.txt",
            "offset": 0
        }))
        .await
        .expect_err("typed read input error should fail before execution");

    assert!(matches!(
        &error,
        loong_runtime::tool_plane::error::ToolInvocationError::Dispatch {
            source: loong_runtime::tool_plane::RegisteredToolError::Input(
                loong_contracts::ToolInputError::InvalidField { field, reason }
            ),
            ..
        } if field == "offset" && reason == "must be a positive integer"
    ));
    let events = fixture.audit.snapshot();
    assert!(matches!(
        terminal_action_execution(&events, &tool_path("read")),
        Some(ActionExecutionEvent::InputRejected {
            error: loong_contracts::ToolInputError::InvalidField { field, reason },
        }) if field == "offset" && reason == "must be a positive integer"
    ));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn conversation_read_input_error_is_owned_and_audited_by_typed_tool() {
    use crate::conversation::turn_engine::{ProviderTurn, ToolIntent, TurnResult};
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::new();
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![ToolIntent {
            tool_name: "read".into(),
            args_json: json!({}),
            source: "provider_tool_call".to_owned(),
            turn_id: "typed-read-input-turn".to_owned(),
            tool_call_id: "typed-read-input-call".to_owned(),
        }],
        raw_meta: serde_json::Value::Null,
    };

    let result = harness.execute(&turn).await;

    let TurnResult::ToolError(failure) = result else {
        panic!("typed input rejection should interrupt the turn as a tool error");
    };
    assert!(matches!(
        failure.tool_input.as_deref(),
        Some(crate::conversation::turn_engine::ToolInputFailure {
            path,
            provider_name,
            error: loong_contracts::ToolInputError::MissingOneOf { fields },
            ..
        }) if path == &tool_path("read")
            && provider_name == "read"
            && fields == &["path".to_owned(), "query".to_owned(), "pattern".to_owned()]
    ));
    let events = harness.audit.snapshot();
    assert!(matches!(
        terminal_action_execution(&events, &tool_path("read")),
        Some(ActionExecutionEvent::InputRejected {
            error: loong_contracts::ToolInputError::MissingOneOf { fields }
        }) if fields == &["path".to_owned(), "query".to_owned(), "pattern".to_owned()]
    ));
}

#[tokio::test]
async fn conversation_tool_invoke_preserves_empty_capability_override() {
    use crate::conversation::turn_engine::{
        NoopLegacyToolDispatcher, ProviderTurn, ToolIntent, TurnEngine, TurnResult,
    };

    let root = unique_temp_dir("loong-conversation-tool-invoke-override");
    fs::create_dir_all(&root).expect("create fixture root");
    fs::write(root.join("notes.txt"), "authority must stay narrowed").expect("write fixture file");
    let config = ToolRuntimeConfig {
        file_root: Some(root.clone()),
        ..ToolRuntimeConfig::default()
    };
    let audit = Arc::new(InMemoryAuditSink::default());
    let policy = PolicyPipelineBuilder::<RuntimeContextFactory>::new_legacy_allow_fallback()
        .with_pre_policy(crate::tools::plane::ToolVisibilityPolicy)
        .with_policy(crate::tools::plane::ToolInvocationAllowPolicy)
        .with_policy(FsResolvePathAllowPolicy::target())
        .with_policy(FsPathAllowedRootsPolicy::target())
        .with_policy(FsReadAllowPolicy);
    let mut kernel = Kernel::<RuntimeContextFactory>::with_policy_runtime(
        policy,
        Arc::new(SystemClock),
        audit.clone(),
    );
    kernel
        .register_pack(test_pack())
        .expect("register test pack");
    let token = kernel
        .issue_token("test-pack", "test-agent", 60)
        .expect("issue test token");
    let mut tools = loong_runtime::tool_plane::ToolPlaneRegistry::new();
    // `tool.invoke` rejects provider-exposed paths. Bind the read implementation
    // to a hidden catalog path so this fixture exercises the envelope boundary.
    tools
        .register(
            tool_path("config.import"),
            ToolRegistration::discoverable("config.import"),
            loong_tools::file::ReadTool,
        )
        .expect("register hidden typed test tool");
    let runtime = Arc::new(loong_runtime::runtime::Runtime::new(kernel, tools));
    let session = crate::context::Session::root(
        runtime.as_ref(),
        "test-agent",
        "test-session",
        loong_contracts::GovernedSessionMode::MutatingCapable,
        token.allowed_capabilities.iter().copied().collect(),
        config,
        crate::memory::runtime_config::MemoryRuntimeConfig::default(),
        crate::tools::runtime_tool_view(),
        None,
        None,
    )
    .expect("build test session");
    let legacy_tools = crate::conversation::DefaultLegacyToolDispatcher::from_test_token(
        Arc::clone(&runtime),
        crate::session::store::SessionStoreConfig::default(),
        crate::config::ToolConfig::default(),
        token,
    );
    let owner = crate::test_support::TestRuntimeSession {
        runtime,
        session,
        legacy_tools,
    };
    let ctx = owner.context();
    let arguments = serde_json::Map::from_iter([("path".to_owned(), json!("notes.txt"))]);
    let lease = crate::tools::issue_tool_lease(&tool_path("config.import"), &arguments)
        .expect("hidden typed tool lease should be issued");
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![ToolIntent {
            tool_name: "tool.invoke".into(),
            args_json: json!({
                "tool_id": "config.import",
                "lease": lease,
                "arguments": arguments,
                "capabilities_override": [],
            }),
            source: "provider_tool_call".to_owned(),
            turn_id: "typed-override-turn".to_owned(),
            tool_call_id: "typed-override-call".to_owned(),
        }],
        raw_meta: serde_json::Value::Null,
    };

    let result = TurnEngine::new(1)
        .execute_turn_in_context(&turn, &ctx, &NoopLegacyToolDispatcher, None)
        .await;

    let TurnResult::FinalText(output) = result else {
        panic!("a per-tool capability denial should remain local to the batch: {result:?}");
    };
    assert!(output.contains("missing capability: FilesystemRead"));
    let events = audit.snapshot();
    assert!(matches!(
        terminal_action_execution(&events, &tool_path("config.import")),
        Some(ActionExecutionEvent::Failed { reason })
            if reason.contains("missing capability: FilesystemRead")
    ));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn tool_invoke_does_not_alias_unregistered_file_write_path() {
    use crate::conversation::turn_engine::{
        NoopLegacyToolDispatcher, ProviderTurn, ToolIntent, TurnEngine, TurnResult,
    };

    let root = tempfile::tempdir().expect("temporary file root");
    let target = root.path().join("must-not-exist.txt");
    let mut config = crate::config::LoongConfig::default();
    config.tools.file_root = Some(root.path().display().to_string());
    config.tools.consent.default_mode = crate::config::ToolConsentMode::Full;
    config.tools.approval.mode = crate::config::GovernedToolApprovalMode::Disabled;
    let owner = crate::test_support::TestRuntimeSession::from_config(
        &config,
        "unregistered-file-write-session",
        "test-agent",
        loong_contracts::GovernedSessionMode::MutatingCapable,
    )
    .expect("typed runtime session");
    let arguments = serde_json::Map::from_iter([
        ("path".to_owned(), json!(target)),
        ("content".to_owned(), json!("must not be written")),
    ]);
    let lease = crate::tools::issue_tool_lease(&tool_path("file.write"), &arguments)
        .expect("lease should preserve the requested path");
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![ToolIntent {
            tool_name: "tool.invoke".into(),
            args_json: json!({
                "tool_id": "file.write",
                "lease": lease,
                "arguments": arguments,
            }),
            source: "provider_tool_call".to_owned(),
            turn_id: "unregistered-file-write-turn".to_owned(),
            tool_call_id: "unregistered-file-write-call".to_owned(),
        }],
        raw_meta: serde_json::Value::Null,
    };
    let ctx = owner.context();

    let result = TurnEngine::new(1)
        .execute_turn_in_context(&turn, &ctx, &NoopLegacyToolDispatcher, None)
        .await;

    let TurnResult::ToolDenied(failure) = result else {
        panic!("unregistered file.write path should be denied before dispatch: {result:?}");
    };
    assert_eq!(failure.code, "tool_not_found");
    assert!(
        !target.exists(),
        "tool.invoke must not reinterpret file.write as the registered write path"
    );
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
    let fixture = TypedFileTestRuntime::new(&config).expect("typed file fixture");
    let outcome = fixture
        .context()
        .expect("typed file context")
        .tool(tool_path("glob.search"))
        .expect("glob.search should be registered")
        .invoke(json!({
            "pattern": "src/**/*.rs",
            "max_results": 10
        }))
        .await
        .expect("glob.search should execute through typed registry");

    assert!(outcome.get("adapter").is_none());
    assert!(outcome.get("tool_name").is_none());
    assert_eq!(outcome["match_count"], json!(2));
    assert_eq!(outcome["continuation"]["recommended_tool"], json!("read"));
    let events = fixture.audit.snapshot();
    assert!(matches!(
        terminal_action_execution(&events, &tool_path("glob.search")),
        Some(ActionExecutionEvent::Completed)
    ));
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
    let fixture = TypedFileTestRuntime::new(&config).expect("typed file fixture");
    let outcome = fixture
        .context()
        .expect("typed file context")
        .tool(tool_path("content.search"))
        .expect("content.search should be registered")
        .invoke(json!({
            "query": "hello world",
            "glob": "src/**/*.rs",
            "max_results": 5
        }))
        .await
        .expect("content.search should execute through typed registry");

    assert!(outcome.get("adapter").is_none());
    assert!(outcome.get("tool_name").is_none());
    assert_eq!(outcome["match_count"], json!(1));
    assert_eq!(outcome["matches"][0]["path"], json!("src/main.rs"));
    let events = fixture.audit.snapshot();
    assert!(matches!(
        terminal_action_execution(&events, &tool_path("content.search")),
        Some(ActionExecutionEvent::Completed)
    ));
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
    let fixture = TypedFileTestRuntime::new(&config).expect("typed file fixture");
    let outcome = fixture
        .context()
        .expect("typed file context")
        .tool(tool_path("write"))
        .expect("write should be registered")
        .invoke(json!({
            "path": "nested/notes.txt",
            "content": "alpha\nbeta\n",
        }))
        .await
        .expect("file.write should execute through typed registry");

    assert!(outcome.get("adapter").is_none());
    assert!(outcome.get("tool_name").is_none());
    let response_path = outcome["path"]
        .as_str()
        .expect("response path should be a string");
    assert!(
        response_path.ends_with("/nested/notes.txt"),
        "unexpected response path: {response_path}"
    );
    assert_eq!(outcome["bytes_written"], json!(11));
    assert_eq!(
        fs::read_to_string(root.join("nested/notes.txt")).expect("read written file"),
        "alpha\nbeta\n"
    );
    let events = fixture.audit.snapshot();
    assert!(matches!(
        terminal_action_execution(&events, &tool_path("write")),
        Some(ActionExecutionEvent::Completed)
    ));
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
    let fixture = TypedFileTestRuntime::new(&config).expect("typed file fixture");
    let outcome = fixture
        .context()
        .expect("typed file context")
        .tool(tool_path("write"))
        .expect("lookup typed write")
        .invoke(json!({
            "path": "typed.txt",
            "content": "typed"
        }))
        .await
        .expect("context direct write should execute");

    assert!(outcome.get("adapter").is_none());
    assert!(outcome.get("tool_name").is_none());
    assert_eq!(outcome["bytes_written"], json!(5));
    assert_eq!(
        fs::read_to_string(root.join("typed.txt")).expect("read written file"),
        "typed"
    );
    let events = fixture.audit.snapshot();
    assert!(matches!(
        terminal_action_execution(&events, &tool_path("write")),
        Some(ActionExecutionEvent::Completed)
    ));
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
    let edit_blocks = json!([{
        "old_text": "old line",
        "new_text": "new line",
    }]);
    let sink = Arc::new(RecordingRuntimeSink::default());
    let runtime_sink: Arc<dyn ToolRuntimeEventSink> = sink.clone();
    let fixture = TypedFileTestRuntime::new(&config).expect("typed file fixture");
    let context = fixture.context().expect("typed file context");
    let invocation = context
        .tool(tool_path("edit"))
        .expect("edit should be registered");

    let outcome = with_tool_runtime_event_sink(
        runtime_sink,
        invocation.invoke(json!({
            "path": "notes.txt",
            "edits": edit_blocks,
        })),
    )
    .await
    .expect("file.edit should execute through typed registry");

    assert!(outcome.get("adapter").is_none());
    assert!(outcome.get("tool_name").is_none());
    assert_eq!(outcome["replacements_made"], json!(1));
    assert_eq!(outcome["edit_blocks_applied"], json!(1));
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

    let events = fixture.audit.snapshot();
    assert!(matches!(
        terminal_action_execution(&events, &tool_path("edit")),
        Some(ActionExecutionEvent::Completed)
    ));
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
    let fixture = TypedFileTestRuntime::new(&config).expect("typed file fixture");
    let error = fixture
        .context()
        .expect("typed file context")
        .tool(tool_path("write"))
        .expect("write should be registered")
        .invoke(json!({
            "path": "../outside/secret.txt",
            "content": "secret"
        }))
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
    let fixture = TypedFileTestRuntime::with_capabilities(
        &config,
        loong_contracts::Capabilities::from([Capability::InvokeTool, Capability::FilesystemRead]),
    )
    .expect("typed file fixture");
    fixture
        .context()
        .expect("typed file context")
        .tool(tool_path("write"))
        .expect("write should be registered")
        .invoke(json!({
            "path": "notes.txt",
            "content": "alpha"
        }))
        .await
        .expect_err("filesystem write capability should be required");
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
    let fixture = TypedFileTestRuntime::new(&config).expect("typed file fixture");
    let error = fixture
        .context()
        .expect("typed file context")
        .tool(tool_path("read"))
        .expect("read should be registered")
        .invoke(json!({
            "path": "notes.txt",
            "offset": 3
        }))
        .await
        .expect_err("out-of-bounds file.read window should fail");

    assert!(
        error
            .to_string()
            .contains("offset 3 is beyond end of file (2 lines total)")
    );
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
