#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoongTerminalActivity {
    Idle,
    Working,
    AttentionRequired,
}

fn loong_terminal_title_prefix(activity: LoongTerminalActivity) -> &'static str {
    match activity {
        LoongTerminalActivity::Idle => "🐉",
        LoongTerminalActivity::Working => "⠋",
        LoongTerminalActivity::AttentionRequired => "[ ! ] Action Required",
    }
}

fn compact_path_label(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return "~".to_owned();
    }

    let normalized = trimmed.trim_end_matches(['/', '\\']);
    if normalized.is_empty() {
        return trimmed.to_owned();
    }

    Path::new(normalized)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| normalized.to_owned())
}

fn terminal_title_braille_frame(start: Option<std::time::Instant>) -> &'static str {
    let elapsed_ms = start
        .map(|value| value.elapsed().as_millis() as u64)
        .unwrap_or_default();
    let frame_index = ((elapsed_ms / TERMINAL_TITLE_BRAILLE_INTERVAL_MS.max(1)) as usize)
        % TERMINAL_TITLE_BRAILLE_FRAMES.len();
    TERMINAL_TITLE_BRAILLE_FRAMES
        .get(frame_index)
        .copied()
        .unwrap_or(TERMINAL_TITLE_BRAILLE_FRAMES[0])
}

fn build_loong_terminal_title(
    cwd: &str,
    activity: LoongTerminalActivity,
    turn_start: Option<std::time::Instant>,
) -> String {
    let prefix = match activity {
        LoongTerminalActivity::Idle => loong_terminal_title_prefix(activity),
        LoongTerminalActivity::Working => terminal_title_braille_frame(turn_start),
        LoongTerminalActivity::AttentionRequired => loong_terminal_title_prefix(activity),
    };
    format!("{} - {}", prefix, compact_path_label(cwd))
}

fn refresh_app_cwd(app: &mut App, runtime: &CliTurnRuntime) {
    app.cwd = format_cwd(runtime);
}

fn refresh_app_cwd_dependent_state(app: &mut App, runtime: &CliTurnRuntime) {
    let preserved_skill_query = app
        .command_palette
        .is_skills_mode()
        .then(|| app.command_palette.query_text().to_owned());
    refresh_app_cwd(app, runtime);
    app.detected_skills = detect_available_skills(runtime.effective_working_directory.as_deref());
    let language = app.i18n.language();
    app.command_palette = CommandPalette::new(language, app.detected_skills.clone());
    if let Some(query) = preserved_skill_query.as_deref() {
        app.command_palette.show_skills(query);
    }
    app.title_pending_approval_count = current_pending_approval_count(runtime).unwrap_or(0);
    app.sync_inline_skill_popup();
}

fn current_pending_approval_count(runtime: &CliTurnRuntime) -> CliResult<usize> {
    #[cfg(feature = "memory-sqlite")]
    {
        let store = ChatControlPlaneStore::new(&runtime.memory_config)?;
        let approvals = store.approval_queue(&runtime.session_id, 256)?;
        Ok(approvals.len())
    }
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = runtime;
        Ok(0)
    }
}

fn app_terminal_title_requires_attention(app: &App) -> bool {
    app.title_attention_required
        || app.awaiting_first_turn_bootstrap_reply
        || app.title_pending_approval_count > 0
        || app
            .live_transcript
            .lock()
            .ok()
            .is_some_and(|state| state.has_needs_approval())
}

fn app_terminal_title_activity(app: &App) -> LoongTerminalActivity {
    if app.pending_turn {
        LoongTerminalActivity::Working
    } else if app_terminal_title_requires_attention(app) {
        LoongTerminalActivity::AttentionRequired
    } else {
        LoongTerminalActivity::Idle
    }
}

fn sanitize_terminal_title(title: &str) -> String {
    let mut sanitized = String::new();
    let mut chars_written = 0usize;
    let mut pending_space = false;

    for ch in title.chars() {
        if ch.is_whitespace() {
            pending_space = !sanitized.is_empty();
            continue;
        }

        if is_disallowed_terminal_title_char(ch) {
            continue;
        }

        if pending_space && chars_written < MAX_TERMINAL_TITLE_CHARS.saturating_sub(1) {
            sanitized.push(' ');
            chars_written += 1;
            pending_space = false;
        }

        if chars_written >= MAX_TERMINAL_TITLE_CHARS {
            break;
        }

        sanitized.push(ch);
        chars_written += 1;
    }

    sanitized
}

fn is_disallowed_terminal_title_char(ch: char) -> bool {
    ch.is_control()
        || matches!(
            ch,
            '\u{00AD}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{E0100}'..='\u{E01EF}'
        )
}

