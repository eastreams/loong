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

#[derive(Debug, Clone)]
struct IndentContext {
    prefix: String,
    continuation_prefix: String,
}

#[derive(Debug, Default)]
struct MarkdownWriter {
    lines: Vec<String>,
    current_line: String,
    wrap_width: Option<usize>,
    indent_stack: Vec<IndentContext>,
    in_code_block: bool,
    code_block_lines: Vec<String>,
    in_image: bool,
    image_url: Option<String>,
    image_alt: String,
    current_link_dest: Option<String>,
    current_link_label: String,
}

#[derive(Debug, Clone)]
struct ListContext {
    ordered: bool,
    next_index: usize,
}

impl MarkdownWriter {
    fn new(wrap_width: Option<usize>) -> Self {
        Self {
            wrap_width,
            ..Self::default()
        }
    }

    fn flush_current_line(&mut self) {
        if self.current_line.is_empty() {
            return;
        }
        let line = std::mem::take(&mut self.current_line);
        if let Some(context) = self.indent_stack.last()
            && let Some(body) = line.strip_prefix(context.prefix.as_str())
        {
            self.lines.extend(render_wrapped_markdown_text(
                context.prefix.as_str(),
                context.continuation_prefix.as_str(),
                body,
                self.wrap_width,
            ));
            return;
        }

        self.lines
            .extend(render_wrapped_markdown_text("", "", line.as_str(), self.wrap_width));
    }

    fn push_blank_line(&mut self) {
        self.flush_current_line();
        if self.lines.last().is_some_and(|line| line.is_empty()) {
            return;
        }
        self.lines.push(String::new());
    }

    fn ensure_current_line_prefix(&mut self) {
        if !self.current_line.is_empty() {
            return;
        }
        if let Some(context) = self.indent_stack.last() {
            self.current_line.push_str(&context.prefix);
        }
    }

    fn append_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.ensure_current_line_prefix();
        self.current_line.push_str(text);
    }
}

#[allow(dead_code)]
pub fn render_markdown_to_lines(md: &str) -> Vec<Line<'static>> {
    render_markdown_to_lines_with_width(md, None)
}

pub fn contains_renderable_markdown_structure(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);

    Parser::new_ext(trimmed, options).any(|event| {
        matches!(
            event,
            Event::Start(Tag::Heading { .. })
                | Event::Start(Tag::BlockQuote(_))
                | Event::Start(Tag::CodeBlock(_))
                | Event::Start(Tag::List(_))
                | Event::Start(Tag::Link { .. })
                | Event::Start(Tag::Table(_))
                | Event::Rule
        )
    })
}

