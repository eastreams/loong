// RatatuiOnboardRunner: core TUI runner that implements GuidedOnboardFlowStepRunner.

use std::io::{self, Stdout};
use std::path::PathBuf;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal;
use loongclaw_app as mvp;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::event_source::{CrosstermEventSource, OnboardEventSource};
use super::layout;
use super::widgets::*;
use crate::CliResult;
use crate::onboard_flow::{GuidedOnboardFlowStepRunner, OnboardFlowStepAction};
use crate::onboard_state::{OnboardDraft, OnboardWizardStep};
use crate::provider_credential_policy;

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

enum StandaloneSelectionResult {
    Selected(usize),
    Cancel,
}

// ---------------------------------------------------------------------------
// RatatuiOnboardRunner
// ---------------------------------------------------------------------------

pub(crate) struct RatatuiOnboardRunner<E: OnboardEventSource = CrosstermEventSource> {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    event_source: E,
    owns_tty: bool,
}

impl RatatuiOnboardRunner<CrosstermEventSource> {
    /// Create a new runner that renders inline at the current cursor position.
    pub fn new() -> io::Result<Self> {
        // Install a panic hook that restores the terminal before printing.
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = terminal::disable_raw_mode();
            original_hook(info);
        }));

        terminal::enable_raw_mode()?;
        let (_, rows) = terminal::size().unwrap_or((80, 24));
        let viewport_height = rows.saturating_sub(2).min(30);
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::with_options(
            backend,
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Inline(viewport_height),
            },
        )?;

        Ok(Self {
            terminal,
            event_source: CrosstermEventSource,
            owns_tty: true,
        })
    }
}

