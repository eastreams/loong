use crate::constants::spinners::*;
use regex::Regex;
use ratatui::style::Color;
use serde_json::Value;
use std::env;
use std::sync::LazyLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static LOCAL_LINK_COLON_LOCATION_SUFFIX_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r":[0-9]+(?::[0-9]+)?$").ok());

pub const FOCUS_RING_FRAMES: [&str; 18] = [
    "·", "·", "◦", "○", "◎", "◉", "●", "●", "●", "◉", "◎", "○", "◦", "·", "·", " ", " ", " ",
];

// LOONG Branding & Identity - Primary Palette
pub const LOONG_AMETHYST_SMOKE: Color = Color::Rgb(199, 131, 194); // #c783c2
pub const LOONG_EMERALD: Color = Color::Rgb(109, 190, 126); // #6dbe7e
pub const LOONG_POWDER_BLUE: Color = Color::Rgb(159, 184, 217); // #9fb8d9
pub const LOONG_COTTON_CANDY: Color = Color::Rgb(248, 146, 158); // #f8929e

// Final Targeted Block Colors (User Requested Refinements)
pub const LOONG_USER_HI_BG: Color = Color::Rgb(133, 180, 209); // #85B4D1 (The "hi" block)
pub const LOONG_TOOL_READ_BG: Color = Color::Rgb(197, 220, 169); // #C5DCA9 (The "read" block)
pub const LOONG_COMPACTION_TAG: Color = Color::Rgb(168, 234, 235); // #A8EAEB (The "compaction" label)

// Surface palette
pub const SURFACE_CYAN: Color = LOONG_MAYA_BLUE_FALLBACK;
pub const SURFACE_GREEN: Color = LOONG_EMERALD;
pub const SURFACE_RED: Color = Color::Rgb(255, 46, 0);
pub const SURFACE_HEADING: Color = LOONG_AMETHYST_SMOKE;
pub const SURFACE_ACCENT: Color = LOONG_POWDER_BLUE;
pub const SURFACE_GRAY: Color = Color::Rgb(128, 128, 128);
pub const SURFACE_DIM_GRAY: Color = Color::Rgb(102, 102, 102);
pub const SURFACE_DARK_GRAY: Color = Color::Rgb(40, 40, 40);

const LOONG_MAYA_BLUE_FALLBACK: Color = Color::Rgb(112, 193, 255);

// Dynamic Backgrounds for blocks
pub const SURFACE_USER_MSG_BG: Color = LOONG_USER_HI_BG;
pub const SURFACE_TOOL_BG: Color = LOONG_TOOL_READ_BG;
pub const SURFACE_COMPACTION_BG: Color = Color::Rgb(40, 40, 50); // Muted base for the tag to sit on
pub const SURFACE_COTTON_CANDY: Color = LOONG_COTTON_CANDY;

pub fn reduced_motion_enabled() -> bool {
    env_truthy("LOONG_TUI_REDUCED_MOTION")
        || env::var("TERM")
            .map(|term| term.eq_ignore_ascii_case("dumb"))
            .unwrap_or(false)
}

fn env_truthy(name: &str) -> bool {
    env::var(name)
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            !matches!(normalized.as_str(), "" | "0" | "false" | "off" | "no")
        })
        .unwrap_or(false)
}

/// Dynamic Focus Ring Animation
pub fn focus_ring_frame(start_time: Instant) -> &'static str {
    if reduced_motion_enabled() {
        return "•";
    }
    let elapsed_ms = start_time.elapsed().as_millis() as u64;
    let current_interval = if elapsed_ms < 5000 {
        80 + (70 * elapsed_ms / 5000)
    } else {
        150
    };
    let frame_index = (elapsed_ms / current_interval) as usize;
    let selected_index = frame_index % FOCUS_RING_FRAMES.len();
    FOCUS_RING_FRAMES
        .get(selected_index)
        .copied()
        .unwrap_or(FOCUS_RING_FRAMES.first().copied().unwrap_or("·"))
}

pub fn spinner_seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    nanos ^ ((std::process::id() as u64) << 32)
}

