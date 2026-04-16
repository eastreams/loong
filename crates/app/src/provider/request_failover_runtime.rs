use std::future::Future;

use crate::{CliResult, config::ProviderConfig};

use super::auth_profile_runtime::ProviderAuthProfile;
use super::failover::ModelRequestError;
use super::failover_telemetry_runtime::record_provider_failover_audit_event;
use super::model_candidate_cooldown_runtime::{
    ModelCandidateCooldownPolicy, register_model_candidate_cooldown,
};
use super::profile_health_runtime::{
    ProviderProfileStatePolicy, mark_provider_profile_failure, mark_provider_profile_success,
    prioritize_provider_auth_profiles_by_health,
};
use super::runtime_binding::ProviderRuntimeBinding;

pub(super) async fn request_across_model_candidates<T, F, Fut>(
    provider: &ProviderConfig,
    binding: ProviderRuntimeBinding<'_>,
    auth_profiles: &[ProviderAuthProfile],
    profile_state_policy: Option<&ProviderProfileStatePolicy>,
    model_candidates: &[String],
    auto_model_mode: bool,
    model_candidate_cooldown_policy: Option<&ModelCandidateCooldownPolicy>,
    mut request_with_model: F,
) -> CliResult<T>
where
    F: FnMut(String, bool, ProviderAuthProfile) -> Fut,
    Fut: Future<Output = Result<T, ModelRequestError>>,
{
    if model_candidates.is_empty() {
        return Err("provider request has no model candidates".to_owned());
    }

    let ordered_profiles =
        prioritize_provider_auth_profiles_by_health(auth_profiles, profile_state_policy);
    tracing::debug!(
        target: "loongclaw.provider",
        provider_id = %provider.kind.profile().id,
        binding = %binding.as_str(),
        model_candidate_count = model_candidates.len(),
        auth_profile_count = ordered_profiles.len(),
        auto_model_mode,
        "dispatching provider request across model candidates"
    );
    let mut last_error = None;
    let mut last_error_snapshot = None;
    for (model_index, model) in model_candidates.iter().enumerate() {
        let mut model_switch_reason = None;
        let mut model_cooldown_reason = None;
        let mut model_cooldown_rate_limit = None;
        for (profile_index, profile) in ordered_profiles.iter().enumerate() {
            match request_with_model(model.clone(), auto_model_mode, profile.clone()).await {
                Ok(value) => {
                    if let Some(policy) = profile_state_policy {
                        mark_provider_profile_success(policy, profile);
                    }
                    tracing::debug!(
                        target: "loongclaw.provider",
                        provider_id = %provider.kind.profile().id,
                        binding = %binding.as_str(),
                        model = %model,
                        auth_profile_id = %profile.id,
                        candidate_index = model_index + 1,
                        candidate_count = model_candidates.len(),
                        profile_index = profile_index + 1,
                        profile_count = ordered_profiles.len(),
                        "provider request succeeded"
                    );
                    return Ok(value);
                }
                Err(model_error) => {
                    let ModelRequestError {
                        message,
                        try_next_model,
                        reason,
                        snapshot,
                        rate_limit,
                        ..
                    } = model_error;
                    let exhausted = profile_index + 1 >= ordered_profiles.len()
                        && model_index + 1 >= model_candidates.len();
                    record_provider_failover_audit_event(
                        binding,
                        provider,
                        &snapshot,
                        try_next_model,
                        auto_model_mode,
                        model_index,
                        model_candidates.len(),
                        exhausted,
                    );
                    if let Some(policy) = profile_state_policy {
                        mark_provider_profile_failure(policy, profile, reason);
                    }
                    tracing::warn!(
                        target: "loongclaw.provider",
                        provider_id = %provider.kind.profile().id,
                        binding = %binding.as_str(),
                        model = %snapshot.model,
                        auth_profile_id = %profile.id,
                        reason = %snapshot.reason.as_str(),
                        stage = %snapshot.stage.as_str(),
                        attempt = snapshot.attempt,
                        max_attempts = snapshot.max_attempts,
                        status_code = ?snapshot.status_code,
                        try_next_model,
                        candidate_index = model_index + 1,
                        candidate_count = model_candidates.len(),
                        profile_index = profile_index + 1,
                        profile_count = ordered_profiles.len(),
                        exhausted,
                        error = %crate::observability::summarize_error(message.as_str()),
                        "provider request attempt failed"
                    );
                    last_error = Some(message);
                    last_error_snapshot = Some(snapshot);
                    if matches!(
                        reason,
                        super::failover::ProviderFailoverReason::ModelMismatch
                            | super::failover::ProviderFailoverReason::PayloadIncompatible
                            | super::failover::ProviderFailoverReason::RateLimited
                            | super::failover::ProviderFailoverReason::ProviderOverloaded
                    ) && should_replace_model_cooldown_reason(
                        model_cooldown_reason,
                        model_cooldown_rate_limit.as_ref(),
                        reason,
                        rate_limit.as_ref(),
                    ) {
                        model_cooldown_reason = Some(reason);
                        model_cooldown_rate_limit = rate_limit;
                    }

                    if try_next_model {
                        model_switch_reason = Some(reason);
                        continue;
                    }
                }
            }
        }

        if let Some(reason) = model_cooldown_reason
            && let Some(policy) = model_candidate_cooldown_policy
        {
            register_model_candidate_cooldown(
                policy,
                model.as_str(),
                reason,
                model_cooldown_rate_limit.as_ref(),
            );
        }
        if model_switch_reason.is_some() && model_index + 1 < model_candidates.len() {
            continue;
        }
    }

    Err(last_error.unwrap_or_else(|| {
        if let Some(snapshot) = last_error_snapshot {
            return format!(
                "provider request failed for every model candidate (last_reason={}) | provider_failover={}",
                snapshot.reason.as_str(),
                snapshot.to_json_value()
            );
        }
        "provider request failed for every model candidate".to_owned()
    }))
}

fn should_replace_model_cooldown_reason(
    current_reason: Option<super::failover::ProviderFailoverReason>,
    current_rate_limit: Option<&super::rate_limit::RateLimitObservation>,
    next_reason: super::failover::ProviderFailoverReason,
    next_rate_limit: Option<&super::rate_limit::RateLimitObservation>,
) -> bool {
    let Some(current_reason) = current_reason else {
        return true;
    };

    let current_hint =
        current_rate_limit.and_then(super::rate_limit::RateLimitObservation::cooldown_hint);
    let next_hint =
        next_rate_limit.and_then(super::rate_limit::RateLimitObservation::cooldown_hint);

    if next_hint.is_some() && current_hint.is_none() {
        return true;
    }
    if let (Some(current_hint), Some(next_hint)) = (current_hint, next_hint)
        && next_hint > current_hint
    {
        return true;
    }
    if current_reason != super::failover::ProviderFailoverReason::RateLimited
        && next_reason == super::failover::ProviderFailoverReason::RateLimited
    {
        return true;
    }

    false
}
