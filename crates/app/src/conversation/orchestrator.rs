use serde_json::{json, Value};

use crate::CliResult;
use crate::KernelContext;

use super::super::config::LoongClawConfig;
use super::persistence::{format_provider_error_reply, persist_error_turns, persist_success_turns};
use super::runtime::{ConversationRuntime, DefaultConversationRuntime};
use super::turn_engine::{TurnEngine, TurnResult};
use super::ProviderErrorMode;

#[derive(Default)]
pub struct ConversationOrchestrator;

const MAX_TOOL_STEPS_PER_TURN: usize = 1;
const TOOL_FOLLOWUP_PROMPT: &str = "Use the tool result above to answer the original user request in natural language. Do not include raw JSON, payload wrappers, or status markers unless the user explicitly asks for raw output.";

impl ConversationOrchestrator {
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

        let provider_result = runtime.request_turn(config, &messages).await;
        match provider_result {
            Ok(turn) => {
                let had_tool_intents = !turn.tool_intents.is_empty();
                let turn_result = TurnEngine::new(MAX_TOOL_STEPS_PER_TURN)
                    .execute_turn(&turn, kernel_ctx)
                    .await;
                let reply = match turn_result {
                    TurnResult::FinalText(tool_text) if had_tool_intents => {
                        let raw_reply = join_non_empty_lines(&[
                            turn.assistant_text.as_str(),
                            tool_text.as_str(),
                        ]);
                        if user_requested_raw_tool_output(user_input) {
                            raw_reply
                        } else {
                            let follow_up_messages = build_tool_followup_messages(
                                &messages,
                                turn.assistant_text.as_str(),
                                tool_text.as_str(),
                                user_input,
                            );
                            match runtime
                                .request_completion(config, &follow_up_messages)
                                .await
                            {
                                Ok(final_reply) => {
                                    let trimmed = final_reply.trim();
                                    if trimmed.is_empty() {
                                        raw_reply
                                    } else {
                                        trimmed.to_owned()
                                    }
                                }
                                Err(_) => raw_reply,
                            }
                        }
                    }
                    other => compose_assistant_reply(
                        turn.assistant_text.as_str(),
                        had_tool_intents,
                        other,
                    ),
                };
                persist_success_turns(runtime, session_id, user_input, &reply, kernel_ctx).await?;
                Ok(reply)
            }
            Err(error) => match error_mode {
                ProviderErrorMode::Propagate => Err(error),
                ProviderErrorMode::InlineMessage => {
                    let synthetic = format_provider_error_reply(&error);
                    persist_error_turns(runtime, session_id, user_input, &synthetic, kernel_ctx)
                        .await?;
                    Ok(synthetic)
                }
            },
        }
    }
}

fn build_tool_followup_messages(
    base_messages: &[Value],
    assistant_preface: &str,
    tool_result_text: &str,
    user_input: &str,
) -> Vec<Value> {
    let mut messages = base_messages.to_vec();
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
    messages
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
        TurnResult::ToolDenied(reason) => {
            let inline = format!("[tool_denied] {reason}");
            join_non_empty_lines(&[assistant_preface, inline.as_str()])
        }
        TurnResult::ToolError(reason) => {
            let inline = format!("[tool_error] {reason}");
            join_non_empty_lines(&[assistant_preface, inline.as_str()])
        }
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
