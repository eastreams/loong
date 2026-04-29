use super::compaction_preparation::prepare_compaction_window;
use super::tool_result_line::ToolResultLine;
use crate::config::LoongConfig;
use crate::memory::WindowTurn;
use crate::runtime_self_continuity;

pub(crate) const COMPACTED_SUMMARY_PREFIX: &str = "Compacted ";
const SUMMARY_MAX_RENDERED_TURNS: usize = 4;
const SUMMARY_PREFERRED_USER_TURNS: usize = 3;
const SUMMARY_PREFERRED_ASSISTANT_TURNS: usize = 1;
const SUMMARY_TURN_EXCERPT_CHARS: usize = 96;
const COMPACTED_SUMMARY_MAX_CHARS: usize = 511;
const SUMMARY_TOTAL_CHARS_MAX: usize = COMPACTED_SUMMARY_MAX_CHARS;
const PRIOR_COMPACTED_SUMMARY_PLACEHOLDER: &str = "[prior compacted summary]";
pub(crate) const COMPACTED_SUMMARY_MARKER: &str = "[session_local_recall_compacted_window]";
const COMPACTED_SUMMARY_DISCLAIMER: &str =
    "This compacted checkpoint is session-local recall only.";
const USER_CONTEXT_HEADING: &str = "User context:";
const ASSISTANT_PROGRESS_HEADING: &str = "Assistant progress:";
const OMITTED_CONTEXT_PREFIX: &str = "More omitted context:";
pub(super) const MIN_ADAPTIVE_RECENT_TAIL_TOKEN_BUDGET: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactPolicy {
    preserve_recent_turns: usize,
    preserve_recent_estimated_tokens: Option<usize>,
}

impl CompactPolicy {
    pub fn new(preserve_recent_turns: usize) -> Self {
        Self {
            preserve_recent_turns,
            preserve_recent_estimated_tokens: None,
        }
    }

    pub fn with_recent_token_budget(
        mut self,
        preserve_recent_estimated_tokens: Option<usize>,
    ) -> Self {
        self.preserve_recent_estimated_tokens =
            preserve_recent_estimated_tokens.filter(|value| *value > 0);
        self
    }

    pub(super) fn preserve_recent_turns(self) -> usize {
        self.preserve_recent_turns
    }

    pub(super) fn preserve_recent_estimated_tokens(self) -> Option<usize> {
        self.preserve_recent_estimated_tokens
    }
}

pub(super) fn compaction_policy_from_config(config: &LoongConfig) -> Option<CompactPolicy> {
    let preserve_recent_turns = config
        .conversation
        .compact_preserve_recent_turns()
        .min(config.memory.sliding_window.saturating_sub(1));
    if preserve_recent_turns == 0 {
        return None;
    }
    let recent_token_budget = config
        .conversation
        .compact_preserve_recent_estimated_tokens()
        .filter(|threshold| *threshold >= MIN_ADAPTIVE_RECENT_TAIL_TOKEN_BUDGET);
    Some(CompactPolicy::new(preserve_recent_turns).with_recent_token_budget(recent_token_budget))
}

