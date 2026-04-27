use super::*;

pub(super) fn render_surface_to_string(
    state: &SurfaceState,
    render_data: &SurfaceRenderData,
    area: Rect,
) -> String {
    if area.width == 0 || area.height == 0 {
        return String::new();
    }

    let mut buffer = Buffer::empty(area);
    let composer_height = u16::try_from(render_data.composer_lines.len().max(2))
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let header_height = u16::try_from(render_data.header_lines.len().max(1))
        .unwrap_or(u16::MAX)
        .saturating_add(3);
    let footer_height = 3;

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height.min(area.height.saturating_sub(footer_height + 4))),
            Constraint::Min(6),
            Constraint::Length(composer_height.min(area.height.saturating_sub(footer_height + 2))),
            Constraint::Length(footer_height),
        ])
        .split(area);
    let header_area = rect_or(layout.as_ref(), 0, area);
    let body_area = rect_or(layout.as_ref(), 1, area);
    let composer_area = rect_or(layout.as_ref(), 2, area);
    let footer_area = rect_or(layout.as_ref(), 3, area);

    render_surface_header(render_data, header_area, &mut buffer);
    render_surface_body(state, render_data, body_area, &mut buffer);
    render_surface_composer(render_data, composer_area, &mut buffer);
    render_surface_footer(render_data, footer_area, &mut buffer);
    render_surface_overlays(state, body_area, &mut buffer);

    render_buffer_to_string(&buffer)
}

fn render_surface_header(render_data: &SurfaceRenderData, area: Rect, buffer: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" loong / chat ");
    let inner = block.inner(area);
    block.render(area, buffer);
    if inner.height == 0 {
        return;
    }

    let status_height = 1;
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(status_height)])
        .split(inner);
    let brand_area = rect_or(layout.as_ref(), 0, inner);
    let status_area = rect_or(layout.as_ref(), 1, inner);

    Paragraph::new(text_from_lines(&render_data.header_lines))
        .wrap(Wrap { trim: false })
        .render(brand_area, buffer);
    Paragraph::new(render_data.header_status_line.clone()).render(status_area, buffer);
}

fn render_surface_body(
    state: &SurfaceState,
    render_data: &SurfaceRenderData,
    area: Rect,
    buffer: &mut Buffer,
) {
    if render_data.sidebar_visible {
        let layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(40),
                Constraint::Length(SIDEBAR_WIDTH as u16),
            ])
            .split(area);
        let transcript_area = rect_or(layout.as_ref(), 0, area);
        let sidebar_area = rect_or(layout.as_ref(), 1, area);
        render_transcript_panel(state, render_data, transcript_area, buffer);
        render_sidebar_panel(render_data, sidebar_area, buffer);
    } else {
        render_transcript_panel(state, render_data, area, buffer);
    }
}

fn render_transcript_panel(
    state: &SurfaceState,
    render_data: &SurfaceRenderData,
    area: Rect,
    buffer: &mut Buffer,
) {
    let title = if state.pending_turn {
        " transcript · live turn "
    } else {
        " transcript "
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    block.render(area, buffer);
    Paragraph::new(text_from_lines(&render_data.transcript_lines))
        .wrap(Wrap { trim: false })
        .render(inner, buffer);
}

fn render_sidebar_panel(render_data: &SurfaceRenderData, area: Rect, buffer: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" control deck ");
    let inner = block.inner(area);
    block.render(area, buffer);
    if inner.height == 0 {
        return;
    }
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(inner);
    let tabs_area = rect_or(layout.as_ref(), 0, inner);
    let body_area = rect_or(layout.as_ref(), 1, inner);

    let tab_titles = [
        SidebarTab::Session,
        SidebarTab::Runtime,
        SidebarTab::Tools,
        SidebarTab::Mission,
        SidebarTab::Workers,
        SidebarTab::Review,
        SidebarTab::Help,
    ]
    .into_iter()
    .map(|tab| {
        let label = tab.title();
        if tab == render_data.sidebar_tab {
            Line::from(format!("[{label}]"))
        } else {
            Line::from(label)
        }
    })
    .collect::<Vec<_>>();
    Tabs::new(tab_titles).render(tabs_area, buffer);
    Paragraph::new(text_from_lines(&render_data.sidebar_lines))
        .wrap(Wrap { trim: false })
        .render(body_area, buffer);
}

