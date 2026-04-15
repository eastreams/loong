use std::borrow::Cow;
use std::env;

use console::Term;

const WIDE_BANNER_MIN_WIDTH: usize = 80;
const SPLIT_BANNER_MIN_WIDTH: usize = 46;

const WIDE_BANNER: [&str; 6] = [
    "██╗      ██████╗  ██████╗ ███╗   ██╗ ██████╗  ██████╗██╗      █████╗ ██╗    ██╗",
    "██║     ██╔═══██╗██╔═══██╗████╗  ██║██╔════╝ ██╔════╝██║     ██╔══██╗██║    ██║",
    "██║     ██║   ██║██║   ██║██╔██╗ ██║██║  ███╗██║     ██║     ███████║██║ █╗ ██║",
    "██║     ██║   ██║██║   ██║██║╚██╗██║██║   ██║██║     ██║     ██╔══██║██║███╗██║",
    "███████╗╚██████╔╝╚██████╔╝██║ ╚████║╚██████╔╝╚██████╗███████╗██║  ██║╚███╔███╔╝",
    "╚══════╝ ╚═════╝  ╚═════╝ ╚═╝  ╚═══╝ ╚═════╝  ╚═════╝╚══════╝╚═╝  ╚═╝ ╚══╝╚══╝",
];

const SPLIT_BANNER: [&str; 13] = [
    "██╗      ██████╗  ██████╗ ███╗   ██╗ ██████╗",
    "██║     ██╔═══██╗██╔═══██╗████╗  ██║██╔════╝",
    "██║     ██║   ██║██║   ██║██╔██╗ ██║██║  ███╗",
    "██║     ██║   ██║██║   ██║██║╚██╗██║██║   ██║",
    "███████╗╚██████╔╝╚██████╔╝██║ ╚████║╚██████╔╝",
    "╚══════╝ ╚═════╝  ╚═════╝ ╚═╝  ╚═══╝ ╚═════╝",
    "",
    "██████╗██╗      █████╗ ██╗    ██╗",
    "██╔════╝██║     ██╔══██╗██║    ██║",
    "██║     ██║     ███████║██║ █╗ ██║",
    "██║     ██║     ██╔══██║██║███╗██║",
    "╚██████╗███████╗██║  ██║╚███╔███╔╝",
    "╚═════╝╚══════╝╚═╝  ╚═╝ ╚══╝╚══╝",
];

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BRAND_ACCENT: &str = "\x1b[38;2;253;172;172m";
const ANSI_BRAND_CREAM: &str = "\x1b[38;2;252;245;226m";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrandPalette {
    banner_ansi: &'static str,
    text_ansi: &'static str,
}

impl BrandPalette {
    pub const fn new(banner_ansi: &'static str, text_ansi: &'static str) -> Self {
        Self {
            banner_ansi,
            text_ansi,
        }
    }
}

pub const DEFAULT_BRAND_PALETTE: BrandPalette =
    BrandPalette::new(ANSI_BRAND_ACCENT, ANSI_BRAND_CREAM);
