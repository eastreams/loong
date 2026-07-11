use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use loong_contracts::{
    Capability, ToolExecutionError, ToolInputError, ToolOutcome, ToolPlaneError, ToolSpec,
};
use loong_core::{
    policy::action::ActionMeta,
    policy::context::{CapabilityContext, ContextFactory},
    policy::engine::PolicyEngine,
    tool::ToolImpl,
};
use loong_kernel::PolicyPipeline;
use serde_json::{Value, json};

use super::{AppToolPlane, ToolInvocationAction, ToolInvocationAllowPolicy, ToolPath, ToolPlane};

struct TestContextFactory;

impl ContextFactory for TestContextFactory {
    type Cx<'a> = TestContext;
}

struct TestContext;

impl CapabilityContext for TestContext {
    fn allowed_capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::new()
    }
}

struct EchoTool {
    executions: Arc<AtomicUsize>,
}

#[async_trait]
impl ToolImpl<TestContextFactory> for EchoTool {
    type Input = String;
    type Output = ToolOutcome;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            description: "Echo the provided message.".to_owned(),
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
        self.executions.fetch_add(1, Ordering::Relaxed);
        Ok(ToolOutcome {
            status: "ok".to_owned(),
            payload: json!({ "message": input }),
        })
    }
}

#[test]
fn tool_invocation_action_exposes_policy_metadata() {
    let action = ToolInvocationAction::new(
        ToolPath::from("read"),
        BTreeSet::from([Capability::InvokeTool, Capability::FilesystemRead]),
        json!({ "path": "notes.txt" }),
    );
    let metadata = action.metadata();

    assert_eq!(metadata.kind, "tool.invoke");
    assert_eq!(metadata.operation.as_ref(), "read");
    assert_eq!(
        metadata
            .required_capabilities
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([Capability::FilesystemRead, Capability::InvokeTool])
    );
    let expected_payload = json!({
        "tool_path": "read",
        "payload": { "path": "notes.txt" }
    });
    assert_eq!(action.payload().as_ref(), &expected_payload);
}

#[tokio::test]
async fn app_tool_plane_invokes_registered_tool() {
    let executions = Arc::new(AtomicUsize::new(0));
    let mut plane = AppToolPlane::<TestContextFactory>::new();
    let path = ToolPath::from("test.echo");

    plane
        .register(
            path.clone(),
            EchoTool {
                executions: executions.clone(),
            },
        )
        .expect("tool should register");
    let outcome = plane
        .invoke(
            tool_invocation_grant(path.clone(), json!({ "message": "hello" })).await,
            &TestContext,
        )
        .await
        .expect("tool should execute");

    assert!(plane.contains(&path));
    assert_eq!(plane.len(), 1);
    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload, json!({ "message": "hello" }));
    assert_eq!(executions.load(Ordering::Relaxed), 1);
}

#[test]
fn app_tool_plane_rejects_duplicate_paths() {
    let mut plane = AppToolPlane::<TestContextFactory>::new();
    let path = ToolPath::from("test.echo");

    plane
        .register(
            path.clone(),
            EchoTool {
                executions: Arc::new(AtomicUsize::new(0)),
            },
        )
        .expect("first registration should pass");
    let error = plane
        .register(
            path,
            EchoTool {
                executions: Arc::new(AtomicUsize::new(0)),
            },
        )
        .expect_err("duplicate registration should fail");

    assert_eq!(error, ToolPlaneError::DuplicateTool("test.echo".to_owned()));
}

#[tokio::test]
async fn app_tool_plane_registered_path_reports_tool_input_error() {
    let executions = Arc::new(AtomicUsize::new(0));
    let mut plane = AppToolPlane::<TestContextFactory>::new();
    let path = ToolPath::from("test.echo");
    plane
        .register(
            path.clone(),
            EchoTool {
                executions: executions.clone(),
            },
        )
        .expect("tool should register");

    assert!(plane.contains(&path));
    let error = plane
        .invoke(
            tool_invocation_grant(path.clone(), json!({})).await,
            &TestContext,
        )
        .await
        .expect_err("invalid registered tool input must fail");

    assert!(matches!(error, ToolPlaneError::Execution(reason) if reason.contains("message")));
    assert_eq!(executions.load(Ordering::Relaxed), 0);
}

async fn tool_invocation_grant(
    path: ToolPath,
    payload: Value,
) -> loong_core::policy::grant::Granted<ToolInvocationAction> {
    let mut policy = PolicyPipeline::<TestContextFactory>::new();
    policy.push_policy(ToolInvocationAllowPolicy);
    policy
        .grant(
            &TestContext,
            ToolInvocationAction::new(path, BTreeSet::new(), payload),
        )
        .await
        .expect("test policy should grant tool invocation")
        .granted
}
