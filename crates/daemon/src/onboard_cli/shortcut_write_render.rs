use super::*;

pub fn render_continue_current_setup_screen_lines(
    config: &mvp::config::LoongConfig,
    width: usize,
) -> Vec<String> {
    render_onboard_shortcut_screen_lines_with_style(
        OnboardShortcutKind::CurrentSetup,
        config,
        None,
        width,
        false,
    )
}

pub fn render_continue_detected_setup_screen_lines(
    config: &mvp::config::LoongConfig,
    import_source: &str,
    width: usize,
) -> Vec<String> {
    render_onboard_shortcut_screen_lines_with_style(
        OnboardShortcutKind::DetectedSetup,
        config,
        Some(import_source),
        width,
        false,
    )
}

pub(super) fn render_onboard_shortcut_screen_lines_with_style(
    shortcut_kind: OnboardShortcutKind,
    config: &mvp::config::LoongConfig,
    import_source: Option<&str>,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboard_shortcut_screen_spec(shortcut_kind, config, import_source, true);
    render_onboard_screen_spec(&spec, width, color_enabled)
}

pub(super) fn render_onboard_shortcut_header_lines_with_style(
    shortcut_kind: OnboardShortcutKind,
    config: &mvp::config::LoongConfig,
    import_source: Option<&str>,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboard_shortcut_screen_spec(shortcut_kind, config, import_source, false);
    render_onboard_screen_spec(&spec, width, color_enabled)
}

pub(super) fn render_shortcut_default_choice_footer_line(
    shortcut_kind: OnboardShortcutKind,
) -> String {
    render_default_choice_footer_line("1", shortcut_kind.default_choice_description())
}

pub fn render_onboarding_risk_screen_lines(width: usize) -> Vec<String> {
    render_onboarding_risk_screen_lines_with_style(width, false)
}

pub fn render_write_confirmation_screen_lines(
    config_path: &str,
    warnings_kept: bool,
    width: usize,
) -> Vec<String> {
    render_write_confirmation_screen_lines_with_style(
        config_path,
        warnings_kept,
        width,
        ReviewFlowStyle::Guided(GuidedPromptPath::NativePromptPack),
        false,
    )
}

pub fn render_current_setup_write_confirmation_screen_lines(
    config_path: &str,
    warnings_kept: bool,
    width: usize,
) -> Vec<String> {
    render_write_confirmation_screen_lines_with_style(
        config_path,
        warnings_kept,
        width,
        ReviewFlowStyle::QuickCurrentSetup,
        false,
    )
}

pub fn render_detected_setup_write_confirmation_screen_lines(
    config_path: &str,
    warnings_kept: bool,
    width: usize,
) -> Vec<String> {
    render_write_confirmation_screen_lines_with_style(
        config_path,
        warnings_kept,
        width,
        ReviewFlowStyle::QuickDetectedSetup,
        false,
    )
}

pub fn render_existing_config_write_screen_lines(config_path: &str, width: usize) -> Vec<String> {
    render_existing_config_write_screen_lines_with_style(config_path, width, false)
}

fn render_existing_config_write_screen_lines_with_style(
    config_path: &str,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "decide how to write the config",
        "existing config found",
        None,
        vec![
            format!("- config: {config_path}"),
            "- choose whether to replace it, keep a backup, or cancel".to_owned(),
        ],
        build_existing_config_write_screen_options(),
        vec![render_default_choice_footer_line(
            "b",
            "create backup and replace",
        )],
        true,
        color_enabled,
    )
}

pub(super) fn render_existing_config_write_header_lines_with_style(
    config_path: &str,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "decide how to write the config",
        "existing config found",
        None,
        vec![
            format!("- config: {config_path}"),
            "- choose whether to replace it, keep a backup, or cancel".to_owned(),
        ],
        Vec::new(),
        Vec::new(),
        true,
        color_enabled,
    )
}