pub fn render_markdown_to_lines_with_width(md: &str, width: Option<usize>) -> Vec<Line<'static>> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(md, options);
    let mut lines = Vec::new();
    let mut writer = MarkdownWriter::new(width);

    let mut list_depth: usize = 0;
    let mut list_stack: Vec<ListContext> = Vec::new();
    let mut table_state: Option<MarkdownTableState> = None;

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
                writer.flush_current_line();
                table_state = Some(MarkdownTableState {
                    alignments,
                    ..MarkdownTableState::default()
                });
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                writer.push_blank_line();
                let lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(l) => l.to_string(),
                    _ => "".to_string(),
                };
                writer.in_code_block = true;
                writer.code_block_lines.clear();
                lines.push(Line::from(Span::styled(
                    format!("```{}", lang),
                    Style::default().fg(SURFACE_GRAY).add_modifier(Modifier::DIM),
                )));
            }
            Event::End(TagEnd::CodeBlock) => {
                writer.flush_current_line();
                for line in std::mem::take(&mut writer.code_block_lines) {
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(line, Style::default().fg(SURFACE_GREEN)),
                    ]));
                }
                writer.in_code_block = false;
                lines.push(Line::from(Span::styled(
                    "```",
                    Style::default().fg(SURFACE_GRAY).add_modifier(Modifier::DIM),
                )));
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                writer.push_blank_line();
                writer.in_image = true;
                writer.image_url = Some(dest_url.to_string());
                writer.image_alt.clear();
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                writer.current_link_dest = Some(dest_url.to_string());
                writer.current_link_label.clear();
            }
            Event::End(TagEnd::Link) => {
                if let Some(dest_url) = writer.current_link_dest.take() {
                    let rendered = render_link_spans(
                        dest_url.as_str(),
                        writer.current_link_label.as_str(),
                    );
                    for span in rendered {
                        writer.append_text(span.content.as_ref());
                    }
                }
                writer.current_link_label.clear();
            }
            Event::End(TagEnd::Image) => {
                let alt = if writer.image_alt.trim().is_empty() {
                    "image".to_owned()
                } else {
                    writer.image_alt.trim().to_owned()
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
                if let Some(url) = writer.image_url.take() {
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(url, Style::default().fg(SURFACE_DIM_GRAY)),
                    ]));
                }
                lines.push(Line::from(""));
                writer.in_image = false;
            }
            Event::Start(Tag::BlockQuote(_)) => {
                writer.flush_current_line();
                let depth_prefix = "  ".repeat(list_depth);
                writer.indent_stack.push(IndentContext {
                    prefix: format!("{depth_prefix}┃ "),
                    continuation_prefix: format!("{depth_prefix}┃ "),
                });
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                writer.flush_current_line();
                writer.indent_stack.pop();
                writer.push_blank_line();
            }
            Event::Start(Tag::List(start)) => {
                list_depth += 1;
                list_stack.push(ListContext {
                    ordered: start.is_some(),
                    next_index: start.unwrap_or(1) as usize,
                });
            }
            Event::End(TagEnd::List(_)) => {
                list_depth = list_depth.saturating_sub(1);
                list_stack.pop();
                writer.flush_current_line();
                writer.push_blank_line();
            }
            Event::Start(Tag::Item) => {
                writer.flush_current_line();
                let indent = "  ".repeat(list_depth.saturating_sub(1));
                let marker = if let Some(list) = list_stack.last_mut() {
                    if list.ordered {
                        let index = list.next_index;
                        list.next_index = list.next_index.saturating_add(1);
                        format!("{index}. ")
                    } else {
                        "• ".to_owned()
                    }
                } else {
                    "• ".to_owned()
                };
                let continuation_prefix = " ".repeat(crate::presentation::display_width(marker.as_str()));
                writer.indent_stack.push(IndentContext {
                    prefix: format!("{indent}{marker}"),
                    continuation_prefix: format!("{indent}{continuation_prefix}"),
                });
            }
            Event::Start(Tag::Heading { level, .. }) => {
                writer.flush_current_line();
                let prefix = match level {
                    HeadingLevel::H1 => "# ",
                    HeadingLevel::H2 => "## ",
                    HeadingLevel::H3 => "### ",
                    HeadingLevel::H4 | HeadingLevel::H5 | HeadingLevel::H6 => "#### ",
                };
                writer.append_text(prefix);
            }
            Event::End(TagEnd::Heading(_)) => {
                writer.flush_current_line();
                lines.push(Line::from(""));
            }
            Event::Start(Tag::Strong) | Event::End(TagEnd::Strong) => {}
            Event::Start(Tag::Emphasis) | Event::End(TagEnd::Emphasis) => {}
            Event::Code(text) => {
                if writer.current_link_dest.is_some() {
                    writer.current_link_label.push_str(text.as_ref());
                    continue;
                }
                writer.append_text(text.as_ref());
            }
            Event::Text(text) => {
                if writer.in_image {
                    writer.image_alt.push_str(text.as_ref());
                    continue;
                }
                if writer.current_link_dest.is_some() {
                    writer.current_link_label.push_str(text.as_ref());
                    continue;
                }
                if writer.in_code_block {
                    for line in text.lines() {
                        writer.code_block_lines.push(line.to_owned());
                    }
                } else {
                    writer.append_text(text.as_ref());
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if writer.current_link_dest.is_some() {
                    writer.current_link_label.push(' ');
                    continue;
                }
                if writer.in_code_block {
                    writer.code_block_lines.push(String::new());
                } else {
                    writer.flush_current_line();
                }
            }
            Event::End(TagEnd::Paragraph) => {
                writer.flush_current_line();
                lines.push(Line::from(""));
            }
            Event::End(TagEnd::Item) => {
                writer.flush_current_line();
                writer.indent_stack.pop();
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

    writer.flush_current_line();
    if !writer.lines.is_empty() {
        lines.extend(writer.lines.into_iter().map(Line::from));
    }

    lines
}

pub fn render_markdown_to_strings_with_width(md: &str, width: Option<usize>) -> Vec<String> {
    normalize_blank_string_lines(
        render_markdown_to_lines_with_width(md, width)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>()
        })
        .collect(),
    )
}

