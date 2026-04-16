use std::collections::BTreeSet;
use std::time::Duration;

use loongclaw_protocol::test_support::*;
use loongclaw_protocol::*;
use tokio::io::{AsyncWriteExt, duplex, split};
use tokio::time::{sleep, timeout};

#[test]
fn route_parser_covers_standard_methods() {
    assert_eq!(
        ProtocolRoute::from_method("tools/call"),
        ProtocolRoute::ToolsCall
    );
    assert_eq!(
        ProtocolRoute::from_method("control/challenge"),
        ProtocolRoute::ControlChallenge
    );
    assert_eq!(
        ProtocolRoute::from_method("control/connect"),
        ProtocolRoute::ControlConnect
    );
    assert_eq!(
        ProtocolRoute::from_method("control/subscribe"),
        ProtocolRoute::ControlSubscribe
    );
    assert_eq!(
        ProtocolRoute::from_method("approval/resolve"),
        ProtocolRoute::ApprovalResolve
    );
    assert_eq!(
        ProtocolRoute::from_method("pairing/list"),
        ProtocolRoute::PairingList
    );
    assert_eq!(
        ProtocolRoute::from_method("pairing/resolve"),
        ProtocolRoute::PairingResolve
    );
    assert_eq!(
        ProtocolRoute::from_method("control/snapshot"),
        ProtocolRoute::ControlSnapshot
    );
    assert_eq!(
        ProtocolRoute::from_method("control/events"),
        ProtocolRoute::ControlEvents
    );
    assert_eq!(
        ProtocolRoute::from_method("task/list"),
        ProtocolRoute::TaskList
    );
    assert_eq!(
        ProtocolRoute::from_method("task/read"),
        ProtocolRoute::TaskRead
    );
    assert_eq!(
        ProtocolRoute::from_method("custom/x"),
        ProtocolRoute::Custom("custom/x".to_owned())
    );
    // Previously-standard routes now map to Custom
    assert_eq!(
        ProtocolRoute::from_method("initialize"),
        ProtocolRoute::Custom("initialize".to_owned())
    );
    assert_eq!(
        ProtocolRoute::from_method("ping"),
        ProtocolRoute::Custom("ping".to_owned())
    );
}

#[test]
fn strict_router_rejects_unknown_custom_methods() {
    let router = ProtocolRouter::strict();
    let error = router
        .resolve("internal/unsafe")
        .expect_err("strict mode should block unknown methods");
    assert!(matches!(error, RouterError::UnknownMethod(method) if method == "internal/unsafe"));
}

#[test]
fn custom_route_policy_is_applied() {
    let mut router = ProtocolRouter::strict();
    router
        .register_custom_route(
            "channel/publish",
            RoutePolicy {
                allow_anonymous: false,
                required_capability: Some("channel.publish".to_owned()),
            },
        )
        .expect("custom route registration should succeed");

    let resolved = router
        .resolve("channel/publish")
        .expect("registered custom route should resolve");
    assert_eq!(
        resolved.route,
        ProtocolRoute::Custom("channel/publish".to_owned())
    );
    assert!(!resolved.policy.allow_anonymous);
    assert_eq!(
        resolved.policy.required_capability.as_deref(),
        Some("channel.publish")
    );
}

#[test]
fn resolve_rejects_invalid_method_name() {
    let router = ProtocolRouter::default();
    let error = router
        .resolve("Tools/Call")
        .expect_err("invalid characters should be rejected");
    assert!(matches!(error, RouterError::InvalidMethod(_)));
}

#[test]
fn authorize_denies_when_capability_is_missing() {
    let router = ProtocolRouter::default();
    let resolved = router
        .resolve("tools/call")
        .expect("standard route should resolve");
    let error = router
        .authorize(
            &resolved,
            &RouteAuthorizationRequest {
                authenticated: true,
                capabilities: BTreeSet::from(["discover".to_owned()]),
            },
        )
        .expect_err("tools/call should require invoke");
    assert!(matches!(
        error,
        RouteAuthorizationError::MissingCapability {
            method,
            required_capability
        } if method == "tools/call" && required_capability == "invoke"
    ));
}

#[test]
fn authorize_allows_when_capability_matches() {
    let router = ProtocolRouter::default();
    let resolved = router
        .resolve("tools/call")
        .expect("standard route should resolve");
    let decision = router
        .authorize(
            &resolved,
            &RouteAuthorizationRequest {
                authenticated: true,
                capabilities: BTreeSet::from([" invoke ".to_owned()]),
            },
        )
        .expect("matching capability should authorize");
    assert_eq!(decision, RouteAuthorizationDecision::Allow);
}

#[test]
fn authorize_supports_wildcard_capability() {
    let router = ProtocolRouter::default();
    let resolved = router
        .resolve("tools/call")
        .expect("standard route should resolve");
    let decision = router
        .authorize(
            &resolved,
            &RouteAuthorizationRequest {
                authenticated: true,
                capabilities: BTreeSet::from(["*".to_owned()]),
            },
        )
        .expect("wildcard capability should authorize");
    assert_eq!(decision, RouteAuthorizationDecision::Allow);
}

