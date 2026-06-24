use super::SkillEntry;
use crate::chat::chat_surface::utils::*;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::ListItem,
};

#[derive(Debug, Clone)]
pub(super) struct SkillMatchEntry {
    pub(super) label: String,
    pub(super) description: String,
    pub(super) insertion: String,
    skill: SkillEntry,
    match_target: Option<SkillMatchTarget>,
}

pub(super) fn filtered_skill_items(skills: &[SkillEntry], query: &str) -> Vec<SkillMatchEntry> {
    let query = query.trim().to_ascii_lowercase();
    let mut matches = skills
        .iter()
        .filter_map(|skill| skill_match_target(skill, query.as_str()))
        .collect::<Vec<_>>();

    sort_skill_matches(matches.as_mut_slice(), query.is_empty());

    matches
        .into_iter()
        .map(|(skill, match_target)| {
            let adjusted_target = (!query.is_empty())
                .then_some(adjust_skill_match_target_for_label(match_target.clone(), 1));
            SkillMatchEntry {
                label: format!("${}", skill.name),
                description: format_skill_popup_description(skill, &match_target),
                insertion: format!("${} ", skill.name),
                skill: skill.clone(),
                match_target: adjusted_target,
            }
        })
        .collect()
}

pub(super) fn render_skill_item(
    item: &SkillMatchEntry,
    selected: bool,
    list_width: u16,
    default_label_truncate_len: usize,
) -> ListItem<'static> {
    let row = skill_row_text(item, list_width, default_label_truncate_len);
    let label_spans = render_match_highlight_spans(
        row.label.as_str(),
        item.match_target.clone().filter(|target| target.is_label()),
        skill_label_style(selected),
        skill_label_highlight_style(selected),
    );
    let description_spans = render_skill_description_spans(
        row.truncated_context.as_str(),
        row.description.as_str(),
        item.match_target.clone(),
        selected,
    );

    let mut spans = vec![Span::styled(
        if selected { "› " } else { "  " },
        Style::default().fg(if selected {
            SURFACE_CYAN
        } else {
            SURFACE_DIM_GRAY
        }),
    )];
    spans.extend(label_spans);
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        row.category_tag,
        skill_category_style(selected),
    ));
    if !row.description.is_empty() {
        spans.push(Span::raw(" "));
        spans.extend(description_spans);
    }
    ListItem::new(Line::from(spans))
}

pub(super) fn skill_popup_hint_line() -> Line<'static> {
    Line::from(vec![
        Span::raw("Press "),
        Span::styled(
            "Enter",
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" / "),
        Span::styled(
            "Tab",
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" to insert or "),
        Span::styled(
            "Esc",
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" to close"),
    ])
}

fn skill_match_target<'a>(
    skill: &'a SkillEntry,
    query: &str,
) -> Option<(&'a SkillEntry, SkillMatchTarget)> {
    if query.is_empty() {
        return Some((skill, SkillMatchTarget::Label(usize::MAX, 0)));
    }

    let name = skill.name.to_ascii_lowercase();
    let desc = skill.description.to_ascii_lowercase();
    if let Some(index) = name.find(query) {
        return Some((skill, SkillMatchTarget::Label(index, query.len())));
    }
    if let Some(indices) = fuzzy_match_positions(skill.name.as_str(), query) {
        return Some((skill, SkillMatchTarget::LabelFuzzy(indices)));
    }
    if let Some((index, term)) = skill.search_terms.iter().find_map(|term| {
        let lower = term.to_ascii_lowercase();
        lower.find(query).map(|index| (index, term.to_owned()))
    }) {
        return Some((skill, SkillMatchTarget::SearchTerm { index, term }));
    }
    desc.find(query)
        .map(|index| (skill, SkillMatchTarget::Description(index, query.len())))
}

