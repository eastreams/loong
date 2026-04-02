// SelectionCardWidget: selectable card list for the onboard wizard.

use std::collections::BTreeSet;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::StatefulWidget;

use crate::onboard_tui::theme::OnboardPalette;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SelectionItem {
    pub label: String,
    pub hint: Option<String>,
}

impl SelectionItem {
    pub fn new(label: impl Into<String>, hint: Option<impl Into<String>>) -> Self {
        let label = label.into();
        let hint = hint.map(Into::into);

        Self { label, hint }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SelectionCardTheme {
    pub frame_color: Color,
    pub accent_color: Color,
    pub active_bg: Color,
    pub inactive_label_color: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum SelectionCardLayout {
    Framed,
    PosterList,
}

impl Default for SelectionCardTheme {
    fn default() -> Self {
        let palette = OnboardPalette::current();

        Self {
            frame_color: palette.brand,
            accent_color: palette.brand,
            active_bg: palette.surface_emphasis,
            inactive_label_color: palette.secondary_text,
        }
    }
}

impl SelectionCardTheme {
    pub fn new(frame_color: Color, accent_color: Color, active_bg: Color) -> Self {
        Self {
            frame_color,
            accent_color,
            active_bg,
            ..Self::default()
        }
    }
}

pub(crate) struct SelectionCardState {
    selected: usize,
    count: usize,
    checked: BTreeSet<usize>,
}

impl SelectionCardState {
    pub fn new(count: usize) -> Self {
        let selected = 0;
        let checked = BTreeSet::new();

        Self {
            selected,
            count,
            checked,
        }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn select(&mut self, index: usize) {
        let max_index = self.count.saturating_sub(1);
        let next_index = index.min(max_index);

        self.selected = next_index;
    }

    pub fn select_first(&mut self) {
        self.select(0);
    }

    pub fn select_last(&mut self) {
        if self.count == 0 {
            return;
        }

        let last_index = self.count - 1;
        self.selected = last_index;
    }

    pub fn next(&mut self) {
        if self.count == 0 {
            return;
        }

        let next_index = (self.selected + 1) % self.count;
        self.selected = next_index;
    }

    pub fn previous(&mut self) {
        if self.count == 0 {
            return;
        }

        let previous_index = if self.selected == 0 {
            self.count - 1
        } else {
            self.selected - 1
        };
        self.selected = previous_index;
    }

    pub fn toggle_selected(&mut self) {
        if self.count == 0 {
            return;
        }

        let selected_index = self.selected;
        let already_checked = self.checked.contains(&selected_index);
        if already_checked {
            self.checked.remove(&selected_index);
            return;
        }

        self.checked.insert(selected_index);
    }

    pub fn is_checked(&self, index: usize) -> bool {
        self.checked.contains(&index)
    }

    pub fn checked_indices(&self) -> Vec<usize> {
        self.checked.iter().copied().collect()
    }

    pub fn set_checked_indices<I>(&mut self, indices: I)
    where
        I: IntoIterator<Item = usize>,
    {
        let mut checked = BTreeSet::new();

        for index in indices {
            let is_in_bounds = index < self.count;
            if !is_in_bounds {
                continue;
            }

            checked.insert(index);
        }

        self.checked = checked;
    }
}

pub(crate) struct SelectionCardWidget {
    items: Vec<SelectionItem>,
    theme: SelectionCardTheme,
    layout: SelectionCardLayout,
}

impl SelectionCardWidget {
    pub fn new(items: Vec<SelectionItem>) -> Self {
        let theme = SelectionCardTheme::default();
        let layout = SelectionCardLayout::Framed;

        Self {
            items,
            theme,
            layout,
        }
    }

    pub fn with_theme(mut self, theme: SelectionCardTheme) -> Self {
        self.theme = theme;
        self
    }

    pub fn with_layout(mut self, layout: SelectionCardLayout) -> Self {
        self.layout = layout;
        self
    }

    fn visible_range(count: usize, selected: usize, visible_count: usize) -> (usize, usize) {
        if count == 0 {
            return (0, 0);
        }

        let clamped_visible_count = visible_count.max(1).min(count);
        let center_offset = clamped_visible_count / 2;
        let max_start = count.saturating_sub(clamped_visible_count);
        let centered_start = selected.saturating_sub(center_offset);
        let start = centered_start.min(max_start);
        let end = start.saturating_add(clamped_visible_count);

        (start, end)
    }

    fn ellipsize_copy(value: &str, max_width: usize) -> String {
        if max_width == 0 {
            return String::new();
        }

        let value_width = value.chars().count();
        if value_width <= max_width {
            return value.to_owned();
        }

        if max_width == 1 {
            return "…".to_owned();
        }

        let keep_width = max_width.saturating_sub(1);
        let kept = value.chars().take(keep_width).collect::<String>();
        format!("{kept}…")
    }

    fn item_number_label(item_index: usize) -> String {
        let display_number = item_index + 1;
        format!("[{display_number}]")
    }

    fn visible_origin_y(
        area: Rect,
        content_height: u16,
        selected_anchor_offset: u16,
        pin_to_top: bool,
        pin_to_bottom: bool,
    ) -> u16 {
        let content_fits_without_scrolling = content_height <= area.height;
        if content_fits_without_scrolling {
            return area.y;
        }

        if pin_to_top {
            return area.y;
        }

        let min_origin = area.y;
        let max_origin = area.bottom().saturating_sub(content_height);
        if pin_to_bottom {
            return max_origin;
        }

        let center_y = area.y.saturating_add(area.height / 2);
        let ideal_origin = i32::from(center_y) - i32::from(selected_anchor_offset);
        let min_origin_i32 = i32::from(min_origin);
        let max_origin_i32 = i32::from(max_origin);
        let clamped_origin = ideal_origin.clamp(min_origin_i32, max_origin_i32);

        u16::try_from(clamped_origin).ok().unwrap_or(min_origin)
    }

    fn render_framed(&self, area: Rect, buf: &mut Buffer, state: &SelectionCardState) {
        let palette = OnboardPalette::current();
        let lines_per_item = 3_u16;
        let visible_items = usize::from((area.height / lines_per_item).max(1));
        let (start, end) = Self::visible_range(self.items.len(), state.selected(), visible_items);
        let theme = self.theme;
        let visible_slice = self.items.get(start..end);
        let Some(visible_slice) = visible_slice else {
            return;
        };
        let visible_count = visible_slice.len();
        let visible_count_u16 = u16::try_from(visible_count).ok().unwrap_or(0);
        let content_height = visible_count_u16.saturating_mul(lines_per_item);
        let selected_visible_index = state.selected().saturating_sub(start);
        let selected_visible_index_u16 = u16::try_from(selected_visible_index).ok().unwrap_or(0);
        let selected_anchor_offset = selected_visible_index_u16
            .saturating_mul(lines_per_item)
            .saturating_add(1);
        let pin_to_top = start == 0 && selected_visible_index == 0;
        let pin_to_bottom = end == self.items.len() && selected_visible_index + 1 == visible_count;
        let origin_y = Self::visible_origin_y(
            area,
            content_height,
            selected_anchor_offset,
            pin_to_top,
            pin_to_bottom,
        );

        for (visible_index, item) in visible_slice.iter().enumerate() {
            let item_index = start + visible_index;
            let visible_index_u16 = u16::try_from(visible_index).ok().unwrap_or(0);
            let item_offset = visible_index_u16.saturating_mul(lines_per_item);
            let y = origin_y.saturating_add(item_offset);
            let area_bottom = area.bottom();
            let is_below_area = y >= area_bottom;
            if is_below_area {
                break;
            }

            let is_selected = item_index == state.selected();
            let is_checked = state.is_checked(item_index);
            let card_height = lines_per_item.min(area_bottom.saturating_sub(y));
            let card_rect = Rect::new(area.x, y, area.width, card_height);
            let card_width = usize::from(card_rect.width);
            let inner_width = card_width.saturating_sub(2);
            if inner_width == 0 {
                continue;
            }
            let fill_style = if is_selected {
                Style::default().bg(theme.active_bg)
            } else {
                Style::default()
            };
            if is_selected {
                buf.set_style(card_rect, fill_style);
            }

            let frame_style = if is_selected {
                Style::default()
                    .fg(theme.frame_color)
                    .bg(theme.active_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.border)
            };
            let label_style = if is_selected {
                Style::default()
                    .fg(palette.text)
                    .bg(theme.active_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.inactive_label_color)
            };
            let prefix_style = if is_selected {
                Style::default()
                    .fg(theme.accent_color)
                    .bg(theme.active_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.muted_text)
            };
            let hint_style = if is_selected {
                Style::default()
                    .fg(palette.secondary_text)
                    .bg(theme.active_bg)
            } else {
                Style::default().fg(palette.muted_text)
            };
            let top_left = if is_selected { '╭' } else { '┌' };
            let top_right = if is_selected { '╮' } else { '┐' };
            let bottom_left = if is_selected { '╰' } else { '└' };
            let bottom_right = if is_selected { '╯' } else { '┘' };
            let top_inner = "─".repeat(inner_width);
            let top_line = Line::from(vec![
                Span::styled(top_left.to_string(), frame_style),
                Span::styled(top_inner, frame_style),
                Span::styled(top_right.to_string(), frame_style),
            ]);
            buf.set_line(area.x, y, &top_line, card_rect.width);

            if y + 1 < area_bottom {
                let check_label = if is_checked { "[x]" } else { "[ ]" };
                let hotkey_label = Self::item_number_label(item_index);
                let middle_prefix = format!(" {hotkey_label} {check_label} ");
                let middle_prefix_width = middle_prefix.chars().count();
                let label_width = inner_width.saturating_sub(middle_prefix_width);
                let label_copy = Self::ellipsize_copy(item.label.as_str(), label_width);
                let label_copy_width = label_copy.chars().count();
                let middle_padding_width = inner_width
                    .saturating_sub(middle_prefix_width)
                    .saturating_sub(label_copy_width);
                let middle_padding = " ".repeat(middle_padding_width);
                let label_line = Line::from(vec![
                    Span::styled("│", frame_style),
                    Span::styled(middle_prefix, prefix_style),
                    Span::styled(label_copy, label_style),
                    Span::styled(middle_padding, label_style),
                    Span::styled("│", frame_style),
                ]);
                buf.set_line(area.x, y + 1, &label_line, card_rect.width);
            }

            if y + 2 < area_bottom {
                let fallback_hint = if is_checked {
                    "Press Space to clear this selection."
                } else {
                    "Press Space to add this selection."
                };
                let hint_copy = item.hint.as_deref().unwrap_or(fallback_hint).to_owned();
                let bottom_prefix = "─ ";
                let bottom_prefix_width = bottom_prefix.chars().count();
                let hint_width = inner_width.saturating_sub(bottom_prefix_width);
                let hint_copy = Self::ellipsize_copy(hint_copy.as_str(), hint_width);
                let hint_copy_width = hint_copy.chars().count();
                let bottom_fill_width = inner_width
                    .saturating_sub(bottom_prefix_width)
                    .saturating_sub(hint_copy_width);
                let bottom_fill = "─".repeat(bottom_fill_width);
                let hint_line = Line::from(vec![
                    Span::styled(bottom_left.to_string(), frame_style),
                    Span::styled(bottom_prefix, frame_style),
                    Span::styled(hint_copy, hint_style),
                    Span::styled(bottom_fill, frame_style),
                    Span::styled(bottom_right.to_string(), frame_style),
                ]);
                buf.set_line(area.x, y + 2, &hint_line, card_rect.width);
            }
        }
    }

    fn render_poster_list(&self, area: Rect, buf: &mut Buffer, state: &SelectionCardState) {
        let palette = OnboardPalette::current();
        let theme = self.theme;
        let visible_items = usize::from(area.height.max(1));
        let (start, end) = Self::visible_range(self.items.len(), state.selected(), visible_items);
        let visible_slice = self.items.get(start..end);
        let Some(visible_slice) = visible_slice else {
            return;
        };
        let visible_count = visible_slice.len();
        let content_height = u16::try_from(visible_count).ok().unwrap_or(0);
        let selected_visible_index = state.selected().saturating_sub(start);
        let selected_anchor_offset = u16::try_from(selected_visible_index).ok().unwrap_or(0);
        let pin_to_top = start == 0 && selected_visible_index == 0;
        let pin_to_bottom = end == self.items.len() && selected_visible_index + 1 == visible_count;
        let origin_y = Self::visible_origin_y(
            area,
            content_height,
            selected_anchor_offset,
            pin_to_top,
            pin_to_bottom,
        );

        for (visible_index, item) in visible_slice.iter().enumerate() {
            let item_index = start + visible_index;
            let visible_index_u16 = u16::try_from(visible_index).ok().unwrap_or(0);
            let y = origin_y.saturating_add(visible_index_u16);
            let is_below_area = y >= area.bottom();
            if is_below_area {
                break;
            }

            let is_selected = item_index == state.selected();
            let is_checked = state.is_checked(item_index);
            let cursor = if is_selected { "•" } else { "·" };
            let check = if is_checked { "[x]" } else { "[ ]" };
            let hotkey = Self::item_number_label(item_index);
            let cursor_style = if is_selected {
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.muted_text)
            };
            let check_style = if is_checked {
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.secondary_text)
            };
            let hotkey_style = if is_selected {
                Style::default()
                    .fg(theme.accent_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.secondary_text)
            };
            let label_style = if is_selected {
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.inactive_label_color)
            };
            let line = Line::from(vec![
                Span::styled(cursor, cursor_style),
                Span::raw(" "),
                Span::styled(check, check_style),
                Span::raw(" "),
                Span::styled(&hotkey, hotkey_style),
                Span::raw(" "),
                Span::styled(&item.label, label_style),
            ]);
            let cursor_width = 1_usize;
            let check_width = check.len();
            let hotkey_width = hotkey.len();
            let label_width = item.label.chars().count();
            let content_width = cursor_width + 1 + check_width + 1 + hotkey_width + 1 + label_width;
            let content_width = u16::try_from(content_width).ok().unwrap_or(0);
            let line_x = area.x + area.width.saturating_sub(content_width) / 2;

            buf.set_line(line_x, y, &line, area.right().saturating_sub(line_x));
        }
    }
}

impl StatefulWidget for SelectionCardWidget {
    type State = SelectionCardState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        match self.layout {
            SelectionCardLayout::Framed => self.render_framed(area, buf, state),
            SelectionCardLayout::PosterList => self.render_poster_list(area, buf, state),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScopedEnv;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::widgets::StatefulWidget;

    #[test]
    fn card_highlights_selected_item() {
        let items = vec![
            SelectionItem::new("OpenAI", Some("credentials found")),
            SelectionItem::new("Anthropic", None::<&str>),
            SelectionItem::new("DeepSeek", None::<&str>),
        ];
        let widget = SelectionCardWidget::new(items);
        let mut state = SelectionCardState::new(3);

        state.select(0);

        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &mut state);

        let content = buffer_text(&buf);
        assert!(content.contains("OpenAI"));
        assert!(content.contains("credentials found"));
        assert!(content.contains("[1]"));
        assert!(content.contains("╭"));
        assert!(content.contains("╮"));
        assert!(content.contains("╯"));
        assert!(!content.contains("ACTIVE"));
        assert!(!content.contains("READY"));
    }

