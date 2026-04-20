use super::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OnboardStatusPayload {
    runtime_online: bool,
    token_required: bool,
    token_paired: bool,
    config_exists: bool,
    config_loadable: bool,
    provider_configured: bool,
    provider_reachable: bool,
    active_provider: Option<String>,
    active_model: String,
    provider_base_url: String,
    provider_endpoint: String,
    provider_endpoint_explicit: bool,
    api_key_configured: bool,
    personality: String,
    memory_profile: String,
    sliding_window: usize,
    prompt_addendum: String,
    config_path: String,
    blocking_stage: &'static str,
    next_action: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OnboardProviderWriteRequest {
    kind: String,
    model: String,
    base_url_or_endpoint: String,
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OnboardPreferencesWriteRequest {
    personality: String,
    memory_profile: String,
    sliding_window: Option<usize>,
    prompt_addendum: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OnboardValidationPayload {
    passed: bool,
    endpoint_status: &'static str,
    endpoint_status_code: Option<u16>,
    credential_status: &'static str,
    credential_status_code: Option<u16>,
    status: OnboardStatusPayload,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OnboardPairingPayload {
    paired: bool,
    mode: &'static str,
    status: OnboardStatusPayload,
}

#[derive(Debug, Clone, Copy)]
struct ProviderValidationResult {
    endpoint_status: &'static str,
    endpoint_status_code: Option<u16>,
    credential_status: &'static str,
    credential_status_code: Option<u16>,
}

impl ProviderValidationResult {
    fn passed(self) -> bool {
        self.endpoint_status == "reachable" && self.credential_status == "validated"
    }
}

pub(super) async fn onboard_status(
    State(state): State<Arc<WebApiState>>,
    headers: HeaderMap,
) -> Json<ApiEnvelope<OnboardStatusPayload>> {
    let token_paired =
        request_is_authenticated(state.as_ref(), extract_request_token(&headers).as_deref());
    let payload = build_onboard_status_payload(state.as_ref(), token_paired).await;

    Json(ApiEnvelope {
        ok: true,
        data: payload,
    })
}

pub(super) async fn onboard_provider(
    State(state): State<Arc<WebApiState>>,
    Json(request): Json<OnboardProviderWriteRequest>,
) -> Result<Json<ApiEnvelope<OnboardStatusPayload>>, WebApiError> {
    let config_path = resolve_web_config_path(state.as_ref());
    let mut config = load_or_default_web_config(state.as_ref())?;
    apply_provider_request_to_config(&mut config, &request)?;
    let path_string = config_path.display().to_string();
    mvp::config::write(Some(path_string.as_str()), &config, true).map_err(WebApiError::internal)?;

    record_debug_operation(
        &state,
        "provider_apply",
        format!(
            "{} provider config saved",
            format_timestamp(OffsetDateTime::now_utc().unix_timestamp())
        ),
        vec![
            format!("provider.kind={}", request.kind.trim()),
            format!("provider.model={}", request.model.trim()),
            format!("provider.route={}", request.base_url_or_endpoint.trim()),
        ],
    );

    let payload = build_onboard_status_payload(state.as_ref(), true).await;
    Ok(Json(ApiEnvelope {
        ok: true,
        data: payload,
    }))
}

pub(super) async fn onboard_provider_apply(
    State(state): State<Arc<WebApiState>>,
    Json(request): Json<OnboardProviderWriteRequest>,
) -> Result<Json<ApiEnvelope<OnboardValidationPayload>>, WebApiError> {
    let config_path = resolve_web_config_path(state.as_ref());
    let current_config = load_or_default_web_config(state.as_ref())?;
    let mut candidate_config = current_config.clone();
    apply_provider_request_to_config(&mut candidate_config, &request)?;

    let validation = validate_provider_config(&candidate_config.provider).await;
    if validation.passed() {
        let path_string = config_path.display().to_string();
        mvp::config::write(Some(path_string.as_str()), &candidate_config, true)
            .map_err(WebApiError::internal)?;
    }

    let mut status = build_onboard_status_payload(state.as_ref(), true).await;
    if validation.passed() {
        status.provider_reachable = true;
        status.blocking_stage = "ready";
        status.next_action = "open_chat";
    }

    record_debug_operation(
        &state,
        "provider_apply",
        format!(
            "{} provider apply {}",
            format_timestamp(OffsetDateTime::now_utc().unix_timestamp()),
            if validation.passed() {
                "passed"
            } else {
                "failed"
            }
        ),
        vec![
            format!("provider.kind={}", request.kind.trim()),
            format!("provider.model={}", request.model.trim()),
            format!("provider.route={}", request.base_url_or_endpoint.trim()),
            format!("endpoint_status={}", validation.endpoint_status),
            format!("credential_status={}", validation.credential_status),
        ],
    );

    Ok(Json(ApiEnvelope {
        ok: true,
        data: OnboardValidationPayload {
            passed: validation.passed(),
            endpoint_status: validation.endpoint_status,
            endpoint_status_code: validation.endpoint_status_code,
            credential_status: validation.credential_status,
            credential_status_code: validation.credential_status_code,
            status,
        },
    }))
}

fn route_matches_existing_provider_route(
    route: &str,
    existing_provider: &mvp::config::ProviderConfig,
) -> bool {
    let normalized = route.trim();
    if normalized.is_empty() {
        return true;
    }

    normalized == existing_provider.endpoint()
        || normalized == existing_provider.resolved_base_url()
        || normalized == existing_provider.base_url.trim()
        || existing_provider
            .endpoint
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| normalized == value)
}

fn load_or_default_web_config(
    state: &WebApiState,
) -> Result<mvp::config::LoongConfig, WebApiError> {
    let config_path = resolve_web_config_path(state);
    if config_path.is_file() {
        let (_, loaded) = mvp::config::load(state.config_path.as_deref()).map_err(|error| {
            WebApiError::bad_request(format!("local config could not be loaded: {error}"))
        })?;
        Ok(loaded)
    } else {
        Ok(mvp::config::LoongConfig::default())
    }
}

fn apply_provider_request_to_config(
    config: &mut mvp::config::LoongConfig,
    request: &OnboardProviderWriteRequest,
) -> Result<(), WebApiError> {
    let kind = mvp::config::parse_provider_kind_id(request.kind.as_str()).ok_or_else(|| {
        WebApiError::bad_request(format!("unknown provider kind `{}`", request.kind.trim()))
    })?;
    let model = request.model.trim();
    if model.is_empty() {
        return Err(WebApiError::bad_request("model is required"));
    }

    let existing_provider = config.provider.clone();
    let kind_changed = existing_provider.kind != kind;
    let mut provider = existing_provider.clone();
    provider.set_kind(kind);
    provider.model = model.to_owned();

    let route = request.base_url_or_endpoint.trim();
    let should_reset_route_to_kind_default = route.is_empty()
        || (kind_changed && route_matches_existing_provider_route(route, &existing_provider));

    if should_reset_route_to_kind_default {
        provider.set_base_url(kind.profile().base_url.to_owned());
        provider.set_chat_completions_path(kind.profile().chat_completions_path.to_owned());
        provider.set_endpoint(None);
        provider.set_models_endpoint(None);
    } else if looks_like_provider_endpoint(route) {
        provider.set_base_url(kind.profile().base_url.to_owned());
        provider.set_chat_completions_path(kind.profile().chat_completions_path.to_owned());
        provider.set_endpoint(Some(route.to_owned()));
        provider.set_models_endpoint(None);
    } else {
        provider.set_base_url(route.to_owned());
        provider.set_chat_completions_path(kind.profile().chat_completions_path.to_owned());
        provider.set_endpoint(None);
        provider.set_models_endpoint(None);
    }

    if let Some(hint) = provider.kind_route_mismatch_hint() {
        return Err(WebApiError::bad_request(hint));
    }

    if let Some(api_key) = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        provider.api_key = Some(loong_contracts::SecretRef::Inline(api_key.to_owned()));
    } else if kind_changed {
        provider.api_key = None;
        provider.set_api_key_env(kind.default_api_key_env().map(str::to_owned));
        provider.oauth_access_token = None;
        provider
            .set_oauth_access_token_env(kind.default_oauth_access_token_env().map(str::to_owned));
    }

    let profile_id = provider.inferred_profile_id();
    config.set_active_provider_profile(
        profile_id,
        mvp::config::ProviderProfileConfig::from_provider(provider),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn apply_provider_request_rejects_volcengine_coding_route_on_standard_kind() {
        let mut config = mvp::config::LoongConfig::default();
        let request = OnboardProviderWriteRequest {
            kind: "volcengine".to_owned(),
            model: "ark-code-latest".to_owned(),
            base_url_or_endpoint: "https://ark.cn-beijing.volces.com/api/coding/v3".to_owned(),
            api_key: None,
        };

        let error = apply_provider_request_to_config(&mut config, &request)
            .expect_err("volcengine should reject coding-plan routes");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert!(
            error
                .message
                .contains("switch to `kind = \"volcengine_coding\"`"),
            "expected a clear kind-switch hint, got: {}",
            error.message
        );
    }

    #[test]
    fn apply_provider_request_accepts_volcengine_coding_route_on_coding_kind() {
        let mut config = mvp::config::LoongConfig::default();
        let request = OnboardProviderWriteRequest {
            kind: "volcengine_coding".to_owned(),
            model: "ark-code-latest".to_owned(),
            base_url_or_endpoint: "https://ark.cn-beijing.volces.com/api/coding/v3".to_owned(),
            api_key: None,
        };

        apply_provider_request_to_config(&mut config, &request)
            .expect("volcengine_coding should accept the coding-plan route");

        assert_eq!(
            config.provider.endpoint(),
            "https://ark.cn-beijing.volces.com/api/coding/v3/chat/completions"
        );
    }

    #[test]
    fn provider_validation_result_does_not_treat_request_rejected_as_passed() {
        let validation = ProviderValidationResult {
            endpoint_status: "reachable",
            endpoint_status_code: Some(400),
            credential_status: "request_rejected",
            credential_status_code: Some(400),
        };

        assert!(
            !validation.passed(),
            "request_rejected should block Web entry and provider apply"
        );
    }
}

pub(super) async fn onboard_preferences(
    State(state): State<Arc<WebApiState>>,
    Json(request): Json<OnboardPreferencesWriteRequest>,
) -> Result<Json<ApiEnvelope<OnboardStatusPayload>>, WebApiError> {
    let personality = crate::onboard_cli::parse_prompt_personality(request.personality.as_str())
        .ok_or_else(|| {
            WebApiError::bad_request(format!(
                "unknown personality `{}`",
                request.personality.trim()
            ))
        })?;
    let memory_profile = crate::onboard_cli::parse_memory_profile(request.memory_profile.as_str())
        .ok_or_else(|| {
            WebApiError::bad_request(format!(
                "unknown memory profile `{}`",
                request.memory_profile.trim()
            ))
        })?;

    let config_path = resolve_web_config_path(state.as_ref());
    let config_exists = config_path.is_file();
    let mut config = if config_exists {
        let (_, loaded) = mvp::config::load(state.config_path.as_deref()).map_err(|error| {
            WebApiError::bad_request(format!("local config could not be loaded: {error}"))
        })?;
        loaded
    } else {
        mvp::config::LoongConfig::default()
    };

    config.cli.personality = Some(personality);
    config.memory.profile = memory_profile;
    if let Some(sliding_window) = request.sliding_window {
        config.memory.sliding_window = validate_memory_sliding_window(sliding_window)?;
    }
    config.cli.system_prompt_addendum = request
        .prompt_addendum
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let path_string = config_path.display().to_string();
    mvp::config::write(Some(path_string.as_str()), &config, true).map_err(WebApiError::internal)?;

    record_debug_operation(
        &state,
        "preferences_apply",
        format!(
            "{} preferences updated",
            format_timestamp(OffsetDateTime::now_utc().unix_timestamp())
        ),
        vec![
            format!("personality={}", request.personality.trim()),
            format!("memory_profile={}", request.memory_profile.trim()),
            format!(
                "sliding_window={}",
                request
                    .sliding_window
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| config.memory.sliding_window.to_string())
            ),
            format!(
                "prompt_addendum={}",
                request
                    .prompt_addendum
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|_| "configured")
                    .unwrap_or("empty")
            ),
        ],
    );

    let payload = build_onboard_status_payload(state.as_ref(), true).await;
    Ok(Json(ApiEnvelope {
        ok: true,
        data: payload,
    }))
}

pub(super) async fn onboard_pairing_auto(
    State(state): State<Arc<WebApiState>>,
    headers: HeaderMap,
) -> Result<Response, WebApiError> {
    if extract_allowed_local_origin(&headers).is_none() {
        return Err(WebApiError::forbidden(
            "automatic pairing is limited to trusted local loopback origins",
        ));
    }

    let payload = OnboardPairingPayload {
        paired: true,
        mode: "cookie",
        status: build_onboard_status_payload(state.as_ref(), true).await,
    };
    record_debug_operation(
        &state,
        "token_pairing",
        format!(
            "{} token pairing auto",
            format_timestamp(OffsetDateTime::now_utc().unix_timestamp())
        ),
        vec![
            "pairing.mode=cookie".to_owned(),
            "pairing.result=paired".to_owned(),
        ],
    );
    let mut response = Json(ApiEnvelope {
        ok: true,
        data: payload,
    })
    .into_response();
    response.headers_mut().append(
        SET_COOKIE,
        build_pairing_cookie(state.local_token.as_str())?,
    );
    Ok(response)
}

pub(super) async fn onboard_pairing_clear() -> Result<Response, WebApiError> {
    let mut response = Json(ApiEnvelope {
        ok: true,
        data: Value::Object(Default::default()),
    })
    .into_response();
    response
        .headers_mut()
        .append(SET_COOKIE, build_clear_pairing_cookie()?);
    response
        .headers_mut()
        .append(SET_COOKIE, build_clear_same_origin_session_cookie()?);
    Ok(response)
}

pub(super) async fn onboard_validate(
    State(state): State<Arc<WebApiState>>,
) -> Result<Json<ApiEnvelope<OnboardValidationPayload>>, WebApiError> {
    let snapshot = load_web_snapshot(state.as_ref())?;
    if !provider_is_configured(&snapshot.config) {
        return Err(WebApiError::bad_request(
            "provider is not configured enough to validate yet",
        ));
    }

    let validation = validate_provider_config(&snapshot.config.provider).await;
    let mut status = build_onboard_status_payload(state.as_ref(), true).await;
    status.provider_reachable = validation.passed();
    if validation.passed() {
        status.blocking_stage = "ready";
        status.next_action = "open_chat";
    } else {
        status.blocking_stage = "provider_unreachable";
        status.next_action = "validate_provider_route";
    }

    record_debug_operation(
        &state,
        "provider_validate",
        format!(
            "{} provider validate {}",
            format_timestamp(OffsetDateTime::now_utc().unix_timestamp()),
            if validation.passed() {
                "passed"
            } else {
                "failed"
            }
        ),
        vec![
            format!("provider={}", snapshot.config.provider.kind.profile().id),
            format!("model={}", snapshot.config.provider.model),
            format!("endpoint_status={}", validation.endpoint_status),
            format!("credential_status={}", validation.credential_status),
        ],
    );

    Ok(Json(ApiEnvelope {
        ok: true,
        data: OnboardValidationPayload {
            passed: validation.passed(),
            endpoint_status: validation.endpoint_status,
            endpoint_status_code: validation.endpoint_status_code,
            credential_status: validation.credential_status,
            credential_status_code: validation.credential_status_code,
            status,
        },
    }))
}

fn provider_is_configured(config: &mvp::config::LoongConfig) -> bool {
    let provider = &config.provider;
    let item = provider_item_from_parts("active".to_owned(), provider, true, true);
    !provider.model.trim().is_empty()
        && !provider.endpoint().trim().is_empty()
        && item.api_key_configured
}

async fn build_onboard_status_payload(
    state: &WebApiState,
    token_paired: bool,
) -> OnboardStatusPayload {
    let config_path = resolve_web_config_path(state);
    let config_exists = config_path.is_file();
    let config_path_display = config_path.display().to_string();

    let mut payload = OnboardStatusPayload {
        runtime_online: true,
        token_required: state.web_install_mode != "same_origin_static",
        token_paired,
        config_exists,
        config_loadable: false,
        provider_configured: false,
        provider_reachable: false,
        active_provider: None,
        active_model: String::new(),
        provider_base_url: String::new(),
        provider_endpoint: String::new(),
        provider_endpoint_explicit: false,
        api_key_configured: false,
        personality: "calm_engineering".to_owned(),
        memory_profile: "window_only".to_owned(),
        sliding_window: mvp::config::MemoryConfig::default().sliding_window,
        prompt_addendum: String::new(),
        config_path: config_path_display,
        blocking_stage: if state.web_install_mode == "same_origin_static" {
            "ready"
        } else {
            "token_pairing"
        },
        next_action: if state.web_install_mode == "same_origin_static" {
            "open_chat"
        } else {
            "enter_local_token"
        },
    };

    match load_web_snapshot(state) {
        Ok(snapshot) => {
            payload.config_loadable = true;
            payload.active_provider = snapshot.config.active_provider_id().map(str::to_owned);
            payload.active_model = snapshot.config.provider.model.clone();
            payload.provider_base_url = snapshot.config.provider.resolved_base_url();
            payload.provider_endpoint = snapshot.config.provider.endpoint();
            payload.provider_endpoint_explicit = snapshot.config.provider.endpoint_explicit
                && snapshot
                    .config
                    .provider
                    .endpoint
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|value: &str| !value.is_empty());
            payload.provider_configured = provider_is_configured(&snapshot.config);
            payload.personality = crate::onboard_cli::prompt_personality_id(
                snapshot.config.cli.resolved_personality(),
            )
            .to_owned();
            payload.memory_profile =
                crate::onboard_cli::memory_profile_id(snapshot.config.memory.resolved_profile())
                    .to_owned();
            payload.sliding_window = snapshot.config.memory.sliding_window;
            payload.prompt_addendum = snapshot
                .config
                .cli
                .system_prompt_addendum
                .clone()
                .unwrap_or_default();
            payload.api_key_configured = provider_item_from_parts(
                "active".to_owned(),
                &snapshot.config.provider,
                true,
                true,
            )
            .api_key_configured;

            if payload.provider_configured {
                payload.provider_reachable =
                    probe_provider_reachability(&snapshot.config.provider).await;
            }
        }
        Err(_) => {
            payload.config_loadable = false;
        }
    }

    let (blocking_stage, next_action) = if !payload.token_paired {
        if state.web_install_mode == "same_origin_static" {
            ("session_refresh", "refresh_local_session")
        } else {
            ("token_pairing", "enter_local_token")
        }
    } else if !payload.config_exists {
        ("missing_config", "create_local_config")
    } else if !payload.config_loadable {
        ("config_invalid", "fix_local_config")
    } else if !payload.provider_configured {
        ("provider_setup", "configure_provider")
    } else if !payload.provider_reachable {
        ("provider_unreachable", "validate_provider_route")
    } else {
        ("ready", "open_chat")
    };
    payload.blocking_stage = blocking_stage;
    payload.next_action = next_action;
    payload
}

