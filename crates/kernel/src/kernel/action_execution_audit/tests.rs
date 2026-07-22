use std::{borrow::Cow, sync::Arc};

use loong_contracts::{
    ActionExecutionEvent, AuditEventKind, AuthorizationScope, AuthorizationSubject, Capabilities,
    Capability, GrantId, HistoricalToolInvocationOutcome,
};
use loong_core::{
    kernel::Kernel as CoreKernel,
    policy::{
        action::{ActionMeta, ActionMetadata},
        context::{ContextFactory, PolicyContext},
        engine::PolicyEngine,
    },
};
use serde_json::Value;

use super::super::Kernel;
use crate::{
    AllowPolicy, FixedClock, InMemoryAuditSink, errors::AuditError, policy::PolicyPipelineBuilder,
};

struct TestContextFactory;

impl ContextFactory for TestContextFactory {
    type Cx<'a> = TestContext;
}

struct TestContext;

impl PolicyContext for TestContext {
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        static EMPTY: Capabilities = Capabilities::new();
        Cow::Borrowed(&EMPTY)
    }

    fn authorization_subject(&self) -> AuthorizationSubject {
        AuthorizationSubject {
            actor_id: "actor:test:action-audit".to_owned(),
            scope: AuthorizationScope::Session {
                session_id: "session:test:action-audit".to_owned(),
            },
        }
    }
}

struct TestAction;

impl ActionMeta for TestAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "test.action",
            operation: Cow::Borrowed("run"),
            required_capabilities: Cow::Borrowed(&[]),
        }
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(Value::Null)
    }
}

fn allowing_kernel() -> (Kernel<TestContextFactory>, Arc<InMemoryAuditSink>) {
    let audit = Arc::new(InMemoryAuditSink::default());
    let kernel = Kernel::with_policy_runtime(
        PolicyPipelineBuilder::new().with_fallback_policy(AllowPolicy),
        Arc::new(FixedClock::new(1)),
        audit.clone(),
    );
    (kernel, audit)
}

#[test]
fn generic_recorder_rejects_grant_owned_and_historical_execution_evidence() {
    let (kernel, audit) = Kernel::<TestContextFactory>::new_with_in_memory_audit();
    let cases = [
        (
            AuditEventKind::ActionExecution {
                grant_id: GrantId::new(),
                event: ActionExecutionEvent::Completed,
            },
            AuditError::ActionExecutionEvidenceRequiresGrant,
        ),
        (
            AuditEventKind::ToolInvocation {
                pack_id: "historical".to_owned(),
                path_display: "read".to_owned(),
                required_capabilities: vec![Capability::InvokeTool],
                outcome: HistoricalToolInvocationOutcome::Completed,
            },
            AuditError::HistoricalToolInvocationEvidenceReadOnly,
        ),
    ];

    for (kind, expected) in cases {
        assert_eq!(
            kernel.record_audit_event(Some("forged-actor"), kind),
            Err(expected)
        );
    }
    assert!(audit.snapshot().is_empty());
}

#[tokio::test]
async fn granted_recorder_uses_minted_id_and_subject_for_any_action() {
    let (kernel, audit) = allowing_kernel();
    let grant = kernel
        .policy_engine()
        .grant(&TestContext, TestAction)
        .await
        .expect("allow policy should mint action grant")
        .into_granted();
    let grant_id = grant.id();

    kernel
        .record_granted_action_execution(&grant, ActionExecutionEvent::Started)
        .expect("started event should record");
    kernel
        .record_granted_action_execution(&grant, ActionExecutionEvent::Completed)
        .expect("completed event should record");

    let events = audit.snapshot();
    let execution = events
        .iter()
        .filter(|event| matches!(event.kind, AuditEventKind::ActionExecution { .. }))
        .collect::<Vec<_>>();
    assert_eq!(execution.len(), 2);
    assert!(execution.iter().all(|event| {
        event.agent_id.as_deref() == Some("actor:test:action-audit")
            && matches!(
                event.kind,
                AuditEventKind::ActionExecution {
                    grant_id: event_grant_id,
                    ..
                } if event_grant_id == grant_id
            )
    }));
}
