use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use crate::config::{LoongClawConfig, ProviderConfig};

use super::auth_profile_runtime::{ProviderAuthProfile, auth_profile_supports_scheme};
use super::capability_profile_runtime::ProviderCapabilityProfile;
use super::contracts::{
    ProviderApiError, provider_runtime_contract_for_route, should_disable_tool_schema_for_error,
};
use super::failover::{
    ModelRequestError, ProviderFailoverReason, ProviderFailoverStage, build_model_request_error,
};
use super::policy;
use super::request_executor::{
    ModelRequestRuntime, StreamingModelRequestRuntime, execute_model_request,
    execute_streaming_turn_request,
};
use super::request_payload_runtime::{
    build_completion_request_body_with_capability, build_turn_request_body_with_capability,
};
use super::shape;
use super::transport_profile_runtime::resolve_provider_request_transport_profile;
use super::transport_trait::ProviderTransport;

#[allow(clippy::too_many_arguments)]
pub(super) async fn request_completion_with_model(
    config: &LoongClawConfig,
    messages: &[Value],
    model: String,
    auto_model_mode: bool,
    auth_profile: ProviderAuthProfile,
    request_policy: &policy::ProviderRequestPolicy,
    client: &reqwest::Client,
    auth_context: &super::transport::RequestAuthContext,
) -> Result<String, ModelRequestError> {
    request_completion_with_provider(
        config,
        &config.provider,
        messages,
        model.as_str(),
        auto_model_mode,
        &auth_profile,
        auth_context,
        request_policy,
        client,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn request_turn_with_model(
    config: &LoongClawConfig,
    session_id: &str,
    turn_id: &str,
    messages: &[Value],
    model: String,
    auto_model_mode: bool,
    tool_definitions: &[Value],
    auth_profile: ProviderAuthProfile,
    request_policy: &policy::ProviderRequestPolicy,
    client: &reqwest::Client,
    auth_context: &super::transport::RequestAuthContext,
) -> Result<crate::conversation::turn_engine::ProviderTurn, ModelRequestError> {
    request_turn_with_provider(
        config,
        &config.provider,
        session_id,
        turn_id,
        messages,
        model.as_str(),
        auto_model_mode,
        tool_definitions,
        &auth_profile,
        auth_context,
        request_policy,
        client,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn request_completion_with_provider(
    base_config: &LoongClawConfig,
    request_provider: &ProviderConfig,
    messages: &[Value],
    model: &str,
    auto_model_mode: bool,
    auth_profile: &ProviderAuthProfile,
    auth_context: &super::transport::RequestAuthContext,
    request_policy: &policy::ProviderRequestPolicy,
    client: &reqwest::Client,
) -> Result<String, ModelRequestError> {
    let transport = super::transport::ReqwestTransport::new(client.clone(), auth_context.clone());
    request_completion_with_provider_transport(
        base_config,
        request_provider,
        messages,
        model,
        auto_model_mode,
        auth_profile,
        auth_context,
        request_policy,
        &transport,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn request_completion_with_provider_transport(
    base_config: &LoongClawConfig,
    request_provider: &ProviderConfig,
    messages: &[Value],
    model: &str,
    auto_model_mode: bool,
    auth_profile: &ProviderAuthProfile,
    auth_context: &super::transport::RequestAuthContext,
    request_policy: &policy::ProviderRequestPolicy,
    transport: &dyn ProviderTransport,
) -> Result<String, ModelRequestError> {
    let mut current_provider = request_provider.clone();
    loop {
        let transport_profile = resolve_request_transport_profile(&current_provider, model)
            .map_err(|error| {
                build_model_request_error(
                    error,
                    auto_model_mode,
                    ProviderFailoverReason::ModelMismatch,
                    ProviderFailoverStage::ModelCandidateRejected,
                    model,
                    1,
                    1,
                    None,
                    None,
                )
            })?;
        let runtime_contract = provider_runtime_contract_for_route(
            &current_provider,
            transport_profile.transport_mode,
            transport_profile.feature_family,
        );
        let capability_profile =
            ProviderCapabilityProfile::from_provider(&current_provider, runtime_contract);
        let request_model = transport_profile.request_model;
        let capability = capability_profile.resolve_for_model(request_model.as_str());
        let request_auth_scheme = transport_profile.auth_scheme;

        ensure_auth_profile_supports_route(
            auth_profile,
            request_auth_scheme,
            request_model.as_str(),
            auto_model_mode,
        )?;

        let request_headers =
            super::transport::build_request_headers_without_provider_auth_for_transport(
                &current_provider,
                transport_profile.default_user_agent,
                transport_profile.default_headers,
            )
            .and_then(|mut headers| {
                super::transport::append_prompt_cache_headers(&mut headers, None, None, messages)?;
                Ok(headers)
            })
            .map_err(|error| {
                build_model_request_error(
                    error,
                    auto_model_mode,
                    ProviderFailoverReason::TransportFailure,
                    ProviderFailoverStage::TransportFailure,
                    request_model.as_str(),
                    1,
                    request_policy.max_attempts,
                    None,
                    None,
                )
            })?;
        let mut request_config = base_config.clone();
        request_config.provider = current_provider.clone();
        let runtime = ModelRequestRuntime {
            provider: &current_provider,
            model: request_model.as_str(),
            runtime_contract,
            capability,
            auto_model_mode,
            auth_profile,
            request_auth_scheme,
            endpoint: transport_profile.endpoint.as_str(),
            headers: &request_headers,
            request_policy,
            transport,
            auth_context,
        };

        match execute_model_request(
            runtime,
            |payload_mode| {
                build_completion_request_body_with_capability(
                    &request_config,
                    messages,
                    request_model.as_str(),
                    payload_mode,
                    runtime_contract,
                    capability,
                )
            },
            shape::extract_message_content,
            "choices[0].message.content",
            |_| false,
        )
        .await
        {
            Err(error)
                if should_retry_with_chat_completions_fallback(
                    &current_provider,
                    transport_profile.transport_mode,
                    &error,
                ) =>
            {
                if let Some(fallback_provider) = current_provider.responses_fallback_provider() {
                    current_provider = fallback_provider;
                    continue;
                }
                return Err(error);
            }
            result => return result,
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn request_turn_with_provider(
    base_config: &LoongClawConfig,
    request_provider: &ProviderConfig,
    session_id: &str,
    turn_id: &str,
    messages: &[Value],
    model: &str,
    auto_model_mode: bool,
    tool_definitions: &[Value],
    auth_profile: &ProviderAuthProfile,
    auth_context: &super::transport::RequestAuthContext,
    request_policy: &policy::ProviderRequestPolicy,
    client: &reqwest::Client,
) -> Result<crate::conversation::turn_engine::ProviderTurn, ModelRequestError> {
    let transport = super::transport::ReqwestTransport::new(client.clone(), auth_context.clone());
    request_turn_with_provider_transport(
        base_config,
        request_provider,
        session_id,
        turn_id,
        messages,
        model,
        auto_model_mode,
        tool_definitions,
        auth_profile,
        auth_context,
        request_policy,
        &transport,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn request_turn_with_provider_transport(
    base_config: &LoongClawConfig,
    request_provider: &ProviderConfig,
    session_id: &str,
    turn_id: &str,
    messages: &[Value],
    model: &str,
    auto_model_mode: bool,
    tool_definitions: &[Value],
    auth_profile: &ProviderAuthProfile,
    auth_context: &super::transport::RequestAuthContext,
    request_policy: &policy::ProviderRequestPolicy,
    transport: &dyn ProviderTransport,
) -> Result<crate::conversation::turn_engine::ProviderTurn, ModelRequestError> {
    let mut current_provider = request_provider.clone();
    loop {
        let transport_profile = resolve_request_transport_profile(&current_provider, model)
            .map_err(|error| {
                build_model_request_error(
                    error,
                    auto_model_mode,
                    ProviderFailoverReason::ModelMismatch,
                    ProviderFailoverStage::ModelCandidateRejected,
                    model,
                    1,
                    1,
                    None,
                    None,
                )
            })?;
        let runtime_contract = provider_runtime_contract_for_route(
            &current_provider,
            transport_profile.transport_mode,
            transport_profile.feature_family,
        );
        let capability_profile =
            ProviderCapabilityProfile::from_provider(&current_provider, runtime_contract);
        let request_model = transport_profile.request_model;
        let capability = capability_profile.resolve_for_model(request_model.as_str());
        let request_auth_scheme = transport_profile.auth_scheme;

        ensure_auth_profile_supports_route(
            auth_profile,
            request_auth_scheme,
            request_model.as_str(),
            auto_model_mode,
        )?;

        let include_tool_schema =
            AtomicBool::new(capability.turn_tool_schema_enabled() && !tool_definitions.is_empty());
        let request_headers =
            super::transport::build_request_headers_without_provider_auth_for_transport(
                &current_provider,
                transport_profile.default_user_agent,
                transport_profile.default_headers,
            )
            .and_then(|mut headers| {
                super::transport::append_prompt_cache_headers(
                    &mut headers,
                    Some(session_id),
                    Some(turn_id),
                    messages,
                )?;
                Ok(headers)
            })
            .map_err(|error| {
                build_model_request_error(
                    error,
                    auto_model_mode,
                    ProviderFailoverReason::TransportFailure,
                    ProviderFailoverStage::TransportFailure,
                    request_model.as_str(),
                    1,
                    request_policy.max_attempts,
                    None,
                    None,
                )
            })?;
        let mut request_config = base_config.clone();
        request_config.provider = current_provider.clone();
        let runtime = ModelRequestRuntime {
            provider: &current_provider,
            model: request_model.as_str(),
            runtime_contract,
            capability,
            auto_model_mode,
            auth_profile,
            request_auth_scheme,
            endpoint: transport_profile.endpoint.as_str(),
            headers: &request_headers,
            request_policy,
            transport,
            auth_context,
        };

        match execute_model_request(
            runtime,
            |payload_mode| {
                build_turn_request_body_with_capability(
                    &request_config,
                    messages,
                    request_model.as_str(),
                    payload_mode,
                    runtime_contract,
                    capability,
                    include_tool_schema.load(Ordering::Relaxed),
                    tool_definitions,
                    false,
                )
            },
            |body| {
                shape::extract_provider_turn_with_scope_and_messages(
                    body,
                    Some(session_id),
                    Some(turn_id),
                    messages,
                )
            },
            "choices[0].message",
            |api_error| {
                if include_tool_schema.load(Ordering::Relaxed)
                    && capability.tool_schema_downgrade_on_unsupported()
                    && should_disable_tool_schema_for_error(api_error, runtime_contract)
                {
                    include_tool_schema.store(false, Ordering::Relaxed);
                    return true;
                }
                false
            },
        )
        .await
        {
            Err(error)
                if should_retry_with_chat_completions_fallback(
                    &current_provider,
                    transport_profile.transport_mode,
                    &error,
                ) =>
            {
                if let Some(fallback_provider) = current_provider.responses_fallback_provider() {
                    current_provider = fallback_provider;
                    continue;
                }
                return Err(error);
            }
            result => return result,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn request_turn_streaming(
    base_config: &LoongClawConfig,
    request_provider: &ProviderConfig,
    session_id: &str,
    turn_id: &str,
    messages: &[Value],
    model: &str,
    auto_model_mode: bool,
    tool_definitions: &[Value],
    auth_profile: &ProviderAuthProfile,
    auth_context: &super::transport::RequestAuthContext,
    request_policy: &policy::ProviderRequestPolicy,
    client: &reqwest::Client,
    on_token: super::request_executor::StreamingTokenCallback,
) -> Result<crate::conversation::turn_engine::ProviderTurn, ModelRequestError> {
    let transport = super::transport::ReqwestTransport::new(client.clone(), auth_context.clone());
    request_turn_streaming_with_transport(
        base_config,
        request_provider,
        session_id,
        turn_id,
        messages,
        model,
        auto_model_mode,
        tool_definitions,
        auth_profile,
        auth_context,
        request_policy,
        &transport,
        on_token,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn request_turn_streaming_with_transport(
    base_config: &LoongClawConfig,
    request_provider: &ProviderConfig,
    session_id: &str,
    turn_id: &str,
    messages: &[Value],
    model: &str,
    auto_model_mode: bool,
    tool_definitions: &[Value],
    auth_profile: &ProviderAuthProfile,
    auth_context: &super::transport::RequestAuthContext,
    request_policy: &policy::ProviderRequestPolicy,
    transport: &dyn ProviderTransport,
    on_token: super::request_executor::StreamingTokenCallback,
) -> Result<crate::conversation::turn_engine::ProviderTurn, ModelRequestError> {
    let mut current_provider = request_provider.clone();
    loop {
        let transport_profile = resolve_request_transport_profile(&current_provider, model)
            .map_err(|error| {
                build_model_request_error(
                    error,
                    auto_model_mode,
                    ProviderFailoverReason::ModelMismatch,
                    ProviderFailoverStage::ModelCandidateRejected,
                    model,
                    1,
                    1,
                    None,
                    None,
                )
            })?;
        let runtime_contract = provider_runtime_contract_for_route(
            &current_provider,
            transport_profile.transport_mode,
            transport_profile.feature_family,
        );
        let capability_profile =
            ProviderCapabilityProfile::from_provider(&current_provider, runtime_contract);
        let request_model = transport_profile.request_model;
        let capability = capability_profile.resolve_for_model(request_model.as_str());
        let request_auth_scheme = transport_profile.auth_scheme;

        ensure_auth_profile_supports_route(
            auth_profile,
            request_auth_scheme,
            request_model.as_str(),
            auto_model_mode,
        )?;

        let include_tool_schema =
            AtomicBool::new(capability.turn_tool_schema_enabled() && !tool_definitions.is_empty());
        let request_headers =
            super::transport::build_request_headers_without_provider_auth_for_transport(
                &current_provider,
                transport_profile.default_user_agent,
                transport_profile.default_headers,
            )
            .and_then(|mut headers| {
                super::transport::append_prompt_cache_headers(
                    &mut headers,
                    Some(session_id),
                    Some(turn_id),
                    messages,
                )?;
                Ok(headers)
            })
            .map_err(|error| {
                build_model_request_error(
                    error,
                    auto_model_mode,
                    ProviderFailoverReason::TransportFailure,
                    ProviderFailoverStage::TransportFailure,
                    request_model.as_str(),
                    1,
                    request_policy.max_attempts,
                    None,
                    None,
                )
            })?;
        let mut request_config = base_config.clone();
        request_config.provider = current_provider.clone();
        let runtime = StreamingModelRequestRuntime {
            provider: &current_provider,
            model: request_model.as_str(),
            runtime_contract,
            capability,
            auto_model_mode,
            auth_profile,
            request_auth_scheme,
            endpoint: transport_profile.endpoint.as_str(),
            headers: &request_headers,
            request_policy,
            transport,
            auth_context,
        };

        match execute_streaming_turn_request(
            runtime,
            |payload_mode| {
                build_turn_request_body_with_capability(
                    &request_config,
                    messages,
                    request_model.as_str(),
                    payload_mode,
                    runtime_contract,
                    capability,
                    include_tool_schema.load(Ordering::Relaxed),
                    tool_definitions,
                    true,
                )
            },
            Some(session_id),
            Some(turn_id),
            messages,
            on_token.clone(),
            |api_error| {
                if include_tool_schema.load(Ordering::Relaxed)
                    && capability.tool_schema_downgrade_on_unsupported()
                    && should_disable_tool_schema_for_error(api_error, runtime_contract)
                {
                    include_tool_schema.store(false, Ordering::Relaxed);
                    return true;
                }
                false
            },
        )
        .await
        {
            Err(error)
                if should_retry_with_chat_completions_fallback(
                    &current_provider,
                    transport_profile.transport_mode,
                    &error,
                ) =>
            {
                if let Some(fallback_provider) = current_provider.responses_fallback_provider() {
                    current_provider = fallback_provider;
                    continue;
                }
                return Err(error);
            }
            result => return result,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn request_turn_streaming_with_model(
    config: &LoongClawConfig,
    session_id: &str,
    turn_id: &str,
    messages: &[Value],
    model: String,
    auto_model_mode: bool,
    tool_definitions: &[Value],
    auth_profile: ProviderAuthProfile,
    request_policy: &policy::ProviderRequestPolicy,
    client: &reqwest::Client,
    auth_context: &super::transport::RequestAuthContext,
    on_token: super::request_executor::StreamingTokenCallback,
) -> Result<crate::conversation::turn_engine::ProviderTurn, ModelRequestError> {
    request_turn_streaming(
        config,
        &config.provider,
        session_id,
        turn_id,
        messages,
        model.as_str(),
        auto_model_mode,
        tool_definitions,
        &auth_profile,
        auth_context,
        request_policy,
        client,
        on_token,
    )
    .await
}

fn resolve_request_transport_profile(
    provider: &ProviderConfig,
    model: &str,
) -> Result<super::transport_profile_runtime::ProviderRequestTransportProfile, String> {
    resolve_provider_request_transport_profile(provider, model)
}

fn should_retry_with_chat_completions_fallback(
    provider: &ProviderConfig,
    transport_mode: super::contracts::ProviderTransportMode,
    error: &ModelRequestError,
) -> bool {
    if transport_mode != super::contracts::ProviderTransportMode::Responses {
        return false;
    }

    let Some(status_code) = error.snapshot.status_code else {
        return false;
    };
    let Some(api_error) = error.api_error.as_ref() else {
        return false;
    };
    should_fallback_responses_to_chat_completions(provider, status_code, api_error)
}

#[allow(clippy::result_large_err)]
fn ensure_auth_profile_supports_route(
    auth_profile: &ProviderAuthProfile,
    request_auth_scheme: crate::config::ProviderAuthScheme,
    request_model: &str,
    auto_model_mode: bool,
) -> Result<(), ModelRequestError> {
    if request_auth_scheme == crate::config::ProviderAuthScheme::Bearer {
        return Ok(());
    }

    if auth_profile_supports_scheme(auth_profile, request_auth_scheme) {
        return Ok(());
    }

    let missing_secret_kind = match request_auth_scheme {
        crate::config::ProviderAuthScheme::Bearer => "bearer",
        crate::config::ProviderAuthScheme::XApiKey => "x-api-key",
        crate::config::ProviderAuthScheme::XGoogApiKey => "x-goog-api-key",
    };

    let message = format!(
        "provider auth profile `{}` cannot satisfy the routed `{}` auth requirement for model `{}`; trying the next available auth profile",
        auth_profile.id, missing_secret_kind, request_model
    );

    let error = build_model_request_error(
        message,
        auto_model_mode,
        ProviderFailoverReason::AuthRejected,
        ProviderFailoverStage::TransportFailure,
        request_model,
        1,
        1,
        None,
        None,
    );

    Err(error)
}

fn should_fallback_responses_to_chat_completions(
    provider: &ProviderConfig,
    status_code: u16,
    error: &ProviderApiError,
) -> bool {
    if provider.responses_fallback_provider().is_none() {
        return false;
    }

    let message = error.message.as_deref().unwrap_or_default();
    if message.is_empty()
        || message.contains("unauthorized")
        || message.contains("forbidden")
        || message.contains("invalid api key")
        || message.contains("rate limit")
        || message.contains("insufficient quota")
    {
        return false;
    }

    let compatibility_status = matches!(status_code, 400 | 404 | 405 | 415 | 422);
    let gateway_rejection = matches!(status_code, 500 | 502 | 503 | 504)
        && (message.contains("bad gateway")
            || message.contains("gateway timeout")
            || message.contains("upstream")
            || message.contains("proxy")
            || message.contains("error code: 502")
            || message.contains("error code: 503")
            || message.contains("error code: 504"));
    if !compatibility_status && !gateway_rejection {
        return false;
    }

    let mentions_chat_endpoint =
        message.contains("/v1/chat/completions") || message.contains("chat/completions");
    let rejects_responses_input = matches!(error.param.as_deref(), Some("input" | "instructions"))
        && (message.contains("unknown parameter")
            || message.contains("unsupported parameter")
            || message.contains("expects")
            || message.contains("not supported"));
    let requires_messages = error.param.as_deref() == Some("messages")
        && (message.contains("required")
            || message.contains("missing")
            || message.contains("expects"));
    let textual_messages_hint = message.contains("expects `messages`")
        || message.contains("expects messages")
        || message.contains("use `messages`")
        || message.contains("use 'messages'")
        || message.contains("missing required parameter: `messages`")
        || message.contains("requires `messages`")
        || message.contains("requires messages")
        || message.contains("expected `messages`")
        || message.contains("expected messages")
        || message.contains("unknown parameter `input`")
        || message.contains("unknown parameter: `input`")
        || message.contains("unsupported parameter `input`")
        || message.contains("unsupported parameter: `input`")
        || message.contains("unknown parameter `instructions`")
        || message.contains("unknown parameter: `instructions`")
        || message.contains("unsupported parameter `instructions`")
        || message.contains("unsupported parameter: `instructions`");

    gateway_rejection
        || mentions_chat_endpoint
        || rejects_responses_input
        || requires_messages
        || textual_messages_hint
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderConfig;
    use crate::provider::auth_profile_runtime::resolve_provider_auth_profiles;
    use crate::provider::mock_transport::MockTransport;
    use crate::provider::transport::RequestAuthContext;
    use crate::provider::transport_trait::TransportResponse;
    use loongclaw_contracts::SecretRef;
    use serde_json::json;

    #[tokio::test(flavor = "current_thread")]
    async fn request_completion_with_provider_uses_injected_transport() {
        let provider = ProviderConfig {
            kind: crate::config::ProviderKind::Openai,
            api_key: Some(SecretRef::Inline("dispatch-test-secret".to_owned())),
            api_key_env: None,
            oauth_access_token: None,
            oauth_access_token_env: None,
            ..ProviderConfig::default()
        };
        let config = LoongClawConfig {
            provider: provider.clone(),
            ..LoongClawConfig::default()
        };
        let request_policy = policy::ProviderRequestPolicy::from_config(&provider);
        let auth_context = RequestAuthContext::default();
        let auth_profiles = resolve_provider_auth_profiles(&provider);
        let auth_profile = auth_profiles.first().expect("auth profile");
        let transport = MockTransport::with_execute_responses([Ok(TransportResponse {
            status: reqwest::StatusCode::OK,
            headers: reqwest::header::HeaderMap::new(),
            body: json!({
                "choices": [{
                    "message": {
                        "content": "dispatch mocked completion"
                    }
                }]
            }),
        })]);

        let result = request_completion_with_provider_transport(
            &config,
            &provider,
            &[json!({
                "role": "user",
                "content": "ping"
            })],
            "gpt-5.4",
            false,
            auth_profile,
            &auth_context,
            &request_policy,
            &transport,
        )
        .await
        .expect("dispatch should use injected transport");

        assert_eq!(result, "dispatch mocked completion");
        let requests = transport.requests();
        assert_eq!(requests.len(), 1);
        let authorization_header = requests[0]
            .headers
            .get(reqwest::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok());
        assert_eq!(authorization_header, Some("Bearer dispatch-test-secret"));
    }
}
