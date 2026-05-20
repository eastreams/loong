use super::*;
pub fn render_onboard_entry_screen_lines(
    current_setup_state: crate::migration::CurrentSetupState,
    current_candidate: Option<&ImportCandidate>,
    import_candidates: &[ImportCandidate],
    options: &[OnboardEntryOption],
    workspace_root: Option<&Path>,
    width: usize,
) -> Vec<String> {
    render_onboard_entry_screen_lines_with_style(
        current_setup_state,
        current_candidate,
        import_candidates,
        options,
        workspace_root,
        width,
        false,
    )
}

pub(super) fn render_onboard_entry_screen_lines_with_style(
    current_setup_state: crate::migration::CurrentSetupState,
    current_candidate: Option<&ImportCandidate>,
    import_candidates: &[ImportCandidate],
    options: &[OnboardEntryOption],
    workspace_root: Option<&Path>,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboard_entry_screen_spec(
        current_setup_state,
        current_candidate,
        import_candidates,
        options,
        workspace_root,
        false,
    );

    render_onboard_screen_spec(&spec, width, color_enabled)
}

pub(super) fn render_onboard_entry_interactive_screen_lines_with_style(
    current_setup_state: crate::migration::CurrentSetupState,
    current_candidate: Option<&ImportCandidate>,
    import_candidates: &[ImportCandidate],
    options: &[OnboardEntryOption],
    workspace_root: Option<&Path>,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboard_entry_screen_spec(
        current_setup_state,
        current_candidate,
        import_candidates,
        options,
        workspace_root,
        true,
    );

    render_onboard_screen_spec(&spec, width, color_enabled)
}

fn build_onboard_entry_screen_spec(
    current_setup_state: crate::migration::CurrentSetupState,
    current_candidate: Option<&ImportCandidate>,
    import_candidates: &[ImportCandidate],
    options: &[OnboardEntryOption],
    workspace_root: Option<&Path>,
    interactive: bool,
) -> TuiScreenSpec {
    let recommended_plan_available = import_candidates.iter().any(|candidate| {
        candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan
    });
    let detected_settings_lines = render_detected_settings_digest_lines(
        current_setup_state,
        current_candidate,
        import_candidates,
        workspace_root,
        recommended_plan_available,
    );
    let detected_settings_section = TuiSectionSpec::Narrative {
        title: Some(crate::onboard_presentation::detected_settings_section_heading().to_owned()),
        lines: detected_settings_lines,
    };

    let mut sections = vec![detected_settings_section];

    if !options.is_empty() {
        let entry_choice_section = TuiSectionSpec::Narrative {
            title: Some(crate::onboard_presentation::entry_choice_section_heading().to_owned()),
            lines: Vec::new(),
        };

        sections.push(entry_choice_section);
    }

    let choices = if interactive {
        Vec::new()
    } else {
        let screen_options = build_onboard_entry_screen_options(options);
        tui_choices_from_screen_options(&screen_options)
    };

    let footer_lines = if interactive {
        append_escape_cancel_hint(Vec::<String>::new())
    } else {
        let default_footer_lines = render_onboard_entry_default_choice_footer_line(options)
            .into_iter()
            .collect::<Vec<_>>();

        append_escape_cancel_hint(default_footer_lines)
    };

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some("guided setup for provider, channels, and workspace guidance".to_owned()),
        title: None,
        progress_line: None,
        intro_lines: Vec::new(),
        sections,
        choices,
        footer_lines,
    }
}

fn render_onboard_entry_default_choice_footer_line(
    options: &[OnboardEntryOption],
) -> Option<String> {
    let default_choice = default_onboard_entry_choice(options);
    let default_index = options
        .iter()
        .position(|option| option.choice == default_choice)
        .map(|index| index + 1)?;
    let description = crate::onboard_presentation::entry_default_choice_description(
        onboard_entry_choice_kind(default_choice),
    );
    Some(render_default_choice_footer_line(
        &default_index.to_string(),
        description,
    ))
}

const fn onboard_entry_choice_kind(
    choice: OnboardEntryChoice,
) -> crate::onboard_presentation::EntryChoiceKind {
    match choice {
        OnboardEntryChoice::ContinueCurrentSetup => {
            crate::onboard_presentation::EntryChoiceKind::CurrentSetup
        }
        OnboardEntryChoice::ImportDetectedSetup => {
            crate::onboard_presentation::EntryChoiceKind::DetectedSetup
        }
        OnboardEntryChoice::StartFresh => crate::onboard_presentation::EntryChoiceKind::StartFresh,
    }
}

fn collect_detected_workspace_guidance_files<'a>(
    current_candidate: impl Iterator<Item = &'a ImportCandidate>,
    import_candidates: &'a [ImportCandidate],
) -> Vec<String> {
    let mut files = std::collections::BTreeSet::new();
    for candidate in current_candidate.chain(import_candidates.iter()) {
        for guidance in &candidate.workspace_guidance {
            if let Some(name) = Path::new(&guidance.path).file_name() {
                files.insert(name.to_string_lossy().to_string());
            }
        }
    }
    files.into_iter().collect()
}

fn recommended_starting_point_candidate(
    import_candidates: &[ImportCandidate],
) -> Option<&ImportCandidate> {
    import_candidates.iter().find(|candidate| {
        candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan
    })
}

fn collect_detected_coverage_kinds(
    candidates: impl IntoIterator<Item = impl std::borrow::Borrow<ImportCandidate>>,
) -> std::collections::BTreeSet<crate::migration::SetupDomainKind> {
    let mut kinds = std::collections::BTreeSet::new();
    for candidate in candidates {
        let candidate = candidate.borrow();
        for domain in &candidate.domains {
            if domain.status != crate::migration::PreviewStatus::Unavailable {
                kinds.insert(domain.kind);
            }
        }
        if candidate
            .channel_candidates
            .iter()
            .any(|channel| channel.status != crate::migration::PreviewStatus::Unavailable)
        {
            kinds.insert(crate::migration::SetupDomainKind::Channels);
        }
        if !candidate.workspace_guidance.is_empty() {
            kinds.insert(crate::migration::SetupDomainKind::WorkspaceGuidance);
        }
    }
    kinds
}

