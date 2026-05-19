fn split_surface_command(input: &str) -> (&str, &str) {
    let trimmed = input.trim();
    if let Some((command, rest)) = trimmed.split_once(char::is_whitespace) {
        (command, rest.trim())
    } else {
        (trimmed, "")
    }
}

fn is_known_surface_command(command: &str) -> bool {
    match command {
        super::super::CLI_CHAT_HELP_COMMAND
        | super::super::CLI_CHAT_STATUS_COMMAND
        | super::super::CLI_CHAT_HISTORY_COMMAND
        | super::super::CLI_CHAT_COMPACT_COMMAND
        | "/model"
        | "/settings"
        | "/permissions"
        | "/experimental"
        | "/themes"
        | "/cwd"
        | "/language"
        | "/mcp"
        | "/skills"
        | "/usage"
        | "/sessions"
        | "/subagents"
        | "/missions"
        | "/mission"
        | "/clear"
        | "/new"
        | "/copy"
        | "/diff"
        | "/export"
        | "/share"
        | "/import"
        | "/simplify"
        | "/plan"
        | "/title"
        | "/rename"
        | "/feedback"
        | "/exit" => true,
        _ => slash_command_specs()
            .iter()
            .any(|spec| spec.command == command),
    }
}

fn recognized_surface_command(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if !(trimmed.starts_with('/') || trimmed.starts_with(':')) {
        return None;
    }
    let normalized = if trimmed.starts_with(':') {
        format!("/{}", trimmed.trim_start_matches(':'))
    } else {
        trimmed.to_owned()
    };
    let (command, _) = split_surface_command(normalized.as_str());
    is_known_surface_command(command).then_some(normalized)
}

fn parse_settings_command_action(args: &str) -> Result<CommandAction, String> {
    let tokens = args.split_whitespace().collect::<Vec<_>>();
    match tokens.as_slice() {
        [] => Ok(CommandAction::OpenSettings(SettingsSurfaceFocus::Overview)),
        ["provider"] | ["web"] => Ok(CommandAction::OpenSettings(SettingsSurfaceFocus::Provider)),
        ["workspace"] => Ok(CommandAction::OpenSettings(SettingsSurfaceFocus::Workspace)),
        ["provider", raw_kind] => crate::config::parse_provider_kind_id(raw_kind)
            .map(|kind| CommandAction::ApplySettings(SettingsCommandAction::SetProvider(kind)))
            .ok_or_else(|| format!("unknown provider `{raw_kind}`; use `/settings` to inspect the current setup")),
        ["web", raw_provider] => normalize_web_search_provider(raw_provider)
            .map(|provider| {
                CommandAction::ApplySettings(SettingsCommandAction::SetWebProvider(
                    provider.to_owned(),
                ))
            })
            .ok_or_else(|| format!("unknown web.search provider `{raw_provider}`")),
        ["skills", "install", target_id] => {
            Ok(CommandAction::ApplySettings(
                SettingsCommandAction::InstallSkillPack((*target_id).to_owned()),
            ))
        }
        ["skills", "remove", target_id] | ["skills", "uninstall", target_id] => {
            Ok(CommandAction::ApplySettings(
                SettingsCommandAction::RemoveSkillPack((*target_id).to_owned()),
            ))
        }
        _ => Err(
            "usage: /settings [provider [id] | web [provider] | skills [install|remove <target>] | workspace]"
                .to_owned(),
        ),
    }
}

