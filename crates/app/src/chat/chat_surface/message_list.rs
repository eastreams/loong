use super::utils::*;
use crate::chat::chat_surface::diff_viewer::render_diff_to_lines;
use crate::chat::chat_surface::markdown;
use crate::chat::chat_surface::transcript_scroll_state::TranscriptScrollState;
use crate::conversation::is_compacted_summary_content;
use crate::tui_surface::TuiSectionSpec;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

const PROVIDER_ERROR_REPLY_PREFIX: &str = "[provider_error] ";
static EMPTY_RENDER_LINES: LazyLock<Vec<Line<'static>>> = LazyLock::new(Vec::new);
const STARTUP_WORDMARK: &[&str] = &[
    "░███░         ░████████░    ░████████░   ░█████████░    ░████████░",
    "░███░        ░███    ███░  ░███    ███░  ░███    ███░  ░███    ███░",
    "░███░        ░███    ███░  ░███    ███░  ░███    ███░  ░███",
    "░███░        ░███    ███░  ░███    ███░  ░███    ███░  ░███  █████░",
    "░███░        ░███    ███░  ░███    ███░  ░███    ███░  ░███    ███░",
    "░███░        ░███    ███░  ░███    ███░  ░███    ███░  ░███    ███░",
    "░██████████   ░████████░    ░████████░   ░███    ███░   ░████████░",
];
type StartupEyeFrame = [&'static str; STARTUP_EYE_INTERIOR_ROWS];
const STARTUP_EYE_INTERIOR_ROWS: usize = 5;
const STARTUP_EYE_INTERIOR_WIDTH: usize = 4;
const STARTUP_EYE_CAVITY: &str = "░███    ███░";
const STARTUP_EYE_FRAMES: &[StartupEyeFrame] = &[
    ["    ", "    ", " █  ", "    ", "    "],
    ["    ", "    ", " ▆  ", "    ", "    "],
    ["    ", "    ", "▄   ", "    ", "    "],
    ["    ", "    ", "█   ", "    ", "    "],
    ["▒▒▒▒", "    ", "█   ", "    ", "    "],
    ["▓▓▓▓", "▒▒▒▒", "█   ", "    ", "    "],
    ["▓▓▓▓", "▓▓▓▓", "▓▓▓▓", "    ", "    "],
    ["▒▒▒▒", "    ", "█   ", "    ", "    "],
    ["    ", "    ", "█   ", "    ", "    "],
    ["    ", "    ", "▄   ", "    ", "    "],
    ["    ", "    ", " ▂  ", "    ", "    "],
    ["    ", "    ", "  ▄ ", "    ", "    "],
    ["    ", "    ", "   ▆", "    ", "    "],
    ["    ", "    ", "   █", "    ", "    "],
    ["    ", "   ▂", "   █", "    ", "    "],
    ["    ", "   ▄", "  ██", "    ", "    "],
    ["    ", "  ██", "  ██", "    ", "    "],
    ["    ", "  ██", "  ██", "    ", "    "],
    ["    ", "  ██", "  ██", "    ", "    "],
    ["    ", "  ██", "  ██", "    ", "    "],
    ["    ", "  ▄▄", "  ██", "    ", "    "],
    ["    ", "    ", "   █", "    ", "    "],
    ["    ", "    ", "   ▄", "    ", "    "],
    ["    ", "    ", "  ▂ ", "    ", "    "],
    ["    ", "    ", " █  ", "    ", "    "],
    ["    ", "    ", " ▃  ", " ▆  ", "    "],
    ["    ", "    ", "    ", " █  ", "    "],
    ["    ", "    ", "    ", " █  ", "    "],
    ["    ", "    ", "    ", " █  ", "    "],
    ["▒▒▒▒", "    ", "    ", " █  ", "    "],
    ["▓▓▓▓", "▒▒▒▒", "    ", " █  ", "    "],
    ["▓▓▓▓", "▓▓▓▓", "▒▒▒▒", " █  ", "    "],
    ["▓▓▓▓", "▓▓▓▓", "▓▓▓▓", "▒▒▒▒", "    "],
    ["▓▓▓▓", "▓▓▓▓", "▓▓▓▓", "▓▓▓▓", "    "],
    ["▓▓▓▓", "▓▓▓▓", "▓▓▓▓", "▓▓▓▓", "    "],
    ["▓▓▓▓", "▓▓▓▓", "▓▓▓▓", "▓▓▓▓", "    "],
    ["▓▓▓▓", "▓▓▓▓", "▒▒▒▒", " █  ", "    "],
    ["▓▓▓▓", "▒▒▒▒", "    ", " █  ", "    "],
    ["▒▒▒▒", "    ", "    ", " █  ", "    "],
    ["    ", "    ", "    ", " █  ", "    "],
    ["    ", "    ", " ▂  ", " ▇  ", "    "],
    ["    ", "    ", " ▄  ", " ▅  ", "    "],
    ["    ", "    ", " ▆  ", " ▃  ", "    "],
    ["    ", "    ", " █  ", "    ", "    "],
    ["    ", " ▂  ", " ▇  ", "    ", "    "],
    ["    ", " ▄  ", " ▄  ", "    ", "    "],
    ["    ", " ▆  ", " ▂  ", "    ", "    "],
    ["    ", "█   ", "    ", "    ", "    "],
    ["▒▒▒▒", "█   ", "    ", "    ", "    "],
    ["▓▓▓▓", "▒▒▒▒", "    ", "    ", "    "],
    ["▒▒▒▒", "█   ", "    ", "    ", "    "],
    ["▓▓▓▓", "▒▒▒▒", "    ", "    ", "    "],
    ["    ", "█   ", "    ", "    ", "    "],
    ["    ", " ▄  ", "    ", "    ", "    "],
    ["    ", "    ", " █  ", "    ", "    "],
    ["    ", "    ", " ▂  ", "    ", "    "],
    ["    ", "    ", " ▄  ", "    ", "    "],
    ["    ", "    ", " ▆  ", "    ", "    "],
    ["    ", "    ", " █  ", "    ", "    "],
    ["    ", "    ", " █  ", "    ", "    "],
];
const STARTUP_COMPACT_WORDMARK: &[&str] = &[
    "╷  ╭─╮╭─╮╭╮╷╭─╴",
    "│  │ ││ ││╰┤│╶╮",
    "╰─╴╰─╯╰─╯╵ ╵╰─╯",
    "",
    "",
    "",
];
const STARTUP_FULL_WORDMARK_MARGIN: usize = 8;
const STARTUP_COMPACT_WORDMARK_MARGIN: usize = 4;
const STARTUP_LOGO_EYE_FRAME_MS: u64 = 80;
const STARTUP_EYE_GUIDED_BLINK_PERIOD_STEPS: u64 = 18;
const STARTUP_EYE_GUIDED_BLINK_WINDOW_STEPS: u64 = 2;
const STARTUP_TIP_HOLD_MS: u64 = 2600;
const STARTUP_TIP_FADE_MS: u64 = 420;
const STARTUP_TIP_FRAME_MS: u64 = 70;
const STARTUP_TIP_INTENSITY_STEPS: u64 = 6;
const PROVIDER_ERROR_MAX_DETAIL_ITEMS: usize = 3;
const PROVIDER_ERROR_MAX_WRAPPED_LINES_PER_DETAIL: usize = 2;
const IMAGE_PREVIEW_MAX_BYTES: u64 = 12 * 1024 * 1024;
const IMAGE_PREVIEW_MAX_COLUMNS: u32 = 64;
const IMAGE_PREVIEW_MAX_ROWS: u32 = 12;
const READ_TEXT_PREVIEW_MAX_LINES: usize = 6;
const TOOL_STREAM_PREVIEW_MAX_LINES: usize = 4;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupEyeFocus {
    Center,
    Left,
    Right,
    Up,
    DownCenter,
    DownLeft,
    DownRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupEyeAnimation {
    Ambient,
    Focus(StartupEyeFocus),
    Thinking(StartupEyeFocus),
    Confirm(StartupEyeFocus),
    Celebrate,
}

const STARTUP_EYE_CENTER: StartupEyeFrame = ["    ", " ▂  ", " █  ", " ▄  ", "    "];
const STARTUP_EYE_LEFT: StartupEyeFrame = ["    ", "▂   ", "█   ", "▄   ", "    "];
const STARTUP_EYE_RIGHT: StartupEyeFrame = ["    ", "  ▂ ", "  █ ", "  ▄ ", "    "];
const STARTUP_EYE_UP: StartupEyeFrame = [" ▂  ", " █  ", " ▄  ", "    ", "    "];
const STARTUP_EYE_DOWN_CENTER: StartupEyeFrame = ["    ", "    ", " ▂  ", " █  ", " ▄  "];
const STARTUP_EYE_DOWN_LEFT: StartupEyeFrame = ["    ", "    ", "▂   ", "█   ", "▄   "];
const STARTUP_EYE_DOWN_RIGHT: StartupEyeFrame = ["    ", "    ", "  ▂ ", "  █ ", "  ▄ "];
const STARTUP_EYE_HALF_LID_CENTER: StartupEyeFrame = ["▒▒▒▒", "    ", " █  ", "    ", "    "];
const STARTUP_EYE_HALF_LID_LEFT: StartupEyeFrame = ["▒▒▒▒", "    ", "█   ", "    ", "    "];
const STARTUP_EYE_HALF_LID_RIGHT: StartupEyeFrame = ["▒▒▒▒", "    ", "  █ ", "    ", "    "];
const STARTUP_EYE_HALF_LID_DOWN_CENTER: StartupEyeFrame = ["    ", "▒▒▒▒", " ▂  ", " █  ", "    "];
const STARTUP_EYE_HALF_LID_DOWN_LEFT: StartupEyeFrame = ["    ", "▒▒▒▒", "▂   ", "█   ", "    "];
const STARTUP_EYE_HALF_LID_DOWN_RIGHT: StartupEyeFrame = ["    ", "▒▒▒▒", "  ▂ ", "  █ ", "    "];
const STARTUP_EYE_CONFIRM_CENTER: StartupEyeFrame = ["    ", " ▆  ", " █  ", " ▆  ", "    "];
const STARTUP_EYE_CONFIRM_DOWN_CENTER: StartupEyeFrame = ["    ", "    ", " ▄  ", " █  ", " ▇  "];
const STARTUP_EYE_CONFIRM_LEFT: StartupEyeFrame = ["    ", "▆   ", "█   ", "▆   ", "    "];
const STARTUP_EYE_CONFIRM_RIGHT: StartupEyeFrame = ["    ", "  ▆ ", "  █ ", "  ▆ ", "    "];
const STARTUP_EYE_CONFIRM_DOWN_LEFT: StartupEyeFrame = ["    ", "    ", "▄   ", "█   ", "▇   "];
const STARTUP_EYE_CONFIRM_DOWN_RIGHT: StartupEyeFrame = ["    ", "    ", "  ▄ ", "  █ ", "  ▇ "];
const STARTUP_EYE_CELEBRATE_A: StartupEyeFrame = [" ▂▂ ", " ▇▇ ", " ▄▄ ", "    ", "    "];
const STARTUP_EYE_CELEBRATE_B: StartupEyeFrame = [" ▄▄ ", " ▇▇ ", " ▂▂ ", "    ", "    "];

pub enum MessageContent {
    RenderedLines(Vec<String>),
    Markdown(String),
    Diff {
        title: Option<String>,
        content: String,
    },
    Image {
        alt: String,
        url: String,
    },
    ToolCall {
        title: String,
        lines: Vec<String>,
        status: ToolStatus,
    },
    Error {
        title: String,
        summary: String,
        details: Vec<String>,
    },
    Compaction {
        turn_count: usize,
        summary: String,
        expanded: bool,
    },
    StartupHeader {
        version: String,
        tutorial: String,
        sections: Vec<(String, Vec<String>)>,
        tips: Vec<String>,
        eye_animation: StartupEyeAnimation,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    Success,
    Error,
}

pub struct Message {
    pub role: String,
    pub contents: Vec<MessageContent>,
}

pub struct MessageList {
    pub messages: Vec<Message>,
    page_step: u16,
    mouse_step: u16,
    last_render_height: u16,
    scroll_state: TranscriptScrollState,
    render_revision: u64,
    render_cache: Option<RenderCache>,
    viewport_cache: Option<ViewportRenderCache>,
    startup_animation_started_at: Instant,
    last_startup_animation_signature: Option<u64>,
}

struct RenderCache {
    width: u16,
    revision: u64,
    lines: Vec<Line<'static>>,
}

#[derive(Clone)]
struct ViewportRenderCache {
    width: u16,
    revision: u64,
    height: u16,
    scroll_start: usize,
    top_padding: usize,
    lines: Vec<Line<'static>>,
}

impl MessageList {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            page_step: 12,
            mouse_step: 3,
            last_render_height: 0,
            scroll_state: TranscriptScrollState::new(),
            render_revision: 0,
            render_cache: None,
            viewport_cache: None,
            startup_animation_started_at: Instant::now(),
            last_startup_animation_signature: None,
        }
    }

    pub fn add_user_message(&mut self, msg: String) {
        self.messages.push(Message {
            role: "You".to_string(),
            contents: vec![MessageContent::Markdown(msg)],
        });
        self.scroll_state.prepare_for_appended_content();
        self.invalidate_render_cache();
    }

    pub fn add_assistant_message(&mut self, msg: String) {
        let contents = build_assistant_contents(&msg);
        self.messages.push(Message {
            role: "Assistant".to_string(),
            contents,
        });
        self.scroll_state.prepare_for_appended_content();
        self.invalidate_render_cache();
    }

    pub fn add_rendered_lines(&mut self, lines: Vec<String>) {
        self.messages.push(Message {
            role: "System".to_string(),
            contents: vec![MessageContent::RenderedLines(lines)],
        });
        self.scroll_state.prepare_for_appended_content();
        self.invalidate_render_cache();
    }

    pub fn clear_transcript(&mut self) {
        self.messages.clear();
        self.scroll_state.reset();
        self.last_startup_animation_signature = None;
        self.invalidate_render_cache();
    }

    pub fn latest_copy_text(&self) -> Option<String> {
        self.messages
            .iter()
            .rev()
            .filter(|message| message.role != "System")
            .filter_map(message_plain_text)
            .find(|text| !text.trim().is_empty())
            .or_else(|| {
                self.messages
                    .iter()
                    .rev()
                    .filter_map(message_plain_text)
                    .find(|text| !text.trim().is_empty())
            })
    }

    pub fn export_markdown(&self) -> String {
        let mut sections = Vec::new();
        for message in &self.messages {
            let Some(body) = message_plain_text(message) else {
                continue;
            };
            if body.trim().is_empty() {
                continue;
            }
            sections.push(format!("## {}\n\n{}", message.role, body.trim_end()));
        }
        sections.join("\n\n")
    }

    #[cfg(test)]
    pub fn add_startup_header(
        &mut self,
        version: String,
        tutorial: String,
        sections: Vec<(String, Vec<String>)>,
    ) {
        self.add_startup_header_with_tips(version, tutorial, sections, Vec::new());
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn add_startup_header_with_tips(
        &mut self,
        version: String,
        tutorial: String,
        sections: Vec<(String, Vec<String>)>,
        tips: Vec<String>,
    ) {
        self.add_startup_header_with_tips_and_eye(
            version,
            tutorial,
            sections,
            tips,
            StartupEyeAnimation::Ambient,
        );
    }

    pub fn add_startup_header_with_tips_and_eye(
        &mut self,
        version: String,
        tutorial: String,
        sections: Vec<(String, Vec<String>)>,
        tips: Vec<String>,
        eye_animation: StartupEyeAnimation,
    ) {
        self.startup_animation_started_at = Instant::now();
        self.last_startup_animation_signature = None;
        self.messages.push(Message {
            role: "System".to_string(),
            contents: vec![MessageContent::StartupHeader {
                version,
                tutorial,
                sections,
                tips,
                eye_animation,
            }],
        });
        self.scroll_state.prepare_for_appended_content();
        self.invalidate_render_cache();
    }

    #[allow(dead_code)]
    pub fn replace_latest_startup_header_with_tips(
        &mut self,
        version: String,
        tutorial: String,
        sections: Vec<(String, Vec<String>)>,
        tips: Vec<String>,
    ) {
        self.replace_latest_startup_header_with_eye(
            version,
            tutorial,
            sections,
            tips,
            StartupEyeAnimation::Ambient,
        );
    }

    pub fn replace_latest_startup_header_with_eye(
        &mut self,
        version: String,
        tutorial: String,
        sections: Vec<(String, Vec<String>)>,
        tips: Vec<String>,
        eye_animation: StartupEyeAnimation,
    ) {
        for message in self.messages.iter_mut().rev() {
            for content in message.contents.iter_mut().rev() {
                if let MessageContent::StartupHeader {
                    version: current_version,
                    tutorial: current_tutorial,
                    sections: current_sections,
                    tips: current_tips,
                    eye_animation: current_eye_animation,
                } = content
                {
                    *current_version = version;
                    *current_tutorial = tutorial;
                    *current_sections = sections;
                    *current_tips = tips;
                    *current_eye_animation = eye_animation;
                    self.invalidate_render_cache();
                    return;
                }
            }
        }

        self.add_startup_header_with_tips_and_eye(version, tutorial, sections, tips, eye_animation);
    }

    pub fn toggle_latest_compaction(&mut self) -> bool {
        for message in self.messages.iter_mut().rev() {
            for content in message.contents.iter_mut().rev() {
                if let MessageContent::Compaction { expanded, .. } = content {
                    *expanded = !*expanded;
                    self.invalidate_render_cache();
                    return true;
                }
            }
        }
        false
    }

    pub fn get_rendered_lines(&mut self, width: u16) -> Vec<Line<'static>> {
        self.ensure_render_cache(width).clone()
    }

    pub fn rendered_line_count(&mut self, width: u16) -> usize {
        self.ensure_render_cache(width).len()
    }

    fn ensure_render_cache(&mut self, width: u16) -> &Vec<Line<'static>> {
        let needs_rebuild = self
            .render_cache
            .as_ref()
            .is_none_or(|cache| cache.width != width || cache.revision != self.render_revision);
        if needs_rebuild {
            let lines = self.compute_rendered_lines(width);
            self.render_cache = Some(RenderCache {
                width,
                revision: self.render_revision,
                lines,
            });
        }
        self.render_cache
            .as_ref()
            .map(|cache| &cache.lines)
            .unwrap_or(&EMPTY_RENDER_LINES)
    }

    fn compute_rendered_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut text_lines = Vec::new();
        let mut previous_colored_block = false;

        for msg in &self.messages {
            for content in &msg.contents {
                let current_colored_block =
                    content_renders_colored_block(msg.role.as_str(), content);
                if previous_colored_block
                    && current_colored_block
                    && text_lines.last().is_none_or(|line| {
                        !(is_visual_blank_line(line) && dominant_block_bg(line).is_none())
                    })
                {
                    text_lines.push(Line::from(""));
                }
                match content {
                    MessageContent::RenderedLines(lines) => {
                        for line in lines {
                            if let Some(normalized) = normalize_rendered_system_line(line) {
                                if normalized.trim().is_empty() {
                                    text_lines.push(Line::from(""));
                                    continue;
                                }
                                text_lines.extend(render_rendered_system_line(&normalized, width));
                            }
                        }
                    }
                    MessageContent::StartupHeader {
                        version,
                        tutorial,
                        sections,
                        tips,
                        eye_animation,
                    } => {
                        let elapsed = self.startup_animation_started_at.elapsed();
                        let tip_state = startup_tip_render_state(tips, elapsed);
                        text_lines.extend(render_startup_header_lines(
                            version,
                            tutorial,
                            sections,
                            tip_state.as_ref(),
                            *eye_animation,
                            elapsed,
                            width,
                        ));
                    }
                    MessageContent::Markdown(md) => {
                        let is_user = msg.role == "You";
                        let markdown_width = width.saturating_sub(2) as usize;
                        let md_lines =
                            markdown::render_markdown_to_lines_with_width(md, Some(markdown_width));

                        if is_user {
                            let mut padding =
                                Line::from(vec![Span::raw(" ".repeat(width as usize))]);
                            for span in &mut padding.spans {
                                span.style = span.style.bg(SURFACE_USER_MSG_BG);
                            }
                            text_lines.push(padding);

                            for line in render_user_markdown_lines(md_lines, width) {
                                text_lines.push(user_block_line(line));
                            }
                            let mut padding =
                                Line::from(vec![Span::raw(" ".repeat(width as usize))]);
                            for span in &mut padding.spans {
                                span.style = span.style.bg(SURFACE_USER_MSG_BG);
                            }
                            text_lines.push(padding);
                        } else {
                            let wrapped_lines = wrap_assistant_markdown_lines(md_lines, width);
                            text_lines.push(Line::from(""));
                            text_lines.extend(wrapped_lines);
                            text_lines.push(Line::from(""));
                        }
                    }
                    MessageContent::Diff { title, content } => {
                        text_lines.extend(render_diff_block_lines(
                            title.as_deref(),
                            content.as_str(),
                            width,
                        ));
                    }
                    MessageContent::Image { alt, url } => {
                        text_lines.extend(render_image_block_lines(alt, url, width));
                    }
                    MessageContent::ToolCall {
                        title,
                        lines,
                        status,
                    } => {
                        text_lines.extend(render_tool_block_lines(title, lines, *status, width));
                    }
                    MessageContent::Error {
                        title,
                        summary,
                        details,
                    } => {
                        text_lines.extend(render_error_block_lines(title, summary, details, width));
                    }
                    MessageContent::Compaction {
                        turn_count,
                        summary,
                        expanded,
                    } => {
                        text_lines.extend(render_compaction_block_lines(
                            *turn_count,
                            summary.as_str(),
                            *expanded,
                            width,
                        ));
                    }
                }
                previous_colored_block = current_colored_block;
            }
        }

        for line in &mut text_lines {
            let is_user_bg = line
                .spans
                .iter()
                .any(|span| span.style.bg == Some(SURFACE_USER_MSG_BG));
            let is_compaction_bg = line
                .spans
                .iter()
                .any(|span| span.style.bg == Some(SURFACE_COMPACTION_BG));
            let background = if is_user_bg {
                Some(SURFACE_USER_MSG_BG)
            } else if is_compaction_bg {
                Some(SURFACE_COMPACTION_BG)
            } else {
                None
            };
            if let Some(background) = background {
                pad_and_bg(line, width, background);
            } else {
                pad_plain(line, width);
            }
        }

        text_lines
    }

    fn invalidate_render_cache(&mut self) {
        self.render_revision = self.render_revision.saturating_add(1);
        self.render_cache = None;
        self.viewport_cache = None;
        self.scroll_state.note_cache_invalidated();
    }

    pub fn trailing_colored_block(&mut self, width: u16) -> bool {
        self.ensure_render_cache(width)
            .iter()
            .rev()
            .find(|line| !is_visual_blank_line(line) || dominant_block_bg(line).is_some())
            .and_then(dominant_block_bg)
            .is_some()
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        self.page_step = page_step_for_height(area.height);
        self.mouse_step = mouse_step_for_height(area.height);
        let startup_mode = self.startup_mode_active();
        let (rendered_line_count, top_padding) = {
            let rendered_lines = self.ensure_render_cache(area.width);
            let top_padding = if startup_mode {
                startup_top_padding(rendered_lines.len(), area.height)
            } else {
                0
            };
            (rendered_lines.len(), top_padding)
        };

        let total_lines = rendered_line_count.saturating_add(top_padding);
        if total_lines == 0 {
            self.last_render_height = area.height;
            self.scroll_state.reset_for_empty_render();
            f.render_widget(
                Paragraph::new(Text::from(Vec::<Line<'static>>::new())),
                area,
            );
            return;
        }
        let max_scroll_start = total_lines.saturating_sub(area.height as usize);
        let raw_scroll_val = self.scroll_state.raw_scroll_start(max_scroll_start);
        let mut scroll_start = if self.scroll_state.follow_tail() {
            raw_scroll_val
        } else if !self.scroll_state.snap_on_next_render() {
            self.scroll_state.last_scroll_start().min(max_scroll_start)
        } else {
            let text_lines = self.get_rendered_lines(area.width);
            let centered_lines = if startup_mode {
                vertically_center_startup_lines(text_lines, area.height)
            } else {
                text_lines
            };
            adjust_scroll_start_for_message_boundary(&centered_lines, raw_scroll_val)
        };
        scroll_start = scroll_start.min(max_scroll_start);
        self.last_render_height = area.height;
        self.scroll_state
            .apply_rendered_scroll_start(max_scroll_start, scroll_start);

        let visible_lines = self.viewport_lines(area.width, area.height, scroll_start, top_padding);
        let paragraph = Paragraph::new(Text::from(visible_lines));

        f.render_widget(paragraph, area);
    }

    fn viewport_lines(
        &mut self,
        width: u16,
        height: u16,
        scroll_start: usize,
        top_padding: usize,
    ) -> Vec<Line<'static>> {
        if let Some(cache) = self.viewport_cache.as_ref()
            && cache.width == width
            && cache.revision == self.render_revision
            && cache.height == height
            && cache.scroll_start == scroll_start
            && cache.top_padding == top_padding
        {
            return cache.lines.clone();
        }

        let visible_end = scroll_start.saturating_add(height as usize);
        let lines = {
            let rendered_lines = self.ensure_render_cache(width);
            (scroll_start..visible_end)
                .filter_map(|visual_index| {
                    if visual_index < top_padding {
                        Some(Line::from(""))
                    } else {
                        rendered_lines
                            .get(visual_index.saturating_sub(top_padding))
                            .cloned()
                    }
                })
                .collect::<Vec<_>>()
        };

        self.viewport_cache = Some(ViewportRenderCache {
            width,
            revision: self.render_revision,
            height,
            scroll_start,
            top_padding,
            lines: lines.clone(),
        });
        lines
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll_state.scroll_line_up(),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_state.scroll_line_down(),
            KeyCode::PageUp | KeyCode::Char(' ') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.scroll_state.scroll_page_up(self.page_step)
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.scroll_state.scroll_page_down(self.page_step)
            }
            KeyCode::Home => self.scroll_state.jump_home(),
            KeyCode::End => self.scroll_state.jump_end(),
            KeyCode::Backspace
            | KeyCode::Enter
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::PageUp
            | KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Delete
            | KeyCode::Insert
            | KeyCode::F(_)
            | KeyCode::Char(_)
            | KeyCode::Null
            | KeyCode::Esc
            | KeyCode::CapsLock
            | KeyCode::ScrollLock
            | KeyCode::NumLock
            | KeyCode::PrintScreen
            | KeyCode::Pause
            | KeyCode::Menu
            | KeyCode::KeypadBegin
            | KeyCode::Media(_)
            | KeyCode::Modifier(_) => {}
        }
    }

    pub fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        match mouse.kind {
            crossterm::event::MouseEventKind::ScrollUp => {
                self.scroll_state.scroll_page_up(self.mouse_step)
            }
            crossterm::event::MouseEventKind::ScrollDown => {
                self.scroll_state.scroll_page_down(self.mouse_step)
            }
            crossterm::event::MouseEventKind::Down(_)
            | crossterm::event::MouseEventKind::Up(_)
            | crossterm::event::MouseEventKind::Drag(_)
            | crossterm::event::MouseEventKind::Moved
            | crossterm::event::MouseEventKind::ScrollLeft
            | crossterm::event::MouseEventKind::ScrollRight => {}
        }
    }

    pub fn is_following_tail(&self) -> bool {
        self.scroll_state.follow_tail()
    }

    #[cfg(test)]
    pub(crate) fn scroll_offset_for_test(&self) -> u16 {
        self.scroll_state.scroll_offset()
    }

    #[cfg(test)]
    pub(crate) fn set_scroll_offset_for_test(&mut self, value: u16) {
        self.scroll_state.set_scroll_offset_for_test(value);
    }

    #[cfg(test)]
    pub(crate) fn last_scroll_start_for_test(&self) -> usize {
        self.scroll_state.last_scroll_start()
    }

    #[cfg(test)]
    pub(crate) fn set_last_scroll_start_for_test(&mut self, value: usize) {
        self.scroll_state.set_last_scroll_start_for_test(value);
    }

    #[cfg(test)]
    pub(crate) fn snap_scroll_on_next_render_for_test(&self) -> bool {
        self.scroll_state.snap_on_next_render()
    }

    #[cfg(test)]
    pub(crate) fn set_snap_scroll_on_next_render_for_test(&mut self, value: bool) {
        self.scroll_state.set_snap_on_next_render_for_test(value);
    }

    pub fn refresh_startup_animation(&mut self) -> bool {
        if reduced_motion_enabled() {
            self.last_startup_animation_signature = None;
            return false;
        }
        let signature = self.startup_animation_signature();
        if signature == self.last_startup_animation_signature {
            return false;
        }
        self.last_startup_animation_signature = signature;
        if signature.is_some() {
            self.invalidate_render_cache();
            return true;
        }
        false
    }

    pub fn startup_animation_active(&self) -> bool {
        self.startup_animation_signature().is_some()
    }

    fn startup_animation_signature(&self) -> Option<u64> {
        if reduced_motion_enabled() {
            return None;
        }
        if !self.startup_mode_active() {
            return None;
        }

        let elapsed = self.startup_animation_started_at.elapsed();
        let eye_signature = self
            .startup_eye_animation()
            .map(|animation| startup_eye_signature(animation, elapsed) as u64)
            .unwrap_or_else(|| startup_logo_eye_frame_index(elapsed) as u64);
        let tip_signature = self
            .startup_tips()
            .and_then(|tips| {
                let tip_count = tips.len();
                let (_, intensity_step) = startup_tip_cycle_state(tip_count, elapsed)?;
                let tip_index = startup_tip_index(tip_count, elapsed)?;
                Some(((tip_index as u64) << 8) | intensity_step)
            })
            .unwrap_or(0);

        Some((eye_signature << 16) | tip_signature)
    }

    fn startup_tips(&self) -> Option<&[String]> {
        if self
            .messages
            .iter()
            .any(|message| message.role == "You" || message.role == "Assistant")
        {
            return None;
        }

        self.messages
            .iter()
            .flat_map(|message| message.contents.iter())
            .find_map(|content| match content {
                MessageContent::StartupHeader { tips, .. } if !tips.is_empty() => {
                    Some(tips.as_slice())
                }
                MessageContent::RenderedLines(_)
                | MessageContent::Markdown(_)
                | MessageContent::Diff { .. }
                | MessageContent::Image { .. }
                | MessageContent::ToolCall { .. }
                | MessageContent::Error { .. }
                | MessageContent::Compaction { .. }
                | MessageContent::StartupHeader { .. } => None,
            })
    }

    fn startup_eye_animation(&self) -> Option<StartupEyeAnimation> {
        if self
            .messages
            .iter()
            .any(|message| message.role == "You" || message.role == "Assistant")
        {
            return None;
        }

        self.messages
            .iter()
            .flat_map(|message| message.contents.iter())
            .find_map(|content| match content {
                MessageContent::StartupHeader { eye_animation, .. } => Some(*eye_animation),
                MessageContent::RenderedLines(_)
                | MessageContent::Markdown(_)
                | MessageContent::Diff { .. }
                | MessageContent::Image { .. }
                | MessageContent::ToolCall { .. }
                | MessageContent::Error { .. }
                | MessageContent::Compaction { .. } => None,
            })
    }

    fn startup_mode_active(&self) -> bool {
        self.messages.iter().all(|message| message.role == "System")
            && self
                .messages
                .iter()
                .flat_map(|message| message.contents.iter())
                .any(|content| matches!(content, MessageContent::StartupHeader { .. }))
    }
}

