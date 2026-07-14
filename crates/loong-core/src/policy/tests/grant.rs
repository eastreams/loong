use super::*;

#[tokio::test]
async fn missing_capability_is_terminally_audited_before_policy_evaluation() {
    let backend = CollectingBackend::new(allow_report());
    let context = TestContext::with_capabilities(Capabilities::new());

    let error = backend
        .grant(&context, TestAction)
        .await
        .expect_err("missing capability must deny the grant");

    assert!(matches!(
        error,
        crate::error::PolicyGrantError::MissingCapability {
            capability: Capability::FilesystemRead
        }
    ));
    assert_eq!(backend.decisions.load(Ordering::Relaxed), 0);
    let evidence = backend.evidence.lock().expect("evidence lock");
    assert_eq!(evidence.len(), 1);
    assert_eq!(
        evidence[0].attempt,
        AuthorizationAttempt::Started {
            id: AuthorizationAttemptId(1),
            event: AuthorizationAttemptEvent::CapabilityDenied {
                capability: Capability::FilesystemRead,
            },
        }
    );
    assert_eq!(
        evidence[0].subject,
        AuthorizationSubject {
            actor_id: "actor:test:core-policy".to_owned(),
            scope: AuthorizationScope::Session {
                session_id: "session:test:core-policy".to_owned(),
            },
        }
    );
    assert_eq!(
        evidence[0].action,
        AuthorizationActionSnapshot {
            kind: "test.action".to_owned(),
            operation: "read".to_owned(),
            resource: Some("fixture://core-policy".to_owned()),
            required_capabilities: vec![Capability::FilesystemRead],
        }
    );
}

#[tokio::test]
async fn allow_evidence_is_written_with_the_minted_grant_id() {
    let report = allow_report();
    let backend = CollectingBackend::new(report.clone());
    let context = TestContext::with_capabilities(Capabilities::from([Capability::FilesystemRead]));

    let grant = backend
        .grant(&context, TestAction)
        .await
        .expect("allow should mint a grant");

    let evidence = backend.evidence.lock().expect("evidence lock");
    assert_eq!(evidence.len(), 1);
    assert!(matches!(
        &evidence[0].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                report: evidence_report,
                event: AuthorizationPolicyEvent::Terminal(
                    AuthorizationTerminalOutcome::Allow { grant_id }
                ),
            },
            ..
        } if evidence_report == &report && *grant_id == grant.id
    ));
}

#[tokio::test]
async fn direct_allow_rechecks_capabilities_changed_during_decide() {
    let report = allow_report();
    let backend = CollectingBackend {
        revoke_capabilities_during_decide: true,
        ..CollectingBackend::new(report.clone())
    };
    let context = TestContext::with_capabilities(Capabilities::from([Capability::FilesystemRead]));

    let error = backend
        .grant(&context, TestAction)
        .await
        .expect_err("capability revoked during decide must prevent grant minting");

    assert!(matches!(
        error,
        crate::error::PolicyGrantError::MissingCapability {
            capability: Capability::FilesystemRead
        }
    ));
    assert_eq!(context.capability_reads.load(Ordering::Relaxed), 2);
    assert_eq!(backend.grants.load(Ordering::Relaxed), 0);
    let evidence = backend.evidence.lock().expect("evidence lock");
    assert_eq!(evidence.len(), 1);
    assert!(matches!(
        &evidence[0].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                report: evidence_report,
                event: AuthorizationPolicyEvent::Terminal(
                    AuthorizationTerminalOutcome::Deny {
                        reason: AuthorizationDenial::MissingCapability {
                            capability: Capability::FilesystemRead,
                        },
                    }
                ),
            },
            ..
        } if evidence_report == &report
    ));
}

#[tokio::test]
async fn policy_deny_preserves_report_and_reason_in_error_and_evidence() {
    let report = PolicyReport {
        evaluations: Vec::new(),
        outcome: PolicyOutcome::Deny {
            grant_source: Some(policy_entry("test-deny")),
            reason: Cow::Borrowed("blocked by test policy"),
        },
    };
    let backend = CollectingBackend::new(report.clone());
    let context = TestContext::with_capabilities(Capabilities::from([Capability::FilesystemRead]));

    let error = backend
        .grant(&context, TestAction)
        .await
        .expect_err("deny should reject the grant");

    let crate::error::PolicyGrantError::Denied {
        report: error_report,
        reason,
    } = error
    else {
        panic!("expected policy denial");
    };
    assert_eq!(*error_report, report);
    assert_eq!(reason, "blocked by test policy");
    let evidence = backend.evidence.lock().expect("evidence lock");
    assert!(matches!(
        &evidence[0].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                report: evidence_report,
                event: AuthorizationPolicyEvent::Terminal(
                    AuthorizationTerminalOutcome::Deny {
                        reason: AuthorizationDenial::Policy { reason },
                    }
                ),
            },
            ..
        } if evidence_report == &report && reason == "blocked by test policy"
    ));
}

#[tokio::test]
async fn terminal_audit_failure_returns_typed_source_without_minting_grant() {
    let backend = CollectingBackend {
        fail_write_at: Some(1),
        ..CollectingBackend::new(allow_report())
    };
    let context = TestContext::with_capabilities(Capabilities::from([Capability::FilesystemRead]));

    let error = backend
        .grant(&context, TestAction)
        .await
        .expect_err("failed terminal evidence must prevent grant minting");

    let crate::error::PolicyGrantError::Audit { evidence, .. } = &error else {
        panic!("expected audit failure");
    };
    assert!(matches!(
        &evidence.attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                event: AuthorizationPolicyEvent::Terminal(AuthorizationTerminalOutcome::Allow {
                    grant_id: GrantId(1),
                }),
                ..
            },
            ..
        }
    ));
    assert_eq!(backend.grants.load(Ordering::Relaxed), 1);
    assert!(backend.evidence.lock().expect("evidence lock").is_empty());
    assert_eq!(
        Error::source(&error).and_then(|source| source.downcast_ref::<TestAuditError>()),
        Some(&TestAuditError::Write)
    );
}

