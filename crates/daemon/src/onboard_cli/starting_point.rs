use super::starting_point_render::{
    render_single_detected_setup_preview_screen_lines_with_style,
    render_starting_point_selection_header_lines_with_style,
};
use super::*;

pub(super) fn load_import_starting_config(
    output_path: &Path,
    options: &OnboardCommandOptions,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<StartingConfigSelection> {
    let state = prepare_import_starting_state(
        output_path,
        &context.codex_config_paths,
        context.workspace_root.as_deref(),
    )?;

    if state.current_candidate.is_none() && state.import_candidates.is_empty() {
        return Ok(default_starting_config_selection());
    }

    if options.non_interactive {
        return Ok(select_non_interactive_starting_config_from_state(&state));
    }

    if state
        .entry_options
        .first()
        .is_some_and(|option| option.choice == OnboardEntryChoice::StartFresh)
    {
        return Ok(default_starting_config_selection());
    }

    print_onboard_entry_options(
        ui,
        state.current_setup_state,
        state.current_candidate.as_ref(),
        &state.import_candidates,
        &state.entry_options,
        context,
    )?;

    match prompt_onboard_entry_choice(ui, &state.entry_options)? {
        OnboardEntryChoice::ContinueCurrentSetup => Ok(state
            .current_candidate
            .clone()
            .map(|candidate| {
                crate::onboard::import::starting_config_selection_from_current_candidate(
                    candidate,
                    state.current_setup_state,
                )
            })
            .unwrap_or_else(default_starting_config_selection)),
        OnboardEntryChoice::ImportDetectedSetup => select_interactive_import_starting_config(
            ui,
            context,
            state.current_setup_state,
            state.import_candidates.clone(),
            &state.all_candidates,
        ),
        OnboardEntryChoice::StartFresh => Ok(default_starting_config_selection()),
    }
}

pub(super) fn print_onboard_entry_options(
    ui: &mut impl OnboardUi,
    current_setup_state: crate::migration::CurrentSetupState,
    current_candidate: Option<&ImportCandidate>,
    import_candidates: &[ImportCandidate],
    options: &[OnboardEntryOption],
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    print_lines(
        ui,
        render_onboard_entry_interactive_screen_lines_with_style(
            current_setup_state,
            current_candidate,
            import_candidates,
            options,
            context.workspace_root.as_deref(),
            context.render_width,
            true,
        ),
    )
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
                crate::onboard::import::starting_config_selection_from_import_candidate(
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
            crate::onboard::import::starting_config_selection_from_import_candidate(
                candidate.clone(),
                all_candidates,
                current_setup_state,
            ),
        );
    }
    Ok(default_starting_config_selection())
}

pub(super) fn prompt_import_candidate_choice(
    ui: &mut impl OnboardUi,
    candidates: &[ImportCandidate],
    width: usize,
) -> CliResult<Option<usize>> {
    let sorted_candidates = sort_starting_point_candidates(candidates.to_vec());
    let screen_options = build_starting_point_selection_screen_options(&sorted_candidates, width);
    let idx = select_screen_option(ui, "Starting point", &screen_options, Some("1"))?;
    let selected = screen_options
        .get(idx)
        .ok_or_else(|| format!("starting point selection index {idx} out of range"))?;
    if selected.key == "0" {
        return Ok(None);
    }
    selected
        .key
        .parse::<usize>()
        .map(|value| Some(value - 1))
        .map_err(|error| {
            format!(
                "invalid starting point selection key {}: {error}",
                selected.key
            )
        })
}

pub(super) fn prompt_onboard_shortcut_choice(
    ui: &mut impl OnboardUi,
    shortcut_kind: OnboardShortcutKind,
) -> CliResult<OnboardShortcutChoice> {
    let options = build_onboard_shortcut_screen_options(shortcut_kind);
    match select_screen_option(ui, "Your choice", &options, Some("1"))? {
        0 => Ok(OnboardShortcutChoice::UseShortcut),
        1 => Ok(OnboardShortcutChoice::AdjustSettings),
        idx => Err(format!("shortcut selection index {idx} out of range")),
    }
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

pub fn detect_import_starting_config_with_channel_readiness(
    readiness: ChannelImportReadiness,
) -> mvp::config::LoongConfig {
    crate::migration::detect_import_starting_config_with_channel_readiness(to_migration_readiness(
        readiness,
    ))
}

pub(super) fn default_codex_config_paths() -> Vec<PathBuf> {
    crate::migration::discovery::default_detected_codex_config_paths()
}

pub(super) fn to_migration_readiness(
    readiness: ChannelImportReadiness,
) -> crate::migration::ChannelImportReadiness {
    readiness
}

pub(super) fn import_surface_from_migration(
    surface: crate::migration::ImportSurface,
) -> ImportSurface {
    ImportSurface {
        name: surface.name,
        domain: surface.domain,
        level: match surface.level {
            crate::migration::ImportSurfaceLevel::Ready => ImportSurfaceLevel::Ready,
            crate::migration::ImportSurfaceLevel::Review => ImportSurfaceLevel::Review,
            crate::migration::ImportSurfaceLevel::Blocked => ImportSurfaceLevel::Blocked,
        },
        detail: surface.detail,
    }
}

pub(super) fn detect_render_width() -> usize {
    mvp::presentation::detect_render_width()
}

pub(super) fn enabled_channel_ids(config: &mvp::config::LoongConfig) -> Vec<String> {
    config.enabled_channel_ids()
}

pub fn validate_non_interactive_risk_gate(
    non_interactive: bool,
    accept_risk: bool,
) -> CliResult<()> {
    if non_interactive && !accept_risk {
        return Err(
            "non-interactive onboarding requires --accept-risk (explicit acknowledgement)"
                .to_owned(),
        );
    }
    Ok(())
}

pub fn should_offer_current_setup_shortcut(
    options: &OnboardCommandOptions,
    current_setup_state: crate::migration::CurrentSetupState,
    entry_choice: OnboardEntryChoice,
) -> bool {
    !options.non_interactive
        && entry_choice == OnboardEntryChoice::ContinueCurrentSetup
        && current_setup_state == crate::migration::CurrentSetupState::Healthy
        && !onboard_has_explicit_overrides(options)
}

pub fn should_offer_detected_setup_shortcut(
    options: &OnboardCommandOptions,
    entry_choice: OnboardEntryChoice,
    provider_selection: &crate::migration::ProviderSelectionPlan,
) -> bool {
    !options.non_interactive
        && entry_choice == OnboardEntryChoice::ImportDetectedSetup
        && !provider_selection.requires_explicit_choice
        && !onboard_has_explicit_overrides(options)
}

pub(super) fn resolve_onboard_shortcut_kind(
    options: &OnboardCommandOptions,
    starting_selection: &StartingConfigSelection,
) -> Option<OnboardShortcutKind> {
    if should_offer_current_setup_shortcut(
        options,
        starting_selection.current_setup_state,
        starting_selection.entry_choice,
    ) {
        return Some(OnboardShortcutKind::CurrentSetup);
    }
    if should_offer_detected_setup_shortcut(
        options,
        starting_selection.entry_choice,
        &starting_selection.provider_selection,
    ) {
        return Some(OnboardShortcutKind::DetectedSetup);
    }
    None
}
