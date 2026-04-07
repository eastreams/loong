use std::time::Instant;

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::theme::Palette;

pub(super) const FRAMES: &[&str] = &["\u{25d0}", "\u{25d3}", "\u{25d1}", "\u{25d2}"];
pub(super) const DOTS: &[&str] = &["   ", ".  ", ".. ", "..."];

/// Status message fade-out threshold in seconds.
const STATUS_FADE_SECS: u64 = 10;

// ---------------------------------------------------------------------------
// View trait — decouples rendering from the concrete `Pane` struct
// ---------------------------------------------------------------------------

pub(super) trait SpinnerView {
    fn agent_running(&self) -> bool;
    fn spinner_frame(&self) -> usize;
    fn dots_frame(&self) -> usize;
    fn loop_state(&self) -> &str;
    fn loop_action(&self) -> &str {
        ""
    }
    fn loop_iteration(&self) -> u32;
    fn running_tool_call_count(&self) -> usize {
        0
    }
    fn tool_call_count(&self) -> usize {
        0
    }
    fn worktree_dirty(&self) -> bool {
        false
    }
    fn dirty_file_count(&self) -> usize {
        0
    }
    fn dirty_file_preview(&self) -> &[String] {
        &[]
    }
    fn active_subagent_count(&self) -> Option<usize> {
        None
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
    fn status_message(&self) -> Option<(&str, &Instant)>;
}

pub(super) fn render_spinner(
    frame: &mut Frame<'_>,
    area: Rect,
    pane: &impl SpinnerView,
    palette: &Palette,
) {
    let content = if pane.agent_running() {
        let idx = pane.spinner_frame() % FRAMES.len();
        let spinner = FRAMES.get(idx).copied().unwrap_or("");
        let didx = pane.dots_frame() % DOTS.len();
        let dots = DOTS.get(didx).copied().unwrap_or("");
        let working_label = "Working";
        let round_label = running_round_label(pane.loop_iteration());
        let state_label = optional_running_segment(pane.loop_state());
        let action_label = optional_running_segment(pane.loop_action());

        Line::from(vec![
            Span::styled(
                format!(" {spinner} "),
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(working_label.to_owned(), Style::default().fg(palette.text)),
            Span::styled(round_label, Style::default().fg(palette.dim)),
            Span::styled(state_label, Style::default().fg(palette.info)),
            Span::styled(action_label, Style::default().fg(palette.dim)),
            Span::styled(format!(" {dots}"), Style::default().fg(palette.brand)),
        ])
    } else if let Some((msg, when)) = pane.status_message() {
        if when.elapsed().as_secs() < STATUS_FADE_SECS {
            idle_line(Some(msg), pane, area.width, palette)
        } else {
            idle_line(None, pane, area.width, palette)
        }
    } else {
        idle_line(None, pane, area.width, palette)
    };

    frame.render_widget(Paragraph::new(content), area);
}

fn running_round_label(loop_iteration: u32) -> String {
    if loop_iteration <= 1 {
        return String::new();
    }

    format!(" · round {loop_iteration}")
}

fn optional_running_segment(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    format!(" · {trimmed}")
}

fn idle_line(
    status_message: Option<&str>,
    pane: &impl SpinnerView,
    width: u16,
    palette: &Palette,
) -> Line<'static> {
    let mut spans = Vec::new();
    spans.push(Span::styled(
        " Ready".to_string(),
        Style::default()
            .fg(palette.dim)
            .add_modifier(Modifier::ITALIC),
    ));

    if let Some(msg) = status_message {
        spans.push(Span::styled(
            " · ".to_string(),
            Style::default().fg(palette.separator),
        ));
        spans.push(Span::styled(
            msg.to_owned(),
            Style::default()
                .fg(palette.dim)
                .add_modifier(Modifier::ITALIC),
        ));
    }

    let dock_spans = operation_dock_spans(pane, width, palette);
    if !dock_spans.is_empty() {
        spans.push(Span::styled(
            " · ".to_string(),
            Style::default().fg(palette.separator),
        ));
        spans.extend(dock_spans);
    }

    Line::from(spans)
}

fn operation_dock_spans(
    pane: &impl SpinnerView,
    width: u16,
    palette: &Palette,
) -> Vec<Span<'static>> {
    let extra_wide = width >= 160;
    let wide = width >= 120;
    let pulse = activity_pulse(pane.dots_frame());
    let mut items = Vec::new();

