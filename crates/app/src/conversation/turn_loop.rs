use serde_json::{json, Value};

use crate::CliResult;
use crate::KernelContext;

use super::super::config::LoongClawConfig;
use super::persistence::{format_provider_error_reply, persist_error_turns, persist_success_turns};
use super::runtime::{ConversationRuntime, DefaultConversationRuntime};
use super::turn_engine::{ProviderTurn, ToolIntent, TurnEngine, TurnResult};
use super::ProviderErrorMode;

#[derive(Default)]
pub struct ConversationTurnLoop;

const TOOL_FOLLOWUP_PROMPT: &str = "Use the tool result above to answer the original user request in natural language. Do not include raw JSON, payload wrappers, or status markers unless the user explicitly asks for raw output.";
const REPEATED_TOOL_CALL_GUARD_PROMPT: &str = "Detected repeated identical tool calls without progress. Stop requesting the same tool again and provide the best possible natural-language answer from available context. If context is insufficient, state what is missing.";

impl ConversationTurnLoop {
    pub fn new() -> Self {
        Self
    }

    pub async fn handle_turn(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        let runtime = DefaultConversationRuntime;
        self.handle_turn_with_runtime(
            config, session_id, user_input, error_mode, &runtime, kernel_ctx,
        )
        .await
    }