fn sort_skill_matches(matches: &mut [(&SkillEntry, SkillMatchTarget)], empty_query: bool) {
    matches.sort_by(|(left_skill, left_target), (right_skill, right_target)| {
        if empty_query {
            skill_label_priority(left_skill)
                .cmp(&skill_label_priority(right_skill))
                .then_with(|| left_skill.name.cmp(&right_skill.name))
        } else {
            left_target
                .sort_priority()
                .cmp(&right_target.sort_priority())
                .then_with(|| left_target.match_index().cmp(&right_target.match_index()))
                .then_with(|| {
                    skill_label_priority(left_skill).cmp(&skill_label_priority(right_skill))
                })
                .then_with(|| left_skill.name.cmp(&right_skill.name))
        }
    });
}

struct SkillRowText {
    category_tag: String,
    label: String,
    truncated_context: String,
    description: String,
}

struct SkillDescriptionParts<'a> {
    context: &'a str,
    detail: &'a str,
    has_detail: bool,
}

impl<'a> SkillDescriptionParts<'a> {
    fn new(raw_description: &'a str) -> Self {
        let (context, detail) = split_description_context(raw_description);
        Self {
            context,
            detail,
            has_detail: !detail.is_empty(),
        }
    }
}

fn skill_row_text(
    item: &SkillMatchEntry,
    list_width: u16,
    default_label_truncate_len: usize,
) -> SkillRowText {
    let (category_tag, raw_description) = split_category_tag(item.description.as_str());
    let description = SkillDescriptionParts::new(raw_description);
    let available_after_prefix = list_width.saturating_sub(2) as usize;
    let label_budget = skill_label_budget(
        &item.skill,
        category_tag,
        raw_description,
        description.context,
        description.has_detail,
        available_after_prefix,
    );
    let label_limit = skill_label_max_width(&item.skill).min(default_label_truncate_len);
    let label = super::truncate(item.label.as_str(), label_limit.min(label_budget));
    let desc_available_width = skill_description_available_width(
        category_tag,
        raw_description,
        label.as_str(),
        available_after_prefix,
    );
    let context_limit = skill_context_limit(
        &item.skill,
        description.context,
        description.has_detail,
        desc_available_width,
    );
    let truncated_context = super::truncate(description.context, context_limit);
    let truncated_detail = truncated_description_detail(
        description.detail,
        truncated_context.as_str(),
        description.has_detail,
        desc_available_width,
    );

    SkillRowText {
        category_tag: category_tag.to_owned(),
        label,
        truncated_context: truncated_context.clone(),
        description: compose_skill_description(
            description.context,
            truncated_context,
            truncated_detail,
            description.has_detail,
        ),
    }
}

fn skill_label_budget(
    skill: &SkillEntry,
    category_tag: &str,
    raw_description: &str,
    description_context: &str,
    source_has_detail: bool,
    available_after_prefix: usize,
) -> usize {
    let estimated_context_width =
        crate::presentation::display_width(description_context).min(skill_context_max_width(skill));
    available_after_prefix
        .saturating_sub(
            2 + crate::presentation::display_width(category_tag)
                + usize::from(!raw_description.is_empty())
                + estimated_context_width
                + description_context_separator_width(description_context, source_has_detail),
        )
        .max(1)
}

fn skill_description_available_width(
    category_tag: &str,
    raw_description: &str,
    label: &str,
    available_after_prefix: usize,
) -> usize {
    available_after_prefix.saturating_sub(
        crate::presentation::display_width(label)
            + 2
            + crate::presentation::display_width(category_tag)
            + usize::from(!raw_description.is_empty()),
    )
}

fn skill_context_limit(
    skill: &SkillEntry,
    description_context: &str,
    source_has_detail: bool,
    desc_available_width: usize,
) -> usize {
    let separator_width =
        description_context_separator_width(description_context, source_has_detail);
    let max_context_by_remaining = desc_available_width
        .saturating_sub(
            separator_width + minimum_detail_width(description_context, source_has_detail),
        )
        .max(1);
    let max_context_by_balance = if source_has_detail && description_context.len() > 12 {
        (desc_available_width / 2).max(8)
    } else {
        max_context_by_remaining
    };
    skill_context_max_width(skill).min(max_context_by_remaining.min(max_context_by_balance).max(1))
}