fn page_step_for_height(height: u16) -> u16 {
    height.saturating_sub(2).max(1)
}

fn mouse_step_for_height(height: u16) -> u16 {
    page_step_for_height(height).saturating_add(3) / 4
}

fn pad_and_bg(line: &mut Line, width: u16, bg: Color) {
    let line_len: usize = line.spans.iter().map(|s| s.width()).sum();
    let pad_len = (width as usize).saturating_sub(line_len);
    if pad_len > 0 {
        line.spans.push(Span::raw(" ".repeat(pad_len)));
    }
    for span in &mut line.spans {
        span.style = span.style.bg(bg);
    }
}

fn pad_plain(line: &mut Line, width: u16) {
    let line_len: usize = line.spans.iter().map(|s| s.width()).sum();
    let pad_len = (width as usize).saturating_sub(line_len);
    if pad_len > 0 {
        line.spans.push(Span::raw(" ".repeat(pad_len)));
    }
}

fn adjust_scroll_start_for_message_boundary(lines: &[Line<'static>], start: usize) -> usize {
    let adjusted_for_block = adjust_scroll_start_for_block_boundary(lines, start);
    if adjusted_for_block == start && lines.get(start).and_then(dominant_block_bg).is_none() {
        return start;
    }
    let start = adjusted_for_block;
    if start == 0 || start >= lines.len() || lines.get(start).is_some_and(is_visual_blank_line) {
        return start;
    }

    let lookback = start.saturating_sub(4);
    for index in (lookback..start).rev() {
        if lines.get(index).is_some_and(is_visual_blank_line) {
            return index + 1;
        }
    }
    start
}

fn adjust_scroll_start_for_block_boundary(lines: &[Line<'static>], start: usize) -> usize {
    if start == 0 || start >= lines.len() {
        return start;
    }
    if lines.get(start).is_some_and(|line| line.spans.is_empty()) {
        return start;
    }
    let Some(bg) = lines.get(start).and_then(dominant_block_bg) else {
        return start;
    };
    let mut adjusted = start;
    while adjusted > 0 && lines.get(adjusted - 1).and_then(dominant_block_bg) == Some(bg) {
        adjusted -= 1;
    }
    if lines.get(adjusted).is_some_and(is_visual_blank_line) {
        let mut candidate = adjusted + 1;
        while candidate < lines.len()
            && lines.get(candidate).and_then(dominant_block_bg) == Some(bg)
        {
            if lines
                .get(candidate)
                .is_some_and(|line| !is_visual_blank_line(line))
            {
                return candidate;
            }
            candidate += 1;
        }
    }
    adjusted
}

fn is_visual_blank_line(line: &Line<'static>) -> bool {
    line.spans
        .iter()
        .all(|span| span.content.as_ref().trim().is_empty())
}

fn dominant_block_bg(line: &Line<'static>) -> Option<Color> {
    if line
        .spans
        .iter()
        .any(|span| span.style.bg == Some(SURFACE_USER_MSG_BG))
    {
        return Some(SURFACE_USER_MSG_BG);
    }
    if line
        .spans
        .iter()
        .any(|span| span.style.bg == Some(SURFACE_TOOL_BG))
    {
        return Some(SURFACE_TOOL_BG);
    }
    if line
        .spans
        .iter()
        .any(|span| span.style.bg == Some(SURFACE_COMPACTION_BG))
    {
        return Some(SURFACE_COMPACTION_BG);
    }
    None
}

fn normalize_rendered_system_line(line: &str) -> Option<String> {
    let trimmed = line.trim_end();

    if trimmed.starts_with("╭─ ") {
        return Some(trimmed.trim_start_matches("╭─ ").to_owned());
    }
    if trimmed == "╰─" {
        return None;
    }
    if trimmed == "│" {
        return Some(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("│ ") {
        return Some(rest.to_owned());
    }
    if let Some(rest) = trimmed.strip_prefix("│") {
        return Some(rest.trim_start().to_owned());
    }

    Some(trimmed.to_owned())
}

fn render_rendered_system_line(line: &str, width: u16) -> Vec<Line<'static>> {
    let content_width = width as usize;

    if let Some(rendered) = render_system_activity_headline(line, content_width) {
        return rendered;
    }

    if let Some(rendered) = render_system_activity_child(line, content_width) {
        return rendered;
    }

    let style = if line.trim_start().starts_with("… +") {
        Style::default()
            .fg(SURFACE_GRAY)
            .add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(SURFACE_DARK_GRAY)
    };

    crate::presentation::render_wrapped_display_line(line, content_width)
        .into_iter()
        .map(|wrapped| Line::from(vec![Span::styled(wrapped, style)]))
        .collect()
}

fn render_system_activity_headline(line: &str, content_width: usize) -> Option<Vec<Line<'static>>> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("• ")?;

    let (label, body, label_style) = if let Some(body) = rest.strip_prefix("Ran ") {
        (
            "Ran",
            body,
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
        )
    } else if let Some(body) = rest.strip_prefix("Explored ") {
        (
            "Explored",
            body,
            Style::default()
                .fg(ratatui::style::Color::White)
                .add_modifier(Modifier::BOLD),
        )
    } else if let Some(body) = rest.strip_prefix("Called ") {
        (
            "Called",
            body,
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
        )
    } else if let Some(body) = rest.strip_prefix("Closed ") {
        (
            "Closed",
            body,
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        return None;
    };

    let body_width = content_width
        .saturating_sub(2 + crate::presentation::display_width(label) + 1)
        .max(1);
    let wrapped = crate::presentation::render_wrapped_display_line(body, body_width);

    Some(
        wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                if index == 0 {
                    Line::from(vec![
                        Span::styled("• ", Style::default().fg(SURFACE_GREEN)),
                        Span::styled(format!("{label} "), label_style),
                        Span::styled(
                            wrapped_line,
                            Style::default().fg(ratatui::style::Color::White),
                        ),
                    ])
                } else {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::raw(" ".repeat(crate::presentation::display_width(label) + 1)),
                        Span::styled(
                            wrapped_line,
                            Style::default().fg(ratatui::style::Color::White),
                        ),
                    ])
                }
            })
            .collect(),
    )
}

fn render_system_activity_child(line: &str, content_width: usize) -> Option<Vec<Line<'static>>> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("└ ")?;

    let (label, body) = if let Some(body) = rest.strip_prefix("Read ") {
        ("Read", body)
    } else if let Some(body) = rest.strip_prefix("List ") {
        ("List", body)
    } else if let Some(body) = rest.strip_prefix("Search ") {
        ("Search", body)
    } else if let Some(body) = rest.strip_prefix("Inspect ") {
        ("Inspect", body)
    } else {
        return None;
    };

    let body_width = content_width
        .saturating_sub(2 + 2 + crate::presentation::display_width(label) + 1)
        .max(1);
    let wrapped = crate::presentation::render_wrapped_display_line(body, body_width);

    Some(
        wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                if index == 0 {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            "└ ",
                            Style::default()
                                .fg(SURFACE_GRAY)
                                .add_modifier(Modifier::DIM),
                        ),
                        Span::styled(format!("{label} "), Style::default().fg(SURFACE_ACCENT)),
                        Span::styled(
                            wrapped_line,
                            Style::default().fg(ratatui::style::Color::White),
                        ),
                    ])
                } else {
                    Line::from(vec![
                        Span::raw("    "),
                        Span::raw(" ".repeat(crate::presentation::display_width(label) + 1)),
                        Span::styled(
                            wrapped_line,
                            Style::default().fg(ratatui::style::Color::White),
                        ),
                    ])
                }
            })
            .collect(),
    )
}

fn content_renders_colored_block(role: &str, content: &MessageContent) -> bool {
    match content {
        MessageContent::Markdown(_) => role == "You",
        MessageContent::Diff { .. }
        | MessageContent::ToolCall { .. }
        | MessageContent::Compaction { .. } => true,
        MessageContent::Error { .. }
        | MessageContent::RenderedLines(_)
        | MessageContent::Image { .. }
        | MessageContent::StartupHeader { .. } => false,
    }
}

fn render_startup_header_lines(
    version: &str,
    tutorial: &str,
    sections: &[(String, Vec<String>)],
    tip_state: Option<&StartupTipRenderState>,
    eye_animation: StartupEyeAnimation,
    elapsed: Duration,
    width: u16,
) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();

    rendered.push(Line::from(""));
    rendered.push(Line::from(""));
    rendered.extend(render_centered_logo_lines(width, eye_animation, elapsed));
    rendered.push(Line::from(""));
    rendered.extend(render_centered_startup_text_lines(
        version,
        width,
        Style::default()
            .fg(SURFACE_ACCENT)
            .add_modifier(Modifier::BOLD),
    ));

    let startup_status = sections
        .iter()
        .filter_map(|(title, values)| {
            values.first().map(|value| StartupStatusItem {
                title: title.clone(),
                value: value.clone(),
                count: value.parse::<usize>().ok(),
            })
        })
        .collect::<Vec<_>>();
    if !startup_status.is_empty() {
        rendered.push(Line::from(""));
        rendered.extend(render_startup_status_lines(&startup_status, width));
    }

    rendered.push(Line::from(""));
    let mut rendered_tip = false;
    if let Some(tip_state) = tip_state {
        rendered.extend(render_startup_tip_lines(tip_state, width));
        rendered_tip = true;
    } else if !tutorial.trim().is_empty() {
        let fallback_tip = StartupTipRenderState::steady(format!("• {tutorial}"));
        rendered.extend(render_startup_tip_lines(&fallback_tip, width));
        rendered_tip = true;
    }
    if rendered_tip {
        rendered.push(Line::from(""));
    }

    rendered
}

#[derive(Debug, Clone)]
struct StartupStatusItem {
    title: String,
    value: String,
    count: Option<usize>,
}

impl StartupStatusItem {
    fn display_width(&self) -> usize {
        crate::presentation::display_width(self.label_text().as_str())
            + self
                .marker_text()
                .map_or(0, |marker| 1 + crate::presentation::display_width(marker))
    }

    fn label_text(&self) -> String {
        self.count.map_or_else(
            || format!("{} · {}", self.title, self.value),
            |count| format!("{} ({count})", self.title),
        )
    }

    fn marker_text(&self) -> Option<&'static str> {
        self.count.map(|count| if count > 0 { "✓" } else { "✗" })
    }

    fn marker_style(&self) -> Style {
        let color = self
            .count
            .map(|count| {
                if count > 0 {
                    SURFACE_GREEN
                } else {
                    SURFACE_RED
                }
            })
            .unwrap_or(SURFACE_GRAY);
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    }

    fn spans(&self) -> Vec<Span<'static>> {
        let mut spans = vec![Span::styled(
            self.label_text(),
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::BOLD),
        )];
        if let Some(marker) = self.marker_text() {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(marker, self.marker_style()));
        }
        spans
    }
}

fn render_startup_status_lines(items: &[StartupStatusItem], width: u16) -> Vec<Line<'static>> {
    const GAP: &str = "   ";
    let width = width as usize;
    let gap_width = crate::presentation::display_width(GAP);
    let joined_width = items
        .iter()
        .map(StartupStatusItem::display_width)
        .sum::<usize>()
        + gap_width.saturating_mul(items.len().saturating_sub(1));

    if joined_width <= width {
        let mut spans = vec![Span::raw(" ".repeat((width - joined_width) / 2))];
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw(GAP));
            }
            spans.extend(item.spans());
        }
        return vec![Line::from(spans)];
    }

    items
        .iter()
        .map(|item| {
            let item_width = item.display_width();
            let mut spans = vec![Span::raw(" ".repeat(width.saturating_sub(item_width) / 2))];
            spans.extend(item.spans());
            Line::from(spans)
        })
        .collect()
}

