fn build_assistant_contents(text: &str) -> Vec<MessageContent> {
    if let Some(body) = text.trim().strip_prefix(PROVIDER_ERROR_REPLY_PREFIX) {
        return vec![parse_provider_error_content(body)];
    }

    if is_compacted_summary_content(text) {
        return vec![parse_compaction_content(text)];
    }

    let sanitized_text = sanitize_plain_assistant_text(text);

    if !assistant_text_has_explicit_structure(sanitized_text.as_str()) {
        let mut contents = Vec::new();
        append_markdown_or_image_contents(sanitized_text.as_str(), &mut contents);
        if contents.is_empty() {
            contents.push(MessageContent::Markdown(sanitized_text));
        }
        return contents;
    }

    let sections = super::super::parse_cli_chat_markdown_sections(sanitized_text.as_str());
    let mut contents = Vec::new();

    for section in sections {
        match section {
            TuiSectionSpec::Preformatted {
                title,
                language: Some(language),
                lines,
            } if matches!(
                language.trim().to_ascii_lowercase().as_str(),
                "diff" | "patch"
            ) =>
            {
                contents.push(MessageContent::Diff {
                    title,
                    content: lines.join("\n"),
                });
            }
            TuiSectionSpec::Callout { title, lines, .. }
                if title
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case("tool activity")) =>
            {
                contents.push(MessageContent::ToolCall {
                    title: title.unwrap_or_else(|| "tool activity".to_owned()),
                    status: infer_tool_status(&lines),
                    lines,
                });
            }
            other @ (TuiSectionSpec::Narrative { .. }
            | TuiSectionSpec::KeyValues { .. }
            | TuiSectionSpec::ActionGroup { .. }
            | TuiSectionSpec::Checklist { .. }
            | TuiSectionSpec::Callout { .. }
            | TuiSectionSpec::Preformatted { .. }) => {
                let markdown = render_section_markdown(&other);
                append_markdown_or_image_contents(&markdown, &mut contents);
            }
        }
    }

    if contents.is_empty() {
        append_markdown_or_image_contents(sanitized_text.as_str(), &mut contents);
    }

    if contents.is_empty() {
        contents.push(MessageContent::Markdown(sanitized_text));
    }

    contents
}

fn sanitize_plain_assistant_text(text: &str) -> String {
    let mut visible_lines = Vec::new();
    let mut saw_visible_content = false;
    let mut internal_tail_started = false;

    for line in text.lines() {
        let trimmed = line.trim();
        let internal_tail_line = looks_like_internal_tool_result_line(trimmed)
            || looks_like_provider_transport_tail(trimmed);

        if !internal_tail_started && internal_tail_line && saw_visible_content {
            internal_tail_started = true;
        }

        if internal_tail_started {
            continue;
        }

        if !trimmed.is_empty() {
            saw_visible_content = true;
        }
        visible_lines.push(line);
    }

    if !internal_tail_started {
        return text.to_owned();
    }

    while visible_lines
        .last()
        .is_some_and(|line| line.trim().is_empty())
    {
        visible_lines.pop();
    }

    if visible_lines.is_empty() {
        text.to_owned()
    } else {
        visible_lines.join("\n")
    }
}

fn looks_like_internal_tool_result_line(line: &str) -> bool {
    looks_like_status_prefixed_tool_result_envelope(line)
        || line.starts_with("[tool_result]")
        || line.starts_with("[tool_failure]")
}

fn looks_like_provider_transport_tail(line: &str) -> bool {
    line.contains("provider request failed for model `")
        || (line.contains("candidate_index=")
            && line.contains("candidate_count=")
            && line.contains("profile_index="))
        || line.contains("transport_failure")
}

