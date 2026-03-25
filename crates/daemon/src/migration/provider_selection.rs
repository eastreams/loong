use std::collections::{BTreeMap, BTreeSet};

use loongclaw_app as mvp;

use super::{ImportCandidate, ImportSourceKind, PreviewStatus, SetupDomainKind};

pub const PROVIDER_SELECTOR_PLACEHOLDER: &str = mvp::config::PROVIDER_SELECTOR_PLACEHOLDER;
pub const PROVIDER_SELECTOR_NOTE: &str = mvp::config::PROVIDER_SELECTOR_NOTE;
const PROVIDER_SELECTION_MERGE_NOTE: &str = "other detected settings stay merged";
const COMPACT_SELECTOR_DETAIL_WIDTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportedChoiceSelectorResolution {
    Match(String),
    Ambiguous(Vec<String>),
    NoMatch,
}

#[derive(Debug, Clone, Default)]
pub struct ProviderSelectionPlan {
    pub imported_choices: Vec<ImportedProviderChoice>,
    pub default_kind: Option<mvp::config::ProviderKind>,
    pub default_profile_id: Option<String>,
    pub requires_explicit_choice: bool,
}

#[derive(Debug, Clone)]
pub struct ImportedProviderChoice {
    pub profile_id: String,
    pub kind: mvp::config::ProviderKind,
    pub source: String,
    pub summary: String,
    pub config: mvp::config::ProviderConfig,
}

pub fn build_provider_selection_plan_for_candidate(
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

    let selected_identity_key = selected_candidate
        .domains
        .iter()
        .any(|domain| domain.kind == SetupDomainKind::Provider)
        .then(|| provider_profile_merge_key(&selected_candidate.config.provider));

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
            profile_id: String::new(),
            kind: candidate.config.provider.kind,
            source: candidate.source.clone(),
            summary: provider_domain.summary.clone(),
            config: candidate.config.provider.clone(),
        };
        if let Some(existing) = imported_choices.iter_mut().find(|choice| {
            provider_profile_merge_key(&choice.config)
                == provider_profile_merge_key(&incoming.config)
        }) {
            if provider_choice_status_rank(provider_domain.status)
                > provider_choice_status_rank(provider_status_for_choice(existing))
            {
                let merged_config = merge_provider_config(&incoming.config, &existing.config);
                *existing = ImportedProviderChoice {
                    config: merged_config,
                    ..incoming
                };
            } else {
                existing.config = merge_provider_config(&existing.config, &incoming.config);
            }
            continue;
        }
        imported_choices.push(incoming);
    }

    let mut default_identity_key = selected_identity_key.filter(|identity_key| {
        imported_choices
            .iter()
            .any(|choice| provider_profile_merge_key(&choice.config) == *identity_key)
    });
    if default_identity_key.is_none() && imported_choices.len() == 1 {
        default_identity_key = imported_choices
            .first()
            .map(|choice| provider_profile_merge_key(&choice.config));
    }

    if let Some(default_identity_key) = default_identity_key.as_deref() {
        imported_choices.sort_by_key(|choice| {
            provider_profile_merge_key(&choice.config) != default_identity_key
        });
    }
    assign_profile_ids(&mut imported_choices);

    let default_profile_id = default_identity_key
        .as_deref()
        .and_then(|default_identity_key| {
            imported_choices
                .iter()
                .find(|choice| provider_profile_merge_key(&choice.config) == default_identity_key)
                .map(|choice| choice.profile_id.clone())
        });
    let default_kind = default_profile_id.as_deref().and_then(|profile_id| {
        imported_choices
            .iter()
            .find(|choice| choice.profile_id == profile_id)
            .map(|choice| choice.kind)
    });

    ProviderSelectionPlan {
        requires_explicit_choice: default_profile_id.is_none() && imported_choices.len() > 1,
        imported_choices,
        default_kind,
        default_profile_id,
    }
}

pub fn resolve_provider_config_from_selection(
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

pub fn provider_profile_merge_key(provider: &mvp::config::ProviderConfig) -> String {
    let endpoint = provider
        .endpoint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| {
            format!(
                "{}{}",
                provider.base_url.trim().to_ascii_lowercase(),
                provider.chat_completions_path.trim().to_ascii_lowercase()
            )
        });
    let model = normalize_provider_profile_id_segment(provider.model.as_str())
        .unwrap_or_else(|| "auto".to_owned());
    format!("{}|{}|{}", provider.kind.as_str(), endpoint, model)
}