fn collect_detected_channel_labels(import_candidates: &[ImportCandidate]) -> Vec<String> {
    let mut labels = std::collections::BTreeSet::new();
    for candidate in import_candidates {
        for channel in &candidate.channel_candidates {
            if channel.status != crate::migration::PreviewStatus::Unavailable {
                labels.insert(channel.label.to_owned());
            }
        }
    }
    labels.into_iter().collect()
}

fn detected_reusable_source_count_for_entry(
    current_candidate: Option<&ImportCandidate>,
    import_candidates: &[ImportCandidate],
) -> usize {
    if let Some(recommended_candidate) = recommended_starting_point_candidate(import_candidates) {
        let mut labels = crate::migration::render::candidate_source_rollup_labels(
            &migration_candidate_from_onboard(recommended_candidate),
        );
        if let Some(current_candidate) = current_candidate {
            labels.retain(|label| label != &current_candidate.source);
        }
        return labels.len();
    }

    import_candidates
        .iter()
        .filter(|candidate| {
            !matches!(
                candidate.source_kind,
                crate::migration::ImportSourceKind::ExistingLoongConfig
                    | crate::migration::ImportSourceKind::RecommendedPlan
            )
        })
        .count()
}

fn render_detected_settings_digest_lines(
    current_setup_state: crate::migration::CurrentSetupState,
    current_candidate: Option<&ImportCandidate>,
    import_candidates: &[ImportCandidate],
    workspace_root: Option<&Path>,
    recommended_plan_available: bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(workspace_root) = workspace_root {
        lines.push(format!("- workspace: {}", workspace_root.display()));
    }
    lines.push(format!(
        "- current setup: {}",
        crate::onboard_presentation::current_setup_state_label(current_setup_state)
    ));
    if let Some(candidate) = current_candidate {
        lines.push(format!("- current config: {}", candidate.source));
    }

    let coverage_kinds = recommended_starting_point_candidate(import_candidates)
        .map(|candidate| collect_detected_coverage_kinds([candidate]))
        .filter(|kinds| !kinds.is_empty())
        .or_else(|| {
            let kinds = collect_detected_coverage_kinds(import_candidates.iter());
            (!kinds.is_empty()).then_some(kinds)
        });
    if let Some(coverage_kinds) = coverage_kinds {
        let coverage = coverage_kinds
            .into_iter()
            .map(|kind| kind.label())
            .collect::<Vec<_>>()
            .join(", ");
        let prefix =
            crate::onboard_presentation::detected_coverage_prefix(recommended_plan_available);
        lines.push(format!("{prefix}{coverage}"));
    } else if recommended_plan_available {
        lines.push(crate::onboard_presentation::suggested_starting_point_ready_line().to_owned());
    }

    let channel_labels = collect_detected_channel_labels(import_candidates);
    if !channel_labels.is_empty() {
        lines.push(format!(
            "- channels detected: {}",
            channel_labels.join(", ")
        ));
    }

    let guidance_files =
        collect_detected_workspace_guidance_files(current_candidate.into_iter(), import_candidates);
    if !guidance_files.is_empty() {
        lines.push(format!(
            "- workspace guidance: {}",
            guidance_files.join(", ")
        ));
    }

    let reusable_source_count =
        detected_reusable_source_count_for_entry(current_candidate, import_candidates);
    if reusable_source_count > 0 {
        lines.push(format!("- reusable sources: {reusable_source_count}"));
    }

    lines
}
pub(super) fn prompt_onboard_entry_choice(
    ui: &mut impl OnboardUi,
    options: &[OnboardEntryOption],
) -> CliResult<OnboardEntryChoice> {
    let screen_options = build_onboard_entry_screen_options(options);
    let default_key = screen_options
        .iter()
        .find(|option| option.recommended)
        .map(|option| option.key.as_str())
        .or_else(|| screen_options.first().map(|option| option.key.as_str()));
    let idx = select_screen_option(ui, "Setup path", &screen_options, default_key)?;
    options
        .get(idx)
        .map(|option| option.choice)
        .ok_or_else(|| format!("entry selection index {idx} out of range"))
}

pub(super) fn select_interactive_import_starting_config(
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
    current_setup_state: crate::migration::CurrentSetupState,
    import_candidates: Vec<ImportCandidate>,
    all_candidates: &[ImportCandidate],
) -> CliResult<StartingConfigSelection> {
    let import_candidates = sort_starting_point_candidates(import_candidates);
    if import_candidates.is_empty() {
        return Ok(default_starting_config_selection());
    }
    if import_candidates.len() == 1 {
        if let Some(candidate) = import_candidates.first() {
            print_import_candidate_preview(ui, candidate, all_candidates, context)?;
            return Ok(
                crate::onboard_import::starting_config_selection_from_import_candidate(
                    candidate.clone(),
                    all_candidates,
                    current_setup_state,
                ),
            );
        }
        return Ok(default_starting_config_selection());
    }

    print_import_candidates(ui, &import_candidates, context)?;
    let Some(index) = prompt_import_candidate_choice(ui, &import_candidates, context.render_width)?
    else {
        return Ok(default_starting_config_selection());
    };
    if let Some(candidate) = import_candidates.get(index) {
        return Ok(
            crate::onboard_import::starting_config_selection_from_import_candidate(
                candidate.clone(),
                all_candidates,
                current_setup_state,
            ),
        );
    }
    Ok(default_starting_config_selection())
}

pub fn collect_import_candidates_with_paths(
    output_path: &Path,
    codex_config_path: Option<&Path>,
    readiness: ChannelImportReadiness,
) -> CliResult<Vec<ImportCandidate>> {
    let workspace_root = env::current_dir().ok();
    crate::migration::collect_import_candidates_with_paths_and_readiness(
        output_path,
        codex_config_path,
        workspace_root.as_deref(),
        to_migration_readiness(readiness),
    )
    .map(crate::migration::prepend_recommended_import_candidate)
    .map(|candidates| {
        candidates
            .into_iter()
            .map(import_candidate_from_migration)
            .collect()
    })
}

fn print_import_candidate_preview(
    ui: &mut impl OnboardUi,
    candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    print_lines(
        ui,
        render_single_detected_setup_preview_screen_lines_with_style(
            candidate,
            all_candidates,
            context.render_width,
            true,
        ),
    )
}

