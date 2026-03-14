use loongclaw_app as mvp;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SetupNextActionKind {
    Chat,
    Channel,
    Doctor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SetupNextAction {
    pub(crate) kind: SetupNextActionKind,
    pub(crate) label: String,
    pub(crate) command: String,
}

pub(crate) fn collect_setup_next_actions(
    config: &mvp::config::LoongClawConfig,
    config_path: &str,
) -> Vec<SetupNextAction> {
    let mut actions = Vec::new();
    if config.cli.enabled {
        actions.push(SetupNextAction {
            kind: SetupNextActionKind::Chat,
            label: "chat".to_owned(),
            command: format!(
                "{} chat --config {}",
                mvp::config::CLI_COMMAND_NAME,
                config_path
            ),
        });
    }
    actions.extend(
        crate::migration::channels::collect_channel_next_actions(config, config_path)
            .into_iter()
            .map(|action| SetupNextAction {
                kind: SetupNextActionKind::Channel,
                label: action.label.to_owned(),
                command: action.command,
            }),
    );
    if actions.is_empty() {
        actions.push(SetupNextAction {
            kind: SetupNextActionKind::Doctor,
            label: "doctor".to_owned(),
            command: format!(
                "{} doctor --config {}",
                mvp::config::CLI_COMMAND_NAME,
                config_path
            ),
        });
    }
    actions
}