pub fn normalize_blank_string_lines(mut lines: Vec<String>) -> Vec<String> {
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }

    let mut normalized = Vec::new();
    let mut last_was_blank = false;
    for line in lines {
        let is_blank = line.trim().is_empty();
        if is_blank && last_was_blank {
            continue;
        }
        last_was_blank = is_blank;
        normalized.push(line);
    }
    normalized
}

fn normalize_markdown_table_cell(cell: &str) -> String {
    cell.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn render_link_spans(dest_url: &str, label: &str) -> Vec<Span<'static>> {
    let trimmed_label = label.trim();
    if let Some(local_target) = render_local_link_target(dest_url) {
        let _ = trimmed_label;
        return vec![Span::styled(local_target, Style::default().fg(SURFACE_ACCENT))];
    }

    let display_label = if trimmed_label.is_empty() {
        dest_url
    } else {
        trimmed_label
    };
    let mut spans = vec![Span::styled(
        display_label.to_owned(),
        Style::default()
            .fg(SURFACE_ACCENT)
            .add_modifier(Modifier::UNDERLINED),
    )];
    if !trimmed_label.is_empty() && trimmed_label != dest_url {
        spans.push(Span::styled(
            format!(" ({dest_url})"),
            Style::default().fg(SURFACE_DIM_GRAY),
        ));
    }
    spans
}

fn render_local_link_target(dest_url: &str) -> Option<String> {
    super::utils::render_local_link_target_text(dest_url)
}