pub fn render_single_detected_setup_preview_screen_lines(
    candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
    width: usize,
) -> Vec<String> {
    render_single_detected_setup_preview_screen_lines_with_style(
        candidate,
        all_candidates,
        width,
        false,
    )
}

pub(super) fn render_single_detected_setup_preview_screen_lines_with_style(
    candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let migration_candidate = migration_candidate_from_onboard(candidate);
    let migration_candidates = all_candidates
        .iter()
        .map(migration_candidate_from_onboard)
        .collect::<Vec<_>>();
    let provider_selection = crate::migration::build_provider_selection_plan_for_candidate(
        &migration_candidate,
        &migration_candidates,
    );
    let mut intro_lines = Vec::new();
    if let Some(reason_line) =
        format_starting_point_reason(&collect_starting_point_fit_hints(candidate))
    {
        intro_lines.push(reason_line);
    }
    let preview_candidate = migration_candidate_for_onboard_display(candidate);
    let preview_lines =
        crate::migration::render::candidate_preview_display_lines(&preview_candidate);
    intro_lines.extend(preview_lines);

    let provider_selection_lines =
        crate::migration::render::provider_selection_display_lines(&provider_selection);
    intro_lines.extend(provider_selection_lines);

    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        crate::onboard_presentation::single_detected_starting_point_preview_subtitle(),
        crate::onboard_presentation::single_detected_starting_point_preview_title(),
        None,
        intro_lines,
        Vec::new(),
        vec![
            crate::onboard_presentation::single_detected_starting_point_preview_footer().to_owned(),
        ],
        false,
        color_enabled,
    )
}

fn print_import_candidates(
    ui: &mut impl OnboardUi,
    candidates: &[ImportCandidate],
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    print_lines(
        ui,
        render_starting_point_selection_header_lines_with_style(
            candidates,
            context.render_width,
            true,
        ),
    )
}

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