fn looks_like_status_prefixed_tool_result_envelope(line: &str) -> bool {
    let trimmed = line.trim();
    let Some((prefix, payload)) = trimmed.split_once(' ') else {
        return false;
    };
    let Some(status_marker) = prefix
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    else {
        return false;
    };
    if status_marker.trim().is_empty() {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(payload).is_ok()
}

fn assistant_text_has_explicit_structure(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return false;
        }

        trimmed.starts_with("```")
            || trimmed.starts_with('>')
            || super::super::parse_markdown_heading(trimmed).is_some()
    })
}

fn parse_provider_error_content(body: &str) -> MessageContent {
    let segments = body
        .split(" | ")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    let mut summary = segments
        .first()
        .copied()
        .unwrap_or("provider request failed")
        .to_owned();
    let mut details = Vec::new();

    if let Some((trimmed_summary, inline_details)) = extract_summary_details(&summary) {
        summary = trimmed_summary;
        details.extend(inline_details);
    }

    for segment in segments.iter().skip(1) {
        append_provider_error_segment(segment, &mut details);
    }
    summary = compact_provider_error_summary(&summary);
    details = compact_provider_error_details(details);

    MessageContent::Error {
        title: "provider error".to_owned(),
        summary,
        details,
    }
}

fn compact_provider_error_summary(summary: &str) -> String {
    let Some(rest) = summary.strip_prefix("provider returned status ") else {
        return summary.to_owned();
    };
    let Some((status, rest)) = rest.split_once(" for model `") else {
        return summary.to_owned();
    };
    let Some((model, rest)) = rest.split_once("` on attempt ") else {
        return summary.to_owned();
    };
    let attempt = rest.trim();
    if attempt.is_empty() {
        return summary.to_owned();
    }

    format!("{status} · {model} · {attempt}")
}

fn extract_summary_details(summary: &str) -> Option<(String, Vec<String>)> {
    let mut trimmed_summary = summary.trim().to_owned();
    let mut details = Vec::new();

    if let Some(start) = trimmed_summary.find(" (last_reason=")
        && trimmed_summary.ends_with(')')
    {
        let reason =
            trimmed_summary[start + " (last_reason=".len()..trimmed_summary.len() - 1].trim();
        if !reason.is_empty() {
            details.push(format!("last_reason: {reason}"));
        }
        trimmed_summary = trimmed_summary[..start].trim().to_owned();
    }

    if let Some((prefix, json_value, suffix, separator)) =
        extract_inline_json_payload(&trimmed_summary)
    {
        let key = if separator == '=' {
            Some("response")
        } else {
            None
        };
        append_json_detail_lines(key, &json_value, &mut details);
        if !suffix.trim().is_empty() {
            details.push(suffix.trim().to_owned());
        }
        trimmed_summary = prefix.trim().trim_end_matches(':').trim().to_owned();
    }

    if details.is_empty() {
        None
    } else {
        Some((trimmed_summary, details))
    }
}

fn append_provider_error_segment(segment: &str, details: &mut Vec<String>) {
    if let Some((key, value)) = segment.split_once('=') {
        if let Ok(json_value) = serde_json::from_str::<Value>(value) {
            append_json_value_lines(key.trim(), &json_value, details);
        } else {
            details.push(format!("{}: {}", key.trim(), value.trim()));
        }
    } else if let Some((prefix, json_value, suffix, separator)) =
        extract_inline_json_payload(segment)
    {
        let key = if separator == '=' {
            prefix.trim().trim_end_matches('=').trim()
        } else {
            "response"
        };
        append_json_detail_lines(Some(key), &json_value, details);
        if !suffix.trim().is_empty() {
            details.push(suffix.trim().to_owned());
        }
    } else {
        details.push(segment.to_owned());
    }
}