#[test]
fn control_connect_allows_anonymous_requests() {
    let router = ProtocolRouter::default();
    let resolved = router
        .resolve("control/connect")
        .expect("control/connect should resolve");
    assert!(resolved.policy.allow_anonymous);
    assert_eq!(resolved.policy.required_capability, None);

    let decision = router
        .authorize(
            &resolved,
            &RouteAuthorizationRequest {
                authenticated: false,
                capabilities: BTreeSet::new(),
            },
        )
        .expect("anonymous connect should be allowed");
    assert_eq!(decision, RouteAuthorizationDecision::Allow);
}

#[test]
fn control_challenge_allows_anonymous_requests() {
    let router = ProtocolRouter::default();
    let resolved = router
        .resolve("control/challenge")
        .expect("control/challenge should resolve");
    assert!(resolved.policy.allow_anonymous);
    assert_eq!(resolved.policy.required_capability, None);

    let decision = router
        .authorize(
            &resolved,
            &RouteAuthorizationRequest {
                authenticated: false,
                capabilities: BTreeSet::new(),
            },
        )
        .expect("anonymous challenge should be allowed");
    assert_eq!(decision, RouteAuthorizationDecision::Allow);
}

#[test]
fn presence_read_requires_authenticated_control_read_capability() {
    let router = ProtocolRouter::default();
    let resolved = router
        .resolve("presence/read")
        .expect("presence/read should resolve");
    assert!(!resolved.policy.allow_anonymous);
    assert_eq!(
        resolved.policy.required_capability.as_deref(),
        Some("control_read")
    );

    let unauthenticated_error = router
        .authorize(
            &resolved,
            &RouteAuthorizationRequest {
                authenticated: false,
                capabilities: BTreeSet::from(["control.read".to_owned()]),
            },
        )
        .expect_err("presence/read should reject unauthenticated requests");
    assert!(matches!(
        unauthenticated_error,
        RouteAuthorizationError::Unauthenticated { method } if method == "presence/read"
    ));

    let decision = router
        .authorize(
            &resolved,
            &RouteAuthorizationRequest {
                authenticated: true,
                capabilities: BTreeSet::from(["control.read".to_owned()]),
            },
        )
        .expect("authenticated control.read should authorize");
    assert_eq!(decision, RouteAuthorizationDecision::Allow);
}

#[test]
fn task_routes_require_authenticated_control_read_capability() {
    let router = ProtocolRouter::default();

    for method in ["task/list", "task/read"] {
        let resolved = router
            .resolve(method)
            .unwrap_or_else(|error| panic!("{method} should resolve: {error}"));
        assert!(!resolved.policy.allow_anonymous);
        assert_eq!(
            resolved.policy.required_capability.as_deref(),
            Some("control_read")
        );

        let missing_capability_error = router
            .authorize(
                &resolved,
                &RouteAuthorizationRequest {
                    authenticated: true,
                    capabilities: BTreeSet::from(["control.pairing".to_owned()]),
                },
            )
            .expect_err("task routes should reject non-read capabilities");
        assert!(matches!(
            missing_capability_error,
            RouteAuthorizationError::MissingCapability {
                method: missing_method,
                required_capability
            } if missing_method == method && required_capability == "control_read"
        ));
    }
}

#[test]
fn control_subscribe_requires_authenticated_control_read_capability() {
    let router = ProtocolRouter::default();
    let resolved = router
        .resolve("control/subscribe")
        .expect("control/subscribe should resolve");
    assert!(!resolved.policy.allow_anonymous);
    assert_eq!(
        resolved.policy.required_capability.as_deref(),
        Some("control_read")
    );

    let unauthenticated = router
        .authorize(
            &resolved,
            &RouteAuthorizationRequest {
                authenticated: false,
                capabilities: BTreeSet::from(["control.read".to_owned()]),
            },
        )
        .expect_err("control/subscribe should reject unauthenticated requests");
    assert!(matches!(
        unauthenticated,
        RouteAuthorizationError::Unauthenticated { method } if method == "control/subscribe"
    ));

    let authorized = router
        .authorize(
            &resolved,
            &RouteAuthorizationRequest {
                authenticated: true,
                capabilities: BTreeSet::from(["control.read".to_owned()]),
            },
        )
        .expect("authenticated control.read should authorize");
    assert_eq!(authorized, RouteAuthorizationDecision::Allow);
}

#[test]
fn approval_resolve_requires_control_approvals_capability() {
    let router = ProtocolRouter::default();
    let resolved = router
        .resolve("approval/resolve")
        .expect("approval/resolve should resolve");
    assert_eq!(
        resolved.policy.required_capability.as_deref(),
        Some("control_approvals")
    );

    let error = router
        .authorize(
            &resolved,
            &RouteAuthorizationRequest {
                authenticated: true,
                capabilities: BTreeSet::from(["control.read".to_owned()]),
            },
        )
        .expect_err("approval/resolve should require control.approvals");
    assert!(matches!(
        error,
        RouteAuthorizationError::MissingCapability {
            method,
            required_capability
        } if method == "approval/resolve" && required_capability == "control_approvals"
    ));
}

