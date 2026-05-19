fn render_run_tool_preview_block(preview: &RunToolPreview, width: u16) -> Vec<Line<'static>> {
    let bg = SURFACE_TOOL_BG;
    let mut rendered = Vec::new();
    rendered.push(background_line(width, bg));

    let content_width = width.saturating_sub(8).max(1) as usize;
    let command = truncate_middle_display(preview.command.as_str(), content_width);
    rendered.push(pad_preserving_backgrounds(
        Line::from(vec![
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled(
                "run ",
                Style::default()
                    .fg(SURFACE_DARK_GRAY)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(command, Style::default().fg(SURFACE_DARK_GRAY).bg(bg)),
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled(
                tool_status_label(preview.status),
                Style::default()
                    .fg(tool_status_color(preview.status))
                    .bg(bg),
            ),
        ]),
        width,
        bg,
    ));

    rendered.push(pad_preserving_backgrounds(
        Line::from(vec![
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled("tool: ", Style::default().fg(SURFACE_GRAY).bg(bg)),
            Span::styled(
                preview.tool_name.clone(),
                Style::default().fg(SURFACE_DARK_GRAY).bg(bg),
            ),
        ]),
        width,
        bg,
    ));

    rendered.extend(render_tool_stream_preview_section(
        "stdout",
        &preview.stdout,
        width,
        bg,
    ));
    rendered.extend(render_tool_stream_preview_section(
        "stderr",
        &preview.stderr,
        width,
        bg,
    ));

    if let Some(metrics) = preview.metrics.as_deref() {
        for wrapped in crate::presentation::render_wrapped_plain_display_line(
            metrics,
            width.saturating_sub(12).max(1) as usize,
        ) {
            rendered.push(pad_preserving_backgrounds(
                Line::from(vec![
                    Span::styled("  ", Style::default().bg(bg)),
                    Span::styled("metrics ", Style::default().fg(SURFACE_GRAY).bg(bg)),
                    Span::styled(wrapped, Style::default().fg(SURFACE_DARK_GRAY).bg(bg)),
                ]),
                width,
                bg,
            ));
        }
    }

    rendered.push(background_line(width, bg));
    rendered
}

fn render_inspect_tool_preview_block(
    preview: &InspectToolPreview,
    width: u16,
) -> Vec<Line<'static>> {
    let bg = SURFACE_TOOL_BG;
    let mut rendered = Vec::new();
    rendered.push(background_line(width, bg));

    let content_width = width.saturating_sub(12).max(1) as usize;
    let primary = truncate_middle_display(preview.primary.as_str(), content_width);
    rendered.push(pad_preserving_backgrounds(
        Line::from(vec![
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled(
                format!("{} ", preview.kind),
                Style::default()
                    .fg(SURFACE_DARK_GRAY)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(primary, Style::default().fg(SURFACE_DARK_GRAY).bg(bg)),
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled(
                tool_status_label(preview.status),
                Style::default()
                    .fg(tool_status_color(preview.status))
                    .bg(bg),
            ),
        ]),
        width,
        bg,
    ));

    rendered.push(pad_preserving_backgrounds(
        Line::from(vec![
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled("tool: ", Style::default().fg(SURFACE_GRAY).bg(bg)),
            Span::styled(
                preview.tool_name.clone(),
                Style::default().fg(SURFACE_DARK_GRAY).bg(bg),
            ),
        ]),
        width,
        bg,
    ));

    rendered.extend(render_tool_stream_preview_section(
        "stdout",
        &preview.stdout,
        width,
        bg,
    ));
    rendered.extend(render_tool_stream_preview_section(
        "stderr",
        &preview.stderr,
        width,
        bg,
    ));

    if let Some(metrics) = preview.metrics.as_deref() {
        for wrapped in crate::presentation::render_wrapped_plain_display_line(
            metrics,
            width.saturating_sub(12).max(1) as usize,
        ) {
            rendered.push(pad_preserving_backgrounds(
                Line::from(vec![
                    Span::styled("  ", Style::default().bg(bg)),
                    Span::styled("metrics ", Style::default().fg(SURFACE_GRAY).bg(bg)),
                    Span::styled(wrapped, Style::default().fg(SURFACE_DARK_GRAY).bg(bg)),
                ]),
                width,
                bg,
            ));
        }
    }

    rendered.push(background_line(width, bg));
    rendered
}

