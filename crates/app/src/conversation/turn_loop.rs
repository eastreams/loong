use std::sync::Arc;

use crate::CliResult;
use crate::acp::{AcpTurnEventSink, JsonlAcpTurnEventSink};
use crate::session::store;

use super::super::config::LoongConfig;
use super::ProviderErrorMode;
use super::runtime::{ConversationRuntime, load_default_conversation_runtime};
use super::runtime_binding::ConversationRuntimeBinding;
use super::turn_budget::TurnRoundBudget;
use super::turn_engine::DefaultAppToolDispatcher;
use super::turn_loop_round::{
    apply_turn_loop_terminal_action, evaluate_round_kernel, initialize_turn_loop_session,
    resolve_round_kernel_terminal_action,
};
use super::turn_loop_state::{
    TurnLoopPolicy, TurnLoopTerminalAction, build_round_limit_terminal_action,
    decide_round_kernel_action,
};
use super::turn_observer::map_streaming_callback_data_to_token_event;
use super::turn_shared::{
    ProviderTurnRequestAction, ReplyPersistenceMode, decide_provider_turn_request_action,
    tool_loop_circuit_breaker_reply,
};

#[derive(Default)]
pub struct ConversationTurnLoop;

impl ConversationTurnLoop {
    pub fn new() -> Self {
        Self
    }

    pub async fn handle_turn(
        &self,
        config: &LoongConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let runtime = load_default_conversation_runtime(config)?;
        self.handle_turn_with_runtime(
            config, session_id, user_input, error_mode, &runtime, binding,
        )
        .await
    }

    pub async fn handle_turn_with_runtime<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        binding: ConversationRuntimeBinding<'_>,
    ) -> CliResult<String> {
        let policy = TurnLoopPolicy::from_config(config);
        let session_context = runtime.session_context(config, session_id, binding)?;
        let tool_view = session_context.tool_view.clone();
        let app_dispatcher = DefaultAppToolDispatcher::with_config(
            store::session_store_config_from_memory_config(&config.memory),
            config.clone(),
        );
        let turn_id = super::turn_shared::next_conversation_turn_id();
        let mut session = initialize_turn_loop_session(
            runtime
                .build_messages(config, session_id, true, &tool_view, binding)
                .await?,
            user_input,
            &policy,
        );

        for round_index in 0..policy.max_rounds {
            let use_streaming = crate::provider::supports_turn_streaming_events(config);
            let on_token: crate::provider::StreamingTokenCallback = if use_streaming {
                let sink = JsonlAcpTurnEventSink::stderr_with_prefix("");
                Some(Arc::new(
                    move |data: crate::provider::StreamingCallbackData| {
                        let event = map_streaming_callback_data_to_token_event(data);
                        let _ = sink.on_event(&serde_json::to_value(&event).unwrap_or_default());
                    },
                ))
            } else {
                None
            };
            let turn = match decide_provider_turn_request_action(
                if use_streaming {
                    runtime
                        .request_turn_streaming(
                            config,
                            session_id,
                            turn_id.as_str(),
                            &session.messages,
                            &tool_view,
                            binding,
                            on_token,
                        )
                        .await
                } else {
                    runtime
                        .request_turn(
                            config,
                            session_id,
                            turn_id.as_str(),
                            &session.messages,
                            &tool_view,
                            binding,
                        )
                        .await
                },
                error_mode,
            ) {
                ProviderTurnRequestAction::Continue { turn } => turn,
                ProviderTurnRequestAction::FinalizeInlineProviderError { reply } => {
                    return apply_turn_loop_terminal_action(
                        runtime,
                        session_id,
                        user_input,
                        TurnLoopTerminalAction::PersistReply {
                            reply,
                            persistence_mode: ReplyPersistenceMode::InlineProviderError,
                        },
                        binding,
                    )
                    .await;
                }
                ProviderTurnRequestAction::ReturnError { error } => {
                    return apply_turn_loop_terminal_action(
                        runtime,
                        session_id,
                        user_input,
                        TurnLoopTerminalAction::ReturnError { error },
                        binding,
                    )
                    .await;
                }
            };

            // Global circuit breaker: prospective check before dispatching tools.
            // Trips if adding this round's intents would exceed the per-turn limit,
            // ensuring the configured max remains inclusive for executed tool calls.
            let prospective_total = session
                .total_tool_calls
                .saturating_add(turn.tool_intents.len());
            if let Some(reply) =
                tool_loop_circuit_breaker_reply(prospective_total, policy.max_total_tool_calls)
            {
                return apply_turn_loop_terminal_action(
                    runtime,
                    session_id,
                    user_input,
                    TurnLoopTerminalAction::PersistReply {
                        reply,
                        persistence_mode: ReplyPersistenceMode::Success,
                    },
                    binding,
                )
                .await;
            }

            let evaluation = evaluate_round_kernel(
                config,
                &policy,
                &turn,
                &session_context,
                &app_dispatcher,
                binding,
                &mut session.loop_supervisor,
            )
            .await;

            session.total_tool_calls = prospective_total;

            let reply_phase = evaluation.reply_phase(session.raw_tool_output_requested);
            if let Some(raw_reply) = reply_phase.raw_reply() {
                session.last_raw_reply = raw_reply.to_owned();
            }
            let decision = decide_round_kernel_action(
                TurnRoundBudget::for_round_index(round_index, policy.max_rounds),
                evaluation,
                reply_phase,
            );

            if let Some(action) = resolve_round_kernel_terminal_action(
                runtime,
                config,
                &mut session,
                user_input,
                decision,
                binding,
            )
            .await?
            {
                return apply_turn_loop_terminal_action(
                    runtime, session_id, user_input, action, binding,
                )
                .await;
            }
        }

        apply_turn_loop_terminal_action(
            runtime,
            session_id,
            user_input,
            build_round_limit_terminal_action(session.last_raw_reply.as_str()),
            binding,
        )
        .await
    }
}

