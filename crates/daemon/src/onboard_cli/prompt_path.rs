use super::*;

fn resolve_guided_system_prompt_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongConfig,
    guided_prompt_path: GuidedPromptPath,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<Option<String>> {
    match guided_prompt_path {
        GuidedPromptPath::NativePromptPack => Ok(None),
        GuidedPromptPath::InlineOverride => {
            if options.non_interactive {
                return Ok(options.system_prompt.clone().map(|system_prompt| {
                    if is_explicit_onboard_clear_input(system_prompt.as_str()) {
                        mvp::config::CliChannelConfig::default().system_prompt
                    } else {
                        system_prompt
                    }
                }));
            }

            let prompt_default = options
                .system_prompt
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    if config.cli.uses_native_prompt_pack() {
                        String::new()
                    } else {
                        config.cli.system_prompt.clone()
                    }
                });
            print_lines(
                ui,
                render_system_prompt_selection_screen_lines_with_style(
                    config,
                    prompt_default.as_str(),
                    guided_prompt_path,
                    context.render_width,
                    true,
                ),
            )?;
            let value = ui.prompt_with_default("System prompt", prompt_default.as_str())?;
            if is_explicit_onboard_clear_input(&value) {
                return Ok(Some(mvp::config::CliChannelConfig::default().system_prompt));
            }
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_owned()))
            }
        }
    }
}

pub(super) fn apply_guided_prompt_configuration(
    config: &mut mvp::config::LoongConfig,
    prompt_pack_id: Option<String>,
    personality: Option<mvp::prompt::PromptPersonality>,
    selected_system_prompt: Option<String>,
) {
    config.cli.prompt_pack_id = prompt_pack_id;
    config.cli.personality = personality;
    apply_selected_system_prompt(config, selected_system_prompt);
}

pub(super) fn resolve_guided_prompt_path(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongConfig,
) -> GuidedPromptPath {
    if options.system_prompt.is_some() {
        return GuidedPromptPath::InlineOverride;
    }
    if options
        .personality
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return GuidedPromptPath::NativePromptPack;
    }
    if options.non_interactive {
        if config.cli.uses_native_prompt_pack() {
            return GuidedPromptPath::NativePromptPack;
        }
        if !config.cli.system_prompt.trim().is_empty() {
            return GuidedPromptPath::InlineOverride;
        }
    }
    GuidedPromptPath::NativePromptPack
}

pub fn resolve_guided_prompt_path_label_for_test(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongConfig,
) -> &'static str {
    match resolve_guided_prompt_path(options, config) {
        GuidedPromptPath::NativePromptPack => "native",
        GuidedPromptPath::InlineOverride => "inline",
    }
}

pub(super) fn apply_selected_system_prompt(
    config: &mut mvp::config::LoongConfig,
    system_prompt: Option<String>,
) {
    match system_prompt.as_deref().map(str::trim) {
        Some(value) if !value.is_empty() => {
            config.cli.prompt_pack_id = Some(String::new());
            config.cli.personality = None;
            config.cli.system_prompt_addendum = None;
            config.cli.system_prompt = value.to_owned();
        }
        _ => config.cli.refresh_native_system_prompt(),
    }
}

pub(super) async fn collect_guided_prompt_settings(
    options: &OnboardCommandOptions,
    draft_config: &mvp::config::LoongConfig,
    guided_prompt_path: GuidedPromptPath,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<(
    Option<String>,
    Option<mvp::prompt::PromptPersonality>,
    Option<String>,
)> {
    let selected_system_prompt = resolve_guided_system_prompt_selection(
        options,
        draft_config,
        guided_prompt_path,
        ui,
        context,
    )?;
    let prompt_pack_id = if guided_prompt_path == GuidedPromptPath::NativePromptPack {
        Some(mvp::prompt::DEFAULT_PROMPT_PACK_ID.to_owned())
    } else {
        None
    };
    let personality =
        if guided_prompt_path == GuidedPromptPath::NativePromptPack && options.non_interactive {
            options
                .personality
                .as_deref()
                .map(|personality_raw| {
                    parse_prompt_personality(personality_raw).ok_or_else(|| {
                        format!(
                            "unsupported --personality value \"{personality_raw}\". supported: {}",
                            supported_personality_list()
                        )
                    })
                })
                .transpose()?
        } else {
            draft_config.cli.personality
        };
    Ok((prompt_pack_id, personality, selected_system_prompt))
}
