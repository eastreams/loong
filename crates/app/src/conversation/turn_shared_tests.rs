use super::*;
use crate::conversation::tool_result_reduction::SHELL_FOLLOWUP_STDIO_PREVIEW_CHARS;
use crate::conversation::turn_engine::{
    ApprovalRequirement, ApprovalRequirementKind, TurnFailure, TurnResult,
};
use crate::conversation::turn_shared::tool_result::ToolResultContinuationKind;
use async_trait::async_trait;
use serde_json::json;

#[test]
fn raw_tool_output_detection_keeps_known_signals() {
    assert!(user_requested_raw_tool_output("show raw tool output"));
    assert!(user_requested_raw_tool_output("give exact output as JSON"));
    assert!(!user_requested_raw_tool_output(
        "summarize the result briefly"
    ));
}

#[test]
fn raw_tool_output_detection_ignores_payload_mentions_without_output_request() {
    assert!(!user_requested_raw_tool_output(
        "Callback hints mention the payload JSON, but just summarize the action."
    ));
    assert!(!user_requested_raw_tool_output(
        "The card callback token stays in internal payload context."
    ));
    assert!(user_requested_raw_tool_output(
        "Return the payload as JSON."
    ));
}

#[test]
fn raw_tool_output_detection_ignores_generic_json_and_tool_output_requests() {
    assert!(!user_requested_raw_tool_output("summarize the tool output"));
    assert!(!user_requested_raw_tool_output("answer in json"));
    assert!(!user_requested_raw_tool_output("format the result as json"));
    assert!(user_requested_raw_tool_output("[ok]"));
}

#[test]
fn compose_assistant_reply_keeps_tool_error_inline_reason() {
    let reply = compose_assistant_reply(
        "preface",
        true,
        TurnResult::ToolError(TurnFailure::retryable("tool_error", "temporary failure")),
    );
    assert_eq!(reply, "preface\ntemporary failure");
}

#[test]
fn compose_assistant_reply_formats_governed_tool_approval_requirement() {
    let reply = compose_assistant_reply(
        "preface",
        true,
        TurnResult::NeedsApproval(ApprovalRequirement {
            kind: ApprovalRequirementKind::GovernedTool,
            reason: "operator approval required for governed tool".to_owned(),
            rule_id: "governed_tool_requires_approval".to_owned(),
            tool_name: Some("delegate_async".to_owned()),
            approval_key: Some("tool:delegate_async".to_owned()),
            approval_request_id: Some("apr_123".to_owned()),
        }),
    );

    assert!(reply.contains("[tool_approval_required]"));
    assert!(reply.contains("delegate_async"));
    assert!(reply.contains("apr_123"));
    assert!(reply.contains("yes"));
    assert!(reply.contains("auto"));
    assert!(reply.contains("full"));
    assert!(reply.contains("esc"));
}

#[test]
fn parse_approval_prompt_view_recovers_localized_action_contract() {
    let reply = format_approval_required_reply(
        "我准备调用 provider.switch 来切换后续会话的 provider。",
        &ApprovalRequirement {
            kind: ApprovalRequirementKind::GovernedTool,
            reason:
                "`provider.switch` is not eligible for auto mode and needs operator confirmation"
                    .to_owned(),
            rule_id: "session_tool_consent_auto_blocked".to_owned(),
            tool_name: Some("provider.switch".to_owned()),
            approval_key: Some("tool:provider.switch".to_owned()),
            approval_request_id: Some("apr_provider_switch".to_owned()),
        },
    );

    let parsed = parse_approval_prompt_view(reply.as_str()).expect("parse approval prompt");
    assert_eq!(parsed.marker, ApprovalPromptMarker::ToolApprovalRequired);
    assert_eq!(
        parsed.preface.as_deref(),
        Some("我准备调用 provider.switch 来切换后续会话的 provider。")
    );
    assert_eq!(parsed.tool_name.as_deref(), Some("provider.switch"));
    assert_eq!(parsed.request_id.as_deref(), Some("apr_provider_switch"));
    assert_eq!(
        parsed.rule_id.as_deref(),
        Some("session_tool_consent_auto_blocked")
    );
    assert_eq!(parsed.locale, ApprovalPromptLocale::Cjk);
    assert_eq!(
        parsed
            .actions
            .iter()
            .map(|action| action.command.as_str())
            .collect::<Vec<_>>(),
        vec!["yes", "auto", "full", "esc"]
    );
    assert_eq!(
        parsed
            .actions
            .iter()
            .map(|action| action.label.as_str())
            .collect::<Vec<_>>(),
        vec!["本次运行", "本会话自动", "本会话全自动", "跳过这次"]
    );
}

#[test]
fn approval_prompt_action_input_parser_accepts_skip_and_localized_aliases() {
    assert_eq!(
        parse_approval_prompt_action_input("run once"),
        Some(ApprovalPromptActionId::Yes)
    );
    assert_eq!(
        parse_approval_prompt_action_input("session full-auto"),
        Some(ApprovalPromptActionId::Full)
    );
    assert_eq!(
        parse_approval_prompt_action_input("跳过这次"),
        Some(ApprovalPromptActionId::Esc)
    );
    assert_eq!(
        parse_approval_prompt_action_input("skip call"),
        Some(ApprovalPromptActionId::Esc)
    );
    assert_eq!(parse_approval_prompt_action_input("maybe"), None);
}

#[test]
fn approval_prompt_action_input_parser_accepts_full_width_aliases() {
    assert_eq!(
        parse_approval_prompt_action_input("ｙｅｓ"),
        Some(ApprovalPromptActionId::Yes)
    );
    assert_eq!(
        parse_approval_prompt_action_input("３"),
        Some(ApprovalPromptActionId::Full)
    );
    assert_eq!(
        parse_approval_prompt_action_input("ｓｋｉｐ　ｃａｌｌ"),
        Some(ApprovalPromptActionId::Esc)
    );
}

#[test]
fn compose_assistant_reply_strips_think_tags_from_final_text() {
    let reply = compose_assistant_reply(
        "preface",
        false,
        TurnResult::FinalText("<think>internal reasoning</think>visible reply".to_owned()),
    );

    assert_eq!(reply, "visible reply");
}

#[test]
fn tool_driven_reply_kernel_extracts_raw_reply_and_result_followup() {
    let result = TurnResult::FinalText("tool output".to_owned());
    let kernel = ToolDrivenReplyKernel::new("preface", true, &result);

    assert_eq!(kernel.fallback_reply(), "preface\ntool output");
    assert_eq!(kernel.raw_reply(), Some("preface\ntool output".to_owned()));
    assert_eq!(
        kernel.followup_payload(),
        Some(ToolDrivenFollowupPayload::ToolResult {
            text: "tool output".to_owned(),
        })
    );
}

#[test]
fn tool_driven_reply_kernel_strips_think_tags_from_raw_reply() {
    let result =
        TurnResult::FinalText("<think>internal reasoning</think>visible tool output".to_owned());
    let kernel = ToolDrivenReplyKernel::new("preface", true, &result);

    assert_eq!(
        kernel.raw_reply(),
        Some("preface\nvisible tool output".to_owned())
    );
}

#[test]
fn tool_driven_reply_kernel_extracts_raw_reply_and_failure_followup() {
    let result = TurnResult::ToolError(TurnFailure::retryable("tool_error", "temporary failure"));
    let kernel = ToolDrivenReplyKernel::new("preface", true, &result);

    assert_eq!(kernel.fallback_reply(), "preface\ntemporary failure");
    assert_eq!(
        kernel.raw_reply(),
        Some("preface\ntemporary failure".to_owned())
    );
    assert_eq!(
        kernel.followup_payload(),
        Some(ToolDrivenFollowupPayload::ToolFailure {
            reason: "temporary failure".to_owned(),
            retryable: true,
        })
    );
}

#[test]
fn tool_driven_reply_kernel_rejects_non_tool_followup_paths() {
    let provider_error = TurnResult::ProviderError(TurnFailure::provider(
        "provider_error",
        "provider unavailable",
    ));
    let kernel = ToolDrivenReplyKernel::new("preface", true, &provider_error);
    assert_eq!(kernel.raw_reply(), None);
    assert_eq!(kernel.followup_payload(), None);

    let plain_text = TurnResult::FinalText("plain reply".to_owned());
    let non_tool_kernel = ToolDrivenReplyKernel::new("preface", false, &plain_text);
    assert_eq!(non_tool_kernel.raw_reply(), None);
    assert_eq!(non_tool_kernel.followup_payload(), None);
    assert_eq!(non_tool_kernel.fallback_reply(), "plain reply");
}

