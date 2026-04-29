use super::utils::*;
use pulldown_cmark::{Alignment, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

const MARKDOWN_TABLE_DEFAULT_RENDER_WIDTH: usize = 96;
const MARKDOWN_TABLE_MAX_CELL_WIDTH: usize = 36;
const MARKDOWN_TABLE_MIN_CELL_WIDTH: usize = 3;

#[derive(Debug, Default)]
struct MarkdownTableState {
    in_header: bool,
    alignments: Vec<Alignment>,
    current_row: Vec<String>,
    current_cell: String,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

#[allow(dead_code)]
pub fn render_markdown_to_lines(md: &str) -> Vec<Line<'static>> {
    render_markdown_to_lines_with_width(md, None)
}

pub fn render_markdown_to_lines_with_width(md: &str, width: Option<usize>) -> Vec<Line<'static>> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(md, options);
    let mut lines = Vec::new();
    let mut current_spans = Vec::new();

    let mut in_code_block = false;
    let mut in_quote = false;
    let mut in_image = false;
    let mut image_url: Option<String> = None;
    let mut image_alt = String::new();
    let mut list_depth: usize = 0;
    let mut table_state: Option<MarkdownTableState> = None;

    let mut current_style = Style::default();

    for event in parser {
        if let Some(table) = table_state.as_mut() {
            match event {
                Event::Start(Tag::TableHead) => {
                    table.in_header = true;
                    continue;
                }
                Event::End(TagEnd::TableHead) => {
                    if table.headers.is_empty() && !table.current_row.is_empty() {
                        table.headers = std::mem::take(&mut table.current_row);
                    }
                    table.in_header = false;
                    continue;
                }
                Event::Start(Tag::TableRow) => {
                    table.current_row.clear();
                    continue;
                }
                Event::End(TagEnd::TableRow) => {
                    let row = std::mem::take(&mut table.current_row);
                    if table.headers.is_empty() {
                        table.headers = row;
                    } else {
                        table.rows.push(row);
                    }
                    continue;
                }
                Event::Start(Tag::TableCell) => {
                    table.current_cell.clear();
                    continue;
                }
                Event::End(TagEnd::TableCell) => {
                    table
                        .current_row
                        .push(normalize_markdown_table_cell(table.current_cell.as_str()));
                    table.current_cell.clear();
                    continue;
                }
                Event::Text(text) => {
                    table.current_cell.push_str(text.as_ref());
                    continue;
                }
                Event::Code(text) => {
                    table.current_cell.push_str(text.as_ref());
                    continue;
                }
                Event::SoftBreak | Event::HardBreak => {
                    if !table.current_cell.ends_with(' ') {
                        table.current_cell.push(' ');
                    }
                    continue;
                }
                Event::End(TagEnd::Table) => {
                    let rendered = render_markdown_table(
                        std::mem::take(&mut table.headers),
                        std::mem::take(&mut table.rows),
                        std::mem::take(&mut table.alignments),
                        width,
                    );
                    lines.extend(rendered);
                    lines.push(Line::from(""));
                    table_state = None;
                    continue;
                }
                Event::Start(Tag::Table(_))
                | Event::Start(Tag::Paragraph)
                | Event::Start(Tag::Heading { .. })
                | Event::Start(Tag::BlockQuote(_))
                | Event::Start(Tag::CodeBlock(_))
                | Event::Start(Tag::HtmlBlock)
                | Event::Start(Tag::List(_))
                | Event::Start(Tag::Item)
                | Event::Start(Tag::FootnoteDefinition(_))
                | Event::Start(Tag::DefinitionList)
                | Event::Start(Tag::DefinitionListTitle)
                | Event::Start(Tag::DefinitionListDefinition)
                | Event::Start(Tag::Emphasis)
                | Event::Start(Tag::Strong)
                | Event::Start(Tag::Strikethrough)
                | Event::Start(Tag::Superscript)
                | Event::Start(Tag::Subscript)
                | Event::Start(Tag::Link { .. })
                | Event::Start(Tag::Image { .. })
                | Event::Start(Tag::MetadataBlock(_))
                | Event::End(_)
                | Event::InlineMath(_)
                | Event::DisplayMath(_)
                | Event::Html(_)
                | Event::InlineHtml(_)
                | Event::FootnoteReference(_)
                | Event::Rule
                | Event::TaskListMarker(_) => continue,
            }
        }

        match event {
            Event::Start(Tag::Table(alignments)) => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
                table_state = Some(MarkdownTableState {
                    alignments,
                    ..MarkdownTableState::default()
                });
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
                let lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(l) => l.to_string(),
                    _ => "".to_string(),
                };
                in_code_block = true;
                // chat-surface style: ```lang in dim gray
                lines.push(Line::from(Span::styled(
                    format!("```{}", lang),
                    Style::default()
                        .fg(SURFACE_GRAY)
                        .add_modifier(Modifier::DIM),
                )));
            }
            Event::End(TagEnd::CodeBlock) => {
                if !current_spans.is_empty() {
                    let content = std::mem::take(&mut current_spans)
                        .into_iter()
                        .map(|s| s.content.into_owned())
                        .collect::<String>();
                    for l in content.lines() {
                        lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(l.to_string(), Style::default().fg(SURFACE_GREEN)),
                        ]));
                    }
                }
                in_code_block = false;
                lines.push(Line::from(Span::styled(
                    "```",
                    Style::default()
                        .fg(SURFACE_GRAY)
                        .add_modifier(Modifier::DIM),
                )));
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
                in_image = true;
                image_url = Some(dest_url.to_string());
                image_alt.clear();
            }
            Event::End(TagEnd::Image) => {
                let alt = if image_alt.trim().is_empty() {
                    "image".to_owned()
                } else {
                    image_alt.trim().to_owned()
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        "[image] ",
                        Style::default()
                            .fg(SURFACE_CYAN)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(alt, Style::default().fg(SURFACE_ACCENT)),
                ]));
                if let Some(url) = image_url.take() {
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(url, Style::default().fg(SURFACE_DIM_GRAY)),
                    ]));
                }
                lines.push(Line::from(""));
                in_image = false;
            }
            Event::Start(Tag::BlockQuote(_)) => in_quote = true,
            Event::End(TagEnd::BlockQuote(_)) => {
                if !current_spans.is_empty() {
                    let mut line_spans =
                        vec![Span::styled("┃ ", Style::default().fg(SURFACE_GRAY))];
                    line_spans.extend(std::mem::take(&mut current_spans));
                    lines.push(Line::from(line_spans));
                }
                in_quote = false;
            }
            Event::Start(Tag::List(_)) => list_depth += 1,
            Event::End(TagEnd::List(_)) => list_depth -= 1,
            Event::Start(Tag::Item) => {
                let indent = "  ".repeat(list_depth.saturating_sub(1));
                current_spans.push(Span::styled(
                    format!("{indent}• "),
                    Style::default().fg(SURFACE_ACCENT),
                ));
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let prefix = match level {
                    HeadingLevel::H1 => "# ",
                    HeadingLevel::H2 => "## ",
                    HeadingLevel::H3 => "### ",
                    HeadingLevel::H4 | HeadingLevel::H5 | HeadingLevel::H6 => "#### ",
                };
                current_spans.push(Span::styled(
                    prefix.to_string(),
                    Style::default()
                        .fg(SURFACE_HEADING)
                        .add_modifier(Modifier::BOLD),
                ));
                current_style = current_style
                    .add_modifier(Modifier::BOLD)
                    .fg(SURFACE_HEADING);
            }
            Event::End(TagEnd::Heading(_)) => {
                current_style = current_style
                    .remove_modifier(Modifier::BOLD)
                    .fg(ratatui::style::Color::Reset);
                lines.push(Line::from(std::mem::take(&mut current_spans)));
                lines.push(Line::from(""));
            }
            Event::Start(Tag::Strong) => current_style = current_style.add_modifier(Modifier::BOLD),
            Event::End(TagEnd::Strong) => {
                current_style = current_style.remove_modifier(Modifier::BOLD)
            }
            Event::Start(Tag::Emphasis) => {
                current_style = current_style.add_modifier(Modifier::ITALIC)
            }
            Event::End(TagEnd::Emphasis) => {
                current_style = current_style.remove_modifier(Modifier::ITALIC)
            }
            Event::Code(text) => {
                current_spans.push(Span::styled(
                    text.to_string(),
                    Style::default().fg(SURFACE_ACCENT),
                ));
            }
            Event::Text(text) => {
                if in_image {
                    image_alt.push_str(text.as_ref());
                    continue;
                }
                if in_code_block {
                    for (i, line) in text.lines().enumerate() {
                        if i > 0 {
                            lines.push(Line::from(vec![
                                Span::raw("  "),
                                Span::styled(line.to_string(), Style::default().fg(SURFACE_GREEN)),
                            ]));
                        } else {
                            current_spans.push(Span::styled(
                                line.to_string(),
                                Style::default().fg(SURFACE_GREEN),
                            ));
                        }
                    }
                } else {
                    current_spans.push(Span::styled(text.to_string(), current_style));
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_quote {
                    let mut line_spans =
                        vec![Span::styled("┃ ", Style::default().fg(SURFACE_GRAY))];
                    line_spans.extend(std::mem::take(&mut current_spans));
                    lines.push(Line::from(line_spans));
                } else {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if in_quote {
                    let mut line_spans =
                        vec![Span::styled("┃ ", Style::default().fg(SURFACE_GRAY))];
                    line_spans.extend(std::mem::take(&mut current_spans));
                    lines.push(Line::from(line_spans));
                } else {
                    lines.push(Line::from(std::mem::take(&mut current_spans)));
                }
                lines.push(Line::from(""));
            }
            Event::Start(_)
            | Event::End(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
            | Event::Html(_)
            | Event::InlineHtml(_)
            | Event::FootnoteReference(_)
            | Event::Rule
            | Event::TaskListMarker(_) => {}
        }
    }

    if !current_spans.is_empty() {
        lines.push(Line::from(current_spans));
    }

    lines
}

