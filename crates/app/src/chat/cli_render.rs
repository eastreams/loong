use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChatCommandMatchResult {
    Matched,
    NotMatched,
    UsageError(String),
}

pub(super) fn classify_chat_command_match_result(
    result: CliResult<bool>,
) -> CliResult<ChatCommandMatchResult> {
    match result {
        Ok(true) => Ok(ChatCommandMatchResult::Matched),
        Ok(false) => Ok(ChatCommandMatchResult::NotMatched),
        Err(error) if error.starts_with("usage:") => Ok(ChatCommandMatchResult::UsageError(error)),
        Err(error) => Err(error),
    }
}

pub(super) fn detect_cli_chat_render_width() -> usize {
    crate::presentation::detect_render_width()
}

#[allow(clippy::print_stdout)] // CLI output
pub(super) fn print_rendered_cli_chat_lines(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

pub(super) fn render_cli_chat_assistant_lines_with_width(
    assistant_text: &str,
    width: usize,
) -> Vec<String> {
    if let Some(screen_spec) = build_cli_chat_approval_screen_spec(assistant_text) {
        return render_tui_screen_spec(&screen_spec, width, false);
    }
    let message_spec = build_cli_chat_assistant_message_spec(assistant_text);
    render_cli_chat_message_spec_with_width(&message_spec, width)
}

pub(super) fn render_cli_chat_command_usage_lines_with_width(
    usage: &str,
    width: usize,
) -> Vec<String> {
    let message_spec = TuiMessageSpec {
        role: "chat".to_owned(),
        caption: Some("command".to_owned()),
        sections: vec![TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Warning,
            title: Some("usage".to_owned()),
            lines: vec![usage.to_owned()],
        }],
        footer_lines: vec!["Use /help to inspect the available command surface.".to_owned()],
    };

    render_cli_chat_message_spec_with_width(&message_spec, width)
}

pub(super) fn maybe_render_nonfatal_usage_error(error: &str) -> Option<Vec<String>> {
    let usage_error = error.contains("usage:");
    if !usage_error {
        return None;
    }

    let render_width = detect_cli_chat_render_width();
    let usage_lines = render_cli_chat_command_usage_lines_with_width(error, render_width);

    Some(usage_lines)
}

fn build_cli_chat_assistant_message_spec(assistant_text: &str) -> TuiMessageSpec {
    let sections = parse_cli_chat_markdown_sections(assistant_text);

    TuiMessageSpec {
        role: config::CLI_COMMAND_NAME.to_owned(),
        caption: Some("reply".to_owned()),
        sections,
        footer_lines: vec!["/help commands · /status runtime · /history transcript".to_owned()],
    }
}

pub(super) fn build_cli_chat_approval_screen_spec(assistant_text: &str) -> Option<TuiScreenSpec> {
    let parsed = parse_approval_prompt_view(assistant_text)?;
    let mut intro_lines = Vec::new();
    if let Some(preface) = parsed
        .preface
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        intro_lines.extend(preface.lines().map(|line| line.to_owned()));
    }

    let title = parsed.title();

    let mut sections = Vec::new();
    if let Some(reason) = parsed.reason.as_deref() {
        sections.push(TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Warning,
            title: Some(parsed.pause_reason_title()),
            lines: vec![reason.to_owned()],
        });
    }

    let mut kv_items = Vec::new();
    if let Some(tool_name) = parsed.tool_name.as_deref() {
        kv_items.push(TuiKeyValueSpec::Plain {
            key: parsed.tool_label(),
            value: tool_name.to_owned(),
        });
    }
    if let Some(request_id) = parsed.request_id.as_deref() {
        kv_items.push(TuiKeyValueSpec::Plain {
            key: parsed.request_id_label(),
            value: request_id.to_owned(),
        });
    }
    if !kv_items.is_empty() {
        sections.push(TuiSectionSpec::KeyValues {
            title: Some(parsed.request_section_title()),
            items: kv_items,
        });
    }

    let choices = parsed
        .actions
        .iter()
        .map(|action| TuiChoiceSpec {
            key: action.numeric_alias.clone(),
            label: action.label.clone(),
            detail_lines: action.detail_lines.clone(),
            recommended: action.recommended,
        })
        .collect::<Vec<_>>();

    let footer_lines = if parsed.actions.is_empty() {
        Vec::new()
    } else if parsed.locale.is_cjk() {
        vec![
            format!("也可以直接回复：{}", parsed.action_commands_text()),
            format!("数字别名：{}", parsed.action_numeric_aliases_text()),
        ]
    } else {
        vec![
            format!("You can also reply with: {}", parsed.action_commands_text()),
            format!("Numeric aliases: {}", parsed.action_numeric_aliases_text()),
        ]
    };

    Some(TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some(parsed.subtitle()),
        title,
        progress_line: None,
        intro_lines,
        sections,
        choices,
        footer_lines,
    })
}