fn extract_inline_json_payload(segment: &str) -> Option<(String, String, String, char)> {
    let bytes = segment.as_bytes();
    let mut start = None;
    let mut separator = ':';

    for (idx, ch) in segment.char_indices() {
        if (ch == '{' || ch == '[') && idx > 0 {
            let mut separator_index = idx.saturating_sub(1);
            while separator_index > 0 && bytes.get(separator_index).copied() == Some(b' ') {
                separator_index -= 1;
            }
            let sep = bytes.get(separator_index).copied().map(char::from);
            if matches!(sep, Some(':') | Some('=')) {
                start = Some(idx);
                separator = sep.unwrap_or(':');
                break;
            }
        }
    }

    let start = start?;
    let opening = segment[start..].chars().next()?;
    let closing = match opening {
        '{' => '}',
        '[' => ']',
        _ => return None,
    };

    let mut depth = 0usize;
    let mut end = None;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, ch) in segment[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            continue;
        }
        if ch == opening {
            depth += 1;
        } else if ch == closing {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                end = Some(start + offset + ch.len_utf8());
                break;
            }
        }
    }

    let end = end?;
    Some((
        segment[..start].to_owned(),
        segment[start..end].to_owned(),
        segment[end..].to_owned(),
        separator,
    ))
}

fn append_json_detail_lines(key: Option<&str>, json_text: &str, details: &mut Vec<String>) {
    if let Ok(value) = serde_json::from_str::<Value>(json_text) {
        if let (None, Value::Object(map)) = (key, &value) {
            for (entry_key, entry_value) in map {
                append_json_value_lines(entry_key, entry_value, details);
            }
        } else {
            append_json_value_lines(key.unwrap_or("response"), &value, details);
        }
    } else {
        let label = key.unwrap_or("response");
        details.push(format!("{label}: {json_text}"));
    }
}

fn append_json_value_lines(prefix: &str, value: &Value, details: &mut Vec<String>) {
    if prefix == "provider_failover"
        && let Some(object) = value.as_object()
    {
        if let (Some(attempt), Some(max_attempts)) = (
            object.get("attempt").and_then(Value::as_u64),
            object.get("max_attempts").and_then(Value::as_u64),
        ) {
            details.push(format!(
                "provider_failover.attempt: {attempt}/{max_attempts}"
            ));
        }
        if let Some(reason) = object.get("reason").and_then(Value::as_str) {
            details.push(format!("provider_failover.reason: {reason}"));
        }
        if let Some(stage) = object.get("stage").and_then(Value::as_str) {
            details.push(format!("provider_failover.stage: {stage}"));
        }
        if let Some(model) = object.get("model").and_then(Value::as_str) {
            details.push(format!("provider_failover.model: {model}"));
        }
        if let Some(status_code) = object.get("status_code").and_then(Value::as_u64) {
            details.push(format!("provider_failover.status_code: {status_code}"));
        }
        for (key, value) in object {
            if matches!(
                key.as_str(),
                "attempt" | "max_attempts" | "reason" | "stage" | "model" | "status_code"
            ) {
                continue;
            }
            append_json_value_lines(&format!("{prefix}.{key}"), value, details);
        }
        return;
    }

    match value {
        Value::Object(map) => {
            for (key, value) in map {
                append_json_value_lines(&format!("{prefix}.{key}"), value, details);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                append_json_value_lines(&format!("{prefix}[{index}]"), item, details);
            }
        }
        Value::String(text) => details.push(format!("{prefix}: {text}")),
        Value::Null | Value::Bool(_) | Value::Number(_) => {
            details.push(format!("{prefix}: {value}"))
        }
    }
}

