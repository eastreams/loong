use super::*;

pub(super) fn pad_and_clip(line: &str, width: usize) -> String {
    let clipped = clipped_display_line(line, width);
    let clipped_len = clipped.chars().count();
    if clipped_len >= width {
        return clipped;
    }
    format!("{clipped}{}", " ".repeat(width - clipped_len))
}

pub(super) fn clipped_display_line(line: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let char_count = line.chars().count();
    if char_count <= width {
        return line.to_owned();
    }
    let mut result = line
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>();
    result.push('…');
    result
}

pub(super) fn summarize_state_mix<'a>(states: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut ready = 0usize;
    let mut running = 0usize;
    let mut completed = 0usize;
    let mut failed = 0usize;
    let mut timed_out = 0usize;
    let mut other = 0usize;
    let mut seen_any = false;

    for state in states {
        seen_any = true;
        match state {
            "ready" => ready += 1,
            "running" => running += 1,
            "completed" => completed += 1,
            "failed" => failed += 1,
            "timed_out" => timed_out += 1,
            _ => other += 1,
        }
    }

    if !seen_any {
        return None;
    }

    let mut parts = Vec::new();
    if ready > 0 {
        parts.push(format!("ready={ready}"));
    }
    if running > 0 {
        parts.push(format!("running={running}"));
    }
    if completed > 0 {
        parts.push(format!("completed={completed}"));
    }
    if failed > 0 {
        parts.push(format!("failed={failed}"));
    }
    if timed_out > 0 {
        parts.push(format!("timed_out={timed_out}"));
    }
    if other > 0 {
        parts.push(format!("other={other}"));
    }

    Some(parts.join(" · "))
}

pub(super) fn fallback_startup_summary(session_id: &str) -> ops::CliChatStartupSummary {
    ops::CliChatStartupSummary {
        config_path: "-".to_owned(),
        memory_label: "-".to_owned(),
        session_id: session_id.to_owned(),
        context_engine_id: "-".to_owned(),
        context_engine_source: "-".to_owned(),
        compaction_enabled: false,
        compaction_min_messages: None,
        compaction_trigger_estimated_tokens: None,
        compaction_preserve_recent_turns: 0,
        compaction_preserve_recent_estimated_tokens: None,
        compaction_fail_open: false,
        acp_enabled: false,
        dispatch_enabled: false,
        conversation_routing: "-".to_owned(),
        allowed_channels: Vec::new(),
        acp_backend_id: "-".to_owned(),
        acp_backend_source: "-".to_owned(),
        explicit_acp_request: false,
        event_stream_enabled: false,
        bootstrap_mcp_servers: Vec::new(),
        working_directory: None,
    }
}

pub(super) fn session_subtitle(state: &SurfaceState) -> &str {
    state
        .session_title_override
        .as_deref()
        .unwrap_or("operator cockpit")
}

pub(super) fn default_export_path(session_id: &str) -> String {
    let sanitized_session_id = sanitize_session_id_for_export(session_id);
    let file_name = format!("loong-{sanitized_session_id}-transcript.txt");
    let exports_dir = loong_exports_dir();
    let export_path = exports_dir.join(file_name);

    export_path.display().to_string()
}

pub(super) fn sanitize_session_id_for_export(session_id: &str) -> String {
    session_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                return character;
            }

            '_'
        })
        .collect()
}

pub(super) fn loong_exports_dir() -> PathBuf {
    let loong_home = crate::config::default_loong_home();
    loong_home.join("exports")
}

pub(super) fn ensure_parent_directory_exists(path: &Path) -> CliResult<()> {
    let Some(parent_dir) = path.parent() else {
        return Ok(());
    };

    if parent_dir.as_os_str().is_empty() {
        return Ok(());
    }

    std::fs::create_dir_all(parent_dir).map_err(|error| {
        let display_path = parent_dir.display();
        format!("failed to create transcript export directory `{display_path}`: {error}")
    })
}

pub(super) fn format_transcript_export(entries: &[SurfaceEntry]) -> String {
    let mut rendered = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        rendered.push(format!("## Entry {}", index + 1));
        rendered.extend(entry.lines.iter().cloned());
        rendered.push(String::new());
    }
    rendered.join("\n")
}