pub fn merge_provider_config(
    existing: &mvp::config::ProviderConfig,
    incoming: &mvp::config::ProviderConfig,
) -> mvp::config::ProviderConfig {
    let mut merged = existing.clone();
    merged.canonicalize_configured_auth_env_bindings();
    let mut incoming = incoming.clone();
    incoming.canonicalize_configured_auth_env_bindings();
    if merged.model.trim().is_empty() || merged.model.eq_ignore_ascii_case("auto") {
        merged.model = incoming.model.clone();
    }
    super::provider_transport::supplement_provider_transport(&mut merged, &incoming);
    if merged.api_key.is_none() {
        merged.api_key = incoming.api_key.clone();
    }
    if merged.oauth_access_token.is_none() {
        merged.oauth_access_token = incoming.oauth_access_token.clone();
    }
    if merged.endpoint.is_none() {
        merged.endpoint = incoming.endpoint.clone();
    }
    if merged.models_endpoint.is_none() {
        merged.models_endpoint = incoming.models_endpoint.clone();
    }
    if merged.headers.is_empty() {
        merged.headers = incoming.headers.clone();
    } else {
        for (key, value) in &incoming.headers {
            merged
                .headers
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }
    if merged.preferred_models.is_empty() && !incoming.preferred_models.is_empty() {
        merged.preferred_models = incoming.preferred_models.clone();
    }
    if merged.reasoning_effort.is_none() {
        merged.reasoning_effort = incoming.reasoning_effort;
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_provider_config_canonicalizes_legacy_api_key_env_binding() {
        let existing =
            mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::Openai);
        let mut incoming =
            mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::Openai);
        incoming.set_api_key_env(Some("OPENAI_API_KEY".to_owned()));

        let merged = merge_provider_config(&existing, &incoming);

        assert_eq!(
            merged.api_key,
            Some(loongclaw_contracts::SecretRef::Env {
                env: "OPENAI_API_KEY".to_owned(),
            })
        );
        assert_eq!(merged.api_key_env, None);
    }
}

pub fn resolve_choice_by_selector_resolution(
    plan: &ProviderSelectionPlan,
    selector: &str,
) -> ImportedChoiceSelectorResolution {
    match mvp::config::resolve_provider_selector(provider_selector_profiles(plan), selector) {
        mvp::config::ProviderSelectorResolution::Match(profile_id) => {
            ImportedChoiceSelectorResolution::Match(profile_id)
        }
        mvp::config::ProviderSelectorResolution::Ambiguous(profile_ids) => {
            ImportedChoiceSelectorResolution::Ambiguous(profile_ids)
        }
        mvp::config::ProviderSelectorResolution::NoMatch => {
            ImportedChoiceSelectorResolution::NoMatch
        }
    }
}

pub fn resolve_choice_by_selector<'a>(
    plan: &'a ProviderSelectionPlan,
    selector: &str,
) -> Option<&'a ImportedProviderChoice> {
    let ImportedChoiceSelectorResolution::Match(profile_id) =
        resolve_choice_by_selector_resolution(plan, selector)
    else {
        return None;
    };
    plan.imported_choices
        .iter()
        .find(|choice| choice.profile_id == profile_id)
}

pub fn accepted_selectors_for_choice(
    plan: &ProviderSelectionPlan,
    profile_id: &str,
) -> Vec<String> {
    mvp::config::accepted_provider_selectors(provider_selector_profiles(plan), profile_id)
}

pub fn preferred_selector_for_choice(
    plan: &ProviderSelectionPlan,
    profile_id: &str,
) -> Option<String> {
    mvp::config::preferred_provider_selector(provider_selector_profiles(plan), profile_id)
}

pub fn selector_catalog(plan: &ProviderSelectionPlan) -> Vec<String> {
    mvp::config::provider_selector_catalog(provider_selector_profiles(plan))
}

pub fn recommendation_hint(plan: &ProviderSelectionPlan) -> Option<String> {
    mvp::config::provider_selector_recommendation_hint(
        provider_selector_profiles(plan),
        plan.imported_choices
            .iter()
            .map(|choice| choice.profile_id.as_str()),
    )
}

pub fn describe_choice(plan: &ProviderSelectionPlan, profile_id: &str) -> Option<String> {
    mvp::config::describe_provider_selector_target(provider_selector_profiles(plan), profile_id)
}

