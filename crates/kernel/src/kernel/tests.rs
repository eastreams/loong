use std::{borrow::Cow, collections::BTreeSet};

use loong_contracts::{
    AuditEventKind, AuthorizationActionSnapshot, AuthorizationAttempt, AuthorizationEvidence,
    AuthorizationScope, AuthorizationSubject, Capabilities, Capability, ExecutionRoute,
    HarnessKind, InvocationOutcome, VerticalPackManifest,
};
use loong_core::policy::{
    action::{ActionMeta, ActionMetadata},
    context::{ContextFactory, PolicyContext},
};
use serde_json::json;

use super::Kernel;
use crate::test_support::{TestContextFactory, TestPolicyContext};
use crate::{AllowPolicy, InMemoryAuditSink, SystemClock, policy::PolicyPipelineBuilder};
use std::sync::Arc;

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
        crate::KernelError::Audit(crate::AuditError::AuthorizationEvidenceOwnedByPolicyEngine)
    ));
    assert!(audit.snapshot().is_empty());
}

fn kernel_with_tool_invocation_policy() -> (Kernel<TestContextFactory>, Arc<InMemoryAuditSink>) {
    let mut policy = PolicyPipelineBuilder::<TestContextFactory>::new();
    policy.push_fallback_policy(AllowPolicy);
    let audit = Arc::new(InMemoryAuditSink::default());
    let kernel = Kernel::with_policy_runtime(policy, Arc::new(SystemClock), audit.clone());
    (kernel, audit)
}

#[tokio::test]
async fn grant_action_grants_without_recording_tool_outcome() {
    let (mut kernel, audit) = kernel_with_tool_invocation_policy();
    register_tool_pack(&mut kernel, "typed-auth");
    let token = kernel
        .issue_token("typed-auth", "agent-typed", 120)
        .expect("token should issue");

    let _authorized = kernel
        .grant_action(
            "typed-auth",
            &token,
            tool_invocation_action("read", BTreeSet::from([Capability::InvokeTool])),
            &TestPolicyContext::from_token(&token, kernel.now_epoch_s()),
        )
        .await
        .expect("tool invocation should authorize");

    assert!(
        !audit
            .snapshot()
            .iter()
            .any(|event| { matches!(event.kind, AuditEventKind::ToolInvocation { .. }) })
    );
}

#[tokio::test]
async fn grant_action_audits_legacy_pack_capability_rejection() {
    let (mut kernel, audit) = kernel_with_tool_invocation_policy();
    register_tool_pack(&mut kernel, "typed-pack-deny");
    let token = kernel
        .issue_token("typed-pack-deny", "agent-typed", 120)
        .expect("token should issue");

    let error = kernel
        .grant_action(
            "typed-pack-deny",
            &token,
            tool_invocation_action(
                "read",
                BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead]),
            ),
            &TestPolicyContext::from_token(&token, kernel.now_epoch_s()),
        )
        .await
        .expect_err("pack capability boundary must reject the grant");

    assert!(matches!(
        error,
        crate::KernelError::PackCapabilityBoundary {
            capability: Capability::FilesystemRead,
            ..
        }
    ));
    assert!(matches!(
        audit.snapshot().last().map(|event| &event.kind),
        Some(AuditEventKind::AuthorizationDenied { reason, .. })
            if reason.contains("does not grant capability FilesystemRead")
    ));
}

#[tokio::test]
async fn record_tool_invocation_records_typed_completed_event() {
    let (mut kernel, audit) = kernel_with_tool_invocation_policy();
    register_tool_pack(&mut kernel, "typed-completed");
    let token = kernel
        .issue_token("typed-completed", "agent-typed", 120)
        .expect("token should issue");
    let path = "read".to_owned();
    let ctx = TestPolicyContext::from_token(&token, kernel.now_epoch_s());
    let grant = kernel
        .grant_action(
            "typed-completed",
            &token,
            tool_invocation_action(path.as_str(), BTreeSet::from([Capability::InvokeTool])),
            &ctx,
        )
        .await
        .expect("tool invocation should authorize");
    let audit_caps = grant
        .into_granted()
        .as_ref()
        .metadata()
        .required_capabilities
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    kernel
        .record_tool_invocation(
            &ctx,
            path.clone(),
            &audit_caps,
            InvocationOutcome::Completed,
        )
        .expect("tool invocation audit should record");

    assert!(audit.snapshot().iter().any(|event| {
        matches!(
            &event.kind,
            AuditEventKind::ToolInvocation {
                path_display,
                outcome: InvocationOutcome::Completed,
                ..
            } if path_display == path.as_str()
        )
    }));
}

#[tokio::test]
async fn record_tool_invocation_records_typed_failed_event() {
    let (mut kernel, audit) = kernel_with_tool_invocation_policy();
    register_tool_pack(&mut kernel, "typed-failed");
    let token = kernel
        .issue_token("typed-failed", "agent-typed", 120)
        .expect("token should issue");
    let path = "read";
    let policy_context = TestPolicyContext::from_token(&token, kernel.now_epoch_s());
    let grant = kernel
        .grant_action(
            "typed-failed",
            &token,
            tool_invocation_action(path, BTreeSet::from([Capability::InvokeTool])),
            &policy_context,
        )
        .await
        .expect("tool invocation should authorize");
    let audit_caps = grant
        .into_granted()
        .as_ref()
        .metadata()
        .required_capabilities
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    kernel
        .record_tool_invocation(
            &policy_context,
            path.to_owned(),
            &audit_caps,
            InvocationOutcome::Failed {
                error_kind: "input".to_owned(),
                reason: "read requires payload.path".to_owned(),
            },
        )
        .expect("tool invocation audit should record");

    assert!(audit.snapshot().iter().any(|event| {
        matches!(
            &event.kind,
            AuditEventKind::ToolInvocation {
                outcome: InvocationOutcome::Failed { error_kind, reason },
                ..
            } if error_kind == "input" && reason.contains("payload.path")
        )
    }));
}

#[derive(Debug, Clone)]
struct TestAction {
    operation: String,
    required_capabilities: Vec<Capability>,
    payload: serde_json::Value,
}

impl ActionMeta for TestAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "tool.invoke",
            operation: Cow::Borrowed(self.operation.as_str()),
            required_capabilities: Cow::Borrowed(self.required_capabilities.as_slice()),
        }
    }

    fn payload(&self) -> Cow<'_, serde_json::Value> {
        Cow::Borrowed(&self.payload)
    }
}

fn tool_invocation_action(
    operation: &str,
    required_capabilities: BTreeSet<Capability>,
) -> TestAction {
    TestAction {
        operation: operation.to_owned(),
        required_capabilities: required_capabilities.into_iter().collect(),
        payload: json!({ "path": "notes.txt" }),
    }
}

fn register_tool_pack(kernel: &mut Kernel<TestContextFactory>, pack_id: &str) {
    kernel
        .register_pack(VerticalPackManifest {
            pack_id: pack_id.to_owned(),
            domain: "tools".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: None,
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
            metadata: Default::default(),
        })
        .expect("pack should register");
}
