use super::*;

pub(super) fn map_pairing_request(
    request: mvp::control_plane::ControlPlanePairingRequestRecord,
) -> ControlPlanePairingRequestSummary {
    crate::control_plane_server::pairing_projection::map_pairing_request_summary(request)
}

pub(super) fn principal_from_connect(
    request: &ControlPlaneConnectRequest,
    connection_id: String,
    granted_scopes: std::collections::BTreeSet<ControlPlaneScope>,
) -> ControlPlanePrincipal {
    crate::control_plane_device_auth::protocol_principal_from_connect_request(
        request,
        connection_id,
        granted_scopes,
    )
}

pub(super) fn parse_pairing_status(
    raw: &str,
) -> Result<mvp::control_plane::ControlPlanePairingStatus, String> {
    crate::control_plane_server::pairing_projection::parse_pairing_status(raw)
}

pub(super) fn normalize_required_text(value: &str, field_name: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field_name} is required"));
    }
    Ok(trimmed.to_owned())
}

pub(super) fn require_nonempty_text(value: &str, field_name: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field_name} is required"));
    }
    Ok(value.to_owned())
}