pub fn describe_matching_choices(plan: &ProviderSelectionPlan, profile_ids: &[String]) -> String {
    profile_ids
        .iter()
        .filter_map(|profile_id| describe_choice(plan, profile_id))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn recommendation_hint_for_profile_ids(
    plan: &ProviderSelectionPlan,
    profile_ids: &[String],
) -> Option<String> {
    mvp::config::provider_selector_recommendation_hint(
        provider_selector_profiles(plan),
        profile_ids.iter().map(String::as_str),
    )
}

pub fn guidance_lines(plan: &ProviderSelectionPlan, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    if width >= COMPACT_SELECTOR_DETAIL_WIDTH {
        lines.push(format!(
            "accepted selectors: {}",
            selector_catalog(plan).join(", ")
        ));
    }
    lines.push(
        if width >= COMPACT_SELECTOR_DETAIL_WIDTH {
            PROVIDER_SELECTOR_NOTE
        } else {
            mvp::config::PROVIDER_SELECTOR_COMPACT_NOTE
        }
        .to_owned(),
    );
    if plan.imported_choices.len() > 1
        && let Some(hint) = recommendation_hint(plan)
    {
        lines.push(hint);
    }
    lines
}

pub fn selector_detail_line(
    plan: &ProviderSelectionPlan,
    profile_id: &str,
    width: usize,
) -> Option<String> {
    let selectors = accepted_selectors_for_choice(plan, profile_id);
    let preferred =
        preferred_selector_for_choice(plan, profile_id).or_else(|| selectors.first().cloned())?;
    if selectors.len() == 1 || width < COMPACT_SELECTOR_DETAIL_WIDTH {
        return Some(format!("selector: {preferred}"));
    }
    Some(format!("selectors: {}", selectors.join(", ")))
}

pub fn format_ambiguous_selector_error(
    plan: &ProviderSelectionPlan,
    selector: &str,
    profile_ids: &[String],
) -> String {
    let recommendation = recommendation_hint_for_profile_ids(plan, profile_ids)
        .map(|hint| format!("; {hint}"))
        .unwrap_or_default();
    format!(
        "provider selector `{selector}` is ambiguous; matching profiles: {}{}",
        describe_matching_choices(plan, profile_ids),
        recommendation
    )
}

pub fn format_unknown_selector_error(
    plan: &ProviderSelectionPlan,
    invalid_selector_message: &str,
) -> String {
    let recommendation = recommendation_hint(plan)
        .map(|hint| format!(" {hint}"))
        .unwrap_or_default();
    format!(
        "{invalid_selector_message}. accepted selectors: {}. {}{}",
        selector_catalog(plan).join(", "),
        PROVIDER_SELECTOR_NOTE,
        recommendation
    )
}

pub fn unresolved_choice_note_segments(plan: &ProviderSelectionPlan) -> Vec<String> {
    let mut segments = vec![
        PROVIDER_SELECTION_MERGE_NOTE.to_owned(),
        format!(
            "use --provider {} to choose the active provider",
            PROVIDER_SELECTOR_PLACEHOLDER
        ),
    ];
    if let Some(hint) = recommendation_hint(plan) {
        segments.push(hint);
    }
    segments
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

fn provider_selector_profiles(
    plan: &ProviderSelectionPlan,
) -> Vec<mvp::config::ProviderSelectorProfileRef<'_>> {
    plan.imported_choices
        .iter()
        .map(|choice| {
            mvp::config::ProviderSelectorProfileRef::new(
                choice.profile_id.as_str(),
                choice.kind,
                choice.config.model.as_str(),
                Some(choice.profile_id.as_str()) == plan.default_profile_id.as_deref(),
            )
        })
        .collect()
}

fn assign_profile_ids(imported_choices: &mut [ImportedProviderChoice]) {
    let mut kind_counts = BTreeMap::new();
    for choice in imported_choices.iter() {
        *kind_counts
            .entry(choice.kind.as_str().to_owned())
            .or_insert(0usize) += 1;
    }

    let mut used_ids = BTreeSet::new();
    for choice in imported_choices.iter_mut() {
        let base_id = if kind_counts
            .get(choice.kind.as_str())
            .copied()
            .unwrap_or_default()
            <= 1
        {
            choice.kind.as_str().to_owned()
        } else if let Some(suffix) = provider_profile_id_suffix(&choice.config) {
            format!("{}-{suffix}", choice.kind.as_str())
        } else {
            choice.kind.as_str().to_owned()
        };
        let mut candidate_id = base_id.clone();
        let mut suffix = 2;
        while used_ids.contains(&candidate_id) {
            candidate_id = format!("{base_id}-{suffix}");
            suffix += 1;
        }
        choice.profile_id = candidate_id.clone();
        used_ids.insert(candidate_id);
    }
}

fn provider_profile_id_suffix(provider: &mvp::config::ProviderConfig) -> Option<String> {
    let model_segment = provider
        .model
        .rsplit('/')
        .next()
        .unwrap_or(provider.model.as_str());
    normalize_provider_profile_id_segment(model_segment)
        .filter(|segment| segment != "auto" && segment != provider.kind.as_str())
        .or_else(|| {
            let endpoint = provider
                .endpoint
                .as_deref()
                .unwrap_or(provider.base_url.as_str());
            let host = endpoint
                .split_once("://")
                .map(|(_, rest)| rest)
                .unwrap_or(endpoint)
                .split('/')
                .next()
                .unwrap_or(endpoint);
            normalize_provider_profile_id_segment(host)
                .filter(|segment| segment != provider.kind.as_str())
        })
}

fn normalize_provider_profile_id_segment(raw: &str) -> Option<String> {
    let mut normalized = String::new();
    let mut previous_was_dash = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
            previous_was_dash = false;
        } else if !normalized.is_empty() && !previous_was_dash {
            normalized.push('-');
            previous_was_dash = true;
        }
    }
    while normalized.ends_with('-') {
        normalized.pop();
    }
    (!normalized.is_empty()).then_some(normalized)
}
