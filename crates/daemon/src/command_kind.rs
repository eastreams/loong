use crate::Commands;

impl Commands {
    pub fn command_kind_for_logging(&self) -> &'static str {
        match self {
            Self::Welcome => "welcome",
            Self::Demo => "demo",
            Self::Update => "update",
            Self::RunTask { .. } => "run_task",
            Self::Turn { command } => match command {
                crate::TurnCommands::Run { .. } => "turn_run",
            },
            Self::InvokeConnector { .. } => "invoke_connector",
            Self::AuditDemo => "audit_demo",
            Self::Web { .. } => "web",
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
            Self::Debug { .. } => "debug",
            Self::Audit { .. } => "audit",
            Self::Skills { .. } => "skills",
            Self::Status { .. } => "status",
            Self::Tasks { .. } => "tasks",
            Self::DelegateChildRun { .. } => "delegate_child_run",
            Self::Sessions { .. } => "sessions",
            Self::Plugins { .. } => "plugins",
            Self::Channels { .. } => "channels",
            Self::Runtime { .. } => "runtime",
            Self::Ask { .. } => "ask",
            Self::Chat { .. } => "chat",
            Self::Gateway { .. } => "gateway",
            Self::Feishu { .. } => "feishu",
            Self::Completions { .. } => "completions",
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Commands;
    #[test]
    fn command_kind_for_logging_uses_stable_variant_names() {
        assert_eq!(Commands::Welcome.command_kind_for_logging(), "welcome");
        assert_eq!(Commands::AuditDemo.command_kind_for_logging(), "audit_demo");
        assert_eq!(
            Commands::Web {
                command: crate::WebCommand::Status,
            }
            .command_kind_for_logging(),
            "web"
        );
        assert_eq!(Commands::Update.command_kind_for_logging(), "update");
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
            Commands::Debug {
                config: None,
                json: false,
                session: "default".to_owned(),
                command: crate::debug_cli::DebugCommands::Bundle {
                    session_id: None,
                    output: None,
                    audit_limit: 10,
                    session_event_limit: 10,
                    history_limit: 10,
                    acp_event_limit: 50,
                    include_history: false,
                },
            }
            .command_kind_for_logging(),
            "debug"
        );
        assert_eq!(
            Commands::Runtime {
                command: crate::runtime_cli::RuntimeCommands::Snapshot(
                    crate::runtime_cli::RuntimeSnapshotArgs {
                        config: None,
                        json: false,
                        output: None,
                        label: None,
                        experiment_id: None,
                        parent_snapshot_id: None,
                    }
                )
            }
            .command_kind_for_logging(),
            "runtime"
        );
    }
}
