#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SlashCommand {
    Help,
    Commands,
    Clear,
    Compact,
    Resume,
    Tasks,
    Approvals,
    Permissions,
    Export,
    Diff,
    Model,
    Mode,
    Stats,
    Session,
    Status,
    Context,
    Skills,
    Review,
    Tools,
    Thinking,
    Latest,
    Top,
    Copy,
    Exit,
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SlashCommandSpec {
    pub(super) name: &'static str,
    pub(super) help: &'static str,
    pub(super) category: &'static str,
    pub(super) aliases: &'static [&'static str],
    pub(super) argument_hint: Option<&'static str>,
    pub(super) discoverable: bool,
}

const COMMANDS: &[SlashCommandSpec] = &[
    SlashCommandSpec {
        name: "/help",
        help: "Toggle the help overlay",
        category: "General",
        aliases: &[],
        argument_hint: None,
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/commands",
        help: "Append the command catalog to the transcript",
        category: "General",
        aliases: &[],
        argument_hint: None,
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/clear",
        help: "Clear conversation history",
        category: "General",
        aliases: &[],
        argument_hint: None,
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/compact",
        help: "Compact the current session context",
        category: "General",
        aliases: &[],
        argument_hint: None,
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/resume",
        help: "List visible sessions, inspect one, or switch the active session",
        category: "Navigation",
        aliases: &[],
        argument_hint: Some("[inspect|switch] <session-id>"),
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/tasks",
        help: "Show delegate task sessions, or inspect one task session",
        category: "Status",
        aliases: &[],
        argument_hint: Some("[all|overdue|queued|running|failed|completed|timed_out|session-id]"),
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/approvals",
        help: "List approval requests, inspect one, or resolve it",
        category: "Status",
        aliases: &[],
        argument_hint: Some("[pending|attention|all|<id>|resolve <id> <decision> [mode]]"),
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/permissions",
        help: "Show effective tool permissions for the current or target session",
        category: "Status",
        aliases: &[],
        argument_hint: Some("[session-id]"),
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/export",
        help: "Export the latest output or full transcript to a file",
        category: "General",
        aliases: &[],
        argument_hint: Some("[latest|transcript] [path]"),
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/diff",
        help: "Show current working tree changes",
        category: "Status",
        aliases: &[],
        argument_hint: Some("[status|full]"),
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/exit",
        help: "Exit the TUI",
        category: "General",
        aliases: &["/quit", "/q"],
        argument_hint: None,
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/model",
        help: "Show current model details, switch model, and set reasoning effort",
        category: "Status",
        aliases: &[],
        argument_hint: Some("[selector] [auto|none|minimal|low|medium|high|xhigh]"),
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/mode",
        help: "Show or set busy-turn submit behavior (queue or steer)",
        category: "View",
        aliases: &[],
        argument_hint: Some("[queue|steer|toggle]"),
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/stats",
        help: "Open the usage and model statistics overlay",
        category: "Status",
        aliases: &["/usage"],
        argument_hint: Some("[overview|models] [all|7d|30d]"),
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/session",
        help: "Append current session details to the transcript",
        category: "Status",
        aliases: &[],
        argument_hint: None,
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/status",
        help: "Append runtime status and token usage to the transcript",
        category: "Status",
        aliases: &[],
        argument_hint: None,
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/context",
        help: "Append context budget and token usage details",
        category: "Status",
        aliases: &[],
        argument_hint: None,
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/skills",
        help: "Show available external skills, or inspect one skill",
        category: "Status",
        aliases: &[],
        argument_hint: Some("[skill-id]"),
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/review",
        help: "Toggle transcript review mode",
        category: "Navigation",
        aliases: &[],
        argument_hint: None,
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/tools",
        help: "Show tool activity, or `/tools open` for latest tool details",
        category: "Status",
        aliases: &[],
        argument_hint: Some("[open]"),
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/thinking",
        help: "Show, hide, or toggle thinking blocks",
        category: "View",
        aliases: &["/think"],
        argument_hint: Some("[on|off|toggle]"),
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/latest",
        help: "Jump transcript to latest output",
        category: "Navigation",
        aliases: &[],
        argument_hint: None,
        discoverable: false,
    },
    SlashCommandSpec {
        name: "/top",
        help: "Jump transcript to oldest visible output",
        category: "Navigation",
        aliases: &[],
        argument_hint: None,
        discoverable: false,
    },
    SlashCommandSpec {
        name: "/copy",
        help: "Copy the latest output, current selection, or full transcript",
        category: "Navigation",
        aliases: &[],
        argument_hint: Some("[latest|selection|transcript]"),
        discoverable: true,
    },
    SlashCommandSpec {
        name: "/think-on",
        help: "Show thinking blocks",
        category: "View",
        aliases: &[],
        argument_hint: None,
        discoverable: false,
    },
    SlashCommandSpec {
        name: "/think-off",
        help: "Hide thinking blocks",
        category: "View",
        aliases: &[],
        argument_hint: None,
        discoverable: false,
    },
];

pub(super) fn command_specs() -> &'static [SlashCommandSpec] {
    COMMANDS
}

pub(super) fn discoverable_command_specs() -> impl Iterator<Item = &'static SlashCommandSpec> {
    COMMANDS.iter().filter(|spec| spec.discoverable)
}

