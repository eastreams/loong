use super::*;

pub(super) async fn require_local_token(
    State(state): State<Arc<WebApiState>>,
    request: Request,
    next: Next,
) -> Result<Response, WebApiError> {
    if request.method() == Method::OPTIONS {
        return Ok(next.run(request).await);
    }

    let token = extract_request_token(request.headers());
    if request_is_authenticated(state.as_ref(), token.as_deref()) {
        return Ok(next.run(request).await);
    }

    if state.web_install_mode == "same_origin_static" {
        return Err(WebApiError::unauthorized(
            "Local Web session required. Open the same-origin Web surface again to refresh the session.",
        ));
    }

    Err(WebApiError::unauthorized(format!(
        "Local API token required. Read it from `{}` or set `{WEB_API_TOKEN_ENV}`.",
        state.local_token_path.display()
    )))
}

pub(super) async fn require_same_origin_write_origin(
    State(state): State<Arc<WebApiState>>,
    request: Request,
    next: Next,
) -> Result<Response, WebApiError> {
    if request.method() == Method::OPTIONS || request.method() == Method::GET {
        return Ok(next.run(request).await);
    }

    if state.web_install_mode != "same_origin_static" {
        return Ok(next.run(request).await);
    }

    if extract_allowed_local_origin(request.headers()).is_none() {
        return Err(WebApiError::forbidden(
            "same-origin Web writes require a trusted local loopback origin",
        ));
    }

    Ok(next.run(request).await)
}

pub(super) fn extract_request_token(headers: &HeaderMap) -> Option<String> {
    if let Some(raw) = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(raw.to_owned());
    }

    headers
        .get("x-loongclaw-token")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            headers
                .get(COOKIE)
                .and_then(|value| value.to_str().ok())
                .and_then(extract_any_web_cookie_token)
        })
}

fn extract_any_web_cookie_token(raw_cookie: &str) -> Option<String> {
    raw_cookie
        .split(';')
        .map(str::trim)
        .filter_map(|segment| segment.split_once('='))
        .find_map(|(name, value)| {
            matches!(name.trim(), WEB_API_PAIRING_COOKIE | WEB_API_SESSION_COOKIE)
                .then(|| value.trim())
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
}

pub(super) fn request_is_authenticated(state: &WebApiState, token: Option<&str>) -> bool {
    token == Some(state.local_token.as_str())
}

pub(super) fn extract_allowed_local_origin(headers: &HeaderMap) -> Option<String> {
    headers
        .get(ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| is_allowed_local_origin(value))
        .map(ToOwned::to_owned)
}

fn is_allowed_local_origin(origin: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(origin) else {
        return false;
    };

    matches!(url.scheme(), "http" | "https")
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"))
}

pub(super) fn build_pairing_cookie(token: &str) -> Result<HeaderValue, WebApiError> {
    HeaderValue::from_str(&format!(
        "{WEB_API_PAIRING_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000"
    ))
    .map_err(|error| WebApiError::internal(format!("build pairing cookie failed: {error}")))
}

pub(super) fn build_clear_pairing_cookie() -> Result<HeaderValue, WebApiError> {
    HeaderValue::from_str(&format!(
        "{WEB_API_PAIRING_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"
    ))
    .map_err(|error| WebApiError::internal(format!("build pairing cookie clear failed: {error}")))
}

pub(super) fn build_same_origin_session_cookie(token: &str) -> Result<HeaderValue, WebApiError> {
    HeaderValue::from_str(&format!(
        "{WEB_API_SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000"
    ))
    .map_err(|error| WebApiError::internal(format!("build same-origin session cookie failed: {error}")))
}

pub(super) fn build_clear_same_origin_session_cookie() -> Result<HeaderValue, WebApiError> {
    HeaderValue::from_str(&format!(
        "{WEB_API_SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"
    ))
    .map_err(|error| {
        WebApiError::internal(format!(
            "build same-origin session cookie clear failed: {error}"
        ))
    })
}
