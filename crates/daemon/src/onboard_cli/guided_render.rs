use super::render::render_onboard_input_screen;
use super::review::provider_supports_blank_api_key_env;
use super::*;

fn render_default_input_hint_line(description: impl AsRef<str>) -> String {
    format!("- press Enter to {}", description.as_ref())
}

fn render_clear_input_hint_line(description: impl AsRef<str>) -> String {
    format!(
        "- type {ONBOARD_CLEAR_INPUT_TOKEN} to {}",
        description.as_ref()
    )
}

fn render_model_selection_default_hint_line(
    config: &mvp::config::LoongConfig,
    prompt_default: &str,
) -> String {
    let prompt_default = prompt_default.trim();
    let current_model = config.provider.model.trim();
    if prompt_default == current_model {
        render_default_input_hint_line("keep current model")
    } else if prompt_default.is_empty() {
        render_default_input_hint_line("leave the model blank")
    } else {
        render_default_input_hint_line(format!("use prefilled model: {prompt_default}"))
    }
}

fn render_api_key_env_selection_default_hint_line(
    config: &mvp::config::LoongConfig,
    suggested_env: &str,
    prompt_default: &str,
) -> String {
    let prompt_default =
        provider_credential_policy::render_provider_credential_source_value(Some(prompt_default))
            .unwrap_or_default();
    let suggested_env =
        provider_credential_policy::render_provider_credential_source_value(Some(suggested_env))
            .unwrap_or_default();
    let current_env =
        provider_credential_policy::configured_provider_credential_env_binding(&config.provider)
            .and_then(|binding| {
                provider_credential_policy::render_provider_credential_source_value(Some(
                    binding.env_name.as_str(),
                ))
            });

    if prompt_default.is_empty() {
        return render_default_input_hint_line("leave this blank");
    }

    if current_env
        .as_deref()
        .is_some_and(|current_env| current_env == prompt_default)
    {
        return render_default_input_hint_line("keep current source");
    }

    if !suggested_env.is_empty() && prompt_default == suggested_env {
        return render_default_input_hint_line(format!("use suggested source: {prompt_default}"));
    }

    render_default_input_hint_line(format!("use prefilled source: {prompt_default}"))
}

fn render_web_search_credential_selection_default_hint_line(
    config: &mvp::config::LoongConfig,
    provider: &str,
    prompt_default: &str,
) -> String {
    let prompt_default =
        provider_credential_policy::render_provider_credential_source_value(Some(prompt_default))
            .unwrap_or_default();
    let suggested_env = mvp::config::web_search_provider_descriptor(provider)
        .and_then(|descriptor| descriptor.default_api_key_env)
        .and_then(|env_name| {
            provider_credential_policy::render_provider_credential_source_value(Some(env_name))
        })
        .unwrap_or_default();
    let current_env =
        configured_query_search_credential_env_name(config, provider).and_then(|env_name| {
            provider_credential_policy::render_provider_credential_source_value(Some(
                env_name.as_str(),
            ))
        });

    if prompt_default.is_empty() {
        return render_default_input_hint_line("leave this blank");
    }

    if current_env
        .as_deref()
        .is_some_and(|current_env| current_env == prompt_default)
    {
        return render_default_input_hint_line("keep current source");
    }

    if !suggested_env.is_empty() && prompt_default == suggested_env {
        return render_default_input_hint_line(format!("use suggested source: {prompt_default}"));
    }

    render_default_input_hint_line(format!("use prefilled source: {prompt_default}"))
}

fn render_system_prompt_selection_default_hint_line(
    config: &mvp::config::LoongConfig,
    prompt_default: &str,
) -> String {
    let prompt_default = prompt_default.trim();
    let current_prompt = config.cli.system_prompt.trim();

    if prompt_default == current_prompt {
        if current_prompt.is_empty() {
            render_default_input_hint_line("keep the built-in default")
        } else {
            render_default_input_hint_line("keep current prompt")
        }
    } else if prompt_default.is_empty() {
        render_default_input_hint_line("keep the built-in default")
    } else {
        render_default_input_hint_line(format!("use prefilled prompt: {prompt_default}"))
    }
}

