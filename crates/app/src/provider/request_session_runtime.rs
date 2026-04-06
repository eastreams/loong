use std::time::Duration;

use crate::{CliResult, config::LoongClawConfig};

use super::auth_profile_runtime::{ProviderAuthProfile, resolve_provider_auth_profiles};
use super::http_client_runtime::build_http_client;
use super::model_candidate_cooldown_runtime::ModelCandidateCooldownPolicy;
use super::model_candidate_resolver_runtime::resolve_request_models;
use super::policy;
use super::profile_health_policy::classify_profile_failure_reason_from_message;
use super::profile_health_runtime::{
    ProviderProfileStatePolicy, build_provider_profile_state_policy, mark_provider_profile_failure,
    prioritize_provider_auth_profiles_by_health,
};
use super::profile_state_backend::ensure_provider_profile_state_backend;
use super::provider_keyspace::build_model_candidate_cooldown_namespace;
use super::provider_validation_runtime::{
    validate_provider_auth_readiness, validate_provider_configuration,
    validate_provider_feature_gate,
};

pub(super) struct ProviderRequestSession {
    pub(super) endpoint: String,
    pub(super) headers: reqwest::header::HeaderMap,
    pub(super) request_policy: policy::ProviderRequestPolicy,
    pub(super) client: reqwest::Client,
    pub(super) auth_profiles: Vec<ProviderAuthProfile>,
    pub(super) profile_state_policy: Option<ProviderProfileStatePolicy>,
    pub(super) model_candidates: Vec<String>,
    pub(super) auto_model_mode: bool,
    pub(super) model_candidate_cooldown_policy: Option<ModelCandidateCooldownPolicy>,
    pub(super) auth_context: super::transport::RequestAuthContext,
}

pub(super) async fn prepare_provider_request_session(
    config: &LoongClawConfig,
) -> CliResult<ProviderRequestSession> {
    validate_provider_configuration(config)?;
    validate_provider_feature_gate(config)?;
    validate_provider_auth_readiness(config).await?;
    ensure_provider_profile_state_backend(config);

    let endpoint = config.provider.endpoint();
    let auth_context = super::transport::resolve_request_auth_context(&config.provider).await?;
    let headers = super::transport::build_request_headers_without_provider_auth(&config.provider)?;
    let request_policy = policy::ProviderRequestPolicy::from_config(&config.provider);
    let client = build_http_client(&request_policy)?;
    let profile_state_policy =
        build_provider_profile_state_policy(&config.provider, &endpoint, &headers);
    let auth_profiles = prioritize_provider_auth_profiles_by_health(
        &resolve_provider_auth_profiles(&config.provider),
        profile_state_policy.as_ref(),
    );
    let primary_auth_cache_key = auth_profiles
        .first()
        .and_then(|profile| profile.auth_cache_key.as_deref());
    let model_candidate_cooldown_policy = build_model_candidate_cooldown_policy(
        &config.provider,
        &endpoint,
        &headers,
        primary_auth_cache_key,
    );
    let auto_model_mode = config.provider.model_selection_requires_fetch();
    let model_candidates = if auto_model_mode {
        let mut resolved_candidates = None;
        let mut last_error = None;
        for profile in &auth_profiles {
            match resolve_request_models(
                config,
                &headers,
                &request_policy,
                model_candidate_cooldown_policy.as_ref(),
                Some(profile),
                &auth_context,
            )
            .await
            {
                Ok(candidates) => {
                    resolved_candidates = Some(candidates);
                    break;
                }
                Err(error) => {
                    if let Some(policy) = profile_state_policy.as_ref() {
                        mark_provider_profile_failure(
                            policy,
                            profile,
                            classify_profile_failure_reason_from_message(error.as_str()),
                        );
                    }
                    tracing::debug!(
                        target: "loongclaw.provider",
                        provider_id = %config.provider.kind.profile().id,
                        auth_profile_id = %profile.id,
                        auto_model_mode,
                        error = %crate::observability::summarize_error(error.as_str()),
                        "provider model catalog resolution failed for auth profile"
                    );
                    last_error = Some(error);
                }
            }
        }
        if let Some(model_candidates) = resolved_candidates {
            model_candidates
        } else {
            let error_message = last_error.unwrap_or_else(|| {
                "provider model-list unavailable for every auth profile".to_owned()
            });

            tracing::warn!(
                target: "loongclaw.provider",
                provider_id = %config.provider.kind.profile().id,
                auth_profile_count = auth_profiles.len(),
                auto_model_mode,
                error = %crate::observability::summarize_error(error_message.as_str()),
                "provider model catalog resolution failed for every auth profile"
            );

            return Err(error_message);
        }
    } else {
        resolve_request_models(
            config,
            &headers,
            &request_policy,
            model_candidate_cooldown_policy.as_ref(),
            auth_profiles.first(),
            &auth_context,
        )
        .await?
    };

    let session = ProviderRequestSession {
        endpoint,
        headers,
        request_policy,
        client,
        auth_profiles,
        profile_state_policy,
        model_candidates,
        auto_model_mode,
        model_candidate_cooldown_policy,
        auth_context,
    };
    tracing::debug!(
        target: "loongclaw.provider",
        provider_id = %config.provider.kind.profile().id,
        auth_profile_count = session.auth_profiles.len(),
        model_candidate_count = session.model_candidates.len(),
        auto_model_mode = session.auto_model_mode,
        "prepared provider request session"
    );
    Ok(session)
}

fn build_model_candidate_cooldown_policy(
    provider: &crate::config::ProviderConfig,
    endpoint: &str,
    headers: &reqwest::header::HeaderMap,
    auth_cache_key: Option<&str>,
) -> Option<ModelCandidateCooldownPolicy> {
    if !provider.model_selection_requires_fetch() {
        return None;
    }

    let cooldown_ms = provider.resolved_model_candidate_cooldown_ms();
    if cooldown_ms == 0 {
        return None;
    }
    let cooldown_max_ms = provider.resolved_model_candidate_cooldown_max_ms();

    Some(ModelCandidateCooldownPolicy {
        namespace: build_model_candidate_cooldown_namespace(endpoint, headers, auth_cache_key),
        cooldown: Duration::from_millis(cooldown_ms),
        max_cooldown: Duration::from_millis(cooldown_max_ms),
        max_entries: provider.resolved_model_candidate_cooldown_max_entries(),
    })
}
