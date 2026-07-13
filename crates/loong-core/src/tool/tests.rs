use std::{
    borrow::Cow,
    collections::BTreeSet,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

use async_trait::async_trait;
use loong_contracts::{Capabilities, ToolExecutionError, ToolInputError, ToolSpec};
use serde_json::{Value, json};

use crate::{
    policy::context::{ContextFactory, PolicyContext},
    tool::{RegisteredTool, ToolImpl, ToolProvenance},
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
}

struct EchoTool {
    executions: Arc<AtomicUsize>,
}

#[async_trait]
impl ToolImpl<TestContextFactory> for EchoTool {
    type Input = String;
    type Output = Value;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            description: "Echo the provided message.".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                },
                "required": ["message"],
                "additionalProperties": false
            }),
            required_capabilities: BTreeSet::new(),
            argument_hint: None,
            search_hint: None,
            tags: Vec::new(),
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
        Ok(json!({ "message": input }))
    }
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

    assert_eq!(tool.spec().description, "Echo the provided message.");
    assert_eq!(outcome, json!({ "message": "hello" }));
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

#[test]
fn registered_tool_success_observer_sees_typed_output_before_erasure() {
    let executions = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let tool = RegisteredTool::<TestContextFactory>::from_tool_with_success_observer(
        ToolProvenance::Builtin,
        EchoTool {
            executions: executions.clone(),
        },
        {
            let observed = observed.clone();
            move |_ctx, output: &Value| {
                observed.lock().expect("observer lock").push(
                    output
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                );
                Ok(())
            }
        },
    );

    let outcome = block_on(tool.invoke(&TestContext, json!({ "message": "hello" })))
        .expect("tool should execute");

    assert_eq!(outcome, json!({ "message": "hello" }));
    assert_eq!(executions.load(Ordering::Relaxed), 1);
    assert_eq!(
        observed.lock().expect("observer lock").as_slice(),
        ["hello"]
    );
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = std::pin::pin!(future);

    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}
