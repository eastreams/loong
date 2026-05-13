use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use similar::{ChangeTag, TextDiff};

const DEFAULT_DIFF_WRAP_WIDTH: usize = 72;

#[derive(Clone, Debug)]
struct DiffGutter {
    number_width: usize,
    number: Option<usize>,
    sign: char,
}

impl DiffGutter {
    fn first_line_spans(&self, style: Style) -> Vec<Span<'static>> {
        let number = self
            .number
            .map(|value| format!("{value:>width$}", width = self.number_width))
            .unwrap_or_else(|| " ".repeat(self.number_width));
        vec![
            Span::raw("  "),
            Span::styled(number, Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(self.sign.to_string(), style),
            Span::raw(" "),
        ]
    }

    fn continuation_spans(&self, style: Style) -> Vec<Span<'static>> {
        vec![
            Span::raw("  "),
            Span::styled(" ".repeat(self.number_width), Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(" ".to_owned(), style),
            Span::raw(" "),
        ]
    }

    fn content_width(&self, wrap_width: usize) -> usize {
        wrap_width
            .saturating_sub(self.number_width + 4)
            .max(1)
    }
}

pub fn render_diff_to_lines(diff: &str, wrap_width: usize) -> Vec<Line<'static>> {
    let raw_lines = diff.lines().collect::<Vec<_>>();
    let mut rendered = Vec::new();
    let mut index = 0usize;
    let line_number_width = diff_line_number_width(raw_lines.as_slice());
    let wrap_width = wrap_width.max(line_number_width + 6).max(DEFAULT_DIFF_WRAP_WIDTH.min(wrap_width.max(1)));
    let mut old_line = 1usize;
    let mut new_line = 1usize;

    while index < raw_lines.len() {
        let Some(current) = raw_lines.get(index).copied() else {
            break;
        };
        if current.starts_with("@@") {
            if let Some((old_start, new_start)) = parse_hunk_line_numbers(current) {
                old_line = old_start;
                new_line = new_start;
            }
            rendered.extend(render_plain_diff_line(current, line_number_width, wrap_width, None));
            index += 1;
            continue;
        }
        if is_removed_content_line(current)
            && let Some(removed) = current.strip_prefix('-')
            && let Some(next) = raw_lines
                .get(index + 1)
                .filter(|line| is_added_content_line(line))
                .and_then(|line| line.strip_prefix('+'))
        {
            let (removed_line, added_line) = render_intraline_pair(removed, next);
            rendered.extend(prefixed_wrapped_line(
                Some(old_line),
                '-',
                removed_line,
                line_number_width,
                wrap_width,
            ));
            rendered.extend(prefixed_wrapped_line(
                Some(new_line),
                '+',
                added_line,
                line_number_width,
                wrap_width,
            ));
            old_line += 1;
            new_line += 1;
            index += 2;
            continue;
        }

        let number = if is_removed_content_line(current) {
            let current_number = old_line;
            old_line += 1;
            Some(current_number)
        } else if is_added_content_line(current) {
            let current_number = new_line;
            new_line += 1;
            Some(current_number)
        } else if current.starts_with("diff --git ")
            || current.starts_with("--- ")
            || current.starts_with("+++ ")
            || current.starts_with("index ")
        {
            None
        } else {
            let current_number = new_line.max(old_line);
            old_line += 1;
            new_line += 1;
            Some(current_number)
        };

        rendered.extend(render_plain_diff_line(
            current,
            line_number_width,
            wrap_width,
            number,
        ));
        index += 1;
    }
    if rendered.is_empty() {
        rendered.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("(empty diff)", Style::default().fg(Color::DarkGray)),
        ]));
    }
    rendered
}

pub fn render_diff_to_strings(diff: &str, wrap_width: usize) -> Vec<String> {
    render_diff_to_lines(diff, wrap_width)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>()
        })
        .collect()
}

fn render_plain_diff_line(
    raw_line: &str,
    line_number_width: usize,
    wrap_width: usize,
    line_number: Option<usize>,
) -> Vec<Line<'static>> {
    if let Some(path) = diff_file_path(raw_line) {
        return wrap_diff_header_line("file ", path.as_str(), wrap_width, true);
    }

    if raw_line.starts_with("@@") {
        return wrap_diff_header_line("", raw_line, wrap_width, false);
    }

    if let Some(rest) = raw_line.strip_prefix("--- ") {
        return vec![file_marker_line("old ", rest, Color::Rgb(255, 120, 120))];
    }
    if let Some(rest) = raw_line.strip_prefix("+++ ") {
        return vec![file_marker_line("new ", rest, Color::Rgb(120, 255, 120))];
    }

    let (style, sign, text) = if let Some(rest) = raw_line.strip_prefix('+') {
        (Style::default().fg(Color::Rgb(100, 255, 100)), '+', rest)
    } else if let Some(rest) = raw_line.strip_prefix('-') {
        (Style::default().fg(Color::Rgb(255, 100, 100)), '-', rest)
    } else {
        (Style::default().fg(Color::DarkGray), ' ', raw_line)
    };

    prefixed_wrapped_plain_line(text, sign, style, line_number, line_number_width, wrap_width)
}

