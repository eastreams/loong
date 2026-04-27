use super::*;

pub(super) fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub(super) fn control_plane_device_signature_message(
    request: &ControlPlaneConnectRequest,
    device: &loong_protocol::ControlPlaneDeviceIdentity,
) -> Vec<u8> {
    let scopes = request
        .scopes
        .iter()
        .map(|scope| scope.as_str())
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "loong-control-plane-connect-v1\nnonce={}\ndevice_id={}\nclient_id={}\nrole={}\nscopes={}\nsigned_at_ms={}",
        device.nonce,
        device.device_id,
        request.client.id,
        request.role.as_str(),
        scopes,
        device.signed_at_ms
    )
    .into_bytes()
}

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

    let now_ms = current_time_ms();
    if device.signed_at_ms < challenge.issued_at_ms
        || device.signed_at_ms
            > challenge
                .expires_at_ms
                .saturating_add(CONTROL_PLANE_CHALLENGE_MAX_FUTURE_SKEW_MS)
        || device.signed_at_ms > now_ms.saturating_add(CONTROL_PLANE_CHALLENGE_MAX_FUTURE_SKEW_MS)
    {
        return Err(Box::new(error_response(
            StatusCode::UNAUTHORIZED,
            format!(
                "control-plane device signature timestamp is outside the challenge window for `{}`",
                device.device_id
            ),
        )));
    }

    let public_key_bytes = base64::engine::general_purpose::STANDARD
        .decode(device.public_key.as_bytes())
        .map_err(|error| {
            Box::new(error_response(
                StatusCode::BAD_REQUEST,
                format!("invalid control-plane device public_key encoding: {error}"),
            ))
        })?;
    let signature_bytes = base64::engine::general_purpose::STANDARD
        .decode(device.signature.as_bytes())
        .map_err(|error| {
            Box::new(error_response(
                StatusCode::BAD_REQUEST,
                format!("invalid control-plane device signature encoding: {error}"),
            ))
        })?;

    let public_key_array: [u8; 32] = public_key_bytes.try_into().map_err(|_error| {
        Box::new(error_response(
            StatusCode::BAD_REQUEST,
            "control-plane device public_key must decode to 32 bytes",
        ))
    })?;
    let verifying_key = VerifyingKey::from_bytes(&public_key_array).map_err(|error| {
        Box::new(error_response(
            StatusCode::BAD_REQUEST,
            format!("invalid control-plane device public_key: {error}"),
        ))
    })?;
    let signature = Signature::from_slice(&signature_bytes).map_err(|error| {
        Box::new(error_response(
            StatusCode::BAD_REQUEST,
            format!("invalid control-plane device signature bytes: {error}"),
        ))
    })?;
    let message = control_plane_device_signature_message(request, device);
    verifying_key.verify(&message, &signature).map_err(|error| {
        Box::new(error_response(
            StatusCode::UNAUTHORIZED,
            format!("control-plane device signature verification failed: {error}"),
        ))
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

pub(super) fn pairing_required_response(
    request: &mvp::control_plane::ControlPlanePairingRequestRecord,
) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ControlPlaneConnectErrorResponse {
            code: ControlPlaneConnectErrorCode::PairingRequired,
            error: format!(
                "device `{}` requires operator pairing approval before connect can complete",
                request.device_id
            ),
            pairing_request_id: Some(request.pairing_request_id.clone()),
        }),
    )
        .into_response()
}

pub(super) fn device_token_error_response(
    code: ControlPlaneConnectErrorCode,
    error: impl Into<String>,
) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(ControlPlaneConnectErrorResponse {
            code,
            error: error.into(),
            pairing_request_id: None,
        }),
    )
        .into_response()
}

pub(super) fn connect_error_response(
    status: StatusCode,
    code: ControlPlaneConnectErrorCode,
    error: impl Into<String>,
) -> Response {
    (
        status,
        Json(ControlPlaneConnectErrorResponse {
            code,
            error: error.into(),
            pairing_request_id: None,
        }),
    )
        .into_response()
}

pub(super) fn error_response(status: StatusCode, error: impl Into<String>) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": error.into(),
        })),
    )
        .into_response()
}

pub(super) fn connection_principal_from_connect(
    request: &ControlPlaneConnectRequest,
    connection_id: String,
    granted_scopes: &std::collections::BTreeSet<ControlPlaneScope>,
) -> mvp::control_plane::ControlPlaneConnectionPrincipal {
    mvp::control_plane::ControlPlaneConnectionPrincipal {
        connection_id,
        client_id: request.client.id.clone(),
        role: request.role.as_str().to_owned(),
        scopes: granted_scopes
            .iter()
            .map(|scope| scope.as_str().to_owned())
            .collect::<std::collections::BTreeSet<_>>(),
        device_id: request
            .device
            .as_ref()
            .map(|device| device.device_id.clone()),
    }
}

pub(super) fn granted_connect_scopes(
    state: &ControlPlaneHttpState,
    request: &ControlPlaneConnectRequest,
) -> std::collections::BTreeSet<ControlPlaneScope> {
    let remote_bootstrap = state.exposure_policy.requires_remote_auth() && request.device.is_none();
    if !remote_bootstrap {
        return request.scopes.clone();
    }

    let allowed_scopes = std::collections::BTreeSet::from(CONTROL_PLANE_REMOTE_BOOTSTRAP_SCOPES);
    let requested_scopes = request.scopes.clone();
    requested_scopes
        .intersection(&allowed_scopes)
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
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

pub(super) fn authorize_control_plane_request(
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
        .map_err(|error| Box::new(error_response(StatusCode::FORBIDDEN, error)))?;

    Ok(lease)
}
