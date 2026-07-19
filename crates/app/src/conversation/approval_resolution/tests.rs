use loong_contracts::{Capabilities, ToolPath};
use loong_runtime::tool_plane::error::LookupError;
use serde_json::json;

use super::*;
use crate::conversation::runtime::DefaultConversationRuntime;
use crate::session::repository::ApprovalRequestStatus;

// Path validation is covered by contracts; these tests focus on replay owner
// selection for valid catalog identities.
#[allow(clippy::expect_used)]
fn tool_path(segment: &str) -> ToolPath {
    ToolPath::new([segment]).expect("test tool path must be valid")
}

#[test]
fn approval_replay_restores_empty_capability_override() {
    let app_ctx = crate::context::bootstrap_test_app_context("approval-replay", 60)
        .expect("bootstrap test context");
    let config = LoongConfig::default();
    let runtime = DefaultConversationRuntime::new();
    let fallback = DefaultAppToolDispatcher::runtime();
    let replay_runtime = CoordinatorApprovalResolutionRuntime::new(
        &config,
        &app_ctx,
        &runtime,
        &fallback,
        ConversationRuntimeBinding::Context(&app_ctx),
    );
    let approval_request = ApprovalRequestRecord {
        approval_request_id: "approval-replay".to_owned(),
        session_id: "test-session".to_owned(),
        turn_id: "test-turn".to_owned(),
        tool_call_id: "test-call".to_owned(),
        tool_name: "glob.search".to_owned(),
        approval_key: "tool:glob.search".to_owned(),
        status: ApprovalRequestStatus::Approved,
        decision: Some(ApprovalDecision::ApproveOnce),
        request_payload_json: json!({
            "tool_name": "glob.search",
            "args_json": {
                "pattern": "**/*.rs",
                "_loong": { "workspace_root": "/approved/workspace" },
            },
            "capabilities_override": [],
            "dispatch_kind": "typed",
            "trusted_internal_context": true,
        }),
        governance_snapshot_json: json!({}),
        requested_at: 1,
        resolved_at: Some(2),
        resolved_by_session_id: Some("test-session".to_owned()),
        executed_at: None,
        last_error: None,
    };

    let replay = replay_runtime
        .replay_request(&approval_request)
        .expect("approval replay request should parse");

    assert_eq!(replay.capabilities_override, Some(Capabilities::new()));
    assert!(replay.trusted_internal_context);
    assert_eq!(
        replay.request.payload,
        json!({
            "pattern": "**/*.rs",
            "_loong": { "workspace_root": "/approved/workspace" },
        })
    );
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn typed_approval_replay_uses_the_exact_persisted_path() {
    let app_ctx = crate::context::bootstrap_test_app_context("approval-exact-path", 60)
        .expect("bootstrap test context");
    assert!(app_ctx.runtime().tool_spec(&tool_path("read")).is_ok());
    assert!(matches!(
        app_ctx.runtime().tool_spec(&tool_path("file.read")),
        Err(LookupError::NotRegistered { .. })
    ));
    let config = LoongConfig::default();
    let runtime = DefaultConversationRuntime::new();
    let fallback = DefaultAppToolDispatcher::runtime();
    let replay_runtime = CoordinatorApprovalResolutionRuntime::new(
        &config,
        &app_ctx,
        &runtime,
        &fallback,
        ConversationRuntimeBinding::Context(&app_ctx),
    );
    let approval_request = ApprovalRequestRecord {
        approval_request_id: "approval-exact-path".to_owned(),
        session_id: "test-session".to_owned(),
        turn_id: "test-turn".to_owned(),
        tool_call_id: "test-call".to_owned(),
        tool_name: "file.read".to_owned(),
        approval_key: "tool:file.read".to_owned(),
        status: ApprovalRequestStatus::Approved,
        decision: Some(ApprovalDecision::ApproveOnce),
        request_payload_json: json!({
            "tool_name": "file.read",
            "args_json": { "path": "notes.txt" },
            "capabilities_override": null,
            "dispatch_kind": "typed",
            "trusted_internal_context": false,
        }),
        governance_snapshot_json: json!({}),
        requested_at: 1,
        resolved_at: Some(2),
        resolved_by_session_id: Some("test-session".to_owned()),
        executed_at: None,
        last_error: None,
    };

    let error = replay_runtime
        .replay_approved_request(&approval_request)
        .await
        .expect_err("typed replay must not reinterpret the persisted path through aliases");

    assert!(
        error.contains("typed tool `/file.read` is missing its runtime registration"),
        "unexpected replay error: {error}"
    );
}

#[cfg(feature = "tool-file")]
#[test]
fn approval_replay_keeps_the_persisted_owner_when_registrations_change() {
    let app_ctx = crate::context::bootstrap_test_app_context("approval-owner-replay", 60)
        .expect("bootstrap test context");
    assert!(
        app_ctx.runtime().tool_spec(&tool_path("read")).is_ok(),
        "read must currently be registered to prove replay ignores a new typed owner"
    );
    assert!(matches!(
        app_ctx.runtime().tool_spec(&tool_path("config.import")),
        Err(LookupError::NotRegistered { .. })
    ));
    let config = LoongConfig::default();
    let runtime = DefaultConversationRuntime::new();
    let fallback = DefaultAppToolDispatcher::runtime();
    let replay_runtime = CoordinatorApprovalResolutionRuntime::new(
        &config,
        &app_ctx,
        &runtime,
        &fallback,
        ConversationRuntimeBinding::Context(&app_ctx),
    );

    let legacy_request = ApprovalRequestRecord {
        approval_request_id: "approval-legacy-owner".to_owned(),
        session_id: "test-session".to_owned(),
        turn_id: "test-turn".to_owned(),
        tool_call_id: "test-legacy-call".to_owned(),
        tool_name: "read".to_owned(),
        approval_key: "tool:read".to_owned(),
        status: ApprovalRequestStatus::Approved,
        decision: Some(ApprovalDecision::ApproveOnce),
        request_payload_json: json!({
            "tool_name": "read",
            "args_json": { "path": "notes.txt" },
            "capabilities_override": null,
            "dispatch_kind": "legacy_core",
            "trusted_internal_context": false,
        }),
        governance_snapshot_json: json!({}),
        requested_at: 1,
        resolved_at: Some(2),
        resolved_by_session_id: Some("test-session".to_owned()),
        executed_at: None,
        last_error: None,
    };
    let typed_request = ApprovalRequestRecord {
        approval_request_id: "approval-typed-owner".to_owned(),
        tool_call_id: "test-typed-call".to_owned(),
        tool_name: "config.import".to_owned(),
        approval_key: "tool:config.import".to_owned(),
        request_payload_json: json!({
            "tool_name": "config.import",
            "args_json": {},
            "capabilities_override": null,
            "dispatch_kind": "typed",
            "trusted_internal_context": false,
        }),
        ..legacy_request.clone()
    };

    let legacy_replay = replay_runtime
        .replay_request(&legacy_request)
        .expect("legacy replay should parse");
    let typed_replay = replay_runtime
        .replay_request(&typed_request)
        .expect("typed replay should parse");

    assert_eq!(legacy_replay.dispatch_kind, ToolDispatchKind::LegacyCore);
    assert_eq!(typed_replay.dispatch_kind, ToolDispatchKind::Typed);
}

#[test]
fn approval_replay_rejects_missing_capability_override() {
    let app_ctx = crate::context::bootstrap_test_app_context("approval-missing-caps", 60)
        .expect("bootstrap test context");
    let config = LoongConfig::default();
    let runtime = DefaultConversationRuntime::new();
    let fallback = DefaultAppToolDispatcher::runtime();
    let replay_runtime = CoordinatorApprovalResolutionRuntime::new(
        &config,
        &app_ctx,
        &runtime,
        &fallback,
        ConversationRuntimeBinding::Context(&app_ctx),
    );
    let approval_request = ApprovalRequestRecord {
        approval_request_id: "approval-missing-caps".to_owned(),
        session_id: "test-session".to_owned(),
        turn_id: "test-turn".to_owned(),
        tool_call_id: "test-call".to_owned(),
        tool_name: "config.import".to_owned(),
        approval_key: "tool:config.import".to_owned(),
        status: ApprovalRequestStatus::Approved,
        decision: Some(ApprovalDecision::ApproveOnce),
        request_payload_json: json!({
            "tool_name": "config.import",
            "args_json": {},
            "dispatch_kind": "legacy_core",
            "trusted_internal_context": false,
        }),
        governance_snapshot_json: json!({}),
        requested_at: 1,
        resolved_at: Some(2),
        resolved_by_session_id: Some("test-session".to_owned()),
        executed_at: None,
        last_error: None,
    };

    let error = match replay_runtime.replay_request(&approval_request) {
        Ok(_) => panic!("missing capability ownership must fail closed"),
        Err(error) => error,
    };

    assert_eq!(
        error,
        "approval_request_invalid_payload: missing capabilities_override"
    );
}