fn render_centered_logo_lines(
    width: u16,
    eye_animation: StartupEyeAnimation,
    elapsed: Duration,
) -> Vec<Line<'static>> {
    let max_logo_width = STARTUP_WORDMARK
        .iter()
        .map(|line| crate::presentation::display_width(line))
        .max()
        .unwrap_or(0);
    let compact_logo_width = STARTUP_COMPACT_WORDMARK
        .iter()
        .map(|line| crate::presentation::display_width(line))
        .max()
        .unwrap_or(0);

    let available_full_logo_width = width as usize;
    let (logo_lines, base_logo_lines): (Vec<String>, Vec<String>) =
        if max_logo_width.saturating_add(STARTUP_FULL_WORDMARK_MARGIN) <= available_full_logo_width
        {
            (
                startup_wordmark_eye_frame_for_animation(eye_animation, elapsed),
                STARTUP_WORDMARK
                    .iter()
                    .map(|line| (*line).to_owned())
                    .collect(),
            )
        } else if compact_logo_width.saturating_add(STARTUP_COMPACT_WORDMARK_MARGIN)
            <= available_full_logo_width
        {
            (
                STARTUP_COMPACT_WORDMARK
                    .iter()
                    .map(|line| (*line).to_owned())
                    .collect(),
                STARTUP_COMPACT_WORDMARK
                    .iter()
                    .map(|line| (*line).to_owned())
                    .collect(),
            )
        } else {
            (vec!["LOONG".to_owned()], vec!["LOONG".to_owned()])
        };
    let target_width = logo_lines
        .iter()
        .map(|line| crate::presentation::display_width(line))
        .max()
        .unwrap_or(0);

    logo_lines
        .into_iter()
        .zip(base_logo_lines)
        .map(|(line, base_line)| {
            let centered = center_text_for_width(
                pad_text_to_display_width(line.as_str(), target_width).as_str(),
                width as usize,
            );
            let centered_base = center_text_for_width(
                pad_text_to_display_width(base_line.as_str(), target_width).as_str(),
                width as usize,
            );
            startup_logo_line_spans(centered, centered_base)
        })
        .collect()
}

fn startup_wordmark_eye_frame(frame_index: usize) -> Vec<String> {
    let Some(frame) = STARTUP_EYE_FRAMES
        .get(frame_index)
        .or_else(|| STARTUP_EYE_FRAMES.first())
    else {
        return STARTUP_WORDMARK
            .iter()
            .map(|line| (*line).to_owned())
            .collect();
    };

    STARTUP_WORDMARK
        .iter()
        .enumerate()
        .map(|(line_index, line)| {
            let Some(interior_row_index) = line_index.checked_sub(1) else {
                return (*line).to_owned();
            };
            let Some(pattern) = frame.get(interior_row_index) else {
                return (*line).to_owned();
            };
            apply_startup_eye_pattern(line, pattern)
        })
        .collect()
}

fn startup_wordmark_eye_frame_for_animation(
    animation: StartupEyeAnimation,
    elapsed: Duration,
) -> Vec<String> {
    match animation {
        StartupEyeAnimation::Ambient => {
            startup_wordmark_eye_frame(startup_logo_eye_frame_index(elapsed))
        }
        StartupEyeAnimation::Focus(_)
        | StartupEyeAnimation::Thinking(_)
        | StartupEyeAnimation::Confirm(_)
        | StartupEyeAnimation::Celebrate => {
            let (left_frame, right_frame, _) = startup_eye_frame_for_animation(animation, elapsed);
            STARTUP_WORDMARK
                .iter()
                .enumerate()
                .map(|(line_index, line)| {
                    let Some(interior_row_index) = line_index.checked_sub(1) else {
                        return (*line).to_owned();
                    };
                    let Some(left_pattern) = left_frame.get(interior_row_index) else {
                        return (*line).to_owned();
                    };
                    let Some(right_pattern) = right_frame.get(interior_row_index) else {
                        return (*line).to_owned();
                    };
                    apply_startup_eye_patterns(line, left_pattern, right_pattern)
                })
                .collect()
        }
    }
}

fn startup_eye_signature(animation: StartupEyeAnimation, elapsed: Duration) -> u16 {
    let (_, _, signature) = startup_eye_frame_for_animation(animation, elapsed);
    signature
}

fn startup_eye_frame_for_animation(
    animation: StartupEyeAnimation,
    elapsed: Duration,
) -> (&'static StartupEyeFrame, &'static StartupEyeFrame, u16) {
    match animation {
        StartupEyeAnimation::Ambient => {
            let frame_index = startup_logo_eye_frame_index(elapsed);
            let left_frame = STARTUP_EYE_FRAMES
                .get(frame_index)
                .or_else(|| STARTUP_EYE_FRAMES.first())
                .unwrap_or(&STARTUP_EYE_CENTER);
            let right_index = frame_index.saturating_add(3) % STARTUP_EYE_FRAMES.len().max(1);
            let right_frame = STARTUP_EYE_FRAMES
                .get(right_index)
                .or_else(|| STARTUP_EYE_FRAMES.first())
                .unwrap_or(&STARTUP_EYE_CENTER);
            (
                left_frame,
                right_frame,
                ((frame_index as u16) << 8) | right_index as u16,
            )
        }
        StartupEyeAnimation::Focus(focus) => {
            let step = startup_eye_step(elapsed, STARTUP_EYE_GUIDED_BLINK_PERIOD_STEPS);
            let (left_frame, right_frame) = if step + STARTUP_EYE_GUIDED_BLINK_WINDOW_STEPS
                >= STARTUP_EYE_GUIDED_BLINK_PERIOD_STEPS
            {
                startup_eye_half_lid_pair(focus)
            } else {
                startup_eye_focus_pair(focus)
            };
            (
                left_frame,
                right_frame,
                100 + startup_eye_focus_code(focus) + u16::from(step > 0),
            )
        }
        StartupEyeAnimation::Thinking(focus) => {
            let variant = startup_timed_choice(
                elapsed,
                &[
                    (260, 0u16),
                    (110, 1),
                    (180, 2),
                    (90, 3),
                    (220, 4),
                    (110, 5),
                    (320, 6),
                ],
            );
            let (left_frame, right_frame) = match variant {
                0 | 6 => startup_eye_focus_pair(focus),
                1 => startup_eye_neighbor_pair(focus, -1),
                2 => startup_eye_half_lid_pair(focus),
                3 => startup_eye_focus_pair(focus),
                4 => startup_eye_neighbor_pair(focus, 1),
                5 => startup_eye_half_lid_pair(focus),
                _ => startup_eye_focus_pair(focus),
            };
            (
                left_frame,
                right_frame,
                200 + startup_eye_focus_code(focus) * 8 + variant,
            )
        }
        StartupEyeAnimation::Confirm(focus) => {
            let variant = startup_timed_choice(
                elapsed,
                &[(70, 0u16), (90, 1), (80, 2), (120, 3), (90, 4), (180, 5)],
            );
            let (left_frame, right_frame) = match variant {
                0 | 5 => startup_eye_focus_pair(focus),
                1 | 2 => startup_eye_confirm_pair(focus),
                3 => startup_eye_half_lid_pair(focus),
                4 => startup_eye_confirm_pair(focus),
                _ => startup_eye_focus_pair(focus),
            };
            (
                left_frame,
                right_frame,
                300 + startup_eye_focus_code(focus) * 8 + variant,
            )
        }
        StartupEyeAnimation::Celebrate => {
            let variant = startup_timed_choice(
                elapsed,
                &[
                    (90, 0u16),
                    (100, 1),
                    (120, 2),
                    (100, 3),
                    (120, 4),
                    (100, 5),
                    (220, 6),
                ],
            );
            let (left_frame, right_frame) = match variant {
                0 | 4 => (&STARTUP_EYE_CONFIRM_LEFT, &STARTUP_EYE_CONFIRM_RIGHT),
                1 => (&STARTUP_EYE_CELEBRATE_A, &STARTUP_EYE_CELEBRATE_B),
                2 => (&STARTUP_EYE_CELEBRATE_B, &STARTUP_EYE_CELEBRATE_A),
                3 => (&STARTUP_EYE_CONFIRM_CENTER, &STARTUP_EYE_CONFIRM_CENTER),
                5 => (&STARTUP_EYE_CONFIRM_RIGHT, &STARTUP_EYE_CONFIRM_LEFT),
                6 => (&STARTUP_EYE_CENTER, &STARTUP_EYE_CENTER),
                _ => (&STARTUP_EYE_CENTER, &STARTUP_EYE_CENTER),
            };
            (left_frame, right_frame, 400 + variant)
        }
    }
}

fn startup_timed_choice(elapsed: Duration, schedule: &[(u64, u16)]) -> u16 {
    let total = schedule
        .iter()
        .map(|(duration, _)| *duration)
        .sum::<u64>()
        .max(1);
    let mut remaining = (elapsed.as_millis() as u64) % total;
    for (duration, value) in schedule {
        if remaining < *duration {
            return *value;
        }
        remaining = remaining.saturating_sub(*duration);
    }
    schedule.last().map(|(_, value)| *value).unwrap_or(0)
}

fn startup_eye_step(elapsed: Duration, period_steps: u64) -> u64 {
    (elapsed.as_millis() as u64 / STARTUP_LOGO_EYE_FRAME_MS.max(1)) % period_steps.max(1)
}

fn startup_eye_focus_frame(focus: StartupEyeFocus) -> &'static StartupEyeFrame {
    match focus {
        StartupEyeFocus::Center => &STARTUP_EYE_CENTER,
        StartupEyeFocus::Left => &STARTUP_EYE_LEFT,
        StartupEyeFocus::Right => &STARTUP_EYE_RIGHT,
        StartupEyeFocus::Up => &STARTUP_EYE_UP,
        StartupEyeFocus::DownCenter => &STARTUP_EYE_DOWN_CENTER,
        StartupEyeFocus::DownLeft => &STARTUP_EYE_DOWN_LEFT,
        StartupEyeFocus::DownRight => &STARTUP_EYE_DOWN_RIGHT,
    }
}

fn startup_eye_focus_pair(
    focus: StartupEyeFocus,
) -> (&'static StartupEyeFrame, &'static StartupEyeFrame) {
    match focus {
        StartupEyeFocus::Center => (&STARTUP_EYE_CENTER, &STARTUP_EYE_CENTER),
        StartupEyeFocus::Left => (&STARTUP_EYE_LEFT, &STARTUP_EYE_LEFT),
        StartupEyeFocus::Right => (&STARTUP_EYE_RIGHT, &STARTUP_EYE_RIGHT),
        StartupEyeFocus::Up => (&STARTUP_EYE_UP, &STARTUP_EYE_UP),
        StartupEyeFocus::DownCenter => (&STARTUP_EYE_DOWN_RIGHT, &STARTUP_EYE_DOWN_LEFT),
        StartupEyeFocus::DownLeft => (&STARTUP_EYE_DOWN_LEFT, &STARTUP_EYE_DOWN_CENTER),
        StartupEyeFocus::DownRight => (&STARTUP_EYE_DOWN_CENTER, &STARTUP_EYE_DOWN_RIGHT),
    }
}

#[allow(dead_code)]
fn startup_eye_half_lid_frame(focus: StartupEyeFocus) -> &'static StartupEyeFrame {
    match focus {
        StartupEyeFocus::Center | StartupEyeFocus::Up => &STARTUP_EYE_HALF_LID_CENTER,
        StartupEyeFocus::Left | StartupEyeFocus::DownLeft => &STARTUP_EYE_HALF_LID_LEFT,
        StartupEyeFocus::Right | StartupEyeFocus::DownRight => &STARTUP_EYE_HALF_LID_RIGHT,
        StartupEyeFocus::DownCenter => &STARTUP_EYE_HALF_LID_DOWN_CENTER,
    }
}

fn startup_eye_half_lid_pair(
    focus: StartupEyeFocus,
) -> (&'static StartupEyeFrame, &'static StartupEyeFrame) {
    match focus {
        StartupEyeFocus::Center => (&STARTUP_EYE_HALF_LID_CENTER, &STARTUP_EYE_HALF_LID_CENTER),
        StartupEyeFocus::Left => (&STARTUP_EYE_HALF_LID_LEFT, &STARTUP_EYE_HALF_LID_LEFT),
        StartupEyeFocus::Right => (&STARTUP_EYE_HALF_LID_RIGHT, &STARTUP_EYE_HALF_LID_RIGHT),
        StartupEyeFocus::Up => (&STARTUP_EYE_HALF_LID_CENTER, &STARTUP_EYE_HALF_LID_CENTER),
        StartupEyeFocus::DownCenter => (
            &STARTUP_EYE_HALF_LID_DOWN_RIGHT,
            &STARTUP_EYE_HALF_LID_DOWN_LEFT,
        ),
        StartupEyeFocus::DownLeft => (
            &STARTUP_EYE_HALF_LID_DOWN_LEFT,
            &STARTUP_EYE_HALF_LID_DOWN_CENTER,
        ),
        StartupEyeFocus::DownRight => (
            &STARTUP_EYE_HALF_LID_DOWN_CENTER,
            &STARTUP_EYE_HALF_LID_DOWN_RIGHT,
        ),
    }
}

#[allow(dead_code)]
fn startup_eye_confirm_frame(focus: StartupEyeFocus) -> &'static StartupEyeFrame {
    match focus {
        StartupEyeFocus::Center | StartupEyeFocus::Up => &STARTUP_EYE_CONFIRM_CENTER,
        StartupEyeFocus::Left | StartupEyeFocus::DownLeft => &STARTUP_EYE_CONFIRM_LEFT,
        StartupEyeFocus::Right | StartupEyeFocus::DownRight => &STARTUP_EYE_CONFIRM_RIGHT,
        StartupEyeFocus::DownCenter => &STARTUP_EYE_CONFIRM_DOWN_CENTER,
    }
}

fn startup_eye_confirm_pair(
    focus: StartupEyeFocus,
) -> (&'static StartupEyeFrame, &'static StartupEyeFrame) {
    match focus {
        StartupEyeFocus::Center => (&STARTUP_EYE_CONFIRM_CENTER, &STARTUP_EYE_CONFIRM_CENTER),
        StartupEyeFocus::Left => (&STARTUP_EYE_CONFIRM_LEFT, &STARTUP_EYE_CONFIRM_LEFT),
        StartupEyeFocus::Right => (&STARTUP_EYE_CONFIRM_RIGHT, &STARTUP_EYE_CONFIRM_RIGHT),
        StartupEyeFocus::Up => (&STARTUP_EYE_CONFIRM_CENTER, &STARTUP_EYE_CONFIRM_CENTER),
        StartupEyeFocus::DownCenter => (
            &STARTUP_EYE_CONFIRM_DOWN_RIGHT,
            &STARTUP_EYE_CONFIRM_DOWN_LEFT,
        ),
        StartupEyeFocus::DownLeft => (
            &STARTUP_EYE_CONFIRM_DOWN_LEFT,
            &STARTUP_EYE_CONFIRM_DOWN_CENTER,
        ),
        StartupEyeFocus::DownRight => (
            &STARTUP_EYE_CONFIRM_DOWN_CENTER,
            &STARTUP_EYE_CONFIRM_DOWN_RIGHT,
        ),
    }
}

#[allow(dead_code)]
fn startup_eye_neighbor_frame(
    focus: StartupEyeFocus,
    horizontal_delta: i8,
) -> &'static StartupEyeFrame {
    match (focus, horizontal_delta.signum()) {
        (StartupEyeFocus::DownCenter, -1) => &STARTUP_EYE_DOWN_LEFT,
        (StartupEyeFocus::DownCenter, 1) => &STARTUP_EYE_DOWN_RIGHT,
        (StartupEyeFocus::Center, -1) => &STARTUP_EYE_LEFT,
        (StartupEyeFocus::Center, 1) => &STARTUP_EYE_RIGHT,
        (StartupEyeFocus::Left | StartupEyeFocus::DownLeft, 1) => &STARTUP_EYE_CENTER,
        (StartupEyeFocus::Right | StartupEyeFocus::DownRight, -1) => &STARTUP_EYE_CENTER,
        (StartupEyeFocus::Up, -1) => &STARTUP_EYE_LEFT,
        (StartupEyeFocus::Up, 1) => &STARTUP_EYE_RIGHT,
        _ => startup_eye_focus_frame(focus),
    }
}

fn startup_eye_neighbor_pair(
    focus: StartupEyeFocus,
    horizontal_delta: i8,
) -> (&'static StartupEyeFrame, &'static StartupEyeFrame) {
    let (left_focus, right_focus) = match (focus, horizontal_delta.signum()) {
        (StartupEyeFocus::DownCenter, -1) => {
            (StartupEyeFocus::DownLeft, StartupEyeFocus::DownCenter)
        }
        (StartupEyeFocus::DownCenter, 1) => {
            (StartupEyeFocus::DownCenter, StartupEyeFocus::DownRight)
        }
        (StartupEyeFocus::Center, -1) => (StartupEyeFocus::Left, StartupEyeFocus::Center),
        (StartupEyeFocus::Center, 1) => (StartupEyeFocus::Center, StartupEyeFocus::Right),
        (StartupEyeFocus::Left, 1) => (StartupEyeFocus::Center, StartupEyeFocus::Right),
        (StartupEyeFocus::Right, -1) => (StartupEyeFocus::Left, StartupEyeFocus::Center),
        (StartupEyeFocus::Up, -1) => (StartupEyeFocus::Left, StartupEyeFocus::Center),
        (StartupEyeFocus::Up, 1) => (StartupEyeFocus::Center, StartupEyeFocus::Right),
        _ => (focus, focus),
    };
    (
        startup_eye_focus_frame(left_focus),
        startup_eye_focus_frame(right_focus),
    )
}

fn startup_eye_focus_code(focus: StartupEyeFocus) -> u16 {
    match focus {
        StartupEyeFocus::Center => 0,
        StartupEyeFocus::Left => 1,
        StartupEyeFocus::Right => 2,
        StartupEyeFocus::Up => 3,
        StartupEyeFocus::DownCenter => 4,
        StartupEyeFocus::DownLeft => 5,
        StartupEyeFocus::DownRight => 6,
    }
}

fn startup_logo_eye_frame_index(elapsed: Duration) -> usize {
    if reduced_motion_enabled() {
        return 0;
    }
    if STARTUP_EYE_FRAMES.is_empty() {
        return 0;
    }

    let sequence_step = elapsed.as_millis() as u64 / STARTUP_LOGO_EYE_FRAME_MS.max(1);
    sequence_step as usize % STARTUP_EYE_FRAMES.len()
}

fn apply_startup_eye_patterns(line: &str, left_pattern: &str, right_pattern: &str) -> String {
    debug_assert_eq!(
        left_pattern.chars().count(),
        STARTUP_EYE_INTERIOR_WIDTH,
        "startup eye pattern width must remain fixed"
    );
    debug_assert_eq!(
        right_pattern.chars().count(),
        STARTUP_EYE_INTERIOR_WIDTH,
        "startup eye pattern width must remain fixed"
    );

    let mut characters = line.chars().collect::<Vec<_>>();
    let cavity = STARTUP_EYE_CAVITY.chars().collect::<Vec<_>>();
    let left_pattern = left_pattern.chars().collect::<Vec<_>>();
    let right_pattern = right_pattern.chars().collect::<Vec<_>>();
    let mut search_from = 0;

    for eye_index in 0..2 {
        let Some(cavity_start) = find_startup_eye_cavity(&characters, &cavity, search_from) else {
            break;
        };
        let interior_start = cavity_start + 4;
        let pattern = if eye_index == 0 {
            &left_pattern
        } else {
            &right_pattern
        };
        for (offset, character) in pattern.iter().copied().enumerate() {
            if let Some(slot) = characters.get_mut(interior_start + offset) {
                *slot = character;
            }
        }
        search_from = cavity_start + cavity.len();
    }

    characters.into_iter().collect()
}

fn apply_startup_eye_pattern(line: &str, pattern: &str) -> String {
    apply_startup_eye_patterns(line, pattern, pattern)
}

fn find_startup_eye_cavity(haystack: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() || from >= haystack.len() {
        return None;
    }

    haystack
        .get(from..)
        .and_then(|tail| {
            tail.windows(needle.len())
                .position(|window| window == needle)
        })
        .map(|offset| from + offset)
}

fn startup_logo_line_spans(line: String, base_line: String) -> Line<'static> {
    let logo_style = Style::default()
        .fg(SURFACE_ACCENT)
        .add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();

    for (character, base_character) in line.chars().zip(base_line.chars()) {
        if character == ' ' {
            spans.push(Span::raw(" "));
        } else if base_character == ' ' && character != ' ' {
            spans.push(Span::styled(
                character.to_string(),
                startup_logo_eye_style(character),
            ));
        } else {
            spans.push(Span::styled(character.to_string(), logo_style));
        }
    }

    Line::from(spans)
}

fn startup_logo_eye_style(character: char) -> Style {
    let color = match character {
        '░' | '▁' | '▂' => SURFACE_DIM_GRAY,
        '▒' | '▃' | '▄' => SURFACE_GRAY,
        '▓' | '▅' | '▆' => SURFACE_ACCENT,
        '█' | '▇' => Color::White,
        _ => Color::White,
    };

    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn render_centered_startup_text_lines(text: &str, width: u16, style: Style) -> Vec<Line<'static>> {
    let content_width = width.saturating_sub(4).max(1) as usize;
    crate::presentation::render_wrapped_display_line(text, content_width)
        .into_iter()
        .map(|line| {
            let centered = center_text_for_width(line.as_str(), width as usize);
            Line::from(vec![Span::styled(centered, style)])
        })
        .collect()
}

fn center_text_for_width(text: &str, width: usize) -> String {
    let text_width = crate::presentation::display_width(text);
    if text_width >= width {
        return text.to_owned();
    }
    let left_pad = (width - text_width) / 2;
    format!("{}{}", " ".repeat(left_pad), text)
}

