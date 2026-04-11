use std::collections::HashSet;

use crate::config::{ProviderAuthScheme, ProviderConfig, ProviderKind};

use super::provider_keyspace::build_provider_auth_profile_id;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProviderAuthProfile {
    pub(super) id: String,
    pub(super) authorization_secret: Option<String>,
    pub(super) api_key_secret: Option<String>,
    pub(super) auth_cache_key: Option<String>,
}

pub(super) fn resolve_provider_auth_profiles(
    provider: &ProviderConfig,
) -> Vec<ProviderAuthProfile> {
    let mut profiles = Vec::new();
    let mut seen = HashSet::new();

    match provider.kind.auth_scheme() {
        ProviderAuthScheme::Bearer => {
            if provider.kind == ProviderKind::GithubCopilot {
                if let Some(api_key) =
                    super::copilot_auth::cached_provider_copilot_api_key(provider)
                {
                    push_bearer_profile(&mut profiles, &mut seen, "copilot", &api_key);
                }
            } else if let Some(token) = provider.oauth_access_token() {
                push_bearer_profile(&mut profiles, &mut seen, "oauth", token.as_str());
            }

            for api_key in provider.api_key_candidates() {
                push_bearer_api_key_profile(&mut profiles, &mut seen, api_key.as_str());
            }
        }
        ProviderAuthScheme::XApiKey | ProviderAuthScheme::XGoogApiKey => {
            for api_key in provider.api_key_candidates() {
                push_x_api_key_profile(&mut profiles, &mut seen, api_key.as_str());
            }
        }
    }

    if profiles.is_empty() || provider.kind == ProviderKind::Bedrock {
        profiles.push(anonymous_auth_profile());
    }

    profiles
}

pub(super) fn auth_profile_supports_scheme(
    profile: &ProviderAuthProfile,
    auth_scheme: ProviderAuthScheme,
) -> bool {
    match auth_scheme {
        ProviderAuthScheme::Bearer => {
            profile.authorization_secret.is_some() || profile.api_key_secret.is_some()
        }
        ProviderAuthScheme::XApiKey | ProviderAuthScheme::XGoogApiKey => {
            profile.api_key_secret.is_some()
        }
    }
}

fn anonymous_auth_profile() -> ProviderAuthProfile {
    ProviderAuthProfile {
        id: "anonymous".to_owned(),
        authorization_secret: None,
        api_key_secret: None,
        auth_cache_key: None,
    }
}

fn push_bearer_profile(
    profiles: &mut Vec<ProviderAuthProfile>,
    seen: &mut HashSet<String>,
    prefix: &str,
    secret: &str,
) {
    let auth_cache_key = format!("bearer:{secret}");
    if !seen.insert(auth_cache_key.clone()) {
        return;
    }
    profiles.push(ProviderAuthProfile {
        id: build_provider_auth_profile_id(prefix, secret),
        authorization_secret: Some(secret.to_owned()),
        api_key_secret: None,
        auth_cache_key: Some(auth_cache_key),
    });
}

fn push_bearer_api_key_profile(
    profiles: &mut Vec<ProviderAuthProfile>,
    seen: &mut HashSet<String>,
    secret: &str,
) {
    let auth_cache_key = format!("bearer:{secret}");
    if !seen.insert(auth_cache_key.clone()) {
        if let Some(profile) = profiles
            .iter_mut()
            .find(|profile| profile.auth_cache_key.as_deref() == Some(auth_cache_key.as_str()))
            && profile.api_key_secret.is_none()
        {
            profile.api_key_secret = Some(secret.to_owned());
        }
        return;
    }
    profiles.push(ProviderAuthProfile {
        id: build_provider_auth_profile_id("api_key", secret),
        authorization_secret: Some(secret.to_owned()),
        api_key_secret: Some(secret.to_owned()),
        auth_cache_key: Some(auth_cache_key),
    });
}

