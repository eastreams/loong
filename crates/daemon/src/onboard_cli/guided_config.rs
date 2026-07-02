use super::*;

struct GuidedOnboardConfigUpdate {
    provider: mvp::config::ProviderConfig,
    model: String,
    selected_api_key_env: Option<String>,
    prompt_pack_id: Option<String>,
    personality: Option<mvp::prompt::PromptPersonality>,
    selected_system_prompt: Option<String>,
    selected_web_search_provider: String,
    web_search_credential_selection: WebSearchCredentialSelection,
}

pub(super) async fn apply_guided_onboard_configuration(
    options: &OnboardCommandOptions,
    output_path: &Path,
    provider_selection: &crate::migration::ProviderSelectionPlan,
    config: &mut mvp::config::LoongConfig,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    let guided_prompt_path = resolve_guided_prompt_path(options, config);
    let update = collect_guided_onboard_config_update(
        options,
        provider_selection,
        config,
        guided_prompt_path,
        ui,
        context,
    )
    .await?;

    config.provider = update.provider;
    config.provider.model = update.model;

    if config.provider.kind == mvp::config::ProviderKind::GithubCopilot {
        finalize_github_copilot_onboard_credentials(
            &mut config.provider,
            output_path,
            options.non_interactive,
        )
        .await?;
    } else if let Some(selected_api_key_env) = update.selected_api_key_env {
        apply_selected_api_key_env(&mut config.provider, selected_api_key_env);
    }

    apply_guided_prompt_configuration(
        config,
        update.prompt_pack_id,
        update.personality,
        update.selected_system_prompt,
    );

    if let Some(profile_raw) = options.memory_profile.as_deref() {
        config.memory.profile = parse_memory_profile(profile_raw).ok_or_else(|| {
            format!(
                "unsupported --memory-profile value \"{profile_raw}\". supported: {}",
                supported_memory_profile_list()
            )
        })?;
    }

    config.tools.web_search.default_provider = update.selected_web_search_provider.clone();
    apply_selected_web_search_credential(
        config,
        update.selected_web_search_provider.as_str(),
        update.web_search_credential_selection,
    )?;

    Ok(())
}

async fn collect_guided_onboard_config_update(
    options: &OnboardCommandOptions,
    provider_selection: &crate::migration::ProviderSelectionPlan,
    config: &mvp::config::LoongConfig,
    guided_prompt_path: GuidedPromptPath,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<GuidedOnboardConfigUpdate> {
    let provider = resolve_provider_selection(
        options,
        config,
        provider_selection,
        guided_prompt_path,
        ui,
        context,
    )?;
    let mut draft_config = config.clone();
    draft_config.provider = provider.clone();

    let available_models = load_onboarding_model_catalog(options, &draft_config).await;
    let model = resolve_model_selection(
        options,
        &draft_config,
        guided_prompt_path,
        &available_models,
        ui,
        context,
    )?;
    draft_config.provider.model = model.clone();

    let selected_api_key_env =
        if draft_config.provider.kind == mvp::config::ProviderKind::GithubCopilot {
            None
        } else {
            let default_api_key_env = preferred_api_key_env_default(&draft_config);
            Some(resolve_api_key_env_selection(
                options,
                &draft_config,
                default_api_key_env,
                guided_prompt_path,
                ui,
                context,
            )?)
        };

    let (prompt_pack_id, personality, selected_system_prompt) =
        collect_guided_prompt_settings(options, &draft_config, guided_prompt_path, ui, context)
            .await?;

    let selected_web_search_provider = resolve_web_search_provider_selection(
        options,
        &draft_config,
        guided_prompt_path,
        ui,
        context,
    )
    .await?;
    draft_config.tools.web_search.default_provider = selected_web_search_provider.clone();
    let web_search_credential_selection = resolve_web_search_credential_selection(
        options,
        &draft_config,
        selected_web_search_provider.as_str(),
        guided_prompt_path,
        options.non_interactive,
        ui,
        context,
    )?;

    Ok(GuidedOnboardConfigUpdate {
        provider,
        model,
        selected_api_key_env,
        prompt_pack_id,
        personality,
        selected_system_prompt,
        selected_web_search_provider,
        web_search_credential_selection,
    })
}

#[cfg(test)]
mod volcengine_coding_plan_catalog_tests {
    use super::*;

    #[test]
    fn volcengine_coding_plan_domestic_static_catalog_detects_cn_beijing_coding_v3() {
        let provider = mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::VolcengineCoding,
            base_url: "https://ark.cn-beijing.volces.com/api/coding/v3".to_owned(),
            ..mvp::config::ProviderConfig::default()
        };

        assert!(is_volcengine_coding_plan_domestic_static_catalog(&provider));
    }

    #[test]
    fn volcengine_coding_plan_domestic_static_catalog_rejects_non_coding_plan_endpoints() {
        let provider = mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::VolcengineCoding,
            base_url: "https://ark.cn-beijing.volces.com/api/v3".to_owned(),
            ..mvp::config::ProviderConfig::default()
        };

        assert!(!is_volcengine_coding_plan_domestic_static_catalog(
            &provider
        ));
    }

    #[test]
    fn volcengine_coding_plan_domestic_static_catalog_rejects_proxy_path() {
        let provider = mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::VolcengineCoding,
            base_url: "https://proxy.example.com/api/coding/v3".to_owned(),
            ..mvp::config::ProviderConfig::default()
        };

        assert!(!is_volcengine_coding_plan_domestic_static_catalog(
            &provider
        ));
    }
}
