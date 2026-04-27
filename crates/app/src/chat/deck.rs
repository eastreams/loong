use crate::tui_surface::TuiCalloutTone;
use crate::tui_surface::TuiKeyValueSpec;
use crate::tui_surface::TuiMessageSpec;
use crate::tui_surface::TuiSectionSpec;

use super::CLI_CHAT_COMPACT_COMMAND;
use super::CLI_CHAT_HELP_COMMAND;
use super::CLI_CHAT_HISTORY_COMMAND;
use super::CLI_CHAT_MISSION_COMMAND;
use super::CLI_CHAT_REVIEW_COMMAND;
use super::CLI_CHAT_SESSIONS_COMMAND;
use super::CLI_CHAT_STATUS_COMMAND;
use super::CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND;
use super::CLI_CHAT_WORKERS_COMMAND;
use super::detect_cli_chat_render_width;
use super::print_rendered_cli_chat_lines;
use super::render_cli_chat_message_spec_with_width;

pub(super) fn render_cli_chat_help_lines_with_width(width: usize) -> Vec<String> {
    let message_spec = build_cli_chat_help_message_spec();
    render_cli_chat_message_spec_with_width(&message_spec, width)
}

#[allow(clippy::print_stdout)] // CLI output
pub(super) fn print_help() {
    let render_width = detect_cli_chat_render_width();
    let rendered_lines = render_cli_chat_help_lines_with_width(render_width);
    print_rendered_cli_chat_lines(&rendered_lines);
}

fn build_cli_chat_help_message_spec() -> TuiMessageSpec {
    let command_items = vec![
        TuiKeyValueSpec::Plain {
            key: CLI_CHAT_HELP_COMMAND.to_owned(),
            value: "show chat commands".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: CLI_CHAT_COMPACT_COMMAND.to_owned(),
            value: "write a continuity-safe checkpoint into the active window".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: CLI_CHAT_STATUS_COMMAND.to_owned(),
            value: "show session, runtime, compaction, and durability status".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: CLI_CHAT_HISTORY_COMMAND.to_owned(),
            value: "print the current session sliding window".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: CLI_CHAT_SESSIONS_COMMAND.to_owned(),
            value: "inspect visible sessions rooted at the current session scope".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: CLI_CHAT_MISSION_COMMAND.to_owned(),
            value: "open the orchestration/mission control overview for the current session scope"
                .to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: CLI_CHAT_REVIEW_COMMAND.to_owned(),
            value: "reopen the latest approval/review summary in the current session".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: CLI_CHAT_WORKERS_COMMAND.to_owned(),
            value: "inspect visible worker/delegate sessions from the current session scope"
                .to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/fast_lane_summary [limit]".to_owned(),
            value: "summarize fast-lane batch execution events".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/safe_lane_summary [limit]".to_owned(),
            value: "summarize safe-lane runtime events".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/turn_checkpoint_summary [limit]".to_owned(),
            value: "summarize durable turn finalization state".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: CLI_CHAT_TURN_CHECKPOINT_REPAIR_COMMAND.to_owned(),
            value: "repair durable turn finalization tail when safe".to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "$skill-name <request>".to_owned(),
            value: "explicitly activate a visible external skill before handling the request"
                .to_owned(),
        },
        TuiKeyValueSpec::Plain {
            key: "/exit".to_owned(),
            value: "quit chat".to_owned(),
        },
    ];
    let command_menu_section = TuiSectionSpec::Callout {
        tone: TuiCalloutTone::Info,
        title: Some("surface controls".to_owned()),
        lines: vec![
            "Use : from an empty composer to open the command menu for session-level actions."
                .to_owned(),
            "Use Esc to clear draft input first, then Esc again to leave the surface.".to_owned(),
            "Use the control deck and timeline overlay when you need transcript navigation instead of a new turn.".to_owned(),
        ],
    };
    let keyboard_section = TuiSectionSpec::Narrative {
        title: Some("keyboard".to_owned()),
        lines: vec![
            "Enter sends the current draft. A trailing \\ keeps composing on a new line."
                .to_owned(),
            "Tab cycles transcript / composer / control deck focus. [ and ] switch control-deck tabs."
                .to_owned(),
            "PgUp / PgDn scroll the transcript, j / k move selected entries, and t opens the transcript timeline."
                .to_owned(),
            "M opens the mission-control overview for the current session scope.".to_owned(),
            "r reopens the latest approval screen when one is available.".to_owned(),
            "S opens the visible session queue when related sessions are available.".to_owned(),
            "W opens the worker queue when delegate sessions are visible.".to_owned(),
        ],
    };
    let usage_section = TuiSectionSpec::Callout {
        tone: TuiCalloutTone::Info,
        title: Some("usage notes".to_owned()),
        lines: vec![
            "Type any non-command text to send a normal assistant turn.".to_owned(),
            "Use /status to inspect runtime maintenance settings without sending a turn."
                .to_owned(),
            "Use /mission to inspect orchestration posture before spawning or reviewing lanes."
                .to_owned(),
            "Use /history to inspect the active memory window when a reply feels off.".to_owned(),
            "Use /compact to checkpoint the active session before the next turn.".to_owned(),
            "Prefix a prompt with $skill-name to force explicit activation of a visible external skill."
                .to_owned(),
        ],
    };
    let command_section = TuiSectionSpec::KeyValues {
        title: Some("slash commands".to_owned()),
        items: command_items,
    };

    TuiMessageSpec {
        role: "help".to_owned(),
        caption: Some("operator deck".to_owned()),
        sections: vec![
            command_section,
            command_menu_section,
            keyboard_section,
            usage_section,
        ],
        footer_lines: vec![
            "Send normal text to continue the transcript.".to_owned(),
            "Use /exit to leave chat, or Esc from an empty composer to confirm exit.".to_owned(),
        ],
    }
}