fn push_x_api_key_profile(
    profiles: &mut Vec<ProviderAuthProfile>,
    seen: &mut HashSet<String>,
    secret: &str,
) {
    let cache_key = format!("x-api-key:{secret}");
    if !seen.insert(cache_key.clone()) {
        return;
    }
    profiles.push(ProviderAuthProfile {
        id: build_provider_auth_profile_id("api_key", secret),
        authorization_secret: None,
        api_key_secret: Some(secret.to_owned()),
        auth_cache_key: Some(cache_key),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderKind;

    #[test]
    fn resolve_provider_auth_profiles_deduplicates_identical_bearer_headers() {
        let provider = ProviderConfig {
            kind: ProviderKind::Ollama,
            api_key: Some(loongclaw_contracts::SecretRef::Inline(
                "same-secret".to_owned(),
            )),
            api_key_env: None,
            oauth_access_token: Some(loongclaw_contracts::SecretRef::Inline(
                "same-secret".to_owned(),
            )),
            oauth_access_token_env: None,
            ..ProviderConfig::default()
        };

        let profiles = resolve_provider_auth_profiles(&provider);
        assert_eq!(profiles.len(), 1);
        assert_eq!(
            profiles[0].authorization_secret.as_deref(),
            Some("same-secret")
        );
        assert_eq!(profiles[0].api_key_secret.as_deref(), Some("same-secret"));
    }

    #[test]
    fn resolve_provider_auth_profiles_uses_x_api_key_for_native_auth_providers() {
        let provider = ProviderConfig {
            kind: ProviderKind::Anthropic,
            api_key: Some(loongclaw_contracts::SecretRef::Inline(
                "anthropic-secret".to_owned(),
            )),
            api_key_env: None,
            oauth_access_token: Some(loongclaw_contracts::SecretRef::Inline(
                "ignored-oauth".to_owned(),
            )),
            oauth_access_token_env: None,
            ..ProviderConfig::default()
        };

        let profiles = resolve_provider_auth_profiles(&provider);
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].authorization_secret, None);
        assert_eq!(
            profiles[0].api_key_secret.as_deref(),
            Some("anthropic-secret")
        );
        assert_eq!(
            profiles[0].auth_cache_key.as_deref(),
            Some("x-api-key:anthropic-secret")
        );
    }

    #[test]
    fn auth_profile_supports_scheme_requires_api_key_for_x_api_key_routes() {
        let oauth_only_profile = ProviderAuthProfile {
            id: "oauth:test".to_owned(),
            authorization_secret: Some("oauth-only".to_owned()),
            api_key_secret: None,
            auth_cache_key: Some("bearer:oauth-only".to_owned()),
        };

        assert!(auth_profile_supports_scheme(
            &oauth_only_profile,
            ProviderAuthScheme::Bearer
        ));
        assert!(!auth_profile_supports_scheme(
            &oauth_only_profile,
            ProviderAuthScheme::XApiKey
        ));
        assert!(!auth_profile_supports_scheme(
            &oauth_only_profile,
            ProviderAuthScheme::XGoogApiKey
        ));
    }

    #[test]
    fn resolve_provider_auth_profiles_falls_back_to_anonymous_when_no_secret_available() {
        let provider = ProviderConfig {
            kind: ProviderKind::Ollama,
            api_key: None,
            api_key_env: None,
            oauth_access_token: None,
            oauth_access_token_env: None,
            ..ProviderConfig::default()
        };

        let profiles = resolve_provider_auth_profiles(&provider);
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].id, "anonymous");
        assert_eq!(profiles[0].authorization_secret, None);
        assert_eq!(profiles[0].api_key_secret, None);
        assert_eq!(profiles[0].auth_cache_key, None);
    }

    #[test]
    fn resolve_provider_auth_profiles_uses_copilot_cache_for_github_copilot() {
        let _guard = super::super::copilot_auth::acquire_cache_test_lock();

        super::super::copilot_auth::set_cached_key_for_auth_source_for_test(
            "github-oauth-token-should-not-be-used",
            "test-copilot-api-key",
            super::super::copilot_auth::now_unix_for_test() + 3600,
        );

        let provider = ProviderConfig {
            kind: ProviderKind::GithubCopilot,
            api_key: None,
            api_key_env: None,
            oauth_access_token: Some(loongclaw_contracts::SecretRef::Inline(
                "github-oauth-token-should-not-be-used".to_owned(),
            )),
            oauth_access_token_env: None,
            ..ProviderConfig::default()
        };

        let profiles = resolve_provider_auth_profiles(&provider);
        assert_eq!(profiles.len(), 1);
        assert_eq!(
            profiles[0].authorization_secret.as_deref(),
            Some("test-copilot-api-key")
        );

        super::super::copilot_auth::clear_cache_for_test();
    }

    #[test]
    fn resolve_provider_auth_profiles_isolates_copilot_cache_by_oauth_token() {
        let _guard = super::super::copilot_auth::acquire_cache_test_lock();

        super::super::copilot_auth::set_cached_key_for_auth_source_for_test(
            "github-oauth-token-a",
            "test-copilot-api-key-a",
            super::super::copilot_auth::now_unix_for_test() + 3600,
        );

        let provider = ProviderConfig {
            kind: ProviderKind::GithubCopilot,
            api_key: None,
            api_key_env: None,
            oauth_access_token: Some(loongclaw_contracts::SecretRef::Inline(
                "github-oauth-token-b".to_owned(),
            )),
            oauth_access_token_env: None,
            ..ProviderConfig::default()
        };

        let profiles = resolve_provider_auth_profiles(&provider);

        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].id, "anonymous");
        assert_eq!(profiles[0].authorization_secret, None);
        assert_eq!(profiles[0].api_key_secret, None);

        super::super::copilot_auth::clear_cache_for_test();
    }

    #[test]
    fn resolve_provider_auth_profiles_adds_bedrock_sigv4_fallback_after_bearer_profiles() {
        let provider = ProviderConfig {
            kind: ProviderKind::Bedrock,
            api_key: Some(loongclaw_contracts::SecretRef::Inline(
                "bedrock-bearer-token".to_owned(),
            )),
            api_key_env: None,
            oauth_access_token: None,
            oauth_access_token_env: None,
            ..ProviderConfig::default()
        };

        let profiles = resolve_provider_auth_profiles(&provider);
        assert_eq!(profiles.len(), 2);
        assert_eq!(
            profiles[0].authorization_secret.as_deref(),
            Some("bedrock-bearer-token")
        );
        assert_eq!(profiles[1].id, "anonymous");
        assert_eq!(profiles[1].authorization_secret, None);
        assert_eq!(profiles[1].api_key_secret, None);
    }
}
