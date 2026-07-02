use std::path::{Path, PathBuf};

use loong_app as mvp;
use loong_spec::CliResult;

pub type ChannelImportReadiness = crate::migration::ChannelImportReadiness;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportSurfaceLevel {
    Ready,
    Review,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSurface {
    pub name: &'static str,
    pub domain: crate::migration::SetupDomainKind,
    pub level: ImportSurfaceLevel,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct ImportCandidate {
    pub source_kind: crate::migration::ImportSourceKind,
    pub source: String,
    pub config: mvp::config::LoongConfig,
    pub surfaces: Vec<ImportSurface>,
    pub domains: Vec<crate::migration::DomainPreview>,
    pub channel_candidates: Vec<crate::migration::ChannelCandidate>,
    pub workspace_guidance: Vec<crate::migration::WorkspaceGuidanceCandidate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardEntryChoice {
    ContinueCurrentSetup,
    ImportDetectedSetup,
    StartFresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardEntryOption {
    pub choice: OnboardEntryChoice,
    pub label: &'static str,
    pub detail: String,
    pub recommended: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct StartingConfigSelection {
    pub(crate) config: mvp::config::LoongConfig,
    pub(crate) import_source: Option<String>,
    pub(crate) provider_selection: crate::migration::ProviderSelectionPlan,
    pub(crate) entry_choice: OnboardEntryChoice,
    pub(crate) current_setup_state: crate::migration::CurrentSetupState,
    pub(crate) review_candidate: Option<ImportCandidate>,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportStartingState {
    pub(crate) current_setup_state: crate::migration::CurrentSetupState,
    pub(crate) all_candidates: Vec<ImportCandidate>,
    pub(crate) entry_options: Vec<OnboardEntryOption>,
    pub(crate) current_candidate: Option<ImportCandidate>,
    pub(crate) import_candidates: Vec<ImportCandidate>,
}

pub(crate) fn resolve_channel_import_readiness(
    config: &mvp::config::LoongConfig,
) -> ChannelImportReadiness {
    crate::migration::resolve_channel_import_readiness_from_config(config)
}

pub(crate) fn collect_import_candidates_with_context(
    output_path: &Path,
    codex_config_paths: &[PathBuf],
    workspace_root: Option<&Path>,
    readiness: ChannelImportReadiness,
) -> CliResult<Vec<ImportCandidate>> {
    crate::migration::discovery::collect_import_candidates_with_path_list_and_readiness(
        output_path,
        codex_config_paths,
        workspace_root,
        readiness,
    )
    .map(crate::migration::prepend_recommended_import_candidate)
    .map(|candidates| {
        candidates
            .into_iter()
            .map(import_candidate_from_migration)
            .collect()
    })
}

pub fn build_onboard_entry_options(
    current_setup_state: crate::migration::CurrentSetupState,
    candidates: &[ImportCandidate],
) -> Vec<OnboardEntryOption> {
    let has_current_setup = candidates.iter().any(|candidate| {
        candidate.source_kind == crate::migration::ImportSourceKind::ExistingLoongConfig
    });
    let recommended_plan_available = candidates.iter().any(|candidate| {
        candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan
    });
    let detected_source_count = detected_reusable_source_count_for_entry(
        candidates.iter().find(|candidate| {
            candidate.source_kind == crate::migration::ImportSourceKind::ExistingLoongConfig
        }),
        candidates,
    );
    let mut options = Vec::new();

    if has_current_setup {
        options.push(OnboardEntryOption {
            choice: OnboardEntryChoice::ContinueCurrentSetup,
            label: crate::onboard::presentation::current_setup_option_label(),
            detail: crate::onboard::presentation::current_setup_option_detail(current_setup_state)
                .to_owned(),
            recommended: matches!(
                current_setup_state,
                crate::migration::CurrentSetupState::Healthy
            ) || matches!(
                current_setup_state,
                crate::migration::CurrentSetupState::Repairable
            ) && detected_source_count == 0,
        });
    }

    if detected_source_count > 0 || recommended_plan_available {
        options.push(OnboardEntryOption {
            choice: OnboardEntryChoice::ImportDetectedSetup,
            label: crate::onboard::presentation::detected_setup_option_label(),
            detail: crate::onboard::presentation::import_option_detail(
                has_current_setup,
                recommended_plan_available,
                detected_source_count,
            ),
            recommended: matches!(
                current_setup_state,
                crate::migration::CurrentSetupState::Absent
                    | crate::migration::CurrentSetupState::LegacyOrIncomplete
                    | crate::migration::CurrentSetupState::Repairable
            ),
        });
    }

    options.push(OnboardEntryOption {
        choice: OnboardEntryChoice::StartFresh,
        label: crate::onboard::presentation::start_fresh_option_label(),
        detail: crate::onboard::presentation::start_fresh_option_detail().to_owned(),
        recommended: !options.iter().any(|option| option.recommended),
    });

    options
}

pub(crate) fn prepare_import_starting_state(
    output_path: &Path,
    codex_config_paths: &[PathBuf],
    workspace_root: Option<&Path>,
) -> CliResult<ImportStartingState> {
    let default_config = mvp::config::LoongConfig::default();
    let readiness = resolve_channel_import_readiness(&default_config);
    let current_setup_state = crate::migration::classify_current_setup(output_path);
    let all_candidates = collect_import_candidates_with_context(
        output_path,
        codex_config_paths,
        workspace_root,
        readiness,
    )?;
    let entry_options = build_onboard_entry_options(current_setup_state, &all_candidates);
    let (current_candidate, import_candidates) = split_onboard_candidates(all_candidates.clone());

    Ok(ImportStartingState {
        current_setup_state,
        all_candidates,
        entry_options,
        current_candidate,
        import_candidates,
    })
}

pub(crate) fn split_onboard_candidates(
    candidates: Vec<ImportCandidate>,
) -> (Option<ImportCandidate>, Vec<ImportCandidate>) {
    let mut current_candidate = None;
    let mut import_candidates = Vec::new();

    for candidate in candidates {
        if candidate.source_kind == crate::migration::ImportSourceKind::ExistingLoongConfig
            && current_candidate.is_none()
        {
            current_candidate = Some(candidate);
        } else {
            import_candidates.push(candidate);
        }
    }

    (current_candidate, import_candidates)
}

pub(crate) fn select_non_interactive_starting_config(
    current_setup_state: crate::migration::CurrentSetupState,
    entry_options: &[OnboardEntryOption],
    current_candidate: Option<ImportCandidate>,
    import_candidates: Vec<ImportCandidate>,
    all_candidates: &[ImportCandidate],
) -> StartingConfigSelection {
    match default_onboard_entry_choice(entry_options) {
        OnboardEntryChoice::ContinueCurrentSetup => current_candidate
            .map(|candidate| {
                starting_config_selection_from_current_candidate(candidate, current_setup_state)
            })
            .unwrap_or_else(default_starting_config_selection),
        OnboardEntryChoice::ImportDetectedSetup => {
            sort_starting_point_candidates(import_candidates)
                .into_iter()
                .map(|candidate| {
                    starting_config_selection_from_import_candidate(
                        candidate,
                        all_candidates,
                        current_setup_state,
                    )
                })
                .next()
                .unwrap_or_else(default_starting_config_selection)
        }
        OnboardEntryChoice::StartFresh => default_starting_config_selection(),
    }
}

pub(crate) fn select_non_interactive_starting_config_from_state(
    state: &ImportStartingState,
) -> StartingConfigSelection {
    select_non_interactive_starting_config(
        state.current_setup_state,
        &state.entry_options,
        state.current_candidate.clone(),
        state.import_candidates.clone(),
        &state.all_candidates,
    )
}

pub(crate) fn default_onboard_entry_choice(options: &[OnboardEntryOption]) -> OnboardEntryChoice {
    options
        .iter()
        .find(|option| option.recommended)
        .map(|option| option.choice)
        .unwrap_or(OnboardEntryChoice::StartFresh)
}

pub(crate) fn sort_starting_point_candidates(
    mut candidates: Vec<ImportCandidate>,
) -> Vec<ImportCandidate> {
    candidates.sort_by_key(|candidate| {
        (
            usize::from(
                candidate.source_kind != crate::migration::ImportSourceKind::RecommendedPlan,
            ),
            std::cmp::Reverse(starting_point_candidate_coverage_breadth(candidate)),
            candidate.source_kind.direct_starting_point_rank(),
            candidate.source.to_ascii_lowercase(),
        )
    });
    candidates
}

pub(crate) fn default_starting_config_selection() -> StartingConfigSelection {
    StartingConfigSelection {
        config: mvp::config::LoongConfig::default(),
        import_source: None,
        provider_selection: crate::migration::ProviderSelectionPlan::default(),
        entry_choice: OnboardEntryChoice::StartFresh,
        current_setup_state: crate::migration::CurrentSetupState::Absent,
        review_candidate: None,
    }
}

pub(crate) fn starting_config_selection_from_current_candidate(
    candidate: ImportCandidate,
    current_setup_state: crate::migration::CurrentSetupState,
) -> StartingConfigSelection {
    StartingConfigSelection {
        config: candidate.config.clone(),
        import_source: Some(onboard_starting_point_label(
            Some(candidate.source_kind),
            &candidate.source,
        )),
        provider_selection: crate::migration::ProviderSelectionPlan::default(),
        entry_choice: OnboardEntryChoice::ContinueCurrentSetup,
        current_setup_state,
        review_candidate: Some(candidate),
    }
}

pub(crate) fn starting_config_selection_from_import_candidate(
    candidate: ImportCandidate,
    all_candidates: &[ImportCandidate],
    current_setup_state: crate::migration::CurrentSetupState,
) -> StartingConfigSelection {
    let migration_selected = migration_candidate_from_onboard(&candidate);
    let migration_candidates = all_candidates
        .iter()
        .map(migration_candidate_from_onboard)
        .collect::<Vec<_>>();
    let provider_selection = crate::migration::build_provider_selection_plan_for_candidate(
        &migration_selected,
        &migration_candidates,
    );
    StartingConfigSelection {
        config: candidate.config.clone(),
        import_source: Some(onboard_starting_point_label(
            Some(candidate.source_kind),
            &candidate.source,
        )),
        provider_selection,
        entry_choice: OnboardEntryChoice::ImportDetectedSetup,
        current_setup_state,
        review_candidate: Some(candidate),
    }
}

pub(crate) fn onboard_starting_point_label(
    source_kind: Option<crate::migration::ImportSourceKind>,
    source: &str,
) -> String {
    crate::migration::ImportSourceKind::onboarding_label(source_kind, source)
}

pub(crate) fn import_candidate_from_migration(
    candidate: crate::migration::ImportCandidate,
) -> ImportCandidate {
    ImportCandidate {
        source_kind: candidate.source_kind,
        source: candidate.source,
        config: candidate.config,
        surfaces: candidate
            .surfaces
            .into_iter()
            .map(import_surface_from_migration)
            .collect(),
        domains: candidate.domains,
        channel_candidates: candidate.channel_candidates,
        workspace_guidance: candidate.workspace_guidance,
    }
}

pub(crate) fn migration_candidate_from_onboard(
    candidate: &ImportCandidate,
) -> crate::migration::ImportCandidate {
    crate::migration::ImportCandidate {
        source_kind: candidate.source_kind,
        source: candidate.source.clone(),
        config: candidate.config.clone(),
        surfaces: candidate
            .surfaces
            .iter()
            .map(import_surface_to_migration)
            .collect(),
        domains: candidate.domains.clone(),
        channel_candidates: candidate.channel_candidates.clone(),
        workspace_guidance: candidate.workspace_guidance.clone(),
    }
}

pub(crate) fn migration_candidate_for_onboard_display(
    candidate: &ImportCandidate,
) -> crate::migration::ImportCandidate {
    let mut migration_candidate = migration_candidate_from_onboard(candidate);
    migration_candidate.source =
        onboard_starting_point_label(Some(candidate.source_kind), &candidate.source);
    migration_candidate
}

pub(crate) fn import_surface_from_migration(
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

fn import_surface_to_migration(surface: &ImportSurface) -> crate::migration::ImportSurface {
    crate::migration::ImportSurface {
        name: surface.name,
        domain: surface.domain,
        level: match surface.level {
            ImportSurfaceLevel::Ready => crate::migration::ImportSurfaceLevel::Ready,
            ImportSurfaceLevel::Review => crate::migration::ImportSurfaceLevel::Review,
            ImportSurfaceLevel::Blocked => crate::migration::ImportSurfaceLevel::Blocked,
        },
        detail: surface.detail.clone(),
    }
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

fn recommended_starting_point_candidate(
    import_candidates: &[ImportCandidate],
) -> Option<&ImportCandidate> {
    import_candidates.iter().find(|candidate| {
        candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan
    })
}

fn starting_point_candidate_coverage_breadth(candidate: &ImportCandidate) -> usize {
    collect_detected_coverage_kinds([candidate]).len()
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