    let attention_approvals = pane.attention_approval_count().unwrap_or(0);
    if attention_approvals > 0 {
        items.push(operation_item(
            pulse,
            if extra_wide {
                format!(" approvals! {attention_approvals} · /approvals attention")
            } else if wide {
                format!(" apr! {attention_approvals} /approvals")
            } else {
                format!(" apr! {attention_approvals}")
            },
            palette.warning,
            palette,
        ));
    }

    let pending_approvals = pane.pending_approval_count().unwrap_or(0);
    let passive_approvals = pending_approvals.saturating_sub(attention_approvals);
    if passive_approvals > 0 {
        items.push(operation_item(
            pulse,
            if extra_wide {
                format!(" approvals {passive_approvals} · /approvals")
            } else if wide {
                format!(" apr {passive_approvals} /approvals")
            } else {
                format!(" apr {passive_approvals}")
            },
            palette.info,
            palette,
        ));
    }

    let overdue_tasks = pane.overdue_task_count().unwrap_or(0);
    if overdue_tasks > 0 {
        items.push(operation_item(
            pulse,
            if extra_wide {
                format!(" overdue {overdue_tasks} · /tasks running")
            } else if wide {
                format!(" late {overdue_tasks} /tasks")
            } else {
                format!(" late {overdue_tasks}")
            },
            palette.error,
            palette,
        ));
    }

    let running_tasks = pane.running_task_count().unwrap_or(0);
    if running_tasks > 0 {
        items.push(operation_item(
            pulse,
            if extra_wide {
                format!(" background {running_tasks} · /tasks running")
            } else if wide {
                format!(" bg {running_tasks} /tasks")
            } else {
                format!(" bg {running_tasks}")
            },
            palette.tool_running,
            palette,
        ));
    }

    let active_subagents = pane.active_subagent_count().unwrap_or(0);
    if active_subagents > 0 {
        items.push(operation_item(
            pulse,
            if extra_wide {
                format!(" subagents {active_subagents} · /subagents")
            } else if wide {
                format!(" sub {active_subagents} /subagents")
            } else {
                format!(" sub {active_subagents}")
            },
            palette.brand,
            palette,
        ));
    }

    let running_tools = pane.running_tool_call_count();
    let total_tools = pane.tool_call_count();
    if running_tools > 0 || total_tools > 0 {
        let label = if extra_wide {
            if running_tools > 0 {
                format!(" tools {running_tools}/{total_tools} · /tools open")
            } else {
                format!(" tools {total_tools} · /tools")
            }
        } else if wide {
            if running_tools > 0 {
                format!(" tool {running_tools}/{total_tools} /tools")
            } else {
                format!(" tool {total_tools} /tools")
            }
        } else if running_tools > 0 {
            format!(" tool {running_tools}/{total_tools}")
        } else {
            format!(" tool {total_tools}")
        };
        let color = if running_tools > 0 {
            palette.tool_running
        } else {
            palette.info
        };
        items.push(operation_item(pulse, label, color, palette));
    }

    let dirty_file_count = pane.dirty_file_count();
    if pane.worktree_dirty() && dirty_file_count > 0 {
        let dirty_preview = pane.dirty_file_preview();
        let dirty_summary =
            summarize_dirty_files(dirty_preview, dirty_file_count, extra_wide, wide);
        items.push(operation_item('•', dirty_summary, palette.info, palette));
    }

    items.push(passive_operation_item(
        if extra_wide {
            " commands · /commands".to_owned()
        } else if wide {
            " cmds /commands".to_owned()
        } else {
            " cmds".to_owned()
        },
        palette,
    ));

