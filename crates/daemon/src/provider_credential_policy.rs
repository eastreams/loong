use loongclaw_app as mvp;
use loongclaw_contracts::SecretRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderCredentialEnvField {
    ApiKey,
    OAuthAccessToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderCredentialEnvBinding {
    pub(crate) field: ProviderCredentialEnvField,
    pub(crate) env_name: String,
}

pub(crate) fn provider_credential_env_hints(provider: &mvp::config::ProviderConfig) -> Vec<String> {
    let support_facts = provider.support_facts();
    let auth_support = support_facts.auth;

    auth_support.hint_env_names
}

pub(crate) fn provider_credential_env_hint(
    provider: &mvp::config::ProviderConfig,
) -> Option<String> {
    let hints = provider_credential_env_hints(provider);
    hints.into_iter().next()
}

pub(crate) fn preferred_provider_credential_env_binding(
    provider: &mvp::config::ProviderConfig,
) -> Option<ProviderCredentialEnvBinding> {
    let configured_oauth = provider.configured_oauth_access_token_env_override();
    let configured_api_key = provider.configured_api_key_env_override();
    let configured_oauth = binding_for_env_name(
        ProviderCredentialEnvField::OAuthAccessToken,
        configured_oauth.as_deref(),
    );
    let configured_api_key = binding_for_env_name(
        ProviderCredentialEnvField::ApiKey,
        configured_api_key.as_deref(),
    );
    let default_oauth = binding_for_env_name(
        ProviderCredentialEnvField::OAuthAccessToken,
        provider.kind.default_oauth_access_token_env(),
    );
    let default_api_key = binding_for_env_name(
        ProviderCredentialEnvField::ApiKey,
        provider.kind.default_api_key_env(),
    );

    configured_oauth
        .or(configured_api_key)
        .or(default_oauth)
        .or(default_api_key)
}

pub(crate) fn configured_provider_credential_env_binding(
    provider: &mvp::config::ProviderConfig,
) -> Option<ProviderCredentialEnvBinding> {
    let configured_oauth = provider.configured_oauth_access_token_env_override();
    let configured_api_key = provider.configured_api_key_env_override();
    let configured_oauth = binding_for_env_name(
        ProviderCredentialEnvField::OAuthAccessToken,
        configured_oauth.as_deref(),
    );
    let configured_api_key = binding_for_env_name(
        ProviderCredentialEnvField::ApiKey,
        configured_api_key.as_deref(),
    );

    configured_oauth.or(configured_api_key)
}

pub(crate) fn provider_available_credential_env_binding(
    provider: &mvp::config::ProviderConfig,
) -> Option<ProviderCredentialEnvBinding> {
    let env_hints = provider_credential_env_hints(provider);

    for env_name in env_hints {
        let env_value = std::env::var(env_name.as_str()).ok();
        let has_value = env_value
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        if !has_value {
            continue;
        }

        let field = selected_provider_credential_env_field(provider, env_name.as_str());
        let binding = ProviderCredentialEnvBinding { field, env_name };

        return Some(binding);
    }

    None
}

pub(crate) fn apply_provider_credential_env_binding(
    provider: &mut mvp::config::ProviderConfig,
    binding: &ProviderCredentialEnvBinding,
) {
    let env_name = Some(binding.env_name.clone());

    match binding.field {
        ProviderCredentialEnvField::ApiKey => {
            provider.oauth_access_token = None;
            provider.clear_oauth_access_token_env_binding();
            provider.set_api_key_env_binding(env_name);
        }
        ProviderCredentialEnvField::OAuthAccessToken => {
            provider.api_key = None;
            provider.clear_api_key_env_binding();
            provider.set_oauth_access_token_env_binding(env_name);
        }
    }
}

pub(crate) fn provider_has_locally_available_credentials(
    provider: &mvp::config::ProviderConfig,
) -> bool {
    if provider.resolved_auth_secret().is_some() {
        return true;
    }

    provider_has_configured_auth_header(provider)
}

pub(crate) fn provider_is_credential_ready(provider: &mvp::config::ProviderConfig) -> bool {
    if !provider.requires_explicit_auth_configuration() {
        return true;
    }

    provider_has_locally_available_credentials(provider)
}

pub(crate) fn provider_has_inline_credential(provider: &mvp::config::ProviderConfig) -> bool {
    let api_key_present = secret_ref_has_inline_literal(provider.api_key.as_ref());
    let oauth_present = secret_ref_has_inline_literal(provider.oauth_access_token.as_ref());

    api_key_present || oauth_present
}

pub(crate) fn provider_has_configured_credential_env(
    provider: &mvp::config::ProviderConfig,
) -> bool {
    provider
        .configured_oauth_access_token_env_override()
        .is_some()
        || provider.configured_api_key_env_override().is_some()
}

pub(crate) fn selected_provider_credential_env_field(
    provider: &mvp::config::ProviderConfig,
    selected_env_name: &str,
) -> ProviderCredentialEnvField {
    let normalized = normalize_provider_credential_env_name(selected_env_name);
    let matches_oauth = normalized
        .as_deref()
        .is_some_and(|env_name| env_name_matches_oauth_binding(provider, env_name));
    let matches_api_key = normalized
        .as_deref()
        .is_some_and(|env_name| env_name_matches_api_key_binding(provider, env_name));

    match (matches_oauth, matches_api_key) {
        (true, false) => ProviderCredentialEnvField::OAuthAccessToken,
        (false, true) => ProviderCredentialEnvField::ApiKey,
        (true, true) => configured_provider_credential_env_binding(provider)
            .or_else(|| preferred_provider_credential_env_binding(provider))
            .map(|binding| binding.field)
            .unwrap_or(ProviderCredentialEnvField::ApiKey),
        (false, false) => ProviderCredentialEnvField::ApiKey,
    }
}

pub(crate) fn render_provider_credential_source_value(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = normalize_provider_credential_env_name(trimmed);
    normalized.or_else(|| Some("environment variable".to_owned()))
}

fn binding_for_env_name(
    field: ProviderCredentialEnvField,
    raw_env_name: Option<&str>,
) -> Option<ProviderCredentialEnvBinding> {
    let env_name = raw_env_name.and_then(normalize_provider_credential_env_name)?;
    Some(ProviderCredentialEnvBinding { field, env_name })
}

pub(crate) fn provider_has_configured_auth_header(provider: &mvp::config::ProviderConfig) -> bool {
    for header_name in ["authorization", "x-api-key"] {
        let header_value = provider.header_value(header_name);
        let has_value = header_value.is_some_and(|value| !value.trim().is_empty());
        if has_value {
            return true;
        }
    }

    false
}

fn env_name_matches_oauth_binding(provider: &mvp::config::ProviderConfig, env_name: &str) -> bool {
    let default_oauth = provider.kind.default_oauth_access_token_env();
    let oauth_aliases = provider.kind.oauth_access_token_env_aliases();
    let configured_oauth = provider
        .configured_oauth_access_token_env_override()
        .as_deref()
        .and_then(normalize_provider_credential_env_name);

    if default_oauth == Some(env_name) {
        return true;
    }
    if oauth_aliases.contains(&env_name) {
        return true;
    }

    configured_oauth.as_deref() == Some(env_name)
}

fn env_name_matches_api_key_binding(
    provider: &mvp::config::ProviderConfig,
    env_name: &str,
) -> bool {
    let default_api_key = provider.kind.default_api_key_env();
    let api_key_aliases = provider.kind.api_key_env_aliases();
    let configured_api_key = provider
        .configured_api_key_env_override()
        .as_deref()
        .and_then(normalize_provider_credential_env_name);

    if default_api_key == Some(env_name) {
        return true;
    }
    if api_key_aliases.contains(&env_name) {
        return true;
    }

    configured_api_key.as_deref() == Some(env_name)
}

fn provider_credential_env_name_is_safe(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }

    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.api_key = Some(SecretRef::Env {
        env: trimmed.to_owned(),
    });
    config.provider.api_key_env = None;

    config.validate().is_ok()
}

