use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use loongclaw_app as mvp;
use loongclaw_spec::CliResult;
use serde::Deserialize;

use super::channels;
use super::provider_transport::ImportedProviderTransport;
use super::types::{
    ChannelCandidate, ChannelImportReadiness, CurrentSetupState, DomainPreview, ImportCandidate,
    ImportSourceKind, ImportSurface, ImportSurfaceLevel, PreviewStatus, SetupDomainKind,
    WorkspaceGuidanceCandidate, WorkspaceGuidanceKind,
};

#[derive(Debug, Deserialize)]
struct CodexImportConfig {
    model_provider: Option<String>,
    model: Option<String>,
    #[serde(default)]
    model_providers: std::collections::BTreeMap<String, CodexModelProviderConfig>,
}

#[derive(Debug, Deserialize)]
struct CodexModelProviderConfig {
    base_url: Option<String>,
    chat_completions_path: Option<String>,
    wire_api: Option<String>,
    requires_openai_auth: Option<bool>,
}

pub fn classify_current_setup(output_path: &Path) -> CurrentSetupState {
    if !output_path.exists() {
        return CurrentSetupState::Absent;
    }
    let Some(path_str) = output_path.to_str() else {
        return CurrentSetupState::Repairable;
    };
    let Ok((_, config)) = mvp::config::load(Some(path_str)) else {
        return CurrentSetupState::Repairable;
    };
    let readiness = resolve_channel_import_readiness_from_config(&config);
    let has_provider_auth = config.provider.authorization_header().is_some();
    let channel_blockers = channels::enabled_channels_have_blockers(&config, &readiness);
    if channel_blockers {
        return CurrentSetupState::Repairable;
    }

    let default_config = mvp::config::LoongClawConfig::default();
    let has_only_provider_selection_changes = config.provider.has_only_selection_changes()
        && config.cli.enabled == default_config.cli.enabled
        && config.cli.system_prompt == default_config.cli.system_prompt
        && config.cli.prompt_pack_id == default_config.cli.prompt_pack_id
        && config.cli.personality == default_config.cli.personality
        && config.cli.system_prompt_addendum == default_config.cli.system_prompt_addendum
        && config.cli.exit_commands == default_config.cli.exit_commands
        && channels::registered_enabled_channel_ids(&config).is_empty()
        && config.tools.shell_allow == default_config.tools.shell_allow
        && config.tools.file_root == default_config.tools.file_root
        && config.memory.profile == default_config.memory.profile
        && memory_sqlite_path_looks_default(&config.memory.sqlite_path, &default_config.memory)
        && config.memory.sliding_window == default_config.memory.sliding_window;

    if has_only_provider_selection_changes && !has_provider_auth {
        return CurrentSetupState::LegacyOrIncomplete;
    }
    if has_provider_auth {
        return CurrentSetupState::Healthy;
    }
    CurrentSetupState::Repairable
}

#[allow(dead_code)]
pub fn collect_import_candidates_with_paths(
    output_path: &Path,
    codex_config_path: Option<&Path>,
    workspace_root: Option<&Path>,
) -> CliResult<Vec<ImportCandidate>> {
    let codex_config_paths = codex_config_path
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    collect_import_candidates_with_path_list(output_path, &codex_config_paths, workspace_root)
}

pub fn collect_import_candidates_with_path_list(
    output_path: &Path,
    codex_config_paths: &[PathBuf],
    workspace_root: Option<&Path>,
) -> CliResult<Vec<ImportCandidate>> {
    let readiness =
        resolve_channel_import_readiness_from_config(&mvp::config::LoongClawConfig::default());
    collect_import_candidates_with_path_list_and_readiness(
        output_path,
        codex_config_paths,
        workspace_root,
        readiness,
    )
}

pub fn collect_import_candidates_with_paths_and_readiness(
    output_path: &Path,
    codex_config_path: Option<&Path>,
    workspace_root: Option<&Path>,
    readiness: ChannelImportReadiness,
) -> CliResult<Vec<ImportCandidate>> {
    let codex_config_paths = codex_config_path
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    collect_import_candidates_with_path_list_and_readiness(
        output_path,
        &codex_config_paths,
        workspace_root,
        readiness,
    )
}

