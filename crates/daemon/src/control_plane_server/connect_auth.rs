use super::*;

pub(super) fn verify_connect_device_challenge(
    state: &ControlPlaneHttpState,
    request: &ControlPlaneConnectRequest,
) -> Result<(), Box<Response>> {
    let Some(device) = request.device.as_ref() else {
        return Ok(());
    };

    let challenge = state
        .challenge_registry
        .consume(&device.nonce)
        .map_err(|error| Box::new(error_response(StatusCode::INTERNAL_SERVER_ERROR, error)))?
        .ok_or_else(|| {
            Box::new(error_response(
                StatusCode::UNAUTHORIZED,
                format!(
                    "unknown or expired control-plane challenge `{}`",
                    device.nonce
                ),
            ))
        })?;

    crate::control_plane_device_auth::validate_control_plane_device_challenge(
        request,
        &challenge,
        CONTROL_PLANE_CHALLENGE_MAX_FUTURE_SKEW_MS,
        crate::control_plane_device_auth::current_time_ms(),
    )
    .map_err(|error| {
        let status = if error.starts_with("invalid control-plane device public_key")
            || error.starts_with("invalid control-plane device signature")
            || error == "control-plane device public_key must decode to 32 bytes"
        {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::UNAUTHORIZED
        };
        Box::new(error_response(status, error))
    })
}

pub(super) fn verify_remote_connect_bootstrap_auth(
    state: &ControlPlaneHttpState,
    request: &ControlPlaneConnectRequest,
) -> Result<(), Box<Response>> {
    let requires_remote_auth = state.exposure_policy.requires_remote_auth();
    if !requires_remote_auth {
        return Ok(());
    }

    let device_present = request.device.is_some();
    if device_present {
        return Ok(());
    }

    let shared_token = state
        .exposure_policy
        .shared_token
        .as_deref()
        .ok_or_else(|| {
            Box::new(connect_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ControlPlaneConnectErrorCode::SharedTokenRequired,
                "remote control-plane posture is missing exposure shared token",
            ))
        })?;

    let presented_token = request
        .auth
        .as_ref()
        .and_then(|auth| auth.token.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(presented_token) = presented_token else {
        return Err(Box::new(connect_error_response(
            StatusCode::UNAUTHORIZED,
            ControlPlaneConnectErrorCode::SharedTokenRequired,
            "remote non-loopback operator connect requires auth.token",
        )));
    };

    let token_matches =
        mvp::crypto::timing_safe_eq(presented_token.as_bytes(), shared_token.as_bytes());
    if !token_matches {
        return Err(Box::new(connect_error_response(
            StatusCode::UNAUTHORIZED,
            ControlPlaneConnectErrorCode::SharedTokenInvalid,
            "remote non-loopback operator connect presented an invalid auth.token",
        )));
    }

    Ok(())
}

pub(super) fn extract_connection_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            headers
                .get("x-loong-control-token")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
}

pub(super) fn connection_scoped_capabilities(
    lease: &mvp::control_plane::ControlPlaneConnectionLease,
) -> std::collections::BTreeSet<Capability> {
    let mut capabilities = std::collections::BTreeSet::new();
    for raw_scope in &lease.principal.scopes {
        let Some(scope) = ControlPlaneScope::parse(raw_scope.as_str()) else {
            continue;
        };
        match scope {
            ControlPlaneScope::OperatorRead => {
                capabilities.insert(Capability::ControlRead);
            }
            ControlPlaneScope::OperatorWrite => {
                capabilities.insert(Capability::ControlWrite);
            }
            ControlPlaneScope::OperatorApprovals => {
                capabilities.insert(Capability::ControlApprovals);
            }
            ControlPlaneScope::OperatorPairing => {
                capabilities.insert(Capability::ControlPairing);
            }
            ControlPlaneScope::OperatorAcp => {
                capabilities.insert(Capability::ControlAcp);
            }
            ControlPlaneScope::OperatorAdmin => {
                capabilities.insert(Capability::ControlRead);
                capabilities.insert(Capability::ControlWrite);
                capabilities.insert(Capability::ControlApprovals);
                capabilities.insert(Capability::ControlPairing);
                capabilities.insert(Capability::ControlAcp);
            }
        }
    }
    capabilities
}

pub(super) fn required_capabilities_for_route(
    resolved: &loong_protocol::ResolvedRoute,
) -> Result<std::collections::BTreeSet<Capability>, String> {
    let mut capabilities = std::collections::BTreeSet::new();
    if let Some(required_capability) = resolved.policy.required_capability.as_deref() {
        let normalized_required = required_capability.replace('.', "_");
        let Some(capability) = Capability::parse(normalized_required.as_str()) else {
            return Err(format!(
                "unsupported control-plane required capability mapping `{required_capability}`"
            ));
        };
        let is_control_plane_capability = matches!(
            capability,
            Capability::ControlRead
                | Capability::ControlWrite
                | Capability::ControlApprovals
                | Capability::ControlPairing
                | Capability::ControlAcp
        );
        if !is_control_plane_capability {
            return Err(format!(
                "unsupported control-plane required capability mapping `{}`",
                capability.as_str()
            ));
        }
        capabilities.insert(capability);
    }
    Ok(capabilities)
}

pub(super) async fn authorize_control_plane_request(
    state: &ControlPlaneHttpState,
    method: &str,
    headers: &HeaderMap,
) -> Result<mvp::control_plane::ControlPlaneConnectionLease, Box<Response>> {
    let router = ProtocolRouter::default();
    let resolved = router.resolve(method).map_err(|error| {
        Box::new(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("control plane route resolution failed for `{method}`: {error}"),
        ))
    })?;

    let Some(token) = extract_connection_token(headers) else {
        return Err(Box::new(error_response(
            StatusCode::UNAUTHORIZED,
            format!("missing control-plane token for `{method}`"),
        )));
    };
    let Some(lease) = state.connection_registry.resolve(&token).map_err(|error| {
        Box::new(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("control plane connection lookup failed: {error}"),
        ))
    })?
    else {
        state.kernel_authority.remove_binding(&token);
        return Err(Box::new(error_response(
            StatusCode::UNAUTHORIZED,
            format!("unknown or expired control-plane token for `{method}`"),
        )));
    };

    if lease.principal.role != "operator" {
        return Err(Box::new(error_response(
            StatusCode::FORBIDDEN,
            format!(
                "role `{}` is not allowed to access `{method}`",
                lease.principal.role
            ),
        )));
    }

    let route_capabilities = required_capabilities_for_route(&resolved)
        .map_err(|error| Box::new(error_response(StatusCode::INTERNAL_SERVER_ERROR, error)))?;
    let scoped_capabilities = connection_scoped_capabilities(&lease);
    let missing_capability = route_capabilities
        .iter()
        .find(|capability| !scoped_capabilities.contains(capability))
        .copied();
    if let Some(capability) = missing_capability {
        let reason = format!(
            "missing control-plane capability `{}` for method `{method}`",
            capability.as_str()
        );
        return Err(Box::new(error_response(StatusCode::FORBIDDEN, reason)));
    }

    state
        .kernel_authority
        .authorize(&lease.token, method, &route_capabilities)
        .await
        .map_err(|error| Box::new(error_response(StatusCode::FORBIDDEN, error)))?;

    Ok(lease)
}