pub const ONBOARD_BRAND_PALETTE: BrandPalette =
    BrandPalette::new(ANSI_BRAND_ACCENT, ANSI_BRAND_CREAM);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrandLineRole {
    Banner,
    Version,
    Subtitle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrandLine {
    pub role: BrandLineRole,
    pub text: String,
}

impl BrandLine {
    pub fn new(role: BrandLineRole, text: impl Into<String>) -> Self {
        Self {
            role,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildVersionInfo {
    version: Cow<'static, str>,
    channel: Option<Cow<'static, str>>,
    short_sha: Option<Cow<'static, str>>,
    release_build: bool,
}

impl BuildVersionInfo {
    pub fn current() -> Self {
        let release_build = option_env!("LOONG_RELEASE_BUILD")
            .or(option_env!("LOONGCLAW_RELEASE_BUILD"))
            .map(|raw| raw.trim())
            .is_some_and(is_truthy_env_value);
        let short_sha = option_env!("LOONG_GIT_SHA")
            .or(option_env!("LOONGCLAW_GIT_SHA"))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(short_sha);
        let channel = option_env!("LOONG_BUILD_CHANNEL")
            .or(option_env!("LOONGCLAW_BUILD_CHANNEL"))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Cow::Borrowed)
            .or_else(|| {
                if release_build {
                    None
                } else if cfg!(debug_assertions) || short_sha.is_some() {
                    Some(Cow::Borrowed("dev"))
                } else {
                    None
                }
            });

        Self {
            version: Cow::Borrowed(env!("CARGO_PKG_VERSION")),
            channel,
            short_sha,
            release_build,
        }
    }

    #[cfg(test)]
    pub fn new_for_test(
        version: &'static str,
        channel: Option<&'static str>,
        short_sha: Option<&'static str>,
        release_build: bool,
    ) -> Self {
        Self {
            version: Cow::Borrowed(version),
            channel: channel.map(Cow::Borrowed),
            short_sha: short_sha.map(Cow::Borrowed),
            release_build,
        }
    }

    pub fn render_version_line(&self) -> String {
        let mut parts = vec![format!("v{}", self.version)];
        if !self.release_build {
            if let Some(channel) = self.channel.as_deref() {
                parts.push(channel.to_owned());
            }
            if let Some(short_sha) = self.short_sha.as_deref() {
                parts.push(short_sha.to_owned());
            }
        }
        parts.join(" · ")
    }
}

pub fn render_brand_banner_lines(width: usize) -> Vec<&'static str> {
    if width >= WIDE_BANNER_MIN_WIDTH {
        return WIDE_BANNER.to_vec();
    }
    if width >= SPLIT_BANNER_MIN_WIDTH {
        return SPLIT_BANNER.to_vec();
    }
    vec!["LOONG"]
}

pub fn render_brand_header(
    width: usize,
    build: &BuildVersionInfo,
    subtitle: Option<&str>,
) -> Vec<BrandLine> {
    let mut lines = render_brand_banner_lines(width)
        .into_iter()
        .map(|line| BrandLine::new(BrandLineRole::Banner, line))
        .collect::<Vec<_>>();
    let wrap_width = width.max("LOONG".len());
    lines.extend(
        render_wrapped_text_line("", &build.render_version_line(), wrap_width)
            .into_iter()
            .filter(|line| !line.is_empty())
            .map(|line| BrandLine::new(BrandLineRole::Version, line)),
    );
    if let Some(subtitle) = subtitle.map(str::trim).filter(|value| !value.is_empty()) {
        lines.extend(
            render_wrapped_text_line("", subtitle, wrap_width)
                .into_iter()
                .filter(|line| !line.is_empty())
                .map(|line| BrandLine::new(BrandLineRole::Subtitle, line)),
        );
    }
    lines
}

pub fn render_compact_brand_header(
    width: usize,
    build: &BuildVersionInfo,
    subtitle: Option<&str>,
) -> Vec<BrandLine> {
    let brand = "LOONG";
    let version = build.render_version_line();
    let width = width.max(brand.len());
    let combined = format!("{brand}  {version}");
    let mut lines = if combined.len() <= width {
        vec![BrandLine::new(BrandLineRole::Banner, combined)]
    } else {
        let mut compact_lines = vec![BrandLine::new(BrandLineRole::Banner, brand)];
        compact_lines.extend(
            render_wrapped_text_line("", &version, width)
                .into_iter()
                .filter(|line| !line.is_empty())
                .map(|line| BrandLine::new(BrandLineRole::Version, line)),
        );
        compact_lines
    };
    if let Some(subtitle) = subtitle.map(str::trim).filter(|value| !value.is_empty()) {
        lines.extend(
            render_wrapped_text_line("", subtitle, width)
                .into_iter()
                .filter(|line| !line.is_empty())
                .map(|line| BrandLine::new(BrandLineRole::Subtitle, line)),
        );
    }
    lines
}

pub fn render_brand_header_for_current_build(width: usize, subtitle: Option<&str>) -> Vec<String> {
    style_brand_lines(
        &render_brand_header(width, &BuildVersionInfo::current(), subtitle),
        terminal_supports_color(),
    )
}

pub fn style_brand_lines(lines: &[BrandLine], color_enabled: bool) -> Vec<String> {
    style_brand_lines_with_palette(lines, color_enabled, DEFAULT_BRAND_PALETTE)
}

pub fn style_brand_lines_with_palette(
    lines: &[BrandLine],
    color_enabled: bool,
    palette: BrandPalette,
) -> Vec<String> {
    lines
        .iter()
        .map(|line| style_brand_line(line, color_enabled, palette))
        .collect()
}

pub fn detect_render_width() -> usize {
    let terminal_width = probe_terminal_width();
    let columns = env::var("COLUMNS").ok();

    resolve_render_width(terminal_width, columns.as_deref())
}

fn probe_terminal_width() -> Option<usize> {
    let stdout_width = terminal_width_from_term(&Term::stdout());
    if stdout_width.is_some() {
        return stdout_width;
    }

    terminal_width_from_term(&Term::stderr())
}

fn terminal_width_from_term(term: &Term) -> Option<usize> {
    let (_rows, columns) = term.size_checked()?;
    let width = usize::from(columns);
    (width > 0).then_some(width)
}

fn resolve_render_width(terminal_width: Option<usize>, columns: Option<&str>) -> usize {
    if let Some(width) = terminal_width.filter(|width| *width > 0) {
        return width;
    }

    parse_columns_width(columns).unwrap_or(80)
}

fn parse_columns_width(columns: Option<&str>) -> Option<usize> {
    columns
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|width| *width > 0)
}

pub fn render_wrapped_text_line(prefix: &str, value: &str, width: usize) -> Vec<String> {
    render_wrapped_text_line_with_continuation(prefix, "  ", value, width)
}

pub fn render_wrapped_text_line_with_continuation(
    prefix: &str,
    continuation_prefix: &str,
    value: &str,
    width: usize,
) -> Vec<String> {
    let segments = value
        .split_whitespace()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    render_wrapped_segments(prefix, continuation_prefix, &segments, " ", width)
}

pub fn render_wrapped_csv_line(prefix: &str, values: &[&str], width: usize) -> Vec<String> {
    render_wrapped_segments(prefix, "  ", values, ", ", width)
}

pub fn render_wrapped_display_line(line: &str, width: usize) -> Vec<String> {
    if line.trim().is_empty() {
        return vec![String::new()];
    }

    let indent_width = line
        .chars()
        .take_while(|character| character.is_ascii_whitespace())
        .count();
    let indent = &line[..indent_width];
    let trimmed = &line[indent_width..];

    if let Some((prefix, continuation_prefix, rest)) = parse_display_list_item(indent, trimmed) {
        if let Some((label, value)) = rest.split_once(": ") {
            return render_wrapped_labeled_display_line(
                &prefix,
                &continuation_prefix,
                label,
                value,
                width,
            );
        }
        return render_wrapped_text_line_with_continuation(
            &prefix,
            &continuation_prefix,
            rest,
            width,
        );
    }

    if let Some((label, value)) = trimmed.split_once(": ") {
        return render_wrapped_labeled_display_line(
            indent,
            &format!("{indent}  "),
            label,
            value,
            width,
        );
    }

    render_wrapped_text_line_with_continuation(indent, indent, trimmed, width)
}

fn parse_display_list_item<'a>(
    indent: &str,
    trimmed: &'a str,
) -> Option<(String, String, &'a str)> {
    if let Some(rest) = trimmed.strip_prefix("- ") {
        return Some((format!("{indent}- "), format!("{indent}  "), rest));
    }

    if let Some(rest) = trimmed.strip_prefix("* ") {
        return Some((format!("{indent}- "), format!("{indent}  "), rest));
    }

    if let Some(rest) = trimmed.strip_prefix("+ ") {
        return Some((format!("{indent}- "), format!("{indent}  "), rest));
    }

    let marker_length = parse_ordered_list_marker_length(trimmed)?;
    let marker = &trimmed[..marker_length];
    let rest = &trimmed[marker_length..];
    let continuation_padding = " ".repeat(marker.chars().count());
    let prefix = format!("{indent}{marker}");
    let continuation_prefix = format!("{indent}{continuation_padding}");

    Some((prefix, continuation_prefix, rest))
}

