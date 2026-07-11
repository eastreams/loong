use std::{borrow::Cow, collections::BTreeSet};

use loong_contracts::{
    AuditEventKind, Capability, ExecutionRoute, HarnessKind, ToolInvocationOutcome,
    VerticalPackManifest,
};
use loong_core::policy::action::{ActionMeta, ActionMetadata};
use serde_json::json;

use super::Kernel;
use crate::test_support::{TestContextFactory, TestPolicyContext};
use crate::{AllowPolicy, InMemoryAuditSink, PolicyPipeline, SystemClock};
use std::sync::Arc;

fn kernel_with_tool_invocation_policy() -> (Kernel<TestContextFactory>, Arc<InMemoryAuditSink>) {
    let mut policy = PolicyPipeline::<TestContextFactory>::new();
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
        .granted
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
            ToolInvocationOutcome::Completed,
        )
        .expect("tool invocation audit should record");

    assert!(audit.snapshot().iter().any(|event| {
        matches!(
            &event.kind,
            AuditEventKind::ToolInvocation {
                path_display,
                outcome: ToolInvocationOutcome::Completed,
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
        .granted
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
            ToolInvocationOutcome::Failed {
                error_kind: "input".to_owned(),
                reason: "read requires payload.path".to_owned(),
            },
        )
        .expect("tool invocation audit should record");

    assert!(audit.snapshot().iter().any(|event| {
        matches!(
            &event.kind,
            AuditEventKind::ToolInvocation {
                outcome: ToolInvocationOutcome::Failed { error_kind, reason },
                ..
            } if error_kind == "input" && reason.contains("payload.path")
        )
    }));
}

#[tokio::test]
async fn record_tool_invocation_records_typed_denied_event() {
    let (mut kernel, audit) = kernel_with_tool_invocation_policy();
    register_tool_pack(&mut kernel, "typed-denied");
    let token = kernel
        .issue_token("typed-denied", "agent-typed", 120)
        .expect("token should issue");
    let path = "read";
    let policy_context = TestPolicyContext::from_token(&token, kernel.now_epoch_s());
    let grant = kernel
        .grant_action(
            "typed-denied",
            &token,
            tool_invocation_action(path, BTreeSet::from([Capability::InvokeTool])),
            &policy_context,
        )
        .await
        .expect("tool invocation should authorize");
    let audit_caps = grant
        .granted
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
            ToolInvocationOutcome::Denied {
                reason: "blocked by typed policy".to_owned(),
                report: None,
            },
        )
        .expect("tool invocation audit should record");

    assert!(audit.snapshot().iter().any(|event| {
        matches!(
            &event.kind,
            AuditEventKind::ToolInvocation {
                outcome: ToolInvocationOutcome::Denied { reason, report },
                ..
            } if reason == "blocked by typed policy" && report.is_none()
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
