use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use similar::{ChangeTag, TextDiff};

pub fn render_diff_to_lines(diff: &str) -> Vec<Line<'static>> {
    let raw_lines = diff.lines().collect::<Vec<_>>();
    let mut rendered = Vec::new();
    let mut index = 0usize;

    while index < raw_lines.len() {
        let Some(current) = raw_lines.get(index).copied() else {
            break;
        };
        if is_removed_content_line(current)
            && let Some(removed) = current.strip_prefix('-')
            && let Some(next) = raw_lines
                .get(index + 1)
                .filter(|line| is_added_content_line(line))
                .and_then(|line| line.strip_prefix('+'))
        {
            let (removed_line, added_line) = render_intraline_pair(removed, next);
            rendered.push(prefixed_line("  ", "- ", removed_line));
            rendered.push(prefixed_line("  ", "+ ", added_line));
            index += 2;
            continue;
        }

        rendered.push(render_plain_diff_line(current));
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

fn render_plain_diff_line(raw_line: &str) -> Line<'static> {
    if let Some(path) = diff_file_path(raw_line) {
        return Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "file ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                path,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
    }

    if raw_line.starts_with("@@") {
        return Line::from(vec![
            Span::raw("  "),
            Span::styled(
                raw_line.to_owned(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
    }

    if let Some(rest) = raw_line.strip_prefix("--- ") {
        return file_marker_line("old ", rest, Color::Rgb(255, 120, 120));
    }
    if let Some(rest) = raw_line.strip_prefix("+++ ") {
        return file_marker_line("new ", rest, Color::Rgb(120, 255, 120));
    }

    let (style, prefix, text) = if let Some(rest) = raw_line.strip_prefix('+') {
        (
            Style::default().fg(Color::Rgb(100, 255, 100)),
            "+ ",
            rest.to_owned(),
        )
    } else if let Some(rest) = raw_line.strip_prefix('-') {
        (
            Style::default().fg(Color::Rgb(255, 100, 100)),
            "- ",
            rest.to_owned(),
        )
    } else {
        (
            Style::default().fg(Color::DarkGray),
            "  ",
            raw_line.to_owned(),
        )
    };
    Line::from(vec![
        Span::raw("  "),
        Span::styled(prefix, style),
        Span::styled(text, style),
    ])
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

fn prefixed_line(indent: &str, prefix: &str, spans: Vec<Span<'static>>) -> Line<'static> {
    let mut line_spans = vec![Span::raw(indent.to_owned())];
    if let Some(first_style) = spans.first().map(|span| span.style) {
        line_spans.push(Span::styled(prefix.to_owned(), first_style));
    } else {
        line_spans.push(Span::raw(prefix.to_owned()));
    }
    line_spans.extend(spans);
    Line::from(line_spans)
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
        let lines = render_diff_to_lines("")
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
        )
        .into_iter()
        .map(line_texts())
        .collect::<Vec<_>>();

        assert_eq!(lines.first().map(String::as_str), Some("     context line"));
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
            Some("     trailing context")
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
}
