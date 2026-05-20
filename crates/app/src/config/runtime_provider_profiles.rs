use std::collections::BTreeMap;

use crate::CliResult;

use super::{
    ConfigValidationIssue, ConfigValidationSeverity, LoongConfig, ProviderConfig,
    ProviderProfileConfig, normalize_dispatch_channel_id,
};

pub(super) fn normalize_provider_profile_id(raw: &str) -> Option<String> {
    normalize_dispatch_channel_id(raw)
}

#[derive(Debug, Clone, Default)]
pub(super) struct RawProviderSelectionIntent {
    pub legacy_provider_explicit: bool,
    pub active_provider_explicit: bool,
    pub raw_active_provider: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActiveProviderSelectionBasis {
    ExplicitActiveProvider,
    ExplicitLegacyProvider,
    FirstSavedProfile,
    LegacyOnly,
}

impl ActiveProviderSelectionBasis {
    pub const fn diagnostic_summary(self) -> &'static str {
        match self {
            Self::ExplicitActiveProvider => "the explicit active_provider value",
            Self::ExplicitLegacyProvider => "the explicit legacy [provider] table",
            Self::FirstSavedProfile => "the first saved provider profile in sorted order",
            Self::LegacyOnly => "the legacy [provider] table",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct ProviderSelectionNormalizationReport {
    pub legacy_provider_explicit: bool,
    pub active_provider_explicit: bool,
    pub requested_active_provider: Option<String>,
    pub selected_active_provider: Option<String>,
    pub configured_profile_ids: Vec<String>,
    pub selection_basis: Option<ActiveProviderSelectionBasis>,
    pub warn_implicit_active_provider: bool,
    pub warn_unknown_active_provider: bool,
    pub recovered_legacy_profile_id: Option<String>,
    pub legacy_profile_inserted: bool,
    pub legacy_provider_validation_issues: Vec<ConfigValidationIssue>,
}

impl ProviderSelectionNormalizationReport {
    pub fn validation_issues(&self) -> Vec<ConfigValidationIssue> {
        let mut issues = self.legacy_provider_validation_issues.clone();
        let configured_profile_ids = self.configured_profile_ids.join(", ");
        let selected_profile_id = self
            .selected_active_provider
            .as_deref()
            .unwrap_or("unknown")
            .to_owned();
        let selection_basis = self
            .selection_basis
            .map(ActiveProviderSelectionBasis::diagnostic_summary)
            .unwrap_or("provider profile normalization")
            .to_owned();

        if self.warn_implicit_active_provider {
            let mut extra_message_variables = BTreeMap::new();
            extra_message_variables.insert("selected_profile_id".to_owned(), selected_profile_id);
            extra_message_variables.insert("selection_basis".to_owned(), selection_basis.clone());
            extra_message_variables.insert(
                "configured_profile_ids".to_owned(),
                configured_profile_ids.clone(),
            );
            issues.push(ConfigValidationIssue {
                severity: ConfigValidationSeverity::Warn,
                code: super::super::shared::ConfigValidationCode::ImplicitActiveProvider,
                field_path: "active_provider".to_owned(),
                inline_field_path: "providers".to_owned(),
                example_env_name: String::new(),
                suggested_env_name: self.selected_active_provider.clone(),
                extra_message_variables,
            });
        }

        if self.warn_unknown_active_provider {
            let mut extra_message_variables = BTreeMap::new();
            extra_message_variables.insert(
                "requested_profile_id".to_owned(),
                self.requested_active_provider
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("(blank)")
                    .to_owned(),
            );
            extra_message_variables.insert(
                "selected_profile_id".to_owned(),
                self.selected_active_provider
                    .clone()
                    .unwrap_or_else(|| "unknown".to_owned()),
            );
            extra_message_variables.insert("selection_basis".to_owned(), selection_basis);
            extra_message_variables
                .insert("configured_profile_ids".to_owned(), configured_profile_ids);
            issues.push(ConfigValidationIssue {
                severity: ConfigValidationSeverity::Warn,
                code: super::super::shared::ConfigValidationCode::UnknownActiveProvider,
                field_path: "active_provider".to_owned(),
                inline_field_path: "providers".to_owned(),
                example_env_name: String::new(),
                suggested_env_name: self.selected_active_provider.clone(),
                extra_message_variables,
            });
        }

        issues
    }
}

fn normalized_inferred_profile_id(provider: &ProviderConfig) -> String {
    normalize_provider_profile_id(provider.inferred_profile_id().as_str())
        .unwrap_or_else(|| provider.inferred_profile_id())
}

fn matching_legacy_provider_profile_id(
    providers: &BTreeMap<String, ProviderProfileConfig>,
    legacy_provider: &ProviderConfig,
) -> Option<String> {
    let inferred_profile_id = normalized_inferred_profile_id(legacy_provider);
    if providers
        .get(&inferred_profile_id)
        .is_some_and(|profile| profile.provider == *legacy_provider)
    {
        return Some(inferred_profile_id);
    }

    let exact_matches = providers
        .iter()
        .filter(|(_profile_id, profile)| profile.provider == *legacy_provider)
        .map(|(profile_id, _profile)| profile_id.clone())
        .collect::<Vec<_>>();
    if exact_matches.len() == 1 {
        return exact_matches.into_iter().next();
    }
    exact_matches.into_iter().next()
}

fn next_available_provider_profile_id(
    providers: &BTreeMap<String, ProviderProfileConfig>,
    base_profile_id: &str,
) -> String {
    if !providers.contains_key(base_profile_id) {
        return base_profile_id.to_owned();
    }
    let max_suffix = providers.len().saturating_add(2);
    for suffix in 2..=max_suffix {
        let candidate = format!("{base_profile_id}-{suffix}");
        if !providers.contains_key(&candidate) {
            return candidate;
        }
    }
    format!("{base_profile_id}-{max_suffix}")
}

pub(super) fn recover_active_provider_from_legacy_config(
    legacy_provider: &ProviderConfig,
    providers: &mut BTreeMap<String, ProviderProfileConfig>,
) -> (String, bool) {
    if let Some(profile_id) = matching_legacy_provider_profile_id(providers, legacy_provider) {
        return (profile_id, false);
    }

    let profile_id = next_available_provider_profile_id(
        providers,
        normalized_inferred_profile_id(legacy_provider).as_str(),
    );
    let mut recovered_profile = ProviderProfileConfig::from_provider(legacy_provider.clone());
    recovered_profile.default_for_kind = !providers
        .values()
        .any(|profile| profile.provider.kind == recovered_profile.provider.kind);
    providers.insert(profile_id.clone(), recovered_profile);
    (profile_id, true)
}

#[cfg(feature = "config-toml")]
fn inspect_raw_provider_selection_intent(raw: &str) -> CliResult<RawProviderSelectionIntent> {
    let value = toml::from_str::<toml::Value>(raw)
        .map_err(|error| format!("failed to parse TOML config: {error}"))?;
    let table = value.as_table();
    Ok(RawProviderSelectionIntent {
        legacy_provider_explicit: table.is_some_and(|root| root.contains_key("provider")),
        active_provider_explicit: table.is_some_and(|root| root.contains_key("active_provider")),
        raw_active_provider: table
            .and_then(|root| root.get("active_provider"))
            .and_then(toml::Value::as_str)
            .map(str::to_owned),
    })
}

#[cfg(feature = "config-toml")]
pub(super) fn parse_toml_config_components(
    raw: &str,
) -> CliResult<(LoongConfig, ProviderSelectionNormalizationReport)> {
    let mut config = toml::from_str::<LoongConfig>(raw)
        .map_err(|error| format!("failed to parse TOML config: {error}"))?;
    let selection_intent = inspect_raw_provider_selection_intent(raw)?;
    let had_saved_provider_profiles = !config.providers.is_empty();
    let legacy_provider_before_normalization = config.provider.clone();
    let mut selection_report =
        config.normalize_provider_profiles_with_intent(Some(&selection_intent));
    if selection_intent.legacy_provider_explicit && had_saved_provider_profiles {
        selection_report.legacy_provider_validation_issues =
            legacy_provider_before_normalization.validate();
    }
    Ok((config, selection_report))
}

#[cfg(not(feature = "config-toml"))]
pub(super) fn parse_toml_config_components(
    _raw: &str,
) -> CliResult<(LoongConfig, ProviderSelectionNormalizationReport)> {
    Err("config-toml feature is disabled for this build".to_owned())
}
