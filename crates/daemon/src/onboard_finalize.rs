use std::fs;
use std::path::{Path, PathBuf};

use loong_app as mvp;
use loong_spec::CliResult;
use time::OffsetDateTime;
use time::format_description::FormatItem;
use time::macros::format_description;

use crate::onboard_types::OnboardingCredentialSummary;
use mvp::tui_surface::{
    TuiActionSpec, TuiHeaderStyle, TuiKeyValueSpec, TuiScreenSpec, TuiSectionSpec,
    render_onboard_screen_spec,
};

const BACKUP_TIMESTAMP_FORMAT: &[FormatItem<'static>] =
    format_description!("[year][month][day]-[hour][minute][second]");
const CLI_CHANNEL_ID: &str = "cli";
const MAX_SUGGESTED_RUNTIME_CHANNELS: usize = 3;

#[derive(Debug, Clone)]
pub(crate) struct ConfigWritePlan {
    pub(crate) force: bool,
    pub(crate) backup_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub(crate) struct OnboardWriteRecovery {
    pub(crate) output_preexisted: bool,
    pub(crate) backup_path: Option<PathBuf>,
    pub(crate) keep_backup_on_success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingSuccessSummary {
    pub import_source: Option<String>,
    pub config_path: String,
    pub config_status: Option<String>,
    pub provider: String,
    pub saved_provider_profiles: Vec<String>,
    pub model: String,
    pub transport: String,
    pub provider_endpoint: Option<String>,
    pub credential: Option<OnboardingCredentialSummary>,
    pub prompt_mode: String,
    pub personality: Option<String>,
    pub prompt_addendum: Option<String>,
    pub memory_profile: String,
    pub web_search_provider: String,
    pub web_search_credential: Option<OnboardingCredentialSummary>,
    pub memory_path: Option<String>,
    pub channel_surface_summary: OnboardingChannelSurfaceSummary,
    pub channels: Vec<String>,
    pub runtime_backed_channels: Vec<String>,
    pub plugin_backed_channels: Vec<String>,
    pub outbound_only_channels: Vec<String>,
    pub suggested_channels: Vec<String>,
    pub domain_outcomes: Vec<OnboardingDomainOutcome>,
    pub next_actions: Vec<OnboardingAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingChannelSurfaceSummary {
    pub total_surface_count: usize,
    pub runtime_backed_surface_count: usize,
    pub config_backed_surface_count: usize,
    pub plugin_backed_surface_count: usize,
    pub catalog_only_surface_count: usize,
}

impl OnboardingChannelSurfaceSummary {
    pub fn render_compact(&self) -> String {
        format!(
            "{} total ({} runtime-backed, {} config-backed, {} plugin-backed, {} catalog-only)",
            self.total_surface_count,
            self.runtime_backed_surface_count,
            self.config_backed_surface_count,
            self.plugin_backed_surface_count,
            self.catalog_only_surface_count,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingDomainOutcome {
    pub kind: crate::migration::SetupDomainKind,
    pub decision: crate::migration::types::PreviewDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingActionKind {
    Ask,
    Chat,
    Personalize,
    Channel,
    BrowserPreview,
    Doctor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingAction {
    pub kind: OnboardingActionKind,
    pub label: String,
    pub command: String,
}

pub fn build_onboarding_success_summary(
    path: &Path,
    config: &mvp::config::LoongConfig,
    import_source: Option<&str>,
) -> OnboardingSuccessSummary {
    build_onboarding_success_summary_with_memory(path, config, import_source, None, None, None)
}

pub(crate) fn build_onboarding_success_summary_with_memory(
    path: &Path,
    config: &mvp::config::LoongConfig,
    import_source: Option<&str>,
    review_candidate: Option<&crate::migration::ImportCandidate>,
    memory_path: Option<&str>,
    config_status: Option<&str>,
) -> OnboardingSuccessSummary {
    let config_path = path.display().to_string();
    let next_actions = crate::next_actions::collect_setup_next_actions(config, &config_path)
        .into_iter()
        .map(|action| {
            let kind = match action.kind {
                crate::next_actions::SetupNextActionKind::Ask => OnboardingActionKind::Ask,
                crate::next_actions::SetupNextActionKind::Chat => OnboardingActionKind::Chat,
                crate::next_actions::SetupNextActionKind::Personalize => {
                    OnboardingActionKind::Personalize
                }
                crate::next_actions::SetupNextActionKind::Channel => OnboardingActionKind::Channel,
                crate::next_actions::SetupNextActionKind::BrowserPreview => {
                    OnboardingActionKind::BrowserPreview
                }
                crate::next_actions::SetupNextActionKind::Doctor => OnboardingActionKind::Doctor,
            };

            OnboardingAction {
                kind,
                label: action.label,
                command: action.command,
            }
        })
        .collect();
    let personality = if config.cli.uses_native_prompt_pack() {
        let personality_id =
            crate::onboard_cli::prompt_personality_id(config.cli.resolved_personality());
        Some(personality_id.to_owned())
    } else {
        None
    };
    let prompt_mode = crate::onboard_cli::summarize_prompt_mode(config);
    let prompt_addendum = crate::onboard_cli::summarize_prompt_addendum(config);
    let credential = crate::onboard_cli::summarize_provider_credential(&config.provider);
    let web_search_provider = crate::onboard_web_search::web_search_provider_display_name(
        config.tools.web_search.default_provider.as_str(),
    );
    let web_search_credential = crate::onboard_web_search::summarize_web_search_provider_credential(
        config,
        config.tools.web_search.default_provider.as_str(),
    );
    let domain_outcomes = collect_onboarding_domain_outcomes(review_candidate);
    let channel_surface_summary = collect_onboarding_channel_surface_summary(config);
    let channels = config.enabled_channel_ids();
    let runtime_backed_channels = config.enabled_runtime_backed_channel_ids();
    let plugin_backed_channels = config.enabled_plugin_backed_channel_ids();
    let outbound_only_channels = config.enabled_outbound_only_channel_ids();
    let suggested_channels = collect_onboarding_suggested_channels(config);

    OnboardingSuccessSummary {
        import_source: import_source.map(str::to_owned),
        config_path,
        config_status: config_status.map(str::to_owned),
        provider: crate::provider_presentation::active_provider_label(config),
        saved_provider_profiles: crate::provider_presentation::saved_provider_profile_ids(config),
        model: config.provider.model.clone(),
        transport: config.provider.transport_readiness().summary,
        provider_endpoint: config.provider.region_endpoint_note(),
        credential,
        prompt_mode,
        personality,
        prompt_addendum,
        memory_profile: config.memory.profile.as_str().to_owned(),
        web_search_provider,
        web_search_credential,
        memory_path: memory_path.map(str::to_owned),
        channel_surface_summary,
        channels,
        runtime_backed_channels,
        plugin_backed_channels,
        outbound_only_channels,
        suggested_channels,
        domain_outcomes,
        next_actions,
    }
}

pub(crate) fn render_onboarding_success_summary_lines(
    summary: &OnboardingSuccessSummary,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    render_onboarding_success_summary_with_style(summary, width, color_enabled)
}

pub fn render_onboarding_success_summary_with_width(
    summary: &OnboardingSuccessSummary,
    width: usize,
) -> Vec<String> {
    render_onboarding_success_summary_with_style(summary, width, false)
}

pub(crate) fn prepare_output_path_for_write(
    output_path: &Path,
    plan: &ConfigWritePlan,
) -> CliResult<OnboardWriteRecovery> {
    let output_preexisted = output_path.exists();
    let keep_backup_on_success = plan.backup_path.is_some();
    let backup_path = if output_preexisted {
        let resolved_backup_path = plan
            .backup_path
            .clone()
            .unwrap_or(resolve_rollback_backup_path(output_path)?);
        Some(resolved_backup_path)
    } else {
        None
    };

    if let Some(backup_path) = backup_path.as_deref() {
        backup_existing_config(output_path, backup_path)?;
    }

    Ok(OnboardWriteRecovery {
        output_preexisted,
        backup_path,
        keep_backup_on_success,
    })
}

pub fn backup_existing_config(output_path: &Path, backup_path: &Path) -> CliResult<()> {
    fs::copy(output_path, backup_path)
        .map_err(|error| format!("failed to backup config: {error}"))?;
    Ok(())
}

impl OnboardWriteRecovery {
    pub(crate) fn rollback(&self, output_path: &Path) -> CliResult<()> {
        if self.output_preexisted {
            let backup_path = self
                .backup_path
                .as_deref()
                .ok_or_else(|| "missing rollback backup for existing config".to_owned())?;

            fs::copy(backup_path, output_path).map_err(|error| {
                format!(
                    "failed to restore original config {} from backup {}: {error}",
                    output_path.display(),
                    backup_path.display(),
                )
            })?;
            self.finish_success();
            return Ok(());
        }

        if output_path.exists() {
            fs::remove_file(output_path).map_err(|error| {
                format!(
                    "failed to remove partial config {} after onboarding failure: {error}",
                    output_path.display()
                )
            })?;
        }

        self.finish_success();
        Ok(())
    }

    pub(crate) fn finish_success(&self) {
        if self.keep_backup_on_success {
            return;
        }

        if let Some(backup_path) = self.backup_path.as_deref() {
            let _ = fs::remove_file(backup_path);
        }
    }
}

pub(crate) fn rollback_onboard_write_failure(
    output_path: &Path,
    write_recovery: &OnboardWriteRecovery,
    failure: impl Into<String>,
) -> String {
    let failure = failure.into();
    let rollback_result = write_recovery.rollback(output_path);

    match rollback_result {
        Ok(()) => failure,
        Err(rollback_error) => {
            format!("{failure}; additionally failed to restore original config: {rollback_error}")
        }
    }
}

pub(crate) fn resolve_backup_path(original: &Path) -> CliResult<PathBuf> {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    resolve_backup_path_at(original, now)
}

pub(crate) fn resolve_backup_path_at(
    original: &Path,
    timestamp: OffsetDateTime,
) -> CliResult<PathBuf> {
    let parent = original.parent().unwrap_or(Path::new("."));
    let file_stem = original
        .file_stem()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "config".to_owned());
    let formatted_timestamp = format_backup_timestamp_at(timestamp)?;

    Ok(parent.join(format!("{}.toml.bak-{}", file_stem, formatted_timestamp)))
}

pub(crate) fn resolve_rollback_backup_path(original: &Path) -> CliResult<PathBuf> {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    resolve_rollback_backup_path_at(original, now)
}

pub(crate) fn resolve_rollback_backup_path_at(
    original: &Path,
    timestamp: OffsetDateTime,
) -> CliResult<PathBuf> {
    let parent = original.parent().unwrap_or(Path::new("."));
    let file_name = original
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "config.toml".to_owned());
    let formatted_timestamp = format_backup_timestamp_at(timestamp)?;

    Ok(parent.join(format!(
        ".{file_name}.onboard-rollback-{formatted_timestamp}"
    )))
}

fn collect_onboarding_domain_outcomes(
    review_candidate: Option<&crate::migration::ImportCandidate>,
) -> Vec<OnboardingDomainOutcome> {
    review_candidate
        .into_iter()
        .flat_map(|candidate| candidate.domains.iter())
        .filter_map(|domain| {
            domain.decision.map(|decision| OnboardingDomainOutcome {
                kind: domain.kind,
                decision,
            })
        })
        .collect()
}

fn collect_onboarding_suggested_channels(config: &mvp::config::LoongConfig) -> Vec<String> {
    let has_enabled_non_cli_channels = config
        .enabled_channel_ids()
        .into_iter()
        .any(|channel_id| channel_id != CLI_CHANNEL_ID);
    if has_enabled_non_cli_channels {
        return Vec::new();
    }

    let enabled_service_channel_ids = config.enabled_service_channel_ids();
    if !enabled_service_channel_ids.is_empty() {
        return Vec::new();
    }

    let inventory = mvp::channel::channel_inventory(config);
    inventory
        .channel_surfaces
        .into_iter()
        .filter_map(|surface| {
            let serve_operation = surface
                .catalog
                .operation(mvp::channel::CHANNEL_OPERATION_SERVE_ID)?;
            let implementation_status = surface.catalog.implementation_status;
            let availability = serve_operation.availability;
            if implementation_status
                != mvp::channel::ChannelCatalogImplementationStatus::RuntimeBacked
            {
                return None;
            }
            if availability != mvp::channel::ChannelCatalogOperationAvailability::Implemented {
                return None;
            }

            let label = surface.catalog.label;
            let selection_label = surface.catalog.selection_label;
            let suggested_channel = format!("{label} ({selection_label})");
            Some(suggested_channel)
        })
        .take(MAX_SUGGESTED_RUNTIME_CHANNELS)
        .collect()
}

fn collect_onboarding_channel_surface_summary(
    config: &mvp::config::LoongConfig,
) -> OnboardingChannelSurfaceSummary {
    let inventory = mvp::channel::channel_inventory(config);

    let total_surface_count = inventory.channel_surfaces.len();
    let runtime_backed_surface_count = inventory
        .channel_surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::RuntimeBacked
        })
        .count();
    let config_backed_surface_count = inventory
        .channel_surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::ConfigBacked
        })
        .count();
    let plugin_backed_surface_count = inventory
        .channel_surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::PluginBacked
        })
        .count();
    let catalog_only_surface_count = inventory
        .channel_surfaces
        .iter()
        .filter(|surface| {
            surface.catalog.implementation_status
                == mvp::channel::ChannelCatalogImplementationStatus::Stub
        })
        .count();

    OnboardingChannelSurfaceSummary {
        total_surface_count,
        runtime_backed_surface_count,
        config_backed_surface_count,
        plugin_backed_surface_count,
        catalog_only_surface_count,
    }
}

fn build_onboarding_domain_outcome_items(
    outcomes: &[OnboardingDomainOutcome],
) -> Vec<TuiKeyValueSpec> {
    let mut grouped: Vec<(crate::migration::types::PreviewDecision, Vec<&'static str>)> =
        Vec::new();
    let mut sorted = outcomes.to_vec();

    sorted.sort_by_key(|outcome| (outcome.decision.outcome_rank(), outcome.kind));

    for outcome in sorted {
        let maybe_group = grouped
            .iter_mut()
            .find(|(decision, _)| *decision == outcome.decision);

        if let Some((_, labels)) = maybe_group {
            labels.push(outcome.kind.label());
            continue;
        }

        grouped.push((outcome.decision, vec![outcome.kind.label()]));
    }

    grouped
        .into_iter()
        .map(|(decision, labels)| TuiKeyValueSpec::Csv {
            key: decision.outcome_label().to_owned(),
            values: labels.into_iter().map(str::to_owned).collect(),
        })
        .collect()
}

fn render_onboarding_success_summary_with_style(
    summary: &OnboardingSuccessSummary,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboarding_success_screen_spec(summary);
    render_onboard_screen_spec(&spec, width, color_enabled)
}

fn build_onboarding_success_screen_spec(summary: &OnboardingSuccessSummary) -> TuiScreenSpec {
    let mut sections = Vec::new();

    if let Some(primary) = summary.next_actions.first() {
        sections.push(TuiSectionSpec::ActionGroup {
            title: Some("start here".to_owned()),
            inline_title_when_wide: false,
            items: vec![TuiActionSpec {
                label: primary.label.clone(),
                command: primary.command.clone(),
            }],
        });
    }

    let (setup_actions, general_actions): (Vec<_>, Vec<_>) = summary
        .next_actions
        .iter()
        .skip(1)
        .partition(|action| onboarding_action_is_continue_setup(action.kind));

    if !general_actions.is_empty() {
        sections.push(TuiSectionSpec::ActionGroup {
            title: Some("also available".to_owned()),
            inline_title_when_wide: false,
            items: general_actions
                .into_iter()
                .map(|action| TuiActionSpec {
                    label: action.label.clone(),
                    command: action.command.clone(),
                })
                .collect(),
        });
    }

    if !setup_actions.is_empty() {
        sections.push(TuiSectionSpec::ActionGroup {
            title: Some("continue setup".to_owned()),
            inline_title_when_wide: false,
            items: setup_actions
                .into_iter()
                .map(|action| TuiActionSpec {
                    label: action.label.clone(),
                    command: action.command.clone(),
                })
                .collect(),
        });
    }

    sections.push(TuiSectionSpec::KeyValues {
        title: Some("saved setup".to_owned()),
        items: build_onboarding_saved_setup_items(summary),
    });

    if !summary.domain_outcomes.is_empty() {
        sections.push(TuiSectionSpec::KeyValues {
            title: Some("setup outcome".to_owned()),
            items: build_onboarding_domain_outcome_items(&summary.domain_outcomes),
        });
    }

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some("setup complete".to_owned()),
        title: Some("onboarding complete".to_owned()),
        progress_line: None,
        intro_lines: Vec::new(),
        sections,
        choices: Vec::new(),
        footer_lines: Vec::new(),
    }
}

fn build_onboarding_saved_setup_items(summary: &OnboardingSuccessSummary) -> Vec<TuiKeyValueSpec> {
    let mut items = vec![TuiKeyValueSpec::Plain {
        key: "config".to_owned(),
        value: summary.config_path.clone(),
    }];

    if let Some(config_status) = summary.config_status.as_deref() {
        items.push(TuiKeyValueSpec::Plain {
            key: "config status".to_owned(),
            value: config_status.to_owned(),
        });
    }

    if let Some(source) = summary.import_source.as_deref() {
        items.push(TuiKeyValueSpec::Plain {
            key: "starting point".to_owned(),
            value: crate::migration::ImportSourceKind::onboarding_label(None, source),
        });
    }

    if summary.saved_provider_profiles.len() > 1 {
        items.push(TuiKeyValueSpec::Plain {
            key: "active provider".to_owned(),
            value: summary.provider.clone(),
        });
        items.push(TuiKeyValueSpec::Csv {
            key: "saved provider profiles".to_owned(),
            values: summary.saved_provider_profiles.clone(),
        });
    } else {
        items.push(TuiKeyValueSpec::Plain {
            key: "provider".to_owned(),
            value: summary.provider.clone(),
        });
    }

    items.push(TuiKeyValueSpec::Plain {
        key: "model".to_owned(),
        value: summary.model.clone(),
    });
    items.push(TuiKeyValueSpec::Plain {
        key: "transport".to_owned(),
        value: summary.transport.clone(),
    });

    if let Some(provider_endpoint) = summary.provider_endpoint.as_deref() {
        items.push(TuiKeyValueSpec::Plain {
            key: "provider endpoint".to_owned(),
            value: provider_endpoint.to_owned(),
        });
    }

    if let Some(credential) = summary.credential.as_ref() {
        items.push(TuiKeyValueSpec::Plain {
            key: credential.label.to_owned(),
            value: credential.value.clone(),
        });
    }

    items.push(TuiKeyValueSpec::Plain {
        key: "prompt mode".to_owned(),
        value: summary.prompt_mode.clone(),
    });

    if let Some(personality) = summary.personality.as_deref() {
        items.push(TuiKeyValueSpec::Plain {
            key: "personality".to_owned(),
            value: personality.to_owned(),
        });
    }

    if let Some(prompt_addendum) = summary.prompt_addendum.as_deref() {
        items.push(TuiKeyValueSpec::Plain {
            key: "prompt addendum".to_owned(),
            value: prompt_addendum.to_owned(),
        });
    }

    items.push(TuiKeyValueSpec::Plain {
        key: "memory profile".to_owned(),
        value: summary.memory_profile.clone(),
    });

    let web_search_provider = summary.web_search_provider.clone();
    items.push(TuiKeyValueSpec::Plain {
        key: "web search".to_owned(),
        value: web_search_provider,
    });

    if let Some(web_search_credential) = summary.web_search_credential.as_ref() {
        let credential_label = web_search_credential.label.to_owned();
        let credential_value = web_search_credential.value.clone();
        items.push(TuiKeyValueSpec::Plain {
            key: credential_label,
            value: credential_value,
        });
    }

    if let Some(memory_path) = summary.memory_path.as_deref() {
        items.push(TuiKeyValueSpec::Plain {
            key: "sqlite memory".to_owned(),
            value: memory_path.to_owned(),
        });
    }

    items.push(TuiKeyValueSpec::Plain {
        key: "channel surfaces".to_owned(),
        value: summary.channel_surface_summary.render_compact(),
    });

    push_onboarding_enabled_channel_group_items(&mut items, summary);

    if !summary.suggested_channels.is_empty() {
        items.push(TuiKeyValueSpec::Csv {
            key: "suggested channels".to_owned(),
            values: summary.suggested_channels.clone(),
        });
    }

    items
}

fn push_onboarding_enabled_channel_group_items(
    items: &mut Vec<TuiKeyValueSpec>,
    summary: &OnboardingSuccessSummary,
) {
    if !summary.runtime_backed_channels.is_empty() {
        items.push(TuiKeyValueSpec::Csv {
            key: "runtime-backed channels".to_owned(),
            values: summary.runtime_backed_channels.clone(),
        });
    }

    if !summary.plugin_backed_channels.is_empty() {
        items.push(TuiKeyValueSpec::Csv {
            key: "plugin-backed channels".to_owned(),
            values: summary.plugin_backed_channels.clone(),
        });
    }

    if !summary.outbound_only_channels.is_empty() {
        items.push(TuiKeyValueSpec::Csv {
            key: "outbound-only channels".to_owned(),
            values: summary.outbound_only_channels.clone(),
        });
    }

    let remaining_channels = summary
        .channels
        .iter()
        .filter(|channel| channel.as_str() != CLI_CHANNEL_ID)
        .filter(|channel| {
            !summary.runtime_backed_channels.contains(channel)
                && !summary.plugin_backed_channels.contains(channel)
                && !summary.outbound_only_channels.contains(channel)
        })
        .cloned()
        .collect::<Vec<_>>();
    if !remaining_channels.is_empty() {
        items.push(TuiKeyValueSpec::Csv {
            key: "channels".to_owned(),
            values: remaining_channels,
        });
    }
}

fn onboarding_action_is_continue_setup(kind: OnboardingActionKind) -> bool {
    matches!(
        kind,
        OnboardingActionKind::Channel | OnboardingActionKind::BrowserPreview
    )
}

pub(crate) fn format_backup_timestamp_at(timestamp: OffsetDateTime) -> CliResult<String> {
    timestamp
        .format(BACKUP_TIMESTAMP_FORMAT)
        .map_err(|error| format!("format backup timestamp failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::personalize_presentation::personalize_action_label;

    fn sample_success_summary() -> OnboardingSuccessSummary {
        OnboardingSuccessSummary {
            import_source: None,
            config_path: "/tmp/loong.toml".to_owned(),
            config_status: None,
            provider: "OpenAI".to_owned(),
            saved_provider_profiles: vec!["openai".to_owned()],
            model: "gpt-5.4".to_owned(),
            transport: "ready".to_owned(),
            provider_endpoint: None,
            credential: None,
            prompt_mode: "native prompt pack".to_owned(),
            personality: None,
            prompt_addendum: None,
            memory_profile: "profile_plus_window".to_owned(),
            web_search_provider: "none".to_owned(),
            web_search_credential: None,
            memory_path: None,
            channel_surface_summary: OnboardingChannelSurfaceSummary {
                total_surface_count: 4,
                runtime_backed_surface_count: 2,
                config_backed_surface_count: 1,
                plugin_backed_surface_count: 1,
                catalog_only_surface_count: 0,
            },
            channels: vec!["cli".to_owned()],
            runtime_backed_channels: Vec::new(),
            plugin_backed_channels: Vec::new(),
            outbound_only_channels: Vec::new(),
            suggested_channels: vec!["Telegram (telegram)".to_owned()],
            domain_outcomes: Vec::new(),
            next_actions: vec![
                OnboardingAction {
                    kind: OnboardingActionKind::Ask,
                    label: "first answer".to_owned(),
                    command: "loong ask --config '/tmp/loong.toml'".to_owned(),
                },
                OnboardingAction {
                    kind: OnboardingActionKind::Chat,
                    label: "chat".to_owned(),
                    command: "loong chat --config '/tmp/loong.toml'".to_owned(),
                },
                OnboardingAction {
                    kind: OnboardingActionKind::Personalize,
                    label: personalize_action_label().to_owned(),
                    command: "loong personalize --config '/tmp/loong.toml'".to_owned(),
                },
                OnboardingAction {
                    kind: OnboardingActionKind::Channel,
                    label: "channels".to_owned(),
                    command: "loong channels --config '/tmp/loong.toml'".to_owned(),
                },
                OnboardingAction {
                    kind: OnboardingActionKind::BrowserPreview,
                    label: "browser preview".to_owned(),
                    command: "loong browser preview --config '/tmp/loong.toml'".to_owned(),
                },
            ],
        }
    }

    #[test]
    fn build_onboarding_success_screen_spec_separates_continue_setup_actions() {
        let summary = sample_success_summary();

        let spec = build_onboarding_success_screen_spec(&summary);

        assert!(
            spec.sections.iter().any(|section| matches!(
                section,
                TuiSectionSpec::ActionGroup { title: Some(title), items, .. }
                    if title == "start here"
                        && items.len() == 1
                        && items[0].label == "first answer"
            )),
            "expected the primary action to stay in start here: {spec:#?}"
        );
        assert!(
            spec.sections.iter().any(|section| matches!(
                section,
                TuiSectionSpec::ActionGroup { title: Some(title), items, .. }
                    if title == "also available"
                        && items.iter().all(|item| item.label != "channels" && item.label != "browser preview")
                        && items.iter().any(|item| item.label == "chat")
                        && items.iter().any(|item| item.label == personalize_action_label())
            )),
            "expected general follow-up actions to stay separate from setup surfaces: {spec:#?}"
        );
        assert!(
            spec.sections.iter().any(|section| matches!(
                section,
                TuiSectionSpec::ActionGroup { title: Some(title), items, .. }
                    if title == "continue setup"
                        && items.iter().any(|item| item.label == "channels")
                        && items.iter().any(|item| item.label == "browser preview")
            )),
            "expected setup-surface actions to be grouped under continue setup: {spec:#?}"
        );
    }
}
