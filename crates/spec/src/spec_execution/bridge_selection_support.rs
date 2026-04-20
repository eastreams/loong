use kernel::BridgeSupportMatrix;

use crate::spec_runtime::RunnerSpec;

use super::bridge_runtime_policy_support::bridge_support_spec_matrix;

pub(super) fn bridge_support_matrix(spec: &RunnerSpec) -> (BridgeSupportMatrix, bool) {
    match &spec.bridge_support {
        Some(bridge) if bridge.enabled => {
            (bridge_support_spec_matrix(bridge), bridge.enforce_supported)
        }
        _ => (BridgeSupportMatrix::default(), false),
    }
}