fn tool_status_label(status: ToolStatus) -> &'static str {
    match status {
        ToolStatus::Pending => "working",
        ToolStatus::Success => "ok",
        ToolStatus::Error => "failed",
    }
}

fn tool_status_color(status: ToolStatus) -> Color {
    match status {
        ToolStatus::Pending => SURFACE_CYAN,
        ToolStatus::Success => SURFACE_GREEN,
        ToolStatus::Error => SURFACE_RED,
    }
}

fn render_tool_stream_preview_section(
    label: &str,
    preview: &ToolStreamPreview,
    width: u16,
    bg: Color,
) -> Vec<Line<'static>> {
    if preview.lines.is_empty() && preview.omitted_count == 0 {
        return Vec::new();
    }

    let mut rendered = Vec::new();
    let label_style = match label {
        "stderr" => Style::default()
            .fg(SURFACE_RED)
            .bg(bg)
            .add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(SURFACE_GREEN)
            .bg(bg)
            .add_modifier(Modifier::BOLD),
    };
    let body_style = match label {
        "stderr" => Style::default().fg(SURFACE_RED).bg(bg),
        _ => Style::default().fg(SURFACE_DARK_GRAY).bg(bg),
    };
    let label_text = format!("{label} ");
    let body_width = width
        .saturating_sub((4 + crate::presentation::display_width(label_text.as_str())) as u16)
        .max(1) as usize;

    for line in &preview.lines {
        let mut wrapped =
            crate::presentation::render_wrapped_literal_display_line(line.as_str(), body_width);
        if wrapped.is_empty() {
            wrapped.push(String::new());
        }
        for (index, wrapped_line) in wrapped.into_iter().enumerate() {
            let label_span = if index == 0 {
                Span::styled(label_text.clone(), label_style)
            } else {
                Span::styled(
                    " ".repeat(crate::presentation::display_width(label_text.as_str())),
                    label_style,
                )
            };
            rendered.push(pad_preserving_backgrounds(
                Line::from(vec![
                    Span::styled("  ", Style::default().bg(bg)),
                    label_span,
                    Span::styled(wrapped_line, body_style),
                ]),
                width,
                bg,
            ));
        }
    }

    if preview.omitted_count > 0 {
        let overflow_text = if preview.truncated_from_start {
            format!("… +{} earlier lines", preview.omitted_count)
        } else {
            format!("… +{} more lines", preview.omitted_count)
        };
        let overflow_line = pad_preserving_backgrounds(
            Line::from(vec![
                Span::styled("  ", Style::default().bg(bg)),
                Span::styled(label_text, label_style),
                Span::styled(overflow_text, Style::default().fg(SURFACE_GRAY).bg(bg)),
            ]),
            width,
            bg,
        );
        if preview.truncated_from_start {
            rendered.insert(0, overflow_line);
        } else {
            rendered.push(overflow_line);
        }
    }

    rendered
}

