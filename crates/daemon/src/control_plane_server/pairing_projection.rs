use loong_protocol::{
    ControlPlanePairingRequestSummary, ControlPlanePairingStatus, ControlPlaneRole,
    ControlPlaneScope,
};

pub(crate) fn map_pairing_status(
    status: crate::mvp::control_plane::ControlPlanePairingStatus,
) -> ControlPlanePairingStatus {
    match status {
        crate::mvp::control_plane::ControlPlanePairingStatus::Pending => {
            ControlPlanePairingStatus::Pending
        }
        crate::mvp::control_plane::ControlPlanePairingStatus::Approved => {
            ControlPlanePairingStatus::Approved
        }
        crate::mvp::control_plane::ControlPlanePairingStatus::Rejected => {
            ControlPlanePairingStatus::Rejected
        }
    }
}

pub(crate) fn map_pairing_request_summary(
    request: crate::mvp::control_plane::ControlPlanePairingRequestRecord,
) -> ControlPlanePairingRequestSummary {
    ControlPlanePairingRequestSummary {
        pairing_request_id: request.pairing_request_id,
        device_id: request.device_id,
        client_id: request.client_id,
        public_key: request.public_key,
        role: match request.role.as_str() {
            "operator" => ControlPlaneRole::Operator,
            _ => ControlPlaneRole::Node,
        },
        requested_scopes: request
            .requested_scopes
            .into_iter()
            .filter_map(|scope| ControlPlaneScope::parse(scope.as_str()))
            .collect::<std::collections::BTreeSet<_>>(),
        status: map_pairing_status(request.status),
        requested_at_ms: request.requested_at_ms,
        resolved_at_ms: request.resolved_at_ms,
    }
}

pub(crate) fn parse_pairing_status(
    raw: &str,
) -> Result<crate::mvp::control_plane::ControlPlanePairingStatus, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pending" => Ok(crate::mvp::control_plane::ControlPlanePairingStatus::Pending),
        "approved" => Ok(crate::mvp::control_plane::ControlPlanePairingStatus::Approved),
        "rejected" => Ok(crate::mvp::control_plane::ControlPlanePairingStatus::Rejected),
        _ => Err(format!("unknown pairing status `{raw}`")),
    }
}
