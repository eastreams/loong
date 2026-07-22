use super::*;

pub(super) fn lane_policy_from_config(_config: &LoongConfig) -> LaneArbiterPolicy {
    LaneArbiterPolicy {
        ..LaneArbiterPolicy::default()
    }
}
