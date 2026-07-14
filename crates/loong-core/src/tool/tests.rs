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
use loong_contracts::{
    AuthorizationScope, AuthorizationSubject, Capabilities, ToolInputError, ToolSpec,
};
use serde_json::{Value, json};

use crate::{
    policy::context::{ContextFactory, PolicyContext},
    tool::{RegisteredTool, RegisteredToolError, ToolImpl, ToolProvenance},
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
            actor_id: "actor:test:tool".to_owned(),
            scope: AuthorizationScope::Session {
                session_id: "session:test:tool".to_owned(),
            },
        }
    }
}

struct EchoTool {
    executions: Arc<AtomicUsize>,
    fail_execution: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("echo execution failed")]
struct EchoExecutionError;

#[async_trait]
impl ToolImpl<TestContextFactory> for EchoTool {
    type Input = String;
    type Output = Value;
    type Error = EchoExecutionError;

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
    ) -> Result<Self::Output, Self::Error> {
        self.executions.fetch_add(1, Ordering::Relaxed);
        if self.fail_execution {
            return Err(EchoExecutionError);
        }
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
            fail_execution: false,
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
            fail_execution: false,
        },
    );

    let error = block_on(tool.invoke(&TestContext, json!({})))
        .expect_err("missing message should be rejected");

    assert!(matches!(
        error,
        RegisteredToolError::Input(ToolInputError::MissingField { field }) if field == "message"
    ));
    assert_eq!(executions.load(Ordering::Relaxed), 0);
}

#[test]
fn registered_tool_preserves_concrete_execution_error_as_source() {
    let tool = RegisteredTool::<TestContextFactory>::from_tool(
        ToolProvenance::Builtin,
        EchoTool {
            executions: Arc::new(AtomicUsize::new(0)),
            fail_execution: true,
        },
    );

    let error = block_on(tool.invoke(&TestContext, json!({ "message": "hello" })))
        .expect_err("concrete execution failure should escape erasure");

    assert!(matches!(error, RegisteredToolError::Execution { .. }));
    std::error::Error::source(&error)
        .and_then(|source| source.downcast_ref::<EchoExecutionError>())
        .expect("erased error should retain the concrete execution source");
}

#[test]
fn registered_tool_success_observer_sees_typed_output_before_erasure() {
    let executions = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let tool = RegisteredTool::<TestContextFactory>::from_tool_with_success_observer(
        ToolProvenance::Builtin,
        EchoTool {
            executions: executions.clone(),
            fail_execution: false,
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
