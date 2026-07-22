use std::borrow::Cow;

use loong_contracts::ToolPath;
use loong_contracts::{
    AuditEventKind, AuthorizationAttempt, AuthorizationAttemptEvent, AuthorizationPolicyEvent,
    AuthorizationTerminalOutcome, Capability, PolicyDecision,
};
use loong_core::policy::{
    action::{ActionMeta, ActionMetadata},
    policy::PolicyAny,
};
use serde_json::{Value, json};

use crate::config::ToolConsentMode;
use crate::context::RuntimeContextFactory;
use crate::test_support::TurnTestHarness;
use crate::tools::ToolView;

// Contracts tests path validation; plane fixtures use valid one-segment
// identities and focus on visibility and dispatch.
#[allow(clippy::expect_used)]
fn tool_path(segment: &str) -> ToolPath {
    ToolPath::new([segment]).expect("test tool path must be valid")
}

/// An arbitrary action may copy tool metadata, but that metadata is not proof
/// that it came through the runtime-owned tool invocation boundary.
struct SpoofedToolInvocation;

impl ActionMeta for SpoofedToolInvocation {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "tool.invoke",
            operation: Cow::Borrowed("write"),
            required_capabilities: Cow::Borrowed(&[Capability::InvokeTool]),
        }
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({"path": "blocked.txt"}))
    }
}

#[cfg(feature = "tool-file")]
#[test]
fn builtin_tool_plane_exposes_registered_file_paths() {
    let plane = super::test_builtin_tool_plane();
    let paths = plane.registered_paths();

    assert!(paths.contains(&tool_path("read")));
    assert!(paths.contains(&tool_path("write")));
    assert!(paths.contains(&tool_path("edit")));
    assert!(paths.contains(&tool_path("glob.search")));
    assert!(paths.contains(&tool_path("content.search")));
}

#[cfg(feature = "tool-file")]
#[test]
fn registered_file_specs_keep_write_and_edit_inputs_distinct() {
    let owner = crate::test_support::runtime_session_for_test(
        "file-specs",
        ToolView::from_legacy_paths(["write", "edit"]),
    );
    let (_, write) = owner
        .runtime
        .tool_metadata(&tool_path("write"))
        .expect("write registration");
    let (_, edit) = owner
        .runtime
        .tool_metadata(&tool_path("edit"))
        .expect("edit registration");

    let write_properties = write.input_schema["properties"]
        .as_object()
        .expect("write properties");
    assert!(write_properties.contains_key("content"));
    assert!(write_properties.contains_key("overwrite"));
    assert!(!write_properties.contains_key("edits"));
    assert_eq!(write.input_schema["required"], json!(["path", "content"]));
    assert!(
        write
            .argument_hint
            .as_deref()
            .is_some_and(|hint| hint.contains("overwrite?:boolean"))
    );

    let edit_properties = edit.input_schema["properties"]
        .as_object()
        .expect("edit properties");
    assert!(edit_properties.contains_key("edits"));
    assert!(!edit_properties.contains_key("content"));
    assert_eq!(edit.input_schema["required"], json!(["path", "edits"]));
    assert!(
        edit.argument_hint
            .as_deref()
            .is_some_and(|hint| hint.contains("edits:["))
    );
}