fn truncated_description_detail(
    description_detail: &str,
    truncated_context: &str,
    source_has_detail: bool,
    desc_available_width: usize,
) -> String {
    let separator_width = if !truncated_context.is_empty() && source_has_detail {
        3
    } else {
        0
    };
    let context_width = crate::presentation::display_width(truncated_context);
    let max_desc = desc_available_width.saturating_sub(context_width + separator_width);
    super::truncate(description_detail, max_desc)
}

fn compose_skill_description(
    description_context: &str,
    truncated_context: String,
    truncated_detail: String,
    source_has_detail: bool,
) -> String {
    if description_context.is_empty() {
        truncated_detail
    } else if !source_has_detail || truncated_detail.is_empty() {
        truncated_context
    } else {
        format!("{truncated_context} · {truncated_detail}")
    }
}

fn description_context_separator_width(
    description_context: &str,
    source_has_detail: bool,
) -> usize {
    if !description_context.is_empty() && source_has_detail {
        3
    } else {
        0
    }
}

fn minimum_detail_width(description_context: &str, source_has_detail: bool) -> usize {
    if source_has_detail && !description_context.is_empty() {
        2
    } else if source_has_detail {
        6
    } else {
        0
    }
}

fn skill_label_max_width(skill: &SkillEntry) -> usize {
    match skill.category_tag.as_str() {
        "[Plugin]" | "[Connector]" => 22,
        _ => 24,
    }
}

fn skill_context_max_width(skill: &SkillEntry) -> usize {
    match skill.category_tag.as_str() {
        "[Plugin]" | "[Connector]" => 14,
        _ => 18,
    }
}

fn skill_label_priority(skill: &SkillEntry) -> u8 {
    let category = skill.category_tag.to_ascii_lowercase();
    if category.contains("repo") {
        0
    } else if category.contains("plugin") {
        1
    } else if category.contains("connector") {
        2
    } else if category.contains("skill") {
        3
    } else if skill.name.contains("browser") {
        4
    } else {
        5
    }
}

fn format_skill_popup_description(skill: &SkillEntry, match_target: &SkillMatchTarget) -> String {
    match match_target {
        SkillMatchTarget::SearchTerm { term, .. } if term != &skill.name => {
            format!("{} {} · {}", skill.category_tag, term, skill.description)
        }
        SkillMatchTarget::Label(..)
        | SkillMatchTarget::LabelFuzzy(..)
        | SkillMatchTarget::SearchTerm { .. }
        | SkillMatchTarget::Description(..) => {
            if let Some(alias) = skill.source_alias.as_deref() {
                format!("{} {} · {}", skill.category_tag, alias, skill.description)
            } else {
                format!("{} {}", skill.category_tag, skill.description)
            }
        }
    }
}

fn split_category_tag(description: &str) -> (&str, &str) {
    let trimmed = description.trim_start();
    if let Some(rest) = trimmed.strip_prefix('[')
        && let Some((tag_body, remainder)) = rest.split_once(']')
    {
        let tag_len = tag_body.len() + 2;
        let (tag, _) = trimmed.split_at(tag_len);
        return (tag, remainder.trim_start());
    }
    ("", trimmed)
}

fn adjust_skill_match_target_for_label(
    match_target: SkillMatchTarget,
    offset: usize,
) -> SkillMatchTarget {
    match match_target {
        SkillMatchTarget::Label(start, len) => {
            SkillMatchTarget::Label(start.saturating_add(offset), len)
        }
        SkillMatchTarget::LabelFuzzy(indices) => SkillMatchTarget::LabelFuzzy(
            indices
                .into_iter()
                .map(|index| index.saturating_add(offset))
                .collect(),
        ),
        other @ SkillMatchTarget::SearchTerm { .. } | other @ SkillMatchTarget::Description(..) => {
            other
        }
    }
}