#[test]
fn pairing_resolve_requires_control_pairing_capability() {
    let router = ProtocolRouter::default();
    let resolved = router
        .resolve("pairing/resolve")
        .expect("pairing/resolve should resolve");
    assert_eq!(
        resolved.policy.required_capability.as_deref(),
        Some("control_pairing")
    );

    let error = router
        .authorize(
            &resolved,
            &RouteAuthorizationRequest {
                authenticated: true,
                capabilities: BTreeSet::from(["control.read".to_owned()]),
            },
        )
        .expect_err("pairing/resolve should require control.pairing");
    assert!(matches!(
        error,
        RouteAuthorizationError::MissingCapability {
            method,
            required_capability
        } if method == "pairing/resolve" && required_capability == "control_pairing"
    ));
}

#[test]
fn control_plane_scope_serializes_with_dot_notation() {
    let encoded =
        serde_json::to_string(&ControlPlaneScope::OperatorRead).expect("scope should serialize");

    assert_eq!(encoded, "\"operator.read\"");
}

#[test]
fn control_plane_scope_deserializes_legacy_snake_case() {
    let decoded: ControlPlaneScope =
        serde_json::from_str("\"operator_read\"").expect("scope should deserialize");

    assert_eq!(decoded, ControlPlaneScope::OperatorRead);
}

#[test]
fn control_plane_auth_claims_debug_redacts_secret_values() {
    let claims = ControlPlaneAuthClaims {
        token: Some("shared-token".to_owned()),
        device_token: Some("device-secret".to_owned()),
        bootstrap_token: Some("bootstrap-secret".to_owned()),
        password: Some("super-secret".to_owned()),
    };

    let debug = format!("{claims:?}");

    assert!(!debug.contains("shared-token"));
    assert!(!debug.contains("device-secret"));
    assert!(!debug.contains("bootstrap-secret"));
    assert!(!debug.contains("super-secret"));
    assert!(debug.contains("Some(<redacted>)"));
}

#[test]
fn control_plane_connect_response_debug_redacts_connection_token() {
    let response = ControlPlaneConnectResponse {
        protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        principal: ControlPlanePrincipal {
            connection_id: "cp-0001".to_owned(),
            client_id: "cli".to_owned(),
            role: ControlPlaneRole::Operator,
            scopes: BTreeSet::from([ControlPlaneScope::OperatorRead]),
            device_id: Some("device-1".to_owned()),
        },
        connection_token: "cpt-secret-token".to_owned(),
        connection_token_expires_at_ms: 1_700_000_000_000,
        snapshot: ControlPlaneSnapshot {
            state_version: ControlPlaneStateVersion::default(),
            presence_count: 1,
            session_count: 2,
            pending_approval_count: 3,
            acp_session_count: 4,
            runtime_ready: true,
        },
        policy: ControlPlanePolicy {
            max_payload_bytes: 1024,
            max_buffered_bytes: 2048,
            tick_interval_ms: 15_000,
        },
    };

    let debug = format!("{response:?}");

    assert!(!debug.contains("cpt-secret-token"));
    assert!(debug.contains("<redacted>"));
}

#[test]
fn control_plane_pairing_resolve_response_debug_redacts_device_token() {
    let response = ControlPlanePairingResolveResponse {
        request: ControlPlanePairingRequestSummary {
            pairing_request_id: "pair-1".to_owned(),
            device_id: "device-1".to_owned(),
            client_id: "cli".to_owned(),
            public_key: "base64-key".to_owned(),
            role: ControlPlaneRole::Operator,
            requested_scopes: BTreeSet::from([ControlPlaneScope::OperatorRead]),
            status: ControlPlanePairingStatus::Approved,
            requested_at_ms: 1_700_000_000_000,
            resolved_at_ms: Some(1_700_000_000_123),
        },
        device_token: Some("device-token-secret".to_owned()),
    };

    let debug = format!("{response:?}");

    assert!(!debug.contains("device-token-secret"));
    assert!(debug.contains("Some(<redacted>)"));
}

#[test]
fn acp_session_list_requires_control_acp_capability() {
    let router = ProtocolRouter::default();
    let resolved = router
        .resolve("acp/session/list")
        .expect("acp/session/list should resolve");
    assert_eq!(
        resolved.policy.required_capability.as_deref(),
        Some("control_acp")
    );
}

#[test]
fn control_plane_connect_request_roundtrips_through_json() {
    let request = ControlPlaneConnectRequest {
        min_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        max_protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        client: ControlPlaneClientIdentity {
            id: "cli".to_owned(),
            version: "1.0.0".to_owned(),
            mode: "operator_ui".to_owned(),
            platform: "macos".to_owned(),
            display_name: Some("LoongClaw CLI".to_owned()),
        },
        role: ControlPlaneRole::Operator,
        scopes: BTreeSet::from([
            ControlPlaneScope::OperatorRead,
            ControlPlaneScope::OperatorApprovals,
        ]),
        caps: BTreeSet::from(["shell".to_owned()]),
        commands: BTreeSet::from(["shell.exec".to_owned()]),
        permissions: std::collections::BTreeMap::from([("shell.exec".to_owned(), true)]),
        auth: Some(ControlPlaneAuthClaims {
            token: Some("shared-token".to_owned()),
            device_token: None,
            bootstrap_token: None,
            password: None,
        }),
        device: Some(ControlPlaneDeviceIdentity {
            device_id: "device-1".to_owned(),
            public_key: "pk".to_owned(),
            signature: "sig".to_owned(),
            signed_at_ms: 42,
            nonce: "nonce-1".to_owned(),
        }),
    };

    let encoded = serde_json::to_string(&request).expect("request should serialize");
    assert!(encoded.contains("\"operator.read\""));
    assert!(encoded.contains("\"operator.approvals\""));
    let decoded: ControlPlaneConnectRequest =
        serde_json::from_str(&encoded).expect("request should deserialize");
    assert_eq!(decoded, request);
}

