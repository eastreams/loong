use loongclaw_app as mvp;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SetupNextActionKind {
    Ask,
    Chat,
    Channel,
    Doctor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SetupNextAction {
    pub(crate) kind: SetupNextActionKind,
    pub(crate) label: String,
    pub(crate) command: String,
    pub(crate) detail: String,
}

pub(crate) fn collect_setup_next_actions(
    config: &mvp::config::LoongClawConfig,
    config_path: &str,
) -> Vec<SetupNextAction> {
    let mut actions = Vec::new();
    if config.cli.enabled {
        actions.push(SetupNextAction {
            kind: SetupNextActionKind::Ask,
            label: "ask".to_owned(),
            command: crate::cli_handoff::format_ask_with_config(
                config_path,
                crate::cli_handoff::DEFAULT_SETUP_SMOKE_TEST_MESSAGE,
            ),
            detail: "run one quick message to verify provider, personality, and memory".to_owned(),
        });
        actions.push(SetupNextAction {
            kind: SetupNextActionKind::Chat,
            label: "chat".to_owned(),
            command: crate::cli_handoff::format_subcommand_with_config("chat", config_path),
            detail: "open the interactive CLI session after the one-shot smoke test".to_owned(),
        });
    }
    let mut channel_actions =
        crate::migration::channels::collect_channel_next_actions(config, config_path)
            .into_iter()
            .map(|action| SetupNextAction {
                kind: SetupNextActionKind::Channel,
                label: action.label.to_owned(),
                command: action.command,
                detail: "start a configured channel listener when you want non-CLI handoff"
                    .to_owned(),
            })
            .collect::<Vec<_>>();
    channel_actions.sort_by(|left, right| left.label.cmp(&right.label));
    actions.extend(channel_actions);
    if actions.is_empty() {
        actions.push(SetupNextAction {
            kind: SetupNextActionKind::Doctor,
            label: "doctor".to_owned(),
            command: crate::cli_handoff::format_subcommand_with_config("doctor", config_path),
            detail: "inspect and repair the config when no direct runtime handoff is ready"
                .to_owned(),
        });
    }
    actions
}
