use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execute_single_tool_intent_direct_binding_reports_no_kernel_context() {
    let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
        "file.read",
        json!({
            "path": "README.md",
        }),
        Some("root-session"),
        Some("turn-direct-core"),
    );
    let intent = ToolIntent {
        tool_name,
        args_json,
        source: "provider_tool_call".to_owned(),
        session_id: "root-session".to_owned(),
        turn_id: "turn-direct-core".to_owned(),
        tool_call_id: "call-direct-core".to_owned(),
    };
    let session_context =
        SessionContext::root_with_tool_view("root-session", crate::tools::planned_root_tool_view());
    let error = execute_single_tool_intent(
        &intent,
        &session_context,
        &crate::conversation::NoopAppToolDispatcher,
        ConversationRuntimeBinding::direct(),
        None,
        2_048,
    )
    .await
    .expect_err("direct core execution should fail closed without kernel context");

    assert_eq!(error.kind, PlanNodeErrorKind::PolicyDenied);
    assert_eq!(error.message, "no_kernel_context");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execute_single_tool_intent_marks_repairable_file_read_failure_retryable() {
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::new();
    let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
        "file.read",
        json!({}),
        Some("root-session"),
        Some("turn-file-read-plan-node"),
    );
    let intent = ToolIntent {
        tool_name,
        args_json,
        source: "provider_tool_call".to_owned(),
        session_id: "root-session".to_owned(),
        turn_id: "turn-file-read-plan-node".to_owned(),
        tool_call_id: "call-file-read-plan-node".to_owned(),
    };
    let session_context =
        SessionContext::root_with_tool_view("root-session", crate::tools::planned_root_tool_view());

    let error = execute_single_tool_intent(
        &intent,
        &session_context,
        &DefaultAppToolDispatcher::runtime(),
        ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
        None,
        2_048,
    )
    .await
    .expect_err("repairable file.read preflight should return a plan-node error");

    assert_eq!(error.kind, PlanNodeErrorKind::Retryable);
    assert!(error.message.contains("tool input needs repair"));
    assert!(error.message.contains("file.read payload.path is required"));
}

#[cfg(feature = "tool-shell")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execute_single_tool_intent_marks_repairable_shell_preflight_failure_retryable() {
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::new();
    let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
        "shell.exec",
        json!({
            "command": "\"ls -la\"",
        }),
        Some("root-session"),
        Some("turn-shell-plan-node"),
    );
    let intent = ToolIntent {
        tool_name,
        args_json,
        source: "provider_tool_call".to_owned(),
        session_id: "root-session".to_owned(),
        turn_id: "turn-shell-plan-node".to_owned(),
        tool_call_id: "call-shell-plan-node".to_owned(),
    };
    let session_context =
        SessionContext::root_with_tool_view("root-session", crate::tools::planned_root_tool_view());

    let error = execute_single_tool_intent(
        &intent,
        &session_context,
        &DefaultAppToolDispatcher::runtime(),
        ConversationRuntimeBinding::kernel(&harness.kernel_ctx),
        None,
        2_048,
    )
    .await
    .expect_err("repairable shell preflight should return a plan-node error");

    assert_eq!(error.kind, PlanNodeErrorKind::Retryable);
    assert!(error.message.contains("tool input needs repair"));
}
