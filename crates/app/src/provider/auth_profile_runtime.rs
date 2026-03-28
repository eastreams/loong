use std::collections::HashSet;

use crate::config::{ProviderAuthScheme, ProviderConfig, ProviderKind};

use super::provider_keyspace::build_provider_auth_profile_id;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProviderAuthProfile {
    pub(super) id: String,
    pub(super) authorization_header: Option<String>,
    pub(super) x_api_key_header: Option<String>,
    pub(super) auth_cache_key: Option<String>,
}

pub(super) fn resolve_provider_auth_profiles(
    provider: &ProviderConfig,
) -> Vec<ProviderAuthProfile> {
    let mut profiles = Vec::new();
    let mut seen = HashSet::new();

    match provider.kind.auth_scheme() {
        ProviderAuthScheme::Bearer => {
            if let Some(token) = provider.oauth_access_token() {
                push_bearer_profile(&mut profiles, &mut seen, "oauth", token.as_str());
            }

            for api_key in provider.api_key_candidates() {
                push_bearer_profile(&mut profiles, &mut seen, "api_key", api_key.as_str());
            }
        }
        ProviderAuthScheme::XApiKey => {
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

fn anonymous_auth_profile() -> ProviderAuthProfile {
    ProviderAuthProfile {
        id: "anonymous".to_owned(),
        authorization_header: None,
        x_api_key_header: None,
        auth_cache_key: None,
    }
}

fn push_bearer_profile(
    profiles: &mut Vec<ProviderAuthProfile>,
    seen: &mut HashSet<String>,
    prefix: &str,
    secret: &str,
) {
    let authorization_header = format!("Bearer {secret}");
    if !seen.insert(authorization_header.clone()) {
        return;
    }
    profiles.push(ProviderAuthProfile {
        id: build_provider_auth_profile_id(prefix, secret),
        authorization_header: Some(authorization_header.clone()),
        x_api_key_header: None,
        auth_cache_key: Some(authorization_header),
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
        authorization_header: None,
        x_api_key_header: Some(secret.to_owned()),
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
            profiles[0].authorization_header.as_deref(),
            Some("Bearer same-secret")
        );
        assert_eq!(profiles[0].x_api_key_header, None);
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
        assert_eq!(profiles[0].authorization_header, None);
        assert_eq!(
            profiles[0].x_api_key_header.as_deref(),
            Some("anthropic-secret")
        );
        assert_eq!(
            profiles[0].auth_cache_key.as_deref(),
            Some("x-api-key:anthropic-secret")
        );
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
        assert_eq!(profiles[0].authorization_header, None);
        assert_eq!(profiles[0].x_api_key_header, None);
        assert_eq!(profiles[0].auth_cache_key, None);
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
            profiles[0].authorization_header.as_deref(),
            Some("Bearer bedrock-bearer-token")
        );
        assert_eq!(profiles[1].id, "anonymous");
        assert_eq!(profiles[1].authorization_header, None);
        assert_eq!(profiles[1].x_api_key_header, None);
    }
}