    let mut spans = Vec::new();
    for (index, item) in items.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(
                " · ".to_string(),
                Style::default().fg(palette.separator),
            ));
        }
        spans.extend(item);
    }

    spans
}

fn activity_pulse(frame: usize) -> char {
    match frame % 4 {
        0 => '·',
        1 => '•',
        2 => '●',
        _ => '•',
    }
}

fn operation_item(
    marker: char,
    label: String,
    color: ratatui::style::Color,
    palette: &Palette,
) -> Vec<Span<'static>> {
    vec![
        Span::styled(
            format!("{marker}"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(label, Style::default().fg(palette.dim)),
    ]
}

fn passive_operation_item(label: String, palette: &Palette) -> Vec<Span<'static>> {
    vec![Span::styled(label, Style::default().fg(palette.separator))]
}

fn summarize_dirty_files(
    dirty_preview: &[String],
    dirty_file_count: usize,
    extra_wide: bool,
    wide: bool,
) -> String {
    let first_label = dirty_preview
        .first()
        .map(|path| summarize_dirty_file_name(path.as_str()))
        .unwrap_or_else(|| "edited files".to_owned());
    let remaining_count = dirty_file_count.saturating_sub(1);

    if extra_wide {
        if remaining_count > 0 {
            return format!(" edits {first_label} +{remaining_count}");
        }
        return format!(" edits {first_label}");
    }

    if wide {
        if remaining_count > 0 {
            return format!(" edit {first_label} +{remaining_count}");
        }
        return format!(" edit {first_label}");
    }

    if remaining_count > 0 {
        return format!(" edit +{dirty_file_count}");
    }

    format!(" edit {first_label}")
}

