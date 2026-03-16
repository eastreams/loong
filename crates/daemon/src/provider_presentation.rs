use loongclaw_app as mvp;

pub(crate) fn guided_provider_label(kind: mvp::config::ProviderKind) -> &'static str {
    kind.display_name()
}

pub(crate) fn provider_choice_label(profile_id: &str, kind: mvp::config::ProviderKind) -> String {
    format!("{} [{profile_id}]", guided_provider_label(kind))
}

pub(crate) fn provider_identity_summary(config: &mvp::config::ProviderConfig) -> String {
    provider_identity_summary_with_credential_state(config, provider_credential_state(config))
}

pub(crate) fn active_provider_label(config: &mvp::config::LoongClawConfig) -> String {
    config
        .active_provider_id()
        .and_then(|profile_id| config.providers.get(profile_id))
        .map(|profile| guided_provider_label(profile.provider.kind).to_owned())
        .unwrap_or_else(|| guided_provider_label(config.provider.kind).to_owned())
}

pub(crate) fn active_provider_detail_label(config: &mvp::config::LoongClawConfig) -> String {
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

pub(crate) fn saved_provider_profile_ids(config: &mvp::config::LoongClawConfig) -> Vec<String> {
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

pub(crate) fn render_provider_profile_state_lines(
    config: &mvp::config::LoongClawConfig,
    width: usize,
    single_provider_prefix: Option<&str>,
) -> Vec<String> {
    render_provider_profile_state_lines_from_parts(
        &active_provider_label(config),
        &saved_provider_profile_ids(config),
        width,
        single_provider_prefix,
    )
}

pub(crate) fn render_provider_profile_state_lines_from_parts(
    active_provider_label: &str,
    saved_provider_profiles: &[String],
    width: usize,
    single_provider_prefix: Option<&str>,
) -> Vec<String> {
    if saved_provider_profiles.len() > 1 {
        let mut lines = mvp::presentation::render_wrapped_text_line(
            "- active provider: ",
            active_provider_label,
            width,
        );
        let profiles = saved_provider_profiles
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        lines.extend(mvp::presentation::render_wrapped_csv_line(
            "- saved provider profiles: ",
            &profiles,
            width,
        ));
        return lines;
    }

    single_provider_prefix
        .map(|prefix| {
            mvp::presentation::render_wrapped_text_line(prefix, active_provider_label, width)
        })
        .unwrap_or_default()
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
