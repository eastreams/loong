async fn submit_user_turn<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    runtime: &mut CliTurnRuntime,
    input: String,
) -> CliResult<()> {
    start_turn(terminal, app, runtime, input, true).await
}

impl App {
    fn pending_lines_for(
        &mut self,
        width: u16,
        height: u16,
        composer_height: u16,
        palette_height: u16,
    ) -> Vec<Line<'static>> {
        if !self.pending_turn {
            self.pending_render_cache = None;
            return Vec::new();
        }

        let max_pending_height = pending_band_max_height(height, composer_height, palette_height);
        let Some(signature) = pending_render_signature_for_geometry(
            self,
            width,
            height,
            composer_height,
            palette_height,
        ) else {
            self.pending_render_cache = None;
            return Vec::new();
        };

        if let Some(cache) = self.pending_render_cache.as_ref()
            && cache.signature == signature
            && cache.max_pending_height == max_pending_height
        {
            return cache.lines.clone();
        }

        let max_pending_preview_lines = max_pending_height.saturating_sub(2).max(1) as usize;
        let live_lines =
            pending_live_tool_activity_lines(&self.live_transcript, max_pending_preview_lines);
        let raw_pending_lines = build_pending_lines(
            self.turn_start,
            &live_lines,
            self.spinner_seed,
            &self.pending_steers,
            &self.pending_queue,
            width,
        );
        let lines = compact_pending_lines_for_height(raw_pending_lines, max_pending_height);
        self.pending_render_cache = Some(PendingRenderCache {
            signature,
            max_pending_height,
            lines: lines.clone(),
        });
        lines
    }
}

async fn start_turn<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    runtime: &mut CliTurnRuntime,
    input: String,
    echo_user_message: bool,
) -> CliResult<()> {
    refresh_app_cwd_dependent_state(app, runtime);
    maybe_capture_and_persist_first_turn_bootstrap_reply(app, runtime, input.as_str())?;
    let width = current_render_width(terminal)?;
    app.live_render_width.store(width.max(1), Ordering::Relaxed);
    if echo_user_message {
        app.message_list.add_user_message(input.clone());
    }
    app.composer_follow_up_intent = false;
    app.spinner_seed = spinner_seed();
    app.last_pending_signature = None;
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    app.title_attention_required = false;
    app.focus = Focus::Composer;
    clear_live_transcript(&app.live_transcript);

    terminal
        .draw(|f| app.render(f))
        .map_err(|e| format!("draw error: {}", e))?;

    let sink = {
        let live_transcript = Arc::clone(&app.live_transcript);
        Arc::new(
            move |payload: super::super::CliChatLiveSurfaceRenderPayload| {
                if let Ok(mut state) = live_transcript.lock() {
                    state.draft_preview = payload.draft_preview;
                    state.tool_activity_lines = payload.tool_activity_lines;
                }
            },
        )
    };
    let (observer, rerender) = super::super::build_cli_chat_live_compact_observer_controller(
        Arc::clone(&app.live_render_width),
        sink,
    );
    app.live_rerender = Some(rerender);
    let mut turn_runtime = runtime.clone();
    if let Some(addendum) = app.pending_first_turn_bootstrap_addendum.take() {
        apply_first_turn_bootstrap_addendum(&mut turn_runtime, addendum);
        app.awaiting_first_turn_bootstrap_reply = true;
    }
    app.pending_task = Some(spawn_pending_turn(turn_runtime, input, observer));
    Ok(())
}

fn queue_pending_steer(app: &mut App, input: String) {
    if input.trim().is_empty() {
        return;
    }
    app.pending_steers.push_back(input);
    app.focus = Focus::Composer;
}

fn queue_pending_message(app: &mut App) {
    let input = app.composer.take_input();
    if input.trim().is_empty() {
        return;
    }
    app.composer_follow_up_intent = false;
    app.pending_queue.push_back(input);
    app.focus = Focus::Composer;
}

fn dequeue_pending_steer(app: &mut App) -> bool {
    if let Some(input) = app.pending_queue.pop_back() {
        app.composer.set_input(input);
        app.focus = Focus::Composer;
        return true;
    }
    let Some(input) = app.pending_steers.pop_back() else {
        return false;
    };
    app.composer.set_input(input);
    app.focus = Focus::Composer;
    true
}

fn is_transcript_navigation_key(key: crossterm::event::KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::Up
            | KeyCode::Down
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End
    )
}

fn should_focus_composer_for_transcript_key(key: crossterm::event::KeyEvent) -> bool {
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
    {
        return false;
    }

    matches!(
        key.code,
        KeyCode::Char(_)
            | KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::Enter
            | KeyCode::Left
            | KeyCode::Right
    )
}

fn route_transcript_key_to_composer(
    app: &mut App,
    key: crossterm::event::KeyEvent,
) -> Option<String> {
    app.focus = Focus::Composer;
    let submitted = app.composer.handle_key(key);
    app.sync_inline_skill_popup();
    submitted
}

fn should_route_composer_key_to_transcript(app: &App, key: crossterm::event::KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown
    ) || (app.composer.is_empty() && is_transcript_navigation_key(key))
}

fn submitted_message_is_follow_up(app: &App, msg: &str) -> bool {
    app.pending_turn
        && app.composer_follow_up_intent
        && !msg.starts_with('/')
        && !msg.starts_with(':')
}

fn display_columns(text: &str) -> usize {
    crate::presentation::display_width(text)
}

fn truncate_right_for_width(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if display_columns(text) <= width {
        return text.to_owned();
    }
    if width == 1 {
        return "…".to_owned();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let ch_width = crate::presentation::char_display_width(ch);
        if used + ch_width > width.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out.push('…');
    out
}

fn truncate_middle_for_width(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if display_columns(text) <= width {
        return text.to_owned();
    }
    if width == 1 {
        return "…".to_owned();
    }

    let target_prefix_width = width.saturating_sub(1).div_ceil(2);
    let target_suffix_width = width.saturating_sub(1).saturating_sub(target_prefix_width);

    let mut prefix = String::new();
    let mut prefix_used = 0usize;
    for ch in text.chars() {
        let ch_width = crate::presentation::char_display_width(ch);
        if prefix_used + ch_width > target_prefix_width {
            break;
        }
        prefix.push(ch);
        prefix_used += ch_width;
    }

    let mut suffix_chars = Vec::new();
    let mut suffix_used = 0usize;
    for ch in text.chars().rev() {
        let ch_width = crate::presentation::char_display_width(ch);
        if suffix_used + ch_width > target_suffix_width {
            break;
        }
        suffix_chars.push(ch);
        suffix_used += ch_width;
    }
    suffix_chars.reverse();
    let suffix = suffix_chars.into_iter().collect::<String>();

    format!("{prefix}…{suffix}")
}

fn rect_contains_point(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

fn current_skill_token_query(composer: &Composer) -> Option<String> {
    let range = current_skill_token_range(composer)?;
    composer.text()[range]
        .strip_prefix('$')
        .map(|query| query.to_owned())
}

fn current_skill_token_range(composer: &Composer) -> Option<std::ops::Range<usize>> {
    let text = composer.text();
    let cursor = composer.cursor().min(text.len());
    if text.is_empty() {
        return None;
    }

    let before_cursor = &text[..cursor];
    let token_start = before_cursor
        .char_indices()
        .rfind(|(_, ch)| ch.is_whitespace())
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0);
    let after_cursor = &text[cursor..];
    let token_end = after_cursor
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(idx, _)| cursor + idx)
        .unwrap_or(text.len());
    if token_start >= token_end {
        return None;
    }

    let token = &text[token_start..token_end];
    token.starts_with('$').then_some(token_start..token_end)
}

fn inline_skill_replacement_text(
    text: &str,
    range: &std::ops::Range<usize>,
    replacement: &str,
) -> String {
    let should_trim_trailing_space = replacement.ends_with(' ')
        && text
            .get(range.end..)
            .and_then(|tail| tail.chars().next())
            .is_some_and(char::is_whitespace);

    if should_trim_trailing_space {
        replacement.trim_end_matches(' ').to_owned()
    } else {
        replacement.to_owned()
    }
}

