use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLane {
    Fast,
    Safe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneDecision {
    pub lane: ExecutionLane,
    pub routing_score: u32,
    pub complexity_score: u32,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaneArbiterPolicy {
    #[serde(default = "default_safe_lane_routing_threshold")]
    pub safe_lane_routing_threshold: u32,
    #[serde(default = "default_safe_lane_complexity_threshold")]
    pub safe_lane_complexity_threshold: u32,
    #[serde(default = "default_fast_lane_max_input_chars")]
    pub fast_lane_max_input_chars: usize,
    #[serde(default = "default_high_complexity_keywords")]
    pub high_complexity_keywords: BTreeSet<String>,
}

impl Default for LaneArbiterPolicy {
    fn default() -> Self {
        Self {
            safe_lane_routing_threshold: default_safe_lane_routing_threshold(),
            safe_lane_complexity_threshold: default_safe_lane_complexity_threshold(),
            fast_lane_max_input_chars: default_fast_lane_max_input_chars(),
            high_complexity_keywords: default_high_complexity_keywords(),
        }
    }
}

impl LaneArbiterPolicy {
    pub fn decide(&self, user_input: &str) -> LaneDecision {
        let routing_score = self.routing_score(user_input);
        let complexity_score = self.complexity_score(user_input);
        let mut reasons = Vec::new();

        if routing_score >= self.safe_lane_routing_threshold {
            reasons.push(format!(
                "routing_score_exceeded score={routing_score} threshold={}",
                self.safe_lane_routing_threshold
            ));
        }
        if complexity_score >= self.safe_lane_complexity_threshold {
            reasons.push(format!(
                "complexity_score_exceeded score={complexity_score} threshold={}",
                self.safe_lane_complexity_threshold
            ));
        }
        if user_input.chars().count() > self.fast_lane_max_input_chars {
            reasons.push(format!(
                "input_length_exceeded chars={} threshold={}",
                user_input.chars().count(),
                self.fast_lane_max_input_chars
            ));
        }

        let lane = if reasons.is_empty() {
            ExecutionLane::Fast
        } else {
            ExecutionLane::Safe
        };

        LaneDecision {
            lane,
            routing_score,
            complexity_score,
            reasons,
        }
    }

    fn routing_score(&self, user_input: &str) -> u32 {
        let normalized = user_input.to_ascii_lowercase();
        self.high_complexity_keywords
            .iter()
            .filter(|keyword| normalized.contains(keyword.as_str()))
            .count()
            .saturating_mul(2) as u32
    }

    fn complexity_score(&self, user_input: &str) -> u32 {
        let normalized = user_input.to_ascii_lowercase();
        let tokens = normalized.split_whitespace().count() as u32;
        let connectors = [
            " and ",
            " then ",
            " after ",
            " before ",
            " meanwhile ",
            "同时",
            "然后",
            "接着",
        ]
        .iter()
        .filter(|connector| normalized.contains(*connector))
        .count() as u32;
        let punctuation = user_input
            .chars()
            .filter(|c| matches!(c, ',' | ';' | ':' | '，' | '；' | '：'))
            .count() as u32;

        let token_component = if tokens > 50 {
            6
        } else if tokens > 30 {
            4
        } else if tokens > 15 {
            2
        } else {
            0
        };
        token_component + connectors.saturating_mul(2) + punctuation.min(3)
    }
}

const fn default_safe_lane_routing_threshold() -> u32 {
    4
}

const fn default_safe_lane_complexity_threshold() -> u32 {
    6
}

const fn default_fast_lane_max_input_chars() -> usize {
    400
}

fn default_high_complexity_keywords() -> BTreeSet<String> {
    [
        "drop table",
        "delete",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_risk_simple_request_routes_to_fast_lane() {
        let policy = LaneArbiterPolicy::default();
        let decision = policy.decide("read note.md and summarize briefly");
        assert_eq!(decision.lane, ExecutionLane::Fast);
        assert!(decision.reasons.is_empty());
    }

    #[test]
    fn high_complexity_keywords_route_to_safe_lane() {
        let policy = LaneArbiterPolicy::default();
        let decision = policy.decide("connect to production and deploy the update");
        assert_eq!(decision.lane, ExecutionLane::Safe);
        assert!(
            decision
                .reasons
                .iter()
                .any(|reason| reason.contains("routing_score_exceeded")),
            "expected routing reason, got: {:?}",
            decision.reasons
        );
    }

    #[test]
    fn complex_multi_clause_request_routes_to_safe_lane() {
        let policy = LaneArbiterPolicy::default();
        let decision = policy.decide(
            "first collect runtime evidence, then compare failure modes, and finally produce \
             a mitigation matrix before generating rollout checks",
        );
        assert_eq!(decision.lane, ExecutionLane::Safe);
        assert!(
            decision
                .reasons
                .iter()
                .any(|reason| reason.contains("complexity_score_exceeded")),
            "expected complexity reason, got: {:?}",
            decision.reasons
        );
    }
}
