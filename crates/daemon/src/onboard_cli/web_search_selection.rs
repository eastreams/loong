use super::*;

pub(super) async fn resolve_web_search_provider_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongConfig,
    guided_prompt_path: GuidedPromptPath,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<String> {
    let explicit_override = explicit_web_search_provider_override(options)?;
    if mvp::provider::native_query_search_active(config) && explicit_override.is_none() {
        return Ok(current_web_search_provider(config).to_owned());
    }

    let recommendation = resolve_web_search_provider_recommendation(options, config).await?;
    let recommended_provider = recommendation.provider;
    let default_provider =
        resolve_effective_web_search_default_provider(options, config, &recommendation);

    if options.non_interactive {
        return Ok(default_provider.to_owned());
    }

    let screen_options = build_web_search_provider_screen_options(config, recommended_provider);
    let select_options = select_options_from_screen_options(&screen_options);
    let default_idx = screen_options
        .iter()
        .position(|option| option.key == default_provider);

    print_lines(
        ui,
        render_web_search_provider_selection_screen_lines_with_style(
            config,
            recommended_provider,
            default_provider,
            recommendation.reason.as_str(),
            guided_prompt_path,
            context.render_width,
            true,
        ),
    )?;
    let idx = ui.select_one(
        crate::access_terms::QUERY_SEARCH_PROVIDER_LABEL,
        &select_options,
        default_idx,
        SelectInteractionMode::List,
    )?;
    let selected = select_options
        .get(idx)
        .ok_or_else(|| crate::access_terms::query_search_provider_selection_index_error(idx))?;
    Ok(selected.slug.clone())
}

pub(super) fn resolve_web_search_credential_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongConfig,
    provider: &str,
    guided_prompt_path: GuidedPromptPath,
    non_interactive: bool,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<WebSearchCredentialSelection> {
    let explicit_override = explicit_web_search_provider_override(options)?;
    if mvp::provider::native_query_search_active(config) && explicit_override.is_none() {
        return Ok(WebSearchCredentialSelection::KeepCurrent);
    }

    let Some(descriptor) = mvp::config::web_search_provider_descriptor(provider) else {
        return Ok(WebSearchCredentialSelection::KeepCurrent);
    };
    if !descriptor.requires_api_key {
        return Ok(WebSearchCredentialSelection::KeepCurrent);
    }

    let explicit_selection = if let Some(raw_env_name) = options.web_search_api_key_env.as_deref() {
        if is_explicit_onboard_clear_input(raw_env_name) {
            return Ok(WebSearchCredentialSelection::ClearConfigured);
        }

        let trimmed_env_name = raw_env_name.trim();
        if trimmed_env_name.is_empty() {
            None
        } else {
            let validated_env_name =
                validate_selected_web_search_credential_env(provider, trimmed_env_name)?;
            Some(validated_env_name)
        }
    } else {
        None
    };

    let prompt_default = preferred_query_search_credential_env_default(config, provider);
    if non_interactive {
        if let Some(explicit_env_name) = explicit_selection {
            return Ok(WebSearchCredentialSelection::UseEnv(explicit_env_name));
        }

        return Ok(if prompt_default.trim().is_empty() {
            WebSearchCredentialSelection::KeepCurrent
        } else {
            WebSearchCredentialSelection::UseEnv(prompt_default)
        });
    }

    let initial_value = explicit_selection
        .as_deref()
        .unwrap_or(prompt_default.as_str());
    let example_env_name = descriptor
        .default_api_key_env
        .or_else(|| descriptor.api_key_env_names.first().copied())
        .unwrap_or("WEB_SEARCH_API_KEY")
        .to_owned();
    loop {
        print_lines(
            ui,
            render_web_search_credential_selection_screen_lines_with_style(
                config,
                provider,
                initial_value,
                guided_prompt_path,
                context.render_width,
                true,
            ),
        )?;
        let value = ui.prompt_with_default(
            crate::access_terms::query_search_credential_prompt_label(),
            initial_value,
        )?;
        if is_explicit_onboard_clear_input(&value) {
            return Ok(WebSearchCredentialSelection::ClearConfigured);
        }
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Ok(WebSearchCredentialSelection::KeepCurrent);
        }
        match validate_selected_web_search_credential_env(provider, trimmed) {
            Ok(validated) => return Ok(WebSearchCredentialSelection::UseEnv(validated)),
            Err(error) => {
                print_message(ui, error)?;
                print_message(
                    ui,
                    crate::access_terms::query_search_credential_input_hint(
                        example_env_name.as_str(),
                    ),
                )?;
            }
        }
    }
}

pub(super) fn build_web_search_provider_screen_options(
    config: &mvp::config::LoongConfig,
    recommended_provider: &str,
) -> Vec<OnboardScreenOption> {
    mvp::config::web_search_provider_descriptors()
        .iter()
        .map(|descriptor| {
            let mut detail_lines = vec![descriptor.description.to_owned()];
            if let Some(credential) = summarize_query_search_credential(config, descriptor.id) {
                detail_lines.push(format!("{}: {}", credential.label, credential.value));
            }
            OnboardScreenOption {
                key: descriptor.id.to_owned(),
                label: descriptor.display_name.to_owned(),
                detail_lines,
                recommended: descriptor.id == recommended_provider,
            }
        })
        .collect()
}

pub(super) fn render_web_search_provider_selection_screen_lines_with_style(
    config: &mvp::config::LoongConfig,
    recommended_provider: &str,
    default_provider: &str,
    recommendation_reason: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let current_provider = current_web_search_provider(config);
    let current_provider_label = query_search_provider_display_name(current_provider);
    let recommended_provider_label = query_search_provider_display_name(recommended_provider);
    let default_provider_label = query_search_provider_display_name(default_provider);
    let options = build_web_search_provider_screen_options(config, recommended_provider);
    let default_footer_description = if default_provider == current_provider {
        format!("keep {current_provider_label}")
    } else {
        format!("use {default_provider_label}")
    };

    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        crate::access_terms::CHOOSE_QUERY_SEARCH_TITLE,
        crate::access_terms::CHOOSE_QUERY_SEARCH_PROVIDER_TITLE,
        Some((GuidedOnboardStep::WebSearchProvider, guided_prompt_path)),
        vec![
            format!("- current provider: {current_provider_label}"),
            format!("- recommended provider: {recommended_provider_label}"),
            format!("- why this is recommended: {recommendation_reason}"),
        ],
        options,
        vec![render_default_choice_footer_line(
            "Enter",
            default_footer_description.as_str(),
        )],
        true,
        color_enabled,
    )
}
