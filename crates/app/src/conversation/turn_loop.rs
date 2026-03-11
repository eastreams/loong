use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};

use serde_json::{Value, json};

use crate::CliResult;
use crate::KernelContext;

use super::super::config::LoongClawConfig;
use super::ProviderErrorMode;
use super::persistence::{format_provider_error_reply, persist_error_turns, persist_success_turns};
use super::runtime::{ConversationRuntime, DefaultConversationRuntime};
use super::turn_engine::{ProviderTurn, ToolIntent, TurnEngine, TurnResult};

#[derive(Default)]
pub struct ConversationTurnLoop;

const TOOL_FOLLOWUP_PROMPT: &str = "Use the tool result above to answer the original user request in natural language. Do not include raw JSON, payload wrappers, or status markers unless the user explicitly asks for raw output.";
const TOOL_LOOP_GUARD_PROMPT: &str = "Detected tool-loop behavior across rounds. Do not repeat identical or cyclical tool calls without new evidence. Adjust strategy (different tool, arguments, or decomposition) or provide the best possible final answer and clearly state remaining gaps.";

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
        let mut messages = runtime
            .build_messages(config, session_id, true, kernel_ctx)
            .await?;
        messages.push(json!({
            "role": "user",
            "content": user_input,
        }));
        let raw_tool_output_requested = user_requested_raw_tool_output(user_input);
        let mut last_raw_reply = String::new();
        let policy = TurnLoopPolicy::from_config(config);
        let mut loop_supervisor = ToolLoopSupervisor::default();
        let mut followup_payload_budget = FollowupPayloadBudget::new(
            policy.max_followup_tool_payload_chars,
            policy.max_followup_tool_payload_chars_total,
        );

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
            let current_tool_signature =
                had_tool_intents.then(|| tool_intent_signature_for_turn(&turn));
            let current_tool_name_signature =
                had_tool_intents.then(|| tool_name_signature(&turn.tool_intents));

            let turn_result = TurnEngine::new(policy.max_tool_steps_per_round)
                .execute_turn(&turn, kernel_ctx)
                .await;
            let loop_supervisor_verdict = if let (Some(signature), Some(name_signature)) = (
                current_tool_signature.as_deref(),
                current_tool_name_signature.as_deref(),
            ) {
                tool_round_outcome(&turn_result).map(|outcome| {
                    loop_supervisor.observe_round(
                        &policy,
                        signature,
                        name_signature,
                        outcome.fingerprint.as_str(),
                        outcome.failed,
                    )
                })
            } else {
                None
            };

            let reply = match turn_result {
                TurnResult::FinalText(tool_text) if had_tool_intents => {
                    let raw_reply =
                        join_non_empty_lines(&[turn.assistant_text.as_str(), tool_text.as_str()]);
                    last_raw_reply = raw_reply.clone();
                    if let Some(ToolLoopSupervisorVerdict::HardStop { reason }) =
                        loop_supervisor_verdict.as_ref()
                    {
                        if raw_tool_output_requested {
                            raw_reply
                        } else {
                            append_repeated_tool_guard_followup_messages(
                                &mut messages,
                                turn.assistant_text.as_str(),
                                reason.as_str(),
                                user_input,
                                Some(("tool_result", tool_text.as_str())),
                                &mut followup_payload_budget,
                            );
                            request_completion_with_raw_fallback(
                                runtime,
                                config,
                                &messages,
                                raw_reply.as_str(),
                            )
                            .await
                        }
                    } else {
                        let loop_warning_reason = match loop_supervisor_verdict.as_ref() {
                            Some(ToolLoopSupervisorVerdict::InjectWarning { reason }) => {
                                Some(reason.as_str())
                            }
                            _ => None,
                        };
                        if raw_tool_output_requested {
                            raw_reply
                        } else {
                            append_tool_followup_messages(
                                &mut messages,
                                turn.assistant_text.as_str(),
                                tool_text.as_str(),
                                user_input,
                                &mut followup_payload_budget,
                                loop_warning_reason,
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
                }
                TurnResult::ToolDenied(reason) if had_tool_intents => {
                    let raw_reply = compose_assistant_reply(
                        turn.assistant_text.as_str(),
                        had_tool_intents,
                        TurnResult::ToolDenied(reason.clone()),
                    );
                    last_raw_reply = raw_reply.clone();
                    if let Some(ToolLoopSupervisorVerdict::HardStop {
                        reason: loop_reason,
                    }) = loop_supervisor_verdict.as_ref()
                    {
                        if raw_tool_output_requested {
                            raw_reply
                        } else {
                            append_repeated_tool_guard_followup_messages(
                                &mut messages,
                                turn.assistant_text.as_str(),
                                loop_reason.as_str(),
                                user_input,
                                Some(("tool_failure", reason.as_str())),
                                &mut followup_payload_budget,
                            );
                            request_completion_with_raw_fallback(
                                runtime,
                                config,
                                &messages,
                                raw_reply.as_str(),
                            )
                            .await
                        }
                    } else {
                        let loop_warning_reason = match loop_supervisor_verdict.as_ref() {
                            Some(ToolLoopSupervisorVerdict::InjectWarning { reason }) => {
                                Some(reason.as_str())
                            }
                            _ => None,
                        };
                        if raw_tool_output_requested {
                            raw_reply
                        } else {
                            append_tool_failure_followup_messages(
                                &mut messages,
                                turn.assistant_text.as_str(),
                                reason.as_str(),
                                user_input,
                                &mut followup_payload_budget,
                                loop_warning_reason,
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
                }
                TurnResult::ToolError(reason) if had_tool_intents => {
                    let raw_reply = compose_assistant_reply(
                        turn.assistant_text.as_str(),
                        had_tool_intents,
                        TurnResult::ToolError(reason.clone()),
                    );
                    last_raw_reply = raw_reply.clone();
                    if let Some(ToolLoopSupervisorVerdict::HardStop {
                        reason: loop_reason,
                    }) = loop_supervisor_verdict.as_ref()
                    {
                        if raw_tool_output_requested {
                            raw_reply
                        } else {
                            append_repeated_tool_guard_followup_messages(
                                &mut messages,
                                turn.assistant_text.as_str(),
                                loop_reason.as_str(),
                                user_input,
                                Some(("tool_failure", reason.as_str())),
                                &mut followup_payload_budget,
                            );
                            request_completion_with_raw_fallback(
                                runtime,
                                config,
                                &messages,
                                raw_reply.as_str(),
                            )
                            .await
                        }
                    } else {
                        let loop_warning_reason = match loop_supervisor_verdict.as_ref() {
                            Some(ToolLoopSupervisorVerdict::InjectWarning { reason }) => {
                                Some(reason.as_str())
                            }
                            _ => None,
                        };
                        if raw_tool_output_requested {
                            raw_reply
                        } else {
                            append_tool_failure_followup_messages(
                                &mut messages,
                                turn.assistant_text.as_str(),
                                reason.as_str(),
                                user_input,
                                &mut followup_payload_budget,
                                loop_warning_reason,
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
    followup_payload_budget: &mut FollowupPayloadBudget,
    loop_warning_reason: Option<&str>,
) {
    let preface = assistant_preface.trim();
    if !preface.is_empty() {
        messages.push(json!({
            "role": "assistant",
            "content": preface,
        }));
    }
    let bounded_result = followup_payload_budget.truncate_payload("tool_result", tool_result_text);
    messages.push(json!({
        "role": "assistant",
        "content": format!("[tool_result]\n{bounded_result}"),
    }));
    if let Some(reason) = loop_warning_reason {
        messages.push(json!({
            "role": "assistant",
            "content": format!("[tool_loop_warning]\n{reason}"),
        }));
    }
    messages.push(json!({
        "role": "user",
        "content": build_tool_followup_prompt(user_input, loop_warning_reason),
    }));
}

fn append_tool_failure_followup_messages(
    messages: &mut Vec<Value>,
    assistant_preface: &str,
    tool_failure_reason: &str,
    user_input: &str,
    followup_payload_budget: &mut FollowupPayloadBudget,
    loop_warning_reason: Option<&str>,
) {
    let preface = assistant_preface.trim();
    if !preface.is_empty() {
        messages.push(json!({
            "role": "assistant",
            "content": preface,
        }));
    }
    let bounded_failure =
        followup_payload_budget.truncate_payload("tool_failure", tool_failure_reason);
    messages.push(json!({
        "role": "assistant",
        "content": format!("[tool_failure]\n{bounded_failure}"),
    }));
    if let Some(reason) = loop_warning_reason {
        messages.push(json!({
            "role": "assistant",
            "content": format!("[tool_loop_warning]\n{reason}"),
        }));
    }
    messages.push(json!({
        "role": "user",
        "content": build_tool_followup_prompt(user_input, loop_warning_reason),
    }));
}

fn append_repeated_tool_guard_followup_messages(
    messages: &mut Vec<Value>,
    assistant_preface: &str,
    reason: &str,
    user_input: &str,
    latest_tool_context: Option<(&str, &str)>,
    followup_payload_budget: &mut FollowupPayloadBudget,
) {
    let preface = assistant_preface.trim();
    if !preface.is_empty() {
        messages.push(json!({
            "role": "assistant",
            "content": preface,
        }));
    }
    if let Some((label, text)) = latest_tool_context {
        let bounded = followup_payload_budget.truncate_payload(label, text);
        messages.push(json!({
            "role": "assistant",
            "content": format!("[{label}]\n{bounded}"),
        }));
    }
    messages.push(json!({
        "role": "assistant",
        "content": format!("[tool_loop_guard]\n{reason}"),
    }));
    messages.push(json!({
        "role": "user",
        "content": build_tool_loop_guard_prompt(user_input, reason),
    }));
}

fn build_tool_loop_guard_prompt(user_input: &str, reason: &str) -> String {
    format!(
        "{TOOL_LOOP_GUARD_PROMPT}\n\nLoop guard reason:\n{reason}\n\nOriginal request:\n{user_input}"
    )
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

fn build_tool_followup_prompt(user_input: &str, loop_warning_reason: Option<&str>) -> String {
    if let Some(reason) = loop_warning_reason {
        return format!(
            "{TOOL_FOLLOWUP_PROMPT}\n\nLoop warning:\n{reason}\nAvoid repeating the same tool call with unchanged results. Try a different tool, adjust arguments, or provide a best-effort final answer if evidence is sufficient.\n\nOriginal request:\n{user_input}"
        );
    }
    format!("{TOOL_FOLLOWUP_PROMPT}\n\nOriginal request:\n{user_input}")
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
struct FollowupPayloadBudget {
    per_round_max_chars: usize,
    remaining_total_chars: usize,
}

impl FollowupPayloadBudget {
    fn new(per_round_max_chars: usize, total_max_chars: usize) -> Self {
        Self {
            per_round_max_chars: per_round_max_chars.max(1),
            remaining_total_chars: total_max_chars,
        }
    }

    fn truncate_payload(&mut self, label: &str, text: &str) -> String {
        let per_round_allowed = self
            .per_round_max_chars
            .min(self.remaining_total_chars.max(1));
        if self.remaining_total_chars == 0 {
            let removed = text.trim().chars().count();
            return format!("[{label}_truncated] removed_chars={removed} budget_exhausted=true");
        }

        let bounded = truncate_followup_tool_payload(label, text, per_round_allowed);
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

#[derive(Debug, Clone)]
struct ToolRoundOutcome {
    fingerprint: String,
    failed: bool,
}

fn tool_round_outcome(turn_result: &TurnResult) -> Option<ToolRoundOutcome> {
    match turn_result {
        TurnResult::FinalText(text) => Some(ToolRoundOutcome {
            fingerprint: text_fingerprint("tool_final_text", text),
            failed: false,
        }),
        TurnResult::ToolDenied(reason) => Some(ToolRoundOutcome {
            fingerprint: text_fingerprint("tool_denied", reason),
            failed: true,
        }),
        TurnResult::ToolError(reason) => Some(ToolRoundOutcome {
            fingerprint: text_fingerprint("tool_error", reason),
            failed: true,
        }),
        TurnResult::NeedsApproval(_) | TurnResult::ProviderError(_) => None,
    }
}

fn text_fingerprint(label: &str, text: &str) -> String {
    let normalized = text.trim();
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    let digest = hasher.finish();
    format!("{label}:{digest:016x}")
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

fn tool_name_signature(intents: &[ToolIntent]) -> String {
    intents
        .iter()
        .map(|intent| intent.tool_name.trim())
        .collect::<Vec<_>>()
        .join("||")
}

#[derive(Debug, Clone, Copy)]
struct TurnLoopPolicy {
    max_rounds: usize,
    max_tool_steps_per_round: usize,
    max_repeated_tool_call_rounds: usize,
    max_ping_pong_cycles: usize,
    max_same_tool_failure_rounds: usize,
    max_followup_tool_payload_chars: usize,
    max_followup_tool_payload_chars_total: usize,
}

impl TurnLoopPolicy {
    fn from_config(config: &LoongClawConfig) -> Self {
        let turn_loop = &config.conversation.turn_loop;
        Self {
            max_rounds: turn_loop.max_rounds.max(1),
            max_tool_steps_per_round: turn_loop.max_tool_steps_per_round.max(1),
            max_repeated_tool_call_rounds: turn_loop.max_repeated_tool_call_rounds.max(1),
            max_ping_pong_cycles: turn_loop.max_ping_pong_cycles.max(1),
            max_same_tool_failure_rounds: turn_loop.max_same_tool_failure_rounds.max(1),
            max_followup_tool_payload_chars: turn_loop.max_followup_tool_payload_chars.max(256),
            max_followup_tool_payload_chars_total: turn_loop
                .max_followup_tool_payload_chars_total
                .max(1),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ToolLoopSupervisor {
    last_pattern: Option<String>,
    last_pattern_streak: usize,
    warned_reason_key: Option<String>,
    recent_rounds: VecDeque<ToolLoopObservation>,
}

#[derive(Debug, Clone)]
enum ToolLoopSupervisorVerdict {
    Continue,
    InjectWarning { reason: String },
    HardStop { reason: String },
}

#[derive(Debug, Clone)]
struct ToolLoopObservation {
    pattern: String,
    tool_name_signature: String,
    failed: bool,
}

#[derive(Debug, Clone)]
struct LoopDetectionReason {
    key: String,
    text: String,
}

impl ToolLoopSupervisor {
    const MAX_RECENT_ROUNDS: usize = 24;

    fn observe_round(
        &mut self,
        policy: &TurnLoopPolicy,
        tool_signature: &str,
        tool_name_signature: &str,
        outcome_fingerprint: &str,
        failed: bool,
    ) -> ToolLoopSupervisorVerdict {
        let pattern = format!("{tool_signature}::{outcome_fingerprint}");
        if self.last_pattern.as_deref() == Some(pattern.as_str()) {
            self.last_pattern_streak += 1;
        } else {
            self.last_pattern = Some(pattern.clone());
            self.last_pattern_streak = 1;
        }

        self.recent_rounds.push_back(ToolLoopObservation {
            pattern: pattern.clone(),
            tool_name_signature: tool_name_signature.to_owned(),
            failed,
        });
        if self.recent_rounds.len() > Self::MAX_RECENT_ROUNDS {
            self.recent_rounds.pop_front();
        }

        let detection = self
            .check_no_progress(policy.max_repeated_tool_call_rounds)
            .or_else(|| self.check_ping_pong(policy.max_ping_pong_cycles))
            .or_else(|| self.check_failure_streak(policy.max_same_tool_failure_rounds));

        match detection {
            Some(reason) => {
                if self.warned_reason_key.as_deref() == Some(reason.key.as_str()) {
                    ToolLoopSupervisorVerdict::HardStop {
                        reason: reason.text,
                    }
                } else {
                    self.warned_reason_key = Some(reason.key);
                    ToolLoopSupervisorVerdict::InjectWarning {
                        reason: reason.text,
                    }
                }
            }
            None => {
                self.warned_reason_key = None;
                ToolLoopSupervisorVerdict::Continue
            }
        }
    }

    fn check_no_progress(&self, threshold: usize) -> Option<LoopDetectionReason> {
        let pattern = self.last_pattern.as_deref()?;
        if self.last_pattern_streak <= threshold {
            return None;
        }
        Some(LoopDetectionReason {
            key: format!("no_progress:{pattern}"),
            text: format!(
                "repeated_tool_call_no_progress signature_streak={} threshold={threshold}",
                self.last_pattern_streak
            ),
        })
    }

    fn check_ping_pong(&self, cycles: usize) -> Option<LoopDetectionReason> {
        let minimum_rounds = cycles.saturating_mul(2);
        if cycles == 0 || self.recent_rounds.len() < minimum_rounds {
            return None;
        }

        let tail = self
            .recent_rounds
            .iter()
            .rev()
            .take(minimum_rounds)
            .collect::<Vec<_>>();
        let first = tail.first()?.pattern.as_str();
        let second = tail.get(1)?.pattern.as_str();
        if first == second {
            return None;
        }

        let alternating = tail.iter().enumerate().all(|(index, round)| {
            if index % 2 == 0 {
                round.pattern == first
            } else {
                round.pattern == second
            }
        });
        if !alternating {
            return None;
        }

        let (left, right) = if first <= second {
            (first, second)
        } else {
            (second, first)
        };
        Some(LoopDetectionReason {
            key: format!("ping_pong:{left}<->{right}"),
            text: format!(
                "ping_pong_tool_patterns cycles={} threshold={cycles}",
                minimum_rounds / 2
            ),
        })
    }

    fn check_failure_streak(&self, threshold: usize) -> Option<LoopDetectionReason> {
        let last = self.recent_rounds.back()?;
        if !last.failed {
            return None;
        }
        let streak = self
            .recent_rounds
            .iter()
            .rev()
            .take_while(|round| {
                round.failed && round.tool_name_signature == last.tool_name_signature
            })
            .count();
        if streak < threshold {
            return None;
        }
        Some(LoopDetectionReason {
            key: format!("failure_streak:{}", last.tool_name_signature),
            text: format!(
                "tool_failure_streak rounds={streak} threshold={threshold} tool={}",
                last.tool_name_signature
            ),
        })
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