fn render_read_tool_preview_block(preview: &ReadToolPreview, width: u16) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    let bg = SURFACE_TOOL_BG;
    rendered.push(background_line(width, bg));

    let path = preview.display_path.clone().or_else(|| {
        preview
            .local_path
            .as_deref()
            .map(|path| path.to_string_lossy().into_owned())
    });
    let path = path
        .as_deref()
        .unwrap_or(if preview.is_image { "image" } else { "file" });
    let content_width = width.saturating_sub(7).max(1) as usize;
    let path_buf = Path::new(path);
    let file_name = path_buf.file_name().and_then(|value| value.to_str());
    let preferred_path = if path_buf.is_absolute() {
        if let Some(file_name) = file_name {
            if crate::presentation::display_width(file_name) < content_width {
                file_name
            } else {
                path
            }
        } else {
            path
        }
    } else {
        path
    };
    let compact_path = truncate_middle_display(preferred_path, content_width);
    rendered.push(pad_preserving_backgrounds(
        Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "read ",
                Style::default()
                    .fg(SURFACE_DARK_GRAY)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(compact_path, Style::default().fg(SURFACE_DARK_GRAY).bg(bg)),
        ]),
        width,
        bg,
    ));

    if let Some(summary) = preview.summary.as_deref() {
        for line in render_read_preview_text_line(summary, width) {
            rendered.push(line);
        }
    } else if let Some(mime) = preview.mime.as_deref() {
        for line in render_read_preview_text_line(&format!("Read image file [{mime}]"), width) {
            rendered.push(line);
        }
    } else if !preview.is_image {
        for line in render_read_preview_text_line("Read file", width) {
            rendered.push(line);
        }
    }

    if preview.is_image
        && let Some(path) = preview.local_path.as_deref()
    {
        rendered.extend(render_local_image_preview_lines(
            path,
            preview.mime.as_deref(),
            width,
            bg,
        ));
    } else if !preview.text_excerpt.is_empty() {
        rendered.extend(render_read_text_excerpt_lines(
            preview.text_excerpt.as_slice(),
            width,
            bg,
        ));
    }

    rendered.push(background_line(width, bg));
    rendered
}

fn render_read_text_excerpt_lines(excerpt: &[String], width: u16, bg: Color) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    rendered.push(pad_preserving_backgrounds(
        Line::from(vec![
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled("preview:", Style::default().fg(SURFACE_GRAY).bg(bg)),
        ]),
        width,
        bg,
    ));

    let content_width = width.saturating_sub(5).max(1) as usize;
    for line in excerpt {
        for (index, wrapped) in
            crate::presentation::render_wrapped_literal_display_line(line.as_str(), content_width)
                .into_iter()
                .enumerate()
        {
            let marker = if index == 0 { "│ " } else { "  " };
            rendered.push(pad_preserving_backgrounds(
                Line::from(vec![
                    Span::styled("  ", Style::default().bg(bg)),
                    Span::styled(marker, Style::default().fg(SURFACE_ACCENT).bg(bg)),
                    Span::styled(wrapped, Style::default().fg(SURFACE_DARK_GRAY).bg(bg)),
                ]),
                width,
                bg,
            ));
        }
    }

    rendered
}

fn render_read_preview_text_line(text: &str, width: u16) -> Vec<Line<'static>> {
    let content_width = width.saturating_sub(2).max(1) as usize;
    crate::presentation::render_wrapped_plain_display_line(text, content_width)
        .into_iter()
        .map(|wrapped| {
            pad_preserving_backgrounds(
                Line::from(vec![
                    Span::styled(" ", Style::default().bg(SURFACE_TOOL_BG)),
                    Span::styled(
                        wrapped,
                        Style::default().fg(SURFACE_GRAY).bg(SURFACE_TOOL_BG),
                    ),
                ]),
                width,
                SURFACE_TOOL_BG,
            )
        })
        .collect()
}

fn render_local_image_preview_lines(
    path: &Path,
    mime: Option<&str>,
    width: u16,
    bg: Color,
) -> Vec<Line<'static>> {
    match load_image_preview(path, mime, width, bg) {
        Ok(lines) => lines,
        Err(error) => vec![pad_preserving_backgrounds(
            Line::from(vec![
                Span::styled(" ", Style::default().bg(bg)),
                Span::styled(
                    format!("preview unavailable: {error}"),
                    Style::default().fg(SURFACE_GRAY).bg(bg),
                ),
            ]),
            width,
            bg,
        )],
    }
}

