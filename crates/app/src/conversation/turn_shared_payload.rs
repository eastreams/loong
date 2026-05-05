use serde::Serialize;

pub use super::super::tool_result_line::ToolResultLine;
use super::super::turn_engine::{TurnFailure, TurnResult};
use super::{
    ToolResultContinuation, parse_tool_result_continuation, parse_tool_result_followup_context,
    sanitize_reply_text,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolDrivenFollowupPayload {
    ToolResult { text: String },
    ToolFailure { reason: String, retryable: bool },
    DiscoveryRecovery { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolDrivenFollowupKind {
    ToolResult,
    ToolFailure,
    DiscoveryRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolDrivenFollowupLabel {
    ToolResult,
    ToolFailure,
    DiscoveryRecovery,
}

impl ToolDrivenFollowupLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToolResult => "tool_result",
            Self::ToolFailure => "tool_failure",
            Self::DiscoveryRecovery => "tool_recovery",
        }
    }

    #[cfg(test)]
    pub fn from_marker(marker: &str) -> Option<Self> {
        match marker {
            "tool_result" => Some(Self::ToolResult),
            "tool_failure" => Some(Self::ToolFailure),
            "tool_recovery" => Some(Self::DiscoveryRecovery),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolDrivenFollowupTextRef<'a> {
    label: ToolDrivenFollowupLabel,
    text: &'a str,
}

impl<'a> ToolDrivenFollowupTextRef<'a> {
    pub fn new(label: ToolDrivenFollowupLabel, text: &'a str) -> Self {
        Self { label, text }
    }

    pub fn label(self) -> ToolDrivenFollowupLabel {
        self.label
    }

    pub fn text(self) -> &'a str {
        self.text
    }

    pub fn render_assistant_content(self) -> String {
        let marker = self.label.as_str();
        format!("[{marker}]\n{}", self.text)
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDrivenFollowupMessageOwned {
    label: ToolDrivenFollowupLabel,
    body: String,
}

#[cfg(test)]
impl ToolDrivenFollowupMessageOwned {
    pub fn parse_assistant_content(content: &str) -> Option<Self> {
        let (marker_line, body) = content.split_once('\n')?;
        let marker = marker_line.trim().strip_prefix('[')?.strip_suffix(']')?;
        let label = ToolDrivenFollowupLabel::from_marker(marker)?;
        Some(Self {
            label,
            body: body.to_owned(),
        })
    }

    pub fn label(&self) -> ToolDrivenFollowupLabel {
        self.label
    }

    pub fn body(&self) -> &str {
        self.body.as_str()
    }
}

impl ToolDrivenFollowupPayload {
    pub fn kind(&self) -> ToolDrivenFollowupKind {
        match self {
            Self::ToolResult { .. } => ToolDrivenFollowupKind::ToolResult,
            Self::ToolFailure { .. } => ToolDrivenFollowupKind::ToolFailure,
            Self::DiscoveryRecovery { .. } => ToolDrivenFollowupKind::DiscoveryRecovery,
        }
    }

    pub fn label(&self) -> ToolDrivenFollowupLabel {
        match self {
            Self::ToolResult { .. } => ToolDrivenFollowupLabel::ToolResult,
            Self::ToolFailure { .. } => ToolDrivenFollowupLabel::ToolFailure,
            Self::DiscoveryRecovery { .. } => ToolDrivenFollowupLabel::DiscoveryRecovery,
        }
    }

    pub fn message_context(&self) -> ToolDrivenFollowupTextRef<'_> {
        let label = self.label();
        match self {
            Self::ToolResult { text } => ToolDrivenFollowupTextRef::new(label, text.as_str()),
            Self::ToolFailure { reason, .. } => {
                ToolDrivenFollowupTextRef::new(label, reason.as_str())
            }
            Self::DiscoveryRecovery { reason } => {
                ToolDrivenFollowupTextRef::new(label, reason.as_str())
            }
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn retryable_failure(&self) -> bool {
        matches!(
            self,
            Self::ToolFailure {
                retryable: true,
                ..
            }
        )
    }

    pub fn requests_runtime_followup_chain(&self) -> bool {
        match self {
            Self::DiscoveryRecovery { .. } => true,
            Self::ToolResult { .. } => self.has_nonterminal_tool_result_continuation(),
            Self::ToolFailure { .. } => false,
        }
    }

    pub fn has_nonterminal_tool_result_continuation(&self) -> bool {
        self.tool_result_continuation()
            .is_some_and(|continuation| !continuation.is_terminal)
    }

    pub fn tool_result_continuation(&self) -> Option<ToolResultContinuation> {
        match self {
            Self::ToolResult { text } => {
                let tool_result_context = parse_tool_result_followup_context(text.as_str());
                tool_result_context
                    .as_ref()
                    .and_then(|context| parse_tool_result_continuation(&context.payload_json))
            }
            Self::ToolFailure { .. } | Self::DiscoveryRecovery { .. } => None,
        }
    }
}

pub fn turn_failure_supports_discovery_recovery(failure: &TurnFailure) -> bool {
    failure.supports_discovery_recovery
}

pub fn tool_driven_followup_payload(
    had_tool_intents: bool,
    turn_result: &TurnResult,
) -> Option<ToolDrivenFollowupPayload> {
    if !had_tool_intents {
        return None;
    }

    match turn_result {
        TurnResult::FinalText(text)
        | TurnResult::StreamingText(text)
        | TurnResult::StreamingDone(text) => {
            let sanitized_text = sanitize_reply_text(text);
            Some(ToolDrivenFollowupPayload::ToolResult {
                text: sanitized_text,
            })
        }
        TurnResult::NeedsApproval(_) => None,
        TurnResult::ToolDenied(failure) if turn_failure_supports_discovery_recovery(failure) => {
            Some(ToolDrivenFollowupPayload::DiscoveryRecovery {
                reason: failure.reason.clone(),
            })
        }
        TurnResult::ToolDenied(failure) | TurnResult::ToolError(failure) => {
            Some(ToolDrivenFollowupPayload::ToolFailure {
                reason: failure.reason.clone(),
                retryable: failure.retryable,
            })
        }
        TurnResult::ProviderError(_) => None,
    }
}
