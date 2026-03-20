use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConversationConfig {
    #[serde(default)]
    pub context_engine: Option<String>,
    #[serde(default)]
    pub turn_middlewares: Vec<String>,
    #[serde(default = "default_true")]
    pub compact_enabled: bool,
    #[serde(default)]
    pub compact_min_messages: Option<usize>,
    #[serde(default)]
    pub compact_trigger_estimated_tokens: Option<usize>,
    #[serde(default = "default_true")]
    pub compact_fail_open: bool,
    #[serde(default)]
    pub turn_loop: ConversationTurnLoopConfig,
    #[serde(default = "default_true")]
    pub hybrid_lane_enabled: bool,
    #[serde(default)]
    pub safe_lane_plan_execution_enabled: bool,
    #[serde(default = "default_fast_lane_max_tool_steps_per_turn")]
    pub fast_lane_max_tool_steps_per_turn: usize,
    #[serde(default)]
    pub fast_lane_parallel_tool_execution_enabled: bool,
    #[serde(default = "default_fast_lane_parallel_tool_execution_max_in_flight")]
    pub fast_lane_parallel_tool_execution_max_in_flight: usize,
    #[serde(default = "default_safe_lane_max_tool_steps_per_turn")]
    pub safe_lane_max_tool_steps_per_turn: usize,
    #[serde(default = "default_safe_lane_node_max_attempts")]
    pub safe_lane_node_max_attempts: u8,
    #[serde(default = "default_safe_lane_plan_max_wall_time_ms")]
    pub safe_lane_plan_max_wall_time_ms: u64,
    #[serde(default = "default_true")]
    pub safe_lane_verify_output_non_empty: bool,
    #[serde(default = "default_safe_lane_verify_min_output_chars")]
    pub safe_lane_verify_min_output_chars: usize,
    #[serde(default = "default_true")]
    pub safe_lane_verify_require_status_prefix: bool,
    #[serde(default = "default_true")]
    pub safe_lane_verify_adaptive_anchor_escalation: bool,
    #[serde(default = "default_safe_lane_verify_anchor_escalation_after_failures")]
    pub safe_lane_verify_anchor_escalation_after_failures: u32,
    #[serde(default = "default_safe_lane_verify_anchor_escalation_min_matches")]
    pub safe_lane_verify_anchor_escalation_min_matches: usize,
    #[serde(default = "default_true")]
    pub safe_lane_emit_runtime_events: bool,
    #[serde(default = "default_safe_lane_event_sample_every")]
    pub safe_lane_event_sample_every: u32,
    #[serde(default = "default_true")]
    pub safe_lane_event_adaptive_sampling: bool,
    #[serde(default = "default_safe_lane_event_adaptive_failure_threshold")]
    pub safe_lane_event_adaptive_failure_threshold: u32,
    #[serde(default = "default_safe_lane_verify_deny_markers")]
    pub safe_lane_verify_deny_markers: Vec<String>,
    #[serde(default = "default_safe_lane_replan_max_rounds")]
    pub safe_lane_replan_max_rounds: u8,
    #[serde(default = "default_safe_lane_replan_max_node_attempts")]
    pub safe_lane_replan_max_node_attempts: u8,
    #[serde(default = "default_true")]
    pub safe_lane_session_governor_enabled: bool,
    #[serde(default = "default_safe_lane_session_governor_window_turns")]
    pub safe_lane_session_governor_window_turns: usize,
    #[serde(default = "default_safe_lane_session_governor_failed_final_status_threshold")]
    pub safe_lane_session_governor_failed_final_status_threshold: u32,
    #[serde(default = "default_safe_lane_session_governor_backpressure_failure_threshold")]
    pub safe_lane_session_governor_backpressure_failure_threshold: u32,
    #[serde(default = "default_true")]
    pub safe_lane_session_governor_trend_enabled: bool,
    #[serde(default = "default_safe_lane_session_governor_trend_min_samples")]
    pub safe_lane_session_governor_trend_min_samples: usize,
    #[serde(default = "default_safe_lane_session_governor_trend_ewma_alpha")]
    pub safe_lane_session_governor_trend_ewma_alpha: f64,
    #[serde(default = "default_safe_lane_session_governor_trend_failure_ewma_threshold")]
    pub safe_lane_session_governor_trend_failure_ewma_threshold: f64,
    #[serde(default = "default_safe_lane_session_governor_trend_backpressure_ewma_threshold")]
    pub safe_lane_session_governor_trend_backpressure_ewma_threshold: f64,
    #[serde(default = "default_safe_lane_session_governor_recovery_success_streak")]
    pub safe_lane_session_governor_recovery_success_streak: u32,
    #[serde(default = "default_safe_lane_session_governor_recovery_max_failure_ewma")]
    pub safe_lane_session_governor_recovery_max_failure_ewma: f64,
    #[serde(default = "default_safe_lane_session_governor_recovery_max_backpressure_ewma")]
    pub safe_lane_session_governor_recovery_max_backpressure_ewma: f64,
    #[serde(default = "default_true")]
    pub safe_lane_session_governor_force_no_replan: bool,
    #[serde(default = "default_safe_lane_session_governor_force_node_max_attempts")]
    pub safe_lane_session_governor_force_node_max_attempts: u8,
    #[serde(default = "default_true")]
    pub safe_lane_backpressure_guard_enabled: bool,
    #[serde(default = "default_safe_lane_backpressure_max_total_attempts")]
    pub safe_lane_backpressure_max_total_attempts: u64,
    #[serde(default = "default_safe_lane_backpressure_max_replans")]
    pub safe_lane_backpressure_max_replans: u32,
    #[serde(default = "default_safe_lane_risk_threshold")]
    pub safe_lane_risk_threshold: u32,
    #[serde(default = "default_safe_lane_complexity_threshold")]
    pub safe_lane_complexity_threshold: u32,
    #[serde(default = "default_fast_lane_max_input_chars")]
    pub fast_lane_max_input_chars: usize,
    #[serde(default = "default_tool_result_payload_summary_limit_chars")]
    pub tool_result_payload_summary_limit_chars: usize,
    #[serde(default = "default_safe_lane_health_truncation_warn_threshold")]
    pub safe_lane_health_truncation_warn_threshold: f64,
    #[serde(default = "default_safe_lane_health_truncation_critical_threshold")]
    pub safe_lane_health_truncation_critical_threshold: f64,
    #[serde(default = "default_safe_lane_health_verify_failure_warn_threshold")]
    pub safe_lane_health_verify_failure_warn_threshold: f64,
    #[serde(default = "default_safe_lane_health_replan_warn_threshold")]
    pub safe_lane_health_replan_warn_threshold: f64,
    #[serde(default = "default_high_risk_keywords")]
    pub high_risk_keywords: Vec<String>,
}