fn parse_ordered_list_marker_length(trimmed: &str) -> Option<usize> {
    let digit_count = trimmed
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count();
    if digit_count == 0 {
        return None;
    }

    let marker = trimmed.as_bytes().get(digit_count).copied()?;
    if marker != b'.' && marker != b')' {
        return None;
    }

    let separator = trimmed.as_bytes().get(digit_count + 1).copied()?;
    if separator != b' ' {
        return None;
    }

    Some(digit_count + 2)
}

fn render_wrapped_labeled_display_line(
    prefix: &str,
    continuation_prefix: &str,
    label: &str,
    value: &str,
    width: usize,
) -> Vec<String> {
    let labeled_prefix = format!("{prefix}{label}: ");
    if labeled_prefix.len() <= width {
        return render_wrapped_text_line_with_continuation(
            &labeled_prefix,
            continuation_prefix,
            value,
            width,
        );
    }

    let mut lines = render_wrapped_text_line_with_continuation(
        prefix,
        continuation_prefix,
        &format!("{label}:"),
        width,
    );
    lines.extend(render_wrapped_text_line_with_continuation(
        continuation_prefix,
        continuation_prefix,
        value,
        width,
    ));
    lines
}

pub fn render_wrapped_segments(
    prefix: &str,
    continuation_prefix: &str,
    segments: &[&str],
    separator: &str,
    width: usize,
) -> Vec<String> {
    let width = width
        .max(prefix.trim_end().len())
        .max(continuation_prefix.len());
    let mut lines = Vec::new();
    let mut current_line = prefix.to_owned();
    let mut line_has_content = false;

    for segment in segments
        .iter()
        .copied()
        .filter(|segment| !segment.is_empty())
    {
        let mut remaining = segment;
        loop {
            let joiner = if line_has_content { separator } else { "" };
            let available = width.saturating_sub(current_line.len() + joiner.len());

            if remaining.len() <= available {
                current_line.push_str(joiner);
                current_line.push_str(remaining);
                line_has_content = true;
                break;
            }

            if line_has_content {
                lines.push(current_line.trim_end().to_owned());
                current_line = continuation_prefix.to_owned();
                line_has_content = false;
                continue;
            }

            if available == 0 {
                let prefix_line = current_line.trim_end();
                if !prefix_line.is_empty() {
                    lines.push(prefix_line.to_owned());
                }
                current_line = continuation_prefix.to_owned();
                line_has_content = false;
                continue;
            }

            let split_index = take_fitting_prefix(remaining, available);
            current_line.push_str(&remaining[..split_index]);
            lines.push(current_line.trim_end().to_owned());
            current_line = continuation_prefix.to_owned();
            line_has_content = false;
            remaining = &remaining[split_index..];
        }
    }

    if line_has_content || lines.is_empty() {
        lines.push(current_line.trim_end().to_owned());
    }
    lines
}

