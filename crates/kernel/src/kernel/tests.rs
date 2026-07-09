use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use loong_contracts::{
    Capability, ExecutionRoute, HarnessKind, ToolExecutionError, ToolInputError, ToolOutcome,
    ToolPath, ToolSpec, VerticalPackManifest,
};
use loong_core::{policy::context::ContextFactory, tool::ToolImpl};
use serde_json::{Value, json};

use super::Kernel;
use crate::{
    AuditEventKind, ExecutionPlane, PlaneTier, ToolPlaneError,
    test_support::{MockCoreTool, TestContextFactory, TestPolicyContext},
};

struct EchoTool;

#[async_trait]
impl ToolImpl<TestContextFactory> for EchoTool {
    type Input = String;
    type Output = ToolOutcome;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            path: ToolPath::from("typed.echo"),
            description: "Echo a message from the typed tool registry.".to_owned(),
            required_capabilities: BTreeSet::new(),
        }
    }

    fn parse_input(&self, payload: Value) -> Result<Self::Input, ToolInputError> {
        payload
            .get("message")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| ToolInputError::missing_field("message"))
    }

    async fn execute(
        &self,
        _ctx: &<TestContextFactory as ContextFactory>::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, ToolExecutionError> {
        Ok(ToolOutcome {
            status: "ok".to_owned(),
            payload: json!({
                "source": "typed",
                "message": input,
            }),
        })
    }
}

#[tokio::test]
async fn invoke_tool_uses_typed_registry_before_legacy_plane() {
    let (mut kernel, audit) = Kernel::<TestContextFactory>::new_with_in_memory_audit();
    register_tool_pack(&mut kernel, "typed-first");
    kernel.register_core_tool_adapter(MockCoreTool);
    let path = ToolPath::from("typed.echo");
    kernel
        .register_tool(path.clone(), EchoTool)
        .expect("typed tool should register");

    let token = kernel
        .issue_token("typed-first", "agent-typed", 120)
        .expect("token should issue");
    let outcome = kernel
        .invoke_tool(
            "typed-first",
            &token,
            &BTreeSet::from([Capability::InvokeTool]),
            &path,
            json!({ "message": "hello" }),
            TestPolicyContext::from_token(&token, kernel.now_epoch_s()),
        )
        .await
        .expect("typed tool should execute");

    assert_eq!(outcome.payload["source"], "typed");
    assert_eq!(outcome.payload["message"], "hello");

    let events = audit.snapshot();
    assert!(events.iter().any(|event| {
        matches!(
            &event.kind,
            AuditEventKind::PlaneInvoked {
                plane: ExecutionPlane::Tool,
                tier: PlaneTier::Core,
                primary_adapter,
                operation,
                ..
            } if primary_adapter == "typed-tool-plane" && operation == "typed.echo"
        )
    }));
    assert!(!events.iter().any(|event| {
        matches!(
            &event.kind,
            AuditEventKind::PlaneInvoked {
                primary_adapter,
                ..
            } if primary_adapter.starts_with("legacy:")
        )
    }));
}

#[tokio::test]
async fn invoke_tool_falls_back_to_legacy_plane_only_on_typed_miss() {
    let (mut kernel, audit) = Kernel::<TestContextFactory>::new_with_in_memory_audit();
    register_tool_pack(&mut kernel, "legacy-fallback");
    kernel.register_core_tool_adapter(MockCoreTool);
    let path = ToolPath::from("legacy.echo");

    let token = kernel
        .issue_token("legacy-fallback", "agent-legacy", 120)
        .expect("token should issue");
    let outcome = kernel
        .invoke_tool(
            "legacy-fallback",
            &token,
            &BTreeSet::from([Capability::InvokeTool]),
            &path,
            json!({ "message": "hello" }),
            TestPolicyContext::from_token(&token, kernel.now_epoch_s()),
        )
        .await
        .expect("legacy fallback should execute");

    assert_eq!(outcome.payload["tool"], "legacy.echo");
    assert_eq!(outcome.payload["payload"]["message"], "hello");

    let events = audit.snapshot();
    assert!(events.iter().any(|event| {
        matches!(
            &event.kind,
            AuditEventKind::PlaneInvoked {
                plane: ExecutionPlane::Tool,
                tier: PlaneTier::Core,
                primary_adapter,
                operation,
                ..
            } if primary_adapter == "legacy:core-tools" && operation == "legacy.echo"
        )
    }));
}

#[tokio::test]
async fn invoke_tool_does_not_fallback_when_typed_tool_returns_error() {
    let (mut kernel, audit) = Kernel::<TestContextFactory>::new_with_in_memory_audit();
    register_tool_pack(&mut kernel, "typed-error");
    kernel.register_core_tool_adapter(MockCoreTool);
    let path = ToolPath::from("typed.echo");
    kernel
        .register_tool(path.clone(), EchoTool)
        .expect("typed tool should register");

    let token = kernel
        .issue_token("typed-error", "agent-error", 120)
        .expect("token should issue");
    let error = kernel
        .invoke_tool(
            "typed-error",
            &token,
            &BTreeSet::from([Capability::InvokeTool]),
            &path,
            json!({}),
            TestPolicyContext::from_token(&token, kernel.now_epoch_s()),
        )
        .await
        .expect_err("typed input error should not fallback");

    assert!(matches!(
        error,
        crate::KernelError::ToolPlane(ToolPlaneError::Execution(reason))
            if reason.contains("missing tool input field `message`")
    ));
    assert!(
        !audit
            .snapshot()
            .iter()
            .any(|event| { matches!(&event.kind, AuditEventKind::PlaneInvoked { .. }) })
    );
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
            metadata: BTreeMap::new(),
        })
        .expect("pack should register");
}
