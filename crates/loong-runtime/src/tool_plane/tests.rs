use std::{
    borrow::Cow,
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use loong_contracts::{
    AuthorizationScope, AuthorizationSubject, Capabilities, Capability, ToolInputError,
    ToolSchedulingClass, ToolSpec,
};
use loong_core::{
    policy::{
        action::ActionMeta,
        context::{ContextFactory, PolicyContext},
    },
    tool::ToolImpl,
};
use serde_json::{Value, json};

use super::{
    ToolInvocationAction, ToolPath, ToolPlaneRegistry,
    error::{LookupError, RegistrationError},
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
            actor_id: "test:runtime:tool-plane:actor".to_owned(),
            scope: AuthorizationScope::Session {
                session_id: "test:runtime:tool-plane:session".to_owned(),
            },
        }
    }
}

struct EchoTool {
    executions: Arc<AtomicUsize>,
}

#[async_trait]
impl ToolImpl<TestContextFactory> for EchoTool {
    type Input = String;
    type Output = Value;
    type Error = std::convert::Infallible;

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
            scheduling: ToolSchedulingClass::SerialOnly,
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
        Ok(json!({ "message": input }))
    }
}

#[test]
fn tool_path_keeps_plane_local_segments() {
    let path = ToolPath::from("test.echo");

    assert_eq!(path.segments(), ["test", "echo"]);
    assert_eq!(path.to_string(), "test.echo");
}

#[test]
fn tool_invocation_action_exposes_policy_metadata() {
    let action = ToolInvocationAction::new(
        ToolPath::from("read"),
        Capabilities::from([Capability::InvokeTool, Capability::FilesystemRead]),
        json!({ "path": "notes.txt" }),
    );
    let metadata = action.metadata();

    assert_eq!(action.path(), &ToolPath::from("read"));
    assert_eq!(
        action
            .required_capabilities()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([Capability::FilesystemRead, Capability::InvokeTool])
    );
    assert_eq!(action.payload(), &json!({ "path": "notes.txt" }));
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
    assert_eq!(
        ActionMeta::payload(&action).as_ref(),
        &json!({
            "tool_path": "read",
            "payload": { "path": "notes.txt" }
        })
    );
}

#[test]
fn registry_resolves_registered_tool_metadata() {
    let executions = Arc::new(AtomicUsize::new(0));
    let mut plane = ToolPlaneRegistry::<TestContextFactory>::new();
    let path = ToolPath::from("test.echo");
    plane
        .register(
            path.clone(),
            EchoTool {
                executions: Arc::clone(&executions),
            },
        )
        .expect("test tool registration should succeed");
    let registered = plane
        .resolve(&path)
        .expect("registered path should resolve its concrete entry");

    assert_eq!(registered.spec().description, "Echo the provided message.");
    assert_eq!(executions.load(Ordering::Relaxed), 0);
}

#[test]
fn registry_rejects_duplicate_paths_without_leaking_slots() {
    let mut plane = ToolPlaneRegistry::<TestContextFactory>::new();
    let path = ToolPath::from("test.echo");
    plane
        .register(
            path.clone(),
            EchoTool {
                executions: Arc::new(AtomicUsize::new(0)),
            },
        )
        .expect("first registration should succeed");

    let error = plane
        .register(
            path,
            EchoTool {
                executions: Arc::new(AtomicUsize::new(0)),
            },
        )
        .expect_err("duplicate path should fail");

    assert!(matches!(
        error,
        RegistrationError::AlreadyRegistered { path }
            if path == ToolPath::from("test.echo")
    ));
    assert_eq!(plane.entry_count(), 1);
    assert_eq!(plane.path_count(), 1);
}

#[test]
fn registry_enumerates_paths_in_path_order() {
    let mut plane = ToolPlaneRegistry::<TestContextFactory>::new();
    for path in ["test.beta", "test.alpha"] {
        plane
            .register(
                ToolPath::from(path),
                EchoTool {
                    executions: Arc::new(AtomicUsize::new(0)),
                },
            )
            .expect("test tool registration should succeed");
    }

    assert_eq!(
        plane.registered_paths(),
        vec![ToolPath::from("test.alpha"), ToolPath::from("test.beta")]
    );
}

#[test]
fn registry_reports_unregistered_path_before_invocation() {
    let mut plane = ToolPlaneRegistry::<TestContextFactory>::new();
    plane
        .register(
            ToolPath::from("test.echo"),
            EchoTool {
                executions: Arc::new(AtomicUsize::new(0)),
            },
        )
        .expect("test tool registration should succeed");

    let path = ToolPath::from("test.missing");
    let error = match plane.resolve(&path) {
        Ok(_) => panic!("unregistered path should fail before invocation"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        LookupError::NotRegistered { path: missing } if missing == path
    ));
}