#[tokio::test]
async fn request_completion_with_raw_fallback_detailed_preserves_state_and_uses_raw_reply_body() {
    #[derive(Clone)]
    struct StateOnlyRuntime;

    #[async_trait]
    impl ConversationRuntime for StateOnlyRuntime {
        fn tool_view(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ToolView> {
            Ok(crate::tools::runtime_tool_view())
        }

        async fn build_messages(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _include_system_prompt: bool,
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<Vec<Value>> {
            Ok(Vec::new())
        }

        async fn request_completion(
            &self,
            _config: &LoongConfig,
            _messages: &[Value],
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<String> {
            Ok("[followup_state:continue]".to_owned())
        }

        async fn request_turn(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ProviderTurn> {
            Ok(ProviderTurn::default())
        }

        async fn request_turn_streaming(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
            _on_token: crate::provider::StreamingTokenCallback,
        ) -> CliResult<ProviderTurn> {
            Ok(ProviderTurn::default())
        }

        async fn persist_turn(
            &self,
            _session_id: &str,
            _role: &str,
            _content: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    let reply = request_completion_with_raw_fallback_detailed(
        &StateOnlyRuntime,
        &LoongConfig::default(),
        &[],
        ConversationRuntimeBinding::direct(),
        "<think>hidden</think>fallback body",
        None,
    )
    .await;

    assert_eq!(reply.state, Some(ToolDrivenContinuationState::Continue));
    assert_eq!(reply.reply, "fallback body");
}

#[tokio::test]
async fn request_completion_with_raw_fallback_detailed_uses_raw_reply_when_completion_is_empty() {
    #[derive(Clone)]
    struct EmptyReplyRuntime;

    #[async_trait]
    impl ConversationRuntime for EmptyReplyRuntime {
        fn tool_view(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ToolView> {
            Ok(crate::tools::runtime_tool_view())
        }

        async fn build_messages(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _include_system_prompt: bool,
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<Vec<Value>> {
            Ok(Vec::new())
        }

        async fn request_completion(
            &self,
            _config: &LoongConfig,
            _messages: &[Value],
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<String> {
            Ok(String::new())
        }

        async fn request_turn(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ProviderTurn> {
            Ok(ProviderTurn::default())
        }

        async fn request_turn_streaming(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
            _on_token: crate::provider::StreamingTokenCallback,
        ) -> CliResult<ProviderTurn> {
            Ok(ProviderTurn::default())
        }

        async fn persist_turn(
            &self,
            _session_id: &str,
            _role: &str,
            _content: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    let reply = request_completion_with_raw_fallback_detailed(
        &EmptyReplyRuntime,
        &LoongConfig::default(),
        &[],
        ConversationRuntimeBinding::direct(),
        "<think>hidden</think>fallback body",
        None,
    )
    .await;

    assert_eq!(reply.state, None);
    assert_eq!(reply.reply, "fallback body");
}

#[tokio::test]
async fn request_completion_with_raw_fallback_detailed_salvages_glued_tool_request_markup() {
    #[derive(Clone)]
    struct ToolMarkupRuntime;

    #[async_trait]
    impl ConversationRuntime for ToolMarkupRuntime {
        fn tool_view(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ToolView> {
            Ok(crate::tools::runtime_tool_view())
        }

        async fn build_messages(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _include_system_prompt: bool,
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<Vec<Value>> {
            Ok(Vec::new())
        }

        async fn request_completion(
            &self,
            _config: &LoongConfig,
            _messages: &[Value],
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<String> {
            Ok("[tool_request]\n{\"url\":\"https://example.com\"}Example Domain is a reserved placeholder page.".to_owned())
        }

        async fn request_turn(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<ProviderTurn> {
            Ok(ProviderTurn::default())
        }

        async fn request_turn_streaming(
            &self,
            _config: &LoongConfig,
            _session_id: &str,
            _turn_id: &str,
            _messages: &[Value],
            _tool_view: &ToolView,
            _binding: ConversationRuntimeBinding<'_>,
            _on_token: crate::provider::StreamingTokenCallback,
        ) -> CliResult<ProviderTurn> {
            Ok(ProviderTurn::default())
        }

        async fn persist_turn(
            &self,
            _session_id: &str,
            _role: &str,
            _content: &str,
            _binding: ConversationRuntimeBinding<'_>,
        ) -> CliResult<()> {
            Ok(())
        }
    }

    let reply = request_completion_with_raw_fallback_detailed(
        &ToolMarkupRuntime,
        &LoongConfig::default(),
        &[],
        ConversationRuntimeBinding::direct(),
        "fallback body",
        None,
    )
    .await;

    assert_eq!(reply.state, None);
    assert_eq!(
        reply.reply,
        "Example Domain is a reserved placeholder page."
    );
}

#[test]
fn tool_driven_followup_payload_reports_result_kind_and_context() {
    let payload = ToolDrivenFollowupPayload::ToolResult {
        text: "tool output".to_owned(),
    };
    let message_context = payload.message_context();

    assert_eq!(payload.kind(), ToolDrivenFollowupKind::ToolResult);
    assert_eq!(message_context.label(), ToolDrivenFollowupLabel::ToolResult);
    assert_eq!(message_context.label().as_str(), "tool_result");
    assert_eq!(message_context.text(), "tool output");
}

#[test]
fn tool_driven_followup_payload_strips_think_tags_from_tool_result_text() {
    let turn_result =
        TurnResult::FinalText("<think>internal reasoning</think>visible tool output".to_owned());

    assert_eq!(
        tool_driven_followup_payload(true, &turn_result),
        Some(ToolDrivenFollowupPayload::ToolResult {
            text: "visible tool output".to_owned(),
        })
    );
}

#[test]
fn tool_driven_followup_payload_reports_failure_kind_and_context() {
    let payload = ToolDrivenFollowupPayload::ToolFailure {
        reason: "tool failed".to_owned(),
        retryable: false,
    };
    let message_context = payload.message_context();

    assert_eq!(payload.kind(), ToolDrivenFollowupKind::ToolFailure);
    assert_eq!(
        message_context.label(),
        ToolDrivenFollowupLabel::ToolFailure
    );
    assert_eq!(message_context.label().as_str(), "tool_failure");
    assert_eq!(message_context.text(), "tool failed");
    assert!(!payload.retryable_failure());
}

#[test]
fn tool_driven_followup_payload_reports_discovery_recovery_context() {
    let payload = ToolDrivenFollowupPayload::DiscoveryRecovery {
        reason: "tool_not_found: requested tool is not available".to_owned(),
    };
    let message_context = payload.message_context();

    assert_eq!(payload.kind(), ToolDrivenFollowupKind::DiscoveryRecovery);
    assert_eq!(
        message_context.label(),
        ToolDrivenFollowupLabel::DiscoveryRecovery
    );
    assert_eq!(message_context.label().as_str(), "tool_recovery");
    assert_eq!(
        message_context.text(),
        "tool_not_found: requested tool is not available"
    );
}

#[test]
fn tool_driven_followup_kind_serializes_as_snake_case() {
    assert_eq!(
        serde_json::to_value(ToolDrivenFollowupKind::ToolResult).expect("serialize kind"),
        Value::String("tool_result".to_owned())
    );
    assert_eq!(
        serde_json::to_value(ToolDrivenFollowupKind::ToolFailure).expect("serialize kind"),
        Value::String("tool_failure".to_owned())
    );
    assert_eq!(
        serde_json::to_value(ToolDrivenFollowupKind::DiscoveryRecovery).expect("serialize kind"),
        Value::String("discovery_recovery".to_owned())
    );
}

#[test]
fn parse_tool_driven_continuation_reply_extracts_leading_marker() {
    let parsed = parse_tool_driven_continuation_reply(
        "<think>hidden</think>\n[followup_state:continue]\nNow merging the files.",
    );

    assert_eq!(parsed.state, Some(ToolDrivenContinuationState::Continue));
    assert_eq!(parsed.reply, "Now merging the files.");
    assert_eq!(
        sanitize_reply_text("[followup_state:done]\nFinished."),
        "Finished."
    );
}

#[test]
fn parse_tool_driven_continuation_reply_leaves_unknown_marker_text_intact() {
    let parsed =
        parse_tool_driven_continuation_reply("[followup_state:unknown]\nkeep this literal");

    assert_eq!(parsed.state, None);
    assert_eq!(parsed.reply, "[followup_state:unknown]\nkeep this literal");
}

#[test]
fn render_tool_followup_continuation_contract_includes_repair_and_retryable_context() {
    let repair = render_tool_followup_continuation_contract(
        ToolDrivenFollowupContractMode::RepairRetryableFailure,
    );

    assert!(repair.contains("[followup_state:continue]"));
    assert!(repair.contains("[followup_state:done]"));
    assert!(repair.contains("[followup_state:blocked]"));
    assert!(repair.contains("Do not describe the plan again"));
    assert!(repair.contains("retryable"));
}

#[test]
fn missing_tool_call_followup_detects_pseudo_tool_commands() {
    let payload = missing_tool_call_followup_payload(
        "/workspace:df -h\n/tool.search:disk usage\n/web:disk usage command line",
    )
    .expect("pseudo-tool lines should trigger missing-tool-call recovery");

    let ToolDrivenFollowupPayload::ToolFailure { reason, retryable } = payload else {
        panic!("expected tool failure payload");
    };

    assert!(reason.contains("pseudo-tool text"));
    assert!(reason.contains("Reply excerpt"));
    assert!(retryable);
}

#[test]
fn missing_tool_call_followup_detects_to_equals_pseudo_tool_commands() {
    let payload = missing_tool_call_followup_payload("to=read content='{\"path\":\"README.md\"}'")
        .expect("to= pseudo-tool text should trigger missing-tool-call recovery");

    let ToolDrivenFollowupPayload::ToolFailure { reason, retryable } = payload else {
        panic!("expected tool failure payload");
    };

    assert!(reason.contains("pseudo-tool text"));
    assert!(retryable);
}

#[test]
fn missing_tool_call_followup_detects_to_equals_pseudo_tool_commands_with_leading_noise() {
    let payload = missing_tool_call_followup_payload(
        "### to=web json\n{\"url\":\"https://example.com\"}summary",
    )
    .expect("noisy to= pseudo-tool text should trigger missing-tool-call recovery");

    let ToolDrivenFollowupPayload::ToolFailure { reason, retryable } = payload else {
        panic!("expected tool failure payload");
    };

    assert!(reason.contains("pseudo-tool text"));
    assert!(retryable);
}

#[test]
fn missing_tool_call_followup_truncates_long_reply_excerpt() {
    let long_excerpt = format!(
        "/tool.search:{}",
        "x".repeat(MISSING_TOOL_CALL_REPLY_EXCERPT_CHARS + 40)
    );
    let payload = missing_tool_call_followup_payload(long_excerpt.as_str())
        .expect("pseudo-tool excerpt should still produce recovery payload");

    let ToolDrivenFollowupPayload::ToolFailure { reason, retryable } = payload else {
        panic!("expected tool failure payload");
    };

    assert!(reason.contains("[reply_excerpt_truncated]"));
    assert!(reason.contains("omitted_chars="));
    assert!(retryable);
}

#[test]
fn missing_tool_call_followup_detects_empty_followup() {
    let payload = missing_tool_call_followup_payload("   ")
        .expect("empty followup should trigger missing-tool-call recovery");

    let ToolDrivenFollowupPayload::ToolFailure { reason, retryable } = payload else {
        panic!("expected tool failure payload");
    };

    assert!(reason.contains("without any content or tool call"));
    assert!(retryable);
}

#[test]
fn missing_tool_call_followup_detects_tool_request_markup_with_glued_summary_text() {
    let payload = missing_tool_call_followup_payload(
        "[tool_request]\n{\"url\":\"https://example.com\"},\"name\":\"web\"}page summary here",
    )
    .expect("tool-request markup should trigger missing-tool-call recovery");

    let ToolDrivenFollowupPayload::ToolFailure { reason, retryable } = payload else {
        panic!("expected tool failure payload");
    };

    assert!(reason.contains("malformed tool-call markup"));
    assert!(reason.contains("Reply excerpt"));
    assert!(retryable);
}

#[test]
fn missing_tool_call_followup_ignores_normal_final_answer_text() {
    let payload = missing_tool_call_followup_payload(
        "The disk is nearly full because the cache directory is consuming most of the space.",
    );

    assert!(payload.is_none());
}

#[test]
fn turn_failure_supports_discovery_recovery_requires_structured_metadata() {
    let recovery_failure = TurnFailure::policy_denied_with_discovery_recovery(
        "invalid_tool_lease",
        "The previous hidden-tool lease is stale. Refresh the visible tool request and continue with a fresh lease.",
    );
    let plain_failure = TurnFailure::policy_denied(
        "invalid_tool_lease",
        "tool execution failed: invalid_tool_lease: expired lease",
    );

    assert!(turn_failure_supports_discovery_recovery(&recovery_failure));
    assert!(!turn_failure_supports_discovery_recovery(&plain_failure));
}

#[test]
fn tool_result_line_roundtrip_preserves_envelope() {
    let envelope = ToolResultEnvelope {
        status: "ok".to_owned(),
        tool: "file.read".to_owned(),
        tool_call_id: "call-1".to_owned(),
        payload_semantics: None,
        payload_summary: json!({
            "path": "README.md",
            "content": "hello"
        })
        .to_string(),
        payload_chars: 42,
        payload_truncated: false,
    };
    let tool_result_line = ToolResultLine::new("ok", envelope.clone());
    let rendered = tool_result_line.render().expect("render tool result line");
    let reparsed = ToolResultLine::parse(rendered.as_str()).expect("parse tool result line");

    assert_eq!(reparsed.envelope(), &envelope);
    assert_eq!(reparsed.tool_name(), "file.read");
    assert!(!reparsed.payload_truncated());
}

#[test]
fn tool_driven_followup_message_owned_parses_typed_assistant_marker() {
    let message = ToolDrivenFollowupTextRef::new(
        ToolDrivenFollowupLabel::DiscoveryRecovery,
        "tool_not_found: requested tool is not available",
    )
    .render_assistant_content();
    let parsed = ToolDrivenFollowupMessageOwned::parse_assistant_content(message.as_str())
        .expect("parse assistant followup content");

    assert_eq!(parsed.label(), ToolDrivenFollowupLabel::DiscoveryRecovery);
    assert_eq!(
        parsed.body(),
        "tool_not_found: requested tool is not available"
    );
}

#[test]
fn reply_resolution_mode_serializes_as_snake_case() {
    assert_eq!(
        serde_json::to_value(ReplyResolutionMode::Direct).expect("serialize mode"),
        Value::String("direct".to_owned())
    );
    assert_eq!(
        serde_json::to_value(ReplyResolutionMode::CompletionPass).expect("serialize mode"),
        Value::String("completion_pass".to_owned())
    );
}

#[test]
fn tool_driven_reply_kernel_base_decision_finalizes_non_tool_reply_directly() {
    let result = TurnResult::FinalText("plain reply".to_owned());
    let kernel = ToolDrivenReplyKernel::new("preface", false, &result);

    assert_eq!(
        kernel.base_decision(false),
        ToolDrivenReplyBaseDecision::FinalizeDirect {
            reply: "plain reply".to_owned(),
        }
    );
}

#[test]
fn tool_driven_reply_kernel_base_decision_honors_raw_tool_output_mode() {
    let result = TurnResult::FinalText("tool output".to_owned());
    let kernel = ToolDrivenReplyKernel::new("preface", true, &result);

    assert_eq!(
        kernel.base_decision(true),
        ToolDrivenReplyBaseDecision::FinalizeDirect {
            reply: "preface\ntool output".to_owned(),
        }
    );
}

#[test]
fn tool_driven_reply_kernel_base_decision_requires_followup_for_tool_failure() {
    let result = TurnResult::ToolError(TurnFailure::retryable("tool_error", "temporary failure"));
    let kernel = ToolDrivenReplyKernel::new("preface", true, &result);

    assert_eq!(
        kernel.base_decision(false),
        ToolDrivenReplyBaseDecision::RequireFollowup {
            raw_reply: "preface\ntemporary failure".to_owned(),
            payload: ToolDrivenFollowupPayload::ToolFailure {
                reason: "temporary failure".to_owned(),
                retryable: true,
            },
        }
    );
}

#[test]
fn tool_driven_reply_base_decision_reports_followup_kind_only_for_followup_paths() {
    let direct = ToolDrivenReplyBaseDecision::FinalizeDirect {
        reply: "reply".to_owned(),
    };
    let followup = ToolDrivenReplyBaseDecision::RequireFollowup {
        raw_reply: "raw".to_owned(),
        payload: ToolDrivenFollowupPayload::ToolResult {
            text: "tool output".to_owned(),
        },
    };

    assert_eq!(direct.resolution_mode(), ReplyResolutionMode::Direct);
    assert_eq!(
        followup.resolution_mode(),
        ReplyResolutionMode::CompletionPass
    );
    assert_eq!(direct.followup_kind(), None);
    assert_eq!(
        followup.followup_kind(),
        Some(ToolDrivenFollowupKind::ToolResult)
    );
}

#[test]
fn tool_driven_reply_phase_finalizes_non_tool_reply_directly() {
    let result = TurnResult::FinalText("plain reply".to_owned());
    let phase = ToolDrivenReplyPhase::new("preface", false, false, &result);

    assert_eq!(phase.resolution_mode(), ReplyResolutionMode::Direct);
    assert_eq!(phase.followup_kind(), None);
    assert_eq!(
        phase.decision(),
        &ToolDrivenReplyBaseDecision::FinalizeDirect {
            reply: "plain reply".to_owned(),
        }
    );
}

#[test]
fn tool_driven_reply_phase_requires_followup_for_tool_success() {
    let result = TurnResult::FinalText("tool output".to_owned());
    let phase = ToolDrivenReplyPhase::new("preface", true, false, &result);

    assert_eq!(phase.resolution_mode(), ReplyResolutionMode::CompletionPass);
    assert_eq!(
        phase.followup_kind(),
        Some(ToolDrivenFollowupKind::ToolResult)
    );
    assert_eq!(
        phase.decision(),
        &ToolDrivenReplyBaseDecision::RequireFollowup {
            raw_reply: "preface\ntool output".to_owned(),
            payload: ToolDrivenFollowupPayload::ToolResult {
                text: "tool output".to_owned(),
            },
        }
    );
}

#[test]
fn tool_driven_reply_phase_requires_followup_for_tool_failure() {
    let result = TurnResult::ToolError(TurnFailure::retryable("tool_error", "temporary failure"));
    let phase = ToolDrivenReplyPhase::new("preface", true, false, &result);

    assert_eq!(phase.resolution_mode(), ReplyResolutionMode::CompletionPass);
    assert_eq!(
        phase.followup_kind(),
        Some(ToolDrivenFollowupKind::ToolFailure)
    );
    assert_eq!(
        phase.decision(),
        &ToolDrivenReplyBaseDecision::RequireFollowup {
            raw_reply: "preface\ntemporary failure".to_owned(),
            payload: ToolDrivenFollowupPayload::ToolFailure {
                reason: "temporary failure".to_owned(),
                retryable: true,
            },
        }
    );
}

#[test]
fn tool_driven_reply_phase_finalizes_approval_requirement_directly() {
    let result = TurnResult::NeedsApproval(ApprovalRequirement {
        kind: ApprovalRequirementKind::GovernedTool,
        reason: "operator approval required for governed tool".to_owned(),
        rule_id: "governed_tool_requires_approval".to_owned(),
        tool_name: Some("delegate_async".to_owned()),
        approval_key: Some("tool:delegate_async".to_owned()),
        approval_request_id: Some("apr_direct".to_owned()),
    });
    let phase = ToolDrivenReplyPhase::new("preface", true, false, &result);

    assert_eq!(phase.resolution_mode(), ReplyResolutionMode::Direct);
    assert_eq!(phase.followup_kind(), None);
    assert_eq!(
        phase.raw_reply(),
        Some(
            "preface\n[tool_approval_required]\ntool: delegate_async\nrequest_id: apr_direct\nrule_id: governed_tool_requires_approval\nreason: operator approval required for governed tool\nallowed_decisions: yes / auto / full / esc\nExecute only this tool call\nLow-risk tools continue automatically\nWrites, shell exec, provider switching, and similar actions still pause\nStop asking for tool consent in this session\nGoverned approvals and kernel hard limits still apply\nDo not run this tool call\n\nReply with: yes / auto / full / esc\nyes = run once, auto = session auto mode, full = session full-auto mode, esc = skip this call"
        )
    );
    assert_eq!(
            phase.decision(),
            &ToolDrivenReplyBaseDecision::FinalizeDirect {
                reply: "preface\n[tool_approval_required]\ntool: delegate_async\nrequest_id: apr_direct\nrule_id: governed_tool_requires_approval\nreason: operator approval required for governed tool\nallowed_decisions: yes / auto / full / esc\nExecute only this tool call\nLow-risk tools continue automatically\nWrites, shell exec, provider switching, and similar actions still pause\nStop asking for tool consent in this session\nGoverned approvals and kernel hard limits still apply\nDo not run this tool call\n\nReply with: yes / auto / full / esc\nyes = run once, auto = session auto mode, full = session full-auto mode, esc = skip this call".to_owned(),
            }
        );
}

#[test]
fn tool_driven_reply_phase_exposes_raw_reply_for_tool_success() {
    let result = TurnResult::FinalText("tool output".to_owned());
    let phase = ToolDrivenReplyPhase::new("preface", true, false, &result);

    assert_eq!(phase.raw_reply(), Some("preface\ntool output"));
}

#[test]
fn tool_driven_reply_phase_exposes_raw_reply_for_tool_failure() {
    let result = TurnResult::ToolError(TurnFailure::retryable("tool_error", "temporary failure"));
    let phase = ToolDrivenReplyPhase::new("preface", true, false, &result);

    assert_eq!(phase.raw_reply(), Some("preface\ntemporary failure"));
}

#[test]
fn tool_driven_reply_phase_omits_raw_reply_for_non_tool_paths() {
    let result = TurnResult::FinalText("plain reply".to_owned());
    let phase = ToolDrivenReplyPhase::new("preface", false, false, &result);

    assert_eq!(phase.raw_reply(), None);
}

#[test]
fn tool_driven_reply_phase_raw_mode_bypasses_completion_pass() {
    let result = TurnResult::FinalText("tool output".to_owned());
    let phase = ToolDrivenReplyPhase::new("preface", true, true, &result);

    assert_eq!(phase.resolution_mode(), ReplyResolutionMode::Direct);
    assert_eq!(phase.followup_kind(), None);
    assert_eq!(
        phase.decision(),
        &ToolDrivenReplyBaseDecision::FinalizeDirect {
            reply: "preface\ntool output".to_owned(),
        }
    );
}

#[test]
fn tool_result_followup_tail_promotes_external_skill_without_payload_mapping() {
    let tail = build_tool_result_followup_tail(
        "preface",
        r#"[ok] {"status":"ok","tool":"read","tool_call_id":"call-1","payload_semantics":"skill_context","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":false}"#,
        "summarize note.md",
        Some("warning"),
        |_, _| panic!("external skill payload should bypass payload mapper"),
    );

    assert!(tail.iter().any(|message| {
        message.get("role") == Some(&Value::String("system".to_owned()))
            && message
                .get("content")
                .and_then(Value::as_str)
                .map(|content| {
                    content.contains("Follow the managed skill instruction before answering.")
                })
                .unwrap_or(false)
    }));
    assert!(tail.iter().any(|message| {
        message.get("role") == Some(&Value::String("assistant".to_owned()))
            && message
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content.contains("[tool_loop_warning]\nwarning"))
                .unwrap_or(false)
    }));
    assert!(
        tail.iter()
            .filter_map(|message| message.get("content").and_then(Value::as_str))
            .all(|content| !content.contains("[tool_result]\n[ok]"))
    );
}

#[test]
fn tool_result_followup_tail_promotes_external_skill_from_semantic_envelope() {
    let tail = build_tool_result_followup_tail(
        "preface",
        r#"[ok] {"status":"ok","tool":"read","tool_call_id":"call-1","payload_semantics":"skill_context","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":false}"#,
        "summarize note.md",
        Some("warning"),
        |_, _| panic!("external skill payload should bypass payload mapper"),
    );

    assert!(tail.iter().any(|message| {
        message.get("role") == Some(&Value::String("system".to_owned()))
            && message
                .get("content")
                .and_then(Value::as_str)
                .map(|content| {
                    content.contains("Follow the managed skill instruction before answering.")
                })
                .unwrap_or(false)
    }));
    assert!(
        tail.iter()
            .filter_map(|message| message.get("content").and_then(Value::as_str))
            .all(|content| !content.contains("[tool_result]\n[ok]"))
    );
}

#[test]
fn tool_result_followup_tail_uses_payload_mapper_and_keeps_truncation_hint() {
    let tail = build_tool_result_followup_tail(
        "preface",
        r#"[ok] {"payload_truncated":true}"#,
        "summarize note.md",
        Some("warning"),
        |_, _| "bounded-result".to_owned(),
    );

    assert!(tail.iter().any(|message| {
        message.get("role") == Some(&Value::String("assistant".to_owned()))
            && message
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content == "[tool_result]\nbounded-result")
                .unwrap_or(false)
    }));
    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");
    assert!(user_prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
    assert!(user_prompt.contains("Loop warning:\nwarning"));
}

#[test]
fn tool_result_followup_tail_keeps_truncation_hint_when_payload_mapper_marks_result_truncated() {
    let tail = build_tool_result_followup_tail(
        "preface",
        r#"[ok] {"payload_truncated":false}"#,
        "summarize note.md",
        Some("warning"),
        |_, _| r#"[ok] {"payload_truncated":true}"#.to_owned(),
    );

    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");
    assert!(user_prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
    assert!(user_prompt.contains("Loop warning:\nwarning"));
}

#[test]
fn tool_failure_followup_tail_uses_payload_mapper_without_truncation_hint() {
    let tail = build_tool_failure_followup_tail(
        "preface",
        "tool_timeout ...(truncated 200 chars)",
        None,
        "summarize note.md",
        Some("warning"),
        |_, _| "bounded-failure".to_owned(),
    );

    assert!(tail.iter().any(|message| {
        message.get("role") == Some(&Value::String("assistant".to_owned()))
            && message
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content == "[tool_failure]\nbounded-failure")
                .unwrap_or(false)
    }));
    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");
    assert!(!user_prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
    assert!(user_prompt.contains("Loop warning:\nwarning"));
}

#[test]
fn tool_driven_followup_tail_dispatches_result_payload() {
    let payload = ToolDrivenFollowupPayload::ToolResult {
        text: r#"[ok] {"payload_truncated":true}"#.to_owned(),
    };
    let tail = build_tool_driven_followup_tail(
        "preface",
        &payload,
        None,
        "summarize note.md",
        Some("warning"),
        |_, _| "bounded-result".to_owned(),
    );

    assert!(tail.iter().any(|message| {
        message.get("role") == Some(&Value::String("assistant".to_owned()))
            && message
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content == "[tool_result]\nbounded-result")
                .unwrap_or(false)
    }));
    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");
    assert!(user_prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
    assert!(user_prompt.contains("Loop warning:\nwarning"));
}

#[test]
fn tool_driven_followup_tail_dispatches_failure_payload() {
    let payload = ToolDrivenFollowupPayload::ToolFailure {
        reason: "tool_timeout ...(truncated 200 chars)".to_owned(),
        retryable: false,
    };
    let tail = build_tool_driven_followup_tail(
        "preface",
        &payload,
        None,
        "summarize note.md",
        Some("warning"),
        |_, _| "bounded-failure".to_owned(),
    );

    assert!(tail.iter().any(|message| {
        message.get("role") == Some(&Value::String("assistant".to_owned()))
            && message
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content == "[tool_failure]\nbounded-failure")
                .unwrap_or(false)
    }));
    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");
    assert!(!user_prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
    assert!(user_prompt.contains("Loop warning:\nwarning"));
}

#[test]
fn tool_driven_followup_tail_preserves_request_summary_for_failure_payloads() {
    let payload = ToolDrivenFollowupPayload::ToolFailure {
        reason: "payload.command contains path separators".to_owned(),
        retryable: false,
    };
    let tool_request_summary =
        r#"{"tool":"bash","request":{"command":"C:\\Windows\\System32\\RM.EXE"}}"#;
    let tail = build_tool_driven_followup_tail(
        "preface",
        &payload,
        Some(tool_request_summary),
        "summarize note.md",
        Some("warning"),
        |label, _| match label {
            "tool_request" => "bounded-request".to_owned(),
            _ => "bounded-failure".to_owned(),
        },
    );

    assert!(tail.iter().any(|message| {
        message.get("role") == Some(&Value::String("assistant".to_owned()))
            && message
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content == "[tool_request]\nbounded-request")
                .unwrap_or(false)
    }));
    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");
    assert!(user_prompt.contains("Repair guidance for bash"));
    assert!(user_prompt.contains("retry with `rm.exe`"));
}

#[test]
fn tool_driven_followup_tail_dispatches_discovery_recovery_payload() {
    let payload = ToolDrivenFollowupPayload::DiscoveryRecovery {
        reason: "tool_not_found: tool.invoke".to_owned(),
    };
    let tail = build_tool_driven_followup_tail(
        "preface",
        &payload,
        None,
        "summarize note.md",
        Some("warning"),
        |_, _| "bounded-recovery".to_owned(),
    );

    assert!(tail.iter().any(|message| {
        message.get("role") == Some(&Value::String("assistant".to_owned()))
            && message
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content == "[tool_recovery]\nbounded-recovery")
                .unwrap_or(false)
    }));
    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");
    assert!(user_prompt.contains(prompt::TOOL_FAILURE_FOLLOWUP_PROMPT));
    assert!(user_prompt.contains("Recovery reason:\nbounded-recovery"));
    assert!(!user_prompt.contains("tool_not_found"));
    assert!(user_prompt.contains("Retry with a valid direct tool call"));
    assert!(
        user_prompt.contains("refreshed visible tool request"),
        "discovery recovery prompt should mention the direct-tool refresh path: {user_prompt}"
    );
    assert!(user_prompt.contains("Loop warning:\nwarning"));
}

#[test]
fn tool_loop_guard_tail_uses_payload_mapper_and_builds_guard_prompt() {
    let latest_tool_context =
        ToolDrivenFollowupTextRef::new(ToolDrivenFollowupLabel::ToolResult, "tool output");
    let tail = build_tool_loop_guard_tail(
        "preface",
        "stop",
        "summarize note.md",
        Some(latest_tool_context),
        |_, _| "bounded-result".to_owned(),
    );

    assert!(tail.iter().any(|message| {
        message.get("role") == Some(&Value::String("assistant".to_owned()))
            && message
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content == "preface")
                .unwrap_or(false)
    }));
    assert!(tail.iter().any(|message| {
        message.get("role") == Some(&Value::String("assistant".to_owned()))
            && message
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content == "[tool_result]\nbounded-result")
                .unwrap_or(false)
    }));
    assert!(tail.iter().any(|message| {
        message.get("role") == Some(&Value::String("assistant".to_owned()))
            && message
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content == "[tool_loop_guard]\nstop")
                .unwrap_or(false)
    }));
    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");
    assert!(user_prompt.contains(TOOL_LOOP_GUARD_PROMPT));
    assert!(user_prompt.contains("Loop guard reason:\nstop"));
    assert!(user_prompt.contains("Original request:\nsummarize note.md"));
}

#[test]
fn tool_failure_followup_tail_strips_shell_arguments_from_repair_guidance() {
    let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason: "tool_preflight_denied: tool input needs repair: shell.exec payload.command must be a bare executable name; move arguments into payload.args.".to_owned(),
            retryable: false,
        };
    let tool_request_summary = r#"{"tool":"bash","request":{"command":"ls -la"}}"#;
    let tail = build_tool_driven_followup_tail(
        "preface",
        &payload,
        Some(tool_request_summary),
        "list the current directory",
        None,
        |_, text| text.to_owned(),
    );

    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");

    assert!(user_prompt.contains("Repair guidance for bash"));
    assert!(user_prompt.contains("The failed request used `ls -la`; retry with `ls`"));
}

#[test]
fn tool_failure_followup_tail_strips_quoted_shell_arguments_from_repair_guidance() {
    let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason: "tool_preflight_denied: tool input needs repair: shell.exec payload.command must be a bare executable name; move arguments into payload.args.".to_owned(),
            retryable: false,
        };
    let tool_request_summary = r#"{"tool":"bash","request":{"command":"\"ls -la\" "}}"#;
    let tail = build_tool_driven_followup_tail(
        "preface",
        &payload,
        Some(tool_request_summary),
        "list the current directory",
        None,
        |_, text| text.to_owned(),
    );

    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");

    assert!(user_prompt.contains("Repair guidance for bash"));
    assert!(user_prompt.contains("The failed request used `\"ls -la\" `; retry with `ls`"));
}

#[test]
fn tool_failure_followup_tail_renders_required_field_guidance_for_file_read() {
    let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason:
                "tool_preflight_denied: tool input needs repair: file.read payload.path is required (string)"
                    .to_owned(),
            retryable: false,
        };
    let tool_request_summary = r#"{"tool":"read","request":{}}"#;
    let tail = build_tool_driven_followup_tail(
        "preface",
        &payload,
        Some(tool_request_summary),
        "read the file",
        None,
        |_, text| text.to_owned(),
    );

    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");

    assert!(user_prompt.contains("Repair guidance for read"));
    assert!(user_prompt.contains("Add required field `payload.path` as a string."));
    assert!(user_prompt.contains(
        "Expected payload shape: path:string,offset?:integer,limit?:integer,max_bytes?:integer."
    ));
}

#[test]
fn tool_failure_followup_tail_renders_hidden_agent_operation_guidance() {
    let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason: "hidden_agent_requires_operation: provide `operation` for archive, cancel, recover, or other multi-session control work".to_owned(),
            retryable: true,
        };
    let tool_request_summary = r#"{"tool":"agent","request":{"session_ids":["child-1"]}}"#;
    let tail = build_tool_driven_followup_tail(
        "preface",
        &payload,
        Some(tool_request_summary),
        "archive these sessions",
        None,
        |_, text| text.to_owned(),
    );

    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");

    assert!(user_prompt.contains("Repair guidance for agent"));
    assert!(user_prompt.contains("Add `operation`"));
    assert!(user_prompt.contains(r#"Current request preview: {"session_ids":["child-1"]}"#));
}

#[test]
fn tool_failure_followup_tail_renders_hidden_skills_actionable_guidance() {
    let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason:
                "hidden_skills_requires_actionable_fields: provide actionable fields for grouped skills requests"
                    .to_owned(),
            retryable: true,
        };
    let tool_request_summary = r#"{"tool":"skills","request":{"query":"browser companion"}}"#;
    let tail = build_tool_driven_followup_tail(
        "preface",
        &payload,
        Some(tool_request_summary),
        "inspect the browser companion skill",
        None,
        |_, text| text.to_owned(),
    );

    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");

    assert!(user_prompt.contains("Repair guidance for skills"));
    assert!(user_prompt.contains("search, inspect, install, run, or list fields"));
    assert!(user_prompt.contains(r#"Current request preview: {"query":"browser companion"}"#));
}

#[test]
fn tool_failure_followup_tail_renders_hidden_channel_operation_guidance() {
    let payload = ToolDrivenFollowupPayload::ToolFailure {
            reason: "hidden_channel_requires_operation: provide `operation`, such as `messages.send`, `messages.reply`, `card.update`, or `feishu.whoami`".to_owned(),
            retryable: true,
        };
    let tool_request_summary = r#"{"tool":"channel","request":{"account_id":"default"}}"#;
    let tail = build_tool_driven_followup_tail(
        "preface",
        &payload,
        Some(tool_request_summary),
        "send a message",
        None,
        |_, text| text.to_owned(),
    );

    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");

    assert!(user_prompt.contains("Repair guidance for channel"));
    assert!(user_prompt.contains("messages.send"));
    assert!(user_prompt.contains(r#"Current request preview: {"account_id":"default"}"#));
}

#[test]
fn tool_failure_followup_tail_renders_max_bytes_retry_guidance_for_web() {
    let payload = ToolDrivenFollowupPayload::ToolFailure {
        reason: "tool execution failed: web.fetch response exceeded max_bytes limit (120000 bytes)"
            .to_owned(),
        retryable: true,
    };
    let tool_request_summary =
        r#"{"name":"web","arguments":{"url":"https://example.com","max_bytes":120000}}"#;
    let tail = build_tool_driven_followup_tail(
        "preface",
        &payload,
        Some(tool_request_summary),
        "看一下 https://example.com",
        None,
        |_, text| text.to_owned(),
    );

    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");

    assert!(user_prompt.contains("Repair guidance for web"));
    assert!(user_prompt.contains("Suggested retry:"));
    assert!(user_prompt.contains("\"name\": \"web\""));
    assert!(user_prompt.contains("\"max_bytes\": 32768"));
}

#[test]
fn tool_failure_followup_tail_uses_failure_reason_when_shell_summary_redacts_args_type() {
    let payload = ToolDrivenFollowupPayload::ToolFailure {
        reason:
            "tool_preflight_denied: tool input needs repair: shell.exec payload.args must be array"
                .to_owned(),
        retryable: false,
    };
    let tool_request_summary = r#"{"tool":"bash","request":{"command":"echo"}}"#;
    let tail = build_tool_driven_followup_tail(
        "preface",
        &payload,
        Some(tool_request_summary),
        "run echo safely",
        None,
        |_, text| text.to_owned(),
    );

    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");

    assert!(user_prompt.contains("Repair guidance for bash"));
    assert!(user_prompt.contains("Set `payload.args` to an array value."));
    assert!(user_prompt.contains(
        "Expected payload shape: command:string,args?:string[],timeout_ms?:integer,cwd?:string."
    ));
}

#[test]
fn tool_loop_guard_tail_includes_truncation_hint_when_payload_mapper_truncates_result() {
    let latest_tool_context = ToolDrivenFollowupTextRef::new(
        ToolDrivenFollowupLabel::ToolResult,
        r#"[ok] {"payload_truncated":false}"#,
    );
    let tail = build_tool_loop_guard_tail(
        "preface",
        "stop",
        "summarize note.md",
        Some(latest_tool_context),
        |_, _| r#"[ok] {"payload_truncated":true}"#.to_owned(),
    );

    let user_prompt = tail
        .last()
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .expect("user followup prompt should exist");
    assert!(user_prompt.contains(TOOL_LOOP_GUARD_PROMPT));
    assert!(user_prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
    assert!(user_prompt.contains("Loop guard reason:\nstop"));
}

#[test]
fn tool_loop_guard_tail_skips_latest_tool_context_without_payload_mapping() {
    let tail = build_tool_loop_guard_tail("", "stop", "summarize note.md", None, |_, _| {
        panic!("missing latest tool context should bypass payload mapper")
    });

    assert_eq!(tail.len(), 2);
    assert_eq!(tail[0]["role"], "assistant");
    assert_eq!(tail[0]["content"], "[tool_loop_guard]\nstop");
    assert_eq!(tail[1]["role"], "user");
}

#[test]
fn truncation_signal_detection_matches_structured_tool_result() {
    assert!(tool_result_contains_truncation_signal(
        r#"[ok] {"payload_truncated":true}"#
    ));
    assert!(tool_result_contains_truncation_signal(
        "payload ... (truncated 200 chars)"
    ));
    assert!(!tool_result_contains_truncation_signal(
        r#"[ok] {"payload_truncated":false}"#
    ));
}

#[test]
fn truncation_signal_detection_ignores_payload_summary_lookalikes() {
    let deceptive_line = format!(
        "[ok] {}",
        json!({
            "status": "ok",
            "payload_summary": r#"{"payload_truncated":true}"#,
            "payload_truncated": false
        })
    );
    assert!(!tool_result_contains_truncation_signal(
        deceptive_line.as_str()
    ));
}

#[test]
fn followup_prompt_includes_truncation_hint_when_needed() {
    let prompt = build_tool_followup_user_prompt(
        "summarize this result",
        None,
        Some(r#"[ok] {"payload_truncated":true}"#),
        None,
        None,
    );
    assert!(prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
    assert!(prompt.contains("Original request:\nsummarize this result"));
}

#[test]
fn followup_prompt_includes_truncation_hint_when_rendered_payload_is_truncated() {
    let prompt = build_tool_followup_user_prompt(
        "summarize this result",
        None,
        Some(r#"[ok] {"payload_truncated":false}"#),
        Some(r#"[ok] {"payload_truncated":true}"#),
        None,
    );
    assert!(prompt.contains(TOOL_TRUNCATION_HINT_PROMPT));
    assert!(prompt.contains("Original request:\nsummarize this result"));
}

#[test]
fn followup_prompt_appends_extra_context_without_dropping_original_request() {
    let prompt = build_tool_followup_user_prompt_with_context(
        "summarize this result",
        None,
        None,
        None,
        Some("External skill context:\nUse terse output."),
    );

    assert!(prompt.contains("External skill context:\nUse terse output."));
    assert!(prompt.contains("Original request:\nsummarize this result"));
}

#[test]
fn followup_prompt_uses_discovery_guidance_for_discovery_shaped_results() {
    let payload_summary = json!({
        "query": "latest ai news",
        "results": [
            {
                "tool_id": "web.search",
                "lease": "lease-web-search"
            }
        ]
    })
    .to_string();
    let tool_result = format!(
        "[ok] {}",
        json!({
            "status": "ok",
            "tool": "file.read",
            "tool_call_id": "call-search",
            "payload_summary": payload_summary,
            "payload_chars": 512,
            "payload_truncated": false
        })
    );

    let prompt = build_tool_followup_user_prompt(
        "find the latest ai news and summarize it",
        None,
        Some(tool_result.as_str()),
        None,
        None,
    );

    assert!(prompt.contains(prompt::TOOL_FOLLOWUP_PROMPT));
    assert!(prompt.contains(
        "If the result is only a path listing, metadata summary, or other partial evidence"
    ));
    assert!(prompt.contains("Original request:\nfind the latest ai news and summarize it"));
}

#[test]
fn followup_prompt_treats_path_listing_as_partial_evidence() {
    let tool_result = format!(
        "[ok] {}",
        json!({
            "status": "ok",
            "tool": "read",
            "tool_call_id": "call-read-listing",
            "payload_summary": json!({
                "tool_name": "glob.search",
                "matches": [
                    {"path": "README.md", "kind": "file"},
                    {"path": "AGENTS.md", "kind": "file"}
                ]
            })
            .to_string(),
            "payload_chars": 256,
            "payload_truncated": false
        })
    );

    let prompt = build_tool_followup_user_prompt(
        "summarize this repository and suggest the best next step",
        None,
        Some(tool_result.as_str()),
        None,
        None,
    );

    assert!(prompt.contains("The last read result only listed matching paths."));
    assert!(prompt.contains("continue with another direct `read` call"));
}

#[test]
fn followup_prompt_uses_generic_continuation_metadata_for_delegate_queue() {
    let payload_summary = json!({
        "child_session_id": "delegate:child-1",
        "mode": "async",
        "state": "queued",
        "continuation": {
            "state": "queued",
            "is_terminal": false,
            "recommended_tool": "session_wait",
            "recommended_payload": {
                "session_id": "delegate:child-1",
                "timeout_ms": 30000
            },
            "note": "The delegated child is still running in the background."
        }
    })
    .to_string();
    let tool_result = format!(
        "[ok] {}",
        json!({
            "status": "ok",
            "tool": "delegate_async",
            "tool_call_id": "call-delegate-1",
            "payload_summary": payload_summary,
            "payload_chars": 256,
            "payload_truncated": false
        })
    );

    let prompt = build_tool_followup_user_prompt(
        "finish the delegated research and summarize the result",
        None,
        Some(tool_result.as_str()),
        None,
        None,
    );

    assert!(prompt.contains("Continuation guidance:"));
    assert!(prompt.contains("intermediate state `queued`"));
    assert!(prompt.contains("still running in the background"));
    assert!(prompt.contains("`session_wait`"));
    assert!(prompt.contains("{\"session_id\":\"delegate:child-1\",\"timeout_ms\":30000}"));
}

#[test]
fn followup_prompt_uses_generic_continuation_metadata_for_waiting_task() {
    let payload_summary = json!({
        "wait_status": "waiting",
        "continuation": {
            "state": "waiting",
            "is_terminal": false,
            "note": "The runtime is still waiting on an approval or external completion gate."
        }
    })
    .to_string();
    let tool_result = format!(
        "[ok] {}",
        json!({
            "status": "ok",
            "tool": "task_wait",
            "tool_call_id": "call-task-wait-1",
            "payload_summary": payload_summary,
            "payload_chars": 256,
            "payload_truncated": false
        })
    );

    let prompt = build_tool_followup_user_prompt(
        "wait until the task is complete and then summarize it",
        None,
        Some(tool_result.as_str()),
        None,
        None,
    );

    assert!(prompt.contains("Continuation guidance:"));
    assert!(prompt.contains("intermediate state `waiting`"));
    assert!(prompt.contains("approval or external completion gate"));
    assert!(prompt.contains("exact blocker"));
}

#[test]
fn tool_result_payload_requests_runtime_followup_chain_for_nonterminal_continuation() {
    let payload = ToolDrivenFollowupPayload::ToolResult {
        text: format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "session_wait",
                "tool_call_id": "call-session-wait",
                "payload_summary": json!({
                    "wait_status": "waiting",
                    "continuation": {
                        "state": "waiting",
                        "is_terminal": false,
                        "recommended_tool": "session_wait",
                        "recommended_payload": {
                            "session_id": "child-session",
                            "timeout_ms": 1000
                        }
                    }
                })
                .to_string(),
                "payload_chars": 256,
                "payload_truncated": false
            })
        ),
    };

    assert!(payload.requests_runtime_followup_chain());
}

#[test]
fn tool_result_payload_does_not_request_runtime_followup_chain_for_terminal_continuation() {
    let payload = ToolDrivenFollowupPayload::ToolResult {
        text: format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "session_wait",
                "tool_call_id": "call-session-wait",
                "payload_summary": json!({
                    "wait_status": "completed",
                    "continuation": {
                        "state": "completed",
                        "is_terminal": true
                    }
                })
                .to_string(),
                "payload_chars": 256,
                "payload_truncated": false
            })
        ),
    };

    assert!(!payload.requests_runtime_followup_chain());
}

#[test]
fn tool_result_payload_exposes_structured_continuation_kind() {
    let path_listing_payload = ToolDrivenFollowupPayload::ToolResult {
        text: format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "read",
                "tool_call_id": "call-read-path-listing",
                "payload_summary": json!({
                    "continuation": {
                        "state": "path_listing",
                        "is_terminal": false,
                        "recommended_tool": "read",
                        "recommended_payload": {
                            "path": "ARCHITECTURE.md"
                        }
                    }
                })
                .to_string(),
                "payload_chars": 128,
                "payload_truncated": false
            })
        ),
    };
    assert_eq!(
        path_listing_payload
            .tool_result_continuation()
            .map(|continuation| continuation.kind()),
        Some(ToolResultContinuationKind::PathListing)
    );

    let insufficient_page_payload = ToolDrivenFollowupPayload::ToolResult {
        text: format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "web",
                "tool_call_id": "call-web-page",
                "payload_summary": json!({
                    "continuation": {
                        "state": "insufficient_page_evidence",
                        "is_terminal": false,
                        "recommended_tool": "web",
                        "recommended_payload": {
                            "url": "https://example.com"
                        }
                    }
                })
                .to_string(),
                "payload_chars": 128,
                "payload_truncated": false
            })
        ),
    };
    assert_eq!(
        insufficient_page_payload
            .tool_result_continuation()
            .map(|continuation| continuation.kind()),
        Some(ToolResultContinuationKind::InsufficientPageEvidence)
    );
}

#[test]
fn reduce_followup_payload_for_model_preserves_shell_payload_metadata() {
    let payload = json!({
        "adapter": "core-tools",
        "tool_name": "shell.exec",
        "command": "cargo",
        "args": ["test", "--workspace"],
        "cwd": "/repo",
        "exit_code": 0,
        "stdout": format!("prefix {}", "x".repeat(512)),
        "stderr": "",
        "trace_id": "trace-123",
        "details": {
            "truncated": true,
            "handoff": {
                "tool": "read",
                "recommended_stream": "stdout",
                "recommended_recipe": "last_page",
                "recommended_payload": {"path": "/repo/.loongclaw/tool-output/stdout.log", "offset": 801, "limit": 200},
                "recipes": {
                    "stdout": {
                        "recommended_recipe": "last_page",
                        "first_page": {"path": "/repo/.loongclaw/tool-output/stdout.log", "offset": 1, "limit": 200},
                        "last_page": {"path": "/repo/.loongclaw/tool-output/stdout.log", "offset": 801, "limit": 200},
                        "head": {"path": "/repo/.loongclaw/tool-output/stdout.log", "offset": 1, "limit": 200},
                        "tail": {"path": "/repo/.loongclaw/tool-output/stdout.log", "offset": 801, "limit": 200}
                    }
                }
            }
        }
    });
    let line = format!(
        "[ok] {}",
        json!({
            "status": "ok",
            "tool": "shell.exec",
            "tool_call_id": "call-shell",
            "payload_summary": serde_json::to_string(&payload).expect("encode payload"),
            "payload_chars": 8_192,
            "payload_truncated": false
        })
    );

    let reduced = reduce_followup_payload_for_model("tool_result", line.as_str());
    let envelope: Value = serde_json::from_str(
        reduced
            .strip_prefix("[ok] ")
            .expect("tool result line should preserve status prefix"),
    )
    .expect("reduced followup envelope should stay valid json");
    let summary: Value = serde_json::from_str(
        envelope["payload_summary"]
            .as_str()
            .expect("payload summary should stay encoded json"),
    )
    .expect("shell payload summary should stay valid json");

    assert_eq!(envelope["tool"], "bash");
    assert_eq!(summary["adapter"], "core-tools");
    assert_eq!(summary["tool_name"], "shell.exec");
    assert_eq!(summary["trace_id"], "trace-123");
    assert_eq!(summary["command"], "cargo");
    assert_eq!(summary["exit_code"], 0);
    assert!(summary.get("stdout_preview").is_some());
    assert_eq!(summary["stdout_truncated"], true);
    assert_eq!(summary["details"]["handoff"]["tool"], json!("read"));
    assert_eq!(
        summary["details"]["handoff"]["recommended_stream"],
        json!("stdout")
    );
    assert_eq!(
        summary["details"]["handoff"]["recommended_recipe"],
        json!("last_page")
    );
    assert_eq!(
        summary["details"]["handoff"]["recipes"]["stdout"]["recommended_recipe"],
        json!("last_page")
    );
    assert_eq!(
        summary["details"]["handoff"]["recipes"]["stdout"]["last_page"]["offset"],
        json!(801)
    );
    assert_eq!(
        summary["details"]["handoff"]["recommended_payload"]["offset"],
        json!(801)
    );
}

#[test]
fn reduce_followup_payload_for_model_counts_raw_shell_whitespace() {
    let payload = json!({
        "adapter": "core-tools",
        "tool_name": "shell.exec",
        "command": "printf",
        "args": ["%s", " "],
        "cwd": "/repo",
        "exit_code": 0,
        "stdout": " ".repeat(SHELL_FOLLOWUP_STDIO_PREVIEW_CHARS + 32),
        "stderr": "",
    });
    let line = format!(
        "[ok] {}",
        json!({
            "status": "ok",
            "tool": "shell.exec",
            "tool_call_id": "call-shell",
            "payload_summary": serde_json::to_string(&payload).expect("encode payload"),
            "payload_chars": 8_192,
            "payload_truncated": false
        })
    );

    let reduced = reduce_followup_payload_for_model("tool_result", line.as_str());
    let envelope: Value = serde_json::from_str(
        reduced
            .strip_prefix("[ok] ")
            .expect("tool result line should preserve status prefix"),
    )
    .expect("reduced followup envelope should stay valid json");
    let summary: Value = serde_json::from_str(
        envelope["payload_summary"]
            .as_str()
            .expect("payload summary should stay encoded json"),
    )
    .expect("shell payload summary should stay valid json");

    assert_eq!(summary["stdout_truncated"], true);
    assert_eq!(
        summary["stdout_chars"],
        json!(SHELL_FOLLOWUP_STDIO_PREVIEW_CHARS + 32)
    );
    assert_eq!(
        summary["stdout_preview"]
            .as_str()
            .expect("stdout preview should exist")
            .chars()
            .count(),
        SHELL_FOLLOWUP_STDIO_PREVIEW_CHARS
    );
}

#[test]
fn reduce_followup_payload_for_model_preserves_shell_tail_context() {
    let stdout = format!(
        "{}\n{}\n{}",
        "build log ".repeat(80),
        "intermediate output ".repeat(80),
        "final status: test suite failed on browser companion startup"
    );
    let payload = json!({
        "adapter": "core-tools",
        "tool_name": "shell.exec",
        "command": "cargo",
        "args": ["test", "--workspace"],
        "cwd": "/repo",
        "exit_code": 1,
        "stdout": stdout,
        "stderr": "",
    });
    let line = format!(
        "[ok] {}",
        json!({
            "status": "ok",
            "tool": "shell.exec",
            "tool_call_id": "call-shell",
            "payload_summary": serde_json::to_string(&payload).expect("encode payload"),
            "payload_chars": 8_192,
            "payload_truncated": false
        })
    );

    let reduced = reduce_followup_payload_for_model("tool_result", line.as_str());
    let envelope: Value = serde_json::from_str(
        reduced
            .strip_prefix("[ok] ")
            .expect("tool result line should preserve status prefix"),
    )
    .expect("reduced followup envelope should stay valid json");
    let summary: Value = serde_json::from_str(
        envelope["payload_summary"]
            .as_str()
            .expect("payload summary should stay encoded json"),
    )
    .expect("shell payload summary should stay valid json");
    let preview = summary["stdout_preview"]
        .as_str()
        .expect("stdout preview should exist");

    assert!(
        preview.contains("build log"),
        "preview should keep shell prefix"
    );
    assert!(
        preview.contains("final status: test suite failed on browser companion startup"),
        "preview should keep the final shell status"
    );
    assert!(
        preview.contains("[... omitted ...]"),
        "preview should signal when middle content is omitted"
    );
}

#[test]
fn parse_external_skill_invoke_context_extracts_full_instructions_from_semantic_envelope() {
    let instructions = format!("prefix {}\nsuffix-marker", "x".repeat(256));
    let payload = json!({
        "skill_id": "demo-skill",
        "display_name": "Demo Skill",
        "instructions": instructions,
        "metadata": {
            "allowed_tools": ["shell.exec"],
            "blocked_tools": ["web.fetch"]
        }
    });
    let line = format!(
        "[ok] {}",
        json!({
            "status": "ok",
            "tool": "file.read",
            "tool_call_id": "call-1",
            "payload_semantics": "skill_context",
            "payload_summary": serde_json::to_string(&payload).expect("encode payload"),
            "payload_chars": 512,
            "payload_truncated": false
        })
    );

    let parsed = parse_skill_context(line.as_str()).expect("invoke context should parse");
    assert_eq!(parsed.skill_id, "demo-skill");
    assert_eq!(parsed.display_name, "Demo Skill");
    assert!(parsed.instructions.contains("suffix-marker"));
    assert_eq!(parsed.allowed_tools, vec!["shell.exec"]);
    assert_eq!(parsed.blocked_tools, vec!["web.fetch"]);
}

#[test]
fn parse_external_skill_invoke_context_requires_semantics_or_legacy_tool_name() {
    let payload = json!({
        "skill_id": "demo-skill",
        "display_name": "Demo Skill",
        "instructions": "Follow the managed skill instruction before answering.",
    });
    let line = format!(
        "[ok] {}",
        json!({
            "status": "ok",
            "tool": "read",
            "tool_call_id": "call-1",
            "payload_summary": serde_json::to_string(&payload).expect("encode payload"),
            "payload_chars": 512,
            "payload_truncated": false
        })
    );

    assert!(
        parse_skill_context(line.as_str()).is_none(),
        "skill-shaped payloads should not activate managed skill context without explicit external-skill semantics"
    );
}

#[test]
fn parse_external_skill_invoke_context_rejects_truncated_payload() {
    let payload = json!({
        "skill_id": "demo-skill",
        "display_name": "Demo Skill",
        "instructions": "Follow the managed skill instruction before answering.",
    });
    let line = format!(
        "[ok] {}",
        json!({
            "status": "ok",
            "tool": "read",
            "tool_call_id": "call-1",
            "payload_semantics": "skill_context",
            "payload_summary": serde_json::to_string(&payload).expect("encode payload"),
            "payload_chars": 512,
            "payload_truncated": true
        })
    );

    assert!(
        parse_skill_context(line.as_str()).is_none(),
        "truncated external skill payload should not activate managed skill context"
    );
}

#[test]
fn reduce_followup_payload_for_model_compacts_tool_search_summary() {
    let payload_summary = json!({
            "adapter": "core-tools",
            "tool_name": "tool.search",
            "query": "read repo file",
            "exact_tool_id": "file.read",
            "returned": 1,
            "diagnostics": {
                "reason": "exact_tool_id_not_visible",
                "requested_tool_id": "file.read"
            },
            "results": [
                {
                    "tool_id": "file.read",
                    "summary": "Read a UTF-8 text file from the configured workspace root and return contents.",
                    "argument_hint": "path:string",
                    "required_fields": ["path"],
                    "required_field_groups": [["path"]],
                    "tags": ["core", "file", "read"],
                    "why": ["summary matches query"],
                    "lease": "lease-file"
                }
            ]
        })
        .to_string();
    let tool_result = format!(
        "[ok] {}",
        json!({
            "status": "ok",
            "tool": "tool.search",
            "tool_call_id": "call-search",
            "payload_summary": payload_summary,
            "payload_chars": 512,
            "payload_truncated": false
        })
    );

    let reduced = reduce_followup_payload_for_model("tool_result", tool_result.as_str());
    let envelope: Value = serde_json::from_str(
        reduced
            .strip_prefix("[ok] ")
            .expect("tool result should keep status prefix"),
    )
    .expect("reduced envelope should stay valid json");
    let summary: Value = serde_json::from_str(
        envelope["payload_summary"]
            .as_str()
            .expect("payload summary should stay encoded json"),
    )
    .expect("reduced payload summary should stay valid json");
    let first = summary["results"]
        .as_array()
        .and_then(|results| results.first())
        .expect("reduced payload should keep the first result");

    assert_eq!(summary["query"], "read repo file");
    assert_eq!(summary["exact_tool_id"], "read");
    assert_eq!(
        summary["diagnostics"]["reason"],
        "exact_tool_id_not_visible"
    );
    assert_eq!(summary["diagnostics"]["requested_tool_id"], "read");
    assert_eq!(summary["returned"], 1);
    assert_eq!(first["tool_id"], "read");
    assert_eq!(first["lease"], "lease-file");
    assert_eq!(summary["adapter"], Value::Null);
    assert_eq!(summary["tool_name"], Value::Null);
    assert!(first.get("tags").is_none());
    assert!(first.get("why").is_none());
}

#[test]
fn reduce_followup_payload_for_model_preserves_empty_required_arrays() {
    let payload_summary = json!({
        "query": "install a skill",
        "results": [
            {
                "tool_id": "skills.install",
                "summary": "Install a bundled skill or a local skill path.",
                "argument_hint": "bundled_skill_id?:string,path?:string",
                "required_fields": [],
                "required_field_groups": [],
                "lease": "lease-install"
            }
        ]
    })
    .to_string();
    let tool_result = format!(
        "[ok] {}",
        json!({
            "status": "ok",
            "tool": "tool.search",
            "tool_call_id": "call-search",
            "payload_summary": payload_summary,
            "payload_chars": 512,
            "payload_truncated": false
        })
    );

    let reduced = reduce_followup_payload_for_model("tool_result", tool_result.as_str());
    let envelope: Value = serde_json::from_str(
        reduced
            .strip_prefix("[ok] ")
            .expect("tool result should keep status prefix"),
    )
    .expect("reduced envelope should stay valid json");
    let summary: Value = serde_json::from_str(
        envelope["payload_summary"]
            .as_str()
            .expect("payload summary should stay encoded json"),
    )
    .expect("reduced payload summary should stay valid json");
    let first = summary["results"]
        .as_array()
        .and_then(|results| results.first())
        .expect("reduced payload should keep the first result");

    assert_eq!(first["required_fields"], json!([]));
    assert_eq!(first["required_field_groups"], json!([]));
}

#[test]
fn reduce_followup_payload_for_model_borrows_unmodified_tool_results() {
    let tool_result = r#"[ok] {"status":"ok","tool":"discovery","tool_call_id":"call-search","payload_summary":"{\"query\":\"status\"}","payload_chars":32,"payload_truncated":true}"#;

    let reduced = reduce_followup_payload_for_model("tool_result", tool_result);

    assert_eq!(reduced.as_ref(), tool_result);
    assert_eq!(reduced.as_ptr(), tool_result.as_ptr());
}

#[test]
fn summarize_failed_provider_lane_tool_request_preserves_multi_intent_context_without_trace() {
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![
            ToolIntent {
                tool_name: "file.read".to_owned(),
                args_json: json!({"path": "Cargo.toml"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-a".to_owned(),
                turn_id: "turn-a".to_owned(),
                tool_call_id: "call-1".to_owned(),
            },
            ToolIntent {
                tool_name: "shell.exec".to_owned(),
                args_json: json!({"command": "ls /root"}),
                source: "provider_tool_call".to_owned(),
                session_id: "session-a".to_owned(),
                turn_id: "turn-a".to_owned(),
                tool_call_id: "call-2".to_owned(),
            },
        ],
        raw_meta: Value::Null,
    };

    let request_summary = summarize_failed_provider_lane_tool_request(&turn, None)
        .expect("multi-intent failures should retain a request summary");
    let request_summary_json: Value =
        serde_json::from_str(&request_summary).expect("request summary should be valid json");
    let request_entries = request_summary_json
        .as_array()
        .expect("multi-intent request summary should be an array");

    assert_eq!(request_entries.len(), 2);
    assert_eq!(request_entries[0]["name"], "read");
    assert_eq!(request_entries[1]["name"], "bash");
    assert_eq!(request_entries[1]["arguments"]["command"], "ls");
    assert_eq!(request_entries[1]["arguments"]["args_redacted"], 1);
}

#[test]
fn summarize_single_tool_followup_request_resolves_grouped_hidden_invoke_to_precise_operation() {
    let intent = ToolIntent {
        tool_name: "tool.invoke".to_owned(),
        args_json: json!({
            "tool_id": "agent",
            "lease": "lease-agent",
            "arguments": {
                "operation": "delegate-background",
                "task": "summarize the repo"
            }
        }),
        source: "provider_tool_call".to_owned(),
        session_id: "session-a".to_owned(),
        turn_id: "turn-a".to_owned(),
        tool_call_id: "call-agent".to_owned(),
    };

    let request_summary = summarize_single_tool_followup_request(&intent)
        .expect("grouped hidden invoke should retain a request summary");
    let request_summary_json: Value =
        serde_json::from_str(&request_summary).expect("request summary should be valid json");

    assert_eq!(request_summary_json["name"], "delegate_async");
    assert_eq!(
        request_summary_json["arguments"]["task"],
        "summarize the repo"
    );
    assert!(request_summary_json["arguments"].get("operation").is_none());
}

#[test]
fn strip_think_tags_removes_think_content() {
    let input = "<think>Let me think about this...\nThe user wants to know the weather.\nI should check the forecast.</think>The weather today is sunny.";
    let expected = "The weather today is sunny.";
    assert_eq!(strip_think_tags(input), expected);
}

#[test]
fn strip_think_tags_handles_empty_tags() {
    let input = "Hello <think></think>world";
    assert_eq!(strip_think_tags(input), "Hello world");
}

#[test]
fn strip_think_tags_handles_multiple_tags() {
    let input = "<think>First thought</think>Middle<think>Second thought</think>End";
    assert_eq!(strip_think_tags(input), "MiddleEnd");
}

#[test]
fn strip_think_tags_handles_nested_content() {
    let input = "<think>Think content with <tag> inside</think>Real response";
    assert_eq!(strip_think_tags(input), "Real response");
}

#[test]
fn strip_think_tags_handles_nested_think_tags() {
    let input = "<think>outer<think>inner</think>visible</think>done";
    assert_eq!(strip_think_tags(input), "done");
}

#[test]
fn strip_think_tags_case_insensitive() {
    let input = "<ThInK>think content</tHiNk>Result";
    assert_eq!(strip_think_tags(input), "Result");
}

#[test]
fn strip_think_tags_drops_unterminated_opening_tag() {
    let input = "Answer<think>internal reasoning";
    assert_eq!(strip_think_tags(input), "Answer");
}

#[test]
fn strip_think_tags_drops_stray_closing_tag() {
    let input = "Answer</think>";
    assert_eq!(strip_think_tags(input), "Answer");
}