fn load_image_preview(
    path: &Path,
    mime: Option<&str>,
    width: u16,
    bg: Color,
) -> Result<Vec<Line<'static>>, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if metadata.len() > IMAGE_PREVIEW_MAX_BYTES {
        return Err(format!(
            "image is {} (limit {})",
            format_bytes(metadata.len()),
            format_bytes(IMAGE_PREVIEW_MAX_BYTES)
        ));
    }

    let reader = image::ImageReader::open(path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?
        .with_guessed_format()
        .map_err(|error| format!("cannot detect image format: {error}"))?;
    let image = reader
        .decode()
        .map_err(|error| format!("cannot decode image: {error}"))?;
    let rgba = image.to_rgba8();
    let (source_width, source_height) = rgba.dimensions();
    if source_width == 0 || source_height == 0 {
        return Err("empty image".to_owned());
    }

    let available_columns = u32::from(width)
        .saturating_sub(4)
        .clamp(1, IMAGE_PREVIEW_MAX_COLUMNS);
    let max_pixel_height = IMAGE_PREVIEW_MAX_ROWS.saturating_mul(2).max(2);
    let width_scale = available_columns as f32 / source_width as f32;
    let height_scale = max_pixel_height as f32 / source_height as f32;
    let scale = width_scale.min(height_scale).clamp(0.01, 1.0);
    let target_width =
        ((source_width as f32 * scale).round() as u32).clamp(1, available_columns.max(1));
    let target_height = ((source_height as f32 * scale).round() as u32).clamp(1, max_pixel_height);
    let resized = image::imageops::resize(
        &rgba,
        target_width,
        target_height,
        image::imageops::FilterType::Triangle,
    );

    let mut rendered = Vec::new();
    let mime = mime
        .map(ToOwned::to_owned)
        .or_else(|| image_mime_from_path(path).map(ToOwned::to_owned))
        .unwrap_or_else(|| "image".to_owned());
    let header = format!(
        "preview: {}×{} · {} · {}",
        source_width,
        source_height,
        mime,
        format_bytes(metadata.len())
    );
    rendered.push(pad_preserving_backgrounds(
        Line::from(vec![
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled(header, Style::default().fg(SURFACE_GRAY).bg(bg)),
        ]),
        width,
        bg,
    ));

    let terminal_rows = target_height.div_ceil(2);
    for row in 0..terminal_rows {
        let upper_y = row * 2;
        let lower_y = upper_y + 1;
        let mut spans = vec![Span::styled("  ", Style::default().bg(bg))];
        for x in 0..target_width {
            let upper = rgba_pixel_as_rgb(resized.get_pixel(x, upper_y).0, bg);
            let lower = if lower_y < target_height {
                rgba_pixel_as_rgb(resized.get_pixel(x, lower_y).0, bg)
            } else {
                color_to_rgb(bg)
            };
            spans.push(Span::styled(
                "▀",
                Style::default()
                    .fg(Color::Rgb(upper.0, upper.1, upper.2))
                    .bg(Color::Rgb(lower.0, lower.1, lower.2)),
            ));
        }
        rendered.push(pad_preserving_backgrounds(Line::from(spans), width, bg));
    }

    Ok(rendered)
}

fn image_mime_from_path(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

fn rgba_pixel_as_rgb(pixel: [u8; 4], bg: Color) -> (u8, u8, u8) {
    let (bg_r, bg_g, bg_b) = color_to_rgb(bg);
    let alpha = u16::from(pixel[3]);
    let blend = |foreground: u8, background: u8| -> u8 {
        let foreground = u16::from(foreground);
        let background = u16::from(background);
        ((foreground * alpha + background * (255 - alpha)) / 255) as u8
    };
    (
        blend(pixel[0], bg_r),
        blend(pixel[1], bg_g),
        blend(pixel[2], bg_b),
    )
}

fn color_to_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::Red => (255, 0, 0),
        Color::Green => (0, 255, 0),
        Color::Yellow => (255, 255, 0),
        Color::Blue => (0, 0, 255),
        Color::Magenta => (255, 0, 255),
        Color::Cyan => (0, 255, 255),
        Color::Gray => (128, 128, 128),
        Color::DarkGray => (64, 64, 64),
        Color::LightRed => (255, 128, 128),
        Color::LightGreen => (128, 255, 128),
        Color::LightYellow => (255, 255, 128),
        Color::LightBlue => (128, 128, 255),
        Color::LightMagenta => (255, 128, 255),
        Color::LightCyan => (128, 255, 255),
        Color::White => (255, 255, 255),
        Color::Indexed(_) | Color::Reset => (0, 0, 0),
    }
}

