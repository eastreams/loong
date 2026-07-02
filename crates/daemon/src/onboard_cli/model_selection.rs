use super::*;

pub(super) fn resolve_model_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongConfig,
    guided_prompt_path: GuidedPromptPath,
    available_models: &[String],
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<String> {
    let prompt_default = onboarding_model_policy::resolve_onboarding_model_prompt_default(
        &config.provider,
        options.model.as_deref(),
    )?;

    if options.non_interactive {
        return Ok(prompt_default);
    }

    print_lines(
        ui,
        render_model_selection_screen_lines_with_style(
            config,
            prompt_default.as_str(),
            guided_prompt_path,
            context.render_width,
            true,
            !available_models.is_empty(),
        ),
    )?;
    if !available_models.is_empty() {
        // When we render the model catalog choices from a static provider list,
        // we still compute `prompt_default` (often `auto`) for the prompt UI.
        // Hide `auto` from the selectable catalog to match operator expectations.
        let hide_prompt_default_from_catalog = prompt_default.trim().eq_ignore_ascii_case("auto")
            && is_volcengine_coding_plan_domestic_static_catalog(&config.provider);

        let effective_prompt_default = if hide_prompt_default_from_catalog {
            ""
        } else {
            prompt_default.as_str()
        };

        let catalog_choices = onboarding_model_policy::onboarding_model_catalog_choices(
            effective_prompt_default,
            available_models,
        );
        let (select_options, default_idx) = build_model_selection_options(&catalog_choices);
        let idx = ui.select_one(
            "Model",
            &select_options,
            default_idx,
            SelectInteractionMode::Search,
        )?;
        let selected = select_options
            .get(idx)
            .ok_or_else(|| format!("model selection index {idx} out of range"))?;
        if selected.slug != ONBOARD_CUSTOM_MODEL_OPTION_SLUG {
            return Ok(selected.slug.clone());
        }
        let custom_model = ui.prompt_with_default("Custom model id", effective_prompt_default)?;
        let trimmed = custom_model.trim();
        if trimmed.is_empty() {
            return Err("model cannot be empty".to_owned());
        }
        return Ok(trimmed.to_owned());
    }
    let value = ui.prompt_with_default("Model", prompt_default.as_str())?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("model cannot be empty".to_owned());
    }
    Ok(trimmed.to_owned())
}

pub(super) async fn load_onboarding_model_catalog(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongConfig,
) -> Vec<String> {
    // Volcano Engine "Coding Plan" domestic endpoint has a stable, operator-provided model list.
    // Using it avoids an interactive onboarding dependency on `GET /models`.
    if is_volcengine_coding_plan_domestic_static_catalog(&config.provider) {
        return vec![
            // Keep the historical default model id as an explicit choice.
            "ark-code-latest".to_owned(),
            "doubao-seed-2.0-code".to_owned(),
            "doubao-seed-2.0-pro".to_owned(),
            "doubao-seed-2.0-lite".to_owned(),
            "doubao-seed-code".to_owned(),
            "minimax-m2.5".to_owned(),
            "glm-4.7".to_owned(),
            "deepseek-v3.2".to_owned(),
            "kimi-k2.5".to_owned(),
        ];
    }

    if options.non_interactive || options.skip_model_probe {
        return Vec::new();
    }
    let has_provider_credentials = mvp::provider::provider_auth_ready(config).await;
    let provider_requires_explicit_auth = config.provider.requires_explicit_auth_configuration();
    if !has_provider_credentials && provider_requires_explicit_auth {
        return Vec::new();
    }
    mvp::provider::fetch_available_models(config)
        .await
        .unwrap_or_default()
}

pub(super) fn is_volcengine_coding_plan_domestic_static_catalog(
    provider: &mvp::config::ProviderConfig,
) -> bool {
    if provider.kind != mvp::config::ProviderKind::VolcengineCoding {
        return false;
    }

    let Ok(actual_url) = reqwest::Url::parse(provider.resolved_base_url().trim()) else {
        return false;
    };
    let Ok(canonical_url) = reqwest::Url::parse(
        mvp::config::ProviderKind::VolcengineCoding
            .profile()
            .base_url,
    ) else {
        return false;
    };

    actual_url.scheme() == canonical_url.scheme()
        && actual_url.host_str() == canonical_url.host_str()
        && actual_url.port_or_known_default() == canonical_url.port_or_known_default()
        && actual_url.path().trim_end_matches('/') == canonical_url.path().trim_end_matches('/')
}

pub(super) fn build_model_selection_options(
    catalog_choices: &onboarding_model_policy::OnboardingModelCatalogChoices,
) -> (Vec<SelectOption>, Option<usize>) {
    let default_idx = catalog_choices.default_index;
    let mut options = Vec::new();

    for (index, model) in catalog_choices.ordered_models.iter().enumerate() {
        let is_default_model = default_idx == Some(index);
        let description = if is_default_model {
            "current or suggested default".to_owned()
        } else {
            String::new()
        };

        let option = SelectOption {
            label: model.clone(),
            slug: model.clone(),
            description,
            recommended: is_default_model,
        };
        options.push(option);
    }

    options.push(SelectOption {
        label: "enter custom model id".to_owned(),
        slug: ONBOARD_CUSTOM_MODEL_OPTION_SLUG.to_owned(),
        description: "manually type any provider model id".to_owned(),
        recommended: false,
    });

    (options, default_idx)
}