impl<E: OnboardEventSource> RatatuiOnboardRunner<E> {
    /// Create a runner without touching the real terminal.
    ///
    /// Used in tests where raw-mode is unavailable.
    #[cfg(test)]
    fn headless(event_source: E) -> io::Result<Self> {
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
            owns_tty: false,
        })
    }

    // -----------------------------------------------------------------------
    // Terminal size guard
    // -----------------------------------------------------------------------

    /// Returns `true` when the terminal area is too small to render dialog
    /// boxes without garbled output.
    fn is_terminal_too_small(area: Rect) -> bool {
        area.width < 30 || area.height < 8
    }

    /// Render a minimal "resize your terminal" fallback message.
    fn render_too_small_fallback(frame: &mut ratatui::Frame<'_>) {
        let msg = Paragraph::new("Terminal too small.\nResize to at least 30x8.")
            .alignment(Alignment::Center);
        frame.render_widget(msg, frame.area());
    }

    // -----------------------------------------------------------------------
    // Dialog box helpers
    // -----------------------------------------------------------------------

    /// Compute a centered dialog box rect within the given area.
    /// Returns `(outer_rect, inner_rect)` where inner_rect has 2-char left
    /// padding inside the border.
    fn dialog_box_rect(area: Rect, content_lines: u16) -> (Rect, Rect) {
        let max_inner_width = (area.width.saturating_sub(4)).min(60);
        let box_height = content_lines + 2; // +2 for top/bottom border
        let box_width = max_inner_width + 2; // +2 for left/right border

        let x = area.x + (area.width.saturating_sub(box_width)) / 2;
        let y = area.y + (area.height.saturating_sub(box_height)) / 2;

        let outer = Rect::new(x, y, box_width.min(area.width), box_height.min(area.height));
        let inner = Rect::new(
            x + 3,
            y + 1,
            max_inner_width.saturating_sub(1),
            content_lines.min(area.height.saturating_sub(2)),
        );
        (outer, inner)
    }

    /// Draw the rounded border of a dialog box.
    fn draw_dialog_border(buf: &mut Buffer, rect: Rect, border_color: Color) {
        if rect.width < 2 || rect.height < 2 {
            return;
        }
        let style = Style::default().fg(border_color);
        // Top border: ╭───╮
        buf.set_string(rect.x, rect.y, "\u{256d}", style);
        for x in (rect.x + 1)..(rect.x + rect.width - 1) {
            buf.set_string(x, rect.y, "\u{2500}", style);
        }
        buf.set_string(rect.x + rect.width - 1, rect.y, "\u{256e}", style);

        // Side borders: │ ... │
        for y in (rect.y + 1)..(rect.y + rect.height - 1) {
            buf.set_string(rect.x, y, "\u{2502}", style);
            buf.set_string(rect.x + rect.width - 1, y, "\u{2502}", style);
        }

        // Bottom border: ╰───╯
        buf.set_string(rect.x, rect.y + rect.height - 1, "\u{2570}", style);
        for x in (rect.x + 1)..(rect.x + rect.width - 1) {
            buf.set_string(x, rect.y + rect.height - 1, "\u{2500}", style);
        }
        buf.set_string(
            rect.x + rect.width - 1,
            rect.y + rect.height - 1,
            "\u{256f}",
            style,
        );
    }

    /// Render body lines inside a centered dialog box. Returns the inner rect
    /// used for content, which callers can use for stateful widgets.
    fn render_dialog(
        buf: &mut Buffer,
        area: Rect,
        lines: &[Line<'_>],
        border_color: Color,
    ) -> Rect {
        let content_lines = lines.len() as u16;
        let (outer, inner) = Self::dialog_box_rect(area, content_lines);
        Self::draw_dialog_border(buf, outer, border_color);

        // Render each line inside the inner rect
        for (i, line) in lines.iter().enumerate() {
            let y = inner.y + i as u16;
            if y >= inner.y + inner.height {
                break;
            }
            buf.set_line(inner.x, y, line, inner.width);
        }

        inner
    }

    // -----------------------------------------------------------------------
    // Welcome step
    // -----------------------------------------------------------------------

    fn run_welcome_step(&mut self) -> CliResult<OnboardFlowStepAction> {
        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
        loop {
            let ver = version.clone();
            self.terminal
                .draw(|frame| {
                    if Self::is_terminal_too_small(frame.area()) {
                        Self::render_too_small_fallback(frame);
                        return;
                    }

                    let areas = layout::compute_layout(frame.area(), false);

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

                    // Welcome content inside dialog box
                    let dialog_lines: Vec<Line<'_>> = vec![
                        Line::from(""),
                        Line::from(Span::styled(
                            "LOONGCLAW",
                            Style::default()
                                .fg(Color::Rgb(245, 169, 127))
                                .add_modifier(Modifier::BOLD),
                        )),
                        Line::from(Span::styled(&ver, Style::default().fg(Color::DarkGray))),
                        Line::from(""),
                        Line::from(Span::styled(
                            "Setup Wizard",
                            Style::default().fg(Color::White),
                        )),
                        Line::from(""),
                        Line::from(Span::styled(
                            "This wizard will configure authentication,",
                            Style::default().fg(Color::Gray),
                        )),
                        Line::from(Span::styled(
                            "runtime defaults, workspace paths, protocols,",
                            Style::default().fg(Color::Gray),
                        )),
                        Line::from(Span::styled(
                            "and environment readiness.",
                            Style::default().fg(Color::Gray),
                        )),
                        Line::from(""),
                        Line::from(Span::styled(
                            "Safe to rerun. Press Enter to begin.",
                            Style::default().fg(Color::DarkGray),
                        )),
                    ];
                    Self::render_dialog(
                        frame.buffer_mut(),
                        areas.content,
                        &dialog_lines,
                        Color::Cyan,
                    );

                    // Footer
                    let footer_line = Line::from(vec![
                        Span::styled(" Press Enter to begin ", Style::default().fg(Color::Gray)),
                        Span::raw(" ".repeat((areas.footer.width as usize).saturating_sub(26))),
                        Span::styled(
                            format!(" 1/{} ", total_step_count()),
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
                }) => return Ok(OnboardFlowStepAction::Next),
                Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => return Err("onboarding cancelled".to_owned()),
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
        if items.is_empty() {
            return Err("no items to select from".to_owned());
        }

        let mut state = SelectionCardState::new(items.len());
        state.select(default_index);

        loop {
            let step_number = step_ordinal(step);
            let title_owned = title.to_owned();
            let hint_owned = footer_hint.to_owned();

            self.terminal
                .draw(|frame| {
                    if Self::is_terminal_too_small(frame.area()) {
                        Self::render_too_small_fallback(frame);
                        return;
                    }

                    let areas = layout::compute_layout(frame.area(), false);

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

                    // Content: title line + selection cards inside dialog box
                    // Compute dialog box for selection items
                    let lines_per_item: u16 = 3;
                    let item_count = items.len() as u16;
                    // title line + blank + items + title-prefix blank
                    let dialog_content_lines = 2 + item_count * lines_per_item - item_count.min(1);
                    let (outer, inner) = Self::dialog_box_rect(areas.content, dialog_content_lines);
                    Self::draw_dialog_border(frame.buffer_mut(), outer, Color::Cyan);

                    // Render title inside dialog
                    if inner.height > 0 {
                        let title_line = Line::from(vec![
                            Span::styled("? ", Style::default().fg(Color::Cyan)),
                            Span::styled(
                                &title_owned,
                                Style::default()
                                    .fg(Color::White)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]);
                        frame
                            .buffer_mut()
                            .set_line(inner.x, inner.y, &title_line, inner.width);
                    }

                    // Render selection widget in the remaining inner area
                    let sel_area = Rect::new(
                        inner.x,
                        inner.y + 2,
                        inner.width,
                        inner.height.saturating_sub(2),
                    );
                    let widget = SelectionCardWidget::new(
                        items
                            .iter()
                            .map(|i| SelectionItem::new(i.label.as_str(), i.hint.as_deref()))
                            .collect(),
                    );
                    frame.render_stateful_widget(widget, sel_area, &mut state);

                    // Footer
                    let footer_line = Line::from(vec![
                        Span::styled(format!(" {hint_owned} "), Style::default().fg(Color::Gray)),
                        Span::raw(
                            " ".repeat(
                                (areas.footer.width as usize)
                                    .saturating_sub(hint_owned.chars().count() + 10),
                            ),
                        ),
                        Span::styled(
                            format!(" {step_number}/{} ", total_step_count()),
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

        loop {
            let step_number = step_ordinal(step);
            let label_owned = label.to_owned();
            let hint_owned = footer_hint.to_owned();

            self.terminal
                .draw(|frame| {
                    if Self::is_terminal_too_small(frame.area()) {
                        Self::render_too_small_fallback(frame);
                        return;
                    }

                    let areas = layout::compute_layout(frame.area(), false);

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

                    // Content: text input inside dialog box
                    // Dialog: title + blank + input + error = 4 lines
                    let dialog_content_lines: u16 = 5;
                    let (outer, inner) = Self::dialog_box_rect(areas.content, dialog_content_lines);
                    Self::draw_dialog_border(frame.buffer_mut(), outer, Color::Cyan);

                    // Render title inside dialog
                    if inner.height > 0 {
                        let title_line = Line::from(vec![
                            Span::styled("? ", Style::default().fg(Color::Cyan)),
                            Span::styled(
                                &label_owned,
                                Style::default()
                                    .fg(Color::White)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]);
                        frame
                            .buffer_mut()
                            .set_line(inner.x, inner.y, &title_line, inner.width);
                    }

                    // Render text input widget below the title
                    let input_area = Rect::new(
                        inner.x,
                        inner.y + 2,
                        inner.width,
                        inner.height.saturating_sub(2),
                    );
                    let widget = TextInputWidget::new(&label_owned);
                    widget.render_with_state(input_area, frame.buffer_mut(), &input_state);

                    // Footer
                    let footer_line = Line::from(vec![
                        Span::styled(format!(" {hint_owned} "), Style::default().fg(Color::Gray)),
                        Span::raw(
                            " ".repeat(
                                (areas.footer.width as usize)
                                    .saturating_sub(hint_owned.chars().count() + 10),
                            ),
                        ),
                        Span::styled(
                            format!(" {step_number}/{} ", total_step_count()),
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
                    code: KeyCode::Delete,
                    ..
                }) => input_state.delete(),
                Event::Key(KeyEvent {
                    code: KeyCode::Left,
                    ..
                }) => input_state.move_left(),
                Event::Key(KeyEvent {
                    code: KeyCode::Right,
                    ..
                }) => input_state.move_right(),
                Event::Key(KeyEvent {
                    code: KeyCode::Home,
                    ..
                }) => input_state.move_home(),
                Event::Key(KeyEvent {
                    code: KeyCode::End, ..
                }) => input_state.move_end(),
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
        let mut sub_step: u8 = 0;
        loop {
            match sub_step {
                0 => {
                    // Sub-step 1: Provider confirmation (read-only, single item)
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
                        "Enter to continue with current provider",
                    )? {
                        SelectionLoopResult::Back => return Ok(OnboardFlowStepAction::Back),
                        SelectionLoopResult::Selected(_) => {
                            sub_step = 1;
                        }
                    }
                }
                1 => {
                    // Sub-step 2: Model (text input with current model as default)
                    let current_model = draft.config.provider.model.clone();
                    match self.run_input_loop(
                        OnboardWizardStep::Authentication,
                        "Model:",
                        &current_model,
                        "Enter to confirm model, or type a custom model",
                    )? {
                        InputLoopResult::Back => {
                            sub_step = 0;
                        }
                        InputLoopResult::Submitted(model) => {
                            if !model.is_empty() {
                                draft.set_provider_model(model);
                            }
                            sub_step = 2;
                        }
                    }
                }
                2 => {
                    // Sub-step 3: Credential environment variable
                    let current_credential_env =
                        provider_credential_policy::provider_credential_env_hint(
                            &draft.config.provider,
                        )
                        .unwrap_or_default();
                    match self.run_input_loop(
                        OnboardWizardStep::Authentication,
                        "API key env:",
                        &current_credential_env,
                        "Enter to confirm, or type a custom env var name",
                    )? {
                        InputLoopResult::Back => {
                            sub_step = 1;
                        }
                        InputLoopResult::Submitted(env_name) => {
                            draft.set_provider_credential_env(env_name);
                            return Ok(OnboardFlowStepAction::Next);
                        }
                    }
                }
                _ => {
                    return Err(format!(
                        "internal error: unexpected auth sub-step {sub_step}"
                    ));
                }
            }
        }
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
        let mut sub_step: u8 = 0;
        loop {
            match sub_step {
                0 => {
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
                            sub_step = 1;
                        }
                    }
                }
                1 => {
                    // Sub-step 2: File root
                    let current_file_root = draft.workspace.file_root.display().to_string();
                    match self.run_input_loop(
                        OnboardWizardStep::Workspace,
                        "File root:",
                        &current_file_root,
                        "Enter to confirm, or type a custom path",
                    )? {
                        InputLoopResult::Back => {
                            sub_step = 0;
                        }
                        InputLoopResult::Submitted(path) => {
                            if !path.is_empty() {
                                draft.set_workspace_file_root(PathBuf::from(path));
                            }
                            return Ok(OnboardFlowStepAction::Next);
                        }
                    }
                }
                _ => {
                    return Err(format!(
                        "internal error: unexpected workspace sub-step {sub_step}"
                    ));
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Pre/post-flow screens (no spine, full-width content)
    // -----------------------------------------------------------------------

    /// Generic confirmation screen: renders lines of content with a yes/no
    /// key binding.  Returns `true` when the user accepts.
    fn run_confirm_screen(
        &mut self,
        title: &str,
        body_lines: Vec<Line<'static>>,
        footer_hint: &str,
    ) -> CliResult<bool> {
        loop {
            let title_owned = title.to_owned();
            let hint_owned = footer_hint.to_owned();
            let lines = body_lines.clone();

            self.terminal
                .draw(|frame| {
                    if Self::is_terminal_too_small(frame.area()) {
                        Self::render_too_small_fallback(frame);
                        return;
                    }

                    // No spine for pre/post screens — pass `false`.
                    let areas = layout::compute_layout(frame.area(), false);

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
                        Span::styled("  Esc cancel", Style::default().fg(Color::DarkGray)),
                    ]);
                    frame.render_widget(Paragraph::new(header_line), areas.header);

                    // Content inside dialog box
                    Self::render_dialog(frame.buffer_mut(), areas.content, &lines, Color::DarkGray);

                    // Footer
                    let footer_line = Line::from(vec![Span::styled(
                        format!(" {hint_owned} "),
                        Style::default().fg(Color::Gray),
                    )]);
                    frame.render_widget(Paragraph::new(footer_line), areas.footer);
                })
                .map_err(|e| e.to_string())?;

            match self.event_source.next_event().map_err(|e| e.to_string())? {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter | KeyCode::Char('y' | 'Y'),
                    ..
                }) => return Ok(true),
                Event::Key(KeyEvent {
                    code: KeyCode::Char('n' | 'N') | KeyCode::Esc,
                    ..
                }) => return Ok(false),
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

    /// Generic "press Enter to continue" screen with scrollable content.
    fn run_info_screen(
        &mut self,
        title: &str,
        body_lines: Vec<Line<'static>>,
        footer_hint: &str,
    ) -> CliResult<()> {
        let mut scroll_offset: u16 = 0;
        let total_lines = body_lines.len() as u16;
        let mut captured_visible_height: u16 = total_lines;

        loop {
            let title_owned = title.to_owned();
            let hint_owned = footer_hint.to_owned();
            let lines = body_lines.clone();
            let offset = scroll_offset;

            self.terminal
                .draw(|frame| {
                    if Self::is_terminal_too_small(frame.area()) {
                        Self::render_too_small_fallback(frame);
                        return;
                    }

                    let areas = layout::compute_layout(frame.area(), false);

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
                        Span::styled("  Esc cancel", Style::default().fg(Color::DarkGray)),
                    ]);
                    frame.render_widget(Paragraph::new(header_line), areas.header);

                    // Content inside dialog box (scrollable)
                    let max_dialog_height = areas.content.height.saturating_sub(2);
                    let visible_lines = total_lines.min(max_dialog_height);
                    captured_visible_height = visible_lines;
                    let (outer, inner) = Self::dialog_box_rect(areas.content, visible_lines);
                    Self::draw_dialog_border(frame.buffer_mut(), outer, Color::DarkGray);

                    // Render visible slice of lines
                    let start = offset as usize;
                    for (i, line) in lines.iter().skip(start).enumerate() {
                        let y = inner.y + i as u16;
                        if y >= inner.y + inner.height {
                            break;
                        }
                        frame.buffer_mut().set_line(inner.x, y, line, inner.width);
                    }

                    // Footer
                    let footer_line = Line::from(vec![Span::styled(
                        format!(" {hint_owned} "),
                        Style::default().fg(Color::Gray),
                    )]);
                    frame.render_widget(Paragraph::new(footer_line), areas.footer);
                })
                .map_err(|e| e.to_string())?;

            match self.event_source.next_event().map_err(|e| e.to_string())? {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => return Ok(()),
                Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => return Ok(()),
                Event::Key(KeyEvent {
                    code: KeyCode::Down | KeyCode::Char('j'),
                    ..
                }) => {
                    if scroll_offset < total_lines.saturating_sub(captured_visible_height) {
                        scroll_offset += 1;
                    }
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Up | KeyCode::Char('k'),
                    ..
                }) => {
                    scroll_offset = scroll_offset.saturating_sub(1);
                }
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
    // Pre-flow: risk acknowledgement screen
    // -----------------------------------------------------------------------

    pub fn run_risk_screen(&mut self) -> CliResult<bool> {
        let body_lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("\u{26a0}  ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    "Security Check",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Review the trust boundary before writing",
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                "any config.",
                Style::default().fg(Color::White),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "What onboarding can do:",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "\u{2022} Invoke tools and read local files.",
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                "\u{2022} Keep credentials in env vars, not prompts.",
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                "\u{2022} Prefer allowlist-style tool policy.",
                Style::default().fg(Color::White),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Recommended baseline:",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "Start with the narrowest tool scope that",
                Style::default().fg(Color::Gray),
            )),
            Line::from(Span::styled(
                "lets you verify first success.",
                Style::default().fg(Color::Gray),
            )),
            Line::from(""),
        ];

        self.run_confirm_screen(
            "security check",
            body_lines,
            "Enter/y accept  n/Esc decline",
        )
    }

    // -----------------------------------------------------------------------
    // Pre-flow: entry choice (current / detected / start fresh)
    // -----------------------------------------------------------------------

    pub fn run_entry_choice_screen(
        &mut self,
        options: &[(String, String)],
        default_index: usize,
    ) -> CliResult<usize> {
        let items: Vec<SelectionItem> = options
            .iter()
            .map(|(label, detail)| SelectionItem::new(label.as_str(), Some(detail.as_str())))
            .collect();

        match self.run_standalone_selection_loop(
            "setup path",
            items,
            default_index,
            "Up/Down to select, Enter to confirm",
        )? {
            StandaloneSelectionResult::Selected(idx) => Ok(idx),
            StandaloneSelectionResult::Cancel => {
                Err("onboarding cancelled: entry choice declined".to_owned())
            }
        }
    }

    // -----------------------------------------------------------------------
    // Pre-flow: import candidate selection
    // -----------------------------------------------------------------------

    pub fn run_import_candidate_screen(
        &mut self,
        candidates: &[(String, String)],
        default_index: usize,
    ) -> CliResult<Option<usize>> {
        let mut items: Vec<SelectionItem> = candidates
            .iter()
            .map(|(label, detail)| SelectionItem::new(label.as_str(), Some(detail.as_str())))
            .collect();
        items.push(SelectionItem::new(
            "Start fresh",
            Some("begin with default config"),
        ));

        match self.run_standalone_selection_loop(
            "starting point",
            items,
            default_index,
            "Up/Down to select, Enter to confirm",
        )? {
            StandaloneSelectionResult::Selected(idx) if idx < candidates.len() => Ok(Some(idx)),
            StandaloneSelectionResult::Selected(_) => Ok(None),
            StandaloneSelectionResult::Cancel => {
                Err("onboarding cancelled: import selection cancelled".to_owned())
            }
        }
    }

    // -----------------------------------------------------------------------
    // Pre-flow: shortcut choice (use detected/current vs full setup)
    // -----------------------------------------------------------------------

    pub fn run_shortcut_choice_screen(
        &mut self,
        primary_label: &str,
        _snapshot_lines: &[String],
    ) -> CliResult<bool> {
        let items = vec![
            SelectionItem::new(primary_label, Some("skip detailed edits")),
            SelectionItem::new("Adjust settings", Some("go through full setup")),
        ];

        match self.run_standalone_selection_loop(
            "quick setup",
            items,
            0,
            "Up/Down to select, Enter to confirm",
        )? {
            StandaloneSelectionResult::Selected(0) => Ok(true),
            StandaloneSelectionResult::Selected(_) => Ok(false),
            StandaloneSelectionResult::Cancel => {
                Err("onboarding cancelled: shortcut choice cancelled".to_owned())
            }
        }
    }

    // -----------------------------------------------------------------------
    // Post-flow: preflight check results screen
    // -----------------------------------------------------------------------

    pub fn run_preflight_screen(
        &mut self,
        checks: &[crate::onboard_preflight::OnboardCheck],
    ) -> CliResult<bool> {
        let mut body_lines: Vec<Line<'static>> = Vec::new();
        body_lines.push(Line::from(""));
        body_lines.push(Line::from(Span::styled(
            "Preflight checks:",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        body_lines.push(Line::from(""));

        let mut has_warnings = false;
        for check in checks {
            let (icon, color) = match check.level {
                crate::onboard_preflight::OnboardCheckLevel::Pass => ("\u{2713}", Color::Green),
                crate::onboard_preflight::OnboardCheckLevel::Warn => {
                    has_warnings = true;
                    ("\u{26a0}", Color::Yellow)
                }
                crate::onboard_preflight::OnboardCheckLevel::Fail => ("\u{2717}", Color::Red),
            };
            body_lines.push(Line::from(vec![
                Span::styled(format!("{icon} "), Style::default().fg(color)),
                Span::styled(
                    check.name.to_owned(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}", check.detail),
                    Style::default().fg(Color::Gray),
                ),
            ]));
        }
        body_lines.push(Line::from(""));

        if has_warnings {
            self.run_confirm_screen(
                "preflight results",
                body_lines,
                "Enter/y continue with warnings  n/Esc cancel",
            )
        } else {
            // All green — just show the results and continue.
            self.run_info_screen("preflight results", body_lines, "Enter to continue")?;
            Ok(true)
        }
    }

    // -----------------------------------------------------------------------
    // Post-flow: review screen (scrollable config summary)
    // -----------------------------------------------------------------------

    pub fn run_review_screen(&mut self, review_lines: &[String]) -> CliResult<()> {
        let body_lines: Vec<Line<'static>> = review_lines
            .iter()
            .map(|line| {
                Line::from(Span::styled(
                    line.to_owned(),
                    Style::default().fg(Color::Gray),
                ))
            })
            .collect();

        self.run_info_screen(
            "review config",
            body_lines,
            "Up/Down scroll  Enter continue",
        )
    }

    // -----------------------------------------------------------------------
    // Post-flow: write confirmation screen
    // -----------------------------------------------------------------------

    pub fn run_write_confirmation_screen(
        &mut self,
        config_path: &str,
        warnings_kept: bool,
    ) -> CliResult<bool> {
        let status = if warnings_kept {
            "warnings were kept by choice"
        } else {
            "all checks green"
        };
        let body_lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("Config path: {config_path}"),
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                format!("Status: {status}"),
                Style::default().fg(if warnings_kept {
                    Color::Yellow
                } else {
                    Color::Green
                }),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Write this configuration?",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];

        self.run_confirm_screen("write config", body_lines, "Enter/y write  n/Esc cancel")
    }

    // -----------------------------------------------------------------------
    // Post-flow: success summary screen
    // -----------------------------------------------------------------------

    pub fn run_success_screen(&mut self, summary_lines: &[String]) -> CliResult<()> {
        let mut body_lines: Vec<Line<'static>> = Vec::new();
        body_lines.push(Line::from(""));
        body_lines.push(Line::from(vec![
            Span::styled("\u{2713}  ", Style::default().fg(Color::Green)),
            Span::styled(
                "Setup complete!",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        body_lines.push(Line::from(""));
        for line in summary_lines {
            body_lines.push(Line::from(Span::styled(
                line.to_owned(),
                Style::default().fg(Color::Gray),
            )));
        }
        body_lines.push(Line::from(""));

        self.run_info_screen("setup complete", body_lines, "Enter to exit")
    }

    // -----------------------------------------------------------------------
    // Standalone selection loop (no spine, for pre/post flow)
    // -----------------------------------------------------------------------

    fn run_standalone_selection_loop(
        &mut self,
        title: &str,
        items: Vec<SelectionItem>,
        default_index: usize,
        footer_hint: &str,
    ) -> CliResult<StandaloneSelectionResult> {
        if items.is_empty() {
            return Err("no items to select from".to_owned());
        }

        let mut state = SelectionCardState::new(items.len());
        state.select(default_index);

        loop {
            let title_owned = title.to_owned();
            let hint_owned = footer_hint.to_owned();

            self.terminal
                .draw(|frame| {
                    if Self::is_terminal_too_small(frame.area()) {
                        Self::render_too_small_fallback(frame);
                        return;
                    }

                    let areas = layout::compute_layout(frame.area(), false);

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
                        Span::styled("  Esc cancel", Style::default().fg(Color::DarkGray)),
                    ]);
                    frame.render_widget(Paragraph::new(header_line), areas.header);

                    // Content: selection cards inside dialog box
                    let lines_per_item: u16 = 3;
                    let item_count = items.len() as u16;
                    let dialog_content_lines = 2 + item_count * lines_per_item - item_count.min(1);
                    let (outer, inner) = Self::dialog_box_rect(areas.content, dialog_content_lines);
                    Self::draw_dialog_border(frame.buffer_mut(), outer, Color::Cyan);

                    // Render title inside dialog
                    if inner.height > 0 {
                        let dialog_title = Line::from(vec![
                            Span::styled("? ", Style::default().fg(Color::Cyan)),
                            Span::styled(
                                &title_owned,
                                Style::default()
                                    .fg(Color::White)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        ]);
                        frame
                            .buffer_mut()
                            .set_line(inner.x, inner.y, &dialog_title, inner.width);
                    }

                    // Render selection widget below the title
                    let sel_area = Rect::new(
                        inner.x,
                        inner.y + 2,
                        inner.width,
                        inner.height.saturating_sub(2),
                    );
                    let widget = SelectionCardWidget::new(
                        items
                            .iter()
                            .map(|i| SelectionItem::new(i.label.as_str(), i.hint.as_deref()))
                            .collect(),
                    );
                    frame.render_stateful_widget(widget, sel_area, &mut state);

                    // Footer
                    let footer_line = Line::from(vec![Span::styled(
                        format!(" {hint_owned} "),
                        Style::default().fg(Color::Gray),
                    )]);
                    frame.render_widget(Paragraph::new(footer_line), areas.footer);
                })
                .map_err(|e| e.to_string())?;

            match self.event_source.next_event().map_err(|e| e.to_string())? {
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => return Ok(StandaloneSelectionResult::Selected(state.selected())),
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
                }) => return Ok(StandaloneSelectionResult::Cancel),
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
    // Protocols step
    // -----------------------------------------------------------------------

    fn run_protocols_step(&mut self, draft: &mut OnboardDraft) -> CliResult<OnboardFlowStepAction> {
        let mut sub_step: u8 = 0;
        loop {
            match sub_step {
                0 => {
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
                            sub_step = 1;
                        }
                    }
                }
                1 => {
                    // Sub-step 2: ACP backend selection (only if enabled)
                    let backends = ["builtin", "jsonrpc"];
                    let current_backend =
                        draft.protocols.acp_backend.as_deref().unwrap_or("builtin");
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
                        SelectionLoopResult::Back => {
                            sub_step = 0;
                        }
                        SelectionLoopResult::Selected(idx) => {
                            if let Some(backend) = backends.get(idx) {
                                draft.set_acp_backend(Some((*backend).to_owned()));
                            }
                            return Ok(OnboardFlowStepAction::Next);
                        }
                    }
                }
                _ => {
                    return Err(format!(
                        "internal error: unexpected protocols sub-step {sub_step}"
                    ));
                }
            }
        }
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

fn total_step_count() -> usize {
    use crate::onboard_flow::OnboardFlowController;
    OnboardFlowController::ordered_steps().len()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use ratatui::widgets::StatefulWidget;

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
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner.run_welcome_step();
        assert_eq!(result.unwrap(), OnboardFlowStepAction::Next);
    }

    #[test]
    fn welcome_step_returns_error_on_esc() {
        let events = vec![key(KeyCode::Esc)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner.run_welcome_step();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "onboarding cancelled");
    }

    #[test]
    fn welcome_step_returns_error_on_ctrl_c() {
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner.run_welcome_step();
        assert!(result.is_err());
    }

    #[test]
    fn selection_loop_returns_selected_index() {
        let events = vec![key(KeyCode::Down), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
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
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
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
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
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
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
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
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
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
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
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
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
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
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
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
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
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
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();

        let env_check = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(runner.run_step(OnboardWizardStep::EnvironmentCheck, &mut draft));
        assert_eq!(env_check.unwrap(), OnboardFlowStepAction::Next);
    }

    // -----------------------------------------------------------------------
    // Pre/post-flow screen tests
    // -----------------------------------------------------------------------

    #[test]
    fn risk_screen_accepts_on_enter() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(runner.run_risk_screen().unwrap());
    }

    #[test]
    fn risk_screen_accepts_on_y() {
        let events = vec![key(KeyCode::Char('y'))];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(runner.run_risk_screen().unwrap());
    }

    #[test]
    fn risk_screen_declines_on_n() {
        let events = vec![key(KeyCode::Char('n'))];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(!runner.run_risk_screen().unwrap());
    }

    #[test]
    fn risk_screen_declines_on_esc() {
        let events = vec![key(KeyCode::Esc)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(!runner.run_risk_screen().unwrap());
    }

    #[test]
    fn entry_choice_screen_selects_default() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let options = vec![
            ("Current".to_owned(), "use existing".to_owned()),
            ("Fresh".to_owned(), "start fresh".to_owned()),
        ];
        let idx = runner.run_entry_choice_screen(&options, 0).unwrap();
        assert_eq!(idx, 0);
    }

    #[test]
    fn entry_choice_screen_selects_second() {
        let events = vec![key(KeyCode::Down), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let options = vec![
            ("Current".to_owned(), "use existing".to_owned()),
            ("Fresh".to_owned(), "start fresh".to_owned()),
        ];
        let idx = runner.run_entry_choice_screen(&options, 0).unwrap();
        assert_eq!(idx, 1);
    }

    #[test]
    fn shortcut_choice_screen_returns_true_for_primary() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner
            .run_shortcut_choice_screen("Use current setup", &["provider: openai".to_owned()])
            .unwrap();
        assert!(result);
    }

    #[test]
    fn shortcut_choice_screen_returns_false_for_adjust() {
        let events = vec![key(KeyCode::Down), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner
            .run_shortcut_choice_screen("Use current setup", &["provider: openai".to_owned()])
            .unwrap();
        assert!(!result);
    }

    #[test]
    fn preflight_screen_passes_all_green() {
        let checks = vec![crate::onboard_preflight::OnboardCheck {
            name: "provider credentials",
            level: crate::onboard_preflight::OnboardCheckLevel::Pass,
            detail: "env binding found".to_owned(),
            non_interactive_warning_policy:
                crate::onboard_preflight::OnboardNonInteractiveWarningPolicy::default(),
        }];
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(runner.run_preflight_screen(&checks).unwrap());
    }

    #[test]
    fn preflight_screen_with_warning_accepts_on_enter() {
        let checks = vec![crate::onboard_preflight::OnboardCheck {
            name: "model probe",
            level: crate::onboard_preflight::OnboardCheckLevel::Warn,
            detail: "model not verified".to_owned(),
            non_interactive_warning_policy:
                crate::onboard_preflight::OnboardNonInteractiveWarningPolicy::default(),
        }];
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(runner.run_preflight_screen(&checks).unwrap());
    }

    #[test]
    fn preflight_screen_with_warning_declines_on_n() {
        let checks = vec![crate::onboard_preflight::OnboardCheck {
            name: "model probe",
            level: crate::onboard_preflight::OnboardCheckLevel::Warn,
            detail: "model not verified".to_owned(),
            non_interactive_warning_policy:
                crate::onboard_preflight::OnboardNonInteractiveWarningPolicy::default(),
        }];
        let events = vec![key(KeyCode::Char('n'))];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(!runner.run_preflight_screen(&checks).unwrap());
    }

    #[test]
    fn review_screen_continues_on_enter() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        runner
            .run_review_screen(&["provider: openai".to_owned(), "model: gpt-4".to_owned()])
            .unwrap();
    }

    #[test]
    fn write_confirmation_screen_accepts_on_enter() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(
            runner
                .run_write_confirmation_screen("/tmp/loongclaw.toml", false)
                .unwrap()
        );
    }

    #[test]
    fn write_confirmation_screen_declines_on_n() {
        let events = vec![key(KeyCode::Char('n'))];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        assert!(
            !runner
                .run_write_confirmation_screen("/tmp/loongclaw.toml", false)
                .unwrap()
        );
    }

    #[test]
    fn success_screen_exits_on_enter() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        runner
            .run_success_screen(&["config written".to_owned()])
            .unwrap();
    }

    #[test]
    fn import_candidate_screen_selects_first() {
        let events = vec![key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let candidates = vec![("codex config".to_owned(), "~/.codex/config.json".to_owned())];
        let result = runner.run_import_candidate_screen(&candidates, 0).unwrap();
        assert_eq!(result, Some(0));
    }

    #[test]
    fn import_candidate_screen_selects_start_fresh() {
        // Navigate past all candidates to the "Start fresh" item
        let events = vec![key(KeyCode::Down), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let candidates = vec![("codex config".to_owned(), "~/.codex/config.json".to_owned())];
        let result = runner.run_import_candidate_screen(&candidates, 0).unwrap();
        assert_eq!(result, None);
    }

    // -----------------------------------------------------------------------
    // Integration-level tests: end-to-end guided flow through multiple screens
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "current_thread")]
    async fn guided_flow_completes_with_scripted_events() {
        use crate::onboard_flow::{OnboardFlowController, run_guided_onboard_flow};

        // The sample draft has acp.enabled = false (default), so Protocols
        // presents "Disabled" as the pre-selected choice (idx 1).  A single
        // Enter on that screen therefore picks "Disabled" and returns Next.
        //
        // Event sequence per step:
        //   Welcome:         Enter
        //   Auth sub0:       Enter  (provider confirmation)
        //   Auth sub1:       Enter  (model, accept default)
        //   Auth sub2:       Enter  (api key env, accept default)
        //   RuntimeDefaults: Enter  (memory profile, accept default)
        //   Workspace sub0:  Enter  (sqlite path, accept default)
        //   Workspace sub1:  Enter  (file root, accept default)
        //   Protocols sub0:  Enter  (ACP Disabled selected by default)
        let events: Vec<Event> = vec![key(KeyCode::Enter); 8];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();

        let draft = sample_draft();
        let controller = OnboardFlowController::new(draft);
        let controller = run_guided_onboard_flow(controller, &mut runner)
            .await
            .expect("guided flow should complete");

        assert_eq!(
            controller.current_step(),
            OnboardWizardStep::EnvironmentCheck,
            "flow should stop at EnvironmentCheck boundary"
        );
        // Protocols was accepted as Disabled
        assert!(
            !controller.draft().protocols.acp_enabled,
            "ACP should remain disabled when user accepted default"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn guided_flow_with_acp_enabled_completes() {
        use crate::onboard_flow::{OnboardFlowController, run_guided_onboard_flow};

        // Same as above, but start with acp.enabled = true so the Protocols
        // step requires two selections (toggle + backend).
        //
        //   Welcome:         Enter
        //   Auth sub0-2:     Enter x3
        //   RuntimeDefaults: Enter
        //   Workspace sub0-1:Enter x2
        //   Protocols sub0:  Enter  (Enabled, pre-selected)
        //   Protocols sub1:  Down + Enter  (select jsonrpc backend)
        let mut events: Vec<Event> = vec![key(KeyCode::Enter); 8];
        events.push(key(KeyCode::Down));
        events.push(key(KeyCode::Enter));
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();

        let mut draft = sample_draft();
        draft.set_acp_enabled(true);
        let controller = OnboardFlowController::new(draft);
        let controller = run_guided_onboard_flow(controller, &mut runner)
            .await
            .expect("guided flow with ACP enabled should complete");

        assert_eq!(
            controller.current_step(),
            OnboardWizardStep::EnvironmentCheck
        );
        assert!(controller.draft().protocols.acp_enabled);
        assert_eq!(
            controller.draft().protocols.acp_backend.as_deref(),
            Some("jsonrpc"),
            "second backend option should be jsonrpc"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn guided_flow_back_from_auth_returns_to_welcome() {
        use crate::onboard_flow::{OnboardFlowController, run_guided_onboard_flow};

        // Press Enter on Welcome, then Esc on Auth sub-step 0 (provider),
        // which makes Auth return Back.  The flow should revisit Welcome,
        // then proceed normally with Enter through all remaining steps.
        let events = vec![
            key(KeyCode::Enter), // Welcome -> Next
            key(KeyCode::Esc),   // Auth sub0 -> Back -> revisit Welcome
            key(KeyCode::Enter), // Welcome (replay) -> Next
            key(KeyCode::Enter), // Auth sub0
            key(KeyCode::Enter), // Auth sub1
            key(KeyCode::Enter), // Auth sub2
            key(KeyCode::Enter), // RuntimeDefaults
            key(KeyCode::Enter), // Workspace sub0
            key(KeyCode::Enter), // Workspace sub1
            key(KeyCode::Enter), // Protocols (Disabled)
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();

        let controller = OnboardFlowController::new(sample_draft());
        let controller = run_guided_onboard_flow(controller, &mut runner)
            .await
            .expect("flow with back from auth should complete");

        assert_eq!(
            controller.current_step(),
            OnboardWizardStep::EnvironmentCheck,
            "flow should still reach EnvironmentCheck after back-navigation"
        );
    }

    #[test]
    fn auth_step_back_from_model_returns_to_provider() {
        // Within the Authentication step, navigate into model (sub-step 1),
        // press Esc to go back to provider (sub-step 0), then proceed through
        // all sub-steps with Enter.
        let events = vec![
            key(KeyCode::Enter), // sub0: provider -> sub1
            key(KeyCode::Esc),   // sub1: model -> back to sub0
            key(KeyCode::Enter), // sub0: provider (replay) -> sub1
            key(KeyCode::Enter), // sub1: model -> sub2
            key(KeyCode::Enter), // sub2: api key env -> Next
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        let result = runner.run_authentication_step(&mut draft).unwrap();
        assert_eq!(
            result,
            OnboardFlowStepAction::Next,
            "auth step should complete successfully after sub-step back-nav"
        );
    }

    #[test]
    fn ctrl_c_on_welcome_returns_interrupted_error() {
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let err = runner.run_welcome_step().unwrap_err();
        assert!(
            err.contains("interrupted"),
            "error should mention interrupted: {err}"
        );
    }

    #[test]
    fn ctrl_c_on_auth_returns_interrupted_error() {
        // Ctrl-C on provider selection sub-step
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        let err = runner.run_authentication_step(&mut draft).unwrap_err();
        assert!(
            err.contains("interrupted"),
            "error should mention interrupted: {err}"
        );
    }

    #[test]
    fn ctrl_c_on_runtime_defaults_returns_interrupted_error() {
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        let err = runner.run_runtime_defaults_step(&mut draft).unwrap_err();
        assert!(
            err.contains("interrupted"),
            "error should mention interrupted: {err}"
        );
    }

    #[test]
    fn ctrl_c_on_workspace_returns_interrupted_error() {
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        let err = runner.run_workspace_step(&mut draft).unwrap_err();
        assert!(
            err.contains("interrupted"),
            "error should mention interrupted: {err}"
        );
    }

    #[test]
    fn ctrl_c_on_protocols_returns_interrupted_error() {
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let mut draft = sample_draft();
        let err = runner.run_protocols_step(&mut draft).unwrap_err();
        assert!(
            err.contains("interrupted"),
            "error should mention interrupted: {err}"
        );
    }

    #[test]
    fn ctrl_c_on_risk_screen_returns_interrupted_error() {
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let err = runner.run_risk_screen().unwrap_err();
        assert!(
            err.contains("interrupted"),
            "error should mention interrupted: {err}"
        );
    }

    #[test]
    fn ctrl_c_on_entry_choice_returns_interrupted_error() {
        let events = vec![ctrl_c()];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let options = vec![("A".to_owned(), "option a".to_owned())];
        let err = runner.run_entry_choice_screen(&options, 0).unwrap_err();
        assert!(
            err.contains("interrupted"),
            "error should mention interrupted: {err}"
        );
    }

    #[test]
    fn resize_events_are_handled_gracefully() {
        // Inject a resize event before the Enter that completes the welcome step.
        // The resize should be silently consumed (triggering a redraw) without panic.
        let events = vec![Event::Resize(120, 40), key(KeyCode::Enter)];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let result = runner.run_welcome_step();
        assert_eq!(
            result.unwrap(),
            OnboardFlowStepAction::Next,
            "resize events should be consumed gracefully"
        );
    }

    #[test]
    fn resize_during_selection_loop_is_handled() {
        let events = vec![
            Event::Resize(100, 50),
            key(KeyCode::Down),
            Event::Resize(80, 24),
            key(KeyCode::Enter),
        ];
        let source = ScriptedEventSource::new(events);
        let mut runner = RatatuiOnboardRunner::headless(source).unwrap();
        let items = vec![
            SelectionItem::new("A", None::<&str>),
            SelectionItem::new("B", None::<&str>),
        ];
        let result = runner
            .run_selection_loop(OnboardWizardStep::RuntimeDefaults, "Test", items, 0, "hint")
            .unwrap();
        assert!(
            matches!(result, SelectionLoopResult::Selected(1)),
            "selection should complete normally despite resize events"
        );
    }

    #[test]
    fn selection_card_state_with_zero_items_does_not_panic() {
        // SelectionCardState with zero items: next/previous should be no-ops.
        let mut state = SelectionCardState::new(0);
        state.next();
        state.previous();
        assert_eq!(state.selected(), 0, "selected index should remain 0");

        // SelectionCardWidget with empty items renders without panic.
        let widget = SelectionCardWidget::new(vec![]);
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf, &mut state);
    }
}