#[test]
fn control_plane_connect_response_roundtrips_through_json() {
    let response = ControlPlaneConnectResponse {
        protocol: CONTROL_PLANE_PROTOCOL_VERSION,
        principal: ControlPlanePrincipal {
            connection_id: "cp-0001".to_owned(),
            client_id: "cli".to_owned(),
            role: ControlPlaneRole::Operator,
            scopes: BTreeSet::from([ControlPlaneScope::OperatorRead]),
            device_id: Some("device-1".to_owned()),
        },
        connection_token: "cpt-0000000000000001-0000000000000002".to_owned(),
        connection_token_expires_at_ms: 1_700_000_000_000,
        snapshot: ControlPlaneSnapshot {
            state_version: ControlPlaneStateVersion {
                presence: 1,
                health: 2,
                sessions: 3,
                approvals: 4,
                acp: 5,
            },
            presence_count: 10,
            session_count: 11,
            pending_approval_count: 12,
            acp_session_count: 13,
            runtime_ready: true,
        },
        policy: ControlPlanePolicy {
            max_payload_bytes: 1024,
            max_buffered_bytes: 2048,
            tick_interval_ms: 15_000,
        },
    };

    let encoded = serde_json::to_string(&response).expect("response should serialize");
    let decoded: ControlPlaneConnectResponse =
        serde_json::from_str(&encoded).expect("response should deserialize");
    assert_eq!(decoded, response);
}

#[test]
fn control_plane_challenge_response_roundtrips_through_json() {
    let response = ControlPlaneChallengeResponse {
        nonce: "cpc-0000000000000001-0000000000000002".to_owned(),
        issued_at_ms: 1_700_000_000_000,
        expires_at_ms: 1_700_000_060_000,
    };

    let encoded = serde_json::to_string(&response).expect("response should serialize");
    let decoded: ControlPlaneChallengeResponse =
        serde_json::from_str(&encoded).expect("response should deserialize");
    assert_eq!(decoded, response);
}

#[test]
fn control_plane_pairing_resolve_response_roundtrips_through_json() {
    let response = ControlPlanePairingResolveResponse {
        request: ControlPlanePairingRequestSummary {
            pairing_request_id: "pair-1".to_owned(),
            device_id: "device-1".to_owned(),
            client_id: "cli".to_owned(),
            public_key: "base64-key".to_owned(),
            role: ControlPlaneRole::Operator,
            requested_scopes: BTreeSet::from([ControlPlaneScope::OperatorRead]),
            status: ControlPlanePairingStatus::Approved,
            requested_at_ms: 10,
            resolved_at_ms: Some(20),
        },
        device_token: Some("cpd-1".to_owned()),
    };

    let encoded = serde_json::to_string(&response).expect("response should serialize");
    let decoded: ControlPlanePairingResolveResponse =
        serde_json::from_str(&encoded).expect("response should deserialize");
    assert_eq!(decoded, response);
}

#[test]
fn control_plane_event_envelope_roundtrips_through_json() {
    let envelope = ControlPlaneEventEnvelope {
        event: ControlPlaneEventName::ApprovalRequested,
        seq: 7,
        state_version: Some(ControlPlaneStateVersion {
            presence: 1,
            health: 2,
            sessions: 3,
            approvals: 4,
            acp: 5,
        }),
        payload: serde_json::json!({
            "request_id": "approval-1",
            "run_id": "run-42"
        }),
    };

    let encoded = serde_json::to_string(&envelope).expect("event should serialize");
    let decoded: ControlPlaneEventEnvelope =
        serde_json::from_str(&encoded).expect("event should deserialize");
    assert_eq!(decoded, envelope);
}

#[test]
fn control_plane_snapshot_response_roundtrips_through_json() {
    let response = ControlPlaneSnapshotResponse {
        snapshot: ControlPlaneSnapshot {
            state_version: ControlPlaneStateVersion {
                presence: 1,
                health: 2,
                sessions: 3,
                approvals: 4,
                acp: 5,
            },
            presence_count: 10,
            session_count: 20,
            pending_approval_count: 3,
            acp_session_count: 4,
            runtime_ready: true,
        },
    };
    let encoded = serde_json::to_string(&response).expect("snapshot response should serialize");
    let decoded: ControlPlaneSnapshotResponse =
        serde_json::from_str(&encoded).expect("snapshot response should deserialize");
    assert_eq!(decoded, response);
}

#[test]
fn control_plane_recent_events_response_roundtrips_through_json() {
    let response = ControlPlaneRecentEventsResponse {
        events: vec![ControlPlaneEventEnvelope {
            event: ControlPlaneEventName::SessionChanged,
            seq: 9,
            state_version: Some(ControlPlaneStateVersion {
                presence: 1,
                health: 1,
                sessions: 2,
                approvals: 0,
                acp: 0,
            }),
            payload: serde_json::json!({"session_id": "root"}),
        }],
    };
    let encoded =
        serde_json::to_string(&response).expect("recent events response should serialize");
    let decoded: ControlPlaneRecentEventsResponse =
        serde_json::from_str(&encoded).expect("recent events response should deserialize");
    assert_eq!(decoded, response);
}

