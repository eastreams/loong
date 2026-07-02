use super::*;

fn build_onboard_review_candidate_with_guidance(
    config: &mvp::config::LoongConfig,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
) -> crate::migration::ImportCandidate {
    crate::migration::build_import_candidate(
        crate::migration::ImportSourceKind::CurrentSetup,
        crate::source_presentation::current_onboarding_draft_source_label().to_owned(),
        config.clone(),
        crate::migration::resolve_channel_import_readiness_from_config,
        workspace_guidance.to_vec(),
    )
    .unwrap_or_else(|| crate::migration::ImportCandidate {
        source_kind: crate::migration::ImportSourceKind::CurrentSetup,
        source: crate::source_presentation::current_onboarding_draft_source_label().to_owned(),
        config: config.clone(),
        surfaces: Vec::new(),
        domains: Vec::new(),
        channel_candidates: Vec::new(),
        workspace_guidance: workspace_guidance.to_vec(),
    })
}

pub fn render_onboard_review_lines_with_guidance(
    config: &mvp::config::LoongConfig,
    import_source: Option<&str>,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
    width: usize,
) -> Vec<String> {
    render_onboard_review_lines_with_guidance_and_style(
        config,
        import_source,
        workspace_guidance,
        None,
        width,
        ReviewFlowStyle::Guided(GuidedPromptPath::NativePromptPack),
        false,
    )
}

pub fn render_current_setup_review_lines_with_guidance(
    config: &mvp::config::LoongConfig,
    import_source: Option<&str>,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
    width: usize,
) -> Vec<String> {
    render_onboard_review_lines_with_guidance_and_style(
        config,
        import_source,
        workspace_guidance,
        None,
        width,
        ReviewFlowStyle::QuickCurrentSetup,
        false,
    )
}

pub fn render_detected_setup_review_lines_with_guidance(
    config: &mvp::config::LoongConfig,
    import_source: Option<&str>,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
    width: usize,
) -> Vec<String> {
    render_onboard_review_lines_with_guidance_and_style(
        config,
        import_source,
        workspace_guidance,
        None,
        width,
        ReviewFlowStyle::QuickDetectedSetup,
        false,
    )
}

fn channel_candidates_match(
    left: &[crate::migration::ChannelCandidate],
    right: &[crate::migration::ChannelCandidate],
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.id == right.id
                && left.label == right.label
                && left.status == right.status
                && left.summary == right.summary
        })
}

fn should_preserve_review_domain(
    kind: crate::migration::SetupDomainKind,
    config: &mvp::config::LoongConfig,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
    selected_candidate: &ImportCandidate,
    channels_unchanged: bool,
) -> bool {
    match kind {
        crate::migration::SetupDomainKind::Provider => {
            provider_matches_for_review(&selected_candidate.config.provider, &config.provider)
        }
        crate::migration::SetupDomainKind::Channels => channels_unchanged,
        crate::migration::SetupDomainKind::Cli => selected_candidate.config.cli == config.cli,
        crate::migration::SetupDomainKind::Memory => {
            selected_candidate.config.memory == config.memory
        }
        crate::migration::SetupDomainKind::Tools => selected_candidate.config.tools == config.tools,
        crate::migration::SetupDomainKind::WorkspaceGuidance => {
            selected_candidate.workspace_guidance.as_slice() == workspace_guidance
        }
    }
}

pub(crate) fn provider_matches_for_review(
    left: &mvp::config::ProviderConfig,
    right: &mvp::config::ProviderConfig,
) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();

    left.api_key = None;
    left.api_key_env = None;
    left.api_key_env_explicit = false;
    left.oauth_access_token = None;
    left.oauth_access_token_env = None;
    left.oauth_access_token_env_explicit = false;

    right.api_key = None;
    right.api_key_env = None;
    right.api_key_env_explicit = false;
    right.oauth_access_token = None;
    right.oauth_access_token_env = None;
    right.oauth_access_token_env_explicit = false;

    left == right
}