fn render_surface_composer(render_data: &SurfaceRenderData, area: Rect, buffer: &mut Buffer) {
    let block = Block::default().borders(Borders::ALL).title(" compose ");
    let inner = block.inner(area);
    block.render(area, buffer);
    Paragraph::new(text_from_lines(&render_data.composer_lines))
        .wrap(Wrap { trim: false })
        .render(inner, buffer);
}

fn render_surface_footer(render_data: &SurfaceRenderData, area: Rect, buffer: &mut Buffer) {
    let block = Block::default().borders(Borders::TOP).title(" controls ");
    let inner = block.inner(area);
    block.render(area, buffer);
    Paragraph::new(render_data.status_line.clone()).render(inner, buffer);
}

fn render_surface_overlays(state: &SurfaceState, overlay_area: Rect, buffer: &mut Buffer) {
    if let Some(palette) = state.command_palette.as_ref() {
        let items = filtered_command_palette_items(&palette.query)
            .into_iter()
            .enumerate()
            .map(|(index, (label, detail, _))| {
                let marker = if index == palette.selected { ">" } else { " " };
                ListItem::new(format!("{marker} {label} — {detail}"))
            })
            .collect::<Vec<_>>();
        let title = if palette.query.is_empty() {
            " command menu ".to_owned()
        } else {
            format!(" command menu · {} ", palette.query)
        };
        render_overlay_list(
            overlay_area,
            68,
            14,
            title.as_str(),
            if items.is_empty() {
                vec![ListItem::new("no commands match the current query")]
            } else {
                items
            },
            buffer,
        );
    }

    match state.overlay.as_ref() {
        Some(SurfaceOverlay::Welcome { screen }) => {
            render_overlay_paragraph(
                overlay_area,
                92,
                20,
                " welcome ",
                &render_tui_screen_spec(screen, 84, false),
                buffer,
            );
        }
        Some(SurfaceOverlay::MissionControl { lines }) => {
            render_overlay_paragraph(overlay_area, 92, 20, " mission control ", lines, buffer);
        }
        Some(SurfaceOverlay::SessionQueue { selected, items }) => {
            let rendered_items = items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let marker = if index == *selected { ">" } else { " " };
                    ListItem::new(format!("{marker} {}", item.list_line()))
                })
                .collect::<Vec<_>>();
            render_overlay_list(
                overlay_area,
                92,
                18,
                " session queue ",
                rendered_items,
                buffer,
            );
        }
        Some(SurfaceOverlay::SessionDetails { title, lines }) => {
            render_overlay_paragraph(overlay_area, 88, 16, title, lines, buffer);
        }
        Some(SurfaceOverlay::ReviewQueue { selected, items }) => {
            let rendered_items = items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let marker = if index == *selected { ">" } else { " " };
                    ListItem::new(format!("{marker} {}", item.list_line()))
                })
                .collect::<Vec<_>>();
            render_overlay_list(
                overlay_area,
                92,
                18,
                " review queue ",
                rendered_items,
                buffer,
            );
        }
        Some(SurfaceOverlay::ReviewDetails { title, lines }) => {
            render_overlay_paragraph(overlay_area, 88, 16, title, lines, buffer);
        }
        Some(SurfaceOverlay::WorkerQueue { selected, items }) => {
            let rendered_items = items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let marker = if index == *selected { ">" } else { " " };
                    ListItem::new(format!("{marker} {}", item.list_line()))
                })
                .collect::<Vec<_>>();
            render_overlay_list(
                overlay_area,
                92,
                18,
                " worker queue ",
                rendered_items,
                buffer,
            );
        }
        Some(SurfaceOverlay::WorkerDetails { title, lines }) => {
            render_overlay_paragraph(overlay_area, 88, 16, title, lines, buffer);
        }
        Some(SurfaceOverlay::EntryDetails { entry_index }) => {
            if let Some(entry) = state.transcript.get(*entry_index) {
                render_overlay_paragraph(
                    overlay_area,
                    88,
                    18,
                    format!(" entry details · #{} ", entry_index + 1).as_str(),
                    &entry.lines,
                    buffer,
                );
            }
        }
        Some(SurfaceOverlay::Timeline) => {
            let selected = state
                .selected_entry
                .unwrap_or_else(|| state.transcript.len().saturating_sub(1));
            let items = state
                .transcript
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    let prefix = if index == selected { ">" } else { " " };
                    let title = entry
                        .lines
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "(empty entry)".to_owned());
                    ListItem::new(format!("{prefix} {:>3}. {}", index + 1, title))
                })
                .collect::<Vec<_>>();
            render_overlay_list(overlay_area, 72, 18, " timeline ", items, buffer);
        }
        Some(SurfaceOverlay::Help) => {
            render_overlay_paragraph(
                overlay_area,
                88,
                16,
                " help ",
                &ops::render_cli_chat_help_lines_with_width(82),
                buffer,
            );
        }
        Some(SurfaceOverlay::ConfirmExit) => {
            render_overlay_paragraph(
                overlay_area,
                60,
                7,
                " confirm exit ",
                &[
                    "Press Enter to leave the session surface, or Esc to continue.".to_owned(),
                    String::new(),
                    "Enter confirm · Esc cancel".to_owned(),
                ],
                buffer,
            );
        }
        Some(SurfaceOverlay::InputPrompt {
            kind,
            value,
            cursor,
        }) => {
            let title = match kind {
                OverlayInputKind::RenameSession => " rename session ",
                OverlayInputKind::ExportTranscript => " export transcript ",
            };
            let hint = match kind {
                OverlayInputKind::RenameSession => {
                    "Set a local session title for this fullscreen surface."
                }
                OverlayInputKind::ExportTranscript => {
                    "Choose a file path to write the current transcript."
                }
            };
            let lines = vec![
                hint.to_owned(),
                String::new(),
                composer_text_with_cursor(value, *cursor),
                String::new(),
                "Enter save · Esc cancel".to_owned(),
            ];
            render_overlay_paragraph(overlay_area, 72, 9, title, &lines, buffer);
        }
        Some(SurfaceOverlay::ApprovalPrompt { screen }) => {
            render_overlay_paragraph(
                overlay_area,
                88,
                16,
                " approval required ",
                &render_tui_screen_spec(screen, 82, false),
                buffer,
            );
        }
        None => {}
    }
}

