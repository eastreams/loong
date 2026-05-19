fn render_tool_block_lines(
    _title: &str,
    lines: &[String],
    status: ToolStatus,
    width: u16,
) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    let lines = dedupe_tool_activity_detail_lines(lines);
    if let Some(read_preview) = read_tool_preview_from_lines(&lines) {
        return render_read_tool_preview_block(&read_preview, width);
    }
    if let Some(run_preview) = run_tool_preview_from_lines(&lines, status) {
        return render_run_tool_preview_block(&run_preview, width);
    }
    if let Some(inspect_preview) = inspect_tool_preview_from_lines(&lines, status) {
        return render_inspect_tool_preview_block(&inspect_preview, width);
    }
    rendered.push(Line::from(""));
    for line in &lines {
        rendered.extend(render_tool_detail_lines(line, width));
    }
    rendered.push(Line::from(""));
    rendered
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadToolPreview {
    display_path: Option<String>,
    local_path: Option<PathBuf>,
    mime: Option<String>,
    summary: Option<String>,
    is_image: bool,
    text_excerpt: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadToolRequest {
    path: String,
    offset: Option<u64>,
    limit: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunToolPreview {
    tool_name: String,
    command: String,
    status: ToolStatus,
    stdout: ToolStreamPreview,
    stderr: ToolStreamPreview,
    metrics: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InspectToolPreview {
    kind: &'static str,
    tool_name: String,
    primary: String,
    status: ToolStatus,
    stdout: ToolStreamPreview,
    stderr: ToolStreamPreview,
    metrics: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ToolStreamPreview {
    lines: Vec<String>,
    omitted_count: usize,
    truncated_from_start: bool,
}

fn read_tool_preview_from_lines(lines: &[String]) -> Option<ReadToolPreview> {
    let read_tool_name = lines
        .iter()
        .filter_map(|line| activity_tool_name(line))
        .find(|name| is_read_activity_tool_name(name))
        .map(normalized_activity_tool_name);
    let has_read_tool = read_tool_name.is_some();
    let summary = lines
        .iter()
        .find_map(|line| extract_read_image_summary(line));
    let mime = summary
        .as_deref()
        .and_then(extract_image_mime)
        .or_else(|| lines.iter().find_map(|line| extract_image_mime(line)));
    let request = lines
        .iter()
        .find_map(|line| extract_read_tool_request(line));
    let source_path = request
        .as_ref()
        .map(|request| request.path.clone())
        .or_else(|| lines.iter().find_map(|line| extract_tool_path(line)));
    let display_path = request
        .as_ref()
        .map(format_read_request_display)
        .or_else(|| source_path.as_deref().map(shorten_display_path));
    let local_path = source_path
        .as_deref()
        .and_then(resolve_local_renderable_image_path);
    let path_looks_like_image = local_path.as_deref().is_some_and(path_has_image_extension);
    let output_is_image = mime
        .as_deref()
        .is_some_and(|mime| mime.starts_with("image/"));

    if !(has_read_tool || output_is_image || path_looks_like_image) {
        return None;
    }
    let is_image = output_is_image || path_looks_like_image;
    if !is_image && display_path.is_none() {
        return None;
    }

    Some(ReadToolPreview {
        display_path,
        local_path,
        mime,
        summary,
        is_image,
        text_excerpt: if is_image {
            Vec::new()
        } else {
            extract_read_text_excerpt(lines)
        },
    })
}

fn run_tool_preview_from_lines(lines: &[String], status: ToolStatus) -> Option<RunToolPreview> {
    let tool_name = lines
        .iter()
        .filter_map(|line| activity_tool_name(line))
        .find_map(|name| {
            let normalized = normalized_activity_tool_name(name);
            is_run_activity_tool_name(normalized.as_str()).then_some(normalized)
        })?;
    let command = lines.iter().find_map(|line| extract_tool_command(line))?;
    let stdout = extract_tool_stream_tail_preview(lines, "stdout");
    let stderr = extract_tool_stream_tail_preview(lines, "stderr");
    let metrics = lines
        .iter()
        .find_map(|line| extract_tool_metrics_line(line.as_str()));

    Some(RunToolPreview {
        tool_name,
        command,
        status,
        stdout,
        stderr,
        metrics,
    })
}

fn inspect_tool_preview_from_lines(
    lines: &[String],
    status: ToolStatus,
) -> Option<InspectToolPreview> {
    let tool_name = lines
        .iter()
        .filter_map(|line| activity_tool_name(line))
        .find(|name| {
            is_search_activity_tool_name(name)
                || is_list_activity_tool_name(name)
                || is_glob_activity_tool_name(name)
        })
        .map(|name| {
            name.trim_matches(|ch: char| ch == '`' || ch == '"' || ch == '\'')
                .to_owned()
        })?;

    let (kind, primary) = if is_search_activity_tool_name(tool_name.as_str()) {
        ("search", extract_search_tool_summary(lines)?)
    } else if is_list_activity_tool_name(tool_name.as_str()) {
        ("list", extract_list_tool_summary(lines)?)
    } else if is_glob_activity_tool_name(tool_name.as_str()) {
        ("glob", extract_glob_tool_summary(lines)?)
    } else {
        return None;
    };

    let stdout = extract_tool_stream_preview(lines, "stdout");
    let stderr = extract_tool_stream_preview(lines, "stderr");
    let metrics = lines
        .iter()
        .find_map(|line| extract_tool_metrics_line(line.as_str()));

    Some(InspectToolPreview {
        kind,
        tool_name,
        primary,
        status,
        stdout,
        stderr,
        metrics,
    })
}

fn normalized_activity_tool_name(name: &str) -> String {
    name.trim_matches(|ch: char| ch == '`' || ch == '"' || ch == '\'')
        .rsplit(['.', '/', ':'])
        .next()
        .unwrap_or(name)
        .to_owned()
}

fn is_run_activity_tool_name(name: &str) -> bool {
    matches!(
        name,
        "bash" | "shell" | "sh" | "exec_command" | "run_command" | "terminal" | "cmd"
    )
}

fn is_search_activity_tool_name(name: &str) -> bool {
    matches!(
        name,
        "search" | "grep" | "ripgrep" | "rg" | "find" | "find_text"
    )
}

fn is_list_activity_tool_name(name: &str) -> bool {
    matches!(
        name,
        "list" | "ls" | "list_directory" | "list_dir" | "read_dir" | "dir"
    )
}

fn is_glob_activity_tool_name(name: &str) -> bool {
    matches!(name, "glob" | "find_files" | "find_file" | "walk")
}

fn extract_tool_command(line: &str) -> Option<String> {
    extract_tool_command_from_json(line)
        .or_else(|| extract_tool_key_value(line, "cmd"))
        .or_else(|| extract_tool_key_value(line, "command"))
        .map(|command| command.trim().to_owned())
        .filter(|command| !command.is_empty())
}

fn extract_tool_string_value(line: &str, keys: &[&str]) -> Option<String> {
    extract_tool_string_value_from_json(line, keys).or_else(|| {
        keys.iter()
            .find_map(|key| extract_tool_key_value(line, key))
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
}

fn extract_tool_string_value_from_json(line: &str, keys: &[&str]) -> Option<String> {
    let start = line.find('{')?;
    let end = line.rfind('}')?;
    if end <= start {
        return None;
    }
    let value = serde_json::from_str::<Value>(&line[start..=end]).ok()?;
    first_string_field_recursive(&value, keys, 0)
}

fn extract_search_tool_summary(lines: &[String]) -> Option<String> {
    let query = lines.iter().find_map(|line| {
        extract_tool_string_value(line, &["query", "pattern", "needle", "text"])
    })?;
    let query = truncate_middle_display(query.as_str(), 48);
    let path = lines
        .iter()
        .find_map(|line| extract_tool_path(line))
        .map(|path| shorten_display_path(path.as_str()));

    Some(if let Some(path) = path {
        format!("\"{query}\" in {path}")
    } else {
        format!("\"{query}\"")
    })
}

fn extract_list_tool_summary(lines: &[String]) -> Option<String> {
    lines
        .iter()
        .find_map(|line| extract_tool_path(line))
        .map(|path| shorten_display_path(path.as_str()))
}

fn extract_glob_tool_summary(lines: &[String]) -> Option<String> {
    let pattern = lines.iter().find_map(|line| {
        extract_tool_string_value(line, &["glob", "pattern", "query", "pathspec"])
    })?;
    let pattern = truncate_middle_display(pattern.as_str(), 48);
    let path = lines
        .iter()
        .find_map(|line| extract_tool_path(line))
        .map(|path| shorten_display_path(path.as_str()));

    Some(if let Some(path) = path {
        format!("{pattern} in {path}")
    } else {
        pattern
    })
}

fn extract_tool_command_from_json(line: &str) -> Option<String> {
    let start = line.find('{')?;
    let end = line.rfind('}')?;
    if end <= start {
        return None;
    }
    let value = serde_json::from_str::<Value>(&line[start..=end]).ok()?;
    first_string_field_recursive(&value, &["cmd", "command", "script"], 0)
}

fn first_string_field_recursive(value: &Value, keys: &[&str], depth: usize) -> Option<String> {
    if depth > 3 {
        return None;
    }
    match value {
        Value::Object(object) => {
            for key in keys {
                if let Some(text) = object.get(*key).and_then(Value::as_str)
                    && !text.trim().is_empty()
                {
                    return Some(text.trim().to_owned());
                }
            }
            object
                .values()
                .find_map(|value| first_string_field_recursive(value, keys, depth + 1))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|value| first_string_field_recursive(value, keys, depth + 1)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn extract_tool_stream_preview(lines: &[String], label: &str) -> ToolStreamPreview {
    let mut preview = ToolStreamPreview::default();
    for line in lines {
        let Some(body) = extract_tool_stream_line(line, label) else {
            continue;
        };
        let body = normalize_tool_stream_preview_text(body);
        if body.is_empty() {
            continue;
        }
        if preview.lines.len() < TOOL_STREAM_PREVIEW_MAX_LINES {
            preview.lines.push(body);
        } else {
            preview.omitted_count += 1;
        }
    }
    preview
}

fn extract_tool_stream_tail_preview(lines: &[String], label: &str) -> ToolStreamPreview {
    let mut collected = lines
        .iter()
        .filter_map(|line| extract_tool_stream_line(line, label))
        .map(normalize_tool_stream_preview_text)
        .filter(|body| !body.is_empty())
        .collect::<Vec<_>>();

    let omitted_count = collected
        .len()
        .saturating_sub(TOOL_STREAM_PREVIEW_MAX_LINES);
    if omitted_count > 0 {
        collected = collected.split_off(omitted_count);
    }

    ToolStreamPreview {
        lines: collected,
        omitted_count,
        truncated_from_start: omitted_count > 0,
    }
}

fn extract_tool_stream_line<'a>(line: &'a str, label: &str) -> Option<&'a str> {
    let trimmed = line.trim_start();
    trimmed
        .strip_prefix(&format!("{label}:"))
        .or_else(|| trimmed.strip_prefix(&format!("{label} ")))
        .or_else(|| trimmed.strip_prefix(&format!("↳ {label} ")))
        .map(str::trim_start)
}

fn normalize_tool_stream_preview_text(text: &str) -> String {
    text.replace('\t', "    ").trim_end().to_owned()
}

fn extract_tool_metrics_line(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    trimmed
        .strip_prefix("metrics:")
        .or_else(|| trimmed.strip_prefix("metrics "))
        .or_else(|| trimmed.strip_prefix("↳ metrics "))
        .map(str::trim)
        .filter(|metrics| !metrics.is_empty())
        .map(ToOwned::to_owned)
}

fn activity_tool_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let trimmed = trimmed.strip_prefix("• ").unwrap_or(trimmed);
    let rest = if let Some(status_rest) = trimmed.strip_prefix('[') {
        status_rest.split_once("] ")?.1
    } else if let Some(rest) = trimmed.strip_prefix("Called ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("Closed ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("Approval ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("Denied ") {
        rest
    } else if trimmed == "read" || trimmed.starts_with("read ") {
        trimmed
    } else {
        return None;
    };

    let name = rest
        .split(|ch: char| ch.is_whitespace() || ch == '(' || ch == ':' || ch == ',')
        .next()
        .filter(|name| !name.is_empty())?;
    Some(name)
}

fn is_read_activity_tool_name(name: &str) -> bool {
    let normalized = normalized_activity_tool_name(name);
    matches!(
        normalized.as_str(),
        "read" | "read_file" | "read-file" | "readfile" | "open_file" | "open-file" | "cat"
    )
}

fn extract_read_tool_request(line: &str) -> Option<ReadToolRequest> {
    extract_read_tool_request_from_json(line).or_else(|| extract_read_tool_request_from_text(line))
}

fn extract_read_tool_request_from_json(line: &str) -> Option<ReadToolRequest> {
    let start = line.find('{')?;
    let end = line.rfind('}')?;
    if end <= start {
        return None;
    }
    let value = serde_json::from_str::<Value>(&line[start..=end]).ok()?;
    let path = first_path_field(&value)?;
    Some(ReadToolRequest {
        path,
        offset: numeric_json_field(&value, "offset"),
        limit: numeric_json_field(&value, "limit"),
    })
}

fn numeric_json_field(value: &Value, key: &str) -> Option<u64> {
    numeric_json_field_recursive(value, key, 0)
}

fn numeric_json_field_recursive(value: &Value, key: &str, depth: usize) -> Option<u64> {
    if depth > 3 {
        return None;
    }
    match value {
        Value::Object(object) => object.get(key).and_then(json_value_as_u64).or_else(|| {
            object
                .values()
                .find_map(|value| numeric_json_field_recursive(value, key, depth + 1))
        }),
        Value::Array(items) => items
            .iter()
            .find_map(|value| numeric_json_field_recursive(value, key, depth + 1)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn json_value_as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
}

fn extract_read_tool_request_from_text(line: &str) -> Option<ReadToolRequest> {
    let path = extract_tool_path(line)?;
    Some(ReadToolRequest {
        path,
        offset: extract_tool_numeric_key_value(line, "offset"),
        limit: extract_tool_numeric_key_value(line, "limit"),
    })
}

fn extract_tool_numeric_key_value(line: &str, key: &str) -> Option<u64> {
    let marker = format!("{key}=");
    let start = line.find(marker.as_str())? + marker.len();
    let rest = &line[start..];
    let end = rest
        .find(" · ")
        .or_else(|| rest.find(", "))
        .or_else(|| rest.find('}'))
        .unwrap_or(rest.len());
    rest[..end]
        .trim()
        .trim_matches(',')
        .trim_matches('"')
        .trim_matches('\'')
        .parse::<u64>()
        .ok()
}

fn format_read_request_display(request: &ReadToolRequest) -> String {
    let mut display = shorten_display_path(request.path.as_str());
    if let Some(offset) = request.offset {
        display.push_str(format_read_line_range(offset, request.limit).as_str());
    }
    display
}

fn format_read_line_range(offset: u64, limit: Option<u64>) -> String {
    let start = offset.max(1);
    match limit.and_then(|limit| limit.checked_sub(1)) {
        Some(limit_tail) if limit_tail > 0 => format!(":{start}-{}", start + limit_tail),
        _ => format!(":{start}"),
    }
}

fn shorten_display_path(path: &str) -> String {
    let path = path.trim();
    if let Some(home) = std::env::var_os("HOME").and_then(|home| home.into_string().ok())
        && !home.is_empty()
        && let Some(rest) = path.strip_prefix(home.as_str())
        && (rest.is_empty() || rest.starts_with('/'))
    {
        return format!("~{rest}");
    }
    path.to_owned()
}

fn extract_read_image_summary(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let start = trimmed.find("Read image file [")?;
    Some(trimmed[start..].to_owned())
}

fn extract_image_mime(text: &str) -> Option<String> {
    let start = text.find("[image/")? + 1;
    let rest = &text[start..];
    let end = rest.find(']')?;
    Some(rest[..end].to_owned())
}

fn extract_read_text_excerpt(lines: &[String]) -> Vec<String> {
    let mut excerpt = Vec::new();
    for line in lines {
        let trimmed = line.trim_start();
        let candidate = trimmed
            .strip_prefix("stdout:")
            .or_else(|| trimmed.strip_prefix("stdout "))
            .or_else(|| trimmed.strip_prefix("↳ stdout "))
            .or_else(|| line.strip_prefix("    "))
            .map(str::trim);
        let Some(candidate) = candidate else {
            continue;
        };
        if candidate.is_empty()
            || candidate.starts_with("Read image file [")
            || looks_like_tool_output_summary(candidate)
        {
            continue;
        }
        excerpt.push(candidate.to_owned());
        if excerpt.len() >= READ_TEXT_PREVIEW_MAX_LINES {
            break;
        }
    }
    excerpt
}

fn looks_like_tool_output_summary(candidate: &str) -> bool {
    let mut parts = candidate.split(" · ");
    let Some(line_part) = parts.next() else {
        return false;
    };
    let Some(byte_part) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }

    let line_tokens = line_part.split_whitespace().collect::<Vec<_>>();
    let byte_tokens = byte_part.split_whitespace().collect::<Vec<_>>();
    let line_summary = matches!(
        line_tokens.as_slice(),
        [count, "line" | "lines"] if count.chars().all(|ch| ch.is_ascii_digit())
    );
    let byte_summary = matches!(
        byte_tokens.as_slice(),
        [count, "byte" | "bytes"] if count.chars().all(|ch| ch.is_ascii_digit())
    );

    line_summary && byte_summary
}

fn extract_tool_path(line: &str) -> Option<String> {
    extract_tool_path_from_json(line)
        .or_else(|| extract_tool_key_value(line, "path"))
        .or_else(|| extract_tool_key_value(line, "file_path"))
        .or_else(|| extract_tool_key_value(line, "absolute_path"))
        .or_else(|| extract_raw_path_line(line))
}

fn extract_tool_path_from_json(line: &str) -> Option<String> {
    let start = line.find('{')?;
    let end = line.rfind('}')?;
    if end <= start {
        return None;
    }
    let value = serde_json::from_str::<Value>(&line[start..=end]).ok()?;
    first_path_field(&value)
}

fn first_path_field(value: &Value) -> Option<String> {
    first_path_field_recursive(value, 0)
}

fn first_path_field_recursive(value: &Value, depth: usize) -> Option<String> {
    if depth > 3 {
        return None;
    }
    match value {
        Value::Object(object) => {
            for key in ["path", "file_path", "absolute_path", "source", "url"] {
                if let Some(value) = object.get(key).and_then(Value::as_str)
                    && !value.trim().is_empty()
                {
                    return Some(value.trim().to_owned());
                }
            }
            object
                .values()
                .find_map(|value| first_path_field_recursive(value, depth + 1))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|value| first_path_field_recursive(value, depth + 1)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn extract_tool_key_value(line: &str, key: &str) -> Option<String> {
    let marker = format!("{key}=");
    let start = line.find(marker.as_str())? + marker.len();
    let rest = &line[start..];
    let end = rest
        .find(" · ")
        .or_else(|| rest.find(", "))
        .or_else(|| rest.find('}'))
        .unwrap_or(rest.len());
    let value = rest[..end]
        .trim()
        .trim_matches(',')
        .trim_matches('"')
        .trim_matches('\'')
        .to_owned();
    (!value.is_empty()).then_some(value)
}

fn extract_raw_path_line(line: &str) -> Option<String> {
    let trimmed = line.trim().trim_matches('"').trim_matches('\'');
    if trimmed.starts_with('/')
        || trimmed.starts_with("~/")
        || trimmed.starts_with("./")
        || trimmed.starts_with("../")
        || trimmed.starts_with("file://")
    {
        Some(trimmed.to_owned())
    } else {
        None
    }
}

fn resolve_local_renderable_image_path(source: &str) -> Option<PathBuf> {
    let source = source.trim().trim_matches('"').trim_matches('\'');
    let source = if let Some(rest) = source.strip_prefix("file://") {
        percent_decode_path(rest)
    } else {
        source.to_owned()
    };
    if source.starts_with("http://")
        || source.starts_with("https://")
        || source.starts_with("data:")
        || source.is_empty()
    {
        return None;
    }

    let path = if let Some(rest) = source.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(rest))?
    } else {
        PathBuf::from(source)
    };

    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    path_has_image_extension(path.as_path()).then_some(path)
}

fn path_has_image_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp"
            )
        })
        .unwrap_or(false)
}

fn percent_decode_path(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'%'
            && let (Some(&high_byte), Some(&low_byte)) =
                (bytes.get(index + 1), bytes.get(index + 2))
            && let (Some(high), Some(low)) = (hex_value(high_byte), hex_value(low_byte))
        {
            decoded.push(high * 16 + low);
            index += 3;
            continue;
        }
        decoded.push(byte);
        index += 1;
    }
    String::from_utf8(decoded).unwrap_or_else(|_| path.to_owned())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