fn with_default_choice_footer(
    mut footer_lines: Vec<String>,
    default_choice_line: Option<String>,
) -> Vec<String> {
    if let Some(default_choice_line) = default_choice_line {
        footer_lines.insert(0, default_choice_line);
    }
    footer_lines
}

pub fn render_provider_selection_screen_lines(
    plan: &crate::migration::ProviderSelectionPlan,
    width: usize,
) -> Vec<String> {
    render_provider_selection_screen_lines_with_style(
        plan,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
    )
}

fn render_provider_selection_screen_lines_with_style(
    plan: &crate::migration::ProviderSelectionPlan,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let intro = provider_selection_intro_lines(plan);
    let options = plan
        .imported_choices
        .iter()
        .map(|choice| OnboardScreenOption {
            key: choice.profile_id.clone(),
            label: provider_kind_display_name(choice.kind).to_owned(),
            detail_lines: {
                let mut detail_lines = vec![
                    format!("source: {}", choice.source),
                    format!("summary: {}", choice.summary),
                ];
                if let Some(selector_detail) =
                    crate::migration::provider_selection::selector_detail_line(
                        plan,
                        &choice.profile_id,
                        width,
                    )
                {
                    detail_lines.push(selector_detail);
                }
                if let Some(transport_summary) = choice.config.preview_transport_summary() {
                    detail_lines.push(format!("transport: {transport_summary}"));
                }
                detail_lines
            },
            recommended: Some(choice.profile_id.as_str()) == plan.default_profile_id.as_deref(),
        })
        .collect::<Vec<_>>();
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "choose the current provider",
        "choose active provider",
        Some((GuidedOnboardStep::Provider, guided_prompt_path)),
        intro,
        options,
        with_default_choice_footer(
            crate::migration::guidance_lines(plan, width),
            render_provider_selection_default_choice_footer_line(plan),
        ),
        true,
        color_enabled,
    )
}

pub(super) fn render_provider_selection_header_lines(
    plan: &crate::migration::ProviderSelectionPlan,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
) -> Vec<String> {
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "choose the current provider",
        "choose active provider",
        Some((GuidedOnboardStep::Provider, guided_prompt_path)),
        provider_selection_intro_lines(plan),
        vec![],
        vec![],
        true,
        true,
    )
}

fn provider_selection_intro_lines(plan: &crate::migration::ProviderSelectionPlan) -> Vec<String> {
    if plan.imported_choices.is_empty() {
        vec!["pick the provider that should back this setup".to_owned()]
    } else if plan.requires_explicit_choice {
        vec!["other detected settings stay merged".to_owned()]
    } else {
        vec!["review the detected provider choices for this setup".to_owned()]
    }
}

fn render_provider_selection_default_choice_footer_line(
    plan: &crate::migration::ProviderSelectionPlan,
) -> Option<String> {
    if plan.requires_explicit_choice {
        return None;
    }
    let default_profile_id = plan.default_profile_id.as_deref()?;
    let default_kind = plan
        .imported_choices
        .iter()
        .find(|choice| choice.profile_id == default_profile_id)
        .map(|choice| choice.kind)
        .or(plan.default_kind)?;
    Some(render_default_choice_footer_line(
        default_profile_id,
        &format!("the {} provider", provider_kind_display_name(default_kind)),
    ))
}

pub fn render_model_selection_screen_lines(
    config: &mvp::config::LoongConfig,
    width: usize,
) -> Vec<String> {
    render_model_selection_screen_lines_with_style(
        config,
        config.provider.model.as_str(),
        GuidedPromptPath::NativePromptPack,
        width,
        false,
        false,
    )
}

pub fn render_model_selection_screen_lines_with_default(
    config: &mvp::config::LoongConfig,
    prompt_default: &str,
    width: usize,
) -> Vec<String> {
    render_model_selection_screen_lines_with_style(
        config,
        prompt_default,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
        false,
    )
}