fn validate_memory_sliding_window(sliding_window: usize) -> Result<usize, WebApiError> {
    if !(1..=128).contains(&sliding_window) {
        return Err(WebApiError::bad_request(format!(
            "memory sliding window must be between 1 and 128 turns, got `{sliding_window}`"
        )));
    }

    Ok(sliding_window)
}

fn looks_like_provider_endpoint(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    (normalized.starts_with("http://") || normalized.starts_with("https://"))
        && (normalized.contains("/chat/completions")
            || normalized.ends_with("/completions")
            || normalized.ends_with("/responses"))
}

fn build_provider_probe_headers(
    provider: &mvp::config::ProviderConfig,
) -> Result<reqwest::header::HeaderMap, WebApiError> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    for (name, value) in provider.kind.default_headers() {
        let header_name = reqwest::header::HeaderName::from_static(name);
        let header_value = HeaderValue::from_str(value).map_err(|error| {
            WebApiError::internal(format!("build provider probe headers failed: {error}"))
        })?;
        headers.insert(header_name, header_value);
    }

    match provider.kind.auth_scheme() {
        mvp::config::ProviderAuthScheme::Bearer => {
            let Some(value) = provider.authorization_header() else {
                return Ok(headers);
            };
            let header_value = HeaderValue::from_str(value.as_str()).map_err(|error| {
                WebApiError::internal(format!("build provider probe headers failed: {error}"))
            })?;
            headers.insert(AUTHORIZATION, header_value);
        }
        mvp::config::ProviderAuthScheme::XApiKey => {
            let Some(secret) = provider.resolved_auth_secret() else {
                return Ok(headers);
            };
            let header_value = HeaderValue::from_str(secret.as_str()).map_err(|error| {
                WebApiError::internal(format!("build provider probe headers failed: {error}"))
            })?;
            headers.insert(
                reqwest::header::HeaderName::from_static("x-api-key"),
                header_value,
            );
        }
        mvp::config::ProviderAuthScheme::XGoogApiKey => {
            let Some(secret) = provider.resolved_auth_secret() else {
                return Ok(headers);
            };
            let header_value = HeaderValue::from_str(secret.as_str()).map_err(|error| {
                WebApiError::internal(format!("build provider probe headers failed: {error}"))
            })?;
            headers.insert(
                reqwest::header::HeaderName::from_static("x-goog-api-key"),
                header_value,
            );
        }
    }

    Ok(headers)
}

