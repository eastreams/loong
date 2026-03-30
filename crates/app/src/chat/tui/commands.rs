#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SlashCommand {
    Help,
    Clear,
    Model,
    ThinkOn,
    ThinkOff,
    Exit,
    Unknown(String),
}

const COMMANDS: &[(&str, &str)] = &[
    ("/help", "Show available commands"),
    ("/clear", "Clear conversation history"),
    ("/model", "Show current model info"),
    ("/think-on", "Enable thinking blocks"),
    ("/think-off", "Disable thinking blocks"),
    ("/exit", "Exit the TUI"),
];

pub(super) fn parse(input: &str) -> Option<SlashCommand> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let cmd = trimmed.split_whitespace().next().unwrap_or(trimmed);
    Some(match cmd {
        "/help" => SlashCommand::Help,
        "/clear" => SlashCommand::Clear,
        "/model" => SlashCommand::Model,
        "/think-on" => SlashCommand::ThinkOn,
        "/think-off" => SlashCommand::ThinkOff,
        "/exit" | "/quit" | "/q" => SlashCommand::Exit,
        other => SlashCommand::Unknown(other.to_string()),
    })
}

pub(super) fn completions(prefix: &str) -> Vec<(&'static str, &'static str)> {
    COMMANDS
        .iter()
        .filter(|(name, _)| name.starts_with(prefix))
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_commands() {
        assert_eq!(parse("/help"), Some(SlashCommand::Help));
        assert_eq!(parse("/clear"), Some(SlashCommand::Clear));
        assert_eq!(parse("/model"), Some(SlashCommand::Model));
        assert_eq!(parse("/think-on"), Some(SlashCommand::ThinkOn));
        assert_eq!(parse("/think-off"), Some(SlashCommand::ThinkOff));
        assert_eq!(parse("/exit"), Some(SlashCommand::Exit));
        assert_eq!(parse("/quit"), Some(SlashCommand::Exit));
        assert_eq!(parse("/q"), Some(SlashCommand::Exit));
    }

    #[test]
    fn parse_unknown_command() {
        assert_eq!(
            parse("/foobar"),
            Some(SlashCommand::Unknown("/foobar".to_string()))
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
        assert_eq!(parse("/help me"), Some(SlashCommand::Help));
    }

    #[test]
    fn completions_filter() {
        let results = completions("/th");
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|(name, _)| *name == "/think-on"));
        assert!(results.iter().any(|(name, _)| *name == "/think-off"));
    }

    #[test]
    fn completions_empty_prefix() {
        let results = completions("/");
        assert_eq!(results.len(), COMMANDS.len());
    }

    #[test]
    fn completions_no_match() {
        let results = completions("/zzz");
        assert!(results.is_empty());
    }
}
