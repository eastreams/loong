use std::collections::HashSet;

use crate::config::ProviderConfig;

use super::provider_keyspace::build_provider_auth_profile_id;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProviderAuthProfile {
    pub(super) id: String,
    pub(super) authorization_header: Option<String>,
}

pub(super) fn resolve_provider_auth_profiles(
    provider: &ProviderConfig,
) -> Vec<ProviderAuthProfile> {
    let mut profiles = Vec::new();
    let mut seen_headers = HashSet::new();

    if let Some(token) = provider.oauth_access_token() {
        let authorization_header = format!("Bearer {token}");
        if seen_headers.insert(authorization_header.clone()) {
            profiles.push(ProviderAuthProfile {
                id: build_provider_auth_profile_id("oauth", token.as_str()),
                authorization_header: Some(authorization_header),
            });
        }
    }

    for api_key in provider.api_key_candidates() {
        let authorization_header = format!("Bearer {api_key}");
        if seen_headers.insert(authorization_header.clone()) {
            profiles.push(ProviderAuthProfile {
                id: build_provider_auth_profile_id("api_key", api_key.as_str()),
                authorization_header: Some(authorization_header),
            });
        }
    }

    if profiles.is_empty() {
        profiles.push(ProviderAuthProfile {
            id: "anonymous".to_owned(),
            authorization_header: None,
        });
    }

    profiles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ProviderConfig, ProviderKind};

    #[test]
    fn resolve_provider_auth_profiles_deduplicates_identical_bearer_headers() {
        let provider = ProviderConfig {
            kind: ProviderKind::Ollama,
            api_key: Some("same-secret".to_owned()),
            api_key_env: None,
            oauth_access_token: Some("same-secret".to_owned()),
            oauth_access_token_env: None,
            ..ProviderConfig::default()
        };

        let profiles = resolve_provider_auth_profiles(&provider);
        assert_eq!(profiles.len(), 1);
        assert_eq!(
            profiles[0].authorization_header.as_deref(),
            Some("Bearer same-secret")
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
    }
}