    #[test]
    fn framed_layout_keeps_selected_item_visible_when_list_exceeds_height() {
        let items = vec![
            SelectionItem::new("OpenAI", None::<&str>),
            SelectionItem::new("Anthropic", None::<&str>),
            SelectionItem::new("DeepSeek", None::<&str>),
            SelectionItem::new("Gemini", None::<&str>),
            SelectionItem::new("OpenRouter", None::<&str>),
            SelectionItem::new("Ollama", None::<&str>),
        ];
        let widget = SelectionCardWidget::new(items);
        let mut state = SelectionCardState::new(6);

        state.select(5);

        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &mut state);

        let content = buffer_text(&buf);
        assert!(content.contains("Ollama"));
        assert!(!content.contains("OpenAI"));
    }

    #[test]
    fn card_state_wraps_on_navigation() {
        let mut state = SelectionCardState::new(3);

        state.select(2);
        state.next();

        assert_eq!(state.selected(), 0);
    }

    #[test]
    fn card_state_wraps_backward() {
        let mut state = SelectionCardState::new(3);

        state.select(0);
        state.previous();

        assert_eq!(state.selected(), 2);
    }

    #[test]
    fn themed_card_uses_custom_colors_for_selected_state() {
        let items = vec![SelectionItem::new(
            "Current",
            Some("use existing machine state"),
        )];
        let widget = SelectionCardWidget::new(items).with_theme(SelectionCardTheme::new(
            Color::Green,
            Color::Yellow,
            Color::Rgb(32, 24, 8),
        ));
        let mut state = SelectionCardState::new(1);

        state.select(0);

        let area = Rect::new(0, 0, 40, 4);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &mut state);