pub(super) fn cli_chat_card_inner_width(width: usize) -> usize {
    width.saturating_sub(2).max(24)
}

pub(super) fn build_cli_chat_message_card_title(role: &str, caption: Option<&str>) -> String {
    let trimmed_role = role.trim();
    let trimmed_caption = caption.map(str::trim).unwrap_or("");
    let role_label = if trimmed_role.is_empty() {
        "message"
    } else {
        trimmed_role
    };

    if trimmed_caption.is_empty() {
        return role_label.to_owned();
    }

    format!("{role_label} · {trimmed_caption}")
}

fn render_cli_chat_message_card_lines(
    role: &str,
    caption: Option<&str>,
    rendered_message_lines: &[String],
    width: usize,
) -> Vec<String> {
    let title = build_cli_chat_message_card_title(role, caption);
    render_cli_chat_card_lines(title.as_str(), rendered_message_lines, width)
}

pub(super) fn render_cli_chat_message_spec_with_width(
    spec: &TuiMessageSpec,
    width: usize,
) -> Vec<String> {
    let body_lines = render_tui_message_body_spec(spec, cli_chat_card_inner_width(width));
    render_cli_chat_message_card_lines(
        spec.role.as_str(),
        spec.caption.as_deref(),
        &body_lines,
        width,
    )
}

pub(super) fn render_cli_chat_card_lines(
    title: &str,
    body_lines: &[String],
    width: usize,
) -> Vec<String> {
    let inner_width = cli_chat_card_inner_width(width);
    let mut lines = vec![format!("╭─ {title}")];
    if body_lines.is_empty() {
        lines.push("│".to_owned());
    } else {
        for line in body_lines {
            if line.is_empty() {
                lines.push("│".to_owned());
            } else {
                for wrapped_line in
                    crate::presentation::render_wrapped_display_line(line.as_str(), inner_width)
                {
                    lines.push(format!("│ {wrapped_line}"));
                }
            }
        }
    }

    lines.push("╰─".to_owned());
    lines
}

