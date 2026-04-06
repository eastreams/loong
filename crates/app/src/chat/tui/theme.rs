use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ratatui::style::Color;

#[derive(Debug, Clone)]
pub(super) struct Palette {
    // Brand
    pub(super) brand: Color,
    pub(super) text: Color,
    // UI chrome
    pub(super) surface: Color,
    pub(super) surface_alt: Color,
    pub(super) dim: Color,
    pub(super) separator: Color,
    pub(super) user_msg: Color,
    pub(super) think_block: Color,
    // Status
    pub(super) tool_running: Color,
    pub(super) tool_done: Color,
    pub(super) tool_fail: Color,
    pub(super) success: Color,
    pub(super) warning: Color,
    pub(super) error: Color,
    pub(super) info: Color,
    pub(super) subagent_accents: [Color; 5],
}

impl Palette {
    pub(super) fn dark() -> Self {
        Self {
            brand: hex_color(0xe32a2a),
            text: Color::Rgb(252, 245, 226),
            surface: Color::Rgb(12, 12, 12),
            surface_alt: Color::Rgb(24, 24, 24),
            dim: Color::Rgb(170, 170, 170),
            separator: Color::Rgb(62, 62, 62),
            user_msg: Color::Rgb(184, 171, 158),
            think_block: Color::Rgb(176, 176, 196),
            tool_running: Color::Rgb(236, 196, 94),
            tool_done: Color::Rgb(120, 210, 132),
            tool_fail: Color::Rgb(236, 112, 112),
            success: Color::Rgb(120, 210, 132),
            warning: Color::Rgb(236, 196, 94),
            error: Color::Rgb(236, 112, 112),
            info: Color::Rgb(124, 214, 236),
            subagent_accents: [
                hex_color(0xffcc3e),
                hex_color(0x1dece5),
                hex_color(0xef5b5b),
                hex_color(0x53f2b5),
                hex_color(0x5eb5fc),
            ],
        }
    }

    pub(super) fn light() -> Self {
        Self {
            brand: hex_color(0xe32a2a),
            text: Color::Rgb(40, 40, 40),
            surface: Color::Rgb(249, 247, 244),
            surface_alt: Color::Rgb(239, 236, 232),
            dim: Color::Rgb(92, 92, 92),
            separator: Color::Rgb(196, 190, 184),
            user_msg: Color::Rgb(124, 108, 95),
            think_block: Color::Rgb(72, 72, 96),
            tool_running: Color::Rgb(160, 120, 0),
            tool_done: Color::Rgb(30, 120, 30),
            tool_fail: Color::Rgb(180, 30, 30),
            success: Color::Rgb(30, 120, 30),
            warning: Color::Rgb(160, 100, 0),
            error: Color::Rgb(180, 30, 30),
            info: Color::Rgb(20, 120, 140),
            subagent_accents: [
                hex_color(0xcabac8),
                hex_color(0xff101f),
                hex_color(0xb2ddf7),
                hex_color(0x81d6e3),
                hex_color(0x4cb5ae),
            ],
        }
    }

    pub(super) fn plain() -> Self {
        Self {
            brand: Color::Reset,
            text: Color::Reset,
            surface: Color::Reset,
            surface_alt: Color::Reset,
            dim: Color::Reset,
            separator: Color::Reset,
            user_msg: Color::Reset,
            think_block: Color::Reset,
            tool_running: Color::Reset,
            tool_done: Color::Reset,
            tool_fail: Color::Reset,
            success: Color::Reset,
            warning: Color::Reset,
            error: Color::Reset,
            info: Color::Reset,
            subagent_accents: [
                Color::Reset,
                Color::Reset,
                Color::Reset,
                Color::Reset,
                Color::Reset,
            ],
        }
    }

    pub(super) fn subagent_accent(&self, stable_key: &str) -> Color {
        let accent_count = self.subagent_accents.len();
        if accent_count == 0 {
            return self.brand;
        }

        let mut hasher = DefaultHasher::new();
        stable_key.hash(&mut hasher);
        let hash = hasher.finish() as usize;
        let index = hash % accent_count;

        self.subagent_accents
            .get(index)
            .copied()
            .unwrap_or(self.brand)
    }
}

fn hex_color(value: u32) -> Color {
    let red = ((value >> 16) & 0xff) as u8;
    let green = ((value >> 8) & 0xff) as u8;
    let blue = (value & 0xff) as u8;
    Color::Rgb(red, green, blue)
}

// ---------------------------------------------------------------------------
// SemanticPalette: minimal legacy palette used by terminal.rs tests
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SemanticPalette {
    pub(crate) text: Color,
    pub(crate) border: Color,
    pub(crate) accent: Color,
    pub(crate) warning: Color,
    pub(crate) error: Color,
}

impl Default for SemanticPalette {
    fn default() -> Self {
        Self {
            text: Color::White,
            border: Color::DarkGray,
            accent: Color::Cyan,
            warning: Color::Yellow,
            error: Color::Red,
        }
    }
}

impl SemanticPalette {
    pub(crate) fn plain() -> Self {
        Self {
            text: Color::Reset,
            border: Color::Reset,
            accent: Color::Reset,
            warning: Color::Reset,
            error: Color::Reset,
        }
    }
}
