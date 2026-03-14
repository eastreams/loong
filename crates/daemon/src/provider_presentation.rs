use loongclaw_app as mvp;

pub(crate) fn guided_provider_label(kind: mvp::config::ProviderKind) -> &'static str {
    kind.display_name()
}

pub(crate) fn provider_choice_label(kind: mvp::config::ProviderKind) -> String {
    format!("{} [{}]", guided_provider_label(kind), kind.as_str())
}

pub(crate) fn provider_identity_summary(config: &mvp::config::ProviderConfig) -> String {
    provider_identity_summary_with_credential_state(config, provider_credential_state(config))
}

pub(crate) fn provider_identity_summary_with_credential_state(
    config: &mvp::config::ProviderConfig,
    credential_state: &str,
) -> String {
    format!(
        "{} · {} · {}",
        guided_provider_label(config.kind),
        config.model,
        credential_state
    )
}

pub(crate) fn provider_credential_state(config: &mvp::config::ProviderConfig) -> &'static str {
    if config.authorization_header().is_some() {
        "credentials resolved"
    } else {
        "credential still missing"
    }
}