pub(super) fn provider_matches_for_review(
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

pub(super) fn build_onboard_review_candidate_with_selected_context(
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

pub(super) fn render_onboard_review_lines_with_guidance_and_style(
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

#[cfg(test)]
pub(crate) fn render_onboard_wrapped_display_lines<I, S>(
    display_lines: I,
    width: usize,
) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    display_lines
        .into_iter()
        .flat_map(|line| mvp::presentation::render_wrapped_display_line(line.as_ref(), width))
        .collect()
}

#[cfg(test)]
pub(crate) fn render_onboard_option_lines(
    options: &[OnboardScreenOption],
    width: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    for option in options {
        let suffix = if option.recommended {
            " (recommended)"
        } else {
            ""
        };
        let prefix = render_onboard_option_prefix(&option.key);
        let continuation = " ".repeat(prefix.chars().count());
        lines.extend(
            mvp::presentation::render_wrapped_text_line_with_continuation(
                &prefix,
                &continuation,
                &format!("{}{}", option.label, suffix),
                width,
            ),
        );
        lines.extend(render_onboard_wrapped_display_lines(
            option
                .detail_lines
                .iter()
                .map(|detail| format!("    {detail}"))
                .collect::<Vec<_>>(),
            width,
        ));
    }
    lines
}

pub fn render_default_choice_footer_line(key: &str, description: &str) -> String {
    format!("press Enter to use default {key}, {description}")
}

pub(super) fn render_prompt_with_default_text(label: &str, default: &str) -> String {
    format!("{label} (default: {default}): ")
}

#[cfg(test)]
pub(crate) fn render_onboard_option_prefix(key: &str) -> String {
    format!("{key}) ")
}

fn render_default_input_hint_line(description: impl AsRef<str>) -> String {
    format!("- press Enter to {}", description.as_ref())
}

fn render_clear_input_hint_line(description: impl AsRef<str>) -> String {
    format!(
        "- type {ONBOARD_CLEAR_INPUT_TOKEN} to {}",
        description.as_ref()
    )
}

fn render_model_selection_default_hint_line(
    config: &mvp::config::LoongConfig,
    prompt_default: &str,
) -> String {
    let prompt_default = prompt_default.trim();
    let current_model = config.provider.model.trim();
    if prompt_default == current_model {
        render_default_input_hint_line("keep current model")
    } else if prompt_default.is_empty() {
        render_default_input_hint_line("leave the model blank")
    } else {
        render_default_input_hint_line(format!("use prefilled model: {prompt_default}"))
    }
}

fn render_api_key_env_selection_default_hint_line(
    config: &mvp::config::LoongConfig,
    suggested_env: &str,
    prompt_default: &str,
) -> String {
    let prompt_default =
        provider_credential_policy::render_provider_credential_source_value(Some(prompt_default))
            .unwrap_or_default();
    let suggested_env =
        provider_credential_policy::render_provider_credential_source_value(Some(suggested_env))
            .unwrap_or_default();
    let current_env =
        provider_credential_policy::configured_provider_credential_env_binding(&config.provider)
            .and_then(|binding| {
                provider_credential_policy::render_provider_credential_source_value(Some(
                    binding.env_name.as_str(),
                ))
            });

    if prompt_default.is_empty() {
        return render_default_input_hint_line("leave this blank");
    }

    if current_env
        .as_deref()
        .is_some_and(|current_env| current_env == prompt_default)
    {
        return render_default_input_hint_line("keep current source");
    }

    if !suggested_env.is_empty() && prompt_default == suggested_env {
        return render_default_input_hint_line(format!("use suggested source: {prompt_default}"));
    }

    render_default_input_hint_line(format!("use prefilled source: {prompt_default}"))
}

fn render_web_search_credential_selection_default_hint_line(
    config: &mvp::config::LoongConfig,
    provider: &str,
    prompt_default: &str,
) -> String {
    let prompt_default =
        provider_credential_policy::render_provider_credential_source_value(Some(prompt_default))
            .unwrap_or_default();
    let suggested_env = mvp::config::web_search_provider_descriptor(provider)
        .and_then(|descriptor| descriptor.default_api_key_env)
        .and_then(|env_name| {
            provider_credential_policy::render_provider_credential_source_value(Some(env_name))
        })
        .unwrap_or_default();
    let current_env =
        configured_query_search_credential_env_name(config, provider).and_then(|env_name| {
            provider_credential_policy::render_provider_credential_source_value(Some(
                env_name.as_str(),
            ))
        });

    if prompt_default.is_empty() {
        return render_default_input_hint_line("leave this blank");
    }

    if current_env
        .as_deref()
        .is_some_and(|current_env| current_env == prompt_default)
    {
        return render_default_input_hint_line("keep current source");
    }

    if !suggested_env.is_empty() && prompt_default == suggested_env {
        return render_default_input_hint_line(format!("use suggested source: {prompt_default}"));
    }

    render_default_input_hint_line(format!("use prefilled source: {prompt_default}"))
}

fn render_system_prompt_selection_default_hint_line(
    config: &mvp::config::LoongConfig,
    prompt_default: &str,
) -> String {
    let prompt_default = prompt_default.trim();
    let current_prompt = config.cli.system_prompt.trim();

    if prompt_default == current_prompt {
        if current_prompt.is_empty() {
            render_default_input_hint_line("keep the built-in default")
        } else {
            render_default_input_hint_line("keep current prompt")
        }
    } else if prompt_default.is_empty() {
        render_default_input_hint_line("keep the built-in default")
    } else {
        render_default_input_hint_line(format!("use prefilled prompt: {prompt_default}"))
    }
}

fn with_default_choice_footer(
    mut footer_lines: Vec<String>,
    default_choice_line: Option<String>,
) -> Vec<String> {
    if let Some(default_choice_line) = default_choice_line {
        footer_lines.insert(0, default_choice_line);
    }
    footer_lines
}

pub fn append_escape_cancel_hint(mut lines: Vec<String>) -> Vec<String> {
    if !lines.iter().any(|line| {
        let lower = line.to_ascii_lowercase();
        lower.contains("esc") && lower.contains("cancel")
    }) {
        lines.push(ONBOARD_ESCAPE_CANCEL_HINT.to_owned());
    }
    lines
}

pub(super) fn render_onboard_choice_screen(
    header_style: OnboardHeaderStyle,
    width: usize,
    subtitle: &str,
    title: &str,
    step: Option<(GuidedOnboardStep, GuidedPromptPath)>,
    intro_lines: Vec<String>,
    options: Vec<OnboardScreenOption>,
    footer_lines: Vec<String>,
    show_escape_cancel_hint: bool,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboard_choice_screen_spec(
        header_style,
        subtitle,
        title,
        step,
        intro_lines,
        options,
        footer_lines,
        show_escape_cancel_hint,
    );

    render_onboard_screen_spec(&spec, width, color_enabled)
}

fn render_onboard_input_screen(
    width: usize,
    title: &str,
    step: GuidedOnboardStep,
    guided_prompt_path: GuidedPromptPath,
    context_lines: Vec<String>,
    hint_lines: Vec<String>,
    color_enabled: bool,
) -> Vec<String> {
    let spec =
        build_onboard_input_screen_spec(title, step, guided_prompt_path, context_lines, hint_lines);

    render_onboard_screen_spec(&spec, width, color_enabled)
}

pub fn render_continue_current_setup_screen_lines(
    config: &mvp::config::LoongConfig,
    width: usize,
) -> Vec<String> {
    render_onboard_shortcut_screen_lines_with_style(
        OnboardShortcutKind::CurrentSetup,
        config,
        None,
        width,
        false,
    )
}

pub fn render_continue_detected_setup_screen_lines(
    config: &mvp::config::LoongConfig,
    import_source: &str,
    width: usize,
) -> Vec<String> {
    render_onboard_shortcut_screen_lines_with_style(
        OnboardShortcutKind::DetectedSetup,
        config,
        Some(import_source),
        width,
        false,
    )
}

pub(super) fn render_onboard_shortcut_screen_lines_with_style(
    shortcut_kind: OnboardShortcutKind,
    config: &mvp::config::LoongConfig,
    import_source: Option<&str>,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboard_shortcut_screen_spec(shortcut_kind, config, import_source, true);
    render_onboard_screen_spec(&spec, width, color_enabled)
}

pub(super) fn render_onboard_shortcut_header_lines_with_style(
    shortcut_kind: OnboardShortcutKind,
    config: &mvp::config::LoongConfig,
    import_source: Option<&str>,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboard_shortcut_screen_spec(shortcut_kind, config, import_source, false);
    render_onboard_screen_spec(&spec, width, color_enabled)
}

pub(super) fn render_shortcut_default_choice_footer_line(shortcut_kind: OnboardShortcutKind) -> String {
    render_default_choice_footer_line("1", shortcut_kind.default_choice_description())
}

pub(super) fn tui_header_style(style: OnboardHeaderStyle) -> TuiHeaderStyle {
    match style {
        OnboardHeaderStyle::Compact => TuiHeaderStyle::Compact,
    }
}

pub fn render_onboarding_risk_screen_lines(width: usize) -> Vec<String> {
    render_onboarding_risk_screen_lines_with_style(width, false)
}

pub fn render_write_confirmation_screen_lines(
    config_path: &str,
    warnings_kept: bool,
    width: usize,
) -> Vec<String> {
    render_write_confirmation_screen_lines_with_style(
        config_path,
        warnings_kept,
        width,
        ReviewFlowStyle::Guided(GuidedPromptPath::NativePromptPack),
        false,
    )
}

pub fn render_current_setup_write_confirmation_screen_lines(
    config_path: &str,
    warnings_kept: bool,
    width: usize,
) -> Vec<String> {
    render_write_confirmation_screen_lines_with_style(
        config_path,
        warnings_kept,
        width,
        ReviewFlowStyle::QuickCurrentSetup,
        false,
    )
}

pub fn render_detected_setup_write_confirmation_screen_lines(
    config_path: &str,
    warnings_kept: bool,
    width: usize,
) -> Vec<String> {
    render_write_confirmation_screen_lines_with_style(
        config_path,
        warnings_kept,
        width,
        ReviewFlowStyle::QuickDetectedSetup,
        false,
    )
}

pub(super) fn screen_subtitle(subtitle: &str) -> Option<String> {
    let trimmed_subtitle = subtitle.trim();

    if trimmed_subtitle.is_empty() {
        return None;
    }

    Some(trimmed_subtitle.to_owned())
}

fn push_starting_point_fit_hint(
    hints: &mut Vec<StartingPointFitHint>,
    seen: &mut std::collections::BTreeSet<&'static str>,
    key: &'static str,
    detail: impl Into<String>,
    domain: Option<crate::migration::SetupDomainKind>,
) {
    if seen.insert(key) {
        hints.push(StartingPointFitHint {
            key,
            detail: detail.into(),
            domain,
        });
    }
}

fn summarize_direct_starting_point_source_reason(
    candidate: &ImportCandidate,
) -> Option<&'static str> {
    candidate.source_kind.direct_starting_point_reason()
}

fn collect_starting_point_fit_hints(candidate: &ImportCandidate) -> Vec<StartingPointFitHint> {
    let mut hints = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    if let Some(reason) = summarize_direct_starting_point_source_reason(candidate) {
        push_starting_point_fit_hint(&mut hints, &mut seen, "direct_source", reason, None);
    } else if let Some(provider_domain) = candidate
        .domains
        .iter()
        .find(|domain| domain.kind == crate::migration::SetupDomainKind::Provider)
        && let Some(decision) = provider_domain.decision
        && let Some(reason) = provider_domain.kind.starting_point_reason(decision)
    {
        let key = match decision {
            crate::migration::types::PreviewDecision::KeepCurrent => "provider_keep",
            crate::migration::types::PreviewDecision::UseDetected => "provider_detected",
            crate::migration::types::PreviewDecision::Supplement
            | crate::migration::types::PreviewDecision::ReviewConflict
            | crate::migration::types::PreviewDecision::AdjustedInSession => "provider",
        };
        push_starting_point_fit_hint(
            &mut hints,
            &mut seen,
            key,
            reason,
            Some(crate::migration::SetupDomainKind::Provider),
        );
    }

    if let Some(channels_domain) = candidate
        .domains
        .iter()
        .find(|domain| domain.kind == crate::migration::SetupDomainKind::Channels)
        && let Some(decision) = channels_domain.decision
        && let Some(reason) = channels_domain.kind.starting_point_reason(decision)
    {
        let key = match decision {
            crate::migration::types::PreviewDecision::Supplement => "channels_add",
            crate::migration::types::PreviewDecision::UseDetected => "channels_detected",
            crate::migration::types::PreviewDecision::KeepCurrent
            | crate::migration::types::PreviewDecision::ReviewConflict
            | crate::migration::types::PreviewDecision::AdjustedInSession => "channels",
        };
        push_starting_point_fit_hint(
            &mut hints,
            &mut seen,
            key,
            reason,
            Some(crate::migration::SetupDomainKind::Channels),
        );
    } else if !candidate.channel_candidates.is_empty()
        && let Some(reason) = crate::migration::SetupDomainKind::Channels
            .starting_point_reason(crate::migration::types::PreviewDecision::Supplement)
    {
        push_starting_point_fit_hint(
            &mut hints,
            &mut seen,
            "channels_add",
            reason,
            Some(crate::migration::SetupDomainKind::Channels),
        );
    }

    if (!candidate.workspace_guidance.is_empty()
        || candidate.domains.iter().any(|domain| {
            domain.kind == crate::migration::SetupDomainKind::WorkspaceGuidance
                && matches!(
                    domain.decision,
                    Some(crate::migration::types::PreviewDecision::UseDetected)
                        | Some(crate::migration::types::PreviewDecision::Supplement)
                )
        }))
        && let Some(reason) = crate::migration::SetupDomainKind::WorkspaceGuidance
            .starting_point_reason(crate::migration::types::PreviewDecision::UseDetected)
    {
        push_starting_point_fit_hint(
            &mut hints,
            &mut seen,
            "workspace_guidance",
            reason,
            Some(crate::migration::SetupDomainKind::WorkspaceGuidance),
        );
    }

    for (kind, key) in [
        (crate::migration::SetupDomainKind::Cli, "cli"),
        (crate::migration::SetupDomainKind::Memory, "memory"),
        (crate::migration::SetupDomainKind::Tools, "tools"),
    ] {
        if hints.len() >= 3 {
            break;
        }
        if candidate.domains.iter().any(|domain| {
            domain.kind == kind
                && matches!(
                    domain.decision,
                    Some(crate::migration::types::PreviewDecision::UseDetected)
                        | Some(crate::migration::types::PreviewDecision::Supplement)
                )
        }) && let Some(reason) =
            kind.starting_point_reason(crate::migration::types::PreviewDecision::UseDetected)
        {
            push_starting_point_fit_hint(&mut hints, &mut seen, key, reason, Some(kind));
        }
    }

    if hints.is_empty() {
        let source_count = crate::migration::render::candidate_source_rollup_labels(
            &migration_candidate_from_onboard(candidate),
        )
        .len();
        if source_count > 1 {
            push_starting_point_fit_hint(
                &mut hints,
                &mut seen,
                "combined_sources",
                format!("combine {source_count} reusable sources"),
                None,
            );
        }
    }

    hints
}

fn format_starting_point_reason(hints: &[StartingPointFitHint]) -> Option<String> {
    if hints.is_empty() {
        return None;
    }

    Some(format!(
        "good fit: {}",
        hints
            .iter()
            .take(3)
            .map(|hint| hint.detail.as_str())
            .collect::<Vec<_>>()
            .join(" + ")
    ))
}

fn should_include_starting_point_domain_decision(candidate: &ImportCandidate) -> bool {
    candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan
}

fn format_starting_point_domain_detail(
    candidate: &ImportCandidate,
    domain: &crate::migration::DomainPreview,
) -> String {
    let mut detail = format!("{}: ", domain.kind.label());
    if should_include_starting_point_domain_decision(candidate)
        && let Some(decision) = domain.decision
    {
        detail.push_str(decision.label());
        detail.push_str(" · ");
    }
    detail.push_str(&domain.summary);
    detail
}

pub(super) fn summarize_starting_point_detail_lines(candidate: &ImportCandidate, width: usize) -> Vec<String> {
    let mut details = Vec::new();
    let max_lines = if width < 68 { 4 } else { 5 };
    let mut detail_lines_used = 0usize;
    let has_channel_details = !candidate.channel_candidates.is_empty();
    let has_workspace_guidance_details = !candidate.workspace_guidance.is_empty();
    let migration_candidate = migration_candidate_from_onboard(candidate);
    let fit_hints = collect_starting_point_fit_hints(candidate);
    let emphasized_domains = if width < 68 {
        fit_hints
            .iter()
            .filter_map(|hint| hint.domain)
            .collect::<std::collections::BTreeSet<_>>()
    } else {
        std::collections::BTreeSet::new()
    };

    if let Some(reason_line) = format_starting_point_reason(&fit_hints) {
        details.push(reason_line);
    }

    let mut source_labels =
        crate::migration::render::candidate_source_rollup_labels(&migration_candidate);
    if has_workspace_guidance_details {
        source_labels.retain(|label| label != "workspace guidance");
    }
    let should_render_source_summary =
        if candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan {
            !source_labels.is_empty()
        } else {
            source_labels.len() > 1
        };
    if should_render_source_summary {
        details.push(format!("sources: {}", source_labels.join(" + ")));
        detail_lines_used += 1;
    }

    for domain in &candidate.domains {
        if has_channel_details && domain.kind == crate::migration::SetupDomainKind::Channels {
            continue;
        }
        if has_workspace_guidance_details
            && domain.kind == crate::migration::SetupDomainKind::WorkspaceGuidance
        {
            continue;
        }
        if emphasized_domains.contains(&domain.kind) {
            continue;
        }
        details.push(format_starting_point_domain_detail(candidate, domain));
        detail_lines_used += 1;
        if detail_lines_used >= max_lines {
            return details;
        }
    }

    for channel in &candidate.channel_candidates {
        details.push(format!(
            "{}: {}",
            channel.label.to_ascii_lowercase(),
            channel.summary
        ));
        detail_lines_used += 1;
        if detail_lines_used >= max_lines {
            return details;
        }
    }

    if details.len() < max_lines && !candidate.workspace_guidance.is_empty() {
        let files = candidate
            .workspace_guidance
            .iter()
            .filter_map(|guidance| Path::new(&guidance.path).file_name())
            .map(|name| name.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        if !files.is_empty() {
            details.push(format!("workspace guidance: {}", files.join(", ")));
        }
    }

    if details.is_empty() {
        details.push("ready to use as a starting point".to_owned());
    }

    details
}

pub(super) fn start_fresh_starting_point_detail_lines() -> Vec<String> {
    vec![
        crate::onboard_presentation::start_fresh_starting_point_fit_line().to_owned(),
        crate::onboard_presentation::start_fresh_starting_point_detail_line().to_owned(),
    ]
}

fn render_starting_point_selection_footer_lines(
    sorted_candidates: &[ImportCandidate],
) -> Vec<String> {
    let Some(first_candidate) = sorted_candidates.first() else {
        return Vec::new();
    };

    let first_hint = render_default_choice_footer_line(
        "1",
        crate::onboard_presentation::starting_point_footer_description(first_candidate.source_kind),
    );

    vec![first_hint]
}

pub fn render_starting_point_selection_screen_lines(
    candidates: &[ImportCandidate],
    width: usize,
) -> Vec<String> {
    render_starting_point_selection_screen_lines_with_style(candidates, width, false)
}

fn render_starting_point_selection_screen_lines_with_style(
    candidates: &[ImportCandidate],
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let sorted_candidates = sort_starting_point_candidates(candidates.to_vec());
    let mut options = sorted_candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| OnboardScreenOption {
            key: (index + 1).to_string(),
            label: onboard_starting_point_label(Some(candidate.source_kind), &candidate.source),
            detail_lines: summarize_starting_point_detail_lines(candidate, width),
            recommended: matches!(
                candidate.source_kind,
                crate::migration::ImportSourceKind::RecommendedPlan
            ),
        })
        .collect::<Vec<_>>();
    options.push(OnboardScreenOption {
        key: "0".to_owned(),
        label: crate::onboard_presentation::start_fresh_option_label().to_owned(),
        detail_lines: start_fresh_starting_point_detail_lines(),
        recommended: false,
    });
    let footer_lines = render_starting_point_selection_footer_lines(&sorted_candidates);

    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        crate::onboard_presentation::starting_point_selection_subtitle(),
        crate::onboard_presentation::starting_point_selection_title(),
        None,
        vec![crate::onboard_presentation::starting_point_selection_hint().to_owned()],
        options,
        footer_lines,
        true,
        color_enabled,
    )
}

