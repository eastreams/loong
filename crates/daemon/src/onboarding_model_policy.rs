use loongclaw_app as mvp;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OnboardingModelSelectionContext {
    pub(crate) current_model: String,
    pub(crate) recommended_model: Option<String>,
    pub(crate) preferred_fallback_models: Vec<String>,
    pub(crate) allows_auto_fallback_hint: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OnboardingModelCatalogChoices {
    pub(crate) ordered_models: Vec<String>,
    pub(crate) default_index: Option<usize>,
}

pub(crate) fn onboarding_model_selection_context(
    provider: &mvp::config::ProviderConfig,
) -> OnboardingModelSelectionContext {
    let current_model = provider.model.clone();
    let recommended_model = recommended_onboarding_model_for_context(provider);
    let preferred_fallback_models = provider.configured_auto_model_candidates();
    let has_preferred_fallback_models = !preferred_fallback_models.is_empty();
    let explicit_model = provider.explicit_model();
    let has_explicit_model = explicit_model.is_some();
    let allows_auto_fallback_hint = !has_explicit_model && has_preferred_fallback_models;

    OnboardingModelSelectionContext {
        current_model,
        recommended_model,
        preferred_fallback_models,
        allows_auto_fallback_hint,
    }
}

pub(crate) fn resolve_onboarding_model_prompt_default(
    provider: &mvp::config::ProviderConfig,
    explicit_model_override: Option<&str>,
) -> Result<String, String> {
    let explicit_model_override = validated_explicit_model_override(explicit_model_override)?;
    if let Some(explicit_model_override) = explicit_model_override {
        return Ok(explicit_model_override);
    }

    let explicit_model = provider.explicit_model();
    if let Some(explicit_model) = explicit_model {
        return Ok(explicit_model);
    }

    let configured_model = provider.configured_model_value();
    let uses_auto_discovery = configured_model.eq_ignore_ascii_case("auto");
    if !uses_auto_discovery {
        return Ok(configured_model);
    }

    let recommended_model = provider.kind.recommended_onboarding_model();
    let Some(recommended_model) = recommended_model else {
        return Ok(configured_model);
    };

    let recommended_model = recommended_model.to_owned();
    Ok(recommended_model)
}

pub(crate) fn onboarding_model_catalog_choices(
    prompt_default: &str,
    available_models: &[String],
) -> OnboardingModelCatalogChoices {
    let trimmed_default_model = prompt_default.trim();
    let mut ordered_models = Vec::new();

    if !trimmed_default_model.is_empty() {
        let default_model = trimmed_default_model.to_owned();
        ordered_models.push(default_model);
    }

    for raw_model in available_models {
        let trimmed_model = raw_model.trim();
        let model_is_blank = trimmed_model.is_empty();
        if model_is_blank {
            continue;
        }

        let model_is_duplicate = ordered_models
            .iter()
            .any(|existing_model| existing_model == trimmed_model);
        if model_is_duplicate {
            continue;
        }

        let model = trimmed_model.to_owned();
        ordered_models.push(model);
    }

    let default_index = ordered_models
        .iter()
        .position(|model| model == trimmed_default_model);

    OnboardingModelCatalogChoices {
        ordered_models,
        default_index,
    }
}

fn validated_explicit_model_override(
    explicit_model_override: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(raw_model) = explicit_model_override else {
        return Ok(None);
    };

    let trimmed_model = raw_model.trim();
    let model_is_blank = trimmed_model.is_empty();
    if model_is_blank {
        return Err("model cannot be empty".to_owned());
    }

    let validated_model = trimmed_model.to_owned();
    Ok(Some(validated_model))
}

fn recommended_onboarding_model_for_context(
    provider: &mvp::config::ProviderConfig,
) -> Option<String> {
    let current_model = provider.model.as_str();
    let recommended_model = provider.kind.recommended_onboarding_model()?;
    let recommended_matches_current_model = recommended_model == current_model;
    if recommended_matches_current_model {
        return None;
    }

    let recommended_model = recommended_model.to_owned();
    Some(recommended_model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_onboarding_model_prompt_default_prefers_explicit_override() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "auto".to_owned();

        let prompt_default =
            resolve_onboarding_model_prompt_default(&config.provider, Some("deepseek-reasoner"))
                .expect("resolve prompt default");

        assert_eq!(prompt_default, "deepseek-reasoner");
    }

    #[test]
    fn resolve_onboarding_model_prompt_default_rejects_blank_override() {
        let config = mvp::config::LoongClawConfig::default();

        let error = resolve_onboarding_model_prompt_default(&config.provider, Some("   "))
            .expect_err("blank explicit override should fail");

        assert_eq!(error, "model cannot be empty");
    }

    #[test]
    fn resolve_onboarding_model_prompt_default_keeps_explicit_model() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "deepseek-chat".to_owned();

        let prompt_default = resolve_onboarding_model_prompt_default(&config.provider, None)
            .expect("resolve prompt default");

        assert_eq!(prompt_default, "deepseek-chat");
    }

    #[test]
    fn resolve_onboarding_model_prompt_default_uses_reviewed_default_for_auto() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Minimax;
        config.provider.model = "auto".to_owned();

        let prompt_default = resolve_onboarding_model_prompt_default(&config.provider, None)
            .expect("resolve prompt default");

        assert_eq!(prompt_default, "MiniMax-M2.7");
    }

    #[test]
    fn resolve_onboarding_model_prompt_default_uses_xiaomi_reviewed_default_for_auto() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Xiaomi;
        config.provider.model = "auto".to_owned();

        let prompt_default = resolve_onboarding_model_prompt_default(&config.provider, None)
            .expect("resolve prompt default");

        assert_eq!(prompt_default, "mimo-v2-pro");
    }

    #[test]
    fn resolve_onboarding_model_prompt_default_keeps_auto_for_unreviewed_provider() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Custom;
        config.provider.model = "auto".to_owned();

        let prompt_default = resolve_onboarding_model_prompt_default(&config.provider, None)
            .expect("resolve prompt default");

        assert_eq!(prompt_default, "auto");
    }

    #[test]
    fn onboarding_model_selection_context_surfaces_preferred_fallback_hint() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Minimax;
        config.provider.model = "auto".to_owned();
        config.provider.preferred_models = vec!["MiniMax-M2.5".to_owned()];

        let context = onboarding_model_selection_context(&config.provider);

        assert_eq!(context.current_model, "auto");
        assert_eq!(context.recommended_model, Some("MiniMax-M2.7".to_owned()));
        assert_eq!(context.preferred_fallback_models, vec!["MiniMax-M2.5"]);
        assert!(context.allows_auto_fallback_hint);
    }

    #[test]
    fn onboarding_model_selection_context_hides_duplicate_reviewed_model_note() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "deepseek-chat".to_owned();

        let context = onboarding_model_selection_context(&config.provider);

        assert_eq!(context.recommended_model, None);
    }

    #[test]
    fn onboarding_model_catalog_choices_keep_default_first_and_deduplicate() {
        let available_models = vec![
            " deepseek-chat ".to_owned(),
            "deepseek-reasoner".to_owned(),
            "deepseek-chat".to_owned(),
            "   ".to_owned(),
        ];

        let choices = onboarding_model_catalog_choices("deepseek-chat", &available_models);

        assert_eq!(
            choices.ordered_models,
            vec!["deepseek-chat".to_owned(), "deepseek-reasoner".to_owned(),]
        );
        assert_eq!(choices.default_index, Some(0));
    }
}
