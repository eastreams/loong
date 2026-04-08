use clap::Args;
use clap::Subcommand;
use loongclaw_spec::CliResult;

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum RuntimeTrajectoryCommands {
    /// Export one runtime trajectory artifact from a live session
    Export(RuntimeTrajectoryExportCommandOptions),
    /// Show one persisted runtime trajectory artifact in text or JSON form
    Show(RuntimeTrajectoryShowCommandOptions),
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTrajectoryExportCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub session: Option<String>,
    #[arg(long)]
    pub output: Option<String>,
    #[arg(long)]
    pub turn_limit: Option<usize>,
    #[arg(long, default_value_t = 200)]
    pub event_page_limit: usize,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTrajectoryShowCommandOptions {
    #[arg(long)]
    pub artifact: String,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

pub fn execute_runtime_trajectory_command(command: RuntimeTrajectoryCommands) -> CliResult<()> {
    match command {
        RuntimeTrajectoryCommands::Export(options) => crate::run_runtime_trajectory_cli(
            options.config.as_deref(),
            options.session.as_deref(),
            None,
            options.output.as_deref(),
            options.turn_limit,
            options.event_page_limit,
            options.json,
        ),
        RuntimeTrajectoryCommands::Show(options) => crate::run_runtime_trajectory_cli(
            None,
            None,
            Some(options.artifact.as_str()),
            None,
            None,
            200,
            options.json,
        ),
    }
}