pub(super) fn render_starting_point_selection_header_lines_with_style(
    _candidates: &[ImportCandidate],
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        crate::onboard_presentation::starting_point_selection_subtitle(),
        crate::onboard_presentation::starting_point_selection_title(),
        None,
        vec![crate::onboard_presentation::starting_point_selection_hint().to_owned()],
        Vec::new(),
        Vec::new(),
        true,
        color_enabled,
    )
}

pub fn render_provider_selection_screen_lines(
    plan: &crate::migration::ProviderSelectionPlan,
    width: usize,
) -> Vec<String> {
    render_provider_selection_screen_lines_with_style(
        plan,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
    )
}

fn render_provider_selection_screen_lines_with_style(
    plan: &crate::migration::ProviderSelectionPlan,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let intro = provider_selection_intro_lines(plan);
    let options = plan
        .imported_choices
        .iter()
        .map(|choice| OnboardScreenOption {
            key: choice.profile_id.clone(),
            label: provider_kind_display_name(choice.kind).to_owned(),
            detail_lines: {
                let mut detail_lines = vec![
                    format!("source: {}", choice.source),
                    format!("summary: {}", choice.summary),
                ];
                if let Some(selector_detail) =
                    crate::migration::provider_selection::selector_detail_line(
                        plan,
                        &choice.profile_id,
                        width,
                    )
                {
                    detail_lines.push(selector_detail);
                }
                if let Some(transport_summary) = choice.config.preview_transport_summary() {
                    detail_lines.push(format!("transport: {transport_summary}"));
                }
                detail_lines
            },
            recommended: Some(choice.profile_id.as_str()) == plan.default_profile_id.as_deref(),
        })
        .collect::<Vec<_>>();
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "choose the current provider",
        "choose active provider",
        Some((GuidedOnboardStep::Provider, guided_prompt_path)),
        intro,
        options,
        with_default_choice_footer(
            crate::migration::guidance_lines(plan, width),
            render_provider_selection_default_choice_footer_line(plan),
        ),
        true,
        color_enabled,
    )
}

