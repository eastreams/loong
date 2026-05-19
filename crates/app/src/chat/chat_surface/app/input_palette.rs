fn render_onboarding_option_line(
    selected: bool,
    label: &str,
    badge: Option<&str>,
    content_width: usize,
) -> Vec<Line<'static>> {
    let prefix = if selected { "› " } else { "  " };
    let text = match badge {
        Some(badge) => format!("{label} · {badge}"),
        None => label.to_owned(),
    };
    vec![Line::from(Span::styled(
        truncate_right_for_width(format!("{prefix}{text}").as_str(), content_width),
        if selected {
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        },
    ))]
}

fn render_onboarding_wrapped_line(
    prefix: &str,
    text: &str,
    prefix_style: Style,
    body_style: Style,
    content_width: usize,
) -> Vec<Line<'static>> {
    let prefix_width = crate::presentation::display_width(prefix);
    let body_width = content_width.saturating_sub(prefix_width).max(1);
    let mut wrapped = crate::presentation::render_wrapped_plain_display_line(text, body_width);
    if wrapped.is_empty() {
        wrapped.push(String::new());
    }

    wrapped
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                Line::from(vec![
                    Span::styled(prefix.to_owned(), prefix_style),
                    Span::styled(line, body_style),
                ])
            } else {
                Line::from(vec![
                    Span::raw(" ".repeat(prefix_width)),
                    Span::styled(line, body_style),
                ])
            }
        })
        .collect()
}

fn paste_into_composer(app: &mut App, text: &str) {
    if text.is_empty() {
        return;
    }
    app.composer.insert_paste(text);
    app.focus = Focus::Composer;
    if app.pending_turn && !app.composer.is_empty() {
        app.composer_follow_up_intent = true;
    }
    app.sync_inline_skill_popup();
}

fn open_slash_command_palette(app: &mut App, prefix: char, query: &str) {
    let normalized_prefix = if prefix == ':' { ':' } else { '/' };
    app.command_palette.show_commands(query);
    app.composer
        .set_input(format!("{normalized_prefix}{}", query.trim()));
    app.inline_skill_popup_active = false;
    app.focus = Focus::CommandPalette;
}

fn sync_slash_palette_composer(app: &mut App) {
    if !app.command_palette.is_commands_mode() {
        return;
    }
    let prefix = app
        .composer
        .text()
        .chars()
        .next()
        .filter(|ch| matches!(ch, '/' | ':'))
        .unwrap_or('/');
    app.composer
        .set_input(format!("{prefix}{}", app.command_palette.query_text()));
}

fn clear_slash_palette_composer(app: &mut App) {
    if app.command_palette.is_commands_mode()
        && app
            .composer
            .text()
            .chars()
            .next()
            .is_some_and(|ch| matches!(ch, '/' | ':'))
    {
        app.composer.clear();
        app.composer_follow_up_intent = false;
    }
}

fn push_unique_model_candidate(out: &mut Vec<String>, model: &str) {
    let trimmed = model.trim();
    if trimmed.is_empty() || out.iter().any(|existing| existing == trimmed) {
        return;
    }
    out.push(trimmed.to_owned());
}

fn local_model_candidates(provider: &ProviderConfig) -> Vec<String> {
    let mut models = Vec::new();
    push_unique_model_candidate(&mut models, provider.model.as_str());
    for preferred in &provider.preferred_models {
        push_unique_model_candidate(&mut models, preferred.as_str());
    }
    if let Some(default_model) = provider.kind.default_model() {
        push_unique_model_candidate(&mut models, default_model);
    }
    if let Some(recommended_model) = provider.kind.recommended_onboarding_model() {
        push_unique_model_candidate(&mut models, recommended_model);
    }
    models
}