#[test]
fn control_plane_session_read_response_roundtrips_through_json() {
    let response = ControlPlaneSessionReadResponse {
        current_session_id: "root-session".to_owned(),
        observation: ControlPlaneSessionObservation {
            session: ControlPlaneSessionSummary {
                session_id: "child-session".to_owned(),
                kind: ControlPlaneSessionKind::DelegateChild,
                parent_session_id: Some("root-session".to_owned()),
                label: Some("Child".to_owned()),
                state: ControlPlaneSessionState::Running,
                created_at: 10,
                updated_at: 20,
                archived_at: None,
                turn_count: 3,
                last_turn_at: Some(20),
                last_error: None,
                workflow: ControlPlaneSessionWorkflow {
                    workflow_id: "root-session".to_owned(),
                    task: Some("research control plane parity".to_owned()),
                    phase: Some("execute".to_owned()),
                    operation_kind: Some("task".to_owned()),
                    operation_scope: Some("task".to_owned()),
                    task_session_id: Some("child-session".to_owned()),
                    lineage_root_session_id: Some("root-session".to_owned()),
                    lineage_depth: Some(1),
                    runtime_self_continuity: Some(ControlPlaneSessionWorkflowContinuity {
                        present: true,
                        resolved_identity_present: true,
                        session_profile_projection_present: false,
                    }),
                    binding: Some(ControlPlaneSessionWorkflowBinding {
                        session_id: "child-session".to_owned(),
                        task_id: "child-session".to_owned(),
                        mode: "advisory_only".to_owned(),
                        execution_surface: "delegate.async".to_owned(),
                        worktree: Some(ControlPlaneSessionWorkflowBindingWorktree {
                            worktree_id: "child-session".to_owned(),
                            workspace_root: "/tmp/loongclaw/control-plane/child-session".to_owned(),
                        }),
                    }),
                },
            },
            terminal_outcome: Some(ControlPlaneSessionTerminalOutcome {
                session_id: "child-session".to_owned(),
                status: "completed".to_owned(),
                payload: serde_json::json!({
                    "result": "ok",
                }),
                recorded_at: 30,
            }),
            recent_events: vec![ControlPlaneSessionEvent {
                id: 1,
                session_id: "child-session".to_owned(),
                event_kind: "delegate_started".to_owned(),
                actor_session_id: Some("root-session".to_owned()),
                payload: serde_json::json!({
                    "status": "started",
                }),
                ts: 15,
            }],
            tail_events: Vec::new(),
        },
    };
    let encoded = serde_json::to_string(&response).expect("session read response should serialize");
    let decoded: ControlPlaneSessionReadResponse =
        serde_json::from_str(&encoded).expect("session read response should deserialize");
    assert_eq!(decoded, response);
}

#[test]
fn control_plane_session_read_accepts_legacy_payload_without_workflow() {
    let legacy_payload = serde_json::json!({
        "current_session_id": "root-session",
        "observation": {
            "session": {
                "session_id": "child-session",
                "kind": "delegate_child",
                "parent_session_id": "root-session",
                "label": "Child",
                "state": "running",
                "created_at": 10,
                "updated_at": 20,
                "turn_count": 3,
                "last_turn_at": 20,
                "last_error": null
            },
            "terminal_outcome": null,
            "recent_events": [],
            "tail_events": []
        }
    });

    let decoded: ControlPlaneSessionReadResponse = serde_json::from_value(legacy_payload)
        .expect("legacy session read payload should deserialize");
    assert!(decoded.observation.session.workflow.workflow_id.is_empty());
    assert_eq!(decoded.observation.session.workflow.task, None);
    assert_eq!(decoded.observation.session.workflow.phase, None);
}

#[test]
fn control_plane_session_list_accepts_legacy_payload_without_workflow() {
    let legacy_payload = serde_json::json!({
        "current_session_id": "root-session",
        "matched_count": 1,
        "returned_count": 1,
        "sessions": [{
            "session_id": "child-session",
            "kind": "delegate_child",
            "parent_session_id": "root-session",
            "label": "Child",
            "state": "running",
            "created_at": 10,
            "updated_at": 20,
            "turn_count": 3,
            "last_turn_at": 20,
            "last_error": null
        }]
    });

    let decoded: ControlPlaneSessionListResponse = serde_json::from_value(legacy_payload)
        .expect("legacy session list payload should deserialize");
    assert_eq!(decoded.sessions.len(), 1);
    assert!(decoded.sessions[0].workflow.workflow_id.is_empty());
    assert_eq!(decoded.sessions[0].workflow.binding, None);
}