pub(super) fn parse_cli_chat_markdown_sections(text: &str) -> Vec<TuiSectionSpec> {
    let mut sections = Vec::new();
    let mut pending_title = None;
    let mut narrative_lines = Vec::new();
    let mut callout_lines = Vec::new();
    let mut code_title = None;
    let mut code_language = None;
    let mut code_lines = Vec::new();
    let mut inside_code_block = false;

    for raw_line in text.lines() {
        let trimmed_end = raw_line.trim_end();

        if inside_code_block {
            if is_markdown_fence_close(trimmed_end) {
                push_preformatted_section(
                    &mut sections,
                    &mut code_title,
                    &mut code_language,
                    &mut code_lines,
                );
                inside_code_block = false;
                continue;
            }

            code_lines.push(trimmed_end.to_owned());
            continue;
        }

        if let Some(language) = parse_markdown_fence_language(trimmed_end) {
            push_callout_section(&mut sections, &mut pending_title, &mut callout_lines);
            push_narrative_section(&mut sections, &mut pending_title, &mut narrative_lines);
            code_title = pending_title.take();
            code_language = language;
            inside_code_block = true;
            continue;
        }

        if let Some(heading_text) = parse_markdown_heading(trimmed_end) {
            push_callout_section(&mut sections, &mut pending_title, &mut callout_lines);
            push_narrative_section(&mut sections, &mut pending_title, &mut narrative_lines);
            push_standalone_title_section(&mut sections, &mut pending_title);
            pending_title = Some(heading_text.to_owned());
            continue;
        }

        if let Some(callout_line) = parse_markdown_quote_line(trimmed_end) {
            push_narrative_section(&mut sections, &mut pending_title, &mut narrative_lines);
            callout_lines.push(callout_line);
            continue;
        }

        if !callout_lines.is_empty() {
            push_callout_section(&mut sections, &mut pending_title, &mut callout_lines);
        }

        let normalized_line = normalize_markdown_display_line(trimmed_end);
        let is_blank_line = normalized_line.trim().is_empty();

        if is_blank_line && narrative_lines.is_empty() {
            continue;
        }

        narrative_lines.push(normalized_line);
    }

    if inside_code_block {
        push_preformatted_section(
            &mut sections,
            &mut code_title,
            &mut code_language,
            &mut code_lines,
        );
    }

    push_callout_section(&mut sections, &mut pending_title, &mut callout_lines);
    push_narrative_section(&mut sections, &mut pending_title, &mut narrative_lines);
    push_standalone_title_section(&mut sections, &mut pending_title);

    if sections.is_empty() {
        sections.push(TuiSectionSpec::Narrative {
            title: None,
            lines: vec!["(empty reply)".to_owned()],
        });
    }

    refine_cli_chat_sections(sections)
}

fn refine_cli_chat_sections(sections: Vec<TuiSectionSpec>) -> Vec<TuiSectionSpec> {
    sections
        .into_iter()
        .map(|section| match section {
            TuiSectionSpec::Narrative {
                title: Some(title),
                lines,
            } if is_reasoning_section_title(title.as_str()) && !lines.is_empty() => {
                TuiSectionSpec::Callout {
                    tone: TuiCalloutTone::Info,
                    title: Some("reasoning".to_owned()),
                    lines,
                }
            }
            TuiSectionSpec::Preformatted {
                title,
                language: Some(language),
                lines,
            } if is_diff_language(language.as_str()) => TuiSectionSpec::Preformatted {
                title: Some(title.unwrap_or_else(|| "diff".to_owned())),
                language: Some(language),
                lines,
            },
            TuiSectionSpec::Narrative {
                title: Some(title),
                lines,
            } if is_tool_activity_section_title(title.as_str()) && !lines.is_empty() => {
                TuiSectionSpec::Callout {
                    tone: TuiCalloutTone::Info,
                    title: Some("tool activity".to_owned()),
                    lines,
                }
            }
            other @ TuiSectionSpec::Narrative { .. }
            | other @ TuiSectionSpec::KeyValues { .. }
            | other @ TuiSectionSpec::ActionGroup { .. }
            | other @ TuiSectionSpec::Checklist { .. }
            | other @ TuiSectionSpec::Callout { .. }
            | other @ TuiSectionSpec::Preformatted { .. } => other,
        })
        .collect()
}

fn is_reasoning_section_title(title: &str) -> bool {
    let normalized = title.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "reasoning" | "analysis" | "thinking" | "thought process"
    )
}

fn is_diff_language(language: &str) -> bool {
    matches!(
        language.trim().to_ascii_lowercase().as_str(),
        "diff" | "patch"
    )
}

fn is_tool_activity_section_title(title: &str) -> bool {
    matches!(
        title.trim().to_ascii_lowercase().as_str(),
        "tool activity" | "tools" | "tool calls"
    )
}

fn push_narrative_section(
    sections: &mut Vec<TuiSectionSpec>,
    pending_title: &mut Option<String>,
    narrative_lines: &mut Vec<String>,
) {
    trim_blank_line_edges(narrative_lines);
    if narrative_lines.is_empty() {
        return;
    }

    let title = pending_title.take();
    let lines = std::mem::take(narrative_lines);
    sections.push(TuiSectionSpec::Narrative { title, lines });
}