fn apply_settings_command(
    app: &mut App,
    runtime: &mut CliTurnRuntime,
    action: SettingsCommandAction,
) -> CliResult<(SettingsSurfaceFocus, String, String)> {
    match action {
        SettingsCommandAction::SetProvider(kind) => {
            let summary = persist_runtime_settings(runtime, app, |config| {
                config.provider = startup_provider_config_for_kind(kind);
                Ok(format!("provider switched to {}", kind.display_name()))
            })?;
            Ok((
                SettingsSurfaceFocus::Provider,
                summary,
                kind.display_name().to_owned(),
            ))
        }
        SettingsCommandAction::SetWebProvider(provider) => {
            let provider_for_summary = provider.clone();
            let summary = persist_runtime_settings(runtime, app, |config| {
                config.tools.web_search.enabled = true;
                config.tools.web_search.default_provider = provider.clone();
                if let Some(env_name) = web_search_provider_api_key_env_names(provider.as_str())
                    .iter()
                    .find(|env_name| std::env::var_os(env_name).is_some())
                {
                    let _ = config.tools.web_search.set_configured_api_key_for_provider(
                        provider.as_str(),
                        Some(format!("${{{}}}", env_name)),
                    );
                    Ok(format!(
                        "web-search provider switched to {} using {}",
                        provider_for_summary, env_name
                    ))
                } else {
                    Ok(format!(
                        "web-search provider switched to {}; credentials still need wiring",
                        provider_for_summary
                    ))
                }
            })?;
            let label = web_search_provider_descriptor(provider.as_str())
                .map(|descriptor| descriptor.display_name.to_owned())
                .unwrap_or(provider);
            Ok((SettingsSurfaceFocus::Provider, summary, label))
        }
        SettingsCommandAction::InstallSkillPack(target_id) => {
            let resolved_path = runtime.resolved_path.clone();
            let summary = persist_runtime_settings(runtime, app, |config| {
                config.skills.enabled = true;
                config.skills.auto_expose_installed = true;
                if config.skills.install_root.is_none() {
                    let install_root = resolved_path
                        .parent()
                        .filter(|parent| !parent.as_os_str().is_empty())
                        .map(|parent| parent.join(".loong/skills"))
                        .unwrap_or_else(|| PathBuf::from(".loong/skills"));
                    config.skills.install_root = Some(install_root.display().to_string());
                }
                let runtime_config =
                    crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(
                        config,
                        Some(resolved_path.as_path()),
                    );
                let mut selected = BTreeSet::new();
                selected.insert(target_id.clone());
                let installed = crate::tools::install_bundled_preinstall_targets_for_bootstrap(
                    &runtime_config,
                    &selected,
                )?;
                if installed.is_empty() {
                    Ok(format!(
                        "skill pack `{target_id}` was already present in the managed runtime"
                    ))
                } else {
                    Ok(format!(
                        "installed managed skill pack `{target_id}`: {}",
                        installed.join(", ")
                    ))
                }
            })?;
            let label = bundled_preinstall_targets()
                .iter()
                .find(|target| target.install_id == target_id.as_str())
                .map(|target| target.display_name.to_owned())
                .unwrap_or(target_id);
            Ok((SettingsSurfaceFocus::Workspace, summary, label))
        }
        SettingsCommandAction::RemoveSkillPack(target_id) => {
            let resolved_path = runtime.resolved_path.clone();
            let summary = persist_runtime_settings(runtime, app, |config| {
                let runtime_config =
                    crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(
                        config,
                        Some(resolved_path.as_path()),
                    );
                let mut selected = BTreeSet::new();
                selected.insert(target_id.clone());
                let removed = crate::tools::remove_bundled_preinstall_targets_for_bootstrap(
                    &runtime_config,
                    &selected,
                )?;
                if removed.is_empty() {
                    Ok(format!(
                        "skill pack `{target_id}` was already absent from the managed runtime"
                    ))
                } else {
                    Ok(format!(
                        "removed managed skill pack `{target_id}`: {}",
                        removed.join(", ")
                    ))
                }
            })?;
            let label = bundled_preinstall_targets()
                .iter()
                .find(|target| target.install_id == target_id.as_str())
                .map(|target| target.display_name.to_owned())
                .unwrap_or(target_id);
            Ok((SettingsSurfaceFocus::Workspace, summary, label))
        }
    }
}

fn dispatch_palette_action(
    app: &mut App,
    runtime: &mut CliTurnRuntime,
    width: usize,
    action: CommandAction,
) -> CliResult<Option<String>> {
    let should_clear_slash_buffer = app.command_palette.is_commands_mode();
    match action {
        CommandAction::RunCommand(command) => {
            if should_clear_slash_buffer {
                clear_slash_palette_composer(app);
            }
            app.inline_skill_popup_active = false;
            app.focus = Focus::Composer;
            Ok(Some(command.to_owned()))
        }
        CommandAction::OpenSettings(focus) => {
            if should_clear_slash_buffer {
                clear_slash_palette_composer(app);
            }
            open_settings_palette(app, runtime, focus, width, None, None);
            Ok(None)
        }
        CommandAction::ApplySettings(action) => {
            if should_clear_slash_buffer {
                clear_slash_palette_composer(app);
            }
            let (focus, summary, selected_label) = apply_settings_command(app, runtime, action)?;
            open_settings_palette(
                app,
                runtime,
                focus,
                width,
                Some(summary),
                Some(selected_label.as_str()),
            );
            Ok(None)
        }
        CommandAction::OpenModelReasoning(entry) => {
            if should_clear_slash_buffer {
                clear_slash_palette_composer(app);
            }
            open_reasoning_palette(app, runtime, &entry);
            Ok(None)
        }
        CommandAction::ApplyModelSelection {
            model,
            reasoning_effort,
        } => {
            if should_clear_slash_buffer {
                clear_slash_palette_composer(app);
            }
            apply_model_selection(app, runtime, model, reasoning_effort)?;
            Ok(None)
        }
        CommandAction::Noop => Ok(None),
        CommandAction::InsertText(text) => {
            if let Some(range) = current_skill_token_range(&app.composer) {
                let replacement =
                    inline_skill_replacement_text(app.composer.text(), &range, text.as_str());
                app.composer.replace_range(range, replacement.as_str());
            } else {
                app.composer.set_input(text);
            }
            app.inline_skill_popup_active = false;
            app.focus = Focus::Composer;
            Ok(None)
        }
        CommandAction::Close => {
            if should_clear_slash_buffer {
                clear_slash_palette_composer(app);
            }
            app.inline_skill_popup_active = false;
            app.focus = Focus::Composer;
            Ok(None)
        }
    }
}