#[test]
fn control_plane_task_read_response_roundtrips_through_json() {
    let response = ControlPlaneTaskReadResponse {
        current_session_id: "root-session".to_owned(),
        task: ControlPlaneTaskSummary {
            task_id: "child-session".to_owned(),
            session_id: "child-session".to_owned(),
            scope_session_id: "root-session".to_owned(),
            label: Some("Child".to_owned()),
            session_state: "running".to_owned(),
            delegate_phase: Some("running".to_owned()),
            delegate_mode: Some("async".to_owned()),
            timeout_seconds: Some(90),
            workflow: ControlPlaneSessionWorkflow {
                workflow_id: "root-session".to_owned(),
                task: Some("research control plane parity".to_owned()),
                phase: Some("execute".to_owned()),
                operation_kind: Some("task".to_owned()),
                operation_scope: Some("task".to_owned()),
                task_session_id: Some("child-session".to_owned()),
                lineage_root_session_id: Some("root-session".to_owned()),
                lineage_depth: Some(1),
                runtime_self_continuity: Some(ControlPlaneSessionWorkflowContinuity {
                    present: true,
                    resolved_identity_present: true,
                    session_profile_projection_present: false,
                }),
                binding: Some(ControlPlaneSessionWorkflowBinding {
                    session_id: "child-session".to_owned(),
                    task_id: "child-session".to_owned(),
                    mode: "advisory_only".to_owned(),
                    execution_surface: "delegate.async".to_owned(),
                    worktree: Some(ControlPlaneSessionWorkflowBindingWorktree {
                        worktree_id: "child-session".to_owned(),
                        workspace_root: "/tmp/loongclaw/control-plane/child-session".to_owned(),
                    }),
                }),
            },
            approval_request_count: 1,
            approval_attention_count: 1,
            effective_tool_ids: vec!["file.read".to_owned()],
            effective_runtime_narrowing: serde_json::json!({}),
            last_error: None,
        },
    };
    let encoded = serde_json::to_string(&response).expect("task read response should serialize");
    let decoded: ControlPlaneTaskReadResponse =
        serde_json::from_str(&encoded).expect("task read response should deserialize");
    assert_eq!(decoded, response);
}

#[test]
fn control_plane_approval_list_response_roundtrips_through_json() {
    let response = ControlPlaneApprovalListResponse {
        current_session_id: "root-session".to_owned(),
        matched_count: 1,
        returned_count: 1,
        approvals: vec![ControlPlaneApprovalSummary {
            approval_request_id: "apr-1".to_owned(),
            session_id: "child-session".to_owned(),
            turn_id: "turn-1".to_owned(),
            tool_call_id: "call-1".to_owned(),
            tool_name: "delegate".to_owned(),
            approval_key: "tool:delegate".to_owned(),
            status: ControlPlaneApprovalRequestStatus::Pending,
            decision: Some(ControlPlaneApprovalDecision::ApproveOnce),
            requested_at: 42,
            resolved_at: Some(43),
            resolved_by_session_id: Some("root-session".to_owned()),
            executed_at: None,
            last_error: None,
            reason: Some("governed_tool_requires_approval".to_owned()),
            rule_id: Some("rule-1".to_owned()),
        }],
    };
    let encoded =
        serde_json::to_string(&response).expect("approval list response should serialize");
    let decoded: ControlPlaneApprovalListResponse =
        serde_json::from_str(&encoded).expect("approval list response should deserialize");
    assert_eq!(decoded, response);
}

#[test]
fn control_plane_acp_session_list_response_roundtrips_through_json() {
    let response = ControlPlaneAcpSessionListResponse {
        current_session_id: "root-session".to_owned(),
        matched_count: 1,
        returned_count: 1,
        sessions: vec![ControlPlaneAcpSessionMetadata {
            session_key: "agent:codex:root-session".to_owned(),
            conversation_id: Some("conversation-1".to_owned()),
            binding: Some(ControlPlaneAcpBindingScope {
                route_session_id: "root-session".to_owned(),
                channel_id: Some("feishu".to_owned()),
                account_id: Some("lark-prod".to_owned()),
                conversation_id: Some("oc_123".to_owned()),
                participant_id: Some("ou_sender_1".to_owned()),
                thread_id: Some("thread-1".to_owned()),
            }),
            activation_origin: Some(ControlPlaneAcpRoutingOrigin::ExplicitRequest),
            backend_id: "acpx".to_owned(),
            runtime_session_name: "runtime-1".to_owned(),
            working_directory: Some("/tmp/runtime".to_owned()),
            backend_session_id: Some("backend-1".to_owned()),
            agent_session_id: Some("agent-1".to_owned()),
            mode: Some(ControlPlaneAcpSessionMode::Interactive),
            state: ControlPlaneAcpSessionState::Ready,
            last_activity_ms: 42,
            last_error: None,
        }],
    };
    let encoded =
        serde_json::to_string(&response).expect("ACP session list response should serialize");
    let decoded: ControlPlaneAcpSessionListResponse =
        serde_json::from_str(&encoded).expect("ACP session list response should deserialize");
    assert_eq!(decoded, response);
}