pub(super) fn current_overlay_label(state: &SurfaceState) -> &'static str {
    match state.overlay.as_ref() {
        Some(SurfaceOverlay::Welcome { .. }) => "welcome",
        Some(SurfaceOverlay::MissionControl { .. }) => "mission",
        Some(SurfaceOverlay::SessionQueue { .. }) => "session-queue",
        Some(SurfaceOverlay::SessionDetails { .. }) => "session-detail",
        Some(SurfaceOverlay::ReviewQueue { .. }) => "review-queue",
        Some(SurfaceOverlay::ReviewDetails { .. }) => "review-detail",
        Some(SurfaceOverlay::WorkerQueue { .. }) => "worker-queue",
        Some(SurfaceOverlay::WorkerDetails { .. }) => "worker-detail",
        Some(SurfaceOverlay::EntryDetails { .. }) => "entry",
        Some(SurfaceOverlay::Timeline) => "timeline",
        Some(SurfaceOverlay::Help) => "help",
        Some(SurfaceOverlay::ConfirmExit) => "confirm-exit",
        Some(SurfaceOverlay::InputPrompt { kind, .. }) => match kind {
            OverlayInputKind::RenameSession => "rename",
            OverlayInputKind::ExportTranscript => "export",
        },
        Some(SurfaceOverlay::ApprovalPrompt { .. }) => "approval",
        None => "none",
    }
}

pub(super) fn composer_display_lines(value: &str, width: usize, max_lines: usize) -> Vec<String> {
    let mut wrapped = crate::presentation::render_wrapped_display_line(value, width);
    if wrapped.is_empty() {
        wrapped.push(String::new());
    }
    if wrapped.len() > max_lines {
        wrapped.truncate(max_lines);
    }
    wrapped
}

pub(super) fn composer_text_with_cursor(value: &str, cursor: usize) -> String {
    let mut rendered = String::new();
    let mut inserted = false;
    for (index, character) in value.chars().enumerate() {
        if index == cursor {
            rendered.push('▏');
            inserted = true;
        }
        rendered.push(character);
    }
    if !inserted {
        rendered.push('▏');
    }
    rendered
}

pub(super) fn insert_char_at_cursor(value: &mut String, cursor: &mut usize, character: char) {
    let mut chars = value.chars().collect::<Vec<_>>();
    let insert_at = min(*cursor, chars.len());
    chars.insert(insert_at, character);
    *value = chars.into_iter().collect();
    *cursor = insert_at.saturating_add(1);
}

pub(super) fn remove_char_before_cursor(value: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let mut chars = value.chars().collect::<Vec<_>>();
    let remove_at = cursor.saturating_sub(1);
    if remove_at < chars.len() {
        chars.remove(remove_at);
        *value = chars.into_iter().collect();
        *cursor = remove_at;
    }
}

pub(super) fn move_cursor_vertically(value: &str, cursor: usize, direction: isize) -> usize {
    let chars = value.chars().collect::<Vec<_>>();
    let cursor = min(cursor, chars.len());
    let mut current_line_start = 0;
    let mut index = 0;
    while index < cursor {
        if chars.get(index).is_some_and(|character| *character == '\n') {
            current_line_start = index.saturating_add(1);
        }
        index = index.saturating_add(1);
    }
    let current_column = cursor.saturating_sub(current_line_start);
    let mut current_line_end = chars.len();
    let mut forward_index = cursor;
    while forward_index < chars.len() {
        if chars
            .get(forward_index)
            .is_some_and(|character| *character == '\n')
        {
            current_line_end = forward_index;
            break;
        }
        forward_index = forward_index.saturating_add(1);
    }

    if direction < 0 {
        if current_line_start == 0 {
            return cursor;
        }
        let prev_line_end = current_line_start.saturating_sub(1);
        let mut prev_line_start = 0;
        let mut reverse_index = 0;
        while reverse_index < prev_line_end {
            if chars
                .get(reverse_index)
                .is_some_and(|character| *character == '\n')
            {
                prev_line_start = reverse_index.saturating_add(1);
            }
            reverse_index = reverse_index.saturating_add(1);
        }
        let prev_len = prev_line_end.saturating_sub(prev_line_start);
        return prev_line_start + min(current_column, prev_len);
    }

    if current_line_end >= chars.len() {
        return cursor;
    }
    let next_line_start = current_line_end.saturating_add(1);
    let mut next_line_end = chars.len();
    let mut next_index = next_line_start;
    while next_index < chars.len() {
        if chars
            .get(next_index)
            .is_some_and(|character| *character == '\n')
        {
            next_line_end = next_index;
            break;
        }
        next_index = next_index.saturating_add(1);
    }
    let next_len = next_line_end.saturating_sub(next_line_start);
    next_line_start + min(current_column, next_len)
}