fn normalize_markdown_table_cell(cell: &str) -> String {
    cell.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn render_markdown_table(
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    mut alignments: Vec<Alignment>,
    width: Option<usize>,
) -> Vec<Line<'static>> {
    let column_count = headers
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if column_count == 0 {
        return Vec::new();
    }

    let mut normalized_headers = headers;
    normalized_headers.resize(column_count, String::new());

    let normalized_rows = rows
        .into_iter()
        .map(|mut row| {
            row.resize(column_count, String::new());
            row
        })
        .collect::<Vec<_>>();
    alignments.resize(column_count, Alignment::None);

    let max_render_width = width.unwrap_or(MARKDOWN_TABLE_DEFAULT_RENDER_WIDTH).max(1);
    let max_cell_width = markdown_table_max_cell_width(max_render_width, column_count);
    let mut widths = (0..column_count)
        .map(|index| {
            let header_width = normalized_headers
                .get(index)
                .map(|header| crate::presentation::display_width(header))
                .unwrap_or(0);
            let row_width = normalized_rows
                .iter()
                .filter_map(|row| row.get(index))
                .map(|cell| crate::presentation::display_width(cell))
                .max()
                .unwrap_or(0);
            header_width
                .max(row_width)
                .clamp(MARKDOWN_TABLE_MIN_CELL_WIDTH, max_cell_width)
        })
        .collect::<Vec<_>>();

    if max_render_width < markdown_table_minimum_width(column_count) {
        return render_markdown_table_stacked(
            normalized_headers.as_slice(),
            normalized_rows.as_slice(),
            max_render_width.max(MARKDOWN_TABLE_MIN_CELL_WIDTH + 2),
        );
    }
    fit_markdown_table_widths(&mut widths, max_render_width);

    if markdown_table_total_width(&widths) > max_render_width {
        return render_markdown_table_stacked(
            normalized_headers.as_slice(),
            normalized_rows.as_slice(),
            max_render_width,
        );
    }

    let mut lines = Vec::new();
    lines.push(Line::from(render_markdown_table_separator(
        '┌', '┬', '┐', &widths,
    )));
    lines.extend(render_markdown_table_row_lines(
        normalized_headers.as_slice(),
        widths.as_slice(),
        alignments.as_slice(),
    ));
    lines.push(Line::from(render_markdown_table_separator(
        '├', '┼', '┤', &widths,
    )));
    for row in &normalized_rows {
        lines.extend(render_markdown_table_row_lines(
            row.as_slice(),
            widths.as_slice(),
            alignments.as_slice(),
        ));
    }
    lines.push(Line::from(render_markdown_table_separator(
        '└', '┴', '┘', &widths,
    )));
    lines
}