#[test]
fn control_plane_acp_session_read_response_roundtrips_through_json() {
    let metadata = ControlPlaneAcpSessionMetadata {
        session_key: "agent:codex:root-session".to_owned(),
        conversation_id: Some("conversation-1".to_owned()),
        binding: Some(ControlPlaneAcpBindingScope {
            route_session_id: "root-session".to_owned(),
            channel_id: Some("telegram".to_owned()),
            account_id: None,
            conversation_id: Some("42".to_owned()),
            participant_id: None,
            thread_id: None,
        }),
        activation_origin: Some(ControlPlaneAcpRoutingOrigin::AutomaticDispatch),
        backend_id: "acpx".to_owned(),
        runtime_session_name: "runtime-1".to_owned(),
        working_directory: Some("/tmp/runtime".to_owned()),
        backend_session_id: Some("backend-1".to_owned()),
        agent_session_id: Some("agent-1".to_owned()),
        mode: Some(ControlPlaneAcpSessionMode::Review),
        state: ControlPlaneAcpSessionState::Busy,
        last_activity_ms: 100,
        last_error: Some("transient".to_owned()),
    };
    let response = ControlPlaneAcpSessionReadResponse {
        current_session_id: "root-session".to_owned(),
        metadata: metadata.clone(),
        status: ControlPlaneAcpSessionStatus {
            session_key: metadata.session_key.clone(),
            backend_id: metadata.backend_id.clone(),
            conversation_id: metadata.conversation_id.clone(),
            binding: metadata.binding.clone(),
            activation_origin: metadata.activation_origin,
            state: ControlPlaneAcpSessionState::Busy,
            mode: Some(ControlPlaneAcpSessionMode::Review),
            pending_turns: 2,
            active_turn_id: Some("turn-42".to_owned()),
            last_activity_ms: 100,
            last_error: Some("transient".to_owned()),
        },
    };
    let encoded =
        serde_json::to_string(&response).expect("ACP session read response should serialize");
    let decoded: ControlPlaneAcpSessionReadResponse =
        serde_json::from_str(&encoded).expect("ACP session read response should deserialize");
    assert_eq!(decoded, response);
}

#[tokio::test]
async fn channel_transport_roundtrip_delivers_frame() {
    let (left, right) =
        ChannelTransport::linked(8, test_transport_info("left"), test_transport_info("right"))
            .expect("linked transport should initialize");

    left.send(OutboundFrame {
        method: "tools/call".to_owned(),
        id: Some("req-1".to_owned()),
        payload: serde_json::json!({"tool":"search"}),
        version: PROTOCOL_VERSION,
    })
    .await
    .expect("send should succeed");

    let received = right
        .recv()
        .await
        .expect("recv should succeed")
        .expect("peer frame should be available");
    assert_eq!(received.method, "tools/call");
    assert_eq!(received.id.as_deref(), Some("req-1"));
    assert_eq!(received.payload["tool"], "search");
}

#[tokio::test]
async fn channel_transport_close_stops_future_sends() {
    let (left, _right) =
        ChannelTransport::linked(4, test_transport_info("left"), test_transport_info("right"))
            .expect("linked transport should initialize");

    left.close().await.expect("close should succeed");
    let error = left
        .send(OutboundFrame {
            method: "ping".to_owned(),
            id: None,
            payload: serde_json::json!({}),
            version: PROTOCOL_VERSION,
        })
        .await
        .expect_err("send after close should fail");
    assert!(matches!(error, TransportError::Closed));
}

#[tokio::test]
async fn channel_transport_peer_close_produces_recv_none() {
    let (left, right) =
        ChannelTransport::linked(4, test_transport_info("left"), test_transport_info("right"))
            .expect("linked transport should initialize");

    left.close().await.expect("close should succeed");
    let received = right.recv().await.expect("recv should succeed");
    assert!(received.is_none(), "peer close should end receiver stream");
}

#[tokio::test]
async fn channel_transport_applies_bounded_backpressure() {
    let (left, right) =
        ChannelTransport::linked(1, test_transport_info("left"), test_transport_info("right"))
            .expect("linked transport should initialize");

    left.send(OutboundFrame {
        method: "tools/call".to_owned(),
        id: Some("req-1".to_owned()),
        payload: serde_json::json!({"seq":1}),
        version: PROTOCOL_VERSION,
    })
    .await
    .expect("first send should fill queue");

    let blocked_send = tokio::spawn(async move {
        left.send(OutboundFrame {
            method: "tools/call".to_owned(),
            id: Some("req-2".to_owned()),
            payload: serde_json::json!({"seq":2}),
            version: PROTOCOL_VERSION,
        })
        .await
    });

    sleep(Duration::from_millis(25)).await;
    assert!(
        !blocked_send.is_finished(),
        "second send should remain blocked while queue is full"
    );

    let first = right
        .recv()
        .await
        .expect("recv should succeed")
        .expect("first frame should be present");
    assert_eq!(first.payload["seq"], 1);

    timeout(Duration::from_secs(1), blocked_send)
        .await
        .expect("blocked send should finish once queue drains")
        .expect("join should succeed")
        .expect("send should succeed after drain");

    let second = right
        .recv()
        .await
        .expect("recv should succeed")
        .expect("second frame should be present");
    assert_eq!(second.payload["seq"], 2);
}

#[test]
fn channel_transport_rejects_zero_capacity() {
    let error =
        ChannelTransport::linked(0, test_transport_info("left"), test_transport_info("right"))
            .expect_err("zero capacity must fail");
    assert_eq!(error, TransportBuildError::InvalidCapacity(0));
}

