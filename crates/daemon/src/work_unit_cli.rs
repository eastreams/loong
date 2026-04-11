use clap::{Args, Subcommand, ValueEnum};
use loongclaw_contracts::{
    WorkSourceKind, WorkUnitKind, WorkUnitPriority, WorkUnitRetryPolicy, WorkUnitSnapshot,
    WorkUnitSourceRef, WorkUnitStatus,
};
use loongclaw_spec::CliResult;

use crate::mvp;

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum WorkUnitCommands {
    /// Create one durable work unit record
    Create(WorkUnitCreateCommandOptions),
    /// Show one durable work unit record
    Show(WorkUnitShowCommandOptions),
    /// List durable work units
    List(WorkUnitListCommandOptions),
    /// List recent events for one durable work unit
    Events(WorkUnitEventsCommandOptions),
    /// Claim the next ready work unit for a worker lease
    Claim(WorkUnitClaimCommandOptions),
    /// Transition one leased work unit into the running state
    Start(WorkUnitStartCommandOptions),
    /// Refresh heartbeat and lease expiry for one leased work unit
    Heartbeat(WorkUnitHeartbeatCommandOptions),
    /// Mark one leased work unit as completed, retried, failed, or cancelled
    Complete(WorkUnitCompleteCommandOptions),
    /// Recover expired leases back into retry or terminal failure states
    Recover(WorkUnitRecoverCommandOptions),
    /// Archive one terminal work unit
    Archive(WorkUnitArchiveCommandOptions),
    /// Assign or clear a durable work-unit owner without taking a runtime lease
    Assign(WorkUnitAssignCommandOptions),
    /// Update mutable orchestration fields on a durable work unit
    Update(WorkUnitUpdateCommandOptions),
    /// Add one blocking dependency edge between two durable work units
    Depend(WorkUnitDependCommandOptions),
    /// Remove one blocking dependency edge between two durable work units
    Undepend(WorkUnitUndependCommandOptions),
    /// Append one orchestration note event to a durable work unit
    Note(WorkUnitNoteCommandOptions),
    /// Summarize durable runtime queue health
    Health(WorkUnitHealthCommandOptions),
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitCreateCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub id: Option<String>,
    #[arg(long, value_enum)]
    pub kind: WorkUnitKindArg,
    #[arg(long)]
    pub title: String,
    #[arg(long)]
    pub description: String,
    #[arg(long, value_enum, default_value_t = WorkUnitStatusArg::Ready)]
    pub status: WorkUnitStatusArg,
    #[arg(long, value_enum, default_value_t = WorkUnitPriorityArg::Normal)]
    pub priority: WorkUnitPriorityArg,
    #[arg(long, default_value_t = 3)]
    pub max_attempts: u32,
    #[arg(long, default_value_t = 1000)]
    pub initial_backoff_ms: u64,
    #[arg(long, default_value_t = 60_000)]
    pub max_backoff_ms: u64,
    #[arg(long)]
    pub next_run_at_ms: Option<i64>,
    #[arg(long)]
    pub actor: Option<String>,
    #[arg(long, value_enum, default_value_t = WorkSourceKindArg::Manual)]
    pub source_kind: WorkSourceKindArg,
    #[arg(long)]
    pub project_id: Option<String>,
    #[arg(long)]
    pub channel_id: Option<String>,
    #[arg(long)]
    pub thread_id: Option<String>,
    #[arg(long)]
    pub message_id: Option<String>,
    #[arg(long)]
    pub external_ref: Option<String>,
    #[arg(long)]
    pub source_url: Option<String>,
    #[arg(long)]
    pub parent_work_unit_id: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitShowCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub id: String,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitListCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long, value_enum)]
    pub status: Option<WorkUnitStatusArg>,
    #[arg(long, default_value_t = false)]
    pub include_archived: bool,
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitEventsCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub id: String,
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitClaimCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub owner: String,
    #[arg(long, default_value_t = 300_000)]
    pub ttl_ms: u64,
    #[arg(long)]
    pub actor: Option<String>,
    #[arg(long)]
    pub now_ms: Option<i64>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitStartCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub id: String,
    #[arg(long)]
    pub owner: String,
    #[arg(long)]
    pub actor: Option<String>,
    #[arg(long)]
    pub now_ms: Option<i64>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitHeartbeatCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub id: String,
    #[arg(long)]
    pub owner: String,
    #[arg(long, default_value_t = 300_000)]
    pub ttl_ms: u64,
    #[arg(long)]
    pub actor: Option<String>,
    #[arg(long)]
    pub now_ms: Option<i64>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitCompleteCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub id: String,
    #[arg(long)]
    pub owner: String,
    #[arg(long, value_enum)]
    pub disposition: WorkUnitDispositionArg,
    #[arg(long)]
    pub actor: Option<String>,
    #[arg(long)]
    pub now_ms: Option<i64>,
    #[arg(long)]
    pub next_run_at_ms: Option<i64>,
    #[arg(long)]
    pub result_payload_json: Option<String>,
    #[arg(long)]
    pub error: Option<String>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitRecoverCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub actor: Option<String>,
    #[arg(long)]
    pub now_ms: Option<i64>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitArchiveCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub id: String,
    #[arg(long)]
    pub actor: Option<String>,
    #[arg(long)]
    pub now_ms: Option<i64>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitAssignCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub id: String,
    #[arg(long)]
    pub assigned_to: Option<String>,
    #[arg(long)]
    pub actor: Option<String>,
    #[arg(long)]
    pub now_ms: Option<i64>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitUpdateCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub id: String,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
    #[arg(long, value_enum)]
    pub status: Option<WorkUnitStatusArg>,
    #[arg(long, value_enum)]
    pub priority: Option<WorkUnitPriorityArg>,
    #[arg(long)]
    pub next_run_at_ms: Option<i64>,
    #[arg(long)]
    pub blocking_reason: Option<String>,
    #[arg(long, default_value_t = false)]
    pub clear_blocking_reason: bool,
    #[arg(long)]
    pub actor: Option<String>,
    #[arg(long)]
    pub now_ms: Option<i64>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitDependCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub blocking_id: String,
    #[arg(long)]
    pub blocked_id: String,
    #[arg(long)]
    pub actor: Option<String>,
    #[arg(long)]
    pub now_ms: Option<i64>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitUndependCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub blocking_id: String,
    #[arg(long)]
    pub blocked_id: String,
    #[arg(long)]
    pub actor: Option<String>,
    #[arg(long)]
    pub now_ms: Option<i64>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitNoteCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub id: String,
    #[arg(long)]
    pub actor: Option<String>,
    #[arg(long)]
    pub note: String,
    #[arg(long)]
    pub now_ms: Option<i64>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitHealthCommandOptions {
    #[arg(long)]
    pub config: Option<String>,
    #[arg(long)]
    pub now_ms: Option<i64>,
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum WorkUnitKindArg {
    Feature,
    Issue,
    Review,
    Ops,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum WorkSourceKindArg {
    Manual,
    Discord,
    Github,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum WorkUnitStatusArg {
    Captured,
    Triaged,
    Ready,
    Leased,
    Running,
    WaitingExternal,
    WaitingReview,
    RetryPending,
    Completed,
    FailedTerminal,
    Cancelled,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum WorkUnitPriorityArg {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum WorkUnitDispositionArg {
    Completed,
    RetryPending,
    FailedTerminal,
    Cancelled,
}

impl From<WorkUnitKindArg> for WorkUnitKind {
    fn from(value: WorkUnitKindArg) -> Self {
        match value {
            WorkUnitKindArg::Feature => Self::Feature,
            WorkUnitKindArg::Issue => Self::Issue,
            WorkUnitKindArg::Review => Self::Review,
            WorkUnitKindArg::Ops => Self::Ops,
        }
    }
}

impl From<WorkSourceKindArg> for WorkSourceKind {
    fn from(value: WorkSourceKindArg) -> Self {
        match value {
            WorkSourceKindArg::Manual => Self::Manual,
            WorkSourceKindArg::Discord => Self::Discord,
            WorkSourceKindArg::Github => Self::Github,
        }
    }
}

impl From<WorkUnitStatusArg> for WorkUnitStatus {
    fn from(value: WorkUnitStatusArg) -> Self {
        match value {
            WorkUnitStatusArg::Captured => Self::Captured,
            WorkUnitStatusArg::Triaged => Self::Triaged,
            WorkUnitStatusArg::Ready => Self::Ready,
            WorkUnitStatusArg::Leased => Self::Leased,
            WorkUnitStatusArg::Running => Self::Running,
            WorkUnitStatusArg::WaitingExternal => Self::WaitingExternal,
            WorkUnitStatusArg::WaitingReview => Self::WaitingReview,
            WorkUnitStatusArg::RetryPending => Self::RetryPending,
            WorkUnitStatusArg::Completed => Self::Completed,
            WorkUnitStatusArg::FailedTerminal => Self::FailedTerminal,
            WorkUnitStatusArg::Cancelled => Self::Cancelled,
            WorkUnitStatusArg::Archived => Self::Archived,
        }
    }
}

impl From<WorkUnitPriorityArg> for WorkUnitPriority {
    fn from(value: WorkUnitPriorityArg) -> Self {
        match value {
            WorkUnitPriorityArg::Low => Self::Low,
            WorkUnitPriorityArg::Normal => Self::Normal,
            WorkUnitPriorityArg::High => Self::High,
            WorkUnitPriorityArg::Critical => Self::Critical,
        }
    }
}

impl From<WorkUnitDispositionArg> for mvp::work::repository::WorkUnitCompletionDisposition {
    fn from(value: WorkUnitDispositionArg) -> Self {
        match value {
            WorkUnitDispositionArg::Completed => Self::Completed,
            WorkUnitDispositionArg::RetryPending => Self::RetryPending,
            WorkUnitDispositionArg::FailedTerminal => Self::FailedTerminal,
            WorkUnitDispositionArg::Cancelled => Self::Cancelled,
        }
    }
}

pub fn run_work_unit_cli(command: WorkUnitCommands) -> CliResult<()> {
    match command {
        WorkUnitCommands::Create(options) => run_create_command(options),
        WorkUnitCommands::Show(options) => run_show_command(options),
        WorkUnitCommands::List(options) => run_list_command(options),
        WorkUnitCommands::Events(options) => run_events_command(options),
        WorkUnitCommands::Claim(options) => run_claim_command(options),
        WorkUnitCommands::Start(options) => run_start_command(options),
        WorkUnitCommands::Heartbeat(options) => run_heartbeat_command(options),
        WorkUnitCommands::Complete(options) => run_complete_command(options),
        WorkUnitCommands::Recover(options) => run_recover_command(options),
        WorkUnitCommands::Archive(options) => run_archive_command(options),
        WorkUnitCommands::Assign(options) => run_assign_command(options),
        WorkUnitCommands::Update(options) => run_update_command(options),
        WorkUnitCommands::Depend(options) => run_depend_command(options),
        WorkUnitCommands::Undepend(options) => run_undepend_command(options),
        WorkUnitCommands::Note(options) => run_note_command(options),
        WorkUnitCommands::Health(options) => run_health_command(options),
    }
}

fn run_create_command(options: WorkUnitCreateCommandOptions) -> CliResult<()> {
    let repository = load_work_unit_repository(options.config.as_deref())?;
    let retry_policy = WorkUnitRetryPolicy {
        max_attempts: options.max_attempts,
        initial_backoff_ms: options.initial_backoff_ms,
        max_backoff_ms: options.max_backoff_ms,
    };
    let source_ref = WorkUnitSourceRef {
        source_kind: options.source_kind.into(),
        project_id: options.project_id,
        channel_id: options.channel_id,
        thread_id: options.thread_id,
        message_id: options.message_id,
        external_ref: options.external_ref,
        source_url: options.source_url,
    };
    let new_work_unit = mvp::work::repository::NewWorkUnitRecord {
        work_unit_id: options.id,
        kind: options.kind.into(),
        title: options.title,
        description: options.description,
        source_ref,
        status: options.status.into(),
        priority: options.priority.into(),
        retry_policy,
        parent_work_unit_id: options.parent_work_unit_id,
        next_run_at_ms: options.next_run_at_ms,
    };
    let snapshot = repository.create_work_unit(new_work_unit, options.actor.as_deref())?;
    render_json_or_text(&snapshot, options.json, render_work_unit_snapshot_text)
}

fn run_show_command(options: WorkUnitShowCommandOptions) -> CliResult<()> {
    let repository = load_work_unit_repository(options.config.as_deref())?;
    let snapshot = repository.load_work_unit_snapshot(options.id.as_str())?;
    let Some(snapshot) = snapshot else {
        return Err(format!("work unit `{}` not found", options.id));
    };
    render_json_or_text(&snapshot, options.json, render_work_unit_snapshot_text)
}

fn run_list_command(options: WorkUnitListCommandOptions) -> CliResult<()> {
    let repository = load_work_unit_repository(options.config.as_deref())?;
    let query = mvp::work::repository::WorkUnitListQuery {
        status: options.status.map(WorkUnitStatus::from),
        include_archived: options.include_archived,
        limit: options.limit,
    };
    let snapshots = repository.list_work_units(query)?;
    render_json_or_text(&snapshots, options.json, |value| {
        render_work_unit_list_text(value.as_slice())
    })
}

fn run_events_command(options: WorkUnitEventsCommandOptions) -> CliResult<()> {
    let repository = load_work_unit_repository(options.config.as_deref())?;
    let events = repository.list_work_unit_events(options.id.as_str(), options.limit)?;
    render_json_or_text(&events, options.json, |value| {
        render_work_unit_events_text(value.as_slice())
    })
}

fn run_claim_command(options: WorkUnitClaimCommandOptions) -> CliResult<()> {
    let repository = load_work_unit_repository(options.config.as_deref())?;
    let request = mvp::work::repository::AcquireWorkUnitLeaseRequest {
        owner: options.owner,
        ttl_ms: options.ttl_ms,
        actor: options.actor,
        now_ms: options.now_ms,
    };
    let snapshot = repository.acquire_next_ready_lease(request)?;
    if options.json {
        let payload = serde_json::json!({ "claimed": snapshot.is_some(), "snapshot": snapshot });
        return print_json(payload);
    }
    let Some(snapshot) = snapshot else {
        println!("claimed=false");
        return Ok(());
    };
    println!("claimed=true");
    print!("{}", render_work_unit_snapshot_text(&snapshot));
    Ok(())
}

fn run_start_command(options: WorkUnitStartCommandOptions) -> CliResult<()> {
    let repository = load_work_unit_repository(options.config.as_deref())?;
    let request = mvp::work::repository::StartWorkUnitLeaseRequest {
        work_unit_id: options.id,
        owner: options.owner,
        actor: options.actor,
        now_ms: options.now_ms,
    };
    let snapshot = repository.mark_leased_running(request)?;
    let missing_message = "start did not find a matching active lease";
    render_optional_snapshot(snapshot, options.json, missing_message)
}

fn run_heartbeat_command(options: WorkUnitHeartbeatCommandOptions) -> CliResult<()> {
    let repository = load_work_unit_repository(options.config.as_deref())?;
    let request = mvp::work::repository::WorkUnitHeartbeatRequest {
        work_unit_id: options.id,
        owner: options.owner,
        ttl_ms: options.ttl_ms,
        actor: options.actor,
        now_ms: options.now_ms,
    };
    let snapshot = repository.heartbeat_lease(request)?;
    let missing_message = "heartbeat did not find a matching active lease";
    render_optional_snapshot(snapshot, options.json, missing_message)
}

fn run_complete_command(options: WorkUnitCompleteCommandOptions) -> CliResult<()> {
    let repository = load_work_unit_repository(options.config.as_deref())?;
    let result_payload_json = options
        .result_payload_json
        .as_deref()
        .map(parse_json_value)
        .transpose()?;
    let request = mvp::work::repository::CompleteWorkUnitRequest {
        work_unit_id: options.id,
        owner: options.owner,
        disposition: options.disposition.into(),
        actor: options.actor,
        now_ms: options.now_ms,
        next_run_at_ms: options.next_run_at_ms,
        result_payload_json,
        error: options.error,
    };
    let snapshot = repository.complete_work_unit(request)?;
    let missing_message = "complete did not find a matching active lease";
    render_optional_snapshot(snapshot, options.json, missing_message)
}

fn run_recover_command(options: WorkUnitRecoverCommandOptions) -> CliResult<()> {
    let repository = load_work_unit_repository(options.config.as_deref())?;
    let snapshots = repository.recover_expired_leases(options.actor.as_deref(), options.now_ms)?;
    render_json_or_text(&snapshots, options.json, |value| {
        render_work_unit_list_text(value.as_slice())
    })
}

fn run_archive_command(options: WorkUnitArchiveCommandOptions) -> CliResult<()> {
    let repository = load_work_unit_repository(options.config.as_deref())?;
    let request = mvp::work::repository::ArchiveWorkUnitRequest {
        work_unit_id: options.id,
        actor: options.actor,
        now_ms: options.now_ms,
    };
    let snapshot = repository.archive_work_unit(request)?;
    let missing_message =
        "archive failed: work unit not found, already archived, or not in a terminal state";
    render_optional_snapshot(snapshot, options.json, missing_message)
}

fn run_assign_command(options: WorkUnitAssignCommandOptions) -> CliResult<()> {
    let repository = load_work_unit_repository(options.config.as_deref())?;
    let request = mvp::work::repository::AssignWorkUnitRequest {
        work_unit_id: options.id,
        assigned_to: options.assigned_to,
        actor: options.actor,
        now_ms: options.now_ms,
    };
    let snapshot = repository.assign_work_unit(request)?;
    let missing_message = "assign failed: work unit not found or archived";
    render_optional_snapshot(snapshot, options.json, missing_message)
}

fn run_update_command(options: WorkUnitUpdateCommandOptions) -> CliResult<()> {
    let repository = load_work_unit_repository(options.config.as_deref())?;
    let request = mvp::work::repository::UpdateWorkUnitRequest {
        work_unit_id: options.id,
        title: options.title,
        description: options.description,
        status: options.status.map(WorkUnitStatus::from),
        priority: options.priority.map(WorkUnitPriority::from),
        next_run_at_ms: options.next_run_at_ms,
        blocking_reason: options.blocking_reason,
        clear_blocking_reason: options.clear_blocking_reason,
        actor: options.actor,
        now_ms: options.now_ms,
    };
    let snapshot = repository.update_work_unit(request)?;
    let missing_message = "update failed: work unit not found or archived";
    render_optional_snapshot(snapshot, options.json, missing_message)
}

fn run_depend_command(options: WorkUnitDependCommandOptions) -> CliResult<()> {
    let repository = load_work_unit_repository(options.config.as_deref())?;
    let request = mvp::work::repository::AddWorkUnitDependencyRequest {
        blocking_work_unit_id: options.blocking_id,
        blocked_work_unit_id: options.blocked_id,
        actor: options.actor,
        now_ms: options.now_ms,
    };
    let snapshot = repository.add_dependency(request)?;
    let missing_message = "depend failed: blocked work unit not found after dependency update";
    render_optional_snapshot(snapshot, options.json, missing_message)
}

fn run_undepend_command(options: WorkUnitUndependCommandOptions) -> CliResult<()> {
    let repository = load_work_unit_repository(options.config.as_deref())?;
    let request = mvp::work::repository::RemoveWorkUnitDependencyRequest {
        blocking_work_unit_id: options.blocking_id,
        blocked_work_unit_id: options.blocked_id,
        actor: options.actor,
        now_ms: options.now_ms,
    };
    let snapshot = repository.remove_dependency(request)?;
    let missing_message = "undepend failed: blocked work unit not found";
    render_optional_snapshot(snapshot, options.json, missing_message)
}

fn run_note_command(options: WorkUnitNoteCommandOptions) -> CliResult<()> {
    let repository = load_work_unit_repository(options.config.as_deref())?;
    let request = mvp::work::repository::AppendWorkUnitNoteRequest {
        work_unit_id: options.id,
        actor: options.actor,
        note: options.note,
        now_ms: options.now_ms,
    };
    let event = repository.append_note(request)?;
    let Some(event) = event else {
        return Err("note failed: work unit not found or archived".to_owned());
    };
    render_json_or_text(&event, options.json, render_single_work_unit_event_text)
}

fn run_health_command(options: WorkUnitHealthCommandOptions) -> CliResult<()> {
    let repository = load_work_unit_repository(options.config.as_deref())?;
    let health = repository.load_runtime_health(options.now_ms)?;
    render_json_or_text(&health, options.json, render_work_unit_health_text)
}

fn load_work_unit_repository(
    config_path: Option<&str>,
) -> CliResult<mvp::work::repository::WorkUnitRepository> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = config_path;
        Err("work unit runtime requires feature `memory-sqlite`".to_owned())
    }

    #[cfg(feature = "memory-sqlite")]
    {
        let (_, config) = mvp::config::load(config_path)?;
        let memory_config =
            mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        mvp::work::repository::WorkUnitRepository::new(&memory_config)
    }
}

fn render_optional_snapshot(
    snapshot: Option<WorkUnitSnapshot>,
    as_json: bool,
    missing_message: &str,
) -> CliResult<()> {
    let Some(snapshot) = snapshot else {
        return Err(missing_message.to_owned());
    };
    render_json_or_text(&snapshot, as_json, render_work_unit_snapshot_text)
}

fn render_json_or_text<T>(
    value: &T,
    as_json: bool,
    render_text: impl FnOnce(&T) -> String,
) -> CliResult<()>
where
    T: serde::Serialize,
{
    if as_json {
        let payload = serde_json::to_string_pretty(value)
            .map_err(|error| format!("serialize work unit output failed: {error}"))?;
        println!("{payload}");
        return Ok(());
    }
    print!("{}", render_text(value));
    Ok(())
}

fn print_json(value: serde_json::Value) -> CliResult<()> {
    let payload = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("serialize work unit json payload failed: {error}"))?;
    println!("{payload}");
    Ok(())
}

fn parse_json_value(raw: &str) -> CliResult<serde_json::Value> {
    serde_json::from_str(raw).map_err(|error| format!("parse result payload json failed: {error}"))
}

fn render_work_unit_snapshot_text(snapshot: &WorkUnitSnapshot) -> String {
    let work_unit = &snapshot.work_unit;
    let lease_text = snapshot
        .lease
        .as_ref()
        .map(render_lease_text)
        .unwrap_or_else(|| "lease: (none)".to_owned());
    let result_payload = work_unit
        .result_payload_json
        .as_ref()
        .and_then(|value| serde_json::to_string(value).ok())
        .unwrap_or_else(|| "-".to_owned());
    let last_error = work_unit.last_error.as_deref().unwrap_or("-");
    let blocking_reason = work_unit.blocking_reason.as_deref().unwrap_or("-");
    let parent = work_unit.parent_work_unit_id.as_deref().unwrap_or("-");
    let assigned_to = work_unit.assigned_to.as_deref().unwrap_or("-");
    let blocks = render_string_list(work_unit.blocks_work_unit_ids.as_slice());
    let blocked_by = render_string_list(work_unit.blocked_by_work_unit_ids.as_slice());
    let source = render_source_ref(&work_unit.source_ref);
    let retry = render_retry_policy(&work_unit.retry_policy);
    format!(
        "id={} kind={} status={} priority={} attempts={} next_run_at_ms={} archived_at_ms={}\nsource={}\nretry={}\nparent_work_unit_id={}\nassigned_to={}\nblocks_work_unit_ids={}\nblocked_by_work_unit_ids={}\ntitle={}\ndescription={}\nlast_error={}\nblocking_reason={}\nresult_payload_json={}\n{}\n",
        work_unit.work_unit_id,
        work_unit.kind.as_str(),
        work_unit.status.as_str(),
        work_unit.priority.as_str(),
        work_unit.attempt_count,
        work_unit.next_run_at_ms,
        render_optional_i64(work_unit.archived_at_ms),
        source,
        retry,
        parent,
        assigned_to,
        blocks,
        blocked_by,
        work_unit.title,
        work_unit.description,
        last_error,
        blocking_reason,
        result_payload,
        lease_text,
    )
}

fn render_work_unit_list_text(snapshots: &[WorkUnitSnapshot]) -> String {
    if snapshots.is_empty() {
        return "work_units: (none)\n".to_owned();
    }
    let mut lines = Vec::new();
    lines.push("work_units:".to_owned());
    for snapshot in snapshots {
        let work_unit = &snapshot.work_unit;
        let lease_owner = snapshot
            .lease
            .as_ref()
            .map(|lease| lease.owner.as_str())
            .unwrap_or("-");
        let assigned_to = work_unit.assigned_to.as_deref().unwrap_or("-");
        let blocked_by_count = work_unit.blocked_by_work_unit_ids.len();
        let line = format!(
            "- id={} kind={} status={} priority={} attempts={} next_run_at_ms={} lease_owner={} assigned_to={} blocked_by_count={}",
            work_unit.work_unit_id,
            work_unit.kind.as_str(),
            work_unit.status.as_str(),
            work_unit.priority.as_str(),
            work_unit.attempt_count,
            work_unit.next_run_at_ms,
            lease_owner,
            assigned_to,
            blocked_by_count,
        );
        lines.push(line);
    }
    lines.push(String::new());
    lines.join("\n")
}

fn render_work_unit_events_text(events: &[loongclaw_contracts::WorkUnitEventRecord]) -> String {
    if events.is_empty() {
        return "work_unit_events: (none)\n".to_owned();
    }
    let mut lines = Vec::new();
    lines.push("work_unit_events:".to_owned());
    for event in events {
        let payload =
            serde_json::to_string(&event.payload_json).unwrap_or_else(|_| "{}".to_owned());
        let line = format!(
            "- sequence_id={} event_kind={} actor={} recorded_at_ms={} payload={}",
            event.sequence_id,
            event.event_kind,
            event.actor.as_deref().unwrap_or("-"),
            event.recorded_at_ms,
            payload,
        );
        lines.push(line);
    }
    lines.push(String::new());
    lines.join("\n")
}

fn render_single_work_unit_event_text(event: &loongclaw_contracts::WorkUnitEventRecord) -> String {
    render_work_unit_events_text(std::slice::from_ref(event))
}

fn render_work_unit_health_text(health: &loongclaw_contracts::WorkRuntimeHealthSnapshot) -> String {
    format!(
        "total_count={} ready_count={} leased_count={} running_count={} blocked_count={} retry_pending_count={} terminal_count={} archived_count={} expired_lease_count={}\n",
        health.total_count,
        health.ready_count,
        health.leased_count,
        health.running_count,
        health.blocked_count,
        health.retry_pending_count,
        health.terminal_count,
        health.archived_count,
        health.expired_lease_count,
    )
}

fn render_source_ref(source_ref: &WorkUnitSourceRef) -> String {
    let project_id = source_ref.project_id.as_deref().unwrap_or("-");
    let channel_id = source_ref.channel_id.as_deref().unwrap_or("-");
    let thread_id = source_ref.thread_id.as_deref().unwrap_or("-");
    let message_id = source_ref.message_id.as_deref().unwrap_or("-");
    let external_ref = source_ref.external_ref.as_deref().unwrap_or("-");
    let source_url = source_ref.source_url.as_deref().unwrap_or("-");
    format!(
        "source_kind={} project_id={} channel_id={} thread_id={} message_id={} external_ref={} source_url={}",
        source_ref.source_kind.as_str(),
        project_id,
        channel_id,
        thread_id,
        message_id,
        external_ref,
        source_url,
    )
}

fn render_retry_policy(retry_policy: &WorkUnitRetryPolicy) -> String {
    format!(
        "max_attempts={} initial_backoff_ms={} max_backoff_ms={}",
        retry_policy.max_attempts, retry_policy.initial_backoff_ms, retry_policy.max_backoff_ms,
    )
}

fn render_lease_text(lease: &loongclaw_contracts::WorkUnitLeaseRecord) -> String {
    format!(
        "lease: owner={} lease_version={} acquired_at_ms={} heartbeat_at_ms={} expires_at_ms={}",
        lease.owner,
        lease.lease_version,
        lease.acquired_at_ms,
        lease.heartbeat_at_ms,
        lease.expires_at_ms,
    )
}

fn render_optional_i64(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn render_string_list(values: &[String]) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }
    values.join(",")
}