fn pad_preserving_backgrounds(mut line: Line<'static>, width: u16, bg: Color) -> Line<'static> {
    let line_len: usize = line.spans.iter().map(|span| span.width()).sum();
    let pad_len = (width as usize).saturating_sub(line_len);
    if pad_len > 0 {
        line.spans
            .push(Span::styled(" ".repeat(pad_len), Style::default().bg(bg)));
    }
    line
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / KB)
    } else {
        format!("{:.1} MB", bytes as f64 / MB)
    }
}

fn truncate_middle_display(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if crate::presentation::display_width(text) <= width {
        return text.to_owned();
    }
    if width == 1 {
        return "…".to_owned();
    }

    let prefix_target = width.saturating_sub(1) / 2;
    let suffix_target = width.saturating_sub(1).saturating_sub(prefix_target);
    let mut prefix = String::new();
    let mut prefix_width = 0usize;
    for ch in text.chars() {
        let ch_width = crate::presentation::char_display_width(ch);
        if prefix_width + ch_width > prefix_target {
            break;
        }
        prefix.push(ch);
        prefix_width += ch_width;
    }

    let mut suffix_chars = Vec::new();
    let mut suffix_width = 0usize;
    for ch in text.chars().rev() {
        let ch_width = crate::presentation::char_display_width(ch);
        if suffix_width + ch_width > suffix_target {
            break;
        }
        suffix_chars.push(ch);
        suffix_width += ch_width;
    }
    suffix_chars.reverse();
    let suffix = suffix_chars.into_iter().collect::<String>();
    format!("{prefix}…{suffix}")
}

fn dedupe_tool_activity_detail_lines(lines: &[String]) -> Vec<String> {
    let mut deduped = Vec::with_capacity(lines.len());
    let mut seen_structured_previews = std::collections::BTreeSet::new();
    let mut last_dedupe_key: Option<String> = None;
    for line in lines {
        let dedupe_key = tool_activity_dedupe_key(line);
        if last_dedupe_key.as_deref() == Some(dedupe_key.as_str()) {
            continue;
        }

        if tool_activity_line_starts_new_group(line) {
            seen_structured_previews.clear();
        }

        if let Some(preview) =
            compact_tool_request_preview(line).or_else(|| compact_tool_args_preview(line))
            && !seen_structured_previews.insert(preview)
        {
            continue;
        }

        deduped.push(line.clone());
        last_dedupe_key = Some(dedupe_key);
    }

    deduped
}

fn tool_activity_line_starts_new_group(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('[')
        || trimmed.starts_with("• Called ")
        || trimmed.starts_with("• Closed ")
        || trimmed.starts_with("Called ")
        || trimmed.starts_with("Closed ")
        || trimmed.starts_with("Approval ")
        || trimmed.starts_with("Denied ")
}

fn tool_activity_dedupe_key(line: &str) -> String {
    if let Some(preview) = compact_tool_request_preview(line) {
        return format!("request:{preview}");
    }
    if let Some(preview) = compact_tool_args_preview(line) {
        return format!("args:{preview}");
    }
    if let Some(preview) = compact_tool_child_preview(line, "stdout") {
        return format!("stdout:{preview}");
    }
    if let Some(preview) = compact_tool_child_preview(line, "stderr") {
        return format!("stderr:{preview}");
    }
    if let Some(preview) = compact_tool_child_preview(line, "file") {
        return format!("file:{preview}");
    }
    if let Some(preview) = compact_tool_child_preview(line, "metrics") {
        return format!("metrics:{preview}");
    }
    if let Some((label, body)) = normalized_activity_headline(line) {
        return format!("status:{label}:{body}");
    }

    line.trim().to_owned()
}

fn compact_tool_child_preview(line: &str, label: &str) -> Option<String> {
    let trimmed = line.trim();
    let body = trimmed
        .strip_prefix(&format!("{label}:"))
        .or_else(|| trimmed.strip_prefix(&format!("{label} ")))
        .or_else(|| trimmed.strip_prefix(&format!("↳ {label} ")))?
        .trim_start();
    Some(body.to_owned())
}

