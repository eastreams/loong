use loongclaw_app as mvp;

use super::{ImportCandidate, ImportSourceKind, PreviewStatus, SetupDomainKind};

#[derive(Debug, Clone, Default)]
pub(crate) struct ProviderSelectionPlan {
    pub(crate) imported_choices: Vec<ImportedProviderChoice>,
    pub(crate) default_kind: Option<mvp::config::ProviderKind>,
    pub(crate) requires_explicit_choice: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportedProviderChoice {
    pub(crate) kind: mvp::config::ProviderKind,
    pub(crate) source: String,
    pub(crate) summary: String,
    pub(crate) config: mvp::config::ProviderConfig,
}

pub(crate) fn build_provider_selection_plan_for_candidate(
    selected_candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
) -> ProviderSelectionPlan {
    let provider_sources = if selected_candidate.source_kind == ImportSourceKind::RecommendedPlan {
        let filtered = all_candidates
            .iter()
            .filter(|candidate| candidate.source_kind != ImportSourceKind::RecommendedPlan)
            .collect::<Vec<_>>();
        if filtered.is_empty() {
            vec![selected_candidate]
        } else {
            filtered
        }
    } else {
        vec![selected_candidate]
    };

    let mut imported_choices: Vec<ImportedProviderChoice> = Vec::new();
    for candidate in provider_sources {
        let Some(provider_domain) = candidate
            .domains
            .iter()
            .find(|domain| domain.kind == SetupDomainKind::Provider)
        else {
            continue;
        };

        let incoming = ImportedProviderChoice {
            kind: candidate.config.provider.kind,
            source: candidate.source.clone(),
            summary: provider_domain.summary.clone(),
            config: candidate.config.provider.clone(),
        };
        if let Some(existing) = imported_choices
            .iter_mut()
            .find(|choice| choice.kind == incoming.kind)
        {
            if provider_choice_status_rank(provider_domain.status)
                > provider_choice_status_rank(provider_status_for_choice(existing))
            {
                *existing = incoming;
            }
            continue;
        }
        imported_choices.push(incoming);
    }

    let mut default_kind = selected_candidate
        .domains
        .iter()
        .find(|domain| domain.kind == SetupDomainKind::Provider)
        .map(|_| selected_candidate.config.provider.kind);
    if default_kind.is_none() && imported_choices.len() == 1 {
        default_kind = imported_choices.first().map(|choice| choice.kind);
    }
    if let Some(kind) = default_kind {
        imported_choices.sort_by_key(|choice| choice.kind != kind);
    }

    ProviderSelectionPlan {
        requires_explicit_choice: default_kind.is_none() && imported_choices.len() > 1,
        imported_choices,
        default_kind,
    }
}

pub(crate) fn resolve_provider_config_from_selection(
    current_provider: &mvp::config::ProviderConfig,
    plan: &ProviderSelectionPlan,
    selected_kind: mvp::config::ProviderKind,
) -> mvp::config::ProviderConfig {
    if let Some(choice) = plan
        .imported_choices
        .iter()
        .find(|choice| choice.kind == selected_kind)
    {
        return choice.config.clone();
    }
    if current_provider.kind == selected_kind {
        return current_provider.clone();
    }
    fresh_provider_config_for_kind(selected_kind)
}

fn fresh_provider_config_for_kind(kind: mvp::config::ProviderKind) -> mvp::config::ProviderConfig {
    mvp::config::ProviderConfig::fresh_for_kind(kind)
}

fn provider_choice_status_rank(status: PreviewStatus) -> u8 {
    match status {
        PreviewStatus::Ready => 2,
        PreviewStatus::NeedsReview => 1,
        PreviewStatus::Unavailable => 0,
    }
}

fn provider_status_for_choice(choice: &ImportedProviderChoice) -> PreviewStatus {
    if choice.config.authorization_header().is_some() {
        PreviewStatus::Ready
    } else {
        PreviewStatus::NeedsReview
    }
}
