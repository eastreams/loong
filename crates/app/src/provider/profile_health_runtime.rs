use std::time::{Duration, Instant};

use crate::config::ProviderConfig;

use super::auth_profile_runtime::ProviderAuthProfile;
use super::contracts::provider_runtime_contract;
use super::failover::ProviderFailoverReason;
use super::profile_health_policy::{
    prioritize_profiles_by_health, should_mark_provider_profile_failure,
};
use super::profile_state_backend::{
    persist_provider_profile_state_snapshot, with_provider_profile_states,
};
use super::profile_state_store::ProviderProfileHealthMode;
use super::provider_keyspace::{
    build_provider_profile_state_key, build_provider_profile_state_namespace,
};

#[derive(Debug, Clone)]
pub(super) struct ProviderProfileStatePolicy {
    pub(super) namespace: String,
    pub(super) health_mode: ProviderProfileHealthMode,
    pub(super) cooldown: Duration,
    pub(super) max_cooldown: Duration,
    pub(super) auth_reject_disable: Duration,
    pub(super) max_entries: usize,
}

pub(super) fn mark_provider_profile_success(
    policy: &ProviderProfileStatePolicy,
    profile: &ProviderAuthProfile,
) {
    let key = build_provider_profile_state_key(policy.namespace.as_str(), profile.id.as_str());
    let snapshot = with_provider_profile_states(|store| {
        let now = Instant::now();
        store.mark_success(key, now, policy.max_entries);
        store.to_snapshot(now)
    });
    persist_provider_profile_state_snapshot(&snapshot);
}

pub(super) fn mark_provider_profile_failure(
    policy: &ProviderProfileStatePolicy,
    profile: &ProviderAuthProfile,
    reason: ProviderFailoverReason,
) {
    if !should_mark_provider_profile_failure(reason) {
        return;
    }
    let key = build_provider_profile_state_key(policy.namespace.as_str(), profile.id.as_str());
    let snapshot = with_provider_profile_states(|store| {
        let now = Instant::now();
        store.mark_failure(key, reason, now, policy);
        store.to_snapshot(now)
    });
    persist_provider_profile_state_snapshot(&snapshot);
}

pub(super) fn prioritize_provider_auth_profiles_by_health(
    profiles: &[ProviderAuthProfile],
    policy: Option<&ProviderProfileStatePolicy>,
) -> Vec<ProviderAuthProfile> {
    let Some(policy) = policy else {
        return profiles.to_vec();
    };
    let now = Instant::now();
    prioritize_profiles_by_health(profiles, policy.health_mode, |profile| {
        let state_key =
            build_provider_profile_state_key(policy.namespace.as_str(), profile.id.as_str());
        with_provider_profile_states(|store| store.health_snapshot(state_key.as_str(), now))
    })
}

pub(super) fn build_provider_profile_state_policy(
    provider: &ProviderConfig,
    endpoint: &str,
    headers: &reqwest::header::HeaderMap,
) -> Option<ProviderProfileStatePolicy> {
    let runtime_contract = provider_runtime_contract(provider);
    Some(ProviderProfileStatePolicy {
        namespace: build_provider_profile_state_namespace(endpoint, headers),
        health_mode: runtime_contract.profile_health_mode,
        cooldown: Duration::from_millis(provider.resolved_profile_cooldown_ms()),
        max_cooldown: Duration::from_millis(provider.resolved_profile_cooldown_max_ms()),
        auth_reject_disable: Duration::from_millis(
            provider.resolved_profile_auth_reject_disable_ms(),
        ),
        max_entries: provider.resolved_profile_state_max_entries(),
    })
}
