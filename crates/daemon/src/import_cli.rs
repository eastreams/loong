#![allow(clippy::print_stdout)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use loong_app as mvp;
use loong_spec::CliResult;
use serde::Serialize;

use crate::first_run_action_presentation::{
    FirstRunActionGroup, build_first_run_action_text_lines,
};
use crate::migration::{self, ImportCandidate, ImportSourceKind, SetupDomainKind};

#[derive(Debug, Clone)]
pub struct ImportCommandOptions {
    pub output: Option<String>,
    pub force: bool,
    pub preview: bool,
    pub apply: bool,
    pub json: bool,
    pub from: Option<String>,
    pub source_path: Option<String>,
    pub provider: Option<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

pub async fn run_import_cli(options: ImportCommandOptions) -> CliResult<()> {
    let output_path = options
        .output
        .as_deref()
        .map(mvp::config::expand_path)
        .unwrap_or_else(mvp::config::default_config_path);
    let workspace_root = std::env::current_dir()
        .ok()
        .filter(|path| path.join(".git").exists() || path.join("AGENTS.md").exists());
    let detected_codex_paths = migration::discovery::default_detected_codex_config_paths();
    let mut candidates = migration::discovery::collect_import_candidates_with_path_list(
        &output_path,
        &detected_codex_paths,
        workspace_root.as_deref(),
    )?;
    candidates = migration::prepend_recommended_import_candidate(candidates);
    if let Some(source_kind) = options
        .from
        .as_deref()
        .and_then(parse_import_source_selector)
    {
        candidates.retain(|candidate| candidate.source_kind == source_kind);
    } else if options.from.is_some() {
        return Err(format!(
            "unsupported --from value {:?}. supported: {}",
            options.from,
            ImportSourceKind::supported_import_cli_selector_list()
        ));
    }
    let requested_source_path = options.source_path.as_deref().map(mvp::config::expand_path);
    if let Some(requested_source_path) = requested_source_path.as_deref() {
        candidates
            .retain(|candidate| candidate_matches_source_path(candidate, requested_source_path));
    }

    let include = parse_domain_selectors(&options.include, "--include")?;
    let exclude = parse_domain_selectors(&options.exclude, "--exclude")?;
    let candidates = candidates
        .into_iter()
        .filter_map(|candidate| {
            filter_candidate_by_selected_domains(&candidate, &include, &exclude).ok()
        })
        .collect::<Vec<_>>();

    if candidates.is_empty() {
        return Err(if requested_source_path.is_some() {
            "no import candidates matched the selected --source-path".to_owned()
        } else if !include.is_empty() || !exclude.is_empty() {
            "no import candidates matched the selected domain filters".to_owned()
        } else {
            "no import candidates found".to_owned()
        });
    }

    if options.json {
        let payload = render_import_preview_json(&candidates)?;
        println!("{payload}");
        return Ok(());
    }

    let preview_only = !options.apply || options.preview;
    if preview_only {
        for candidate in &candidates {
            println!(
                "{}",
                render_import_preview_shell_text_for_candidates(
                    candidate,
                    &candidates,
                    detect_render_width(),
                )
            );
        }
        if !options.apply {
            return Ok(());
        }
    }

    let candidate_index = select_apply_candidate_index(&candidates)?;
    let candidate = candidates.get(candidate_index).ok_or_else(|| {
        format!(
            "selected import candidate index {candidate_index} is out of range for {} candidate(s)",
            candidates.len()
        )
    })?;
    apply_import_candidate(
        &output_path,
        options.force,
        &candidates,
        candidate,
        options.provider.as_deref(),
    )
}

pub fn parse_import_source_selector(raw: &str) -> Option<ImportSourceKind> {
    ImportSourceKind::parse_import_cli_selector(raw)
}

pub fn parse_import_domain_selector(raw: &str) -> Option<migration::SetupDomainKind> {
    migration::SetupDomainKind::parse_selector(raw)
}

fn parse_domain_selectors(
    raw_values: &[String],
    flag_name: &str,
) -> CliResult<Vec<migration::SetupDomainKind>> {
    let mut domains = Vec::new();
    for raw in raw_values {
        let Some(domain) = parse_import_domain_selector(raw) else {
            return Err(format!(
                "unsupported {flag_name} value {raw:?}. supported: {}",
                migration::SetupDomainKind::supported_selector_list()
            ));
        };
        domains.push(domain);
    }
    Ok(domains)
}

pub fn resolve_selected_domains(
    candidate: &ImportCandidate,
    include: &[migration::SetupDomainKind],
    exclude: &[migration::SetupDomainKind],
) -> Vec<migration::SetupDomainKind> {
    let include_set = include.iter().copied().collect::<BTreeSet<_>>();
    let exclude_set = exclude.iter().copied().collect::<BTreeSet<_>>();
    let mut selected = Vec::new();
    for domain in &candidate.domains {
        let kind = domain.kind;
        if exclude_set.contains(&kind) {
            continue;
        }
        if include_set.is_empty() || include_set.contains(&kind) {
            selected.push(kind);
        }
    }
    selected
}

pub fn apply_selected_domains_to_config(
    base: &mvp::config::LoongConfig,
    candidate: &ImportCandidate,
    selected: &[migration::SetupDomainKind],
) -> mvp::config::LoongConfig {
    let result = apply_selected_domains_to_config_with_report(base, candidate, selected);

    result.config
}

struct SelectedDomainApplyResult {
    config: mvp::config::LoongConfig,
    channel_conflicts: Vec<migration::channels::ChannelApplyConflict>,
}

fn apply_selected_domains_to_config_with_report(
    base: &mvp::config::LoongConfig,
    candidate: &ImportCandidate,
    selected: &[migration::SetupDomainKind],
) -> SelectedDomainApplyResult {
    let mut config = base.clone();
    let mut channel_conflicts = Vec::new();

    for domain in selected {
        match domain {
            migration::SetupDomainKind::Provider => {
                config.provider = candidate.config.provider.clone();
            }
            migration::SetupDomainKind::Channels => {
                let selected_channels = candidate
                    .channel_candidates
                    .iter()
                    .map(|channel| channel.id)
                    .collect::<Vec<_>>();
                let channel_report = migration::channels::apply_selected_channels_with_report(
                    &mut config,
                    &candidate.config,
                    &selected_channels,
                );

                channel_conflicts.extend(channel_report.conflicts);
            }
            migration::SetupDomainKind::Cli => {
                config.cli = candidate.config.cli.clone();
            }
            migration::SetupDomainKind::Memory => {
                config.memory = candidate.config.memory.clone();
            }
            migration::SetupDomainKind::Tools => {
                config.tools = candidate.config.tools.clone();
            }
            migration::SetupDomainKind::WorkspaceGuidance => {}
        }
    }

    SelectedDomainApplyResult {
        config,
        channel_conflicts,
    }
}

fn filter_candidate_by_selected_domains(
    candidate: &ImportCandidate,
    include: &[migration::SetupDomainKind],
    exclude: &[migration::SetupDomainKind],
) -> CliResult<ImportCandidate> {
    let selected = resolve_selected_domains(candidate, include, exclude);
    if selected.is_empty() {
        return Err("no domains selected".to_owned());
    }

    let selected_set = selected.iter().copied().collect::<BTreeSet<_>>();
    Ok(ImportCandidate {
        source_kind: candidate.source_kind,
        source: candidate.source.clone(),
        config: candidate.config.clone(),
        surfaces: candidate
            .surfaces
            .iter()
            .filter(|surface| surface_matches_selected_domains(surface, &selected_set))
            .cloned()
            .collect(),
        domains: candidate
            .domains
            .iter()
            .filter(|domain| selected_set.contains(&domain.kind))
            .cloned()
            .collect(),
        channel_candidates: if selected_set.contains(&migration::SetupDomainKind::Channels) {
            candidate.channel_candidates.clone()
        } else {
            Vec::new()
        },
        workspace_guidance: if selected_set.contains(&migration::SetupDomainKind::WorkspaceGuidance)
        {
            candidate.workspace_guidance.clone()
        } else {
            Vec::new()
        },
    })
}

pub fn surface_matches_selected_domains(
    surface: &migration::ImportSurface,
    selected: &BTreeSet<migration::SetupDomainKind>,
) -> bool {
    selected.contains(&surface.domain)
}

fn candidate_matches_source_path(
    candidate: &ImportCandidate,
    requested_source_path: &Path,
) -> bool {
    crate::source_presentation::source_path(Some(candidate.source_kind), &candidate.source)
        .is_some_and(|candidate_path| {
            let resolved_candidate_path =
                dunce::canonicalize(&candidate_path).unwrap_or(candidate_path);
            let resolved_requested_path = dunce::canonicalize(requested_source_path)
                .unwrap_or_else(|_| requested_source_path.to_path_buf());
            resolved_candidate_path == resolved_requested_path
        })
}

pub fn render_import_preview_lines_for_width(
    candidate: &ImportCandidate,
    width: usize,
) -> Vec<String> {
    render_import_preview_lines_for_candidates(candidate, std::slice::from_ref(candidate), width)
}

pub fn render_import_preview_lines_for_candidates(
    candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
    width: usize,
) -> Vec<String> {
    render_import_preview_lines_for_candidates_with_style(candidate, all_candidates, width, false)
}

pub fn render_import_preview_shell_text_for_candidates(
    candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
    width: usize,
) -> String {
    crate::render_operator_shell_surface(
        "import preview",
        "explicit import preview",
        Vec::new(),
        build_import_preview_body_lines(candidate, all_candidates, width),
        Vec::new(),
    )
}

fn render_import_preview_lines_for_candidates_with_style(
    candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let body_lines = build_import_preview_body_lines(candidate, all_candidates, width);
    let mut lines = mvp::presentation::style_brand_lines(
        &mvp::presentation::render_brand_header(
            width,
            &mvp::presentation::BuildVersionInfo::current(),
            Some("explicit import preview"),
        ),
        color_enabled,
    );
    lines.push(String::new());
    lines.extend(body_lines);
    lines
}

fn build_import_preview_body_lines(
    candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
    width: usize,
) -> Vec<String> {
    let mut lines = vec!["import preview".to_owned()];
    if all_candidates.len() > 1 {
        let preview_position = all_candidates
            .iter()
            .position(|item| {
                item.source_kind == candidate.source_kind && item.source == candidate.source
            })
            .map(|index| index + 1)
            .unwrap_or(1);
        lines.push(format!(
            "candidate {} of {}",
            preview_position,
            all_candidates.len()
        ));
    }
    let mut candidate_lines = migration::render::render_candidate_preview_lines(candidate, width);
    let provider_selection =
        migration::build_provider_selection_plan_for_candidate(candidate, all_candidates);
    candidate_lines.extend(migration::render::render_provider_selection_lines(
        &provider_selection,
        width,
    ));
    lines.extend(candidate_lines);
    lines
}

pub fn render_import_apply_summary_lines_for_width(
    output_path: &Path,
    candidate: &ImportCandidate,
    selected_domains: &[SetupDomainKind],
    resolved_config: &mvp::config::LoongConfig,
    supplemented_existing_config: bool,
    width: usize,
) -> Vec<String> {
    render_import_apply_summary_lines_with_style(
        output_path,
        candidate,
        selected_domains,
        resolved_config,
        supplemented_existing_config,
        width,
        false,
    )
}

pub fn render_import_apply_summary_shell_text_for_width(
    output_path: &Path,
    candidate: &ImportCandidate,
    selected_domains: &[SetupDomainKind],
    resolved_config: &mvp::config::LoongConfig,
    supplemented_existing_config: bool,
    width: usize,
) -> String {
    crate::render_operator_shell_surface(
        "import applied",
        "explicit import applied",
        Vec::new(),
        build_import_apply_summary_body_lines(
            output_path,
            candidate,
            selected_domains,
            resolved_config,
            supplemented_existing_config,
            width,
        ),
        Vec::new(),
    )
}

fn render_import_apply_summary_lines_with_style(
    output_path: &Path,
    candidate: &ImportCandidate,
    selected_domains: &[SetupDomainKind],
    resolved_config: &mvp::config::LoongConfig,
    supplemented_existing_config: bool,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let body_lines = build_import_apply_summary_body_lines(
        output_path,
        candidate,
        selected_domains,
        resolved_config,
        supplemented_existing_config,
        width,
    );
    let mut lines = mvp::presentation::style_brand_lines(
        &mvp::presentation::render_compact_brand_header(
            width,
            &mvp::presentation::BuildVersionInfo::current(),
            Some("explicit import applied"),
        ),
        color_enabled,
    );
    lines.push(String::new());
    lines.extend(body_lines);
    lines
}

fn build_import_apply_summary_body_lines(
    output_path: &Path,
    candidate: &ImportCandidate,
    selected_domains: &[SetupDomainKind],
    resolved_config: &mvp::config::LoongConfig,
    supplemented_existing_config: bool,
    width: usize,
) -> Vec<String> {
    let write_mode = if supplemented_existing_config {
        "supplemented existing config"
    } else {
        "created new config"
    };
    let domains = selected_domains
        .iter()
        .map(|domain| domain.label())
        .collect::<Vec<_>>();
    let mut lines = vec!["import applied".to_owned(), "saved setup".to_owned()];
    lines.extend(mvp::presentation::render_wrapped_text_line(
        "- write mode: ",
        write_mode,
        width,
    ));
    lines.extend(mvp::presentation::render_wrapped_text_line(
        "- source: ",
        &candidate.source,
        width,
    ));
    lines.extend(mvp::presentation::render_wrapped_text_line(
        "- config: ",
        &output_path.display().to_string(),
        width,
    ));
    let config_path = output_path.display().to_string();
    lines.extend(mvp::presentation::render_wrapped_csv_line(
        "- domains: ",
        &domains,
        width,
    ));
    if selected_domains.contains(&SetupDomainKind::Provider) {
        let provider_summary = candidate
            .domains
            .iter()
            .find(|domain| domain.kind == SetupDomainKind::Provider)
            .map(|domain| domain.summary.clone())
            .unwrap_or_else(|| {
                crate::provider_presentation::provider_identity_summary(&resolved_config.provider)
            });
        lines.extend(mvp::presentation::render_wrapped_text_line(
            "- provider: ",
            &provider_summary,
            width,
        ));
        lines.extend(mvp::presentation::render_wrapped_text_line(
            "- transport: ",
            &resolved_config.provider.transport_readiness().summary,
            width,
        ));
        lines.extend(
            crate::provider_presentation::render_provider_profile_state_lines(
                resolved_config,
                width,
                None,
            ),
        );
    }
    if let Some(channels) = candidate
        .domains
        .iter()
        .find(|domain| domain.kind == SetupDomainKind::Channels)
    {
        lines.extend(mvp::presentation::render_wrapped_text_line(
            "- channels: ",
            &channels.summary,
            width,
        ));
    }
    if let Some(guidance) = candidate
        .domains
        .iter()
        .find(|domain| domain.kind == SetupDomainKind::WorkspaceGuidance)
    {
        lines.extend(mvp::presentation::render_wrapped_text_line(
            "- workspace guidance: ",
            &guidance.summary,
            width,
        ));
    }
    let next_actions =
        crate::next_actions::collect_setup_next_actions(resolved_config, &config_path);
    lines.extend(build_first_run_action_text_lines(
        &next_actions,
        width,
        |action| {
            if action.kind == crate::next_actions::SetupNextActionKind::Channel
                || action.kind == crate::next_actions::SetupNextActionKind::BrowserPreview
            {
                FirstRunActionGroup::ContinueSetup
            } else {
                FirstRunActionGroup::GeneralFollowup
            }
        },
        |action, width| {
            mvp::presentation::render_wrapped_text_line("next step: ", &action.command, width)
        },
        |action, width| {
            let prefix = if action.kind == crate::next_actions::SetupNextActionKind::Channel
                || action.kind == crate::next_actions::SetupNextActionKind::BrowserPreview
            {
                "continue setup: "
            } else {
                "also available: "
            };
            mvp::presentation::render_wrapped_text_line(
                prefix,
                &format!("{} · {}", action.label, action.command),
                width,
            )
        },
    ));
    lines
}

#[derive(Serialize)]
struct ImportPreviewJson {
    source_kind: ImportSourceKind,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_path: Option<String>,
    surfaces: Vec<migration::ImportSurface>,
    domains: Vec<migration::DomainPreview>,
    channel_candidates: Vec<migration::ChannelCandidate>,
    workspace_guidance: Vec<migration::WorkspaceGuidanceCandidate>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    provider_profiles: Vec<ImportPreviewProviderProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_provider: Option<String>,
    provider_selection: Option<ImportPreviewProviderSelection>,
}

#[derive(Serialize)]
struct ImportPreviewProviderSelection {
    required: bool,
    choices: Vec<ImportPreviewProviderChoice>,
}

#[derive(Serialize)]
struct ImportPreviewProviderChoice {
    profile_id: String,
    accepted_selectors: Vec<String>,
    kind: String,
    source: String,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    transport: Option<String>,
    selected_by_default: bool,
}

#[derive(Serialize)]
struct ImportPreviewProviderProfile {
    profile_id: String,
    accepted_selectors: Vec<String>,
    kind: String,
    source: String,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    transport: Option<String>,
    active_candidate: bool,
}

pub fn render_import_preview_json(candidates: &[ImportCandidate]) -> CliResult<String> {
    let preview = candidates
        .iter()
        .map(|candidate| {
            let provider_plan =
                migration::build_provider_selection_plan_for_candidate(candidate, candidates);
            let provider_selection = if provider_plan.imported_choices.is_empty() {
                None
            } else {
                Some(ImportPreviewProviderSelection {
                    required: provider_plan.requires_explicit_choice,
                    choices: provider_plan
                        .imported_choices
                        .iter()
                        .map(|choice| ImportPreviewProviderChoice {
                            profile_id: choice.profile_id.clone(),
                            accepted_selectors: migration::accepted_selectors_for_choice(
                                &provider_plan,
                                &choice.profile_id,
                            ),
                            kind: choice.kind.profile().id.to_owned(),
                            source: choice.source.clone(),
                            summary: choice.summary.clone(),
                            transport: choice.config.preview_transport_summary(),
                            selected_by_default: Some(choice.profile_id.as_str())
                                == provider_plan.default_profile_id.as_deref(),
                        })
                        .collect(),
                })
            };
            let provider_profiles = provider_plan
                .imported_choices
                .iter()
                .map(|choice| ImportPreviewProviderProfile {
                    profile_id: choice.profile_id.clone(),
                    accepted_selectors: migration::accepted_selectors_for_choice(
                        &provider_plan,
                        &choice.profile_id,
                    ),
                    kind: choice.kind.profile().id.to_owned(),
                    source: choice.source.clone(),
                    summary: choice.summary.clone(),
                    transport: choice.config.preview_transport_summary(),
                    active_candidate: Some(choice.profile_id.as_str())
                        == provider_plan.default_profile_id.as_deref(),
                })
                .collect::<Vec<_>>();
            ImportPreviewJson {
                source_kind: candidate.source_kind,
                source: candidate.source.clone(),
                source_path: crate::source_presentation::source_path(
                    Some(candidate.source_kind),
                    &candidate.source,
                )
                .map(|path| path.display().to_string()),
                surfaces: candidate.surfaces.clone(),
                domains: candidate.domains.clone(),
                channel_candidates: candidate.channel_candidates.clone(),
                workspace_guidance: candidate.workspace_guidance.clone(),
                provider_profiles,
                active_provider: provider_plan.default_profile_id,
                provider_selection,
            }
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&preview)
        .map_err(|error| format!("serialize import preview failed: {error}"))
}

fn detect_render_width() -> usize {
    mvp::presentation::detect_render_width()
}

pub fn select_apply_candidate_index(candidates: &[ImportCandidate]) -> CliResult<usize> {
    if candidates.len() == 1 {
        return Ok(0);
    }
    if let Some(index) = candidates
        .iter()
        .position(|candidate| candidate.source_kind == ImportSourceKind::RecommendedPlan)
    {
        return Ok(index);
    }
    if let Some(first_kind) = candidates.first().map(|candidate| candidate.source_kind)
        && candidates
            .iter()
            .all(|candidate| candidate.source_kind == first_kind)
    {
        let matched_sources = candidates
            .iter()
            .map(|candidate| candidate.source.as_str())
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(format!(
            "applying import matched multiple {} candidates; rerun with --source-path <path> to choose one, inspect preview/json first, or remove one detected config. matched sources: {}",
            first_kind.import_cli_selector(),
            matched_sources
        ));
    }
    Err(format!(
        "applying import requires a single selected source; use --from {}",
        ImportSourceKind::supported_import_cli_selector_list().replace(", ", "|")
    ))
}

pub fn resolve_import_provider_selection(
    current_provider: &mvp::config::ProviderConfig,
    all_candidates: &[ImportCandidate],
    candidate: &ImportCandidate,
    provider: Option<&str>,
) -> CliResult<mvp::config::ProviderConfig> {
    let provider_selection =
        migration::build_provider_selection_plan_for_candidate(candidate, all_candidates);
    if provider_selection.requires_explicit_choice && provider.is_none() {
        let choices = provider_selection
            .imported_choices
            .iter()
            .map(|choice| choice.profile_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "recommended import plan requires an active provider choice ({choices}); rerun with --provider {} or use {} onboard",
            migration::provider_selection::PROVIDER_SELECTOR_PLACEHOLDER,
            mvp::config::active_cli_command_name(),
        ));
    }

    if let Some(provider_raw) = provider {
        match migration::resolve_choice_by_selector_resolution(&provider_selection, provider_raw) {
            migration::ImportedChoiceSelectorResolution::Match(profile_id) => {
                let choice = provider_selection
                    .imported_choices
                    .iter()
                    .find(|choice| choice.profile_id == profile_id)
                    .expect("resolved provider choice should exist in plan"); // invariant: selector matched
                return Ok(choice.config.clone());
            }
            migration::ImportedChoiceSelectorResolution::Ambiguous(profile_ids) => {
                return Err(migration::format_ambiguous_selector_error(
                    &provider_selection,
                    provider_raw,
                    &profile_ids,
                ));
            }
            migration::ImportedChoiceSelectorResolution::NoMatch => {
                return Err(migration::format_unknown_selector_error(
                    &provider_selection,
                    format!("unsupported --provider value {provider_raw:?}").as_str(),
                ));
            }
        }
    }

    if let Some(default_profile_id) = provider_selection.default_profile_id.as_deref()
        && let Some(choice) =
            migration::resolve_choice_by_selector(&provider_selection, default_profile_id)
    {
        return Ok(choice.config.clone());
    }

    let selected_kind = provider_selection
        .default_kind
        .unwrap_or(candidate.config.provider.kind);

    Ok(migration::resolve_provider_config_from_selection(
        current_provider,
        &provider_selection,
        selected_kind,
    ))
}

fn merge_provider_profile(
    existing: &mvp::config::ProviderConfig,
    incoming: &mvp::config::ProviderConfig,
) -> mvp::config::ProviderConfig {
    migration::provider_selection::merge_provider_config(existing, incoming)
}

fn insert_or_merge_provider_profile(
    profiles: &mut BTreeMap<String, mvp::config::ProviderProfileConfig>,
    provider: &mvp::config::ProviderConfig,
    preferred_profile_id: Option<&str>,
) -> String {
    let incoming_identity = migration::provider_selection::provider_profile_merge_key(provider);
    if let Some((profile_id, profile)) = profiles.iter_mut().find(|(_, profile)| {
        migration::provider_selection::provider_profile_merge_key(&profile.provider)
            == incoming_identity
    }) {
        profile.provider = merge_provider_profile(&profile.provider, provider);
        return profile_id.clone();
    }

    let inferred_profile_id = provider.inferred_profile_id();
    let base_id = preferred_profile_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(inferred_profile_id.as_str())
        .to_owned();
    let mut profile_id = base_id.clone();
    let mut suffix = 2;
    while profiles.contains_key(&profile_id) {
        profile_id = format!("{base_id}-{suffix}");
        suffix += 1;
    }

    let mut profile = mvp::config::ProviderProfileConfig::from_provider(provider.clone());
    if !profiles
        .values()
        .any(|existing| existing.provider.kind == provider.kind)
    {
        profile.default_for_kind = true;
    }
    profiles.insert(profile_id.clone(), profile);
    profile_id
}

fn apply_provider_profiles_to_config(
    base_config: &mvp::config::LoongConfig,
    resolved_config: &mut mvp::config::LoongConfig,
    all_candidates: &[ImportCandidate],
    candidate: &ImportCandidate,
    provider: Option<&str>,
    supplemented_existing_config: bool,
) -> CliResult<()> {
    let provider_selection =
        migration::build_provider_selection_plan_for_candidate(candidate, all_candidates);
    if provider_selection.imported_choices.is_empty() {
        return Ok(());
    }

    let mut profiles = resolved_config.providers.clone();
    let mut inserted_ids = BTreeMap::new();
    for choice in &provider_selection.imported_choices {
        let profile_id = insert_or_merge_provider_profile(
            &mut profiles,
            &choice.config,
            Some(&choice.profile_id),
        );
        inserted_ids.insert(choice.profile_id.clone(), profile_id);
    }

    let active_provider_id = if supplemented_existing_config && provider.is_none() {
        base_config
            .active_provider_id()
            .map(str::to_owned)
            .or_else(|| profiles.keys().next().cloned())
    } else if let Some(provider_raw) = provider {
        let profile_id = match migration::resolve_choice_by_selector_resolution(
            &provider_selection,
            provider_raw,
        ) {
            migration::ImportedChoiceSelectorResolution::Match(profile_id) => profile_id,
            migration::ImportedChoiceSelectorResolution::Ambiguous(profile_ids) => {
                return Err(migration::format_ambiguous_selector_error(
                    &provider_selection,
                    provider_raw,
                    &profile_ids,
                ));
            }
            migration::ImportedChoiceSelectorResolution::NoMatch => {
                return Err(migration::format_unknown_selector_error(
                    &provider_selection,
                    format!("unsupported --provider value {provider_raw:?}").as_str(),
                ));
            }
        };
        inserted_ids.get(&profile_id).cloned()
    } else {
        provider_selection
            .default_profile_id
            .as_deref()
            .and_then(|profile_id| inserted_ids.get(profile_id).cloned())
            .or_else(|| inserted_ids.values().next().cloned())
    };

    resolved_config.providers = profiles;
    if let Some(active_provider_id) = active_provider_id {
        let active_kind = resolved_config
            .providers
            .get(&active_provider_id)
            .map(|profile| profile.provider.kind);
        if let Some(active_kind) = active_kind {
            for profile in resolved_config
                .providers
                .values_mut()
                .filter(|profile| profile.provider.kind == active_kind)
            {
                profile.default_for_kind = false;
            }
            if let Some(active_profile) = resolved_config.providers.get_mut(&active_provider_id) {
                active_profile.default_for_kind = true;
            }
        }
        resolved_config.active_provider = Some(active_provider_id.clone());
        if let Some(active_profile) = resolved_config.providers.get(&active_provider_id) {
            resolved_config.provider = active_profile.provider.clone();
        }
    }
    if supplemented_existing_config && provider.is_none() {
        resolved_config.last_provider = base_config.last_provider_id().map(str::to_owned);
    } else {
        resolved_config.last_provider = base_config
            .active_provider_id()
            .map(str::to_owned)
            .filter(|previous| Some(previous.as_str()) != resolved_config.active_provider_id());
    }
    Ok(())
}

pub fn apply_import_candidate(
    output_path: &Path,
    force: bool,
    all_candidates: &[ImportCandidate],
    candidate: &ImportCandidate,
    provider: Option<&str>,
) -> CliResult<()> {
    let supplemented_existing_config = output_path.exists();
    let base_config = if output_path.exists() {
        let Some(path_str) = output_path.to_str() else {
            return Err(format!(
                "output path {} is not valid utf-8",
                output_path.display()
            ));
        };
        match mvp::config::load(Some(path_str)) {
            Ok((_, config)) => config,
            Err(error) => {
                return Err(format!(
                    "failed to load existing config {} before import: {error}",
                    output_path.display()
                ));
            }
        }
    } else {
        mvp::config::LoongConfig::default()
    };
    let mut selected_domains = candidate
        .domains
        .iter()
        .map(|domain| domain.kind)
        .collect::<Vec<_>>();
    let has_provider_domain = selected_domains.contains(&SetupDomainKind::Provider);
    if provider.is_some() && !has_provider_domain {
        selected_domains.push(SetupDomainKind::Provider);
    }
    if !selected_domains
        .iter()
        .any(|domain| domain_changes_config(*domain))
    {
        return Err(
            "selected domains do not change config; use --preview to inspect workspace guidance"
                .to_owned(),
        );
    }
    let apply_result =
        apply_selected_domains_to_config_with_report(&base_config, candidate, &selected_domains);
    let channel_conflicts = apply_result.channel_conflicts;

    if !channel_conflicts.is_empty() {
        let conflict_detail = format_channel_apply_conflicts(&channel_conflicts);

        return Err(format!(
            "cannot apply selected channel domains: {conflict_detail}"
        ));
    }

    let mut resolved_config = apply_result.config;
    if provider.is_some() || has_provider_domain {
        apply_provider_profiles_to_config(
            &base_config,
            &mut resolved_config,
            all_candidates,
            candidate,
            provider,
            supplemented_existing_config,
        )?;
    }
    let path_string = output_path.display().to_string();
    let path = mvp::config::write(Some(&path_string), &resolved_config, force)?;
    #[cfg(feature = "memory-sqlite")]
    {
        let mem_config = mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(
            &resolved_config.memory,
        );
        let _ = mvp::memory::ensure_memory_db_ready(
            Some(resolved_config.memory.resolved_sqlite_path()),
            &mem_config,
        )
        .map_err(|error| format!("failed to bootstrap sqlite memory: {error}"))?;
    }

    println!(
        "{}",
        render_import_apply_summary_shell_text_for_width(
            &path,
            candidate,
            &selected_domains,
            &resolved_config,
            supplemented_existing_config,
            detect_render_width(),
        )
    );
    Ok(())
}

fn domain_changes_config(domain: migration::SetupDomainKind) -> bool {
    domain.changes_config()
}

fn format_channel_apply_conflicts(
    conflicts: &[migration::channels::ChannelApplyConflict],
) -> String {
    let summaries = conflicts
        .iter()
        .map(migration::channels::summarize_channel_apply_conflict)
        .collect::<Vec<_>>();

    summaries.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_import_candidate() -> ImportCandidate {
        ImportCandidate {
            source_kind: ImportSourceKind::Environment,
            source: "your current environment".to_owned(),
            config: mvp::config::LoongConfig::default(),
            surfaces: Vec::new(),
            domains: Vec::new(),
            channel_candidates: Vec::new(),
            workspace_guidance: Vec::new(),
        }
    }

    #[test]
    fn render_import_apply_summary_lines_separates_continue_setup_actions() {
        let candidate = sample_import_candidate();
        let rendered = render_import_apply_summary_lines_for_width(
            Path::new("/tmp/config.toml"),
            &candidate,
            &[],
            &mvp::config::LoongConfig::default(),
            false,
            100,
        )
        .join("\n");

        assert!(rendered.contains("start here"), "{rendered}");
        assert!(rendered.contains("also available"), "{rendered}");
        assert!(rendered.contains("continue setup"), "{rendered}");
        assert!(
            rendered.contains("also available: chat · loong chat --config '/tmp/config.toml'"),
            "{rendered}"
        );
        assert!(
            rendered.contains("also available: teach Loong your working style ·"),
            "{rendered}"
        );
        assert!(
            rendered.contains("loong personalize --config '/tmp/config.toml'"),
            "{rendered}"
        );
        assert!(
            rendered.contains("continue setup: choose a channel ·"),
            "{rendered}"
        );
        assert!(
            rendered.contains("loong channels --config '/tmp/config.toml'"),
            "{rendered}"
        );
        assert!(
            rendered.contains("continue setup: enable browser preview ·"),
            "{rendered}"
        );
        assert!(
            rendered.contains("loong skills enable-browser-preview --config"),
            "{rendered}"
        );
    }
}
