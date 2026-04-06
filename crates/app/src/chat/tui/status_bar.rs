use std::time::{Duration, Instant};

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::focus::FocusLayer;
use super::state::BusyInputMode;
use super::theme::Palette;

// ---------------------------------------------------------------------------
// View trait — decouples rendering from the concrete `Pane` struct
// ---------------------------------------------------------------------------

pub(super) trait StatusBarView {
    fn model(&self) -> &str;
    fn input_tokens(&self) -> u32;
    fn output_tokens(&self) -> u32;
    fn context_length(&self) -> u32;
    fn session_id(&self) -> &str;
    fn session_display_label(&self) -> Option<&str> {
        None
    }
    fn busy_input_mode(&self) -> BusyInputMode;
    fn pending_submission_count(&self) -> usize {
        0
    }
    fn running_task_count(&self) -> Option<usize> {
        None
    }
    fn overdue_task_count(&self) -> Option<usize> {
        None
    }
    fn pending_approval_count(&self) -> Option<usize> {
        None
    }
    fn attention_approval_count(&self) -> Option<usize> {
        None
    }
    fn visible_session_count(&self) -> Option<usize> {
        None
    }
    fn scroll_offset(&self) -> u16 {
        0
    }
    fn transcript_selection_line_count(&self) -> usize {
        0
    }
    /// Returns the current status message and the instant it was set.
    /// Defaults to `None` so existing implementations don't break.
    fn status_message(&self) -> Option<(&str, &Instant)> {
        None
    }
}

