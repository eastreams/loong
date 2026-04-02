use std::env;

use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OnboardPalette {
    pub(crate) brand: Color,
    pub(crate) text: Color,
    pub(crate) secondary_text: Color,
    pub(crate) muted_text: Color,
    pub(crate) border: Color,
    pub(crate) surface: Color,
    pub(crate) surface_emphasis: Color,
    pub(crate) info: Color,
    pub(crate) info_surface: Color,
    pub(crate) success: Color,
    pub(crate) success_surface: Color,
    pub(crate) warning: Color,
    pub(crate) warning_surface: Color,
    pub(crate) error: Color,
    pub(crate) error_surface: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnboardThemeMode {
    Dark,
    Light,
    Plain,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct OnboardThemeEnvironment {
    override_mode: Option<String>,
    colorfgbg: Option<String>,
    no_color: bool,
}

impl OnboardThemeEnvironment {
    fn read() -> Self {
        Self {
            override_mode: env::var("LOONGCLAW_ONBOARD_THEME").ok(),
            colorfgbg: env::var("COLORFGBG").ok(),
            no_color: env::var_os("NO_COLOR").is_some(),
        }
    }
}

impl OnboardThemeMode {
    fn detect(env: &OnboardThemeEnvironment) -> Self {
        if env.no_color {
            return Self::Plain;
        }

        if let Some(raw_mode) = env.override_mode.as_deref()
            && let Some(mode) = Self::from_override(raw_mode)
        {
            return mode;
        }

        if let Some(raw_colorfgbg) = env.colorfgbg.as_deref()
            && let Some(background_index) = parse_colorfgbg_background(raw_colorfgbg)
        {
            return if xterm_color_is_light(background_index) {
                Self::Light
            } else {
                Self::Dark
            };
        }

        Self::Dark
    }

    fn from_override(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            "plain" | "none" | "mono" | "monochrome" => Some(Self::Plain),
            "auto" => None,
            _ => None,
        }
    }
}

impl OnboardPalette {
    pub(crate) fn current() -> Self {
        let env = OnboardThemeEnvironment::read();
        Self::for_mode(OnboardThemeMode::detect(&env))
    }

    pub(crate) fn dark() -> Self {
        Self {
            brand: Color::Rgb(255, 86, 110),
            text: Color::Rgb(252, 248, 248),
            secondary_text: Color::Rgb(238, 229, 230),
            muted_text: Color::Rgb(212, 200, 201),
            border: Color::Rgb(186, 165, 168),
            surface: Color::Rgb(16, 14, 15),
            surface_emphasis: Color::Rgb(52, 23, 29),
            info: Color::Rgb(122, 198, 218),
            info_surface: Color::Rgb(18, 31, 36),
            success: Color::Rgb(108, 188, 124),
            success_surface: Color::Rgb(18, 31, 22),
            warning: Color::Rgb(238, 157, 166),
            warning_surface: Color::Rgb(50, 27, 31),
            error: Color::Rgb(255, 96, 112),
            error_surface: Color::Rgb(52, 20, 25),
        }
    }

    pub(crate) fn light() -> Self {
        Self {
            brand: Color::Rgb(222, 26, 54),
            text: Color::Rgb(35, 29, 29),
            secondary_text: Color::Rgb(86, 67, 69),
            muted_text: Color::Rgb(120, 98, 101),
            border: Color::Rgb(198, 161, 165),
            surface: Color::Rgb(250, 245, 245),
            surface_emphasis: Color::Rgb(255, 248, 249),
            info: Color::Rgb(34, 112, 128),
            info_surface: Color::Rgb(228, 240, 243),
            success: Color::Rgb(58, 128, 84),
            success_surface: Color::Rgb(232, 242, 235),
            warning: Color::Rgb(176, 71, 85),
            warning_surface: Color::Rgb(248, 236, 238),
            error: Color::Rgb(194, 54, 70),
            error_surface: Color::Rgb(249, 233, 235),
        }
    }

    pub(crate) fn plain() -> Self {
        Self {
            brand: Color::Reset,
            text: Color::Reset,
            secondary_text: Color::Reset,
            muted_text: Color::Reset,
            border: Color::Reset,
            surface: Color::Reset,
            surface_emphasis: Color::Reset,
            info: Color::Reset,
            info_surface: Color::Reset,
            success: Color::Reset,
            success_surface: Color::Reset,
            warning: Color::Reset,
            warning_surface: Color::Reset,
            error: Color::Reset,
            error_surface: Color::Reset,
        }
    }

    fn for_mode(mode: OnboardThemeMode) -> Self {
        match mode {
            OnboardThemeMode::Dark => Self::dark(),
            OnboardThemeMode::Light => Self::light(),
            OnboardThemeMode::Plain => Self::plain(),
        }
    }
}

fn parse_colorfgbg_background(raw: &str) -> Option<u8> {
    raw.split(';').next_back()?.trim().parse::<u8>().ok()
}

fn xterm_color_is_light(index: u8) -> bool {
    let (red, green, blue) = xterm_color_rgb(index);
    let luma = (u32::from(red) * 299 + u32::from(green) * 587 + u32::from(blue) * 114) / 1000;
    luma >= 150
}

fn xterm_color_rgb(index: u8) -> (u8, u8, u8) {
    const ANSI_16: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 49, 49),
        (13, 188, 121),
        (229, 229, 16),
        (36, 114, 200),
        (188, 63, 188),
        (17, 168, 205),
        (229, 229, 229),
        (102, 102, 102),
        (241, 76, 76),
        (35, 209, 139),
        (245, 245, 67),
        (59, 142, 234),
        (214, 112, 214),
        (41, 184, 219),
        (255, 255, 255),
    ];

    match index {
        0..=15 => ANSI_16
            .get(usize::from(index))
            .copied()
            .unwrap_or((255, 255, 255)),
        16..=231 => {
            const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
            let adjusted = index - 16;
            let red_index = usize::from(adjusted / 36);
            let green_index = usize::from((adjusted % 36) / 6);
            let blue_index = usize::from(adjusted % 6);
            let red = LEVELS.get(red_index).copied().unwrap_or(0);
            let green = LEVELS.get(green_index).copied().unwrap_or(0);
            let blue = LEVELS.get(blue_index).copied().unwrap_or(0);
            (red, green, blue)
        }
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            (gray, gray, gray)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_palette_uses_warmer_high_contrast_neutrals() {
        let palette = OnboardPalette::dark();

        assert_eq!(palette.brand, Color::Rgb(255, 86, 110));
        assert_eq!(palette.text, Color::Rgb(252, 248, 248));
        assert_eq!(palette.secondary_text, Color::Rgb(238, 229, 230));
        assert_eq!(palette.muted_text, Color::Rgb(212, 200, 201));
        assert_eq!(palette.border, Color::Rgb(186, 165, 168));
        assert_eq!(palette.warning, Color::Rgb(238, 157, 166));
    }

    #[test]
    fn light_palette_keeps_brand_warmth_without_dark_mode_values() {
        let palette = OnboardPalette::light();

        assert_eq!(palette.brand, Color::Rgb(222, 26, 54));
        assert_eq!(palette.text, Color::Rgb(35, 29, 29));
        assert_eq!(palette.muted_text, Color::Rgb(120, 98, 101));
        assert_eq!(palette.border, Color::Rgb(198, 161, 165));
        assert_eq!(palette.warning, Color::Rgb(176, 71, 85));
    }
}