fn pad_text_to_display_width(text: &str, width: usize) -> String {
    let text_width = crate::presentation::display_width(text);
    if text_width >= width {
        return text.to_owned();
    }
    format!("{}{}", text, " ".repeat(width - text_width))
}

#[derive(Debug, Clone)]
struct StartupTipRenderState {
    text: String,
    bullet_color: Color,
    text_color: Color,
    emphasize: bool,
}

impl StartupTipRenderState {
    fn steady(text: String) -> Self {
        Self {
            text,
            bullet_color: SURFACE_ACCENT,
            text_color: Color::White,
            emphasize: true,
        }
    }
}

fn render_startup_tip_lines(tip_state: &StartupTipRenderState, width: u16) -> Vec<Line<'static>> {
    let content_width = width.saturating_sub(6).max(1) as usize;
    crate::presentation::render_wrapped_display_line(tip_state.text.as_str(), content_width)
        .into_iter()
        .map(|line| {
            let centered = center_text_for_width(line.as_str(), width as usize);
            let text_style = if tip_state.emphasize {
                Style::default()
                    .fg(tip_state.text_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(tip_state.text_color)
            };
            let indent_width = centered.len().saturating_sub(centered.trim_start().len());
            let (indent, body) = centered.split_at(indent_width);
            if let Some(rest) = body.strip_prefix("• ") {
                Line::from(vec![
                    Span::raw(indent.to_owned()),
                    Span::styled("• ", Style::default().fg(tip_state.bullet_color)),
                    Span::styled(rest.to_owned(), text_style),
                ])
            } else {
                Line::from(vec![Span::styled(centered, text_style)])
            }
        })
        .collect()
}

fn startup_tip_render_state(tips: &[String], elapsed: Duration) -> Option<StartupTipRenderState> {
    let tip_count = tips.len();
    if reduced_motion_enabled() {
        return tips
            .first()
            .map(|tip| StartupTipRenderState::steady(format!("• {tip}")));
    }
    let tip_index = startup_tip_index(tip_count, elapsed)?;
    let (_, intensity_step) = startup_tip_cycle_state(tip_count, elapsed)?;
    let max_step = STARTUP_TIP_INTENSITY_STEPS.max(1);
    let tip_text = format!("• {}", tips.get(tip_index)?);
    let text_color =
        interpolate_rgb_color(SURFACE_DIM_GRAY, Color::White, intensity_step, max_step);
    let bullet_color =
        interpolate_rgb_color(SURFACE_GRAY, SURFACE_ACCENT, intensity_step, max_step);

    Some(StartupTipRenderState {
        text: tip_text,
        bullet_color,
        text_color,
        emphasize: intensity_step >= max_step.saturating_sub(1),
    })
}

fn startup_tip_index(tip_count: usize, elapsed: Duration) -> Option<usize> {
    let tip_count = tip_count.max(1) as u64;
    let cycle_ms = STARTUP_TIP_HOLD_MS
        .saturating_add(STARTUP_TIP_FADE_MS)
        .saturating_add(STARTUP_TIP_FADE_MS);
    let cycle_index = elapsed.as_millis() as u64 / cycle_ms.max(1);
    let cycle_phase = elapsed.as_millis() as u64 % cycle_ms.max(1);
    let current = cycle_index % tip_count;

    if cycle_phase < STARTUP_TIP_HOLD_MS.saturating_add(STARTUP_TIP_FADE_MS) {
        Some(current as usize)
    } else {
        Some(((current + 1) % tip_count) as usize)
    }
}

fn startup_tip_cycle_state(tip_count: usize, elapsed: Duration) -> Option<(usize, u64)> {
    if tip_count == 0 {
        return None;
    }

    let cycle_ms = STARTUP_TIP_HOLD_MS
        .saturating_add(STARTUP_TIP_FADE_MS)
        .saturating_add(STARTUP_TIP_FADE_MS);
    let animation_ms = elapsed.as_millis() as u64 / STARTUP_TIP_FRAME_MS.max(1);
    let cycle_index = animation_ms.saturating_mul(STARTUP_TIP_FRAME_MS) / cycle_ms.max(1);
    let cycle_phase = animation_ms.saturating_mul(STARTUP_TIP_FRAME_MS) % cycle_ms.max(1);
    let current_index = (cycle_index % tip_count as u64) as usize;
    let intensity = if cycle_phase < STARTUP_TIP_HOLD_MS {
        STARTUP_TIP_INTENSITY_STEPS
    } else if cycle_phase < STARTUP_TIP_HOLD_MS.saturating_add(STARTUP_TIP_FADE_MS) {
        let fade_progress = cycle_phase.saturating_sub(STARTUP_TIP_HOLD_MS);
        STARTUP_TIP_INTENSITY_STEPS.saturating_sub(
            fade_progress.saturating_mul(STARTUP_TIP_INTENSITY_STEPS) / STARTUP_TIP_FADE_MS.max(1),
        )
    } else {
        let fade_progress = cycle_phase
            .saturating_sub(STARTUP_TIP_HOLD_MS)
            .saturating_sub(STARTUP_TIP_FADE_MS);
        fade_progress.saturating_mul(STARTUP_TIP_INTENSITY_STEPS) / STARTUP_TIP_FADE_MS.max(1)
    };

    Some((current_index, intensity.min(STARTUP_TIP_INTENSITY_STEPS)))
}

fn interpolate_rgb_color(from: Color, to: Color, numerator: u64, denominator: u64) -> Color {
    let (from_r, from_g, from_b) = rgb_channels(from);
    let (to_r, to_g, to_b) = rgb_channels(to);
    let denominator = denominator.max(1);

    let blend = |start: u8, end: u8| -> u8 {
        let start = start as i64;
        let end = end as i64;
        let delta = end - start;
        let step = start + delta * numerator as i64 / denominator as i64;
        step.clamp(0, 255) as u8
    };

    Color::Rgb(
        blend(from_r, to_r),
        blend(from_g, to_g),
        blend(from_b, to_b),
    )
}

fn rgb_channels(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Reset => (255, 255, 255),
        Color::Red => (255, 0, 0),
        Color::Green => (0, 128, 0),
        Color::Yellow => (255, 255, 0),
        Color::Blue => (0, 0, 255),
        Color::Magenta => (255, 0, 255),
        Color::Cyan => (0, 255, 255),
        Color::White => (255, 255, 255),
        Color::Black => (0, 0, 0),
        Color::Gray => (128, 128, 128),
        Color::DarkGray => (64, 64, 64),
        Color::LightRed => (255, 102, 102),
        Color::LightGreen => (144, 238, 144),
        Color::LightYellow => (255, 255, 153),
        Color::LightBlue => (173, 216, 230),
        Color::LightMagenta => (238, 130, 238),
        Color::LightCyan => (224, 255, 255),
        Color::Indexed(index) => (index, index, index),
    }
}

fn vertically_center_startup_lines(
    mut lines: Vec<Line<'static>>,
    available_height: u16,
) -> Vec<Line<'static>> {
    let top_padding = startup_top_padding(lines.len(), available_height);
    if top_padding == 0 {
        return lines;
    }

    let mut centered = Vec::with_capacity(lines.len() + top_padding);
    centered.extend((0..top_padding).map(|_| Line::from("")));
    centered.append(&mut lines);
    centered
}

fn startup_top_padding(line_count: usize, available_height: u16) -> usize {
    let available_height = available_height as usize;
    if line_count == 0 || line_count >= available_height {
        return 0;
    }

    ((available_height - line_count) / 2).max(2)
}

fn wrap_assistant_markdown_lines(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    let lines = normalize_blank_lines(lines);
    let content_width = width.saturating_sub(2) as usize;
    let mut rendered = Vec::new();
    let mut paragraph_buffer = String::new();
    let mut in_code_block = false;

    let flush_paragraph =
        |rendered: &mut Vec<Line<'static>>, paragraph_buffer: &mut String, content_width: usize| {
            if paragraph_buffer.trim().is_empty() {
                paragraph_buffer.clear();
                return;
            }
            rendered.extend(render_assistant_plain_line(
                paragraph_buffer.as_str(),
                content_width,
                assistant_line_style(paragraph_buffer),
            ));
            paragraph_buffer.clear();
        };

    for line in lines {
        let plain = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        let trimmed = plain.trim_start();
        if plain.trim().is_empty() {
            flush_paragraph(&mut rendered, &mut paragraph_buffer, content_width);
            rendered.push(Line::from(""));
            continue;
        }

        if trimmed.starts_with("```") {
            flush_paragraph(&mut rendered, &mut paragraph_buffer, content_width);
            rendered.extend(render_assistant_plain_line(
                plain.as_str(),
                content_width,
                assistant_line_style(&plain),
            ));
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            flush_paragraph(&mut rendered, &mut paragraph_buffer, content_width);
            rendered.extend(render_assistant_code_line(plain.as_str(), content_width));
            continue;
        }

        if let Some(split_bullets) = split_inline_bullet_runs(&plain) {
            flush_paragraph(&mut rendered, &mut paragraph_buffer, content_width);
            for bullet_line in split_bullets {
                rendered.extend(render_assistant_plain_line(
                    bullet_line.as_str(),
                    content_width,
                    assistant_line_style(&bullet_line),
                ));
            }
            continue;
        }

        if is_reflowable_assistant_line(&plain) {
            if !paragraph_buffer.is_empty() {
                paragraph_buffer.push_str(paragraph_joiner(&paragraph_buffer, &plain));
            }
            paragraph_buffer.push_str(plain.trim());
            continue;
        }

        flush_paragraph(&mut rendered, &mut paragraph_buffer, content_width);
        if is_assistant_table_line(plain.as_str()) {
            rendered.extend(render_assistant_table_line(plain.as_str(), content_width));
        } else {
            rendered.extend(render_assistant_plain_line(
                plain.as_str(),
                content_width,
                assistant_line_style(&plain),
            ));
        }
    }

    flush_paragraph(&mut rendered, &mut paragraph_buffer, content_width);

    rendered
}

fn is_assistant_table_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('┌')
        || trimmed.starts_with('├')
        || trimmed.starts_with('└')
        || trimmed.starts_with('│')
}

fn render_assistant_table_line(line: &str, content_width: usize) -> Vec<Line<'static>> {
    if crate::presentation::display_width(line) > content_width {
        return render_assistant_plain_line(line, content_width, assistant_line_style(line));
    }

    let border_style = Style::default().fg(SURFACE_DIM_GRAY);
    let cell_style = Style::default().fg(Color::White);
    let separator_line = line
        .trim()
        .chars()
        .all(|ch| ch.is_whitespace() || is_table_border_char(ch));
    let mut spans = vec![Span::raw("  ")];

    for ch in line.chars() {
        let style = if separator_line || is_table_border_char(ch) {
            border_style
        } else {
            cell_style
        };
        spans.push(Span::styled(ch.to_string(), style));
    }

    vec![Line::from(spans)]
}

fn is_table_border_char(ch: char) -> bool {
    matches!(
        ch,
        '┌' | '┬' | '┐' | '├' | '┼' | '┤' | '└' | '┴' | '┘' | '─' | '│'
    )
}

fn render_assistant_plain_line(
    line: &str,
    content_width: usize,
    style: Style,
) -> Vec<Line<'static>> {
    crate::presentation::render_wrapped_plain_display_line(line, content_width)
        .into_iter()
        .map(|wrapped| Line::from(vec![Span::raw("  "), Span::styled(wrapped, style)]))
        .collect()
}

fn render_assistant_code_line(line: &str, content_width: usize) -> Vec<Line<'static>> {
    let code_style = Style::default().fg(SURFACE_GREEN);
    let (gutter, code) = line
        .strip_prefix("  ")
        .map_or(("", line), |rest| ("  ", rest));
    let code_width = content_width
        .saturating_sub(crate::presentation::display_width(gutter))
        .max(1);

    crate::presentation::render_wrapped_display_line(code, code_width)
        .into_iter()
        .map(|wrapped| {
            let mut spans = vec![Span::raw("  ")];
            if !gutter.is_empty() {
                spans.push(Span::styled(gutter.to_owned(), code_style));
            }
            spans.push(Span::styled(wrapped, code_style));
            Line::from(spans)
        })
        .collect()
}

fn split_inline_bullet_runs(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if trimmed.matches("• ").count() < 2 {
        return None;
    }

    let items = trimmed
        .split("• ")
        .filter_map(|segment| {
            let segment = segment.trim();
            (!segment.is_empty()).then(|| format!("• {segment}"))
        })
        .collect::<Vec<_>>();

    (items.len() >= 2).then_some(items)
}

fn normalize_blank_lines(mut lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    while lines.first().is_some_and(is_visual_blank_line) {
        lines.remove(0);
    }
    while lines.last().is_some_and(is_visual_blank_line) {
        lines.pop();
    }

    let mut normalized = Vec::new();
    let mut last_was_blank = false;
    for line in lines {
        let is_blank = is_visual_blank_line(&line);
        if is_blank && last_was_blank {
            continue;
        }
        last_was_blank = is_blank;
        normalized.push(line);
    }
    normalized
}

fn render_user_markdown_lines(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    let mut plain_lines = lines
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    while plain_lines
        .first()
        .is_some_and(|line| line.trim().is_empty())
    {
        plain_lines.remove(0);
    }
    while plain_lines
        .last()
        .is_some_and(|line| line.trim().is_empty())
    {
        plain_lines.pop();
    }

    let content_width = width.saturating_sub(2) as usize;
    let mut rendered = Vec::new();
    for line in plain_lines {
        if line.trim().is_empty() {
            rendered.push(Line::from(vec![Span::raw("")]));
            continue;
        }
        for wrapped in
            crate::presentation::render_wrapped_display_line(line.as_str(), content_width)
        {
            rendered.push(Line::from(vec![Span::styled(
                wrapped,
                Style::default().fg(ratatui::style::Color::White),
            )]));
        }
    }
    rendered
}

fn is_reflowable_assistant_line(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty()
        && !trimmed.starts_with('#')
        && !trimmed.starts_with("```")
        && !trimmed.starts_with('┌')
        && !trimmed.starts_with('├')
        && !trimmed.starts_with('└')
        && !trimmed.starts_with('│')
        && !trimmed.starts_with("┃")
        && !trimmed.starts_with('>')
        && !trimmed.starts_with("- ")
        && !trimmed.starts_with("* ")
        && !trimmed.starts_with("• ")
        && !trimmed.starts_with("[image]")
}

fn paragraph_joiner(current: &str, next: &str) -> &'static str {
    if contains_cjk(current) || contains_cjk(next) {
        ""
    } else {
        " "
    }
}

fn contains_cjk(text: &str) -> bool {
    text.chars().any(|ch| {
        ('\u{4E00}'..='\u{9FFF}').contains(&ch)
            || ('\u{3040}'..='\u{30FF}').contains(&ch)
            || ('\u{AC00}'..='\u{D7AF}').contains(&ch)
    })
}

fn assistant_line_style(line: &str) -> Style {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        Style::default()
            .fg(SURFACE_HEADING)
            .add_modifier(Modifier::BOLD)
    } else if trimmed.starts_with("```") {
        Style::default().fg(SURFACE_DIM_GRAY)
    } else if trimmed.starts_with('┌')
        || trimmed.starts_with('├')
        || trimmed.starts_with('└')
        || trimmed.starts_with('│')
        || trimmed.starts_with("┃")
        || trimmed.starts_with('>')
    {
        Style::default().fg(SURFACE_GRAY)
    } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("• ")
    {
        Style::default().fg(ratatui::style::Color::White)
    } else if trimmed.starts_with("[image]") {
        Style::default().fg(SURFACE_ACCENT)
    } else {
        Style::default().fg(ratatui::style::Color::White)
    }
}

fn user_block_line(mut line: Line<'static>) -> Line<'static> {
    line.spans.insert(0, Span::raw("  "));
    if line.spans.is_empty() {
        line.spans
            .push(Span::styled("", Style::default().bg(SURFACE_USER_MSG_BG)));
        return line;
    }

    for span in &mut line.spans {
        span.style = span.style.bg(SURFACE_USER_MSG_BG);
    }
    line
}

fn build_assistant_contents(text: &str) -> Vec<MessageContent> {
    if let Some(body) = text.trim().strip_prefix(PROVIDER_ERROR_REPLY_PREFIX) {
        return vec![parse_provider_error_content(body)];
    }

    if is_compacted_summary_content(text) {
        return vec![parse_compaction_content(text)];
    }

    if !assistant_text_has_explicit_structure(text) {
        let mut contents = Vec::new();
        append_markdown_or_image_contents(text, &mut contents);
        if contents.is_empty() {
            contents.push(MessageContent::Markdown(text.to_owned()));
        }
        return contents;
    }

    let sections = super::super::parse_cli_chat_markdown_sections(text);
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
        append_markdown_or_image_contents(text, &mut contents);
    }

    if contents.is_empty() {
        contents.push(MessageContent::Markdown(text.to_owned()));
    }

    contents
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

fn render_run_tool_preview_block(preview: &RunToolPreview, width: u16) -> Vec<Line<'static>> {
    let bg = SURFACE_TOOL_BG;
    let mut rendered = Vec::new();
    rendered.push(background_line(width, bg));

    let content_width = width.saturating_sub(8).max(1) as usize;
    let command = truncate_middle_display(preview.command.as_str(), content_width);
    rendered.push(pad_preserving_backgrounds(
        Line::from(vec![
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled(
                "run ",
                Style::default()
                    .fg(SURFACE_DARK_GRAY)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(command, Style::default().fg(SURFACE_DARK_GRAY).bg(bg)),
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled(
                tool_status_label(preview.status),
                Style::default()
                    .fg(tool_status_color(preview.status))
                    .bg(bg),
            ),
        ]),
        width,
        bg,
    ));

    rendered.push(pad_preserving_backgrounds(
        Line::from(vec![
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled("tool: ", Style::default().fg(SURFACE_GRAY).bg(bg)),
            Span::styled(
                preview.tool_name.clone(),
                Style::default().fg(SURFACE_DARK_GRAY).bg(bg),
            ),
        ]),
        width,
        bg,
    ));

    rendered.extend(render_tool_stream_preview_section(
        "stdout",
        &preview.stdout,
        width,
        bg,
    ));
    rendered.extend(render_tool_stream_preview_section(
        "stderr",
        &preview.stderr,
        width,
        bg,
    ));

    if let Some(metrics) = preview.metrics.as_deref() {
        for wrapped in crate::presentation::render_wrapped_display_line(
            metrics,
            width.saturating_sub(12).max(1) as usize,
        ) {
            rendered.push(pad_preserving_backgrounds(
                Line::from(vec![
                    Span::styled("  ", Style::default().bg(bg)),
                    Span::styled("metrics ", Style::default().fg(SURFACE_GRAY).bg(bg)),
                    Span::styled(wrapped, Style::default().fg(SURFACE_DARK_GRAY).bg(bg)),
                ]),
                width,
                bg,
            ));
        }
    }

    rendered.push(background_line(width, bg));
    rendered
}

fn render_inspect_tool_preview_block(
    preview: &InspectToolPreview,
    width: u16,
) -> Vec<Line<'static>> {
    let bg = SURFACE_TOOL_BG;
    let mut rendered = Vec::new();
    rendered.push(background_line(width, bg));

    let content_width = width.saturating_sub(12).max(1) as usize;
    let primary = truncate_middle_display(preview.primary.as_str(), content_width);
    rendered.push(pad_preserving_backgrounds(
        Line::from(vec![
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled(
                format!("{} ", preview.kind),
                Style::default()
                    .fg(SURFACE_DARK_GRAY)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(primary, Style::default().fg(SURFACE_DARK_GRAY).bg(bg)),
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled(
                tool_status_label(preview.status),
                Style::default()
                    .fg(tool_status_color(preview.status))
                    .bg(bg),
            ),
        ]),
        width,
        bg,
    ));

    rendered.push(pad_preserving_backgrounds(
        Line::from(vec![
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled("tool: ", Style::default().fg(SURFACE_GRAY).bg(bg)),
            Span::styled(
                preview.tool_name.clone(),
                Style::default().fg(SURFACE_DARK_GRAY).bg(bg),
            ),
        ]),
        width,
        bg,
    ));

    rendered.extend(render_tool_stream_preview_section(
        "stdout",
        &preview.stdout,
        width,
        bg,
    ));
    rendered.extend(render_tool_stream_preview_section(
        "stderr",
        &preview.stderr,
        width,
        bg,
    ));

    if let Some(metrics) = preview.metrics.as_deref() {
        for wrapped in crate::presentation::render_wrapped_display_line(
            metrics,
            width.saturating_sub(12).max(1) as usize,
        ) {
            rendered.push(pad_preserving_backgrounds(
                Line::from(vec![
                    Span::styled("  ", Style::default().bg(bg)),
                    Span::styled("metrics ", Style::default().fg(SURFACE_GRAY).bg(bg)),
                    Span::styled(wrapped, Style::default().fg(SURFACE_DARK_GRAY).bg(bg)),
                ]),
                width,
                bg,
            ));
        }
    }

    rendered.push(background_line(width, bg));
    rendered
}

fn tool_status_label(status: ToolStatus) -> &'static str {
    match status {
        ToolStatus::Pending => "working",
        ToolStatus::Success => "ok",
        ToolStatus::Error => "failed",
    }
}

fn tool_status_color(status: ToolStatus) -> Color {
    match status {
        ToolStatus::Pending => SURFACE_CYAN,
        ToolStatus::Success => SURFACE_GREEN,
        ToolStatus::Error => SURFACE_RED,
    }
}

