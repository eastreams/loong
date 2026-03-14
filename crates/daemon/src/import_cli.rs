#![allow(clippy::print_stdout)]

use std::collections::BTreeSet;
use std::path::Path;

use loongclaw_app as mvp;
use loongclaw_spec::CliResult;
use serde::Serialize;

use crate::migration::{self, ImportCandidate, ImportSourceKind, SetupDomainKind};

#[derive(Debug, Clone)]
pub(crate) struct ImportCommandOptions {
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

pub(crate) async fn run_import_cli(options: ImportCommandOptions) -> CliResult<()> {
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
            for line in render_import_preview_lines_for_candidates_with_style(
                candidate,
                &candidates,
                detect_render_width(),
                true,
            ) {
                println!("{line}");
            }
            println!();
        }
        if !options.apply {
            return Ok(());
        }
    }

    let candidate_index = select_apply_candidate_index(&candidates)?;
    apply_import_candidate(
        &output_path,
        options.force,
        &candidates,
        &candidates[candidate_index],
        options.provider.as_deref(),
    )
}

pub(crate) fn parse_import_source_selector(raw: &str) -> Option<ImportSourceKind> {
    ImportSourceKind::parse_import_cli_selector(raw)
}

pub(crate) fn parse_import_domain_selector(raw: &str) -> Option<migration::SetupDomainKind> {
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

pub(crate) fn resolve_selected_domains(
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

pub(crate) fn apply_selected_domains_to_config(
    base: &mvp::config::LoongClawConfig,
    candidate: &ImportCandidate,
    selected: &[migration::SetupDomainKind],
) -> mvp::config::LoongClawConfig {
    let mut config = base.clone();
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
                migration::channels::apply_selected_channels(
                    &mut config,
                    &candidate.config,
                    &selected_channels,
                );
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
    config
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

pub(crate) fn surface_matches_selected_domains(
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
        .is_some_and(|candidate_path| candidate_path == requested_source_path)
}

#[cfg(test)]
pub(crate) fn render_import_preview_lines_for_width(
    candidate: &ImportCandidate,
    width: usize,
) -> Vec<String> {
    render_import_preview_lines_for_candidates(candidate, std::slice::from_ref(candidate), width)
}

#[cfg(test)]
pub(crate) fn render_import_preview_lines_for_candidates(
    candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
    width: usize,
) -> Vec<String> {
    render_import_preview_lines_for_candidates_with_style(candidate, all_candidates, width, false)
}

fn render_import_preview_lines_for_candidates_with_style(
    candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let mut lines = mvp::presentation::style_brand_lines(
        &mvp::presentation::render_brand_header(
            width,
            &mvp::presentation::BuildVersionInfo::current(),
            Some("explicit import preview"),
        ),
        color_enabled,
    );
    lines.push(String::new());
    lines.push("import preview".to_owned());
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

#[cfg(test)]
pub(crate) fn render_import_apply_summary_lines_for_width(
    output_path: &Path,
    candidate: &ImportCandidate,
    selected_domains: &[SetupDomainKind],
    resolved_config: &mvp::config::LoongClawConfig,
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

fn render_import_apply_summary_lines_with_style(
    output_path: &Path,
    candidate: &ImportCandidate,
    selected_domains: &[SetupDomainKind],
    resolved_config: &mvp::config::LoongClawConfig,
    supplemented_existing_config: bool,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let mut lines = mvp::presentation::style_brand_lines(
        &mvp::presentation::render_compact_brand_header(
            width,
            &mvp::presentation::BuildVersionInfo::current(),
            Some("explicit import applied"),
        ),
        color_enabled,
    );
    let write_mode = if supplemented_existing_config {
        "supplemented existing config"
    } else {
        "created new config"
    };
    let domains = selected_domains
        .iter()
        .map(|domain| domain.label())
        .collect::<Vec<_>>();

    lines.push(String::new());
    lines.push("import applied".to_owned());
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
    if let Some((primary, secondary)) = next_actions.split_first() {
        lines.extend(mvp::presentation::render_wrapped_text_line(
            "next step: ",
            &primary.command,
            width,
        ));
        for action in secondary {
            lines.extend(mvp::presentation::render_wrapped_text_line(
                "also available: ",
                &format!("{} · {}", action.label, action.command),
                width,
            ));
        }
    }
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
    provider_selection: Option<ImportPreviewProviderSelection>,
}

#[derive(Serialize)]
struct ImportPreviewProviderSelection {
    required: bool,
    choices: Vec<ImportPreviewProviderChoice>,
}

#[derive(Serialize)]
struct ImportPreviewProviderChoice {
    kind: String,
    source: String,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    transport: Option<String>,
    selected_by_default: bool,
}

pub(crate) fn render_import_preview_json(candidates: &[ImportCandidate]) -> CliResult<String> {
    let preview = candidates
        .iter()
        .map(|candidate| {
            let provider_selection =
                migration::build_provider_selection_plan_for_candidate(candidate, candidates);
            let provider_selection = if provider_selection.imported_choices.is_empty() {
                None
            } else {
                Some(ImportPreviewProviderSelection {
                    required: provider_selection.requires_explicit_choice,
                    choices: provider_selection
                        .imported_choices
                        .iter()
                        .map(|choice| ImportPreviewProviderChoice {
                            kind: choice.kind.profile().id.to_owned(),
                            source: choice.source.clone(),
                            summary: choice.summary.clone(),
                            transport: choice.config.preview_transport_summary(),
                            selected_by_default: Some(choice.kind)
                                == provider_selection.default_kind,
                        })
                        .collect(),
                })
            };
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
                provider_selection,
            }
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&preview)
        .map_err(|error| format!("serialize import preview failed: {error}"))
}

fn detect_render_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|width| *width > 0)
        .unwrap_or(80)
}

pub(crate) fn select_apply_candidate_index(candidates: &[ImportCandidate]) -> CliResult<usize> {
    if candidates.len() == 1 {
        return Ok(0);
    }
    if let Some(index) = candidates
        .iter()
        .position(|candidate| candidate.source_kind == ImportSourceKind::RecommendedPlan)
    {
        return Ok(index);
    }
    if let Some(first_kind) = candidates.first().map(|candidate| candidate.source_kind) {
        if candidates
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
    }
    Err(format!(
        "applying import requires a single selected source; use --from {}",
        ImportSourceKind::supported_import_cli_selector_list().replace(", ", "|")
    ))
}

pub(crate) fn resolve_import_provider_selection(
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
            .map(|choice| choice.kind.profile().id)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "recommended import plan requires an active provider choice ({choices}); rerun with --provider <id> or use loongclaw onboard"
        ));
    }

    let selected_kind = if let Some(provider_raw) = provider {
        let Some(provider_kind) = crate::onboard_cli::parse_provider_kind(provider_raw) else {
            return Err(format!(
                "unsupported --provider value {:?}. supported: {}",
                provider_raw,
                crate::onboard_cli::supported_provider_list()
            ));
        };
        provider_kind
    } else {
        provider_selection
            .default_kind
            .unwrap_or(candidate.config.provider.kind)
    };

    Ok(migration::resolve_provider_config_from_selection(
        current_provider,
        &provider_selection,
        selected_kind,
    ))
}

fn apply_import_candidate(
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
        mvp::config::LoongClawConfig::default()
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
    let mut resolved_config =
        apply_selected_domains_to_config(&base_config, candidate, &selected_domains);
    if provider.is_some() || has_provider_domain {
        resolved_config.provider = resolve_import_provider_selection(
            &base_config.provider,
            all_candidates,
            candidate,
            provider,
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

    for line in render_import_apply_summary_lines_with_style(
        &path,
        candidate,
        &selected_domains,
        &resolved_config,
        supplemented_existing_config,
        detect_render_width(),
        true,
    ) {
        println!("{line}");
    }
    Ok(())
}

fn domain_changes_config(domain: migration::SetupDomainKind) -> bool {
    domain.changes_config()
}