fn merged_model_catalog_entries(
    provider: &ProviderConfig,
    catalog: &[crate::provider::ProviderModelCatalogEntry],
    include_hidden_and_deprecated: bool,
) -> Vec<crate::provider::ProviderModelCatalogEntry> {
    let mut merged = Vec::new();
    let mut seen = HashSet::new();

    for model in local_model_candidates(provider) {
        if seen.insert(model.clone()) {
            if let Some(entry) = catalog.iter().find(|entry| entry.model == model) {
                merged.push(entry.clone());
            } else {
                merged.push(crate::provider::ProviderModelCatalogEntry {
                    model,
                    display_name: None,
                    description: None,
                    is_default: false,
                    hidden: false,
                    deprecated: false,
                    default_reasoning_effort: None,
                    supported_reasoning_efforts: Vec::new(),
                    supported_reasoning_effort_descriptions: Vec::new(),
                });
            }
        }
    }

    for entry in catalog {
        if !include_hidden_and_deprecated && (entry.hidden || entry.deprecated) {
            continue;
        }
        if seen.insert(entry.model.clone()) {
            merged.push(entry.clone());
        }
    }

    merged
}

fn find_exact_model_catalog_entry<'a>(
    catalog: &'a [crate::provider::ProviderModelCatalogEntry],
    query: &str,
) -> Option<&'a crate::provider::ProviderModelCatalogEntry> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }

    catalog.iter().find(|entry| {
        entry.model.eq_ignore_ascii_case(query)
            || entry
                .display_name
                .as_deref()
                .is_some_and(|display_name| display_name.eq_ignore_ascii_case(query))
    })
}

fn model_entry_label(entry: &crate::provider::ProviderModelCatalogEntry) -> String {
    entry
        .display_name
        .clone()
        .unwrap_or_else(|| entry.model.clone())
}

fn model_entry_description(
    provider: &ProviderConfig,
    entry: &crate::provider::ProviderModelCatalogEntry,
    reasoning_efforts: &[ReasoningEffort],
) -> String {
    let mut parts = Vec::new();
    if let Some(display_name) = entry.display_name.as_deref()
        && !display_name.eq_ignore_ascii_case(entry.model.as_str())
    {
        parts.push(entry.model.clone());
    }
    if let Some(description) = entry.description.as_deref()
        && !description.is_empty()
    {
        parts.push(description.to_owned());
    }
    if entry.is_default {
        parts.push("catalog default".to_owned());
    }
    if entry.hidden {
        parts.push("hidden from default picker".to_owned());
    }
    if entry.deprecated {
        parts.push("deprecated".to_owned());
    }
    if let Some(default_effort) =
        crate::provider::effective_default_reasoning_effort_for_entry(provider, entry)
    {
        parts.push(format!("default {}", default_effort.as_str()));
    }

    match reasoning_efforts {
        [] => parts.push("apply immediately".to_owned()),
        [only_effort] => parts.push(format!("apply {} immediately", only_effort.as_str())),
        _ => parts.push("choose reasoning next".to_owned()),
    }

    parts.join(" · ")
}

fn current_reasoning_label(runtime: &CliTurnRuntime) -> String {
    runtime
        .config
        .provider
        .reasoning_effort
        .map(|effort| effort.as_str().to_owned())
        .unwrap_or_else(|| "default".to_owned())
}

fn reasoning_option_description(reasoning_effort: Option<ReasoningEffort>) -> String {
    match reasoning_effort {
        None => "use the provider or model default reasoning behavior".to_owned(),
        Some(ReasoningEffort::None) => {
            "disable explicit reasoning effort for this model".to_owned()
        }
        Some(ReasoningEffort::Minimal) => "keep reasoning as light as possible".to_owned(),
        Some(ReasoningEffort::Low) => "favor quick turns with light reasoning".to_owned(),
        Some(ReasoningEffort::Medium) => "balance speed and deeper reasoning".to_owned(),
        Some(ReasoningEffort::High) => "prefer deeper reasoning for harder turns".to_owned(),
        Some(ReasoningEffort::Xhigh) => {
            "maximize reasoning depth when the provider supports it".to_owned()
        }
    }
}

fn reasoning_option_description_for_entry(
    entry: &crate::provider::ProviderModelCatalogEntry,
    reasoning_effort: ReasoningEffort,
) -> String {
    crate::provider::reasoning_effort_description_for_entry(entry, reasoning_effort)
        .map(str::to_owned)
        .unwrap_or_else(|| reasoning_option_description(Some(reasoning_effort)))
}