        assert_eq!(buf[(0, 0)].fg, Color::Green);
        assert_eq!(buf[(1, 1)].fg, Color::Yellow);
        assert_eq!(buf[(1, 1)].bg, Color::Rgb(32, 24, 8));
    }

    #[test]
    fn default_theme_tracks_light_palette_tokens() {
        let mut env = ScopedEnv::new();

        env.set("LOONGCLAW_ONBOARD_THEME", "light");
        env.remove("NO_COLOR");
        env.remove("COLORFGBG");

        let theme = SelectionCardTheme::default();
        let palette = OnboardPalette::light();

        assert_eq!(theme.frame_color, palette.brand);
        assert_eq!(theme.accent_color, palette.brand);
        assert_eq!(theme.active_bg, palette.surface_emphasis);
        assert_eq!(theme.inactive_label_color, palette.secondary_text);
    }

    #[test]
    fn poster_list_layout_drops_card_frames_and_state_badges() {
        let items = vec![
            SelectionItem::new("Use current setup", Some("keep the detected draft")),
            SelectionItem::new("Start fresh", Some("open the full setup flow")),
        ];
        let widget = SelectionCardWidget::new(items).with_layout(SelectionCardLayout::PosterList);
        let mut state = SelectionCardState::new(2);

        state.select(0);

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &mut state);

        let content = buffer_text(&buf);
        assert!(content.contains("Use current setup"));
        assert!(content.contains("[1]"));
        assert!(content.contains("[2]"));
        assert!(!content.contains("ACTIVE"));
        assert!(!content.contains("READY"));
        assert!(!content.contains("╭"));
        assert!(!content.contains("┌"));
        assert!(!content.contains("keep the detected draft"));
    }

    #[test]
    fn toggle_selected_marks_and_clears_the_focused_item() {
        let mut state = SelectionCardState::new(3);

        state.select(1);
        state.toggle_selected();
        assert_eq!(state.checked_indices(), vec![1]);

        state.toggle_selected();
        assert!(state.checked_indices().is_empty());
    }

    #[test]
    fn checked_indices_follow_sorted_selection_order() {
        let mut state = SelectionCardState::new(4);

        state.select(3);
        state.toggle_selected();
        state.select(1);
        state.toggle_selected();
        state.select(2);
        state.toggle_selected();

        assert_eq!(state.checked_indices(), vec![1, 2, 3]);
    }

    #[test]
    fn set_checked_indices_discards_out_of_bounds_entries() {
        let mut state = SelectionCardState::new(3);

        state.set_checked_indices([0, 2, 99]);

        assert_eq!(state.checked_indices(), vec![0, 2]);
    }

    #[test]
    fn poster_list_layout_renders_checked_markers() {
        let items = vec![
            SelectionItem::new("telegram", None::<&str>),
            SelectionItem::new("wecom", None::<&str>),
        ];
        let widget = SelectionCardWidget::new(items).with_layout(SelectionCardLayout::PosterList);
        let mut state = SelectionCardState::new(2);

        state.select(1);
        state.set_checked_indices([0, 1]);

        let area = Rect::new(0, 0, 48, 5);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &mut state);

        let content = buffer_text(&buf);
        assert!(content.contains("[x]"));
        assert!(content.contains("telegram"));
        assert!(content.contains("wecom"));
    }

    #[test]
    fn framed_layout_keeps_two_digit_indices_visible() {
        let items = (1..=11)
            .map(|index| SelectionItem::new(format!("Channel {index}"), None::<&str>))
            .collect::<Vec<_>>();
        let widget = SelectionCardWidget::new(items);
        let mut state = SelectionCardState::new(11);

        state.select(10);

        let area = Rect::new(0, 0, 48, 33);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &mut state);

        let content = buffer_text(&buf);
        assert!(
            content.contains("[10]"),
            "missing [10] in rendered cards: {content}"
        );
        assert!(
            content.contains("[11]"),
            "missing [11] in rendered cards: {content}"
        );
    }

    #[test]
    fn framed_layout_centers_selected_item_when_viewport_can_scroll() {
        let items = (1..=7)
            .map(|index| SelectionItem::new(format!("Item {index}"), None::<&str>))
            .collect::<Vec<_>>();
        let widget = SelectionCardWidget::new(items);
        let mut state = SelectionCardState::new(7);

        state.select(3);

        let area = Rect::new(0, 0, 40, 9);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &mut state);

        let line_index = find_line_index(&buf, "Item 4").expect("selected item should render");
        assert_eq!(line_index, 4, "selected item should stay centered");
    }

    #[test]
    fn framed_layout_keeps_first_item_pinned_near_top_when_list_starts() {
        let items = vec![
            SelectionItem::new("OpenAI", None::<&str>),
            SelectionItem::new("Anthropic", None::<&str>),
        ];
        let widget = SelectionCardWidget::new(items);
        let mut state = SelectionCardState::new(2);

        state.select(0);

        let area = Rect::new(0, 0, 48, 12);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &mut state);

        let line_index = find_line_index(&buf, "OpenAI").expect("first item should render");
        assert_eq!(line_index, 1, "short lists should not float vertically");
    }

    #[test]
    fn framed_layout_keeps_short_list_stable_when_middle_item_is_selected() {
        let items = vec![
            SelectionItem::new("OpenAI", None::<&str>),
            SelectionItem::new("Anthropic", None::<&str>),
            SelectionItem::new("Gemini", None::<&str>),
        ];
        let widget = SelectionCardWidget::new(items);
        let mut state = SelectionCardState::new(3);

        state.select(1);

        let area = Rect::new(0, 0, 48, 12);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &mut state);

        let line_index = find_line_index(&buf, "Anthropic").expect("middle item should render");
        assert_eq!(
            line_index, 4,
            "short lists should stay top-anchored instead of reflowing when focus changes"
        );
    }

    #[test]
    fn framed_layout_keeps_short_list_stable_when_last_item_is_selected() {
        let items = vec![
            SelectionItem::new("OpenAI", None::<&str>),
            SelectionItem::new("Anthropic", None::<&str>),
        ];
        let widget = SelectionCardWidget::new(items);
        let mut state = SelectionCardState::new(2);

        state.select(1);

        let area = Rect::new(0, 0, 48, 12);
        let mut buf = Buffer::empty(area);

        widget.render(area, &mut buf, &mut state);

        let line_index = find_line_index(&buf, "Anthropic").expect("last item should render");
        assert_eq!(
            line_index, 4,
            "short lists should not drop toward the footer when the last item is focused"
        );
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

    fn find_line_index(buf: &Buffer, needle: &str) -> Option<u16> {
        let lines = buffer_text(buf)
            .lines()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();

        lines
            .iter()
            .position(|line| line.contains(needle))
            .and_then(|index| u16::try_from(index).ok())
    }
}