impl Default for ConversationConfig {
    fn default() -> Self {
        Self {
            context_engine: None,
            turn_middlewares: Vec::new(),
            compact_enabled: default_true(),
            compact_min_messages: None,
            compact_trigger_estimated_tokens: None,
            compact_fail_open: default_true(),
            turn_loop: ConversationTurnLoopConfig::default(),
            hybrid_lane_enabled: default_true(),
            safe_lane_plan_execution_enabled: false,
            fast_lane_max_tool_steps_per_turn: default_fast_lane_max_tool_steps_per_turn(),
            fast_lane_parallel_tool_execution_enabled: false,
            fast_lane_parallel_tool_execution_max_in_flight:
                default_fast_lane_parallel_tool_execution_max_in_flight(),
            safe_lane_max_tool_steps_per_turn: default_safe_lane_max_tool_steps_per_turn(),
            safe_lane_node_max_attempts: default_safe_lane_node_max_attempts(),
            safe_lane_plan_max_wall_time_ms: default_safe_lane_plan_max_wall_time_ms(),
            safe_lane_verify_output_non_empty: default_true(),
            safe_lane_verify_min_output_chars: default_safe_lane_verify_min_output_chars(),
            safe_lane_verify_require_status_prefix: default_true(),
            safe_lane_verify_adaptive_anchor_escalation: default_true(),
            safe_lane_verify_anchor_escalation_after_failures:
                default_safe_lane_verify_anchor_escalation_after_failures(),
            safe_lane_verify_anchor_escalation_min_matches:
                default_safe_lane_verify_anchor_escalation_min_matches(),
            safe_lane_emit_runtime_events: default_true(),
            safe_lane_event_sample_every: default_safe_lane_event_sample_every(),
            safe_lane_event_adaptive_sampling: default_true(),
            safe_lane_event_adaptive_failure_threshold:
                default_safe_lane_event_adaptive_failure_threshold(),
            safe_lane_verify_deny_markers: default_safe_lane_verify_deny_markers(),
            safe_lane_replan_max_rounds: default_safe_lane_replan_max_rounds(),
            safe_lane_replan_max_node_attempts: default_safe_lane_replan_max_node_attempts(),
            safe_lane_session_governor_enabled: default_true(),
            safe_lane_session_governor_window_turns:
                default_safe_lane_session_governor_window_turns(),
            safe_lane_session_governor_failed_final_status_threshold:
                default_safe_lane_session_governor_failed_final_status_threshold(),
            safe_lane_session_governor_backpressure_failure_threshold:
                default_safe_lane_session_governor_backpressure_failure_threshold(),
            safe_lane_session_governor_trend_enabled: default_true(),
            safe_lane_session_governor_trend_min_samples:
                default_safe_lane_session_governor_trend_min_samples(),
            safe_lane_session_governor_trend_ewma_alpha:
                default_safe_lane_session_governor_trend_ewma_alpha(),
            safe_lane_session_governor_trend_failure_ewma_threshold:
                default_safe_lane_session_governor_trend_failure_ewma_threshold(),
            safe_lane_session_governor_trend_backpressure_ewma_threshold:
                default_safe_lane_session_governor_trend_backpressure_ewma_threshold(),
            safe_lane_session_governor_recovery_success_streak:
                default_safe_lane_session_governor_recovery_success_streak(),
            safe_lane_session_governor_recovery_max_failure_ewma:
                default_safe_lane_session_governor_recovery_max_failure_ewma(),
            safe_lane_session_governor_recovery_max_backpressure_ewma:
                default_safe_lane_session_governor_recovery_max_backpressure_ewma(),
            safe_lane_session_governor_force_no_replan: default_true(),
            safe_lane_session_governor_force_node_max_attempts:
                default_safe_lane_session_governor_force_node_max_attempts(),
            safe_lane_backpressure_guard_enabled: default_true(),
            safe_lane_backpressure_max_total_attempts:
                default_safe_lane_backpressure_max_total_attempts(),
            safe_lane_backpressure_max_replans: default_safe_lane_backpressure_max_replans(),
            safe_lane_risk_threshold: default_safe_lane_risk_threshold(),
            safe_lane_complexity_threshold: default_safe_lane_complexity_threshold(),
            fast_lane_max_input_chars: default_fast_lane_max_input_chars(),
            tool_result_payload_summary_limit_chars:
                default_tool_result_payload_summary_limit_chars(),
            safe_lane_health_truncation_warn_threshold:
                default_safe_lane_health_truncation_warn_threshold(),
            safe_lane_health_truncation_critical_threshold:
                default_safe_lane_health_truncation_critical_threshold(),
            safe_lane_health_verify_failure_warn_threshold:
                default_safe_lane_health_verify_failure_warn_threshold(),
            safe_lane_health_replan_warn_threshold: default_safe_lane_health_replan_warn_threshold(
            ),
            high_risk_keywords: default_high_risk_keywords(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConversationTurnLoopConfig {
    #[serde(default = "default_turn_loop_max_rounds")]
    pub max_rounds: usize,
    #[serde(default = "default_turn_loop_max_tool_steps_per_round")]
    pub max_tool_steps_per_round: usize,
    #[serde(default = "default_turn_loop_max_repeated_tool_call_rounds")]
    pub max_repeated_tool_call_rounds: usize,
    #[serde(default = "default_turn_loop_max_ping_pong_cycles")]
    pub max_ping_pong_cycles: usize,
    #[serde(default = "default_turn_loop_max_same_tool_failure_rounds")]
    pub max_same_tool_failure_rounds: usize,
    #[serde(default = "default_turn_loop_max_followup_tool_payload_chars")]
    pub max_followup_tool_payload_chars: usize,
    #[serde(default = "default_turn_loop_max_followup_tool_payload_chars_total")]
    pub max_followup_tool_payload_chars_total: usize,
    #[serde(default = "default_turn_loop_max_discovery_followup_rounds")]
    pub max_discovery_followup_rounds: usize,
    #[serde(default = "default_turn_loop_max_total_tool_calls")]
    pub max_total_tool_calls: usize,
    #[serde(default = "default_turn_loop_max_consecutive_same_tool")]
    pub max_consecutive_same_tool: usize,
}

impl Default for ConversationTurnLoopConfig {
    fn default() -> Self {
        Self {
            max_rounds: default_turn_loop_max_rounds(),
            max_tool_steps_per_round: default_turn_loop_max_tool_steps_per_round(),
            max_repeated_tool_call_rounds: default_turn_loop_max_repeated_tool_call_rounds(),
            max_ping_pong_cycles: default_turn_loop_max_ping_pong_cycles(),
            max_same_tool_failure_rounds: default_turn_loop_max_same_tool_failure_rounds(),
            max_followup_tool_payload_chars: default_turn_loop_max_followup_tool_payload_chars(),
            max_followup_tool_payload_chars_total:
                default_turn_loop_max_followup_tool_payload_chars_total(),
            max_discovery_followup_rounds: default_turn_loop_max_discovery_followup_rounds(),
            max_total_tool_calls: default_turn_loop_max_total_tool_calls(),
            max_consecutive_same_tool: default_turn_loop_max_consecutive_same_tool(),
        }
    }
}

impl ConversationConfig {
    pub fn context_engine_id(&self) -> Option<String> {
        self.context_engine
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
    }

    pub fn turn_middleware_ids(&self) -> Vec<String> {
        let mut seen = std::collections::BTreeSet::new();
        let mut ids = Vec::new();

        for raw in &self.turn_middlewares {
            let normalized = raw.trim().to_ascii_lowercase();
            if normalized.is_empty() || !seen.insert(normalized.clone()) {
                continue;
            }
            ids.push(normalized);
        }

        ids
    }

    pub fn compact_min_messages(&self) -> Option<usize> {
        self.compact_min_messages.filter(|value| *value > 0)
    }

    pub fn compact_trigger_estimated_tokens(&self) -> Option<usize> {
        self.compact_trigger_estimated_tokens
            .filter(|value| *value > 0)
    }

    pub fn should_compact(&self, message_count: usize) -> bool {
        self.should_compact_with_estimate(message_count, None)
    }

    pub fn should_compact_with_estimate(
        &self,
        message_count: usize,
        estimated_tokens: Option<usize>,
    ) -> bool {
        if !self.compact_enabled {
            return false;
        }

        let min_messages = self.compact_min_messages();
        let trigger_tokens = self.compact_trigger_estimated_tokens();
        if min_messages.is_none() && trigger_tokens.is_none() {
            return true;
        }

        let message_threshold_hit = min_messages.is_some_and(|min| message_count >= min);
        let token_threshold_hit = match (trigger_tokens, estimated_tokens) {
            (Some(threshold), Some(tokens)) => tokens >= threshold,
            _ => false,
        };

        message_threshold_hit || token_threshold_hit
    }

    pub fn compaction_fail_open(&self) -> bool {
        self.compact_fail_open
    }

    pub fn normalized_high_risk_keywords(&self) -> Vec<String> {
        self.high_risk_keywords
            .iter()
            .map(|keyword| keyword.trim().to_ascii_lowercase())
            .filter(|keyword| !keyword.is_empty())
            .collect()
    }

    pub fn fast_lane_max_tool_steps(&self) -> usize {
        self.fast_lane_max_tool_steps_per_turn.max(1)
    }

    pub fn fast_lane_parallel_tool_execution_max_in_flight(&self) -> usize {
        self.fast_lane_parallel_tool_execution_max_in_flight.max(1)
    }

    pub fn safe_lane_max_tool_steps(&self) -> usize {
        self.safe_lane_max_tool_steps_per_turn.max(1)
    }

    pub fn safe_lane_event_sample_every(&self) -> u32 {
        self.safe_lane_event_sample_every.max(1)
    }

    pub fn safe_lane_event_adaptive_failure_threshold(&self) -> u32 {
        self.safe_lane_event_adaptive_failure_threshold.max(1)
    }

    pub fn safe_lane_verify_anchor_escalation_after_failures(&self) -> u32 {
        self.safe_lane_verify_anchor_escalation_after_failures
            .max(1)
    }

    pub fn safe_lane_verify_anchor_escalation_min_matches(&self) -> usize {
        self.safe_lane_verify_anchor_escalation_min_matches.max(1)
    }

    pub fn safe_lane_backpressure_max_total_attempts(&self) -> u64 {
        self.safe_lane_backpressure_max_total_attempts.max(1)
    }

    pub fn safe_lane_backpressure_max_replans(&self) -> u32 {
        self.safe_lane_backpressure_max_replans.max(1)
    }

    pub fn safe_lane_session_governor_window_turns(&self) -> usize {
        self.safe_lane_session_governor_window_turns.max(1)
    }

    pub fn safe_lane_session_governor_failed_final_status_threshold(&self) -> u32 {
        self.safe_lane_session_governor_failed_final_status_threshold
            .max(1)
    }

    pub fn safe_lane_session_governor_backpressure_failure_threshold(&self) -> u32 {
        self.safe_lane_session_governor_backpressure_failure_threshold
            .max(1)
    }

    pub fn safe_lane_session_governor_trend_min_samples(&self) -> usize {
        self.safe_lane_session_governor_trend_min_samples.max(1)
    }

    pub fn safe_lane_session_governor_trend_ewma_alpha(&self) -> f64 {
        clamp_open_unit_interval(
            self.safe_lane_session_governor_trend_ewma_alpha,
            default_safe_lane_session_governor_trend_ewma_alpha(),
        )
    }

    pub fn safe_lane_session_governor_trend_failure_ewma_threshold(&self) -> f64 {
        clamp_unit_interval(
            self.safe_lane_session_governor_trend_failure_ewma_threshold,
            default_safe_lane_session_governor_trend_failure_ewma_threshold(),
        )
    }

    pub fn safe_lane_session_governor_trend_backpressure_ewma_threshold(&self) -> f64 {
        clamp_unit_interval(
            self.safe_lane_session_governor_trend_backpressure_ewma_threshold,
            default_safe_lane_session_governor_trend_backpressure_ewma_threshold(),
        )
    }

    pub fn safe_lane_session_governor_recovery_success_streak(&self) -> u32 {
        self.safe_lane_session_governor_recovery_success_streak
            .max(1)
    }

    pub fn safe_lane_session_governor_recovery_max_failure_ewma(&self) -> f64 {
        clamp_unit_interval(
            self.safe_lane_session_governor_recovery_max_failure_ewma,
            default_safe_lane_session_governor_recovery_max_failure_ewma(),
        )
    }

    pub fn safe_lane_session_governor_recovery_max_backpressure_ewma(&self) -> f64 {
        clamp_unit_interval(
            self.safe_lane_session_governor_recovery_max_backpressure_ewma,
            default_safe_lane_session_governor_recovery_max_backpressure_ewma(),
        )
    }

    pub fn safe_lane_session_governor_force_node_max_attempts(&self) -> u8 {
        self.safe_lane_session_governor_force_node_max_attempts
            .max(1)
    }

    pub fn tool_result_payload_summary_limit_chars(&self) -> usize {
        self.tool_result_payload_summary_limit_chars
            .clamp(256, 64_000)
    }

    pub fn safe_lane_health_truncation_warn_threshold(&self) -> f64 {
        clamp_unit_interval(
            self.safe_lane_health_truncation_warn_threshold,
            default_safe_lane_health_truncation_warn_threshold(),
        )
    }

    pub fn safe_lane_health_truncation_critical_threshold(&self) -> f64 {
        clamp_unit_interval(
            self.safe_lane_health_truncation_critical_threshold,
            default_safe_lane_health_truncation_critical_threshold(),
        )
        .max(self.safe_lane_health_truncation_warn_threshold())
    }

    pub fn safe_lane_health_verify_failure_warn_threshold(&self) -> f64 {
        clamp_unit_interval(
            self.safe_lane_health_verify_failure_warn_threshold,
            default_safe_lane_health_verify_failure_warn_threshold(),
        )
    }

    pub fn safe_lane_health_replan_warn_threshold(&self) -> f64 {
        clamp_unit_interval(
            self.safe_lane_health_replan_warn_threshold,
            default_safe_lane_health_replan_warn_threshold(),
        )
    }
}

const fn default_true() -> bool {
    true
}

const fn default_turn_loop_max_rounds() -> usize {
    4
}

const fn default_turn_loop_max_tool_steps_per_round() -> usize {
    1
}

const fn default_turn_loop_max_repeated_tool_call_rounds() -> usize {
    2
}

const fn default_turn_loop_max_ping_pong_cycles() -> usize {
    2
}

const fn default_turn_loop_max_same_tool_failure_rounds() -> usize {
    3
}

const fn default_turn_loop_max_followup_tool_payload_chars() -> usize {
    8_000
}

const fn default_turn_loop_max_followup_tool_payload_chars_total() -> usize {
    20_000
}

const fn default_turn_loop_max_discovery_followup_rounds() -> usize {
    2
}

const fn default_turn_loop_max_total_tool_calls() -> usize {
    200
}

const fn default_turn_loop_max_consecutive_same_tool() -> usize {
    10
}

const fn default_fast_lane_max_tool_steps_per_turn() -> usize {
    1
}

const fn default_fast_lane_parallel_tool_execution_max_in_flight() -> usize {
    4
}

const fn default_safe_lane_max_tool_steps_per_turn() -> usize {
    1
}

const fn default_safe_lane_node_max_attempts() -> u8 {
    2
}

const fn default_safe_lane_plan_max_wall_time_ms() -> u64 {
    30_000
}

const fn default_safe_lane_verify_min_output_chars() -> usize {
    8
}

const fn default_safe_lane_verify_anchor_escalation_after_failures() -> u32 {
    2
}

const fn default_safe_lane_verify_anchor_escalation_min_matches() -> usize {
    1
}

const fn default_safe_lane_replan_max_rounds() -> u8 {
    1
}

const fn default_safe_lane_replan_max_node_attempts() -> u8 {
    4
}

const fn default_safe_lane_session_governor_window_turns() -> usize {
    96
}

const fn default_safe_lane_session_governor_failed_final_status_threshold() -> u32 {
    3
}

const fn default_safe_lane_session_governor_backpressure_failure_threshold() -> u32 {
    1
}

const fn default_safe_lane_session_governor_trend_min_samples() -> usize {
    4
}

const fn default_safe_lane_session_governor_trend_ewma_alpha() -> f64 {
    0.35
}

const fn default_safe_lane_session_governor_trend_failure_ewma_threshold() -> f64 {
    0.60
}

const fn default_safe_lane_session_governor_trend_backpressure_ewma_threshold() -> f64 {
    0.20
}

const fn default_safe_lane_session_governor_recovery_success_streak() -> u32 {
    3
}

const fn default_safe_lane_session_governor_recovery_max_failure_ewma() -> f64 {
    0.25
}

const fn default_safe_lane_session_governor_recovery_max_backpressure_ewma() -> f64 {
    0.10
}

const fn default_safe_lane_session_governor_force_node_max_attempts() -> u8 {
    1
}

const fn default_safe_lane_event_sample_every() -> u32 {
    1
}

const fn default_safe_lane_event_adaptive_failure_threshold() -> u32 {
    1
}

const fn default_safe_lane_backpressure_max_total_attempts() -> u64 {
    32
}

const fn default_safe_lane_backpressure_max_replans() -> u32 {
    8
}

const fn default_safe_lane_risk_threshold() -> u32 {
    4
}

const fn default_safe_lane_complexity_threshold() -> u32 {
    6
}

const fn default_fast_lane_max_input_chars() -> usize {
    400
}

const fn default_tool_result_payload_summary_limit_chars() -> usize {
    2_048
}

const fn default_safe_lane_health_truncation_warn_threshold() -> f64 {
    0.30
}

const fn default_safe_lane_health_truncation_critical_threshold() -> f64 {
    0.60
}

const fn default_safe_lane_health_verify_failure_warn_threshold() -> f64 {
    0.40
}

const fn default_safe_lane_health_replan_warn_threshold() -> f64 {
    0.50
}

fn default_high_risk_keywords() -> Vec<String> {
    [
        "rm -rf",
        "drop table",
        "delete",
        "credential",
        "token",
        "secret",
        "prod",
        "production",
        "deploy",
        "payment",
        "wallet",
    ]
    .iter()
    .map(|keyword| (*keyword).to_owned())
    .collect()
}

fn default_safe_lane_verify_deny_markers() -> Vec<String> {
    vec![
        "tool_failure".to_owned(),
        "provider_error".to_owned(),
        "no_kernel_context".to_owned(),
        "tool_not_found".to_owned(),
    ]
}

fn clamp_unit_interval(value: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        fallback
    }
}

fn clamp_open_unit_interval(value: f64, fallback: f64) -> f64 {
    clamp_unit_interval(value, fallback).max(0.01)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_governor_trend_parameters_are_clamped_to_valid_ranges() {
        let config = ConversationConfig {
            safe_lane_session_governor_trend_min_samples: 0,
            safe_lane_session_governor_trend_ewma_alpha: f64::NAN,
            safe_lane_session_governor_trend_failure_ewma_threshold: 2.0,
            safe_lane_session_governor_trend_backpressure_ewma_threshold: -1.0,
            safe_lane_session_governor_recovery_success_streak: 0,
            safe_lane_session_governor_recovery_max_failure_ewma: -3.0,
            safe_lane_session_governor_recovery_max_backpressure_ewma: 4.0,
            ..ConversationConfig::default()
        };

        assert_eq!(config.safe_lane_session_governor_trend_min_samples(), 1);
        assert_eq!(
            config.safe_lane_session_governor_trend_ewma_alpha(),
            default_safe_lane_session_governor_trend_ewma_alpha()
        );
        assert_eq!(
            config.safe_lane_session_governor_trend_failure_ewma_threshold(),
            1.0
        );
        assert_eq!(
            config.safe_lane_session_governor_trend_backpressure_ewma_threshold(),
            0.0
        );
        assert_eq!(
            config.safe_lane_session_governor_recovery_success_streak(),
            1
        );
        assert_eq!(
            config.safe_lane_session_governor_recovery_max_failure_ewma(),
            0.0
        );
        assert_eq!(
            config.safe_lane_session_governor_recovery_max_backpressure_ewma(),
            1.0
        );
    }

    #[test]
    fn session_governor_trend_alpha_uses_open_interval_floor() {
        let config = ConversationConfig {
            safe_lane_session_governor_trend_ewma_alpha: 0.0,
            ..ConversationConfig::default()
        };

        assert_eq!(config.safe_lane_session_governor_trend_ewma_alpha(), 0.01);
    }

    #[test]
    fn tool_result_payload_summary_limit_is_clamped() {
        let too_small = ConversationConfig {
            tool_result_payload_summary_limit_chars: 0,
            ..ConversationConfig::default()
        };
        assert_eq!(too_small.tool_result_payload_summary_limit_chars(), 256);

        let too_large = ConversationConfig {
            tool_result_payload_summary_limit_chars: 1_000_000,
            ..ConversationConfig::default()
        };
        assert_eq!(too_large.tool_result_payload_summary_limit_chars(), 64_000);
    }

    #[test]
    fn fast_lane_parallel_tool_execution_max_in_flight_is_clamped() {
        let config = ConversationConfig {
            fast_lane_parallel_tool_execution_max_in_flight: 0,
            ..ConversationConfig::default()
        };

        assert_eq!(config.fast_lane_parallel_tool_execution_max_in_flight(), 1);
    }

    #[test]
    fn safe_lane_health_thresholds_are_clamped_and_ordered() {
        let config = ConversationConfig {
            safe_lane_health_truncation_warn_threshold: 2.0,
            safe_lane_health_truncation_critical_threshold: -1.0,
            safe_lane_health_verify_failure_warn_threshold: f64::NAN,
            safe_lane_health_replan_warn_threshold: -3.0,
            ..ConversationConfig::default()
        };
        assert_eq!(config.safe_lane_health_truncation_warn_threshold(), 1.0);
        assert_eq!(config.safe_lane_health_truncation_critical_threshold(), 1.0);
        assert_eq!(
            config.safe_lane_health_verify_failure_warn_threshold(),
            default_safe_lane_health_verify_failure_warn_threshold()
        );
        assert_eq!(config.safe_lane_health_replan_warn_threshold(), 0.0);
    }

    #[test]
    fn safe_lane_health_truncation_critical_threshold_respects_warn_floor() {
        let config = ConversationConfig {
            safe_lane_health_truncation_warn_threshold: 0.55,
            safe_lane_health_truncation_critical_threshold: 0.20,
            ..ConversationConfig::default()
        };
        assert_eq!(config.safe_lane_health_truncation_warn_threshold(), 0.55);
        assert_eq!(
            config.safe_lane_health_truncation_critical_threshold(),
            0.55
        );
    }
}