fn render_tool_stream_preview_section(
    label: &str,
    preview: &ToolStreamPreview,
    width: u16,
    bg: Color,
) -> Vec<Line<'static>> {
    if preview.lines.is_empty() && preview.omitted_count == 0 {
        return Vec::new();
    }

    let mut rendered = Vec::new();
    let label_style = match label {
        "stderr" => Style::default()
            .fg(SURFACE_RED)
            .bg(bg)
            .add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(SURFACE_GREEN)
            .bg(bg)
            .add_modifier(Modifier::BOLD),
    };
    let body_style = match label {
        "stderr" => Style::default().fg(SURFACE_RED).bg(bg),
        _ => Style::default().fg(SURFACE_DARK_GRAY).bg(bg),
    };
    let label_text = format!("{label} ");
    let body_width = width
        .saturating_sub((4 + crate::presentation::display_width(label_text.as_str())) as u16)
        .max(1) as usize;

    for line in &preview.lines {
        let mut wrapped =
            crate::presentation::render_wrapped_display_line(line.as_str(), body_width);
        if wrapped.is_empty() {
            wrapped.push(String::new());
        }
        for (index, wrapped_line) in wrapped.into_iter().enumerate() {
            let label_span = if index == 0 {
                Span::styled(label_text.clone(), label_style)
            } else {
                Span::styled(
                    " ".repeat(crate::presentation::display_width(label_text.as_str())),
                    label_style,
                )
            };
            rendered.push(pad_preserving_backgrounds(
                Line::from(vec![
                    Span::styled("  ", Style::default().bg(bg)),
                    label_span,
                    Span::styled(wrapped_line, body_style),
                ]),
                width,
                bg,
            ));
        }
    }

    if preview.omitted_count > 0 {
        let overflow_text = if preview.truncated_from_start {
            format!("… +{} earlier lines", preview.omitted_count)
        } else {
            format!("… +{} more lines", preview.omitted_count)
        };
        let overflow_line = pad_preserving_backgrounds(
            Line::from(vec![
                Span::styled("  ", Style::default().bg(bg)),
                Span::styled(label_text, label_style),
                Span::styled(overflow_text, Style::default().fg(SURFACE_GRAY).bg(bg)),
            ]),
            width,
            bg,
        );
        if preview.truncated_from_start {
            rendered.insert(0, overflow_line);
        } else {
            rendered.push(overflow_line);
        }
    }

    rendered
}

fn render_read_tool_preview_block(preview: &ReadToolPreview, width: u16) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    let bg = SURFACE_TOOL_BG;
    rendered.push(background_line(width, bg));

    let path = preview.display_path.clone().or_else(|| {
        preview
            .local_path
            .as_deref()
            .map(|path| path.to_string_lossy().into_owned())
    });
    let path = path
        .as_deref()
        .unwrap_or(if preview.is_image { "image" } else { "file" });
    let content_width = width.saturating_sub(7).max(1) as usize;
    let compact_path = truncate_middle_display(path, content_width);
    rendered.push(pad_preserving_backgrounds(
        Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "read ",
                Style::default()
                    .fg(SURFACE_DARK_GRAY)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(compact_path, Style::default().fg(SURFACE_DARK_GRAY).bg(bg)),
        ]),
        width,
        bg,
    ));

    if let Some(summary) = preview.summary.as_deref() {
        for line in render_read_preview_text_line(summary, width) {
            rendered.push(line);
        }
    } else if let Some(mime) = preview.mime.as_deref() {
        for line in render_read_preview_text_line(&format!("Read image file [{mime}]"), width) {
            rendered.push(line);
        }
    } else if !preview.is_image {
        for line in render_read_preview_text_line("Read file", width) {
            rendered.push(line);
        }
    }

    if preview.is_image
        && let Some(path) = preview.local_path.as_deref()
    {
        rendered.extend(render_local_image_preview_lines(
            path,
            preview.mime.as_deref(),
            width,
            bg,
        ));
    } else if !preview.text_excerpt.is_empty() {
        rendered.extend(render_read_text_excerpt_lines(
            preview.text_excerpt.as_slice(),
            width,
            bg,
        ));
    }

    rendered.push(background_line(width, bg));
    rendered
}

fn render_read_text_excerpt_lines(excerpt: &[String], width: u16, bg: Color) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    rendered.push(pad_preserving_backgrounds(
        Line::from(vec![
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled("preview:", Style::default().fg(SURFACE_GRAY).bg(bg)),
        ]),
        width,
        bg,
    ));

    let content_width = width.saturating_sub(5).max(1) as usize;
    for line in excerpt {
        for (index, wrapped) in
            crate::presentation::render_wrapped_display_line(line.as_str(), content_width)
                .into_iter()
                .enumerate()
        {
            let marker = if index == 0 { "│ " } else { "  " };
            rendered.push(pad_preserving_backgrounds(
                Line::from(vec![
                    Span::styled("  ", Style::default().bg(bg)),
                    Span::styled(marker, Style::default().fg(SURFACE_ACCENT).bg(bg)),
                    Span::styled(wrapped, Style::default().fg(SURFACE_DARK_GRAY).bg(bg)),
                ]),
                width,
                bg,
            ));
        }
    }

    rendered
}

fn render_read_preview_text_line(text: &str, width: u16) -> Vec<Line<'static>> {
    let content_width = width.saturating_sub(2).max(1) as usize;
    crate::presentation::render_wrapped_display_line(text, content_width)
        .into_iter()
        .map(|wrapped| {
            pad_preserving_backgrounds(
                Line::from(vec![
                    Span::styled(" ", Style::default().bg(SURFACE_TOOL_BG)),
                    Span::styled(
                        wrapped,
                        Style::default().fg(SURFACE_GRAY).bg(SURFACE_TOOL_BG),
                    ),
                ]),
                width,
                SURFACE_TOOL_BG,
            )
        })
        .collect()
}

fn render_local_image_preview_lines(
    path: &Path,
    mime: Option<&str>,
    width: u16,
    bg: Color,
) -> Vec<Line<'static>> {
    match load_image_preview(path, mime, width, bg) {
        Ok(lines) => lines,
        Err(error) => vec![pad_preserving_backgrounds(
            Line::from(vec![
                Span::styled(" ", Style::default().bg(bg)),
                Span::styled(
                    format!("preview unavailable: {error}"),
                    Style::default().fg(SURFACE_GRAY).bg(bg),
                ),
            ]),
            width,
            bg,
        )],
    }
}

fn load_image_preview(
    path: &Path,
    mime: Option<&str>,
    width: u16,
    bg: Color,
) -> Result<Vec<Line<'static>>, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if metadata.len() > IMAGE_PREVIEW_MAX_BYTES {
        return Err(format!(
            "image is {} (limit {})",
            format_bytes(metadata.len()),
            format_bytes(IMAGE_PREVIEW_MAX_BYTES)
        ));
    }

    let reader = image::ImageReader::open(path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?
        .with_guessed_format()
        .map_err(|error| format!("cannot detect image format: {error}"))?;
    let image = reader
        .decode()
        .map_err(|error| format!("cannot decode image: {error}"))?;
    let rgba = image.to_rgba8();
    let (source_width, source_height) = rgba.dimensions();
    if source_width == 0 || source_height == 0 {
        return Err("empty image".to_owned());
    }

    let available_columns = u32::from(width)
        .saturating_sub(4)
        .clamp(1, IMAGE_PREVIEW_MAX_COLUMNS);
    let max_pixel_height = IMAGE_PREVIEW_MAX_ROWS.saturating_mul(2).max(2);
    let width_scale = available_columns as f32 / source_width as f32;
    let height_scale = max_pixel_height as f32 / source_height as f32;
    let scale = width_scale.min(height_scale).clamp(0.01, 1.0);
    let target_width =
        ((source_width as f32 * scale).round() as u32).clamp(1, available_columns.max(1));
    let target_height = ((source_height as f32 * scale).round() as u32).clamp(1, max_pixel_height);
    let resized = image::imageops::resize(
        &rgba,
        target_width,
        target_height,
        image::imageops::FilterType::Triangle,
    );

    let mut rendered = Vec::new();
    let mime = mime
        .map(ToOwned::to_owned)
        .or_else(|| image_mime_from_path(path).map(ToOwned::to_owned))
        .unwrap_or_else(|| "image".to_owned());
    let header = format!(
        "preview: {}×{} · {} · {}",
        source_width,
        source_height,
        mime,
        format_bytes(metadata.len())
    );
    rendered.push(pad_preserving_backgrounds(
        Line::from(vec![
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled(header, Style::default().fg(SURFACE_GRAY).bg(bg)),
        ]),
        width,
        bg,
    ));

    let terminal_rows = target_height.div_ceil(2);
    for row in 0..terminal_rows {
        let upper_y = row * 2;
        let lower_y = upper_y + 1;
        let mut spans = vec![Span::styled("  ", Style::default().bg(bg))];
        for x in 0..target_width {
            let upper = rgba_pixel_as_rgb(resized.get_pixel(x, upper_y).0, bg);
            let lower = if lower_y < target_height {
                rgba_pixel_as_rgb(resized.get_pixel(x, lower_y).0, bg)
            } else {
                color_to_rgb(bg)
            };
            spans.push(Span::styled(
                "▀",
                Style::default()
                    .fg(Color::Rgb(upper.0, upper.1, upper.2))
                    .bg(Color::Rgb(lower.0, lower.1, lower.2)),
            ));
        }
        rendered.push(pad_preserving_backgrounds(Line::from(spans), width, bg));
    }

    Ok(rendered)
}

fn image_mime_from_path(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

fn rgba_pixel_as_rgb(pixel: [u8; 4], bg: Color) -> (u8, u8, u8) {
    let (bg_r, bg_g, bg_b) = color_to_rgb(bg);
    let alpha = u16::from(pixel[3]);
    let blend = |foreground: u8, background: u8| -> u8 {
        let foreground = u16::from(foreground);
        let background = u16::from(background);
        ((foreground * alpha + background * (255 - alpha)) / 255) as u8
    };
    (
        blend(pixel[0], bg_r),
        blend(pixel[1], bg_g),
        blend(pixel[2], bg_b),
    )
}

fn color_to_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::Red => (255, 0, 0),
        Color::Green => (0, 255, 0),
        Color::Yellow => (255, 255, 0),
        Color::Blue => (0, 0, 255),
        Color::Magenta => (255, 0, 255),
        Color::Cyan => (0, 255, 255),
        Color::Gray => (128, 128, 128),
        Color::DarkGray => (64, 64, 64),
        Color::LightRed => (255, 128, 128),
        Color::LightGreen => (128, 255, 128),
        Color::LightYellow => (255, 255, 128),
        Color::LightBlue => (128, 128, 255),
        Color::LightMagenta => (255, 128, 255),
        Color::LightCyan => (128, 255, 255),
        Color::White => (255, 255, 255),
        Color::Indexed(_) | Color::Reset => (0, 0, 0),
    }
}

fn pad_preserving_backgrounds(mut line: Line<'static>, width: u16, bg: Color) -> Line<'static> {
    let line_len: usize = line.spans.iter().map(|span| span.width()).sum();
    let pad_len = (width as usize).saturating_sub(line_len);
    if pad_len > 0 {
        line.spans
            .push(Span::styled(" ".repeat(pad_len), Style::default().bg(bg)));
    }
    line
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / KB)
    } else {
        format!("{:.1} MB", bytes as f64 / MB)
    }
}

fn truncate_middle_display(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if crate::presentation::display_width(text) <= width {
        return text.to_owned();
    }
    if width == 1 {
        return "…".to_owned();
    }

    let prefix_target = width.saturating_sub(1) / 2;
    let suffix_target = width.saturating_sub(1).saturating_sub(prefix_target);
    let mut prefix = String::new();
    let mut prefix_width = 0usize;
    for ch in text.chars() {
        let ch_width = crate::presentation::char_display_width(ch);
        if prefix_width + ch_width > prefix_target {
            break;
        }
        prefix.push(ch);
        prefix_width += ch_width;
    }

    let mut suffix_chars = Vec::new();
    let mut suffix_width = 0usize;
    for ch in text.chars().rev() {
        let ch_width = crate::presentation::char_display_width(ch);
        if suffix_width + ch_width > suffix_target {
            break;
        }
        suffix_chars.push(ch);
        suffix_width += ch_width;
    }
    suffix_chars.reverse();
    let suffix = suffix_chars.into_iter().collect::<String>();
    format!("{prefix}…{suffix}")
}

fn dedupe_tool_activity_detail_lines(lines: &[String]) -> Vec<String> {
    let mut deduped = Vec::with_capacity(lines.len());
    let mut seen_structured_previews = std::collections::BTreeSet::new();
    let mut last_dedupe_key: Option<String> = None;
    for line in lines {
        let dedupe_key = tool_activity_dedupe_key(line);
        if last_dedupe_key.as_deref() == Some(dedupe_key.as_str()) {
            continue;
        }

        if tool_activity_line_starts_new_group(line) {
            seen_structured_previews.clear();
        }

        if let Some(preview) =
            compact_tool_request_preview(line).or_else(|| compact_tool_args_preview(line))
            && !seen_structured_previews.insert(preview)
        {
            continue;
        }

        deduped.push(line.clone());
        last_dedupe_key = Some(dedupe_key);
    }

    deduped
}

fn tool_activity_line_starts_new_group(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('[')
        || trimmed.starts_with("• Called ")
        || trimmed.starts_with("• Closed ")
        || trimmed.starts_with("Called ")
        || trimmed.starts_with("Closed ")
        || trimmed.starts_with("Approval ")
        || trimmed.starts_with("Denied ")
}

fn tool_activity_dedupe_key(line: &str) -> String {
    if let Some(preview) = compact_tool_request_preview(line) {
        return format!("request:{preview}");
    }
    if let Some(preview) = compact_tool_args_preview(line) {
        return format!("args:{preview}");
    }
    if let Some(preview) = compact_tool_child_preview(line, "stdout") {
        return format!("stdout:{preview}");
    }
    if let Some(preview) = compact_tool_child_preview(line, "stderr") {
        return format!("stderr:{preview}");
    }
    if let Some(preview) = compact_tool_child_preview(line, "file") {
        return format!("file:{preview}");
    }
    if let Some(preview) = compact_tool_child_preview(line, "metrics") {
        return format!("metrics:{preview}");
    }
    if let Some((label, body)) = normalized_activity_headline(line) {
        return format!("status:{label}:{body}");
    }

    line.trim().to_owned()
}

fn compact_tool_child_preview(line: &str, label: &str) -> Option<String> {
    let trimmed = line.trim();
    let body = trimmed
        .strip_prefix(&format!("{label}:"))
        .or_else(|| trimmed.strip_prefix(&format!("{label} ")))
        .or_else(|| trimmed.strip_prefix(&format!("↳ {label} ")))?
        .trim_start();
    Some(body.to_owned())
}

fn normalized_activity_headline(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim().strip_prefix("• ").unwrap_or(line.trim());

    let (label, rest) = if let Some(status) = trimmed.strip_prefix('[') {
        let (status, rest) = status.split_once("] ")?;
        let label = match status {
            "running" | "pending" => "Called",
            "completed" | "failed" | "interrupted" => "Closed",
            "needs_approval" => "Approval",
            "denied" => "Denied",
            _ => return None,
        };
        (label, rest)
    } else if let Some(rest) = trimmed.strip_prefix("Called ") {
        ("Called", rest)
    } else if let Some(rest) = trimmed.strip_prefix("Closed ") {
        ("Closed", rest)
    } else if let Some(rest) = trimmed.strip_prefix("Approval ") {
        ("Approval", rest)
    } else if let Some(rest) = trimmed.strip_prefix("Denied ") {
        ("Denied", rest)
    } else {
        return None;
    };

    let (body, detail) = normalize_activity_target_and_detail(rest);
    let body = if let Some(detail) = detail {
        format!("{body} · {detail}")
    } else {
        body
    };

    Some((label.to_owned(), body))
}

fn compact_tool_request_preview(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let body = trimmed
        .strip_prefix("request:")
        .or_else(|| trimmed.strip_prefix("request "))?
        .trim_start();
    compact_structured_preview(body, 3)
}

fn compact_tool_args_preview(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let body = trimmed
        .strip_prefix("args:")
        .or_else(|| trimmed.strip_prefix("args "))
        .or_else(|| trimmed.strip_prefix("↳ args "))?
        .trim_start();
    compact_structured_preview(body, 3)
}

fn render_tool_detail_lines(line: &str, width: u16) -> Vec<Line<'static>> {
    let content_width = width.saturating_sub(2).max(1) as usize;
    let trimmed = line.trim_start();

    if let Some(rendered) = render_status_activity_line(line, content_width) {
        return rendered;
    }

    if let Some(rendered) = render_named_activity_line(line, content_width) {
        return rendered;
    }

    if let Some(body) = trimmed.strip_prefix("↳ ") {
        let prefix = "↳ ";
        let body = if let Some(args) = body.strip_prefix("args ") {
            let compacted = compact_structured_preview(args, 3).unwrap_or_else(|| args.to_owned());
            format!("args {compacted}")
        } else if let Some(request) = body.strip_prefix("request ") {
            let compacted =
                compact_structured_preview(request, 3).unwrap_or_else(|| request.to_owned());
            format!("request {compacted}")
        } else {
            body.to_owned()
        };
        let (label, body) = body
            .split_once(' ')
            .map(|(label, body)| (label, body.trim_start()))
            .unwrap_or((body.as_str(), ""));
        let label_text = if body.is_empty() {
            String::new()
        } else {
            format!("{label} ")
        };
        let (label_style, body_style) = tool_child_styles(label);
        let body_width = content_width
            .saturating_sub(
                crate::presentation::display_width(prefix)
                    + crate::presentation::display_width(label_text.as_str()),
            )
            .max(1);
        let mut wrapped = crate::presentation::render_wrapped_display_line(body, body_width);
        if wrapped.is_empty() {
            wrapped.push(String::new());
        }
        return wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                if index == 0 {
                    let mut spans = vec![
                        Span::raw("  "),
                        Span::styled(prefix, Style::default().fg(SURFACE_ACCENT)),
                    ];
                    if !label_text.is_empty() {
                        spans.push(Span::styled(label_text.clone(), label_style));
                    }
                    spans.push(Span::styled(wrapped_line, body_style));
                    Line::from(spans)
                } else {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            " ".repeat(
                                crate::presentation::display_width(prefix)
                                    + crate::presentation::display_width(label_text.as_str()),
                            ),
                            Style::default().fg(SURFACE_ACCENT),
                        ),
                        Span::styled(wrapped_line, body_style),
                    ])
                }
            })
            .collect();
    }

    if let Some(request) = trimmed.strip_prefix("request:") {
        return render_tool_detail_lines(&format!("↳ request {}", request.trim_start()), width);
    }

    if let Some(request) = trimmed.strip_prefix("request ") {
        return render_tool_detail_lines(&format!("↳ request {}", request.trim_start()), width);
    }

    if let Some(args) = trimmed.strip_prefix("args:") {
        return render_tool_detail_lines(&format!("↳ args {}", args.trim_start()), width);
    }

    if let Some(args) = trimmed.strip_prefix("args ") {
        return render_tool_detail_lines(&format!("↳ args {}", args.trim_start()), width);
    }

    if let Some(stdout) = trimmed.strip_prefix("stdout:") {
        return render_tool_detail_lines(&format!("↳ stdout {}", stdout.trim_start()), width);
    }

    if let Some(stderr) = trimmed.strip_prefix("stderr:") {
        return render_tool_detail_lines(&format!("↳ stderr {}", stderr.trim_start()), width);
    }

    if let Some(file) = trimmed.strip_prefix("file:") {
        return render_tool_detail_lines(&format!("↳ file {}", file.trim_start()), width);
    }

    if let Some(metrics) = trimmed.strip_prefix("metrics:") {
        return render_tool_detail_lines(&format!("↳ metrics {}", metrics.trim_start()), width);
    }

    if let Some(rendered) = render_tool_sample_detail_lines(line, content_width) {
        return rendered;
    }

    if let Some((prefix, body)) = line.split_once(':') {
        let prefix = format!("{prefix}: ");
        let (prefix_style, body_style) = match prefix.trim_end() {
            "stdout:" => (
                Style::default()
                    .fg(SURFACE_GREEN)
                    .add_modifier(Modifier::BOLD),
                Style::default().fg(SURFACE_DARK_GRAY),
            ),
            "stderr:" => (
                Style::default()
                    .fg(SURFACE_RED)
                    .add_modifier(Modifier::BOLD),
                Style::default().fg(SURFACE_DARK_GRAY),
            ),
            _ => (
                Style::default().fg(SURFACE_GRAY),
                Style::default().fg(SURFACE_DARK_GRAY),
            ),
        };
        let body_width = content_width
            .saturating_sub(crate::presentation::display_width(&prefix))
            .max(1);
        let wrapped =
            crate::presentation::render_wrapped_display_line(body.trim_start(), body_width);
        let continuation_prefix = " ".repeat(crate::presentation::display_width(&prefix));
        return wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                let display_prefix = if index == 0 {
                    prefix.clone()
                } else {
                    continuation_prefix.clone()
                };
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(display_prefix, prefix_style),
                    Span::styled(wrapped_line, body_style),
                ])
            })
            .collect();
    }

    crate::presentation::render_wrapped_display_line(line, content_width)
        .into_iter()
        .map(|wrapped_line| {
            Line::from(vec![
                Span::raw("  "),
                Span::styled(wrapped_line, Style::default().fg(SURFACE_DARK_GRAY)),
            ])
        })
        .collect()
}

