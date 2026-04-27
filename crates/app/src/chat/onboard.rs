use crate::config;
use crate::tui_surface::TuiActionSpec;
use crate::tui_surface::TuiCalloutTone;
use crate::tui_surface::TuiChoiceSpec;
use crate::tui_surface::TuiHeaderStyle;
use crate::tui_surface::TuiMessageSpec;
use crate::tui_surface::TuiScreenSpec;
use crate::tui_surface::TuiSectionSpec;
use crate::tui_surface::render_tui_screen_spec;

use super::render_cli_chat_message_spec_with_width;

pub(super) fn should_run_missing_config_onboard(read: usize, input: &str) -> bool {
    if read == 0 {
        return false;
    }

    let normalized_input = input.trim().to_ascii_lowercase();

    if normalized_input.is_empty() {
        return true;
    }

    matches!(normalized_input.as_str(), "y" | "yes")
}

pub(super) fn render_cli_chat_missing_config_lines_with_width(
    onboard_hint: &str,
    width: usize,
) -> Vec<String> {
    let screen_spec = build_cli_chat_missing_config_screen_spec(onboard_hint);
    render_tui_screen_spec(&screen_spec, width, false)
}

fn build_cli_chat_missing_config_screen_spec(onboard_hint: &str) -> TuiScreenSpec {
    let intro_lines = vec![
        format!("Welcome to {}!", config::PRODUCT_DISPLAY_NAME),
        "No configuration found for interactive chat.".to_owned(),
    ];
    let sections = vec![TuiSectionSpec::ActionGroup {
        title: Some("setup command".to_owned()),
        inline_title_when_wide: true,
        items: vec![TuiActionSpec {
            label: "start setup".to_owned(),
            command: onboard_hint.to_owned(),
        }],
    }];
    let choices = vec![
        TuiChoiceSpec {
            key: "y".to_owned(),
            label: "run setup wizard".to_owned(),
            detail_lines: vec!["Create a config now and return to interactive chat.".to_owned()],
            recommended: true,
        },
        TuiChoiceSpec {
            key: "n".to_owned(),
            label: "skip for now".to_owned(),
            detail_lines: vec!["Exit chat now and keep the setup command for later.".to_owned()],
            recommended: false,
        },
    ];
    let footer_lines = vec!["Press Enter to accept y.".to_owned()];

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some("interactive chat".to_owned()),
        title: Some("setup required".to_owned()),
        progress_line: None,
        intro_lines,
        sections,
        choices,
        footer_lines,
    }
}

pub(super) fn render_cli_chat_missing_config_decline_lines_with_width(
    onboard_hint: &str,
    width: usize,
) -> Vec<String> {
    let message_spec = build_cli_chat_missing_config_decline_message_spec(onboard_hint);
    render_cli_chat_message_spec_with_width(&message_spec, width)
}

fn build_cli_chat_missing_config_decline_message_spec(onboard_hint: &str) -> TuiMessageSpec {
    let setup_hint = format!("You can run '{onboard_hint}' later to get started.");
    let sections = vec![
        TuiSectionSpec::Callout {
            tone: TuiCalloutTone::Info,
            title: Some("setup skipped".to_owned()),
            lines: vec![setup_hint],
        },
        TuiSectionSpec::ActionGroup {
            title: Some("start later".to_owned()),
            inline_title_when_wide: true,
            items: vec![TuiActionSpec {
                label: "setup command".to_owned(),
                command: onboard_hint.to_owned(),
            }],
        },
    ];

    TuiMessageSpec {
        role: "chat".to_owned(),
        caption: Some("setup required".to_owned()),
        sections,
        footer_lines: vec!["Run setup now to unlock the full chat surface.".to_owned()],
    }
}
