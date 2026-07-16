use std::{borrow::Cow, sync::atomic::Ordering};

use loong_contracts::{
    AuthorizationAttempt, AuthorizationAttemptEvent, AuthorizationAttemptId, AuthorizationDenial,
    AuthorizationFailure, AuthorizationPermissionAuthority, AuthorizationPermissionInteraction,
    AuthorizationPolicyEvent, AuthorizationTerminalOutcome, Capabilities, Capability,
    PermissionResolution, PolicyOutcome, PolicyReport,
};

use crate::{error::PermissionRequestError, policy::engine::PolicyEngine};

use super::test_support::*;

#[tokio::test]
// The five-event escalation trail is the assertion; splitting setup or checks
// would make the shared-attempt invariant harder to audit.
#[allow(clippy::too_many_lines)]
async fn permission_interactions_share_the_authorization_attempt() {
    let report = PolicyReport {
        evaluations: Vec::new(),
        outcome: PolicyOutcome::RequireParentPermission {
            source: policy_entry("test-parent-permission"),
            reason: Cow::Borrowed("parent approval required"),
        },
    };
    let backend = CollectingBackend::new(report.clone());
    let context = TestContext::with_permissions(
        Capabilities::from([Capability::FilesystemRead]),
        [Ok(PermissionResolution::Escalate)],
        [Ok(PermissionResolution::Approved)],
    );

    let grant = backend
        .grant(&context, TestAction)
        .await
        .expect("approved escalation should mint a grant");

    let evidence = backend.evidence.lock().expect("evidence lock");
    assert_eq!(evidence.len(), 5);
    assert!(evidence.iter().all(|item| matches!(
        &item.attempt,
        AuthorizationAttempt::Started {
            id: AuthorizationAttemptId(1),
            event: AuthorizationAttemptEvent::Policy {
                report: evidence_report,
                ..
            },
        } if evidence_report == &report
    )));
    assert!(matches!(
        &evidence[0].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                event: AuthorizationPolicyEvent::Permission(
                    AuthorizationPermissionInteraction::Requested {
                        authority: AuthorizationPermissionAuthority::Parent,
                    }
                ),
                ..
            },
            ..
        }
    ));
    assert!(matches!(
        &evidence[1].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                event: AuthorizationPolicyEvent::Permission(
                    AuthorizationPermissionInteraction::EscalatedToUser
                ),
                ..
            },
            ..
        }
    ));
    assert!(matches!(
        &evidence[2].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                event: AuthorizationPolicyEvent::Permission(
                    AuthorizationPermissionInteraction::Requested {
                        authority: AuthorizationPermissionAuthority::User,
                    }
                ),
                ..
            },
            ..
        }
    ));
    assert!(matches!(
        &evidence[3].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                event: AuthorizationPolicyEvent::Permission(
                    AuthorizationPermissionInteraction::Approved {
                        authority: AuthorizationPermissionAuthority::User,
                    }
                ),
                ..
            },
            ..
        }
    ));
    assert!(matches!(
        &evidence[4].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                event: AuthorizationPolicyEvent::Terminal(
                    AuthorizationTerminalOutcome::Allow { grant_id }
                ),
                ..
            },
            ..
        } if *grant_id == grant.id()
    ));
}

#[tokio::test]
async fn permission_denial_preserves_report_and_reason() {
    let report = user_permission_report();
    let backend = CollectingBackend::new(report.clone());
    let context = TestContext::with_permissions(
        Capabilities::from([Capability::FilesystemRead]),
        [],
        [Ok(PermissionResolution::Denied {
            reason: Cow::Borrowed("user rejected request"),
        })],
    );

    let error = backend
        .grant(&context, TestAction)
        .await
        .expect_err("permission denial must reject the grant");

    let crate::error::PolicyGrantError::PermissionDenied {
        report: error_report,
        reason,
    } = error
    else {
        panic!("expected permission denial");
    };
    assert_eq!(*error_report, report);
    assert_eq!(reason, "user rejected request");
    let evidence = backend.evidence.lock().expect("evidence lock");
    assert_eq!(evidence.len(), 3);
    assert!(matches!(
        &evidence[1].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                report: evidence_report,
                event: AuthorizationPolicyEvent::Permission(
                    AuthorizationPermissionInteraction::Denied {
                        authority: AuthorizationPermissionAuthority::User,
                        reason,
                    }
                ),
            },
            ..
        } if evidence_report == &report && reason == "user rejected request"
    ));
    assert!(matches!(
        &evidence[2].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                report: evidence_report,
                event: AuthorizationPolicyEvent::Terminal(
                    AuthorizationTerminalOutcome::Deny {
                        reason: AuthorizationDenial::Permission {
                            authority: AuthorizationPermissionAuthority::User,
                            reason,
                        },
                    }
                ),
            },
            ..
        } if evidence_report == &report && reason == "user rejected request"
    ));
}

