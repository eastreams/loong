use super::*;
fn default_entry_config_path_override() -> Option<PathBuf> {
    std::env::var_os("LOONG_CONFIG_PATH")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

pub fn resolved_default_entry_config_path() -> PathBuf {
    default_entry_config_path_override().unwrap_or_else(mvp::config::default_config_path)
}

fn default_onboard_command() -> Commands {
    Commands::Onboard {
        output: None,
        force: false,
        non_interactive: false,
        accept_risk: false,
        provider: None,
        model: None,
        api_key_env: None,
        web_search_provider: None,
        web_search_api_key_env: None,
        personality: None,
        memory_profile: None,
        system_prompt: None,
        skip_model_probe: false,
    }
}

fn default_chat_command() -> Commands {
    Commands::Chat {
        config: None,
        session: None,
        acp: false,
        acp_event_stream: false,
        acp_bootstrap_mcp_server: Vec::new(),
        acp_cwd: None,
    }
}

pub(crate) const fn should_resolve_default_entry_to_chat(
    config_exists: bool,
    config_path_is_directory: bool,
    interactive_terminal: bool,
) -> bool {
    config_exists || (!config_path_is_directory && interactive_terminal)
}

pub fn resolve_default_entry_command() -> Commands {
    let config_path = resolved_default_entry_config_path();
    let config_exists = config_path.is_file();
    let config_path_is_directory = config_path.is_dir();
    let interactive_terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if should_resolve_default_entry_to_chat(
        config_exists,
        config_path_is_directory,
        interactive_terminal,
    ) {
        default_chat_command()
    } else {
        default_onboard_command()
    }
}

pub fn resolve_default_entry_post_onboard_command() -> Option<Commands> {
    resolved_default_entry_config_path()
        .is_file()
        .then(default_chat_command)
}

pub fn redacted_command_name(command: &Commands) -> &'static str {
    command.command_kind_for_logging()
}

pub(crate) fn resolve_welcome_config_path() -> CliResult<PathBuf> {
    let config_path = resolved_default_entry_config_path();
    if config_path.is_file() {
        Ok(config_path)
    } else {
        Err(format!(
            "Config file not found at {}. Run `{} onboard` to set up Loong.",
            config_path.display(),
            active_cli_command_name(),
        ))
    }
}

pub fn render_welcome_banner(config_path: &Path, config: &mvp::config::LoongConfig) -> String {
    let config_path_display = config_path.display().to_string();
    let next_actions = next_actions::collect_setup_next_actions(config, &config_path_display);
    let render_width = mvp::presentation::detect_render_width();
    let mut sections = build_first_run_action_sections(
        &next_actions,
        |action| first_run_group_for_setup_action_kind(action.kind),
        |action| mvp::tui_surface::TuiActionSpec {
            label: action.label.clone(),
            command: action.command.clone(),
        },
    );

    sections.push(mvp::tui_surface::TuiSectionSpec::KeyValues {
        title: Some("saved setup".to_owned()),
        items: vec![
            mvp::tui_surface::TuiKeyValueSpec::Plain {
                key: "config".to_owned(),
                value: config_path_display,
            },
            mvp::tui_surface::TuiKeyValueSpec::Plain {
                key: "provider".to_owned(),
                value: crate::provider_presentation::active_provider_detail_label(config),
            },
            mvp::tui_surface::TuiKeyValueSpec::Plain {
                key: "model".to_owned(),
                value: config.provider.model.clone(),
            },
            mvp::tui_surface::TuiKeyValueSpec::Plain {
                key: "memory profile".to_owned(),
                value: config.memory.profile.as_str().to_owned(),
            },
        ],
    });
    sections.push(mvp::tui_surface::TuiSectionSpec::Callout {
        tone: mvp::tui_surface::TuiCalloutTone::Info,
        title: Some("operator flow".to_owned()),
        lines: vec![
            "Start with a first answer, then continue in chat for follow-up work.".to_owned(),
            "Use doctor when setup or runtime health feels off instead of debugging the config by hand.".to_owned(),
        ],
    });

    let screen = mvp::tui_surface::TuiScreenSpec {
        header_style: mvp::tui_surface::TuiHeaderStyle::Compact,
        subtitle: Some("configured install".to_owned()),
        title: Some("welcome back".to_owned()),
        progress_line: None,
        intro_lines: vec!["Loong is configured and ready.".to_owned()],
        sections,
        choices: Vec::new(),
        footer_lines: vec![format!(
            "Use {} --help to browse the full operator surface.",
            CLI_COMMAND_NAME
        )],
    };

    mvp::tui_surface::render_tui_screen_spec_ratatui(&screen, render_width, false).join("\n")
}

pub fn run_welcome_cli() -> CliResult<()> {
    let config_path = resolve_welcome_config_path()?;
    let config_path_string = config_path.display().to_string();
    let load_result = mvp::config::load(Some(config_path_string.as_str()))?;
    let (_resolved_path, config) = load_result;
    println!("{}", render_welcome_banner(config_path.as_path(), &config));
    Ok(())
}