fn default_reasoning_option_description(
    runtime: &CliTurnRuntime,
    entry: &crate::provider::ProviderModelCatalogEntry,
) -> String {
    crate::provider::effective_default_reasoning_effort_for_entry(&runtime.config.provider, entry)
        .map(|effort| {
            let detail = reasoning_option_description_for_entry(entry, effort);
            format!(
                "use the model default reasoning behavior ({} · {})",
                effort.as_str(),
                detail
            )
        })
        .unwrap_or_else(|| "use the provider or model default reasoning behavior".to_owned())
}

fn build_model_palette_entries(
    runtime: &CliTurnRuntime,
    catalog: &[crate::provider::ProviderModelCatalogEntry],
) -> Vec<SettingsEntry> {
    let provider = &runtime.config.provider;
    let current_model = provider.model.trim();
    let default_model = provider.kind.default_model();
    let configured_auto_models = provider.configured_auto_model_candidates();

    let mut ordered = catalog.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        let left_model = left.model.trim();
        let right_model = right.model.trim();
        let left_rank = (
            usize::from(left_model != current_model),
            usize::from(
                !configured_auto_models
                    .iter()
                    .any(|candidate| candidate == left_model),
            ),
            usize::from(Some(left_model) != default_model && !left.is_default),
            usize::from(left.hidden),
            usize::from(left.deprecated),
        );
        let right_rank = (
            usize::from(right_model != current_model),
            usize::from(
                !configured_auto_models
                    .iter()
                    .any(|candidate| candidate == right_model),
            ),
            usize::from(Some(right_model) != default_model && !right.is_default),
            usize::from(right.hidden),
            usize::from(right.deprecated),
        );
        left_rank
            .cmp(&right_rank)
            .then_with(|| model_entry_label(left).cmp(&model_entry_label(right)))
            .then_with(|| left.model.cmp(&right.model))
    });

    ordered
        .into_iter()
        .map(|entry| {
            let trimmed = entry.model.trim();
            let is_current = trimmed == current_model;
            let status_tag = if is_current {
                Some("current".to_owned())
            } else if entry.is_default {
                Some("default".to_owned())
            } else if entry.deprecated {
                Some("deprecated".to_owned())
            } else if entry.hidden {
                Some("hidden".to_owned())
            } else if Some(trimmed) == default_model {
                Some("default".to_owned())
            } else if configured_auto_models
                .iter()
                .any(|candidate| candidate == trimmed)
            {
                Some("preferred".to_owned())
            } else {
                None
            };
            let reasoning_efforts =
                crate::provider::effective_supported_reasoning_efforts_for_entry(provider, entry);
            let description =
                model_entry_description(provider, entry, reasoning_efforts.as_slice());
            let action = if reasoning_efforts.is_empty() {
                CommandAction::ApplyModelSelection {
                    model: trimmed.to_owned(),
                    reasoning_effort: None,
                }
            } else if reasoning_efforts.len() == 1 {
                CommandAction::ApplyModelSelection {
                    model: trimmed.to_owned(),
                    reasoning_effort: reasoning_efforts.first().copied(),
                }
            } else {
                CommandAction::OpenModelReasoning(entry.clone())
            };
            SettingsEntry {
                label: model_entry_label(entry),
                category_tag: "[Model]".to_owned(),
                status_tag,
                description,
                action,
                selectable: true,
            }
        })
        .collect()
}

fn build_reasoning_palette_entries(
    runtime: &CliTurnRuntime,
    entry: &crate::provider::ProviderModelCatalogEntry,
) -> (Vec<SettingsEntry>, String) {
    let supported = crate::provider::effective_supported_reasoning_efforts_for_entry(
        &runtime.config.provider,
        entry,
    );
    let selected_label = runtime
        .config
        .provider
        .reasoning_effort
        .map(|effort| effort.as_str().to_owned())
        .unwrap_or_else(|| "default".to_owned());

    let mut entries = vec![SettingsEntry {
        label: "default".to_owned(),
        category_tag: "[Reasoning]".to_owned(),
        status_tag: (runtime.config.provider.model == entry.model
            && runtime.config.provider.reasoning_effort.is_none())
        .then(|| "current".to_owned()),
        description: default_reasoning_option_description(runtime, entry),
        action: CommandAction::ApplyModelSelection {
            model: entry.model.clone(),
            reasoning_effort: None,
        },
        selectable: true,
    }];

    for effort in supported {
        entries.push(SettingsEntry {
            label: effort.as_str().to_owned(),
            category_tag: "[Reasoning]".to_owned(),
            status_tag: (runtime.config.provider.model == entry.model
                && runtime.config.provider.reasoning_effort == Some(effort))
            .then(|| "current".to_owned()),
            description: reasoning_option_description_for_entry(entry, effort),
            action: CommandAction::ApplyModelSelection {
                model: entry.model.clone(),
                reasoning_effort: Some(effort),
            },
            selectable: true,
        });
    }

    (entries, selected_label)
}