fn normalized_activity_headline(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim().strip_prefix("• ").unwrap_or(line.trim());

    let (label, rest) = if let Some(status) = trimmed.strip_prefix('[') {
        let (status, rest) = status.split_once("] ")?;
        let label = match status {
            "running" | "pending" => "Called",
            "completed" | "failed" | "interrupted" => "Closed",
            "needs_approval" => "Approval",
            "denied" => "Denied",
            _ => return None,
        };
        (label, rest)
    } else if let Some(rest) = trimmed.strip_prefix("Called ") {
        ("Called", rest)
    } else if let Some(rest) = trimmed.strip_prefix("Closed ") {
        ("Closed", rest)
    } else if let Some(rest) = trimmed.strip_prefix("Approval ") {
        ("Approval", rest)
    } else if let Some(rest) = trimmed.strip_prefix("Denied ") {
        ("Denied", rest)
    } else {
        return None;
    };

    let (body, detail) = normalize_activity_target_and_detail(rest);
    let body = if let Some(detail) = detail {
        format!("{body} · {detail}")
    } else {
        body
    };

    Some((label.to_owned(), body))
}

fn compact_tool_request_preview(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let body = trimmed
        .strip_prefix("request:")
        .or_else(|| trimmed.strip_prefix("request "))?
        .trim_start();
    compact_structured_preview(body, 3)
}

fn compact_tool_args_preview(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let body = trimmed
        .strip_prefix("args:")
        .or_else(|| trimmed.strip_prefix("args "))
        .or_else(|| trimmed.strip_prefix("↳ args "))?
        .trim_start();
    compact_structured_preview(body, 3)
}

fn render_tool_detail_lines(line: &str, width: u16) -> Vec<Line<'static>> {
    let content_width = width.saturating_sub(2).max(1) as usize;
    let trimmed = line.trim_start();

    if let Some(rendered) = render_status_activity_line(line, content_width) {
        return rendered;
    }

    if let Some(rendered) = render_named_activity_line(line, content_width) {
        return rendered;
    }

    if let Some(body) = trimmed.strip_prefix("↳ ") {
        let prefix = "↳ ";
        let body = if let Some(args) = body.strip_prefix("args ") {
            let compacted = compact_structured_preview(args, 3).unwrap_or_else(|| args.to_owned());
            format!("args {compacted}")
        } else if let Some(request) = body.strip_prefix("request ") {
            let compacted =
                compact_structured_preview(request, 3).unwrap_or_else(|| request.to_owned());
            format!("request {compacted}")
        } else {
            body.to_owned()
        };
        let (label, body) = body
            .split_once(' ')
            .map(|(label, body)| (label, body.trim_start()))
            .unwrap_or((body.as_str(), ""));
        let label_text = if body.is_empty() {
            String::new()
        } else {
            format!("{label} ")
        };
        let (label_style, body_style) = tool_child_styles(label);
        let body_width = content_width
            .saturating_sub(
                crate::presentation::display_width(prefix)
                    + crate::presentation::display_width(label_text.as_str()),
            )
            .max(1);
        let mut wrapped = crate::presentation::render_wrapped_plain_display_line(body, body_width);
        if wrapped.is_empty() {
            wrapped.push(String::new());
        }
        return wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                if index == 0 {
                    let mut spans = vec![
                        Span::raw("  "),
                        Span::styled(prefix, Style::default().fg(SURFACE_ACCENT)),
                    ];
                    if !label_text.is_empty() {
                        spans.push(Span::styled(label_text.clone(), label_style));
                    }
                    spans.push(Span::styled(wrapped_line, body_style));
                    Line::from(spans)
                } else {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            " ".repeat(
                                crate::presentation::display_width(prefix)
                                    + crate::presentation::display_width(label_text.as_str()),
                            ),
                            Style::default().fg(SURFACE_ACCENT),
                        ),
                        Span::styled(wrapped_line, body_style),
                    ])
                }
            })
            .collect();
    }

    if let Some(request) = trimmed.strip_prefix("request:") {
        return render_tool_detail_lines(&format!("↳ request {}", request.trim_start()), width);
    }

    if let Some(request) = trimmed.strip_prefix("request ") {
        return render_tool_detail_lines(&format!("↳ request {}", request.trim_start()), width);
    }

    if let Some(args) = trimmed.strip_prefix("args:") {
        return render_tool_detail_lines(&format!("↳ args {}", args.trim_start()), width);
    }

    if let Some(args) = trimmed.strip_prefix("args ") {
        return render_tool_detail_lines(&format!("↳ args {}", args.trim_start()), width);
    }

    if let Some(stdout) = trimmed.strip_prefix("stdout:") {
        return render_tool_detail_lines(&format!("↳ stdout {}", stdout.trim_start()), width);
    }

    if let Some(stderr) = trimmed.strip_prefix("stderr:") {
        return render_tool_detail_lines(&format!("↳ stderr {}", stderr.trim_start()), width);
    }

    if let Some(file) = trimmed.strip_prefix("file:") {
        return render_tool_detail_lines(&format!("↳ file {}", file.trim_start()), width);
    }

    if let Some(metrics) = trimmed.strip_prefix("metrics:") {
        return render_tool_detail_lines(&format!("↳ metrics {}", metrics.trim_start()), width);
    }

    if let Some(rendered) = render_tool_sample_detail_lines(line, content_width) {
        return rendered;
    }

    if let Some((prefix, body)) = line.split_once(':') {
        let prefix = format!("{prefix}: ");
        let (prefix_style, body_style) = match prefix.trim_end() {
            "stdout:" => (
                Style::default()
                    .fg(SURFACE_GREEN)
                    .add_modifier(Modifier::BOLD),
                Style::default().fg(SURFACE_DARK_GRAY),
            ),
            "stderr:" => (
                Style::default()
                    .fg(SURFACE_RED)
                    .add_modifier(Modifier::BOLD),
                Style::default().fg(SURFACE_DARK_GRAY),
            ),
            _ => (
                Style::default().fg(SURFACE_GRAY),
                Style::default().fg(SURFACE_DARK_GRAY),
            ),
        };
        let body_width = content_width
            .saturating_sub(crate::presentation::display_width(&prefix))
            .max(1);
        let wrapped =
            crate::presentation::render_wrapped_plain_display_line(body.trim_start(), body_width);
        let continuation_prefix = " ".repeat(crate::presentation::display_width(&prefix));
        return wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                let display_prefix = if index == 0 {
                    prefix.clone()
                } else {
                    continuation_prefix.clone()
                };
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(display_prefix, prefix_style),
                    Span::styled(wrapped_line, body_style),
                ])
            })
            .collect();
    }

    crate::presentation::render_wrapped_plain_display_line(line, content_width)
        .into_iter()
        .map(|wrapped_line| {
            Line::from(vec![
                Span::raw("  "),
                Span::styled(wrapped_line, Style::default().fg(SURFACE_DARK_GRAY)),
            ])
        })
        .collect()
}