fn markdown_table_max_cell_width(max_render_width: usize, column_count: usize) -> usize {
    let decoration_width = column_count.saturating_mul(3).saturating_add(1);
    let available_for_cells = max_render_width.saturating_sub(decoration_width);
    let balanced_width = available_for_cells
        .checked_div(column_count.max(1))
        .unwrap_or(MARKDOWN_TABLE_MIN_CELL_WIDTH);
    balanced_width
        .saturating_add(8)
        .clamp(MARKDOWN_TABLE_MIN_CELL_WIDTH, MARKDOWN_TABLE_MAX_CELL_WIDTH)
}

fn fit_markdown_table_widths(widths: &mut [usize], max_total_width: usize) {
    while markdown_table_total_width(widths) > max_total_width {
        let Some((index, width)) = widths
            .iter()
            .copied()
            .enumerate()
            .max_by_key(|(_, width)| *width)
        else {
            break;
        };
        if width <= MARKDOWN_TABLE_MIN_CELL_WIDTH {
            break;
        }
        if let Some(entry) = widths.get_mut(index) {
            *entry = width.saturating_sub(1);
        }
    }
}

fn markdown_table_total_width(widths: &[usize]) -> usize {
    widths.iter().sum::<usize>() + widths.len() * 3 + 1
}