pub(super) fn slash_command_hint(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if !trimmed.starts_with('/') {
        return None;
    }

    let known = [
        CLI_CHAT_HELP_COMMAND,
        CLI_CHAT_STATUS_COMMAND,
        CLI_CHAT_HISTORY_COMMAND,
        CLI_CHAT_SESSIONS_COMMAND,
        CLI_CHAT_MISSION_COMMAND,
        CLI_CHAT_REVIEW_COMMAND,
        CLI_CHAT_WORKERS_COMMAND,
        CLI_CHAT_COMPACT_COMMAND,
        CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND,
    ];
    let matches = known
        .into_iter()
        .filter(|candidate| candidate.starts_with(trimmed))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Some("unknown slash command".to_owned());
    }

    Some(format!("matches: {}", matches.join(" · ")))
}

pub(super) fn should_continue_multiline(value: &str) -> bool {
    value.ends_with('\\')
}

pub(super) fn should_continue_multiline_at_cursor(value: &str, cursor: usize) -> bool {
    let total_chars = value.chars().count();
    if cursor != total_chars {
        return false;
    }

    should_continue_multiline(value)
}

pub(super) fn flattened_entry_line_ranges(entries: &[SurfaceEntry]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut next_line_index: usize = 0;

    for (entry_index, entry) in entries.iter().enumerate() {
        if entry_index > 0 {
            next_line_index = next_line_index.saturating_add(1);
        }

        let line_count = entry.lines.len().max(1);
        let start_line_index = next_line_index;
        let end_line_index = start_line_index.saturating_add(line_count);
        let entry_range = start_line_index..end_line_index;

        ranges.push(entry_range);
        next_line_index = end_line_index;
    }

    ranges
}

pub(super) fn viewport_start_for_scroll_offset(
    total_lines: usize,
    viewport_height: usize,
    scroll_offset: usize,
) -> usize {
    if total_lines <= viewport_height {
        return 0;
    }

    let max_scroll_offset = total_lines.saturating_sub(viewport_height);
    let clamped_scroll_offset = min(scroll_offset, max_scroll_offset);
    total_lines.saturating_sub(viewport_height.saturating_add(clamped_scroll_offset))
}

pub(super) fn scroll_offset_for_viewport_start(
    total_lines: usize,
    viewport_height: usize,
    viewport_start: usize,
) -> usize {
    if total_lines <= viewport_height {
        return 0;
    }

    let max_viewport_start = total_lines.saturating_sub(viewport_height);
    let clamped_viewport_start = min(viewport_start, max_viewport_start);
    total_lines.saturating_sub(viewport_height.saturating_add(clamped_viewport_start))
}

pub(super) fn align_scroll_offset_to_selected_entry(
    entries: &[SurfaceEntry],
    selected_entry: usize,
    viewport_height: usize,
    scroll_offset: usize,
) -> usize {
    let entry_ranges = flattened_entry_line_ranges(entries);
    let Some(selected_range) = entry_ranges.get(selected_entry) else {
        return scroll_offset;
    };

    let total_lines = entry_ranges.last().map(|range| range.end).unwrap_or(0);

    if total_lines <= viewport_height {
        return 0;
    }

    let viewport_start =
        viewport_start_for_scroll_offset(total_lines, viewport_height, scroll_offset);
    let viewport_end = viewport_start.saturating_add(viewport_height);

    if selected_range.start < viewport_start {
        return scroll_offset_for_viewport_start(
            total_lines,
            viewport_height,
            selected_range.start,
        );
    }

    if selected_range.end > viewport_end {
        let next_viewport_start = selected_range.end.saturating_sub(viewport_height);

        return scroll_offset_for_viewport_start(total_lines, viewport_height, next_viewport_start);
    }

    scroll_offset
}
