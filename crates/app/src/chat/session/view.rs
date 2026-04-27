use super::*;

impl ChatSessionSurface {
    pub(super) fn build_transcript_lines(
        &self,
        state: &SurfaceState,
        width: usize,
        height: usize,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        for (entry_index, entry) in state.transcript.iter().enumerate() {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            for (line_index, line) in entry.lines.iter().enumerate() {
                let clipped = clipped_display_line(line, width.saturating_sub(2));
                if line_index == 0 && state.selected_entry == Some(entry_index) {
                    lines.push(format!("▶ {clipped}"));
                } else {
                    lines.push(clipped);
                }
            }
        }

        if state.pending_turn && !lines.is_empty() {
            lines.push(String::new());
        }

        if state.pending_turn {
            let live_lines = render_cli_chat_live_surface_lines_with_width(
                &state
                    .live
                    .snapshot
                    .clone()
                    .unwrap_or_else(fallback_live_surface_snapshot),
                width,
            );
            lines.extend(
                live_lines
                    .into_iter()
                    .map(|line| clipped_display_line(&line, width)),
            );
        }

        if state.sticky_bottom {
            if lines.len() <= height {
                return lines;
            }

            let start = lines.len().saturating_sub(height);
            return lines.into_iter().skip(start).collect();
        }

        if lines.len() <= height {
            return lines;
        }

        let max_offset = lines.len().saturating_sub(height);
        let scroll_offset = min(state.scroll_offset, max_offset);
        let start = lines.len().saturating_sub(height + scroll_offset);
        lines.into_iter().skip(start).take(height).collect()
    }