fn split_description_context(description: &str) -> (&str, &str) {
    description
        .split_once(" · ")
        .map(|(context, detail)| (context.trim_end(), detail.trim_start()))
        .unwrap_or(("", description))
}

fn render_match_highlight_spans(
    text: &str,
    match_target: Option<SkillMatchTarget>,
    normal_style: Style,
    highlight_style: Style,
) -> Vec<Span<'static>> {
    let Some(ranges) = match_target.and_then(|target| target.highlight_ranges(text)) else {
        return vec![Span::styled(text.to_owned(), normal_style)];
    };
    let mut spans = Vec::new();
    let mut cursor = 0usize;
    for range in ranges {
        if range.start > cursor {
            spans.push(Span::styled(
                text[cursor..range.start].to_owned(),
                normal_style,
            ));
        }
        spans.push(Span::styled(
            text[range.clone()].to_owned(),
            highlight_style,
        ));
        cursor = range.end;
    }
    if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_owned(), normal_style));
    }
    spans
}

fn skill_category_style(selected: bool) -> Style {
    Style::default().fg(if selected {
        SURFACE_GRAY
    } else {
        SURFACE_DIM_GRAY
    })
}

fn skill_label_style(selected: bool) -> Style {
    Style::default()
        .fg(if selected {
            SURFACE_CYAN
        } else {
            ratatui::style::Color::White
        })
        .add_modifier(if selected {
            Modifier::BOLD
        } else {
            Modifier::empty()
        })
}

fn skill_label_highlight_style(selected: bool) -> Style {
    Style::default()
        .fg(if selected {
            SURFACE_CYAN
        } else {
            SURFACE_ACCENT
        })
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

fn skill_context_style(selected: bool) -> Style {
    Style::default().fg(if selected {
        SURFACE_GRAY
    } else {
        SURFACE_DIM_GRAY
    })
}

fn skill_separator_style() -> Style {
    Style::default().fg(SURFACE_DIM_GRAY)
}

fn skill_description_style(selected: bool) -> Style {
    Style::default().fg(if selected {
        SURFACE_ACCENT
    } else {
        SURFACE_GRAY
    })
}

fn skill_highlight_style() -> Style {
    Style::default()
        .fg(ratatui::style::Color::White)
        .add_modifier(Modifier::UNDERLINED)
}

fn render_skill_description_spans(
    context: &str,
    full_description: &str,
    match_target: Option<SkillMatchTarget>,
    selected: bool,
) -> Vec<Span<'static>> {
    let (context_part, detail_part) = split_description_context(full_description);
    let highlighted_range = description_match_target(full_description, match_target)
        .and_then(|target| target.highlight_ranges(full_description))
        .and_then(|mut ranges| ranges.drain(..).next());
    let mut spans = Vec::new();
    if !context_part.is_empty() {
        spans.extend(render_segment_highlight_spans(
            context_part,
            0,
            skill_context_style(selected),
            skill_highlight_style(),
            highlighted_range.clone(),
        ));
    }
    if !context_part.is_empty() && !detail_part.is_empty() {
        spans.push(Span::styled(" · ", skill_separator_style()));
    }
    append_skill_detail_spans(
        &mut spans,
        context_part,
        detail_part,
        highlighted_range,
        selected,
    );
    if context_part.is_empty() && detail_part.is_empty() && !context.is_empty() {
        spans.push(Span::styled(
            context.to_owned(),
            skill_context_style(selected),
        ));
    }
    spans
}

fn append_skill_detail_spans(
    spans: &mut Vec<Span<'static>>,
    context_part: &str,
    detail_part: &str,
    highlighted_range: Option<std::ops::Range<usize>>,
    selected: bool,
) {
    if detail_part.is_empty() {
        return;
    }

    let detail_start = if context_part.is_empty() {
        0
    } else {
        context_part.len() + 3
    };
    spans.extend(render_segment_highlight_spans(
        detail_part,
        detail_start,
        skill_description_style(selected),
        skill_highlight_style(),
        highlighted_range,
    ));
}

