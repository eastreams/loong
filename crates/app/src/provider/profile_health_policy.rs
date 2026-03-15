use super::auth_profile_runtime::ProviderAuthProfile;
use super::failover::ProviderFailoverReason;
use super::profile_state_store::{ProviderProfileHealthMode, ProviderProfileHealthSnapshot};

pub(super) fn should_mark_provider_profile_failure(reason: ProviderFailoverReason) -> bool {
    matches!(
        reason,
        ProviderFailoverReason::RateLimited
            | ProviderFailoverReason::ProviderOverloaded
            | ProviderFailoverReason::AuthRejected
            | ProviderFailoverReason::TransportFailure
            | ProviderFailoverReason::RequestRejected
    )
}

pub(super) fn classify_profile_failure_reason_from_message(
    message: &str,
) -> ProviderFailoverReason {
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("status 401")
        || lowered.contains("status 403")
        || lowered.contains("unauthorized")
        || lowered.contains("forbidden")
    {
        return ProviderFailoverReason::AuthRejected;
    }
    if lowered.contains("status 429") {
        return ProviderFailoverReason::RateLimited;
    }
    if lowered.contains("status 500")
        || lowered.contains("status 502")
        || lowered.contains("status 503")
        || lowered.contains("status 504")
    {
        return ProviderFailoverReason::ProviderOverloaded;
    }
    ProviderFailoverReason::RequestRejected
}

pub(super) fn prioritize_profiles_by_health(
    profiles: &[ProviderAuthProfile],
    health_mode: ProviderProfileHealthMode,
    mut snapshot_for: impl FnMut(&ProviderAuthProfile) -> ProviderProfileHealthSnapshot,
) -> Vec<ProviderAuthProfile> {
    if profiles.len() <= 1 {
        return profiles.to_vec();
    }

    let mut ready = Vec::new();
    let mut cooling = Vec::new();

    for profile in profiles {
        let snapshot = snapshot_for(profile);
        if health_mode == ProviderProfileHealthMode::ObserveOnly {
            ready.push((profile.clone(), snapshot.last_used_at));
            continue;
        }
        if let Some(unusable_until) = snapshot.unusable_until {
            cooling.push((profile.clone(), unusable_until, snapshot.last_used_at));
        } else {
            ready.push((profile.clone(), snapshot.last_used_at));
        }
    }

    ready.sort_by_key(|(_, last_used)| *last_used);
    cooling.sort_by_key(|(_, unusable_until, last_used)| (*unusable_until, *last_used));

    ready
        .into_iter()
        .map(|(profile, _)| profile)
        .chain(cooling.into_iter().map(|(profile, _, _)| profile))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ProviderAuthProfile, ProviderFailoverReason, ProviderProfileHealthMode,
        ProviderProfileHealthSnapshot, classify_profile_failure_reason_from_message,
        prioritize_profiles_by_health, should_mark_provider_profile_failure,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn classify_profile_failure_reason_covers_auth_rate_limit_overload() {
        assert_eq!(
            classify_profile_failure_reason_from_message("provider returned status 401"),
            ProviderFailoverReason::AuthRejected
        );
        assert_eq!(
            classify_profile_failure_reason_from_message("request failed: Forbidden"),
            ProviderFailoverReason::AuthRejected
        );
        assert_eq!(
            classify_profile_failure_reason_from_message("provider returned status 429"),
            ProviderFailoverReason::RateLimited
        );
        assert_eq!(
            classify_profile_failure_reason_from_message("provider returned status 503"),
            ProviderFailoverReason::ProviderOverloaded
        );
        assert_eq!(
            classify_profile_failure_reason_from_message("unexpected decode failure"),
            ProviderFailoverReason::RequestRejected
        );
    }

    #[test]
    fn should_mark_provider_profile_failure_filters_only_health_relevant_reasons() {
        assert!(should_mark_provider_profile_failure(
            ProviderFailoverReason::RateLimited
        ));
        assert!(should_mark_provider_profile_failure(
            ProviderFailoverReason::TransportFailure
        ));
        assert!(!should_mark_provider_profile_failure(
            ProviderFailoverReason::ModelMismatch
        ));
        assert!(!should_mark_provider_profile_failure(
            ProviderFailoverReason::PayloadIncompatible
        ));
        assert!(!should_mark_provider_profile_failure(
            ProviderFailoverReason::ResponseShapeInvalid
        ));
    }

    #[test]
    fn prioritize_profiles_by_health_enforce_mode_prefers_usable_profiles() {
        let now = Instant::now();
        let alpha = ProviderAuthProfile {
            id: "alpha".to_owned(),
            authorization_header: None,
            x_api_key_header: None,
            auth_cache_key: None,
        };
        let beta = ProviderAuthProfile {
            id: "beta".to_owned(),
            authorization_header: None,
            x_api_key_header: None,
            auth_cache_key: None,
        };
        let gamma = ProviderAuthProfile {
            id: "gamma".to_owned(),
            authorization_header: None,
            x_api_key_header: None,
            auth_cache_key: None,
        };
        let profiles = [alpha, beta, gamma];
        let ordered = prioritize_profiles_by_health(
            &profiles,
            ProviderProfileHealthMode::EnforceUnusableWindows,
            |profile| match profile.id.as_str() {
                "alpha" => ProviderProfileHealthSnapshot {
                    unusable_until: Some(now + Duration::from_secs(5)),
                    last_used_at: Some(now - Duration::from_secs(10)),
                },
                "beta" => ProviderProfileHealthSnapshot {
                    unusable_until: None,
                    last_used_at: Some(now - Duration::from_secs(2)),
                },
                "gamma" => ProviderProfileHealthSnapshot {
                    unusable_until: None,
                    last_used_at: Some(now - Duration::from_secs(20)),
                },
                _ => ProviderProfileHealthSnapshot::default(),
            },
        );
        assert_eq!(
            ordered
                .into_iter()
                .map(|profile| profile.id)
                .collect::<Vec<_>>(),
            vec!["gamma", "beta", "alpha"]
        );
    }

    #[test]
    fn prioritize_profiles_by_health_observe_mode_ignores_unusable_windows() {
        let now = Instant::now();
        let alpha = ProviderAuthProfile {
            id: "alpha".to_owned(),
            authorization_header: None,
            x_api_key_header: None,
            auth_cache_key: None,
        };
        let beta = ProviderAuthProfile {
            id: "beta".to_owned(),
            authorization_header: None,
            x_api_key_header: None,
            auth_cache_key: None,
        };
        let profiles = [alpha, beta];
        let ordered = prioritize_profiles_by_health(
            &profiles,
            ProviderProfileHealthMode::ObserveOnly,
            |profile| match profile.id.as_str() {
                "alpha" => ProviderProfileHealthSnapshot {
                    unusable_until: Some(now + Duration::from_secs(3600)),
                    last_used_at: Some(now - Duration::from_secs(1)),
                },
                "beta" => ProviderProfileHealthSnapshot {
                    unusable_until: None,
                    last_used_at: Some(now - Duration::from_secs(10)),
                },
                _ => ProviderProfileHealthSnapshot::default(),
            },
        );
        assert_eq!(
            ordered
                .into_iter()
                .map(|profile| profile.id)
                .collect::<Vec<_>>(),
            vec!["beta", "alpha"]
        );
    }
}