pub fn collect_import_candidates_with_path_list_and_readiness(
    output_path: &Path,
    codex_config_paths: &[PathBuf],
    workspace_root: Option<&Path>,
    readiness: ChannelImportReadiness,
) -> CliResult<Vec<ImportCandidate>> {
    let guidance = workspace_root
        .map(detect_workspace_guidance)
        .unwrap_or_default();
    let mut candidates = Vec::new();

    if output_path.exists() {
        let Some(path_str) = output_path.to_str() else {
            return Ok(candidates);
        };
        match mvp::config::load(Some(path_str)) {
            Ok((_, config)) => {
                if let Some(candidate) = build_import_candidate(
                    ImportSourceKind::ExistingLoongClawConfig,
                    crate::source_presentation::existing_loongclaw_config_source_label(output_path),
                    config,
                    resolve_channel_import_readiness_from_config,
                    guidance.clone(),
                ) {
                    candidates.push(candidate);
                }
            }
            Err(error) => {
                println!(
                    "Detected existing config at {} but could not import it: {error}",
                    output_path.display()
                );
            }
        }
    }

    let mut seen_codex_paths = BTreeSet::new();
    for path in codex_config_paths {
        if !seen_codex_paths.insert(path.clone()) {
            continue;
        }
        match load_codex_import_candidate(path, readiness.clone(), guidance.clone()) {
            Ok(Some(candidate)) => candidates.push(candidate),
            Ok(None) => {}
            Err(error) => {
                println!(
                    "Skipped Codex migration candidate at {}: {error}",
                    path.display()
                );
            }
        }
    }

    let env_config = detect_import_starting_config_with_channel_readiness(readiness.clone());
    if let Some(candidate) = build_import_candidate(
        ImportSourceKind::Environment,
        crate::source_presentation::environment_source_label().to_owned(),
        env_config,
        move |_| readiness.clone(),
        guidance,
    ) {
        candidates.push(candidate);
    }

    Ok(candidates)
}

pub fn build_import_candidate(
    source_kind: ImportSourceKind,
    source: String,
    config: mvp::config::LoongClawConfig,
    readiness: impl Fn(&mvp::config::LoongClawConfig) -> ChannelImportReadiness,
    workspace_guidance: Vec<WorkspaceGuidanceCandidate>,
) -> Option<ImportCandidate> {
    let resolved_readiness = readiness(&config);
    let surfaces = collect_import_surfaces_with_channel_readiness(&config, &resolved_readiness);
    let channel_candidates = collect_channel_candidates(&config, &resolved_readiness, &source);
    let domains = collect_domain_previews(
        source_kind,
        &config,
        &source,
        &channel_candidates,
        &workspace_guidance,
    );
    if surfaces.is_empty() && domains.is_empty() && workspace_guidance.is_empty() {
        return None;
    }
    Some(ImportCandidate {
        source_kind,
        source,
        config,
        surfaces,
        domains,
        channel_candidates,
        workspace_guidance,
    })
}

pub fn detect_import_starting_config_with_channel_readiness(
    readiness: ChannelImportReadiness,
) -> mvp::config::LoongClawConfig {
    apply_channel_import_readiness(mvp::config::LoongClawConfig::default(), readiness)
}

fn apply_channel_import_readiness(
    mut config: mvp::config::LoongClawConfig,
    readiness: ChannelImportReadiness,
) -> mvp::config::LoongClawConfig {
    channels::apply_detected_import_readiness(&mut config, &readiness);
    config
}

pub fn resolve_channel_import_readiness_from_config(
    config: &mvp::config::LoongClawConfig,
) -> ChannelImportReadiness {
    channels::resolve_import_readiness(config)
}

pub fn detect_workspace_guidance(root: &Path) -> Vec<WorkspaceGuidanceCandidate> {
    let mut guidance = Vec::new();
    for kind in [
        WorkspaceGuidanceKind::Agents,
        WorkspaceGuidanceKind::Claude,
        WorkspaceGuidanceKind::Gemini,
        WorkspaceGuidanceKind::Opencode,
    ] {
        let path = root.join(kind.file_name());
        if path.is_file() {
            guidance.push(WorkspaceGuidanceCandidate {
                kind,
                path: path.display().to_string(),
            });
        }
    }
    guidance
}

pub fn collect_import_surfaces(config: &mvp::config::LoongClawConfig) -> Vec<ImportSurface> {
    collect_import_surfaces_with_channel_readiness(
        config,
        &resolve_channel_import_readiness_from_config(config),
    )
}

