use std::time::Duration;

use crate::{CliResult, config::LoongClawConfig};

use super::auth_profile_runtime::{ProviderAuthProfile, resolve_provider_auth_profiles};
use super::capability_profile_runtime::ProviderCapabilityProfile;
use super::contracts::{ProviderRuntimeContract, provider_runtime_contract};
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
    validate_provider_configuration, validate_provider_feature_gate,
};

pub(super) struct ProviderRequestSession {
    pub(super) runtime_contract: ProviderRuntimeContract,
    pub(super) capability_profile: ProviderCapabilityProfile,
    pub(super) endpoint: String,
    pub(super) headers: reqwest::header::HeaderMap,
    pub(super) request_policy: policy::ProviderRequestPolicy,
    pub(super) client: reqwest::Client,
    pub(super) auth_profiles: Vec<ProviderAuthProfile>,
    pub(super) profile_state_policy: Option<ProviderProfileStatePolicy>,
    pub(super) model_candidates: Vec<String>,
    pub(super) auto_model_mode: bool,
    pub(super) model_candidate_cooldown_policy: Option<ModelCandidateCooldownPolicy>,
}

pub(super) async fn prepare_provider_request_session(
    config: &LoongClawConfig,
) -> CliResult<ProviderRequestSession> {
    validate_provider_configuration(config)?;
    validate_provider_feature_gate(config)?;
    ensure_provider_profile_state_backend(config);

    let runtime_contract = provider_runtime_contract(&config.provider);
    let capability_profile =
        ProviderCapabilityProfile::from_provider(&config.provider, runtime_contract);
    let endpoint = config.provider.endpoint();
    let headers = super::transport::build_request_headers(&config.provider)?;
    let request_policy = policy::ProviderRequestPolicy::from_config(&config.provider);
    let client = build_http_client(&request_policy)?;
    let profile_state_policy =
        build_provider_profile_state_policy(&config.provider, &endpoint, &headers);
    let auth_profiles = prioritize_provider_auth_profiles_by_health(
        &resolve_provider_auth_profiles(&config.provider),
        profile_state_policy.as_ref(),
    );
    let primary_authorization = auth_profiles
        .first()
        .and_then(|profile| profile.authorization_header.as_deref());
    let model_candidate_cooldown_policy = build_model_candidate_cooldown_policy(
        &config.provider,
        &endpoint,
        &headers,
        primary_authorization,
    );
    let auto_model_mode = config.provider.model_selection_requires_fetch();
    let model_candidates = if auto_model_mode {
        let mut resolved = None;
        let mut last_error = None;
        for profile in &auth_profiles {
            match resolve_request_models(
                config,
                &headers,
                &request_policy,
                model_candidate_cooldown_policy.as_ref(),
                profile.authorization_header.as_deref(),
            )
            .await
            {
                Ok(candidates) => {
                    resolved = Some(candidates);
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
                    last_error = Some(error);
                }
            }
        }
        resolved.ok_or_else(|| {
            last_error.unwrap_or_else(|| {
                "provider model-list unavailable for every auth profile".to_owned()
            })
        })?
    } else {
        resolve_request_models(
            config,
            &headers,
            &request_policy,
            model_candidate_cooldown_policy.as_ref(),
            primary_authorization,
        )
        .await?
    };

    Ok(ProviderRequestSession {
        runtime_contract,
        capability_profile,
        endpoint,
        headers,
        request_policy,
        client,
        auth_profiles,
        profile_state_policy,
        model_candidates,
        auto_model_mode,
        model_candidate_cooldown_policy,
    })
}

fn build_model_candidate_cooldown_policy(
    provider: &crate::config::ProviderConfig,
    endpoint: &str,
    headers: &reqwest::header::HeaderMap,
    auth_header: Option<&str>,
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
        namespace: build_model_candidate_cooldown_namespace(endpoint, headers, auth_header),
        cooldown: Duration::from_millis(cooldown_ms),
        max_cooldown: Duration::from_millis(cooldown_max_ms),
        max_entries: provider.resolved_model_candidate_cooldown_max_entries(),
    })
}
