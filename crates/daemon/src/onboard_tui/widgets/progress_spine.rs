// ProgressSpineWidget: vertical step indicator for the onboard wizard.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::onboard_flow::OnboardFlowController;
use crate::onboard_state::OnboardWizardStep;
use crate::onboard_tui::theme::OnboardPalette;

#[allow(dead_code)] // consumed by runner/layout in later tasks
pub(crate) struct ProgressSpineWidget {
    current_step: OnboardWizardStep,
}

#[allow(dead_code)] // consumed by runner/layout in later tasks
impl ProgressSpineWidget {
    pub fn new(current_step: OnboardWizardStep) -> Self {
        Self { current_step }
    }

    fn step_entries() -> &'static [(OnboardWizardStep, &'static str)] {
        &[
            (OnboardWizardStep::Welcome, "Hero"),
            (OnboardWizardStep::Authentication, "Access"),
            (OnboardWizardStep::RuntimeDefaults, "Behavior"),
            (OnboardWizardStep::Workspace, "Workspace"),
            (OnboardWizardStep::Protocols, "Protocols"),
            (OnboardWizardStep::EnvironmentCheck, "Verify"),
            (OnboardWizardStep::ReviewAndWrite, "Write"),
            (OnboardWizardStep::Ready, "Launch"),
        ]
    }

    fn step_state(&self, step: OnboardWizardStep) -> StepState {
        let steps = OnboardFlowController::ordered_steps();
        let current_idx = steps
            .iter()
            .position(|s| *s == self.current_step)
            .unwrap_or(0);
        let step_idx = steps.iter().position(|s| *s == step).unwrap_or(0);
        if step_idx < current_idx {
            StepState::Done
        } else if step_idx == current_idx {
            StepState::Active
        } else {
            StepState::Pending
        }
    }

    fn render_compact(self, area: Rect, buf: &mut Buffer) {
        let palette = OnboardPalette::current();
        for (i, (step, label)) in Self::step_entries().iter().enumerate() {
            if i as u16 >= area.height {
                break;
            }
            let y = area.y + i as u16;
            let (icon, style) = match self.step_state(*step) {
                StepState::Done => ("\u{2713}", Style::default().fg(palette.success)),
                StepState::Active => (
                    "\u{25cf}",
                    Style::default()
                        .fg(palette.brand)
                        .add_modifier(Modifier::BOLD),
                ),
                StepState::Pending => ("\u{25cb}", Style::default().fg(palette.muted_text)),
            };
            let line = Line::from(vec![
                Span::styled(format!(" {icon} "), style),
                Span::styled(
                    format!("{:02} ", i + 1),
                    Style::default().fg(palette.muted_text),
                ),
                Span::styled(*label, style),
            ]);
            buf.set_line(area.x, y, &line, area.width);
        }
    }

    fn render_expanded(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }
        let palette = OnboardPalette::current();

        buf.set_line(
            area.x,
            area.y,
            &Line::from(Span::styled(
                " Journey",
                Style::default()
                    .fg(palette.brand)
                    .add_modifier(Modifier::BOLD),
            )),
            area.width,
        );

        let total_steps = Self::step_entries().len();
        for (i, (step, label)) in Self::step_entries().iter().enumerate() {
            let base_y = area.y + 1 + (i as u16) * 2;
            if base_y >= area.bottom() {
                break;
            }

            let state = self.step_state(*step);
            let block_height = 2.min(area.bottom().saturating_sub(base_y));
            let block_rect = Rect::new(area.x, base_y, area.width, block_height);
            let (icon, chip, chip_style, rail_style, label_style, fill_style) = match state {
                StepState::Done => (
                    "\u{2713}",
                    "DONE",
                    Style::default()
                        .fg(palette.success)
                        .add_modifier(Modifier::BOLD),
                    Style::default().fg(palette.success),
                    Style::default().fg(palette.success),
                    Style::default(),
                ),
                StepState::Active => (
                    "\u{25c6}",
                    "LIVE",
                    Style::default()
                        .fg(palette.brand)
                        .bg(palette.surface_emphasis)
                        .add_modifier(Modifier::BOLD),
                    Style::default()
                        .fg(palette.brand)
                        .bg(palette.surface_emphasis)
                        .add_modifier(Modifier::BOLD),
                    Style::default()
                        .fg(palette.text)
                        .bg(palette.surface_emphasis)
                        .add_modifier(Modifier::BOLD),
                    Style::default().bg(palette.surface_emphasis),
                ),
                StepState::Pending => (
                    "\u{25cb}",
                    "NEXT",
                    Style::default().fg(palette.muted_text),
                    Style::default().fg(palette.muted_text),
                    Style::default().fg(palette.secondary_text),
                    Style::default(),
                ),
            };
            if state == StepState::Active {
                buf.set_style(block_rect, fill_style);
            }

            let top_line = Line::from(vec![
                Span::styled(
                    format!(" {:02} ", i + 1),
                    Style::default().fg(palette.muted_text).patch(fill_style),
                ),
                Span::styled(format!("{icon} "), chip_style),
                Span::styled(chip, chip_style),
            ]);
            buf.set_line(area.x, base_y, &top_line, area.width);

            if base_y + 1 < area.bottom() {
                let connector = if i + 1 == total_steps {
                    "    "
                } else {
                    " │  "
                };
                let label_line = Line::from(vec![
                    Span::styled(connector, rail_style),
                    Span::styled(*label, label_style),
                ]);
                buf.set_line(area.x, base_y + 1, &label_line, area.width);
            }
        }
    }
}

