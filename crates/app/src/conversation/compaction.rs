use crate::memory::WindowTurn;
use crate::runtime_self_continuity;

const SUMMARY_MAX_RENDERED_TURNS: usize = 3;
const SUMMARY_PREFERRED_USER_TURNS: usize = 3;
const SUMMARY_PREFERRED_ASSISTANT_TURNS: usize = 0;
const SUMMARY_TURN_EXCERPT_CHARS: usize = 96;
const SUMMARY_TOTAL_CHARS_MAX: usize = 480;
const PRIOR_COMPACTED_SUMMARY_PLACEHOLDER: &str = "[prior compacted summary]";
const USER_CONTEXT_HEADING: &str = "User context:";
const ASSISTANT_PROGRESS_HEADING: &str = "Assistant progress:";
const OMITTED_CONTEXT_PREFIX: &str = "More omitted context:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactPolicy {
    preserve_recent_turns: usize,
}

impl CompactPolicy {
    pub fn new(preserve_recent_turns: usize) -> Self {
        Self {
            preserve_recent_turns,
        }
    }
}

pub fn compact_window(turns: &[WindowTurn], policy: CompactPolicy) -> Option<Vec<WindowTurn>> {
    let preserve = policy.preserve_recent_turns.min(turns.len());
    if turns.len() <= preserve {
        return None;
    }

    let split_at = turns.len() - preserve;
    let (older, recent) = turns.split_at(split_at);
    if older.len() == 1 && older.first().is_some_and(is_compacted_summary_turn) {
        return None;
    }

    let summary = WindowTurn {
        role: "user".to_owned(),
        content: format!(
            "Compacted {} earlier turns\n{}",
            older.len(),
            render_summary(older)
        ),
        ts: older.last().and_then(|turn| turn.ts),
    };

    let mut compacted = Vec::with_capacity(recent.len() + 1);
    compacted.push(summary);
    compacted.extend_from_slice(recent);
    Some(compacted)
}

fn render_summary(turns: &[WindowTurn]) -> String {
    let all_lines = turns
        .iter()
        .flat_map(render_summary_lines)
        .collect::<Vec<_>>();
    let mut selected_indices = Vec::new();
    extend_summary_indices(
        &mut selected_indices,
        &all_lines,
        /*want_user*/ true,
        SUMMARY_PREFERRED_USER_TURNS,
    );
    extend_summary_indices(
        &mut selected_indices,
        &all_lines,
        /*want_user*/ false,
        SUMMARY_PREFERRED_ASSISTANT_TURNS,
    );
    let remaining = SUMMARY_MAX_RENDERED_TURNS.saturating_sub(selected_indices.len());
    extend_remaining_summary_indices(&mut selected_indices, &all_lines, remaining);
    selected_indices.sort_unstable();

    let selected_lines = selected_indices
        .into_iter()
        .filter_map(|idx| all_lines.get(idx).cloned())
        .collect::<Vec<_>>();
    let omitted_lines = all_lines.len().saturating_sub(selected_lines.len());
    render_structured_summary(&selected_lines, omitted_lines)
}

fn extend_summary_indices(
    selected_indices: &mut Vec<usize>,
    all_lines: &[RenderedSummaryLine],
    want_user: bool,
    limit: usize,
) {
    if limit == 0 {
        return;
    }

    let matching_indices = all_lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (line.is_user == want_user).then_some(index));
    let matching_indices = matching_indices.collect::<Vec<_>>();
    let balanced_indices = select_balanced_indices(&matching_indices, limit);

    for index in balanced_indices {
        if selected_indices.len() >= SUMMARY_MAX_RENDERED_TURNS {
            return;
        }

        if selected_indices.contains(&index) {
            continue;
        }

        selected_indices.push(index);
    }
}

fn extend_remaining_summary_indices(
    selected_indices: &mut Vec<usize>,
    all_lines: &[RenderedSummaryLine],
    remaining: usize,
) {
    if remaining == 0 {
        return;
    }

    let all_indices = all_lines.iter().enumerate().map(|(index, _line)| index);
    for index in all_indices {
        if selected_indices.len() >= SUMMARY_MAX_RENDERED_TURNS {
            return;
        }

        if selected_indices.contains(&index) {
            continue;
        }

        selected_indices.push(index);
    }
}

