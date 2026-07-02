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
        title: Some(crate::onboard::presentation::detected_settings_section_heading().to_owned()),
        lines: detected_settings_lines,
    };

    let mut sections = vec![detected_settings_section];

    if !options.is_empty() {
        let entry_choice_section = TuiSectionSpec::Narrative {
            title: Some(crate::onboard::presentation::entry_choice_section_heading().to_owned()),
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
    let description = crate::onboard::presentation::entry_default_choice_description(
        onboard_entry_choice_kind(default_choice),
    );
    Some(render_default_choice_footer_line(
        &default_index.to_string(),
        description,
    ))
}

const fn onboard_entry_choice_kind(
    choice: OnboardEntryChoice,
) -> crate::onboard::presentation::EntryChoiceKind {
    match choice {
        OnboardEntryChoice::ContinueCurrentSetup => {
            crate::onboard::presentation::EntryChoiceKind::CurrentSetup
        }
        OnboardEntryChoice::ImportDetectedSetup => {
            crate::onboard::presentation::EntryChoiceKind::DetectedSetup
        }
        OnboardEntryChoice::StartFresh => crate::onboard::presentation::EntryChoiceKind::StartFresh,
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
        crate::onboard::presentation::current_setup_state_label(current_setup_state)
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
            crate::onboard::presentation::detected_coverage_prefix(recommended_plan_available);
        lines.push(format!("{prefix}{coverage}"));
    } else if recommended_plan_available {
        lines.push(crate::onboard::presentation::suggested_starting_point_ready_line().to_owned());
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