fn open_settings_palette(
    app: &mut App,
    runtime: &CliTurnRuntime,
    focus: SettingsSurfaceFocus,
    width: usize,
    status: Option<String>,
    selected_label: Option<&str>,
) {
    let entries = build_settings_palette_entries(runtime, focus, width);
    app.command_palette
        .show_settings(focus, entries, status, selected_label);
    app.focus = Focus::CommandPalette;
    app.inline_skill_popup_active = false;
}

fn build_settings_palette_entries(
    runtime: &CliTurnRuntime,
    focus: SettingsSurfaceFocus,
    width: usize,
) -> Vec<SettingsEntry> {
    let runtime_config = crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(
        &runtime.config,
        Some(runtime.resolved_path.as_path()),
    );
    let installed_skill_ids =
        crate::tools::installed_managed_skill_ids_for_bootstrap(&runtime_config)
            .unwrap_or_default();
    if focus == SettingsSurfaceFocus::Overview {
        return build_settings_overview_entries(runtime, width, &installed_skill_ids);
    }

    let provider_focus = focus == SettingsSurfaceFocus::Provider;
    let workspace_focus = focus == SettingsSurfaceFocus::Workspace;
    let mut entries = Vec::new();

    if provider_focus {
        let current_auth = runtime
            .config
            .provider
            .resolved_auth_env_name()
            .unwrap_or_else(|| "still needs credentials".to_owned());
        entries.push(SettingsEntry {
            label: "Current provider".to_owned(),
            category_tag: "[State]".to_owned(),
            status_tag: Some("state".to_owned()),
            description: format!(
                "{} · model {} · auth {}",
                runtime.config.provider.kind.display_name(),
                runtime.config.provider.model,
                current_auth
            ),
            action: CommandAction::Noop,
            selectable: false,
        });
        entries.push(SettingsEntry {
            label: "Back to settings".to_owned(),
            category_tag: "[Navigation]".to_owned(),
            status_tag: None,
            description: "return to the top-level settings overview".to_owned(),
            action: CommandAction::OpenSettings(SettingsSurfaceFocus::Overview),
            selectable: true,
        });
        let current_provider = runtime.config.provider.kind;
        let mut provider_kinds = ProviderKind::all_sorted().to_vec();
        provider_kinds.sort_by_key(|kind| {
            let is_current = *kind == current_provider;
            let is_ready = detected_startup_auth_binding(*kind).is_some();
            (
                usize::from(!is_current),
                usize::from(!is_ready && !is_current),
                kind.display_name(),
            )
        });
        for kind in provider_kinds {
            let is_current = runtime.config.provider.kind == kind;
            let (status_tag, description) =
                render_provider_settings_entry(runtime, kind, is_current);
            entries.push(SettingsEntry {
                label: kind.display_name().to_owned(),
                category_tag: "[Provider]".to_owned(),
                status_tag,
                description,
                action: CommandAction::ApplySettings(SettingsCommandAction::SetProvider(kind)),
                selectable: true,
            });
        }
        let current_web_provider = normalize_web_search_provider(
            runtime.config.tools.web_search.default_provider.as_str(),
        )
        .unwrap_or(runtime.config.tools.web_search.default_provider.as_str());
        let mut web_descriptors = crate::config::web_search_provider_descriptors().to_vec();
        web_descriptors.sort_by_key(|descriptor| {
            let is_current = descriptor.id == current_web_provider;
            let is_ready = web_search_provider_env_api_key_name(descriptor.id).is_some()
                || runtime
                    .config
                    .tools
                    .web_search
                    .configured_api_key_for_provider(descriptor.id)
                    .is_some();
            (
                usize::from(!is_current),
                usize::from(!is_ready && !is_current),
                descriptor.display_name,
            )
        });
        entries.push(SettingsEntry {
            label: "Current web search".to_owned(),
            category_tag: "[State]".to_owned(),
            status_tag: Some("state".to_owned()),
            description: render_current_web_search_summary(runtime),
            action: CommandAction::Noop,
            selectable: false,
        });
        for descriptor in web_descriptors {
            let is_current = runtime.config.tools.web_search.default_provider == descriptor.id;
            let (status_tag, description) = render_web_provider_settings_entry(
                runtime,
                descriptor.id,
                descriptor.display_name,
                is_current,
            );
            entries.push(SettingsEntry {
                label: descriptor.display_name.to_owned(),
                category_tag: "[Web]".to_owned(),
                status_tag,
                description,
                action: CommandAction::ApplySettings(SettingsCommandAction::SetWebProvider(
                    descriptor.id.to_owned(),
                )),
                selectable: true,
            });
        }
    }

    if workspace_focus {
        let installed_pack_count = bundled_preinstall_targets()
            .iter()
            .filter(|target| {
                target
                    .skill_ids
                    .iter()
                    .all(|skill_id| installed_skill_ids.contains(*skill_id))
            })
            .count();
        entries.push(SettingsEntry {
            label: "Current workspace".to_owned(),
            category_tag: "[State]".to_owned(),
            status_tag: Some("state".to_owned()),
            description: format!(
                "{} bootstrap MCP · {} installed packs",
                runtime.effective_bootstrap_mcp_servers.len(),
                installed_pack_count
            ),
            action: CommandAction::Noop,
            selectable: false,
        });
        entries.push(SettingsEntry {
            label: "Back to settings".to_owned(),
            category_tag: "[Navigation]".to_owned(),
            status_tag: None,
            description: "return to the top-level settings overview".to_owned(),
            action: CommandAction::OpenSettings(SettingsSurfaceFocus::Overview),
            selectable: true,
        });
        let mut targets = bundled_preinstall_targets().to_vec();
        targets.sort_by_key(|target| (usize::from(!target.recommended), target.display_name));
        for target in targets {
            let is_installed = target
                .skill_ids
                .iter()
                .all(|skill_id| installed_skill_ids.contains(*skill_id));
            entries.push(SettingsEntry {
                label: target.display_name.to_owned(),
                category_tag: "[Skill Pack]".to_owned(),
                status_tag: is_installed.then_some("installed".to_owned()),
                description: if is_installed {
                    format!("remove from the managed runtime · {}", target.summary)
                } else {
                    format!("install into the managed runtime · {}", target.summary)
                },
                action: if is_installed {
                    CommandAction::ApplySettings(SettingsCommandAction::RemoveSkillPack(
                        target.install_id.to_owned(),
                    ))
                } else {
                    CommandAction::ApplySettings(SettingsCommandAction::InstallSkillPack(
                        target.install_id.to_owned(),
                    ))
                },
                selectable: true,
            });
        }
    }

    if entries.is_empty() {
        entries.push(SettingsEntry {
            label: "settings".to_owned(),
            category_tag: String::new(),
            status_tag: None,
            description: "no adjustable settings available in this view".to_owned(),
            action: CommandAction::Close,
            selectable: false,
        });
    }

    let max_desc_width = width.saturating_sub(24).max(24);
    for entry in &mut entries {
        entry.description = truncate_right_for_width(entry.description.as_str(), max_desc_width);
    }

    entries
}