#[tokio::test]
async fn visibility_policy_does_not_trust_copied_tool_metadata() {
    let owner = crate::test_support::runtime_session_for_test(
        "spoofed-tool-action",
        ToolView::from_legacy_paths(["write"]),
    );
    let ctx = owner.context();

    let grant = <super::ToolVisibilityPolicy as PolicyAny<RuntimeContextFactory>>::grant(
        &super::ToolVisibilityPolicy,
        &ctx,
        &SpoofedToolInvocation,
    )
    .await;

    assert_eq!(grant.decision, PolicyDecision::Continue);
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn registered_tool_outside_session_view_is_denied_by_policy() {
    let mut harness = TurnTestHarness::new();
    harness.session.tool_view = ToolView::from_legacy_paths(["read"]);
    let ctx = harness.context();
    let invocation = ctx
        .tool(tool_path("write"))
        .expect("write remains registered globally");

    let error = invocation
        .invoke(json!({"path": "blocked.txt", "content": "blocked"}))
        .await
        .expect_err("Session tool authority must be enforced at the final grant boundary");

    assert!(matches!(
        error,
        loong_runtime::tool_plane::error::ToolInvocationError::Authorization(
            loong_core::PolicyGrantError::Denied { .. }
        )
    ));

    let events = harness.audit.snapshot();
    assert!(events.iter().any(|event| {
        let AuditEventKind::Authorization { evidence } = &event.kind else {
            return false;
        };
        let AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy { report, event },
            ..
        } = &evidence.attempt
        else {
            return false;
        };

        matches!(
            event,
            AuthorizationPolicyEvent::Terminal(AuthorizationTerminalOutcome::Deny { .. })
        ) && report.evaluations.iter().any(|evaluation| {
            evaluation.source.policy_name == "tool-visibility"
                && evaluation.grant.decision == PolicyDecision::Deny
        })
    }));
    assert!(
        !events
            .iter()
            .any(|event| matches!(&event.kind, AuditEventKind::ActionExecution { .. }))
    );
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn hidden_mutation_is_denied_before_consent_can_request_permission() {
    let root = tempfile::tempdir().expect("temporary file root");
    let path = root.path().join("blocked.txt");
    let mut config = crate::config::LoongConfig::default();
    config.tools.file_root = Some(root.path().display().to_string());
    config.tools.consent.default_mode = ToolConsentMode::Prompt;
    let mut owner = crate::test_support::TestRuntimeSession::from_config(
        &config,
        "hidden-prompt-write",
        "test-agent",
        loong_contracts::GovernedSessionMode::MutatingCapable,
    )
    .expect("prompt-consent runtime session");
    owner.session.tool_view = ToolView::from_legacy_paths(["read"]);
    let ctx = owner.context();

    let error = ctx
        .tool(tool_path("write"))
        .expect("write remains registered globally")
        .invoke(json!({"path": path, "content": "blocked"}))
        .await
        .expect_err("consent must not override the session tool view");

    assert!(matches!(
        error,
        loong_runtime::tool_plane::error::ToolInvocationError::Authorization(
            loong_core::PolicyGrantError::Denied { .. }
        )
    ));
    assert!(
        !path.exists(),
        "visibility denial must precede file creation"
    );
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn prompt_consent_blocks_typed_write_before_the_file_side_effect() {
    let root = tempfile::tempdir().expect("temporary file root");
    let path = root.path().join("blocked.txt");
    let mut config = crate::config::LoongConfig::default();
    config.tools.file_root = Some(root.path().display().to_string());
    config.tools.consent.default_mode = ToolConsentMode::Prompt;
    let owner = crate::test_support::TestRuntimeSession::from_config(
        &config,
        "prompt-write",
        "test-agent",
        loong_contracts::GovernedSessionMode::MutatingCapable,
    )
    .expect("prompt-consent runtime session");
    let ctx = owner.context();

    let error = ctx
        .tool(tool_path("write"))
        .expect("write should be registered")
        .invoke(json!({"path": path, "content": "blocked"}))
        .await
        .expect_err("missing user interaction must fail closed");

    assert!(matches!(
        error,
        loong_runtime::tool_plane::error::ToolInvocationError::Authorization(
            loong_core::PolicyGrantError::PermissionRequest {
                source: loong_core::PermissionRequestError::Unavailable { .. },
                ..
            }
        )
    ));
    assert!(!path.exists(), "policy denial must precede file creation");
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn configured_preapproval_allows_typed_write_under_prompt_consent() {
    let root = tempfile::tempdir().expect("temporary file root");
    let path = root.path().join("allowed.txt");
    let mut config = crate::config::LoongConfig::default();
    config.tools.file_root = Some(root.path().display().to_string());
    config.tools.consent.default_mode = ToolConsentMode::Prompt;
    config
        .tools
        .approval
        .approved_calls
        .push("tool:/write".to_owned());
    let owner = crate::test_support::TestRuntimeSession::from_config(
        &config,
        "preapproved-write",
        "test-agent",
        loong_contracts::GovernedSessionMode::MutatingCapable,
    )
    .expect("preapproved runtime session");
    let ctx = owner.context();

    ctx.tool(tool_path("write"))
        .expect("write should be registered")
        .invoke(json!({"path": path, "content": "allowed"}))
        .await
        .expect("configured preapproval should continue to typed access policy");

    assert_eq!(
        std::fs::read_to_string(path).expect("read written file"),
        "allowed"
    );
}
