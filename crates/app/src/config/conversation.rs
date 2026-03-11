use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationConfig {
    #[serde(default)]
    pub turn_loop: ConversationTurnLoopConfig,
    #[serde(default = "default_true")]
    pub hybrid_lane_enabled: bool,
    #[serde(default)]
    pub safe_lane_plan_execution_enabled: bool,
    #[serde(default = "default_fast_lane_max_tool_steps_per_turn")]
    pub fast_lane_max_tool_steps_per_turn: usize,
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
    #[serde(default = "default_high_risk_keywords")]
    pub high_risk_keywords: Vec<String>,
}

impl Default for ConversationConfig {
    fn default() -> Self {
        Self {
            turn_loop: ConversationTurnLoopConfig::default(),
            hybrid_lane_enabled: default_true(),
            safe_lane_plan_execution_enabled: false,
            fast_lane_max_tool_steps_per_turn: default_fast_lane_max_tool_steps_per_turn(),
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
            high_risk_keywords: default_high_risk_keywords(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        }
    }
}

impl ConversationConfig {
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

const fn default_fast_lane_max_tool_steps_per_turn() -> usize {
    1
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
        let mut config = ConversationConfig::default();
        config.safe_lane_session_governor_trend_min_samples = 0;
        config.safe_lane_session_governor_trend_ewma_alpha = f64::NAN;
        config.safe_lane_session_governor_trend_failure_ewma_threshold = 2.0;
        config.safe_lane_session_governor_trend_backpressure_ewma_threshold = -1.0;
        config.safe_lane_session_governor_recovery_success_streak = 0;
        config.safe_lane_session_governor_recovery_max_failure_ewma = -3.0;
        config.safe_lane_session_governor_recovery_max_backpressure_ewma = 4.0;

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
        let mut config = ConversationConfig::default();
        config.safe_lane_session_governor_trend_ewma_alpha = 0.0;

        assert_eq!(config.safe_lane_session_governor_trend_ewma_alpha(), 0.01);
    }
}
