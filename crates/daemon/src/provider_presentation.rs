use loongclaw_app as mvp;

pub fn guided_provider_label(kind: mvp::config::ProviderKind) -> &'static str {
    kind.display_name()
}

pub fn provider_choice_label(profile_id: &str, kind: mvp::config::ProviderKind) -> String {
    format!("{} [{profile_id}]", guided_provider_label(kind))
}

pub fn provider_identity_summary(config: &mvp::config::ProviderConfig) -> String {
    provider_identity_summary_with_credential_state(config, provider_credential_state(config))
}

pub fn active_provider_label(config: &mvp::config::LoongClawConfig) -> String {
    config
        .active_provider_id()
        .and_then(|profile_id| config.providers.get(profile_id))
        .map(|profile| guided_provider_label(profile.provider.kind).to_owned())
        .unwrap_or_else(|| guided_provider_label(config.provider.kind).to_owned())
}

pub fn active_provider_detail_label(config: &mvp::config::LoongClawConfig) -> String {
    let profile_id = config
        .active_provider_id()
        .unwrap_or(config.provider.kind.profile().id);
    let kind = config
        .providers
        .get(profile_id)
        .map(|profile| profile.provider.kind)
        .unwrap_or(config.provider.kind);
    format!("{} [{profile_id}]", guided_provider_label(kind))
}

pub fn saved_provider_profile_ids(config: &mvp::config::LoongClawConfig) -> Vec<String> {
    if config.providers.is_empty() {
        return vec![
            config
                .active_provider_id()
                .unwrap_or(config.provider.kind.profile().id)
                .to_owned(),
        ];
    }
    let mut profile_ids = config.providers.keys().cloned().collect::<Vec<_>>();
    if let Some(active_provider_id) = config.active_provider_id()
        && let Some(active_index) = profile_ids
            .iter()
            .position(|profile_id| profile_id == active_provider_id)
    {
        let active_provider = profile_ids.remove(active_index);
        profile_ids.insert(0, active_provider);
    }
    profile_ids
}

pub fn render_provider_profile_state_lines(
    config: &mvp::config::LoongClawConfig,
    width: usize,
    single_provider_prefix: Option<&str>,
) -> Vec<String> {
    let display_lines = provider_profile_state_display_lines(config, single_provider_prefix);

    display_lines
        .into_iter()
        .flat_map(|line| mvp::presentation::render_wrapped_display_line(&line, width))
        .collect()
}

pub fn provider_profile_state_display_lines(
    config: &mvp::config::LoongClawConfig,
    single_provider_prefix: Option<&str>,
) -> Vec<String> {
    render_provider_profile_state_lines_from_parts(
        &active_provider_label(config),
        &saved_provider_profile_ids(config),
        single_provider_prefix,
    )
}

pub fn render_provider_profile_state_lines_from_parts(
    active_provider_label: &str,
    saved_provider_profiles: &[String],
    single_provider_prefix: Option<&str>,
) -> Vec<String> {
    if saved_provider_profiles.len() > 1 {
        let mut lines = Vec::new();
        lines.push(format!("- active provider: {active_provider_label}"));
        lines.push(format!(
            "- saved provider profiles: {}",
            saved_provider_profiles.join(", ")
        ));
        return lines;
    }

    single_provider_prefix
        .map(|prefix| vec![format!("{prefix}{active_provider_label}")])
        .unwrap_or_default()
}

pub fn provider_identity_summary_with_credential_state(
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

pub fn provider_credential_state(config: &mvp::config::ProviderConfig) -> &'static str {
    if config.authorization_header().is_some() {
        "credentials resolved"
    } else {
        "credential still missing"
    }
}
