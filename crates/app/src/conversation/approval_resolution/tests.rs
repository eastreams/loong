use serde_json::json;

use super::*;
use crate::session::repository::ApprovalRequestStatus;

#[test]
fn approval_replay_rejects_persisted_typed_ownership() {
    let config = LoongConfig::default();
    let owner = crate::test_support::runtime_session_for_test(
        "test-session",
        crate::tools::runtime_tool_view_from_loong_config(&config),
    );
    let context = owner.context();
    let runtime = crate::conversation::DefaultConversationRuntime::new();
    let replay_runtime =
        CoordinatorApprovalResolutionRuntime::new(&config, &context, &runtime, &owner.legacy_tools);
    let approval_request = ApprovalRequestRecord {
        approval_request_id: "approval-typed".to_owned(),
        session_id: "test-session".to_owned(),
        turn_id: "test-turn".to_owned(),
        tool_call_id: "test-call".to_owned(),
        tool_name: "read".to_owned(),
        approval_key: "tool:read".to_owned(),
        status: ApprovalRequestStatus::Approved,
        decision: Some(ApprovalDecision::ApproveOnce),
        request_payload_json: json!({
            "tool_name": "read",
            "args_json": { "path": "notes.txt" },
            "dispatch_kind": "typed",
        }),
        governance_snapshot_json: json!({}),
        requested_at: 1,
        resolved_at: Some(2),
        resolved_by_session_id: Some("test-session".to_owned()),
        executed_at: None,
        last_error: None,
    };

    let error = replay_runtime
        .replay_request(&approval_request)
        .expect_err("typed approval records must fail closed");

    assert_eq!(
        error,
        "approval_request_unsupported_dispatch_kind: typed approval replay is not supported"
    );

    let error = replay_runtime
        .ensure_resolution_allowed(&approval_request, ApprovalDecision::ApproveAlways)
        .expect_err("typed ownership must fail before approval state can be mutated");
    assert_eq!(
        error,
        "approval_request_unsupported_dispatch_kind: typed approval replay is not supported"
    );
}

#[test]
fn approval_replay_preserves_the_legacy_core_owner() {
    let config = LoongConfig::default();
    let owner = crate::test_support::runtime_session_for_test(
        "test-session",
        crate::tools::runtime_tool_view_from_loong_config(&config),
    );
    let context = owner.context();
    let runtime = crate::conversation::DefaultConversationRuntime::new();
    let replay_runtime =
        CoordinatorApprovalResolutionRuntime::new(&config, &context, &runtime, &owner.legacy_tools);
    let approval_request = ApprovalRequestRecord {
        approval_request_id: "approval-legacy-core".to_owned(),
        session_id: "test-session".to_owned(),
        turn_id: "test-turn".to_owned(),
        tool_call_id: "test-call".to_owned(),
        tool_name: "read".to_owned(),
        approval_key: "tool:read".to_owned(),
        status: ApprovalRequestStatus::Approved,
        decision: Some(ApprovalDecision::ApproveOnce),
        request_payload_json: json!({
            "tool_name": "read",
            "args_json": { "path": "notes.txt" },
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

    let replay = replay_runtime
        .replay_request(&approval_request)
        .expect("legacy replay should parse");
    let ApprovalReplayRequest::LegacyCore {
        request,
        trusted_internal_context,
    } = replay
    else {
        panic!("legacy core ownership must not be reinterpreted")
    };

    assert_eq!(request.tool_name, "read");
    assert_eq!(request.payload, json!({ "path": "notes.txt" }));
    assert!(!trusted_internal_context);
}

#[test]
fn legacy_core_replay_requires_its_trust_state() {
    let config = LoongConfig::default();
    let owner = crate::test_support::runtime_session_for_test(
        "test-session",
        crate::tools::runtime_tool_view_from_loong_config(&config),
    );
    let context = owner.context();
    let runtime = crate::conversation::DefaultConversationRuntime::new();
    let replay_runtime =
        CoordinatorApprovalResolutionRuntime::new(&config, &context, &runtime, &owner.legacy_tools);
    let approval_request = ApprovalRequestRecord {
        approval_request_id: "approval-missing-trust-state".to_owned(),
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
        }),
        governance_snapshot_json: json!({}),
        requested_at: 1,
        resolved_at: Some(2),
        resolved_by_session_id: Some("test-session".to_owned()),
        executed_at: None,
        last_error: None,
    };

    let error = replay_runtime
        .replay_request(&approval_request)
        .expect_err("legacy core replay without trust state must fail closed");

    assert_eq!(
        error,
        "approval_request_invalid_payload: missing trusted_internal_context"
    );
}
