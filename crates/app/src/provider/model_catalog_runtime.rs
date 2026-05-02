use super::capability_profile_runtime;
use super::contracts;
use super::provider_runtime_contract;
use crate::config::{ProviderConfig, ReasoningEffort};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderModelCatalogEntry {
    pub model: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub is_default: bool,
    pub hidden: bool,
    pub deprecated: bool,
    pub default_reasoning_effort: Option<ReasoningEffort>,
    pub supported_reasoning_efforts: Vec<ReasoningEffort>,
    pub supported_reasoning_effort_descriptions: Vec<(ReasoningEffort, String)>,
}

const DEFAULT_REASONING_EFFORTS: &[ReasoningEffort] = &[
    ReasoningEffort::None,
    ReasoningEffort::Minimal,
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
    ReasoningEffort::Xhigh,
];

fn normalized_reasoning_model_id(model: &str) -> String {
    model.trim().to_ascii_lowercase()
}

fn model_supports_xhigh_reasoning(model: &str) -> bool {
    let model = normalized_reasoning_model_id(model);
    model.contains("gpt-5.2")
        || model.contains("gpt-5.3")
        || model.contains("gpt-5.4")
        || model.contains("gpt-5.5")
        || model.contains("opus-4-6")
        || model.contains("opus-4.6")
        || model.contains("opus-4-7")
        || model.contains("opus-4.7")
}

fn model_supports_minimal_reasoning(model: &str) -> bool {
    let model = normalized_reasoning_model_id(model);
    !(model.contains("gpt-5.3-codex") || model.contains("gpt-5.4") || model.contains("gpt-5.5"))
}

pub fn default_reasoning_effort_for_model(
    provider: &ProviderConfig,
    model: &str,
) -> Option<ReasoningEffort> {
    let supported = supported_reasoning_efforts_for_model(provider, model);
    let model = normalized_reasoning_model_id(model);
    if supported.is_empty() {
        return None;
    }

    if model.contains("gpt-5.4") && supported.contains(&ReasoningEffort::Xhigh) {
        return Some(ReasoningEffort::Xhigh);
    }

    if (model.contains("gpt-5.2") || model.contains("gpt-5.3") || model.contains("gpt-5.5"))
        && supported.contains(&ReasoningEffort::Medium)
    {
        return Some(ReasoningEffort::Medium);
    }

    supported
        .iter()
        .copied()
        .find(|effort| *effort != ReasoningEffort::None)
        .or_else(|| supported.first().copied())
}

pub fn effective_supported_reasoning_efforts_for_entry(
    provider: &ProviderConfig,
    entry: &ProviderModelCatalogEntry,
) -> Vec<ReasoningEffort> {
    if !entry.supported_reasoning_efforts.is_empty() {
        return entry.supported_reasoning_efforts.clone();
    }

    supported_reasoning_efforts_for_model(provider, entry.model.as_str())
}

pub fn reasoning_effort_description_for_entry(
    entry: &ProviderModelCatalogEntry,
    effort: ReasoningEffort,
) -> Option<&str> {
    entry
        .supported_reasoning_effort_descriptions
        .iter()
        .find(|(candidate, _)| *candidate == effort)
        .map(|(_, description)| description.as_str())
}

pub fn effective_default_reasoning_effort_for_entry(
    provider: &ProviderConfig,
    entry: &ProviderModelCatalogEntry,
) -> Option<ReasoningEffort> {
    entry
        .default_reasoning_effort
        .or_else(|| default_reasoning_effort_for_model(provider, entry.model.as_str()))
}

pub fn supported_reasoning_efforts_for_model(
    provider: &ProviderConfig,
    model: &str,
) -> Vec<ReasoningEffort> {
    let runtime_contract = provider_runtime_contract(provider);
    let capability_profile = capability_profile_runtime::ProviderCapabilityProfile::from_provider(
        provider,
        runtime_contract,
    );
    let capability = capability_profile.resolve_for_model(model);
    let supports_reasoning = runtime_contract.default_reasoning_field
        != contracts::ReasoningField::Omit
        || capability.reasoning_extra_body_mode != contracts::ProviderReasoningExtraBodyMode::Omit;
    if !supports_reasoning {
        return Vec::new();
    }

    let mut supported = provider
        .kind
        .allowed_reasoning_efforts()
        .unwrap_or(DEFAULT_REASONING_EFFORTS)
        .to_vec();
    if !model_supports_xhigh_reasoning(model) {
        supported.retain(|effort| *effort != ReasoningEffort::Xhigh);
    }
    if !model_supports_minimal_reasoning(model) {
        supported.retain(|effort| *effort != ReasoningEffort::Minimal);
    }

    supported
}