/// Session-randomized "Working..." verb order while keeping time-based animation.
pub fn get_spinner_verb_with_seed(start_time: Instant, seed: u64) -> &'static str {
    if reduced_motion_enabled() {
        return SPINNERS_ZH_CN.first().copied().unwrap_or("thinking");
    }
    let elapsed_ms = start_time.elapsed().as_millis() as u64;
    let current_interval = if elapsed_ms < 5000 {
        80 + (70 * elapsed_ms / 5000)
    } else {
        150
    };
    let cycle_count = (elapsed_ms / current_interval) / FOCUS_RING_FRAMES.len() as u64;
    let mut h = cycle_count
        .wrapping_add(seed)
        .wrapping_add(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94D049BB133111EB);
    h = h ^ (h >> 31);
    let selected_index = h as usize % SPINNERS_ZH_CN.len();
    SPINNERS_ZH_CN
        .get(selected_index)
        .copied()
        .unwrap_or(SPINNERS_ZH_CN.first().copied().unwrap_or("thinking"))
}

pub fn divider_rule_text(width: usize) -> String {
    "─".repeat(width.max(12))
}

pub fn compact_structured_preview(text: &str, max_fields: usize) -> Option<String> {
    let value = serde_json::from_str::<Value>(text.trim()).ok()?;
    let object = value.as_object()?;
    if object.is_empty() {
        return Some("{}".to_owned());
    }

    let mut parts = object
        .iter()
        .filter_map(|(key, value)| {
            compact_preview_value(value).map(|value| format!("{key}={value}"))
        })
        .take(max_fields)
        .collect::<Vec<_>>();

    if object.len() > max_fields {
        parts.push("…".to_owned());
    }

    if parts.is_empty() {
        Some("…".to_owned())
    } else {
        Some(parts.join(" · "))
    }
}