#[allow(dead_code)] // internal to ProgressSpineWidget rendering
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StepState {
    Done,
    Active,
    Pending,
}

impl Widget for ProgressSpineWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let needs_compact = area.width < 16
            || area.height < (Self::step_entries().len() as u16).saturating_mul(2) + 1;
        if needs_compact {
            self.render_compact(area, buf);
        } else {
            self.render_expanded(area, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedEnv;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::widgets::Widget;

    #[test]
    fn spine_renders_current_step_highlighted() {
        let widget = ProgressSpineWidget::new(OnboardWizardStep::Authentication);
        let area = Rect::new(0, 0, 20, 20);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let content = buffer_text(&buf);
        assert!(
            content.contains("Journey"),
            "expanded spine should render a title"
        );
        assert!(
            content.contains("LIVE"),
            "expanded spine should show the active chip"
        );
        assert!(content.contains("Hero"), "should show Welcome as completed");
        assert!(content.contains("Access"), "should show current step");
    }

    #[test]
    fn spine_marks_earlier_steps_as_done() {
        let widget = ProgressSpineWidget::new(OnboardWizardStep::Workspace);
        let area = Rect::new(0, 0, 20, 20);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let content = buffer_text(&buf);
        assert!(content.contains("DONE"));
        assert!(content.contains("LIVE"));
        assert!(content.contains("NEXT"));
    }

    #[test]
    fn spine_falls_back_to_compact_mode_when_space_is_tight() {
        let widget = ProgressSpineWidget::new(OnboardWizardStep::Workspace);
        let area = Rect::new(0, 0, 18, 8);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        let content = buffer_text(&buf);
        assert!(
            content.contains("01"),
            "compact spine should still show step numbers"
        );
        assert!(
            !content.contains("Journey"),
            "compact spine should skip the title"
        );
    }

    #[test]
    fn spine_title_tracks_light_palette() {
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let widget = ProgressSpineWidget::new(OnboardWizardStep::Welcome);
        let area = Rect::new(0, 0, 20, 20);
        let mut buf = Buffer::empty(area);
        widget.render(area, &mut buf);

        assert_eq!(buf[(1, 0)].fg, OnboardPalette::light().brand);
    }

    fn buffer_text(buf: &Buffer) -> String {
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
            }
            text.push('\n');
        }
        text
    }
}