fn render_segment_highlight_spans(
    text: &str,
    offset: usize,
    normal_style: Style,
    highlight_style: Style,
    highlight_range: Option<std::ops::Range<usize>>,
) -> Vec<Span<'static>> {
    let Some(range) = highlight_range else {
        return vec![Span::styled(text.to_owned(), normal_style)];
    };
    let segment_start = offset;
    let segment_end = offset.saturating_add(text.len());
    let overlap_start = range.start.max(segment_start);
    let overlap_end = range.end.min(segment_end);
    if overlap_start >= overlap_end {
        return vec![Span::styled(text.to_owned(), normal_style)];
    }

    let relative_start = overlap_start.saturating_sub(segment_start);
    let relative_end = overlap_end.saturating_sub(segment_start);
    let mut spans = Vec::new();
    if relative_start > 0 {
        spans.push(Span::styled(
            text[..relative_start].to_owned(),
            normal_style,
        ));
    }
    spans.push(Span::styled(
        text[relative_start..relative_end].to_owned(),
        highlight_style,
    ));
    if relative_end < text.len() {
        spans.push(Span::styled(text[relative_end..].to_owned(), normal_style));
    }
    spans
}

fn fuzzy_match_positions(text: &str, query: &str) -> Option<Vec<usize>> {
    if query.is_empty() {
        return Some(Vec::new());
    }

    let query_chars = query
        .chars()
        .map(|ch| ch.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut matched_positions = Vec::new();
    let mut query_index = 0usize;

    for (index, ch) in text.char_indices() {
        let Some(expected) = query_chars.get(query_index) else {
            break;
        };
        if ch.to_ascii_lowercase() == *expected {
            matched_positions.push(index);
            query_index += 1;
        }
    }

    (query_index == query_chars.len()).then_some(matched_positions)
}

fn description_match_target(
    description: &str,
    match_target: Option<SkillMatchTarget>,
) -> Option<SkillMatchTarget> {
    match match_target {
        Some(SkillMatchTarget::SearchTerm { term, .. }) => {
            let lower_description = description.to_ascii_lowercase();
            let lower_term = term.to_ascii_lowercase();
            lower_description
                .find(lower_term.as_str())
                .map(|index| SkillMatchTarget::Description(index, term.len()))
        }
        other => other.filter(|target| target.is_description()),
    }
}

#[derive(Debug, Clone)]
enum SkillMatchTarget {
    Label(usize, usize),
    LabelFuzzy(Vec<usize>),
    SearchTerm { index: usize, term: String },
    Description(usize, usize),
}

impl SkillMatchTarget {
    fn sort_priority(&self) -> usize {
        match self {
            Self::Label(..) => 0,
            Self::LabelFuzzy(..) => 1,
            Self::SearchTerm { .. } => 2,
            Self::Description(..) => 3,
        }
    }

    fn match_index(&self) -> usize {
        match self {
            Self::Label(index, _) | Self::Description(index, _) => *index,
            Self::LabelFuzzy(indices) => indices.first().copied().unwrap_or(usize::MAX),
            Self::SearchTerm { index, .. } => *index,
        }
    }

    fn is_label(&self) -> bool {
        matches!(self, Self::Label(..) | Self::LabelFuzzy(..))
    }

    fn is_description(&self) -> bool {
        matches!(self, Self::Description(..))
    }

    fn highlight_ranges(&self, text: &str) -> Option<Vec<std::ops::Range<usize>>> {
        match self {
            Self::Label(start, len) | Self::Description(start, len) => {
                let end = start.saturating_add(*len);
                if *start <= text.len()
                    && end <= text.len()
                    && text.is_char_boundary(*start)
                    && text.is_char_boundary(end)
                {
                    Some(std::iter::once(*start..end).collect())
                } else {
                    None
                }
            }
            Self::LabelFuzzy(indices) => {
                let mut ranges = Vec::new();
                for start in indices {
                    if !text.is_char_boundary(*start) {
                        return None;
                    }
                    let end = text[*start..]
                        .chars()
                        .next()
                        .map(|ch| start.saturating_add(ch.len_utf8()))?;
                    ranges.push(*start..end);
                }
                Some(ranges)
            }
            Self::SearchTerm { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;

    fn skill(name: &str, description: &str) -> SkillEntry {
        SkillEntry {
            name: name.to_owned(),
            description: description.to_owned(),
            search_terms: vec![name.to_owned()],
            category_tag: "[Skill]".to_owned(),
            source_alias: None,
        }
    }

    fn plugin(name: &str, description: &str) -> SkillEntry {
        SkillEntry {
            name: name.to_owned(),
            description: description.to_owned(),
            search_terms: vec![name.to_owned()],
            category_tag: "[Plugin]".to_owned(),
            source_alias: None,
        }
    }

    #[test]
    fn filtered_skill_items_return_insertable_mentions() {
        let items = filtered_skill_items(&[skill("demo-skill", "demo query description")], "demo");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "$demo-skill");
        assert_eq!(items[0].insertion, "$demo-skill ");
    }

    #[test]
    fn split_category_tag_separates_tag_from_description() {
        let (tag, description) = split_category_tag("[Skill] demo description");
        assert_eq!(tag, "[Skill]");
        assert_eq!(description, "demo description");

        let (tag, description) = split_category_tag("demo description");
        assert_eq!(tag, "");
        assert_eq!(description, "demo description");
    }

    #[test]
    fn split_description_context_preserves_alias_prefix() {
        let (context, detail) = split_description_context("babysit-pr · triage pull requests");
        assert_eq!(context, "babysit-pr");
        assert_eq!(detail, "triage pull requests");

        let (context, detail) = split_description_context("plain description");
        assert_eq!(context, "");
        assert_eq!(detail, "plain description");
    }

    #[test]
    fn adjust_skill_match_target_for_label_offsets_name_matches() {
        let target = adjust_skill_match_target_for_label(SkillMatchTarget::Label(0, 4), 1);

        assert!(matches!(target, SkillMatchTarget::Label(1, 4)));
    }

    #[test]
    fn render_match_highlight_spans_splits_highlighted_segment() {
        let spans = render_match_highlight_spans(
            "$demo-skill",
            Some(SkillMatchTarget::Label(1, 4)),
            Style::default(),
            Style::default().add_modifier(Modifier::BOLD),
        );

        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content.as_ref(), "$");
        assert_eq!(spans[1].content.as_ref(), "demo");
        assert_eq!(spans[2].content.as_ref(), "-skill");
    }

    #[test]
    fn render_match_highlight_spans_supports_fuzzy_positions() {
        let spans = render_match_highlight_spans(
            "$github",
            Some(SkillMatchTarget::LabelFuzzy(vec![1, 3, 6])),
            Style::default(),
            Style::default().add_modifier(Modifier::BOLD),
        );

        assert_eq!(spans.len(), 6);
        assert_eq!(spans[0].content.as_ref(), "$");
        assert_eq!(spans[1].content.as_ref(), "g");
        assert_eq!(spans[2].content.as_ref(), "i");
        assert_eq!(spans[3].content.as_ref(), "t");
        assert_eq!(spans[4].content.as_ref(), "hu");
        assert_eq!(spans[5].content.as_ref(), "b");
    }

    #[test]
    fn skill_highlight_style_uses_underlined_white_text() {
        let style = skill_highlight_style();
        assert_eq!(style.fg, Some(ratatui::style::Color::White));
        assert!(style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn skill_label_highlight_style_uses_accent_when_unselected() {
        let style = skill_label_highlight_style(false);
        assert_eq!(style.fg, Some(SURFACE_ACCENT));
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert!(style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn skill_label_highlight_style_uses_cyan_when_selected() {
        let style = skill_label_highlight_style(true);
        assert_eq!(style.fg, Some(SURFACE_CYAN));
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert!(style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn skill_label_style_selected_matches_popup_emphasis() {
        let style = skill_label_style(true);
        assert_eq!(style.fg, Some(SURFACE_CYAN));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn skill_category_style_dims_unselected_rows() {
        assert_eq!(skill_category_style(false).fg, Some(SURFACE_DIM_GRAY));
        assert_eq!(skill_category_style(true).fg, Some(SURFACE_GRAY));
    }

    #[test]
    fn render_skill_description_spans_splits_context_and_detail_without_match() {
        let spans = render_skill_description_spans(
            "babysit-pr",
            "babysit-pr · triage pull requests",
            None,
            false,
        );

        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content.as_ref(), "babysit-pr");
        assert_eq!(spans[1].content.as_ref(), " · ");
        assert_eq!(spans[2].content.as_ref(), "triage pull requests");
        assert_eq!(spans[0].style.fg, Some(SURFACE_DIM_GRAY));
        assert_eq!(spans[2].style.fg, Some(SURFACE_GRAY));
    }

    #[test]
    fn render_skill_description_spans_preserves_context_and_detail_styles_around_highlight() {
        let spans = render_skill_description_spans(
            "babysit-pr",
            "babysit-pr · triage pull requests",
            Some(SkillMatchTarget::SearchTerm {
                index: 0,
                term: "babysit-pr".to_owned(),
            }),
            false,
        );

        assert_eq!(spans[0].content.as_ref(), "babysit-pr");
        assert_eq!(spans[1].content.as_ref(), " · ");
        assert_eq!(spans[2].content.as_ref(), "triage pull requests");
        assert!(spans[0].style.add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(spans[2].style.fg, Some(SURFACE_GRAY));
    }

    #[test]
    fn description_match_target_uses_alias_term_when_search_term_matched() {
        let target = description_match_target(
            "babysit-pr · triage pull requests",
            Some(SkillMatchTarget::SearchTerm {
                index: 0,
                term: "babysit-pr".to_owned(),
            }),
        );

        assert!(matches!(
            target,
            Some(SkillMatchTarget::Description(0, len)) if len == "babysit-pr".len()
        ));
    }

    #[test]
    fn plugin_labels_use_compact_width_budget() {
        let plugin = plugin("very-long-plugin-name-for-popup", "plugin description");
        assert_eq!(skill_label_max_width(&plugin), 22);

        let skill = skill("very-long-skill-name-for-popup", "skill description");
        assert_eq!(skill_label_max_width(&skill), 24);
    }

    #[test]
    fn plugin_context_uses_compact_width_budget() {
        let plugin = plugin("very-long-plugin-name-for-popup", "plugin description");
        assert_eq!(skill_context_max_width(&plugin), 14);

        let skill = skill("very-long-skill-name-for-popup", "skill description");
        assert_eq!(skill_context_max_width(&skill), 18);
    }

    #[test]
    fn skill_popup_alias_match_highlights_alias_in_description() {
        let target = description_match_target(
            "babysit-pr · triage pull requests",
            Some(SkillMatchTarget::SearchTerm {
                index: 0,
                term: "babysit-pr".to_owned(),
            }),
        );
        let spans = render_match_highlight_spans(
            "babysit-pr · triage pull requests",
            target,
            Style::default(),
            Style::default().add_modifier(Modifier::UNDERLINED),
        );

        assert_eq!(spans[0].content.as_ref(), "babysit-pr");
        assert!(spans[0].style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn format_skill_popup_description_includes_alias_when_present() {
        let skill = SkillEntry {
            name: "PR Babysitter".to_owned(),
            description: "triage pull requests".to_owned(),
            search_terms: vec!["PR Babysitter".to_owned(), "babysit-pr".to_owned()],
            category_tag: "[Skill]".to_owned(),
            source_alias: Some("babysit-pr".to_owned()),
        };

        let description = format_skill_popup_description(&skill, &SkillMatchTarget::Label(0, 2));
        assert_eq!(description, "[Skill] babysit-pr · triage pull requests");
    }
}
