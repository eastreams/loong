use loong_app as mvp;
use loong_contracts::SecretRef;

pub(crate) const PROVIDER_CREDENTIALS_LABEL: &str = "provider credentials";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderCredentialStatusKind {
    InlineOauth,
    InlineApiKey,
    Optional,
    Available,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderCredentialStatus {
    pub(crate) kind: ProviderCredentialStatusKind,
    pub(crate) detail: String,
}

impl ProviderCredentialStatus {
    pub(crate) fn is_ready(&self) -> bool {
        !matches!(self.kind, ProviderCredentialStatusKind::Missing)
    }
}

pub(crate) fn provider_credential_status(
    provider: &mvp::config::ProviderConfig,
    has_available_credentials: bool,
) -> ProviderCredentialStatus {
    let auth_support = provider.support_facts().auth;

    if secret_ref_has_inline_literal(provider.oauth_access_token.as_ref()) {
        return ProviderCredentialStatus {
            kind: ProviderCredentialStatusKind::InlineOauth,
            detail: "inline oauth access token configured".to_owned(),
        };
    }

    if secret_ref_has_inline_literal(provider.api_key.as_ref()) {
        return ProviderCredentialStatus {
            kind: ProviderCredentialStatusKind::InlineApiKey,
            detail: "inline api key configured".to_owned(),
        };
    }

    if !auth_support.requires_explicit_configuration {
        return ProviderCredentialStatus {
            kind: ProviderCredentialStatusKind::Optional,
            detail: "provider credentials are optional for this provider".to_owned(),
        };
    }

    if has_available_credentials {
        let detail = crate::provider_credential_policy::provider_credential_env_hint(provider)
            .map(|env_name| format!("{env_name} is available"))
            .unwrap_or_else(|| "provider credentials are available".to_owned());
        return ProviderCredentialStatus {
            kind: ProviderCredentialStatusKind::Available,
            detail,
        };
    }

    ProviderCredentialStatus {
        kind: ProviderCredentialStatusKind::Missing,
        detail: auth_support.missing_configuration_message,
    }
}

fn secret_ref_has_inline_literal(secret_ref: Option<&SecretRef>) -> bool {
    let Some(secret_ref) = secret_ref else {
        return false;
    };

    secret_ref.inline_literal_value().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedEnv;

    #[test]
    fn provider_credential_status_prefers_inline_details_before_env() {
        let mut config = mvp::config::LoongConfig::default();
        config.provider.oauth_access_token = Some(SecretRef::Inline("token".to_owned()));

        let status = provider_credential_status(&config.provider, true);

        assert_eq!(status.kind, ProviderCredentialStatusKind::InlineOauth);
        assert_eq!(status.detail, "inline oauth access token configured");
    }

    #[test]
    fn provider_credential_status_renders_missing_auth_guidance_for_volcengine() {
        let mut config = mvp::config::LoongConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Volcengine;
        config.provider.api_key = None;
        config.provider.api_key_env = None;
        config.provider.oauth_access_token = None;
        config.provider.oauth_access_token_env = None;
        let mut env = ScopedEnv::new();
        for env_name in config.provider.auth_hint_env_names() {
            env.remove(env_name);
        }

        let status = provider_credential_status(&config.provider, false);

        assert_eq!(status.kind, ProviderCredentialStatusKind::Missing);
        assert!(status.detail.contains("ARK_API_KEY"));
        assert!(
            status
                .detail
                .contains("Authorization: Bearer <ARK_API_KEY>")
        );
    }

    #[test]
    fn provider_credential_status_reports_env_hint_when_credentials_are_available() {
        let mut env = ScopedEnv::new();
        env.set("ANTHROPIC_API_KEY", "test-anthropic-key");
        let mut config = mvp::config::LoongConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Anthropic;
        config.provider.api_key = None;
        config.provider.api_key_env = None;
        config.provider.oauth_access_token = None;
        config.provider.oauth_access_token_env = None;

        let status = provider_credential_status(&config.provider, true);

        assert_eq!(status.kind, ProviderCredentialStatusKind::Available);
        assert_eq!(status.detail, "ANTHROPIC_API_KEY is available");
    }
}