fn normalize_provider_credential_env_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let is_empty = trimmed.is_empty();
    if is_empty {
        return None;
    }

    let is_safe = provider_credential_env_name_is_safe(trimmed);
    if !is_safe {
        return None;
    }

    Some(trimmed.to_owned())
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

    #[test]
    fn provider_credential_env_hints_prioritize_oauth_defaults() {
        let provider = mvp::config::ProviderConfig::default();
        let hints = provider_credential_env_hints(&provider);

        assert_eq!(
            hints.first().map(String::as_str),
            Some("OPENAI_CODEX_OAUTH_TOKEN")
        );
        assert!(
            hints.contains(&"OPENAI_API_KEY".to_owned()),
            "api key fallback should still be visible: {hints:?}"
        );
    }

    #[test]
    fn selected_provider_credential_env_field_prefers_configured_binding_on_overlap() {
        let mut provider =
            mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::Openai);
        let shared_env = "OPENAI_SHARED_TOKEN".to_owned();
        provider.oauth_access_token_env = Some(shared_env.clone());
        provider.api_key_env = Some(shared_env.clone());

        let field = selected_provider_credential_env_field(&provider, shared_env.as_str());

        assert_eq!(field, ProviderCredentialEnvField::OAuthAccessToken);
    }

    #[test]
    fn render_provider_credential_source_value_redacts_invalid_env_names() {
        let rendered = render_provider_credential_source_value(Some("sk-live-direct-secret-value"));

        assert_eq!(rendered.as_deref(), Some("environment variable"));
    }

    #[test]
    fn provider_has_locally_available_credentials_accepts_x_api_key_providers() {
        let mut env = mvp::test_support::ScopedEnv::new();
        env.set("ANTHROPIC_API_KEY", "test-anthropic-key");
        let provider = mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::Anthropic,
            ..mvp::config::ProviderConfig::default()
        };

        let has_credentials = provider_has_locally_available_credentials(&provider);

        assert!(has_credentials);
    }

    #[test]
    fn provider_has_locally_available_credentials_accepts_header_only_x_api_key_providers() {
        let mut provider =
            mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::Anthropic);
        provider.api_key = None;
        provider.headers.insert(
            "x-api-key".to_owned(),
            "test-anthropic-header-key".to_owned(),
        );

        let has_credentials = provider_has_locally_available_credentials(&provider);

        assert!(has_credentials);
    }

    #[test]
    fn provider_has_configured_auth_header_accepts_header_only_auth() {
        let mut provider =
            mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::Custom);
        provider.headers.insert(
            "authorization".to_owned(),
            "Bearer test-custom-token".to_owned(),
        );

        let has_configured_auth = provider_has_configured_auth_header(&provider);

        assert!(has_configured_auth);
    }

    #[test]
    fn provider_available_credential_env_binding_preserves_oauth_env_bindings() {
        let mut env = mvp::test_support::ScopedEnv::new();
        env.set("OPENAI_SESSION_TOKEN", "test-openai-session-token");
        let provider = mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::Openai,
            oauth_access_token: Some(SecretRef::Env {
                env: "OPENAI_SESSION_TOKEN".to_owned(),
            }),
            ..mvp::config::ProviderConfig::default()
        };

        let binding = provider_available_credential_env_binding(&provider);

        assert_eq!(
            binding,
            Some(ProviderCredentialEnvBinding {
                field: ProviderCredentialEnvField::OAuthAccessToken,
                env_name: "OPENAI_SESSION_TOKEN".to_owned(),
            })
        );
    }

    #[test]
    fn provider_is_credential_ready_accepts_auth_optional_providers() {
        let provider =
            mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::Ollama);

        let is_ready = provider_is_credential_ready(&provider);

        assert!(is_ready);
    }

    #[test]
    fn provider_available_credential_env_binding_uses_first_populated_env_hint() {
        let mut env = mvp::test_support::ScopedEnv::new();
        env.set("OPENAI_API_KEY", "test-openai-key");
        let provider =
            mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::Openai);

        let binding = provider_available_credential_env_binding(&provider);

        assert_eq!(
            binding,
            Some(ProviderCredentialEnvBinding {
                field: ProviderCredentialEnvField::ApiKey,
                env_name: "OPENAI_API_KEY".to_owned(),
            })
        );
    }
}