impl TurnLoopPolicy {
    fn from_config(config: &LoongConfig) -> Self {
        let turn_loop = &config.conversation.turn_loop;
        Self {
            max_rounds: super::TURN_LOOP_MAX_ROUNDS,
            max_tool_steps_per_round: super::FAST_LANE_MAX_TOOL_STEPS_PER_TURN,
            max_followup_tool_payload_chars: turn_loop.max_followup_tool_payload_chars.max(256),
            max_followup_tool_payload_chars_total: turn_loop
                .max_followup_tool_payload_chars_total
                .max(1),
            max_total_tool_calls: super::TURN_LOOP_MAX_TOTAL_TOOL_CALLS,
            max_consecutive_same_tool: super::TURN_LOOP_MAX_CONSECUTIVE_SAME_TOOL,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::turn_budget::TurnRoundBudgetDecision;
    use crate::conversation::turn_engine::{ProviderTurn, TurnFailure, TurnResult};
    use crate::conversation::turn_loop_followup::{
        FollowupPayloadBudget, append_repeated_tool_guard_followup_messages,
        append_tool_driven_followup_messages,
    };
    use crate::conversation::turn_loop_state::{
        RoundFollowup, RoundKernelDecision, RoundKernelEvaluation, ToolLoopSupervisor,
        ToolLoopSupervisorVerdict,
    };
    use crate::conversation::turn_shared::{
        ToolDrivenFollowupLabel, ToolDrivenFollowupPayload, ToolDrivenFollowupTextRef,
    };
    use serde_json::{Value, json};

    fn build_large_file_read_tool_result() -> String {
        let content = (0..96)
            .map(|index| format!("line {index}: {}", "x".repeat(48)))
            .collect::<Vec<_>>()
            .join("\n");
        let payload_summary = json!({
            "adapter": "core-tools",
            "tool_name": "file.read",
            "path": "/repo/README.md",
            "bytes": 8_192,
            "truncated": false,
            "content": content,
        })
        .to_string();
        format!(
            "[ok] {}",
            json!({
                "status": "ok",
                "tool": "file.read",
                "tool_call_id": "call-file",
                "payload_summary": payload_summary,
                "payload_chars": 8_192,
                "payload_truncated": false
            })
        )
    }

    fn assert_reduced_file_read_followup_message(messages: &[Value]) {
        let assistant_tool_result = messages
            .iter()
            .find(|message| {
                message.get("role") == Some(&Value::String("assistant".to_owned()))
                    && message
                        .get("content")
                        .and_then(Value::as_str)
                        .is_some_and(|content| content.starts_with("[tool_result]\n[ok] "))
            })
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("assistant tool_result followup message should exist");
        let line = assistant_tool_result
            .lines()
            .nth(1)
            .expect("assistant tool_result should keep payload line");
        let envelope: Value = serde_json::from_str(
            line.strip_prefix("[ok] ")
                .expect("tool result line should preserve status prefix"),
        )
        .expect("reduced followup envelope should stay valid json");
        let summary: Value = serde_json::from_str(
            envelope["payload_summary"]
                .as_str()
                .expect("payload summary should stay encoded json"),
        )
        .expect("file.read payload summary should stay valid json");

        assert_eq!(envelope["tool"], "read");
        assert_eq!(envelope["payload_truncated"], true);
        assert_eq!(summary["path"], "/repo/README.md");
        assert_eq!(summary["bytes"], 8_192);
        assert_eq!(summary["truncated"], false);
        assert!(summary.get("content_preview").is_some());
        assert!(summary.get("content_chars").is_some());
        assert_eq!(summary["content_truncated"], true);
    }

    #[test]
    fn append_tool_driven_followup_messages_adds_truncation_hint_to_user_prompt() {
        let mut messages = Vec::new();
        let mut budget = FollowupPayloadBudget::new(8_000, 20_000);

        append_tool_driven_followup_messages(
            &mut messages,
            "preface",
            &ToolDrivenFollowupPayload::ToolResult {
                text: r#"[ok] {"payload_truncated":true,"payload_summary":"..."}"#.to_owned(),
            },
            "summarize note.md",
            &mut budget,
            None,
            None,
        );

        let user_prompt = messages
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");
        assert!(
            user_prompt.contains(crate::conversation::turn_shared::TOOL_TRUNCATION_HINT_PROMPT)
        );
    }

    #[test]
    fn append_tool_driven_followup_messages_omits_truncation_hint_in_user_prompt() {
        let mut messages = Vec::new();
        let mut budget = FollowupPayloadBudget::new(8_000, 20_000);

        append_tool_driven_followup_messages(
            &mut messages,
            "preface",
            &ToolDrivenFollowupPayload::ToolFailure {
                reason: "tool_timeout ...(truncated 200 chars)".to_owned(),
                retryable: false,
            },
            "summarize note.md",
            &mut budget,
            None,
            None,
        );

        let user_prompt = messages
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");
        assert!(
            !user_prompt.contains(crate::conversation::turn_shared::TOOL_TRUNCATION_HINT_PROMPT)
        );
    }

    #[test]
    fn append_tool_driven_followup_messages_includes_request_summary_guidance() {
        let mut messages = Vec::new();
        let mut budget = FollowupPayloadBudget::new(8_000, 20_000);
        let tool_request_summary = json!({
            "tool": "shell.exec",
            "request": {
                "command": r#"C:\Windows\System32\CMD.EXE"#
            }
        })
        .to_string();

        append_tool_driven_followup_messages(
            &mut messages,
            "preface",
            &ToolDrivenFollowupPayload::ToolFailure {
                reason: "tool_preflight_denied: tool input needs repair".to_owned(),
                retryable: false,
            },
            "retry the command",
            &mut budget,
            None,
            Some(tool_request_summary.as_str()),
        );

        let user_prompt = messages
            .last()
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("user followup prompt should exist");

        assert!(user_prompt.contains("Repair guidance for exec:"));
        assert!(user_prompt.contains("CMD.EXE"));
        assert!(user_prompt.contains("cmd.exe"));
    }

    #[test]
    fn append_tool_driven_followup_messages_promotes_external_skill_invoke_into_system_context() {
        let mut messages = Vec::new();
        let mut budget = FollowupPayloadBudget::new(64, 64);

        append_tool_driven_followup_messages(
            &mut messages,
            "preface",
            &ToolDrivenFollowupPayload::ToolResult {
                text: r#"[ok] {"status":"ok","tool":"external_skills.invoke","tool_call_id":"call-1","payload_summary":"{\"skill_id\":\"demo-skill\",\"display_name\":\"Demo Skill\",\"instructions\":\"Follow the managed skill instruction before answering.\"}","payload_chars":180,"payload_truncated":false}"#.to_owned(),
            },
            "summarize note.md",
            &mut budget,
            None,
            None,
        );

        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[1]["role"], "system");
        let system_content = messages[1]["content"]
            .as_str()
            .expect("system content should exist");
        assert!(system_content.contains("Demo Skill"));
        assert!(system_content.contains("Follow the managed skill instruction before answering."));
        assert!(
            !system_content.contains("[tool_result_truncated]"),
            "invoke instructions should not be funneled through followup truncation markers"
        );

        let user_prompt = messages[2]["content"]
            .as_str()
            .expect("user prompt should exist");
        assert!(user_prompt.contains("external skill"));
        assert!(user_prompt.contains("Original request:\nsummarize note.md"));
    }

    #[test]
    fn append_tool_driven_followup_messages_keeps_large_external_skill_instructions_intact() {
        let mut messages = Vec::new();
        let mut budget = FollowupPayloadBudget::new(32, 32);
        let instructions = format!("prefix {}\nsuffix-marker", "x".repeat(512));
        let payload_summary = serde_json::json!({
            "skill_id": "demo-skill",
            "display_name": "Demo Skill",
            "instructions": instructions,
        })
        .to_string();
        let tool_result = format!(
            "[ok] {}",
            serde_json::json!({
                "status": "ok",
                "tool": "external_skills.invoke",
                "tool_call_id": "call-2",
                "payload_summary": payload_summary,
                "payload_chars": 2048,
                "payload_truncated": false
            })
        );

        append_tool_driven_followup_messages(
            &mut messages,
            "",
            &ToolDrivenFollowupPayload::ToolResult { text: tool_result },
            "apply the skill",
            &mut budget,
            None,
            None,
        );

        let system_content = messages[0]["content"]
            .as_str()
            .expect("system content should exist");
        assert!(
            system_content.contains("suffix-marker"),
            "system context should preserve the tail of large invoke instructions"
        );
    }

    #[test]
    fn append_tool_driven_followup_messages_reduces_file_read_payload_summary() {
        let mut messages = Vec::new();
        let mut budget = FollowupPayloadBudget::new(8_000, 20_000);
        let tool_result = build_large_file_read_tool_result();

        append_tool_driven_followup_messages(
            &mut messages,
            "preface",
            &ToolDrivenFollowupPayload::ToolResult { text: tool_result },
            "summarize README.md",
            &mut budget,
            None,
            None,
        );

        assert_reduced_file_read_followup_message(&messages);
    }

    fn build_large_shell_exec_tool_result() -> String {
        format!(
            "[ok] {}",
            serde_json::json!({
                "status": "ok",
                "tool": "shell.exec",
                "tool_call_id": "call-shell",
                "payload_summary": serde_json::json!({
                    "adapter": "core-tools",
                    "tool_name": "shell.exec",
                    "command": "cargo",
                    "args": ["test", "--workspace"],
                    "cwd": "/repo",
                    "exit_code": 0,
                    "stdout": (0..80)
                        .map(|index| format!("stdout line {index}: {}", "x".repeat(40)))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    "stderr": (0..48)
                        .map(|index| format!("stderr line {index}: {}", "e".repeat(32)))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .to_string(),
                "payload_chars": 8_192,
                "payload_truncated": false
            })
        )
    }

    #[test]
    fn append_tool_driven_followup_messages_reduces_shell_exec_payload_summary() {
        let mut messages = Vec::new();
        let mut budget = FollowupPayloadBudget::new(8_000, 20_000);
        let tool_result = build_large_shell_exec_tool_result();

        append_tool_driven_followup_messages(
            &mut messages,
            "preface",
            &ToolDrivenFollowupPayload::ToolResult { text: tool_result },
            "summarize the test run",
            &mut budget,
            None,
            None,
        );

        let (envelope, summary) =
            crate::conversation::turn_shared::parse_tool_result_followup_for_test(&messages);

        assert_eq!(envelope["tool"], "exec");
        assert_eq!(envelope["payload_truncated"], true);
        assert_eq!(summary["command"], "cargo");
        assert_eq!(summary["exit_code"], 0);
        assert!(summary.get("stdout_preview").is_some());
        assert!(summary.get("stdout_chars").is_some());
        assert_eq!(summary["stdout_truncated"], true);
        assert!(summary.get("stderr_preview").is_some());
        assert!(summary.get("stderr_chars").is_some());
        assert_eq!(summary["stderr_truncated"], true);
        assert!(
            summary["stdout_preview"]
                .as_str()
                .expect("stdout preview should exist")
                .contains("stdout line 0"),
            "expected compact stdout preview, got: {summary:?}"
        );
        assert!(
            summary["stderr_preview"]
                .as_str()
                .expect("stderr preview should exist")
                .contains("stderr line 0"),
            "expected compact stderr preview, got: {summary:?}"
        );
    }

    #[test]
    fn append_tool_driven_followup_messages_compacts_tool_search_payload_summary() {
        let mut messages = Vec::new();
        let mut budget = FollowupPayloadBudget::new(8_000, 20_000);
        let payload_summary = serde_json::json!({
            "adapter": "core-tools",
            "tool_name": "tool.search",
            "query": "read repo file",
            "exact_tool_id": "file.read",
            "returned": 2,
            "diagnostics": {
                "reason": "exact_tool_id_not_visible",
                "requested_tool_id": "file.read"
            },
            "results": [
                {
                    "tool_id": "file.read",
                    "summary": "Read a UTF-8 text file from the configured workspace root and return contents.",
                    "argument_hint": "path:string,offset?:integer,limit?:integer",
                    "required_fields": ["path"],
                    "required_field_groups": [["path"]],
                    "tags": ["core", "file", "read"],
                    "why": ["summary matches query", "tag matches read"],
                    "lease": "lease-file"
                },
                {
                    "tool_id": "shell.exec",
                    "summary": "Execute a shell command in the workspace.",
                    "argument_hint": "command:string,args?:string[]",
                    "required_fields": ["command"],
                    "required_field_groups": [["command"]],
                    "tags": ["core", "shell", "exec"],
                    "why": ["summary matches query", "tag matches exec"],
                    "lease": "lease-shell"
                }
            ]
        })
        .to_string();
        let tool_result = format!(
            "[ok] {}",
            serde_json::json!({
                "status": "ok",
                "tool": "tool.search",
                "tool_call_id": "call-search",
                "payload_summary": payload_summary,
                "payload_chars": 2_048,
                "payload_truncated": false
            })
        );

        append_tool_driven_followup_messages(
            &mut messages,
            "preface",
            &ToolDrivenFollowupPayload::ToolResult { text: tool_result },
            "find the right tool",
            &mut budget,
            None,
            None,
        );

        let assistant_tool_result = messages
            .iter()
            .find(|message| {
                message.get("role") == Some(&Value::String("assistant".to_owned()))
                    && message
                        .get("content")
                        .and_then(Value::as_str)
                        .is_some_and(|content| content.starts_with("[tool_result]\n[ok] "))
            })
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .expect("assistant tool_result followup message should exist");
        let line = assistant_tool_result
            .lines()
            .nth(1)
            .expect("assistant tool_result should keep payload line");
        let envelope: Value = serde_json::from_str(
            line.strip_prefix("[ok] ")
                .expect("tool result line should preserve status prefix"),
        )
        .expect("compacted followup envelope should stay valid json");
        let summary: Value = serde_json::from_str(
            envelope["payload_summary"]
                .as_str()
                .expect("payload summary should stay encoded json"),
        )
        .expect("compacted payload summary should stay json");
        let first = summary["results"]
            .as_array()
            .and_then(|results| results.first())
            .expect("compacted results should contain the first entry");

        assert_eq!(envelope["tool"], "tool.search");
        assert_eq!(envelope["payload_truncated"], false);
        assert_eq!(summary["query"], "read repo file");
        assert_eq!(summary["exact_tool_id"], "file.read");
        assert_eq!(
            summary["diagnostics"]["reason"],
            "exact_tool_id_not_visible"
        );
        assert!(summary.get("adapter").is_none());
        assert!(summary.get("tool_name").is_none());
        assert_eq!(summary["returned"], 2);
        assert_eq!(first["tool_id"], "file.read");
        assert_eq!(first["lease"], "lease-file");
        for entry in summary["results"]
            .as_array()
            .expect("results should be an array")
        {
            assert!(entry.get("tool_id").and_then(Value::as_str).is_some());
            assert!(entry.get("summary").and_then(Value::as_str).is_some());
            assert!(entry.get("argument_hint").and_then(Value::as_str).is_some());
            assert!(
                entry
                    .get("required_fields")
                    .and_then(Value::as_array)
                    .is_some()
            );
            assert!(
                entry
                    .get("required_field_groups")
                    .and_then(Value::as_array)
                    .is_some()
            );
            assert!(entry.get("lease").and_then(Value::as_str).is_some());
            assert!(entry.get("tags").is_none());
            assert!(entry.get("why").is_none());
        }
    }

    #[test]
    fn append_repeated_tool_guard_followup_messages_reduces_file_read_payload_summary() {
        let mut messages = Vec::new();
        let mut budget = FollowupPayloadBudget::new(8_000, 20_000);
        let tool_result = build_large_file_read_tool_result();

        append_repeated_tool_guard_followup_messages(
            &mut messages,
            "preface",
            "stop",
            "summarize README.md",
            Some(ToolDrivenFollowupTextRef::new(
                ToolDrivenFollowupLabel::ToolResult,
                tool_result.as_str(),
            )),
            &mut budget,
        );

        assert_reduced_file_read_followup_message(&messages);
    }

    #[test]
    fn append_repeated_tool_guard_followup_messages_reduces_shell_exec_payload_summary() {
        let mut messages = Vec::new();
        let mut budget = FollowupPayloadBudget::new(8_000, 20_000);
        let tool_result = build_large_shell_exec_tool_result();

        append_repeated_tool_guard_followup_messages(
            &mut messages,
            "preface",
            "stop",
            "summarize the test run",
            Some(ToolDrivenFollowupTextRef::new(
                ToolDrivenFollowupLabel::ToolResult,
                tool_result.as_str(),
            )),
            &mut budget,
        );

        let (envelope, summary) =
            crate::conversation::turn_shared::parse_tool_result_followup_for_test(&messages);

        assert_eq!(envelope["tool"], "exec");
        assert_eq!(envelope["payload_truncated"], true);
        assert_eq!(summary["command"], "cargo");
        assert_eq!(summary["exit_code"], 0);
        assert!(summary.get("stdout_preview").is_some());
        assert!(summary.get("stdout_chars").is_some());
        assert_eq!(summary["stdout_truncated"], true);
        assert!(summary.get("stderr_preview").is_some());
        assert!(summary.get("stderr_chars").is_some());
        assert_eq!(summary["stderr_truncated"], true);
    }

    #[test]
    fn append_repeated_tool_guard_followup_messages_compacts_tool_search_payload_summary() {
        let mut messages = Vec::new();
        let mut budget = FollowupPayloadBudget::new(8_000, 20_000);
        let payload_summary = serde_json::json!({
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
                    "argument_hint": "path:string,offset?:integer,limit?:integer",
                    "required_fields": ["path"],
                    "required_field_groups": [["path"]],
                    "tags": ["core", "file", "read"],
                    "why": ["summary matches query", "tag matches read"],
                    "lease": "lease-file"
                }
            ]
        })
        .to_string();
        let tool_result = format!(
            "[ok] {}",
            serde_json::json!({
                "status": "ok",
                "tool": "tool.search",
                "tool_call_id": "call-search",
                "payload_summary": payload_summary,
                "payload_chars": 1_024,
                "payload_truncated": false
            })
        );

        append_repeated_tool_guard_followup_messages(
            &mut messages,
            "preface",
            "stop",
            "find the right tool",
            Some(ToolDrivenFollowupTextRef::new(
                ToolDrivenFollowupLabel::ToolResult,
                tool_result.as_str(),
            )),
            &mut budget,
        );

        let (envelope, summary) =
            crate::conversation::turn_shared::parse_tool_result_followup_for_test(&messages);
        let first = summary["results"]
            .as_array()
            .and_then(|results| results.first())
            .expect("compacted results should contain the first entry");

        assert_eq!(envelope["tool"], "tool.search");
        assert_eq!(envelope["payload_truncated"], false);
        assert_eq!(summary["query"], "read repo file");
        assert_eq!(summary["exact_tool_id"], "file.read");
        assert_eq!(
            summary["diagnostics"]["reason"],
            "exact_tool_id_not_visible"
        );
        assert!(summary.get("adapter").is_none());
        assert!(summary.get("tool_name").is_none());
        assert_eq!(summary["returned"], 1);
        assert_eq!(first["tool_id"], "file.read");
        assert_eq!(first["lease"], "lease-file");
        assert!(first.get("tags").is_none());
        assert!(first.get("why").is_none());
    }

    #[test]
    fn decide_round_kernel_action_continues_tool_result_with_warning_before_round_limit() {
        let evaluation = RoundKernelEvaluation {
            assistant_preface: "preface".to_owned(),
            had_tool_intents: true,
            tool_request_summary: None,
            turn_result: TurnResult::FinalText("tool output".to_owned()),
            loop_verdict: Some(ToolLoopSupervisorVerdict::InjectWarning {
                reason: "warning".to_owned(),
            }),
        };

        let reply_phase = evaluation.reply_phase(false);
        let decision = decide_round_kernel_action(
            TurnRoundBudget::for_round_index(0, 3),
            evaluation,
            reply_phase,
        );

        if let RoundKernelDecision::ContinueWithFollowup(RoundFollowup::Tool {
            assistant_preface,
            payload: ToolDrivenFollowupPayload::ToolResult { text },
            tool_request_summary,
            loop_warning_reason,
            ..
        }) = decision
        {
            assert_eq!(assistant_preface, "preface");
            assert_eq!(text, "tool output");
            assert!(tool_request_summary.is_none());
            assert_eq!(loop_warning_reason.as_deref(), Some("warning"));
        } else {
            panic!("unexpected decision: {decision:?}");
        }
    }

    #[test]
    fn decide_round_kernel_action_hard_stop_tool_result_uses_completion_pass() {
        let evaluation = RoundKernelEvaluation {
            assistant_preface: "preface".to_owned(),
            had_tool_intents: true,
            tool_request_summary: None,
            turn_result: TurnResult::FinalText("tool output".to_owned()),
            loop_verdict: Some(ToolLoopSupervisorVerdict::HardStop {
                reason: "stop".to_owned(),
            }),
        };

        let reply_phase = evaluation.reply_phase(false);
        let decision = decide_round_kernel_action(
            TurnRoundBudget::for_round_index(0, 3),
            evaluation,
            reply_phase,
        );

        if let RoundKernelDecision::FinalizeWithCompletionPass {
            raw_reply,
            followup:
                RoundFollowup::Guard {
                    assistant_preface,
                    reason,
                    latest_tool_payload: Some(ToolDrivenFollowupPayload::ToolResult { text }),
                },
        } = decision
        {
            assert_eq!(raw_reply, "preface\ntool output");
            assert_eq!(assistant_preface, "preface");
            assert_eq!(reason, "stop");
            assert_eq!(text, "tool output");
        } else {
            panic!("unexpected decision: {decision:?}");
        }
    }

    #[test]
    fn decide_round_kernel_action_hard_stop_tool_failure_uses_completion_pass() {
        let evaluation = RoundKernelEvaluation {
            assistant_preface: "preface".to_owned(),
            had_tool_intents: true,
            tool_request_summary: None,
            turn_result: TurnResult::ToolError(TurnFailure::retryable(
                "tool_failed",
                "tool failure",
            )),
            loop_verdict: Some(ToolLoopSupervisorVerdict::HardStop {
                reason: "stop".to_owned(),
            }),
        };

        let reply_phase = evaluation.reply_phase(false);
        let decision = decide_round_kernel_action(
            TurnRoundBudget::for_round_index(0, 3),
            evaluation,
            reply_phase,
        );

        if let RoundKernelDecision::FinalizeWithCompletionPass {
            raw_reply,
            followup:
                RoundFollowup::Guard {
                    assistant_preface,
                    reason,
                    latest_tool_payload:
                        Some(ToolDrivenFollowupPayload::ToolFailure {
                            reason: tool_reason,
                            ..
                        }),
                },
        } = decision
        {
            assert_eq!(raw_reply, "preface\ntool failure");
            assert_eq!(assistant_preface, "preface");
            assert_eq!(reason, "stop");
            assert_eq!(tool_reason, "tool failure");
        } else {
            panic!("unexpected decision: {decision:?}");
        }
    }

    #[test]
    fn decide_round_kernel_action_raw_mode_finalizes_tool_result_directly() {
        let evaluation = RoundKernelEvaluation {
            assistant_preface: "preface".to_owned(),
            had_tool_intents: true,
            tool_request_summary: None,
            turn_result: TurnResult::FinalText("tool output".to_owned()),
            loop_verdict: Some(ToolLoopSupervisorVerdict::InjectWarning {
                reason: "warning".to_owned(),
            }),
        };

        let reply_phase = evaluation.reply_phase(true);
        let decision = decide_round_kernel_action(
            TurnRoundBudget::for_round_index(0, 3),
            evaluation,
            reply_phase,
        );

        if let RoundKernelDecision::FinalizeDirect { reply } = decision {
            assert_eq!(reply, "preface\ntool output");
        } else {
            panic!("unexpected decision: {decision:?}");
        }
    }

    #[test]
    fn turn_round_budget_detects_followup_capacity() {
        let first_round = TurnRoundBudget::for_round_index(0, 3);
        let last_round = TurnRoundBudget::for_round_index(2, 3);

        assert_eq!(
            first_round.followup_decision(),
            TurnRoundBudgetDecision::ContinueWithFollowup
        );
        assert_eq!(
            last_round.followup_decision(),
            TurnRoundBudgetDecision::FinalizeWithCompletionPass
        );
    }

    #[test]
    fn decide_provider_turn_request_action_continues_successful_turns() {
        let action = decide_provider_turn_request_action(
            Ok(ProviderTurn {
                assistant_text: "preface".to_owned(),
                tool_intents: Vec::new(),
                raw_meta: Value::Null,
            }),
            ProviderErrorMode::Propagate,
        );

        match action {
            ProviderTurnRequestAction::Continue { turn } => {
                assert_eq!(turn.assistant_text, "preface");
                assert!(turn.tool_intents.is_empty());
            }
            ProviderTurnRequestAction::FinalizeInlineProviderError { reply } => {
                panic!("unexpected inline error action: {reply}");
            }
            ProviderTurnRequestAction::ReturnError { error } => {
                panic!("unexpected propagated error action: {error}");
            }
        }
    }

    #[test]
    fn decide_provider_turn_request_action_formats_inline_provider_errors() {
        let action = decide_provider_turn_request_action(
            Err("timeout".to_owned()),
            ProviderErrorMode::InlineMessage,
        );

        match action {
            ProviderTurnRequestAction::FinalizeInlineProviderError { reply } => {
                assert_eq!(reply, "[provider_error] timeout");
            }
            ProviderTurnRequestAction::Continue { turn } => {
                panic!("unexpected continue action: {turn:?}");
            }
            ProviderTurnRequestAction::ReturnError { error } => {
                panic!("unexpected propagated error action: {error}");
            }
        }
    }

    #[test]
    fn decide_provider_turn_request_action_propagates_provider_errors() {
        let action = decide_provider_turn_request_action(
            Err("timeout".to_owned()),
            ProviderErrorMode::Propagate,
        );

        match action {
            ProviderTurnRequestAction::ReturnError { error } => {
                assert_eq!(error, "timeout");
            }
            ProviderTurnRequestAction::Continue { turn } => {
                panic!("unexpected continue action: {turn:?}");
            }
            ProviderTurnRequestAction::FinalizeInlineProviderError { reply } => {
                panic!("unexpected inline error action: {reply}");
            }
        }
    }

    #[test]
    fn build_round_limit_terminal_action_prefers_last_raw_reply() {
        let action = build_round_limit_terminal_action("last raw reply");

        match action {
            TurnLoopTerminalAction::PersistReply {
                reply,
                persistence_mode,
            } => {
                assert_eq!(reply, "last raw reply");
                assert_eq!(persistence_mode, ReplyPersistenceMode::Success);
            }
            TurnLoopTerminalAction::ReturnError { error } => {
                panic!("unexpected propagated error terminal action: {error}");
            }
        }
    }

    #[test]
    fn build_round_limit_terminal_action_uses_synthetic_reply_when_raw_reply_missing() {
        let action = build_round_limit_terminal_action("");

        match action {
            TurnLoopTerminalAction::PersistReply {
                reply,
                persistence_mode,
            } => {
                assert_eq!(reply, "agent_loop_round_limit_reached");
                assert_eq!(persistence_mode, ReplyPersistenceMode::Success);
            }
            TurnLoopTerminalAction::ReturnError { error } => {
                panic!("unexpected propagated error terminal action: {error}");
            }
        }
    }

    fn test_policy_with_consecutive_limit(limit: usize) -> TurnLoopPolicy {
        TurnLoopPolicy {
            max_rounds: 100,
            max_tool_steps_per_round: 1,
            max_followup_tool_payload_chars: 8_000,
            max_followup_tool_payload_chars_total: 20_000,
            max_total_tool_calls: 200,
            max_consecutive_same_tool: limit,
        }
    }

    fn observe(
        supervisor: &mut ToolLoopSupervisor,
        policy: &TurnLoopPolicy,
        tool_name: &str,
    ) -> ToolLoopSupervisorVerdict {
        supervisor.observe_round(policy, tool_name)
    }

    #[test]
    fn consecutive_same_tool_injects_warning_at_threshold() {
        let policy = test_policy_with_consecutive_limit(3);
        let mut supervisor = ToolLoopSupervisor::default();

        // First two calls: below threshold
        assert!(matches!(
            observe(&mut supervisor, &policy, "shell.exec"),
            ToolLoopSupervisorVerdict::Continue
        ));
        assert!(matches!(
            observe(&mut supervisor, &policy, "shell.exec"),
            ToolLoopSupervisorVerdict::Continue
        ));
        // Third call: hits threshold (>= 3) -> InjectWarning
        assert!(matches!(
            observe(&mut supervisor, &policy, "shell.exec"),
            ToolLoopSupervisorVerdict::InjectWarning { .. }
        ));
    }

    #[test]
    fn consecutive_same_tool_hard_stops_on_repeat_warning() {
        let policy = test_policy_with_consecutive_limit(3);
        let mut supervisor = ToolLoopSupervisor::default();

        // Get to threshold
        observe(&mut supervisor, &policy, "shell.exec");
        observe(&mut supervisor, &policy, "shell.exec");
        observe(&mut supervisor, &policy, "shell.exec"); // InjectWarning
        // Same pattern again -> HardStop
        assert!(matches!(
            observe(&mut supervisor, &policy, "shell.exec"),
            ToolLoopSupervisorVerdict::HardStop { .. }
        ));
    }

    #[test]
    fn consecutive_same_tool_resets_on_tool_name_change() {
        let policy = test_policy_with_consecutive_limit(3);
        let mut supervisor = ToolLoopSupervisor::default();

        observe(&mut supervisor, &policy, "shell.exec");
        observe(&mut supervisor, &policy, "shell.exec");
        // Switch tool - resets consecutive counter
        assert!(matches!(
            observe(&mut supervisor, &policy, "file.read"),
            ToolLoopSupervisorVerdict::Continue
        ));
        // Back to shell.exec - should start fresh, not trigger warning
        assert!(matches!(
            observe(&mut supervisor, &policy, "shell.exec"),
            ToolLoopSupervisorVerdict::Continue
        ));
    }

    #[test]
    fn global_circuit_breaker_allows_reaching_limit_and_trips_only_above_it() {
        assert_eq!(tool_loop_circuit_breaker_reply(200, 200), None);
        assert_eq!(
            tool_loop_circuit_breaker_reply(201, 200).as_deref(),
            Some(
                "tool_loop_circuit_breaker: would exceed 201/200 tool calls this turn. Do you want to continue? Reply to resume."
            )
        );
        assert!(tool_loop_circuit_breaker_reply(usize::MAX, 200).is_some());
    }
}
