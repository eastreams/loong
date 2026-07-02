use super::*;

pub fn build_channel_onboarding_follow_up_lines(config: &mvp::config::LoongConfig) -> Vec<String> {
    let inventory = mvp::channel::channel_inventory(config);
    let mut lines = Vec::with_capacity(inventory.channel_surfaces.len() + 1);
    lines.push("channel next steps:".to_owned());

    for surface in inventory.channel_surfaces {
        let aliases = if surface.catalog.aliases.is_empty() {
            "-".to_owned()
        } else {
            surface.catalog.aliases.join(",")
        };
        let repair_command = surface
            .catalog
            .onboarding
            .repair_command
            .map(|command| format!("\"{command}\""))
            .unwrap_or_else(|| "-".to_owned());
        lines.push(format!(
            "- {} [{}] selection_order={} selection_label=\"{}\" strategy={} aliases={} status_command=\"{}\" repair_command={} setup_hint=\"{}\" blurb=\"{}\"",
            surface.catalog.label,
            surface.catalog.id,
            surface.catalog.selection_order,
            surface.catalog.selection_label,
            surface.catalog.onboarding.strategy.as_str(),
            aliases,
            surface.catalog.onboarding.status_command,
            repair_command,
            surface.catalog.onboarding.setup_hint,
            surface.catalog.blurb,
        ));
    }

    lines
}

fn normalize_onboard_credential_env_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    if !onboard_credential_env_name_is_safe(trimmed) {
        return None;
    }

    Some(trimmed.to_owned())
}

pub(super) fn validate_selected_web_search_credential_env(
    provider: &str,
    selected_env_name: &str,
) -> CliResult<String> {
    let trimmed = selected_env_name.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }

    if let Some(normalized) = normalize_onboard_credential_env_name(trimmed) {
        return Ok(normalized);
    }

    let example_env_name = mvp::config::web_search_provider_descriptor(provider)
        .and_then(|descriptor| {
            descriptor
                .default_api_key_env
                .or_else(|| descriptor.api_key_env_names.first().copied())
        })
        .unwrap_or("WEB_SEARCH_API_KEY");

    Err(crate::access_terms::query_search_credential_source_validation_error(example_env_name))
}

pub(super) fn apply_selected_web_search_credential(
    config: &mut mvp::config::LoongConfig,
    provider: &str,
    selection: WebSearchCredentialSelection,
) -> CliResult<()> {
    let next_value = match selection {
        WebSearchCredentialSelection::KeepCurrent => return Ok(()),
        WebSearchCredentialSelection::ClearConfigured => None,
        WebSearchCredentialSelection::UseEnv(env_name) => Some(format!("${{{}}}", env_name.trim())),
    };

    let updated = config
        .tools
        .web_search
        .set_configured_api_key_for_provider(provider, next_value);

    if !updated {
        return Err(format!(
            "unsupported web.search provider `{provider}`; credential update was skipped"
        ));
    }

    Ok(())
}

pub(super) fn validate_selected_provider_credential_env(
    config: &mvp::config::LoongConfig,
    selected_env_name: &str,
) -> CliResult<String> {
    let trimmed = selected_env_name.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }

    let mut candidate = config.clone();
    apply_selected_api_key_env(&mut candidate.provider, trimmed.to_owned());
    candidate.validate().map(|_| trimmed.to_owned())
}

pub(super) fn non_interactive_preflight_warning_message(
    checks: &[OnboardCheck],
    options: &OnboardCommandOptions,
) -> String {
    let blocking_warning = checks.iter().find(|check| {
        check.level == OnboardCheckLevel::Warn
            && !is_explicitly_accepted_non_interactive_warning(check, options)
    });

    let detail = blocking_warning
        .map(|check| format!("{}: {}", check.name, check.detail))
        .unwrap_or_else(|| "unresolved warnings require interactive review".to_owned());

    format!(
        "onboard preflight failed: {detail}; rerun without --non-interactive to inspect and confirm them"
    )
}

pub fn preferred_api_key_env_default(config: &mvp::config::LoongConfig) -> String {
    provider_credential_policy::preferred_provider_credential_env_name(config)
}

pub fn collect_import_surfaces(config: &mvp::config::LoongConfig) -> Vec<ImportSurface> {
    crate::migration::collect_import_surfaces(config)
        .into_iter()
        .map(import_surface_from_migration)
        .collect()
}

pub fn collect_import_surfaces_with_channel_readiness(
    config: &mvp::config::LoongConfig,
    readiness: ChannelImportReadiness,
) -> Vec<ImportSurface> {
    crate::migration::collect_import_surfaces_with_channel_readiness(
        config,
        &to_migration_readiness(readiness),
    )
    .into_iter()
    .map(import_surface_from_migration)
    .collect()
}

pub(super) fn secret_ref_has_inline_literal(secret_ref: Option<&SecretRef>) -> bool {
    let Some(secret_ref) = secret_ref else {
        return false;
    };

    secret_ref.inline_literal_value().is_some()
}