fn summarize_dirty_file_name(path: &str) -> String {
    let file_name = path.rsplit('/').next().unwrap_or(path);

    file_name.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use std::time::Instant;

    struct TestSpinner {
        running: bool,
        spinner_frame: usize,
        dots_frame: usize,
        loop_state: String,
        loop_action: String,
        loop_iteration: u32,
        running_tool_call_count: usize,
        tool_call_count: usize,
        worktree_dirty: bool,
        dirty_files: Vec<String>,
        active_subagent_count: Option<usize>,
        running_task_count: Option<usize>,
        overdue_task_count: Option<usize>,
        pending_approval_count: Option<usize>,
        attention_approval_count: Option<usize>,
        status_message: Option<(String, Instant)>,
    }

    impl TestSpinner {
        fn idle() -> Self {
            Self {
                running: false,
                spinner_frame: 0,
                dots_frame: 0,
                loop_state: String::new(),
                loop_action: String::new(),
                loop_iteration: 0,
                running_tool_call_count: 0,
                tool_call_count: 0,
                worktree_dirty: false,
                dirty_files: Vec::new(),
                active_subagent_count: None,
                running_task_count: None,
                overdue_task_count: None,
                pending_approval_count: None,
                attention_approval_count: None,
                status_message: None,
            }
        }

        fn active() -> Self {
            Self {
                running: true,
                spinner_frame: 1,
                dots_frame: 2,
                loop_state: "requesting provider".into(),
                loop_action: "10 messages | est. 500 tok".into(),
                loop_iteration: 2,
                running_tool_call_count: 0,
                tool_call_count: 0,
                worktree_dirty: false,
                dirty_files: Vec::new(),
                active_subagent_count: None,
                running_task_count: None,
                overdue_task_count: None,
                pending_approval_count: None,
                attention_approval_count: None,
                status_message: None,
            }
        }
    }

    impl SpinnerView for TestSpinner {
        fn agent_running(&self) -> bool {
            self.running
        }
        fn spinner_frame(&self) -> usize {
            self.spinner_frame
        }
        fn dots_frame(&self) -> usize {
            self.dots_frame
        }
        fn loop_state(&self) -> &str {
            &self.loop_state
        }
        fn loop_action(&self) -> &str {
            &self.loop_action
        }
        fn loop_iteration(&self) -> u32 {
            self.loop_iteration
        }
        fn running_tool_call_count(&self) -> usize {
            self.running_tool_call_count
        }
        fn tool_call_count(&self) -> usize {
            self.tool_call_count
        }
        fn worktree_dirty(&self) -> bool {
            self.worktree_dirty
        }
        fn dirty_file_count(&self) -> usize {
            self.dirty_files.len()
        }
        fn dirty_file_preview(&self) -> &[String] {
            self.dirty_files.as_slice()
        }
        fn active_subagent_count(&self) -> Option<usize> {
            self.active_subagent_count
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
    fn idle_renders_ready() {
        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let pane = TestSpinner::idle();
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_spinner(f, f.area(), &pane, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(text.contains("Ready"), "idle state should show Ready");
    }

    #[test]
    fn running_renders_iteration_and_state() {
        let backend = TestBackend::new(90, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let pane = TestSpinner::active();
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_spinner(f, f.area(), &pane, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(text.contains("Working"), "should show running label");
        assert!(text.contains("round 2"), "should show round number");
        assert!(
            text.contains("requesting provider"),
            "should show loop state"
        );
        assert!(
            text.contains("10 messages | est. 500 tok"),
            "should show loop action summary"
        );
    }

    #[test]
    fn running_spinner_surfaces_live_edited_files() {
        let backend = TestBackend::new(120, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let pane = TestSpinner {
            worktree_dirty: true,
            dirty_files: vec![
                "crates/app/src/chat/tui/render.rs".to_owned(),
                "crates/app/src/chat/tui/shell.rs".to_owned(),
            ],
            ..TestSpinner::active()
        };
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_spinner(f, f.area(), &pane, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(
            !text.contains("edit"),
            "spinner should leave live edits to the dedicated context rail: {text:?}"
        );
    }

    #[test]
    fn spinner_frame_cycles() {
        assert_eq!(FRAMES.len(), 4);
        assert_eq!(DOTS.len(), 4);
        // Modular access never panics
        for i in 0..20 {
            let _ = FRAMES.get(i % FRAMES.len());
            let _ = DOTS.get(i % DOTS.len());
        }
    }

    #[test]
    fn status_message_displayed_when_recent() {
        let backend = TestBackend::new(60, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let pane = TestSpinner {
            status_message: Some(("Model switched".into(), Instant::now())),
            ..TestSpinner::idle()
        };
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_spinner(f, f.area(), &pane, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(
            text.contains("Model switched"),
            "recent status message should be visible"
        );
    }

    #[test]
    fn idle_operation_dock_surfaces_active_entry_points() {
        let backend = TestBackend::new(140, 1);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let pane = TestSpinner {
            dots_frame: 1,
            running_tool_call_count: 1,
            tool_call_count: 4,
            worktree_dirty: true,
            dirty_files: vec![
                "crates/app/src/chat/tui/render.rs".to_owned(),
                "crates/app/src/chat/tui/shell.rs".to_owned(),
                "crates/app/src/chat/tui/state.rs".to_owned(),
            ],
            active_subagent_count: Some(2),
            running_task_count: Some(1),
            pending_approval_count: Some(3),
            attention_approval_count: Some(1),
            ..TestSpinner::idle()
        };
        let palette = Palette::dark();

        terminal
            .draw(|f| {
                render_spinner(f, f.area(), &pane, &palette);
            })
            .expect("draw");

        let text = buffer_text(&terminal);
        assert!(text.contains("apr! 1 /approvals"), "text={text:?}");
        assert!(text.contains("bg 1 /tasks"), "text={text:?}");
        assert!(text.contains("/subagents"), "text={text:?}");
        assert!(text.contains("/tools"), "text={text:?}");
        assert!(text.contains("edit render.rs +2"), "text={text:?}");
        assert!(text.contains("cmds"), "text={text:?}");
    }
}