pub(super) fn render_provider_selection_header_lines(
    plan: &crate::migration::ProviderSelectionPlan,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
) -> Vec<String> {
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "choose the current provider",
        "choose active provider",
        Some((GuidedOnboardStep::Provider, guided_prompt_path)),
        provider_selection_intro_lines(plan),
        vec![],
        vec![],
        true,
        true,
    )
}

fn provider_selection_intro_lines(plan: &crate::migration::ProviderSelectionPlan) -> Vec<String> {
    if plan.imported_choices.is_empty() {
        vec!["pick the provider that should back this setup".to_owned()]
    } else if plan.requires_explicit_choice {
        vec!["other detected settings stay merged".to_owned()]
    } else {
        vec!["review the detected provider choices for this setup".to_owned()]
    }
}

fn render_provider_selection_default_choice_footer_line(
    plan: &crate::migration::ProviderSelectionPlan,
) -> Option<String> {
    if plan.requires_explicit_choice {
        return None;
    }
    let default_profile_id = plan.default_profile_id.as_deref()?;
    let default_kind = plan
        .imported_choices
        .iter()
        .find(|choice| choice.profile_id == default_profile_id)
        .map(|choice| choice.kind)
        .or(plan.default_kind)?;
    Some(render_default_choice_footer_line(
        default_profile_id,
        &format!("the {} provider", provider_kind_display_name(default_kind)),
    ))
}