#[tokio::test]
async fn json_line_transport_roundtrip_is_bidirectional() {
    let (left_stream, right_stream) = duplex(4 * 1024);
    let (left_read, left_write) = split(left_stream);
    let (right_read, right_write) = split(right_stream);

    let left = JsonLineTransport::new(test_transport_info("json-left"), left_read, left_write);
    let right = JsonLineTransport::new(test_transport_info("json-right"), right_read, right_write);

    left.send(OutboundFrame {
        method: "tools/call".to_owned(),
        id: Some("left-1".to_owned()),
        payload: serde_json::json!({"side":"left"}),
        version: PROTOCOL_VERSION,
    })
    .await
    .expect("left send should succeed");
    let from_left = right
        .recv()
        .await
        .expect("right recv should succeed")
        .expect("right should receive frame");
    assert_eq!(from_left.method, "tools/call");
    assert_eq!(from_left.id.as_deref(), Some("left-1"));
    assert_eq!(from_left.payload["side"], "left");

    right
        .send(OutboundFrame {
            method: "resources/read".to_owned(),
            id: Some("right-1".to_owned()),
            payload: serde_json::json!({"side":"right"}),
            version: PROTOCOL_VERSION,
        })
        .await
        .expect("right send should succeed");
    let from_right = left
        .recv()
        .await
        .expect("left recv should succeed")
        .expect("left should receive frame");
    assert_eq!(from_right.method, "resources/read");
    assert_eq!(from_right.id.as_deref(), Some("right-1"));
    assert_eq!(from_right.payload["side"], "right");
}

#[tokio::test]
async fn json_line_transport_rejects_invalid_json_frame() {
    let (transport_stream, mut peer_stream) = duplex(1024);
    let (reader, writer) = split(transport_stream);
    let transport = JsonLineTransport::new(test_transport_info("json-parse"), reader, writer);

    peer_stream
        .write_all(b"{\"method\":123,\"id\":null,\"payload\":{}}\n")
        .await
        .expect("peer write should succeed");

    let error = transport
        .recv()
        .await
        .expect_err("invalid frame should fail decode");
    assert!(
        matches!(error, TransportError::Failure(ref message) if message.contains("failed to decode inbound frame")),
        "unexpected decode error: {error}"
    );
}

#[tokio::test]
async fn json_line_transport_skips_empty_lines() {
    let (transport_stream, mut peer_stream) = duplex(1024);
    let (reader, writer) = split(transport_stream);
    let transport = JsonLineTransport::new(test_transport_info("json-empty"), reader, writer);

    peer_stream
        .write_all(b"\n\n{\"method\":\"ping\",\"id\":null,\"payload\":{}}\n")
        .await
        .expect("peer write should succeed");

    let received = transport
        .recv()
        .await
        .expect("recv should succeed")
        .expect("frame should be returned");
    assert_eq!(received.method, "ping");
}

#[tokio::test]
async fn json_line_transport_close_blocks_future_sends() {
    let (left_stream, _right_stream) = duplex(1024);
    let (left_read, left_write) = split(left_stream);
    let left = JsonLineTransport::new(test_transport_info("json-close"), left_read, left_write);

    left.close().await.expect("close should succeed");
    let error = left
        .send(OutboundFrame {
            method: "ping".to_owned(),
            id: None,
            payload: serde_json::json!({}),
            version: PROTOCOL_VERSION,
        })
        .await
        .expect_err("send after close should fail");
    assert!(matches!(error, TransportError::Closed));
}

#[test]
fn frame_without_version_deserializes_with_default() {
    let json = r#"{"method":"ping","id":null,"payload":{}}"#;
    let frame: InboundFrame = serde_json::from_str(json).expect("should deserialize");
    assert_eq!(frame.version, PROTOCOL_VERSION);
}

#[test]
fn frame_with_explicit_version_is_preserved() {
    let json = format!(
        r#"{{"method":"ping","id":null,"payload":{{}},"version":{}}}"#,
        PROTOCOL_VERSION
    );
    let frame: InboundFrame = serde_json::from_str(&json).expect("should deserialize");
    assert_eq!(frame.version, PROTOCOL_VERSION);
}

#[test]
fn outbound_frame_serializes_version() {
    let frame = OutboundFrame {
        method: "ping".to_owned(),
        id: None,
        payload: serde_json::json!({}),
        version: PROTOCOL_VERSION,
    };
    let serialized = serde_json::to_value(&frame).expect("should serialize");
    assert_eq!(serialized["version"], PROTOCOL_VERSION);
}

#[tokio::test]
async fn json_line_transport_rejects_unsupported_version() {
    let (transport_stream, mut peer_stream) = duplex(1024);
    let (reader, writer) = split(transport_stream);
    let transport = JsonLineTransport::new(test_transport_info("json-version"), reader, writer);

    let future_frame = format!(
        r#"{{"method":"ping","id":null,"payload":{{}},"version":{}}}"#,
        PROTOCOL_VERSION + 1
    );
    peer_stream
        .write_all(format!("{future_frame}\n").as_bytes())
        .await
        .expect("peer write should succeed");

    let error = transport
        .recv()
        .await
        .expect_err("future version should be rejected");
    assert!(
        matches!(error, TransportError::Protocol(ref msg) if msg.contains("unsupported frame version")),
        "unexpected error: {error}"
    );
}