fn is_removed_content_line(raw_line: &str) -> bool {
    raw_line.starts_with('-') && !raw_line.starts_with("--- ")
}

fn is_added_content_line(raw_line: &str) -> bool {
    raw_line.starts_with('+') && !raw_line.starts_with("+++ ")
}

fn diff_file_path(raw_line: &str) -> Option<String> {
    let rest = raw_line.strip_prefix("diff --git ")?;
    let path = rest
        .split_whitespace()
        .nth(1)
        .or_else(|| rest.split_whitespace().next())?;
    Some(
        path.trim_start_matches("b/")
            .trim_start_matches("a/")
            .to_owned(),
    )
}

fn file_marker_line(label: &str, path: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            label.to_owned(),
            Style::default().fg(color).add_modifier(Modifier::DIM),
        ),
        Span::styled(path.to_owned(), Style::default().fg(color)),
    ])
}

fn wrap_diff_header_line(
    prefix: &str,
    body: &str,
    wrap_width: usize,
    keep_prefix_visible: bool,
) -> Vec<Line<'static>> {
    let style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let content_width = wrap_width.saturating_sub(2).max(1);
    let wrapped = if keep_prefix_visible {
        crate::presentation::render_wrapped_text_line_with_continuation(
            prefix,
            " ".repeat(prefix.chars().count()).as_str(),
            body,
            content_width,
        )
    } else {
        crate::presentation::render_wrapped_plain_display_line(
            format!("{prefix}{body}").as_str(),
            content_width,
        )
    };
    wrapped
        .into_iter()
        .map(|wrapped| {
            Line::from(vec![
                Span::raw("  "),
                Span::styled(wrapped, style),
            ])
        })
    .collect()
}

fn render_intraline_pair(removed: &str, added: &str) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    let base_removed = Style::default().fg(Color::Rgb(255, 100, 100));
    let base_added = Style::default().fg(Color::Rgb(100, 255, 100));
    let highlight_removed = base_removed.add_modifier(Modifier::REVERSED);
    let highlight_added = base_added.add_modifier(Modifier::REVERSED);
    let diff = TextDiff::from_words(removed, added);
    let mut removed_spans = Vec::new();
    let mut added_spans = Vec::new();

    for change in diff.iter_all_changes() {
        let text = change.to_string().replace('\n', "");
        match change.tag() {
            ChangeTag::Delete => removed_spans.push(Span::styled(text, highlight_removed)),
            ChangeTag::Insert => added_spans.push(Span::styled(text, highlight_added)),
            ChangeTag::Equal => {
                removed_spans.push(Span::styled(text.clone(), base_removed));
                added_spans.push(Span::styled(text, base_added));
            }
        }
    }

    (removed_spans, added_spans)
}

fn diff_line_number_width(lines: &[&str]) -> usize {
    let mut old_line = 1usize;
    let mut new_line = 1usize;
    let mut max_line = 1usize;
    for line in lines {
        if line.starts_with("@@") {
            continue;
        }
        if is_removed_content_line(line) {
            max_line = max_line.max(old_line);
            old_line += 1;
        } else if is_added_content_line(line) {
            max_line = max_line.max(new_line);
            new_line += 1;
        } else {
            max_line = max_line.max(old_line.max(new_line));
            old_line += 1;
            new_line += 1;
        }
    }
    max_line.to_string().len().max(1)
}

fn prefixed_wrapped_plain_line(
    text: &str,
    sign: char,
    style: Style,
    line_number: Option<usize>,
    line_number_width: usize,
    wrap_width: usize,
) -> Vec<Line<'static>> {
    let gutter = DiffGutter {
        number_width: line_number_width,
        number: line_number,
        sign,
    };
    let content_width = gutter.content_width(wrap_width);
    crate::presentation::render_wrapped_plain_display_line(text, content_width)
        .into_iter()
        .enumerate()
        .map(|(index, wrapped)| {
            let mut spans = if index == 0 {
                gutter.first_line_spans(style)
            } else {
                gutter.continuation_spans(style)
            };
            spans.push(Span::styled(wrapped, style));
            Line::from(spans)
        })
        .collect()
}