fn write_terminal_title(title: &str) -> std::io::Result<()> {
    if !std::io::stdout().is_terminal() {
        return Ok(());
    }

    crossterm::execute!(std::io::stdout(), SetTitle(title))
}

fn sync_app_terminal_title(app: &mut App) {
    let title = sanitize_terminal_title(
        build_loong_terminal_title(&app.cwd, app_terminal_title_activity(app), app.turn_start)
            .as_str(),
    );
    if title.is_empty() || app.last_terminal_title.as_deref() == Some(title.as_str()) {
        return;
    }

    if write_terminal_title(title.as_str()).is_ok() {
        app.last_terminal_title = Some(title);
    }
}

fn clear_app_terminal_title(app: &mut App) {
    let stable_title = sanitize_terminal_title(
        build_loong_terminal_title(&app.cwd, LoongTerminalActivity::Idle, None).as_str(),
    );
    if stable_title.is_empty() || app.last_terminal_title.as_deref() == Some(stable_title.as_str())
    {
        return;
    }

    if write_terminal_title(stable_title.as_str()).is_ok() {
        app.last_terminal_title = Some(stable_title);
    }
}

pub async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    runtime: CliTurnRuntime,
    options: CliChatOptions,
) -> CliResult<()> {
    let mut runtime = runtime;
    let mut last_known_size = terminal
        .size()
        .map_err(|e| format!("failed to query terminal size: {e}"))?;
    let render_width = last_known_size.width as usize;
    let mut app = App::new(&runtime, &options, render_width)?;
    refresh_app_cwd_dependent_state(&mut app, &runtime);
    sync_app_terminal_title(&mut app);
    let mut startup_release_task = Some(tokio::spawn(load_startup_release_lines(render_width)));
    let mut dirty = true;
    let mut last_resize_at: Option<std::time::Instant> = None;
    let mut pending_live_resize_rerender = false;

    loop {
        if let Some(task) = startup_release_task.as_ref()
            && task.is_finished()
            && let Some(task) = startup_release_task.take()
            && let Ok(Some(lines)) = task.await
        {
            app.message_list.add_rendered_lines(lines);
            dirty = true;
        }

        if maybe_finalize_pending_turn(terminal, &mut app, &mut runtime).await? {
            dirty = true;
        }

        if app.message_list.refresh_startup_animation() {
            dirty = true;
        }

        if app.pending_turn {
            let signature = pending_render_signature(&app);
            if signature != app.last_pending_signature {
                app.last_pending_signature = signature;
                dirty = true;
            }
            let live_transcript_signature = transcript_preview_signature(&app);
            if live_transcript_signature != app.last_live_transcript_signature {
                app.last_live_transcript_signature = live_transcript_signature;
                dirty = true;
            }
        } else {
            app.last_pending_signature = None;
            app.last_live_transcript_signature = None;
        }

        if resize_live_rerender_ready(
            pending_live_resize_rerender,
            last_resize_at.map(|instant| instant.elapsed()),
        ) {
            if let Some(rerender) = app.live_rerender.as_ref() {
                rerender();
            }
            pending_live_resize_rerender = false;
            last_resize_at = None;
            dirty = true;
        }

        if dirty {
            sync_app_terminal_title(&mut app);
            terminal
                .draw(|f| app.render(f))
                .map_err(|e| format!("draw error: {}", e))?;
            dirty = false;
            if !pending_live_resize_rerender {
                last_resize_at = None;
            }
        }

        let poll_timeout = if pending_live_resize_rerender {
            Duration::from_millis(16)
        } else if app.pending_turn {
            Duration::from_millis(40)
        } else if app.message_list.startup_animation_active() {
            Duration::from_millis(70)
        } else {
            Duration::from_millis(250)
        };

        if event::poll(poll_timeout).map_err(|e| format!("poll error: {}", e))? {
            let event = event::read().map_err(|e| format!("read error: {}", e))?;

            match event {
                Event::Key(key) => {
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        clear_app_terminal_title(&mut app);
                        break;
                    }

                    if key.code == KeyCode::Char('o')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        if app.message_list.toggle_latest_compaction() {
                            app.focus = Focus::MessageList;
                        }
                        continue;
                    }

                    if app.pending_turn {
                        let mut pending_command = None;
                        let mut pending_submission = None;
                        if key.code == KeyCode::Up
                            && key.modifiers.contains(KeyModifiers::ALT)
                            && dequeue_pending_steer(&mut app)
                        {
                            continue;
                        }
                        match app.focus {
                            Focus::Composer => {
                                if matches!(key.code, KeyCode::Char('/') | KeyCode::Char(':'))
                                    && app.composer.is_empty()
                                {
                                    let prefix = if key.code == KeyCode::Char(':') {
                                        ':'
                                    } else {
                                        '/'
                                    };
                                    open_slash_command_palette(&mut app, prefix, "");
                                } else if app.handle_inline_skill_popup_key(key) {
                                } else if should_route_composer_key_to_transcript(&app, key) {
                                    app.message_list.handle_key(key);
                                } else if key.code == KeyCode::Tab {
                                    if !app.composer.is_empty() {
                                        queue_pending_message(&mut app);
                                        app.inline_skill_popup_active = false;
                                    } else {
                                        app.focus = Focus::MessageList;
                                    }
                                } else if let Some(msg) = app.composer.handle_key(key) {
                                    pending_submission = Some(msg);
                                    app.sync_inline_skill_popup();
                                } else if !app.composer.is_empty() {
                                    app.composer_follow_up_intent = true;
                                    app.sync_inline_skill_popup();
                                } else {
                                    app.sync_inline_skill_popup();
                                }
                            }
                            Focus::MessageList => {
                                if matches!(key.code, KeyCode::Char('/') | KeyCode::Char(':'))
                                    && app.composer.is_empty()
                                {
                                    let prefix = if key.code == KeyCode::Char(':') {
                                        ':'
                                    } else {
                                        '/'
                                    };
                                    open_slash_command_palette(&mut app, prefix, "");
                                } else if should_focus_composer_for_transcript_key(key) {
                                    pending_submission =
                                        route_transcript_key_to_composer(&mut app, key);
                                } else {
                                    app.message_list.handle_key(key);
                                    if key.code == KeyCode::Esc || key.code == KeyCode::Enter {
                                        app.focus = Focus::Composer;
                                    }
                                }
                            }
                            Focus::CommandPalette => {
                                if app.command_palette.is_commands_mode()
                                    && key.code == KeyCode::Backspace
                                    && app.command_palette.query_text().is_empty()
                                {
                                    clear_slash_palette_composer(&mut app);
                                    app.inline_skill_popup_active = false;
                                    app.focus = Focus::Composer;
                                    dirty = true;
                                    continue;
                                }
                                if let Some(action) = app.command_palette.handle_key(key)
                                    && let Some(command) = dispatch_palette_action(
                                        &mut app,
                                        &mut runtime,
                                        current_render_width(terminal)?,
                                        action,
                                    )?
                                {
                                    pending_command = Some(command);
                                } else if app.command_palette.is_commands_mode() {
                                    sync_slash_palette_composer(&mut app);
                                }
                            }
                        }
                        if let Some(msg) = pending_submission {
                            if msg == "/exit" {
                                clear_app_terminal_title(&mut app);
                                break;
                            }
                            let trimmed_msg = msg.trim();
                            if matches!(trimmed_msg, "/" | ":") {
                                let prefix = if trimmed_msg.starts_with(':') {
                                    ':'
                                } else {
                                    '/'
                                };
                                open_slash_command_palette(&mut app, prefix, "");
                                dirty = true;
                                continue;
                            }
                            if let Some(command) = recognized_surface_command(trimmed_msg) {
                                pending_command = Some(command);
                            } else {
                                queue_pending_steer(&mut app, msg);
                            }
                        }
                        if let Some(command) = pending_command {
                            if command == "/exit" {
                                clear_app_terminal_title(&mut app);
                                break;
                            }
                            run_surface_command(
                                terminal,
                                &mut app,
                                &mut runtime,
                                &options,
                                &command,
                            )
                            .await?;
                        }
                        dirty = true;
                        continue;
                    }

                    let mut command_to_run = None;
                    let mut submitted_message = None;

                    if app.startup_onboarding.is_some()
                        && app.composer.is_empty()
                        && matches!(app.focus, Focus::Composer)
                    {
                        let action = app
                            .startup_onboarding
                            .as_mut()
                            .map(|state| state.handle_key(key))
                            .unwrap_or(StartupOnboardingAction::Ignored);
                        if app.apply_startup_onboarding_action(action, &mut runtime)? {
                            dirty = true;
                            continue;
                        }
                    }

                    match app.focus {
                        Focus::Composer => {
                            if key.code == KeyCode::Esc {
                                if !app.composer.is_empty() {
                                    app.composer.clear();
                                    app.composer_follow_up_intent = false;
                                    app.inline_skill_popup_active = false;
                                }
                            } else if matches!(key.code, KeyCode::Char('/') | KeyCode::Char(':'))
                                && app.composer.is_empty()
                            {
                                let prefix = if key.code == KeyCode::Char(':') {
                                    ':'
                                } else {
                                    '/'
                                };
                                open_slash_command_palette(&mut app, prefix, "");
                            } else if app.handle_inline_skill_popup_key(key) {
                            } else if should_route_composer_key_to_transcript(&app, key) {
                                app.message_list.handle_key(key);
                            } else if key.code == KeyCode::Tab {
                                app.focus = Focus::MessageList;
                            } else if let Some(msg) = app.composer.handle_key(key) {
                                submitted_message = Some(msg);
                                app.sync_inline_skill_popup();
                            } else {
                                app.sync_inline_skill_popup();
                            }
                        }
                        Focus::CommandPalette => {
                            if app.command_palette.is_commands_mode()
                                && key.code == KeyCode::Backspace
                                && app.command_palette.query_text().is_empty()
                            {
                                clear_slash_palette_composer(&mut app);
                                app.inline_skill_popup_active = false;
                                app.focus = Focus::Composer;
                                dirty = true;
                                continue;
                            }
                            if let Some(action) = app.command_palette.handle_key(key)
                                && let Some(command) = dispatch_palette_action(
                                    &mut app,
                                    &mut runtime,
                                    current_render_width(terminal)?,
                                    action,
                                )?
                            {
                                command_to_run = Some(command);
                            } else if app.command_palette.is_commands_mode() {
                                sync_slash_palette_composer(&mut app);
                            }
                        }
                        Focus::MessageList => {
                            if key.code == KeyCode::Tab {
                                app.focus = Focus::Composer;
                            } else if should_focus_composer_for_transcript_key(key) {
                                submitted_message = route_transcript_key_to_composer(&mut app, key);
                            } else {
                                app.message_list.handle_key(key);
                                if key.code == KeyCode::Esc || key.code == KeyCode::Enter {
                                    app.focus = Focus::Composer;
                                }
                            }
                        }
                    }

                    if let Some(msg) = submitted_message {
                        if msg == "/exit" {
                            clear_app_terminal_title(&mut app);
                            break;
                        }

                        let trimmed_msg = msg.trim();
                        if matches!(trimmed_msg, "/" | ":") {
                            let prefix = if trimmed_msg.starts_with(':') {
                                ':'
                            } else {
                                '/'
                            };
                            open_slash_command_palette(&mut app, prefix, "");
                            continue;
                        }

                        if let Some(command) = recognized_surface_command(trimmed_msg) {
                            command_to_run = Some(command);
                        } else if submitted_message_is_follow_up(&app, &msg) {
                            start_turn(terminal, &mut app, &mut runtime, msg, false).await?;
                        } else {
                            submit_user_turn(terminal, &mut app, &mut runtime, msg).await?;
                        }
                    }

                    if let Some(command) = command_to_run {
                        if command == "/exit" {
                            clear_app_terminal_title(&mut app);
                            break;
                        }

                        run_surface_command(terminal, &mut app, &mut runtime, &options, &command)
                            .await?;
                    }
                    dirty = true;
                }
                Event::Mouse(mouse_event) => {
                    if let Some(command) = app.handle_mouse_event(mouse_event) {
                        if command == "/exit" {
                            clear_app_terminal_title(&mut app);
                            break;
                        }
                        run_surface_command(terminal, &mut app, &mut runtime, &options, &command)
                            .await?;
                    }
                    dirty = true;
                }
                Event::Resize(width, height) => {
                    let new_size = ratatui::layout::Size::new(width, height);
                    if new_size.width == last_known_size.width
                        && new_size.height == last_known_size.height
                    {
                        continue;
                    }
                    let width_changed = last_known_size.width != new_size.width;
                    let layout_changed = resize_reflow_required(
                        last_known_size.width,
                        last_known_size.height,
                        new_size.width,
                        new_size.height,
                    );
                    if layout_changed {
                        last_resize_at = Some(std::time::Instant::now());
                    }
                    last_known_size = new_size;
                    app.last_render_width = new_size.width;
                    app.last_render_height = new_size.height;
                    app.live_render_width
                        .store(new_size.width.max(1) as usize, Ordering::Relaxed);
                    if width_changed && app.live_rerender.is_some() {
                        pending_live_resize_rerender = true;
                    }
                    dirty = true;
                }
                Event::Paste(text) => {
                    paste_into_composer(&mut app, text.as_str());
                    dirty = true;
                }
                Event::FocusGained | Event::FocusLost => {}
            }
        }
    }
    clear_app_terminal_title(&mut app);
    Ok(())
}

