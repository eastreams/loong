use std::collections::BTreeSet;

use loong_contracts::{
    AuditEventKind, Capability, ExecutionRoute, HarnessKind, ToolInvocationOutcome, ToolPath,
    VerticalPackManifest,
};
use loong_core::tool::ToolInvocationAction;
use serde_json::json;

use super::Kernel;
use crate::test_support::{TestContextFactory, TestPolicyContext};
use crate::{InMemoryAuditSink, PolicyPipeline, SystemClock};
use std::sync::Arc;

fn kernel_with_tool_invocation_policy() -> (Kernel<TestContextFactory>, Arc<InMemoryAuditSink>) {
    let mut policy = PolicyPipeline::<TestContextFactory>::new();
    policy.push_tool_invocation_allow_policy();
    let audit = Arc::new(InMemoryAuditSink::default());
    let kernel = Kernel::with_policy_runtime(policy, Arc::new(SystemClock), audit.clone());
    (kernel, audit)
}

#[tokio::test]
async fn grant_tool_invocation_grants_without_recording_tool_outcome() {
    let (mut kernel, audit) = kernel_with_tool_invocation_policy();
    register_tool_pack(&mut kernel, "typed-auth");
    let token = kernel
        .issue_token("typed-auth", "agent-typed", 120)
        .expect("token should issue");
    let path = ToolPath::from("read");

    let _authorized = kernel
        .grant_tool_invocation(
            "typed-auth",
            &token,
            tool_invocation_action(path, BTreeSet::from([Capability::InvokeTool])),
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
    let path = ToolPath::from("read");
    let ctx = TestPolicyContext::from_token(&token, kernel.now_epoch_s());
    let grant = kernel
        .grant_tool_invocation(
            "typed-completed",
            &token,
            tool_invocation_action(path.clone(), BTreeSet::from([Capability::InvokeTool])),
            &ctx,
        )
        .await
        .expect("tool invocation should authorize");
    let audit_caps = grant
        .granted
        .as_ref()
        .required_capabilities()
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
    let path = ToolPath::from("read");
    let policy_context = TestPolicyContext::from_token(&token, kernel.now_epoch_s());
    let grant = kernel
        .grant_tool_invocation(
            "typed-failed",
            &token,
            tool_invocation_action(path.clone(), BTreeSet::from([Capability::InvokeTool])),
            &policy_context,
        )
        .await
        .expect("tool invocation should authorize");
    let audit_caps = grant
        .granted
        .as_ref()
        .required_capabilities()
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    kernel
        .record_tool_invocation(
            &policy_context,
            path,
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
    let path = ToolPath::from("read");
    let policy_context = TestPolicyContext::from_token(&token, kernel.now_epoch_s());
    let grant = kernel
        .grant_tool_invocation(
            "typed-denied",
            &token,
            tool_invocation_action(path.clone(), BTreeSet::from([Capability::InvokeTool])),
            &policy_context,
        )
        .await
        .expect("tool invocation should authorize");
    let audit_caps = grant
        .granted
        .as_ref()
        .required_capabilities()
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    kernel
        .record_tool_invocation(
            &policy_context,
            path,
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

fn tool_invocation_action(
    path: ToolPath,
    required_capabilities: BTreeSet<Capability>,
) -> ToolInvocationAction {
    ToolInvocationAction::new(path, required_capabilities, json!({ "path": "notes.txt" }))
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
