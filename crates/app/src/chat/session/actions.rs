use super::*;

impl ChatSessionSurface {
    pub(super) fn handle_key(&self, key: Key) -> CliResult<SurfaceLoopAction> {
        match key {
            Key::CtrlC => Ok(SurfaceLoopAction::Exit),
            Key::Escape => {
                let mut state = self.lock_state();
                if matches!(state.overlay, Some(SurfaceOverlay::Welcome { .. })) {
                    state.overlay = None;
                    state.focus = SurfaceFocus::Composer;
                    drop(state);
                    self.render()?;
                    return Ok(SurfaceLoopAction::Continue);
                }
                if matches!(state.overlay, Some(SurfaceOverlay::ConfirmExit)) {
                    state.overlay = None;
                    state.focus = SurfaceFocus::Composer;
                    drop(state);
                    self.render()?;
                    return Ok(SurfaceLoopAction::Continue);
                }
                if state.overlay.is_some() {
                    state.overlay = None;
                    if state.command_palette.is_none() {
                        state.focus = SurfaceFocus::Transcript;
                    }
                    drop(state);
                    self.render()?;
                    return Ok(SurfaceLoopAction::Continue);
                }
                if state.command_palette.is_some() {
                    state.command_palette = None;
                    state.focus = SurfaceFocus::Composer;
                    state.composer.clear();
                    drop(state);
                    self.render()?;
                    return Ok(SurfaceLoopAction::Continue);
                }
                if state.composer.is_empty() {
                    state.overlay = Some(SurfaceOverlay::ConfirmExit);
                    state.focus = SurfaceFocus::Composer;
                    drop(state);
                    self.render()?;
                    return Ok(SurfaceLoopAction::Continue);
                }
                state.composer.clear();
                state.composer_cursor = 0;
                state.history_index = None;
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::Tab => {
                let mut state = self.lock_state();
                if state.command_palette.is_some() {
                    state.command_palette = None;
                    state.focus = SurfaceFocus::Composer;
                } else {
                    state.focus = state
                        .focus
                        .next(state.sidebar_visible, state.command_palette.is_some());
                }
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::BackTab => {
                let mut state = self.lock_state();
                state.sidebar_tab = state.sidebar_tab.previous();
                state.focus = SurfaceFocus::Sidebar;
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::ArrowUp => {
                let mut state = self.lock_state();
                if let Some(SurfaceOverlay::SessionQueue { selected, .. }) = state.overlay.as_mut()
                {
                    *selected = selected.saturating_sub(1);
                } else if let Some(SurfaceOverlay::ReviewQueue { selected, .. }) =
                    state.overlay.as_mut()
                {
                    *selected = selected.saturating_sub(1);
                } else if let Some(SurfaceOverlay::WorkerQueue { selected, .. }) =
                    state.overlay.as_mut()
                {
                    *selected = selected.saturating_sub(1);
                } else if let Some(palette) = state.command_palette.as_mut() {
                    palette.selected = palette.selected.saturating_sub(1);
                } else if state.focus == SurfaceFocus::Composer
                    && state.composer.contains('\n')
                    && !state.composer.is_empty()
                {
                    state.composer_cursor =
                        move_cursor_vertically(&state.composer, state.composer_cursor, -1);
                } else if state.focus == SurfaceFocus::Transcript || state.composer.is_empty() {
                    state.scroll_offset = state.scroll_offset.saturating_add(3);
                    state.sticky_bottom = false;
                    let selected = state
                        .selected_entry
                        .unwrap_or_else(|| state.transcript.len().saturating_sub(1));
                    state.selected_entry = Some(selected.saturating_sub(1));
                } else if !state.history.is_empty() {
                    let next_index = match state.history_index {
                        Some(index) => index.saturating_sub(1),
                        None => state.history.len().saturating_sub(1),
                    };
                    state.history_index = Some(next_index);
                    if let Some(entry) = state.history.get(next_index) {
                        state.composer = entry.clone();
                        state.composer_cursor = state.composer.chars().count();
                    }
                }
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::ArrowDown => {
                let mut state = self.lock_state();
                if let Some(SurfaceOverlay::SessionQueue { selected, items }) =
                    state.overlay.as_mut()
                {
                    let max_index = items.len().saturating_sub(1);
                    *selected = min(selected.saturating_add(1), max_index);
                } else if let Some(SurfaceOverlay::ReviewQueue { selected, items }) =
                    state.overlay.as_mut()
                {
                    let max_index = items.len().saturating_sub(1);
                    *selected = min(selected.saturating_add(1), max_index);
                } else if let Some(SurfaceOverlay::WorkerQueue { selected, items }) =
                    state.overlay.as_mut()
                {
                    let max_index = items.len().saturating_sub(1);
                    *selected = min(selected.saturating_add(1), max_index);
                } else if let Some(palette) = state.command_palette.as_mut() {
                    let max_index = filtered_command_palette_items(&palette.query)
                        .len()
                        .saturating_sub(1);
                    palette.selected = min(palette.selected.saturating_add(1), max_index);
                } else if state.focus == SurfaceFocus::Composer
                    && state.composer.contains('\n')
                    && !state.composer.is_empty()
                {
                    state.composer_cursor =
                        move_cursor_vertically(&state.composer, state.composer_cursor, 1);
                } else if state.focus == SurfaceFocus::Transcript || state.composer.is_empty() {
                    state.scroll_offset = state.scroll_offset.saturating_sub(3);
                    if state.scroll_offset == 0 {
                        state.sticky_bottom = true;
                    }
                    let next_selected = state
                        .selected_entry
                        .unwrap_or_else(|| state.transcript.len().saturating_sub(1))
                        .saturating_add(1);
                    state.selected_entry =
                        Some(min(next_selected, state.transcript.len().saturating_sub(1)));
                } else if let Some(index) = state.history_index {
                    let next_index = index.saturating_add(1);
                    if next_index >= state.history.len() {
                        state.history_index = None;
                        state.composer.clear();
                    } else {
                        state.history_index = Some(next_index);
                        if let Some(entry) = state.history.get(next_index) {
                            state.composer = entry.clone();
                            state.composer_cursor = state.composer.chars().count();
                        }
                    }
                }
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::Home => {
                let mut state = self.lock_state();
                if state.focus == SurfaceFocus::Composer {
                    state.composer_cursor = 0;
                } else {
                    state.sidebar_tab = state.sidebar_tab.previous();
                    state.focus = SurfaceFocus::Sidebar;
                }
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::End => {
                let mut state = self.lock_state();
                if state.focus == SurfaceFocus::Composer {
                    state.composer_cursor = state.composer.chars().count();
                } else {
                    state.sidebar_tab = state.sidebar_tab.next();
                    state.focus = SurfaceFocus::Sidebar;
                }
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::PageUp => {
                let mut state = self.lock_state();
                state.scroll_offset = state.scroll_offset.saturating_add(10);
                state.sticky_bottom = false;
                state.focus = SurfaceFocus::Transcript;
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::PageDown => {
                let mut state = self.lock_state();
                state.scroll_offset = state.scroll_offset.saturating_sub(10);
                if state.scroll_offset == 0 {
                    state.sticky_bottom = true;
                }
                state.focus = SurfaceFocus::Transcript;
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::Backspace => {
                let mut state = self.lock_state();
                if let Some(SurfaceOverlay::InputPrompt { value, cursor, .. }) =
                    state.overlay.as_mut()
                {
                    remove_char_before_cursor(value, cursor);
                } else if let Some(palette) = state.command_palette.as_mut() {
                    palette.query.pop();
                    let max_index = filtered_command_palette_items(&palette.query)
                        .len()
                        .saturating_sub(1);
                    palette.selected = min(palette.selected, max_index);
                } else {
                    let mut cursor = state.composer_cursor;
                    remove_char_before_cursor(&mut state.composer, &mut cursor);
                    state.composer_cursor = cursor;
                }
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::ArrowLeft => {
                let mut state = self.lock_state();
                if let Some(SurfaceOverlay::InputPrompt { value, cursor, .. }) =
                    state.overlay.as_mut()
                {
                    *cursor = cursor.saturating_sub(1).min(value.chars().count());
                } else if state.command_palette.is_none() && state.focus == SurfaceFocus::Composer {
                    state.composer_cursor = state.composer_cursor.saturating_sub(1);
                }
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::ArrowRight => {
                let mut state = self.lock_state();
                if let Some(SurfaceOverlay::InputPrompt { value, cursor, .. }) =
                    state.overlay.as_mut()
                {
                    *cursor = min(cursor.saturating_add(1), value.chars().count());
                } else if state.command_palette.is_none() && state.focus == SurfaceFocus::Composer {
                    state.composer_cursor = min(
                        state.composer_cursor.saturating_add(1),
                        state.composer.chars().count(),
                    );
                }
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::Enter => {
                {
                    let state = self.lock_state();
                    if matches!(state.overlay, Some(SurfaceOverlay::ConfirmExit)) {
                        return Ok(SurfaceLoopAction::Exit);
                    }
                }
                {
                    let overlay_input = {
                        let state = self.lock_state();
                        match state.overlay.as_ref() {
                            Some(SurfaceOverlay::InputPrompt {
                                kind,
                                value,
                                cursor: _,
                            }) => Some((*kind, value.clone())),
                            _ => None,
                        }
                    };
                    if let Some((kind, value)) = overlay_input {
                        self.submit_input_overlay(kind, value)?;
                        return Ok(SurfaceLoopAction::Continue);
                    }
                }
                {
                    let mut state = self.lock_state();
                    if matches!(state.overlay, Some(SurfaceOverlay::ApprovalPrompt { .. })) {
                        let response = state.composer.trim().to_owned();
                        if !response.is_empty() {
                            state.overlay = None;
                            state.focus = SurfaceFocus::Composer;
                            return Ok(SurfaceLoopAction::Submit);
                        }
                    }
                }
                {
                    let mut state = self.lock_state();
                    if state.command_palette.is_none()
                        && state.focus == SurfaceFocus::Composer
                        && should_continue_multiline_at_cursor(
                            &state.composer,
                            state.composer_cursor,
                        )
                    {
                        let mut cursor = state.composer_cursor;
                        remove_char_before_cursor(&mut state.composer, &mut cursor);
                        insert_char_at_cursor(&mut state.composer, &mut cursor, '\n');
                        state.composer_cursor = cursor;
                        drop(state);
                        self.render()?;
                        return Ok(SurfaceLoopAction::Continue);
                    }
                }
                let maybe_action = self
                    .lock_state()
                    .command_palette
                    .as_ref()
                    .and_then(|palette| {
                        filtered_command_palette_items(&palette.query)
                            .get(palette.selected)
                            .map(|item| item.2)
                    });
                if let Some(action) = maybe_action {
                    return self.execute_palette_action(action);
                }
                {
                    let mut state = self.lock_state();
                    if let Some(SurfaceOverlay::SessionQueue { selected, items }) =
                        state.overlay.as_ref()
                        && let Some(item) = items.get(*selected)
                    {
                        let detail_lines = self.build_session_detail_lines(item);
                        state.overlay = Some(SurfaceOverlay::SessionDetails {
                            title: format!("session {}", item.session_id),
                            lines: detail_lines,
                        });
                        drop(state);
                        self.render()?;
                        return Ok(SurfaceLoopAction::Continue);
                    }
                }
                {
                    let mut state = self.lock_state();
                    if let Some(SurfaceOverlay::ReviewQueue { selected, items }) =
                        state.overlay.as_ref()
                        && let Some(item) = items.get(*selected)
                    {
                        state.overlay = Some(SurfaceOverlay::ReviewDetails {
                            title: format!("approval {}", item.approval_request_id),
                            lines: item.detail_lines(),
                        });
                        drop(state);
                        self.render()?;
                        return Ok(SurfaceLoopAction::Continue);
                    }
                }
                {
                    let mut state = self.lock_state();
                    if let Some(SurfaceOverlay::WorkerQueue { selected, items }) =
                        state.overlay.as_ref()
                        && let Some(item) = items.get(*selected)
                    {
                        let detail_lines = self.build_worker_detail_lines(item);
                        state.overlay = Some(SurfaceOverlay::WorkerDetails {
                            title: format!("worker {}", item.session_id),
                            lines: detail_lines,
                        });
                        drop(state);
                        self.render()?;
                        return Ok(SurfaceLoopAction::Continue);
                    }
                }
                {
                    let mut state = self.lock_state();
                    if matches!(state.overlay, Some(SurfaceOverlay::Timeline)) {
                        let entry_index = state
                            .selected_entry
                            .or_else(|| state.transcript.len().checked_sub(1));
                        if let Some(entry_index) = entry_index {
                            state.overlay = Some(SurfaceOverlay::EntryDetails { entry_index });
                            drop(state);
                            self.render()?;
                            return Ok(SurfaceLoopAction::Continue);
                        }
                    }
                }
                let maybe_overlay = {
                    let state = self.lock_state();
                    if state.focus == SurfaceFocus::Transcript {
                        state
                            .selected_entry
                            .or_else(|| state.transcript.len().checked_sub(1))
                    } else {
                        None
                    }
                };
                if let Some(entry_index) = maybe_overlay {
                    let mut state = self.lock_state();
                    state.overlay = Some(SurfaceOverlay::EntryDetails { entry_index });
                    drop(state);
                    self.render()?;
                    return Ok(SurfaceLoopAction::Continue);
                }
                Ok(SurfaceLoopAction::Submit)
            }
            Key::Char(character) => {
                let mut state = self.lock_state();
                if matches!(state.overlay, Some(SurfaceOverlay::Welcome { .. })) {
                    state.overlay = None;
                    state.focus = SurfaceFocus::Composer;
                }
                if (character == ':' || character == '/')
                    && state.composer.is_empty()
                    && state.command_palette.is_none()
                {
                    state.command_palette = Some(CommandPaletteState::default());
                    state.focus = SurfaceFocus::CommandPalette;
                } else if character == '?'
                    && state.composer.is_empty()
                    && state.command_palette.is_none()
                {
                    state.overlay = Some(SurfaceOverlay::Help);
                    state.focus = SurfaceFocus::Transcript;
                } else if let Some(SurfaceOverlay::InputPrompt { value, cursor, .. }) =
                    state.overlay.as_mut()
                {
                    if !character.is_control() {
                        insert_char_at_cursor(value, cursor, character);
                    }
                } else if let Some(palette) = state.command_palette.as_mut() {
                    if !character.is_control() {
                        palette.query.push(character);
                        let max_index = filtered_command_palette_items(&palette.query)
                            .len()
                            .saturating_sub(1);
                        palette.selected = min(palette.selected, max_index);
                    }
                } else if character == ']' && state.composer.is_empty() {
                    state.sidebar_tab = state.sidebar_tab.next();
                    state.focus = SurfaceFocus::Sidebar;
                } else if character == '[' && state.composer.is_empty() {
                    state.sidebar_tab = state.sidebar_tab.previous();
                    state.focus = SurfaceFocus::Sidebar;
                } else if character == 't'
                    && state.composer.is_empty()
                    && state.command_palette.is_none()
                {
                    state.overlay = Some(SurfaceOverlay::Timeline);
                    state.focus = SurfaceFocus::Transcript;
                } else if character == 'M'
                    && state.composer.is_empty()
                    && state.command_palette.is_none()
                {
                    match self.try_build_mission_control_lines(&state, 10, 6, 6) {
                        Ok(lines) => {
                            state.overlay = Some(SurfaceOverlay::MissionControl { lines });
                            state.focus = SurfaceFocus::Transcript;
                        }
                        Err(error) => {
                            let lines = render_control_plane_unavailable_lines_with_width(
                                "mission",
                                "control plane",
                                error.as_str(),
                                vec![
                                    "Mission control needs a readable control-plane store before it can summarize related sessions."
                                        .to_owned(),
                                ],
                                self.content_width(),
                            );
                            push_transcript_message(&mut state, lines);
                        }
                    }
                } else if character == 'S'
                    && state.composer.is_empty()
                    && state.command_palette.is_none()
                {
                    match self.load_visible_sessions(24) {
                        Ok(items) if !items.is_empty() => {
                            state.overlay =
                                Some(SurfaceOverlay::SessionQueue { selected: 0, items });
                            state.focus = SurfaceFocus::Transcript;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            let lines = render_control_plane_unavailable_lines_with_width(
                                "sessions",
                                "queue",
                                error.as_str(),
                                vec![
                                    "Related sessions and worker lanes will appear here when the control-plane store is available."
                                        .to_owned(),
                                ],
                                self.content_width(),
                            );
                            push_transcript_message(&mut state, lines);
                        }
                    }
                } else if character == 'r'
                    && state.composer.is_empty()
                    && state.command_palette.is_none()
                {
                    if let Some(approval) = state.last_approval.as_ref() {
                        state.overlay = Some(SurfaceOverlay::ApprovalPrompt {
                            screen: approval.screen_spec(),
                        });
                        state.focus = SurfaceFocus::Transcript;
                    } else {
                        match self.load_review_queue_items(24) {
                            Ok(items) if !items.is_empty() => {
                                state.overlay =
                                    Some(SurfaceOverlay::ReviewQueue { selected: 0, items });
                                state.focus = SurfaceFocus::Transcript;
                            }
                            Ok(_) => {}
                            Err(error) => {
                                let lines = render_control_plane_unavailable_lines_with_width(
                                    "review",
                                    "queue",
                                    error.as_str(),
                                    vec![
                                        "Governed actions will appear here after a turn pauses for approval and the control-plane store is readable."
                                            .to_owned(),
                                    ],
                                    self.content_width(),
                                );
                                push_transcript_message(&mut state, lines);
                            }
                        }
                    }
                } else if character == 'R'
                    && state.composer.is_empty()
                    && state.command_palette.is_none()
                {
                    match self.load_review_queue_items(24) {
                        Ok(items) if !items.is_empty() => {
                            state.overlay =
                                Some(SurfaceOverlay::ReviewQueue { selected: 0, items });
                            state.focus = SurfaceFocus::Transcript;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            let lines = render_control_plane_unavailable_lines_with_width(
                                "review",
                                "queue",
                                error.as_str(),
                                vec![
                                    "Governed actions will appear here after a turn pauses for approval and the control-plane store is readable."
                                        .to_owned(),
                                ],
                                self.content_width(),
                            );
                            push_transcript_message(&mut state, lines);
                        }
                    }
                } else if character == 'W'
                    && state.composer.is_empty()
                    && state.command_palette.is_none()
                {
                    match self.load_visible_worker_sessions(24) {
                        Ok(items) if !items.is_empty() => {
                            state.overlay =
                                Some(SurfaceOverlay::WorkerQueue { selected: 0, items });
                            state.focus = SurfaceFocus::Transcript;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            let lines = render_control_plane_unavailable_lines_with_width(
                                "workers",
                                "queue",
                                error.as_str(),
                                vec![
                                    "Async delegate or worker sessions will appear here when the control-plane store is readable."
                                        .to_owned(),
                                ],
                                self.content_width(),
                            );
                            push_transcript_message(&mut state, lines);
                        }
                    }
                } else if matches!(state.overlay, Some(SurfaceOverlay::ApprovalPrompt { .. })) {
                    let quick_response = character.to_string();
                    let quick_response_action =
                        crate::conversation::parse_approval_prompt_action_input(
                            quick_response.as_str(),
                        );

                    if quick_response_action.is_some() {
                        let mut cursor = state.composer_cursor;
                        insert_char_at_cursor(&mut state.composer, &mut cursor, character);
                        state.composer_cursor = cursor;
                        state.overlay = None;
                        state.focus = SurfaceFocus::Composer;
                        drop(state);
                        self.render()?;
                        return Ok(SurfaceLoopAction::Submit);
                    }
                } else if (character == 'j' || character == 'k')
                    && state.focus == SurfaceFocus::Transcript
                    && state.command_palette.is_none()
                {
                    if state.transcript.is_empty() {
                        state.selected_entry = None;
                        state.scroll_offset = 0;
                        state.sticky_bottom = true;
                    } else {
                        let transcript_height = self.transcript_viewport_height_for_state(&state);
                        let current = state
                            .selected_entry
                            .unwrap_or_else(|| state.transcript.len().saturating_sub(1));
                        if character == 'j' {
                            let next_selected = min(
                                current.saturating_add(1),
                                state.transcript.len().saturating_sub(1),
                            );
                            state.selected_entry = Some(next_selected);
                        } else {
                            let next_selected = current.saturating_sub(1);
                            state.selected_entry = Some(next_selected);
                        }
                        if let Some(selected_entry) = state.selected_entry {
                            let aligned_offset = align_scroll_offset_to_selected_entry(
                                &state.transcript,
                                selected_entry,
                                transcript_height,
                                state.scroll_offset,
                            );
                            state.scroll_offset = aligned_offset;
                        }
                        state.sticky_bottom = state.scroll_offset == 0;
                    }
                } else if character == 'g'
                    && state.focus == SurfaceFocus::Transcript
                    && state.command_palette.is_none()
                {
                    if state.transcript.is_empty() {
                        state.selected_entry = None;
                        state.scroll_offset = 0;
                        state.sticky_bottom = true;
                    } else {
                        state.selected_entry = Some(0);
                        state.sticky_bottom = false;
                        let transcript_height = self.transcript_viewport_height_for_state(&state);
                        let aligned_offset = align_scroll_offset_to_selected_entry(
                            &state.transcript,
                            0,
                            transcript_height,
                            state.scroll_offset,
                        );
                        state.scroll_offset = aligned_offset;
                    }
                } else if character == 'G'
                    && state.focus == SurfaceFocus::Transcript
                    && state.command_palette.is_none()
                {
                    state.selected_entry = state.transcript.len().checked_sub(1);
                    state.scroll_offset = 0;
                    state.sticky_bottom = true;
                } else {
                    let mut cursor = state.composer_cursor;
                    insert_char_at_cursor(&mut state.composer, &mut cursor, character);
                    state.composer_cursor = cursor;
                    state.focus = SurfaceFocus::Composer;
                }
                state.history_index = None;
                drop(state);
                self.render()?;
                Ok(SurfaceLoopAction::Continue)
            }
            Key::Unknown
            | Key::UnknownEscSeq(_)
            | Key::Alt
            | Key::Del
            | Key::Shift
            | Key::Insert => Ok(SurfaceLoopAction::Continue),
            _ => Ok(SurfaceLoopAction::Continue),
        }
    }

    pub(super) fn execute_palette_action(
        &self,
        action: CommandPaletteAction,
    ) -> CliResult<SurfaceLoopAction> {
        let mut state = self.lock_state();
        state.command_palette = None;
        match action {
            CommandPaletteAction::Help => {
                return Ok(SurfaceLoopAction::RunCommand(
                    CLI_CHAT_HELP_COMMAND.to_owned(),
                ));
            }
            CommandPaletteAction::Status => {
                return Ok(SurfaceLoopAction::RunCommand(
                    CLI_CHAT_STATUS_COMMAND.to_owned(),
                ));
            }
            CommandPaletteAction::History => {
                return Ok(SurfaceLoopAction::RunCommand(
                    CLI_CHAT_HISTORY_COMMAND.to_owned(),
                ));
            }
            CommandPaletteAction::SessionQueue => match self.load_visible_sessions(24) {
                Ok(items) if items.is_empty() => {
                    push_transcript_message(
                            &mut state,
                            render_cli_chat_message_spec_with_width(
                                &TuiMessageSpec {
                                    role: "sessions".to_owned(),
                                    caption: Some("queue".to_owned()),
                                    sections: vec![TuiSectionSpec::Callout {
                                        tone: TuiCalloutTone::Info,
                                        title: Some("no visible sessions".to_owned()),
                                        lines: vec![
                                            "No visible sessions are currently rooted at this session scope."
                                                .to_owned(),
                                        ],
                                    }],
                                    footer_lines: vec![
                                        "Related sessions and worker lanes will appear here when they exist."
                                            .to_owned(),
                                    ],
                                },
                                self.content_width(),
                            ),
                        );
                }
                Ok(items) => {
                    state.overlay = Some(SurfaceOverlay::SessionQueue { selected: 0, items });
                    state.focus = SurfaceFocus::Transcript;
                }
                Err(error) => {
                    push_transcript_message(
                            &mut state,
                            render_control_plane_unavailable_lines_with_width(
                                "sessions",
                                "queue",
                                error.as_str(),
                                vec![
                                    "Related sessions and worker lanes will appear here when the control-plane store is available."
                                        .to_owned(),
                                ],
                                self.content_width(),
                            ),
                        );
                }
            },
            CommandPaletteAction::Compact => {
                return Ok(SurfaceLoopAction::RunCommand(
                    CLI_CHAT_COMPACT_COMMAND.to_owned(),
                ));
            }
            CommandPaletteAction::Timeline => {
                state.overlay = Some(SurfaceOverlay::Timeline);
                state.focus = SurfaceFocus::Transcript;
            }
            CommandPaletteAction::MissionControl => {
                match self.try_build_mission_control_lines(&state, 10, 6, 6) {
                    Ok(lines) => {
                        state.overlay = Some(SurfaceOverlay::MissionControl { lines });
                        state.focus = SurfaceFocus::Transcript;
                    }
                    Err(error) => {
                        push_transcript_message(
                            &mut state,
                            render_control_plane_unavailable_lines_with_width(
                                "mission",
                                "control plane",
                                error.as_str(),
                                vec![
                                    "Mission control needs a readable control-plane store before it can summarize related sessions."
                                        .to_owned(),
                                ],
                                self.content_width(),
                            ),
                        );
                    }
                }
            }
            CommandPaletteAction::ReviewApproval => {
                if let Some(approval) = state.last_approval.as_ref() {
                    state.overlay = Some(SurfaceOverlay::ApprovalPrompt {
                        screen: approval.screen_spec(),
                    });
                    state.focus = SurfaceFocus::Transcript;
                } else {
                    push_transcript_message(
                        &mut state,
                        render_cli_chat_message_spec_with_width(
                            &TuiMessageSpec {
                                role: "system".to_owned(),
                                caption: Some("review".to_owned()),
                                sections: vec![TuiSectionSpec::Callout {
                                    tone: TuiCalloutTone::Info,
                                    title: Some("no pending approval".to_owned()),
                                    lines: vec![
                                        "The latest turn does not have an approval screen to reopen."
                                            .to_owned(),
                                    ],
                                }],
                                footer_lines: vec![
                                    "Approvals appear automatically when governed actions pause the turn."
                                        .to_owned(),
                                ],
                            },
                            self.content_width(),
                        ),
                    );
                }
            }
            CommandPaletteAction::ReviewQueue => match self.load_review_queue_items(24) {
                Ok(items) if items.is_empty() => {
                    push_transcript_message(
                            &mut state,
                            render_cli_chat_message_spec_with_width(
                                &TuiMessageSpec {
                                    role: "review".to_owned(),
                                    caption: Some("queue".to_owned()),
                                    sections: vec![TuiSectionSpec::Callout {
                                        tone: TuiCalloutTone::Info,
                                        title: Some("approval queue empty".to_owned()),
                                        lines: vec![
                                            "No approval requests are currently recorded for this session."
                                                .to_owned(),
                                        ],
                                    }],
                                    footer_lines: vec![
                                        "Governed actions will appear here after a turn pauses for approval."
                                            .to_owned(),
                                    ],
                                },
                                self.content_width(),
                            ),
                        );
                }
                Ok(items) => {
                    state.overlay = Some(SurfaceOverlay::ReviewQueue { selected: 0, items });
                    state.focus = SurfaceFocus::Transcript;
                }
                Err(error) => {
                    push_transcript_message(
                            &mut state,
                            render_control_plane_unavailable_lines_with_width(
                                "review",
                                "queue",
                                error.as_str(),
                                vec![
                                    "Governed actions will appear here after a turn pauses for approval and the control-plane store is readable."
                                        .to_owned(),
                                ],
                                self.content_width(),
                            ),
                        );
                }
            },
            CommandPaletteAction::WorkerQueue => match self.load_visible_worker_sessions(24) {
                Ok(items) if items.is_empty() => {
                    push_transcript_message(
                            &mut state,
                            render_cli_chat_message_spec_with_width(
                                &TuiMessageSpec {
                                    role: "workers".to_owned(),
                                    caption: Some("queue".to_owned()),
                                    sections: vec![TuiSectionSpec::Callout {
                                        tone: TuiCalloutTone::Info,
                                        title: Some("no visible worker sessions".to_owned()),
                                        lines: vec![
                                            "No delegate child sessions are currently visible from this session scope."
                                                .to_owned(),
                                        ],
                                    }],
                                    footer_lines: vec![
                                        "Async delegate or worker sessions will appear here after they are spawned."
                                            .to_owned(),
                                    ],
                                },
                                self.content_width(),
                            ),
                        );
                }
                Ok(items) => {
                    state.overlay = Some(SurfaceOverlay::WorkerQueue { selected: 0, items });
                    state.focus = SurfaceFocus::Transcript;
                }
                Err(error) => {
                    push_transcript_message(
                            &mut state,
                            render_control_plane_unavailable_lines_with_width(
                                "workers",
                                "queue",
                                error.as_str(),
                                vec![
                                    "Async delegate or worker sessions will appear here when the control-plane store is readable."
                                        .to_owned(),
                                ],
                                self.content_width(),
                            ),
                        );
                }
            },
            CommandPaletteAction::RenameSession => {
                let initial = state
                    .session_title_override
                    .clone()
                    .unwrap_or_else(|| self.runtime.session_id.clone());
                state.overlay = Some(SurfaceOverlay::InputPrompt {
                    kind: OverlayInputKind::RenameSession,
                    cursor: initial.chars().count(),
                    value: initial,
                });
                state.focus = SurfaceFocus::Composer;
            }
            CommandPaletteAction::ExportTranscript => {
                let initial = default_export_path(self.runtime.session_id.as_str());
                state.overlay = Some(SurfaceOverlay::InputPrompt {
                    kind: OverlayInputKind::ExportTranscript,
                    cursor: initial.chars().count(),
                    value: initial,
                });
                state.focus = SurfaceFocus::Composer;
            }
            CommandPaletteAction::JumpLatest => {
                state.sticky_bottom = true;
                state.scroll_offset = 0;
                state.selected_entry = state.transcript.len().checked_sub(1);
                state.focus = SurfaceFocus::Transcript;
            }
            CommandPaletteAction::ToggleSticky => {
                state.sticky_bottom = !state.sticky_bottom;
                if state.sticky_bottom {
                    state.scroll_offset = 0;
                    state.selected_entry = state.transcript.len().checked_sub(1);
                }
                state.focus = SurfaceFocus::Transcript;
            }
            CommandPaletteAction::ToggleSidebar => {
                state.sidebar_visible = !state.sidebar_visible;
                state.focus = if state.sidebar_visible {
                    SurfaceFocus::Sidebar
                } else {
                    SurfaceFocus::Composer
                };
            }
            CommandPaletteAction::CycleSidebarTab => {
                state.sidebar_tab = state.sidebar_tab.next();
                state.focus = SurfaceFocus::Sidebar;
            }
            CommandPaletteAction::ClearComposer => {
                state.composer.clear();
                state.composer_cursor = 0;
                state.focus = SurfaceFocus::Composer;
            }
            CommandPaletteAction::Exit => return Ok(SurfaceLoopAction::Exit),
        }
        state.command_palette = None;
        drop(state);
        self.render()?;
        Ok(SurfaceLoopAction::Continue)
    }

    pub(super) async fn handle_command(&self, input: &str) -> CliResult<()> {
        let width = self.content_width();
        let help_match = classify_chat_command_match_result(parse_exact_chat_command(
            input,
            &[CLI_CHAT_HELP_COMMAND],
            "usage: /help",
        ))?;

        let status_match =
            classify_chat_command_match_result(ops::is_cli_chat_status_command(input))?;

        let compact_match =
            classify_chat_command_match_result(ops::is_manual_compaction_command(input))?;

        let history_match = classify_chat_command_match_result(parse_exact_chat_command(
            input,
            &[CLI_CHAT_HISTORY_COMMAND],
            "usage: /history",
        ))?;

        let sessions_match = classify_chat_command_match_result(parse_exact_chat_command(
            input,
            &[CLI_CHAT_SESSIONS_COMMAND],
            "usage: /sessions",
        ))?;

        let mission_match = classify_chat_command_match_result(parse_exact_chat_command(
            input,
            &[CLI_CHAT_MISSION_COMMAND],
            "usage: /mission",
        ))?;

        let review_match = classify_chat_command_match_result(parse_exact_chat_command(
            input,
            &[CLI_CHAT_REVIEW_COMMAND],
            "usage: /review",
        ))?;

        let workers_match = classify_chat_command_match_result(parse_exact_chat_command(
            input,
            &[CLI_CHAT_WORKERS_COMMAND],
            "usage: /workers",
        ))?;

        let turn_checkpoint_repair_match =
            classify_chat_command_match_result(ops::is_turn_checkpoint_repair_command(input))?;

        if !matches!(sessions_match, ChatCommandMatchResult::NotMatched) {
            let lines = match sessions_match {
                ChatCommandMatchResult::Matched => match self.load_visible_sessions(24) {
                    Ok(items) => {
                        let session_queue_lines = items
                            .iter()
                            .take(12)
                            .map(SessionQueueItemSummary::list_line)
                            .collect::<Vec<_>>();
                        if !items.is_empty() {
                            let mut state = self.lock_state();
                            state.overlay =
                                Some(SurfaceOverlay::SessionQueue { selected: 0, items });
                        }
                        render_cli_chat_message_spec_with_width(
                            &TuiMessageSpec {
                                role: "sessions".to_owned(),
                                caption: Some("visible lineage".to_owned()),
                                sections: vec![TuiSectionSpec::Narrative {
                                    title: Some("queue".to_owned()),
                                    lines: if session_queue_lines.is_empty() {
                                        vec![
                                            "No visible sessions are currently rooted at this session scope."
                                                .to_owned(),
                                        ]
                                    } else {
                                        session_queue_lines
                                    },
                                }],
                                footer_lines: vec![
                                    "Use S to open the session queue overlay or Enter on the queue to inspect one session."
                                        .to_owned(),
                                ],
                            },
                            width,
                        )
                    }
                    Err(error) => render_control_plane_unavailable_lines_with_width(
                        "sessions",
                        "visible lineage",
                        error.as_str(),
                        vec![
                            "Use /status or restore the control-plane store before inspecting related sessions."
                                .to_owned(),
                        ],
                        width,
                    ),
                },
                ChatCommandMatchResult::UsageError(usage) => {
                    render_cli_chat_command_usage_lines_with_width(&usage, width)
                }
                ChatCommandMatchResult::NotMatched => {
                    render_cli_chat_command_usage_lines_with_width("usage: /sessions", width)
                }
            };

            let mut state = self.lock_state();
            push_transcript_message(&mut state, lines);
            return Ok(());
        }

        if !matches!(mission_match, ChatCommandMatchResult::NotMatched) {
            let lines = match mission_match {
                ChatCommandMatchResult::Matched => {
                    let mission_result = {
                        let state = self.lock_state();
                        self.try_build_mission_control_lines(&state, 10, 6, 6)
                    };
                    match mission_result {
                        Ok(mission_lines) => {
                            let mut state = self.lock_state();
                            state.overlay = Some(SurfaceOverlay::MissionControl {
                                lines: mission_lines.clone(),
                            });
                            state.focus = SurfaceFocus::Transcript;
                            render_cli_chat_message_spec_with_width(
                                &TuiMessageSpec {
                                    role: "mission".to_owned(),
                                    caption: Some("control plane".to_owned()),
                                    sections: vec![TuiSectionSpec::Narrative {
                                        title: Some("overview".to_owned()),
                                        lines: mission_lines,
                                    }],
                                    footer_lines: vec![
                                        "Use M to reopen mission control, S for sessions, W for workers, and R for approvals."
                                            .to_owned(),
                                    ],
                                },
                                width,
                            )
                        }
                        Err(error) => render_control_plane_unavailable_lines_with_width(
                            "mission",
                            "control plane",
                            error.as_str(),
                            vec![
                                "Mission control needs a readable control-plane store before it can summarize related sessions."
                                    .to_owned(),
                            ],
                            width,
                        ),
                    }
                }
                ChatCommandMatchResult::UsageError(usage) => {
                    render_cli_chat_command_usage_lines_with_width(&usage, width)
                }
                ChatCommandMatchResult::NotMatched => {
                    render_cli_chat_command_usage_lines_with_width("usage: /mission", width)
                }
            };

            let mut state = self.lock_state();
            push_transcript_message(&mut state, lines);
            return Ok(());
        }

        let lines = match help_match {
            ChatCommandMatchResult::Matched => {
                ops::render_cli_chat_help_lines_with_width(width)
            }
            ChatCommandMatchResult::UsageError(usage) => {
                render_cli_chat_command_usage_lines_with_width(&usage, width)
            }
            ChatCommandMatchResult::NotMatched => match status_match {
                ChatCommandMatchResult::Matched => {
                    let summary = ops::build_cli_chat_startup_summary(
                        &self.runtime,
                        &self.options,
                    )?;
                    ops::render_cli_chat_status_lines_with_width(&summary, width)
                }
                ChatCommandMatchResult::UsageError(usage) => {
                    render_cli_chat_command_usage_lines_with_width(&usage, width)
                }
                ChatCommandMatchResult::NotMatched => match compact_match {
                    ChatCommandMatchResult::Matched => {
                        #[cfg(feature = "memory-sqlite")]
                        {
                            let binding =
                                self.runtime.conversation_binding();
                            let result = ops::load_manual_compaction_result(
                                &self.runtime.config,
                                &self.runtime.session_id,
                                &self.runtime.turn_coordinator,
                                binding,
                            )
                            .await?;
                            ops::render_manual_compaction_lines_with_width(
                                &self.runtime.session_id,
                                &result,
                                width,
                            )
                        }
                        #[cfg(not(feature = "memory-sqlite"))]
                        {
                            render_cli_chat_feature_unavailable_lines_with_width(
                                "compact",
                                "manual compaction unavailable: memory-sqlite feature disabled",
                                width,
                            )
                        }
                    }
                    ChatCommandMatchResult::UsageError(usage) => {
                        render_cli_chat_command_usage_lines_with_width(&usage, width)
                    }
                    ChatCommandMatchResult::NotMatched => match history_match {
                        ChatCommandMatchResult::Matched => {
                            #[cfg(feature = "memory-sqlite")]
                            {
                                let history_lines = ops::load_history_lines(
                                    &self.runtime.session_id,
                                    self.runtime.config.memory.sliding_window,
                                    self.runtime.conversation_binding(),
                                    &self.runtime.memory_config,
                                )
                                .await?;
                                ops::render_cli_chat_history_lines_with_width(
                                    &self.runtime.session_id,
                                    self.runtime.config.memory.sliding_window,
                                    &history_lines,
                                    width,
                                )
                            }
                            #[cfg(not(feature = "memory-sqlite"))]
                            {
                                render_cli_chat_feature_unavailable_lines_with_width(
                                    "history",
                                    "history unavailable: memory-sqlite feature disabled",
                                    width,
                                )
                            }
                        }
                        ChatCommandMatchResult::UsageError(usage) => {
                            render_cli_chat_command_usage_lines_with_width(&usage, width)
                        }
                        ChatCommandMatchResult::NotMatched => match review_match {
                            ChatCommandMatchResult::Matched => {
                                let maybe_lines = {
                                    let state = self.lock_state();
                                    state.last_approval.as_ref().map(|approval| {
                                        let review_queue_lines = match self.load_review_queue_items(6) {
                                            Ok(items) if items.is_empty() => vec!["approval queue: empty".to_owned()],
                                            Ok(items) => build_review_queue_lines_from_items(&items),
                                            Err(error) => vec![format!(
                                                "approval queue unavailable: {error}"
                                            )],
                                        };
                                        render_cli_chat_message_spec_with_width(
                                            &TuiMessageSpec {
                                                role: "review".to_owned(),
                                                caption: Some("latest approval".to_owned()),
                                                sections: vec![
                                                    TuiSectionSpec::Narrative {
                                                        title: Some("queue".to_owned()),
                                                        lines: review_queue_lines,
                                                    },
                                                    TuiSectionSpec::Narrative {
                                                        title: Some("title".to_owned()),
                                                        lines: vec![approval.title.clone()],
                                                    },
                                                    TuiSectionSpec::Narrative {
                                                        title: Some("request".to_owned()),
                                                        lines: approval.request_items.clone(),
                                                    },
                                                    TuiSectionSpec::Narrative {
                                                        title: Some("reason".to_owned()),
                                                        lines: approval.rationale_lines.clone(),
                                                    },
                                                    TuiSectionSpec::Narrative {
                                                        title: Some("choices".to_owned()),
                                                        lines: approval.choice_lines.clone(),
                                                    },
                                                ],
                                                footer_lines: approval.footer_lines.clone(),
                                            },
                                            width,
                                        )
                                    })
                                };

                                if let Some(lines) = maybe_lines {
                                    let mut state = self.lock_state();
                                    if let Some(approval) = state.last_approval.as_ref() {
                                        state.overlay = Some(SurfaceOverlay::ApprovalPrompt {
                                            screen: approval.screen_spec(),
                                        });
                                    }
                                    lines
                                } else {
                                    match self.load_review_queue_items(6) {
                                        Ok(items) => {
                                            let review_queue_lines = if items.is_empty() {
                                                vec!["approval queue: empty".to_owned()]
                                            } else {
                                                build_review_queue_lines_from_items(&items)
                                            };
                                            render_cli_chat_message_spec_with_width(
                                                &TuiMessageSpec {
                                                    role: "review".to_owned(),
                                                    caption: Some("latest approval".to_owned()),
                                                    sections: vec![
                                                        TuiSectionSpec::Narrative {
                                                            title: Some("queue".to_owned()),
                                                            lines: review_queue_lines,
                                                        },
                                                        TuiSectionSpec::Callout {
                                                            tone: TuiCalloutTone::Info,
                                                            title: Some("no retained approval screen".to_owned()),
                                                            lines: vec![
                                                                "No approval/review item is currently retained in this session surface."
                                                                    .to_owned(),
                                                            ],
                                                        },
                                                    ],
                                                    footer_lines: vec![
                                                        "Governed actions automatically surface review screens when needed."
                                                            .to_owned(),
                                                    ],
                                                },
                                                width,
                                            )
                                        }
                                        Err(error) => render_control_plane_unavailable_lines_with_width(
                                            "review",
                                            "latest approval",
                                            error.as_str(),
                                            vec![
                                                "Governed actions automatically surface review screens when the control-plane store is readable."
                                                    .to_owned(),
                                            ],
                                            width,
                                        ),
                                    }
                                }
                            }
                            ChatCommandMatchResult::UsageError(usage) => {
                                render_cli_chat_command_usage_lines_with_width(&usage, width)
                            }
                            ChatCommandMatchResult::NotMatched => match workers_match {
                                ChatCommandMatchResult::Matched => match self.load_visible_worker_sessions(24) {
                                    Ok(items) => {
                                        let worker_queue_lines = items
                                            .iter()
                                            .take(12)
                                            .map(WorkerQueueItemSummary::list_line)
                                            .collect::<Vec<_>>();
                                        if !items.is_empty() {
                                            let mut state = self.lock_state();
                                            state.overlay = Some(SurfaceOverlay::WorkerQueue {
                                                selected: 0,
                                                items,
                                            });
                                        }
                                        render_cli_chat_message_spec_with_width(
                                            &TuiMessageSpec {
                                                role: "workers".to_owned(),
                                                caption: Some("visible delegates".to_owned()),
                                                sections: vec![TuiSectionSpec::Narrative {
                                                    title: Some("queue".to_owned()),
                                                    lines: if worker_queue_lines.is_empty() {
                                                        vec![
                                                            "No visible delegate child sessions are currently active in this session scope."
                                                                .to_owned(),
                                                        ]
                                                    } else {
                                                        worker_queue_lines
                                                    },
                                                }],
                                                footer_lines: vec![
                                                    "Use W to open the worker queue overlay or Enter on the queue to inspect one worker."
                                                        .to_owned(),
                                                ],
                                            },
                                            width,
                                        )
                                    }
                                    Err(error) => render_control_plane_unavailable_lines_with_width(
                                        "workers",
                                        "visible delegates",
                                        error.as_str(),
                                        vec![
                                            "Use /status or restore the control-plane store before inspecting worker sessions."
                                                .to_owned(),
                                        ],
                                        width,
                                    ),
                                },
                                ChatCommandMatchResult::UsageError(usage) => {
                                    render_cli_chat_command_usage_lines_with_width(&usage, width)
                                }
                                ChatCommandMatchResult::NotMatched => {
                                    match turn_checkpoint_repair_match {
                                        ChatCommandMatchResult::Matched => {
                                            let outcome = self
                                                .runtime
                                                .turn_coordinator
                                                .repair_production_turn_checkpoint_tail(
                                                    &self.runtime.config,
                                                    &self.runtime.session_id,
                                                    self.runtime.conversation_binding(),
                                                )
                                                .await?;
                                            render_turn_checkpoint_repair_lines_with_width(
                                                &self.runtime.session_id,
                                                &outcome,
                                                width,
                                            )
                                        }
                                        ChatCommandMatchResult::UsageError(usage) => {
                                            render_cli_chat_command_usage_lines_with_width(
                                                &usage, width,
                                            )
                                        }
                                        ChatCommandMatchResult::NotMatched => {
                                            render_cli_chat_command_usage_lines_with_width(
                                                "usage: /help | /status | /history | /sessions | /mission | /review | /workers | /compact | /turn_checkpoint_repair | /exit",
                                                width,
                                            )
                                        }
                                    }
                                }
                            },
                        },
                    },
                },
            },
        };

        let mut state = self.lock_state();
        push_transcript_message(&mut state, lines);
        Ok(())
    }

    pub(super) async fn submit_text(&self, text: &str) -> CliResult<SurfaceLoopAction> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(SurfaceLoopAction::Continue);
        }

        if is_exit_command(&self.runtime.config, trimmed) {
            return Ok(SurfaceLoopAction::Exit);
        }

        if trimmed.starts_with('/') {
            {
                let mut state = self.lock_state();
                state.composer.clear();
                state.composer_cursor = 0;
                state.history_index = None;
                state.focus = SurfaceFocus::Composer;
            }
            self.handle_command(trimmed).await?;
            self.render()?;
            return Ok(SurfaceLoopAction::Continue);
        }

        {
            let mut state = self.lock_state();
            state.transcript.push(SurfaceEntry {
                lines: render_cli_chat_message_spec_with_width(
                    &TuiMessageSpec {
                        role: "you".to_owned(),
                        caption: Some("prompt".to_owned()),
                        sections: vec![TuiSectionSpec::Narrative {
                            title: None,
                            lines: vec![trimmed.to_owned()],
                        }],
                        footer_lines: vec!["Enter send · Esc clear · Tab sidebar".to_owned()],
                    },
                    self.content_width(),
                ),
            });
            state.history.push(trimmed.to_owned());
            state.composer.clear();
            state.composer_cursor = 0;
            state.history_index = None;
            state.pending_turn = true;
            state.scroll_offset = 0;
            state.sticky_bottom = true;
            state.selected_entry = Some(state.transcript.len().saturating_sub(1));
            state.focus = SurfaceFocus::Transcript;
        }
        self.render()?;

        let observer = build_surface_live_observer(self.state.clone(), self.term.clone());
        let turn_request = crate::agent_runtime::AgentTurnRequest {
            message: trimmed.to_owned(),
            turn_mode: crate::agent_runtime::AgentTurnMode::Interactive,
            channel_id: self.runtime.session_address.channel_id.clone(),
            account_id: self.runtime.session_address.account_id.clone(),
            conversation_id: self.runtime.session_address.conversation_id.clone(),
            participant_id: self.runtime.session_address.participant_id.clone(),
            thread_id: self.runtime.session_address.thread_id.clone(),
            metadata: std::collections::BTreeMap::new(),
            acp: self.runtime.explicit_acp_request,
            acp_event_stream: false,
            acp_bootstrap_mcp_servers: self.runtime.effective_bootstrap_mcp_servers.clone(),
            acp_cwd: self
                .runtime
                .effective_working_directory
                .as_ref()
                .map(|path| path.display().to_string()),
            live_surface_enabled: true,
        };
        let turn_options = crate::agent_runtime::TurnExecutionOptions {
            observer: Some(observer),
            ..Default::default()
        };
        let turn_service = crate::agent_runtime::RuntimeTurnExecutionService::new(&self.runtime);
        let assistant_text = turn_service
            .execute(&turn_request, turn_options)
            .await?
            .output_text;

        {
            let mut state = self.lock_state();
            state.transcript.push(SurfaceEntry {
                lines: render_cli_chat_assistant_lines_with_width(
                    &assistant_text,
                    self.content_width(),
                ),
            });
            if let Some(screen) = build_cli_chat_approval_screen_spec(&assistant_text) {
                state.last_approval = Some(ApprovalSurfaceSummary::from_screen_spec(&screen));
                state.overlay = Some(SurfaceOverlay::ApprovalPrompt { screen });
            } else {
                state.last_approval = None;
            }
            state.pending_turn = false;
            state.live.last_assistant_preview = Some(assistant_text);
            state.live.snapshot = None;
            state.live.state = CliChatLiveSurfaceState::default();
            state.selected_entry = Some(state.transcript.len().saturating_sub(1));
            state.sticky_bottom = true;
        }
        self.render()?;
        Ok(SurfaceLoopAction::Continue)
    }

    pub(super) fn submit_input_overlay(
        &self,
        kind: OverlayInputKind,
        value: String,
    ) -> CliResult<()> {
        let trimmed = value.trim();
        let mut state = self.lock_state();
        match kind {
            OverlayInputKind::RenameSession => {
                if trimmed.is_empty() {
                    state.overlay = None;
                    state.focus = SurfaceFocus::Composer;
                    return Ok(());
                }
                state.session_title_override = Some(trimmed.to_owned());
                state.transcript.push(SurfaceEntry {
                    lines: render_cli_chat_message_spec_with_width(
                        &TuiMessageSpec {
                            role: "system".to_owned(),
                            caption: Some("session".to_owned()),
                            sections: vec![TuiSectionSpec::Callout {
                                tone: TuiCalloutTone::Success,
                                title: Some("session renamed".to_owned()),
                                lines: vec![format!("Session title updated to `{trimmed}`.")],
                            }],
                            footer_lines: vec![
                                "This rename is local to the current surface.".to_owned(),
                            ],
                        },
                        self.content_width(),
                    ),
                });
            }
            OverlayInputKind::ExportTranscript => {
                if trimmed.is_empty() {
                    state.overlay = None;
                    state.focus = SurfaceFocus::Composer;
                    return Ok(());
                }
                let export_path = PathBuf::from(trimmed);
                ensure_parent_directory_exists(export_path.as_path())?;
                let export_text = format_transcript_export(&state.transcript);
                std::fs::write(export_path.as_path(), export_text).map_err(|error| {
                    let display_path = export_path.display();
                    format!("failed to export transcript to `{display_path}`: {error}")
                })?;
                let exported_path = export_path.display().to_string();
                state.transcript.push(SurfaceEntry {
                    lines: render_cli_chat_message_spec_with_width(
                        &TuiMessageSpec {
                            role: "system".to_owned(),
                            caption: Some("export".to_owned()),
                            sections: vec![TuiSectionSpec::Callout {
                                tone: TuiCalloutTone::Success,
                                title: Some("transcript exported".to_owned()),
                                lines: vec![format!("Saved transcript to `{exported_path}`.")],
                            }],
                            footer_lines: vec![
                                "Use the exported text file for external review or sharing."
                                    .to_owned(),
                            ],
                        },
                        self.content_width(),
                    ),
                });
            }
        }
        state.overlay = None;
        state.focus = SurfaceFocus::Transcript;
        state.selected_entry = Some(state.transcript.len().saturating_sub(1));
        state.sticky_bottom = true;
        Ok(())
    }

    pub(super) fn render(&self) -> CliResult<()> {
        let (height_u16, width_u16) = self.term.size();
        let total_height = usize::from(height_u16);
        let total_width = usize::from(width_u16);
        let state = self.lock_state().clone();
        let header_lines = crate::presentation::render_compact_brand_header(
            total_width.saturating_sub(2),
            &crate::presentation::BuildVersionInfo::current(),
            Some(session_subtitle(&state)),
        )
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>();
        let sidebar_visible = state.sidebar_visible && total_width >= MIN_SIDEBAR_TOTAL_WIDTH;
        let sidebar_width = if sidebar_visible { SIDEBAR_WIDTH } else { 0 };
        let content_width = total_width
            .saturating_sub(sidebar_width)
            .saturating_sub(if sidebar_visible { 3 } else { 2 })
            .max(24);
        let reserved_height =
            header_lines.len() + HEADER_GAP + COMPOSER_HEIGHT + STATUS_BAR_HEIGHT + 1;
        let transcript_height = total_height.saturating_sub(reserved_height).max(5);
        let render_data = SurfaceRenderData {
            header_lines,
            header_status_line: self
                .build_header_status_line(&state, total_width.saturating_sub(4)),
            transcript_lines: self.build_transcript_lines(&state, content_width, transcript_height),
            sidebar_visible,
            sidebar_tab: state.sidebar_tab,
            sidebar_lines: self.build_sidebar_lines(
                &state,
                SIDEBAR_WIDTH.saturating_sub(2),
                transcript_height,
            ),
            composer_lines: self.build_composer_lines(&state, total_width.saturating_sub(6)),
            status_line: self.build_status_line(&state, total_width.saturating_sub(4)),
        };
        let output =
            render_surface_to_string(&state, &render_data, Rect::new(0, 0, width_u16, height_u16));

        self.term
            .write_str(format!("{CLEAR_AND_HOME}{output}").as_str())
            .map_err(|error| format!("failed to render chat surface: {error}"))?;
        self.term
            .flush()
            .map_err(|error| format!("failed to flush chat surface: {error}"))?;
        Ok(())
    }
}