fn markdown_table_minimum_width(column_count: usize) -> usize {
    column_count * (MARKDOWN_TABLE_MIN_CELL_WIDTH + 3) + 1
}

fn render_markdown_table_separator(
    left: char,
    middle: char,
    right: char,
    widths: &[usize],
) -> String {
    let mut line = String::new();
    line.push(left);
    for (index, width) in widths.iter().enumerate() {
        line.push_str(&"─".repeat(width.saturating_add(2)));
        line.push(if index + 1 == widths.len() {
            right
        } else {
            middle
        });
    }
    line
}

fn render_markdown_table_row_lines(
    cells: &[String],
    widths: &[usize],
    alignments: &[Alignment],
) -> Vec<Line<'static>> {
    let wrapped_cells = cells
        .iter()
        .zip(widths.iter().copied())
        .map(|(cell, width)| wrap_markdown_table_cell(cell, width))
        .collect::<Vec<_>>();
    let row_height = wrapped_cells.iter().map(Vec::len).max().unwrap_or(1).max(1);

    (0..row_height)
        .map(|line_index| {
            let mut line = String::new();
            line.push('│');
            for ((cell_lines, width), alignment) in wrapped_cells
                .iter()
                .zip(widths.iter().copied())
                .zip(alignments.iter().copied())
            {
                let rendered_cell = cell_lines.get(line_index).map(String::as_str).unwrap_or("");
                let rendered_width = crate::presentation::display_width(rendered_cell);
                let (left_padding, right_padding) =
                    markdown_table_cell_padding(width, rendered_width, alignment);
                line.push(' ');
                line.push_str(&" ".repeat(left_padding));
                line.push_str(rendered_cell);
                line.push_str(&" ".repeat(right_padding));
                line.push(' ');
                line.push('│');
            }
            Line::from(line)
        })
        .collect()
}

fn wrap_markdown_table_cell(cell: &str, width: usize) -> Vec<String> {
    if cell.trim().is_empty() {
        return vec![String::new()];
    }

    let wrapped = crate::presentation::render_wrapped_display_line(cell.trim(), width.max(1));
    if wrapped.is_empty() {
        vec![String::new()]
    } else {
        wrapped
    }
}

fn markdown_table_cell_padding(
    width: usize,
    rendered_width: usize,
    alignment: Alignment,
) -> (usize, usize) {
    let remaining = width.saturating_sub(rendered_width);
    match alignment {
        Alignment::Right => (remaining, 0),
        Alignment::Center => (remaining / 2, remaining - (remaining / 2)),
        Alignment::None | Alignment::Left => (0, remaining),
    }
}

fn render_markdown_table_stacked(
    headers: &[String],
    rows: &[Vec<String>],
    max_width: usize,
) -> Vec<Line<'static>> {
    let content_width = max_width.max(1);
    let mut rendered = Vec::new();
    for (row_index, row) in rows.iter().enumerate() {
        let row_marker = if row_index == 0 { '┌' } else { '├' };
        rendered.push(Line::from(format!("{row_marker}─ row {} ─", row_index + 1)));
        for (header, cell) in headers.iter().zip(row.iter()) {
            let label = if header.trim().is_empty() {
                "value"
            } else {
                header.trim()
            };
            let label = fit_markdown_table_label(label, content_width.saturating_sub(4).max(1));
            let prefix = format!("  {label}: ");
            let body_width = content_width
                .saturating_sub(crate::presentation::display_width(prefix.as_str()))
                .max(1);
            let wrapped_cell =
                crate::presentation::render_wrapped_display_line(cell.trim(), body_width);
            if wrapped_cell.is_empty() {
                rendered.push(Line::from(prefix));
                continue;
            }
            for (line_index, wrapped) in wrapped_cell.into_iter().enumerate() {
                if line_index == 0 {
                    rendered.push(Line::from(format!("{prefix}{wrapped}")));
                } else {
                    rendered.push(Line::from(format!(
                        "{}{wrapped}",
                        " ".repeat(crate::presentation::display_width(prefix.as_str()))
                    )));
                }
            }
        }
    }
    rendered
}

