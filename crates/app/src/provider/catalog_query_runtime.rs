use crate::{CliResult, config::LoongClawConfig};

use super::auth_profile_runtime::resolve_provider_auth_profiles;
use super::catalog_executor::{ModelCatalogRequestRuntime, fetch_available_models_with_policy};
use super::policy;
use super::profile_health_policy::classify_profile_failure_reason_from_message;
use super::profile_health_runtime::{
    build_provider_profile_state_policy, mark_provider_profile_failure,
    mark_provider_profile_success, prioritize_provider_auth_profiles_by_health,
};
use super::provider_validation_runtime::{
    validate_provider_configuration, validate_provider_feature_gate,
};

pub(super) async fn fetch_available_models_with_profiles(
    config: &LoongClawConfig,
) -> CliResult<Vec<String>> {
    validate_provider_configuration(config)?;
    validate_provider_feature_gate(config)?;
    let headers = super::transport::build_request_headers(&config.provider)?;
    let request_policy = policy::ProviderRequestPolicy::from_config(&config.provider);
    let endpoint = config.provider.models_endpoint();
    let profile_state_policy =
        build_provider_profile_state_policy(&config.provider, &endpoint, &headers);
    let auth_profiles = prioritize_provider_auth_profiles_by_health(
        &resolve_provider_auth_profiles(&config.provider),
        profile_state_policy.as_ref(),
    );

    let mut last_error = None;
    for profile in &auth_profiles {
        match fetch_available_models_with_policy(ModelCatalogRequestRuntime {
            provider: &config.provider,
            headers: &headers,
            request_policy: &request_policy,
            authorization_header: profile.authorization_header.as_deref(),
        })
        .await
        {
            Ok(models) => {
                if let Some(policy) = profile_state_policy.as_ref() {
                    mark_provider_profile_success(policy, profile);
                }
                return Ok(models);
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

    Err(last_error
        .unwrap_or_else(|| "provider model-list unavailable for every auth profile".to_owned()))
}