fn render_overlay_paragraph(
    area: Rect,
    desired_width: u16,
    desired_height: u16,
    title: &str,
    lines: &[String],
    buffer: &mut Buffer,
) {
    let overlay_area = centered_rect(area, desired_width, desired_height);
    Clear.render(overlay_area, buffer);
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(overlay_area);
    block.render(overlay_area, buffer);
    Paragraph::new(text_from_lines(lines))
        .wrap(Wrap { trim: false })
        .render(inner, buffer);
}

fn render_overlay_list(
    area: Rect,
    desired_width: u16,
    desired_height: u16,
    title: &str,
    items: Vec<ListItem<'static>>,
    buffer: &mut Buffer,
) {
    let overlay_area = centered_rect(area, desired_width, desired_height);
    Clear.render(overlay_area, buffer);
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(overlay_area);
    block.render(overlay_area, buffer);
    List::new(items).render(inner, buffer);
}

fn centered_rect(area: Rect, desired_width: u16, desired_height: u16) -> Rect {
    let width = desired_width.min(area.width.saturating_sub(2)).max(10);
    let height = desired_height.min(area.height.saturating_sub(2)).max(5);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(height) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);
    let vertical_area = rect_or(vertical.as_ref(), 1, area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(area.width.saturating_sub(width) / 2),
            Constraint::Length(width),
            Constraint::Min(0),
        ])
        .split(vertical_area);
    rect_or(horizontal.as_ref(), 1, vertical_area)
}

fn text_from_lines(lines: &[String]) -> Text<'static> {
    Text::from(lines.iter().cloned().map(Line::from).collect::<Vec<_>>())
}

fn rect_or(layout: &[Rect], index: usize, fallback: Rect) -> Rect {
    layout.get(index).copied().unwrap_or(fallback)
}

fn render_buffer_to_string(buffer: &Buffer) -> String {
    let area = buffer.area;
    let mut rendered_lines = Vec::new();
    for y in area.top()..area.bottom() {
        let mut line = String::new();
        for x in area.left()..area.right() {
            line.push_str(buffer[(x, y)].symbol());
        }
        rendered_lines.push(line.trim_end_matches(' ').to_owned());
    }
    rendered_lines.join("\n")
}