pub(super) fn onboard_has_explicit_overrides(options: &OnboardCommandOptions) -> bool {
    option_has_non_empty_value(options.provider.as_deref())
        || option_has_non_empty_value(options.model.as_deref())
        || option_has_non_empty_value(options.api_key_env.as_deref())
        || option_has_non_empty_value(options.web_search_provider.as_deref())
        || option_has_non_empty_value(options.web_search_api_key_env.as_deref())
        || option_has_non_empty_value(options.personality.as_deref())
        || option_has_non_empty_value(options.memory_profile.as_deref())
        || option_has_non_empty_value(options.system_prompt.as_deref())
        || option_has_non_empty_value(env::var("LOONG_WEB_SEARCH_PROVIDER").ok().as_deref())
}

fn option_has_non_empty_value(raw: Option<&str>) -> bool {
    raw.is_some_and(|value| !value.trim().is_empty())
}

pub(super) fn load_existing_output_config(output_path: &Path) -> Option<mvp::config::LoongConfig> {
    let path_str = output_path.to_str()?;
    mvp::config::load(Some(path_str))
        .ok()
        .map(|(_, config)| config)
}

pub fn should_skip_config_write(
    existing_config: Option<&mvp::config::LoongConfig>,
    draft: &mvp::config::LoongConfig,
) -> bool {
    existing_config.is_some_and(|existing| {
        if existing == draft {
            return true;
        }

        let existing_rendered = mvp::config::render(existing).ok();
        let draft_rendered = mvp::config::render(draft).ok();
        existing_rendered.is_some() && existing_rendered == draft_rendered
    })
}

pub fn parse_provider_kind(raw: &str) -> Option<mvp::config::ProviderKind> {
    mvp::config::ProviderKind::parse(raw)
}

pub fn parse_prompt_personality(raw: &str) -> Option<mvp::prompt::PromptPersonality> {
    mvp::prompt::parse_prompt_personality(raw)
}

pub fn parse_memory_profile(raw: &str) -> Option<mvp::config::MemoryProfile> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "window_only" | "window" => Some(mvp::config::MemoryProfile::WindowOnly),
        "window_plus_summary" | "summary" | "summary_window" => {
            Some(mvp::config::MemoryProfile::WindowPlusSummary)
        }
        "profile_plus_window" | "profile" | "profile_window" => {
            Some(mvp::config::MemoryProfile::ProfilePlusWindow)
        }
        _ => None,
    }
}

pub fn provider_default_api_key_env(kind: mvp::config::ProviderKind) -> Option<&'static str> {
    kind.default_api_key_env()
}

pub fn provider_kind_id(kind: mvp::config::ProviderKind) -> &'static str {
    kind.as_str()
}

pub fn provider_kind_display_name(kind: mvp::config::ProviderKind) -> &'static str {
    kind.display_name()
}

pub fn prompt_personality_id(personality: mvp::prompt::PromptPersonality) -> &'static str {
    personality.id()
}

pub fn memory_profile_id(profile: mvp::config::MemoryProfile) -> &'static str {
    match profile {
        mvp::config::MemoryProfile::WindowOnly => "window_only",
        mvp::config::MemoryProfile::WindowPlusSummary => "window_plus_summary",
        mvp::config::MemoryProfile::ProfilePlusWindow => "profile_plus_window",
    }
}

pub fn supported_provider_list() -> String {
    mvp::config::ProviderKind::all_sorted()
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn supported_personality_list() -> String {
    mvp::prompt::supported_prompt_personality_list()
}

pub fn supported_memory_profile_list() -> &'static str {
    "window_only, window_plus_summary, profile_plus_window"
}

pub(super) fn resolve_write_plan(
    output_path: &Path,
    options: &OnboardCommandOptions,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<ConfigWritePlan> {
    if !output_path.exists() {
        return Ok(ConfigWritePlan {
            force: false,
            backup_path: None,
        });
    }
    if options.force {
        return Ok(ConfigWritePlan {
            force: true,
            backup_path: None,
        });
    }

    if options.non_interactive {
        return Err(format!(
            "config {} already exists (use --force to overwrite)",
            output_path.display()
        ));
    }

    let existing_path = output_path.display().to_string();
    print_lines(
        ui,
        render_existing_config_write_header_lines_with_style(
            &existing_path,
            context.render_width,
            true,
        ),
    )?;
    let options = build_existing_config_write_screen_options();
    let selected = options
        .get(select_screen_option(
            ui,
            "Your choice",
            &options,
            Some("b"),
        )?)
        .ok_or_else(|| "existing-config write selection out of range".to_owned())?;
    match selected.key.as_str() {
        "o" => Ok(ConfigWritePlan {
            force: true,
            backup_path: None,
        }),
        "b" => Ok(ConfigWritePlan {
            force: true,
            backup_path: Some(resolve_backup_path(output_path)?),
        }),
        "c" => Err("onboarding cancelled: config file already exists".to_owned()),
        key => Err(format!(
            "unexpected existing-config write selection key: {key}"
        )),
    }
}