    pub async fn handle_turn_with_runtime<R: ConversationRuntime + ?Sized>(
        &self,
        config: &LoongClawConfig,
        session_id: &str,
        user_input: &str,
        error_mode: ProviderErrorMode,
        runtime: &R,
        kernel_ctx: Option<&KernelContext>,
    ) -> CliResult<String> {
        let mut messages = runtime.build_messages(config, session_id, true, kernel_ctx)?;
        messages.push(json!({
            "role": "user",
            "content": user_input,
        }));
        let raw_tool_output_requested = user_requested_raw_tool_output(user_input);
        let mut last_raw_reply = String::new();
        let policy = TurnLoopPolicy::from_config(config);
        let mut last_tool_signature: Option<String> = None;
        let mut repeated_tool_signature_rounds = 0usize;

        for round_index in 0..policy.max_rounds {
            let turn = match runtime.request_turn(config, &messages).await {
                Ok(turn) => turn,
                Err(error) => {
                    return match error_mode {
                        ProviderErrorMode::Propagate => Err(error),
                        ProviderErrorMode::InlineMessage => {
                            let synthetic = format_provider_error_reply(&error);
                            persist_error_turns(
                                runtime, session_id, user_input, &synthetic, kernel_ctx,
                            )
                            .await?;
                            Ok(synthetic)
                        }
                    };
                }
            };

            let had_tool_intents = !turn.tool_intents.is_empty();
            if had_tool_intents {
                let current_tool_signature = tool_intent_signature_for_turn(&turn);
                if last_tool_signature.as_deref() == Some(current_tool_signature.as_str()) {
                    repeated_tool_signature_rounds += 1;
                } else {
                    repeated_tool_signature_rounds = 1;
                    last_tool_signature = Some(current_tool_signature);
                }
                // Guard against model loops repeatedly requesting the same tool payload.
                if repeated_tool_signature_rounds > policy.max_repeated_tool_call_rounds {
                    let raw_reply = if last_raw_reply.trim().is_empty() {
                        join_non_empty_lines(&[
                            turn.assistant_text.as_str(),
                            "repeated_tool_call_loop_guard_triggered",
                        ])
                    } else {
                        last_raw_reply.clone()
                    };
                    let reply = if raw_tool_output_requested {
                        raw_reply
                    } else {
                        append_repeated_tool_guard_followup_messages(
                            &mut messages,
                            turn.assistant_text.as_str(),
                            user_input,
                        );
                        request_completion_with_raw_fallback(
                            runtime,
                            config,
                            &messages,
                            raw_reply.as_str(),
                        )
                        .await
                    };
                    persist_success_turns(runtime, session_id, user_input, &reply, kernel_ctx)
                        .await?;
                    return Ok(reply);
                }
            } else {
                last_tool_signature = None;
                repeated_tool_signature_rounds = 0;
            }

            let turn_result = TurnEngine::new(policy.max_tool_steps_per_round)
                .execute_turn(&turn, kernel_ctx)
                .await;

            let reply = match turn_result {
                TurnResult::FinalText(tool_text) if had_tool_intents => {
                    let raw_reply =
                        join_non_empty_lines(&[turn.assistant_text.as_str(), tool_text.as_str()]);
                    last_raw_reply = raw_reply.clone();
                    if raw_tool_output_requested {
                        raw_reply
                    } else {
                        append_tool_followup_messages(
                            &mut messages,
                            turn.assistant_text.as_str(),
                            tool_text.as_str(),
                            user_input,
                        );
                        if round_index + 1 < policy.max_rounds {
                            continue;
                        }
                        request_completion_with_raw_fallback(
                            runtime,
                            config,
                            &messages,
                            raw_reply.as_str(),
                        )
                        .await
                    }
                }
                TurnResult::ToolDenied(reason) if had_tool_intents => {
                    let raw_reply = compose_assistant_reply(
                        turn.assistant_text.as_str(),
                        had_tool_intents,
                        TurnResult::ToolDenied(reason.clone()),
                    );
                    last_raw_reply = raw_reply.clone();
                    if raw_tool_output_requested {
                        raw_reply
                    } else {
                        append_tool_failure_followup_messages(
                            &mut messages,
                            turn.assistant_text.as_str(),
                            reason.as_str(),
                            user_input,
                        );
                        if round_index + 1 < policy.max_rounds {
                            continue;
                        }
                        request_completion_with_raw_fallback(
                            runtime,
                            config,
                            &messages,
                            raw_reply.as_str(),
                        )
                        .await
                    }
                }
                TurnResult::ToolError(reason) if had_tool_intents => {
                    let raw_reply = compose_assistant_reply(
                        turn.assistant_text.as_str(),
                        had_tool_intents,
                        TurnResult::ToolError(reason.clone()),
                    );
                    last_raw_reply = raw_reply.clone();
                    if raw_tool_output_requested {
                        raw_reply
                    } else {
                        append_tool_failure_followup_messages(
                            &mut messages,
                            turn.assistant_text.as_str(),
                            reason.as_str(),
                            user_input,
                        );
                        if round_index + 1 < policy.max_rounds {
                            continue;
                        }
                        request_completion_with_raw_fallback(
                            runtime,
                            config,
                            &messages,
                            raw_reply.as_str(),
                        )
                        .await
                    }
                }
                other => {
                    compose_assistant_reply(turn.assistant_text.as_str(), had_tool_intents, other)
                }
            };
            persist_success_turns(runtime, session_id, user_input, &reply, kernel_ctx).await?;
            return Ok(reply);
        }

        let reply = if last_raw_reply.is_empty() {
            "agent_loop_round_limit_reached".to_owned()
        } else {
            last_raw_reply
        };
        persist_success_turns(runtime, session_id, user_input, &reply, kernel_ctx).await?;
        Ok(reply)
    }
}

fn append_tool_followup_messages(
    messages: &mut Vec<Value>,
    assistant_preface: &str,
    tool_result_text: &str,
    user_input: &str,
) {
    let preface = assistant_preface.trim();
    if !preface.is_empty() {
        messages.push(json!({
            "role": "assistant",
            "content": preface,
        }));
    }
    messages.push(json!({
        "role": "assistant",
        "content": format!("[tool_result]\n{tool_result_text}"),
    }));
    messages.push(json!({
        "role": "user",
        "content": format!("{TOOL_FOLLOWUP_PROMPT}\n\nOriginal request:\n{user_input}"),
    }));
}