pub(super) fn render_model_selection_screen_lines_with_style(
    config: &mvp::config::LoongConfig,
    prompt_default: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
    catalog_models_available: bool,
) -> Vec<String> {
    let selection_context =
        onboarding_model_policy::onboarding_model_selection_context(&config.provider);
    let current_model = selection_context.current_model;
    let recommended_model = selection_context.recommended_model;
    let preferred_fallback_models = selection_context.preferred_fallback_models;
    let allows_auto_fallback_hint = selection_context.allows_auto_fallback_hint;
    let mut context_lines = vec![
        format!(
            "- provider: {}",
            crate::provider::presentation::guided_provider_label(config.provider.kind)
        ),
        format!("- current model: {current_model}"),
    ];
    if let Some(recommended_model) = recommended_model {
        context_lines.push(format!("- recommended model: {recommended_model}"));
    }
    if !preferred_fallback_models.is_empty() {
        let preferred_fallback_summary = preferred_fallback_models.join(", ");
        context_lines.push(format!(
            "- configured preferred fallback: {preferred_fallback_summary}",
        ));
    }

    let mut hint_lines = vec![render_model_selection_default_hint_line(
        config,
        prompt_default,
    )];
    if catalog_models_available {
        hint_lines.push(
            "- use arrow keys to browse or type to filter available provider models".to_owned(),
        );
        hint_lines.push(
            "- choose `enter custom model id` if you want to type an override manually".to_owned(),
        );
    } else {
        hint_lines.push("- type any provider model id to override it".to_owned());
    }
    if allows_auto_fallback_hint {
        let preferred_fallback_summary = preferred_fallback_models.join(", ");
        hint_lines.push(format!(
            "- type `auto` to let runtime try configured preferred fallbacks first: {preferred_fallback_summary}",
        ));
    }

    render_onboard_input_screen(
        width,
        "choose model",
        GuidedOnboardStep::Model,
        guided_prompt_path,
        context_lines,
        hint_lines,
        color_enabled,
    )
}

pub fn render_api_key_env_selection_screen_lines(
    config: &mvp::config::LoongConfig,
    default_api_key_env: &str,
    width: usize,
) -> Vec<String> {
    render_api_key_env_selection_screen_lines_with_style(
        config,
        default_api_key_env,
        default_api_key_env,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
    )
}

pub fn render_api_key_env_selection_screen_lines_with_default(
    config: &mvp::config::LoongConfig,
    default_api_key_env: &str,
    prompt_default: &str,
    width: usize,
) -> Vec<String> {
    render_api_key_env_selection_screen_lines_with_style(
        config,
        default_api_key_env,
        prompt_default,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
    )
}

pub(super) fn render_api_key_env_selection_screen_lines_with_style(
    config: &mvp::config::LoongConfig,
    default_api_key_env: &str,
    prompt_default: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let mut context_lines = vec![format!(
        "- provider: {}",
        crate::provider::presentation::guided_provider_label(config.provider.kind)
    )];
    if let Some(current_env) =
        provider_credential_policy::render_configured_provider_credential_source_value(
            &config.provider,
        )
    {
        context_lines.push(format!("- current source: {current_env}"));
    }
    if let Some(suggested_source) =
        provider_credential_policy::render_provider_credential_source_value(Some(
            default_api_key_env,
        ))
    {
        context_lines.push(format!("- suggested source: {suggested_source}"));
    }

    let example_env_name =
        provider_credential_policy::provider_credential_env_hint(&config.provider)
            .unwrap_or_else(|| "PROVIDER_API_KEY".to_owned());
    let mut hint_lines = vec![render_api_key_env_selection_default_hint_line(
        config,
        default_api_key_env,
        prompt_default,
    )];
    hint_lines.push("- enter an env var name, not the secret value itself".to_owned());
    hint_lines.push(format!("- example: {example_env_name}"));
    if prompt_default.trim().is_empty() {
        if provider_credential_policy::provider_has_inline_credential(&config.provider) {
            hint_lines.push("- leave this blank to keep inline credentials".to_owned());
        }
    } else if provider_supports_blank_api_key_env(config) {
        hint_lines.push(render_clear_input_hint_line(
            "clear the configured credential env",
        ));
    }

    render_onboard_input_screen(
        width,
        "choose credential source",
        GuidedOnboardStep::CredentialEnv,
        guided_prompt_path,
        context_lines,
        hint_lines,
        color_enabled,
    )
}