fn take_fitting_prefix(text: &str, max_width: usize) -> usize {
    let mut used = 0;
    let mut end = 0;

    for (index, character) in text.char_indices() {
        let char_width = character.len_utf8();
        if used + char_width > max_width {
            break;
        }
        used += char_width;
        end = index + char_width;
    }

    if end == 0 {
        text.chars().next().map(char::len_utf8).unwrap_or(0)
    } else {
        end
    }
}

fn style_brand_line(line: &BrandLine, color_enabled: bool, palette: BrandPalette) -> String {
    if !color_enabled {
        return line.text.clone();
    }

    let color = match line.role {
        BrandLineRole::Banner => palette.banner_ansi,
        BrandLineRole::Version | BrandLineRole::Subtitle => palette.text_ansi,
    };
    format!("{color}{}{ANSI_RESET}", line.text)
}

fn short_sha(raw: &str) -> Cow<'static, str> {
    Cow::Owned(raw.chars().take(7).collect())
}

fn is_truthy_env_value(raw: &str) -> bool {
    matches!(raw, "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "Yes")
}

fn terminal_supports_color() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    !matches!(
        std::env::var("TERM").ok().as_deref(),
        Some("dumb") | Some("")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_banner_variant_uses_wide_logo_for_standard_width() {
        let lines = render_brand_banner_lines(80);

        assert_eq!(lines.len(), 6);
        assert!(
            lines[0].starts_with("██╗"),
            "wide banner should use the full block-logo variant: {lines:#?}"
        );
    }

    #[test]
    fn presentation_banner_variant_uses_split_logo_for_medium_width() {
        let lines = render_brand_banner_lines(60);

        assert!(
            lines.len() > 6,
            "split banner should use both LOONG and CLAW blocks: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line.is_empty()),
            "split banner should include a spacer between the two halves: {lines:#?}"
        );
    }

    #[test]
    fn presentation_banner_variant_uses_plain_logo_for_narrow_width() {
        let lines = render_brand_banner_lines(32);

        assert_eq!(lines, vec!["LOONG"]);
    }

    #[test]
    fn presentation_version_line_is_clean_for_release_build() {
        let build = BuildVersionInfo::new_for_test("0.1.2", None, None, true);

        assert_eq!(build.render_version_line(), "v0.1.2");
    }

    #[test]
    fn presentation_version_line_is_traceable_for_dev_build() {
        let build = BuildVersionInfo::new_for_test("0.1.2", Some("dev"), Some("1a2b3c4"), false);

        assert_eq!(build.render_version_line(), "v0.1.2 · dev · 1a2b3c4");
    }

    #[test]
    fn presentation_current_build_surfaces_embedded_git_trace_metadata_when_available() {
        let release_build = option_env!("LOONG_RELEASE_BUILD")
            .or(option_env!("LOONGCLAW_RELEASE_BUILD"))
            .map(str::trim)
            .is_some_and(is_truthy_env_value);
        if release_build {
            return;
        }

        let version_line = BuildVersionInfo::current().render_version_line();

        if let Some(short_sha) = option_env!("LOONG_GIT_SHA")
            .or(option_env!("LOONGCLAW_GIT_SHA"))
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            assert!(
                version_line.contains(short_sha),
                "current build version line should expose the embedded short git sha when build metadata provides it: {version_line}"
            );
        }

        if let Some(channel) = option_env!("LOONG_BUILD_CHANNEL")
            .or(option_env!("LOONGCLAW_BUILD_CHANNEL"))
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            assert!(
                version_line.contains(channel),
                "current build version line should surface the embedded build channel: {version_line}"
            );
        }
    }

    #[test]
    fn presentation_style_brand_lines_can_disable_color() {
        let lines = vec![
            BrandLine::new(BrandLineRole::Banner, "LOONG"),
            BrandLine::new(BrandLineRole::Version, "v0.1.2 · dev"),
        ];

        let rendered = style_brand_lines(&lines, false);

        assert_eq!(
            rendered,
            vec!["LOONG".to_owned(), "v0.1.2 · dev".to_owned()]
        );
    }

    #[test]
    fn presentation_style_brand_lines_uses_soft_red_banner_by_default() {
        let lines = vec![BrandLine::new(BrandLineRole::Banner, "LOONG")];

        let rendered = style_brand_lines(&lines, true);

        assert_eq!(
            rendered,
            vec!["\u{1b}[38;2;253;172;172mLOONG\u{1b}[0m".to_owned()]
        );
    }

    #[test]
    fn presentation_style_brand_lines_with_onboard_palette_uses_soft_red_banner() {
        let lines = vec![BrandLine::new(BrandLineRole::Banner, "LOONG")];

        let rendered = style_brand_lines_with_palette(&lines, true, ONBOARD_BRAND_PALETTE);

        assert_eq!(
            rendered,
            vec!["\u{1b}[38;2;253;172;172mLOONG\u{1b}[0m".to_owned()]
        );
    }

    #[test]
    fn presentation_compact_brand_header_keeps_brand_and_version_on_one_line() {
        let build = BuildVersionInfo::new_for_test("0.1.2", Some("dev"), Some("1a2b3c4"), false);

        let lines = render_compact_brand_header(80, &build, Some("choose model"));

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "LOONG  v0.1.2 · dev · 1a2b3c4");
        assert_eq!(lines[1].text, "choose model");
    }

    #[test]
    fn presentation_compact_brand_header_wraps_on_narrow_width() {
        let build = BuildVersionInfo::new_for_test("0.1.2", Some("dev"), Some("1a2b3c4"), false);

        let lines = render_compact_brand_header(22, &build, Some("choose credential env"));

        assert!(
            lines.iter().all(|line| line.text.len() <= 22),
            "compact brand header should respect narrow widths instead of forcing the brand and version onto one overflowing line: {lines:#?}"
        );
        assert_eq!(lines[0].text, "LOONG");
        assert!(
            lines.iter().any(|line| line.role == BrandLineRole::Version),
            "narrow compact header should keep version information visible on its own wrapped line: {lines:#?}"
        );
    }

    #[test]
    fn presentation_brand_header_wraps_version_and_subtitle_on_narrow_width() {
        let build = BuildVersionInfo::new_for_test("0.1.2", Some("dev"), Some("1a2b3c4"), false);

        let lines = render_brand_header(
            18,
            &build,
            Some("guided setup for provider, channels, and workspace guidance"),
        );

        assert!(
            lines.iter().all(|line| line.text.len() <= 18),
            "full brand header should respect narrow widths for version and subtitle lines: {lines:#?}"
        );
        assert_eq!(lines[0].text, "LOONG");
        assert!(
            lines
                .iter()
                .any(|line| line.role == BrandLineRole::Version && line.text.starts_with("v0.1.2")),
            "narrow full header should keep version information visible after the logo: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.role == BrandLineRole::Subtitle && line.text.contains("guided")),
            "narrow full header should wrap subtitle copy instead of overflowing it: {lines:#?}"
        );
    }

    #[test]
    fn presentation_wraps_text_lines_for_narrow_width() {
        let lines = render_wrapped_text_line(
            "source: ",
            "Codex config at ~/.codex/agents/loong/config.toml",
            48,
        );

        assert_eq!(
            lines,
            vec![
                "source: Codex config at".to_owned(),
                "  ~/.codex/agents/loong/config.toml".to_owned(),
            ]
        );
    }

    #[test]
    fn presentation_wraps_segment_lists_at_separator_boundaries() {
        let lines = render_wrapped_segments(
            "note: ",
            "  ",
            &[
                "other detected settings stay merged",
                "use --provider <id> to choose the active provider",
            ],
            "; ",
            52,
        );

        assert_eq!(
            lines,
            vec![
                "note: other detected settings stay merged".to_owned(),
                "  use --provider <id> to choose the active provider".to_owned(),
            ]
        );
    }

    #[test]
    fn presentation_wraps_display_line_with_label_prefix() {
        let lines = render_wrapped_display_line(
            "    source: Codex config at ~/.codex/agents/loong/config.toml",
            48,
        );

        assert_eq!(
            lines,
            vec![
                "    source: Codex config at".to_owned(),
                "      ~/.codex/agents/loong/config.toml".to_owned(),
            ]
        );
    }

    #[test]
    fn presentation_wraps_display_line_with_bullet_prefix() {
        let lines = render_wrapped_display_line(
            "- workspace guidance: AGENTS.md, CLAUDE.md, and local policy",
            42,
        );

        assert_eq!(
            lines,
            vec![
                "- workspace guidance: AGENTS.md,",
                "  CLAUDE.md, and local policy",
            ]
        );
    }

    #[test]
    fn presentation_normalizes_markdown_star_bullets() {
        let lines = render_wrapped_display_line(
            "* workspace guidance: AGENTS.md, CLAUDE.md, and local policy",
            42,
        );

        assert_eq!(
            lines,
            vec![
                "- workspace guidance: AGENTS.md,",
                "  CLAUDE.md, and local policy",
            ]
        );
    }

    #[test]
    fn presentation_wraps_display_line_with_ordered_list_prefix() {
        let lines = render_wrapped_display_line(
            "12. validate the current runtime before changing the prompt hydration path",
            36,
        );

        assert_eq!(
            lines,
            vec![
                "12. validate the current runtime",
                "    before changing the prompt",
                "    hydration path",
            ]
        );
    }

    #[test]
    fn presentation_normalizes_markdown_plus_bullets() {
        let lines = render_wrapped_display_line(
            "+ workspace guidance: AGENTS.md, CLAUDE.md, and local policy",
            42,
        );

        assert_eq!(
            lines,
            vec![
                "- workspace guidance: AGENTS.md,",
                "  CLAUDE.md, and local policy",
            ]
        );
    }

    #[test]
    fn presentation_wraps_display_line_with_numeric_paren_prefix() {
        let lines = render_wrapped_display_line(
            "12) validate the current runtime before changing the prompt hydration path",
            36,
        );

        assert_eq!(
            lines,
            vec![
                "12) validate the current runtime",
                "    before changing the prompt",
                "    hydration path",
            ]
        );
    }

    #[test]
    fn presentation_detect_render_width_prefers_live_terminal_width() {
        let width = resolve_render_width(Some(96), Some("42"));

        assert_eq!(width, 96);
    }

    #[test]
    fn presentation_detect_render_width_uses_columns_fallback() {
        let width = resolve_render_width(None, Some("72"));

        assert_eq!(width, 72);
    }

    #[test]
    fn presentation_detect_render_width_defaults_when_no_signal_exists() {
        let width = resolve_render_width(None, Some("0"));

        assert_eq!(width, 80);
    }

    #[test]
    fn presentation_wraps_long_unbroken_segment_without_overflow() {
        let lines = render_wrapped_display_line(
            "- current env: OPENAI_COMPATIBLE_PROVIDER_SUPER_LONG_ENV_POINTER",
            28,
        );

        assert!(
            lines.iter().all(|line| line.len() <= 28),
            "shared presentation wrapping should split oversized single segments instead of overflowing the target width: {lines:#?}"
        );
        assert!(
            lines
                .first()
                .is_some_and(|line| line.starts_with("- current env: ")),
            "long-token wrapping should still keep the label visible on the first line: {lines:#?}"
        );
    }

    #[test]
    fn presentation_wraps_long_label_prefix_without_overflow() {
        let lines =
            render_wrapped_display_line("- press Enter to use suggested env: OPENAI_API_KEY", 22);

        assert!(
            lines.iter().all(|line| line.len() <= 22),
            "long label prefixes should wrap instead of overflowing narrow widths: {lines:#?}"
        );
        assert_eq!(
            lines,
            vec![
                "- press Enter to use".to_owned(),
                "  suggested env:".to_owned(),
                "  OPENAI_API_KEY".to_owned(),
            ]
        );
    }
}
