use crate::memory::WindowTurn;

const SUMMARY_MAX_RENDERED_TURNS: usize = 4;
const SUMMARY_TURN_EXCERPT_CHARS: usize = 96;
const PRIOR_COMPACTED_SUMMARY_PLACEHOLDER: &str = "[prior compacted summary]";

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
    let mut selected_indices = all_lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| line.is_user.then_some(idx))
        .take(SUMMARY_MAX_RENDERED_TURNS)
        .collect::<Vec<_>>();
    if selected_indices.len() < SUMMARY_MAX_RENDERED_TURNS {
        let remaining = SUMMARY_MAX_RENDERED_TURNS - selected_indices.len();
        selected_indices.extend(
            all_lines
                .iter()
                .enumerate()
                .filter_map(|(idx, line)| (!line.is_user).then_some(idx))
                .take(remaining),
        );
    }
    selected_indices.sort_unstable();

    let mut lines = selected_indices
        .into_iter()
        .filter_map(|idx| all_lines.get(idx).map(|line| line.text.clone()))
        .collect::<Vec<_>>();
    let omitted_turns = all_lines.len().saturating_sub(lines.len());
    if omitted_turns > 0 {
        lines.push(format!("... {} earlier turns omitted", omitted_turns));
    }
    lines.join("\n")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderedSummaryLine {
    text: String,
    is_user: bool,
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
    if trimmed.is_empty() || trimmed.starts_with("... ") {
        return None;
    }

    let (role, content) = strip_repeated_summary_role_prefixes(trimmed);
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

    let mut trimmed = value.chars().take(max_chars - 3).collect::<String>();
    trimmed.push_str("...");
    trimmed
}
