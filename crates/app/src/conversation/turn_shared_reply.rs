use super::super::persistence::format_provider_error_reply;
use super::{
    ReplyResolutionMode, ToolDrivenFollowupKind, ToolDrivenFollowupPayload, TurnResult,
    format_approval_required_reply, join_non_empty_lines, sanitize_reply_text,
    tool_driven_followup_payload,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolDrivenReplyBaseDecision {
    FinalizeDirect {
        reply: String,
    },
    RequireFollowup {
        raw_reply: String,
        payload: ToolDrivenFollowupPayload,
    },
}

impl ToolDrivenReplyBaseDecision {
    pub fn resolution_mode(&self) -> ReplyResolutionMode {
        match self {
            Self::FinalizeDirect { .. } => ReplyResolutionMode::Direct,
            Self::RequireFollowup { .. } => ReplyResolutionMode::CompletionPass,
        }
    }

    pub fn followup_kind(&self) -> Option<ToolDrivenFollowupKind> {
        match self {
            Self::FinalizeDirect { .. } => None,
            Self::RequireFollowup { payload, .. } => Some(payload.kind()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDrivenReplyPhase {
    raw_reply: Option<String>,
    decision: ToolDrivenReplyBaseDecision,
}

impl ToolDrivenReplyPhase {
    pub fn new(
        assistant_preface: &str,
        had_tool_intents: bool,
        raw_tool_output_requested: bool,
        turn_result: &TurnResult,
    ) -> Self {
        let kernel = ToolDrivenReplyKernel::new(assistant_preface, had_tool_intents, turn_result);
        Self {
            raw_reply: kernel.raw_reply(),
            decision: kernel.base_decision(raw_tool_output_requested),
        }
    }

    #[cfg(test)]
    pub fn raw_reply(&self) -> Option<&str> {
        self.raw_reply.as_deref()
    }

    pub fn decision(&self) -> &ToolDrivenReplyBaseDecision {
        &self.decision
    }

    pub fn resolution_mode(&self) -> ReplyResolutionMode {
        self.decision.resolution_mode()
    }

    pub fn followup_kind(&self) -> Option<ToolDrivenFollowupKind> {
        self.decision.followup_kind()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ToolDrivenReplyKernel<'a> {
    assistant_preface: &'a str,
    had_tool_intents: bool,
    turn_result: &'a TurnResult,
}

impl<'a> ToolDrivenReplyKernel<'a> {
    pub fn new(
        assistant_preface: &'a str,
        had_tool_intents: bool,
        turn_result: &'a TurnResult,
    ) -> Self {
        Self {
            assistant_preface,
            had_tool_intents,
            turn_result,
        }
    }

    pub fn fallback_reply(&self) -> String {
        compose_assistant_reply(
            self.assistant_preface,
            self.had_tool_intents,
            self.turn_result.clone(),
        )
    }

    pub fn raw_reply(&self) -> Option<String> {
        if !self.had_tool_intents {
            return None;
        }
        match self.turn_result {
            TurnResult::FinalText(text)
            | TurnResult::StreamingText(text)
            | TurnResult::StreamingDone(text) => {
                let sanitized_text = sanitize_reply_text(text);
                let reply =
                    join_non_empty_lines(&[self.assistant_preface, sanitized_text.as_str()]);
                Some(reply)
            }
            TurnResult::NeedsApproval(requirement) => Some(format_approval_required_reply(
                self.assistant_preface,
                requirement,
            )),
            TurnResult::ToolDenied(failure) | TurnResult::ToolError(failure) => {
                Some(join_non_empty_lines(&[
                    self.assistant_preface,
                    failure.reason.as_str(),
                ]))
            }
            TurnResult::ProviderError(_) => None,
        }
    }

    pub fn followup_payload(&self) -> Option<ToolDrivenFollowupPayload> {
        tool_driven_followup_payload(self.had_tool_intents, self.turn_result)
    }

    pub fn base_decision(&self, raw_tool_output_requested: bool) -> ToolDrivenReplyBaseDecision {
        let fallback_reply = self.fallback_reply();
        let Some(payload) = self.followup_payload() else {
            return ToolDrivenReplyBaseDecision::FinalizeDirect {
                reply: fallback_reply,
            };
        };
        let raw_reply = self.raw_reply().unwrap_or_else(|| fallback_reply.clone());
        let recovery_requires_followup =
            matches!(payload, ToolDrivenFollowupPayload::DiscoveryRecovery { .. });
        if raw_tool_output_requested && !recovery_requires_followup {
            ToolDrivenReplyBaseDecision::FinalizeDirect { reply: raw_reply }
        } else {
            ToolDrivenReplyBaseDecision::RequireFollowup { raw_reply, payload }
        }
    }
}

pub fn user_requested_raw_tool_output(user_input: &str) -> bool {
    let normalized = user_input.to_ascii_lowercase();
    let trimmed = normalized.trim();

    if trimmed == "[ok]" {
        return true;
    }

    let explicit_signals = [
        "raw tool output",
        "raw output",
        "exact output",
        "full output",
        "verbatim",
        "raw json",
        "raw payload",
        "full payload",
        "exact payload",
        "payload as json",
        "output as json",
    ];

    explicit_signals
        .iter()
        .any(|signal| normalized.contains(signal))
}

pub fn compose_assistant_reply(
    assistant_preface: &str,
    had_tool_intents: bool,
    turn_result: TurnResult,
) -> String {
    match turn_result {
        TurnResult::FinalText(text)
        | TurnResult::StreamingText(text)
        | TurnResult::StreamingDone(text) => {
            let sanitized_text = sanitize_reply_text(text.as_str());
            if had_tool_intents {
                join_non_empty_lines(&[assistant_preface, sanitized_text.as_str()])
            } else {
                sanitized_text
            }
        }
        TurnResult::NeedsApproval(requirement) => {
            format_approval_required_reply(assistant_preface, &requirement)
        }
        TurnResult::ToolDenied(failure) => {
            join_non_empty_lines(&[assistant_preface, failure.reason.as_str()])
        }
        TurnResult::ToolError(failure) => {
            join_non_empty_lines(&[assistant_preface, failure.reason.as_str()])
        }
        TurnResult::ProviderError(failure) => {
            let inline = format_provider_error_reply(failure.reason.as_str());
            join_non_empty_lines(&[assistant_preface, inline.as_str()])
        }
    }
}
