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
    let mut hints = Vec::new();
    let configured_oauth = provider.oauth_access_token_env.as_deref();
    let configured_api_key = provider.api_key_env.as_deref();
    let default_oauth = provider.kind.default_oauth_access_token_env();
    let default_api_key = provider.kind.default_api_key_env();

    push_provider_credential_env_hint(&mut hints, configured_oauth);
    push_provider_credential_env_hint(&mut hints, configured_api_key);
    push_provider_credential_env_hint(&mut hints, default_oauth);
    push_provider_credential_env_hint(&mut hints, default_api_key);

    hints
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
    let configured_oauth = binding_for_env_name(
        ProviderCredentialEnvField::OAuthAccessToken,
        provider.oauth_access_token_env.as_deref(),
    );
    let configured_api_key = binding_for_env_name(
        ProviderCredentialEnvField::ApiKey,
        provider.api_key_env.as_deref(),
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
    let configured_oauth = binding_for_env_name(
        ProviderCredentialEnvField::OAuthAccessToken,
        provider.oauth_access_token_env.as_deref(),
    );
    let configured_api_key = binding_for_env_name(
        ProviderCredentialEnvField::ApiKey,
        provider.api_key_env.as_deref(),
    );

    configured_oauth.or(configured_api_key)
}

pub(crate) fn provider_has_inline_credential(provider: &mvp::config::ProviderConfig) -> bool {
    let api_key_present = secret_ref_has_inline_literal(provider.api_key.as_ref());
    let oauth_present = secret_ref_has_inline_literal(provider.oauth_access_token.as_ref());

    api_key_present || oauth_present
}

pub(crate) fn provider_has_configured_credential_env(
    provider: &mvp::config::ProviderConfig,
) -> bool {
    let oauth_env_present = provider
        .oauth_access_token_env
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let api_key_env_present = provider
        .api_key_env
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());

    oauth_env_present || api_key_env_present
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

fn env_name_matches_oauth_binding(provider: &mvp::config::ProviderConfig, env_name: &str) -> bool {
    let default_oauth = provider.kind.default_oauth_access_token_env();
    let oauth_aliases = provider.kind.oauth_access_token_env_aliases();
    let configured_oauth = provider
        .oauth_access_token_env
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
        .api_key_env
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

fn push_provider_credential_env_hint(hints: &mut Vec<String>, maybe_env_name: Option<&str>) {
    let normalized = maybe_env_name.and_then(normalize_provider_credential_env_name);
    let Some(env_name) = normalized else {
        return;
    };

    let already_present = hints.iter().any(|existing| existing == &env_name);
    if already_present {
        return;
    }

    hints.push(env_name);
}

fn provider_credential_env_name_is_safe(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }

    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.api_key = None;
    config.provider.api_key_env = Some(trimmed.to_owned());

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
}