pub fn collect_import_surfaces_with_channel_readiness(
    config: &mvp::config::LoongClawConfig,
    readiness: &ChannelImportReadiness,
) -> Vec<ImportSurface> {
    let mut surfaces = Vec::new();
    if let Some(surface) = provider_import_surface(config) {
        surfaces.push(surface);
    }
    if let Some(surface) = cli_import_surface(config) {
        surfaces.push(surface);
    }
    surfaces.extend(
        channels::collect_channel_previews(config, readiness, "")
            .into_iter()
            .map(|preview| preview.surface),
    );
    surfaces
}

fn collect_channel_candidates(
    config: &mvp::config::LoongClawConfig,
    readiness: &ChannelImportReadiness,
    source: &str,
) -> Vec<ChannelCandidate> {
    channels::collect_channel_previews(config, readiness, source)
        .into_iter()
        .map(|preview| preview.candidate)
        .collect()
}

fn collect_domain_previews(
    source_kind: ImportSourceKind,
    config: &mvp::config::LoongClawConfig,
    source: &str,
    channel_candidates: &[ChannelCandidate],
    workspace_guidance: &[WorkspaceGuidanceCandidate],
) -> Vec<DomainPreview> {
    let mut domains = Vec::new();
    if let Some(surface) = provider_import_surface(config) {
        domains.push(DomainPreview {
            kind: SetupDomainKind::Provider,
            status: map_surface_level(surface.level),
            decision: source_kind.default_domain_decision(),
            source: source.to_owned(),
            summary: surface.detail,
        });
    }
    if let Some(surface) = cli_import_surface(config) {
        domains.push(DomainPreview {
            kind: SetupDomainKind::Cli,
            status: PreviewStatus::Ready,
            decision: source_kind.default_domain_decision(),
            source: source.to_owned(),
            summary: surface.detail,
        });
    }
    if !channel_candidates.is_empty() {
        let status = if channel_candidates
            .iter()
            .all(|channel| channel.status == PreviewStatus::Ready)
        {
            PreviewStatus::Ready
        } else if channel_candidates
            .iter()
            .any(|channel| channel.status == PreviewStatus::NeedsReview)
        {
            PreviewStatus::NeedsReview
        } else {
            PreviewStatus::Unavailable
        };
        let summary = channel_candidates
            .iter()
            .map(|channel| format!("{} {}", channel.label, channel.status.label()))
            .collect::<Vec<_>>()
            .join(" · ");
        domains.push(DomainPreview {
            kind: SetupDomainKind::Channels,
            status,
            decision: source_kind.default_domain_decision(),
            source: source.to_owned(),
            summary,
        });
    }

    let default_memory = mvp::config::MemoryConfig::default();
    if config.memory.profile != default_memory.profile
        || !memory_sqlite_path_looks_default(&config.memory.sqlite_path, &default_memory)
        || config.memory.sliding_window != default_memory.sliding_window
    {
        domains.push(DomainPreview {
            kind: SetupDomainKind::Memory,
            status: PreviewStatus::Ready,
            decision: source_kind.default_domain_decision(),
            source: source.to_owned(),
            summary: memory_behavior_summary(&config.memory),
        });
    }

    let default_tools = mvp::config::ToolConfig::default();
    if config.tools.shell_allow != default_tools.shell_allow
        || config.tools.file_root != default_tools.file_root
    {
        let mut parts = Vec::new();
        if config.tools.file_root != default_tools.file_root {
            parts.push(format!(
                "workspace root {}",
                config.tools.resolved_file_root().display()
            ));
        }
        if config.tools.shell_allow != default_tools.shell_allow {
            parts.push(format!(
                "shell permissions {}",
                config.tools.shell_allow.join(", ")
            ));
        }
        domains.push(DomainPreview {
            kind: SetupDomainKind::Tools,
            status: PreviewStatus::NeedsReview,
            decision: source_kind.default_domain_decision(),
            source: source.to_owned(),
            summary: parts.join(" · "),
        });
    }

    if !workspace_guidance.is_empty() {
        let summary = workspace_guidance
            .iter()
            .map(|item| {
                Path::new(&item.path)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| item.path.clone())
            })
            .collect::<Vec<_>>()
            .join(", ");
        domains.push(DomainPreview {
            kind: SetupDomainKind::WorkspaceGuidance,
            status: PreviewStatus::Ready,
            decision: source_kind.default_domain_decision(),
            source: crate::source_presentation::workspace_source_label().to_owned(),
            summary,
        });
    }

    domains
}