fn tool_child_styles(label: &str) -> (Style, Style) {
    match label {
        "stdout" => (
            Style::default()
                .fg(SURFACE_GREEN)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        "stderr" => (
            Style::default()
                .fg(SURFACE_RED)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_RED).add_modifier(Modifier::DIM),
        ),
        "file" => (
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        "metrics" => (
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        "request" | "args" => (
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        _ => (
            Style::default().fg(SURFACE_ACCENT),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
    }
}

fn render_tool_sample_detail_lines(line: &str, content_width: usize) -> Option<Vec<Line<'static>>> {
    if !line.starts_with("    ") {
        return None;
    }

    let sample = line.trim_start();
    if sample.is_empty() {
        return None;
    }

    let sample_style = if sample.starts_with('+') {
        Style::default().fg(SURFACE_GREEN)
    } else if sample.starts_with('-') {
        Style::default().fg(SURFACE_RED)
    } else {
        Style::default().fg(SURFACE_DARK_GRAY)
    };
    let sample_width = content_width.saturating_sub(4).max(1);

    Some(
        crate::presentation::render_wrapped_literal_display_line(sample, sample_width)
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                let guide = if index == 0 { "    " } else { "      " };
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(guide, Style::default().fg(SURFACE_DARK_GRAY)),
                    Span::styled(wrapped_line, sample_style),
                ])
            })
            .collect(),
    )
}