fn build_settings_overview_entries(
    runtime: &CliTurnRuntime,
    width: usize,
    installed_skill_ids: &BTreeSet<String>,
) -> Vec<SettingsEntry> {
    let provider_label = runtime.config.provider.kind.display_name();
    let web_provider =
        normalize_web_search_provider(runtime.config.tools.web_search.default_provider.as_str())
            .unwrap_or(runtime.config.tools.web_search.default_provider.as_str());
    let web_label = web_search_provider_descriptor(web_provider)
        .map(|descriptor| descriptor.display_name)
        .unwrap_or(web_provider);
    let mcp_count = runtime.effective_bootstrap_mcp_servers.len();
    let installed_pack_count = bundled_preinstall_targets()
        .iter()
        .filter(|target| {
            target
                .skill_ids
                .iter()
                .all(|skill_id| installed_skill_ids.contains(*skill_id))
        })
        .count();
    let skills_state = if runtime.config.skills.enabled {
        if installed_pack_count == 0 {
            "managed skills enabled"
        } else {
            "managed skills active"
        }
    } else {
        "managed skills disabled"
    };

    let mut entries = vec![
        SettingsEntry {
            label: "Provider & web".to_owned(),
            category_tag: "[Setup]".to_owned(),
            status_tag: None,
            description: format!("{provider_label} · {web_label}"),
            action: CommandAction::OpenSettings(SettingsSurfaceFocus::Provider),
            selectable: true,
        },
        SettingsEntry {
            label: "Workspace setup".to_owned(),
            category_tag: "[Setup]".to_owned(),
            status_tag: None,
            description: if installed_pack_count == 0 {
                format!("{mcp_count} bootstrap MCP · {skills_state}")
            } else {
                format!("{mcp_count} bootstrap MCP · {installed_pack_count} packs installed")
            },
            action: CommandAction::OpenSettings(SettingsSurfaceFocus::Workspace),
            selectable: true,
        },
        SettingsEntry {
            label: "Permissions".to_owned(),
            category_tag: "[Review]".to_owned(),
            status_tag: None,
            description: "inspect the current tool-permission posture".to_owned(),
            action: CommandAction::RunCommand("/permissions"),
            selectable: true,
        },
    ];

    let max_desc_width = width.saturating_sub(24).max(24);
    for entry in &mut entries {
        entry.description = truncate_right_for_width(entry.description.as_str(), max_desc_width);
    }
    entries
}