fn build_status_footer_line(cwd: &str, model: &str, width: u16) -> Line<'static> {
    let width = width as usize;
    if width == 0 {
        return Line::from(String::new());
    }

    if width <= 24 {
        return single_footer_span(model, width, Style::default().fg(SURFACE_GRAY));
    }

    let mut model_text = model.to_owned();
    let mut cwd_text = cwd.to_owned();
    let mut model_width = display_columns(&model_text);
    let mut cwd_width = display_columns(&cwd_text);

    if model_width >= width {
        model_text = truncate_right_for_width(&model_text, width.saturating_sub(1).max(1));
        model_width = display_columns(&model_text);
    }

    let available_for_cwd = width.saturating_sub(model_width + 1);
    if cwd_width > available_for_cwd {
        cwd_text = truncate_middle_for_width(&cwd_text, available_for_cwd);
        cwd_width = display_columns(&cwd_text);
    }

    let mut spacer_width = width.saturating_sub(cwd_width + model_width);
    if !cwd_text.is_empty() && !model_text.is_empty() && spacer_width == 0 {
        if cwd_width > model_width {
            cwd_text = truncate_middle_for_width(&cwd_text, cwd_width.saturating_sub(1));
            cwd_width = display_columns(&cwd_text);
        } else {
            model_text = truncate_right_for_width(&model_text, model_width.saturating_sub(1));
            model_width = display_columns(&model_text);
        }
        spacer_width = width.saturating_sub(cwd_width + model_width);
    }

    Line::from(vec![
        Span::styled(cwd_text, Style::default().fg(SURFACE_GRAY)),
        Span::raw(" ".repeat(spacer_width)),
        Span::styled(model_text, Style::default().fg(SURFACE_GRAY)),
    ])
}

fn single_footer_span(text: &str, width: usize, style: Style) -> Line<'static> {
    let mut rendered = truncate_right_for_width(text, width);
    let rendered_width = display_columns(&rendered);
    if rendered_width < width {
        rendered.push_str(&" ".repeat(width - rendered_width));
    }
    Line::from(vec![Span::styled(rendered, style)])
}

fn footer_content_area(area: Rect) -> Rect {
    if area.width <= FOOTER_HORIZONTAL_INDENT {
        return area;
    }

    Rect {
        x: area.x.saturating_add(FOOTER_HORIZONTAL_INDENT),
        y: area.y,
        width: area.width.saturating_sub(FOOTER_HORIZONTAL_INDENT),
        height: area.height,
    }
}

fn build_queue_footer_line(i18n: &I18nService, queued: usize, width: u16) -> Line<'static> {
    let max_width = width as usize;
    if max_width == 0 {
        return Line::from(String::new());
    }
    if max_width <= 18 {
        let text = if queued > 0 {
            format!("queued ×{queued}")
        } else {
            i18n.text(SurfaceCopy::FooterQueueShort).to_owned()
        };
        return single_footer_span(
            text.as_str(),
            max_width,
            Style::default().fg(SURFACE_ACCENT),
        );
    }

    let hint = i18n.text(SurfaceCopy::FooterQueueHint).to_owned();
    let short_hint = i18n.text(SurfaceCopy::FooterQueueShort).to_owned();
    let suffix = if queued > 0 {
        format!(" · queued ×{queued}")
    } else {
        String::new()
    };
    let total_width = display_columns(&hint) + display_columns(&suffix);
    if total_width <= max_width {
        let mut spans = vec![Span::styled(hint, Style::default().fg(SURFACE_ACCENT))];
        if !suffix.is_empty() {
            spans.push(Span::styled(suffix, Style::default().fg(SURFACE_GRAY)));
        }
        return Line::from(spans);
    }

    let short_total_width = display_columns(&short_hint) + display_columns(&suffix);
    if short_total_width <= max_width {
        let mut spans = vec![Span::styled(
            short_hint,
            Style::default().fg(SURFACE_ACCENT),
        )];
        if !suffix.is_empty() {
            spans.push(Span::styled(suffix, Style::default().fg(SURFACE_GRAY)));
        }
        return Line::from(spans);
    }

    if display_columns(&short_hint) >= max_width {
        return Line::from(vec![Span::styled(
            truncate_right_for_width(&short_hint, max_width),
            Style::default().fg(SURFACE_ACCENT),
        )]);
    }

    let remaining = max_width.saturating_sub(display_columns(&short_hint));
    Line::from(vec![
        Span::styled(short_hint, Style::default().fg(SURFACE_ACCENT)),
        Span::styled(
            truncate_right_for_width(&suffix, remaining),
            Style::default().fg(SURFACE_GRAY),
        ),
    ])
}

fn build_restore_footer_line(i18n: &I18nService, queued: usize, width: u16) -> Line<'static> {
    let max_width = width as usize;
    if max_width == 0 {
        return Line::from(String::new());
    }
    if max_width <= 18 {
        return single_footer_span(
            format!("restore ×{queued}").as_str(),
            max_width,
            Style::default().fg(SURFACE_GRAY),
        );
    }

    let full_text = format!(
        "{} {} · queued ×{}",
        queue_restore_shortcut_label(),
        i18n.text(SurfaceCopy::FooterRestoreQueued),
        queued
    );
    let short_text = format!(
        "{} {} · ×{}",
        queue_restore_shortcut_label(),
        i18n.text(SurfaceCopy::FooterRestoreShort),
        queued
    );
    let selected = if display_columns(&full_text) <= width as usize {
        full_text
    } else {
        short_text
    };
    Line::from(vec![Span::styled(
        truncate_right_for_width(&selected, max_width),
        Style::default().fg(SURFACE_GRAY),
    )])
}

fn build_follow_footer_line(i18n: &I18nService, model: &str, width: u16) -> Line<'static> {
    let max_width = width as usize;
    if max_width == 0 {
        return Line::from(String::new());
    }
    if max_width <= 24 {
        return single_footer_span(
            i18n.text(SurfaceCopy::FooterFollowShort),
            max_width,
            Style::default().fg(SURFACE_ACCENT),
        );
    }

    let full_hint = i18n.text(SurfaceCopy::FooterFollowHint).to_owned();
    let short_hint = i18n.text(SurfaceCopy::FooterFollowShort).to_owned();
    let hint = if display_columns(&full_hint) <= max_width {
        full_hint
    } else {
        short_hint
    };

    if display_columns(&hint) >= max_width {
        return Line::from(vec![Span::styled(
            truncate_right_for_width(&hint, max_width),
            Style::default().fg(SURFACE_ACCENT),
        )]);
    }

    let available_for_model = max_width.saturating_sub(display_columns(&hint) + 1);
    let model_text = truncate_right_for_width(model, available_for_model);
    let spacer_width =
        max_width.saturating_sub(display_columns(&hint) + display_columns(&model_text));

    Line::from(vec![
        Span::styled(hint, Style::default().fg(SURFACE_ACCENT)),
        Span::raw(" ".repeat(spacer_width)),
        Span::styled(model_text, Style::default().fg(SURFACE_GRAY)),
    ])
}

fn queue_restore_shortcut_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Option + Up"
    } else {
        "Alt + Up"
    }
}