pub fn split_inline_bullet_runs(line: &str) -> Option<Vec<String>> {
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

pub fn split_reasoning_preview_text(preview: &str) -> (Option<String>, Option<String>) {
    let open_tag = "<think>";
    let close_tag = "</think>";
    let lower = preview.to_ascii_lowercase();
    let mut visible = String::new();
    let mut thinking = String::new();
    let mut idx = 0usize;
    let mut in_think = false;

    while idx < preview.len() {
        let remaining = &lower[idx..];
        if remaining.starts_with(open_tag) {
            in_think = true;
            idx += open_tag.len();
            continue;
        }
        if remaining.starts_with(close_tag) {
            in_think = false;
            idx += close_tag.len();
            continue;
        }
        if open_tag.starts_with(remaining) || close_tag.starts_with(remaining) {
            break;
        }

        let Some(ch) = preview[idx..].chars().next() else {
            break;
        };
        if in_think {
            thinking.push(ch);
        } else {
            visible.push(ch);
        }
        idx += ch.len_utf8();
    }

    let thinking = thinking.trim().to_owned();
    let visible = visible.trim().to_owned();
    let saw_explicit_think_tag =
        preview.to_ascii_lowercase().contains("<think>") || preview.to_ascii_lowercase().contains("</think>");
    if saw_explicit_think_tag {
        return (
            (!thinking.is_empty()).then_some(thinking),
            (!visible.is_empty()).then_some(visible),
        );
    }

    let lines = preview.lines().collect::<Vec<_>>();
    if let Some(first_non_blank_idx) = lines.iter().position(|line| !line.trim().is_empty())
        && lines
            .get(first_non_blank_idx)
            .is_some_and(|line| is_reasoning_heading_line(line.trim()))
    {
        let after_heading = lines.get(first_non_blank_idx + 1..).unwrap_or(&[]);
        let blank_idx = after_heading.iter().position(|line| line.trim().is_empty());
        let (reasoning_slice, visible_slice) = match blank_idx {
            Some(idx) => (
                after_heading.get(..idx).unwrap_or(&[]),
                after_heading.get(idx + 1..).unwrap_or(&[]),
            ),
            None => (after_heading, &[][..]),
        };
        let reasoning = reasoning_slice.join("\n").trim().to_owned();
        let visible = visible_slice
            .iter()
            .filter(|line| !line.trim().is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_owned();
        return (
            (!reasoning.is_empty()).then_some(reasoning),
            (!visible.is_empty()).then_some(visible),
        );
    }

    let Some(blank_idx) = lines.iter().position(|line| line.trim().is_empty()) else {
        let trimmed = preview.trim();
        if trimmed.is_empty() {
            return (None, Some(String::new()));
        }

        let mut truncated = trimmed.to_owned();
        let partial_tag_prefixes = [
            "<",
            "</",
            "<t",
            "</t",
            "<th",
            "</th",
            "<thi",
            "</thi",
            "<thin",
            "</thin",
            "<think",
            "</think",
        ];
        if partial_tag_prefixes
            .iter()
            .any(|prefix| truncated.to_ascii_lowercase().ends_with(prefix))
        {
            while partial_tag_prefixes
                .iter()
                .any(|prefix| truncated.to_ascii_lowercase().ends_with(prefix))
            {
                truncated.pop();
            }
            truncated.truncate(truncated.trim_end().len());
        }

        return (None, (!truncated.is_empty()).then_some(truncated));
    };
    let reasoning = lines
        .get(..blank_idx)
        .unwrap_or(&[])
        .join("\n")
        .trim()
        .to_owned();
    let visible = lines
        .get(blank_idx + 1..)
        .unwrap_or(&[])
        .iter()
        .filter(|line| !line.trim().is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned();

    let looks_like_structured_markdown = |text: &str| {
        text.lines().map(str::trim).any(|line| {
            line.starts_with("```")
                || line.starts_with('#')
                || line.starts_with('>')
                || line.starts_with("- ")
                || line.starts_with("* ")
                || line
                    .strip_prefix(|ch: char| ch.is_ascii_digit())
                    .is_some_and(|rest| rest.starts_with(". "))
                || (line.contains("](") && line.contains('['))
                || (line.starts_with('|') && line.ends_with('|') && line.matches('|').count() >= 2)
        })
    };

    if reasoning.is_empty() || visible.is_empty() || looks_like_structured_markdown(&reasoning) {
        return (
            None,
            (!preview.trim().is_empty()).then(|| preview.trim().to_owned()),
        );
    }

    (Some(reasoning), Some(visible))
}

pub fn strip_reasoning_heading_block(text: &str) -> String {
    let lines = text.lines();
    let mut kept = Vec::new();
    let mut skipping_reasoning = false;

    for line in lines {
        let trimmed = line.trim();
        let reasoning_heading = is_reasoning_heading_line(trimmed);
        let any_heading = trimmed.starts_with('#');
        if !skipping_reasoning
            && reasoning_heading
        {
            skipping_reasoning = true;
            continue;
        }

        if skipping_reasoning {
            if any_heading {
                skipping_reasoning = false;
                kept.push(line.to_owned());
                continue;
            }
            if trimmed.is_empty() {
                skipping_reasoning = false;
                continue;
            }
            continue;
        }

        kept.push(line.to_owned());
    }

    kept.join("\n").trim().to_owned()
}

fn is_reasoning_heading_line(line: &str) -> bool {
    line.strip_prefix('#')
        .and_then(|rest| rest.strip_prefix('#'))
        .map(str::trim)
        .is_some_and(is_reasoning_section_title)
}

pub fn is_reasoning_section_title(title: &str) -> bool {
    matches!(
        title.trim().to_ascii_lowercase().as_str(),
        "reasoning" | "analysis" | "thinking" | "thought process"
    )
}

pub fn visible_text_for_reasoning_mode(text: &str, show_reasoning: bool) -> String {
    if show_reasoning {
        return split_reasoning_preview_text(text)
            .1
            .unwrap_or_else(|| text.to_owned());
    }

    let filtered = strip_reasoning_heading_block(text);
    if filtered.as_str() != text {
        return filtered;
    }

    let (reasoning, visible) = split_reasoning_preview_text(text);
    if reasoning.is_some() {
        return visible.unwrap_or_default();
    }

    text.to_owned()
}

pub fn shorten_home_display_path(path: &str) -> String {
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

pub fn normalize_local_link_path_text(path_text: &str) -> String {
    path_text.replace('\\', "/")
}

pub fn render_local_link_target_text(dest_url: &str) -> Option<String> {
    let raw = if let Some(rest) = dest_url.strip_prefix("file://") {
        rest
    } else {
        dest_url
    };

    if !(raw.starts_with('/')
        || raw.starts_with("~/")
        || raw.starts_with("./")
        || raw.starts_with("../"))
    {
        return None;
    }

    let (path_part, location_suffix) = split_local_link_location_suffix(raw);
    let mut rendered = shorten_home_display_path(&normalize_local_link_path_text(path_part));
    if let Some(location_suffix) = location_suffix {
        rendered.push_str(location_suffix.as_str());
    }
    Some(rendered)
}

pub fn normalize_tool_activity_detail_text(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("↳ ") {
        return Some(rest.to_owned());
    }
    if let Some(request) = trimmed.strip_prefix("request:") {
        return Some(format!("request {}", request.trim_start()));
    }
    if let Some(request) = trimmed.strip_prefix("request ") {
        return Some(format!("request {}", request.trim_start()));
    }
    if let Some(args) = trimmed.strip_prefix("args:") {
        return Some(format!("args {}", args.trim_start()));
    }
    if let Some(args) = trimmed.strip_prefix("args ") {
        return Some(format!("args {}", args.trim_start()));
    }
    if let Some(stdout) = trimmed.strip_prefix("stdout:") {
        return Some(format!("stdout {}", stdout.trim_start()));
    }
    if let Some(stderr) = trimmed.strip_prefix("stderr:") {
        return Some(format!("stderr {}", stderr.trim_start()));
    }
    if let Some(file) = trimmed.strip_prefix("file:") {
        return Some(format!("file {}", file.trim_start()));
    }
    if let Some(metrics) = trimmed.strip_prefix("metrics:") {
        return Some(format!("metrics {}", metrics.trim_start()));
    }
    None
}

pub fn is_tool_activity_section_title(title: &str) -> bool {
    matches!(
        title.trim().to_ascii_lowercase().as_str(),
        "tool activity" | "tools" | "tool calls"
    )
}

pub fn split_local_link_location_suffix(dest_url: &str) -> (&str, Option<String>) {
    if let Some((path, suffix)) = dest_url.rsplit_once('#')
        && !suffix.trim().is_empty()
    {
        return (path, Some(format!("#{suffix}")));
    }

    if let Some(matched) = LOCAL_LINK_COLON_LOCATION_SUFFIX_RE
        .as_ref()
        .and_then(|regex| regex.find(dest_url))
        && matched.end() == dest_url.len()
    {
        let path_len = dest_url.len().saturating_sub(matched.as_str().len());
        return (&dest_url[..path_len], Some(matched.as_str().to_owned()));
    }

    (dest_url, None)
}

fn compact_preview_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Bool(boolean) => Some(boolean.to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::Null => Some("null".to_owned()),
        Value::Array(items) => Some(if items.is_empty() {
            "[]".to_owned()
        } else {
            "…".to_owned()
        }),
        Value::Object(object) => Some(if object.is_empty() {
            "{}".to_owned()
        } else {
            "…".to_owned()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_tool_activity_section_title, normalize_tool_activity_detail_text,
        render_local_link_target_text, shorten_home_display_path,
        split_local_link_location_suffix, split_reasoning_preview_text, strip_reasoning_heading_block,
        visible_text_for_reasoning_mode,
    };

    #[test]
    fn reasoning_preview_split_supports_blank_line_separator() {
        let (reasoning, visible) =
            split_reasoning_preview_text("quiet reasoning\nsecond line\n\nvisible reply");

        assert_eq!(reasoning.as_deref(), Some("quiet reasoning\nsecond line"));
        assert_eq!(visible.as_deref(), Some("visible reply"));
    }

    #[test]
    fn reasoning_preview_split_strips_partial_closing_think_suffix_from_visible_text() {
        let (reasoning, visible) = split_reasoning_preview_text("visible answer</t");

        assert_eq!(reasoning, None);
        assert_eq!(visible.as_deref(), Some("visible answer"));
    }

    #[test]
    fn reasoning_preview_split_supports_explicit_reasoning_heading_block() {
        let (reasoning, visible) = split_reasoning_preview_text(
            "## Reasoning\nThe provider compared two options.\n\nVisible answer.",
        );

        assert_eq!(
            reasoning.as_deref(),
            Some("The provider compared two options.")
        );
        assert_eq!(visible.as_deref(), Some("Visible answer."));
    }

    #[test]
    fn strip_reasoning_heading_block_drops_reasoning_section_and_keeps_following_answer() {
        let stripped = strip_reasoning_heading_block(
            "## Reasoning\nThe provider compared two options.\n\nVisible answer.",
        );

        assert!(!stripped.contains("Reasoning"));
        assert!(!stripped.contains("The provider compared two options."));
        assert_eq!(stripped, "Visible answer.");
    }

    #[test]
    fn visible_text_for_reasoning_mode_hides_think_and_reasoning_heading_blocks() {
        assert_eq!(
            visible_text_for_reasoning_mode(
                "<think>quiet reasoning\nsecond line</think>Hello there",
                false,
            ),
            "Hello there"
        );
        assert_eq!(
            visible_text_for_reasoning_mode(
                "## Reasoning\nThe provider compared two options.\n\nVisible answer.",
                false,
            ),
            "Visible answer."
        );
        assert_eq!(
            visible_text_for_reasoning_mode(
                "## Analysis\nThe provider compared two options.\n\nVisible answer.",
                false,
            ),
            "Visible answer."
        );
    }

    #[test]
    fn shorten_home_display_path_rewrites_home_prefix_to_tilde() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/chum".to_owned());
        assert_eq!(
            shorten_home_display_path(format!("{home}/.loong/config.toml").as_str()),
            "~/.loong/config.toml"
        );
    }

    #[test]
    fn reasoning_preview_split_supports_reasoning_heading_aliases() {
        let (reasoning, visible) = split_reasoning_preview_text(
            "## Analysis\nThe provider compared two options.\n\nVisible answer.",
        );

        assert_eq!(
            reasoning.as_deref(),
            Some("The provider compared two options.")
        );
        assert_eq!(visible.as_deref(), Some("Visible answer."));
    }

    #[test]
    fn normalize_tool_activity_detail_text_rewrites_known_child_prefixes() {
        assert_eq!(
            normalize_tool_activity_detail_text("stdout: done").as_deref(),
            Some("stdout done")
        );
        assert_eq!(
            normalize_tool_activity_detail_text("↳ args {\"limit\":5}").as_deref(),
            Some("args {\"limit\":5}")
        );
        assert_eq!(
            normalize_tool_activity_detail_text("metrics: 42ms · exit=0").as_deref(),
            Some("metrics 42ms · exit=0")
        );
    }

    #[test]
    fn is_tool_activity_section_title_accepts_known_aliases() {
        assert!(is_tool_activity_section_title("Tool activity"));
        assert!(is_tool_activity_section_title("Tools"));
        assert!(is_tool_activity_section_title("Tool Calls"));
        assert!(!is_tool_activity_section_title("Reasoning"));
    }

    #[test]
    fn split_local_link_location_suffix_supports_hash_and_colon_suffixes() {
        assert_eq!(
            split_local_link_location_suffix("/Users/chum/project/src/app.rs#L12"),
            ("/Users/chum/project/src/app.rs", Some("#L12".to_owned()))
        );
        assert_eq!(
            split_local_link_location_suffix("/Users/chum/project/src/app.rs:12:3"),
            ("/Users/chum/project/src/app.rs", Some(":12:3".to_owned()))
        );
    }

    #[test]
    fn render_local_link_target_text_supports_file_urls_and_relative_paths() {
        let file_url = "file:///Users/chum/project/src/app.rs#L12";
        let relative = "./docs/config.toml:12";

        assert!(
            render_local_link_target_text(file_url)
                .is_some_and(|text| text.contains("src/app.rs#L12"))
        );
        assert_eq!(
            render_local_link_target_text(relative).as_deref(),
            Some("./docs/config.toml:12")
        );
    }
}
