// RatatuiOnboardRunner: core TUI runner that implements GuidedOnboardFlowStepRunner.

use std::io::{self, Stdout};
use std::path::PathBuf;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use loongclaw_app as mvp;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::event_source::{CrosstermEventSource, OnboardEventSource};
use super::layout::{self, OnboardLayoutAreas};
use super::widgets::*;
use crate::CliResult;
use crate::onboard_flow::{GuidedOnboardFlowStepRunner, OnboardFlowStepAction};
use crate::onboard_state::{OnboardDraft, OnboardWizardStep};
use crate::provider_credential_policy;

// ---------------------------------------------------------------------------
// TUI mode
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub(crate) enum OnboardTuiMode {
    FullScreen,
    Inline,
}

impl OnboardTuiMode {
    #[allow(dead_code)]
    pub fn detect() -> Self {
        let (cols, rows) = terminal::size().unwrap_or((80, 24));
        if cols >= 40 && rows >= 15 {
            Self::FullScreen
        } else {
            Self::Inline
        }
    }

    const fn is_fullscreen(&self) -> bool {
        matches!(self, Self::FullScreen)
    }
}

// ---------------------------------------------------------------------------
// Loop result types
// ---------------------------------------------------------------------------

enum SelectionLoopResult {
    Selected(usize),
    Back,
}

enum InputLoopResult {
    Submitted(String),
    Back,
}

// ---------------------------------------------------------------------------
// RatatuiOnboardRunner
// ---------------------------------------------------------------------------

pub(crate) struct RatatuiOnboardRunner<E: OnboardEventSource = CrosstermEventSource> {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    event_source: E,
    mode: OnboardTuiMode,
    owns_tty: bool,
}

impl RatatuiOnboardRunner<CrosstermEventSource> {
    /// Create a new runner with real terminal events and auto-detected mode.
    #[allow(dead_code)]
    pub fn new() -> io::Result<Self> {
        Self::with_event_source(CrosstermEventSource, OnboardTuiMode::detect())
    }
}

