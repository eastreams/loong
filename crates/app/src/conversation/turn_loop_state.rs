use serde_json::Value;

use super::turn_budget::{TurnRoundBudget, TurnRoundBudgetDecision};
use super::turn_engine::{ToolIntent, TurnResult};
use super::turn_loop_followup::FollowupPayloadBudget;
use super::turn_shared::{
    ReplyPersistenceMode, ToolDrivenFollowupPayload, ToolDrivenReplyBaseDecision,
    ToolDrivenReplyPhase,
};

#[derive(Debug, Clone)]
pub(super) struct TurnLoopSessionState {
    pub(super) messages: Vec<Value>,
    pub(super) raw_tool_output_requested: bool,
    pub(super) last_raw_reply: String,
    pub(super) loop_supervisor: ToolLoopSupervisor,
    pub(super) followup_payload_budget: FollowupPayloadBudget,
    pub(super) total_tool_calls: usize,
}

#[derive(Debug, Clone)]
pub(super) struct RoundKernelEvaluation {
    pub(super) assistant_preface: String,
    pub(super) had_tool_intents: bool,
    pub(super) tool_request_summary: Option<String>,
    pub(super) turn_result: TurnResult,
    pub(super) loop_verdict: Option<ToolLoopSupervisorVerdict>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RoundKernelDecision {
    ContinueWithFollowup(RoundFollowup),
    FinalizeDirect {
        reply: String,
    },
    FinalizeWithCompletionPass {
        raw_reply: String,
        followup: RoundFollowup,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TurnLoopTerminalAction {
    PersistReply {
        reply: String,
        persistence_mode: ReplyPersistenceMode,
    },
    ReturnError {
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RoundFollowup {
    Tool {
        assistant_preface: String,
        payload: ToolDrivenFollowupPayload,
        tool_request_summary: Option<String>,
        loop_warning_reason: Option<String>,
    },
    Guard {
        assistant_preface: String,
        reason: String,
        latest_tool_payload: Option<ToolDrivenFollowupPayload>,
    },
}

#[derive(Debug, Clone, Copy)]
pub(super) struct TurnLoopPolicy {
    pub(super) max_rounds: usize,
    pub(super) max_tool_steps_per_round: usize,
    pub(super) max_followup_tool_payload_chars: usize,
    pub(super) max_followup_tool_payload_chars_total: usize,
    pub(super) max_total_tool_calls: usize,
    pub(super) max_consecutive_same_tool: usize,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ToolLoopSupervisor {
    warned_same_tool_key: Option<String>,
    consecutive_same_tool: usize,
    last_tool_name: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) enum ToolLoopSupervisorVerdict {
    Continue,
    InjectWarning { reason: String },
    HardStop { reason: String },
}

impl RoundKernelEvaluation {
    pub(super) fn reply_phase(&self, raw_tool_output_requested: bool) -> ToolDrivenReplyPhase {
        ToolDrivenReplyPhase::new(
            self.assistant_preface.as_str(),
            self.had_tool_intents,
            raw_tool_output_requested,
            &self.turn_result,
        )
    }

    pub(super) fn loop_warning_reason(&self) -> Option<String> {
        match self.loop_verdict.as_ref() {
            Some(ToolLoopSupervisorVerdict::InjectWarning { reason }) => Some(reason.clone()),
            _ => None,
        }
    }

    pub(super) fn hard_stop_reason(&self) -> Option<String> {
        match self.loop_verdict.as_ref() {
            Some(ToolLoopSupervisorVerdict::HardStop { reason }) => Some(reason.clone()),
            _ => None,
        }
    }
}

impl ToolLoopSupervisor {
    pub(super) fn observe_round(
        &mut self,
        policy: &TurnLoopPolicy,
        tool_name_signature: &str,
    ) -> ToolLoopSupervisorVerdict {
        if self.last_tool_name.as_deref() == Some(tool_name_signature) {
            self.consecutive_same_tool += 1;
        } else {
            self.last_tool_name = Some(tool_name_signature.to_owned());
            self.consecutive_same_tool = 1;
            self.warned_same_tool_key = None;
        }

        if self.consecutive_same_tool < policy.max_consecutive_same_tool {
            self.warned_same_tool_key = None;
            return ToolLoopSupervisorVerdict::Continue;
        }

        let reason_key = format!("consecutive_same_tool:{tool_name_signature}");
        let reason = format!(
            "consecutive_same_tool: {tool_name_signature} called {} times in a row (limit={})",
            self.consecutive_same_tool, policy.max_consecutive_same_tool
        );

        if self.warned_same_tool_key.as_deref() == Some(reason_key.as_str()) {
            ToolLoopSupervisorVerdict::HardStop { reason }
        } else {
            self.warned_same_tool_key = Some(reason_key);
            ToolLoopSupervisorVerdict::InjectWarning { reason }
        }
    }
}

pub(super) fn build_round_limit_terminal_action(last_raw_reply: &str) -> TurnLoopTerminalAction {
    TurnLoopTerminalAction::PersistReply {
        persistence_mode: ReplyPersistenceMode::Success,
        reply: if last_raw_reply.is_empty() {
            "agent_loop_round_limit_reached".to_owned()
        } else {
            last_raw_reply.to_owned()
        },
    }
}

pub(super) fn tool_round_has_observable_outcome(turn_result: &TurnResult) -> bool {
    !matches!(turn_result, TurnResult::ProviderError(_))
}

pub(super) fn tool_name_signature(intents: &[ToolIntent]) -> String {
    intents
        .iter()
        .map(|intent| intent.tool_name.trim())
        .collect::<Vec<_>>()
        .join("||")
}

pub(super) fn decide_round_kernel_action(
    round_budget: TurnRoundBudget,
    evaluation: RoundKernelEvaluation,
    reply_phase: ToolDrivenReplyPhase,
) -> RoundKernelDecision {
    let (raw_reply, tool_payload) = match reply_phase.into_decision() {
        ToolDrivenReplyBaseDecision::FinalizeDirect { reply } => {
            return RoundKernelDecision::FinalizeDirect { reply };
        }
        ToolDrivenReplyBaseDecision::RequireFollowup { raw_reply, payload } => (raw_reply, payload),
    };

    if let Some(reason) = evaluation.hard_stop_reason() {
        return RoundKernelDecision::FinalizeWithCompletionPass {
            raw_reply,
            followup: RoundFollowup::Guard {
                assistant_preface: evaluation.assistant_preface,
                reason,
                latest_tool_payload: Some(tool_payload),
            },
        };
    }

    let followup = RoundFollowup::Tool {
        assistant_preface: evaluation.assistant_preface.clone(),
        payload: tool_payload,
        tool_request_summary: evaluation.tool_request_summary.clone(),
        loop_warning_reason: evaluation.loop_warning_reason(),
    };

    match round_budget.followup_decision() {
        TurnRoundBudgetDecision::ContinueWithFollowup => {
            RoundKernelDecision::ContinueWithFollowup(followup)
        }
        TurnRoundBudgetDecision::FinalizeWithCompletionPass => {
            RoundKernelDecision::FinalizeWithCompletionPass {
                raw_reply,
                followup,
            }
        }
    }
}