fn tool_child_styles(label: &str) -> (Style, Style) {
    match label {
        "stdout" => (
            Style::default()
                .fg(SURFACE_GREEN)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        "stderr" => (
            Style::default()
                .fg(SURFACE_RED)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_RED).add_modifier(Modifier::DIM),
        ),
        "file" => (
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        "metrics" => (
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        "request" | "args" => (
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
        _ => (
            Style::default().fg(SURFACE_ACCENT),
            Style::default().fg(SURFACE_DARK_GRAY),
        ),
    }
}

fn render_tool_sample_detail_lines(line: &str, content_width: usize) -> Option<Vec<Line<'static>>> {
    if !line.starts_with("    ") {
        return None;
    }

    let sample = line.trim_start();
    if sample.is_empty() {
        return None;
    }

    let sample_style = if sample.starts_with('+') {
        Style::default().fg(SURFACE_GREEN)
    } else if sample.starts_with('-') {
        Style::default().fg(SURFACE_RED)
    } else {
        Style::default().fg(SURFACE_DARK_GRAY)
    };
    let sample_width = content_width.saturating_sub(4).max(1);

    Some(
        crate::presentation::render_wrapped_display_line(sample, sample_width)
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                let guide = if index == 0 { "    " } else { "      " };
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(guide, Style::default().fg(SURFACE_DARK_GRAY)),
                    Span::styled(wrapped_line, sample_style),
                ])
            })
            .collect(),
    )
}

fn render_named_activity_line(line: &str, content_width: usize) -> Option<Vec<Line<'static>>> {
    let trimmed = line.trim().strip_prefix("• ").unwrap_or(line.trim());

    let (headline_label, headline_style, rest) = if let Some(rest) = trimmed.strip_prefix("Called ")
    {
        (
            "Called",
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD),
            rest,
        )
    } else if let Some(rest) = trimmed.strip_prefix("Closed ") {
        (
            "Closed",
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::BOLD),
            rest,
        )
    } else if let Some(rest) = trimmed.strip_prefix("Approval ") {
        (
            "Approval",
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
            rest,
        )
    } else if let Some(rest) = trimmed.strip_prefix("Denied ") {
        (
            "Denied",
            Style::default()
                .fg(SURFACE_RED)
                .add_modifier(Modifier::BOLD),
            rest,
        )
    } else {
        return None;
    };

    let display_body = if rest.contains(" (id=") || rest.contains(" - ") {
        let (headline_body, detail_suffix) = normalize_activity_target_and_detail(rest);
        if let Some(detail_suffix) = detail_suffix {
            format!("{headline_body} · {detail_suffix}")
        } else {
            headline_body
        }
    } else {
        rest.to_owned()
    };

    let body_width = content_width
        .saturating_sub(crate::presentation::display_width(headline_label) + 3)
        .max(1);
    let wrapped = crate::presentation::render_wrapped_display_line(&display_body, body_width);

    Some(
        wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                if index == 0 {
                    Line::from(vec![
                        Span::styled("• ", Style::default().fg(SURFACE_GRAY)),
                        Span::styled(format!("{headline_label} "), headline_style),
                        Span::styled(wrapped_line, Style::default().fg(SURFACE_DARK_GRAY)),
                    ])
                } else {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            " ".repeat(crate::presentation::display_width(headline_label) + 1),
                            headline_style,
                        ),
                        Span::styled(wrapped_line, Style::default().fg(SURFACE_DARK_GRAY)),
                    ])
                }
            })
            .collect(),
    )
}

fn render_status_activity_line(line: &str, content_width: usize) -> Option<Vec<Line<'static>>> {
    let trimmed = line.trim();
    let status = trimmed.strip_prefix('[')?;
    let (status, rest) = status.split_once("] ")?;

    let (headline_label, headline_style) = match status {
        "running" | "pending" => (
            "Called",
            Style::default()
                .fg(SURFACE_CYAN)
                .add_modifier(Modifier::BOLD),
        ),
        "completed" | "failed" | "interrupted" => (
            "Closed",
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::BOLD),
        ),
        "needs_approval" => (
            "Approval",
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        "denied" => (
            "Denied",
            Style::default()
                .fg(SURFACE_RED)
                .add_modifier(Modifier::BOLD),
        ),
        _ => return None,
    };

    let (headline_body, detail_suffix) = normalize_activity_target_and_detail(rest);
    let mut display_body = headline_body;
    if let Some(detail_suffix) = detail_suffix {
        display_body.push_str(" · ");
        display_body.push_str(detail_suffix.as_str());
    }

    let body_width = content_width
        .saturating_sub(crate::presentation::display_width(headline_label) + 3)
        .max(1);
    let wrapped = crate::presentation::render_wrapped_display_line(&display_body, body_width);

    Some(
        wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                if index == 0 {
                    Line::from(vec![
                        Span::styled("• ", Style::default().fg(SURFACE_GRAY)),
                        Span::styled(format!("{headline_label} "), headline_style),
                        Span::styled(wrapped_line, Style::default().fg(SURFACE_DARK_GRAY)),
                    ])
                } else {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            " ".repeat(crate::presentation::display_width(headline_label) + 1),
                            headline_style,
                        ),
                        Span::styled(wrapped_line, Style::default().fg(SURFACE_DARK_GRAY)),
                    ])
                }
            })
            .collect(),
    )
}

fn normalize_activity_target_and_detail(rest: &str) -> (String, Option<String>) {
    let (target_with_id, detail_suffix) = rest
        .split_once(" - ")
        .map(|(target, detail)| (target.trim(), Some(detail.trim().to_owned())))
        .unwrap_or((rest.trim(), None));

    let target = if let Some(id_index) = target_with_id.find(" (id=") {
        target_with_id[..id_index].trim().to_owned()
    } else {
        target_with_id.to_owned()
    };

    (target, detail_suffix.filter(|detail| !detail.is_empty()))
}

fn render_error_block_lines(
    title: &str,
    summary: &str,
    details: &[String],
    width: u16,
) -> Vec<Line<'static>> {
    let title_label = format!("[{title}]");
    let summary = summary.trim();
    let detail_segments = details
        .iter()
        .map(|detail| detail.trim())
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    let mut rendered = Vec::new();

    rendered.push(Line::from(""));

    let inline_width = crate::presentation::display_width(&title_label)
        + if summary.is_empty() {
            0
        } else {
            1 + crate::presentation::display_width(summary)
        };
    if inline_width <= width as usize {
        let mut spans = vec![Span::styled(
            title_label,
            Style::default()
                .fg(SURFACE_RED)
                .add_modifier(Modifier::BOLD),
        )];
        if !summary.is_empty() {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                summary.to_owned(),
                Style::default().fg(SURFACE_RED).add_modifier(Modifier::DIM),
            ));
        }
        rendered.push(Line::from(spans));
    } else {
        rendered.push(Line::from(vec![Span::styled(
            title_label,
            Style::default()
                .fg(SURFACE_RED)
                .add_modifier(Modifier::BOLD),
        )]));

        if !summary.is_empty() {
            for wrapped in crate::presentation::render_wrapped_display_line(summary, width as usize)
            {
                rendered.push(Line::from(vec![Span::styled(
                    wrapped,
                    Style::default().fg(SURFACE_RED).add_modifier(Modifier::DIM),
                )]));
            }
        }
    }

    if !detail_segments.is_empty() {
        let detail_width = width.saturating_sub(4).max(1) as usize;
        let displayed_detail_count = detail_segments.len().min(PROVIDER_ERROR_MAX_DETAIL_ITEMS);
        for detail in detail_segments.iter().take(PROVIDER_ERROR_MAX_DETAIL_ITEMS) {
            let wrapped_lines =
                crate::presentation::render_wrapped_display_line(detail, detail_width);
            let wrapped_count = wrapped_lines.len();
            for (line_index, wrapped) in wrapped_lines
                .into_iter()
                .take(PROVIDER_ERROR_MAX_WRAPPED_LINES_PER_DETAIL)
                .enumerate()
            {
                let prefix = if line_index == 0 { "  ↳ " } else { "    " };
                rendered.push(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(
                        wrapped,
                        Style::default().fg(SURFACE_RED).add_modifier(Modifier::DIM),
                    ),
                ]));
            }
            if wrapped_count > PROVIDER_ERROR_MAX_WRAPPED_LINES_PER_DETAIL {
                rendered.push(Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        "…",
                        Style::default().fg(SURFACE_RED).add_modifier(Modifier::DIM),
                    ),
                ]));
            }
        }
        if detail_segments.len() > displayed_detail_count {
            rendered.push(Line::from(vec![
                Span::raw("  ↳ "),
                Span::styled(
                    format!(
                        "… +{} more details",
                        detail_segments.len() - displayed_detail_count
                    ),
                    Style::default().fg(SURFACE_RED).add_modifier(Modifier::DIM),
                ),
            ]));
        }
    }

    rendered.push(Line::from(""));
    rendered
}

fn render_compaction_block_lines(
    turn_count: usize,
    summary: &str,
    expanded: bool,
    width: u16,
) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    rendered.push(background_line(width, SURFACE_COMPACTION_BG));
    rendered.push(styled_background_line(
        vec![
            Span::raw(" "),
            Span::styled(
                "[compaction]",
                Style::default()
                    .fg(LOONG_COMPACTION_TAG)
                    .add_modifier(Modifier::BOLD),
            ),
        ],
        width,
        SURFACE_COMPACTION_BG,
    ));
    if expanded {
        rendered.push(styled_background_line(
            vec![
                Span::raw("  "),
                Span::styled(
                    format!("Compacted from {turn_count} earlier turns"),
                    Style::default().fg(SURFACE_GRAY),
                ),
            ],
            width,
            SURFACE_COMPACTION_BG,
        ));
        for line in summary.lines() {
            rendered.push(styled_background_line(
                vec![
                    Span::raw("  "),
                    Span::styled(line.to_owned(), Style::default().fg(SURFACE_GRAY)),
                ],
                width,
                SURFACE_COMPACTION_BG,
            ));
        }
    } else {
        rendered.push(styled_background_line(
            vec![
                Span::raw("  "),
                Span::styled(
                    format!("Compacted from {turn_count} earlier turns (Ctrl+O to expand)"),
                    Style::default().fg(SURFACE_GRAY),
                ),
            ],
            width,
            SURFACE_COMPACTION_BG,
        ));
    }
    rendered.push(background_line(width, SURFACE_COMPACTION_BG));
    rendered
}

fn render_diff_block_lines(title: Option<&str>, diff: &str, width: u16) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    rendered.push(background_line(width, SURFACE_TOOL_BG));
    rendered.push(styled_background_line(
        vec![
            Span::raw(" "),
            Span::styled(
                format!("[{}]", title.unwrap_or("diff")),
                Style::default()
                    .fg(SURFACE_CYAN)
                    .add_modifier(Modifier::BOLD),
            ),
        ],
        width,
        SURFACE_TOOL_BG,
    ));
    for line in render_diff_to_lines(diff) {
        let mut line = line;
        pad_and_bg(&mut line, width, SURFACE_TOOL_BG);
        rendered.push(line);
    }
    rendered.push(background_line(width, SURFACE_TOOL_BG));
    rendered
}

fn render_image_block_lines(alt: &str, url: &str, width: u16) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();
    let alt_text = if alt.trim().is_empty() {
        "image".to_owned()
    } else {
        alt.trim().to_owned()
    };
    let source = url.trim();
    let content_width = width.saturating_sub(10).max(1) as usize;

    for (index, wrapped) in
        crate::presentation::render_wrapped_display_line(alt_text.as_str(), content_width)
            .into_iter()
            .enumerate()
    {
        let mut spans = Vec::new();
        if index == 0 {
            spans.push(Span::styled(
                "[image] ",
                Style::default()
                    .fg(SURFACE_CYAN)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::raw("        "));
        }
        spans.push(Span::styled(wrapped, Style::default().fg(SURFACE_ACCENT)));
        rendered.push(Line::from(spans));
    }

    if !source.is_empty() {
        let source_width = width.saturating_sub(10).max(1) as usize;
        let source_lines = crate::presentation::render_wrapped_display_line(source, source_width);
        for (index, wrapped) in source_lines.iter().take(2).enumerate() {
            let label = if index == 0 { "source: " } else { "        " };
            rendered.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(label, Style::default().fg(SURFACE_GRAY)),
                Span::styled(wrapped.clone(), Style::default().fg(SURFACE_DIM_GRAY)),
            ]));
        }
        if source_lines.len() > 2 {
            rendered.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("        …", Style::default().fg(SURFACE_DIM_GRAY)),
            ]));
        }

        if let Some(path) = resolve_local_renderable_image_path(source) {
            rendered.extend(render_local_image_preview_lines(
                path.as_path(),
                image_mime_from_path(path.as_path()),
                width,
                Color::Reset,
            ));
        }
    }

    let action_text = if source.is_empty() {
        "media card"
    } else {
        "open source · copy url"
    };
    let action_width = width.saturating_sub(11).max(1) as usize;
    for (index, wrapped) in
        crate::presentation::render_wrapped_display_line(action_text, action_width)
            .into_iter()
            .enumerate()
    {
        let label = if index == 0 { "actions: " } else { "         " };
        rendered.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(label, Style::default().fg(SURFACE_GRAY)),
            Span::styled(wrapped, Style::default().fg(SURFACE_GRAY)),
        ]));
    }
    for line in &mut rendered {
        let current_width: usize = line.spans.iter().map(|span| span.width()).sum();
        if current_width < width as usize {
            line.spans
                .push(Span::raw(" ".repeat(width as usize - current_width)));
        }
    }
    rendered
}

fn background_line(width: u16, bg: Color) -> Line<'static> {
    let mut line = Line::from(vec![Span::raw(" ".repeat(width as usize))]);
    for span in &mut line.spans {
        span.style = span.style.bg(bg);
    }
    line
}