fn render_wrapped_markdown_text(
    prefix: &str,
    continuation_prefix: &str,
    text: &str,
    width: Option<usize>,
) -> Vec<String> {
    let Some(width) = width else {
        return vec![format!("{prefix}{text}")];
    };
    crate::presentation::render_wrapped_text_line_with_continuation(
        prefix,
        continuation_prefix,
        text,
        width.max(1),
    )
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
    use super::{
        contains_renderable_markdown_structure, render_markdown_to_lines,
        render_markdown_to_lines_with_width,
    };

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
    fn renderable_markdown_structure_detects_headings_lists_and_links() {
        assert!(contains_renderable_markdown_structure("## Heading"));
        assert!(contains_renderable_markdown_structure("- item"));
        assert!(contains_renderable_markdown_structure(
            "[app](file:///Users/chum/project/src/app.rs#L12)"
        ));
        assert!(contains_renderable_markdown_structure(
            "```diff\n-old\n+new\n```"
        ));
    }

    #[test]
    fn renderable_markdown_structure_ignores_plain_label_like_text() {
        assert!(!contains_renderable_markdown_structure(
            "source: imported config at ~/.loong/config.toml"
        ));
        assert!(!contains_renderable_markdown_structure(
            "request: still plain prose"
        ));
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

    #[test]
    fn renders_local_file_links_using_target_path_text() {
        let lines = render_markdown_to_lines_with_width(
            "[app](file:///Users/chum/project/src/app.rs#L12)",
            Some(80),
        );
        let joined = lines_to_strings(lines).join("\n");

        assert!(joined.contains("project/src/app.rs"));
        assert!(joined.contains("#L12"));
        assert!(!joined.contains("[app]"));
    }

    #[test]
    fn local_links_under_home_are_shortened_for_display() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/chum".to_owned());
        let lines = render_markdown_to_lines_with_width(
            format!("[cfg](file://{home}/.loong/config.toml)").as_str(),
            Some(80),
        );
        let joined = lines_to_strings(lines).join("\n");

        assert!(joined.contains("~/.loong/config.toml"));
    }

    #[test]
    fn renders_web_links_with_label_and_destination() {
        let lines = render_markdown_to_lines_with_width(
            "[search docs](https://example.com/docs)",
            Some(80),
        );
        let joined = lines_to_strings(lines).join("\n");

        assert!(joined.contains("search docs"));
        assert!(joined.contains("https://example.com/docs"));
    }

    #[test]
    fn wraps_nested_lists_preserving_indent() {
        let lines = render_markdown_to_lines_with_width(
            "- outer item with several words to wrap\n  - inner item that also needs wrapping",
            Some(20),
        );
        let rendered = lines_to_strings(lines);

        assert!(rendered.iter().any(|line| line.contains("• outer item")));
        assert!(rendered.iter().any(|line| line.contains("  • inner item")));
    }

    #[test]
    fn wraps_blockquotes_inside_lists() {
        let lines = render_markdown_to_lines_with_width(
            "- list item\n  > block quote inside list that wraps",
            Some(24),
        );
        let rendered = lines_to_strings(lines).join("\n");

        assert!(rendered.contains("• list item"));
        assert!(rendered.contains("┃"));
        assert!(rendered.contains("block quote inside"));
    }

    #[test]
    fn fenced_code_blocks_preserve_content_without_ellipsis() {
        let lines = render_markdown_to_lines_with_width(
            "```rust\nfn main() { println!(\"hi from a long line\"); }\n```",
            Some(12),
        );
        let rendered = lines_to_strings(lines).join("\n");

        assert!(rendered.contains("```rust"));
        assert!(rendered.contains("println!(\"hi from a long line\")"));
        assert!(!rendered.contains('…'));
    }

    #[test]
    fn renders_relative_local_links_as_target_text() {
        let lines = render_markdown_to_lines_with_width(
            "[local](./src/main.rs:12)",
            Some(80),
        );
        let joined = lines_to_strings(lines).join("\n");

        assert!(joined.contains("./src/main.rs:12"));
        assert!(!joined.contains("[local]"));
    }

    #[test]
    fn soft_break_inside_link_stays_inline() {
        let lines = render_markdown_to_lines_with_width(
            "[docs\nlink](https://example.com/docs)",
            Some(80),
        );
        let joined = lines_to_strings(lines).join("\n");

        assert!(joined.contains("docs link"));
        assert!(!joined.contains("docs\nlink"));
    }

    #[test]
    fn wraps_ordered_lists_preserving_numeric_marker() {
        let lines = render_markdown_to_lines_with_width(
            "1. ordered item contains many words for wrapping",
            Some(18),
        );
        let joined = lines_to_strings(lines).join("\n");

        assert!(joined.contains("1. ordered item"));
        assert!(joined.contains("contains many") || joined.contains("words for"));
    }

    #[test]
    fn local_file_links_preserve_hash_location_suffix() {
        let lines = render_markdown_to_lines_with_width(
            "[app](file:///Users/chum/project/src/app.rs#L12)",
            Some(80),
        );
        let joined = lines_to_strings(lines).join("\n");

        assert!(joined.contains("#L12"));
    }
}