async fn open_model_palette(
    app: &mut App,
    runtime: &mut CliTurnRuntime,
    query: &str,
) -> CliResult<()> {
    let (catalog, status) = match crate::provider::fetch_model_catalog(&runtime.config).await {
        Ok(catalog) => {
            let count = catalog.len();
            (
                catalog,
                Some(format!(
                    "{count} models available for {}",
                    runtime.config.provider.kind.display_name()
                )),
            )
        }
        Err(error) => (
            merged_model_catalog_entries(&runtime.config.provider, &[], false),
            Some(format!(
                "model catalog unavailable; showing local candidates ({error})"
            )),
        ),
    };
    let exact_catalog =
        merged_model_catalog_entries(&runtime.config.provider, catalog.as_slice(), true);
    if let Some(entry) = find_exact_model_catalog_entry(exact_catalog.as_slice(), query) {
        let reasoning_efforts = crate::provider::effective_supported_reasoning_efforts_for_entry(
            &runtime.config.provider,
            entry,
        );
        if reasoning_efforts.is_empty() {
            apply_model_selection(app, runtime, entry.model.clone(), None)?;
            return Ok(());
        }
        if reasoning_efforts.len() == 1 {
            apply_model_selection(
                app,
                runtime,
                entry.model.clone(),
                reasoning_efforts.first().copied(),
            )?;
            return Ok(());
        }
        open_reasoning_palette(app, runtime, entry);
        return Ok(());
    }
    let merged = merged_model_catalog_entries(&runtime.config.provider, catalog.as_slice(), false);
    let entries = build_model_palette_entries(runtime, merged.as_slice());
    app.command_palette.show_model_selector(
        entries,
        status,
        Some(runtime.config.provider.model.as_str()),
        query,
    );
    app.inline_skill_popup_active = false;
    app.focus = Focus::CommandPalette;
    app.composer.clear();
    Ok(())
}

fn open_reasoning_palette(
    app: &mut App,
    runtime: &CliTurnRuntime,
    entry: &crate::provider::ProviderModelCatalogEntry,
) {
    let (entries, selected_label) = build_reasoning_palette_entries(runtime, entry);
    app.command_palette.show_reasoning_selector(
        entry.model.as_str(),
        entries,
        Some(format!(
            "Current reasoning: {} · Enter apply · Esc back",
            current_reasoning_label(runtime)
        )),
        Some(selected_label.as_str()),
    );
    app.inline_skill_popup_active = false;
    app.focus = Focus::CommandPalette;
}

fn apply_model_selection(
    app: &mut App,
    runtime: &mut CliTurnRuntime,
    model: String,
    reasoning_effort: Option<ReasoningEffort>,
) -> CliResult<()> {
    let _ = persist_runtime_settings(runtime, app, |config| {
        config.provider.model = model.clone();
        config.provider.reasoning_effort = reasoning_effort;
        Ok(format!(
            "model switched to {} · reasoning {}",
            model,
            reasoning_effort
                .map(|effort| effort.as_str().to_owned())
                .unwrap_or_else(|| "default".to_owned())
        ))
    })?;
    app.inline_skill_popup_active = false;
    app.focus = Focus::Composer;
    app.composer.clear();
    Ok(())
}