fn select_balanced_indices(indices: &[usize], limit: usize) -> Vec<usize> {
    if limit == 0 {
        return Vec::new();
    }
    if indices.len() <= limit {
        return indices.to_vec();
    }

    let leading_count = limit / 2;
    let trailing_count = limit.saturating_sub(leading_count);
    let trailing_start = indices.len().saturating_sub(trailing_count);

    let mut selected = Vec::with_capacity(limit);
    let leading_indices = indices.iter().take(leading_count).copied();
    selected.extend(leading_indices);
    let trailing_indices = indices.iter().skip(trailing_start).copied();
    selected.extend(trailing_indices);
    selected.sort_unstable();
    selected.dedup();

    if selected.len() >= limit {
        return selected;
    }

    for index in indices {
        if selected.contains(index) {
            continue;
        }

        selected.push(*index);

        if selected.len() == limit {
            break;
        }
    }

    selected.sort_unstable();
    selected
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderedSummaryLine {
    text: String,
    is_user: bool,
}

fn render_structured_summary(lines: &[RenderedSummaryLine], omitted_lines: usize) -> String {
    let mut sections = Vec::new();
    let scope_note = runtime_self_continuity::compaction_summary_scope_note();
    sections.push(scope_note.to_owned());

    let user_lines = collect_summary_group(lines, true);
    append_summary_section(&mut sections, USER_CONTEXT_HEADING, &user_lines);

    let assistant_lines = collect_summary_group(lines, false);
    append_summary_section(&mut sections, ASSISTANT_PROGRESS_HEADING, &assistant_lines);

    if omitted_lines > 0 {
        let omitted_line =
            format!("{OMITTED_CONTEXT_PREFIX} {omitted_lines} earlier turns omitted.");
        sections.push(omitted_line);
    }

    let rendered = sections.join("\n");
    trim_to_chars(&rendered, SUMMARY_TOTAL_CHARS_MAX)
}

fn collect_summary_group(lines: &[RenderedSummaryLine], is_user: bool) -> Vec<String> {
    lines
        .iter()
        .filter(|line| line.is_user == is_user)
        .map(|line| line.text.clone())
        .collect::<Vec<_>>()
}

fn append_summary_section(sections: &mut Vec<String>, heading: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }

    sections.push(heading.to_owned());

    for line in lines {
        let bullet_line = format!("- {line}");
        sections.push(bullet_line);
    }
}

fn render_summary_lines(turn: &WindowTurn) -> Vec<RenderedSummaryLine> {
    if is_internal_assistant_event_turn(turn) {
        return Vec::new();
    }

    if turn.content.trim_start().starts_with("Compacted ") {
        let lines = extract_prior_summary_lines(&turn.content);
        if !lines.is_empty() {
            return lines;
        }
        return vec![RenderedSummaryLine {
            text: format!("{}: {}", turn.role, PRIOR_COMPACTED_SUMMARY_PLACEHOLDER),
            is_user: turn.role == "user",
        }];
    }

    vec![RenderedSummaryLine {
        text: format!("{}: {}", turn.role, summarize_turn_content(&turn.content)),
        is_user: turn.role == "user",
    }]
}

fn is_compacted_summary_turn(turn: &WindowTurn) -> bool {
    turn.content.trim_start().starts_with("Compacted ")
}

fn is_internal_assistant_event_turn(turn: &WindowTurn) -> bool {
    if turn.role != "assistant" {
        return false;
    }

    let parsed = match serde_json::from_str::<serde_json::Value>(&turn.content) {
        Ok(value) => value,
        Err(_) => return false,
    };
    matches!(
        parsed.get("type").and_then(serde_json::Value::as_str),
        Some("conversation_event" | "tool_decision" | "tool_outcome")
    )
}

fn summarize_turn_content(content: &str) -> String {
    let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
    trim_to_chars(&normalized, SUMMARY_TURN_EXCERPT_CHARS)
}

fn extract_prior_summary_lines(content: &str) -> Vec<RenderedSummaryLine> {
    content
        .split_once('\n')
        .map(|(_, body)| body)
        .unwrap_or_default()
        .lines()
        .filter_map(normalize_prior_summary_line)
        .collect()
}

fn normalize_prior_summary_line(line: &str) -> Option<RenderedSummaryLine> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let bullet_trimmed = trimmed.strip_prefix("- ").unwrap_or(trimmed);
    if is_summary_metadata_line(bullet_trimmed) {
        return None;
    }

    let (role, content) = strip_repeated_summary_role_prefixes(bullet_trimmed);
    if role == "assistant" && is_internal_assistant_summary_content(content) {
        return None;
    }

    Some(RenderedSummaryLine {
        text: format!(
            "{role}: {}",
            trim_to_chars(content, SUMMARY_TURN_EXCERPT_CHARS)
        ),
        is_user: role == "user",
    })
}

fn is_summary_metadata_line(line: &str) -> bool {
    let scope_note = runtime_self_continuity::compaction_summary_scope_note();
    if line == scope_note {
        return true;
    }
    if line == USER_CONTEXT_HEADING {
        return true;
    }
    if line == ASSISTANT_PROGRESS_HEADING {
        return true;
    }
    if line.starts_with(OMITTED_CONTEXT_PREFIX) {
        return true;
    }
    false
}

fn strip_repeated_summary_role_prefixes(mut line: &str) -> (&str, &str) {
    let mut role = "user";
    loop {
        if let Some(rest) = line.strip_prefix("user:") {
            role = "user";
            line = rest.trim_start();
            continue;
        }
        if let Some(rest) = line.strip_prefix("assistant:") {
            role = "assistant";
            line = rest.trim_start();
            continue;
        }
        break;
    }
    (role, line)
}

fn is_internal_assistant_summary_content(content: &str) -> bool {
    let parsed = match serde_json::from_str::<serde_json::Value>(content) {
        Ok(value) => value,
        Err(_) => return false,
    };
    matches!(
        parsed.get("type").and_then(serde_json::Value::as_str),
        Some("conversation_event" | "tool_decision" | "tool_outcome")
    )
}

fn trim_to_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }

    if max_chars <= 3 {
        return value.chars().take(max_chars).collect();
    }

    let remaining_chars = max_chars - 3;
    let head_chars = remaining_chars / 2;
    let tail_chars = remaining_chars - head_chars;
    let char_count = value.chars().count();

    let prefix = value.chars().take(head_chars).collect::<String>();
    let suffix = value
        .chars()
        .skip(char_count.saturating_sub(tail_chars))
        .collect::<String>();

    format!("{prefix}...{suffix}")
}
