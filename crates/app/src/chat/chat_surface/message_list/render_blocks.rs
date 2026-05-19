fn render_error_block_lines(
    title: &str,
    summary: &str,
    details: &[String],
    width: u16,
) -> Vec<Line<'static>> {
    let title_label = format!("[{title}]");
    let summary = summary.trim();
    let detail_segments = details
        .iter()
        .map(|detail| detail.trim())
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    let mut rendered = Vec::new();

    rendered.push(Line::from(""));

    let inline_width = crate::presentation::display_width(&title_label)
        + if summary.is_empty() {
            0
        } else {
            1 + crate::presentation::display_width(summary)
        };
    if inline_width <= width as usize {
        let mut spans = vec![Span::styled(
            title_label,
            Style::default()
                .fg(SURFACE_RED)
                .add_modifier(Modifier::BOLD),
        )];
        if !summary.is_empty() {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                summary.to_owned(),
                Style::default().fg(SURFACE_RED).add_modifier(Modifier::DIM),
            ));
        }
        rendered.push(Line::from(spans));
    } else {
        rendered.push(Line::from(vec![Span::styled(
            title_label,
            Style::default()
                .fg(SURFACE_RED)
                .add_modifier(Modifier::BOLD),
        )]));

        if !summary.is_empty() {
            for wrapped in
                crate::presentation::render_wrapped_plain_display_line(summary, width as usize)
            {
                rendered.push(Line::from(vec![Span::styled(
                    wrapped,
                    Style::default().fg(SURFACE_RED).add_modifier(Modifier::DIM),
                )]));
            }
        }
    }

    if !detail_segments.is_empty() {
        let detail_width = width.saturating_sub(4).max(1) as usize;
        let displayed_detail_count = detail_segments.len().min(PROVIDER_ERROR_MAX_DETAIL_ITEMS);
        for detail in detail_segments.iter().take(PROVIDER_ERROR_MAX_DETAIL_ITEMS) {
            let wrapped_lines =
                crate::presentation::render_wrapped_literal_display_line(detail, detail_width);
            let wrapped_count = wrapped_lines.len();
            for (line_index, wrapped) in wrapped_lines
                .into_iter()
                .take(PROVIDER_ERROR_MAX_WRAPPED_LINES_PER_DETAIL)
                .enumerate()
            {
                let prefix = if line_index == 0 { "  ↳ " } else { "    " };
                rendered.push(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(
                        wrapped,
                        Style::default().fg(SURFACE_RED).add_modifier(Modifier::DIM),
                    ),
                ]));
            }
            if wrapped_count > PROVIDER_ERROR_MAX_WRAPPED_LINES_PER_DETAIL {
                rendered.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        "…",
                        Style::default().fg(SURFACE_RED).add_modifier(Modifier::DIM),
                    ),
                ]));
            }
        }
        if detail_segments.len() > displayed_detail_count {
            rendered.push(Line::from(vec![
                Span::raw("  ↳ "),
                Span::styled(
                    format!(
                        "… +{} more details",
                        detail_segments.len() - displayed_detail_count
                    ),
                    Style::default().fg(SURFACE_RED).add_modifier(Modifier::DIM),
                ),
            ]));
        }
    }

    rendered.push(Line::from(""));
    rendered
}

fn render_compaction_block_lines(
    turn_count: usize,
    summary: &str,
    expanded: bool,
    width: u16,
) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    rendered.push(background_line(width, SURFACE_COMPACTION_BG));
    rendered.push(styled_background_line(
        vec![
            Span::raw(" "),
            Span::styled(
                "[compaction]",
                Style::default()
                    .fg(LOONG_COMPACTION_TAG)
                    .add_modifier(Modifier::BOLD),
            ),
        ],
        width,
        SURFACE_COMPACTION_BG,
    ));
    if expanded {
        rendered.push(styled_background_line(
            vec![
                Span::raw("  "),
                Span::styled(
                    format!("Compacted from {turn_count} earlier turns"),
                    Style::default().fg(SURFACE_GRAY),
                ),
            ],
            width,
            SURFACE_COMPACTION_BG,
        ));
        for line in summary.lines() {
            rendered.push(styled_background_line(
                vec![
                    Span::raw("  "),
                    Span::styled(line.to_owned(), Style::default().fg(SURFACE_GRAY)),
                ],
                width,
                SURFACE_COMPACTION_BG,
            ));
        }
    } else {
        rendered.push(styled_background_line(
            vec![
                Span::raw("  "),
                Span::styled(
                    format!("Compacted from {turn_count} earlier turns (Ctrl+O to expand)"),
                    Style::default().fg(SURFACE_GRAY),
                ),
            ],
            width,
            SURFACE_COMPACTION_BG,
        ));
    }
    rendered.push(background_line(width, SURFACE_COMPACTION_BG));
    rendered
}