async fn run_surface_command<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    runtime: &mut CliTurnRuntime,
    options: &CliChatOptions,
    input: &str,
) -> CliResult<()> {
    refresh_app_cwd_dependent_state(app, runtime);
    let trimmed = input.trim();
    let (command, args) = split_surface_command(trimmed);
    let width = current_render_width(terminal)?;

    match command {
        "/clear" => {
            app.message_list.clear_transcript();
            app.focus = Focus::Composer;
            Ok(())
        }
        "/new" => {
            app.message_list.clear_transcript();
            app.message_list
                .add_rendered_lines(render_new_conversation_lines_with_width(width));
            app.focus = Focus::Composer;
            Ok(())
        }
        "/copy" => {
            let copy_result = copy_command_text(app, args)
                .and_then(|text| copy_to_system_clipboard(text.as_str()).map(|()| text));
            app.message_list
                .add_rendered_lines(render_copy_command_lines_with_width(copy_result, width));
            app.focus = Focus::Composer;
            Ok(())
        }
        "/diff" => {
            let cwd = current_working_directory(runtime);
            let lines = render_git_diff_command_lines_with_width(cwd.as_path(), width);
            app.message_list.add_rendered_lines(lines);
            app.focus = Focus::Composer;
            Ok(())
        }
        "/export" | "/share" => {
            let cwd = current_working_directory(runtime);
            let markdown = app.message_list.export_markdown();
            let result = write_transcript_export(
                cwd.as_path(),
                runtime.session_id.as_str(),
                command.trim_start_matches('/'),
                markdown.as_str(),
            );
            app.message_list
                .add_rendered_lines(render_export_command_lines_with_width(
                    command, result, width,
                ));
            app.focus = Focus::Composer;
            Ok(())
        }
        "/import" => {
            if args.trim().is_empty() {
                let lines = build_command_lines(runtime, options, input, width).await?;
                app.message_list.add_rendered_lines(lines);
            } else {
                let cwd = current_working_directory(runtime);
                let result = import_context_into_composer(app, cwd.as_path(), args);
                app.message_list
                    .add_rendered_lines(render_import_command_lines_with_width(result, width));
            }
            app.focus = Focus::Composer;
            Ok(())
        }
        "/cwd" => {
            if args.trim().is_empty() {
                let lines = render_cwd_command_lines_with_width(runtime, width);
                app.message_list.add_rendered_lines(lines);
            } else {
                let result = resolve_cwd_change_path(runtime, args);
                if let Ok(path) = result.as_ref() {
                    runtime.effective_working_directory = Some(path.clone());
                    refresh_app_cwd_dependent_state(app, runtime);
                }
                app.message_list
                    .add_rendered_lines(render_cwd_change_command_lines_with_width(result, width));
            }
            app.focus = Focus::Composer;
            Ok(())
        }
        "/simplify" => {
            let result = stage_simplify_prompt(app, args);
            app.message_list
                .add_rendered_lines(render_prompt_staging_lines_with_width(
                    "simplify", result, width,
                ));
            app.focus = Focus::Composer;
            Ok(())
        }
        "/plan" => {
            let result = stage_plan_prompt(app, args);
            app.message_list
                .add_rendered_lines(render_prompt_staging_lines_with_width(
                    "plan", result, width,
                ));
            app.focus = Focus::Composer;
            Ok(())
        }
        "/title" | "/rename" => {
            if !args.trim().is_empty() {
                app.title = Some(args.trim().to_owned());
            }
            let lines = render_title_command_lines_with_width(command, args, width);
            app.message_list.add_rendered_lines(lines);
            app.focus = Focus::Composer;
            Ok(())
        }
        "/feedback" => {
            let result = stage_feedback_prompt(app, args);
            app.message_list
                .add_rendered_lines(render_prompt_staging_lines_with_width(
                    "feedback", result, width,
                ));
            app.focus = Focus::Composer;
            Ok(())
        }
        "/model" => open_model_palette(app, runtime, args).await,
        "/settings" if args.trim().is_empty() => {
            open_settings_palette(
                app,
                runtime,
                SettingsSurfaceFocus::Overview,
                width,
                None,
                None,
            );
            Ok(())
        }
        "/settings" if !args.trim().is_empty() => {
            let action = parse_settings_command_action(args)?;
            let _ = dispatch_palette_action(app, runtime, width, action)?;
            Ok(())
        }
        _ => {
            let lines = build_command_lines(runtime, options, input, width).await?;
            app.message_list.add_rendered_lines(lines);
            app.focus = Focus::Composer;
            Ok(())
        }
    }
}