fn push_standalone_title_section(
    sections: &mut Vec<TuiSectionSpec>,
    pending_title: &mut Option<String>,
) {
    let Some(title) = pending_title.take() else {
        return;
    };

    sections.push(TuiSectionSpec::Narrative {
        title: Some(title),
        lines: Vec::new(),
    });
}

fn push_callout_section(
    sections: &mut Vec<TuiSectionSpec>,
    pending_title: &mut Option<String>,
    callout_lines: &mut Vec<String>,
) {
    trim_blank_line_edges(callout_lines);
    if callout_lines.is_empty() {
        return;
    }

    let lines = std::mem::take(callout_lines);
    let title = pending_title
        .take()
        .or_else(|| Some("quoted context".to_owned()));

    sections.push(TuiSectionSpec::Callout {
        tone: TuiCalloutTone::Info,
        title,
        lines,
    });
}

fn push_preformatted_section(
    sections: &mut Vec<TuiSectionSpec>,
    code_title: &mut Option<String>,
    code_language: &mut Option<String>,
    code_lines: &mut Vec<String>,
) {
    let title = code_title.take();
    let language = code_language.take();
    let lines = std::mem::take(code_lines);
    sections.push(TuiSectionSpec::Preformatted {
        title,
        language,
        lines,
    });
}

fn trim_blank_line_edges(lines: &mut Vec<String>) {
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }

    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
}

fn is_markdown_fence_close(line: &str) -> bool {
    line.trim() == "```"
}

fn parse_markdown_fence_language(line: &str) -> Option<Option<String>> {
    let trimmed = line.trim();
    let raw_language = trimmed.strip_prefix("```")?;
    let language = raw_language.trim();

    if language.is_empty() {
        return Some(None);
    }

    Some(Some(language.to_owned()))
}

pub(super) fn parse_markdown_heading(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let marker_count = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();

    if marker_count == 0 || marker_count > 6 {
        return None;
    }

    let heading_text = trimmed.get(marker_count..)?;
    let separator = heading_text.chars().next()?;
    if separator != ' ' && separator != '\t' {
        return None;
    }

    let heading_text = heading_text.trim_start_matches([' ', '\t']);
    let normalized_text = trim_markdown_heading_closing_sequence(heading_text).trim();

    if normalized_text.is_empty() {
        return None;
    }

    Some(normalized_text)
}

fn trim_markdown_heading_closing_sequence(text: &str) -> &str {
    let trimmed_end = text.trim_end_matches([' ', '\t']);
    let trailing_hash_count = trimmed_end
        .chars()
        .rev()
        .take_while(|character| *character == '#')
        .count();

    if trailing_hash_count == 0 {
        return trimmed_end;
    }

    let content_end = trimmed_end.len().saturating_sub(trailing_hash_count);
    let content = trimmed_end.get(..content_end).unwrap_or(trimmed_end);
    let ends_with_heading_space = content
        .chars()
        .last()
        .is_some_and(|character| character == ' ' || character == '\t');

    if !ends_with_heading_space {
        return trimmed_end;
    }

    content.trim_end_matches([' ', '\t'])
}

fn parse_markdown_quote_line(line: &str) -> Option<String> {
    let trimmed_start = line.trim_start();
    let quote_body = trimmed_start.strip_prefix('>')?;
    let normalized_text = quote_body.trim_start();
    Some(normalized_text.to_owned())
}

fn normalize_markdown_display_line(line: &str) -> String {
    let trimmed_end = line.trim_end();
    let leading_space_count = trimmed_end
        .chars()
        .take_while(|character| character.is_ascii_whitespace())
        .count();
    let indent = trimmed_end.get(..leading_space_count).unwrap_or("");
    let trimmed_start = trimmed_end.get(leading_space_count..).unwrap_or("");

    if let Some(rest) = trimmed_start.strip_prefix("* ") {
        return format!("{indent}- {rest}");
    }

    if let Some(rest) = trimmed_start.strip_prefix("+ ") {
        return format!("{indent}- {rest}");
    }

    trimmed_end.to_owned()
}
