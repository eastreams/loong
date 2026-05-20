impl App {
    pub fn new(
        runtime: &CliTurnRuntime,
        options: &CliChatOptions,
        render_width: usize,
    ) -> CliResult<Self> {
        let language = resolve_default_language();
        let detected_skills =
            detect_available_skills(runtime.effective_working_directory.as_deref());
        let startup_mcp_count = runtime.effective_bootstrap_mcp_servers.len();
        let mut app = Self {
            message_list: MessageList::new(),
            composer: Composer::new(),
            command_palette: CommandPalette::new(language, detected_skills.clone()),
            focus: Focus::Composer,
            pending_turn: false,
            turn_start: None,
            live_transcript: Arc::new(StdMutex::new(LiveTranscriptState::default())),
            pending_task: None,
            pending_steers: VecDeque::new(),
            pending_queue: VecDeque::new(),
            composer_follow_up_intent: false,
            pending_first_turn_bootstrap_addendum: None,
            awaiting_first_turn_bootstrap_reply: false,
            live_render_width: Arc::new(AtomicUsize::new(render_width.max(1))),
            live_rerender: None,
            spinner_seed: spinner_seed(),
            last_pending_signature: None,
            last_live_transcript_signature: None,
            pending_render_cache: None,
            inline_skill_popup_active: false,
            last_render_width: render_width as u16,
            last_render_height: 0,
            last_transcript_area: Rect::default(),
            last_composer_area: Rect::default(),
            last_palette_area: Rect::default(),
            startup_onboarding: StartupOnboardingState::new(runtime, language),
            startup_version: String::new(),
            startup_mcp_count,
            detected_skills,
            cwd: format_cwd(runtime),
            model: runtime.config.provider.model.clone(),
            title: None,
            last_terminal_title: None,
            title_attention_required: false,
            title_pending_approval_count: 0,
            i18n: I18nService::new(language),
        };

        let (version, tutorial, sections, tips) =
            build_chat_startup_content(runtime, options, render_width, &app.i18n);
        app.startup_version = version.clone();
        let startup_eye_animation =
            startup_eye_animation_for_state(app.startup_onboarding.as_ref());
        app.message_list.add_startup_header_with_tips_and_eye(
            version,
            tutorial,
            sections,
            tips,
            startup_eye_animation,
        );

        Ok(app)
    }

    pub fn render(&mut self, f: &mut Frame) {
        let size = f.area();
        self.last_render_width = size.width;
        self.last_render_height = size.height;
        let composer_height = self.composer.height_for_area(size.width, size.height);
        let palette_visible =
            matches!(self.focus, Focus::CommandPalette) || self.inline_skill_popup_active;
        let palette_height = if palette_visible {
            self.command_palette.desired_height() as u16
        } else {
            0
        };
        let interstitial_lines =
            self.interstitial_lines_for(size.width, size.height, composer_height, palette_height);
        let interstitial_height = interstitial_lines.len() as u16;
        let provisional_assistant_text = provisional_assistant_text(&self.live_transcript);
        let transcript_line_count = self
            .message_list
            .rendered_line_count_with_provisional_assistant(
                size.width,
                provisional_assistant_text.as_deref(),
            ) as u16;
        let bottom_band_height = interstitial_height
            + 1
            + composer_height
            + if palette_height > 0 {
                1 + palette_height
            } else {
                0
            }
            + 1
            + 1
            + FOOTER_BOTTOM_BREATHING_HEIGHT;
        let available_transcript_height = size.height.saturating_sub(bottom_band_height).max(1);
        let transcript_height = if self.message_list.messages.is_empty() {
            0
        } else {
            transcript_line_count.min(available_transcript_height)
        };
        let main_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(transcript_height),
                Constraint::Length(0),
                Constraint::Length(interstitial_height),
                Constraint::Length(1),
                Constraint::Length(composer_height),
                Constraint::Length(if palette_height > 0 { 1 } else { 0 }),
                Constraint::Length(palette_height),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(FOOTER_BOTTOM_BREATHING_HEIGHT),
            ])
            .split(size);

        let [
            transcript_area,
            _spacer_area,
            pending_area,
            composer_separator_area,
            composer_area,
            palette_separator_area,
            palette_area,
            footer_separator_area,
            footer_area,
            footer_bottom_spacing_area,
        ] = main_layout.as_ref()
        else {
            return;
        };

        self.last_transcript_area = *transcript_area;
        self.last_composer_area = *composer_area;
        self.last_palette_area = if palette_visible {
            *palette_area
        } else {
            Rect::default()
        };

        self.message_list.render_with_provisional_assistant(
            f,
            *transcript_area,
            provisional_assistant_text.as_deref(),
        );

        if interstitial_height > 0 {
            f.render_widget(Paragraph::new(interstitial_lines), *pending_area);
        }

        let line_color = SURFACE_COTTON_CANDY;
        let composer_separator_is_blank =
            interstitial_height == 0 && self.message_list.trailing_colored_block(size.width);
        if composer_separator_is_blank {
            f.render_widget(Paragraph::new(""), *composer_separator_area);
        } else {
            f.render_widget(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(line_color)),
                *composer_separator_area,
            );
        }

        self.composer
            .render(f, *composer_area, matches!(self.focus, Focus::Composer));
        if matches!(self.focus, Focus::Composer) {
            let (x, y) = self.composer.cursor_position(*composer_area);
            f.set_cursor_position((x, y));
        }

        if palette_visible {
            f.render_widget(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(line_color)),
                *palette_separator_area,
            );
            self.command_palette.render(f, *palette_area);
        }

        f.render_widget(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(line_color)),
            *footer_separator_area,
        );

        let footer_content_area = footer_content_area(*footer_area);
        let footer_line = if self.pending_turn && !self.composer.is_empty() {
            build_queue_footer_line(
                &self.i18n,
                self.pending_queue.len(),
                footer_content_area.width,
            )
        } else if let Some(state) = self.startup_onboarding.as_ref() {
            build_startup_onboarding_footer_line(state, footer_content_area.width)
        } else if self.pending_turn && !self.pending_queue.is_empty() {
            build_restore_footer_line(
                &self.i18n,
                self.pending_queue.len(),
                footer_content_area.width,
            )
        } else if !self.message_list.is_following_tail() {
            build_follow_footer_line(&self.i18n, &self.model, footer_content_area.width)
        } else {
            build_status_footer_line(&self.cwd, &self.model, footer_content_area.width)
        };
        f.render_widget(Paragraph::new(footer_line), footer_content_area);
        f.render_widget(Paragraph::new(""), *footer_bottom_spacing_area);
    }

    fn refresh_startup_header(&mut self) {
        let tutorial = self.i18n.text(SurfaceCopy::Tutorial).to_owned();
        let sections = vec![
            (
                self.i18n.text(SurfaceCopy::StartupSectionSkills).to_owned(),
                vec![self.detected_skills.len().to_string()],
            ),
            (
                self.i18n.text(SurfaceCopy::StartupSectionMcp).to_owned(),
                vec![self.startup_mcp_count.to_string()],
            ),
        ];
        let tips = vec![
            tutorial.clone(),
            self.i18n.text(SurfaceCopy::StartupTipCommands).to_owned(),
            self.i18n.text(SurfaceCopy::StartupTipSkills).to_owned(),
            self.i18n.text(SurfaceCopy::StartupTipQueue).to_owned(),
            self.i18n.text(SurfaceCopy::StartupTipHistory).to_owned(),
        ];
        let eye_animation = startup_eye_animation_for_state(self.startup_onboarding.as_ref());
        self.message_list.replace_latest_startup_header_with_eye(
            self.startup_version.clone(),
            tutorial,
            sections,
            tips,
            eye_animation,
        );
    }

    fn apply_startup_onboarding_action(
        &mut self,
        action: StartupOnboardingAction,
        runtime: &mut CliTurnRuntime,
    ) -> CliResult<bool> {
        match action {
            StartupOnboardingAction::Ignored => Ok(false),
            StartupOnboardingAction::Handled => {
                self.refresh_startup_header();
                Ok(true)
            }
            StartupOnboardingAction::ApplyLanguage(language) => {
                self.i18n = I18nService::new(language);
                self.command_palette = CommandPalette::new(language, self.detected_skills.clone());
                self.inline_skill_popup_active = false;
                if let Some(state) = self.startup_onboarding.as_mut() {
                    state.refresh_localized_runtime_content(runtime);
                }
                self.refresh_startup_header();
                Ok(true)
            }
            StartupOnboardingAction::PersistProviderSelection(option) => {
                let language = self
                    .startup_onboarding
                    .as_ref()
                    .map(StartupOnboardingState::current_language)
                    .unwrap_or(Language::En);
                let summary = persist_startup_provider_selection(runtime, option, language)?;
                if let Some(state) = self.startup_onboarding.as_mut() {
                    state.refresh_localized_runtime_content(runtime);
                    state.feedback = Some(summary);
                    state.stage = StartupOnboardingStage::Skills;
                }
                self.refresh_startup_header();
                Ok(true)
            }
            StartupOnboardingAction::PersistPersonalization(preset) => {
                let language = self
                    .startup_onboarding
                    .as_ref()
                    .map(StartupOnboardingState::current_language)
                    .unwrap_or(Language::En);
                let summary = persist_startup_personalization(runtime, preset, language)?;
                if let Some(state) = self.startup_onboarding.as_mut() {
                    state.selected_personalization = Some(preset);
                    state.feedback = Some(summary);
                    state.stage = StartupOnboardingStage::Finish;
                }
                self.pending_first_turn_bootstrap_addendum =
                    startup_first_turn_bootstrap_addendum(preset, language);
                self.refresh_startup_header();
                Ok(true)
            }
            StartupOnboardingAction::Complete => {
                self.startup_onboarding = None;
                self.refresh_startup_header();
                Ok(true)
            }
        }
    }

    fn interstitial_lines_for(
        &mut self,
        width: u16,
        height: u16,
        composer_height: u16,
        palette_height: u16,
    ) -> Vec<Line<'static>> {
        if self.pending_turn {
            return self.pending_lines_for(width, height, composer_height, palette_height);
        }

        self.startup_onboarding
            .as_ref()
            .map(|state| render_startup_onboarding_lines(state, width))
            .unwrap_or_default()
    }

    fn apply_palette_action(&mut self, action: CommandAction) -> Option<String> {
        match action {
            CommandAction::RunCommand(command) => {
                self.inline_skill_popup_active = false;
                self.focus = Focus::Composer;
                Some(command.to_owned())
            }
            CommandAction::OpenSettings(_)
            | CommandAction::ApplySettings(_)
            | CommandAction::OpenModelReasoning(_)
            | CommandAction::ApplyModelSelection { .. } => None,
            CommandAction::Noop => None,
            CommandAction::InsertText(text) => {
                if let Some(range) = current_skill_token_range(&self.composer) {
                    let replacement =
                        inline_skill_replacement_text(self.composer.text(), &range, text.as_str());
                    self.composer.replace_range(range, replacement.as_str());
                } else {
                    self.composer.set_input(text);
                }
                self.inline_skill_popup_active = false;
                self.focus = Focus::Composer;
                None
            }
            CommandAction::Close => {
                self.inline_skill_popup_active = false;
                self.focus = Focus::Composer;
                None
            }
        }
    }

    fn handle_mouse_event(&mut self, mouse_event: MouseEvent) -> Option<String> {
        if rect_contains_point(self.last_palette_area, mouse_event.column, mouse_event.row)
            && (matches!(self.focus, Focus::CommandPalette) || self.inline_skill_popup_active)
        {
            return self
                .command_palette
                .handle_mouse(mouse_event, self.last_palette_area)
                .and_then(|action| self.apply_palette_action(action));
        }

        if rect_contains_point(self.last_composer_area, mouse_event.column, mouse_event.row) {
            if matches!(mouse_event.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.focus = Focus::Composer;
                self.sync_inline_skill_popup();
            }
            return None;
        }

        if rect_contains_point(
            self.last_transcript_area,
            mouse_event.column,
            mouse_event.row,
        ) {
            if matches!(
                mouse_event.kind,
                MouseEventKind::Down(MouseButton::Left)
                    | MouseEventKind::Down(MouseButton::Right)
                    | MouseEventKind::Down(MouseButton::Middle)
            ) {
                self.focus = Focus::MessageList;
                self.sync_inline_skill_popup();
            }
            self.message_list.handle_mouse(mouse_event);
        }

        None
    }

    fn sync_inline_skill_popup(&mut self) {
        if !matches!(self.focus, Focus::Composer) {
            self.inline_skill_popup_active = false;
            return;
        }

        if self.command_palette.has_skills()
            && let Some(query) = current_skill_token_query(&self.composer)
        {
            self.command_palette.show_skills(query.as_str());
            self.inline_skill_popup_active = true;
        } else {
            self.inline_skill_popup_active = false;
        }
    }

    fn confirm_inline_skill_popup(&mut self) {
        if let Some(action) = self
            .command_palette
            .handle_key(crossterm::event::KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            ))
        {
            let _ = self.apply_palette_action(action);
        } else {
            self.inline_skill_popup_active = false;
        }
    }

    fn handle_inline_skill_popup_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        if !self.inline_skill_popup_active {
            return false;
        }

        if matches!(
            key.code,
            KeyCode::Up
                | KeyCode::Down
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Home
                | KeyCode::End
        ) {
            let _ = self.command_palette.handle_key(key);
            return true;
        }

        if key.code == KeyCode::Esc {
            self.inline_skill_popup_active = false;
            return true;
        }

        if (key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT))
            || key.code == KeyCode::Tab
        {
            self.confirm_inline_skill_popup();
            return true;
        }

        false
    }
}
