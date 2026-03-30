use ratatui::style::Color;

#[derive(Debug, Clone)]
pub(super) struct Palette {
    // Brand
    pub(super) brand: Color,
    pub(super) text: Color,
    // UI chrome
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
}

impl Palette {
    pub(super) fn dark() -> Self {
        Self {
            brand: Color::Rgb(253, 172, 172),
            text: Color::Rgb(252, 245, 226),
            dim: Color::DarkGray,
            separator: Color::Rgb(80, 80, 80),
            user_msg: Color::Rgb(100, 180, 255),
            think_block: Color::Rgb(140, 140, 160),
            tool_running: Color::Yellow,
            tool_done: Color::Rgb(100, 200, 100),
            tool_fail: Color::Rgb(220, 80, 80),
            success: Color::Rgb(100, 200, 100),
            warning: Color::Rgb(255, 180, 50),
            error: Color::Rgb(220, 80, 80),
            info: Color::Rgb(80, 200, 220),
        }
    }

    pub(super) fn light() -> Self {
        Self {
            brand: Color::Rgb(253, 172, 172),
            text: Color::DarkGray,
            dim: Color::Rgb(252, 245, 226),
            separator: Color::Rgb(180, 180, 180),
            user_msg: Color::Rgb(30, 100, 200),
            think_block: Color::Rgb(100, 100, 120),
            tool_running: Color::Rgb(180, 140, 0),
            tool_done: Color::Rgb(40, 140, 40),
            tool_fail: Color::Rgb(180, 40, 40),
            success: Color::Rgb(40, 140, 40),
            warning: Color::Rgb(180, 120, 0),
            error: Color::Rgb(180, 40, 40),
            info: Color::Rgb(30, 140, 160),
        }
    }

    pub(super) fn plain() -> Self {
        Self {
            brand: Color::Reset,
            text: Color::Reset,
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
        }
    }
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
