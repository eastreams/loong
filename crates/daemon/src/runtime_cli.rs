use clap::{Args, Subcommand};

use crate::{
    CliResult, acp_cli, control_plane_server, mcp_cli, operator_inventory_cli,
    runtime_capability_cli, runtime_experiment_cli, runtime_restore_cli, runtime_trajectory_cli,
    session_cli, trajectory_cli, work_unit_cli,
};

#[derive(Subcommand, Debug)]
pub enum RuntimeCommands {
    /// Print a unified runtime snapshot for experiment reproducibility and lineage capture
    Snapshot(RuntimeSnapshotArgs),
    /// Restore a persisted runtime snapshot artifact into the current config and managed skill state. Dry-run by default; pass --apply to mutate state.
    Restore(RuntimeRestoreArgs),
    /// Manage snapshot-linked experiment run records
    Experiment {
        #[command(subcommand)]
        command: runtime_experiment_cli::RuntimeExperimentCommands,
    },
    /// Manage run-derived capability candidates, family readiness, promotion plans, and governed apply outputs
    Capability {
        #[command(subcommand)]
        command: runtime_capability_cli::RuntimeCapabilityCommands,
    },
    /// Manage durable work units for long-running runtime orchestration
    WorkUnit {
        #[command(subcommand)]
        command: work_unit_cli::WorkUnitCommands,
    },
    /// Inspect provider model runtime inventory
    Models {
        #[command(subcommand)]
        command: RuntimeModelsCommands,
    },
    /// Inspect context-engine runtime inventory
    Context {
        #[command(subcommand)]
        command: RuntimeContextCommands,
    },
    /// Inspect memory-system runtime inventory
    Memory {
        #[command(subcommand)]
        command: RuntimeMemoryCommands,
    },
    /// Inspect configured MCP server runtime inventory
    Mcp {
        #[command(subcommand)]
        command: RuntimeMcpCommands,
    },
    /// Inspect ACP runtime state and diagnostics
    Acp {
        #[command(subcommand)]
        command: RuntimeAcpCommands,
    },
    /// Run or inspect control-plane surfaces
    ControlPlane {
        #[command(subcommand)]
        command: RuntimeControlPlaneCommands,
    },
    /// Inspect safe-lane runtime summaries
    SafeLane {
        #[command(subcommand)]
        command: RuntimeSafeLaneCommands,
    },
    /// Inspect session-search artifacts and transcript search results
    Session {
        #[command(subcommand)]
        command: RuntimeSessionCommands,
    },
    /// Export or inspect runtime trajectory artifacts for replay, evaluation, or research workflows
    Trajectory {
        #[command(subcommand)]
        command: RuntimeTrajectoryCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum RuntimeModelsCommands {
    /// Fetch and print currently available provider model list
    List(RuntimeReadArgs),
}

#[derive(Subcommand, Debug)]
pub enum RuntimeContextCommands {
    /// List available conversation context engines and selected runtime engine
    Engines(RuntimeReadArgs),
}

#[derive(Subcommand, Debug)]
pub enum RuntimeMemoryCommands {
    /// List available memory systems and selected runtime memory system
    Systems(RuntimeReadArgs),
}

#[derive(Subcommand, Debug)]
pub enum RuntimeMcpCommands {
    /// List configured MCP servers and their runtime-visible inventory state
    List(RuntimeReadArgs),
    /// Show one configured MCP server and its runtime-visible inventory state
    Show(RuntimeShowMcpServerArgs),
}

#[derive(Subcommand, Debug)]
pub enum RuntimeAcpCommands {
    /// List available ACP runtime backends and current control-plane selection
    Backends(RuntimeReadArgs),
    /// List persisted ACP session metadata from the local control-plane store
    Sessions(RuntimeReadArgs),
    /// Inspect live ACP session status by session key or conversation identity
    Status(RuntimeAcpStatusArgs),
    /// Inspect ACP control-plane observability snapshot from the shared session manager
    Observability(RuntimeReadArgs),
    /// Print ACP runtime event summary for a conversation session
    EventSummary(RuntimeAcpEventSummaryArgs),
    /// Evaluate ACP conversation dispatch policy for a session or structured channel address
    Dispatch(RuntimeAcpDispatchArgs),
    /// Run ACP backend readiness diagnostics for the selected or requested backend
    Doctor(RuntimeAcpDoctorArgs),
}

#[derive(Subcommand, Debug)]
pub enum RuntimeControlPlaneCommands {
    /// Run the loopback-only internal control-plane skeleton
    Serve(RuntimeControlPlaneServeArgs),
}

#[derive(Subcommand, Debug)]
pub enum RuntimeSafeLaneCommands {
    /// Print safe-lane runtime event summary for a session
    Summary(RuntimeSafeLaneSummaryArgs),
}

#[derive(Subcommand, Debug)]
pub enum RuntimeSessionCommands {
    /// Search transcript turns across visible sessions
    Search(RuntimeSessionSearchArgs),
    /// Inspect one exported session-search artifact
    Inspect(RuntimeSessionSearchInspectArgs),
}

#[derive(Subcommand, Debug)]
pub enum RuntimeTrajectoryCommands {
    /// Export one session trajectory artifact with transcript turns and session events
    Export(RuntimeTrajectoryExportArgs),
    /// Inspect one exported trajectory artifact
    Inspect(RuntimeTrajectoryInspectArgs),
    /// Export or inspect runtime trajectory artifacts for replay, evaluation, or research workflows
    Runtime {
        #[command(subcommand)]
        command: runtime_trajectory_cli::RuntimeTrajectoryCommands,
    },
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeReadArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeShowMcpServerArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub name: String,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSnapshotArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
    #[arg(long)]
    pub output: Option<String>,
    #[arg(long)]
    pub label: Option<String>,
    #[arg(long)]
    pub experiment_id: Option<String>,
    #[arg(long)]
    pub parent_snapshot_id: Option<String>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRestoreArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub snapshot: String,
    #[arg(long, default_value_t = false)]
    pub json: bool,
    #[arg(long, default_value_t = false)]
    pub apply: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAcpStatusArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long, conflicts_with_all = ["conversation_id", "route_session_id"])]
    pub session: Option<String>,
    #[arg(long, conflicts_with_all = ["session", "route_session_id"])]
    pub conversation_id: Option<String>,
    #[arg(long, conflicts_with_all = ["session", "conversation_id"])]
    pub route_session_id: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAcpEventSummaryArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub session: Option<String>,
    #[arg(long, default_value_t = 200)]
    pub limit: usize,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAcpDispatchArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub session: Option<String>,
    #[arg(long)]
    pub channel: Option<String>,
    #[arg(long)]
    pub conversation_id: Option<String>,
    #[arg(long)]
    pub account_id: Option<String>,
    #[arg(long)]
    pub participant_id: Option<String>,
    #[arg(long)]
    pub thread_id: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAcpDoctorArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub backend: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeControlPlaneServeArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub session: Option<String>,
    #[arg(long)]
    pub bind: Option<String>,
    #[arg(long, default_value_t = 0)]
    pub port: u16,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSafeLaneSummaryArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub session: Option<String>,
    #[arg(long, default_value_t = 200)]
    pub limit: usize,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSessionSearchArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub session: Option<String>,
    #[arg(long)]
    pub query: String,
    #[arg(long)]
    pub search_scope: Option<String>,
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
    #[arg(long)]
    pub output: Option<String>,
    #[arg(long, default_value_t = false)]
    pub include_archived: bool,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSessionSearchInspectArgs {
    #[arg(long)]
    pub artifact: String,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTrajectoryExportArgs {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub session: Option<String>,
    #[arg(long)]
    pub output: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTrajectoryInspectArgs {
    #[arg(long)]
    pub artifact: String,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

pub async fn run_runtime_cli(command: RuntimeCommands) -> CliResult<()> {
    match command {
        RuntimeCommands::Snapshot(args) => crate::run_runtime_snapshot_cli(
            args.config.as_deref(),
            args.json,
            args.output.as_deref(),
            args.label.as_deref(),
            args.experiment_id.as_deref(),
            args.parent_snapshot_id.as_deref(),
        ),
        RuntimeCommands::Restore(args) => runtime_restore_cli::run_runtime_restore_cli(
            runtime_restore_cli::RuntimeRestoreCommandOptions {
                config: args.config,
                snapshot: args.snapshot,
                json: args.json,
                apply: args.apply,
            },
        ),
        RuntimeCommands::Experiment { command } => {
            runtime_experiment_cli::run_runtime_experiment_cli(command)
        }
        RuntimeCommands::Capability { command } => {
            runtime_capability_cli::run_runtime_capability_cli(command)
        }
        RuntimeCommands::WorkUnit { command } => work_unit_cli::run_work_unit_cli(command),
        RuntimeCommands::Models { command } => match command {
            RuntimeModelsCommands::List(args) => {
                crate::run_list_models_cli(args.config.as_deref(), args.json).await
            }
        },
        RuntimeCommands::Context { command } => match command {
            RuntimeContextCommands::Engines(args) => {
                operator_inventory_cli::run_list_context_engines_cli(
                    args.config.as_deref(),
                    args.json,
                )
            }
        },
        RuntimeCommands::Memory { command } => match command {
            RuntimeMemoryCommands::Systems(args) => {
                operator_inventory_cli::run_list_memory_systems_cli(
                    args.config.as_deref(),
                    args.json,
                )
            }
        },
        RuntimeCommands::Mcp { command } => match command {
            RuntimeMcpCommands::List(args) => {
                mcp_cli::run_list_mcp_servers_cli(args.config.as_deref(), args.json)
            }
            RuntimeMcpCommands::Show(args) => mcp_cli::run_show_mcp_server_cli(
                args.config.as_deref(),
                args.name.as_str(),
                args.json,
            ),
        },
        RuntimeCommands::Acp { command } => match command {
            RuntimeAcpCommands::Backends(args) => {
                acp_cli::run_list_acp_backends_cli(args.config.as_deref(), args.json)
            }
            RuntimeAcpCommands::Sessions(args) => {
                acp_cli::run_list_acp_sessions_cli(args.config.as_deref(), args.json)
            }
            RuntimeAcpCommands::Status(args) => {
                acp_cli::run_acp_status_cli(
                    args.config.as_deref(),
                    args.session.as_deref(),
                    args.conversation_id.as_deref(),
                    args.route_session_id.as_deref(),
                    args.json,
                )
                .await
            }
            RuntimeAcpCommands::Observability(args) => {
                acp_cli::run_acp_observability_cli(args.config.as_deref(), args.json).await
            }
            RuntimeAcpCommands::EventSummary(args) => acp_cli::run_acp_event_summary_cli(
                args.config.as_deref(),
                args.session.as_deref(),
                args.limit,
                args.json,
            ),
            RuntimeAcpCommands::Dispatch(args) => acp_cli::run_acp_dispatch_cli(
                args.config.as_deref(),
                args.session.as_deref(),
                args.channel.as_deref(),
                args.conversation_id.as_deref(),
                args.account_id.as_deref(),
                args.participant_id.as_deref(),
                args.thread_id.as_deref(),
                args.json,
            ),
            RuntimeAcpCommands::Doctor(args) => {
                acp_cli::run_acp_doctor_cli(
                    args.config.as_deref(),
                    args.backend.as_deref(),
                    args.json,
                )
                .await
            }
        },
        RuntimeCommands::ControlPlane { command } => match command {
            RuntimeControlPlaneCommands::Serve(args) => {
                control_plane_server::run_control_plane_serve_cli(
                    args.config.as_deref(),
                    args.session.as_deref(),
                    args.bind.as_deref(),
                    args.port,
                )
                .await
            }
        },
        RuntimeCommands::SafeLane { command } => match command {
            RuntimeSafeLaneCommands::Summary(args) => {
                operator_inventory_cli::run_safe_lane_summary_cli(
                    args.config.as_deref(),
                    args.session.as_deref(),
                    args.limit,
                    args.json,
                )
            }
        },
        RuntimeCommands::Session { command } => match command {
            RuntimeSessionCommands::Search(args) => session_cli::run_session_search_cli(
                args.config.as_deref(),
                args.session.as_deref(),
                args.query.as_str(),
                args.search_scope.as_deref(),
                args.limit,
                args.output.as_deref(),
                args.include_archived,
                args.json,
            ),
            RuntimeSessionCommands::Inspect(args) => {
                session_cli::run_session_search_inspect_cli(args.artifact.as_str(), args.json)
            }
        },
        RuntimeCommands::Trajectory { command } => match command {
            RuntimeTrajectoryCommands::Export(args) => trajectory_cli::run_trajectory_export_cli(
                args.config.as_deref(),
                args.session.as_deref(),
                args.output.as_deref(),
                args.json,
            ),
            RuntimeTrajectoryCommands::Inspect(args) => {
                trajectory_cli::run_trajectory_inspect_cli(args.artifact.as_str(), args.json)
            }
            RuntimeTrajectoryCommands::Runtime { command } => {
                runtime_trajectory_cli::execute_runtime_trajectory_command(command)
            }
        },
    }
}
