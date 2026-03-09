use serde_json::json;

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
                let reply = compose_assistant_reply(
                    turn.assistant_text.as_str(),
                    had_tool_intents,
                    turn_result,
                );
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