pub(crate) fn build_onboard_review_candidate_with_selected_context(
    config: &mvp::config::LoongConfig,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
    selected_candidate: Option<&ImportCandidate>,
) -> crate::migration::ImportCandidate {
    let draft_candidate = build_onboard_review_candidate_with_guidance(config, workspace_guidance);
    let Some(selected_candidate) = selected_candidate else {
        return draft_candidate;
    };
    if selected_candidate.config == *config
        && selected_candidate.workspace_guidance.as_slice() == workspace_guidance
    {
        return migration_candidate_for_onboard_display(selected_candidate);
    }

    let channels_unchanged = channel_candidates_match(
        &draft_candidate.channel_candidates,
        &selected_candidate.channel_candidates,
    );
    let mut review_candidate = draft_candidate;

    if channels_unchanged {
        review_candidate.channel_candidates = selected_candidate.channel_candidates.clone();
    }
    if selected_candidate.workspace_guidance.as_slice() == workspace_guidance {
        review_candidate.workspace_guidance = selected_candidate.workspace_guidance.clone();
    }

    for domain in &mut review_candidate.domains {
        if should_preserve_review_domain(
            domain.kind,
            config,
            workspace_guidance,
            selected_candidate,
            channels_unchanged,
        ) {
            if let Some(selected_domain) = selected_candidate
                .domains
                .iter()
                .find(|selected_domain| selected_domain.kind == domain.kind)
            {
                *domain = selected_domain.clone();
            }
        } else {
            domain.decision = Some(crate::migration::types::PreviewDecision::AdjustedInSession);
        }
    }

    review_candidate
}

pub(crate) fn render_onboard_review_lines_with_guidance_and_style(
    config: &mvp::config::LoongConfig,
    import_source: Option<&str>,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
    selected_candidate: Option<&ImportCandidate>,
    width: usize,
    flow_style: ReviewFlowStyle,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboard_review_screen_spec(
        config,
        import_source,
        workspace_guidance,
        selected_candidate,
        flow_style,
    );

    render_onboard_screen_spec(&spec, width, color_enabled)
}

fn build_onboard_review_screen_spec(
    config: &mvp::config::LoongConfig,
    import_source: Option<&str>,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
    selected_candidate: Option<&ImportCandidate>,
    flow_style: ReviewFlowStyle,
) -> TuiScreenSpec {
    let mut sections = Vec::new();

    if let Some(source) = import_source {
        let starting_point_label = onboard_starting_point_label(None, source);
        let starting_point_lines = vec![onboard_display_line(
            "- starting point: ",
            &starting_point_label,
        )];
        let starting_point_section = TuiSectionSpec::Narrative {
            title: Some("starting point".to_owned()),
            lines: starting_point_lines,
        };

        sections.push(starting_point_section);
    }

    let configuration_lines = build_onboard_review_digest_display_lines(config);
    let configuration_section = TuiSectionSpec::Narrative {
        title: Some("configuration".to_owned()),
        lines: configuration_lines,
    };

    sections.push(configuration_section);

    let review_candidate = build_onboard_review_candidate_with_selected_context(
        config,
        workspace_guidance,
        selected_candidate,
    );
    let draft_source_lines =
        crate::migration::render::candidate_preview_display_lines(&review_candidate);
    let draft_source_section = TuiSectionSpec::Narrative {
        title: Some("draft source".to_owned()),
        lines: draft_source_lines,
    };

    sections.push(draft_source_section);

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some(flow_style.header_subtitle().to_owned()),
        title: Some("review setup".to_owned()),
        progress_line: Some(flow_style.progress_line()),
        intro_lines: Vec::new(),
        sections,
        choices: Vec::new(),
        footer_lines: Vec::new(),
    }
}

pub(crate) fn onboard_display_line(prefix: &str, value: &str) -> String {
    format!("{prefix}{value}")
}

pub(crate) fn build_onboard_review_digest_display_lines(
    config: &mvp::config::LoongConfig,
) -> Vec<String> {
    let mut lines = crate::provider::presentation::provider_profile_state_display_lines(
        config,
        Some("- provider: "),
    );
    lines.push(onboard_display_line("- model: ", &config.provider.model));
    lines.push(onboard_display_line(
        "- transport: ",
        &config.provider.transport_readiness().summary,
    ));

    if let Some(provider_endpoint) = config.provider.region_endpoint_note() {
        lines.push(onboard_display_line(
            "- provider endpoint: ",
            &provider_endpoint,
        ));
    }

    if let Some(credential_line) = render_onboard_review_credential_line(&config.provider) {
        lines.push(credential_line);
    }

    let prompt_mode = summarize_prompt_mode(config);
    lines.push(onboard_display_line("- prompt mode: ", &prompt_mode));

    if config.cli.uses_native_prompt_pack() {
        lines.push(onboard_display_line(
            "- personality: ",
            prompt_personality_id(config.cli.resolved_personality()),
        ));

        if let Some(prompt_addendum) = summarize_prompt_addendum(config) {
            lines.push(onboard_display_line(
                "- prompt addendum: ",
                &prompt_addendum,
            ));
        }
    }

    lines.push(onboard_display_line(
        "- memory profile: ",
        memory_profile_id(config.memory.profile),
    ));

    let web_search_provider =
        query_search_provider_display_name(config.tools.web_search.default_provider.as_str());
    lines.push(onboard_display_line("- web search: ", &web_search_provider));

    if let Some(web_search_credential) =
        summarize_query_search_credential(config, config.tools.web_search.default_provider.as_str())
    {
        let credential_prefix = format!("- {}: ", web_search_credential.label);
        lines.push(onboard_display_line(
            &credential_prefix,
            &web_search_credential.value,
        ));
    }

    push_onboard_review_enabled_channel_lines(&mut lines, config);

    lines
}