fn append_tool_failure_followup_messages(
    messages: &mut Vec<Value>,
    assistant_preface: &str,
    tool_failure_reason: &str,
    user_input: &str,
) {
    let preface = assistant_preface.trim();
    if !preface.is_empty() {
        messages.push(json!({
            "role": "assistant",
            "content": preface,
        }));
    }
    messages.push(json!({
        "role": "assistant",
        "content": format!("[tool_failure]\n{tool_failure_reason}"),
    }));
    messages.push(json!({
        "role": "user",
        "content": format!("{TOOL_FOLLOWUP_PROMPT}\n\nOriginal request:\n{user_input}"),
    }));
}

fn append_repeated_tool_guard_followup_messages(
    messages: &mut Vec<Value>,
    assistant_preface: &str,
    user_input: &str,
) {
    let preface = assistant_preface.trim();
    if !preface.is_empty() {
        messages.push(json!({
            "role": "assistant",
            "content": preface,
        }));
    }
    messages.push(json!({
        "role": "assistant",
        "content": "[tool_loop_guard]\nrepeated_tool_call_signature",
    }));
    messages.push(json!({
        "role": "user",
        "content": format!("{REPEATED_TOOL_CALL_GUARD_PROMPT}\n\nOriginal request:\n{user_input}"),
    }));
}

async fn request_completion_with_raw_fallback<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    config: &LoongClawConfig,
    messages: &[Value],
    raw_reply: &str,
) -> String {
    match runtime.request_completion(config, messages).await {
        Ok(final_reply) => {
            let trimmed = final_reply.trim();
            if trimmed.is_empty() {
                raw_reply.to_owned()
            } else {
                trimmed.to_owned()
            }
        }
        Err(_) => raw_reply.to_owned(),
    }
}

fn user_requested_raw_tool_output(user_input: &str) -> bool {
    let normalized = user_input.to_ascii_lowercase();
    [
        "raw",
        "json",
        "payload",
        "verbatim",
        "exact output",
        "full output",
        "tool output",
        "[ok]",
    ]
    .iter()
    .any(|signal| normalized.contains(signal))
}

fn tool_intent_signature_for_turn(turn: &ProviderTurn) -> String {
    tool_intent_signature(&turn.tool_intents)
}

fn tool_intent_signature(intents: &[ToolIntent]) -> String {
    intents
        .iter()
        .map(|intent| {
            let args = serde_json::to_string(&intent.args_json)
                .unwrap_or_else(|_| "<invalid_tool_args_json>".to_owned());
            format!("{}:{args}", intent.tool_name.trim())
        })
        .collect::<Vec<_>>()
        .join("||")
}

#[derive(Debug, Clone, Copy)]
struct TurnLoopPolicy {
    max_rounds: usize,
    max_tool_steps_per_round: usize,
    max_repeated_tool_call_rounds: usize,
}

impl TurnLoopPolicy {
    fn from_config(config: &LoongClawConfig) -> Self {
        let turn_loop = &config.conversation.turn_loop;
        Self {
            max_rounds: turn_loop.max_rounds.max(1),
            max_tool_steps_per_round: turn_loop.max_tool_steps_per_round.max(1),
            max_repeated_tool_call_rounds: turn_loop.max_repeated_tool_call_rounds.max(1),
        }
    }
}

fn compose_assistant_reply(
    assistant_preface: &str,
    had_tool_intents: bool,
    turn_result: TurnResult,
) -> String {
    match turn_result {
        TurnResult::FinalText(text) => {
            if had_tool_intents {
                join_non_empty_lines(&[assistant_preface, text.as_str()])
            } else {
                text
            }
        }
        TurnResult::NeedsApproval(reason) => {
            let inline = format!("[tool_approval_required] {reason}");
            join_non_empty_lines(&[assistant_preface, inline.as_str()])
        }
        TurnResult::ToolDenied(reason) => join_non_empty_lines(&[assistant_preface, &reason]),
        TurnResult::ToolError(reason) => join_non_empty_lines(&[assistant_preface, &reason]),
        TurnResult::ProviderError(reason) => {
            let inline = format_provider_error_reply(&reason);
            join_non_empty_lines(&[assistant_preface, inline.as_str()])
        }
    }
}

fn join_non_empty_lines(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