pub(super) fn render_status_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    pane: &impl StatusBarView,
    focus: FocusLayer,
    palette: &Palette,
) {
    let model_display = truncate_str(pane.model(), 24, "no model");
    let raw_session_display = pane
        .session_display_label()
        .unwrap_or_else(|| pane.session_id());
    let session_display = truncate_str(raw_session_display, 16, "no session");

    let total = pane.input_tokens().saturating_add(pane.output_tokens());
    let ctx = pane.context_length();
    let pct = if ctx == 0 {
        0.0f32
    } else {
        (total as f32 / ctx as f32) * 100.0
    };

    let token_spans = if ctx == 0 {
        // Unknown context window — show dash instead of misleading 0%
        vec![
            Span::styled(format!("{total}"), Style::default().fg(palette.info)),
            Span::styled(" tokens".to_string(), Style::default().fg(palette.dim)),
            Span::styled(" (\u{2014})".to_string(), Style::default().fg(palette.dim)),
        ]
    } else {
        let pct_style = context_percent_style(pct, palette);
        vec![
            Span::styled(format!("{total}"), Style::default().fg(palette.info)),
            Span::styled(" tokens".to_string(), Style::default().fg(palette.dim)),
            Span::styled(format!(" ({pct:.0}%)"), pct_style),
        ]
    };

    // Check for a non-expired status message (3-second window).
    let status_span: Option<Span<'_>> = pane
        .status_message()
        .filter(|(_, when)| when.elapsed() < Duration::from_secs(3))
        .map(|(msg, _)| {
            Span::styled(
                format!(" · {msg}"),
                Style::default()
                    .fg(palette.dim)
                    .add_modifier(Modifier::ITALIC),
            )
        });

    let mut spans = vec![
        Span::styled(
            format!(" {model_display}"),
            Style::default().fg(palette.dim),
        ),
        Span::styled(" · ".to_string(), Style::default().fg(palette.separator)),
    ];
    spans.extend(token_spans);
    spans.push(Span::styled(
        " · ".to_string(),
        Style::default().fg(palette.separator),
    ));
    spans.push(Span::styled(
        session_display,
        Style::default().fg(palette.dim),
    ));
    spans.push(Span::styled(
        " · ".to_string(),
        Style::default().fg(palette.separator),
    ));
    spans.push(scroll_state_span(pane.scroll_offset(), palette));
    spans.push(Span::styled(
        " · ".to_string(),
        Style::default().fg(palette.separator),
    ));
    spans.push(focus_state_span(focus, palette));
    spans.push(Span::styled(
        " · ".to_string(),
        Style::default().fg(palette.separator),
    ));
    spans.push(Span::styled(
        pane.busy_input_mode().label().to_owned(),
        Style::default().fg(palette.brand),
    ));
    let pending_submission_count = pane.pending_submission_count();
    if pending_submission_count > 0 {
        spans.push(Span::styled(
            " · ".to_string(),
            Style::default().fg(palette.separator),
        ));
        spans.push(Span::styled(
            format!("PEND {pending_submission_count}"),
            Style::default()
                .fg(palette.warning)
                .add_modifier(Modifier::BOLD),
        ));
    }

    append_activity_spans(&mut spans, pane, palette);

    let transcript_selection_line_count = pane.transcript_selection_line_count();
    if focus == FocusLayer::Transcript && transcript_selection_line_count > 0 {
        spans.push(Span::styled(
            " · ".to_string(),
            Style::default().fg(palette.separator),
        ));
        spans.push(Span::styled(
            format!("SEL {transcript_selection_line_count}"),
            Style::default()
                .fg(palette.warning)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(s) = status_span {
        spans.push(s);
    }

    let line = Line::from(spans);

    frame.render_widget(Paragraph::new(line), area);
}

fn append_activity_spans(
    spans: &mut Vec<Span<'static>>,
    pane: &impl StatusBarView,
    palette: &Palette,
) {
    let attention_approval_count = pane.attention_approval_count().unwrap_or(0);
    if attention_approval_count > 0 {
        let label = format!("APR! {attention_approval_count}");
        push_activity_span(spans, label, palette.warning, palette);
    }

    let pending_approval_count = pane.pending_approval_count().unwrap_or(0);
    let remaining_approval_count = pending_approval_count.saturating_sub(attention_approval_count);
    if remaining_approval_count > 0 {
        let label = format!("APR {remaining_approval_count}");
        push_activity_span(spans, label, palette.info, palette);
    }

    let overdue_task_count = pane.overdue_task_count().unwrap_or(0);
    if overdue_task_count > 0 {
        let label = format!("LATE {overdue_task_count}");
        push_activity_span(spans, label, palette.error, palette);
    }

    let running_task_count = pane.running_task_count().unwrap_or(0);
    if running_task_count > 0 {
        let label = format!("TASK {running_task_count}");
        push_activity_span(spans, label, palette.tool_running, palette);
    }

    let visible_session_count = pane.visible_session_count().unwrap_or(0);
    if visible_session_count > 1 {
        let label = format!("SESS {visible_session_count}");
        push_activity_span(spans, label, palette.brand, palette);
    }
}

fn push_activity_span(
    spans: &mut Vec<Span<'static>>,
    label: String,
    color: ratatui::style::Color,
    palette: &Palette,
) {
    spans.push(Span::styled(
        " · ".to_string(),
        Style::default().fg(palette.separator),
    ));
    spans.push(Span::styled(
        label,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ));
}

fn scroll_state_span(scroll_offset: u16, palette: &Palette) -> Span<'static> {
    let is_tail_following = scroll_offset == 0;
    let label = if is_tail_following {
        "LIVE"
    } else {
        "SCROLLED"
    };
    let color = if is_tail_following {
        palette.success
    } else {
        palette.warning
    };
    let style = Style::default().fg(color).add_modifier(Modifier::BOLD);

    Span::styled(label.to_string(), style)
}

fn focus_state_span(focus: FocusLayer, palette: &Palette) -> Span<'static> {
    let (label, color) = match focus {
        FocusLayer::Composer => ("COMPOSE", palette.info),
        FocusLayer::Transcript => ("REVIEW", palette.warning),
        FocusLayer::Help => ("HELP", palette.brand),
        FocusLayer::SessionPicker => ("PICKER", palette.brand),
        FocusLayer::StatsOverlay => ("STATS", palette.brand),
        FocusLayer::ToolInspector => ("TOOL", palette.tool_running),
        FocusLayer::ClarifyDialog => ("QUESTION", palette.warning),
    };
    let style = Style::default().fg(color).add_modifier(Modifier::BOLD);

    Span::styled(label.to_string(), style)
}

/// Truncate a string to `max_chars`, showing ellipsis if shortened.
/// Returns `fallback` if the input is empty.
fn truncate_str(s: &str, max_chars: usize, fallback: &str) -> String {
    if s.is_empty() {
        return fallback.to_string();
    }
    let char_count = s.chars().count();
    if char_count <= max_chars {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .nth(max_chars.saturating_sub(1))
            .map_or(s.len(), |(i, _)| i);
        let truncated = s.get(..end).unwrap_or(s);
        format!("{truncated}\u{2026}")
    }
}

/// Color the context percentage according to thresholds:
/// <50% green, 50-75% warning, 75-90% error, >90% bold error.
fn context_percent_style(pct: f32, palette: &Palette) -> Style {
    if pct < 50.0 {
        Style::default().fg(palette.success)
    } else if pct < 75.0 {
        Style::default().fg(palette.warning)
    } else if pct < 90.0 {
        Style::default().fg(palette.error)
    } else {
        Style::default()
            .fg(palette.error)
            .add_modifier(Modifier::BOLD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    struct TestBar {
        model: String,
        input_tokens: u32,
        output_tokens: u32,
        context_length: u32,
        session_id: String,
        busy_input_mode: BusyInputMode,
        pending_submission_count: usize,
        running_task_count: Option<usize>,
        overdue_task_count: Option<usize>,
        pending_approval_count: Option<usize>,
        attention_approval_count: Option<usize>,
        visible_session_count: Option<usize>,
        scroll_offset: u16,
        status_message: Option<(String, Instant)>,
    }

    impl StatusBarView for TestBar {
        fn model(&self) -> &str {
            &self.model
        }
        fn input_tokens(&self) -> u32 {
            self.input_tokens
        }
        fn output_tokens(&self) -> u32 {
            self.output_tokens
        }
        fn context_length(&self) -> u32 {
            self.context_length
        }
        fn session_id(&self) -> &str {
            &self.session_id
        }
        fn busy_input_mode(&self) -> BusyInputMode {
            self.busy_input_mode
        }
        fn pending_submission_count(&self) -> usize {
            self.pending_submission_count
        }
        fn running_task_count(&self) -> Option<usize> {
            self.running_task_count
        }
        fn overdue_task_count(&self) -> Option<usize> {
            self.overdue_task_count
        }
        fn pending_approval_count(&self) -> Option<usize> {
            self.pending_approval_count
        }
        fn attention_approval_count(&self) -> Option<usize> {
            self.attention_approval_count
        }
        fn visible_session_count(&self) -> Option<usize> {
            self.visible_session_count
        }
        fn scroll_offset(&self) -> u16 {
            self.scroll_offset
        }
        fn transcript_selection_line_count(&self) -> usize {
            0
        }
        fn status_message(&self) -> Option<(&str, &Instant)> {
            self.status_message.as_ref().map(|(s, i)| (s.as_str(), i))
        }
    }

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push(
                    buf.cell((x, y))
                        .map_or(' ', |c| c.symbol().chars().next().unwrap_or(' ')),
                );
            }
        }
        out
    }

    #[test]
    fn status_bar_shows_model_and_tokens() {
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let bar = TestBar {
            model: "claude-3.5-sonnet".into(),
            input_tokens: 1000,
            output_tokens: 234,
            context_length: 10000,
            session_id: "sess-abc123".into(),
            busy_input_mode: BusyInputMode::Queue,
            pending_submission_count: 0,
            running_task_count: None,
            overdue_task_count: None,
            pending_approval_count: None,
            attention_approval_count: None,
            visible_session_count: None,
            scroll_offset: 0,
            status_message: None,
        };
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_status_bar(f, f.area(), &bar, FocusLayer::Composer, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(text.contains("claude-3.5-sonnet"));
        assert!(text.contains("1234")); // 1000 + 234
        assert!(text.contains("12%")); // 1234/10000 ~= 12%
        assert!(text.contains("sess-abc123"));
        assert!(text.contains("QUEUE"));
    }

    #[test]
    fn status_bar_shows_activity_spans_when_context_has_tasks_and_approvals() {
        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let bar = TestBar {
            model: "gpt-5".into(),
            input_tokens: 400,
            output_tokens: 100,
            context_length: 10000,
            session_id: "sess-ops".into(),
            busy_input_mode: BusyInputMode::Queue,
            pending_submission_count: 1,
            running_task_count: Some(3),
            overdue_task_count: Some(1),
            pending_approval_count: Some(4),
            attention_approval_count: Some(2),
            visible_session_count: Some(5),
            scroll_offset: 0,
            status_message: None,
        };
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_status_bar(f, f.area(), &bar, FocusLayer::Composer, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);

        assert!(text.contains("PEND 1"));
        assert!(text.contains("APR! 2"));
        assert!(text.contains("APR 2"));
        assert!(text.contains("LATE 1"));
        assert!(text.contains("TASK 3"));
        assert!(text.contains("SESS 5"));
    }

    #[test]
    fn model_truncated_at_24_chars() {
        let result = truncate_str("anthropic-claude-3.5-opus-2026-01-01", 24, "no model");
        assert!(result.chars().count() <= 24);
        assert!(result.ends_with('\u{2026}'));
    }

    #[test]
    fn session_id_truncated_at_16_chars() {
        let result = truncate_str("very-long-session-identifier", 16, "no session");
        assert!(result.chars().count() <= 16);
        assert!(result.ends_with('\u{2026}'));
    }

    #[test]
    fn empty_model_shows_fallback() {
        assert_eq!(truncate_str("", 24, "no model"), "no model");
    }

    #[test]
    fn empty_session_shows_fallback() {
        assert_eq!(truncate_str("", 16, "no session"), "no session");
    }

    #[test]
    fn context_percent_colors() {
        let palette = Palette::dark();

        let s = context_percent_style(10.0, &palette);
        assert_eq!(s.fg, Some(palette.success));

        let s = context_percent_style(60.0, &palette);
        assert_eq!(s.fg, Some(palette.warning));

        let s = context_percent_style(80.0, &palette);
        assert_eq!(s.fg, Some(palette.error));

        let s = context_percent_style(95.0, &palette);
        assert_eq!(s.fg, Some(palette.error));
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn zero_context_length_no_panic() {
        let backend = TestBackend::new(60, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let bar = TestBar {
            model: "gpt-4".into(),
            input_tokens: 500,
            output_tokens: 100,
            context_length: 0,
            session_id: "s1".into(),
            busy_input_mode: BusyInputMode::Queue,
            pending_submission_count: 0,
            running_task_count: None,
            overdue_task_count: None,
            pending_approval_count: None,
            attention_approval_count: None,
            visible_session_count: None,
            scroll_offset: 0,
            status_message: None,
        };
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_status_bar(f, f.area(), &bar, FocusLayer::Composer, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(text.contains("gpt-4"));
        assert!(text.contains("\u{2014}")); // 0 context shows em-dash instead of 0%
    }

    #[test]
    fn tail_follow_status_bar_shows_live() {
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let bar = TestBar {
            model: "gpt-4".into(),
            input_tokens: 0,
            output_tokens: 0,
            context_length: 0,
            session_id: "sess-live".into(),
            busy_input_mode: BusyInputMode::Queue,
            pending_submission_count: 0,
            running_task_count: None,
            overdue_task_count: None,
            pending_approval_count: None,
            attention_approval_count: None,
            visible_session_count: None,
            scroll_offset: 0,
            status_message: None,
        };
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_status_bar(f, f.area(), &bar, FocusLayer::Composer, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);

        assert!(
            text.contains("LIVE"),
            "tail-follow status bar should show a LIVE indicator: {text:?}"
        );
    }

    #[test]
    fn scrolled_history_status_bar_shows_scrolled() {
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let bar = TestBar {
            model: "gpt-4".into(),
            input_tokens: 0,
            output_tokens: 0,
            context_length: 0,
            session_id: "sess-scroll".into(),
            busy_input_mode: BusyInputMode::Queue,
            pending_submission_count: 0,
            running_task_count: None,
            overdue_task_count: None,
            pending_approval_count: None,
            attention_approval_count: None,
            visible_session_count: None,
            scroll_offset: 4,
            status_message: None,
        };
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_status_bar(f, f.area(), &bar, FocusLayer::Composer, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);

        assert!(
            text.contains("SCROLLED"),
            "scrolled history status bar should show a SCROLLED indicator: {text:?}"
        );
    }

    #[test]
    fn composer_focus_status_bar_shows_compose_indicator() {
        let backend = TestBackend::new(90, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let bar = TestBar {
            model: "gpt-4".into(),
            input_tokens: 0,
            output_tokens: 0,
            context_length: 0,
            session_id: "sess-input".into(),
            busy_input_mode: BusyInputMode::Queue,
            pending_submission_count: 0,
            running_task_count: None,
            overdue_task_count: None,
            pending_approval_count: None,
            attention_approval_count: None,
            visible_session_count: None,
            scroll_offset: 0,
            status_message: None,
        };
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_status_bar(f, f.area(), &bar, FocusLayer::Composer, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);

        assert!(
            text.contains("COMPOSE"),
            "composer focus should keep the COMPOSE indicator visible: {text:?}"
        );
    }

    #[test]
    fn transcript_focus_status_bar_shows_review_indicator() {
        let backend = TestBackend::new(90, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let bar = TestBar {
            model: "gpt-4".into(),
            input_tokens: 0,
            output_tokens: 0,
            context_length: 0,
            session_id: "sess-output".into(),
            busy_input_mode: BusyInputMode::Queue,
            pending_submission_count: 0,
            running_task_count: None,
            overdue_task_count: None,
            pending_approval_count: None,
            attention_approval_count: None,
            visible_session_count: None,
            scroll_offset: 2,
            status_message: None,
        };
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_status_bar(f, f.area(), &bar, FocusLayer::Transcript, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);

        assert!(
            text.contains("REVIEW"),
            "transcript focus should expose the REVIEW indicator: {text:?}"
        );
    }

    #[test]
    fn transcript_focus_status_bar_shows_selection_count() {
        let backend = TestBackend::new(90, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");

        struct SelectionBar {
            inner: TestBar,
            selection_count: usize,
        }

        impl StatusBarView for SelectionBar {
            fn model(&self) -> &str {
                self.inner.model()
            }
            fn input_tokens(&self) -> u32 {
                self.inner.input_tokens()
            }
            fn output_tokens(&self) -> u32 {
                self.inner.output_tokens()
            }
            fn context_length(&self) -> u32 {
                self.inner.context_length()
            }
            fn session_id(&self) -> &str {
                self.inner.session_id()
            }
            fn busy_input_mode(&self) -> BusyInputMode {
                self.inner.busy_input_mode()
            }
            fn pending_submission_count(&self) -> usize {
                self.inner.pending_submission_count()
            }
            fn running_task_count(&self) -> Option<usize> {
                self.inner.running_task_count()
            }
            fn overdue_task_count(&self) -> Option<usize> {
                self.inner.overdue_task_count()
            }
            fn pending_approval_count(&self) -> Option<usize> {
                self.inner.pending_approval_count()
            }
            fn attention_approval_count(&self) -> Option<usize> {
                self.inner.attention_approval_count()
            }
            fn visible_session_count(&self) -> Option<usize> {
                self.inner.visible_session_count()
            }
            fn scroll_offset(&self) -> u16 {
                self.inner.scroll_offset()
            }
            fn transcript_selection_line_count(&self) -> usize {
                self.selection_count
            }
            fn status_message(&self) -> Option<(&str, &Instant)> {
                self.inner.status_message()
            }
        }

        let bar = SelectionBar {
            inner: TestBar {
                model: "gpt-4".into(),
                input_tokens: 0,
                output_tokens: 0,
                context_length: 0,
                session_id: "sess-select".into(),
                busy_input_mode: BusyInputMode::Queue,
                pending_submission_count: 0,
                running_task_count: None,
                overdue_task_count: None,
                pending_approval_count: None,
                attention_approval_count: None,
                visible_session_count: None,
                scroll_offset: 2,
                status_message: None,
            },
            selection_count: 4,
        };
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_status_bar(f, f.area(), &bar, FocusLayer::Transcript, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);

        assert!(
            text.contains("SEL 4"),
            "status bar should show selection count: {text:?}"
        );
    }
}
