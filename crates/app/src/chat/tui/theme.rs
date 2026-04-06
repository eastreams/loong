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
            brand: hex_color(0xd14a42),
            text: Color::Rgb(252, 245, 226),
            surface: Color::Rgb(15, 14, 14),
            surface_alt: Color::Rgb(29, 26, 26),
            dim: Color::Rgb(164, 158, 154),
            separator: Color::Rgb(72, 64, 62),
            user_msg: Color::Rgb(176, 164, 154),
            think_block: Color::Rgb(176, 176, 196),
            tool_running: Color::Rgb(210, 173, 96),
            tool_done: Color::Rgb(110, 177, 128),
            tool_fail: Color::Rgb(207, 111, 111),
            success: Color::Rgb(110, 177, 128),
            warning: Color::Rgb(210, 173, 96),
            error: Color::Rgb(207, 111, 111),
            info: Color::Rgb(118, 177, 204),
            subagent_accents: [
                hex_color(0xd9bb56),
                hex_color(0x5dbab5),
                hex_color(0xd2746f),
                hex_color(0x79c7a5),
                hex_color(0x79a9d9),
            ],
        }
    }

    pub(super) fn light() -> Self {
        Self {
            brand: hex_color(0xc63d3d),
            text: Color::Rgb(40, 40, 40),
            surface: Color::Rgb(250, 247, 244),
            surface_alt: Color::Rgb(242, 237, 233),
            dim: Color::Rgb(102, 92, 88),
            separator: Color::Rgb(201, 193, 188),
            user_msg: Color::Rgb(129, 110, 98),
            think_block: Color::Rgb(72, 72, 96),
            tool_running: Color::Rgb(144, 110, 48),
            tool_done: Color::Rgb(62, 132, 86),
            tool_fail: Color::Rgb(173, 74, 74),
            success: Color::Rgb(62, 132, 86),
            warning: Color::Rgb(144, 110, 48),
            error: Color::Rgb(173, 74, 74),
            info: Color::Rgb(67, 128, 151),
            subagent_accents: [
                hex_color(0xc8b9c0),
                hex_color(0xd76464),
                hex_color(0x9dc4dc),
                hex_color(0x95c6ca),
                hex_color(0x79afa5),
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