async fn build_command_lines(
    runtime: &CliTurnRuntime,
    options: &CliChatOptions,
    input: &str,
    width: usize,
) -> CliResult<Vec<String>> {
    let trimmed = input.trim();

    match trimmed {
        super::super::CLI_CHAT_HELP_COMMAND => Ok(render_chat_surface_help_lines_with_width(width)),
        super::super::CLI_CHAT_STATUS_COMMAND => {
            let summary = super::super::ops::build_cli_chat_startup_summary(runtime, options)?;
            Ok(super::super::ops::render_cli_chat_status_lines_with_width(
                &summary, width,
            ))
        }
        super::super::CLI_CHAT_HISTORY_COMMAND => {
            #[cfg(feature = "memory-sqlite")]
            {
                let history_lines = super::super::ops::load_history_lines(
                    &runtime.session_id,
                    runtime.config.memory.sliding_window,
                    runtime.conversation_binding(),
                    &runtime.memory_config,
                )
                .await?;
                Ok(super::super::ops::render_cli_chat_history_lines_with_width(
                    &runtime.session_id,
                    runtime.config.memory.sliding_window,
                    &history_lines,
                    width,
                ))
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "history",
                        "history unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        super::super::CLI_CHAT_COMPACT_COMMAND => {
            #[cfg(feature = "memory-sqlite")]
            {
                let result = super::super::ops::load_manual_compaction_result(
                    &runtime.config,
                    &runtime.session_id,
                    &runtime.turn_coordinator,
                    runtime.conversation_binding(),
                )
                .await?;
                Ok(
                    super::super::ops::render_manual_compaction_lines_with_width(
                        &runtime.session_id,
                        &result,
                        width,
                    ),
                )
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "compact",
                        "manual compaction unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        "/model" => Ok(render_model_command_lines_with_width(runtime, width)),
        "/permissions" => Ok(render_permissions_command_lines_with_width(width)),
        "/experimental" => Ok(render_experimental_command_lines_with_width(width)),
        "/themes" => Ok(render_themes_command_lines_with_width(width)),
        "/cwd" => Ok(render_cwd_command_lines_with_width(runtime, width)),
        "/language" => Ok(render_language_command_lines_with_width(width)),
        "/mcp" => Ok(render_mcp_command_lines_with_width(runtime, width)),
        "/skills" => Ok(render_skills_command_lines_with_width(runtime, width)),
        "/usage" => Ok(render_slash_command_usage_lines_with_width(width)),
        "/fast_lane_summary" => {
            #[cfg(feature = "memory-sqlite")]
            {
                let summary = crate::conversation::load_fast_lane_tool_batch_event_summary(
                    &runtime.session_id,
                    runtime.config.memory.sliding_window,
                    runtime.conversation_binding(),
                    &runtime.memory_config,
                )
                .await?;
                Ok(super::super::render_fast_lane_summary_lines_with_width(
                    &runtime.session_id,
                    runtime.config.memory.sliding_window,
                    &summary,
                    width,
                ))
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "fast_lane_summary",
                        "fast lane summary unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        "/safe_lane_summary" => {
            #[cfg(feature = "memory-sqlite")]
            {
                let summary = crate::conversation::load_safe_lane_event_summary(
                    &runtime.session_id,
                    runtime.config.memory.sliding_window,
                    runtime.conversation_binding(),
                    &runtime.memory_config,
                )
                .await?;
                Ok(super::super::render_safe_lane_summary_lines_with_width(
                    &runtime.session_id,
                    runtime.config.memory.sliding_window,
                    &runtime.config.conversation,
                    &summary,
                    width,
                ))
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "safe_lane_summary",
                        "safe lane summary unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        "/turn_checkpoint_summary" => {
            #[cfg(feature = "memory-sqlite")]
            {
                let diagnostics = runtime
                    .turn_coordinator
                    .load_production_turn_checkpoint_diagnostics_with_limit(
                        &runtime.config,
                        &runtime.session_id,
                        runtime.config.memory.sliding_window,
                        runtime.conversation_binding(),
                    )
                    .await?;
                Ok(
                    super::super::render_turn_checkpoint_summary_lines_with_width(
                        &runtime.session_id,
                        runtime.config.memory.sliding_window,
                        &diagnostics,
                        width,
                    ),
                )
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "turn_checkpoint_summary",
                        "turn checkpoint summary unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        "/turn_checkpoint_repair" => {
            #[cfg(feature = "memory-sqlite")]
            {
                let outcome = runtime
                    .turn_coordinator
                    .repair_production_turn_checkpoint_tail(
                        &runtime.config,
                        &runtime.session_id,
                        runtime.conversation_binding(),
                    )
                    .await?;
                Ok(
                    super::super::render_turn_checkpoint_repair_lines_with_width(
                        &runtime.session_id,
                        &outcome,
                        width,
                    ),
                )
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "turn_checkpoint_repair",
                        "turn checkpoint repair unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        "/sessions" => {
            #[cfg(feature = "memory-sqlite")]
            {
                Ok(render_sessions_lines(runtime, width)?)
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "sessions",
                        "session queue unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        "/subagents" | "/workers" => {
            #[cfg(feature = "memory-sqlite")]
            {
                Ok(render_workers_lines(runtime, width)?)
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "workers",
                        "worker queue unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        "/review" => {
            #[cfg(feature = "memory-sqlite")]
            {
                Ok(render_review_lines(runtime, width)?)
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "review",
                        "review queue unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        "/missions" | "/mission" => {
            #[cfg(feature = "memory-sqlite")]
            {
                Ok(render_mission_lines(runtime, width)?)
            }
            #[cfg(not(feature = "memory-sqlite"))]
            {
                Ok(
                    super::super::render_cli_chat_feature_unavailable_lines_with_width(
                        "mission",
                        "mission control unavailable: memory-sqlite feature disabled",
                        width,
                    ),
                )
            }
        }
        _ => {
            if let Some(spec) = slash_command_specs()
                .iter()
                .find(|spec| spec.command == trimmed)
            {
                Ok(render_slash_command_detail_lines_with_width(spec, width))
            } else {
                Ok(render_slash_command_usage_lines_with_width(width))
            }
        }
    }
}

fn render_slash_command_usage_lines_with_width(width: usize) -> Vec<String> {
    let command_items = slash_command_specs()
        .iter()
        .map(|spec| TuiKeyValueSpec::Plain {
            key: spec.command.to_owned(),
            value: slash_command_help_value(spec),
        })
        .collect::<Vec<_>>();

    let message_spec = TuiMessageSpec {
        role: "usage".to_owned(),
        caption: Some("slash commands".to_owned()),
        sections: vec![
            TuiSectionSpec::KeyValues {
                title: Some("commands".to_owned()),
                items: command_items,
            },
            TuiSectionSpec::Narrative {
                title: Some("navigation".to_owned()),
                lines: vec![
                    "Open this deck with / or : from an empty composer.".to_owned(),
                    "Every command stays visible in the same product order so muscle memory keeps working across releases."
                        .to_owned(),
                ],
            },
        ],
        footer_lines: vec![
            "Enter runs the command or opens its detail card without permission ceremony.".to_owned(),
        ],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn render_slash_command_detail_lines_with_width(
    spec: &super::command_palette::SlashCommandSpec,
    width: usize,
) -> Vec<String> {
    let message_spec = TuiMessageSpec {
        role: "command".to_owned(),
        caption: Some(spec.command.trim_start_matches('/').to_owned()),
        sections: vec![
            TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Info,
                title: Some("enabled".to_owned()),
                lines: vec![format!(
                    "{} is available in the command deck and keeps a stable slot in the local TUI.",
                    spec.command
                )],
            },
            TuiSectionSpec::Narrative {
                title: Some("intent".to_owned()),
                lines: vec![spec.description.to_owned()],
            },
        ],
        footer_lines: vec!["Use /usage to see the complete command deck.".to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn slash_command_help_value(spec: &super::command_palette::SlashCommandSpec) -> String {
    spec.description.to_owned()
}

fn render_model_command_lines_with_width(runtime: &CliTurnRuntime, width: usize) -> Vec<String> {
    let provider = &runtime.config.provider;
    let active_profile = runtime
        .config
        .active_provider_id()
        .unwrap_or("legacy provider");
    let reasoning_effort = provider
        .reasoning_effort
        .map(|effort| format!("{effort:?}").to_ascii_lowercase())
        .unwrap_or_else(|| "default".to_owned());

    let message_spec = TuiMessageSpec {
        role: "model".to_owned(),
        caption: Some("active model".to_owned()),
        sections: vec![TuiSectionSpec::KeyValues {
            title: Some("provider".to_owned()),
            items: vec![
                TuiKeyValueSpec::Plain {
                    key: "profile".to_owned(),
                    value: active_profile.to_owned(),
                },
                TuiKeyValueSpec::Plain {
                    key: "provider".to_owned(),
                    value: provider.kind.display_name().to_owned(),
                },
                TuiKeyValueSpec::Plain {
                    key: "model".to_owned(),
                    value: provider.model.clone(),
                },
                TuiKeyValueSpec::Plain {
                    key: "wire api".to_owned(),
                    value: format!("{:?}", provider.wire_api).to_ascii_lowercase(),
                },
                TuiKeyValueSpec::Plain {
                    key: "reasoning".to_owned(),
                    value: reasoning_effort,
                },
            ],
        }],
        footer_lines: vec![
            "Use /model <selector> to switch when you want a different model.".to_owned(),
        ],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn render_permissions_command_lines_with_width(width: usize) -> Vec<String> {
    let message_spec = TuiMessageSpec {
        role: "permissions".to_owned(),
        caption: Some("YOLO".to_owned()),
        sections: vec![
            TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Info,
                title: Some("YOLO by default".to_owned()),
                lines: vec![
                    "Hey yo, you only live once, take care.".to_owned(),
                ],
            },
            TuiSectionSpec::KeyValues {
                title: Some("default posture".to_owned()),
                items: vec![
                    TuiKeyValueSpec::Plain {
                        key: "mode".to_owned(),
                        value: "YOLO".to_owned(),
                    },
                    TuiKeyValueSpec::Plain {
                        key: "commands".to_owned(),
                        value: "enabled".to_owned(),
                    },
                    TuiKeyValueSpec::Plain {
                        key: "tools".to_owned(),
                        value: "enabled".to_owned(),
                    },
                    TuiKeyValueSpec::Plain {
                        key: "slash deck".to_owned(),
                        value: "enabled".to_owned(),
                    },
                    TuiKeyValueSpec::Plain {
                        key: "permission prompts".to_owned(),
                        value: "not part of the happy path".to_owned(),
                    },
                ],
            },
            TuiSectionSpec::Narrative {
                title: Some("behavior".to_owned()),
                lines: vec![
                    "This screen stays intentionally simple; it does not show allow/deny tables or ask the user to negotiate routine actions."
                        .to_owned(),
                ],
            },
        ],
        footer_lines: vec!["The default local TUI stays open; stricter deployments can still configure policy explicitly."
            .to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn render_experimental_command_lines_with_width(width: usize) -> Vec<String> {
    let message_spec = TuiMessageSpec {
        role: "experimental".to_owned(),
        caption: Some("experimental features".to_owned()),
        sections: vec![TuiSectionSpec::KeyValues {
            title: Some("enabled surface work".to_owned()),
            items: vec![
                TuiKeyValueSpec::Plain {
                    key: "streaming renderer".to_owned(),
                    value: "enabled".to_owned(),
                },
                TuiKeyValueSpec::Plain {
                    key: "startup animation".to_owned(),
                    value: "enabled".to_owned(),
                },
                TuiKeyValueSpec::Plain {
                    key: "markdown/diff/table preview".to_owned(),
                    value: "enabled".to_owned(),
                },
                TuiKeyValueSpec::Plain {
                    key: "resize smoothing".to_owned(),
                    value: "enabled".to_owned(),
                },
                TuiKeyValueSpec::Plain {
                    key: "slash command deck".to_owned(),
                    value: "enabled".to_owned(),
                },
                TuiKeyValueSpec::Plain {
                    key: "tool activity compaction".to_owned(),
                    value: "enabled".to_owned(),
                },
            ],
        }],
        footer_lines: vec!["No toggle ceremony in the default TUI path.".to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn render_themes_command_lines_with_width(width: usize) -> Vec<String> {
    let message_spec = TuiMessageSpec {
        role: "themes".to_owned(),
        caption: Some("theme".to_owned()),
        sections: vec![
            TuiSectionSpec::KeyValues {
                title: Some("current surface".to_owned()),
                items: vec![
                    TuiKeyValueSpec::Plain {
                        key: "palette".to_owned(),
                        value: "terminal-adaptive dark surface".to_owned(),
                    },
                    TuiKeyValueSpec::Plain {
                        key: "accent".to_owned(),
                        value: "startup blue with semantic red/green/yellow states".to_owned(),
                    },
                    TuiKeyValueSpec::Plain {
                        key: "resize".to_owned(),
                        value: "layout recalculates from viewport on every draw".to_owned(),
                    },
                ],
            },
            TuiSectionSpec::Narrative {
                title: Some("behavior".to_owned()),
                lines: vec![
                    "The default theme path is already active: dark, terminal-adaptive, and readable without extra setup."
                        .to_owned(),
                ],
            },
        ],
        footer_lines: vec!["The terminal-adaptive theme is active for this session.".to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn render_cwd_command_lines_with_width(runtime: &CliTurnRuntime, width: usize) -> Vec<String> {
    let cwd = current_working_directory_display(runtime);
    let message_spec = TuiMessageSpec {
        role: "cwd".to_owned(),
        caption: Some("working directory".to_owned()),
        sections: vec![TuiSectionSpec::KeyValues {
            title: Some("current scope".to_owned()),
            items: vec![
                TuiKeyValueSpec::Plain {
                    key: "cwd".to_owned(),
                    value: cwd,
                },
                TuiKeyValueSpec::Plain {
                    key: "session".to_owned(),
                    value: runtime.session_id.clone(),
                },
            ],
        }],
        footer_lines: vec!["Use /cwd <path> to move the chat working directory.".to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn render_language_command_lines_with_width(width: usize) -> Vec<String> {
    let language = resolve_default_language();
    let message_spec = TuiMessageSpec {
        role: "language".to_owned(),
        caption: Some("language".to_owned()),
        sections: vec![TuiSectionSpec::KeyValues {
            title: Some("current language".to_owned()),
            items: vec![TuiKeyValueSpec::Plain {
                key: "detected".to_owned(),
                value: language_label(language).to_owned(),
            }],
        }],
        footer_lines: vec!["Use /language <locale> to switch the UI language.".to_owned()],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn language_label(language: super::i18n::Language) -> &'static str {
    match language {
        super::i18n::Language::En => "English",
        super::i18n::Language::ZhCn => "简体中文",
        super::i18n::Language::ZhTw => "繁體中文",
        super::i18n::Language::Ja => "日本語",
        super::i18n::Language::Ru => "Русский",
    }
}

fn render_mcp_command_lines_with_width(runtime: &CliTurnRuntime, width: usize) -> Vec<String> {
    let mut items = runtime
        .effective_bootstrap_mcp_servers
        .iter()
        .map(|server| TuiKeyValueSpec::Plain {
            key: server.clone(),
            value: "enabled for this chat".to_owned(),
        })
        .collect::<Vec<_>>();

    if items.is_empty() {
        items.push(TuiKeyValueSpec::Plain {
            key: "configured".to_owned(),
            value: "0".to_owned(),
        });
    }

    let message_spec = TuiMessageSpec {
        role: "mcp".to_owned(),
        caption: Some("MCP".to_owned()),
        sections: vec![TuiSectionSpec::KeyValues {
            title: Some(format!(
                "servers ({})",
                runtime.effective_bootstrap_mcp_servers.len()
            )),
            items,
        }],
        footer_lines: vec![
            "Startup keeps this compact; /mcp shows the details on demand.".to_owned(),
        ],
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn render_skills_command_lines_with_width(runtime: &CliTurnRuntime, width: usize) -> Vec<String> {
    let skills = detect_available_skills(runtime.effective_working_directory.as_deref());
    let mut items = skills
        .iter()
        .take(14)
        .map(|skill| {
            let key = if let Some(alias) = skill.source_alias.as_deref() {
                format!("${} ({alias})", skill.name)
            } else {
                format!("${}", skill.name)
            };
            TuiKeyValueSpec::Plain {
                key,
                value: skill.description.clone(),
            }
        })
        .collect::<Vec<_>>();

    if items.is_empty() {
        items.push(TuiKeyValueSpec::Plain {
            key: "available".to_owned(),
            value: "0".to_owned(),
        });
    }

    let hidden_count = skills.len().saturating_sub(items.len());
    let mut footer_lines =
        vec!["Type $skill-name directly in the composer to invoke a skill.".to_owned()];
    if hidden_count > 0 {
        footer_lines.push(format!(
            "Showing 14 of {}; keep typing to filter.",
            skills.len()
        ));
    }

    let message_spec = TuiMessageSpec {
        role: "skills".to_owned(),
        caption: Some("skills".to_owned()),
        sections: vec![TuiSectionSpec::KeyValues {
            title: Some(format!("available ({})", skills.len())),
            items,
        }],
        footer_lines,
    };
    super::super::render_cli_chat_message_spec_with_width(&message_spec, width)
}

#[cfg(feature = "memory-sqlite")]
fn render_sessions_lines(runtime: &CliTurnRuntime, width: usize) -> CliResult<Vec<String>> {
    let store = ChatControlPlaneStore::new(&runtime.memory_config)?;
    let sessions = store.visible_sessions(&runtime.session_id, 24)?;
    let mut items = Vec::new();
    for session in sessions.iter().take(12) {
        items.push(TuiKeyValueSpec::Plain {
            key: session.session_id.clone(),
            value: format!(
                "{} · {} · turns={}{}",
                session.label,
                session.state,
                session.turn_count,
                session
                    .last_error
                    .as_deref()
                    .map(|error| format!(" · error={error}"))
                    .unwrap_or_default()
            ),
        });
    }
    if items.is_empty() {
        items.push(TuiKeyValueSpec::Plain {
            key: "queue".to_owned(),
            value: "No visible sessions rooted at the current scope.".to_owned(),
        });
    }
    let mut sections = vec![TuiSectionSpec::KeyValues {
        title: Some("visible lineage".to_owned()),
        items,
    }];
    if let Some(primary) = sessions.first()
        && let Some(details) = store.session_details(&primary.session_id, false)?
    {
        sections.push(TuiSectionSpec::KeyValues {
            title: Some("selected session detail".to_owned()),
            items: vec![
                TuiKeyValueSpec::Plain {
                    key: "label".to_owned(),
                    value: primary.label.clone(),
                },
                TuiKeyValueSpec::Plain {
                    key: "lineage root".to_owned(),
                    value: details
                        .lineage_root_session_id
                        .unwrap_or_else(|| "-".to_owned()),
                },
                TuiKeyValueSpec::Plain {
                    key: "lineage depth".to_owned(),
                    value: details.lineage_depth.to_string(),
                },
                TuiKeyValueSpec::Plain {
                    key: "trajectory turns".to_owned(),
                    value: details.trajectory_turn_count.to_string(),
                },
                TuiKeyValueSpec::Plain {
                    key: "events".to_owned(),
                    value: details.event_count.to_string(),
                },
                TuiKeyValueSpec::Plain {
                    key: "approvals".to_owned(),
                    value: details.approval_count.to_string(),
                },
            ],
        });
        if !details.recent_events.is_empty() {
            sections.push(TuiSectionSpec::Narrative {
                title: Some("recent events".to_owned()),
                lines: details.recent_events,
            });
        }
    }
    let message_spec = TuiMessageSpec {
        role: "sessions".to_owned(),
        caption: Some(format!("scope={}", runtime.session_id)),
        sections,
        footer_lines: vec![
            "Use /subagents for delegate lanes and /review for approvals.".to_owned(),
        ],
    };
    Ok(super::super::render_cli_chat_message_spec_with_width(
        &message_spec,
        width,
    ))
}

#[cfg(feature = "memory-sqlite")]
fn render_workers_lines(runtime: &CliTurnRuntime, width: usize) -> CliResult<Vec<String>> {
    let store = ChatControlPlaneStore::new(&runtime.memory_config)?;
    let workers = store.visible_worker_sessions(&runtime.session_id, 24)?;
    let mut items = Vec::new();
    for worker in workers.iter().take(12) {
        items.push(TuiKeyValueSpec::Plain {
            key: worker.session_id.clone(),
            value: format!(
                "{} · {} · turns={}{}",
                worker.label,
                worker.state,
                worker.turn_count,
                worker
                    .last_error
                    .as_deref()
                    .map(|error| format!(" · error={error}"))
                    .unwrap_or_default()
            ),
        });
    }
    if items.is_empty() {
        items.push(TuiKeyValueSpec::Plain {
            key: "queue".to_owned(),
            value: "No visible delegate workers in the current scope.".to_owned(),
        });
    }
    let mut sections = vec![TuiSectionSpec::KeyValues {
        title: Some("delegate lanes".to_owned()),
        items,
    }];
    if let Some(primary) = workers.first()
        && let Some(details) = store.session_details(&primary.session_id, true)?
    {
        sections.push(TuiSectionSpec::KeyValues {
            title: Some("selected worker detail".to_owned()),
            items: vec![
                TuiKeyValueSpec::Plain {
                    key: "label".to_owned(),
                    value: primary.label.clone(),
                },
                TuiKeyValueSpec::Plain {
                    key: "state".to_owned(),
                    value: primary.state.clone(),
                },
                TuiKeyValueSpec::Plain {
                    key: "turns".to_owned(),
                    value: primary.turn_count.to_string(),
                },
                TuiKeyValueSpec::Plain {
                    key: "lineage depth".to_owned(),
                    value: details.lineage_depth.to_string(),
                },
                TuiKeyValueSpec::Plain {
                    key: "delegate events".to_owned(),
                    value: details.delegate_events.len().to_string(),
                },
            ],
        });
        if !details.delegate_events.is_empty() {
            sections.push(TuiSectionSpec::Narrative {
                title: Some("delegate lifecycle".to_owned()),
                lines: details.delegate_events,
            });
        }
    }
    let message_spec = TuiMessageSpec {
        role: "workers".to_owned(),
        caption: Some(format!("scope={}", runtime.session_id)),
        sections,
        footer_lines: vec![
            "Use /sessions for the full lineage and /mission for lane rollups.".to_owned(),
        ],
    };
    Ok(super::super::render_cli_chat_message_spec_with_width(
        &message_spec,
        width,
    ))
}

#[cfg(feature = "memory-sqlite")]
fn render_review_lines(runtime: &CliTurnRuntime, width: usize) -> CliResult<Vec<String>> {
    let store = ChatControlPlaneStore::new(&runtime.memory_config)?;
    let approvals = store.approval_queue(&runtime.session_id, 16)?;
    let mut sections = Vec::new();
    let mut queue_items = Vec::new();
    for approval in approvals.iter().take(8) {
        queue_items.push(TuiKeyValueSpec::Plain {
            key: approval.approval_request_id.clone(),
            value: format!(
                "{} · {}{}{}",
                approval.tool_name,
                approval.status,
                approval
                    .reason
                    .as_deref()
                    .map(|reason| format!(" · {reason}"))
                    .unwrap_or_default(),
                approval
                    .last_error
                    .as_deref()
                    .map(|error| format!(" · error={error}"))
                    .unwrap_or_default()
            ),
        });
    }
    if queue_items.is_empty() {
        queue_items.push(TuiKeyValueSpec::Plain {
            key: "queue".to_owned(),
            value: "No approval requests are currently recorded for this session.".to_owned(),
        });
    }
    sections.push(TuiSectionSpec::KeyValues {
        title: Some("review queue".to_owned()),
        items: queue_items,
    });
    if let Some(latest) = approvals.first() {
        let mut detail_lines = vec![
            format!("tool={}", latest.tool_name),
            format!("status={}", latest.status),
            format!("turn_id={}", latest.turn_id),
            format!("requested_at={}", latest.requested_at),
        ];
        if let Some(reason) = latest.reason.as_deref() {
            detail_lines.push(format!("reason={reason}"));
        }
        if let Some(rule_id) = latest.rule_id.as_deref() {
            detail_lines.push(format!("rule_id={rule_id}"));
        }
        if let Some(error) = latest.last_error.as_deref() {
            detail_lines.push(format!("last_error={error}"));
        }
        sections.push(TuiSectionSpec::Narrative {
            title: Some("latest approval".to_owned()),
            lines: detail_lines,
        });
    }
    let message_spec = TuiMessageSpec {
        role: "review".to_owned(),
        caption: Some(format!("scope={}", runtime.session_id)),
        sections,
        footer_lines: vec![
            "Governed actions will surface approval screens here when needed.".to_owned(),
        ],
    };
    Ok(super::super::render_cli_chat_message_spec_with_width(
        &message_spec,
        width,
    ))
}

#[cfg(feature = "memory-sqlite")]
fn render_mission_lines(runtime: &CliTurnRuntime, width: usize) -> CliResult<Vec<String>> {
    let store = ChatControlPlaneStore::new(&runtime.memory_config)?;
    let sessions = store.visible_sessions(&runtime.session_id, 32)?;
    let workers = store.visible_worker_sessions(&runtime.session_id, 32)?;
    let approvals = store.approval_queue(&runtime.session_id, 32)?;
    let state_mix = summarize_state_mix(sessions.iter().map(|session| session.state.as_str()));
    let worker_mix = summarize_state_mix(workers.iter().map(|worker| worker.state.as_str()));
    let summary_items = vec![
        TuiKeyValueSpec::Plain {
            key: "scope".to_owned(),
            value: runtime.session_id.clone(),
        },
        TuiKeyValueSpec::Plain {
            key: "provider".to_owned(),
            value: runtime
                .config
                .active_provider_id()
                .unwrap_or("-")
                .to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "visible sessions".to_owned(),
            value: sessions.len().to_string(),
        },
        TuiKeyValueSpec::Plain {
            key: "delegate lanes".to_owned(),
            value: workers.len().to_string(),
        },
        TuiKeyValueSpec::Plain {
            key: "review queue".to_owned(),
            value: approvals.len().to_string(),
        },
        TuiKeyValueSpec::Plain {
            key: "session mix".to_owned(),
            value: state_mix.unwrap_or_else(|| "-".to_owned()),
        },
        TuiKeyValueSpec::Plain {
            key: "worker mix".to_owned(),
            value: worker_mix.unwrap_or_else(|| "-".to_owned()),
        },
    ];
    let recent_session_values = sessions
        .iter()
        .take(6)
        .map(|session| format!("{} ({})", session.label, session.state))
        .collect::<Vec<_>>();
    let recent_worker_values = workers
        .iter()
        .take(6)
        .map(|worker| format!("{} ({})", worker.label, worker.state))
        .collect::<Vec<_>>();
    let mut sections = vec![TuiSectionSpec::KeyValues {
        title: Some("mission control".to_owned()),
        items: summary_items,
    }];
    if !recent_session_values.is_empty() {
        sections.push(TuiSectionSpec::KeyValues {
            title: Some("recent sessions".to_owned()),
            items: vec![TuiKeyValueSpec::Csv {
                key: "sessions".to_owned(),
                values: recent_session_values,
            }],
        });
    }
    if !recent_worker_values.is_empty() {
        sections.push(TuiSectionSpec::KeyValues {
            title: Some("recent workers".to_owned()),
            items: vec![TuiKeyValueSpec::Csv {
                key: "workers".to_owned(),
                values: recent_worker_values,
            }],
        });
    }
    let message_spec = TuiMessageSpec {
        role: "mission".to_owned(),
        caption: Some("control plane".to_owned()),
        sections,
        footer_lines: vec![
            "Use /sessions, /subagents, and /review to drill into each lane.".to_owned(),
        ],
    };
    Ok(super::super::render_cli_chat_message_spec_with_width(
        &message_spec,
        width,
    ))
}

#[cfg(feature = "memory-sqlite")]
fn summarize_state_mix<'a>(states: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut counts = std::collections::BTreeMap::new();
    for state in states {
        *counts.entry(state.to_owned()).or_insert(0usize) += 1;
    }
    if counts.is_empty() {
        return None;
    }
    Some(
        counts
            .into_iter()
            .map(|(state, count)| format!("{state}={count}"))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

async fn maybe_finalize_pending_turn<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    runtime: &mut CliTurnRuntime,
) -> CliResult<bool> {
    let Some(handle) = app.pending_task.as_ref() else {
        return Ok(false);
    };
    if !handle.is_finished() {
        return Ok(false);
    }

    let handle = app
        .pending_task
        .take()
        .ok_or_else(|| "pending turn handle disappeared".to_owned())?;
    let assistant_text = handle
        .await
        .map_err(|error| format!("pending turn task failed to join: {error}"))??;
    let width = current_render_width(terminal)?;
    app.pending_turn = false;
    app.turn_start = None;
    app.live_rerender = None;
    app.composer_follow_up_intent = false;
    refresh_app_cwd_dependent_state(app, runtime);
    clear_live_transcript(&app.live_transcript);
    app.focus = Focus::Composer;
    let produced_approval_screen =
        super::super::build_cli_chat_approval_screen_spec(&assistant_text).is_some();
    app.title_attention_required = produced_approval_screen;
    if produced_approval_screen {
        app.message_list.add_rendered_lines(
            super::super::render_cli_chat_assistant_lines_with_width(&assistant_text, width),
        );
    } else {
        app.message_list.add_assistant_message(assistant_text);
    }
    if let Some(next_input) = app.pending_steers.pop_front() {
        start_turn(terminal, app, runtime, next_input, true).await?;
    } else if let Some(next_input) = app.pending_queue.pop_front() {
        start_turn(terminal, app, runtime, next_input, true).await?;
    }
    Ok(true)
}

fn current_render_width<B: Backend>(terminal: &Terminal<B>) -> CliResult<usize> {
    terminal
        .size()
        .map(|size| size.width as usize)
        .map_err(|e| format!("failed to query terminal size: {e}"))
}

fn spawn_pending_turn(
    runtime: CliTurnRuntime,
    input: String,
    observer: crate::conversation::ConversationTurnObserverHandle,
) -> JoinHandle<CliResult<String>> {
    tokio::spawn(async move {
        let result = crate::agent_runtime::AgentRuntime::new()
            .run_turn_with_runtime_and_observer(
                &runtime,
                &crate::agent_runtime::AgentTurnRequest {
                    message: input,
                    turn_mode: crate::agent_runtime::AgentTurnMode::Interactive,
                    channel_id: runtime.session_address.channel_id.clone(),
                    account_id: runtime.session_address.account_id.clone(),
                    conversation_id: runtime.session_address.conversation_id.clone(),
                    participant_id: runtime.session_address.participant_id.clone(),
                    thread_id: runtime.session_address.thread_id.clone(),
                    metadata: std::collections::BTreeMap::new(),
                    live_surface_enabled: true,
                },
                None,
                Some(observer),
            )
            .await?;
        Ok(result.output_text)
    })
}

fn clear_live_transcript(live_transcript: &Arc<StdMutex<LiveTranscriptState>>) {
    if let Ok(mut state) = live_transcript.lock() {
        *state = LiveTranscriptState::default();
    }
}

fn pending_live_lines(
    live_transcript: &Arc<StdMutex<LiveTranscriptState>>,
    max_lines: usize,
) -> Vec<String> {
    let max_lines = max_lines.max(1);
    live_transcript
        .lock()
        .map(|state| {
            let state = &state.tool_activity_lines;
            let normalize = |mut lines: Vec<String>| {
                while lines.first().is_some_and(|line| line.trim().is_empty()) {
                    lines.remove(0);
                }
                while lines.last().is_some_and(|line| line.trim().is_empty()) {
                    lines.pop();
                }

                let mut normalized = Vec::new();
                let mut last_was_blank = false;
                for line in lines {
                    let is_blank = line.trim().is_empty();
                    if is_blank && last_was_blank {
                        continue;
                    }
                    last_was_blank = is_blank;
                    normalized.push(line);
                }
                normalized
            };

            if state.len() <= max_lines {
                return normalize(state.clone());
            }

            if let Some(blank_idx) = state.iter().position(|line| line.trim().is_empty()) {
                let (reasoning_lines, trailing_lines) = state.split_at(blank_idx);
                let visible_lines = trailing_lines.get(1..).unwrap_or(&[]);
                let reasoning = reasoning_lines
                    .iter()
                    .filter(|line| !line.trim().is_empty())
                    .take((max_lines / 2).max(1))
                    .cloned()
                    .collect::<Vec<_>>();
                let visible = visible_lines
                    .iter()
                    .filter(|line| !line.trim().is_empty())
                    .take(max_lines.saturating_sub(reasoning.len() + 1))
                    .cloned()
                    .collect::<Vec<_>>();
                if !reasoning.is_empty() && !visible.is_empty() {
                    let mut lines = reasoning;
                    lines.push(String::new());
                    lines.extend(visible);
                    return normalize(lines);
                }
            }

            normalize(state.iter().take(max_lines).cloned().collect())
        })
        .unwrap_or_default()
}

fn pending_live_tool_activity_lines(
    live_transcript: &Arc<StdMutex<LiveTranscriptState>>,
    max_lines: usize,
) -> Vec<String> {
    pending_live_lines(live_transcript, max_lines)
        .into_iter()
        .filter(|line| pending_line_is_tool_activity(line))
        .collect()
}

fn provisional_assistant_text(
    live_transcript: &Arc<StdMutex<LiveTranscriptState>>,
) -> Option<String> {
    live_transcript
        .lock()
        .ok()
        .and_then(|state| state.draft_preview.clone())
        .filter(|text| !text.trim().is_empty())
}

fn pending_line_is_tool_activity(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('•')
        || trimmed.starts_with("[running]")
        || trimmed.starts_with("[pending]")
        || trimmed.starts_with("[completed]")
        || trimmed.starts_with("[failed]")
        || trimmed.starts_with("[interrupted]")
        || trimmed.starts_with("[needs_approval]")
        || trimmed.starts_with("[denied]")
        || trimmed.starts_with("request:")
        || trimmed.starts_with("args:")
        || trimmed.starts_with("stdout:")
        || trimmed.starts_with("stderr:")
        || trimmed.starts_with("file:")
        || trimmed.starts_with("metrics:")
        || trimmed.starts_with("↳ ")
}

fn pending_render_signature(app: &App) -> Option<u64> {
    if app.last_render_width == 0 || app.last_render_height == 0 {
        if !app.pending_turn {
            return None;
        }
        let start = app.turn_start?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        focus_ring_frame(start).hash(&mut hasher);
        get_spinner_verb_with_seed(start, app.spinner_seed).hash(&mut hasher);
        app.pending_steers
            .iter()
            .for_each(|message| message.hash(&mut hasher));
        app.pending_queue
            .iter()
            .for_each(|message| message.hash(&mut hasher));
        for line in pending_live_tool_activity_lines(&app.live_transcript, 6) {
            line.hash(&mut hasher);
        }
        return Some(hasher.finish());
    }

    let composer_height = app.composer.height_for_width(app.last_render_width);
    let palette_height = if matches!(app.focus, Focus::CommandPalette) {
        app.command_palette.desired_height() as u16
    } else {
        0
    };
    pending_render_signature_for_geometry(
        app,
        app.last_render_width,
        app.last_render_height,
        composer_height,
        palette_height,
    )
}

fn transcript_preview_signature(app: &App) -> Option<u64> {
    let preview = provisional_assistant_text(&app.live_transcript)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    app.last_render_width.hash(&mut hasher);
    preview.hash(&mut hasher);
    Some(hasher.finish())
}

#[cfg_attr(not(test), allow(dead_code))]
fn pending_signature_preview_budget(app: &App) -> usize {
    if app.last_render_width == 0 || app.last_render_height == 0 {
        return 6;
    }

    let composer_height = app.composer.height_for_width(app.last_render_width);
    let palette_height = if matches!(app.focus, Focus::CommandPalette) {
        app.command_palette.desired_height() as u16
    } else {
        0
    };
    pending_signature_preview_budget_for_geometry(
        app.last_render_height,
        composer_height,
        palette_height,
    )
}

fn pending_signature_preview_budget_for_geometry(
    height: u16,
    composer_height: u16,
    palette_height: u16,
) -> usize {
    let max_pending_height = pending_band_max_height(height, composer_height, palette_height);
    max_pending_height.saturating_sub(2).max(1) as usize
}

fn pending_band_max_height(height: u16, composer_height: u16, palette_height: u16) -> u16 {
    let reserved_without_pending = 1
        + composer_height
        + if palette_height > 0 {
            1 + palette_height
        } else {
            0
        }
        + 1
        + 1
        + 1;
    height.saturating_sub(reserved_without_pending).max(3)
}

fn pending_render_signature_for_geometry(
    app: &App,
    width: u16,
    height: u16,
    composer_height: u16,
    palette_height: u16,
) -> Option<u64> {
    if !app.pending_turn {
        return None;
    }
    let start = app.turn_start?;
    let max_pending_preview_lines =
        pending_signature_preview_budget_for_geometry(height, composer_height, palette_height);
    let visible_lines =
        pending_live_tool_activity_lines(&app.live_transcript, max_pending_preview_lines);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    focus_ring_frame(start).hash(&mut hasher);
    get_spinner_verb_with_seed(start, app.spinner_seed).hash(&mut hasher);
    width.hash(&mut hasher);
    height.hash(&mut hasher);
    visible_lines.hash(&mut hasher);
    app.pending_steers
        .iter()
        .for_each(|message| message.hash(&mut hasher));
    app.pending_queue
        .iter()
        .for_each(|message| message.hash(&mut hasher));
    Some(hasher.finish())
}

fn build_pending_lines(
    turn_start: Option<std::time::Instant>,
    live_lines: &[String],
    spinner_seed: u64,
    pending_steers: &VecDeque<String>,
    pending_queue: &VecDeque<String>,
    width: u16,
) -> Vec<Line<'static>> {
    let start = turn_start.unwrap_or_else(std::time::Instant::now);
    let spinner_spans = vec![
        Span::raw(" "),
        Span::styled(
            format!("{} ", focus_ring_frame(start)),
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}...", get_spinner_verb_with_seed(start, spinner_seed)),
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    let content_width = width.saturating_sub(2).max(1) as usize;
    let mut lines = Vec::new();
    let has_visible_reply_after_blank = live_lines
        .iter()
        .position(|line| line.trim().is_empty())
        .is_some_and(|blank_idx| {
            live_lines
                .iter()
                .skip(blank_idx + 1)
                .any(|line| !line.trim().is_empty())
        });
    let mut in_reasoning_block = has_visible_reply_after_blank;

    for line in live_lines {
        if line.trim().is_empty() {
            lines.push(Line::from(""));
            if has_visible_reply_after_blank {
                in_reasoning_block = false;
            }
            continue;
        }

        let style = if in_reasoning_block {
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::DIM)
        } else {
            Style::default().fg(ratatui::style::Color::White)
        };
        lines.extend(render_pending_live_line(
            line.as_str(),
            content_width,
            style,
            start,
        ));
    }
    append_pending_input_preview_lines(
        &mut lines,
        pending_steers,
        pending_queue,
        width,
        !live_lines.is_empty(),
    );
    lines.push(Line::from(""));
    lines.push(Line::from(spinner_spans));
    lines
}

fn render_pending_live_line(
    line: &str,
    content_width: usize,
    default_style: Style,
    start: std::time::Instant,
) -> Vec<Line<'static>> {
    if let Some(lines) = render_pending_tool_headline_line(line, content_width, start) {
        return lines;
    }

    if let Some(lines) = render_pending_tool_child_line(line, content_width) {
        return lines;
    }

    if let Some(lines) = render_pending_tool_sample_line(line, content_width) {
        return lines;
    }

    crate::presentation::render_wrapped_plain_display_line(line, content_width)
        .into_iter()
        .map(|wrapped| Line::from(vec![Span::raw("  "), Span::styled(wrapped, default_style)]))
        .collect()
}

fn render_pending_tool_headline_line(
    line: &str,
    content_width: usize,
    start: std::time::Instant,
) -> Option<Vec<Line<'static>>> {
    let trimmed = line.trim_start();
    let trimmed = trimmed.strip_prefix("• ").unwrap_or(trimmed);
    let (label, rest, label_style, body_style) = pending_tool_headline_parts(trimmed, start)?;
    let label_text = format!("{label} ");
    let prefix_width = 2 + crate::presentation::display_width(label_text.as_str());
    let body_width = content_width.saturating_sub(prefix_width).max(1);
    let mut wrapped =
        crate::presentation::render_wrapped_literal_display_line(rest.trim(), body_width);
    if wrapped.is_empty() {
        wrapped.push(String::new());
    }

    Some(
        wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                if index == 0 {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled("• ", Style::default().fg(SURFACE_GRAY)),
                        Span::styled(label_text.clone(), label_style),
                        Span::styled(wrapped_line, body_style),
                    ])
                } else {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::raw(" ".repeat(prefix_width)),
                        Span::styled(wrapped_line, body_style),
                    ])
                }
            })
            .collect(),
    )
}

fn pending_tool_headline_parts(
    trimmed: &str,
    start: std::time::Instant,
) -> Option<(&'static str, &str, Style, Style)> {
    if let Some(rest) = trimmed.strip_prefix("Called ") {
        return Some((
            "Called",
            rest,
            Style::default()
                .fg(pending_tool_label_color(start))
                .add_modifier(Modifier::BOLD),
            Style::default()
                .fg(pending_tool_body_color(start))
                .add_modifier(Modifier::BOLD),
        ));
    }

    if let Some(rest) = trimmed.strip_prefix("Closed ") {
        return Some((
            "Closed",
            rest,
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_GRAY),
        ));
    }

    if let Some(rest) = trimmed.strip_prefix("Approval ") {
        return Some((
            "Approval",
            rest,
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(Color::White),
        ));
    }

    if let Some(rest) = trimmed.strip_prefix("Denied ") {
        return Some((
            "Denied",
            rest,
            Style::default()
                .fg(SURFACE_RED)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_RED),
        ));
    }

    None
}

fn pending_tool_animation_frame(start: std::time::Instant) -> usize {
    if reduced_motion_enabled() {
        return PENDING_TOOL_LABEL_COLORS.len().saturating_sub(2);
    }
    pending_tool_animation_frame_for_elapsed(start.elapsed())
}

fn pending_tool_animation_frame_for_elapsed(elapsed: Duration) -> usize {
    let frame_count = PENDING_TOOL_LABEL_COLORS.len().max(1) as u64;
    ((elapsed.as_millis() as u64 / PENDING_TOOL_ANIMATION_FRAME_MS.max(1)) % frame_count) as usize
}

fn pending_tool_label_color(start: std::time::Instant) -> Color {
    let frame = pending_tool_animation_frame(start);
    *PENDING_TOOL_LABEL_COLORS
        .get(frame)
        .unwrap_or(&SURFACE_CYAN)
}

fn pending_tool_body_color(start: std::time::Instant) -> Color {
    let frame = pending_tool_animation_frame(start);
    *PENDING_TOOL_BODY_COLORS.get(frame).unwrap_or(&Color::White)
}

fn render_pending_tool_child_line(line: &str, content_width: usize) -> Option<Vec<Line<'static>>> {
    let trimmed = line.trim_start();
    let body = trimmed.strip_prefix("↳ ")?;
    let (label, rest) = body.split_once(' ').unwrap_or((body, ""));
    let label_text = if rest.is_empty() {
        String::new()
    } else {
        format!("{label} ")
    };
    let (label_style, body_style) = pending_tool_child_styles(label);
    let prefix_width = 2 + crate::presentation::display_width(label_text.as_str());
    let body_width = content_width.saturating_sub(prefix_width).max(1);
    let mut wrapped =
        crate::presentation::render_wrapped_literal_display_line(rest.trim(), body_width);
    if wrapped.is_empty() {
        wrapped.push(String::new());
    }

    Some(
        wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                if index == 0 {
                    let mut spans = vec![
                        Span::raw("  "),
                        Span::styled("↳ ", Style::default().fg(SURFACE_ACCENT)),
                    ];
                    if !label_text.is_empty() {
                        spans.push(Span::styled(label_text.clone(), label_style));
                    }
                    spans.push(Span::styled(wrapped_line, body_style));
                    Line::from(spans)
                } else {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::raw(" ".repeat(prefix_width)),
                        Span::styled(wrapped_line, body_style),
                    ])
                }
            })
            .collect(),
    )
}

fn pending_tool_child_styles(label: &str) -> (Style, Style) {
    match label {
        "stdout" => (
            Style::default()
                .fg(SURFACE_GREEN)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        "stderr" => (
            Style::default()
                .fg(SURFACE_RED)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_RED).add_modifier(Modifier::DIM),
        ),
        "file" => (
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        "metrics" => (
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        "request" | "args" => (
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        _ => (
            Style::default().fg(SURFACE_ACCENT),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
    }
}

fn render_pending_tool_sample_line(line: &str, content_width: usize) -> Option<Vec<Line<'static>>> {
    if !line.starts_with("    ") {
        return None;
    }

    let sample = line.trim_start();
    if sample.is_empty() {
        return None;
    }

    let sample_style = if sample.starts_with('+') {
        Style::default().fg(SURFACE_GREEN)
    } else if sample.starts_with('-') {
        Style::default().fg(SURFACE_RED)
    } else {
        Style::default().fg(SURFACE_DARK_GRAY)
    };
    let sample_width = content_width.saturating_sub(4).max(1);

    Some(
        crate::presentation::render_wrapped_literal_display_line(sample, sample_width)
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                let guide = if index == 0 { "    " } else { "      " };
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(guide, Style::default().fg(SURFACE_DARK_GRAY)),
                    Span::styled(wrapped_line, sample_style),
                ])
            })
            .collect(),
    )
}

fn append_pending_input_preview_lines(
    lines: &mut Vec<Line<'static>>,
    pending_steers: &VecDeque<String>,
    pending_queue: &VecDeque<String>,
    width: u16,
    has_live_preview: bool,
) {
    const MAX_PENDING_PREVIEW_MESSAGES: usize = 3;

    if pending_steers.is_empty() && pending_queue.is_empty() {
        return;
    }

    if has_live_preview || lines.last().is_some_and(|line| !line.spans.is_empty()) {
        lines.push(Line::from(""));
    }

    let content_width = width.saturating_sub(6).max(1) as usize;
    let mut remaining_preview_budget = MAX_PENDING_PREVIEW_MESSAGES;
    if !pending_steers.is_empty() {
        push_pending_input_header(
            lines,
            content_width,
            "Messages to be submitted after next tool call",
            Some("Esc"),
            "to interrupt and send immediately",
        );
        let preview_items = pending_steers
            .iter()
            .map(|message| {
                (
                    message.as_str(),
                    Style::default()
                        .fg(SURFACE_CYAN)
                        .add_modifier(Modifier::DIM),
                )
            })
            .collect::<Vec<_>>();
        let displayed = push_pending_input_lines(
            lines,
            &preview_items,
            content_width,
            "    ↳ ",
            remaining_preview_budget,
        );
        remaining_preview_budget = remaining_preview_budget.saturating_sub(displayed);
    }

    if !pending_queue.is_empty() {
        if !pending_steers.is_empty() {
            lines.push(Line::from(""));
        }
        push_pending_input_header(lines, content_width, "Queued follow-up messages", None, "");
        let preview_items = pending_queue
            .iter()
            .map(|message| {
                (
                    message.as_str(),
                    Style::default()
                        .fg(SURFACE_GRAY)
                        .add_modifier(Modifier::DIM | Modifier::ITALIC),
                )
            })
            .collect::<Vec<_>>();
        push_pending_input_lines(
            lines,
            &preview_items,
            content_width,
            "    ↳ ",
            remaining_preview_budget,
        );
    }
}

fn push_pending_input_header(
    lines: &mut Vec<Line<'static>>,
    content_width: usize,
    title: &str,
    key_hint: Option<&str>,
    suffix: &str,
) {
    let mut spans = vec![
        Span::styled(
            "• ",
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::DIM),
        ),
        Span::styled(title.to_owned(), Style::default().fg(SURFACE_GRAY)),
    ];
    if let Some(key_hint) = key_hint {
        spans.push(Span::styled(
            " (press ".to_owned(),
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::DIM),
        ));
        spans.push(Span::styled(
            key_hint.to_owned(),
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {suffix})"),
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::DIM),
        ));
    }
    for (line_index, wrapped) in crate::presentation::render_wrapped_text_line(
        "",
        &spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>(),
        content_width + 2,
    )
    .into_iter()
    .enumerate()
    {
        let prefix = if line_index == 0 { "" } else { "  " };
        lines.push(Line::from(vec![Span::styled(
            format!("{prefix}{wrapped}"),
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::DIM),
        )]));
    }
}

fn push_pending_input_lines(
    lines: &mut Vec<Line<'static>>,
    messages: &[(&str, Style)],
    content_width: usize,
    first_prefix: &str,
    max_preview_messages: usize,
) -> usize {
    let displayed_messages = messages.len().min(max_preview_messages);
    for (message, message_style) in messages.iter().take(max_preview_messages) {
        let wrapped_lines =
            crate::presentation::render_wrapped_literal_display_line(message, content_width);
        let wrapped_count = wrapped_lines.len();
        for (line_index, wrapped) in wrapped_lines.into_iter().take(3).enumerate() {
            let prefix = if line_index == 0 {
                first_prefix.to_owned()
            } else {
                "      ".to_owned()
            };
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(wrapped, *message_style),
            ]));
        }

        if wrapped_count > 3 {
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled("…".to_owned(), *message_style),
            ]));
        }
    }

    let remaining_messages = messages.len().saturating_sub(displayed_messages);
    if remaining_messages > 0 {
        lines.push(Line::from(vec![
            Span::raw("      "),
            Span::styled(
                format!("… +{remaining_messages} more"),
                Style::default()
                    .fg(SURFACE_GRAY)
                    .add_modifier(Modifier::DIM),
            ),
        ]));
    }

    displayed_messages
}

fn compact_pending_lines_for_height(
    mut lines: Vec<Line<'static>>,
    max_height: u16,
) -> Vec<Line<'static>> {
    let max_height = max_height.max(1) as usize;
    if lines.len() <= max_height {
        return lines;
    }

    let removable_blank_indices = [0usize, lines.len().saturating_sub(1), 2usize];
    for index in removable_blank_indices {
        if lines.len() <= max_height {
            break;
        }
        if lines
            .get(index)
            .is_some_and(|line| line.spans.iter().all(|span| span.content.trim().is_empty()))
        {
            lines.remove(index);
        }
    }

    while lines.len() > max_height {
        if let Some(index) = lines.iter().enumerate().skip(2).find_map(|(idx, line)| {
            line.spans
                .iter()
                .all(|span| span.content.trim().is_empty())
                .then_some(idx)
        }) {
            lines.remove(index);
        } else {
            break;
        }
    }

    lines.truncate(max_height);
    lines
}
