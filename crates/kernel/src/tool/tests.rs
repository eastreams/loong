use std::collections::BTreeSet;

use async_trait::async_trait;
use loong_contracts::{
    Capability, ToolExecutionError, ToolInputError, ToolOutcome, ToolPath, ToolSpec,
};
use loong_core::{
    policy::context::{ContextFactory, PolicyContext},
    tool::ToolImpl,
};
use serde_json::{Value, json};

use crate::{ToolPlaneError, tool::ToolPlane};

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

struct EchoTool;

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
        Ok(ToolOutcome {
            status: "ok".to_owned(),
            payload: json!({ "message": input }),
        })
    }
}

#[tokio::test]
async fn typed_tool_plane_invokes_registered_tool() {
    let mut plane = ToolPlane::<TestContextFactory>::new();
    let path = ToolPath::from("test.echo");

    plane
        .register(path.clone(), EchoTool)
        .expect("tool should register");
    let outcome = plane
        .invoke(&path, &TestContext, json!({ "message": "hello" }))
        .await
        .expect("tool should execute");

    assert!(plane.contains(&path));
    assert_eq!(plane.len(), 1);
    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload, json!({ "message": "hello" }));
}

#[test]
fn typed_tool_plane_rejects_duplicate_paths() {
    let mut plane = ToolPlane::<TestContextFactory>::new();
    let path = ToolPath::from("test.echo");

    plane
        .register(path.clone(), EchoTool)
        .expect("first registration should pass");
    let error = plane
        .register(path, EchoTool)
        .expect_err("duplicate registration should fail");

    assert_eq!(error, ToolPlaneError::DuplicateTool("test.echo".to_owned()));
}

#[tokio::test]
async fn typed_tool_plane_reports_missing_path_without_legacy_fallback() {
    let plane = ToolPlane::<TestContextFactory>::new();
    let path = ToolPath::from("test.missing");

    let error = plane
        .invoke(&path, &TestContext, json!({ "message": "hello" }))
        .await
        .expect_err("missing tool should be structural miss");

    assert_eq!(
        error,
        ToolPlaneError::ToolNotFound("test.missing".to_owned())
    );
}