fn compact_provider_error_details(details: Vec<String>) -> Vec<String> {
    let mut code = None;
    let mut message = None;
    let mut last_reason = None;
    let mut failover_reason = None;
    let mut failover_stage = None;
    let mut failover_attempt = None;
    let mut failover_status = None;
    let mut passthrough = Vec::new();

    for detail in details {
        if let Some(value) = detail.strip_prefix("code: ") {
            code = Some(value.to_owned());
        } else if let Some(value) = detail.strip_prefix("message: ") {
            message = Some(value.to_owned());
        } else if let Some(value) = detail.strip_prefix("last_reason: ") {
            last_reason = Some(value.to_owned());
        } else if let Some(value) = detail.strip_prefix("provider_failover.reason: ") {
            failover_reason = Some(value.to_owned());
        } else if let Some(value) = detail.strip_prefix("provider_failover.stage: ") {
            failover_stage = Some(value.to_owned());
        } else if let Some(value) = detail.strip_prefix("provider_failover.attempt: ") {
            failover_attempt = Some(value.to_owned());
        } else if detail.starts_with("provider_failover.model: ") {
        } else if let Some(value) = detail.strip_prefix("provider_failover.status_code: ") {
            failover_status = Some(value.to_owned());
        } else {
            passthrough.push(detail);
        }
    }

    let mut compacted = Vec::new();

    let mut response_parts = Vec::new();
    if let Some(code) = code {
        response_parts.push(code);
    }
    if let Some(message) = message {
        response_parts.push(message);
    }
    if !response_parts.is_empty() {
        compacted.push(response_parts.join(" · "));
    }

    let mut failover_parts = Vec::new();
    if let Some(reason) = failover_reason.or(last_reason) {
        failover_parts.push(reason);
    }
    if let Some(stage) = failover_stage {
        failover_parts.push(stage);
    }
    if let Some(attempt) = failover_attempt {
        failover_parts.push(attempt);
    }
    if let Some(status) = failover_status {
        failover_parts.push(status);
    }
    if !failover_parts.is_empty() {
        compacted.push(failover_parts.join(" · "));
    }

    compacted.extend(passthrough);
    compacted
}

fn parse_compaction_content(text: &str) -> MessageContent {
    let mut turn_count = 0usize;
    let mut summary_lines = Vec::new();

    for line in text.lines() {
        if let Some(value) = line
            .strip_prefix("Compacted ")
            .and_then(|rest| rest.strip_suffix(" earlier turns"))
            .and_then(|value| value.parse::<usize>().ok())
        {
            turn_count = value;
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            continue;
        }

        if line == "This compacted checkpoint is session-local recall only." {
            continue;
        }

        if !line.trim().is_empty() {
            summary_lines.push(line.to_owned());
        }
    }

    MessageContent::Compaction {
        turn_count,
        summary: summary_lines.join("\n"),
        expanded: false,
    }
}

