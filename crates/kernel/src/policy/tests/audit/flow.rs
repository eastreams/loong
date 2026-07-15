use super::*;

#[tokio::test]
// Keep the full permission interaction sequence in one scenario so attempt
// correlation is reviewed alongside every emitted event.
#[allow(clippy::too_many_lines)]
async fn permission_interaction_and_terminal_share_sink_and_attempt() {
    let policy = PolicyPipelineBuilder::<PermissionAuditContextFactory>::new().with_pre_policy(
        StaticAnyPolicy {
            name: "user-permission",
            decision: PolicyDecision::RequireUserPermission,
            reason: "user must approve",
        },
    );
    let audit = Arc::new(InMemoryAuditSink::default());
    let kernel = Kernel::with_policy_runtime(policy, Arc::new(FixedClock::new(42)), audit.clone());
    let ctx = PermissionAuditContext {
        capabilities: Capabilities::from([Capability::InvokeTool]),
        resolution: PermissionResolution::Approved,
    };

    kernel
        .policy_engine()
        .grant(&ctx, AuditAction::invoke_tool("tool"))
        .await
        .expect("approved permission should grant");

    let evidence = audit
        .snapshot()
        .into_iter()
        .filter_map(|event| {
            let AuditEventKind::Authorization { evidence } = event.kind else {
                return None;
            };
            Some(evidence)
        })
        .collect::<Vec<_>>();
    assert!(matches!(
        evidence.as_slice(),
        [
            AuthorizationEvidence {
                attempt: AuthorizationAttempt::Started {
                    event: AuthorizationAttemptEvent::Policy {
                        event: AuthorizationPolicyEvent::Permission(
                            AuthorizationPermissionInteraction::Requested { .. }
                        ),
                        ..
                    },
                    ..
                },
                ..
            },
            AuthorizationEvidence {
                attempt: AuthorizationAttempt::Started {
                    event: AuthorizationAttemptEvent::Policy {
                        event: AuthorizationPolicyEvent::Permission(
                            AuthorizationPermissionInteraction::Approved { .. }
                        ),
                        ..
                    },
                    ..
                },
                ..
            },
            AuthorizationEvidence {
                attempt: AuthorizationAttempt::Started {
                    event: AuthorizationAttemptEvent::Policy {
                        event: AuthorizationPolicyEvent::Terminal(
                            AuthorizationTerminalOutcome::Allow { .. }
                        ),
                        ..
                    },
                    ..
                },
                ..
            }
        ]
    ));
    let AuthorizationAttempt::Started { id: attempt_id, .. } = evidence[0].attempt else {
        panic!("permission evidence must have an attempt id");
    };
    assert!(evidence.iter().all(|item| matches!(
        item.attempt,
        AuthorizationAttempt::Started { id, .. } if id == attempt_id
    )));
}

#[tokio::test]
// Setup and both typed/legacy assertions form one evidence-boundary scenario.
#[allow(clippy::too_many_lines)]
async fn permission_denial_records_typed_terminal_without_legacy_denial() {
    let policy = PolicyPipelineBuilder::<PermissionAuditContextFactory>::new().with_pre_policy(
        StaticAnyPolicy {
            name: "user-permission",
            decision: PolicyDecision::RequireUserPermission,
            reason: "user must approve",
        },
    );
    let audit = Arc::new(InMemoryAuditSink::default());
    let kernel = Kernel::with_policy_runtime(policy, Arc::new(FixedClock::new(42)), audit.clone());
    let ctx = PermissionAuditContext {
        capabilities: Capabilities::from([Capability::InvokeTool]),
        resolution: PermissionResolution::Denied {
            reason: "user denied".into(),
        },
    };

    kernel
        .policy_engine()
        .grant(&ctx, AuditAction::invoke_tool("tool"))
        .await
        .expect_err("permission denial must reject the action");

    let events = audit.snapshot();
    assert!(events.iter().any(|event| matches!(
        event.kind,
        AuditEventKind::Authorization {
            evidence: AuthorizationEvidence {
                attempt: AuthorizationAttempt::Started {
                    event: AuthorizationAttemptEvent::Policy {
                        event: AuthorizationPolicyEvent::Terminal(
                            AuthorizationTerminalOutcome::Deny { .. }
                        ),
                        ..
                    },
                    ..
                },
                ..
            }
        }
    )));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.kind, AuditEventKind::AuthorizationDenied { .. }))
    );
}

#[tokio::test]
// The grant and persisted envelope must be compared in one visible scenario.
#[allow(clippy::too_many_lines)]
async fn kernel_bound_allow_records_authorization_with_matching_grant_id() {
    let policy =
        PolicyPipelineBuilder::<AuditContextFactory>::new().with_fallback_policy(AllowPolicy);
    let audit = Arc::new(InMemoryAuditSink::default());
    let kernel = Kernel::with_policy_runtime(policy, Arc::new(FixedClock::new(42)), audit.clone());
    let ctx = AuditContext {
        capabilities: Capabilities::from([Capability::InvokeTool]),
    };

    let grant = kernel
        .policy_engine()
        .grant(&ctx, AuditAction::invoke_tool("tool"))
        .await
        .expect("bound allow should grant");

    let events = audit
        .snapshot()
        .into_iter()
        .filter(|event| matches!(event.kind, AuditEventKind::Authorization { .. }))
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].agent_id.as_deref(),
        Some("test:kernel:audit:actor")
    );
    let AuditEventKind::Authorization { evidence } = &events[0].kind else {
        panic!("expected authorization evidence");
    };
    assert_eq!(
        evidence.subject,
        AuthorizationSubject {
            actor_id: "test:kernel:audit:actor".to_owned(),
            scope: AuthorizationScope::Session {
                session_id: "test:kernel:audit:session".to_owned(),
            },
        }
    );
    assert!(matches!(
        evidence.attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                event: AuthorizationPolicyEvent::Terminal(
                    AuthorizationTerminalOutcome::Allow { grant_id }
                ),
                ..
            },
            ..
        } if grant_id == grant.id()
    ));
}

#[tokio::test]
// Keep both writers and the resulting sequence together; extracting fixture
// helpers would obscure which operations allocate each event id.
#[allow(clippy::too_many_lines)]
async fn typed_and_legacy_events_share_one_event_sequence() {
    let policy =
        PolicyPipelineBuilder::<AuditContextFactory>::new().with_fallback_policy(AllowPolicy);
    let audit = Arc::new(InMemoryAuditSink::default());
    let kernel = Kernel::with_policy_runtime(policy, Arc::new(FixedClock::new(42)), audit.clone());
    let ctx = AuditContext {
        capabilities: Capabilities::from([Capability::InvokeTool]),
    };

    kernel
        .policy_engine()
        .grant(&ctx, AuditAction::invoke_tool("tool"))
        .await
        .expect("grant action");
    kernel
        .record_audit_event(
            Some("legacy-agent"),
            AuditEventKind::TokenRevoked {
                token_id: "legacy-event".to_owned(),
            },
        )
        .expect("record legacy event");

    let events = audit.snapshot();
    let ids = events
        .iter()
        .map(|event| event.event_id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), events.len());
    assert_eq!(
        events
            .iter()
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>(),
        vec!["evt-0000000000000001", "evt-0000000000000002"]
    );
}
