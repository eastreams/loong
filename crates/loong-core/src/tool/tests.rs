use std::{
    collections::BTreeSet,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use async_trait::async_trait;
use loong_contracts::{
    Capability, ToolExecutionError, ToolInputError, ToolOutcome, ToolPath, ToolSpec,
};
use serde_json::{Value, json};

use crate::{
    policy::action::ActionMeta,
    policy::context::{ContextFactory, PolicyContext},
    tool::{RegisteredTool, ToolImpl, ToolInvocationAction, ToolProvenance},
};

struct TestContextFactory;

impl ContextFactory for TestContextFactory {
    type Cx<'a> = TestContext;
}

struct TestContext;

impl PolicyContext for TestContext {
    fn capabilities(&self) -> BTreeSet<Capability> {
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
            path: ToolPath::from("test.echo"),
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

#[test]
fn registered_tool_invokes_erased_tool_impl() {
    let executions = Arc::new(AtomicUsize::new(0));
    let tool = RegisteredTool::<TestContextFactory>::from_tool(
        ToolProvenance::Builtin,
        EchoTool {
            executions: executions.clone(),
        },
    );

    let outcome = block_on(tool.invoke(&TestContext, json!({ "message": "hello" })))
        .expect("tool should execute");

    assert_eq!(tool.spec().path.as_str(), "test.echo");
    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload, json!({ "message": "hello" }));
    assert_eq!(executions.load(Ordering::Relaxed), 1);
}

#[test]
fn registered_tool_parse_failure_does_not_execute_tool() {
    let executions = Arc::new(AtomicUsize::new(0));
    let tool = RegisteredTool::<TestContextFactory>::from_tool(
        ToolProvenance::Builtin,
        EchoTool {
            executions: executions.clone(),
        },
    );

    let error = block_on(tool.invoke(&TestContext, json!({})))
        .expect_err("missing message should be rejected");

    assert!(matches!(
        error,
        ToolExecutionError::Input(ToolInputError::MissingField { field }) if field == "message"
    ));
    assert_eq!(executions.load(Ordering::Relaxed), 0);
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);

    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}