fn push_onboard_review_enabled_channel_lines(
    lines: &mut Vec<String>,
    config: &mvp::config::LoongConfig,
) {
    let runtime_backed_channels = config.enabled_runtime_backed_channel_ids();
    if !runtime_backed_channels.is_empty() {
        lines.push(onboard_display_line(
            "- runtime-backed channels: ",
            &runtime_backed_channels.join(", "),
        ));
    }

    let plugin_backed_channels = config.enabled_plugin_backed_channel_ids();
    if !plugin_backed_channels.is_empty() {
        lines.push(onboard_display_line(
            "- plugin-backed channels: ",
            &plugin_backed_channels.join(", "),
        ));
    }

    let outbound_only_channels = config.enabled_outbound_only_channel_ids();
    if !outbound_only_channels.is_empty() {
        lines.push(onboard_display_line(
            "- outbound-only channels: ",
            &outbound_only_channels.join(", "),
        ));
    }

    let remaining_channels = enabled_channel_ids(config)
        .into_iter()
        .filter(|channel| channel != "cli")
        .filter(|channel| {
            !runtime_backed_channels.contains(channel)
                && !plugin_backed_channels.contains(channel)
                && !outbound_only_channels.contains(channel)
        })
        .collect::<Vec<_>>();
    if !remaining_channels.is_empty() {
        lines.push(onboard_display_line(
            "- channels: ",
            &remaining_channels.join(", "),
        ));
    }
}

pub(crate) fn render_onboard_review_credential_line(
    provider: &mvp::config::ProviderConfig,
) -> Option<String> {
    summarize_provider_credential(provider)
        .map(|credential| format!("- {}: {}", credential.label, credential.value))
}

pub fn summarize_prompt_mode(config: &mvp::config::LoongConfig) -> String {
    if config.cli.uses_native_prompt_pack() {
        return "native prompt pack".to_owned();
    }

    "inline system prompt override".to_owned()
}

pub fn summarize_prompt_addendum(config: &mvp::config::LoongConfig) -> Option<String> {
    config
        .cli
        .system_prompt_addendum
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub fn summarize_provider_credential(
    provider: &mvp::config::ProviderConfig,
) -> Option<OnboardingCredentialSummary> {
    if secret_ref_has_inline_literal(provider.oauth_access_token.as_ref()) {
        return Some(OnboardingCredentialSummary {
            label: "credential",
            value: "inline oauth token".to_owned(),
        });
    }
    if let Some(configured_env) =
        provider_credential_policy::render_configured_provider_credential_source_value(provider)
    {
        return Some(OnboardingCredentialSummary {
            label: "credential source",
            value: configured_env,
        });
    }
    if secret_ref_has_inline_literal(provider.api_key.as_ref()) {
        return Some(OnboardingCredentialSummary {
            label: "credential",
            value: "inline api key".to_owned(),
        });
    }
    provider_credential_policy::preferred_provider_credential_env_binding(provider)
        .and_then(|binding| {
            provider_credential_policy::render_provider_credential_source_value(Some(
                binding.env_name.as_str(),
            ))
        })
        .map(|credential_env| OnboardingCredentialSummary {
            label: "credential source",
            value: credential_env,
        })
}

pub(crate) fn provider_supports_blank_api_key_env(config: &mvp::config::LoongConfig) -> bool {
    provider_credential_policy::provider_has_inline_credential(&config.provider)
        || provider_credential_policy::provider_has_configured_credential_env(&config.provider)
}