fn provider_probe_model(provider: &mvp::config::ProviderConfig) -> String {
    if let Some(model) = provider.explicit_model() {
        return model;
    }

    match provider.model_catalog_probe_recovery() {
        mvp::config::ModelCatalogProbeRecovery::ExplicitModel(model) => model,
        mvp::config::ModelCatalogProbeRecovery::ConfiguredPreferredModels(models) => models
            .into_iter()
            .next()
            .unwrap_or_else(|| provider.configured_model_value()),
        mvp::config::ModelCatalogProbeRecovery::RequiresExplicitModel {
            recommended_onboarding_model: Some(model),
        } => model.to_owned(),
        mvp::config::ModelCatalogProbeRecovery::RequiresExplicitModel {
            recommended_onboarding_model: None,
        } => provider.configured_model_value(),
    }
}

// Keep onboarding validation lightweight: prove the route is reachable and the
// provider accepts an authenticated probe. For first-run onboarding, a provider-
// specific request-shape rejection is still good enough to let users proceed,
// because it proves the endpoint and credentials are basically wired up.
async fn validate_provider_config(
    provider: &mvp::config::ProviderConfig,
) -> ProviderValidationResult {
    let endpoint = provider.endpoint();
    let (endpoint_status, endpoint_status_code) = probe_provider_endpoint(endpoint.as_str()).await;
    if endpoint_status != "reachable" {
        return ProviderValidationResult {
            endpoint_status,
            endpoint_status_code,
            credential_status: "transport_failure",
            credential_status_code: None,
        };
    }

    if provider.resolved_auth_secret().is_none() {
        return ProviderValidationResult {
            endpoint_status,
            endpoint_status_code,
            credential_status: "missing_credentials",
            credential_status_code: None,
        };
    }

    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
    else {
        return ProviderValidationResult {
            endpoint_status,
            endpoint_status_code,
            credential_status: "transport_failure",
            credential_status_code: None,
        };
    };

    let headers = match build_provider_probe_headers(provider) {
        Ok(headers) => headers,
        Err(_) => {
            return ProviderValidationResult {
                endpoint_status,
                endpoint_status_code,
                credential_status: "transport_failure",
                credential_status_code: None,
            };
        }
    };

    let credential_result = match provider.kind.protocol_family() {
        mvp::config::ProviderProtocolFamily::OpenAiChatCompletions => {
            let request = json!({
                "model": provider_probe_model(provider),
                "messages": [
                    {
                        "role": "user",
                        "content": "ping"
                    }
                ],
                "max_tokens": 1,
                "temperature": 0,
                "stream": false
            });

            match client
                .post(endpoint.as_str())
                .headers(headers)
                .json(&request)
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    ("validated", Some(response.status().as_u16()))
                }
                Ok(response) if matches!(response.status().as_u16(), 401 | 403) => {
                    ("auth_rejected", Some(response.status().as_u16()))
                }
                Ok(response) if response.status().is_server_error() => {
                    ("upstream_unavailable", Some(response.status().as_u16()))
                }
                Ok(response) => ("request_rejected", Some(response.status().as_u16())),
                Err(_) => ("transport_failure", None),
            }
        }
        mvp::config::ProviderProtocolFamily::AnthropicMessages
        | mvp::config::ProviderProtocolFamily::BedrockConverse => {
            match client.head(endpoint.as_str()).headers(headers).send().await {
                Ok(response) if matches!(response.status().as_u16(), 401 | 403) => {
                    ("auth_rejected", Some(response.status().as_u16()))
                }
                Ok(response)
                    if response.status().is_success() || response.status().as_u16() == 405 =>
                {
                    ("validated", Some(response.status().as_u16()))
                }
                Ok(response) if response.status().is_server_error() => {
                    ("upstream_unavailable", Some(response.status().as_u16()))
                }
                Ok(response) => ("request_rejected", Some(response.status().as_u16())),
                Err(_) => ("transport_failure", None),
            }
        }
    };

    ProviderValidationResult {
        endpoint_status,
        endpoint_status_code,
        credential_status: credential_result.0,
        credential_status_code: credential_result.1,
    }
}