impl<E: OnboardEventSource> RatatuiOnboardRunner<E> {
    /// Create a runner with a custom event source and explicit mode.
    ///
    /// When a real terminal is available this acquires raw mode and (for
    /// full-screen) an alternate screen buffer.
    pub fn with_event_source(event_source: E, mode: OnboardTuiMode) -> io::Result<Self> {
        // Install a panic hook that restores the terminal before printing.
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = terminal::disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            original_hook(info);
        }));

        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        if mode.is_fullscreen() {
            execute!(stdout, EnterAlternateScreen)?;
        }
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self {
            terminal,
            event_source,
            mode,
            owns_tty: true,
        })
    }

    /// Create a runner without touching the real terminal.
    ///
    /// Used in tests where raw-mode / alternate-screen is unavailable.
    #[cfg(test)]
    fn headless(event_source: E, mode: OnboardTuiMode) -> io::Result<Self> {
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::with_options(
            backend,
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Fixed(ratatui::layout::Rect::new(0, 0, 80, 24)),
            },
        )?;
        Ok(Self {
            terminal,
            event_source,
            mode,
            owns_tty: false,
        })
    }

    // -----------------------------------------------------------------------
    // Chrome (header + spine + footer)
    // -----------------------------------------------------------------------

    #[allow(dead_code)]
    fn render_chrome(
        &mut self,
        step: OnboardWizardStep,
        title: &str,
        footer_hint: &str,
    ) -> io::Result<OnboardLayoutAreas> {
        let wide_spine = self.mode.is_fullscreen();
        let mut captured_areas: Option<OnboardLayoutAreas> = None;

        let step_number = step_ordinal(step);
        let total_steps = 8;

        self.terminal.draw(|frame| {
            let areas = layout::compute_layout(frame.area(), wide_spine);

            // Header bar
            let header_line = Line::from(vec![
                Span::styled(
                    " LOONGCLAW ",
                    Style::default()
                        .fg(Color::Rgb(245, 169, 127))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {title} "), Style::default().fg(Color::White)),
                Span::styled("  Esc cancel", Style::default().fg(Color::DarkGray)),
            ]);
            frame.render_widget(Paragraph::new(header_line), areas.header);

            // Spine sidebar
            if wide_spine {
                let spine = ProgressSpineWidget::new(step);
                frame.render_widget(spine, areas.spine);
            }

            // Footer
            let footer_line =
                Line::from(vec![
                    Span::styled(format!(" {footer_hint} "), Style::default().fg(Color::Gray)),
                    Span::raw(" ".repeat(
                        (areas.footer.width as usize).saturating_sub(footer_hint.len() + 10),
                    )),
                    Span::styled(
                        format!(" {step_number}/{total_steps} "),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);
            frame.render_widget(Paragraph::new(footer_line), areas.footer);

            captured_areas = Some(areas);
        })?;

        captured_areas.ok_or_else(|| io::Error::other("draw callback skipped"))
    }

    // -----------------------------------------------------------------------
    // Welcome step
    // -----------------------------------------------------------------------

    fn run_welcome_step(&mut self) -> CliResult<OnboardFlowStepAction> {
        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
        loop {
            let wide = self.mode.is_fullscreen();
            let ver = version.clone();
            self.terminal
                .draw(|frame| {
                    let areas = layout::compute_layout(frame.area(), wide);

                    // Header
                    let header_line = Line::from(vec![
                        Span::styled(
                            " LOONGCLAW ",
                            Style::default()
                                .fg(Color::Rgb(245, 169, 127))
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(" setup wizard ", Style::default().fg(Color::White)),
                        Span::styled("  Esc cancel", Style::default().fg(Color::DarkGray)),
                    ]);
                    frame.render_widget(Paragraph::new(header_line), areas.header);

                    if wide {
                        let spine = ProgressSpineWidget::new(OnboardWizardStep::Welcome);
                        frame.render_widget(spine, areas.spine);
                    }

                    // Welcome content
                    let welcome = WelcomeScreen::new(&ver);
                    frame.render_widget(welcome, areas.content);

                    // Footer
                    let footer_line = Line::from(vec![
                        Span::styled(" Press Enter to begin ", Style::default().fg(Color::Gray)),
                        Span::raw(" ".repeat((areas.footer.width as usize).saturating_sub(26))),
                        Span::styled(" 1/8 ", Style::default().fg(Color::DarkGray)),
                    ]);
                    frame.render_widget(Paragraph::new(footer_line), areas.footer);
                })
                .map_err(|e| e.to_string())?;

            match self.event_source.next_event().map_err(|e| e.to_string())? {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => return Ok(OnboardFlowStepAction::Next),
                Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => return Ok(OnboardFlowStepAction::Back),
                Event::Key(KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers,
                    ..
                }) if modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err("interrupted by user".to_owned());
                }
                Event::Resize(..) | Event::FocusGained | Event::FocusLost => { /* redraw */ }
                Event::Key(_) | Event::Mouse(_) | Event::Paste(_) => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Generic selection loop
    // -----------------------------------------------------------------------

    fn run_selection_loop(
        &mut self,
        step: OnboardWizardStep,
        title: &str,
        items: Vec<SelectionItem>,
        default_index: usize,
        footer_hint: &str,
    ) -> CliResult<SelectionLoopResult> {
        let mut state = SelectionCardState::new(items.len());
        state.select(default_index);
        let wide = self.mode.is_fullscreen();

        loop {
            let step_number = step_ordinal(step);
            let title_owned = title.to_owned();
            let hint_owned = footer_hint.to_owned();

            self.terminal
                .draw(|frame| {
                    let areas = layout::compute_layout(frame.area(), wide);

                    // Header
                    let header_line = Line::from(vec![
                        Span::styled(
                            " LOONGCLAW ",
                            Style::default()
                                .fg(Color::Rgb(245, 169, 127))
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!(" {title_owned} "),
                            Style::default().fg(Color::White),
                        ),
                        Span::styled("  Esc back", Style::default().fg(Color::DarkGray)),
                    ]);
                    frame.render_widget(Paragraph::new(header_line), areas.header);

                    // Spine
                    if wide {
                        frame.render_widget(ProgressSpineWidget::new(step), areas.spine);
                    }

                    // Content: selection cards
                    let widget = SelectionCardWidget::new(
                        items
                            .iter()
                            .map(|i| SelectionItem::new(i.label.as_str(), i.hint.as_deref()))
                            .collect(),
                    );
                    frame.render_stateful_widget(widget, areas.content, &mut state);

                    // Footer
                    let footer_line = Line::from(vec![
                        Span::styled(format!(" {hint_owned} "), Style::default().fg(Color::Gray)),
                        Span::raw(" ".repeat(
                            (areas.footer.width as usize).saturating_sub(hint_owned.len() + 10),
                        )),
                        Span::styled(
                            format!(" {step_number}/8 "),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]);
                    frame.render_widget(Paragraph::new(footer_line), areas.footer);
                })
                .map_err(|e| e.to_string())?;

            match self.event_source.next_event().map_err(|e| e.to_string())? {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => return Ok(SelectionLoopResult::Selected(state.selected())),
                Event::Key(KeyEvent {
                    code: KeyCode::Down | KeyCode::Char('j'),
                    ..
                }) => state.next(),
                Event::Key(KeyEvent {
                    code: KeyCode::Up | KeyCode::Char('k'),
                    ..
                }) => state.previous(),
                Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => return Ok(SelectionLoopResult::Back),
                Event::Key(KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers,
                    ..
                }) if modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err("interrupted by user".to_owned());
                }
                Event::Resize(..) | Event::FocusGained | Event::FocusLost => { /* redraw */ }
                Event::Key(_) | Event::Mouse(_) | Event::Paste(_) => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Generic text input loop
    // -----------------------------------------------------------------------

    fn run_input_loop(
        &mut self,
        step: OnboardWizardStep,
        label: &str,
        default_value: &str,
        footer_hint: &str,
    ) -> CliResult<InputLoopResult> {
        let mut input_state = if default_value.is_empty() {
            TextInputState::new()
        } else {
            TextInputState::with_default(default_value)
        };
        let wide = self.mode.is_fullscreen();

        loop {
            let step_number = step_ordinal(step);
            let label_owned = label.to_owned();
            let hint_owned = footer_hint.to_owned();

            self.terminal
                .draw(|frame| {
                    let areas = layout::compute_layout(frame.area(), wide);

                    // Header
                    let header_line = Line::from(vec![
                        Span::styled(
                            " LOONGCLAW ",
                            Style::default()
                                .fg(Color::Rgb(245, 169, 127))
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!(" {label_owned} "),
                            Style::default().fg(Color::White),
                        ),
                        Span::styled("  Esc back", Style::default().fg(Color::DarkGray)),
                    ]);
                    frame.render_widget(Paragraph::new(header_line), areas.header);

                    // Spine
                    if wide {
                        frame.render_widget(ProgressSpineWidget::new(step), areas.spine);
                    }

                    // Content: text input
                    let widget = TextInputWidget::new(&label_owned);
                    widget.render_with_state(areas.content, frame.buffer_mut(), &input_state);

                    // Footer
                    let footer_line = Line::from(vec![
                        Span::styled(format!(" {hint_owned} "), Style::default().fg(Color::Gray)),
                        Span::raw(" ".repeat(
                            (areas.footer.width as usize).saturating_sub(hint_owned.len() + 10),
                        )),
                        Span::styled(
                            format!(" {step_number}/8 "),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]);
                    frame.render_widget(Paragraph::new(footer_line), areas.footer);
                })
                .map_err(|e| e.to_string())?;

            match self.event_source.next_event().map_err(|e| e.to_string())? {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => {
                    let value = input_state.submit_value().to_owned();
                    return Ok(InputLoopResult::Submitted(value));
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Backspace,
                    ..
                }) => input_state.backspace(),
                Event::Key(KeyEvent {
                    code: KeyCode::Char(c),
                    modifiers,
                    ..
                }) if modifiers.contains(KeyModifiers::CONTROL) && c == 'c' => {
                    return Err("interrupted by user".to_owned());
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Char(c),
                    ..
                }) => input_state.push(c),
                Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => return Ok(InputLoopResult::Back),
                Event::Resize(..) | Event::FocusGained | Event::FocusLost => { /* redraw */ }
                Event::Key(_) | Event::Mouse(_) | Event::Paste(_) => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Authentication step
    // -----------------------------------------------------------------------

    fn run_authentication_step(
        &mut self,
        draft: &mut OnboardDraft,
    ) -> CliResult<OnboardFlowStepAction> {
        // Sub-step 1: Provider selection (show current provider kind)
        let provider_kind = draft.config.provider.kind;
        let provider_label = provider_kind.display_name().to_owned();
        let items = vec![SelectionItem::new(
            &provider_label,
            Some("current provider"),
        )];
        match self.run_selection_loop(
            OnboardWizardStep::Authentication,
            "Provider",
            items,
            0,
            "Enter to confirm provider",
        )? {
            SelectionLoopResult::Back => return Ok(OnboardFlowStepAction::Back),
            SelectionLoopResult::Selected(_) => { /* continue */ }
        }

        // Sub-step 2: Model (text input with current model as default)
        let current_model = draft.config.provider.model.clone();
        match self.run_input_loop(
            OnboardWizardStep::Authentication,
            "Model:",
            &current_model,
            "Enter to confirm model, or type a custom model",
        )? {
            InputLoopResult::Back => return Ok(OnboardFlowStepAction::Back),
            InputLoopResult::Submitted(model) => {
                if !model.is_empty() {
                    draft.set_provider_model(model);
                }
            }
        }

        // Sub-step 3: Credential environment variable
        let current_credential_env =
            provider_credential_policy::provider_credential_env_hint(&draft.config.provider)
                .unwrap_or_default();
        match self.run_input_loop(
            OnboardWizardStep::Authentication,
            "API key env:",
            &current_credential_env,
            "Enter to confirm, or type a custom env var name",
        )? {
            InputLoopResult::Back => return Ok(OnboardFlowStepAction::Back),
            InputLoopResult::Submitted(env_name) => {
                draft.set_provider_credential_env(env_name);
            }
        }

        Ok(OnboardFlowStepAction::Next)
    }

    // -----------------------------------------------------------------------
    // Runtime defaults step
    // -----------------------------------------------------------------------

    fn run_runtime_defaults_step(
        &mut self,
        draft: &mut OnboardDraft,
    ) -> CliResult<OnboardFlowStepAction> {
        let profiles = [
            mvp::config::MemoryProfile::WindowOnly,
            mvp::config::MemoryProfile::WindowPlusSummary,
            mvp::config::MemoryProfile::ProfilePlusWindow,
        ];
        let current = draft.config.memory.profile;
        let default_idx = profiles.iter().position(|p| *p == current).unwrap_or(0);

        let items: Vec<SelectionItem> = profiles
            .iter()
            .map(|p| {
                let hint = match p {
                    mvp::config::MemoryProfile::WindowOnly => "sliding window only",
                    mvp::config::MemoryProfile::WindowPlusSummary => "window + summary",
                    mvp::config::MemoryProfile::ProfilePlusWindow => "profile + window",
                };
                SelectionItem::new(p.as_str(), Some(hint))
            })
            .collect();

        match self.run_selection_loop(
            OnboardWizardStep::RuntimeDefaults,
            "Memory Profile",
            items,
            default_idx,
            "Up/Down to select, Enter to confirm",
        )? {
            SelectionLoopResult::Back => Ok(OnboardFlowStepAction::Back),
            SelectionLoopResult::Selected(idx) => {
                if let Some(profile) = profiles.get(idx) {
                    draft.set_memory_profile(*profile);
                }
                Ok(OnboardFlowStepAction::Next)
            }
        }
    }

    // -----------------------------------------------------------------------
    // Workspace step
    // -----------------------------------------------------------------------

    fn run_workspace_step(&mut self, draft: &mut OnboardDraft) -> CliResult<OnboardFlowStepAction> {
        // Sub-step 1: SQLite path
        let current_sqlite = draft.workspace.sqlite_path.display().to_string();
        match self.run_input_loop(
            OnboardWizardStep::Workspace,
            "SQLite path:",
            &current_sqlite,
            "Enter to confirm, or type a custom path",
        )? {
            InputLoopResult::Back => return Ok(OnboardFlowStepAction::Back),
            InputLoopResult::Submitted(path) => {
                if !path.is_empty() {
                    draft.set_workspace_sqlite_path(PathBuf::from(path));
                }
            }
        }

        // Sub-step 2: File root
        let current_file_root = draft.workspace.file_root.display().to_string();
        match self.run_input_loop(
            OnboardWizardStep::Workspace,
            "File root:",
            &current_file_root,
            "Enter to confirm, or type a custom path",
        )? {
            InputLoopResult::Back => return Ok(OnboardFlowStepAction::Back),
            InputLoopResult::Submitted(path) => {
                if !path.is_empty() {
                    draft.set_workspace_file_root(PathBuf::from(path));
                }
            }
        }

        Ok(OnboardFlowStepAction::Next)
    }

    // -----------------------------------------------------------------------
    // Protocols step
    // -----------------------------------------------------------------------

    fn run_protocols_step(&mut self, draft: &mut OnboardDraft) -> CliResult<OnboardFlowStepAction> {
        // Sub-step 1: ACP enabled/disabled
        let current_enabled = draft.protocols.acp_enabled;
        let default_idx = if current_enabled { 0 } else { 1 };
        let items = vec![
            SelectionItem::new("Enabled", Some("connect to ACP agents")),
            SelectionItem::new("Disabled", Some("standalone mode")),
        ];

        match self.run_selection_loop(
            OnboardWizardStep::Protocols,
            "ACP Protocol",
            items,
            default_idx,
            "Up/Down to select, Enter to confirm",
        )? {
            SelectionLoopResult::Back => return Ok(OnboardFlowStepAction::Back),
            SelectionLoopResult::Selected(idx) => {
                let enabled = idx == 0;
                draft.set_acp_enabled(enabled);
                if !enabled {
                    return Ok(OnboardFlowStepAction::Next);
                }
            }
        }

        // Sub-step 2: ACP backend selection (only if enabled)
        let backends = ["builtin", "jsonrpc"];
        let current_backend = draft.protocols.acp_backend.as_deref().unwrap_or("builtin");
        let default_backend_idx = backends
            .iter()
            .position(|b| *b == current_backend)
            .unwrap_or(0);

        let items: Vec<SelectionItem> = backends
            .iter()
            .map(|b| {
                let hint = match *b {
                    "builtin" => "in-process backend",
                    "jsonrpc" => "JSON-RPC remote backend",
                    _ => "",
                };
                SelectionItem::new(*b, Some(hint))
            })
            .collect();

        match self.run_selection_loop(
            OnboardWizardStep::Protocols,
            "ACP Backend",
            items,
            default_backend_idx,
            "Up/Down to select, Enter to confirm",
        )? {
            SelectionLoopResult::Back => return Ok(OnboardFlowStepAction::Back),
            SelectionLoopResult::Selected(idx) => {
                if let Some(backend) = backends.get(idx) {
                    draft.set_acp_backend(Some((*backend).to_owned()));
                }
            }
        }

        Ok(OnboardFlowStepAction::Next)
    }
}

// ---------------------------------------------------------------------------
// GuidedOnboardFlowStepRunner implementation
// ---------------------------------------------------------------------------

impl<E: OnboardEventSource> GuidedOnboardFlowStepRunner for RatatuiOnboardRunner<E> {
    async fn run_step(
        &mut self,
        step: OnboardWizardStep,
        draft: &mut OnboardDraft,
    ) -> CliResult<OnboardFlowStepAction> {
        match step {
            OnboardWizardStep::Welcome => self.run_welcome_step(),
            OnboardWizardStep::Authentication => self.run_authentication_step(draft),
            OnboardWizardStep::RuntimeDefaults => self.run_runtime_defaults_step(draft),
            OnboardWizardStep::Workspace => self.run_workspace_step(draft),
            OnboardWizardStep::Protocols => self.run_protocols_step(draft),
            // Post-boundary steps are handled outside the guided flow loop.
            OnboardWizardStep::EnvironmentCheck
            | OnboardWizardStep::ReviewAndWrite
            | OnboardWizardStep::Ready => Ok(OnboardFlowStepAction::Next),
        }
    }
}

// ---------------------------------------------------------------------------
// Drop — restore terminal
// ---------------------------------------------------------------------------

impl<E: OnboardEventSource> Drop for RatatuiOnboardRunner<E> {
    fn drop(&mut self) {
        if self.owns_tty {
            let _ = terminal::disable_raw_mode();
            if self.mode.is_fullscreen() {
                let _ = execute!(io::stdout(), LeaveAlternateScreen);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn step_ordinal(step: OnboardWizardStep) -> usize {
    use crate::onboard_flow::OnboardFlowController;
    OnboardFlowController::ordered_steps()
        .iter()
        .position(|s| *s == step)
        .map(|i| i + 1)
        .unwrap_or(1)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::onboard_flow::OnboardFlowStepAction;
    use crate::onboard_state::{OnboardDraft, OnboardValueOrigin, OnboardWizardStep};
    use crate::onboard_tui::event_source::ScriptedEventSource;

    fn sample_draft() -> OnboardDraft {
        let mut config = mvp::config::LoongClawConfig::default();
        config.memory.sqlite_path = "/tmp/memory.sqlite3".to_owned();
        config.tools.file_root = Some("/tmp/workspace".to_owned());
        config.acp.backend = Some("builtin".to_owned());
        OnboardDraft::from_config(
            config,
            PathBuf::from("/tmp/loongclaw.toml"),
            Some(OnboardValueOrigin::DetectedStartingPoint),
        )
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl_c() -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
    }

    #[test]
    fn welcome_step_returns_next_on_enter() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source, OnboardTuiMode::Inline).unwrap();
        let result = runner.run_welcome_step();
        assert_eq!(result.unwrap(), OnboardFlowStepAction::Next);
    }

    #[test]
    fn welcome_step_returns_back_on_esc() {
        let events = vec![key(KeyCode::Esc)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source, OnboardTuiMode::Inline).unwrap();
        let result = runner.run_welcome_step();
        assert_eq!(result.unwrap(), OnboardFlowStepAction::Back);
    }

    #[test]
    fn welcome_step_returns_error_on_ctrl_c() {
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source, OnboardTuiMode::Inline).unwrap();
        let result = runner.run_welcome_step();
        assert!(result.is_err());
    }

    #[test]
    fn selection_loop_returns_selected_index() {
        let events = vec![key(KeyCode::Down), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source, OnboardTuiMode::Inline).unwrap();
        let items = vec![
            SelectionItem::new("A", None::<&str>),
            SelectionItem::new("B", None::<&str>),
        ];
        let result = runner
            .run_selection_loop(OnboardWizardStep::RuntimeDefaults, "Test", items, 0, "hint")
            .unwrap();
        assert!(matches!(result, SelectionLoopResult::Selected(1)));
    }

    #[test]
    fn selection_loop_wraps_around() {
        // Start at 0, go up (wraps to last), then enter
        let events = vec![key(KeyCode::Up), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source, OnboardTuiMode::Inline).unwrap();
        let items = vec![
            SelectionItem::new("A", None::<&str>),
            SelectionItem::new("B", None::<&str>),
            SelectionItem::new("C", None::<&str>),
        ];
        let result = runner
            .run_selection_loop(OnboardWizardStep::RuntimeDefaults, "Test", items, 0, "hint")
            .unwrap();
        assert!(matches!(result, SelectionLoopResult::Selected(2)));
    }

    #[test]
    fn input_loop_returns_typed_value() {
        let events = vec![
            key(KeyCode::Char('h')),
            key(KeyCode::Char('i')),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source, OnboardTuiMode::Inline).unwrap();
        let result = runner
            .run_input_loop(OnboardWizardStep::Workspace, "Label:", "", "hint")
            .unwrap();
        match result {
            InputLoopResult::Submitted(val) => assert_eq!(val, "hi"),
            InputLoopResult::Back => panic!("expected Submitted"),
        }
    }

    #[test]
    fn input_loop_returns_default_on_immediate_enter() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source, OnboardTuiMode::Inline).unwrap();
        let result = runner
            .run_input_loop(OnboardWizardStep::Workspace, "Label:", "/default", "hint")
            .unwrap();
        match result {
            InputLoopResult::Submitted(val) => assert_eq!(val, "/default"),
            InputLoopResult::Back => panic!("expected Submitted"),
        }
    }

    #[test]
    fn input_loop_handles_backspace() {
        let events = vec![
            key(KeyCode::Char('a')),
            key(KeyCode::Char('b')),
            key(KeyCode::Backspace),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source, OnboardTuiMode::Inline).unwrap();
        let result = runner
            .run_input_loop(OnboardWizardStep::Workspace, "Label:", "", "hint")
            .unwrap();
        match result {
            InputLoopResult::Submitted(val) => assert_eq!(val, "a"),
            InputLoopResult::Back => panic!("expected Submitted"),
        }
    }

    #[test]
    fn runtime_defaults_step_sets_memory_profile() {
        // Down once to select window_plus_summary, then Enter
        let events = vec![key(KeyCode::Down), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source, OnboardTuiMode::Inline).unwrap();
        let mut draft = sample_draft();
        let result = runner.run_runtime_defaults_step(&mut draft).unwrap();
        assert_eq!(result, OnboardFlowStepAction::Next);
        assert_eq!(
            draft.config.memory.profile,
            mvp::config::MemoryProfile::WindowPlusSummary
        );
    }

    #[test]
    fn workspace_step_sets_paths() {
        // Accept default sqlite path (Enter), then type custom file root + Enter
        let events = vec![
            key(KeyCode::Enter),
            key(KeyCode::Char('/')),
            key(KeyCode::Char('x')),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source, OnboardTuiMode::Inline).unwrap();
        let mut draft = sample_draft();
        let result = runner.run_workspace_step(&mut draft).unwrap();
        assert_eq!(result, OnboardFlowStepAction::Next);
        assert_eq!(draft.workspace.file_root, PathBuf::from("/x"));
    }

    #[test]
    fn protocols_step_disables_acp() {
        // Select "Disabled" (Down once), then Enter
        let events = vec![key(KeyCode::Down), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source, OnboardTuiMode::Inline).unwrap();
        let mut draft = sample_draft();
        draft.set_acp_enabled(true);
        let result = runner.run_protocols_step(&mut draft).unwrap();
        assert_eq!(result, OnboardFlowStepAction::Next);
        assert!(!draft.protocols.acp_enabled);
    }

    #[test]
    fn protocols_step_selects_backend_when_enabled() {
        // Draft starts with acp_enabled=true so the default selection is "Enabled" (idx 0).
        // Enter confirms Enabled, then Down selects "jsonrpc", Enter confirms.
        let events = vec![key(KeyCode::Enter), key(KeyCode::Down), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source, OnboardTuiMode::Inline).unwrap();
        let mut draft = sample_draft();
        draft.set_acp_enabled(true);
        let result = runner.run_protocols_step(&mut draft).unwrap();
        assert_eq!(result, OnboardFlowStepAction::Next);
        assert!(draft.protocols.acp_enabled);
        assert_eq!(draft.protocols.acp_backend.as_deref(), Some("jsonrpc"));
    }

    #[test]
    fn run_step_dispatches_post_boundary_steps_as_next() {
        let events: Vec<Event> = vec![];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source, OnboardTuiMode::Inline).unwrap();
        let mut draft = sample_draft();

        let env_check = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(runner.run_step(OnboardWizardStep::EnvironmentCheck, &mut draft));
        assert_eq!(env_check.unwrap(), OnboardFlowStepAction::Next);
    }
}
