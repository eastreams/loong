use serde_json::Value;

use super::super::super::config::LoongConfig;
use super::super::ProviderErrorMode;
use super::super::persistence::format_provider_error_reply;
use super::super::runtime::ConversationRuntime;
use super::super::runtime_binding::ConversationRuntimeBinding;
use super::super::turn_engine::ProviderTurn;
use super::{
    ParsedToolDrivenContinuationReply, parse_tool_driven_continuation_reply,
    salvage_missing_tool_call_reply_text, sanitize_reply_text,
};
use crate::CliResult;

#[derive(Debug, Clone)]
pub enum ProviderTurnRequestAction {
    Continue { turn: ProviderTurn },
    FinalizeInlineProviderError { reply: String },
    ReturnError { error: String },
}

pub fn decide_provider_turn_request_action(
    result: CliResult<ProviderTurn>,
    error_mode: ProviderErrorMode,
) -> ProviderTurnRequestAction {
    match result {
        Ok(turn) => ProviderTurnRequestAction::Continue { turn },
        Err(error) => match error_mode {
            ProviderErrorMode::Propagate => ProviderTurnRequestAction::ReturnError { error },
            ProviderErrorMode::InlineMessage => {
                ProviderTurnRequestAction::FinalizeInlineProviderError {
                    reply: format_provider_error_reply(&error),
                }
            }
        },
    }
}

pub async fn request_completion_with_raw_fallback_detailed<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    config: &LoongConfig,
    messages: &[Value],
    binding: ConversationRuntimeBinding<'_>,
    raw_reply: &str,
    retry_progress: crate::provider::ProviderRetryProgressCallback,
) -> ParsedToolDrivenContinuationReply {
    match runtime
        .request_completion_with_retry_progress(config, messages, binding, retry_progress)
        .await
    {
        Ok(final_reply) => {
            let parsed_reply = parse_tool_driven_continuation_reply(final_reply.as_str());
            let parsed_reply = ParsedToolDrivenContinuationReply {
                state: parsed_reply.state,
                reply: salvage_missing_tool_call_reply_text(parsed_reply.reply.as_str())
                    .unwrap_or(parsed_reply.reply),
            };
            if parsed_reply.reply.is_empty() && parsed_reply.state.is_none() {
                parse_tool_driven_continuation_reply(raw_reply)
            } else if parsed_reply.reply.is_empty() {
                ParsedToolDrivenContinuationReply {
                    state: parsed_reply.state,
                    reply: sanitize_reply_text(raw_reply),
                }
            } else {
                parsed_reply
            }
        }
        Err(_) => parse_tool_driven_continuation_reply(raw_reply),
    }
}

pub async fn request_completion_with_raw_fallback<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    config: &LoongConfig,
    messages: &[Value],
    binding: ConversationRuntimeBinding<'_>,
    raw_reply: &str,
    retry_progress: crate::provider::ProviderRetryProgressCallback,
) -> String {
    request_completion_with_raw_fallback_detailed(
        runtime,
        config,
        messages,
        binding,
        raw_reply,
        retry_progress,
    )
    .await
    .reply
}
