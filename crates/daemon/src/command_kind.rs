use crate::Commands;

impl Commands {
    pub fn command_kind_for_logging(&self) -> &'static str {
        match self {
            Self::Welcome => "welcome",
            Self::Demo => "demo",
            Self::Turn { command } => match command {
                crate::TurnCommands::Run { .. } => "turn_run",
            },
            Self::InvokeConnector { .. } => "invoke_connector",
            Self::AuditDemo => "audit_demo",
            Self::InitSpec { .. } => "init_spec",
            Self::RunSpec { .. } => "run_spec",
            Self::BenchmarkProgrammaticPressure { .. } => "benchmark_programmatic_pressure",
            Self::BenchmarkProgrammaticPressureLint { .. } => {
                "benchmark_programmatic_pressure_lint"
            }
            Self::BenchmarkWasmCache { .. } => "benchmark_wasm_cache",
            Self::BenchmarkMemoryContext { .. } => "benchmark_memory_context",
            Self::ValidateConfig { .. } => "validate_config",
            Self::Onboard { .. } => "onboard",
            Self::Personalize { .. } => "personalize",
            Self::Import { .. } => "import",
            Self::Migrate { .. } => "migrate",
            Self::Doctor { .. } => "doctor",
            Self::Audit { .. } => "audit",
            Self::Skills { .. } => "skills",
            Self::Status { .. } => "status",
            Self::Tasks { .. } => "tasks",
            Self::DelegateChildRun { .. } => "delegate_child_run",
            Self::Sessions { .. } => "sessions",
            Self::Plugins { .. } => "plugins",
            Self::Channels { .. } => "channels",
            Self::ListModels { .. } => "list_models",
            Self::RuntimeSnapshot { .. } => "runtime_snapshot",
            Self::RuntimeRestore { .. } => "runtime_restore",
            Self::RuntimeExperiment { .. } => "runtime_experiment",
            Self::RuntimeCapability { .. } => "runtime_capability",
            Self::ListContextEngines { .. } => "list_context_engines",
            Self::ListMemorySystems { .. } => "list_memory_systems",
            Self::ListMcpServers { .. } => "list_mcp_servers",
            Self::ShowMcpServer { .. } => "show_mcp_server",
            Self::ListAcpBackends { .. } => "list_acp_backends",
            Self::ListAcpSessions { .. } => "list_acp_sessions",
            Self::AcpStatus { .. } => "acp_status",
            Self::AcpObservability { .. } => "acp_observability",
            Self::AcpEventSummary { .. } => "acp_event_summary",
            Self::AcpDispatch { .. } => "acp_dispatch",
            Self::AcpDoctor { .. } => "acp_doctor",
            Self::ControlPlaneServe { .. } => "control_plane_serve",
            Self::Ask { .. } => "ask",
            Self::Chat { .. } => "chat",
            Self::SafeLaneSummary { .. } => "safe_lane_summary",
            Self::SessionSearch { .. } => "session_search",
            Self::SessionSearchInspect { .. } => "session_search_inspect",
            Self::TrajectoryExport { .. } => "trajectory_export",
            Self::TrajectoryInspect { .. } => "trajectory_inspect",
            Self::RuntimeTrajectory { .. } => "runtime_trajectory",
            Self::TelegramSend { .. } => "telegram_send",
            Self::TelegramServe { .. } => "telegram_serve",
            Self::FeishuSend { .. } => "feishu_send",
            Self::FeishuServe { .. } => "feishu_serve",
            Self::MatrixSend { .. } => "matrix_send",
            Self::MatrixServe { .. } => "matrix_serve",
            Self::WecomSend { .. } => "wecom_send",
            Self::WecomServe { .. } => "wecom_serve",
            Self::WhatsappServe { .. } => "whatsapp_serve",
            Self::DiscordSend { .. } => "discord_send",
            Self::DingtalkSend { .. } => "dingtalk_send",
            Self::SlackSend { .. } => "slack_send",
            Self::LineSend { .. } => "line_send",
            Self::WhatsappSend { .. } => "whatsapp_send",
            Self::EmailSend { .. } => "email_send",
            Self::WebhookSend { .. } => "webhook_send",
            Self::GoogleChatSend { .. } => "google_chat_send",
            Self::TeamsSend { .. } => "teams_send",
            Self::TlonSend { .. } => "tlon_send",
            Self::SignalSend { .. } => "signal_send",
            Self::TwitchSend { .. } => "twitch_send",
            Self::MattermostSend { .. } => "mattermost_send",
            Self::NextcloudTalkSend { .. } => "nextcloud_talk_send",
            Self::SynologyChatSend { .. } => "synology_chat_send",
            Self::IrcSend { .. } => "irc_send",
            Self::ImessageSend { .. } => "imessage_send",
            Self::NostrSend { .. } => "nostr_send",
            Self::MultiChannelServe { .. } => "multi_channel_serve",
            Self::Gateway { .. } => "gateway",
            Self::Feishu { .. } => "feishu",
            Self::Completions { .. } => "completions",
            Self::WorkUnit { .. } => "work_unit",
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Commands;
    use crate::work_unit_cli::{
        WorkUnitCommands, WorkUnitHealthCommandOptions, WorkUnitStatusArg,
        WorkUnitUpdateCommandOptions,
    };

    #[test]
    fn command_kind_for_logging_uses_stable_variant_names() {
        assert_eq!(Commands::Welcome.command_kind_for_logging(), "welcome");
        assert_eq!(Commands::AuditDemo.command_kind_for_logging(), "audit_demo");
        assert_eq!(
            Commands::ValidateConfig {
                config: None,
                output: None,
                locale: "en".to_owned(),
                json: false,
                fail_on_diagnostics: false,
            }
            .command_kind_for_logging(),
            "validate_config"
        );
        assert_eq!(
            Commands::Status {
                config: None,
                json: false,
            }
            .command_kind_for_logging(),
            "status"
        );
        assert_eq!(
            Commands::WorkUnit {
                command: WorkUnitCommands::Update(WorkUnitUpdateCommandOptions {
                    config: None,
                    id: "wu-demo".to_owned(),
                    title: None,
                    description: None,
                    status: Some(WorkUnitStatusArg::Ready),
                    priority: None,
                    next_run_at_ms: None,
                    blocking_reason: None,
                    clear_blocking_reason: false,
                    actor: None,
                    now_ms: None,
                    json: false,
                }),
            }
            .command_kind_for_logging(),
            "work_unit"
        );
        assert_eq!(
            Commands::WorkUnit {
                command: WorkUnitCommands::Health(WorkUnitHealthCommandOptions {
                    config: None,
                    now_ms: None,
                    json: false,
                }),
            }
            .command_kind_for_logging(),
            "work_unit"
        );
        assert_eq!(
            Commands::RuntimeTrajectory {
                command: crate::runtime_trajectory_cli::RuntimeTrajectoryCommands::Show(
                    crate::runtime_trajectory_cli::RuntimeTrajectoryShowCommandOptions {
                        artifact: "artifact.json".to_owned(),
                        json: false,
                    },
                ),
            }
            .command_kind_for_logging(),
            "runtime_trajectory"
        );
    }
}