async fn probe_provider_reachability(provider: &mvp::config::ProviderConfig) -> bool {
    let validation = validate_provider_headers_only(provider).await;
    validation.endpoint_status == "reachable"
        && !matches!(
            validation.credential_status,
            "transport_failure" | "missing_credentials" | "auth_rejected"
        )
}

async fn validate_provider_headers_only(
    provider: &mvp::config::ProviderConfig,
) -> ProviderValidationResult {
    let endpoint = provider.endpoint();
    let (endpoint_status, endpoint_status_code) = probe_provider_endpoint(endpoint.as_str()).await;
    if endpoint_status != "reachable" {
        return ProviderValidationResult {
            endpoint_status,
            endpoint_status_code,
            credential_status: "transport_failure",
            credential_status_code: None,
        };
    }

    if provider.resolved_auth_secret().is_none() {
        return ProviderValidationResult {
            endpoint_status,
            endpoint_status_code,
            credential_status: "missing_credentials",
            credential_status_code: None,
        };
    }

    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    else {
        return ProviderValidationResult {
            endpoint_status,
            endpoint_status_code,
            credential_status: "transport_failure",
            credential_status_code: None,
        };
    };

    let headers = match build_provider_probe_headers(provider) {
        Ok(headers) => headers,
        Err(_) => {
            return ProviderValidationResult {
                endpoint_status,
                endpoint_status_code,
                credential_status: "transport_failure",
                credential_status_code: None,
            };
        }
    };

    let credential_result = match client.head(endpoint.as_str()).headers(headers).send().await {
        Ok(response) if matches!(response.status().as_u16(), 401 | 403) => {
            ("auth_rejected", Some(response.status().as_u16()))
        }
        Ok(response) if response.status().is_server_error() => {
            ("upstream_unavailable", Some(response.status().as_u16()))
        }
        Ok(response) => ("validated", Some(response.status().as_u16())),
        Err(_) => ("transport_failure", None),
    };

    ProviderValidationResult {
        endpoint_status,
        endpoint_status_code,
        credential_status: credential_result.0,
        credential_status_code: credential_result.1,
    }
}
