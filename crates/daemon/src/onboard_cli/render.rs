use super::*;
#[cfg(test)]
pub(crate) fn render_onboard_wrapped_display_lines<I, S>(
    display_lines: I,
    width: usize,
) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    display_lines
        .into_iter()
        .flat_map(|line| mvp::presentation::render_wrapped_display_line(line.as_ref(), width))
        .collect()
}

#[cfg(test)]
pub(crate) fn render_onboard_option_lines(
    options: &[OnboardScreenOption],
    width: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    for option in options {
        let suffix = if option.recommended {
            " (recommended)"
        } else {
            ""
        };
        let prefix = render_onboard_option_prefix(&option.key);
        let continuation = " ".repeat(prefix.chars().count());
        lines.extend(
            mvp::presentation::render_wrapped_text_line_with_continuation(
                &prefix,
                &continuation,
                &format!("{}{}", option.label, suffix),
                width,
            ),
        );
        lines.extend(render_onboard_wrapped_display_lines(
            option
                .detail_lines
                .iter()
                .map(|detail| format!("    {detail}"))
                .collect::<Vec<_>>(),
            width,
        ));
    }
    lines
}

pub fn render_default_choice_footer_line(key: &str, description: &str) -> String {
    format!("press Enter to use default {key}, {description}")
}

pub(super) fn render_prompt_with_default_text(label: &str, default: &str) -> String {
    format!("{label} (default: {default}): ")
}

#[cfg(test)]
pub(crate) fn render_onboard_option_prefix(key: &str) -> String {
    format!("{key}) ")
}

pub fn append_escape_cancel_hint(mut lines: Vec<String>) -> Vec<String> {
    if !lines.iter().any(|line| {
        let lower = line.to_ascii_lowercase();
        lower.contains("esc") && lower.contains("cancel")
    }) {
        lines.push(ONBOARD_ESCAPE_CANCEL_HINT.to_owned());
    }
    lines
}

pub(super) fn render_onboard_choice_screen(
    header_style: OnboardHeaderStyle,
    width: usize,
    subtitle: &str,
    title: &str,
    step: Option<(GuidedOnboardStep, GuidedPromptPath)>,
    intro_lines: Vec<String>,
    options: Vec<OnboardScreenOption>,
    footer_lines: Vec<String>,
    show_escape_cancel_hint: bool,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboard_choice_screen_spec(
        header_style,
        subtitle,
        title,
        step,
        intro_lines,
        options,
        footer_lines,
        show_escape_cancel_hint,
    );

    render_onboard_screen_spec(&spec, width, color_enabled)
}

pub(super) fn render_onboard_input_screen(
    width: usize,
    title: &str,
    step: GuidedOnboardStep,
    guided_prompt_path: GuidedPromptPath,
    context_lines: Vec<String>,
    hint_lines: Vec<String>,
    color_enabled: bool,
) -> Vec<String> {
    let spec =
        build_onboard_input_screen_spec(title, step, guided_prompt_path, context_lines, hint_lines);

    render_onboard_screen_spec(&spec, width, color_enabled)
}

pub(super) fn tui_header_style(style: OnboardHeaderStyle) -> TuiHeaderStyle {
    match style {
        OnboardHeaderStyle::Compact => TuiHeaderStyle::Compact,
    }
}

pub(super) fn screen_subtitle(subtitle: &str) -> Option<String> {
    let trimmed_subtitle = subtitle.trim();

    if trimmed_subtitle.is_empty() {
        return None;
    }

    Some(trimmed_subtitle.to_owned())
}