fn fit_markdown_table_label(label: &str, max_width: usize) -> String {
    if crate::presentation::display_width(label) <= max_width {
        return label.to_owned();
    }
    if max_width <= 1 {
        return "…".to_owned();
    }

    let mut rendered = String::new();
    let mut used_width = 0usize;
    for ch in label.chars() {
        let ch_width = crate::presentation::char_display_width(ch);
        if used_width.saturating_add(ch_width).saturating_add(1) > max_width {
            break;
        }
        rendered.push(ch);
        used_width = used_width.saturating_add(ch_width);
    }
    rendered.push('…');
    rendered
}

#[cfg(test)]
mod tests {
    use super::{render_markdown_to_lines, render_markdown_to_lines_with_width};

    fn lines_to_strings(lines: Vec<ratatui::text::Line<'static>>) -> Vec<String> {
        lines
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.into_owned())
                    .collect::<String>()
            })
            .collect()
    }

    fn non_blank_lines(lines: Vec<ratatui::text::Line<'static>>) -> Vec<String> {
        lines_to_strings(lines)
            .into_iter()
            .filter(|line| !line.trim().is_empty())
            .collect()
    }

    fn assert_uniform_display_width(lines: &[String]) {
        let Some(first_width) = lines
            .first()
            .map(|line| crate::presentation::display_width(line))
        else {
            return;
        };

        for line in lines {
            assert_eq!(
                crate::presentation::display_width(line),
                first_width,
                "table line has a different display width: {line:?}"
            );
        }
    }

    #[test]
    fn renders_markdown_images_as_placeholder_lines() {
        let lines = render_markdown_to_lines("before\n\n![diagram](https://example.com/a.png)\n");
        let joined = lines_to_strings(lines).join("\n");

        assert!(joined.contains("[image] diagram"));
        assert!(joined.contains("https://example.com/a.png"));
    }

    #[test]
    fn renders_markdown_tables_as_grid_lines() {
        let lines = render_markdown_to_lines(
            "| 指标 | 数值 |\n| --- | --- |\n| 覆盖率 | 68% |\n| 平均响应时间 | 220ms |",
        );
        let joined = lines_to_strings(lines).join("\n");

        assert!(joined.contains("┌"));
        assert!(joined.contains("┬"));
        assert!(joined.contains("指标"));
        assert!(joined.contains("覆盖率"));
        assert!(joined.contains("220ms"));
    }

    #[test]
    fn renders_markdown_tables_with_stable_padding_and_borders() {
        let lines = non_blank_lines(render_markdown_to_lines_with_width(
            "| Name | Value |\n| --- | --- |\n| A | 1 |\n| B | 2 |",
            Some(32),
        ));

        assert_eq!(
            lines,
            vec![
                "┌──────┬───────┐".to_owned(),
                "│ Name │ Value │".to_owned(),
                "├──────┼───────┤".to_owned(),
                "│ A    │ 1     │".to_owned(),
                "│ B    │ 2     │".to_owned(),
                "└──────┴───────┘".to_owned(),
            ]
        );
        assert_uniform_display_width(lines.as_slice());
    }

    #[test]
    fn renders_cjk_markdown_tables_with_uniform_display_widths() {
        let lines = non_blank_lines(render_markdown_to_lines_with_width(
            "| 指标 | 数值 |\n| --- | --- |\n| 覆盖率 | 68% |\n| 平均响应时间 | 220ms |",
            Some(40),
        ));

        assert!(lines.iter().any(|line| line.contains("覆盖率")));
        assert!(lines.iter().any(|line| line.contains("220ms")));
        assert_uniform_display_width(lines.as_slice());
    }

    #[test]
    fn renders_markdown_tables_as_stacked_rows_when_width_is_tight() {
        let lines = render_markdown_to_lines_with_width(
            "| 指标 | 数值 |\n| --- | --- |\n| 覆盖率 | 68% |\n| 平均响应时间 | 220ms |",
            Some(12),
        );
        let joined = lines_to_strings(lines).join("\n");

        assert!(joined.contains("┌─ row 1 ─"));
        assert!(joined.contains("指标:"));
        assert!(joined.contains("覆盖率") || (joined.contains("覆盖") && joined.contains("率")));
        assert!(joined.contains("数值: 68%"));
    }

    #[test]
    fn wraps_markdown_table_cells_instead_of_truncating_values() {
        let lines = render_markdown_to_lines_with_width(
            "| key | value |\n| --- | --- |\n| status | this value should wrap without losing the important trailing words |",
            Some(42),
        );
        let joined = lines_to_strings(lines).join("\n");

        assert!(joined.contains("important"));
        assert!(joined.contains("trailing words"));
        assert!(!joined.contains('…'));
    }
}
