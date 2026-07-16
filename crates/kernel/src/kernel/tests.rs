use std::{borrow::Cow, collections::BTreeSet};

use loong_contracts::{
    AuditEventKind, AuthorizationActionSnapshot, AuthorizationAttempt, AuthorizationEvidence,
    AuthorizationScope, AuthorizationSubject, Capabilities, Capability, ExecutionRoute,
    HarnessKind, VerticalPackManifest,
};
use loong_core::policy::context::{ContextFactory, PolicyContext};

use super::Kernel;

struct MinimalContextFactory;

struct MinimalContext;

impl PolicyContext for MinimalContext {
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        static EMPTY: Capabilities = Capabilities::new();
        Cow::Borrowed(&EMPTY)
    }

    fn authorization_subject(&self) -> AuthorizationSubject {
        AuthorizationSubject {
            actor_id: "test:kernel:minimal:actor".to_owned(),
            scope: AuthorizationScope::Session {
                session_id: "test:kernel:minimal:session".to_owned(),
            },
        }
    }
}

impl ContextFactory for MinimalContextFactory {
    type Cx<'a> = MinimalContext;
}

#[test]
fn kernel_context_free_api_does_not_require_legacy_invocation_context() {
    let mut kernel = Kernel::<MinimalContextFactory>::new();
    let pack_id = "context-free";

    kernel
        .register_pack(VerticalPackManifest {
            pack_id: pack_id.to_owned(),
            domain: "kernel".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: None,
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::ObserveTelemetry]),
            metadata: Default::default(),
        })
        .expect("pack should register without an invocation context");
    assert_eq!(
        kernel
            .get_namespace(pack_id)
            .map(|namespace| namespace.pack_id.as_str()),
        Some(pack_id)
    );

    let token = kernel
        .issue_token(pack_id, "context-free-agent", 120)
        .expect("token should issue without an invocation context");
    kernel
        .revoke_token(&token.token_id, Some(&token.agent_id))
        .expect("token should revoke without an invocation context");
    kernel
        .record_audit_event(
            None,
            AuditEventKind::TokenRevoked {
                token_id: "external-token".to_owned(),
            },
        )
        .expect("audit event should record without an invocation context");
}

#[test]
fn generic_recorder_rejects_engine_owned_authorization_evidence() {
    let (kernel, audit) = Kernel::<MinimalContextFactory>::new_with_in_memory_audit();
    let error = kernel
        .record_audit_event(
            Some("forged-actor"),
            AuditEventKind::Authorization {
                evidence: AuthorizationEvidence {
                    attempt: AuthorizationAttempt::StartFailed,
                    subject: AuthorizationSubject {
                        actor_id: "different-actor".to_owned(),
                        scope: AuthorizationScope::Session {
                            session_id: "forged-session".to_owned(),
                        },
                    },
                    action: AuthorizationActionSnapshot {
                        kind: "forged.action".to_owned(),
                        operation: "forge".to_owned(),
                        resource: None,
                        required_capabilities: Vec::new(),
                    },
                },
            },
        )
        .expect_err("generic recorder must not accept authorization evidence");

    assert!(matches!(
        error,
        crate::AuditError::AuthorizationEvidenceOwnedByPolicyEngine
    ));
    assert!(audit.snapshot().is_empty());
}
