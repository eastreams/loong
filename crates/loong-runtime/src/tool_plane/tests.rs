use std::{
    borrow::Cow,
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use loong_contracts::{
    Capabilities, Capability, GrantId, PolicyEntry, PolicyOutcome, PolicyRegistration,
    PolicyRegistrationSource, PolicyReport, ToolInputError, ToolSpec,
};
use loong_core::{
    policy::{
        action::ActionMeta,
        context::{ContextFactory, PolicyContext},
        engine::PolicyEngine,
    },
    tool::ToolImpl,
};
use serde_json::{Value, json};

use super::{
    ToolInvocationAction, ToolPath, ToolPlane, ToolPlaneRegistry,
    error::{DispatchError, LookupError, RegistrationError},
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

struct AllowPolicyEngine {
    next_grant_id: AtomicU64,
}

impl Default for AllowPolicyEngine {
    fn default() -> Self {
        Self {
            next_grant_id: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl PolicyEngine<TestContextFactory> for AllowPolicyEngine {
    async fn decide<A: ActionMeta + 'static>(
        &self,
        _ctx: &<TestContextFactory as ContextFactory>::Cx<'_>,
        _action: &A,
    ) -> PolicyReport {
        PolicyReport {
            evaluations: Vec::new(),
            outcome: PolicyOutcome::Allow {
                source: PolicyEntry {
                    policy_name: Cow::Borrowed("test-allow"),
                    policy_id: 1,
                    registration: PolicyRegistration {
                        order: 1,
                        registered_at_unix_ms: 1,
                        source: PolicyRegistrationSource {
                            file: "tool_plane/tests.rs".to_owned(),
                            line: 1,
                            column: 1,
                        },
                    },
                },
                reason: Cow::Borrowed("test policy allows invocation"),
            },
        }
    }

    async fn next_grant_id(&self) -> GrantId {
        GrantId(self.next_grant_id.fetch_add(1, Ordering::Relaxed) + 1)
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
    assert_eq!(
        action.payload().as_ref(),
        &json!({
            "tool_path": "read",
            "payload": { "path": "notes.txt" }
        })
    );
}

#[tokio::test]
async fn registry_invokes_registered_tool() {
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
    let grant = grant_invocation(path, json!({ "message": "hello" })).await;

    let output = plane
        .invoke(grant, &TestContext)
        .await
        .expect("registered tool should execute");

    assert_eq!(output, json!({ "message": "hello" }));
    assert_eq!(executions.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn registry_runs_success_observer_after_tool_execution() {
    let executions = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mut plane = ToolPlaneRegistry::<TestContextFactory>::new();
    let path = ToolPath::from("test.echo");
    plane
        .register_with_provenance_and_success_observer(
            path.clone(),
            loong_core::tool::ToolProvenance::Builtin,
            EchoTool {
                executions: Arc::clone(&executions),
            },
            {
                let observed = Arc::clone(&observed);
                move |_ctx, output: &Value| {
                    observed
                        .lock()
                        .expect("observer lock should remain available")
                        .push(
                            output
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        );
                }
            },
        )
        .expect("test tool registration should succeed");

    let output = plane
        .invoke(
            grant_invocation(path, json!({ "message": "hello" })).await,
            &TestContext,
        )
        .await
        .expect("registered tool should execute");

    assert_eq!(output, json!({ "message": "hello" }));
    assert_eq!(executions.load(Ordering::Relaxed), 1);
    assert_eq!(
        observed
            .lock()
            .expect("observer lock should remain available")
            .as_slice(),
        ["hello"]
    );
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

#[tokio::test]
async fn registry_preserves_registered_tool_input_errors() {
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

    let error = plane
        .invoke(grant_invocation(path, json!({})).await, &TestContext)
        .await
        .expect_err("invalid registered tool input should fail");

    assert!(matches!(
        error,
        DispatchError::Tool {
            source: loong_core::tool::RegisteredToolError::Input(
                ToolInputError::MissingField { field }
            )
        } if field == "message"
    ));
    assert_eq!(executions.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn registry_distinguishes_lookup_miss_from_post_grant_dispatch_failure() {
    let executions = Arc::new(AtomicUsize::new(0));
    let mut plane = ToolPlaneRegistry::<TestContextFactory>::new();
    plane
        .register(
            ToolPath::from("test.echo"),
            EchoTool {
                executions: Arc::clone(&executions),
            },
        )
        .expect("test tool registration should succeed");

    let path = ToolPath::from("test.missing");
    let lookup_error = plane
        .spec(&path)
        .expect_err("unregistered path should fail lookup");
    let dispatch_error = plane
        .invoke(
            grant_invocation(path.clone(), json!({ "message": "hello" })).await,
            &TestContext,
        )
        .await
        .expect_err("a granted but absent entry should fail during dispatch");

    assert!(matches!(
        lookup_error,
        LookupError::NotRegistered { path: missing } if missing == path
    ));
    assert!(matches!(
        dispatch_error,
        DispatchError::RegistryInvariant { path: missing } if missing == path
    ));
    assert_eq!(executions.load(Ordering::Relaxed), 0);
}

async fn grant_invocation(
    path: ToolPath,
    payload: Value,
) -> loong_core::policy::grant::Granted<ToolInvocationAction> {
    AllowPolicyEngine::default()
        .grant(
            &TestContext,
            ToolInvocationAction::new(path, BTreeSet::new(), payload),
        )
        .await
        .expect("test policy should grant invocation")
        .granted
}
