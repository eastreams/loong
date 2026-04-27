use crate::CliResult;
use crate::tui_surface::TuiActionSpec;
use crate::tui_surface::TuiCalloutTone;
use crate::tui_surface::TuiHeaderStyle;
use crate::tui_surface::TuiScreenSpec;
use crate::tui_surface::TuiSectionSpec;
use crate::tui_surface::render_tui_screen_spec;

use super::CliChatOptions;
use super::CliTurnRuntime;
use super::DEFAULT_FIRST_PROMPT;
use super::detect_cli_chat_render_width;
use super::startup_state::CliChatStartupSummary;
use super::startup_state::build_cli_chat_startup_summary;
use super::status_view::build_cli_chat_runtime_sections;

const PRIMARY_QUICK_COMMANDS_HINT: &str =
    "Start with a first answer, then keep moving with /help · /status · /history · /compact.";
const TRANSCRIPT_START_HINT: &str =
    "Type any request to start the transcript, or use the command deck before your first turn.";

pub(super) fn render_cli_chat_startup_lines_with_width(
    summary: &CliChatStartupSummary,
    width: usize,
) -> Vec<String> {
    let screen_spec = build_cli_chat_startup_screen_spec(summary);
    render_tui_screen_spec(&screen_spec, width, false)
}

#[allow(clippy::print_stdout)] // CLI output
pub(super) fn print_cli_chat_startup(
    runtime: &CliTurnRuntime,
    options: &CliChatOptions,
) -> CliResult<()> {
    let summary = build_cli_chat_startup_summary(runtime, options)?;
    let render_width = detect_cli_chat_render_width();
    let lines = render_cli_chat_startup_lines_with_width(&summary, render_width);
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

pub(super) fn build_cli_chat_startup_screen_spec(summary: &CliChatStartupSummary) -> TuiScreenSpec {
    let first_prompt_action = TuiActionSpec {
        label: "first answer".to_owned(),
        command: DEFAULT_FIRST_PROMPT.to_owned(),
    };
    let start_here_section = TuiSectionSpec::ActionGroup {
        title: Some("start here".to_owned()),
        inline_title_when_wide: true,
        items: vec![first_prompt_action],
    };
    let command_deck_section = TuiSectionSpec::ActionGroup {
        title: Some("command deck".to_owned()),
        inline_title_when_wide: false,
        items: vec![
            TuiActionSpec {
                label: "slash commands".to_owned(),
                command: "/help".to_owned(),
            },
            TuiActionSpec {
                label: "runtime status".to_owned(),
                command: "/status".to_owned(),
            },
            TuiActionSpec {
                label: "recent window".to_owned(),
                command: "/history".to_owned(),
            },
            TuiActionSpec {
                label: "manual checkpoint".to_owned(),
                command: "/compact".to_owned(),
            },
        ],
    };
    let narrative_section = TuiSectionSpec::Callout {
        tone: TuiCalloutTone::Info,
        title: Some("how this surface works".to_owned()),
        lines: vec![
            "Type your request in the composer to run the next assistant turn.".to_owned(),
            "Use the control deck for runtime posture, tool activity, and shortcuts.".to_owned(),
            "Use the command menu and help surfaces before you need lower-level runtime detail."
                .to_owned(),
        ],
    };
    let runtime_sections = build_cli_chat_runtime_sections(summary);
    let mut sections = vec![start_here_section, command_deck_section, narrative_section];
    sections.extend(runtime_sections);

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some("interactive chat".to_owned()),
        title: Some("operator cockpit ready".to_owned()),
        progress_line: None,
        intro_lines: Vec::new(),
        sections,
        choices: Vec::new(),
        footer_lines: vec![
            PRIMARY_QUICK_COMMANDS_HINT.to_owned(),
            TRANSCRIPT_START_HINT.to_owned(),
        ],
    }
}