fn memory_sqlite_path_looks_default(
    sqlite_path: &str,
    default_memory: &mvp::config::MemoryConfig,
) -> bool {
    if sqlite_path == default_memory.sqlite_path {
        return true;
    }

    let current_default_path = Path::new(default_memory.sqlite_path.as_str());
    let candidate_path = Path::new(sqlite_path);
    candidate_path.file_name() == current_default_path.file_name()
        && candidate_path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|component| component == ".loongclaw")
}

fn map_surface_level(level: ImportSurfaceLevel) -> PreviewStatus {
    match level {
        ImportSurfaceLevel::Ready => PreviewStatus::Ready,
        ImportSurfaceLevel::Review => PreviewStatus::NeedsReview,
        ImportSurfaceLevel::Blocked => PreviewStatus::Unavailable,
    }
}

fn default_codex_config_paths() -> Vec<PathBuf> {
    let Some(home) = env::var_os("HOME") else {
        return Vec::new();
    };
    let home = PathBuf::from(home);
    let mut seen = BTreeSet::new();
    let mut paths = Vec::new();
    let base_codex_path = home.join(".codex/config.toml");
    let default_agent_codex_path = home
        .join(".codex/agents")
        .join(mvp::config::CLI_COMMAND_NAME)
        .join("config.toml");
    let legacy_agent_codex_path = home
        .join(".codex/agents")
        .join(mvp::config::LEGACY_CLI_COMMAND_NAME)
        .join("config.toml");
    for path in [
        base_codex_path,
        default_agent_codex_path,
        legacy_agent_codex_path,
    ] {
        if path.is_file() && seen.insert(path.clone()) {
            paths.push(path);
        }
    }
    paths
}

fn load_codex_import_candidate(
    path: &Path,
    readiness: ChannelImportReadiness,
    workspace_guidance: Vec<WorkspaceGuidanceCandidate>,
) -> CliResult<Option<ImportCandidate>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("failed to read Codex config {}: {error}", path.display()))?;
    let parsed: CodexImportConfig = toml::from_str(&raw)
        .map_err(|error| format!("failed to parse Codex config {}: {error}", path.display()))?;
    let Some(config) = codex_import_config_to_loongclaw(parsed, readiness.clone())? else {
        return Ok(None);
    };
    Ok(build_import_candidate(
        ImportSourceKind::CodexConfig,
        crate::source_presentation::codex_config_source_label(path),
        config,
        move |_| readiness.clone(),
        workspace_guidance,
    ))
}

pub fn default_detected_codex_config_paths() -> Vec<PathBuf> {
    default_codex_config_paths()
}

fn codex_import_config_to_loongclaw(
    parsed: CodexImportConfig,
    readiness: ChannelImportReadiness,
) -> CliResult<Option<mvp::config::LoongClawConfig>> {
    let Some(model_provider) = parsed.model_provider.as_deref().map(str::trim) else {
        return Ok(None);
    };
    if model_provider.is_empty() {
        return Ok(None);
    }

    let provider_section = parsed.model_providers.get(model_provider);
    let Some(provider_kind) = codex_provider_kind(model_provider, provider_section) else {
        return Err(format!(
            "unsupported Codex model_provider {model_provider:?}; add a recognized provider id or an OpenAI-compatible provider section with base_url plus wire_api/requires_openai_auth"
        ));
    };
    let mut config =
        apply_channel_import_readiness(mvp::config::LoongClawConfig::default(), readiness);
    config.provider = baseline_codex_import_provider_config(provider_kind);
    if let Some(model) = parsed
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        config.provider.model = model.to_owned();
    }

    let transport = resolve_codex_provider_transport(config.provider.kind, provider_section);
    transport.apply_to_provider(&mut config.provider);

    if provider_section.and_then(|provider| provider.requires_openai_auth) == Some(true) {
        let suggested_env = config
            .provider
            .kind
            .default_api_key_env()
            .unwrap_or("OPENAI_API_KEY");
        config
            .provider
            .set_api_key_env_binding(Some(suggested_env.to_owned()));
    }

    Ok(Some(config))
}

fn baseline_codex_import_provider_config(
    provider_kind: mvp::config::ProviderKind,
) -> mvp::config::ProviderConfig {
    let mut provider = mvp::config::ProviderConfig {
        kind: provider_kind,
        ..mvp::config::ProviderConfig::default()
    };
    provider.set_api_key_env_binding(provider_kind.default_api_key_env().map(str::to_owned));
    ImportedProviderTransport::default_for_kind(provider_kind).apply_to_provider(&mut provider);
    provider
}