#[cfg(test)]
pub(super) fn grouped_command_specs() -> Vec<(&'static str, Vec<&'static SlashCommandSpec>)> {
    let mut groups: Vec<(&'static str, Vec<&'static SlashCommandSpec>)> = Vec::new();

    for spec in discoverable_command_specs() {
        if let Some((_, entries)) = groups
            .iter_mut()
            .find(|(category, _)| *category == spec.category)
        {
            entries.push(spec);
            continue;
        }

        groups.push((spec.category, vec![spec]));
    }

    groups
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParsedSlashCommand {
    pub(super) command: SlashCommand,
    pub(super) args: String,
}

pub(super) fn parse(input: &str) -> Option<ParsedSlashCommand> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let command_end = trimmed
        .char_indices()
        .find_map(|(index, ch)| (index > 0 && (ch.is_whitespace() || ch == ':')).then_some(index))
        .unwrap_or(trimmed.len());
    let cmd = trimmed.get(..command_end).unwrap_or(trimmed);
    let trailing = trimmed.get(command_end..).unwrap_or_default().trim();
    let args = trailing
        .strip_prefix(':')
        .unwrap_or(trailing)
        .trim()
        .to_owned();
    let command = match cmd {
        "/help" => SlashCommand::Help,
        "/commands" => SlashCommand::Commands,
        "/clear" => SlashCommand::Clear,
        "/compact" => SlashCommand::Compact,
        "/resume" => SlashCommand::Resume,
        "/tasks" => SlashCommand::Tasks,
        "/approvals" => SlashCommand::Approvals,
        "/permissions" => SlashCommand::Permissions,
        "/export" => SlashCommand::Export,
        "/diff" => SlashCommand::Diff,
        "/model" => SlashCommand::Model,
        "/mode" => SlashCommand::Mode,
        "/stats" | "/usage" => SlashCommand::Stats,
        "/session" => SlashCommand::Session,
        "/status" => SlashCommand::Status,
        "/context" => SlashCommand::Context,
        "/skills" => SlashCommand::Skills,
        "/review" => SlashCommand::Review,
        "/tools" => SlashCommand::Tools,
        "/thinking" | "/think" => SlashCommand::Thinking,
        "/latest" => SlashCommand::Latest,
        "/top" => SlashCommand::Top,
        "/copy" => SlashCommand::Copy,
        "/think-on" => SlashCommand::Thinking,
        "/think-off" => SlashCommand::Thinking,
        "/exit" | "/quit" | "/q" => SlashCommand::Exit,
        other => SlashCommand::Unknown(other.to_string()),
    };
    Some(ParsedSlashCommand { command, args })
}

pub(super) fn completions(prefix: &str) -> Vec<&'static SlashCommandSpec> {
    discoverable_command_specs()
        .filter(|spec| {
            spec.name.starts_with(prefix)
                || spec.aliases.iter().any(|alias| alias.starts_with(prefix))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_commands() {
        assert_eq!(
            parse("/help"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Help,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/commands"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Commands,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/clear"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Clear,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/compact"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Compact,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/resume"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Resume,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/tasks"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Tasks,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/approvals"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Approvals,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/permissions"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Permissions,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/export"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Export,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/diff"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Diff,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/model"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Model,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/mode"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Mode,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/stats"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Stats,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/usage"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Stats,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/session"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Session,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/status"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Status,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/context"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Context,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/skills"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Skills,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/review"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Review,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/tools"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Tools,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/thinking"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Thinking,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/latest"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Latest,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/top"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Top,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/copy"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Copy,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/think-on"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Thinking,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/think-off"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Thinking,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/exit"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Exit,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/quit"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Exit,
                args: String::new(),
            })
        );
        assert_eq!(
            parse("/q"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Exit,
                args: String::new(),
            })
        );
    }

    #[test]
    fn parse_unknown_command() {
        assert_eq!(
            parse("/foobar"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Unknown("/foobar".to_string()),
                args: String::new(),
            })
        );
    }

    #[test]
    fn parse_non_command_returns_none() {
        assert_eq!(parse("hello world"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("  "), None);
    }

    #[test]
    fn parse_with_trailing_args() {
        assert_eq!(
            parse("/help me"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Help,
                args: "me".to_owned(),
            })
        );
    }

    #[test]
    fn parse_with_colon_delimited_args() {
        assert_eq!(
            parse("/export: notes.txt"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Export,
                args: "notes.txt".to_owned(),
            })
        );
        assert_eq!(
            parse("/diff: full"),
            Some(ParsedSlashCommand {
                command: SlashCommand::Diff,
                args: "full".to_owned(),
            })
        );
    }

    #[test]
    fn completions_filter() {
        let results = completions("/th");
        assert_eq!(results.len(), 1);
        assert!(results.iter().any(|spec| spec.name == "/thinking"));
    }

    #[test]
    fn completions_include_review_command() {
        let results = completions("/re");

        assert!(
            results.iter().any(|spec| spec.name == "/review"),
            "review command should be discoverable via completion"
        );
    }

    #[test]
    fn completions_empty_prefix() {
        let results = completions("/");
        assert_eq!(
            results.len(),
            COMMANDS.iter().filter(|spec| spec.discoverable).count()
        );
    }

    #[test]
    fn completions_no_match() {
        let results = completions("/zzz");
        assert!(results.is_empty());
    }

    #[test]
    fn grouped_specs_include_status_and_navigation_sections() {
        let groups = grouped_command_specs();

        assert!(groups.iter().any(|(name, _)| *name == "Status"));
        assert!(groups.iter().any(|(name, _)| *name == "Navigation"));
    }
}
