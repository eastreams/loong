use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

use super::{ApprovalRequirement, ApprovalRequirementKind, join_non_empty_lines};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPromptMarker {
    ToolApprovalRequired,
    ApprovalRequired,
}

impl ApprovalPromptMarker {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ToolApprovalRequired => "[tool_approval_required]",
            Self::ApprovalRequired => "[approval_required]",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPromptLocale {
    En,
    Cjk,
}

impl ApprovalPromptLocale {
    pub const fn is_cjk(self) -> bool {
        matches!(self, Self::Cjk)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPromptActionId {
    Yes,
    Auto,
    Full,
    Esc,
}

impl ApprovalPromptActionId {
    pub const fn command(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::Auto => "auto",
            Self::Full => "full",
            Self::Esc => "esc",
        }
    }

    pub const fn numeric_alias(self) -> &'static str {
        match self {
            Self::Yes => "1",
            Self::Auto => "2",
            Self::Full => "3",
            Self::Esc => "4",
        }
    }

    pub const fn all() -> [Self; 4] {
        [Self::Yes, Self::Auto, Self::Full, Self::Esc]
    }

    fn matches_normalized_input(self, normalized: &str) -> bool {
        match self {
            Self::Yes => matches!(
                normalized,
                "1" | "y"
                    | "yes"
                    | "run"
                    | "once"
                    | "run once"
                    | "本次"
                    | "一次"
                    | "运行一次"
                    | "本次运行"
                    | "仅这次"
            ),
            Self::Auto => matches!(
                normalized,
                "2" | "auto" | "session auto" | "自动" | "本会话自动"
            ),
            Self::Full => matches!(
                normalized,
                "3" | "full"
                    | "full auto"
                    | "session full"
                    | "session full auto"
                    | "全自动"
                    | "本会话全自动"
            ),
            Self::Esc => matches!(
                normalized,
                "4" | "esc"
                    | "cancel"
                    | "skip"
                    | "skip call"
                    | "取消"
                    | "跳过"
                    | "跳过这次"
                    | "这次跳过"
                    | "不运行"
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPromptActionEffect {
    CurrentCallOnly,
    SessionAuto,
    SessionFull,
    SkipCurrentCall,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalPromptActionView {
    pub id: ApprovalPromptActionId,
    pub effect: ApprovalPromptActionEffect,
    pub command: String,
    pub numeric_alias: String,
    pub label: String,
    pub summary: String,
    #[serde(default)]
    pub detail_lines: Vec<String>,
    #[serde(default)]
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalPromptView {
    pub marker: ApprovalPromptMarker,
    pub preface: Option<String>,
    pub tool_name: Option<String>,
    pub request_id: Option<String>,
    pub rule_id: Option<String>,
    pub reason: Option<String>,
    pub locale: ApprovalPromptLocale,
    #[serde(default)]
    pub actions: Vec<ApprovalPromptActionView>,
}

impl ApprovalPromptView {
    pub fn title(&self) -> Option<String> {
        match self.locale {
            ApprovalPromptLocale::Cjk => self
                .tool_name
                .as_ref()
                .map(|tool_name| format!("准备调用 {tool_name}"))
                .or_else(|| Some("工具调用需要确认".to_owned())),
            ApprovalPromptLocale::En => self
                .tool_name
                .as_ref()
                .map(|tool_name| format!("loong wants to call {tool_name}"))
                .or_else(|| Some("Tool call needs confirmation".to_owned())),
        }
    }

    pub fn pause_reason_title(&self) -> String {
        if self.locale.is_cjk() {
            "为什么停下来".to_owned()
        } else {
            "Why it paused".to_owned()
        }
    }

    pub fn request_section_title(&self) -> String {
        if self.locale.is_cjk() {
            "当前请求".to_owned()
        } else {
            "Pending request".to_owned()
        }
    }

    pub fn request_id_label(&self) -> String {
        if self.locale.is_cjk() {
            "请求 ID".to_owned()
        } else {
            "request id".to_owned()
        }
    }

    pub fn tool_label(&self) -> String {
        if self.locale.is_cjk() {
            "工具".to_owned()
        } else {
            "tool".to_owned()
        }
    }

    pub fn subtitle(&self) -> String {
        if self.locale.is_cjk() {
            "工具确认".to_owned()
        } else {
            "tool consent".to_owned()
        }
    }

    pub fn action_commands_text(&self) -> String {
        self.actions
            .iter()
            .map(|action| action.command.as_str())
            .collect::<Vec<_>>()
            .join(" / ")
    }

    pub fn action_numeric_aliases_text(&self) -> String {
        self.actions
            .iter()
            .map(|action| action.numeric_alias.as_str())
            .collect::<Vec<_>>()
            .join(" / ")
    }

    pub fn reply_hint_lines(&self) -> Vec<String> {
        if self.actions.is_empty() {
            return Vec::new();
        }

        let action_commands = self.action_commands_text();
        match self.locale {
            ApprovalPromptLocale::Cjk => vec![
                format!("可直接回复：{action_commands}"),
                self.actions
                    .iter()
                    .map(|action| format!("{}={}", action.command, action.summary))
                    .collect::<Vec<_>>()
                    .join("，"),
            ],
            ApprovalPromptLocale::En => vec![
                format!("Reply with: {action_commands}"),
                self.actions
                    .iter()
                    .map(|action| format!("{} = {}", action.command, action.summary))
                    .collect::<Vec<_>>()
                    .join(", "),
            ],
        }
    }
}

pub fn format_approval_required_reply(
    assistant_preface: &str,
    requirement: &ApprovalRequirement,
) -> String {
    render_approval_prompt_view(&approval_prompt_view_from_requirement(
        assistant_preface,
        requirement,
    ))
}

pub fn parse_approval_prompt_view(assistant_text: &str) -> Option<ApprovalPromptView> {
    let (marker_index, marker) = find_approval_prompt_marker(assistant_text)?;
    let preface = trimmed_non_empty(assistant_text.get(..marker_index).unwrap_or_default());
    let body = assistant_text.get(marker_index..)?;
    let locale = approval_prompt_locale_from_text(assistant_text);
    let mut tool_name = None;
    let mut request_id = None;
    let mut rule_id = None;
    let mut reason = None;

    for line in body.lines() {
        if let Some(value) = line.strip_prefix("tool: ") {
            tool_name = trimmed_non_empty(value);
        } else if let Some(value) = line.strip_prefix("request_id: ") {
            request_id = trimmed_non_empty(value);
        } else if let Some(value) = line.strip_prefix("rule_id: ") {
            rule_id = trimmed_non_empty(value);
        } else if let Some(value) = line.strip_prefix("reason: ") {
            reason = trimmed_non_empty(value);
        }
    }

    Some(ApprovalPromptView {
        marker,
        preface,
        tool_name,
        request_id,
        rule_id,
        reason,
        locale,
        actions: approval_prompt_actions(marker, locale),
    })
}

pub fn normalize_approval_prompt_control_input(input: &str) -> String {
    let compatibility = input.nfkc().collect::<String>();
    let trimmed = compatibility.trim().trim_matches(|character: char| {
        character.is_whitespace()
            || matches!(
                character,
                '`' | '"'
                    | '\''
                    | '.'
                    | ','
                    | ':'
                    | ';'
                    | '!'
                    | '?'
                    | '，'
                    | '。'
                    | '：'
                    | '；'
                    | '！'
                    | '？'
            )
    });
    let lowercased = trimmed.to_lowercase();

    let normalized = lowercased
        .chars()
        .map(|character| match character {
            '_' | '-' => ' ',
            other => other,
        })
        .collect::<String>();

    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn parse_approval_prompt_action_input(input: &str) -> Option<ApprovalPromptActionId> {
    let normalized = normalize_approval_prompt_control_input(input);
    ApprovalPromptActionId::all()
        .into_iter()
        .find(|action| action.matches_normalized_input(normalized.as_str()))
}

fn render_approval_prompt_view(view: &ApprovalPromptView) -> String {
    let mut lines = Vec::new();
    lines.push(view.marker.as_str().to_owned());
    if let Some(tool_name) = view.tool_name.as_deref() {
        lines.push(format!("tool: {tool_name}"));
    }
    if let Some(request_id) = view.request_id.as_deref() {
        lines.push(format!("request_id: {request_id}"));
    }
    if let Some(rule_id) = view.rule_id.as_deref() {
        lines.push(format!("rule_id: {rule_id}"));
    }
    if let Some(reason) = view.reason.as_deref() {
        lines.push(format!("reason: {reason}"));
    }
    if !view.actions.is_empty() {
        lines.push(format!(
            "allowed_decisions: {}",
            view.action_commands_text()
        ));
        for action in &view.actions {
            for detail_line in &action.detail_lines {
                lines.push(detail_line.clone());
            }
        }
        lines.push(String::new());
        lines.extend(view.reply_hint_lines());
    }
    let body = lines.join("\n");
    join_non_empty_lines(&[view.preface.as_deref().unwrap_or_default(), body.as_str()])
}

fn approval_prompt_view_from_requirement(
    assistant_preface: &str,
    requirement: &ApprovalRequirement,
) -> ApprovalPromptView {
    let marker = match requirement.kind {
        ApprovalRequirementKind::GovernedTool => ApprovalPromptMarker::ToolApprovalRequired,
        ApprovalRequirementKind::KernelContextRequired => ApprovalPromptMarker::ApprovalRequired,
    };
    let locale = approval_prompt_locale_from_text(
        join_non_empty_lines(&[assistant_preface, requirement.reason.as_str()]).as_str(),
    );

    ApprovalPromptView {
        marker,
        preface: trimmed_non_empty(assistant_preface),
        tool_name: requirement
            .tool_name
            .as_deref()
            .map(crate::tools::legacy_display_tool_name),
        request_id: requirement.approval_request_id.clone(),
        rule_id: trimmed_non_empty(requirement.rule_id.as_str()),
        reason: trimmed_non_empty(requirement.reason.as_str()),
        locale,
        actions: approval_prompt_actions(marker, locale),
    }
}

fn approval_prompt_actions(
    marker: ApprovalPromptMarker,
    locale: ApprovalPromptLocale,
) -> Vec<ApprovalPromptActionView> {
    if marker != ApprovalPromptMarker::ToolApprovalRequired {
        return Vec::new();
    }

    let make_action = |id,
                       effect,
                       label_cjk: &str,
                       label_en: &str,
                       summary_cjk: &str,
                       summary_en: &str,
                       detail_cjk: &[&str],
                       detail_en: &[&str],
                       recommended| ApprovalPromptActionView {
        id,
        effect,
        command: id.command().to_owned(),
        numeric_alias: id.numeric_alias().to_owned(),
        label: if locale.is_cjk() {
            label_cjk.to_owned()
        } else {
            label_en.to_owned()
        },
        summary: if locale.is_cjk() {
            summary_cjk.to_owned()
        } else {
            summary_en.to_owned()
        },
        detail_lines: if locale.is_cjk() {
            detail_cjk.iter().map(|line| (*line).to_owned()).collect()
        } else {
            detail_en.iter().map(|line| (*line).to_owned()).collect()
        },
        recommended,
    };

    vec![
        make_action(
            ApprovalPromptActionId::Yes,
            ApprovalPromptActionEffect::CurrentCallOnly,
            "本次运行",
            "Run once",
            "仅本次运行",
            "run once",
            &["只运行当前这次 tool call"],
            &["Execute only this tool call"],
            true,
        ),
        make_action(
            ApprovalPromptActionId::Auto,
            ApprovalPromptActionEffect::SessionAuto,
            "本会话自动",
            "Session auto",
            "本会话自动",
            "session auto mode",
            &[
                "后续低风险工具自动运行",
                "写文件、执行 shell、切换 provider 等仍会停下来",
            ],
            &[
                "Low-risk tools continue automatically",
                "Writes, shell exec, provider switching, and similar actions still pause",
            ],
            false,
        ),
        make_action(
            ApprovalPromptActionId::Full,
            ApprovalPromptActionEffect::SessionFull,
            "本会话全自动",
            "Session full-auto",
            "本会话全自动",
            "session full-auto mode",
            &[
                "本会话内不再询问 tool consent",
                "仍不会绕过 governed approval、shell allowlist 等硬限制",
            ],
            &[
                "Stop asking for tool consent in this session",
                "Governed approvals and kernel hard limits still apply",
            ],
            false,
        ),
        make_action(
            ApprovalPromptActionId::Esc,
            ApprovalPromptActionEffect::SkipCurrentCall,
            "跳过这次",
            "Skip call",
            "跳过这次",
            "skip this call",
            &["不执行这次 tool call"],
            &["Do not run this tool call"],
            false,
        ),
    ]
}

fn find_approval_prompt_marker(text: &str) -> Option<(usize, ApprovalPromptMarker)> {
    let tool_marker = text.find(ApprovalPromptMarker::ToolApprovalRequired.as_str());
    let generic_marker = text.find(ApprovalPromptMarker::ApprovalRequired.as_str());
    match (tool_marker, generic_marker) {
        (Some(tool_index), Some(generic_index)) if tool_index <= generic_index => {
            Some((tool_index, ApprovalPromptMarker::ToolApprovalRequired))
        }
        (Some(_tool_index), Some(generic_index)) => {
            Some((generic_index, ApprovalPromptMarker::ApprovalRequired))
        }
        (Some(tool_index), None) => Some((tool_index, ApprovalPromptMarker::ToolApprovalRequired)),
        (None, Some(generic_index)) => {
            Some((generic_index, ApprovalPromptMarker::ApprovalRequired))
        }
        (None, None) => None,
    }
}

fn approval_prompt_locale_from_text(text: &str) -> ApprovalPromptLocale {
    if contains_cjk_text(text) {
        ApprovalPromptLocale::Cjk
    } else {
        ApprovalPromptLocale::En
    }
}

fn trimmed_non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn contains_cjk_text(text: &str) -> bool {
    text.chars().any(is_cjk_character)
}

fn is_cjk_character(character: char) -> bool {
    matches!(
        character as u32,
        0x3040..=0x30ff
            | 0x3400..=0x4dbf
            | 0x4e00..=0x9fff
            | 0xac00..=0xd7af
            | 0xf900..=0xfaff
    )
}
