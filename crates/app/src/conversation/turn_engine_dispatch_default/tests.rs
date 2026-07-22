use loong_contracts::{GovernedSessionMode, ToolCoreRequest};
use serde_json::Value;

use super::*;

#[tokio::test]
async fn legacy_dispatcher_rejects_a_context_from_another_runtime() {
    let config = crate::config::LoongConfig::default();
    let dispatcher_owner = crate::test_support::TestRuntimeSession::from_config(
        &config,
        "dispatcher-session",
        "test-agent",
        GovernedSessionMode::MutatingCapable,
    )
    .expect("dispatcher owner");
    let context_owner = crate::test_support::TestRuntimeSession::from_config(
        &config,
        "context-session",
        "test-agent",
        GovernedSessionMode::MutatingCapable,
    )
    .expect("context owner");
    let context = context_owner.context();

    let core_error = LegacyToolDispatcher::execute_core_tool(
        &dispatcher_owner.legacy_tools,
        &context,
        ToolCoreRequest {
            tool_name: "config.import".to_owned(),
            payload: Value::Null,
        },
        false,
    )
    .await
    .expect_err("legacy core fallback must remain in one Runtime domain");
    assert!(matches!(
        core_error,
        crate::tools::LegacyToolRequestError::RuntimeMismatch
    ));

    let app_error = LegacyToolDispatcher::execute_app_tool(
        &dispatcher_owner.legacy_tools,
        &context,
        ToolCoreRequest {
            tool_name: "session_status".to_owned(),
            payload: Value::Null,
        },
    )
    .await
    .expect_err("legacy app fallback must remain in one Runtime domain");
    assert_eq!(app_error, "legacy dispatcher belongs to another Runtime");
}