pub fn render_model_selection_screen_lines(
    config: &mvp::config::LoongConfig,
    width: usize,
) -> Vec<String> {
    render_model_selection_screen_lines_with_style(
        config,
        config.provider.model.as_str(),
        GuidedPromptPath::NativePromptPack,
        width,
        false,
        false,
    )
}

pub fn render_model_selection_screen_lines_with_default(
    config: &mvp::config::LoongConfig,
    prompt_default: &str,
    width: usize,
) -> Vec<String> {
    render_model_selection_screen_lines_with_style(
        config,
        prompt_default,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
        false,
    )
}

pub(super) fn render_model_selection_screen_lines_with_style(
    config: &mvp::config::LoongConfig,
    prompt_default: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
    catalog_models_available: bool,
) -> Vec<String> {
    let selection_context =
        onboarding_model_policy::onboarding_model_selection_context(&config.provider);
    let current_model = selection_context.current_model;
    let recommended_model = selection_context.recommended_model;
    let preferred_fallback_models = selection_context.preferred_fallback_models;
    let allows_auto_fallback_hint = selection_context.allows_auto_fallback_hint;
    let mut context_lines = vec![
        format!(
            "- provider: {}",
            crate::provider_presentation::guided_provider_label(config.provider.kind)
        ),
        format!("- current model: {current_model}"),
    ];
    if let Some(recommended_model) = recommended_model {
        context_lines.push(format!("- recommended model: {recommended_model}"));
    }
    if !preferred_fallback_models.is_empty() {
        let preferred_fallback_summary = preferred_fallback_models.join(", ");
        context_lines.push(format!(
            "- configured preferred fallback: {preferred_fallback_summary}",
        ));
    }

    let mut hint_lines = vec![render_model_selection_default_hint_line(
        config,
        prompt_default,
    )];
    if catalog_models_available {
        hint_lines.push(
            "- use arrow keys to browse or type to filter available provider models".to_owned(),
        );
        hint_lines.push(
            "- choose `enter custom model id` if you want to type an override manually".to_owned(),
        );
    } else {
        hint_lines.push("- type any provider model id to override it".to_owned());
    }
    if allows_auto_fallback_hint {
        let preferred_fallback_summary = preferred_fallback_models.join(", ");
        hint_lines.push(format!(
            "- type `auto` to let runtime try configured preferred fallbacks first: {preferred_fallback_summary}",
        ));
    }

    render_onboard_input_screen(
        width,
        "choose model",
        GuidedOnboardStep::Model,
        guided_prompt_path,
        context_lines,
        hint_lines,
        color_enabled,
    )
}

pub fn render_api_key_env_selection_screen_lines(
    config: &mvp::config::LoongConfig,
    default_api_key_env: &str,
    width: usize,
) -> Vec<String> {
    render_api_key_env_selection_screen_lines_with_style(
        config,
        default_api_key_env,
        default_api_key_env,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
    )
}

pub fn render_api_key_env_selection_screen_lines_with_default(
    config: &mvp::config::LoongConfig,
    default_api_key_env: &str,
    prompt_default: &str,
    width: usize,
) -> Vec<String> {
    render_api_key_env_selection_screen_lines_with_style(
        config,
        default_api_key_env,
        prompt_default,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
    )
}

pub(super) fn render_api_key_env_selection_screen_lines_with_style(
    config: &mvp::config::LoongConfig,
    default_api_key_env: &str,
    prompt_default: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let mut context_lines = vec![format!(
        "- provider: {}",
        crate::provider_presentation::guided_provider_label(config.provider.kind)
    )];
    if let Some(current_env) =
        provider_credential_policy::render_configured_provider_credential_source_value(
            &config.provider,
        )
    {
        context_lines.push(format!("- current source: {current_env}"));
    }
    if let Some(suggested_source) =
        provider_credential_policy::render_provider_credential_source_value(Some(
            default_api_key_env,
        ))
    {
        context_lines.push(format!("- suggested source: {suggested_source}"));
    }

    let example_env_name =
        provider_credential_policy::provider_credential_env_hint(&config.provider)
            .unwrap_or_else(|| "PROVIDER_API_KEY".to_owned());
    let mut hint_lines = vec![render_api_key_env_selection_default_hint_line(
        config,
        default_api_key_env,
        prompt_default,
    )];
    hint_lines.push("- enter an env var name, not the secret value itself".to_owned());
    hint_lines.push(format!("- example: {example_env_name}"));
    if prompt_default.trim().is_empty() {
        if provider_credential_policy::provider_has_inline_credential(&config.provider) {
            hint_lines.push("- leave this blank to keep inline credentials".to_owned());
        }
    } else if provider_supports_blank_api_key_env(config) {
        hint_lines.push(render_clear_input_hint_line(
            "clear the configured credential env",
        ));
    }

    render_onboard_input_screen(
        width,
        "choose credential source",
        GuidedOnboardStep::CredentialEnv,
        guided_prompt_path,
        context_lines,
        hint_lines,
        color_enabled,
    )
}