fn styled_background_line(spans: Vec<Span<'static>>, width: u16, bg: Color) -> Line<'static> {
    let mut line = Line::from(spans);
    pad_and_bg(&mut line, width, bg);
    line
}

#[cfg(test)]
mod tests {
    use super::{
        MessageContent, MessageList, ReadToolRequest, STARTUP_COMPACT_WORDMARK, STARTUP_EYE_FRAMES,
        STARTUP_TIP_FADE_MS, STARTUP_TIP_HOLD_MS, STARTUP_WORDMARK, ToolStatus,
        adjust_scroll_start_for_message_boundary, build_assistant_contents, dominant_block_bg,
        format_read_request_display, startup_logo_eye_frame_index, startup_logo_eye_style,
        startup_tip_render_state, startup_wordmark_eye_frame,
    };
    use crate::chat::chat_surface::utils::{
        SURFACE_ACCENT, SURFACE_DIM_GRAY, SURFACE_GRAY, SURFACE_GREEN, SURFACE_RED,
        SURFACE_TOOL_BG, SURFACE_USER_MSG_BG,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
    use ratatui::{Terminal, backend::TestBackend, style::Color};
    use std::time::Duration;

    #[test]
    fn assistant_reply_promotes_diff_fences_to_diff_content() {
        let contents = build_assistant_contents("### Diff\n```diff\n- old\n+ new\n```");

        assert!(matches!(
            contents.first(),
            Some(MessageContent::Diff { title, content })
                if title.as_deref() == Some("Diff") && content.contains("- old") && content.contains("+ new")
        ));
    }

    #[test]
    fn assistant_reply_promotes_tool_activity_callout_to_tool_block() {
        let contents = build_assistant_contents(
            "### Tool activity\n> [completed] read_file (id=call-1)\n> stdout: ok",
        );

        assert!(matches!(
            contents.first(),
            Some(MessageContent::ToolCall { title, status, lines })
                if title.eq_ignore_ascii_case("tool activity")
                    && *status == ToolStatus::Success
                    && !lines.is_empty()
        ));
    }

    #[test]
    fn tool_activity_renders_without_background_block() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> [completed] read_file (id=call-1)\n> stdout: ok".to_owned(),
        );

        let rendered = list.get_rendered_lines(40);
        assert!(
            rendered
                .iter()
                .filter(|line| line.spans.iter().any(|span| {
                    span.content.contains("Closed") || span.content.contains("stdout")
                }))
                .all(|line| dominant_block_bg(line).is_none())
        );
    }

    #[test]
    fn tool_activity_wraps_long_called_lines_cleanly() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.state_write({\"mode\":\"workflow\",\"current_phase\":\"verification\",\"iteration\":2})".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(42)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .all(|line| crate::presentation::display_width(line) <= 42)
        );
        assert!(rendered.iter().any(|line| line.contains("Called")));
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("demo_mcp.state_write"))
        );
    }

    #[test]
    fn tool_activity_wraps_arrow_child_lines_cleanly() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> ↳ args {\"path\":\"src/README.md\",\"depth\":2,\"includeHidden\":false}".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(44)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .all(|line| crate::presentation::display_width(line) <= 44)
        );
        assert!(rendered.iter().any(|line| line.contains("↳ args")));
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("path=src/README.md"))
        );
    }

    #[test]
    fn tool_activity_compacts_bracket_status_lines_into_called_closed_flow() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> [completed] read_file (id=call-1) - ok\n> stdout: done"
                .to_owned(),
        );

        let rendered = list
            .get_rendered_lines(48)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .any(|line| line.contains("• Closed read_file · ok"))
        );
        assert!(!rendered.iter().any(|line| line.contains("(id=call-1)")));
    }

    #[test]
    fn tool_activity_compacts_request_json_into_arrow_child_line() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> request: {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(52)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line.contains("↳ request")));
        assert!(rendered.iter().any(|line| line.contains("query=rust")));
        assert!(rendered.iter().any(|line| line.contains("limit=5")));
    }

    #[test]
    fn tool_activity_compacts_plain_args_without_arrow_prefix() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> args {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(52)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line.contains("↳ args")));
        assert!(rendered.iter().any(|line| line.contains("query=rust")));
        assert!(rendered.iter().any(|line| line.contains("limit=5")));
    }

    #[test]
    fn tool_activity_compacts_plain_args_with_colon_prefix() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> args: {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(52)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line.contains("↳ args")));
        assert!(rendered.iter().any(|line| line.contains("query=rust")));
        assert!(rendered.iter().any(|line| line.contains("limit=5")));
    }

    #[test]
    fn tool_activity_compacts_indented_arrow_args_lines() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n>   ↳ args {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}"
                .to_owned(),
        );

        let rendered = list
            .get_rendered_lines(52)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line.contains("↳ args")));
        assert!(rendered.iter().any(|line| line.contains("query=rust")));
        assert!(rendered.iter().any(|line| line.contains("limit=5")));
    }

    #[test]
    fn plain_called_closed_lines_render_with_bullet_status_flow() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> Closed demo_mcp.search · ok".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(52)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .any(|line| line.contains("• Called demo_mcp.search"))
        );
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("• Closed demo_mcp.search · ok"))
        );
    }

    #[test]
    fn plain_approval_and_denied_lines_render_with_bullet_status_flow() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Approval demo_mcp.search\n> Denied demo_mcp.search · blocked"
                .to_owned(),
        );

        let rendered = list
            .get_rendered_lines(56)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .any(|line| line.contains("• Approval demo_mcp.search"))
        );
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("• Denied demo_mcp.search · blocked"))
        );
    }

    #[test]
    fn bracket_approval_and_denied_lines_normalize_into_status_flow() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> [needs_approval] read_file (id=call-1) - operator confirmation required\n> [denied] read_file (id=call-1) - blocked".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(64)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .any(|line| line.contains("• Approval read_file · operator confirmation required"))
        );
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("• Denied read_file · blocked"))
        );
    }

    #[test]
    fn tool_activity_dedupes_consecutive_duplicate_approval_and_denied_lines() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Approval demo_mcp.search\n> Approval demo_mcp.search\n> Denied demo_mcp.search · blocked\n> Denied demo_mcp.search · blocked".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(64)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let approval_count = rendered
            .iter()
            .filter(|line| line.contains("• Approval demo_mcp.search"))
            .count();
        let denied_count = rendered
            .iter()
            .filter(|line| line.contains("• Denied demo_mcp.search · blocked"))
            .count();

        assert_eq!(approval_count, 1);
        assert_eq!(denied_count, 1);
    }

    #[test]
    fn tool_activity_dedupes_consecutive_bracket_approval_lines_with_different_ids() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> [needs_approval] read_file (id=call-1) - operator confirmation required\n> [needs_approval] read_file (id=call-2) - operator confirmation required".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(72)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let approval_count = rendered
            .iter()
            .filter(|line| line.contains("• Approval read_file · operator confirmation required"))
            .count();

        assert_eq!(approval_count, 1);
    }

    #[test]
    fn tool_activity_dedupes_args_when_request_and_args_match() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> request: {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}\n> args {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(60)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line.contains("↳ request")));
        assert!(
            !rendered
                .iter()
                .any(|line| line.contains("↳ args query=rust"))
        );
    }

    #[test]
    fn tool_activity_resets_request_dedupe_for_new_called_group() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> request: {\"query\":\"rust\",\"limit\":5}\n> Called demo_mcp.search_again\n> request: {\"query\":\"rust\",\"limit\":5}".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(64)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let request_label_count = rendered
            .iter()
            .filter(|line| line.contains("↳ request"))
            .count();
        let query_count = rendered
            .iter()
            .filter(|line| line.contains("query=rust"))
            .count();

        assert_eq!(request_label_count, 2);
        assert!(query_count >= 2);
    }

    #[test]
    fn tool_activity_dedupes_request_when_matching_args_arrive_first() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> args {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}\n> request: {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(60)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line.contains("↳ args")));
        assert!(
            !rendered
                .iter()
                .any(|line| line.contains("↳ request query=rust"))
        );
    }

    #[test]
    fn tool_activity_dedupes_args_with_colon_prefix_when_request_matches() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> request: {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}\n> args: {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(60)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line.contains("↳ request")));
        assert!(
            !rendered
                .iter()
                .any(|line| line.contains("↳ args query=rust"))
        );
    }

    #[test]
    fn tool_activity_dedupes_consecutive_duplicate_status_lines() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> Called demo_mcp.search\n> Closed demo_mcp.search · ok\n> Closed demo_mcp.search · ok".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(60)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let called_count = rendered
            .iter()
            .filter(|line| line.contains("• Called demo_mcp.search"))
            .count();
        let closed_count = rendered
            .iter()
            .filter(|line| line.contains("• Closed demo_mcp.search · ok"))
            .count();

        assert_eq!(called_count, 1);
        assert_eq!(closed_count, 1);
    }

    #[test]
    fn tool_activity_dedupes_consecutive_bracket_status_lines_with_different_ids() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> [completed] read_file (id=call-1) - ok\n> [completed] read_file (id=call-2) - ok".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(60)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let closed_count = rendered
            .iter()
            .filter(|line| line.contains("• Closed read_file · ok"))
            .count();

        assert_eq!(closed_count, 1);
    }

    #[test]
    fn tool_activity_dedupes_bracket_approval_lines_with_different_ids() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> [needs_approval] read_file (id=call-1) - operator confirmation required\n> [needs_approval] read_file (id=call-2) - operator confirmation required".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(72)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let approval_count = rendered
            .iter()
            .filter(|line| line.contains("• Approval read_file · operator confirmation required"))
            .count();

        assert_eq!(approval_count, 1);
    }

    #[test]
    fn tool_activity_compacts_file_and_metrics_into_arrow_children() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.edit\n> file: edit src/lib.rs (+2 / -1)\n> metrics: 42ms · exit=0".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(52)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .any(|line| line.contains("↳ file edit src/lib.rs (+2 / -1)"))
        );
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("↳ metrics 42ms · exit=0"))
        );
    }

    #[test]
    fn tool_activity_compacts_stdout_into_arrow_children() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.exec\n> stdout: 2 lines · 22 bytes".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(52)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .any(|line| line.contains("↳ stdout 2 lines · 22 bytes"))
        );
    }

    #[test]
    fn tool_activity_dedupes_consecutive_duplicate_stdout_lines() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.exec\n> stdout: 2 lines · 22 bytes\n> stdout: 2 lines · 22 bytes".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(60)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let stdout_count = rendered
            .iter()
            .filter(|line| line.contains("↳ stdout 2 lines · 22 bytes"))
            .count();

        assert_eq!(stdout_count, 1);
    }

    #[test]
    fn tool_activity_dedupes_consecutive_duplicate_stderr_lines() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.exec\n> stderr: 1 lines · 12 bytes\n> stderr: 1 lines · 12 bytes".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(60)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let stderr_count = rendered
            .iter()
            .filter(|line| line.contains("↳ stderr 1 lines · 12 bytes"))
            .count();

        assert_eq!(stderr_count, 1);
    }

    #[test]
    fn tool_activity_compacts_stderr_into_arrow_children() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.exec\n> stderr: 1 lines · 12 bytes".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(52)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .any(|line| line.contains("↳ stderr 1 lines · 12 bytes"))
        );
    }

    #[test]
    fn tool_activity_dedupes_consecutive_duplicate_metrics_lines() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.exec\n> metrics: 42ms · exit=0\n> metrics: 42ms · exit=0".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(60)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let metrics_count = rendered
            .iter()
            .filter(|line| line.contains("↳ metrics 42ms · exit=0"))
            .count();

        assert_eq!(metrics_count, 1);
    }

    #[test]
    fn tool_activity_dedupes_consecutive_duplicate_file_lines() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.edit\n> file: edit src/lib.rs (+2 / -1)\n> file: edit src/lib.rs (+2 / -1)".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(64)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let file_count = rendered
            .iter()
            .filter(|line| line.contains("↳ file edit src/lib.rs (+2 / -1)"))
            .count();

        assert_eq!(file_count, 1);
    }

    #[test]
    fn run_tool_activity_renders_command_and_bounded_stream_preview_card() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> [completed] bash (id=call-1) - ok\n> args: {\"cmd\":\"cargo test --workspace --all-features\"}\n> stdout: first line\n> stdout: second line\n> stdout: third line\n> stdout: fourth line\n> stdout: fifth line\n> stderr: warning: slow\n> metrics: 842ms · exit=0"
                .to_owned(),
        );

        let rendered = list
            .get_rendered_lines(72)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        let joined = rendered.join("\n");

        assert!(joined.contains("run cargo test --workspace --all-features ok"));
        assert!(joined.contains("tool: bash"));
        assert!(joined.contains("stdout … +1 earlier lines"));
        assert!(joined.contains("stdout second line"));
        assert!(joined.contains("stdout fifth line"));
        assert!(joined.contains("stderr warning: slow"));
        assert!(joined.contains("metrics 842ms · exit=0"));
    }

    #[test]
    fn search_tool_activity_renders_semantic_preview_card() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called grep\n> args: {\"query\":\"稳定|wenjian|robust|stable\",\"path\":\"~/chat\"}\n> stdout: match one\n> stdout: match two"
                .to_owned(),
        );

        let rendered = list
            .get_rendered_lines(80)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        let joined = rendered.join("\n");

        assert!(joined.contains("search \"稳定|wenjian|robust|stable\" in ~/chat"));
        assert!(joined.contains("tool: grep"));
        assert!(joined.contains("stdout match one"));
    }

    #[test]
    fn search_alias_tool_activity_renders_semantic_preview_card() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called search\n> args: {\"query\":\"rust\",\"path\":\"src\"}\n> stdout: match one"
                .to_owned(),
        );

        let rendered = list
            .get_rendered_lines(72)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        let joined = rendered.join("\n");

        assert!(joined.contains("search \"rust\" in src"));
        assert!(joined.contains("tool: search"));
        assert!(joined.contains("stdout match one"));
    }

    #[test]
    fn list_tool_activity_renders_semantic_preview_card() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called list_directory\n> args: {\"path\":\"~/chat/.omx\"}\n> stdout: agents\n> stdout: logs"
                .to_owned(),
        );

        let rendered = list
            .get_rendered_lines(72)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        let joined = rendered.join("\n");

        assert!(joined.contains("list ~/chat/.omx"));
        assert!(joined.contains("tool: list_directory"));
        assert!(joined.contains("stdout agents"));
    }

    #[test]
    fn glob_tool_activity_renders_semantic_preview_card() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called find_files\n> args: {\"glob\":\"src/**/*.rs\",\"path\":\"~/chat\"}\n> stdout: src/main.rs\n> stdout: src/lib.rs"
                .to_owned(),
        );

        let rendered = list
            .get_rendered_lines(80)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        let joined = rendered.join("\n");

        assert!(joined.contains("glob src/**/*.rs in ~/chat"));
        assert!(joined.contains("tool: find_files"));
        assert!(joined.contains("stdout src/main.rs"));
    }

    #[test]
    fn read_tool_preview_recognizes_namespaced_alias_and_nested_request_path() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called filesystem.open_file\n> request: {\"arguments\":{\"path\":\"src/main.rs\",\"offset\":5,\"limit\":2}}\n> stdout: fn main() {}"
                .to_owned(),
        );

        let rendered = list
            .get_rendered_lines(64)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        let joined = rendered.join("\n");

        assert!(joined.contains("read src/main.rs:5-6"));
        assert!(joined.contains("preview:"));
        assert!(joined.contains("fn main() {}"));
    }

    #[test]
    fn tool_activity_burst_keeps_unique_request_children_per_called_group() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> request: {\"query\":\"rust\",\"limit\":5}\n> Called demo_mcp.search_again\n> request: {\"query\":\"rust\",\"limit\":5}\n> file: edit src/lib.rs (+2 / -1)\n> metrics: 42ms · exit=0".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(64)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let request_label_count = rendered
            .iter()
            .filter(|line| line.contains("↳ request"))
            .count();

        assert_eq!(request_label_count, 2);
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("↳ file edit src/lib.rs"))
        );
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("↳ metrics 42ms · exit=0"))
        );
    }

    #[test]
    fn provider_error_promotes_to_structured_error_block() {
        let contents = build_assistant_contents(
            "[provider_error] provider returned status 401 for model `gpt-5.4` on attempt 1/3: {\"code\":\"INVALID_API_KEY\",\"message\":\"Invalid API key\"} | provider_failover={\"reason\":\"auth_rejected\",\"stage\":\"status_failure\",\"model\":\"gpt-5.4\",\"attempt\":1,\"max_attempts\":3,\"status_code\":401}",
        );

        assert!(matches!(
            contents.first(),
            Some(MessageContent::Error { title, summary, details })
                if title == "provider error"
                    && summary == "401 · gpt-5.4 · 1/3"
                    && details.iter().any(|line| {
                        line.contains("INVALID_API_KEY") && line.contains("Invalid API key")
                    })
                    && details.iter().any(|line| line.contains("auth_rejected"))
        ));
    }

    #[test]
    fn provider_error_rendering_wraps_long_jsonish_details() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "[provider_error] provider returned status 401 for model `gpt-5.4` on attempt 1/3: {\"code\":\"INVALID_API_KEY\",\"message\":\"Invalid API key\"} | provider_failover={\"reason\":\"auth_rejected\",\"stage\":\"status_failure\",\"model\":\"gpt-5.4\",\"attempt\":1,\"max_attempts\":3,\"status_code\":401}".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(36)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .all(|line| crate::presentation::display_width(line) <= 36)
        );
        assert!(
            rendered
                .iter()
                .any(|line| line.contains("[provider error]"))
        );
        assert!(rendered.iter().any(|line| line.contains("INVALID_API_KEY")));
        assert!(rendered.iter().any(|line| line.contains("auth_rejected")));
        assert!(
            !rendered
                .iter()
                .any(|line| line.contains("provider_failover."))
        );
    }

    #[test]
    fn provider_error_renders_title_and_summary_inline_when_width_allows() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "[provider_error] status 401 · gpt-5.4 · attempt 1/3: {\"code\":\"INVALID_API_KEY\",\"message\":\"Invalid API key\"}".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(80)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| {
            line.contains("[provider error]") && line.contains("gpt-5.4") && line.contains("1/3")
        }));
    }

    #[test]
    fn provider_error_renders_without_background_block() {
        let mut list = MessageList::new();
        list.add_user_message("hi".to_owned());
        list.add_assistant_message(
            "[provider_error] provider returned status 401 for model `gpt-5.4` on attempt 1/3: {\"code\":\"INVALID_API_KEY\",\"message\":\"Invalid API key\"}".to_owned(),
        );

        let rendered = list.get_rendered_lines(40);
        assert!(
            rendered
                .iter()
                .filter(|line| line
                    .spans
                    .iter()
                    .any(|span| span.content.contains("provider error")))
                .all(|line| dominant_block_bg(line).is_none())
        );
    }

    #[test]
    fn provider_error_rendering_bounds_detail_noise() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "[provider_error] status 500 · model · attempt 1/3: {\"code\":\"SERVER\",\"message\":\"temporary failure\",\"request_id\":\"abc\",\"debug\":\"very long diagnostic payload that should not flood the transcript\"} | route=primary | retry_after=none | trace=hidden".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(44)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        let detail_rows = rendered.iter().filter(|line| line.contains("↳")).count();

        assert!(detail_rows <= super::PROVIDER_ERROR_MAX_DETAIL_ITEMS + 1);
        assert!(rendered.iter().any(|line| line.contains("more details")));
        assert!(
            rendered
                .iter()
                .all(|line| crate::presentation::display_width(line) <= 44)
        );
    }

    #[test]
    fn inserts_blank_spacer_between_adjacent_colored_blocks() {
        let mut list = MessageList::new();
        list.add_user_message("hi".to_owned());
        list.add_assistant_message("### Tool activity\n> [completed] read_file".to_owned());

        let rendered = list.get_rendered_lines(40);
        let last_user_block_row = rendered
            .iter()
            .rposition(|line| dominant_block_bg(line) == Some(SURFACE_USER_MSG_BG))
            .expect("user block row");
        let first_tool_block_row = rendered
            .iter()
            .enumerate()
            .find_map(|(idx, line)| {
                line.spans
                    .iter()
                    .any(|span| span.content.contains("read_file"))
                    .then_some(idx)
            })
            .expect("tool block row");

        assert!(first_tool_block_row > last_user_block_row);
        assert!(
            rendered[last_user_block_row + 1..first_tool_block_row]
                .iter()
                .any(|line| line
                    .spans
                    .iter()
                    .all(|span| span.style.bg.is_none() && span.content.trim().is_empty()))
        );
    }

    #[test]
    fn compacted_summary_promotes_to_compaction_block() {
        let text = "[session_local_recall_compacted_window]\nThis compacted checkpoint is session-local recall only.\nCompacted 4 earlier turns\nUser context:\n- earlier ask";
        let contents = build_assistant_contents(text);

        assert!(matches!(
            contents.first(),
            Some(MessageContent::Compaction {
                turn_count,
                summary,
                expanded
            })
                if *turn_count == 4 && summary.contains("User context") && !expanded
        ));
    }

    #[test]
    fn toggle_latest_compaction_flips_expanded_state() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "[session_local_recall_compacted_window]\nThis compacted checkpoint is session-local recall only.\nCompacted 2 earlier turns\nUser context:\n- ask"
                .to_owned(),
        );

        assert!(list.toggle_latest_compaction());

        let Some(message) = list.messages.last() else {
            panic!("expected assistant message");
        };
        assert!(matches!(
            message.contents.first(),
            Some(MessageContent::Compaction { expanded, .. }) if *expanded
        ));
    }

    #[test]
    fn assistant_reply_promotes_markdown_images_to_image_block() {
        let contents = build_assistant_contents(
            "Here is the diagram\n\n![plan](https://example.com/plan.png)",
        );

        assert!(matches!(
            contents.get(1),
            Some(MessageContent::Image { alt, url })
                if alt == "plan" && url == "https://example.com/plan.png"
        ));
    }

    #[test]
    fn plain_assistant_reply_preserves_raw_text_without_section_rewrite() {
        let text = "可以。但我需要先看你当前项目里“配置在哪里”。\n\n我可以直接帮你改成 Responses API endpoint，常见位置包括：\n• .env\n• config.*\n• openai / client 初始化代码";
        let contents = build_assistant_contents(text);

        assert!(matches!(
            contents.as_slice(),
            [MessageContent::Markdown(markdown)]
                if markdown == text
        ));
    }

    #[test]
    fn image_block_renders_bounded_source_and_media_actions() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "![long diagram](https://example.com/a/very/long/path/that/needs/wrapping/diagram.png)"
                .to_owned(),
        );

        let rendered = list
            .get_rendered_lines(36)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line.contains("[image]")));
        assert!(rendered.iter().any(|line| line.contains("source:")));
        assert!(rendered.iter().any(|line| line.contains("actions:")));
        assert!(rendered.iter().any(|line| line.contains("copy url")));
        assert!(
            !rendered
                .iter()
                .any(|line| line.contains("not available") || line.contains("unavailable"))
        );
        assert!(
            rendered
                .iter()
                .all(|line| crate::presentation::display_width(line) <= 36)
        );
    }

    #[test]
    fn read_tool_image_activity_renders_preview_card() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let path = tempdir.path().join("sample.png");
        let image = image::RgbaImage::from_fn(2, 2, |x, y| {
            if (x + y) % 2 == 0 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        });
        image.save(path.as_path()).expect("write png");

        let mut list = MessageList::new();
        list.add_assistant_message(format!(
            "### Tool activity\n> Called read\n> args: {{\"path\":\"{}\"}}\n> stdout: Read image file [image/png]",
            path.display()
        ));

        let rendered = list.get_rendered_lines(72);
        let text = rendered
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(text.iter().any(|line| line.contains("read ")));
        assert!(text.join("").contains("sample.png"));
        assert!(
            text.iter()
                .any(|line| line.contains("Read image file [image/png]"))
        );
        assert!(
            text.iter()
                .any(|line| line.contains("preview: 2×2 · image/png"))
        );
        assert!(text.iter().any(|line| line.contains('▀')));
        assert!(
            rendered
                .iter()
                .any(|line| dominant_block_bg(line) == Some(SURFACE_TOOL_BG))
        );
    }

    #[test]
    fn read_tool_text_activity_renders_path_and_excerpt_card() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Tool activity\n> Called read_file\n> args: {\"path\":\"docs/notes.md\",\"offset\":10,\"limit\":3}\n> stdout: # Notes\n> stdout: The quick brown fox jumps over the lazy dog."
                .to_owned(),
        );

        let rendered = list.get_rendered_lines(64);
        let text = rendered
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(text.iter().any(|line| line.contains("read ")));
        assert!(text.join("").contains("docs/notes.md:10-12"));
        assert!(text.iter().any(|line| line.contains("Read file")));
        assert!(text.iter().any(|line| line.contains("preview:")));
        assert!(text.iter().any(|line| line.contains("# Notes")));
        assert!(
            text.iter()
                .any(|line| line.contains("quick brown fox jumps"))
        );
        assert!(!text.iter().any(|line| line.contains('▀')));
        assert!(
            rendered
                .iter()
                .any(|line| dominant_block_bg(line) == Some(SURFACE_TOOL_BG))
        );
    }

    #[test]
    fn read_tool_request_display_shortens_home_and_single_line_range() {
        let Some(home) = std::env::var_os("HOME").and_then(|home| home.into_string().ok()) else {
            return;
        };
        if home.is_empty() {
            return;
        }
        let request = ReadToolRequest {
            path: format!("{home}/project/src/lib.rs"),
            offset: Some(42),
            limit: Some(1),
        };

        assert_eq!(
            format_read_request_display(&request),
            "~/project/src/lib.rs:42"
        );
    }

    #[test]
    fn assistant_markdown_table_renders_as_structured_grid() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "| 指标 | 数值 |\n| --- | --- |\n| 覆盖率 | 68% |\n| 平均响应时间 | 220ms |".to_owned(),
        );

        let rendered = list
            .get_rendered_lines(64)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line.contains("┌")));
        assert!(rendered.iter().any(|line| line.contains("指标")));
        assert!(rendered.iter().any(|line| line.contains("覆盖率")));
        assert!(rendered.iter().any(|line| line.contains("220ms")));
        assert!(!rendered.iter().any(|line| line.contains("| --- |")));
    }

    #[test]
    fn assistant_markdown_code_block_preserves_line_breaks_and_green_styling() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "```rust