fn resolve_codex_provider_transport(
    provider_kind: mvp::config::ProviderKind,
    provider_section: Option<&CodexModelProviderConfig>,
) -> ImportedProviderTransport {
    ImportedProviderTransport::from_optional_overrides(
        provider_kind,
        provider_section.and_then(|provider| provider.base_url.as_deref()),
        provider_section.and_then(|provider| provider.chat_completions_path.as_deref()),
        provider_section
            .and_then(|provider| provider.wire_api.as_deref())
            .and_then(mvp::config::ProviderWireApi::parse),
    )
}

fn codex_provider_kind(
    model_provider: &str,
    provider_section: Option<&CodexModelProviderConfig>,
) -> Option<mvp::config::ProviderKind> {
    mvp::config::ProviderKind::parse(model_provider).or_else(|| {
        codex_provider_looks_openai_compatible(provider_section)
            .then_some(mvp::config::ProviderKind::Openai)
    })
}

fn codex_provider_looks_openai_compatible(
    provider_section: Option<&CodexModelProviderConfig>,
) -> bool {
    let Some(provider) = provider_section else {
        return false;
    };

    let has_base_url = provider
        .base_url
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if !has_base_url {
        return false;
    }

    provider.requires_openai_auth == Some(true)
        || provider
            .wire_api
            .as_deref()
            .is_some_and(codex_wire_api_looks_openai_compatible)
}

fn codex_wire_api_looks_openai_compatible(raw: &str) -> bool {
    mvp::config::ProviderWireApi::parse(raw).is_some()
}

fn provider_import_surface(config: &mvp::config::LoongClawConfig) -> Option<ImportSurface> {
    let provider_changed = config.provider.differs_from_default();
    let credentials_ready = config.provider.authorization_header().is_some();
    if !provider_changed && !credentials_ready {
        return None;
    }
    Some(ImportSurface {
        name: "provider",
        domain: SetupDomainKind::Provider,
        level: if credentials_ready {
            ImportSurfaceLevel::Ready
        } else {
            ImportSurfaceLevel::Review
        },
        detail: crate::provider_presentation::provider_identity_summary(&config.provider),
    })
}

fn cli_import_surface(config: &mvp::config::LoongClawConfig) -> Option<ImportSurface> {
    let default_cli = mvp::config::CliChannelConfig::default();
    if config.cli.enabled == default_cli.enabled
        && config.cli.system_prompt == default_cli.system_prompt
        && config.cli.prompt_pack_id == default_cli.prompt_pack_id
        && config.cli.personality == default_cli.personality
        && config.cli.system_prompt_addendum == default_cli.system_prompt_addendum
        && config.cli.exit_commands == default_cli.exit_commands
    {
        return None;
    }
    Some(ImportSurface {
        name: "cli channel",
        domain: SetupDomainKind::Cli,
        level: ImportSurfaceLevel::Ready,
        detail: cli_behavior_summary(&config.cli),
    })
}

fn cli_behavior_summary(config: &mvp::config::CliChannelConfig) -> String {
    let default_cli = mvp::config::CliChannelConfig::default();
    let mut parts = Vec::new();
    if config.uses_native_prompt_pack() {
        parts.push("native prompt pack".to_owned());
        parts.push(format!(
            "personality {}",
            crate::onboard_cli::prompt_personality_id(config.resolved_personality())
        ));
        if config
            .system_prompt_addendum
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        {
            parts.push("prompt addendum configured".to_owned());
        }
    } else if !config.system_prompt.trim().is_empty() {
        parts.push("inline system prompt override".to_owned());
    }
    if config.exit_commands != default_cli.exit_commands {
        parts.push(format!("exit commands {}", config.exit_commands.join(", ")));
    }
    if parts.is_empty() {
        "custom CLI behavior detected".to_owned()
    } else {
        parts.join(" · ")
    }
}

fn memory_behavior_summary(config: &mvp::config::MemoryConfig) -> String {
    format!(
        "profile {} · {} · window {}",
        config.profile.as_str(),
        config.resolved_sqlite_path().display(),
        config.sliding_window
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_import_surface_detects_prompt_pack_metadata_changes() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.cli.personality = Some(mvp::prompt::PromptPersonality::FriendlyCollab);

        let surfaces = collect_import_surfaces(&config);

        assert!(
            surfaces
                .iter()
                .any(|surface| surface.domain == SetupDomainKind::Cli),
            "changing prompt-pack personality metadata should mark the CLI domain as imported: {surfaces:#?}"
        );
    }
}
