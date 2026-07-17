use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execute_single_tool_intent_advisory_only_binding_reports_no_app_context() {
    let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
        "browse",
        json!({
            "url": "https://example.com",
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
    let session_context = crate::test_support::app_context_for_session(
        "root-session",
        crate::tools::planned_root_tool_view(),
    );
    let error = execute_single_tool_intent(
        &intent,
        &session_context,
        &crate::conversation::NoopAppToolDispatcher,
        ConversationRuntimeBinding::AdvisoryOnly,
        None,
        2_048,
    )
    .await
    .expect_err("direct core execution should fail closed without app context");

    assert_eq!(error.kind, PlanNodeErrorKind::PolicyDenied);
    assert_eq!(error.message, "no_app_context");
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
    let session_context = crate::test_support::app_context_for_session(
        "root-session",
        crate::tools::planned_root_tool_view(),
    );

    let error = execute_single_tool_intent(
        &intent,
        &session_context,
        &DefaultAppToolDispatcher::runtime(),
        ConversationRuntimeBinding::Context(&harness.app_ctx),
        None,
        2_048,
    )
    .await
    .expect_err("repairable file.read preflight should return a plan-node error");

    assert_eq!(error.kind, PlanNodeErrorKind::Retryable);
    assert!(error.message.contains("invalid tool input"));
    assert!(error.message.contains("direct_read_requires_one_of"));
}

#[cfg(feature = "tool-shell")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execute_single_tool_intent_marks_repairable_shell_preflight_failure_retryable() {
    use crate::test_support::TurnTestHarness;

    let harness = TurnTestHarness::with_capabilities(std::collections::BTreeSet::from([
        loong_contracts::Capability::InvokeTool,
        loong_contracts::Capability::FilesystemRead,
        loong_contracts::Capability::FilesystemWrite,
        loong_contracts::Capability::NetworkEgress,
    ]));
    let intent = ToolIntent {
        tool_name: "bash".to_owned(),
        args_json: json!({}),
        source: "provider_tool_call".to_owned(),
        session_id: "root-session".to_owned(),
        turn_id: "turn-shell-plan-node".to_owned(),
        tool_call_id: "call-shell-plan-node".to_owned(),
    };
    let session_context = crate::test_support::app_context_for_session(
        "root-session",
        crate::tools::planned_root_tool_view(),
    );

    let error = execute_single_tool_intent(
        &intent,
        &session_context,
        &DefaultAppToolDispatcher::runtime(),
        ConversationRuntimeBinding::Context(&harness.app_ctx),
        None,
        2_048,
    )
    .await
    .expect_err("repairable shell preflight should return a plan-node error");

    assert_eq!(error.kind, PlanNodeErrorKind::Retryable);
    assert!(error.message.contains("tool input needs repair"));
    assert!(error.message.contains("direct_bash_requires_command"));
}