#[tokio::test]
async fn permission_request_failure_preserves_report_reason_and_failed_interaction() {
    let report = user_permission_report();
    let backend = CollectingBackend::new(report.clone());
    let context = TestContext::with_permissions(
        Capabilities::from([Capability::FilesystemRead]),
        [],
        [Err(PermissionRequestError::Failed {
            reason: Cow::Borrowed("permission transport offline"),
        })],
    );

    let error = backend
        .grant(&context, TestAction)
        .await
        .expect_err("permission request failure must reject the grant");

    let crate::error::PolicyGrantError::PermissionRequest {
        report: error_report,
        source: PermissionRequestError::Failed { reason },
    } = error
    else {
        panic!("expected permission request failure");
    };
    assert_eq!(*error_report, report);
    assert_eq!(reason, "permission transport offline");
    let evidence = backend.evidence.lock().expect("evidence lock");
    assert_eq!(evidence.len(), 3);
    assert!(matches!(
        &evidence[1].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                report: evidence_report,
                event: AuthorizationPolicyEvent::Permission(
                    AuthorizationPermissionInteraction::Failed {
                        authority: AuthorizationPermissionAuthority::User,
                        reason,
                    }
                ),
            },
            ..
        } if evidence_report == &report
            && reason == "permission transport failed: permission transport offline"
    ));
    assert!(matches!(
        &evidence[2].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                report: evidence_report,
                event: AuthorizationPolicyEvent::Terminal(
                    AuthorizationTerminalOutcome::Failure {
                        reason: AuthorizationFailure::PermissionRequest {
                            authority: AuthorizationPermissionAuthority::User,
                            reason,
                        },
                    }
                ),
            },
            ..
        } if evidence_report == &report
            && reason == "permission transport failed: permission transport offline"
    ));
}

#[tokio::test]
async fn user_escalation_failure_preserves_report_and_reason() {
    let report = user_permission_report();
    let backend = CollectingBackend::new(report.clone());
    let context = TestContext::with_permissions(
        Capabilities::from([Capability::FilesystemRead]),
        [],
        [Ok(PermissionResolution::Escalate)],
    );

    let error = backend
        .grant(&context, TestAction)
        .await
        .expect_err("user escalation has no higher authority");

    let crate::error::PolicyGrantError::PermissionRequest {
        report: error_report,
        source: PermissionRequestError::EscalationUnavailable,
    } = error
    else {
        panic!("expected escalation failure");
    };
    assert_eq!(*error_report, report);
    let evidence = backend.evidence.lock().expect("evidence lock");
    assert_eq!(evidence.len(), 3);
    assert!(matches!(
        &evidence[0].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                event: AuthorizationPolicyEvent::Permission(
                    AuthorizationPermissionInteraction::Requested {
                        authority: AuthorizationPermissionAuthority::User,
                    }
                ),
                ..
            },
            ..
        }
    ));
    assert!(matches!(
        &evidence[1].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                event: AuthorizationPolicyEvent::Permission(
                    AuthorizationPermissionInteraction::Failed {
                        authority: AuthorizationPermissionAuthority::User,
                        reason,
                    }
                ),
                ..
            },
            ..
        } if reason == "permission escalation is unavailable for user authority"
    ));
    assert!(matches!(
        &evidence[2].attempt,
        AuthorizationAttempt::Started {
            event: AuthorizationAttemptEvent::Policy {
                report: evidence_report,
                event: AuthorizationPolicyEvent::Terminal(
                    AuthorizationTerminalOutcome::Failure {
                        reason: AuthorizationFailure::EscalationUnavailable {
                            authority: AuthorizationPermissionAuthority::User,
                        },
                    }
                ),
            },
            ..
        } if evidence_report == &report
    ));
}

#[tokio::test]
async fn capability_gates_take_single_snapshots_and_reject_split_post_permission_authority() {
    let backend = CollectingBackend::new(user_permission_report());
    let context = TestContext::with_permissions(
        Capabilities::from([Capability::FilesystemRead, Capability::FilesystemWrite]),
        [],
        [Ok(PermissionResolution::Approved)],
    );
    *context
        .post_user_permission_capability_snapshots
        .lock()
        .expect("capability snapshot lock") = [
        Capabilities::from([Capability::FilesystemRead]),
        Capabilities::from([Capability::FilesystemWrite]),
    ]
    .into_iter()
    .collect();

    let error = backend
        .grant(&context, MultiCapabilityAction)
        .await
        .expect_err("a gate must not combine capabilities from separate snapshots");

    assert!(matches!(
        error,
        crate::error::PolicyGrantError::MissingCapability {
            capability: Capability::FilesystemWrite
        }
    ));
    assert_eq!(context.capability_reads.load(Ordering::Relaxed), 2);
    assert_eq!(backend.grants.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn approved_permission_rechecks_capabilities_before_allow() {
    let report = user_permission_report();
    let backend = CollectingBackend::new(report.clone());
    let mut context = TestContext::with_permissions(
        Capabilities::from([Capability::FilesystemRead]),
        [],
        [Ok(PermissionResolution::Approved)],
    );
    context.revoke_on_user_permission = true;

    let error = backend
        .grant(&context, TestAction)
        .await
        .expect_err("revoked capability must prevent grant minting");

    assert!(matches!(
        error,
        crate::error::PolicyGrantError::MissingCapability {
            capability: Capability::FilesystemRead
        }
    ));
    assert_eq!(backend.grants.load(Ordering::Relaxed), 0);
    let evidence = backend.evidence.lock().expect("evidence lock");
    assert_eq!(evidence.len(), 3);
    assert!(matches!(
        &evidence[2].attempt,
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