fn parse_hunk_line_numbers(raw_line: &str) -> Option<(usize, usize)> {
    let parts = raw_line.split_whitespace().collect::<Vec<_>>();
    let old_range = parts.get(1)?.trim_start_matches('-');
    let new_range = parts.get(2)?.trim_start_matches('+');
    let old_start = old_range.split(',').next()?.parse::<usize>().ok()?;
    let new_start = new_range.split(',').next()?.parse::<usize>().ok()?;
    Some((old_start, new_start))
}

fn prefixed_wrapped_line(
    line_number: Option<usize>,
    sign: char,
    spans: Vec<Span<'static>>,
    line_number_width: usize,
    wrap_width: usize,
) -> Vec<Line<'static>> {
    let text = spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let style = spans.first().map(|span| span.style).unwrap_or_default();
    let gutter = DiffGutter {
        number_width: line_number_width,
        number: line_number,
        sign,
    };
    let content_width = gutter.content_width(wrap_width);
    crate::presentation::render_wrapped_plain_display_line(text.as_str(), content_width)
        .into_iter()
        .enumerate()
        .map(|(index, wrapped)| {
            let mut line_spans = if index == 0 {
                gutter.first_line_spans(style)
            } else {
                gutter.continuation_spans(style)
            };
            line_spans.push(Span::styled(wrapped, style));
            Line::from(line_spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::render_diff_to_lines;

    fn line_texts() -> impl Fn(ratatui::text::Line<'static>) -> String {
        |line| {
            line.spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>()
        }
    }

    #[test]
    fn renders_empty_diff_placeholder() {
        let lines = render_diff_to_lines("", 72)
            .into_iter()
            .map(line_texts())
            .collect::<Vec<_>>();

        assert_eq!(lines, vec!["  (empty diff)".to_owned()]);
    }

    #[test]
    fn keeps_context_and_changed_lines_legible() {
        let lines = render_diff_to_lines(
            " context line
-old value
+new value
 trailing context",
            72,
        )
        .into_iter()
        .map(line_texts())
        .collect::<Vec<_>>();

        assert_eq!(lines.first().map(String::as_str), Some("  1    context line"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("old") && line.contains("value"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("new") && line.contains("value"))
        );
        assert_eq!(
            lines.last().map(String::as_str),
            Some("  3    trailing context")
        );
    }

    #[test]
    fn renders_file_and_hunk_headers_without_treating_markers_as_edits() {
        let lines = render_diff_to_lines(
            "diff --git a/src/lib.rs b/src/lib.rs
index 111..222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,2 @@
-old value
+new value",
            72,
        )
        .into_iter()
        .map(line_texts())
        .collect::<Vec<_>>();

        assert!(lines.iter().any(|line| line.contains("file src/lib.rs")));
        assert!(lines.iter().any(|line| line.contains("old a/src/lib.rs")));
        assert!(lines.iter().any(|line| line.contains("new b/src/lib.rs")));
        assert!(lines.iter().any(|line| line.contains("@@ -1,2 +1,2 @@")));
        assert!(
            lines.iter().any(|line| line.contains("old value")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("new value")),
            "{lines:?}"
        );
    }

    #[test]
    fn wraps_long_diff_lines_to_requested_width() {
        let lines = render_diff_to_lines(
            "+this is a very long inserted line that should wrap cleanly in the diff gutter",
            28,
        )
        .into_iter()
        .map(line_texts())
        .collect::<Vec<_>>();

        assert!(lines.len() >= 2, "{lines:?}");
        assert!(lines.iter().all(|line| crate::presentation::display_width(line) <= 28));
    }

    #[test]
    fn wrapped_diff_continuation_keeps_gutter_alignment() {
        let lines = render_diff_to_lines(
            "+this is a very long inserted line that should wrap cleanly in the diff gutter",
            28,
        )
        .into_iter()
        .map(line_texts())
        .collect::<Vec<_>>();

        assert!(lines.len() >= 2, "{lines:?}");
        assert!(lines.first().is_some_and(|line| line.contains(" + ")), "{lines:?}");
        assert!(
            lines
                .get(1)
                .is_some_and(|line| line.starts_with("      ")),
            "{lines:?}"
        );
    }

    #[test]
    fn wraps_diff_headers_to_requested_width() {
        let lines = render_diff_to_lines(
            "diff --git a/src/features/very/long/path/example_component.rs b/src/features/very/long/path/example_component.rs\n@@ -120,12 +120,14 @@",
            32,
        )
        .into_iter()
        .map(line_texts())
        .collect::<Vec<_>>();

        assert!(lines.len() >= 2, "{lines:?}");
        assert!(lines.iter().all(|line| crate::presentation::display_width(line) <= 32));
        assert!(lines.iter().any(|line| line.contains("file ")));
        assert!(lines.iter().any(|line| line.contains("@@ -120,12 +120,14 @@")));
    }
}