fn message_plain_text(message: &Message) -> Option<String> {
    let parts = message
        .contents
        .iter()
        .filter_map(content_plain_text)
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

fn content_plain_text(content: &MessageContent) -> Option<String> {
    match content {
        MessageContent::RenderedLines(lines) => Some(lines.join("\n")),
        MessageContent::Markdown(text) => Some(text.clone()),
        MessageContent::Diff { title, content } => {
            let mut rendered = Vec::new();
            if let Some(title) = title {
                rendered.push(format!("### {title}"));
            }
            rendered.push("```diff".to_owned());
            rendered.push(content.clone());
            rendered.push("```".to_owned());
            Some(rendered.join("\n"))
        }
        MessageContent::Image { alt, url } => Some(format!("![{alt}]({url})")),
        MessageContent::ToolCall {
            title,
            lines,
            status,
        } => {
            let status = match status {
                ToolStatus::Pending => "pending",
                ToolStatus::Success => "success",
                ToolStatus::Error => "error",
            };
            let mut rendered = vec![format!("### {title} ({status})")];
            rendered.extend(lines.iter().cloned());
            Some(rendered.join("\n"))
        }
        MessageContent::Error {
            title,
            summary,
            details,
        } => {
            let mut rendered = vec![format!("### {title}"), summary.clone()];
            rendered.extend(details.iter().map(|detail| format!("- {detail}")));
            Some(rendered.join("\n"))
        }
        MessageContent::Compaction {
            turn_count,
            summary,
            ..
        } => Some(format!("### Compaction ({turn_count} turns)\n{summary}")),
        MessageContent::StartupHeader { .. } => None,
    }
}

fn infer_tool_status(lines: &[String]) -> ToolStatus {
    let lower = lines.join("\n").to_ascii_lowercase();
    if lower.contains("[failed]")
        || lower.contains(" interrupted")
        || lower.contains(" error")
        || lower.contains(" exit=") && !lower.contains("exit=0")
    {
        ToolStatus::Error
    } else if lower.contains("[running]") || lower.contains("[pending]") {
        ToolStatus::Pending
    } else {
        ToolStatus::Success
    }
}

fn append_markdown_or_image_contents(markdown: &str, contents: &mut Vec<MessageContent>) {
    let mut markdown_buffer = Vec::new();

    for line in markdown.lines() {
        if let Some((alt, url)) = parse_markdown_image_line(line) {
            if !markdown_buffer.is_empty() {
                let buffered = markdown_buffer.join("\n");
                if !buffered.trim().is_empty() {
                    contents.push(MessageContent::Markdown(buffered));
                }
                markdown_buffer.clear();
            }
            contents.push(MessageContent::Image { alt, url });
            continue;
        }

        markdown_buffer.push(line.to_owned());
    }

    if !markdown_buffer.is_empty() {
        let buffered = markdown_buffer.join("\n");
        if !buffered.trim().is_empty() {
            contents.push(MessageContent::Markdown(buffered));
        }
    }
}

fn parse_markdown_image_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("![")?;
    let (alt, remainder) = rest.split_once("](")?;
    let url = remainder.strip_suffix(')')?;
    Some((alt.trim().to_owned(), url.trim().to_owned()))
}

fn render_section_markdown(section: &TuiSectionSpec) -> String {
    match section {
        TuiSectionSpec::Narrative { title, lines } => {
            let mut parts = Vec::new();
            if let Some(title) = title {
                parts.push(format!("### {title}"));
            }
            parts.extend(lines.iter().cloned());
            parts.join("\n")
        }
        TuiSectionSpec::Callout { title, lines, .. } => {
            let mut rendered = Vec::new();
            if let Some(title) = title {
                rendered.push(format!("### {title}"));
            }
            rendered.extend(lines.iter().map(|line| format!("> {line}")));
            rendered.join("\n")
        }
        TuiSectionSpec::Preformatted {
            title,
            language,
            lines,
        } => {
            let mut parts = Vec::new();
            if let Some(title) = title {
                parts.push(format!("### {title}"));
            }
            let fence = language.as_deref().unwrap_or("");
            parts.push(format!("```{fence}"));
            parts.extend(lines.iter().cloned());
            parts.push("```".to_owned());
            parts.join("\n")
        }
        TuiSectionSpec::KeyValues { title, items } => {
            let mut parts = Vec::new();
            if let Some(title) = title {
                parts.push(format!("### {title}"));
            }
            for item in items {
                match item {
                    crate::tui_surface::TuiKeyValueSpec::Plain { key, value } => {
                        parts.push(format!("- {key}: {value}"));
                    }
                    crate::tui_surface::TuiKeyValueSpec::Csv { key, values } => {
                        parts.push(format!("- {key}: {}", values.join(", ")));
                    }
                }
            }
            parts.join("\n")
        }
        TuiSectionSpec::ActionGroup { title, items, .. } => {
            let mut parts = Vec::new();
            if let Some(title) = title {
                parts.push(format!("### {title}"));
            }
            for item in items {
                parts.push(format!("- {}: `{}`", item.label, item.command));
            }
            parts.join("\n")
        }
        TuiSectionSpec::Checklist { title, items } => {
            let mut parts = Vec::new();
            if let Some(title) = title {
                parts.push(format!("### {title}"));
            }
            for item in items {
                parts.push(format!("- {} — {}", item.label, item.detail));
            }
            parts.join("\n")
        }
    }
}

