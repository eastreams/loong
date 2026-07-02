use crate::first_run_action_presentation::{
    build_first_run_action_sections, first_run_group_for_onboarding_action_kind,
};
use crate::onboard::finalize::{CLI_CHANNEL_ID, OnboardingDomainOutcome, OnboardingSuccessSummary};
use loong_app::tui_surface::{
    TuiActionSpec, TuiHeaderStyle, TuiKeyValueSpec, TuiScreenSpec, TuiSectionSpec,
    render_onboard_screen_spec,
};

pub(crate) fn render_onboarding_success_summary_with_style(
    summary: &OnboardingSuccessSummary,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_success_screen_spec(summary);
    render_onboard_screen_spec(&spec, width, color_enabled)
}

#[cfg(test)]
pub(crate) fn build_onboarding_success_screen_spec(
    summary: &OnboardingSuccessSummary,
) -> TuiScreenSpec {
    build_success_screen_spec(summary)
}

fn build_success_screen_spec(summary: &OnboardingSuccessSummary) -> TuiScreenSpec {
    let mut sections = build_first_run_action_sections(
        &summary.next_actions,
        |action| first_run_group_for_onboarding_action_kind(action.kind),
        |action| TuiActionSpec {
            label: action.label.clone(),
            command: action.command.clone(),
        },
    );

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

    items.push(TuiKeyValueSpec::Plain {
        key: "web search".to_owned(),
        value: summary.web_search_provider.clone(),
    });

    if let Some(web_search_credential) = summary.web_search_credential.as_ref() {
        items.push(TuiKeyValueSpec::Plain {
            key: web_search_credential.label.to_owned(),
            value: web_search_credential.value.clone(),
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
