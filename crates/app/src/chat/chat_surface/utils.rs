use crate::constants::spinners::*;
use ratatui::style::Color;
use serde_json::Value;
use std::env;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