#[tokio::test]
async fn attempt_allocation_failure_records_failed_context_without_fake_id() {
    let backend = CollectingBackend {
        fail_attempt: true,
        ..CollectingBackend::new(allow_report())
    };
    let context = TestContext::with_capabilities(Capabilities::new());

    let error = backend
        .grant(&context, TestAction)
        .await
        .expect_err("attempt allocation failure must stop authorization");

    let crate::error::PolicyGrantError::IdentityAllocation { evidence, .. } = &error else {
        panic!("expected attempt identity allocation failure");
    };
    assert_eq!(evidence.attempt, AuthorizationAttempt::StartFailed);
    assert_eq!(backend.decisions.load(Ordering::Relaxed), 0);
    assert_eq!(
        Error::source(&error).and_then(|source| source.downcast_ref::<TestAuditError>()),
        Some(&TestAuditError::Attempt)
    );
    assert_eq!(backend.evidence.lock().expect("evidence lock").len(), 1);
}

#[tokio::test]
async fn attempt_allocation_and_failure_evidence_write_preserve_both_sources() {
    let backend = CollectingBackend {
        fail_attempt: true,
        fail_write_at: Some(1),
        ..CollectingBackend::new(allow_report())
    };
    let context = TestContext::with_capabilities(Capabilities::new());

    let error = backend
        .grant(&context, TestAction)
        .await
        .expect_err("both attempt allocation and failure evidence writes must be reported");

    let crate::error::PolicyGrantError::IdentityAllocationAndAudit {
        identity,
        evidence,
        allocation_source,
        audit_source,
    } = &error
    else {
        panic!("expected compound attempt identity and audit failure");
    };
    assert_eq!(*identity, AuthorizationIdentityKind::Attempt);
    assert_eq!(evidence.attempt, AuthorizationAttempt::StartFailed);
    assert_eq!(
        allocation_source.downcast_ref::<TestAuditError>(),
        Some(&TestAuditError::Attempt)
    );
    assert_eq!(
        audit_source.downcast_ref::<TestAuditError>(),
        Some(&TestAuditError::Write)
    );
    assert_eq!(backend.decisions.load(Ordering::Relaxed), 0);
    assert!(backend.evidence.lock().expect("evidence lock").is_empty());
}

#[tokio::test]
async fn grant_id_allocation_failure_is_terminally_audited() {
    let report = allow_report();
    let backend = CollectingBackend {
        fail_grant: true,
        ..CollectingBackend::new(report.clone())
    };
    let context = TestContext::with_capabilities(Capabilities::from([Capability::FilesystemRead]));

    let error = backend
        .grant(&context, TestAction)
        .await
        .expect_err("grant id allocation failure must not mint a grant");

    let crate::error::PolicyGrantError::IdentityAllocation { evidence, .. } = &error else {
        panic!("expected grant identity allocation failure");
    };
    assert!(matches!(
        &evidence.attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                report: evidence_report,
                event: AuthorizationPolicyEvent::Terminal(
                    AuthorizationTerminalOutcome::Failure {
                        reason: AuthorizationFailure::GrantAllocation,
                    }
                ),
            },
            ..
        } if evidence_report == &report
    ));
    assert_eq!(backend.grants.load(Ordering::Relaxed), 0);
    assert_eq!(
        Error::source(&error).and_then(|source| source.downcast_ref::<TestAuditError>()),
        Some(&TestAuditError::Grant)
    );
}

#[tokio::test]
async fn grant_allocation_and_failure_evidence_write_preserve_both_sources() {
    let report = allow_report();
    let backend = CollectingBackend {
        fail_grant: true,
        fail_write_at: Some(1),
        ..CollectingBackend::new(report.clone())
    };
    let context = TestContext::with_capabilities(Capabilities::from([Capability::FilesystemRead]));

    let error = backend
        .grant(&context, TestAction)
        .await
        .expect_err("both grant allocation and failure evidence writes must be reported");

    let crate::error::PolicyGrantError::IdentityAllocationAndAudit {
        identity,
        evidence,
        allocation_source,
        audit_source,
    } = &error
    else {
        panic!("expected compound grant identity and audit failure");
    };
    assert_eq!(*identity, AuthorizationIdentityKind::Grant);
    assert!(matches!(
        &evidence.attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                report: evidence_report,
                event: AuthorizationPolicyEvent::Terminal(
                    AuthorizationTerminalOutcome::Failure {
                        reason: AuthorizationFailure::GrantAllocation,
                    }
                ),
            },
            ..
        } if evidence_report == &report
    ));
    assert_eq!(
        allocation_source.downcast_ref::<TestAuditError>(),
        Some(&TestAuditError::Grant)
    );
    assert_eq!(
        audit_source.downcast_ref::<TestAuditError>(),
        Some(&TestAuditError::Write)
    );
    assert_eq!(backend.grants.load(Ordering::Relaxed), 0);
    assert!(backend.evidence.lock().expect("evidence lock").is_empty());
}