    pub(super) fn build_sidebar_lines(
        &self,
        state: &SurfaceState,
        width: usize,
        height: usize,
    ) -> Vec<String> {
        if width == 0 {
            return Vec::new();
        }

        let startup_summary = state
            .startup_summary
            .clone()
            .unwrap_or_else(|| fallback_startup_summary(self.runtime.session_id.as_str()));
        let mut lines = vec![
            format!("control deck · {}", state.sidebar_tab.title()),
            format!("session {}", startup_summary.session_id),
        ];
        lines.push(format!("focus: {}", state.focus.label()));
        let tab_label = format!(
            "tabs: {} | {} | {} | {} | {} | {} | {}",
            if state.sidebar_tab == SidebarTab::Session {
                "[session]"
            } else {
                "session"
            },
            if state.sidebar_tab == SidebarTab::Runtime {
                "[runtime]"
            } else {
                "runtime"
            },
            if state.sidebar_tab == SidebarTab::Tools {
                "[tools]"
            } else {
                "tools"
            },
            if state.sidebar_tab == SidebarTab::Mission {
                "[mission]"
            } else {
                "mission"
            },
            if state.sidebar_tab == SidebarTab::Workers {
                "[workers]"
            } else {
                "workers"
            },
            if state.sidebar_tab == SidebarTab::Review {
                "[review]"
            } else {
                "review"
            },
            if state.sidebar_tab == SidebarTab::Help {
                "[help]"
            } else {
                "help"
            },
        );
        lines.extend(crate::presentation::render_wrapped_display_line(
            &tab_label, width,
        ));
        lines.push(String::new());

        match state.sidebar_tab {
            SidebarTab::Session => {
                lines.push(format!("session: {}", startup_summary.session_id));
                lines.push(format!("config: {}", startup_summary.config_path));
                lines.push(format!("memory: {}", startup_summary.memory_label));
                lines.push(format!("context: {}", startup_summary.context_engine_id));
                lines.push(format!(
                    "context src: {}",
                    startup_summary.context_engine_source
                ));
                lines.push(format!("acp backend: {}", startup_summary.acp_backend_id));
                lines.push(format!("routing: {}", startup_summary.conversation_routing));
                lines.push(format!("sticky: {}", state.sticky_bottom));
                lines.push(format!("entries: {}", state.transcript.len()));
                lines.push(format!(
                    "channels: {}",
                    if startup_summary.allowed_channels.is_empty() {
                        "-".to_owned()
                    } else {
                        startup_summary.allowed_channels.join(", ")
                    }
                ));
            }
            SidebarTab::Runtime => {
                lines.push(format!("acp: {}", startup_summary.acp_enabled));
                lines.push(format!("dispatch: {}", startup_summary.dispatch_enabled));
                lines.push(format!(
                    "event stream: {}",
                    startup_summary.event_stream_enabled
                ));
                let working_directory = startup_summary
                    .working_directory
                    .unwrap_or_else(|| "-".to_owned());
                lines.push(format!("cwd: {}", working_directory));
                lines.push(format!(
                    "phase: {}",
                    state
                        .live
                        .snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.phase.as_str())
                        .unwrap_or("idle")
                ));
                lines.push(format!(
                    "round: {}",
                    state
                        .live
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.provider_round)
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_owned())
                ));
                lines.push(format!(
                    "messages: {}",
                    state
                        .live
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.message_count)
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_owned())
                ));
                lines.push(format!(
                    "tokens: {}",
                    state
                        .live
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.estimated_tokens)
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_owned())
                ));
            }
            SidebarTab::Tools => {
                lines.push(format!(
                    "tool calls: {}",
                    state
                        .live
                        .snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.tool_call_count)
                        .unwrap_or(0)
                ));
                let tool_lines = state
                    .live
                    .snapshot
                    .as_ref()
                    .map(|snapshot| {
                        format_cli_chat_live_tool_activity_lines(snapshot.tools.as_slice())
                    })
                    .unwrap_or_default();
                if tool_lines.is_empty() {
                    lines.push("no tool activity recorded".to_owned());
                } else {
                    lines.extend(tool_lines.into_iter().take(10));
                }
            }
            SidebarTab::Mission => {
                lines.extend(self.build_mission_control_lines(state, 4, 3, 3));
            }
            SidebarTab::Workers => {
                lines.extend(self.build_worker_queue_lines(6));
            }
            SidebarTab::Review => {
                let queue_lines = self.build_review_queue_lines(4);
                lines.extend(queue_lines);
                if let Some(approval) = state.last_approval.as_ref() {
                    lines.push(String::new());
                    lines.push(format!("approval: {}", approval.title));
                    if let Some(subtitle) = approval.subtitle.as_deref() {
                        lines.push(format!("mode: {subtitle}"));
                    }
                    if !approval.request_items.is_empty() {
                        lines.push("request".to_owned());
                        lines.extend(approval.request_items.iter().take(4).cloned());
                    }
                    if !approval.rationale_lines.is_empty() {
                        lines.push("reason".to_owned());
                        lines.extend(approval.rationale_lines.iter().take(4).cloned());
                    }
                    if !approval.choice_lines.is_empty() {
                        lines.push("choices".to_owned());
                        lines.extend(approval.choice_lines.iter().take(4).cloned());
                    }
                } else if lines.is_empty() {
                    lines.push("no pending approval".to_owned());
                    lines.push("Governed actions surface here.".to_owned());
                }
            }
            SidebarTab::Help => {
                lines.push("shortcuts".to_owned());
                lines.push("Enter send".to_owned());
                lines.push("Esc clear / exit".to_owned());
                lines.push("Tab cycle focus".to_owned());
                lines.push("[ ] / Home End switch rail tab".to_owned());
                lines.push("PgUp / PgDn transcript scroll".to_owned());
                lines.push("j / k transcript move".to_owned());
                lines.push("Enter on transcript → detail".to_owned());
                lines.push("g / G transcript jump".to_owned());
                lines.push("t timeline overlay".to_owned());
                lines.push("M open mission control".to_owned());
                lines.push("r reopen latest approval".to_owned());
                lines.push("S open session queue".to_owned());
                lines.push("W open worker queue".to_owned());
                lines.push("R open approval queue".to_owned());
                lines.push("← / → / Home / End composer cursor".to_owned());
                lines.push("↑ / ↓ composer multiline move".to_owned());
                lines.push("?: help overlay".to_owned());
                lines.push(": or / command menu".to_owned());
                lines.push(
                    "/help /status /history /sessions /mission /review /workers /compact"
                        .to_owned(),
                );
            }
        }

        if let Some(preview) = state.live.last_assistant_preview.as_deref() {
            lines.push(String::new());
            lines.push("last reply".to_owned());
            lines.extend(
                crate::presentation::render_wrapped_display_line(preview, width)
                    .into_iter()
                    .take(8),
            );
        }

        if let Some(selected) = state.selected_entry
            && let Some(entry) = state.transcript.get(selected)
        {
            lines.push(String::new());
            lines.push(format!("selected entry: {}", selected + 1));
            lines.extend(
                entry
                    .lines
                    .iter()
                    .flat_map(|line| crate::presentation::render_wrapped_display_line(line, width))
                    .take(6),
            );
        }

        lines.truncate(height);
        lines
    }

    pub(super) fn build_composer_lines(&self, state: &SurfaceState, width: usize) -> Vec<String> {
        let draft_lines = composer_display_lines(
            &composer_text_with_cursor(&state.composer, state.composer_cursor),
            width.saturating_sub(2),
            2,
        );
        let prompt_line = if state.composer.is_empty() {
            format!("╭─ compose · focus={}", state.focus.label())
        } else {
            format!(
                "╭─ compose · {} chars · focus={}",
                state.composer.chars().count(),
                state.focus.label()
            )
        };
        let body_line = format!("│ {}", draft_lines.first().cloned().unwrap_or_default());
        let second_line = if draft_lines.len() > 1 {
            format!("│ {}", draft_lines.get(1).cloned().unwrap_or_default())
        } else if let Some(hint) = slash_command_hint(&state.composer) {
            format!("│ {hint}")
        } else {
            "│".to_owned()
        };
        let hint = if state.command_palette.is_some() {
            "╰─ command menu active · type filter · ↑↓ choose · Enter run · Esc close"
        } else if state.composer.starts_with('/') {
            "╰─ slash mode · Enter send command · : or / opens the command menu"
        } else if should_continue_multiline(&state.composer) {
            "╰─ multiline compose · trailing \\ inserts newline on Enter"
        } else {
            "╰─ Enter send · ? help · : or / command menu"
        };
        vec![prompt_line, body_line, second_line, hint.to_owned()]
    }

    pub(super) fn build_status_line(&self, state: &SurfaceState, width: usize) -> String {
        let mut status = format!(
            "{} · mode=chat · focus={} · deck={} · entries={} · scroll={} · sticky={} · overlay={}",
            state.footer_notice,
            state.focus.label(),
            state.sidebar_tab.title(),
            state.transcript.len(),
            state.scroll_offset,
            state.sticky_bottom,
            current_overlay_label(state)
        );
        if state.pending_turn {
            status.push_str(" · turn running");
        }
        clipped_display_line(&status, width)
    }

    pub(super) fn build_header_status_line(&self, state: &SurfaceState, width: usize) -> String {
        let session_id = state
            .startup_summary
            .as_ref()
            .map(|summary| summary.session_id.as_str())
            .unwrap_or(self.runtime.session_id.as_str());
        let acp = if self.runtime.config.acp.enabled {
            "acp:on"
        } else {
            "acp:off"
        };
        clipped_display_line(
            format!(
                "session={session_id} · provider={} · {} · focus={} · overlay={}",
                state.active_provider_label,
                acp,
                state.focus.label(),
                current_overlay_label(state)
            )
            .as_str(),
            width,
        )
    }

    #[allow(dead_code)]
    pub(super) fn build_command_palette_lines(
        &self,
        state: &SurfaceState,
        total_width: usize,
        _total_height: usize,
        transcript_height: usize,
    ) -> Option<String> {
        let palette = state.command_palette.as_ref()?;
        let filtered_items = filtered_command_palette_items(&palette.query);
        let overlay_width = COMMAND_OVERLAY_WIDTH
            .min(total_width.saturating_sub(4))
            .max(24);
        let x = total_width.saturating_sub(overlay_width + 2);
        let y = transcript_height.saturating_sub(8).max(2);
        let header = if palette.query.is_empty() {
            "╭─ command menu".to_owned()
        } else {
            format!("╭─ command menu · query={}", palette.query)
        };
        let mut lines = vec![format!("\x1b[{};{}H{}", y + 1, x + 1, header)];
        for (index, (label, detail, _)) in filtered_items.iter().enumerate() {
            let marker = if index == palette.selected { '>' } else { ' ' };
            let row = y + 2 + index;
            lines.push(format!(
                "\x1b[{};{}H│ {} {}",
                row + 1,
                x + 1,
                marker,
                pad_and_clip(label, overlay_width.saturating_sub(4))
            ));
            let detail_row = row + 1;
            lines.push(format!(
                "\x1b[{};{}H│   {}",
                detail_row + 1,
                x + 1,
                pad_and_clip(detail, overlay_width.saturating_sub(4))
            ));
        }
        if filtered_items.is_empty() {
            lines.push(format!(
                "\x1b[{};{}H│ {}",
                y + 2,
                x + 1,
                pad_and_clip(
                    "no commands match the current query",
                    overlay_width.saturating_sub(4)
                )
            ));
        }
        let bottom_row = y + 2 + filtered_items.len().max(1) * 2;
        lines.push(format!(
            "\x1b[{};{}H╰─ type to filter · Enter run · Esc close",
            bottom_row + 1,
            x + 1
        ));
        Some(lines.join(""))
    }

    #[allow(dead_code)]
    pub(super) fn build_entry_detail_overlay_lines(
        &self,
        state: &SurfaceState,
        total_width: usize,
        total_height: usize,
    ) -> Option<String> {
        let SurfaceOverlay::EntryDetails { entry_index } = state.overlay.as_ref()?.clone() else {
            return None;
        };
        let entry = state.transcript.get(entry_index)?;
        let overlay_width = min(total_width.saturating_sub(6), 80).max(28);
        let overlay_height = min(total_height.saturating_sub(6), 18).max(8);
        let x = (total_width.saturating_sub(overlay_width)) / 2;
        let y = (total_height.saturating_sub(overlay_height)) / 2;
        let mut lines = vec![format!(
            "\x1b[{};{}H╭─ entry details · #{}",
            y + 1,
            x + 1,
            entry_index + 1
        )];

        let body_width = overlay_width.saturating_sub(4);
        let mut rendered = Vec::new();
        for line in &entry.lines {
            let wrapped = crate::presentation::render_wrapped_display_line(line, body_width);
            if wrapped.is_empty() {
                rendered.push(String::new());
            } else {
                rendered.extend(wrapped);
            }
        }

        let visible_rows = overlay_height.saturating_sub(3);
        for row in 0..visible_rows {
            let rendered_line = rendered.get(row).cloned().unwrap_or_default();
            lines.push(format!(
                "\x1b[{};{}H│ {}",
                y + 2 + row,
                x + 1,
                pad_and_clip(rendered_line.as_str(), body_width)
            ));
        }
        lines.push(format!(
            "\x1b[{};{}H╰─ Esc close · j/k move · g/G jump",
            y + overlay_height - 1,
            x + 1
        ));
        Some(lines.join(""))
    }

    #[allow(dead_code)]
    pub(super) fn build_timeline_overlay_lines(
        &self,
        state: &SurfaceState,
        total_width: usize,
        total_height: usize,
    ) -> Option<String> {
        if !matches!(state.overlay, Some(SurfaceOverlay::Timeline)) {
            return None;
        }
        let overlay_width = min(total_width.saturating_sub(8), 72).max(32);
        let overlay_height = min(total_height.saturating_sub(8), 18).max(8);
        let x = (total_width.saturating_sub(overlay_width)) / 2;
        let y = (total_height.saturating_sub(overlay_height)) / 2;
        let mut lines = vec![format!("\x1b[{};{}H╭─ timeline", y + 1, x + 1)];
        let body_rows = overlay_height.saturating_sub(3);
        let selected = state
            .selected_entry
            .unwrap_or_else(|| state.transcript.len().saturating_sub(1));
        let start_index = selected.saturating_sub(body_rows / 2);

        for row in 0..body_rows {
            let entry_index = start_index.saturating_add(row);
            let label = if let Some(entry) = state.transcript.get(entry_index) {
                let title = entry
                    .lines
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "(empty entry)".to_owned());
                let prefix = if entry_index == selected { '>' } else { ' ' };
                format!("{prefix} {:>3}. {}", entry_index + 1, title)
            } else {
                String::new()
            };
            lines.push(format!(
                "\x1b[{};{}H│ {}",
                y + 2 + row,
                x + 1,
                pad_and_clip(label.as_str(), overlay_width.saturating_sub(4))
            ));
        }
        lines.push(format!(
            "\x1b[{};{}H╰─ j/k move · Enter open · Esc close",
            y + overlay_height - 1,
            x + 1
        ));
        Some(lines.join(""))
    }

    #[allow(dead_code)]
    pub(super) fn build_prompt_overlay_lines(
        &self,
        state: &SurfaceState,
        total_width: usize,
        total_height: usize,
    ) -> Option<String> {
        match state.overlay.as_ref()? {
            SurfaceOverlay::Welcome { screen } => {
                let overlay_width = min(total_width.saturating_sub(8), 92).max(40);
                let x = (total_width.saturating_sub(overlay_width)) / 2;
                let y = (total_height.saturating_sub(20)) / 2;
                let lines = render_tui_screen_spec(screen, overlay_width.saturating_sub(4), false);
                let mut rendered = vec![format!("\x1b[{};{}H╭─ welcome", y + 1, x + 1)];
                for (offset, line) in lines.into_iter().take(16).enumerate() {
                    rendered.push(format!(
                        "\x1b[{};{}H│ {}",
                        y + 2 + offset,
                        x + 1,
                        pad_and_clip(line.as_str(), overlay_width.saturating_sub(4))
                    ));
                }
                rendered.push(format!(
                    "\x1b[{};{}H╰─ Type to begin · ? help · : command menu · Esc close",
                    y + 18,
                    x + 1
                ));
                Some(rendered.join(""))
            }
            SurfaceOverlay::MissionControl { lines } => {
                let overlay_width = min(total_width.saturating_sub(8), 92).max(40);
                let x = (total_width.saturating_sub(overlay_width)) / 2;
                let y = (total_height.saturating_sub(20)) / 2;
                let mut rendered = vec![format!("\x1b[{};{}H╭─ mission control", y + 1, x + 1)];
                for (offset, line) in lines.iter().take(16).enumerate() {
                    rendered.push(format!(
                        "\x1b[{};{}H│ {}",
                        y + 2 + offset,
                        x + 1,
                        pad_and_clip(line.as_str(), overlay_width.saturating_sub(4))
                    ));
                }
                rendered.push(format!(
                    "\x1b[{};{}H╰─ Esc close · S sessions · W workers · R approvals",
                    y + 18,
                    x + 1
                ));
                Some(rendered.join(""))
            }
            SurfaceOverlay::Help => {
                let overlay_width = min(total_width.saturating_sub(10), 88).max(36);
                let x = (total_width.saturating_sub(overlay_width)) / 2;
                let y = (total_height.saturating_sub(16)) / 2;
                let lines =
                    ops::render_cli_chat_help_lines_with_width(overlay_width.saturating_sub(4));
                let mut rendered = vec![format!("\x1b[{};{}H╭─ help", y + 1, x + 1)];
                for (offset, line) in lines.into_iter().take(12).enumerate() {
                    rendered.push(format!(
                        "\x1b[{};{}H│ {}",
                        y + 2 + offset,
                        x + 1,
                        pad_and_clip(line.as_str(), overlay_width.saturating_sub(4))
                    ));
                }
                rendered.push(format!(
                    "\x1b[{};{}H╰─ Esc close · : command menu · /help send command",
                    y + 14,
                    x + 1
                ));
                Some(rendered.join(""))
            }
            SurfaceOverlay::ConfirmExit => {
                let overlay_width = min(total_width.saturating_sub(12), 56).max(28);
                let x = (total_width.saturating_sub(overlay_width)) / 2;
                let y = (total_height.saturating_sub(6)) / 2;
                Some(
                    [
                        format!("\x1b[{};{}H╭─ confirm exit", y + 1, x + 1),
                        format!(
                            "\x1b[{};{}H│ {}",
                            y + 2,
                            x + 1,
                            pad_and_clip(
                                "Press Enter to leave the session surface, or Esc to continue.",
                                overlay_width.saturating_sub(4),
                            )
                        ),
                        format!("\x1b[{};{}H╰─ Enter confirm · Esc cancel", y + 3, x + 1),
                    ]
                    .join(""),
                )
            }
            SurfaceOverlay::InputPrompt {
                kind,
                value,
                cursor,
            } => {
                let overlay_width = min(total_width.saturating_sub(10), 72).max(32);
                let x = (total_width.saturating_sub(overlay_width)) / 2;
                let y = (total_height.saturating_sub(8)) / 2;
                let title = match kind {
                    OverlayInputKind::RenameSession => "rename session",
                    OverlayInputKind::ExportTranscript => "export transcript",
                };
                let hint = match kind {
                    OverlayInputKind::RenameSession => {
                        "Set a local session title for this fullscreen surface."
                    }
                    OverlayInputKind::ExportTranscript => {
                        "Choose a file path to write the current transcript."
                    }
                };
                let input = composer_text_with_cursor(value, *cursor);
                Some(
                    [
                        format!("\x1b[{};{}H╭─ {}", y + 1, x + 1, title),
                        format!(
                            "\x1b[{};{}H│ {}",
                            y + 2,
                            x + 1,
                            pad_and_clip(hint, overlay_width.saturating_sub(4))
                        ),
                        format!(
                            "\x1b[{};{}H│ {}",
                            y + 3,
                            x + 1,
                            pad_and_clip(input.as_str(), overlay_width.saturating_sub(4))
                        ),
                        format!("\x1b[{};{}H╰─ Enter save · Esc cancel", y + 4, x + 1),
                    ]
                    .join(""),
                )
            }
            SurfaceOverlay::ApprovalPrompt { screen } => {
                let overlay_width = min(total_width.saturating_sub(10), 88).max(36);
                let x = (total_width.saturating_sub(overlay_width)) / 2;
                let y = (total_height.saturating_sub(14)) / 2;
                let lines = render_tui_screen_spec(screen, overlay_width.saturating_sub(4), false);
                let mut rendered = vec![format!("\x1b[{};{}H╭─ approval required", y + 1, x + 1)];
                for (offset, line) in lines.into_iter().take(10).enumerate() {
                    rendered.push(format!(
                        "\x1b[{};{}H│ {}",
                        y + 2 + offset,
                        x + 1,
                        pad_and_clip(line.as_str(), overlay_width.saturating_sub(4))
                    ));
                }
                rendered.push(format!(
                    "\x1b[{};{}H╰─ Type approval response in composer · Esc close",
                    y + 12,
                    x + 1
                ));
                Some(rendered.join(""))
            }
            SurfaceOverlay::ReviewQueue { .. }
            | SurfaceOverlay::ReviewDetails { .. }
            | SurfaceOverlay::SessionQueue { .. }
            | SurfaceOverlay::SessionDetails { .. }
            | SurfaceOverlay::WorkerQueue { .. }
            | SurfaceOverlay::WorkerDetails { .. }
            | SurfaceOverlay::EntryDetails { .. }
            | SurfaceOverlay::Timeline => None,
        }
    }
}