pub fn compact_window(turns: &[WindowTurn], policy: CompactPolicy) -> Option<Vec<WindowTurn>> {
    let preparation = prepare_compaction_window(turns, policy)?;
    if preparation.summary_turns.len() == 1
        && preparation
            .summary_turns
            .first()
            .is_some_and(is_compacted_summary_turn)
    {
        return None;
    }

    let summary = WindowTurn {
        role: "user".to_owned(),
        content: render_compacted_summary(
            preparation.compacted_turn_count(),
            preparation.summary_turns.as_slice(),
        ),
        ts: preparation.summary_turns.last().and_then(|turn| turn.ts),
    };

    let mut compacted = Vec::with_capacity(preparation.retained_turns.len() + 1);
    compacted.push(summary);
    compacted.extend(preparation.retained_turns);
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
    render_structured_summary(&selected_lines, all_lines.len())
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

fn render_compacted_summary(compacted_turn_count: usize, turns: &[WindowTurn]) -> String {
    let rendered_summary = render_summary(turns);
    let header_sections = [
        COMPACTED_SUMMARY_MARKER.to_owned(),
        COMPACTED_SUMMARY_DISCLAIMER.to_owned(),
        format!("Compacted {compacted_turn_count} earlier turns"),
    ];
    let header = header_sections.join("\n");
    let available_summary_chars =
        COMPACTED_SUMMARY_MAX_CHARS.saturating_sub(header.chars().count().saturating_add(1));
    let bounded_summary =
        bound_compacted_summary_body(rendered_summary.as_str(), available_summary_chars);
    let mut sections = Vec::new();

    sections.extend(header_sections);
    sections.push(bounded_summary);

    sections.join("\n")
}

fn bound_compacted_summary_body(summary: &str, max_chars: usize) -> String {
    let mut summary_lines = summary.lines().collect::<Vec<_>>();
    let omitted_line = summary_lines
        .last()
        .copied()
        .filter(|line| line.starts_with("... ") || line.starts_with(OMITTED_CONTEXT_PREFIX));

    if omitted_line.is_some() {
        summary_lines.pop();
    }

    let reserved_omitted_chars = omitted_line
        .map(|line| line.chars().count().saturating_add(1))
        .unwrap_or(0);
    let body_limit = max_chars.saturating_sub(reserved_omitted_chars);
    let mut bounded_lines = Vec::new();
    let mut budget = SummaryCharBudget::new(body_limit);

    for line in summary_lines {
        if !push_summary_line(&mut bounded_lines, &mut budget, line) {
            break;
        }
    }

    if let Some(omitted_line) = omitted_line {
        if bounded_lines.is_empty() {
            return trim_to_chars(omitted_line, max_chars);
        }

        bounded_lines.push(omitted_line.to_owned());
    }

    bounded_lines.join("\n")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderedSummaryLine {
    text: String,
    is_user: bool,
}

fn render_structured_summary(lines: &[RenderedSummaryLine], total_line_count: usize) -> String {
    let scope_note = runtime_self_continuity::compaction_summary_scope_note();
    let max_omitted_count_width = total_line_count.to_string().chars().count();
    let omitted_line = format!(
        "{OMITTED_CONTEXT_PREFIX} {} earlier turns omitted.",
        "9".repeat(max_omitted_count_width.max(1))
    );
    let reserved_chars = omitted_line.chars().count().saturating_add(1);
    let content_budget_limit = SUMMARY_TOTAL_CHARS_MAX.saturating_sub(reserved_chars);
    let mut budget = SummaryCharBudget::new(content_budget_limit);
    let mut sections = Vec::new();
    let mut rendered_selected_lines = 0usize;

    push_summary_line(&mut sections, &mut budget, scope_note);

    let user_lines = collect_summary_group(lines, true);
    let user_rendered_lines = append_summary_section(
        &mut sections,
        &mut budget,
        USER_CONTEXT_HEADING,
        &user_lines,
    );
    rendered_selected_lines += user_rendered_lines;

    let assistant_lines = collect_summary_group(lines, false);
    let assistant_rendered_lines = append_summary_section(
        &mut sections,
        &mut budget,
        ASSISTANT_PROGRESS_HEADING,
        &assistant_lines,
    );
    rendered_selected_lines += assistant_rendered_lines;

    let omitted_lines = total_line_count.saturating_sub(rendered_selected_lines);
    if omitted_lines > 0 {
        let omitted_line =
            format!("{OMITTED_CONTEXT_PREFIX} {omitted_lines} earlier turns omitted.");
        sections.push(omitted_line);
    }

    sections.join("\n")
}

fn collect_summary_group(lines: &[RenderedSummaryLine], is_user: bool) -> Vec<String> {
    lines
        .iter()
        .filter(|line| line.is_user == is_user)
        .map(|line| line.text.clone())
        .collect::<Vec<_>>()
}

fn append_summary_section(
    sections: &mut Vec<String>,
    budget: &mut SummaryCharBudget,
    heading: &str,
    lines: &[String],
) -> usize {
    if lines.is_empty() {
        return 0;
    }

    if !push_summary_line(sections, budget, heading) {
        return 0;
    }

    let mut rendered_lines = 0usize;
    for line in lines {
        let bullet_line = format!("- {line}");
        if !push_summary_line(sections, budget, &bullet_line) {
            return rendered_lines;
        }
        rendered_lines += 1;
    }
    rendered_lines
}

#[derive(Debug, Clone, Copy)]
struct SummaryCharBudget {
    remaining: usize,
    has_lines: bool,
}

impl SummaryCharBudget {
    fn new(limit: usize) -> Self {
        Self {
            remaining: limit,
            has_lines: false,
        }
    }
}

fn push_summary_line(
    sections: &mut Vec<String>,
    budget: &mut SummaryCharBudget,
    line: &str,
) -> bool {
    if budget.remaining == 0 {
        return false;
    }

    let separator_chars = if budget.has_lines { 1 } else { 0 };
    if budget.remaining <= separator_chars {
        return false;
    }

    let available_chars = budget.remaining - separator_chars;
    let rendered_line = trim_to_chars(line, available_chars);
    let rendered_chars = rendered_line.chars().count();
    if rendered_chars == 0 {
        return false;
    }

    sections.push(rendered_line);
    budget.remaining -= separator_chars + rendered_chars;
    budget.has_lines = true;
    true
}

fn render_summary_lines(turn: &WindowTurn) -> Vec<RenderedSummaryLine> {
    if is_internal_assistant_event_turn(turn) {
        return Vec::new();
    }

    if is_compacted_summary_content(turn.content.as_str()) {
        let lines = extract_prior_summary_lines(&turn.content);
        if !lines.is_empty() {
            return lines;
        }
        return vec![RenderedSummaryLine {
            text: format!("{}: {}", turn.role, PRIOR_COMPACTED_SUMMARY_PLACEHOLDER),
            is_user: turn.role == "user",
        }];
    }

    let summarized_content = summarize_tool_result_line_content(&turn.content)
        .unwrap_or_else(|| summarize_turn_content(&turn.content));
    vec![RenderedSummaryLine {
        text: format!("{}: {}", turn.role, summarized_content),
        is_user: turn.role == "user",
    }]
}

fn summarize_tool_result_line_content(content: &str) -> Option<String> {
    let tool_result_line = ToolResultLine::parse(content.trim())?;
    let canonical_tool_name = crate::tools::canonical_tool_name(tool_result_line.tool_name());
    let visible_tool_name = crate::tools::user_visible_tool_name(canonical_tool_name);
    let payload_summary = tool_result_line.payload_summary_str().trim();
    if payload_summary.is_empty() {
        return Some(visible_tool_name);
    }

    let normalized_payload_summary = payload_summary
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let reserved_chars = visible_tool_name.chars().count().saturating_add(3);
    let available_summary_chars = SUMMARY_TURN_EXCERPT_CHARS
        .saturating_sub(reserved_chars)
        .max(16);
    let bounded_payload_summary =
        trim_to_chars(&normalized_payload_summary, available_summary_chars);
    Some(format!("{visible_tool_name}: {bounded_payload_summary}"))
}

fn is_compacted_summary_turn(turn: &WindowTurn) -> bool {
    is_compacted_summary_content(turn.content.as_str())
}

pub(crate) fn is_compacted_summary_content(content: &str) -> bool {
    let trimmed = content.trim_start();
    let has_marker = trimmed.starts_with(COMPACTED_SUMMARY_MARKER);
    let has_legacy_prefix = trimmed.starts_with(COMPACTED_SUMMARY_PREFIX);

    has_marker || has_legacy_prefix
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
    if line == COMPACTED_SUMMARY_MARKER {
        return true;
    }
    if line == COMPACTED_SUMMARY_DISCLAIMER {
        return true;
    }
    if line.starts_with(COMPACTED_SUMMARY_PREFIX) {
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
    if is_legacy_omitted_turns_marker(line) {
        return true;
    }
    false
}

fn is_legacy_omitted_turns_marker(line: &str) -> bool {
    let Some(stripped_line) = line.strip_prefix("... ") else {
        return false;
    };

    let Some((count, suffix)) = stripped_line.split_once(' ') else {
        return false;
    };
    if count.parse::<usize>().is_err() {
        return false;
    }

    suffix == "earlier turns omitted"
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