fn render_diff_block_lines(title: Option<&str>, diff: &str, width: u16) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    rendered.push(background_line(width, SURFACE_TOOL_BG));
    rendered.push(styled_background_line(
        vec![
            Span::raw(" "),
            Span::styled(
                format!("[{}]", title.unwrap_or("diff")),
                Style::default()
                    .fg(SURFACE_CYAN)
                    .add_modifier(Modifier::BOLD),
            ),
        ],
        width,
        SURFACE_TOOL_BG,
    ));
    for line in render_diff_to_lines(diff) {
        let mut line = line;
        pad_and_bg(&mut line, width, SURFACE_TOOL_BG);
        rendered.push(line);
    }
    rendered.push(background_line(width, SURFACE_TOOL_BG));
    rendered
}

fn render_image_block_lines(alt: &str, url: &str, width: u16) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    let alt_text = if alt.trim().is_empty() {
        "image".to_owned()
    } else {
        alt.trim().to_owned()
    };
    let source = url.trim();
    let content_width = width.saturating_sub(10).max(1) as usize;

    for (index, wrapped) in
        crate::presentation::render_wrapped_plain_display_line(alt_text.as_str(), content_width)
            .into_iter()
            .enumerate()
    {
        let mut spans = Vec::new();
        if index == 0 {
            spans.push(Span::styled(
                "[image] ",
                Style::default()
                    .fg(SURFACE_CYAN)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::raw("        "));
        }
        spans.push(Span::styled(wrapped, Style::default().fg(SURFACE_ACCENT)));
        rendered.push(Line::from(spans));
    }

    if !source.is_empty() {
        let source_width = width.saturating_sub(10).max(1) as usize;
        let source_lines =
            crate::presentation::render_wrapped_plain_display_line(source, source_width);
        for (index, wrapped) in source_lines.iter().take(2).enumerate() {
            let label = if index == 0 { "source: " } else { "        " };
            rendered.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(label, Style::default().fg(SURFACE_GRAY)),
                Span::styled(wrapped.clone(), Style::default().fg(SURFACE_DIM_GRAY)),
            ]));
        }
        if source_lines.len() > 2 {
            rendered.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("        …", Style::default().fg(SURFACE_DIM_GRAY)),
            ]));
        }

        if let Some(path) = resolve_local_renderable_image_path(source) {
            rendered.extend(render_local_image_preview_lines(
                path.as_path(),
                image_mime_from_path(path.as_path()),
                width,
                Color::Reset,
            ));
        }
    }

    let action_text = if source.is_empty() {
        "media card"
    } else {
        "open source · copy url"
    };
    let action_width = width.saturating_sub(11).max(1) as usize;
    for (index, wrapped) in
        crate::presentation::render_wrapped_plain_display_line(action_text, action_width)
            .into_iter()
            .enumerate()
    {
        let label = if index == 0 { "actions: " } else { "         " };
        rendered.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(label, Style::default().fg(SURFACE_GRAY)),
            Span::styled(wrapped, Style::default().fg(SURFACE_GRAY)),
        ]));
    }
    for line in &mut rendered {
        let current_width: usize = line.spans.iter().map(|span| span.width()).sum();
        if current_width < width as usize {
            line.spans
                .push(Span::raw(" ".repeat(width as usize - current_width)));
        }
    }
    rendered
}

fn background_line(width: u16, bg: Color) -> Line<'static> {
    let mut line = Line::from(vec![Span::raw(" ".repeat(width as usize))]);
    for span in &mut line.spans {
        span.style = span.style.bg(bg);
    }
    line
}

fn styled_background_line(spans: Vec<Span<'static>>, width: u16, bg: Color) -> Line<'static> {
    let mut line = Line::from(spans);
    pad_and_bg(&mut line, width, bg);
    line
}

