use std::path::Path;

use loong_app as mvp;

use crate::onboard::types::OnboardingCredentialSummary;
use crate::setup_boundary::SetupBoundaryKind;
pub(crate) const CLI_CHANNEL_ID: &str = "cli";

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
    Doctor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingAction {
    pub kind: OnboardingActionKind,
    pub label: String,
    pub command: String,
}

pub(crate) const fn setup_boundary_kind_for_onboarding_action_kind(
    kind: OnboardingActionKind,
) -> SetupBoundaryKind {
    match kind {
        OnboardingActionKind::Ask => SetupBoundaryKind::Ask,
        OnboardingActionKind::Chat => SetupBoundaryKind::Chat,
        OnboardingActionKind::Personalize => SetupBoundaryKind::Personalize,
        OnboardingActionKind::Channel => SetupBoundaryKind::ChannelReview,
        OnboardingActionKind::Doctor => SetupBoundaryKind::Doctor,
    }
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
    let web_search_status = crate::query_search_surface::query_search_provider_status(config);
    let web_search_provider = web_search_status.provider_label.clone();
    let web_search_credential = if web_search_status.provider_native {
        Some(OnboardingCredentialSummary {
            label: crate::access_terms::QUERY_SEARCH_CREDENTIAL_LABEL,
            value: "provided by active provider".to_owned(),
        })
    } else {
        crate::query_search_surface::summarize_query_search_credential(
            config,
            config.tools.web_search.default_provider.as_str(),
        )
    };
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
        provider: crate::provider::presentation::active_provider_label(config),
        saved_provider_profiles: crate::provider::presentation::saved_provider_profile_ids(config),
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
    crate::onboard::success_render::render_onboarding_success_summary_with_style(
        summary,
        width,
        color_enabled,
    )
}

pub fn render_onboarding_success_summary_with_width(
    summary: &OnboardingSuccessSummary,
    width: usize,
) -> Vec<String> {
    crate::onboard::success_render::render_onboarding_success_summary_with_style(
        summary, width, false,
    )
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
    let _ = config;
    Vec::new()
}

fn collect_onboarding_channel_surface_summary(
    config: &mvp::config::LoongConfig,
) -> OnboardingChannelSurfaceSummary {
    let inventory = mvp::channel::channel_inventory(config);

    let total_surface_count = inventory.channel_surfaces.len();
    let runtime_backed_surface_count = config.enabled_runtime_backed_channel_ids().len();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onboard::success_render::build_onboarding_success_screen_spec;
    use crate::personalize_presentation::personalize_action_label;
    use loong_app::tui_surface::TuiSectionSpec;

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
                    command: "LOONG_CONFIG_PATH='/tmp/loong.toml' loong".to_owned(),
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
                        && items.iter().all(|item| item.label != "channels")
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
                        && items.iter().any(|item| item.label == "channels")
            )),
            "expected setup-surface actions to be grouped under continue setup: {spec:#?}"
        );
    }
}
