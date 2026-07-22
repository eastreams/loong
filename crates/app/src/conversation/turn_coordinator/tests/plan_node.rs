use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execute_single_tool_intent_marks_file_read_input_repair_required() {
    let (tool_name, args_json) = crate::tools::synthesize_test_provider_tool_call_with_scope(
        "read",
        json!({}),
        Some("root-session"),
        Some("turn-file-read-plan-node"),
    );
    let intent = ToolIntent {
        tool_name: crate::conversation::turn_engine::ToolIntentTarget::registered(
            loong_contracts::ToolPath::new(["read"]).expect("test tool path must be valid"),
            tool_name,
        ),
        args_json,
        source: "provider_tool_call".to_owned(),
        turn_id: "turn-file-read-plan-node".to_owned(),
        tool_call_id: "call-file-read-plan-node".to_owned(),
    };
    let owner = crate::test_support::runtime_session_for_test(
        "root-session",
        crate::tools::ToolView::from_legacy_paths(["read"]),
    );
    let session_context = owner.context();

    let error =
        execute_single_tool_intent(&intent, &session_context, &owner.legacy_tools, None, 2_048)
            .await
            .expect_err("invalid typed read input should return a plan-node error");

    assert_eq!(error.kind, PlanNodeErrorKind::InputRepairRequired);
    assert!(error.message.contains("tool input requires at least one"));
    assert!(matches!(
        error.tool_input.as_deref(),
        Some(crate::conversation::turn_engine::ToolInputFailure {
            error: loong_contracts::ToolInputError::MissingOneOf { .. },
            ..
        })
    ));
}

#[cfg(feature = "tool-shell")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execute_single_tool_intent_marks_repairable_shell_preflight_failure_retryable() {
    let intent = ToolIntent {
        tool_name: "bash".into(),
        args_json: json!({}),
        source: "provider_tool_call".to_owned(),
        turn_id: "turn-shell-plan-node".to_owned(),
        tool_call_id: "call-shell-plan-node".to_owned(),
    };
    let owner = crate::test_support::runtime_session_for_test(
        "root-session",
        crate::tools::planned_root_tool_view(),
    );
    let session_context = owner.context();

    let error =
        execute_single_tool_intent(&intent, &session_context, &owner.legacy_tools, None, 2_048)
            .await
            .expect_err("repairable shell preflight should return a plan-node error");

    assert_eq!(error.kind, PlanNodeErrorKind::Retryable);
    assert!(error.message.contains("tool input needs repair"));
    assert!(error.message.contains("direct_bash_requires_command"));
}
