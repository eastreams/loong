use std::io::IsTerminal;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TuiHeaderStyle {
    Brand,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TuiCalloutTone {
    Info,
    Success,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiChoiceSpec {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub detail_lines: Vec<String>,
    #[serde(default)]
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TuiKeyValueSpec {
    Plain { key: String, value: String },
    Csv { key: String, values: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiActionSpec {
    pub label: String,
    pub command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TuiChecklistStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiChecklistItemSpec {
    pub status: TuiChecklistStatus,
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TuiSectionSpec {
    Narrative {
        title: Option<String>,
        #[serde(default)]
        lines: Vec<String>,
    },
    KeyValues {
        title: Option<String>,
        #[serde(default)]
        items: Vec<TuiKeyValueSpec>,
    },
    ActionGroup {
        title: Option<String>,
        #[serde(default)]
        inline_title_when_wide: bool,
        #[serde(default)]
        items: Vec<TuiActionSpec>,
    },
    Checklist {
        title: Option<String>,
        #[serde(default)]
        items: Vec<TuiChecklistItemSpec>,
    },
    Callout {
        tone: TuiCalloutTone,
        title: Option<String>,
        #[serde(default)]
        lines: Vec<String>,
    },
    Preformatted {
        title: Option<String>,
        language: Option<String>,
        #[serde(default)]
        lines: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiScreenSpec {
    pub header_style: TuiHeaderStyle,
    pub subtitle: Option<String>,
    pub title: Option<String>,
    pub progress_line: Option<String>,
    #[serde(default)]
    pub intro_lines: Vec<String>,
    #[serde(default)]
    pub sections: Vec<TuiSectionSpec>,
    #[serde(default)]
    pub choices: Vec<TuiChoiceSpec>,
    #[serde(default)]
    pub footer_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiMessageSpec {
    pub role: String,
    pub caption: Option<String>,
    #[serde(default)]
    pub sections: Vec<TuiSectionSpec>,
    #[serde(default)]
    pub footer_lines: Vec<String>,
}

const INLINE_ACTION_GROUP_WIDTH: usize = 56;

pub fn render_tui_screen_spec(
    spec: &TuiScreenSpec,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let subtitle = spec.subtitle.as_deref();
    let mut lines = render_header(spec.header_style, width, subtitle, color_enabled);

    if let Some(title) = spec.title.as_deref() {
        lines.push(String::new());
        lines.extend(render_wrapped_display_lines([title], width));
    }

    if let Some(progress_line) = spec.progress_line.as_deref() {
        lines.extend(render_wrapped_display_lines([progress_line], width));
    }

    if !spec.intro_lines.is_empty() {
        lines.extend(render_wrapped_display_lines(&spec.intro_lines, width));
    }

    for section in &spec.sections {
        append_section_lines(&mut lines, section, width);
    }

    if !spec.choices.is_empty() {
        lines.push(String::new());
        lines.extend(render_choice_lines(&spec.choices, width));
    }

    if !spec.footer_lines.is_empty() {
        lines.push(String::new());
        lines.extend(render_wrapped_display_lines(&spec.footer_lines, width));
    }

    lines
}

pub fn render_tui_screen_spec_ratatui(
    spec: &TuiScreenSpec,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let width = width.max(36);
    let header_lines = build_screen_header_lines(spec, width.saturating_sub(4), color_enabled);
    let mut blocks = Vec::new();

    for section in &spec.sections {
        let (title, lines) = build_screen_section_block(section, width.saturating_sub(4));
        blocks.push(RenderedScreenBlock { title, lines });
    }

    if !spec.choices.is_empty() {
        blocks.push(RenderedScreenBlock {
            title: Some("choices".to_owned()),
            lines: render_choice_lines(&spec.choices, width.saturating_sub(4)),
        });
    }

    if !spec.footer_lines.is_empty() {
        blocks.push(RenderedScreenBlock {
            title: Some("next".to_owned()),
            lines: render_wrapped_display_lines(&spec.footer_lines, width.saturating_sub(4)),
        });
    }

    let total_height = rendered_screen_height(&header_lines, &blocks);
    let area = Rect::new(0, 0, width as u16, total_height);
    let mut buffer = Buffer::empty(area);
    let mut row = 0_u16;

    let header_height = block_height(&header_lines);
    render_text_block(
        Rect::new(0, row, area.width, header_height),
        "loong",
        &header_lines,
        &mut buffer,
    );
    row = row.saturating_add(header_height);

    for block in &blocks {
        let height = block_height(&block.lines);
        render_text_block(
            Rect::new(0, row, area.width, height),
            block.title.as_deref().unwrap_or("section"),
            &block.lines,
            &mut buffer,
        );
        row = row.saturating_add(height);
    }

    render_buffer_to_lines(&buffer)
}

pub fn render_onboard_screen_spec(
    spec: &TuiScreenSpec,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    if color_enabled && std::io::stdout().is_terminal() {
        render_tui_screen_spec_ratatui(spec, width, color_enabled)
    } else {
        render_tui_screen_spec(spec, width, color_enabled)
    }
}

pub fn render_tui_message_spec(spec: &TuiMessageSpec, width: usize) -> Vec<String> {
    let mut lines = vec![render_message_heading(spec)];
    let body_lines = render_tui_message_body_spec(spec, width);

    lines.extend(body_lines);
    lines
}

pub fn render_tui_message_body_spec(spec: &TuiMessageSpec, width: usize) -> Vec<String> {
    let mut lines = Vec::new();

    for section in &spec.sections {
        append_section_lines(&mut lines, section, width);
    }

    if !spec.footer_lines.is_empty() {
        lines.push(String::new());
        lines.extend(render_wrapped_display_lines(&spec.footer_lines, width));
    }

    lines
}

fn build_screen_header_lines(
    spec: &TuiScreenSpec,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let subtitle = spec.subtitle.as_deref();
    let mut lines = render_header(spec.header_style, width, subtitle, color_enabled);

    if let Some(title) = spec.title.as_deref() {
        lines.push(String::new());
        lines.extend(render_wrapped_display_lines([title], width));
    }

    if let Some(progress_line) = spec.progress_line.as_deref() {
        lines.extend(render_wrapped_display_lines([progress_line], width));
    }

    if !spec.intro_lines.is_empty() {
        lines.extend(render_wrapped_display_lines(&spec.intro_lines, width));
    }

    lines
}

fn render_header(
    style: TuiHeaderStyle,
    width: usize,
    subtitle: Option<&str>,
    color_enabled: bool,
) -> Vec<String> {
    let brand_lines = match style {
        TuiHeaderStyle::Brand => crate::presentation::render_brand_header(
            width,
            &crate::presentation::BuildVersionInfo::current(),
            subtitle,
        ),
        TuiHeaderStyle::Compact => crate::presentation::render_compact_brand_header(
            width,
            &crate::presentation::BuildVersionInfo::current(),
            subtitle,
        ),
    };

    crate::presentation::style_brand_lines_with_palette(
        &brand_lines,
        color_enabled,
        crate::presentation::ONBOARD_BRAND_PALETTE,
    )
}

fn render_message_heading(spec: &TuiMessageSpec) -> String {
    let trimmed_role = spec.role.trim();
    let trimmed_caption = spec.caption.as_deref().map(str::trim).unwrap_or("");
    let role = if trimmed_role.is_empty() {
        "message"
    } else {
        trimmed_role
    };

    if trimmed_caption.is_empty() {
        return role.to_owned();
    }

    format!("{role}: {trimmed_caption}")
}

fn build_screen_section_block(
    section: &TuiSectionSpec,
    width: usize,
) -> (Option<String>, Vec<String>) {
    match section {
        TuiSectionSpec::Narrative {
            title,
            lines: content,
        } => (
            title
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            render_wrapped_display_lines(content, width),
        ),
        TuiSectionSpec::KeyValues { title, items } => (
            title
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            items
                .iter()
                .flat_map(|item| render_key_value_item_lines(item, width))
                .collect(),
        ),
        TuiSectionSpec::ActionGroup {
            title,
            inline_title_when_wide,
            items,
        } => (
            title
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            render_action_group_lines(None, *inline_title_when_wide, items, width),
        ),
        TuiSectionSpec::Checklist { title, items } => (
            title
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            render_checklist_lines(None, items, width),
        ),
        TuiSectionSpec::Callout {
            tone,
            title,
            lines: content,
        } => (
            Some(
                match title
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    Some(title) => format!("{} · {title}", tone_label(*tone)),
                    None => tone_label(*tone).to_owned(),
                },
            ),
            render_wrapped_display_lines(content, width),
        ),
        TuiSectionSpec::Preformatted {
            title,
            language,
            lines: content,
        } => (
            build_preformatted_heading(
                title
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty()),
                language
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty()),
            ),
            content
                .iter()
                .map(|line| {
                    if line.is_empty() {
                        String::new()
                    } else {
                        format!("    {line}")
                    }
                })
                .collect(),
        ),
    }
}

fn append_section_lines(lines: &mut Vec<String>, section: &TuiSectionSpec, width: usize) {
    let section_lines = match section {
        TuiSectionSpec::Narrative {
            title,
            lines: content,
        } => {
            let mut rendered = Vec::new();
            if let Some(title) = title.as_deref().filter(|title| !title.trim().is_empty()) {
                rendered.push(title.to_owned());
            }
            rendered.extend(render_wrapped_display_lines(content, width));
            rendered
        }
        TuiSectionSpec::KeyValues { title, items } => {
            let mut rendered = Vec::new();
            if let Some(title) = title.as_deref().filter(|title| !title.trim().is_empty()) {
                rendered.push(title.to_owned());
            }
            for item in items {
                rendered.extend(render_key_value_item_lines(item, width));
            }
            rendered
        }
        TuiSectionSpec::ActionGroup {
            title,
            inline_title_when_wide,
            items,
        } => render_action_group_lines(title.as_deref(), *inline_title_when_wide, items, width),
        TuiSectionSpec::Checklist { title, items } => {
            render_checklist_lines(title.as_deref(), items, width)
        }
        TuiSectionSpec::Callout {
            tone,
            title,
            lines: content,
        } => render_callout_lines(*tone, title.as_deref(), content, width),
        TuiSectionSpec::Preformatted {
            title,
            language,
            lines: content,
        } => render_preformatted_lines(title.as_deref(), language.as_deref(), content),
    };

    if section_lines.is_empty() {
        return;
    }

    lines.push(String::new());
    lines.extend(section_lines);
}

fn render_key_value_item_lines(item: &TuiKeyValueSpec, width: usize) -> Vec<String> {
    match item {
        TuiKeyValueSpec::Plain { key, value } => {
            crate::presentation::render_wrapped_text_line(&format!("- {key}: "), value, width)
        }
        TuiKeyValueSpec::Csv { key, values } => {
            let values = values.iter().map(String::as_str).collect::<Vec<_>>();
            crate::presentation::render_wrapped_csv_line(&format!("- {key}: "), &values, width)
        }
    }
}

fn render_action_group_lines(
    title: Option<&str>,
    inline_title_when_wide: bool,
    items: &[TuiActionSpec],
    width: usize,
) -> Vec<String> {
    let title = title.map(str::trim).filter(|value| !value.is_empty());

    if inline_title_when_wide
        && width >= INLINE_ACTION_GROUP_WIDTH
        && items.len() == 1
        && let (Some(title), Some(item)) = (title, items.first())
    {
        return crate::presentation::render_wrapped_text_line(
            &format!("{title}: "),
            &item.command,
            width,
        );
    }

    let mut rendered = Vec::new();
    if let Some(title) = title {
        rendered.push(title.to_owned());
    }

    for item in items {
        rendered.extend(crate::presentation::render_wrapped_text_line(
            &format!("- {}: ", item.label),
            &item.command,
            width,
        ));
    }

    rendered
}

fn render_checklist_lines(
    title: Option<&str>,
    items: &[TuiChecklistItemSpec],
    width: usize,
) -> Vec<String> {
    let mut rendered = Vec::new();
    if let Some(title) = title.map(str::trim).filter(|value| !value.is_empty()) {
        rendered.push(title.to_owned());
    }

    let render_stacked_rows = |items: &[TuiChecklistItemSpec], width: usize| {
        let mut lines = Vec::new();

        for item in items {
            lines.push(format!(
                "{} {}",
                checklist_status_marker(item.status),
                item.label
            ));
            lines.extend(crate::presentation::render_wrapped_text_line(
                "  ",
                &item.detail,
                width,
            ));
        }

        lines
    };

    if width < 68 {
        rendered.extend(render_stacked_rows(items, width));
        return rendered;
    }

    let label_width = items.iter().map(|item| item.label.len()).max().unwrap_or(0);
    let rows = items
        .iter()
        .map(|item| {
            format!(
                "{} {:width$}  {}",
                checklist_status_marker(item.status),
                item.label,
                item.detail,
                width = label_width
            )
        })
        .collect::<Vec<_>>();

    if rows.iter().any(|row| row.len() > width) {
        rendered.extend(render_stacked_rows(items, width));
        return rendered;
    }

    rendered.extend(rows);
    rendered
}

fn checklist_status_marker(status: TuiChecklistStatus) -> &'static str {
    match status {
        TuiChecklistStatus::Pass => "[OK]",
        TuiChecklistStatus::Warn => "[WARN]",
        TuiChecklistStatus::Fail => "[FAIL]",
    }
}

fn render_callout_lines(
    tone: TuiCalloutTone,
    title: Option<&str>,
    lines: &[String],
    width: usize,
) -> Vec<String> {
    let heading = match title.map(str::trim).filter(|value| !value.is_empty()) {
        Some(title) => format!("{}: {title}", tone_label(tone)),
        None => tone_label(tone).to_owned(),
    };

    let mut rendered = vec![heading];
    for line in lines {
        rendered.extend(crate::presentation::render_wrapped_text_line(
            "- ", line, width,
        ));
    }
    rendered
}

fn tone_label(tone: TuiCalloutTone) -> &'static str {
    match tone {
        TuiCalloutTone::Info => "note",
        TuiCalloutTone::Success => "ready",
        TuiCalloutTone::Warning => "attention",
    }
}

fn render_preformatted_lines(
    title: Option<&str>,
    language: Option<&str>,
    lines: &[String],
) -> Vec<String> {
    let trimmed_title = title.map(str::trim).filter(|value| !value.is_empty());
    let trimmed_language = language.map(str::trim).filter(|value| !value.is_empty());
    let mut rendered = Vec::new();

    if let Some(heading) = build_preformatted_heading(trimmed_title, trimmed_language) {
        rendered.push(heading);
    }

    if lines.is_empty() {
        rendered.push("    ".to_owned());
        return rendered;
    }

    for line in lines {
        if line.is_empty() {
            rendered.push(String::new());
            continue;
        }
        rendered.push(format!("    {line}"));
    }

    rendered
}

fn build_preformatted_heading(title: Option<&str>, language: Option<&str>) -> Option<String> {
    match (title, language) {
        (Some(title), Some(language)) => Some(format!("{title} [{language}]")),
        (Some(title), None) => Some(title.to_owned()),
        (None, Some(language)) => Some(format!("code [{language}]")),
        (None, None) => Some("code".to_owned()),
    }
}

fn render_wrapped_display_lines<I, S>(display_lines: I, width: usize) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    display_lines
        .into_iter()
        .flat_map(|line| crate::presentation::render_wrapped_display_line(line.as_ref(), width))
        .collect()
}

fn render_choice_lines(choices: &[TuiChoiceSpec], width: usize) -> Vec<String> {
    let mut lines = Vec::new();

    for choice in choices {
        let suffix = if choice.recommended {
            " (recommended)"
        } else {
            ""
        };
        let prefix = format!("{}) ", choice.key);
        let continuation = " ".repeat(prefix.chars().count());
        lines.extend(
            crate::presentation::render_wrapped_text_line_with_continuation(
                &prefix,
                &continuation,
                &format!("{}{}", choice.label, suffix),
                width,
            ),
        );
        lines.extend(render_wrapped_display_lines(
            choice
                .detail_lines
                .iter()
                .map(|detail| format!("    {detail}"))
                .collect::<Vec<_>>(),
            width,
        ));
    }

    lines
}

fn rendered_screen_height(header_lines: &[String], blocks: &[RenderedScreenBlock]) -> u16 {
    let mut total = block_height(header_lines);
    for block in blocks {
        total = total.saturating_add(block_height(&block.lines));
    }
    total
}

fn block_height(lines: &[String]) -> u16 {
    u16::try_from(lines.len().max(1))
        .unwrap_or(u16::MAX)
        .saturating_add(2)
}

fn render_text_block(area: Rect, title: &str, lines: &[String], buffer: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {title} "));
    let inner = block.inner(area);
    block.render(area, buffer);
    let content_lines = if lines.is_empty() {
        vec![String::new()]
    } else {
        lines.to_vec()
    };
    Paragraph::new(text_from_lines(&content_lines))
        .wrap(Wrap { trim: false })
        .render(inner, buffer);
}

fn text_from_lines(lines: &[String]) -> Text<'static> {
    Text::from(lines.iter().cloned().map(Line::from).collect::<Vec<_>>())
}

fn render_buffer_to_lines(buffer: &Buffer) -> Vec<String> {
    let area = buffer.area;
    let mut rendered = Vec::new();
    for y in area.top()..area.bottom() {
        let mut line = String::new();
        for x in area.left()..area.right() {
            line.push_str(buffer[(x, y)].symbol());
        }
        rendered.push(line.trim_end_matches(' ').to_owned());
    }
    rendered
}

struct RenderedScreenBlock {
    title: Option<String>,
    lines: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_spec_serializes_as_component_tree() {
        let spec = TuiScreenSpec {
            header_style: TuiHeaderStyle::Compact,
            subtitle: Some("guided setup".to_owned()),
            title: Some("security check".to_owned()),
            progress_line: None,
            intro_lines: vec!["review the trust boundary before write".to_owned()],
            sections: vec![
                TuiSectionSpec::Callout {
                    tone: TuiCalloutTone::Warning,
                    title: Some("what onboarding can do".to_owned()),
                    lines: vec!["tool execution can touch local files".to_owned()],
                },
                TuiSectionSpec::KeyValues {
                    title: Some("draft".to_owned()),
                    items: vec![TuiKeyValueSpec::Plain {
                        key: "provider".to_owned(),
                        value: "OpenAI".to_owned(),
                    }],
                },
                TuiSectionSpec::ActionGroup {
                    title: Some("start here".to_owned()),
                    inline_title_when_wide: true,
                    items: vec![TuiActionSpec {
                        label: "ask".to_owned(),
                        command: "loong ask --message 'hello'".to_owned(),
                    }],
                },
                TuiSectionSpec::Checklist {
                    title: Some("preflight".to_owned()),
                    items: vec![TuiChecklistItemSpec {
                        status: TuiChecklistStatus::Warn,
                        label: "provider model probe".to_owned(),
                        detail: "catalog probe failed".to_owned(),
                    }],
                },
                TuiSectionSpec::Preformatted {
                    title: Some("example".to_owned()),
                    language: Some("toml".to_owned()),
                    lines: vec!["model = \"gpt-5\"".to_owned()],
                },
            ],
            choices: vec![TuiChoiceSpec {
                key: "1".to_owned(),
                label: "Continue".to_owned(),
                detail_lines: vec!["write this draft".to_owned()],
                recommended: true,
            }],
            footer_lines: vec!["press Enter to use default 1, continue".to_owned()],
        };

        let encoded = serde_json::to_value(&spec).expect("serialize screen spec");
        assert_eq!(encoded["header_style"], "compact");
        assert_eq!(encoded["sections"][0]["kind"], "callout");
        assert_eq!(encoded["sections"][1]["items"][0]["kind"], "plain");
        assert_eq!(encoded["sections"][2]["kind"], "action_group");
        assert_eq!(encoded["sections"][3]["kind"], "checklist");
        assert_eq!(encoded["sections"][4]["kind"], "preformatted");
        assert_eq!(encoded["choices"][0]["label"], "Continue");
    }

    #[test]
    fn renderer_keeps_callouts_choices_and_footer_visible() {
        let spec = TuiScreenSpec {
            header_style: TuiHeaderStyle::Compact,
            subtitle: Some("guided setup".to_owned()),
            title: Some("security check".to_owned()),
            progress_line: None,
            intro_lines: vec!["review the trust boundary before write".to_owned()],
            sections: vec![
                TuiSectionSpec::Callout {
                    tone: TuiCalloutTone::Warning,
                    title: Some("what onboarding can do".to_owned()),
                    lines: vec!["tool execution can touch local files".to_owned()],
                },
                TuiSectionSpec::ActionGroup {
                    title: Some("start here".to_owned()),
                    inline_title_when_wide: true,
                    items: vec![TuiActionSpec {
                        label: "ask".to_owned(),
                        command: "loong ask --message 'hello'".to_owned(),
                    }],
                },
            ],
            choices: vec![TuiChoiceSpec {
                key: "1".to_owned(),
                label: "Continue".to_owned(),
                detail_lines: vec!["write this draft".to_owned()],
                recommended: true,
            }],
            footer_lines: vec!["press Enter to use default 1, continue".to_owned()],
        };

        let lines = render_tui_screen_spec(&spec, 80, false);

        assert!(
            lines.first().is_some_and(|line| line.starts_with("LOONG")),
            "compact header should keep the LOONG wordmark: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "attention: what onboarding can do"),
            "callout heading should render with its tone label: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "start here: loong ask --message 'hello'"),
            "single primary actions should render inline on wide terminals: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "1) Continue (recommended)"),
            "choices should remain visible after callouts and actions: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "press Enter to use default 1, continue"),
            "footer guidance should remain visible after structured sections: {lines:#?}"
        );
    }

    #[test]
    fn ratatui_renderer_wraps_screen_in_shell_blocks() {
        let spec = TuiScreenSpec {
            header_style: TuiHeaderStyle::Compact,
            subtitle: Some("guided setup".to_owned()),
            title: Some("security check".to_owned()),
            progress_line: Some("step 2 of 3".to_owned()),
            intro_lines: vec!["review the trust boundary before write".to_owned()],
            sections: vec![
                TuiSectionSpec::ActionGroup {
                    title: Some("start here".to_owned()),
                    inline_title_when_wide: false,
                    items: vec![TuiActionSpec {
                        label: "ask".to_owned(),
                        command: "loong ask --message 'hello'".to_owned(),
                    }],
                },
                TuiSectionSpec::Checklist {
                    title: Some("preflight".to_owned()),
                    items: vec![TuiChecklistItemSpec {
                        status: TuiChecklistStatus::Warn,
                        label: "provider probe".to_owned(),
                        detail: "catalog probe failed".to_owned(),
                    }],
                },
            ],
            choices: vec![TuiChoiceSpec {
                key: "1".to_owned(),
                label: "Continue".to_owned(),
                detail_lines: vec!["write this draft".to_owned()],
                recommended: true,
            }],
            footer_lines: vec!["press Enter to continue".to_owned()],
        };

        let lines = render_tui_screen_spec_ratatui(&spec, 80, false);
        let rendered = lines.join("\n");

        assert!(rendered.contains(" loong "), "{rendered}");
        assert!(rendered.contains(" start here "), "{rendered}");
        assert!(rendered.contains(" preflight "), "{rendered}");
        assert!(rendered.contains(" choices "), "{rendered}");
        assert!(rendered.contains(" next "), "{rendered}");
        assert!(rendered.contains("LOONG"), "{rendered}");
        assert!(
            rendered.contains("loong ask --message 'hello'"),
            "{rendered}"
        );
    }

    #[test]
    fn onboard_renderer_falls_back_to_legacy_lines_when_stdout_is_not_a_tty() {
        let spec = TuiScreenSpec {
            header_style: TuiHeaderStyle::Compact,
            subtitle: Some("guided setup".to_owned()),
            title: Some("security check".to_owned()),
            progress_line: None,
            intro_lines: vec!["review the trust boundary before write".to_owned()],
            sections: vec![TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Warning,
                title: Some("what onboarding can do".to_owned()),
                lines: vec!["tool execution can touch local files".to_owned()],
            }],
            choices: vec![TuiChoiceSpec {
                key: "y".to_owned(),
                label: "Continue".to_owned(),
                detail_lines: vec!["proceed with setup".to_owned()],
                recommended: false,
            }],
            footer_lines: vec!["press Enter to continue".to_owned()],
        };

        let rendered = render_onboard_screen_spec(&spec, 80, true).join("\n");

        assert!(rendered.contains("LOONG"), "{rendered}");
        assert!(
            rendered.contains("attention: what onboarding can do"),
            "{rendered}"
        );
        assert!(rendered.contains("y) Continue"), "{rendered}");
    }

    #[test]
    fn message_renderer_preserves_preformatted_blocks() {
        let spec = TuiMessageSpec {
            role: "assistant".to_owned(),
            caption: Some("reply".to_owned()),
            sections: vec![
                TuiSectionSpec::Narrative {
                    title: Some("plan".to_owned()),
                    lines: vec!["- inspect current config".to_owned()],
                },
                TuiSectionSpec::Preformatted {
                    title: Some("patch".to_owned()),
                    language: Some("rust".to_owned()),
                    lines: vec![
                        "let value = input.trim();".to_owned(),
                        String::new(),
                        "println!(\"{value}\");".to_owned(),
                    ],
                },
            ],
            footer_lines: vec!["end of reply".to_owned()],
        };

        let lines = render_tui_message_spec(&spec, 72);

        assert_eq!(lines[0], "assistant: reply");
        assert!(
            lines.iter().any(|line| line == "plan"),
            "message sections should keep their narrative title: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "patch [rust]"),
            "preformatted sections should surface a code heading: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "    let value = input.trim();"),
            "preformatted sections should keep raw line indentation: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == "end of reply"),
            "message footers should remain visible: {lines:#?}"
        );
    }
}
