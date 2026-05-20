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

include!("message_list/core.rs");
include!("message_list/startup_render.rs");
include!("message_list/content_projection.rs");
include!("message_list/tool_preview.rs");
include!("message_list/tool_render.rs");
include!("message_list/render_blocks.rs");

#[cfg(test)]
#[path = "message_list/message_list_tests.rs"]
mod tests;
