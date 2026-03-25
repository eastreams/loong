use loongclaw_app as mvp;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TuiHeaderStyle {
    Brand,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TuiCalloutTone {
    Info,
    Success,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TuiChoiceSpec {
    pub(crate) key: String,
    pub(crate) label: String,
    #[serde(default)]
    pub(crate) detail_lines: Vec<String>,
    #[serde(default)]
    pub(crate) recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TuiKeyValueSpec {
    Plain { key: String, value: String },
    Csv { key: String, values: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TuiActionSpec {
    pub(crate) label: String,
    pub(crate) command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TuiChecklistStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TuiChecklistItemSpec {
    pub(crate) status: TuiChecklistStatus,
    pub(crate) label: String,
    pub(crate) detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TuiSectionSpec {
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TuiScreenSpec {
    pub(crate) header_style: TuiHeaderStyle,
    pub(crate) subtitle: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) progress_line: Option<String>,
    #[serde(default)]
    pub(crate) intro_lines: Vec<String>,
    #[serde(default)]
    pub(crate) sections: Vec<TuiSectionSpec>,
    #[serde(default)]
    pub(crate) choices: Vec<TuiChoiceSpec>,
    #[serde(default)]
    pub(crate) footer_lines: Vec<String>,
}

pub(crate) fn render_onboard_screen_spec(
    spec: &TuiScreenSpec,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let mut lines = render_header(
        spec.header_style,
        width,
        spec.subtitle.as_deref().unwrap_or(""),
        color_enabled,
    );

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

const INLINE_ACTION_GROUP_WIDTH: usize = 56;

fn render_header(
    style: TuiHeaderStyle,
    width: usize,
    subtitle: &str,
    color_enabled: bool,
) -> Vec<String> {
    let brand_lines = match style {
        TuiHeaderStyle::Brand => mvp::presentation::render_brand_header(
            width,
            &mvp::presentation::BuildVersionInfo::current(),
            Some(subtitle),
        ),
        TuiHeaderStyle::Compact => mvp::presentation::render_compact_brand_header(
            width,
            &mvp::presentation::BuildVersionInfo::current(),
            Some(subtitle),
        ),
    };

    mvp::presentation::style_brand_lines_with_palette(
        &brand_lines,
        color_enabled,
        mvp::presentation::ONBOARD_BRAND_PALETTE,
    )
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
            mvp::presentation::render_wrapped_text_line(&format!("- {key}: "), value, width)
        }
        TuiKeyValueSpec::Csv { key, values } => {
            let values = values.iter().map(String::as_str).collect::<Vec<_>>();
            mvp::presentation::render_wrapped_csv_line(&format!("- {key}: "), &values, width)
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
        return mvp::presentation::render_wrapped_text_line(
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
        rendered.extend(mvp::presentation::render_wrapped_text_line(
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
            lines.extend(mvp::presentation::render_wrapped_text_line(
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
        rendered.extend(mvp::presentation::render_wrapped_text_line(
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

fn render_wrapped_display_lines<I, S>(display_lines: I, width: usize) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    display_lines
        .into_iter()
        .flat_map(|line| mvp::presentation::render_wrapped_display_line(line.as_ref(), width))
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
            mvp::presentation::render_wrapped_text_line_with_continuation(
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
                        command: "loongclaw ask --message 'hello'".to_owned(),
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
                        command: "loongclaw ask --message 'hello'".to_owned(),
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

        let lines = render_onboard_screen_spec(&spec, 80, false);

        assert!(
            lines
                .first()
                .is_some_and(|line| line.starts_with("LOONGCLAW")),
            "compact header should keep the LOONGCLAW wordmark: {lines:#?}"
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
                .any(|line| line == "start here: loongclaw ask --message 'hello'"),
            "single primary actions should render inline on wide terminals: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("1) Continue (recommended)")),
            "choice list should stay visible: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "press Enter to use default 1, continue"),
            "footer guidance should remain visible: {lines:#?}"
        );
    }
}