fn render_current_web_search_summary(runtime: &CliTurnRuntime) -> String {
    let provider_id =
        normalize_web_search_provider(runtime.config.tools.web_search.default_provider.as_str())
            .unwrap_or(runtime.config.tools.web_search.default_provider.as_str());
    let provider_label = web_search_provider_descriptor(provider_id)
        .map(|descriptor| descriptor.display_name)
        .unwrap_or(provider_id);
    let credential_state = runtime
        .config
        .tools
        .web_search
        .configured_api_key_for_provider(provider_id)
        .map(str::to_owned)
        .or_else(|| {
            let env_names = web_search_provider_api_key_env_names(provider_id);
            if env_names.is_empty() {
                None
            } else {
                Some(format!("expects {}", env_names.join(" or ")))
            }
        })
        .unwrap_or_else(|| "not required".to_owned());
    format!("{provider_label} · {credential_state}")
}

fn render_provider_settings_entry(
    runtime: &CliTurnRuntime,
    kind: ProviderKind,
    is_current: bool,
) -> (Option<String>, String) {
    if is_current {
        let auth_state = runtime
            .config
            .provider
            .resolved_auth_env_name()
            .map(|env_name| format!("auth {env_name}"))
            .or_else(|| {
                if runtime.config.provider.api_key().is_some()
                    || runtime.config.provider.oauth_access_token().is_some()
                {
                    Some("runtime credentials already loaded".to_owned())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "credentials still need wiring".to_owned());
        return (
            Some("current".to_owned()),
            format!(
                "current active provider · model {} · {auth_state}",
                runtime.config.provider.model
            ),
        );
    }

    if let Some((env_name, binding_kind)) = detected_startup_auth_binding(kind) {
        let binding_label = match binding_kind {
            StartupProviderAuthBindingKind::ApiKey => "api key",
            StartupProviderAuthBindingKind::OauthAccessToken => "oauth",
        };
        return (
            Some("ready".to_owned()),
            format!(
                "switch provider to {} · {binding_label} {env_name} available",
                kind.display_name()
            ),
        );
    }

    (
        Some("unconfigured".to_owned()),
        format!(
            "switch provider to {} · credentials still need wiring",
            kind.display_name()
        ),
    )
}

fn web_search_provider_env_api_key_name(provider_id: &str) -> Option<String> {
    web_search_provider_api_key_env_names(provider_id)
        .iter()
        .find(|env_name| std::env::var_os(env_name).is_some())
        .map(|env_name| (*env_name).to_owned())
}

fn render_web_provider_settings_entry(
    runtime: &CliTurnRuntime,
    provider_id: &str,
    provider_label: &str,
    is_current: bool,
) -> (Option<String>, String) {
    if is_current {
        let credential_state = runtime
            .config
            .tools
            .web_search
            .configured_api_key_for_provider(provider_id)
            .map(|value| format!("configured in tools.web_search as {value}"))
            .or_else(|| {
                web_search_provider_env_api_key_name(provider_id)
                    .map(|env_name| format!("env {env_name} available"))
            })
            .unwrap_or_else(|| "credentials still need wiring".to_owned());
        return (
            Some("current".to_owned()),
            format!("current default web-search provider · {credential_state}"),
        );
    }

    if let Some(env_name) = web_search_provider_env_api_key_name(provider_id) {
        return (
            Some("ready".to_owned()),
            format!("switch default web-search to {provider_label} · env {env_name} available"),
        );
    }

    (
        Some("unconfigured".to_owned()),
        format!("switch default web-search to {provider_label} · credentials still need wiring"),
    )
}

fn persist_runtime_settings(
    runtime: &mut CliTurnRuntime,
    app: &mut App,
    mutate: impl FnOnce(&mut LoongConfig) -> Result<String, String>,
) -> CliResult<String> {
    let mut config = runtime.config.clone();
    let summary = mutate(&mut config)?;
    crate::config::write(
        Some(runtime.resolved_path.to_string_lossy().as_ref()),
        &config,
        true,
    )?;
    #[cfg(not(test))]
    crate::runtime_env::initialize_runtime_environment(
        &config,
        Some(runtime.resolved_path.as_path()),
    );
    runtime.config = config;
    runtime.config_present = true;
    app.model = runtime.config.provider.model.clone();
    Ok(summary)
}
fn current_working_directory(runtime: &CliTurnRuntime) -> PathBuf {
    runtime
        .effective_working_directory
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn current_working_directory_display(runtime: &CliTurnRuntime) -> String {
    let current_directory = current_working_directory(runtime);
    current_directory.display().to_string()
}

fn render_new_conversation_lines_with_width(width: usize) -> Vec<String> {
    let message_spec = TuiMessageSpec {
        role: "new".to_owned(),
        caption: Some("fresh conversation".to_owned()),
        sections: vec![TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Info,
            title: Some("ready".to_owned()),
            lines: vec![
                "The visible transcript has been cleared and the composer is ready for the next turn."
                    .to_owned(),
            ],
        }],
        footer_lines: vec!["Type immediately; no extra focus step is needed.".to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn copy_command_text(app: &App, args: &str) -> Result<String, String> {
    if !args.trim().is_empty() {
        return Ok(args.trim().to_owned());
    }
    app.message_list
        .latest_copy_text()
        .ok_or_else(|| "nothing copyable yet".to_owned())
}

fn copy_to_system_clipboard(text: &str) -> Result<(), String> {
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else {
        &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"])]
    };

    let mut last_error = "no clipboard command attempted".to_owned();
    for (program, args) in candidates {
        let spawn_result = Command::new(program)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn();
        let Ok(mut child) = spawn_result else {
            last_error = format!("{program} unavailable");
            continue;
        };
        if let Some(stdin) = child.stdin.as_mut()
            && let Err(error) = stdin.write_all(text.as_bytes())
        {
            last_error = format!("{program} write failed: {error}");
            let _ = child.kill();
            continue;
        }
        let output = child
            .wait_with_output()
            .map_err(|error| format!("{program} wait failed: {error}"))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        last_error = if stderr.is_empty() {
            format!("{program} exited with {}", output.status)
        } else {
            format!("{program}: {stderr}")
        };
    }
    Err(last_error)
}

fn render_copy_command_lines_with_width(
    result: Result<String, String>,
    width: usize,
) -> Vec<String> {
    let (tone, title, lines) = match result {
        Ok(text) => {
            let char_count = text.chars().count();
            (
                TuiCalloutTone::Info,
                "copied".to_owned(),
                vec![format!(
                    "Copied {char_count} character(s) to the system clipboard."
                )],
            )
        }
        Err(error) => (
            TuiCalloutTone::Warning,
            "copy unavailable".to_owned(),
            vec![error],
        ),
    };
    let message_spec = TuiMessageSpec {
        role: "copy".to_owned(),
        caption: Some("clipboard".to_owned()),
        sections: vec![TuiSectionSpec::Callout {
            tone,
            title: Some(title),
            lines,
        }],
        footer_lines: vec![
            "/copy copies the latest reply, or /copy <text> copies explicit text.".to_owned(),
        ],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn run_git_capture(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(|error| format!("git failed to start: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if output.status.success() {
        Ok(stdout)
    } else if stderr.is_empty() {
        Err(format!("git exited with {}", output.status))
    } else {
        Err(stderr)
    }
}

fn render_git_diff_command_lines_with_width(cwd: &Path, width: usize) -> Vec<String> {
    let status = run_git_capture(cwd, &["status", "--short"]);
    let stat = run_git_capture(cwd, &["diff", "--stat"]);
    let shortstat = run_git_capture(cwd, &["diff", "--shortstat"]);

    let mut sections = Vec::new();
    match (status, stat, shortstat) {
        (Ok(status), Ok(stat), Ok(shortstat)) => {
            let status_lines = if status.trim().is_empty() {
                vec!["working tree clean".to_owned()]
            } else {
                status.lines().map(ToOwned::to_owned).collect()
            };
            sections.push(TuiSectionSpec::Preformatted {
                title: Some("status".to_owned()),
                language: None,
                lines: status_lines,
            });
            if !stat.trim().is_empty() {
                sections.push(TuiSectionSpec::Preformatted {
                    title: Some("diff stat".to_owned()),
                    language: None,
                    lines: stat.lines().map(ToOwned::to_owned).collect(),
                });
            }
            if !shortstat.trim().is_empty() {
                sections.push(TuiSectionSpec::Narrative {
                    title: Some("summary".to_owned()),
                    lines: vec![shortstat],
                });
            }
        }
        (status, stat, shortstat) => {
            let errors = [status.err(), stat.err(), shortstat.err()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            sections.push(TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Warning,
                title: Some("git diff unavailable".to_owned()),
                lines: if errors.is_empty() {
                    vec!["git did not return diff information".to_owned()]
                } else {
                    errors
                },
            });
        }
    }

    let message_spec = TuiMessageSpec {
        role: "diff".to_owned(),
        caption: Some("working tree".to_owned()),
        sections,
        footer_lines: vec![format!("cwd: {}", cwd.display())],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn safe_file_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(64)
        .collect::<String>()
}

fn write_transcript_export(
    cwd: &Path,
    session_id: &str,
    label: &str,
    markdown: &str,
) -> Result<PathBuf, String> {
    if markdown.trim().is_empty() {
        return Err("transcript is empty".to_owned());
    }
    let export_dir = cwd.join(".loong").join("exports");
    fs::create_dir_all(export_dir.as_path())
        .map_err(|error| format!("failed to create export directory: {error}"))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("clock error: {error}"))?
        .as_secs();
    let session = safe_file_component(session_id);
    let label = safe_file_component(label);
    let file_name = format!("{label}-{session}-{timestamp}.md");
    let path = export_dir.join(file_name);
    fs::write(path.as_path(), markdown)
        .map_err(|error| format!("failed to write export: {error}"))?;
    Ok(path)
}

fn render_export_command_lines_with_width(
    command: &str,
    result: Result<PathBuf, String>,
    width: usize,
) -> Vec<String> {
    let (tone, title, lines) = match result {
        Ok(path) => (
            TuiCalloutTone::Info,
            "written".to_owned(),
            vec![format!("{} wrote {}", command, path.display())],
        ),
        Err(error) => (
            TuiCalloutTone::Warning,
            "not written".to_owned(),
            vec![error],
        ),
    };
    let message_spec = TuiMessageSpec {
        role: command.trim_start_matches('/').to_owned(),
        caption: Some("transcript artifact".to_owned()),
        sections: vec![TuiSectionSpec::Callout {
            tone,
            title: Some(title),
            lines,
        }],
        footer_lines: vec![
            "Artifacts stay local until you explicitly move or publish them.".to_owned(),
        ],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn resolve_import_path(cwd: &Path, input: &str) -> PathBuf {
    let trimmed = input.trim().trim_matches('"').trim_matches('\'');
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn resolve_cwd_change_path(runtime: &CliTurnRuntime, input: &str) -> Result<PathBuf, String> {
    let trimmed = input.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() {
        return Err("Usage: /cwd <path>".to_owned());
    }

    let candidate = PathBuf::from(trimmed);
    let resolved = if candidate.is_absolute() {
        candidate
    } else {
        current_working_directory(runtime).join(candidate)
    };

    let normalized = if resolved.exists() {
        dunce::canonicalize(&resolved)
            .map_err(|error| format!("failed to resolve {}: {error}", resolved.display()))?
    } else {
        return Err(format!(
            "working directory does not exist: {}",
            resolved.display()
        ));
    };

    if !normalized.is_dir() {
        return Err(format!(
            "working directory is not a directory: {}",
            normalized.display()
        ));
    }

    Ok(normalized)
}

fn render_cwd_change_command_lines_with_width(
    result: Result<PathBuf, String>,
    width: usize,
) -> Vec<String> {
    let (tone, title, lines) = match result {
        Ok(path) => (
            TuiCalloutTone::Info,
            "cwd updated".to_owned(),
            vec![format!("Working directory set to {}.", path.display())],
        ),
        Err(error) => (
            TuiCalloutTone::Warning,
            "cwd unchanged".to_owned(),
            vec![error],
        ),
    };
    let message_spec = TuiMessageSpec {
        role: "cwd".to_owned(),
        caption: Some("working directory".to_owned()),
        sections: vec![TuiSectionSpec::Callout {
            tone,
            title: Some(title),
            lines,
        }],
        footer_lines: vec![
            "Use /cwd with no arguments to inspect the current working directory.".to_owned(),
        ],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn import_context_into_composer(app: &mut App, cwd: &Path, args: &str) -> Result<PathBuf, String> {
    let path = resolve_import_path(cwd, args);
    let content = fs::read_to_string(path.as_path())
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let clipped = if content.chars().count() > 20_000 {
        let prefix = content.chars().take(20_000).collect::<String>();
        format!("{prefix}\n\n[import truncated to first 20000 characters]")
    } else {
        content
    };
    app.composer.set_input(format!(
        "Use this imported context from {}:\n\n{}",
        path.display(),
        clipped
    ));
    Ok(path)
}

fn render_import_command_lines_with_width(
    result: Result<PathBuf, String>,
    width: usize,
) -> Vec<String> {
    let (tone, title, lines) = match result {
        Ok(path) => (
            TuiCalloutTone::Info,
            "staged".to_owned(),
            vec![format!(
                "Imported {} into the composer draft.",
                path.display()
            )],
        ),
        Err(error) => (
            TuiCalloutTone::Warning,
            "import failed".to_owned(),
            vec![error],
        ),
    };
    let message_spec = TuiMessageSpec {
        role: "import".to_owned(),
        caption: Some("composer context".to_owned()),
        sections: vec![TuiSectionSpec::Callout {
            tone,
            title: Some(title),
            lines,
        }],
        footer_lines: vec![
            "Review the staged draft before sending if the file is large.".to_owned(),
        ],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn latest_text_or_args(app: &App, args: &str) -> Result<String, String> {
    if !args.trim().is_empty() {
        return Ok(args.trim().to_owned());
    }
    app.message_list
        .latest_copy_text()
        .ok_or_else(|| "no previous content to use".to_owned())
}

fn stage_simplify_prompt(app: &mut App, args: &str) -> Result<(), String> {
    let source = latest_text_or_args(app, args)?;
    app.composer.set_input(format!(
        "Please simplify and clarify the following content without losing important details:\n\n{source}"
    ));
    Ok(())
}

fn stage_plan_prompt(app: &mut App, args: &str) -> Result<(), String> {
    let subject = if args.trim().is_empty() {
        "the current task".to_owned()
    } else {
        args.trim().to_owned()
    };
    app.composer.set_input(format!(
        "Create a concise implementation plan for {subject}. Include risks, verification, and the smallest safe sequence."
    ));
    Ok(())
}

fn stage_feedback_prompt(app: &mut App, args: &str) -> Result<(), String> {
    let body = if args.trim().is_empty() {
        "Feedback: ".to_owned()
    } else {
        format!("Feedback: {}", args.trim())
    };
    app.composer.set_input(body);
    Ok(())
}

fn render_prompt_staging_lines_with_width(
    role: &str,
    result: Result<(), String>,
    width: usize,
) -> Vec<String> {
    let (tone, title, lines) = match result {
        Ok(()) => (
            TuiCalloutTone::Info,
            "draft staged".to_owned(),
            vec!["The composer has been populated; edit or press Enter to send.".to_owned()],
        ),
        Err(error) => (
            TuiCalloutTone::Warning,
            "not staged".to_owned(),
            vec![error],
        ),
    };
    let message_spec = TuiMessageSpec {
        role: role.to_owned(),
        caption: Some("composer draft".to_owned()),
        sections: vec![TuiSectionSpec::Callout {
            tone,
            title: Some(title),
            lines,
        }],
        footer_lines: vec!["Typing continues in the composer immediately.".to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn render_title_command_lines_with_width(command: &str, args: &str, width: usize) -> Vec<String> {
    let lines = if args.trim().is_empty() {
        vec![format!("Usage: {command} <title>")]
    } else {
        vec![format!(
            "Title noted for this local chat surface: {}",
            args.trim()
        )]
    };
    let message_spec = TuiMessageSpec {
        role: command.trim_start_matches('/').to_owned(),
        caption: Some("local title".to_owned()),
        sections: vec![TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Info,
            title: Some("title".to_owned()),
            lines,
        }],
        footer_lines: vec!["The title is reflected in the footer for this TUI session.".to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