pub(super) fn render_web_search_credential_selection_screen_lines_with_style(
    config: &mvp::config::LoongConfig,
    provider: &str,
    prompt_default: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let provider_label = query_search_provider_display_name(provider);
    let mut context_lines = vec![format!("- provider: {provider_label}")];
    if let Some(current_value) = configured_query_search_credential_source_value(config, provider) {
        let label = if current_value == "inline api key" {
            "- current credential: "
        } else {
            "- current source: "
        };
        context_lines.extend(mvp::presentation::render_wrapped_text_line(
            label,
            &current_value,
            width,
        ));
    }
    if let Some(suggested_env) = mvp::config::web_search_provider_descriptor(provider)
        .and_then(|descriptor| descriptor.default_api_key_env)
        .and_then(|env_name| {
            provider_credential_policy::render_provider_credential_source_value(Some(env_name))
        })
    {
        context_lines.extend(mvp::presentation::render_wrapped_text_line(
            "- suggested source: ",
            &suggested_env,
            width,
        ));
    }

    let mut hint_lines = vec![render_web_search_credential_selection_default_hint_line(
        config,
        provider,
        prompt_default,
    )];
    hint_lines.push("- enter an env var name, not the secret value itself".to_owned());
    let example_env_name = mvp::config::web_search_provider_descriptor(provider)
        .and_then(|descriptor| {
            descriptor
                .default_api_key_env
                .or_else(|| descriptor.api_key_env_names.first().copied())
        })
        .unwrap_or("WEB_SEARCH_API_KEY");
    hint_lines.push(format!("- example: {example_env_name}"));
    if prompt_default.trim().is_empty() && query_search_has_inline_credential(config, provider) {
        hint_lines.push("- leave this blank to keep inline credentials".to_owned());
    }
    if configured_query_search_secret(config, provider)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        hint_lines.push(render_clear_input_hint_line(
            crate::access_terms::query_search_credential_clear_hint(),
        ));
    }

    render_onboard_input_screen(
        width,
        crate::access_terms::CHOOSE_QUERY_SEARCH_CREDENTIAL_TITLE,
        GuidedOnboardStep::WebSearchProvider,
        guided_prompt_path,
        context_lines,
        hint_lines,
        color_enabled,
    )
}

pub fn render_system_prompt_selection_screen_lines(
    config: &mvp::config::LoongConfig,
    width: usize,
) -> Vec<String> {
    render_system_prompt_selection_screen_lines_with_style(
        config,
        config.cli.system_prompt.as_str(),
        GuidedPromptPath::InlineOverride,
        width,
        false,
    )
}

pub fn render_system_prompt_selection_screen_lines_with_default(
    config: &mvp::config::LoongConfig,
    prompt_default: &str,
    width: usize,
) -> Vec<String> {
    render_system_prompt_selection_screen_lines_with_style(
        config,
        prompt_default,
        GuidedPromptPath::InlineOverride,
        width,
        false,
    )
}

pub(super) fn render_system_prompt_selection_screen_lines_with_style(
    config: &mvp::config::LoongConfig,
    prompt_default: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let current_prompt = config.cli.system_prompt.trim();
    let current_prompt_display = if current_prompt.is_empty() {
        "built-in default".to_owned()
    } else {
        current_prompt.to_owned()
    };

    render_onboard_input_screen(
        width,
        "adjust cli behavior",
        GuidedOnboardStep::PromptCustomization,
        guided_prompt_path,
        vec![format!("- current prompt: {current_prompt_display}")],
        vec![
            render_system_prompt_selection_default_hint_line(config, prompt_default),
            if prompt_default.trim().is_empty() {
                "- leave this blank to use the built-in behavior".to_owned()
            } else {
                render_clear_input_hint_line("use the built-in behavior")
            },
            ONBOARD_SINGLE_LINE_INPUT_HINT.to_owned(),
        ],
        color_enabled,
    )
}