pub(super) fn render_web_search_credential_selection_screen_lines_with_style(
    config: &mvp::config::LoongConfig,
    provider: &str,
    prompt_default: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let provider_label = query_search_provider_display_name(provider);
    let mut context_lines = vec![format!("- provider: {provider_label}")];
    if let Some(current_value) = configured_query_search_credential_source_value(config, provider) {
        let label = if current_value == "inline api key" {
            "- current credential: "
        } else {
            "- current source: "
        };
        context_lines.extend(mvp::presentation::render_wrapped_text_line(
            label,
            &current_value,
            width,
        ));
    }
    if let Some(suggested_env) = mvp::config::web_search_provider_descriptor(provider)
        .and_then(|descriptor| descriptor.default_api_key_env)
        .and_then(|env_name| {
            provider_credential_policy::render_provider_credential_source_value(Some(env_name))
        })
    {
        context_lines.extend(mvp::presentation::render_wrapped_text_line(
            "- suggested source: ",
            &suggested_env,
            width,
        ));
    }

    let mut hint_lines = vec![render_web_search_credential_selection_default_hint_line(
        config,
        provider,
        prompt_default,
    )];
    hint_lines.push("- enter an env var name, not the secret value itself".to_owned());
    let example_env_name = mvp::config::web_search_provider_descriptor(provider)
        .and_then(|descriptor| {
            descriptor
                .default_api_key_env
                .or_else(|| descriptor.api_key_env_names.first().copied())
        })
        .unwrap_or("WEB_SEARCH_API_KEY");
    hint_lines.push(format!("- example: {example_env_name}"));
    if prompt_default.trim().is_empty() && query_search_has_inline_credential(config, provider) {
        hint_lines.push("- leave this blank to keep inline credentials".to_owned());
    }
    if configured_query_search_secret(config, provider)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        hint_lines.push(render_clear_input_hint_line(
            crate::access_terms::query_search_credential_clear_hint(),
        ));
    }

    render_onboard_input_screen(
        width,
        crate::access_terms::CHOOSE_QUERY_SEARCH_CREDENTIAL_TITLE,
        GuidedOnboardStep::WebSearchProvider,
        guided_prompt_path,
        context_lines,
        hint_lines,
        color_enabled,
    )
}

pub fn render_system_prompt_selection_screen_lines(
    config: &mvp::config::LoongConfig,
    width: usize,
) -> Vec<String> {
    render_system_prompt_selection_screen_lines_with_style(
        config,
        config.cli.system_prompt.as_str(),
        GuidedPromptPath::InlineOverride,
        width,
        false,
    )
}

pub fn render_system_prompt_selection_screen_lines_with_default(
    config: &mvp::config::LoongConfig,
    prompt_default: &str,
    width: usize,
) -> Vec<String> {
    render_system_prompt_selection_screen_lines_with_style(
        config,
        prompt_default,
        GuidedPromptPath::InlineOverride,
        width,
        false,
    )
}

pub(super) fn render_system_prompt_selection_screen_lines_with_style(
    config: &mvp::config::LoongConfig,
    prompt_default: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let current_prompt = config.cli.system_prompt.trim();
    let current_prompt_display = if current_prompt.is_empty() {
        "built-in default".to_owned()
    } else {
        current_prompt.to_owned()
    };

    render_onboard_input_screen(
        width,
        "adjust cli behavior",
        GuidedOnboardStep::PromptCustomization,
        guided_prompt_path,
        vec![format!("- current prompt: {current_prompt_display}")],
        vec![
            render_system_prompt_selection_default_hint_line(config, prompt_default),
            if prompt_default.trim().is_empty() {
                "- leave this blank to use the built-in behavior".to_owned()
            } else {
                render_clear_input_hint_line("use the built-in behavior")
            },
            ONBOARD_SINGLE_LINE_INPUT_HINT.to_owned(),
        ],
        color_enabled,
    )
}

pub fn render_existing_config_write_screen_lines(config_path: &str, width: usize) -> Vec<String> {
    render_existing_config_write_screen_lines_with_style(config_path, width, false)
}

fn render_existing_config_write_screen_lines_with_style(
    config_path: &str,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "decide how to write the config",
        "existing config found",
        None,
        vec![
            format!("- config: {config_path}"),
            "- choose whether to replace it, keep a backup, or cancel".to_owned(),
        ],
        build_existing_config_write_screen_options(),
        vec![render_default_choice_footer_line(
            "b",
            "create backup and replace",
        )],
        true,
        color_enabled,
    )
}

pub(super) fn render_existing_config_write_header_lines_with_style(
    config_path: &str,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "decide how to write the config",
        "existing config found",
        None,
        vec![
            format!("- config: {config_path}"),
            "- choose whether to replace it, keep a backup, or cancel".to_owned(),
        ],
        Vec::new(),
        Vec::new(),
        true,
        color_enabled,
    )
}

pub(super) fn onboard_display_line(prefix: &str, value: &str) -> String {
    format!("{prefix}{value}")
}

pub(super) fn build_onboard_review_digest_display_lines(config: &mvp::config::LoongConfig) -> Vec<String> {
    let mut lines = crate::provider_presentation::provider_profile_state_display_lines(
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

pub(super) fn render_onboard_review_credential_line(provider: &mvp::config::ProviderConfig) -> Option<String> {
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

pub(super) fn provider_supports_blank_api_key_env(config: &mvp::config::LoongConfig) -> bool {
    provider_credential_policy::provider_has_inline_credential(&config.provider)
        || provider_credential_policy::provider_has_configured_credential_env(&config.provider)
}
