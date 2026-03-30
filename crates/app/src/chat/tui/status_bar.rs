use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

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
}

pub(super) fn render_status_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    pane: &impl StatusBarView,
    palette: &Palette,
) {
    let model_display = truncate_str(pane.model(), 24, "no model");
    let session_display = truncate_str(pane.session_id(), 16, "no session");

    let total = pane.input_tokens().saturating_add(pane.output_tokens());
    let ctx = pane.context_length();
    let pct = if ctx == 0 {
        0.0f32
    } else {
        (total as f32 / ctx as f32) * 100.0
    };

    let pct_style = context_percent_style(pct, palette);

    let line = Line::from(vec![
        Span::styled(
            format!(" {model_display}"),
            Style::default().fg(palette.dim),
        ),
        Span::styled(" | ".to_string(), Style::default().fg(palette.separator)),
        Span::styled(format!("{total}"), Style::default().fg(palette.info)),
        Span::styled(" tokens".to_string(), Style::default().fg(palette.dim)),
        Span::styled(format!(" ({pct:.0}%)"), pct_style),
        Span::styled(" | ".to_string(), Style::default().fg(palette.separator)),
        Span::styled(session_display, Style::default().fg(palette.dim)),
    ]);

    frame.render_widget(Paragraph::new(line), area);
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
        };
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_status_bar(f, f.area(), &bar, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(text.contains("claude-3.5-sonnet"));
        assert!(text.contains("1234")); // 1000 + 234
        assert!(text.contains("12%")); // 1234/10000 ~= 12%
        assert!(text.contains("sess-abc123"));
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
        };
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_status_bar(f, f.area(), &bar, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(text.contains("gpt-4"));
        assert!(text.contains("(0%)")); // 0 context means 0%
    }
}