fn render_named_activity_line(line: &str, content_width: usize) -> Option<Vec<Line<'static>>> {
    let trimmed = line.trim().strip_prefix("• ").unwrap_or(line.trim());

    let (headline_label, headline_style, rest) = if let Some(rest) = trimmed.strip_prefix("Called ")
    {
        (
            "Called",
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD),
            rest,
        )
    } else if let Some(rest) = trimmed.strip_prefix("Closed ") {
        (
            "Closed",
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::BOLD),
            rest,
        )
    } else if let Some(rest) = trimmed.strip_prefix("Approval ") {
        (
            "Approval",
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
            rest,
        )
    } else if let Some(rest) = trimmed.strip_prefix("Denied ") {
        (
            "Denied",
            Style::default()
                .fg(SURFACE_RED)
                .add_modifier(Modifier::BOLD),
            rest,
        )
    } else {
        return None;
    };

    let display_body = if rest.contains(" (id=") || rest.contains(" - ") {
        let (headline_body, detail_suffix) = normalize_activity_target_and_detail(rest);
        if let Some(detail_suffix) = detail_suffix {
            format!("{headline_body} · {detail_suffix}")
        } else {
            headline_body
        }
    } else {
        rest.to_owned()
    };

    let body_width = content_width
        .saturating_sub(crate::presentation::display_width(headline_label) + 3)
        .max(1);
    let wrapped =
        crate::presentation::render_wrapped_literal_display_line(&display_body, body_width);

    Some(
        wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                if index == 0 {
                    Line::from(vec![
                        Span::styled("• ", Style::default().fg(SURFACE_GRAY)),
                        Span::styled(format!("{headline_label} "), headline_style),
                        Span::styled(wrapped_line, Style::default().fg(SURFACE_DARK_GRAY)),
                    ])
                } else {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            " ".repeat(crate::presentation::display_width(headline_label) + 1),
                            headline_style,
                        ),
                        Span::styled(wrapped_line, Style::default().fg(SURFACE_DARK_GRAY)),
                    ])
                }
            })
            .collect(),
    )
}

fn render_status_activity_line(line: &str, content_width: usize) -> Option<Vec<Line<'static>>> {
    let trimmed = line.trim();
    let status = trimmed.strip_prefix('[')?;
    let (status, rest) = status.split_once("] ")?;

    let (headline_label, headline_style) = match status {
        "running" | "pending" => (
            "Called",
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        "completed" | "failed" | "interrupted" => (
            "Closed",
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::BOLD),
        ),
        "needs_approval" => (
            "Approval",
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        "denied" => (
            "Denied",
            Style::default()
                .fg(SURFACE_RED)
                .add_modifier(Modifier::BOLD),
        ),
        _ => return None,
    };

    let (headline_body, detail_suffix) = normalize_activity_target_and_detail(rest);
    let mut display_body = headline_body;
    if let Some(detail_suffix) = detail_suffix {
        display_body.push_str(" · ");
        display_body.push_str(detail_suffix.as_str());
    }

    let body_width = content_width
        .saturating_sub(crate::presentation::display_width(headline_label) + 3)
        .max(1);
    let wrapped =
        crate::presentation::render_wrapped_literal_display_line(&display_body, body_width);

    Some(
        wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                if index == 0 {
                    Line::from(vec![
                        Span::styled("• ", Style::default().fg(SURFACE_GRAY)),
                        Span::styled(format!("{headline_label} "), headline_style),
                        Span::styled(wrapped_line, Style::default().fg(SURFACE_DARK_GRAY)),
                    ])
                } else {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            " ".repeat(crate::presentation::display_width(headline_label) + 1),
                            headline_style,
                        ),
                        Span::styled(wrapped_line, Style::default().fg(SURFACE_DARK_GRAY)),
                    ])
                }
            })
            .collect(),
    )
}

fn normalize_activity_target_and_detail(rest: &str) -> (String, Option<String>) {
    let (target_with_id, detail_suffix) = rest
        .split_once(" - ")
        .map(|(target, detail)| (target.trim(), Some(detail.trim().to_owned())))
        .unwrap_or((rest.trim(), None));

    let target = if let Some(id_index) = target_with_id.find(" (id=") {
        target_with_id[..id_index].trim().to_owned()
    } else {
        target_with_id.to_owned()
    };

    (target, detail_suffix.filter(|detail| !detail.is_empty()))
}

