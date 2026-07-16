use std::{
    borrow::Cow,
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use loong_contracts::{
    AuthorizationScope, AuthorizationSubject, Capabilities, Capability, ToolInputError, ToolSpec,
};
use loong_core::{
    error::{AuthorizationError, PolicyGrantError},
    policy::context::{ContextFactory, PolicyContext},
    tool::{ToolFailureKind, ToolImpl},
};
use serde_json::{Value, json};

use super::{RegisteredTool, RegisteredToolError};

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

#[derive(Debug, thiserror::Error)]
#[error("nested authorization failure: {0}")]
struct NestedAuthorizationError(#[source] AuthorizationError);

struct DeniedTool;

#[async_trait]
impl ToolImpl<TestContextFactory> for DeniedTool {
    type Input = ();
    type Output = Value;
    type Error = NestedAuthorizationError;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            description: "Always denied.".to_owned(),
            input_schema: json!({ "type": "object" }),
            required_capabilities: BTreeSet::new(),
            argument_hint: None,
            search_hint: None,
            tags: Vec::new(),
        }
    }

    fn parse_input(&self, _payload: Value) -> Result<Self::Input, ToolInputError> {
        Ok(())
    }

    fn failure_kind(&self, _error: &Self::Error) -> ToolFailureKind {
        ToolFailureKind::Denied
    }

    async fn execute(
        &self,
        _ctx: &<TestContextFactory as ContextFactory>::Cx<'_>,
        (): Self::Input,
    ) -> Result<Self::Output, Self::Error> {
        Err(NestedAuthorizationError(AuthorizationError::PolicyGrant(
            PolicyGrantError::MissingCapability {
                capability: Capability::FilesystemRead,
            },
        )))
    }
}

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

#[tokio::test]
async fn registered_tool_invokes_erased_tool_impl() {
    let executions = Arc::new(AtomicUsize::new(0));
    let tool = RegisteredTool::<TestContextFactory>::from_tool(EchoTool {
        executions: Arc::clone(&executions),
        fail_execution: false,
    });

    let outcome = tool
        .invoke(&TestContext, json!({ "message": "hello" }))
        .await
        .expect("tool should execute");

    assert_eq!(tool.spec().description, "Echo the provided message.");
    assert_eq!(outcome, json!({ "message": "hello" }));
    assert_eq!(executions.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn registered_tool_parse_failure_does_not_execute_tool() {
    let executions = Arc::new(AtomicUsize::new(0));
    let tool = RegisteredTool::<TestContextFactory>::from_tool(EchoTool {
        executions: Arc::clone(&executions),
        fail_execution: false,
    });

    let error = tool
        .invoke(&TestContext, json!({}))
        .await
        .expect_err("missing message should be rejected");

    assert!(matches!(
        error,
        RegisteredToolError::Input(ToolInputError::MissingField { field }) if field == "message"
    ));
    assert_eq!(executions.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn registered_tool_preserves_concrete_execution_error_as_source() {
    let tool = RegisteredTool::<TestContextFactory>::from_tool(EchoTool {
        executions: Arc::new(AtomicUsize::new(0)),
        fail_execution: true,
    });

    let error = tool
        .invoke(&TestContext, json!({ "message": "hello" }))
        .await
        .expect_err("concrete execution failure should escape erasure");

    assert!(matches!(error, RegisteredToolError::Execution { .. }));
    std::error::Error::source(&error)
        .and_then(|source| source.downcast_ref::<EchoExecutionError>())
        .expect("erased error should retain the concrete execution source");
}

#[tokio::test]
async fn registered_tool_respects_explicit_denial_class_at_erasure() {
    let nested = NestedAuthorizationError(AuthorizationError::PolicyGrant(
        PolicyGrantError::MissingCapability {
            capability: Capability::FilesystemRead,
        },
    ));
    let authorization = std::error::Error::source(&nested)
        .expect("nested tool error should expose authorization source");
    assert!(authorization.is::<AuthorizationError>());
    assert!(
        authorization
            .source()
            .is_some_and(|source| source.is::<PolicyGrantError>()),
        "authorization error should expose typed policy grant source"
    );

    let error = RegisteredTool::<TestContextFactory>::from_tool(DeniedTool)
        .invoke(&TestContext, json!({}))
        .await
        .expect_err("nested policy denial should remain typed after erasure");

    assert!(matches!(error, RegisteredToolError::Denied { .. }));
    assert!(
        std::error::Error::source(&error)
            .and_then(|source| source.downcast_ref::<NestedAuthorizationError>())
            .is_some(),
        "denied erasure must retain the concrete tool error"
    );
}

#[tokio::test]
async fn registered_tool_success_observer_sees_typed_output_before_erasure() {
    let executions = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let tool = RegisteredTool::<TestContextFactory>::from_tool_with_success_observer(
        EchoTool {
            executions: Arc::clone(&executions),
            fail_execution: false,
        },
        {
            let observed = Arc::clone(&observed);
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

    let outcome = tool
        .invoke(&TestContext, json!({ "message": "hello" }))
        .await
        .expect("tool should execute");

    assert_eq!(outcome, json!({ "message": "hello" }));
    assert_eq!(executions.load(Ordering::Relaxed), 1);
    assert_eq!(
        observed.lock().expect("observer lock").as_slice(),
        ["hello"]
    );
}