let alpha = 1;
let beta = alpha + 1;
```"
            .to_owned(),
        );

        let rendered = list.get_rendered_lines(48);
        let flattened = rendered
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        let alpha_index = flattened
            .iter()
            .position(|line| line.contains("let alpha = 1;"))
            .expect("alpha line");
        let beta_index = flattened
            .iter()
            .position(|line| line.contains("let beta = alpha + 1;"))
            .expect("beta line");

        assert_ne!(alpha_index, beta_index);
        assert!(!flattened[alpha_index].contains("let beta = alpha + 1;"));
        assert!(!flattened[beta_index].contains("let alpha = 1;"));

        let alpha_span = rendered[alpha_index]
            .spans
            .iter()
            .find(|span| span.content.contains("let alpha = 1;"))
            .expect("alpha span");
        let beta_span = rendered[beta_index]
            .spans
            .iter()
            .find(|span| span.content.contains("let beta = alpha + 1;"))
            .expect("beta span");

        assert_eq!(alpha_span.style.fg, Some(SURFACE_GREEN));
        assert_eq!(beta_span.style.fg, Some(SURFACE_GREEN));
    }

    #[test]
    fn assistant_reply_keeps_diff_code_and_tables_consistent_in_one_message() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Patch
```diff
-old value
+new value
```

### Commands
```bash
npm install
npm test
```

| Metric | Value |
| --- | --- |
| coverage | 68% |
| p95 | 220ms |"
                .to_owned(),
        );

        let rendered = list
            .get_rendered_lines(52)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(
                "
",
            );

        assert!(rendered.contains("[Patch]"));
        assert!(rendered.contains("old") && rendered.contains("value"));
        assert!(rendered.contains("new") && rendered.contains("value"));
        assert!(!rendered.contains("```diff"));
        assert!(rendered.contains("```bash"));
        assert!(rendered.contains("npm install"));
        assert!(rendered.contains("npm test"));
        assert!(rendered.contains("┌"));
        assert!(rendered.contains("coverage"));
        assert!(rendered.contains("220ms"));
        assert!(!rendered.contains("| --- |"));
    }

    #[test]
    fn narrow_surface_keeps_code_and_table_blocks_readable() {
        let mut list = MessageList::new();
        list.add_assistant_message(
            "### Commands
```bash
cargo test -p loong-app --lib
```

| Metric | Value |
| --- | --- |
| coverage | 68% |"
                .to_owned(),
        );

        let rendered = list
            .get_rendered_lines(18)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(
                "
",
            );

        assert!(rendered.contains("```bash"));
        assert!(rendered.contains("cargo test"));
        assert!(rendered.contains("┌"));
        assert!(rendered.contains("68%"));
        assert!(rendered.contains("cove"));
    }

    #[test]
    fn user_message_renders_single_bottom_padding_line() {
        let mut list = MessageList::new();
        list.add_user_message("你好".to_owned());

        let rendered = list
            .get_rendered_lines(20)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let non_empty = rendered
            .iter()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.trim_end().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(non_empty, vec!["  你好"]);
    }

    #[test]
    fn rendered_lines_are_pre_padded_for_stable_cached_redraws() {
        let mut list = MessageList::new();
        list.add_assistant_message("hello".to_owned());

        let rendered = list
            .get_rendered_lines(18)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .all(|line| crate::presentation::display_width(line) == 18)
        );
    }

    #[test]
    fn mouse_scroll_is_symmetric() {
        let mut list = MessageList::new();
        list.set_scroll_offset_for_test(10);
        list.mouse_step = 3;

        list.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(list.scroll_offset_for_test(), 7);

        list.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(list.scroll_offset_for_test(), 10);
    }

    #[test]
    fn key_scroll_uses_same_direction_model() {
        let mut list = MessageList::new();
        list.set_scroll_offset_for_test(5);
        list.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(list.scroll_offset_for_test(), 4);
        list.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(list.scroll_offset_for_test(), 5);
    }

    #[test]
    fn space_scroll_matches_page_keys() {
        let mut list = MessageList::new();
        list.set_scroll_offset_for_test(20);

        list.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(list.scroll_offset_for_test(), 8);

        list.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::SHIFT));
        assert_eq!(list.scroll_offset_for_test(), 20);
    }

    #[test]
    fn page_step_uses_viewport_height_minus_overlap() {
        assert_eq!(super::page_step_for_height(1), 1);
        assert_eq!(super::page_step_for_height(2), 1);
        assert_eq!(super::page_step_for_height(8), 6);
        assert_eq!(super::page_step_for_height(20), 18);
    }

    #[test]
    fn mouse_step_tracks_viewport_height_fraction() {
        assert_eq!(super::mouse_step_for_height(1), 1);
        assert_eq!(super::mouse_step_for_height(8), 2);
        assert_eq!(super::mouse_step_for_height(20), 5);
    }

    #[test]
    fn render_updates_page_scroll_step_from_viewport_height() {
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut list = MessageList::new();
        for idx in 0..8 {
            list.add_assistant_message(format!("line-{idx}"));
        }
        list.set_scroll_offset_for_test(20);

        terminal.draw(|f| list.render(f, f.area())).expect("draw");
        let before = list.scroll_offset_for_test();
        list.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));

        assert_eq!(list.scroll_offset_for_test(), before.saturating_sub(6));
    }

    #[test]
    fn render_updates_mouse_scroll_step_from_viewport_height() {
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut list = MessageList::new();
        for idx in 0..8 {
            list.add_assistant_message(format!("line-{idx}"));
        }
        list.set_scroll_offset_for_test(10);

        terminal.draw(|f| list.render(f, f.area())).expect("draw");
        list.handle_mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });

        assert_eq!(list.scroll_offset_for_test(), 8);
    }

    #[test]
    fn resize_preserves_top_visible_line_when_scrolled_up() {
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut list = MessageList::new();
        for idx in 0..20 {
            list.add_assistant_message(format!("line-{idx}"));
        }
        list.set_scroll_offset_for_test(10);

        terminal.draw(|f| list.render(f, f.area())).expect("draw");
        let before = terminal.backend().buffer().clone();
        let before_area = before.area;
        let before_top_line = (0..before_area.width)
            .map(|x| before[(x, 0)].symbol())
            .collect::<String>();

        terminal.backend_mut().resize(40, 10);
        terminal.draw(|f| list.render(f, f.area())).expect("draw");
        let after = terminal.backend().buffer().clone();
        let after_area = after.area;
        let after_top_line = (0..after_area.width)
            .map(|x| after[(x, 0)].symbol())
            .collect::<String>();

        assert_eq!(before_top_line.trim_end(), after_top_line.trim_end());
    }

    #[test]
    fn new_messages_do_not_teleport_transcript_when_scrolled_up() {
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut list = MessageList::new();
        for idx in 0..20 {
            list.add_assistant_message(format!("line-{idx}"));
        }
        list.set_scroll_offset_for_test(10);

        terminal.draw(|f| list.render(f, f.area())).expect("draw");
        let before = terminal.backend().buffer().clone();
        let before_area = before.area;
        let before_top_line = (0..before_area.width)
            .map(|x| before[(x, 0)].symbol())
            .collect::<String>();

        list.add_assistant_message("new-tail-line".to_owned());
        terminal.draw(|f| list.render(f, f.area())).expect("draw");
        let after = terminal.backend().buffer().clone();
        let after_area = after.area;
        let after_top_line = (0..after_area.width)
            .map(|x| after[(x, 0)].symbol())
            .collect::<String>();

        assert_eq!(before_top_line.trim_end(), after_top_line.trim_end());
    }

    #[test]
    fn toggling_compaction_does_not_teleport_transcript_when_scrolled_up() {
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut list = MessageList::new();
        for idx in 0..10 {
            list.add_assistant_message(format!("line-{idx}"));
        }
        list.add_assistant_message(
            "[session_local_recall_compacted_window]\nThis compacted checkpoint is session-local recall only.\nCompacted 2 earlier turns\nUser context:\n- ask"
                .to_owned(),
        );
        list.set_scroll_offset_for_test(6);

        terminal.draw(|f| list.render(f, f.area())).expect("draw");
        let before = terminal.backend().buffer().clone();
        let before_area = before.area;
        let before_top_line = (0..before_area.width)
            .map(|x| before[(x, 0)].symbol())
            .collect::<String>();

        assert!(list.toggle_latest_compaction());
        terminal.draw(|f| list.render(f, f.area())).expect("draw");
        let after = terminal.backend().buffer().clone();
        let after_area = after.area;
        let after_top_line = (0..after_area.width)
            .map(|x| after[(x, 0)].symbol())
            .collect::<String>();

        assert_eq!(before_top_line.trim_end(), after_top_line.trim_end());
    }

    #[test]
    fn new_messages_keep_bottom_anchor_when_following_tail() {
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut list = MessageList::new();
        for idx in 0..20 {
            list.add_assistant_message(format!("line-{idx}"));
        }

        terminal.draw(|f| list.render(f, f.area())).expect("draw");
        list.add_assistant_message("new-tail-line".to_owned());
        terminal.draw(|f| list.render(f, f.area())).expect("draw");
        let after = terminal.backend().buffer().clone();
        let after_area = after.area;
        let flattened = (0..after_area.height)
            .map(|y| {
                (0..after_area.width)
                    .map(|x| after[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(flattened.contains("new-tail-line"));
        assert_eq!(list.scroll_offset_for_test(), 0);
    }

    #[test]
    fn latest_copy_text_prefers_latest_assistant_content() {
        let mut list = MessageList::new();
        list.add_user_message("question".to_owned());
        list.add_assistant_message("answer with details".to_owned());

        assert_eq!(
            list.latest_copy_text().as_deref(),
            Some("answer with details")
        );
    }

    #[test]
    fn export_markdown_includes_roles_and_structured_blocks() {
        let mut list = MessageList::new();
        list.add_user_message("show diff".to_owned());
        list.add_assistant_message("```diff\n- old\n+ new\n```".to_owned());

        let exported = list.export_markdown();

        assert!(exported.contains("## You"));
        assert!(exported.contains("show diff"));
        assert!(exported.contains("## Assistant"));
        assert!(exported.contains("```diff"));
        assert!(exported.contains("+ new"));
    }

    #[test]
    fn width_resize_preserves_bottom_anchor_for_wrapped_tail_content() {
        let backend = TestBackend::new(48, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut list = MessageList::new();
        for idx in 0..8 {
            list.add_assistant_message(format!(
                "line-{idx} keeps a long wrapped transcript chunk stable while the terminal width shrinks"
            ));
        }

        terminal.draw(|f| list.render(f, f.area())).expect("draw");
        terminal.backend_mut().resize(24, 8);
        terminal.draw(|f| list.render(f, f.area())).expect("draw");
        let after = terminal.backend().buffer().clone();
        let after_area = after.area;
        let flattened = (0..after_area.height)
            .map(|y| {
                (0..after_area.width)
                    .map(|x| after[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(flattened.contains("line-7"));
        assert_eq!(list.scroll_offset_for_test(), 0);
    }

    #[test]
    fn resize_preserves_bottom_anchor_when_following_tail() {
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut list = MessageList::new();
        for idx in 0..20 {
            list.add_assistant_message(format!("line-{idx}"));
        }

        terminal.draw(|f| list.render(f, f.area())).expect("draw");
        terminal.backend_mut().resize(40, 10);
        terminal.draw(|f| list.render(f, f.area())).expect("draw");
        let after = terminal.backend().buffer().clone();
        let after_area = after.area;
        let flattened = (0..after_area.height)
            .map(|y| {
                (0..after_area.width)
                    .map(|x| after[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(flattened.contains("line-19"));
        assert_eq!(list.scroll_offset_for_test(), 0);
    }

    #[test]
    fn assistant_message_keeps_single_trailing_blank_line() {
        let mut list = MessageList::new();
        list.add_assistant_message("Hello.".to_owned());
        list.add_user_message("next".to_owned());

        let rendered = list
            .get_rendered_lines(20)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let hello_index = rendered
            .iter()
            .position(|line| line.contains("Hello."))
            .expect("assistant line");
        let next_index = rendered
            .iter()
            .position(|line| line.contains("next"))
            .expect("next user line");

        assert_eq!(next_index.saturating_sub(hello_index), 3);
    }

    #[test]
    fn transcript_does_not_start_with_a_forced_blank_row() {
        let mut list = MessageList::new();
        list.add_startup_header("0.1.0".to_owned(), "help".to_owned(), Vec::new());

        let rendered = list
            .get_rendered_lines(24)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| {
            line.contains("LOONG")
                || line.contains("░███")
                || line.contains("╭─╮")
                || line.contains("╰─╯")
        }));
        assert!(
            rendered
                .iter()
                .find(|line| !line.trim().is_empty())
                .is_some_and(|line| {
                    line.contains("LOONG")
                        || line.contains("░███")
                        || line.contains("╭─╮")
                        || line.contains("╰─╯")
                })
        );
    }

    #[test]
    fn clear_transcript_removes_messages_and_resets_scroll_state() {
        let mut list = MessageList::new();
        list.add_startup_header("0.1.0".to_owned(), "help".to_owned(), Vec::new());
        list.add_user_message("hello".to_owned());
        list.add_rendered_lines(vec!["system card".to_owned()]);
        let _ = list.get_rendered_lines(80);
        list.set_scroll_offset_for_test(6);
        list.set_last_scroll_start_for_test(2);
        list.set_snap_scroll_on_next_render_for_test(false);

        list.clear_transcript();

        assert!(list.messages.is_empty());
        assert_eq!(list.scroll_offset_for_test(), 0);
        assert_eq!(list.last_scroll_start_for_test(), 0);
        assert!(list.is_following_tail());
        assert!(list.snap_scroll_on_next_render_for_test());
        assert!(list.render_cache.is_none());
    }

    #[test]
    fn startup_header_wraps_long_section_values_to_viewport_width() {
        let mut list = MessageList::new();
        list.add_startup_header(
            "0.1.0".to_owned(),
            "help".to_owned(),
            vec![("Skills".to_owned(), vec!["12".to_owned()])],
        );

        let rendered = list
            .get_rendered_lines(28)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .all(|line| crate::presentation::display_width(line) <= 28)
        );
        assert!(rendered.iter().any(|line| line.contains("Skills (12)")));
    }

    #[test]
    fn startup_status_markers_use_state_colors() {
        let mut list = MessageList::new();
        list.add_startup_header(
            "0.1.0".to_owned(),
            "help".to_owned(),
            vec![
                ("Skills".to_owned(), vec!["0".to_owned()]),
                ("MCP".to_owned(), vec!["2".to_owned()]),
            ],
        );

        let rendered = list.get_rendered_lines(80);
        let has_missing_marker = rendered
            .iter()
            .flat_map(|line| line.spans.iter())
            .any(|span| span.content.as_ref() == "✗" && span.style.fg == Some(SURFACE_RED));
        let has_ready_marker = rendered
            .iter()
            .flat_map(|line| line.spans.iter())
            .any(|span| span.content.as_ref() == "✓" && span.style.fg == Some(SURFACE_GREEN));

        assert!(has_missing_marker);
        assert!(has_ready_marker);
    }

    #[test]
    fn startup_tip_keeps_blank_row_below_tip() {
        let mut list = MessageList::new();
        list.add_startup_header_with_tips(
            "0.1.0".to_owned(),
            "fallback".to_owned(),
            Vec::new(),
            vec!["rotating tip".to_owned()],
        );

        let rendered = list
            .get_rendered_lines(80)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        let tip_index = rendered
            .iter()
            .position(|line| line.contains("rotating tip"))
            .expect("startup tip line");

        assert!(
            rendered
                .get(tip_index + 1)
                .is_some_and(|line| line.trim().is_empty())
        );
    }

    #[test]
    fn startup_wordmarks_match_brand_art() {
        assert_eq!(
            STARTUP_WORDMARK,
            &[
                "░███░         ░████████░    ░████████░   ░█████████░    ░████████░",
                "░███░        ░███    ███░  ░███    ███░  ░███    ███░  ░███    ███░",
                "░███░        ░███    ███░  ░███    ███░  ░███    ███░  ░███",
                "░███░        ░███    ███░  ░███    ███░  ░███    ███░  ░███  █████░",
                "░███░        ░███    ███░  ░███    ███░  ░███    ███░  ░███    ███░",
                "░███░        ░███    ███░  ░███    ███░  ░███    ███░  ░███    ███░",
                "░██████████   ░████████░    ░████████░   ░███    ███░   ░████████░",
            ]
        );
        assert_eq!(
            STARTUP_COMPACT_WORDMARK,
            &[
                "╷  ╭─╮╭─╮╭╮╷╭─╴",
                "│  │ ││ ││╰┤│╶╮",
                "╰─╴╰─╯╰─╯╵ ╵╰─╯",
                "",
                "",
                "",
            ]
        );
    }

    #[test]
    fn startup_wordmark_eye_frames_animate_the_two_o_letters() {
        assert_eq!(startup_logo_eye_frame_index(Duration::ZERO), 0);
        assert_eq!(STARTUP_EYE_FRAMES.len(), 60);

        let first_glance = startup_wordmark_eye_frame(0).join(
            "
",
        );
        let upper_wash = startup_wordmark_eye_frame(6).join(
            "
",
        );
        let far_right = startup_wordmark_eye_frame(16).join(
            "
",
        );
        let lower_glance = startup_wordmark_eye_frame(26).join(
            "
",
        );
        let shimmer = startup_wordmark_eye_frame(32).join(
            "
",
        );
        let vertical_sweep = startup_wordmark_eye_frame(40).join(
            "
",
        );

        assert!(first_glance.contains("░███ █  ███░  ░███ █  ███░"));
        assert!(upper_wash.contains("░███▓▓▓▓███░  ░███▓▓▓▓███░"));
        assert!(far_right.contains("░███  █████░  ░███  █████░"));
        assert!(lower_glance.contains("░███ █  ███░  ░███ █  ███░"));
        assert!(shimmer.contains("░███▒▒▒▒███░  ░███▒▒▒▒███░"));
        assert!(vertical_sweep.contains("░███ ▂  ███░  ░███ ▂  ███░"));
        assert_ne!(
            first_glance,
            STARTUP_WORDMARK.join(
                "
"
            )
        );
        assert_ne!(first_glance, far_right);
    }

    #[test]
    fn startup_wordmark_eye_frames_keep_fixed_geometry() {
        for frame_index in 0..STARTUP_EYE_FRAMES.len() {
            let frame = startup_wordmark_eye_frame(frame_index);
            assert_eq!(frame.len(), STARTUP_WORDMARK.len());
            for (line, base_line) in frame.iter().zip(STARTUP_WORDMARK.iter()) {
                assert_eq!(
                    crate::presentation::display_width(line),
                    crate::presentation::display_width(base_line),
                    "{line}"
                );
            }
        }
    }

    #[test]
    fn startup_eye_shadow_blocks_use_layered_intensity() {
        assert_eq!(startup_logo_eye_style('░').fg, Some(SURFACE_DIM_GRAY));
        assert_eq!(startup_logo_eye_style('▒').fg, Some(SURFACE_GRAY));
        assert_eq!(startup_logo_eye_style('▓').fg, Some(SURFACE_ACCENT));
        assert_eq!(startup_logo_eye_style('█').fg, Some(Color::White));
    }

    #[test]
    fn startup_header_uses_full_logo_when_viewport_is_wide() {
        let mut list = MessageList::new();
        list.add_startup_header("0.1.0".to_owned(), "help".to_owned(), Vec::new());

        let rendered = list
            .get_rendered_lines(120)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("░███░         ░████████░"));
        assert!(rendered.contains("░██████████   ░████████░"));
        assert!(!rendered.contains("╷  ╭─╮╭─╮╭╮╷╭─╴"));
    }

    #[test]
    fn startup_header_uses_compact_logo_when_viewport_is_narrow() {
        let mut list = MessageList::new();
        list.add_startup_header("0.1.0".to_owned(), "help".to_owned(), Vec::new());

        let rendered = list
            .get_rendered_lines(24)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("╷  ╭─╮╭─╮╭╮╷╭─╴"));
        assert!(!rendered.contains("░████████░"));
    }

    #[test]
    fn startup_tip_animation_fades_to_next_tip_after_cycle_boundary() {
        let tips = vec!["first tip".to_owned(), "second tip".to_owned()];
        let elapsed = Duration::from_millis(
            STARTUP_TIP_HOLD_MS + STARTUP_TIP_FADE_MS + (STARTUP_TIP_FADE_MS / 2),
        );

        let render_state =
            startup_tip_render_state(tips.as_slice(), elapsed).expect("startup tip render state");

        if super::reduced_motion_enabled() {
            assert!(render_state.text.contains("first tip"));
            return;
        }

        assert!(render_state.text.contains("second tip"));
        assert_ne!(render_state.text_color, Color::White);
        assert_ne!(render_state.bullet_color, SURFACE_ACCENT);
    }

    #[test]
    fn startup_header_wraps_version_and_tutorial_to_viewport_width() {
        let mut list = MessageList::new();
        list.add_startup_header(
            "v0.1.0-alpha.3".to_owned(),
            "escape interrupt · : deck · / commands · ctrl+o compaction".to_owned(),
            Vec::new(),
        );

        let rendered = list
            .get_rendered_lines(24)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(
            rendered
                .iter()
                .all(|line| crate::presentation::display_width(line) <= 24)
        );
        assert!(rendered.iter().any(|line| line.contains("0.1.0-alpha.3")));
        assert!(rendered.iter().any(|line| line.contains("compaction")));
    }

    #[test]
    fn startup_header_version_line_does_not_duplicate_v_prefix() {
        let mut list = MessageList::new();
        list.add_startup_header("v0.1.0-alpha.3".to_owned(), "help".to_owned(), Vec::new());

        let rendered = list
            .get_rendered_lines(40)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("v0.1.0-alpha.3"));
        assert!(!rendered.contains("vv0.1.0-alpha.3"));
    }

    #[test]
    fn startup_header_current_build_version_line_does_not_duplicate_v_prefix() {
        let version = crate::presentation::BuildVersionInfo::current().render_version_line();
        let mut list = MessageList::new();
        list.add_startup_header(version.clone(), "help".to_owned(), Vec::new());

        let rendered = list
            .get_rendered_lines(80)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains(version.as_str()));
        assert!(!rendered.contains(format!("v{version}").as_str()));
    }

    #[test]
    fn assistant_messages_trim_renderer_blank_edges_and_keep_two_space_indent() {
        let mut list = MessageList::new();
        list.add_assistant_message("Hello.".to_owned());

        let rendered = list
            .get_rendered_lines(20)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        let hello_index = rendered
            .iter()
            .position(|line| line.contains("Hello."))
            .expect("assistant line");
        assert!(rendered[hello_index].starts_with("  Hello."));
        assert!(
            rendered
                .get(hello_index + 1)
                .is_some_and(|line| line.trim().is_empty())
        );
    }

    #[test]
    fn assistant_inline_bullet_runs_split_into_separate_lines() {
        let mut list = MessageList::new();
        list.add_assistant_message("• first item • second item • third item".to_owned());

        let rendered = list
            .get_rendered_lines(36)
            .into_iter()
            .map(|line| {
                line.spans
                    .into_iter()
                    .map(|span| span.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(rendered.iter().any(|line| line.contains("• first item")));
        assert!(rendered.iter().any(|line| line.contains("• second item")));
        assert!(rendered.iter().any(|line| line.contains("• third item")));
    }

    #[test]
    fn rendered_system_activity_headline_uses_colored_spans() {
        let mut list = MessageList::new();
        list.add_rendered_lines(vec!["• Ran cargo test -p loong-app".to_owned()]);

        let rendered = list.get_rendered_lines(48);
        let line = rendered
            .iter()
            .find(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content.contains("cargo test"))
            })
            .expect("system activity line");

        assert_eq!(line.spans[0].content.as_ref(), "• ");
        assert_eq!(line.spans[0].style.fg, Some(SURFACE_GREEN));
        assert_eq!(line.spans[1].content.as_ref(), "Ran ");
        assert_eq!(line.spans[1].style.fg, Some(SURFACE_ACCENT));
    }

    #[test]
    fn rendered_system_activity_child_uses_tree_and_action_styling() {
        let mut list = MessageList::new();
        list.add_rendered_lines(vec!["  └ Read app.rs".to_owned()]);

        let rendered = list.get_rendered_lines(32);
        let line = rendered
            .iter()
            .find(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content.contains("app.rs"))
            })
            .expect("system child line");

        assert_eq!(line.spans[0].content.as_ref(), "  ");
        assert_eq!(line.spans[1].content.as_ref(), "└ ");
        assert_eq!(line.spans[1].style.fg, Some(SURFACE_GRAY));
        assert_eq!(line.spans[2].content.as_ref(), "Read ");
        assert_eq!(line.spans[2].style.fg, Some(SURFACE_ACCENT));
    }

    #[test]
    fn scroll_start_snaps_to_user_block_boundary() {
        let mut list = MessageList::new();
        list.add_startup_header("0.1.0".to_owned(), "help".to_owned(), Vec::new());
        list.add_user_message("hello world".to_owned());
        list.add_assistant_message("reply".to_owned());

        let rendered = list.get_rendered_lines(24);
        let first_user_bg = rendered
            .iter()
            .position(|line| dominant_block_bg(line).is_some())
            .expect("user block should be present");
        let inside_user_block = first_user_bg + 1;

        let snapped = adjust_scroll_start_for_message_boundary(&rendered, inside_user_block);
        assert_eq!(snapped, first_user_bg + 1);
    }
}
