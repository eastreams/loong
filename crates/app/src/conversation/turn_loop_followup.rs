use serde_json::Value;

use super::turn_shared::{
    ToolDrivenFollowupLabel, ToolDrivenFollowupPayload, ToolDrivenFollowupTextRef,
    build_tool_driven_followup_tail_with_request_summary, build_tool_loop_guard_tail,
    reduce_followup_payload_for_model,
};

pub(super) fn round_tool_payload_context(
    payload: &ToolDrivenFollowupPayload,
) -> ToolDrivenFollowupTextRef<'_> {
    payload.message_context()
}

pub(super) fn append_tool_driven_followup_messages(
    messages: &mut Vec<Value>,
    assistant_preface: &str,
    payload: &ToolDrivenFollowupPayload,
    user_input: &str,
    followup_payload_budget: &mut FollowupPayloadBudget,
    loop_warning_reason: Option<&str>,
    tool_request_summary: Option<&str>,
) {
    messages.extend(build_tool_driven_followup_tail_with_request_summary(
        assistant_preface,
        payload,
        user_input,
        loop_warning_reason,
        tool_request_summary,
        |label, text| {
            let reduced = reduce_followup_payload_for_model(label, text);
            followup_payload_budget.truncate_payload_text_label(label, reduced.as_ref())
        },
    ));
}

pub(super) fn append_repeated_tool_guard_followup_messages(
    messages: &mut Vec<Value>,
    assistant_preface: &str,
    reason: &str,
    user_input: &str,
    latest_tool_context: Option<ToolDrivenFollowupTextRef<'_>>,
    followup_payload_budget: &mut FollowupPayloadBudget,
) {
    messages.extend(build_tool_loop_guard_tail(
        assistant_preface,
        reason,
        user_input,
        latest_tool_context,
        |label, text| {
            let reduced = reduce_followup_payload_for_model(label.as_str(), text);
            followup_payload_budget.truncate_payload(label, reduced.as_ref())
        },
    ));
}

fn truncate_followup_tool_payload(label: &str, text: &str, max_chars: usize) -> String {
    let normalized = text.trim();
    let total_chars = normalized.chars().count();
    if total_chars <= max_chars {
        return normalized.to_owned();
    }

    let reserved_chars = 80usize;
    let keep_chars = max_chars.saturating_sub(reserved_chars).max(1);
    let truncated = normalized.chars().take(keep_chars).collect::<String>();
    let removed = total_chars.saturating_sub(keep_chars);
    format!("{truncated}\n[{label}_truncated] removed_chars={removed}")
}

#[derive(Debug, Clone)]
pub(super) struct FollowupPayloadBudget {
    per_round_max_chars: usize,
    remaining_total_chars: usize,
}

impl FollowupPayloadBudget {
    pub(super) fn new(per_round_max_chars: usize, total_max_chars: usize) -> Self {
        Self {
            per_round_max_chars: per_round_max_chars.max(1),
            remaining_total_chars: total_max_chars,
        }
    }

    pub(super) fn truncate_payload(
        &mut self,
        label: ToolDrivenFollowupLabel,
        text: &str,
    ) -> String {
        let label_text = label.as_str();
        self.truncate_payload_text_label(label_text, text)
    }

    pub(super) fn truncate_payload_text_label(&mut self, label_text: &str, text: &str) -> String {
        let per_round_allowed = self
            .per_round_max_chars
            .min(self.remaining_total_chars.max(1));
        if self.remaining_total_chars == 0 {
            let removed = text.trim().chars().count();
            return format!(
                "[{label_text}_truncated] removed_chars={removed} budget_exhausted=true"
            );
        }

        let bounded = truncate_followup_tool_payload(label_text, text, per_round_allowed);
        let normalized = text.trim();
        let total_chars = normalized.chars().count();
        let consumed_chars = if total_chars <= per_round_allowed {
            total_chars
        } else if per_round_allowed > 80 {
            per_round_allowed - 80
        } else {
            per_round_allowed
        };
        self.remaining_total_chars = self.remaining_total_chars.saturating_sub(consumed_chars);
        bounded
    }
}
