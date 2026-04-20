use crate::config::ProviderConfig;

use super::contracts::{
    ProviderCapabilityContract, ProviderReasoningExtraBodyMode, ProviderRuntimeContract,
    ProviderToolSchemaMode,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProviderCapabilityProfile {
    tool_schema_disabled: Vec<String>,
    tool_schema_strict: Vec<String>,
    reasoning_extra_body_kimi: Vec<String>,
    reasoning_extra_body_omit: Vec<String>,
    base_capability: ProviderCapabilityContract,
}

impl ProviderCapabilityProfile {
    pub(super) fn from_provider(
        provider: &ProviderConfig,
        runtime_contract: ProviderRuntimeContract,
    ) -> Self {
        Self {
            tool_schema_disabled: normalize_model_hint_values(
                provider.resolved_tool_schema_disabled_model_hints(),
            ),
            tool_schema_strict: normalize_model_hint_values(
                provider.resolved_tool_schema_strict_model_hints(),
            ),
            reasoning_extra_body_kimi: normalize_model_hint_values(
                provider.resolved_reasoning_extra_body_kimi_model_hints(),
            ),
            reasoning_extra_body_omit: normalize_model_hint_values(
                provider.resolved_reasoning_extra_body_omit_model_hints(),
            ),
            base_capability: runtime_contract.capability,
        }
    }

    pub(super) fn resolve_for_model(&self, model: &str) -> ProviderCapabilityContract {
        let mut capability = self.base_capability;
        let normalized_model = model.trim().to_ascii_lowercase();
        if normalized_model.is_empty() {
            return capability;
        }

        if model_matches_any_hint(normalized_model.as_str(), &self.tool_schema_disabled) {
            capability.tool_schema_mode = ProviderToolSchemaMode::Disabled;
        } else if model_matches_any_hint(normalized_model.as_str(), &self.tool_schema_strict) {
            capability.tool_schema_mode = ProviderToolSchemaMode::EnabledStrict;
        }

        if model_matches_any_hint(normalized_model.as_str(), &self.reasoning_extra_body_omit) {
            capability.reasoning_extra_body_mode = ProviderReasoningExtraBodyMode::Omit;
        } else if model_matches_any_hint(normalized_model.as_str(), &self.reasoning_extra_body_kimi)
        {
            capability.reasoning_extra_body_mode = ProviderReasoningExtraBodyMode::KimiThinking;
        }

        capability
    }

    #[cfg(test)]
    fn normalized_hints(&self) -> (&[String], &[String], &[String], &[String]) {
        (
            &self.tool_schema_disabled,
            &self.tool_schema_strict,
            &self.reasoning_extra_body_kimi,
            &self.reasoning_extra_body_omit,
        )
    }
}

fn normalize_model_hint_values<T, I>(hints: I) -> Vec<String>
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let mut normalized = Vec::new();
    for hint in hints {
        let lowercased = hint.as_ref().trim().to_ascii_lowercase();
        if lowercased.is_empty() || normalized.iter().any(|existing| existing == &lowercased) {
            continue;
        }
        normalized.push(lowercased);
    }
    normalized
}

fn model_matches_any_hint(model: &str, hints: &[String]) -> bool {
    hints.iter().any(|hint| model.contains(hint.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ProviderConfig, ProviderKind};
    use crate::provider::contracts::{ProviderToolSchemaMode, provider_runtime_contract};

    #[test]
    fn capability_profile_normalizes_deduplicates_hints() {
        let provider = ProviderConfig {
            kind: ProviderKind::Openai,
            tool_schema_disabled_model_hints: vec![
                " No-Tools ".to_owned(),
                "no-tools".to_owned(),
                "".to_owned(),
            ],
            tool_schema_strict_model_hints: vec![
                " STRICT-TOOLS ".to_owned(),
                "strict-tools".to_owned(),
            ],
            reasoning_extra_body_kimi_model_hints: vec![" THink-ENabled ".to_owned()],
            reasoning_extra_body_omit_model_hints: vec!["think-disabled".to_owned()],
            ..ProviderConfig::default()
        };
        let profile = ProviderCapabilityProfile::from_provider(
            &provider,
            provider_runtime_contract(&provider),
        );
        let (disabled, strict, kimi, omit) = profile.normalized_hints();
        assert_eq!(disabled, &["no-tools"]);
        assert_eq!(strict, &["strict-tools"]);
        assert_eq!(kimi, &["think-enabled"]);
        assert_eq!(omit, &["think-disabled"]);
    }

    #[test]
    fn capability_profile_resolves_model_hint_overrides_with_precedence() {
        let provider = ProviderConfig {
            kind: ProviderKind::Openai,
            tool_schema_disabled_model_hints: vec!["shared".to_owned()],
            tool_schema_strict_model_hints: vec!["shared".to_owned()],
            reasoning_extra_body_kimi_model_hints: vec!["shared".to_owned()],
            reasoning_extra_body_omit_model_hints: vec!["shared".to_owned()],
            ..ProviderConfig::default()
        };
        let profile = ProviderCapabilityProfile::from_provider(
            &provider,
            provider_runtime_contract(&provider),
        );
        let capability = profile.resolve_for_model("gpt-shared-v1");
        assert_eq!(
            capability.tool_schema_mode,
            ProviderToolSchemaMode::Disabled
        );
        assert_eq!(
            capability.reasoning_extra_body_mode,
            ProviderReasoningExtraBodyMode::Omit
        );
    }

    #[test]
    fn capability_profile_returns_base_capability_for_non_matching_model() {
        let provider = ProviderConfig {
            kind: ProviderKind::KimiCoding,
            ..ProviderConfig::default()
        };
        let runtime_contract = provider_runtime_contract(&provider);
        let profile = ProviderCapabilityProfile::from_provider(&provider, runtime_contract);
        let capability = profile.resolve_for_model("kimi-for-coding");
        assert_eq!(capability, runtime_contract.capability);
    }
}
