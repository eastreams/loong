use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::CliResult;
use crate::mvp;

const AUTOMATION_SCHEMA_VERSION: u32 = 1;
const AUTOMATION_DEFAULT_POLL_MS: u64 = 1_000;
const AUTOMATION_DEFAULT_EVENT_PATH: &str = "/automation/events";
const AUTOMATION_FAILURE_RETRY_MS: i64 = 60_000;
const AUTOMATION_RUNTIME_HEARTBEAT_MS: u64 = 5_000;
const AUTOMATION_RUNTIME_STALE_MS: u64 = 15_000;

static AUTOMATION_TRIGGER_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum AutomationCommands {
    /// Create one schedule-based automation trigger
    CreateSchedule(AutomationCreateScheduleCommandOptions),
    /// Create one cron-style automation trigger
    CreateCron(AutomationCreateCronCommandOptions),
    /// Inspect or preview cron expressions without creating a trigger
    Cron(AutomationCronCommandOptions),
    /// Create one event-triggered automation rule
    CreateEvent(AutomationCreateEventCommandOptions),
    /// List durable automation triggers
    List(AutomationListCommandOptions),
    /// Show one durable automation trigger
    Show(AutomationShowCommandOptions),
    /// Remove one durable automation trigger
    Remove(AutomationRemoveCommandOptions),
    /// Pause one durable automation trigger
    Pause(AutomationPauseCommandOptions),
    /// Resume one durable automation trigger
    Resume(AutomationResumeCommandOptions),
    /// Fire one trigger immediately
    Fire(AutomationFireCommandOptions),
    /// Emit one named event to matching triggers
    Emit(AutomationEmitCommandOptions),
    /// Inspect or manage the automation runner owner lifecycle
    Runner(AutomationRunnerCommandOptions),
    /// Inspect or manage the internal automation journal
    Journal(AutomationJournalCommandOptions),
    /// Run the scheduler loop and optional webhook ingress
    Serve(AutomationServeCommandOptions),
}

#[derive(Debug, Clone)]
pub struct AutomationCommandOptions {
    pub config: Option<String>,
    pub json: bool,
    pub command: AutomationCommands,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AutomationCreateScheduleCommandOptions {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub session: String,
    #[arg(long)]
    pub task: String,
    #[arg(long)]
    pub label: Option<String>,
    #[arg(long)]
    pub timeout_seconds: Option<u64>,
    #[arg(long)]
    pub run_at: Option<String>,
    #[arg(long)]
    pub run_at_ms: Option<i64>,
    #[arg(long)]
    pub every_seconds: Option<u64>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AutomationCreateEventCommandOptions {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub event: String,
    #[arg(long)]
    pub json_pointer: Option<String>,
    #[arg(long)]
    pub equals_json: Option<String>,
    #[arg(long)]
    pub equals_text: Option<String>,
    #[arg(long)]
    pub contains_text: Option<String>,
    #[arg(long)]
    pub session: String,
    #[arg(long)]
    pub task: String,
    #[arg(long)]
    pub label: Option<String>,
    #[arg(long)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AutomationCreateCronCommandOptions {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub cron: String,
    #[arg(long)]
    pub session: String,
    #[arg(long)]
    pub task: String,
    #[arg(long)]
    pub label: Option<String>,
    #[arg(long)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AutomationCronCommandOptions {
    #[command(subcommand)]
    pub command: AutomationCronCommands,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum AutomationCronCommands {
    /// Preview the next fire times for one cron expression
    Preview(AutomationCronPreviewCommandOptions),
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AutomationCronPreviewCommandOptions {
    #[arg(long)]
    pub cron: String,
    #[arg(long)]
    pub after: Option<String>,
    #[arg(long)]
    pub after_ms: Option<i64>,
    #[arg(long, default_value_t = 5)]
    pub count: usize,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AutomationListCommandOptions {
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    #[arg(long, default_value_t = false)]
    pub include_completed: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AutomationShowCommandOptions {
    #[arg(long)]
    pub id: String,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AutomationRemoveCommandOptions {
    #[arg(long)]
    pub id: String,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AutomationPauseCommandOptions {
    #[arg(long)]
    pub id: String,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AutomationResumeCommandOptions {
    #[arg(long)]
    pub id: String,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AutomationFireCommandOptions {
    #[arg(long)]
    pub id: String,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AutomationEmitCommandOptions {
    #[arg(long)]
    pub event: String,
    #[arg(long)]
    pub payload_json: Option<String>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AutomationServeCommandOptions {
    #[arg(long)]
    pub bind: Option<String>,
    #[arg(long)]
    pub auth_token: Option<String>,
    #[arg(long)]
    pub path: Option<String>,
    #[arg(long)]
    pub poll_ms: Option<u64>,
    #[arg(long)]
    pub retain_last_sealed: Option<usize>,
    #[arg(long)]
    pub retain_min_age_seconds: Option<u64>,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AutomationRunnerCommandOptions {
    #[command(subcommand)]
    pub command: AutomationRunnerCommands,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum AutomationRunnerCommands {
    /// Inspect the current automation runner owner state
    Inspect(AutomationRunnerInspectCommandOptions),
    /// Request graceful shutdown for the current automation runner owner
    Stop(AutomationRunnerStopCommandOptions),
    /// Reclaim a stale automation runner owner slot
    Reclaim(AutomationRunnerReclaimCommandOptions),
}

#[derive(Args, Debug, Clone, PartialEq, Eq, Default)]
pub struct AutomationRunnerInspectCommandOptions {}

#[derive(Args, Debug, Clone, PartialEq, Eq, Default)]
pub struct AutomationRunnerStopCommandOptions {}

#[derive(Args, Debug, Clone, PartialEq, Eq, Default)]
pub struct AutomationRunnerReclaimCommandOptions {}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AutomationJournalCommandOptions {
    #[command(subcommand)]
    pub command: AutomationJournalCommands,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum AutomationJournalCommands {
    /// Inspect the internal automation journal layout and cursor
    Inspect(AutomationJournalInspectCommandOptions),
    /// Report automation journal health and drift signals
    Health(AutomationJournalHealthCommandOptions),
    /// Rotate the active internal automation journal segment
    Rotate(AutomationJournalRotateCommandOptions),
    /// Prune sealed internal automation journal segments older than the retained segment
    Prune(AutomationJournalPruneCommandOptions),
    /// Repair the internal automation journal state from on-disk layout
    Repair(AutomationJournalRepairCommandOptions),
}

#[derive(Args, Debug, Clone, PartialEq, Eq, Default)]
pub struct AutomationJournalInspectCommandOptions {}

#[derive(Args, Debug, Clone, PartialEq, Eq, Default)]
pub struct AutomationJournalHealthCommandOptions {}

#[derive(Args, Debug, Clone, PartialEq, Eq, Default)]
pub struct AutomationJournalRotateCommandOptions {}

#[derive(Args, Debug, Clone, PartialEq, Eq, Default)]
pub struct AutomationJournalPruneCommandOptions {
    #[arg(long)]
    pub retain_segment_id: Option<String>,
    #[arg(long)]
    pub retain_last_sealed: Option<usize>,
    #[arg(long)]
    pub retain_min_age_seconds: Option<u64>,
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[derive(Args, Debug, Clone, PartialEq, Eq, Default)]
pub struct AutomationJournalRepairCommandOptions {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AutomationTriggerStatus {
    Active,
    Paused,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct AutomationScheduleSpec {
    next_fire_at_ms: i64,
    interval_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct AutomationEventSpec {
    event_name: String,
    json_pointer: Option<String>,
    equals_json: Option<Value>,
    contains_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct AutomationCronSpec {
    expression: String,
    next_fire_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AutomationTriggerSource {
    Schedule { schedule: AutomationScheduleSpec },
    Cron { cron: AutomationCronSpec },
    Event { event: AutomationEventSpec },
}

impl AutomationTriggerSource {
    fn kind_str(&self) -> &'static str {
        match self {
            Self::Schedule { .. } => "schedule",
            Self::Cron { .. } => "cron",
            Self::Event { .. } => "event",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct BackgroundTaskAction {
    session: String,
    task: String,
    label: Option<String>,
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AutomationAction {
    BackgroundTask {
        background_task: BackgroundTaskAction,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct AutomationTriggerRecord {
    trigger_id: String,
    name: String,
    status: AutomationTriggerStatus,
    source: AutomationTriggerSource,
    action: AutomationAction,
    created_at_ms: i64,
    updated_at_ms: i64,
    last_fired_at_ms: Option<i64>,
    last_task_id: Option<String>,
    last_error: Option<String>,
    fire_count: u64,
    #[serde(default)]
    run_history: Vec<AutomationRunRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct AutomationStore {
    schema_version: u32,
    triggers: Vec<AutomationTriggerRecord>,
}

impl Default for AutomationStore {
    fn default() -> Self {
        Self {
            schema_version: AUTOMATION_SCHEMA_VERSION,
            triggers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct AutomationFireResult {
    trigger_id: String,
    name: String,
    source_kind: String,
    queued_task_id: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct AutomationRunRecord {
    fired_at_ms: i64,
    source_kind: String,
    queued_task_id: Option<String>,
    error: Option<String>,
}

#[derive(Clone)]
struct AutomationServeState {
    config: Option<String>,
    auth_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AutomationRunnerStatus {
    runtime_dir: String,
    phase: String,
    running: bool,
    stale: bool,
    pid: Option<u32>,
    version: String,
    config_path: Option<String>,
    bind_address: Option<String>,
    event_path: Option<String>,
    poll_ms: u64,
    retain_last_sealed_segments: usize,
    retain_min_age_ms: Option<u64>,
    lease_timeout_ms: u64,
    lease_expires_at_ms: u64,
    started_at_ms: u64,
    last_heartbeat_at: u64,
    stopped_at_ms: Option<u64>,
    shutdown_reason: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedAutomationRunnerState {
    phase: String,
    running: bool,
    pid: Option<u32>,
    version: String,
    config_path: Option<String>,
    bind_address: Option<String>,
    event_path: Option<String>,
    poll_ms: u64,
    #[serde(default)]
    retain_last_sealed_segments: usize,
    #[serde(default)]
    retain_min_age_ms: Option<u64>,
    started_at_ms: u64,
    last_heartbeat_at: u64,
    stopped_at_ms: Option<u64>,
    shutdown_reason: Option<String>,
    last_error: Option<String>,
    owner_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedAutomationStopRequest {
    requested_at_ms: u64,
    requested_by_pid: u32,
    target_owner_token: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutomationRunnerStopRequestOutcome {
    Requested,
    AlreadyRequested,
    AlreadyStopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutomationRunnerReclaimOutcome {
    Reclaimed,
    AlreadyClean,
    OwnerActive,
}

struct AutomationRunnerTracker {
    active_owner_path: PathBuf,
    status_snapshot_path: PathBuf,
    stop_request_path: PathBuf,
    owner_token: String,
    state: std::sync::Mutex<PersistedAutomationRunnerState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AutomationRunnerRetentionPolicy {
    retain_last_sealed_segments: usize,
    retain_min_age_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct LoadedAutomationConfig {
    resolved_path: PathBuf,
    config: mvp::config::LoongConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedAutomationServeSettings {
    event_path: String,
    poll_ms: u64,
    retention_policy: AutomationRunnerRetentionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CronField {
    any: bool,
    allowed: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CronExpression {
    minute: CronField,
    hour: CronField,
    day_of_month: CronField,
    month: CronField,
    day_of_week: CronField,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct AutomationCronPreviewEntry {
    ordinal: usize,
    fire_at_ms: i64,
    fire_at_rfc3339: String,
}

pub async fn run_automation_cli(options: AutomationCommandOptions) -> CliResult<()> {
    let payload = execute_automation_command(options.clone()).await?;
    if options.json {
        let rendered = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize automation CLI payload failed: {error}"))?;
        println!("{rendered}");
        return Ok(());
    }

    let rendered = render_automation_text(&payload)?;
    println!("{rendered}");
    Ok(())
}

pub async fn execute_automation_command(options: AutomationCommandOptions) -> CliResult<Value> {
    let store_path = automation_store_path();
    match options.command {
        AutomationCommands::CreateSchedule(command) => {
            execute_create_schedule_command(store_path.as_path(), command).await
        }
        AutomationCommands::CreateCron(command) => {
            execute_create_cron_command(store_path.as_path(), command).await
        }
        AutomationCommands::Cron(command) => execute_cron_command(command),
        AutomationCommands::CreateEvent(command) => {
            execute_create_event_command(store_path.as_path(), command).await
        }
        AutomationCommands::List(command) => execute_list_command(store_path.as_path(), command),
        AutomationCommands::Show(command) => {
            execute_show_command(store_path.as_path(), &command.id)
        }
        AutomationCommands::Remove(command) => {
            execute_remove_command(store_path.as_path(), &command.id)
        }
        AutomationCommands::Pause(command) => execute_status_update_command(
            store_path.as_path(),
            &command.id,
            AutomationTriggerStatus::Paused,
        ),
        AutomationCommands::Resume(command) => execute_status_update_command(
            store_path.as_path(),
            &command.id,
            AutomationTriggerStatus::Active,
        ),
        AutomationCommands::Fire(command) => {
            execute_fire_command(store_path.as_path(), options.config, &command.id).await
        }
        AutomationCommands::Emit(command) => {
            execute_emit_command(
                store_path.as_path(),
                options.config,
                command.event.as_str(),
                command.payload_json.as_deref(),
            )
            .await
        }
        AutomationCommands::Runner(command) => execute_runner_command(command),
        AutomationCommands::Journal(command) => {
            execute_journal_command(options.config, command).await
        }
        AutomationCommands::Serve(command) => {
            execute_serve_command(store_path.as_path(), options.config, command).await
        }
    }
}

fn automation_store_path() -> PathBuf {
    crate::mvp::config::default_loong_home()
        .join("automation")
        .join("triggers.json")
}

fn automation_serve_lock_path() -> PathBuf {
    crate::mvp::config::default_loong_home()
        .join("automation")
        .join("serve.lock")
}

pub(crate) fn automation_serve_owner_is_active() -> bool {
    let status = load_automation_runner_status();
    status
        .as_ref()
        .is_some_and(|status| status.running && !status.stale)
}

fn automation_event_cursor_path() -> PathBuf {
    crate::mvp::config::default_loong_home()
        .join("automation")
        .join("internal-events.cursor")
}

fn automation_runner_status_snapshot_path() -> PathBuf {
    crate::mvp::config::default_loong_home()
        .join("automation")
        .join("serve.status.json")
}

fn automation_runner_stop_request_path() -> PathBuf {
    crate::mvp::config::default_loong_home()
        .join("automation")
        .join("serve.stop-request.json")
}

fn automation_journal_state_path() -> PathBuf {
    crate::mvp::internal_events::internal_event_journal_state_path()
}

fn automation_active_segment_marker_path() -> PathBuf {
    crate::mvp::internal_events::internal_event_active_segment_id_path()
}

fn automation_runner_lease_expires_at_ms(last_heartbeat_at: u64) -> u64 {
    last_heartbeat_at.saturating_add(AUTOMATION_RUNTIME_STALE_MS)
}

fn automation_runner_state_is_stale(
    persisted_state: &PersistedAutomationRunnerState,
    now_ms: u64,
) -> bool {
    if !persisted_state.running {
        return false;
    }
    let lease_expires_at_ms =
        automation_runner_lease_expires_at_ms(persisted_state.last_heartbeat_at);
    now_ms > lease_expires_at_ms
}

fn automation_runner_retention_policy_from_options(
    options: &AutomationServeCommandOptions,
) -> AutomationRunnerRetentionPolicy {
    let retain_min_age_ms = options
        .retain_min_age_seconds
        .map(|seconds| seconds.saturating_mul(1_000));
    AutomationRunnerRetentionPolicy {
        retain_last_sealed_segments: options.retain_last_sealed.unwrap_or_default(),
        retain_min_age_ms,
    }
}

fn load_automation_config(config_path: Option<&str>) -> CliResult<Option<LoadedAutomationConfig>> {
    if let Some(config_path) = config_path {
        let (resolved_path, config) = mvp::config::load(Some(config_path))?;
        return Ok(Some(LoadedAutomationConfig {
            resolved_path,
            config,
        }));
    }

    let default_config_path = mvp::config::default_config_path();
    if !default_config_path.is_file() {
        return Ok(None);
    }

    let (resolved_path, config) = mvp::config::load(None)?;
    Ok(Some(LoadedAutomationConfig {
        resolved_path,
        config,
    }))
}

fn resolved_automation_runner_retention_policy(
    retain_last_sealed_override: Option<usize>,
    retain_min_age_seconds_override: Option<u64>,
    config: Option<&mvp::config::LoongConfig>,
) -> AutomationRunnerRetentionPolicy {
    let retain_last_sealed_segments = retain_last_sealed_override.unwrap_or_else(|| {
        config
            .map(|config| config.automation.retain_last_sealed_segments)
            .unwrap_or_default()
    });
    let retain_min_age_seconds = retain_min_age_seconds_override
        .or_else(|| config.and_then(|config| config.automation.retain_min_age_seconds));
    let retain_min_age_ms = retain_min_age_seconds.map(|seconds| seconds.saturating_mul(1_000));
    AutomationRunnerRetentionPolicy {
        retain_last_sealed_segments,
        retain_min_age_ms,
    }
}

fn resolve_automation_serve_settings(
    options: &AutomationServeCommandOptions,
    config: Option<&mvp::config::LoongConfig>,
) -> ResolvedAutomationServeSettings {
    let event_path = options.path.clone().unwrap_or_else(|| {
        config
            .map(|config| config.automation.resolved_event_path())
            .unwrap_or_else(|| AUTOMATION_DEFAULT_EVENT_PATH.to_owned())
    });
    let poll_ms = options.poll_ms.unwrap_or_else(|| {
        config
            .map(|config| config.automation.resolved_poll_ms())
            .unwrap_or(AUTOMATION_DEFAULT_POLL_MS)
    });
    let retention_policy = resolved_automation_runner_retention_policy(
        options.retain_last_sealed,
        options.retain_min_age_seconds,
        config,
    );
    ResolvedAutomationServeSettings {
        event_path,
        poll_ms,
        retention_policy,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or(0)
}

fn generate_trigger_id() -> String {
    let counter = AUTOMATION_TRIGGER_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("atrg-{:016x}{counter:04x}", now_ms())
}

fn normalize_event_name(raw: &str) -> CliResult<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Err("automation event name must not be empty".to_owned());
    }
    if !normalized
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(
            "automation event names may only contain lowercase ascii letters, digits, `.`, `_`, and `-`"
                .to_owned(),
        );
    }
    Ok(normalized)
}

fn parse_run_at_ms(raw_text: Option<String>, raw_ms: Option<i64>) -> CliResult<i64> {
    match (raw_text, raw_ms) {
        (Some(_), Some(_)) => Err(
            "automation schedule create accepts either --run-at or --run-at-ms, not both"
                .to_owned(),
        ),
        (None, None) => {
            Err("automation schedule create requires --run-at or --run-at-ms".to_owned())
        }
        (None, Some(run_at_ms)) => Ok(run_at_ms),
        (Some(text), None) => {
            let parsed = OffsetDateTime::parse(text.trim(), &Rfc3339)
                .map_err(|error| format!("parse automation --run-at failed: {error}"))?;
            let millis = parsed.unix_timestamp_nanos() / 1_000_000;
            let millis = i64::try_from(millis).map_err(|error| {
                format!("automation --run-at overflowed i64 milliseconds: {error}")
            })?;
            Ok(millis)
        }
    }
}

fn parse_cron_preview_after_ms(raw_text: Option<String>, raw_ms: Option<i64>) -> CliResult<i64> {
    match (raw_text, raw_ms) {
        (Some(_), Some(_)) => {
            Err("automation cron preview accepts either --after or --after-ms, not both".to_owned())
        }
        (None, None) => Ok(now_ms()),
        (None, Some(after_ms)) => Ok(after_ms),
        (Some(text), None) => {
            let parsed = OffsetDateTime::parse(text.trim(), &Rfc3339)
                .map_err(|error| format!("parse automation cron --after failed: {error}"))?;
            let millis = parsed.unix_timestamp_nanos() / 1_000_000;
            let millis = i64::try_from(millis).map_err(|error| {
                format!("automation cron --after overflowed i64 milliseconds: {error}")
            })?;
            Ok(millis)
        }
    }
}

fn ensure_positive_interval(every_seconds: Option<u64>) -> CliResult<Option<u64>> {
    match every_seconds {
        Some(0) => Err("--every-seconds must be greater than zero".to_owned()),
        Some(value) => Ok(Some(value * 1_000)),
        None => Ok(None),
    }
}

fn validate_json_pointer(raw: Option<&str>) -> CliResult<Option<String>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if raw.is_empty() || raw.starts_with('/') {
        return Ok(Some(raw.to_owned()));
    }
    Err("automation event filters require RFC6901 json pointers beginning with `/`".to_owned())
}

fn parse_event_equals_value(
    equals_json: Option<&str>,
    equals_text: Option<String>,
) -> CliResult<Option<Value>> {
    match (
        equals_json.map(str::trim).filter(|value| !value.is_empty()),
        equals_text,
    ) {
        (Some(_), Some(_)) => Err(
            "use either --equals-json or --equals-text for automation event filters, not both"
                .to_owned(),
        ),
        (Some(raw), None) => serde_json::from_str(raw)
            .map(Some)
            .map_err(|error| format!("parse --equals-json failed: {error}")),
        (None, Some(text)) => Ok(Some(Value::String(text))),
        (None, None) => Ok(None),
    }
}

fn validate_event_filter_shape(
    json_pointer: Option<&str>,
    equals_json: Option<&Value>,
    contains_text: Option<&str>,
) -> CliResult<()> {
    if contains_text.is_some() && equals_json.is_some() {
        return Err(
            "use either equality matching or contains_text for automation event filters, not both"
                .to_owned(),
        );
    }
    if (equals_json.is_some() || contains_text.is_some()) && json_pointer.is_none() {
        return Err(
            "automation event filters require --json-pointer when using equality or contains_text matching"
                .to_owned(),
        );
    }
    Ok(())
}

fn cron_field_matches(field: &CronField, value: u8) -> bool {
    if field.any {
        return true;
    }
    field.allowed.contains(&value)
}

fn parse_cron_field(raw: &str, min: u8, max: u8, wrap_sunday: bool) -> CliResult<CronField> {
    let trimmed = raw.trim();
    if trimmed == "*" {
        return Ok(CronField {
            any: true,
            allowed: Vec::new(),
        });
    }

    let mut allowed = Vec::new();
    for part in trimmed.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(format!("invalid cron field `{raw}`"));
        }

        let (range_part, step) = match part.split_once('/') {
            Some((range_part, step_part)) => {
                let step = step_part
                    .trim()
                    .parse::<u8>()
                    .map_err(|error| format!("invalid cron step `{step_part}`: {error}"))?;
                if step == 0 {
                    return Err("cron step must be greater than zero".to_owned());
                }
                (range_part.trim(), step)
            }
            None => (part, 1),
        };

        let (start, end) = if range_part == "*" {
            (min, max)
        } else if let Some((start, end)) = range_part.split_once('-') {
            let start = parse_cron_value(start.trim(), min, max, wrap_sunday)?;
            let end = parse_cron_value(end.trim(), min, max, wrap_sunday)?;
            if start > end {
                return Err(format!(
                    "cron range start `{start}` must be <= end `{end}` in `{part}`"
                ));
            }
            (start, end)
        } else {
            let value = parse_cron_value(range_part, min, max, wrap_sunday)?;
            (value, value)
        };

        let mut candidate = start;
        loop {
            if !allowed.contains(&candidate) {
                allowed.push(candidate);
            }
            let Some(next) = candidate.checked_add(step) else {
                break;
            };
            if next > end {
                break;
            }
            candidate = next;
        }
    }

    allowed.sort_unstable();
    Ok(CronField {
        any: false,
        allowed,
    })
}

fn parse_cron_value(raw: &str, min: u8, max: u8, wrap_sunday: bool) -> CliResult<u8> {
    let mut value = raw
        .parse::<u8>()
        .map_err(|error| format!("invalid cron value `{raw}`: {error}"))?;
    if wrap_sunday && value == 7 {
        value = 0;
    }
    if !(min..=max).contains(&value) {
        return Err(format!("cron value `{value}` out of range {min}..={max}"));
    }
    Ok(value)
}

fn parse_cron_expression(raw: &str) -> CliResult<CronExpression> {
    let fields = raw.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 5 {
        return Err(
            "automation cron expressions currently require 5 fields: minute hour day_of_month month day_of_week"
                .to_owned(),
        );
    }

    let minute_field = fields
        .first()
        .copied()
        .ok_or_else(|| "cron expression missing minute field".to_owned())?;
    let hour_field = fields
        .get(1)
        .copied()
        .ok_or_else(|| "cron expression missing hour field".to_owned())?;
    let day_of_month_field = fields
        .get(2)
        .copied()
        .ok_or_else(|| "cron expression missing day_of_month field".to_owned())?;
    let month_field = fields
        .get(3)
        .copied()
        .ok_or_else(|| "cron expression missing month field".to_owned())?;
    let day_of_week_field = fields
        .get(4)
        .copied()
        .ok_or_else(|| "cron expression missing day_of_week field".to_owned())?;

    Ok(CronExpression {
        minute: parse_cron_field(minute_field, 0, 59, false)?,
        hour: parse_cron_field(hour_field, 0, 23, false)?,
        day_of_month: parse_cron_field(day_of_month_field, 1, 31, false)?,
        month: parse_cron_field(month_field, 1, 12, false)?,
        day_of_week: parse_cron_field(day_of_week_field, 0, 6, true)?,
    })
}

fn next_cron_fire_at_ms(expression: &str, after_ms: i64) -> CliResult<i64> {
    let parsed = parse_cron_expression(expression)?;
    let next_minute_ms = after_ms
        .checked_div(60_000)
        .and_then(|value| value.checked_add(1))
        .and_then(|value| value.checked_mul(60_000))
        .ok_or_else(|| "cron next-fire computation overflowed".to_owned())?;
    let mut candidate =
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(next_minute_ms) * 1_000_000)
            .map_err(|error| format!("cron next-fire anchor failed: {error}"))?;

    let max_attempts = 366 * 24 * 60;
    for _ in 0..max_attempts {
        let minute_matches = cron_field_matches(&parsed.minute, candidate.minute());
        let hour_matches = cron_field_matches(&parsed.hour, candidate.hour());
        let month_matches = cron_field_matches(&parsed.month, candidate.month() as u8);
        let dom_matches = cron_field_matches(&parsed.day_of_month, candidate.day());
        let weekday = candidate.weekday().number_days_from_sunday();
        let dow_matches = cron_field_matches(&parsed.day_of_week, weekday);
        let day_matches = if parsed.day_of_month.any && parsed.day_of_week.any {
            true
        } else if parsed.day_of_month.any {
            dow_matches
        } else if parsed.day_of_week.any {
            dom_matches
        } else {
            dom_matches || dow_matches
        };

        if minute_matches && hour_matches && month_matches && day_matches {
            let millis = candidate.unix_timestamp_nanos() / 1_000_000;
            let millis = i64::try_from(millis)
                .map_err(|error| format!("cron next-fire overflowed i64 milliseconds: {error}"))?;
            return Ok(millis);
        }

        candidate += Duration::from_secs(60);
    }

    Err(format!(
        "automation cron expression `{expression}` did not match within the next 366 days"
    ))
}

fn format_automation_fire_at_rfc3339(fire_at_ms: i64) -> CliResult<String> {
    let fire_at_ns = i128::from(fire_at_ms).saturating_mul(1_000_000);
    let fire_at = OffsetDateTime::from_unix_timestamp_nanos(fire_at_ns)
        .map_err(|error| format!("format automation fire-at timestamp failed: {error}"))?;
    fire_at
        .format(&Rfc3339)
        .map_err(|error| format!("format automation fire-at RFC3339 failed: {error}"))
}

fn execute_cron_command(options: AutomationCronCommandOptions) -> CliResult<Value> {
    match options.command {
        AutomationCronCommands::Preview(command) => execute_cron_preview_command(command),
    }
}

fn execute_cron_preview_command(options: AutomationCronPreviewCommandOptions) -> CliResult<Value> {
    if options.count == 0 {
        return Err("automation cron preview --count must be greater than zero".to_owned());
    }
    if options.count > 20 {
        return Err("automation cron preview --count must be <= 20".to_owned());
    }

    let after_ms = parse_cron_preview_after_ms(options.after, options.after_ms)?;
    let mut preview = Vec::new();
    let mut anchor_ms = after_ms;
    for ordinal in 1..=options.count {
        let fire_at_ms = next_cron_fire_at_ms(options.cron.as_str(), anchor_ms)?;
        let fire_at_rfc3339 = format_automation_fire_at_rfc3339(fire_at_ms)?;
        preview.push(AutomationCronPreviewEntry {
            ordinal,
            fire_at_ms,
            fire_at_rfc3339,
        });
        anchor_ms = fire_at_ms;
    }

    Ok(json!({
        "command": "cron_preview",
        "expression": options.cron,
        "timezone": "UTC",
        "after_ms": after_ms,
        "preview": preview,
    }))
}

impl AutomationRunnerTracker {
    fn acquire(config: Option<&str>, options: &AutomationServeCommandOptions) -> CliResult<Self> {
        let active_owner_path = automation_serve_lock_path();
        let status_snapshot_path = automation_runner_status_snapshot_path();
        let stop_request_path = automation_runner_stop_request_path();
        if active_owner_path.exists() {
            let status = load_automation_runner_status();
            let Some(status) = status else {
                return Err(format!(
                    "automation serve owner state at {} is unreadable; reclaim it before starting a new owner",
                    active_owner_path.display()
                ));
            };
            if status.running && !status.stale {
                return Err(format!(
                    "automation serve owner already active at {}",
                    active_owner_path.display()
                ));
            }
            let reclaim_outcome = reclaim_stale_automation_runner_owner()?;
            if reclaim_outcome != AutomationRunnerReclaimOutcome::Reclaimed {
                return Err(format!(
                    "automation serve owner slot at {} could not be reclaimed",
                    active_owner_path.display()
                ));
            }
        }

        let owner_token = new_automation_runner_owner_token(std::process::id());
        let started_at_ms = u64::try_from(now_ms()).unwrap_or_default();
        let retention_policy = automation_runner_retention_policy_from_options(options);
        let poll_ms = options
            .poll_ms
            .unwrap_or(AUTOMATION_DEFAULT_POLL_MS)
            .max(250);
        let initial_state = PersistedAutomationRunnerState {
            phase: "starting".to_owned(),
            running: true,
            pid: Some(std::process::id()),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            config_path: config.map(ToOwned::to_owned),
            bind_address: options.bind.clone(),
            event_path: options.path.clone(),
            poll_ms,
            retain_last_sealed_segments: retention_policy.retain_last_sealed_segments,
            retain_min_age_ms: retention_policy.retain_min_age_ms,
            started_at_ms,
            last_heartbeat_at: started_at_ms,
            stopped_at_ms: None,
            shutdown_reason: None,
            last_error: None,
            owner_token: owner_token.clone(),
        };
        create_json_path_exclusive(
            active_owner_path.as_path(),
            &initial_state,
            "automation serve owner",
        )?;
        write_json_path(
            status_snapshot_path.as_path(),
            &initial_state,
            "automation serve status snapshot",
        )?;

        Ok(Self {
            active_owner_path,
            status_snapshot_path,
            stop_request_path,
            owner_token,
            state: std::sync::Mutex::new(initial_state),
        })
    }

    fn owner_token(&self) -> &str {
        self.owner_token.as_str()
    }

    fn heartbeat(&self, phase: &str) -> CliResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|error| format!("automation runner state lock poisoned: {error}"))?;
        state.phase = phase.to_owned();
        state.last_heartbeat_at = u64::try_from(now_ms()).unwrap_or_default();
        let persisted_state = state.clone();
        write_automation_runner_active_owner_if_owned(
            self.active_owner_path.as_path(),
            &persisted_state,
            self.owner_token.as_str(),
        )?;
        write_json_path(
            self.status_snapshot_path.as_path(),
            &persisted_state,
            "automation serve status snapshot",
        )
    }

    fn finalize(
        &self,
        phase: &str,
        shutdown_reason: Option<&str>,
        last_error: Option<&str>,
    ) -> CliResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|error| format!("automation runner state lock poisoned: {error}"))?;
        let current_owner_token =
            current_automation_runner_owner_token(self.active_owner_path.as_path());
        if current_owner_token.as_deref() != Some(self.owner_token.as_str()) {
            return Ok(());
        }
        state.phase = phase.to_owned();
        state.running = false;
        state.last_heartbeat_at = u64::try_from(now_ms()).unwrap_or_default();
        state.stopped_at_ms = Some(u64::try_from(now_ms()).unwrap_or_default());
        state.shutdown_reason = shutdown_reason.map(ToOwned::to_owned);
        state.last_error = last_error.map(ToOwned::to_owned);
        let persisted_state = state.clone();
        write_json_path(
            self.status_snapshot_path.as_path(),
            &persisted_state,
            "automation serve status snapshot",
        )?;
        remove_automation_runner_active_owner_if_owned(
            self.active_owner_path.as_path(),
            self.owner_token.as_str(),
        )?;
        remove_automation_runner_stop_request_if_owned(
            self.stop_request_path.as_path(),
            self.owner_token.as_str(),
        )?;
        Ok(())
    }
}

fn load_automation_runner_status() -> Option<AutomationRunnerStatus> {
    let status_snapshot_path = automation_runner_status_snapshot_path();
    let active_owner_path = automation_serve_lock_path();
    let status_snapshot =
        read_json_path::<PersistedAutomationRunnerState>(status_snapshot_path.as_path());
    let active_owner =
        read_json_path::<PersistedAutomationRunnerState>(active_owner_path.as_path());
    let persisted_state = match (status_snapshot, active_owner) {
        (Some(status_snapshot), Some(active_owner)) => {
            if active_owner.last_heartbeat_at >= status_snapshot.last_heartbeat_at {
                active_owner
            } else {
                status_snapshot
            }
        }
        (Some(status_snapshot), None) => status_snapshot,
        (None, Some(active_owner)) => active_owner,
        (None, None) => return None,
    };
    let now_ms = u64::try_from(now_ms()).unwrap_or_default();
    let stale = automation_runner_state_is_stale(&persisted_state, now_ms);
    let lease_expires_at_ms =
        automation_runner_lease_expires_at_ms(persisted_state.last_heartbeat_at);
    Some(AutomationRunnerStatus {
        runtime_dir: crate::mvp::config::default_loong_home()
            .join("automation")
            .display()
            .to_string(),
        phase: persisted_state.phase,
        running: persisted_state.running,
        stale,
        pid: persisted_state.pid,
        version: persisted_state.version,
        config_path: persisted_state.config_path,
        bind_address: persisted_state.bind_address,
        event_path: persisted_state.event_path,
        poll_ms: persisted_state.poll_ms,
        retain_last_sealed_segments: persisted_state.retain_last_sealed_segments,
        retain_min_age_ms: persisted_state.retain_min_age_ms,
        lease_timeout_ms: AUTOMATION_RUNTIME_STALE_MS,
        lease_expires_at_ms,
        started_at_ms: persisted_state.started_at_ms,
        last_heartbeat_at: persisted_state.last_heartbeat_at,
        stopped_at_ms: persisted_state.stopped_at_ms,
        shutdown_reason: persisted_state.shutdown_reason,
        last_error: persisted_state.last_error,
    })
}

fn request_automation_runner_stop() -> CliResult<AutomationRunnerStopRequestOutcome> {
    let active_owner_path = automation_serve_lock_path();
    let stop_request_path = automation_runner_stop_request_path();
    let active_owner =
        read_json_path::<PersistedAutomationRunnerState>(active_owner_path.as_path());
    let Some(active_owner) = active_owner else {
        return Ok(AutomationRunnerStopRequestOutcome::AlreadyStopped);
    };
    let current_status = load_automation_runner_status();
    let Some(current_status) = current_status else {
        return Ok(AutomationRunnerStopRequestOutcome::AlreadyStopped);
    };
    if !current_status.running || current_status.stale {
        return Ok(AutomationRunnerStopRequestOutcome::AlreadyStopped);
    }
    let existing_stop_request =
        read_json_path::<PersistedAutomationStopRequest>(stop_request_path.as_path());
    if existing_stop_request
        .as_ref()
        .is_some_and(|request| request.target_owner_token == active_owner.owner_token)
    {
        return Ok(AutomationRunnerStopRequestOutcome::AlreadyRequested);
    }
    let stop_request = PersistedAutomationStopRequest {
        requested_at_ms: u64::try_from(now_ms()).unwrap_or_default(),
        requested_by_pid: std::process::id(),
        target_owner_token: active_owner.owner_token,
    };
    write_json_path(
        stop_request_path.as_path(),
        &stop_request,
        "automation serve stop request",
    )?;
    Ok(AutomationRunnerStopRequestOutcome::Requested)
}

fn automation_runner_stop_requested(owner_token: &str) -> bool {
    let stop_request_path = automation_runner_stop_request_path();
    let request = read_json_path::<PersistedAutomationStopRequest>(stop_request_path.as_path());
    request
        .as_ref()
        .is_some_and(|request| request.target_owner_token == owner_token)
}

fn current_automation_runner_owner_token(path: &Path) -> Option<String> {
    let persisted_state = read_json_path::<PersistedAutomationRunnerState>(path);
    persisted_state.map(|persisted_state| persisted_state.owner_token)
}

fn create_json_path_exclusive<T>(path: &Path, value: &T, label: &str) -> CliResult<()>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create {label} directory {} failed: {error}",
                parent.display()
            )
        })?;
    }

    let encoded = serde_json::to_string_pretty(value)
        .map_err(|error| format!("serialize {label} failed: {error}"))?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create {label} {} failed: {error}", path.display()))?;
    file.write_all(format!("{encoded}\n").as_bytes())
        .map_err(|error| format!("write {label} {} failed: {error}", path.display()))
}

fn write_automation_runner_active_owner_if_owned(
    path: &Path,
    persisted_state: &PersistedAutomationRunnerState,
    owner_token: &str,
) -> CliResult<()> {
    let current_owner_token = current_automation_runner_owner_token(path);
    if current_owner_token.as_deref() != Some(owner_token) {
        return Err(
            "automation runner ownership changed while the serve loop was still active".to_owned(),
        );
    }
    write_json_path(path, persisted_state, "automation serve owner")
}

fn remove_automation_runner_active_owner_if_owned(path: &Path, owner_token: &str) -> CliResult<()> {
    let current_owner_token = current_automation_runner_owner_token(path);
    if current_owner_token.as_deref() != Some(owner_token) {
        return Ok(());
    }
    fs::remove_file(path).map_err(|error| {
        format!(
            "remove automation serve owner {} failed: {error}",
            path.display()
        )
    })
}

fn remove_automation_runner_stop_request_if_owned(path: &Path, owner_token: &str) -> CliResult<()> {
    let stop_request = read_json_path::<PersistedAutomationStopRequest>(path);
    let Some(stop_request) = stop_request else {
        return Ok(());
    };
    if stop_request.target_owner_token != owner_token {
        return Ok(());
    }
    fs::remove_file(path).map_err(|error| {
        format!(
            "remove automation serve stop request {} failed: {error}",
            path.display()
        )
    })
}

fn reclaim_stale_automation_runner_owner() -> CliResult<AutomationRunnerReclaimOutcome> {
    let active_owner_path = automation_serve_lock_path();
    let status_snapshot_path = automation_runner_status_snapshot_path();
    let stop_request_path = automation_runner_stop_request_path();
    let active_owner =
        read_json_path::<PersistedAutomationRunnerState>(active_owner_path.as_path());
    let status = load_automation_runner_status();

    let Some(status) = status else {
        return Ok(AutomationRunnerReclaimOutcome::AlreadyClean);
    };
    if status.running && !status.stale {
        return Ok(AutomationRunnerReclaimOutcome::OwnerActive);
    }

    let Some(mut persisted_state) = active_owner.or_else(|| {
        read_json_path::<PersistedAutomationRunnerState>(status_snapshot_path.as_path())
    }) else {
        return Ok(AutomationRunnerReclaimOutcome::AlreadyClean);
    };

    let reclaimed_at_ms = u64::try_from(now_ms()).unwrap_or_default();
    persisted_state.phase = "stopped".to_owned();
    persisted_state.running = false;
    persisted_state.last_heartbeat_at = reclaimed_at_ms;
    persisted_state.stopped_at_ms = Some(reclaimed_at_ms);
    persisted_state.shutdown_reason = Some("stale_reclaimed".to_owned());
    persisted_state.last_error = None;

    write_json_path(
        status_snapshot_path.as_path(),
        &persisted_state,
        "automation serve status snapshot",
    )?;
    remove_automation_runner_active_owner_if_owned(
        active_owner_path.as_path(),
        persisted_state.owner_token.as_str(),
    )?;
    if stop_request_path.exists() {
        fs::remove_file(stop_request_path.as_path()).map_err(|error| {
            format!(
                "remove stale automation serve stop request {} failed: {error}",
                stop_request_path.display()
            )
        })?;
    }
    Ok(AutomationRunnerReclaimOutcome::Reclaimed)
}

fn read_json_path<T>(path: &Path) -> Option<T>
where
    T: for<'de> Deserialize<'de>,
{
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(raw.as_str()).ok()
}

fn write_json_path<T>(path: &Path, value: &T, label: &str) -> CliResult<()>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create {label} directory {} failed: {error}",
                parent.display()
            )
        })?;
    }
    let encoded = serde_json::to_string_pretty(value)
        .map_err(|error| format!("serialize {label} failed: {error}"))?;
    let tmp_path = path.with_extension("tmp");
    fs::write(tmp_path.as_path(), format!("{encoded}\n"))
        .map_err(|error| format!("write {label} {} failed: {error}", tmp_path.display()))?;
    fs::rename(tmp_path.as_path(), path).map_err(|error| {
        format!(
            "publish {label} {} from {} failed: {error}",
            path.display(),
            tmp_path.display()
        )
    })
}

fn new_automation_runner_owner_token(process_id: u32) -> String {
    let millis = u64::try_from(now_ms()).unwrap_or_default();
    format!("automation-{process_id:08x}-{millis:016x}")
}

fn load_store(path: &Path) -> CliResult<AutomationStore> {
    if !path.exists() {
        return Ok(AutomationStore::default());
    }
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("read automation store {} failed: {error}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(AutomationStore::default());
    }
    let store: AutomationStore = serde_json::from_str(&raw)
        .map_err(|error| format!("parse automation store {} failed: {error}", path.display()))?;
    Ok(store)
}

fn save_store(path: &Path, store: &AutomationStore) -> CliResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create automation store directory {} failed: {error}",
                parent.display()
            )
        })?;
    }
    let encoded = serde_json::to_vec_pretty(store)
        .map_err(|error| format!("serialize automation store failed: {error}"))?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, encoded).map_err(|error| {
        format!(
            "write automation temp store {} failed: {error}",
            tmp_path.display()
        )
    })?;
    fs::rename(&tmp_path, path).map_err(|error| {
        format!(
            "publish automation store {} from {} failed: {error}",
            path.display(),
            tmp_path.display()
        )
    })?;
    Ok(())
}

async fn execute_create_schedule_command(
    store_path: &Path,
    options: AutomationCreateScheduleCommandOptions,
) -> CliResult<Value> {
    let mut store = load_store(store_path)?;
    let created_at_ms = now_ms();
    let next_fire_at_ms = parse_run_at_ms(options.run_at, options.run_at_ms)?;
    let interval_ms = ensure_positive_interval(options.every_seconds)?;
    let trigger = AutomationTriggerRecord {
        trigger_id: generate_trigger_id(),
        name: options.name,
        status: AutomationTriggerStatus::Active,
        source: AutomationTriggerSource::Schedule {
            schedule: AutomationScheduleSpec {
                next_fire_at_ms,
                interval_ms,
            },
        },
        action: AutomationAction::BackgroundTask {
            background_task: BackgroundTaskAction {
                session: options.session,
                task: options.task,
                label: options.label,
                timeout_seconds: options.timeout_seconds,
            },
        },
        created_at_ms,
        updated_at_ms: created_at_ms,
        last_fired_at_ms: None,
        last_task_id: None,
        last_error: None,
        fire_count: 0,
        run_history: Vec::new(),
    };
    store.triggers.push(trigger.clone());
    save_store(store_path, &store)?;
    Ok(json!({
        "command": "create_schedule",
        "store_path": store_path.display().to_string(),
        "trigger": trigger,
    }))
}

async fn execute_create_cron_command(
    store_path: &Path,
    options: AutomationCreateCronCommandOptions,
) -> CliResult<Value> {
    let mut store = load_store(store_path)?;
    let created_at_ms = now_ms();
    let next_fire_at_ms = next_cron_fire_at_ms(options.cron.as_str(), created_at_ms)?;
    let trigger = AutomationTriggerRecord {
        trigger_id: generate_trigger_id(),
        name: options.name,
        status: AutomationTriggerStatus::Active,
        source: AutomationTriggerSource::Cron {
            cron: AutomationCronSpec {
                expression: options.cron,
                next_fire_at_ms,
            },
        },
        action: AutomationAction::BackgroundTask {
            background_task: BackgroundTaskAction {
                session: options.session,
                task: options.task,
                label: options.label,
                timeout_seconds: options.timeout_seconds,
            },
        },
        created_at_ms,
        updated_at_ms: created_at_ms,
        last_fired_at_ms: None,
        last_task_id: None,
        last_error: None,
        fire_count: 0,
        run_history: Vec::new(),
    };
    store.triggers.push(trigger.clone());
    save_store(store_path, &store)?;
    Ok(json!({
        "command": "create_cron",
        "store_path": store_path.display().to_string(),
        "trigger": trigger,
    }))
}

async fn execute_create_event_command(
    store_path: &Path,
    options: AutomationCreateEventCommandOptions,
) -> CliResult<Value> {
    let mut store = load_store(store_path)?;
    let created_at_ms = now_ms();
    let event_name = normalize_event_name(options.event.as_str())?;
    let json_pointer = validate_json_pointer(options.json_pointer.as_deref())?;
    let equals_json =
        parse_event_equals_value(options.equals_json.as_deref(), options.equals_text)?;
    validate_event_filter_shape(
        json_pointer.as_deref(),
        equals_json.as_ref(),
        options.contains_text.as_deref(),
    )?;
    let trigger = AutomationTriggerRecord {
        trigger_id: generate_trigger_id(),
        name: options.name,
        status: AutomationTriggerStatus::Active,
        source: AutomationTriggerSource::Event {
            event: AutomationEventSpec {
                event_name,
                json_pointer,
                equals_json,
                contains_text: options.contains_text,
            },
        },
        action: AutomationAction::BackgroundTask {
            background_task: BackgroundTaskAction {
                session: options.session,
                task: options.task,
                label: options.label,
                timeout_seconds: options.timeout_seconds,
            },
        },
        created_at_ms,
        updated_at_ms: created_at_ms,
        last_fired_at_ms: None,
        last_task_id: None,
        last_error: None,
        fire_count: 0,
        run_history: Vec::new(),
    };
    store.triggers.push(trigger.clone());
    save_store(store_path, &store)?;
    Ok(json!({
        "command": "create_event",
        "store_path": store_path.display().to_string(),
        "trigger": trigger,
    }))
}

fn execute_list_command(
    store_path: &Path,
    options: AutomationListCommandOptions,
) -> CliResult<Value> {
    let store = load_store(store_path)?;
    let mut triggers = store.triggers;
    if !options.include_completed {
        triggers.retain(|trigger| trigger.status != AutomationTriggerStatus::Completed);
    }
    triggers.truncate(options.limit.clamp(1, 200));
    Ok(json!({
        "command": "list",
        "store_path": store_path.display().to_string(),
        "triggers": triggers,
    }))
}

fn execute_show_command(store_path: &Path, trigger_id: &str) -> CliResult<Value> {
    let store = load_store(store_path)?;
    let trigger = store
        .triggers
        .into_iter()
        .find(|trigger| trigger.trigger_id == trigger_id)
        .ok_or_else(|| format!("automation trigger `{trigger_id}` not found"))?;
    Ok(json!({
        "command": "show",
        "store_path": store_path.display().to_string(),
        "trigger": trigger,
    }))
}

fn execute_remove_command(store_path: &Path, trigger_id: &str) -> CliResult<Value> {
    let mut store = load_store(store_path)?;
    let original_count = store.triggers.len();
    store
        .triggers
        .retain(|trigger| trigger.trigger_id != trigger_id);
    if store.triggers.len() == original_count {
        return Err(format!("automation trigger `{trigger_id}` not found"));
    }
    save_store(store_path, &store)?;
    Ok(json!({
        "command": "remove",
        "store_path": store_path.display().to_string(),
        "trigger_id": trigger_id,
    }))
}

fn execute_status_update_command(
    store_path: &Path,
    trigger_id: &str,
    status: AutomationTriggerStatus,
) -> CliResult<Value> {
    let mut store = load_store(store_path)?;
    let updated_at_ms = now_ms();
    let trigger = store
        .triggers
        .iter_mut()
        .find(|trigger| trigger.trigger_id == trigger_id)
        .ok_or_else(|| format!("automation trigger `{trigger_id}` not found"))?;
    trigger.status = status;
    trigger.updated_at_ms = updated_at_ms;
    let trigger = trigger.clone();
    save_store(store_path, &store)?;
    Ok(json!({
        "command": "status_update",
        "store_path": store_path.display().to_string(),
        "trigger": trigger,
    }))
}

async fn execute_fire_command(
    store_path: &Path,
    config: Option<String>,
    trigger_id: &str,
) -> CliResult<Value> {
    let mut store = load_store(store_path)?;
    let trigger_index = store
        .triggers
        .iter()
        .position(|trigger| trigger.trigger_id == trigger_id)
        .ok_or_else(|| format!("automation trigger `{trigger_id}` not found"))?;
    let result = {
        let trigger = store
            .triggers
            .get_mut(trigger_index)
            .ok_or_else(|| format!("automation trigger `{trigger_id}` not found"))?;
        fire_trigger_record(trigger, config.as_ref()).await
    };
    let result_json = serde_json::to_value(&result)
        .map_err(|error| format!("serialize automation fire result failed: {error}"))?;
    let trigger = store
        .triggers
        .get(trigger_index)
        .cloned()
        .ok_or_else(|| format!("automation trigger `{trigger_id}` disappeared during fire"))?;
    save_store(store_path, &store)?;
    Ok(json!({
        "command": "fire",
        "store_path": store_path.display().to_string(),
        "result": result_json,
        "trigger": trigger,
    }))
}

async fn execute_emit_command(
    store_path: &Path,
    config: Option<String>,
    event_name: &str,
    payload_json: Option<&str>,
) -> CliResult<Value> {
    let normalized_event_name = normalize_event_name(event_name)?;
    let parsed_payload = payload_json
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|error| format!("parse --payload-json failed: {error}"))?;
    execute_emit_value_command(
        store_path,
        config,
        normalized_event_name.as_str(),
        parsed_payload,
    )
    .await
}

async fn execute_emit_value_command(
    store_path: &Path,
    config: Option<String>,
    normalized_event_name: &str,
    payload: Option<Value>,
) -> CliResult<Value> {
    let mut store = load_store(store_path)?;
    let mut results = Vec::new();
    for trigger in &mut store.triggers {
        let matches_event = trigger_matches_event(trigger, normalized_event_name, payload.as_ref());
        if !matches_event {
            continue;
        }
        results.push(Box::pin(fire_trigger_record(trigger, config.as_ref())).await);
    }
    save_store(store_path, &store)?;
    Ok(json!({
        "command": "emit",
        "store_path": store_path.display().to_string(),
        "event_name": normalized_event_name,
        "payload": payload,
        "matched_count": results.len(),
        "results": results,
    }))
}

async fn execute_journal_command(
    config: Option<String>,
    options: AutomationJournalCommandOptions,
) -> CliResult<Value> {
    match options.command {
        AutomationJournalCommands::Inspect(_command) => execute_journal_inspect_command(),
        AutomationJournalCommands::Health(_command) => execute_journal_health_command(),
        AutomationJournalCommands::Rotate(_command) => execute_journal_rotate_command(config).await,
        AutomationJournalCommands::Prune(command) => {
            execute_journal_prune_command(config.as_deref(), command)
        }
        AutomationJournalCommands::Repair(_command) => execute_journal_repair_command(),
    }
}

fn execute_runner_command(options: AutomationRunnerCommandOptions) -> CliResult<Value> {
    match options.command {
        AutomationRunnerCommands::Inspect(_command) => execute_runner_inspect_command(),
        AutomationRunnerCommands::Stop(_command) => execute_runner_stop_command(),
        AutomationRunnerCommands::Reclaim(_command) => execute_runner_reclaim_command(),
    }
}

pub(crate) async fn emit_named_automation_event(
    config: Option<String>,
    event_name: &str,
    payload: Option<Value>,
) -> CliResult<Value> {
    let normalized_event_name = normalize_event_name(event_name)?;
    let store_path = automation_store_path();
    execute_emit_value_command(
        store_path.as_path(),
        config,
        normalized_event_name.as_str(),
        payload,
    )
    .await
}

fn execute_journal_inspect_command() -> CliResult<Value> {
    let cursor_path = automation_event_cursor_path();
    let cursor = load_internal_event_cursor(cursor_path.as_path())?;
    let layout = mvp::internal_events::inspect_internal_event_journal_layout()?;
    Ok(json!({
        "command": "journal_inspect",
        "serve_owner_active": automation_serve_owner_is_active(),
        "cursor_path": cursor_path.display().to_string(),
        "cursor": cursor,
        "layout": layout,
        "state_path": automation_journal_state_path().display().to_string(),
        "active_marker_path": automation_active_segment_marker_path().display().to_string(),
    }))
}

fn execute_journal_health_command() -> CliResult<Value> {
    let inspection = execute_journal_inspect_command()?;
    let layout = inspection
        .get("layout")
        .cloned()
        .ok_or_else(|| "automation journal inspect payload missing layout".to_owned())?;
    let cursor = inspection
        .get("cursor")
        .cloned()
        .ok_or_else(|| "automation journal inspect payload missing cursor".to_owned())?;
    let state_path = automation_journal_state_path();
    let active_marker_path = automation_active_segment_marker_path();
    let state_active_segment_id = fs::read_to_string(state_path.as_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(raw.as_str()).ok())
        .and_then(|value| value.get("active_segment_id").cloned())
        .unwrap_or(Value::Null);
    let active_marker_segment_id = fs::read_to_string(active_marker_path.as_path())
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(Value::String)
        .unwrap_or(Value::Null);
    let cursor_segment_id = cursor.get("segment_id").cloned().unwrap_or(Value::Null);
    let layout_segments = layout
        .get("segments")
        .and_then(Value::as_array)
        .ok_or_else(|| "automation journal inspect payload missing segments".to_owned())?;
    let cursor_segment_exists = cursor_segment_id.as_str().is_some_and(|segment_id| {
        layout_segments.iter().any(|segment| {
            let layout_segment_id = segment.get("segment_id").and_then(Value::as_str);
            layout_segment_id == Some(segment_id)
        })
    });
    let layout_active_segment_id = layout.get("active_segment_id").and_then(Value::as_str);
    let active_segment_exists = layout_active_segment_id.is_some_and(|segment_id| {
        layout_segments.iter().any(|segment| {
            let layout_segment_id = segment.get("segment_id").and_then(Value::as_str);
            layout_segment_id == Some(segment_id)
        })
    });
    let active_marker_matches_state = match (
        active_marker_segment_id.as_str(),
        state_active_segment_id.as_str(),
    ) {
        (Some(marker), Some(state)) => marker == state,
        _ => false,
    };
    Ok(json!({
        "command": "journal_health",
        "inspection": inspection,
        "state_active_segment_id": state_active_segment_id,
        "active_marker_segment_id": active_marker_segment_id,
        "cursor_segment_id": cursor_segment_id,
        "cursor_segment_exists": cursor_segment_exists,
        "active_segment_exists": active_segment_exists,
        "active_marker_matches_state": active_marker_matches_state,
    }))
}

async fn execute_journal_rotate_command(config: Option<String>) -> CliResult<Value> {
    let next_segment_id = mvp::internal_events::rotate_internal_event_journal_segment()?;
    let inspection = execute_journal_inspect_command()?;
    Ok(json!({
        "command": "journal_rotate",
        "next_segment_id": next_segment_id,
        "serve_owner_active": automation_serve_owner_is_active(),
        "inspection": inspection,
        "config": config,
    }))
}

fn execute_journal_prune_command(
    config_path: Option<&str>,
    options: AutomationJournalPruneCommandOptions,
) -> CliResult<Value> {
    let loaded_config = load_automation_config(config_path)?;
    let automation_config = loaded_config.as_ref().map(|loaded| &loaded.config);
    let cursor_path = automation_event_cursor_path();
    let cursor = load_internal_event_cursor(cursor_path.as_path())?;
    let retain_floor_segment_id_option = options
        .retain_segment_id
        .clone()
        .or_else(|| cursor.segment_id.clone());
    let Some(retain_floor_segment_id) = retain_floor_segment_id_option else {
        return Err(
            "automation journal prune requires a retained segment id; use --retain-segment-id when no persisted cursor is available"
                .to_owned(),
        );
    };
    let retain_cursor = mvp::internal_events::InternalEventJournalCursor {
        segment_id: Some(retain_floor_segment_id.clone()),
        ..mvp::internal_events::InternalEventJournalCursor::default()
    };
    let retention_policy = resolved_automation_runner_retention_policy(
        options.retain_last_sealed,
        options.retain_min_age_seconds,
        automation_config,
    );
    let gc_policy = mvp::internal_events::InternalEventJournalGcPolicy {
        retain_floor_segment_id: Some(retain_floor_segment_id.clone()),
        retain_last_sealed_segments: retention_policy.retain_last_sealed_segments,
        retain_min_age_ms: retention_policy.retain_min_age_ms,
    };
    let plan = if options.dry_run {
        mvp::internal_events::plan_internal_event_journal_gc(&gc_policy)?
    } else {
        mvp::internal_events::gc_internal_event_journal_segments(&gc_policy)?
    };
    let pruned_segments = plan
        .decisions
        .iter()
        .filter(|decision| decision.action == "prune")
        .map(|decision| Value::String(decision.segment_id.clone()))
        .collect::<Vec<_>>();
    let inspection = execute_journal_inspect_command()?;
    Ok(json!({
        "command": "journal_prune",
        "dry_run": options.dry_run,
        "cursor_path": cursor_path.display().to_string(),
        "cursor": cursor,
        "retain_cursor": retain_cursor,
        "retain_floor_segment_id": retain_floor_segment_id,
        "retain_last_sealed_segments": retention_policy.retain_last_sealed_segments,
        "retain_min_age_ms": retention_policy.retain_min_age_ms,
        "pruned_segments": pruned_segments,
        "plan": plan,
        "inspection": inspection,
    }))
}

fn execute_journal_repair_command() -> CliResult<Value> {
    let layout = mvp::internal_events::repair_internal_event_journal_state()?;
    let cursor_path = automation_event_cursor_path();
    let cursor = load_internal_event_cursor(cursor_path.as_path())?;
    Ok(json!({
        "command": "journal_repair",
        "serve_owner_active": automation_serve_owner_is_active(),
        "cursor_path": cursor_path.display().to_string(),
        "cursor": cursor,
        "layout": layout,
        "state_path": automation_journal_state_path().display().to_string(),
        "active_marker_path": automation_active_segment_marker_path().display().to_string(),
    }))
}

fn execute_runner_inspect_command() -> CliResult<Value> {
    let status = load_automation_runner_status();
    Ok(json!({
        "command": "runner_inspect",
        "status": status,
        "active_owner_path": automation_serve_lock_path().display().to_string(),
        "status_snapshot_path": automation_runner_status_snapshot_path().display().to_string(),
        "stop_request_path": automation_runner_stop_request_path().display().to_string(),
    }))
}

fn execute_runner_stop_command() -> CliResult<Value> {
    let outcome = request_automation_runner_stop()?;
    let status = load_automation_runner_status();
    Ok(json!({
        "command": "runner_stop",
        "outcome": match outcome {
            AutomationRunnerStopRequestOutcome::Requested => "requested",
            AutomationRunnerStopRequestOutcome::AlreadyRequested => "already_requested",
            AutomationRunnerStopRequestOutcome::AlreadyStopped => "already_stopped",
        },
        "status": status,
        "stop_request_path": automation_runner_stop_request_path().display().to_string(),
    }))
}

fn execute_runner_reclaim_command() -> CliResult<Value> {
    let outcome = reclaim_stale_automation_runner_owner()?;
    let status = load_automation_runner_status();
    Ok(json!({
        "command": "runner_reclaim",
        "outcome": match outcome {
            AutomationRunnerReclaimOutcome::Reclaimed => "reclaimed",
            AutomationRunnerReclaimOutcome::AlreadyClean => "already_clean",
            AutomationRunnerReclaimOutcome::OwnerActive => "owner_active",
        },
        "status": status,
        "active_owner_path": automation_serve_lock_path().display().to_string(),
        "status_snapshot_path": automation_runner_status_snapshot_path().display().to_string(),
        "stop_request_path": automation_runner_stop_request_path().display().to_string(),
    }))
}

pub(crate) async fn publish_daemon_internal_event(
    config: Option<String>,
    event_name: &str,
    source_surface: &str,
    payload: Option<Value>,
) -> CliResult<Value> {
    let augmented_payload = augment_internal_event_payload(event_name, source_surface, payload);
    mvp::internal_events::append_internal_event_to_journal(event_name, &augmented_payload)?;
    if automation_serve_owner_is_active() {
        return Ok(json!({
            "published": true,
            "delivery_mode": "journal_only",
            "event_name": event_name,
        }));
    }
    emit_named_automation_event(config, event_name, Some(augmented_payload)).await
}

pub(crate) fn install_daemon_automation_event_sink(config: Option<String>) {
    let sink = std::sync::Arc::new(move |event_name: &str, payload: Value| {
        let config = config.clone();
        let event_name = event_name.to_owned();
        let handle = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            let Ok(runtime) = runtime else {
                return;
            };
            let _ = runtime.block_on(async move {
                emit_named_automation_event(config, event_name.as_str(), Some(payload)).await
            });
        });
        let _ = handle.join();
    });
    mvp::internal_events::install_internal_event_sink(sink);
}

fn augment_internal_event_payload(
    event_name: &str,
    source_surface: &str,
    payload: Option<Value>,
) -> Value {
    let metadata = json!({
        "event_name": event_name,
        "source_surface": source_surface,
        "emitted_at_ms": now_ms(),
    });

    match payload {
        Some(Value::Object(mut object)) => {
            object.insert("_automation".to_owned(), metadata);
            Value::Object(object)
        }
        Some(other) => json!({
            "_automation": metadata,
            "value": other,
        }),
        None => json!({
            "_automation": metadata,
        }),
    }
}

async fn execute_serve_command(
    store_path: &Path,
    config_path: Option<String>,
    options: AutomationServeCommandOptions,
) -> CliResult<Value> {
    let loaded_config = load_automation_config(config_path.as_deref())?;
    if let Some(loaded_automation_config) = loaded_config.as_ref() {
        mvp::runtime_env::initialize_runtime_environment(
            &loaded_automation_config.config,
            Some(loaded_automation_config.resolved_path.as_path()),
        );
    }
    let automation_config = loaded_config.as_ref().map(|loaded| &loaded.config);
    let resolved_settings = resolve_automation_serve_settings(&options, automation_config);
    let poll_ms = resolved_settings.poll_ms.max(250);
    let retention_policy = resolved_settings.retention_policy.clone();
    let retain_min_age_seconds = retention_policy
        .retain_min_age_ms
        .map(|retain_min_age_ms| retain_min_age_ms / 1_000);
    let runner_options = AutomationServeCommandOptions {
        bind: options.bind.clone(),
        auth_token: options.auth_token.clone(),
        path: Some(resolved_settings.event_path.clone()),
        poll_ms: Some(poll_ms),
        retain_last_sealed: Some(retention_policy.retain_last_sealed_segments),
        retain_min_age_seconds,
    };
    let runner_tracker = AutomationRunnerTracker::acquire(config_path.as_deref(), &runner_options)?;
    let stop_requested = Arc::new(AtomicBool::new(false));
    let scheduler_stop = stop_requested.clone();
    let scheduler_store_path = store_path.to_path_buf();
    let scheduler_config = config_path.clone();
    let scheduler_owner_token = runner_tracker.owner_token().to_owned();
    let finalize_owner_token = scheduler_owner_token.clone();
    let scheduler_runner_tracker = Arc::new(runner_tracker);
    let scheduler_runner_tracker_for_loop = scheduler_runner_tracker.clone();
    let scheduler_retention_policy = retention_policy.clone();
    let scheduler_task = tokio::spawn(async move {
        while !scheduler_stop.load(Ordering::SeqCst) {
            if automation_runner_stop_requested(scheduler_owner_token.as_str()) {
                scheduler_stop.store(true, Ordering::SeqCst);
                break;
            }
            let _ = scheduler_runner_tracker_for_loop.heartbeat("running");
            if let Err(error) = process_due_schedule_triggers(
                scheduler_store_path.as_path(),
                scheduler_config.as_ref(),
            )
            .await
            {
                eprintln!("automation scheduler error: {error}");
            }
            if let Err(error) = process_internal_journal_events_with_policy(
                scheduler_config.as_ref(),
                &scheduler_retention_policy,
            )
            .await
            {
                eprintln!("automation internal event journal error: {error}");
            }
            tokio::time::sleep(Duration::from_millis(
                poll_ms.min(AUTOMATION_RUNTIME_HEARTBEAT_MS),
            ))
            .await;
        }
    });

    let shutdown_requested = stop_requested.clone();
    let shutdown_signal = async move {
        loop {
            if shutdown_requested.load(Ordering::SeqCst) {
                return;
            }
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    shutdown_requested.store(true, Ordering::SeqCst);
                    return;
                }
                _ = tokio::time::sleep(Duration::from_millis(
                    poll_ms.min(AUTOMATION_RUNTIME_HEARTBEAT_MS),
                )) => {}
            }
        }
    };

    if let Some(bind) = options.bind {
        let normalized_path = normalize_event_path(resolved_settings.event_path.as_str())?;
        let route_path = format!("{normalized_path}/:event_name");
        let listener = tokio::net::TcpListener::bind(bind.as_str())
            .await
            .map_err(|error| format!("bind automation webhook listener failed: {error}"))?;
        let state = AutomationServeState {
            config: config_path,
            auth_token: options.auth_token,
        };
        let router = Router::new()
            .route(route_path.as_str(), post(automation_event_webhook_handler))
            .with_state(state);
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal)
            .await
            .map_err(|error| format!("automation webhook server stopped: {error}"))?;
    } else {
        shutdown_signal.await;
    }

    stop_requested.store(true, Ordering::SeqCst);
    let _ = scheduler_task.await;
    let shutdown_reason = if automation_runner_stop_requested(finalize_owner_token.as_str()) {
        Some("stop_requested")
    } else {
        Some("stopped")
    };
    let _ = scheduler_runner_tracker.finalize("stopped", shutdown_reason, None);

    Ok(json!({
        "command": "serve",
        "store_path": store_path.display().to_string(),
        "poll_ms": poll_ms,
        "status": "stopped",
    }))
}

async fn process_internal_journal_events_with_policy(
    config: Option<&String>,
    retention_policy: &AutomationRunnerRetentionPolicy,
) -> CliResult<()> {
    let cursor_path = automation_event_cursor_path();
    let current_cursor = load_internal_event_cursor(cursor_path.as_path())?;
    let (events, next_cursor) =
        mvp::internal_events::read_internal_event_journal_after(current_cursor.clone())?;
    if events.is_empty() {
        if next_cursor != current_cursor {
            store_internal_event_cursor(cursor_path.as_path(), next_cursor)?;
        }
        return Ok(());
    }

    let store_path = automation_store_path();
    let mut store = load_store(store_path.as_path())?;
    let mut changed = false;

    for event in events {
        for trigger in &mut store.triggers {
            if !trigger_matches_event(trigger, event.event_name.as_str(), Some(&event.payload)) {
                continue;
            }
            let _ = fire_trigger_record(trigger, config).await;
            changed = true;
        }
    }

    if changed {
        save_store(store_path.as_path(), &store)?;
    }
    store_internal_event_cursor(cursor_path.as_path(), next_cursor.clone())?;
    if next_cursor.segment_id != current_cursor.segment_id {
        let gc_policy = mvp::internal_events::InternalEventJournalGcPolicy {
            retain_floor_segment_id: next_cursor.segment_id.clone(),
            retain_last_sealed_segments: retention_policy.retain_last_sealed_segments,
            retain_min_age_ms: retention_policy.retain_min_age_ms,
        };
        let _ = mvp::internal_events::gc_internal_event_journal_segments(&gc_policy)?;
    }
    Ok(())
}

fn load_internal_event_cursor(
    path: &Path,
) -> CliResult<mvp::internal_events::InternalEventJournalCursor> {
    if !path.exists() {
        return Ok(mvp::internal_events::InternalEventJournalCursor::default());
    }
    let raw = fs::read_to_string(path).map_err(|error| {
        format!(
            "read automation event cursor {} failed: {error}",
            path.display()
        )
    })?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(mvp::internal_events::InternalEventJournalCursor::default());
    }
    if let Ok(line_cursor) = trimmed.parse::<u64>() {
        return mvp::internal_events::internal_event_journal_cursor_from_line_cursor(line_cursor);
    }
    serde_json::from_str(trimmed).map_err(|error| {
        format!(
            "parse automation event cursor {} failed: {error}",
            path.display()
        )
    })
}

fn store_internal_event_cursor(
    path: &Path,
    cursor: mvp::internal_events::InternalEventJournalCursor,
) -> CliResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create automation event cursor directory {} failed: {error}",
                parent.display()
            )
        })?;
    }
    let encoded = serde_json::to_string_pretty(&cursor)
        .map_err(|error| format!("serialize automation event cursor failed: {error}"))?;
    let tmp_path = path.with_extension("cursor.tmp");
    fs::write(&tmp_path, format!("{encoded}\n")).map_err(|error| {
        format!(
            "write automation temp event cursor {} failed: {error}",
            tmp_path.display()
        )
    })?;
    fs::rename(&tmp_path, path).map_err(|error| {
        format!(
            "publish automation event cursor {} from {} failed: {error}",
            path.display(),
            tmp_path.display()
        )
    })
}

async fn process_due_schedule_triggers(
    store_path: &Path,
    config: Option<&String>,
) -> CliResult<()> {
    let mut store = load_store(store_path)?;
    let now = now_ms();
    let mut changed = false;
    for trigger in &mut store.triggers {
        let is_active = trigger.status == AutomationTriggerStatus::Active;
        let schedule_due = matches!(
            &trigger.source,
            AutomationTriggerSource::Schedule { schedule }
                if schedule.next_fire_at_ms <= now
        );
        let cron_due = matches!(
            &trigger.source,
            AutomationTriggerSource::Cron { cron }
                if cron.next_fire_at_ms <= now
        );
        let is_due = is_active && (schedule_due || cron_due);
        if !is_due {
            continue;
        }
        let _ = fire_trigger_record(trigger, config).await;
        changed = true;
    }
    if changed {
        save_store(store_path, &store)?;
    }
    Ok(())
}

async fn fire_trigger_record(
    trigger: &mut AutomationTriggerRecord,
    config: Option<&String>,
) -> AutomationFireResult {
    let fired_at_ms = now_ms();
    let source_kind = trigger.source.kind_str().to_owned();
    let mut result = AutomationFireResult {
        trigger_id: trigger.trigger_id.clone(),
        name: trigger.name.clone(),
        source_kind,
        queued_task_id: None,
        error: None,
    };

    let queue_outcome = match &trigger.action {
        AutomationAction::BackgroundTask { background_task } => {
            queue_background_task(config.cloned(), background_task).await
        }
    };

    match queue_outcome {
        Ok(task_id) => {
            result.queued_task_id = task_id.clone();
            trigger.last_fired_at_ms = Some(fired_at_ms);
            trigger.last_task_id = task_id;
            trigger.last_error = None;
            trigger.fire_count += 1;
            push_run_history(
                trigger,
                AutomationRunRecord {
                    fired_at_ms,
                    source_kind: result.source_kind.clone(),
                    queued_task_id: result.queued_task_id.clone(),
                    error: None,
                },
            );
            apply_post_fire_schedule_state(trigger, fired_at_ms, /*succeeded*/ true);
        }
        Err(error) => {
            result.error = Some(error.clone());
            trigger.last_error = Some(error);
            push_run_history(
                trigger,
                AutomationRunRecord {
                    fired_at_ms,
                    source_kind: result.source_kind.clone(),
                    queued_task_id: None,
                    error: result.error.clone(),
                },
            );
            apply_post_fire_schedule_state(trigger, fired_at_ms, /*succeeded*/ false);
        }
    }

    trigger.updated_at_ms = fired_at_ms;
    result
}

fn push_run_history(trigger: &mut AutomationTriggerRecord, run: AutomationRunRecord) {
    const MAX_RUN_HISTORY: usize = 10;
    trigger.run_history.push(run);
    if trigger.run_history.len() > MAX_RUN_HISTORY {
        let overflow = trigger.run_history.len() - MAX_RUN_HISTORY;
        trigger.run_history.drain(0..overflow);
    }
}

fn apply_post_fire_schedule_state(
    trigger: &mut AutomationTriggerRecord,
    fired_at_ms: i64,
    succeeded: bool,
) {
    match &mut trigger.source {
        AutomationTriggerSource::Schedule { schedule } => match schedule.interval_ms {
            Some(interval_ms) => {
                let next_fire_at_ms = if succeeded {
                    fired_at_ms.saturating_add(i64::try_from(interval_ms).unwrap_or(i64::MAX))
                } else {
                    fired_at_ms.saturating_add(AUTOMATION_FAILURE_RETRY_MS)
                };
                schedule.next_fire_at_ms = next_fire_at_ms;
            }
            None => {
                if succeeded {
                    trigger.status = AutomationTriggerStatus::Completed;
                } else {
                    schedule.next_fire_at_ms =
                        fired_at_ms.saturating_add(AUTOMATION_FAILURE_RETRY_MS);
                }
            }
        },
        AutomationTriggerSource::Cron { cron } => {
            let recompute_anchor = if succeeded {
                fired_at_ms
            } else {
                fired_at_ms.saturating_add(AUTOMATION_FAILURE_RETRY_MS)
            };
            match next_cron_fire_at_ms(cron.expression.as_str(), recompute_anchor) {
                Ok(next_fire_at_ms) => {
                    cron.next_fire_at_ms = next_fire_at_ms;
                }
                Err(error) => {
                    trigger.last_error = Some(error);
                    trigger.status = AutomationTriggerStatus::Paused;
                }
            }
        }
        AutomationTriggerSource::Event { .. } => {}
    }
}

fn trigger_matches_event(
    trigger: &AutomationTriggerRecord,
    normalized_event_name: &str,
    payload: Option<&Value>,
) -> bool {
    if trigger.status != AutomationTriggerStatus::Active {
        return false;
    }
    let AutomationTriggerSource::Event { event } = &trigger.source else {
        return false;
    };
    if event.event_name != normalized_event_name {
        return false;
    }
    let Some(json_pointer) = event.json_pointer.as_deref() else {
        return true;
    };
    let Some(payload) = payload else {
        return false;
    };
    let Some(actual_value) = payload.pointer(json_pointer) else {
        return false;
    };
    if let Some(expected_value) = event.equals_json.as_ref() {
        return actual_value == expected_value;
    }
    if let Some(needle) = event.contains_text.as_deref() {
        return actual_value
            .as_str()
            .is_some_and(|haystack| haystack.contains(needle));
    }
    true
}

async fn queue_background_task(
    config: Option<String>,
    action: &BackgroundTaskAction,
) -> Result<Option<String>, String> {
    let execution =
        crate::tasks_cli::execute_tasks_command(crate::tasks_cli::TasksCommandOptions {
            config,
            json: true,
            session: action.session.clone(),
            command: crate::tasks_cli::TasksCommands::Create {
                task: action.task.clone(),
                label: action.label.clone(),
                timeout_seconds: action.timeout_seconds,
            },
        })
        .await?;

    let queued_task_id = execution
        .payload
        .pointer("/queued_outcome/child_session_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    Ok(queued_task_id)
}

fn normalize_event_path(raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("automation event path must not be empty".to_owned());
    }
    let with_leading = if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    };
    if with_leading == "/" {
        return Err("automation event path must not be `/`".to_owned());
    }
    Ok(with_leading.trim_end_matches('/').to_owned())
}

fn verify_webhook_token(headers: &HeaderMap, required_token: Option<&str>) -> CliResult<()> {
    let Some(required_token) = required_token else {
        return Ok(());
    };
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if let Some(auth_header) = auth_header
        && let Some(presented) = auth_header.strip_prefix("Bearer ")
        && presented == required_token
    {
        return Ok(());
    }
    let legacy_header = headers
        .get("x-loong-automation-token")
        .and_then(|value| value.to_str().ok());
    if legacy_header == Some(required_token) {
        return Ok(());
    }
    Err("automation webhook token missing or invalid".to_owned())
}

async fn automation_event_webhook_handler(
    State(state): State<AutomationServeState>,
    AxumPath(event_name): AxumPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(error) = verify_webhook_token(&headers, state.auth_token.as_deref()) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": error}))).into_response();
    }

    let store_path = automation_store_path();
    let payload_json = if body.is_empty() {
        None
    } else {
        match std::str::from_utf8(body.as_ref()) {
            Ok(text) => Some(text.to_owned()),
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("automation webhook body is not valid utf-8: {error}")})),
                )
                    .into_response();
            }
        }
    };
    match execute_emit_command(
        store_path.as_path(),
        state.config.clone(),
        event_name.as_str(),
        payload_json.as_deref(),
    )
    .await
    {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, Json(json!({"error": error}))).into_response(),
    }
}

fn render_automation_text(payload: &Value) -> CliResult<String> {
    let command = payload
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| "automation payload missing command".to_owned())?;
    match command {
        "cron_preview" => render_cron_preview(payload),
        "create_schedule" | "create_cron" | "create_event" | "show" | "status_update" => {
            let trigger = payload
                .get("trigger")
                .ok_or_else(|| "automation payload missing trigger".to_owned())?;
            render_trigger_detail(trigger)
        }
        "list" => render_trigger_list(payload),
        "remove" => Ok(format!(
            "Removed automation trigger `{}`.",
            payload["trigger_id"].as_str().unwrap_or("unknown")
        )),
        "fire" => {
            let result = payload
                .get("result")
                .ok_or_else(|| "automation payload missing result".to_owned())?;
            render_fire_result(result)
        }
        "emit" => render_emit_result(payload),
        "journal_inspect" => render_journal_inspect(payload),
        "journal_health" => render_journal_health(payload),
        "journal_rotate" => render_journal_rotate(payload),
        "journal_prune" => render_journal_prune(payload),
        "journal_repair" => render_journal_repair(payload),
        "runner_inspect" => render_runner_inspect(payload),
        "runner_stop" => render_runner_stop(payload),
        "runner_reclaim" => render_runner_reclaim(payload),
        "serve" => Ok("Automation runner stopped.".to_owned()),
        other => Err(format!("unsupported automation render command `{other}`")),
    }
}

fn json_pointer_array<'a>(value: &'a Value, pointer: &str) -> Option<&'a Vec<Value>> {
    value.pointer(pointer).and_then(Value::as_array)
}

fn json_pointer_i64(value: &Value, pointer: &str) -> Option<i64> {
    value.pointer(pointer).and_then(Value::as_i64)
}

fn json_pointer_str<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(Value::as_str)
}

fn json_pointer_u64(value: &Value, pointer: &str) -> Option<u64> {
    value.pointer(pointer).and_then(Value::as_u64)
}

fn json_pointer_value<'a>(value: &'a Value, pointer: &str) -> Option<&'a Value> {
    value.pointer(pointer)
}

fn render_trigger_list(payload: &Value) -> CliResult<String> {
    let triggers = json_pointer_array(payload, "/triggers")
        .ok_or_else(|| "automation list payload missing triggers".to_owned())?;
    if triggers.is_empty() {
        return Ok("No automation triggers found.".to_owned());
    }
    let mut lines = vec!["Automation triggers".to_owned(), String::new()];
    for trigger in triggers {
        let id = json_pointer_str(trigger, "/trigger_id").unwrap_or("unknown");
        let name = json_pointer_str(trigger, "/name").unwrap_or("unnamed");
        let status = json_pointer_str(trigger, "/status").unwrap_or("unknown");
        let kind = json_pointer_str(trigger, "/source/type").unwrap_or("unknown");
        lines.push(format!("- {id} [{status}] {kind} {name}"));
    }
    Ok(lines.join("\n"))
}

fn render_trigger_detail(trigger: &Value) -> CliResult<String> {
    let id = json_pointer_str(trigger, "/trigger_id")
        .ok_or_else(|| "automation trigger missing trigger_id".to_owned())?;
    let name = json_pointer_str(trigger, "/name").unwrap_or("unnamed");
    let status = json_pointer_str(trigger, "/status").unwrap_or("unknown");
    let source_kind = json_pointer_str(trigger, "/source/type").unwrap_or("unknown");
    let mut lines = vec![
        format!("{name} ({id})"),
        format!("status: {status}"),
        format!("source: {source_kind}"),
    ];

    match source_kind {
        "schedule" => {
            let next_fire_at_ms =
                json_pointer_i64(trigger, "/source/schedule/next_fire_at_ms").unwrap_or_default();
            lines.push(format!("next_fire_at_ms: {next_fire_at_ms}"));
            let interval_ms = json_pointer_u64(trigger, "/source/schedule/interval_ms");
            if let Some(interval_ms) = interval_ms {
                lines.push(format!("interval_ms: {interval_ms}"));
            } else {
                lines.push("interval_ms: none".to_owned());
            }
        }
        "event" => {
            let event_name =
                json_pointer_str(trigger, "/source/event/event_name").unwrap_or("unknown");
            lines.push(format!("event: {event_name}"));
            let json_pointer = json_pointer_str(trigger, "/source/event/json_pointer");
            if let Some(pointer) = json_pointer {
                lines.push(format!("json_pointer: {pointer}"));
            }
            let equals_json = json_pointer_value(trigger, "/source/event/equals_json");
            if let Some(equals_json) = equals_json
                && !equals_json.is_null()
            {
                lines.push(format!("equals_json: {equals_json}"));
            }
            let contains_text = json_pointer_str(trigger, "/source/event/contains_text");
            if let Some(contains_text) = contains_text {
                lines.push(format!("contains_text: {contains_text}"));
            }
        }
        "cron" => {
            let expression =
                json_pointer_str(trigger, "/source/cron/expression").unwrap_or("unknown");
            lines.push(format!("cron: {expression}"));
            let next_fire_at_ms =
                json_pointer_i64(trigger, "/source/cron/next_fire_at_ms").unwrap_or_default();
            lines.push(format!("next_fire_at_ms: {next_fire_at_ms}"));
        }
        _ => {}
    }

    let session = json_pointer_str(trigger, "/action/background_task/session").unwrap_or("unknown");
    lines.push(format!("session: {session}"));
    let task = json_pointer_str(trigger, "/action/background_task/task").unwrap_or("");
    lines.push(format!("task: {task}"));
    let label = json_pointer_str(trigger, "/action/background_task/label");
    if let Some(label) = label {
        lines.push(format!("label: {label}"));
    }
    let timeout_seconds = json_pointer_u64(trigger, "/action/background_task/timeout_seconds");
    if let Some(timeout_seconds) = timeout_seconds {
        lines.push(format!("timeout_seconds: {timeout_seconds}"));
    }
    if let Some(last_fired_at_ms) = json_pointer_i64(trigger, "/last_fired_at_ms") {
        lines.push(format!("last_fired_at_ms: {last_fired_at_ms}"));
    }
    if let Some(last_task_id) = json_pointer_str(trigger, "/last_task_id") {
        lines.push(format!("last_task_id: {last_task_id}"));
    }
    if let Some(last_error) = trigger["last_error"].as_str() {
        lines.push(format!("last_error: {last_error}"));
    }
    if let Some(run_history) = trigger["run_history"].as_array() {
        lines.push(format!("run_history_count: {}", run_history.len()));
    }
    Ok(lines.join("\n"))
}

fn render_runner_inspect(payload: &Value) -> CliResult<String> {
    let status = payload
        .get("status")
        .ok_or_else(|| "automation runner inspect payload missing status".to_owned())?;
    if status.is_null() {
        return Ok("No automation runner owner is active.".to_owned());
    }

    let phase = json_pointer_str(status, "/phase").unwrap_or("unknown");
    let running = json_pointer_value(status, "/running")
        .cloned()
        .unwrap_or(Value::Null);
    let stale = json_pointer_value(status, "/stale")
        .cloned()
        .unwrap_or(Value::Null);
    let poll_ms = json_pointer_u64(status, "/poll_ms").unwrap_or_default();
    let lease_timeout_ms = json_pointer_u64(status, "/lease_timeout_ms").unwrap_or_default();
    let lease_expires_at_ms = json_pointer_u64(status, "/lease_expires_at_ms").unwrap_or_default();
    let last_heartbeat_at = json_pointer_u64(status, "/last_heartbeat_at").unwrap_or_default();

    let mut lines = vec![
        "Automation runner".to_owned(),
        String::new(),
        format!("phase: {phase}"),
        format!("running: {running}"),
        format!("stale: {stale}"),
        format!("poll_ms: {poll_ms}"),
        format!("lease_timeout_ms: {lease_timeout_ms}"),
        format!("lease_expires_at_ms: {lease_expires_at_ms}"),
        format!("last_heartbeat_at: {last_heartbeat_at}"),
    ];

    if let Some(bind_address) = json_pointer_str(status, "/bind_address") {
        lines.push(format!("bind_address: {bind_address}"));
    }
    if let Some(event_path) = json_pointer_str(status, "/event_path") {
        lines.push(format!("event_path: {event_path}"));
    }
    let retain_last_sealed_segments =
        json_pointer_u64(status, "/retain_last_sealed_segments").unwrap_or_default();
    let retain_min_age_ms = json_pointer_value(status, "/retain_min_age_ms")
        .cloned()
        .unwrap_or(Value::Null);
    lines.push(format!(
        "retain_last_sealed_segments: {retain_last_sealed_segments}"
    ));
    lines.push(format!("retain_min_age_ms: {retain_min_age_ms}"));
    if let Some(shutdown_reason) = json_pointer_str(status, "/shutdown_reason") {
        lines.push(format!("shutdown_reason: {shutdown_reason}"));
    }
    if let Some(last_error) = json_pointer_str(status, "/last_error") {
        lines.push(format!("last_error: {last_error}"));
    }

    Ok(lines.join("\n"))
}

fn render_runner_stop(payload: &Value) -> CliResult<String> {
    let outcome = payload
        .get("outcome")
        .and_then(Value::as_str)
        .ok_or_else(|| "automation runner stop payload missing outcome".to_owned())?;
    Ok(format!("Automation runner stop outcome: {outcome}."))
}

fn render_runner_reclaim(payload: &Value) -> CliResult<String> {
    let outcome = payload
        .get("outcome")
        .and_then(Value::as_str)
        .ok_or_else(|| "automation runner reclaim payload missing outcome".to_owned())?;
    Ok(format!("Automation runner reclaim outcome: {outcome}."))
}

fn render_cron_preview(payload: &Value) -> CliResult<String> {
    let expression = payload
        .get("expression")
        .and_then(Value::as_str)
        .ok_or_else(|| "automation cron preview payload missing expression".to_owned())?;
    let timezone = payload
        .get("timezone")
        .and_then(Value::as_str)
        .ok_or_else(|| "automation cron preview payload missing timezone".to_owned())?;
    let preview = payload
        .get("preview")
        .and_then(Value::as_array)
        .ok_or_else(|| "automation cron preview payload missing preview".to_owned())?;

    let mut lines = vec![
        "Automation cron preview".to_owned(),
        String::new(),
        format!("expression: {expression}"),
        format!("timezone: {timezone}"),
        String::new(),
        "next fires:".to_owned(),
    ];

    for entry in preview {
        let ordinal = entry
            .get("ordinal")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let fire_at_ms = entry
            .get("fire_at_ms")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let fire_at_rfc3339 = entry
            .get("fire_at_rfc3339")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        lines.push(format!("- #{ordinal}: {fire_at_rfc3339} ({fire_at_ms})"));
    }

    Ok(lines.join("\n"))
}

fn render_fire_result(result: &Value) -> CliResult<String> {
    let id = result["trigger_id"]
        .as_str()
        .ok_or_else(|| "automation fire result missing trigger_id".to_owned())?;
    let name = result["name"].as_str().unwrap_or("unnamed");
    if let Some(error) = result["error"].as_str() {
        return Ok(format!(
            "Automation trigger `{name}` ({id}) failed: {error}"
        ));
    }
    let queued_task_id = result["queued_task_id"].as_str().unwrap_or("unknown");
    Ok(format!(
        "Automation trigger `{name}` ({id}) queued background task `{queued_task_id}`."
    ))
}

fn render_emit_result(payload: &Value) -> CliResult<String> {
    let event_name = payload["event_name"]
        .as_str()
        .ok_or_else(|| "automation emit payload missing event_name".to_owned())?;
    let results = payload["results"]
        .as_array()
        .ok_or_else(|| "automation emit payload missing results".to_owned())?;
    if results.is_empty() {
        return Ok(format!(
            "No active automation triggers matched event `{event_name}`."
        ));
    }
    let mut lines = vec![format!("Matched event `{event_name}`."), String::new()];
    for result in results {
        lines.push(render_fire_result(result)?);
    }
    Ok(lines.join("\n"))
}

fn render_journal_inspect(payload: &Value) -> CliResult<String> {
    let layout = payload
        .get("layout")
        .ok_or_else(|| "automation journal inspect payload missing layout".to_owned())?;
    let active_segment_id = layout["active_segment_id"]
        .as_str()
        .ok_or_else(|| "automation journal inspect payload missing active_segment_id".to_owned())?;
    let segments = layout["segments"]
        .as_array()
        .ok_or_else(|| "automation journal inspect payload missing segments".to_owned())?;
    let mut lines = vec![
        "Automation journal".to_owned(),
        format!("serve_owner_active: {}", payload["serve_owner_active"]),
        format!("active_segment_id: {active_segment_id}"),
        format!("cursor: {}", payload["cursor"]),
        format!(
            "state_path: {}",
            payload["state_path"].as_str().unwrap_or("unknown")
        ),
        format!(
            "active_marker_path: {}",
            payload["active_marker_path"].as_str().unwrap_or("unknown")
        ),
        String::new(),
        "segments:".to_owned(),
    ];
    for segment in segments {
        let segment_id = segment["segment_id"].as_str().unwrap_or("unknown");
        let status = segment["status"].as_str().unwrap_or("unknown");
        let path = segment["path"].as_str().unwrap_or("unknown");
        lines.push(format!("- {segment_id} [{status}] {path}"));
    }
    Ok(lines.join("\n"))
}

fn render_journal_health(payload: &Value) -> CliResult<String> {
    let inspection = payload
        .get("inspection")
        .ok_or_else(|| "automation journal health payload missing inspection".to_owned())?;
    let lines = vec![
        "Automation journal health".to_owned(),
        format!(
            "active_marker_matches_state: {}",
            payload["active_marker_matches_state"]
        ),
        format!(
            "active_segment_exists: {}",
            payload["active_segment_exists"]
        ),
        format!(
            "cursor_segment_exists: {}",
            payload["cursor_segment_exists"]
        ),
        format!(
            "state_active_segment_id: {}",
            payload["state_active_segment_id"]
        ),
        format!(
            "active_marker_segment_id: {}",
            payload["active_marker_segment_id"]
        ),
        format!("cursor_segment_id: {}", payload["cursor_segment_id"]),
        String::new(),
        render_journal_inspect(inspection)?,
    ];
    Ok(lines.join("\n"))
}

fn render_journal_rotate(payload: &Value) -> CliResult<String> {
    let next_segment_id = payload["next_segment_id"]
        .as_str()
        .ok_or_else(|| "automation journal rotate payload missing next_segment_id".to_owned())?;
    let inspection = payload
        .get("inspection")
        .ok_or_else(|| "automation journal rotate payload missing inspection".to_owned())?;
    Ok(format!(
        "Rotated automation journal to `{next_segment_id}`.\n\n{}",
        render_journal_inspect(inspection)?
    ))
}

fn render_journal_prune(payload: &Value) -> CliResult<String> {
    let dry_run = payload
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let pruned_segments = payload["pruned_segments"]
        .as_array()
        .ok_or_else(|| "automation journal prune payload missing pruned_segments".to_owned())?;
    let retain_floor_segment_id = payload
        .get("retain_floor_segment_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let retain_last_sealed_segments = payload
        .get("retain_last_sealed_segments")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let retain_min_age_ms = payload
        .get("retain_min_age_ms")
        .cloned()
        .unwrap_or(Value::Null);
    let inspection = payload
        .get("inspection")
        .ok_or_else(|| "automation journal prune payload missing inspection".to_owned())?;
    let mut header_lines = vec![
        format!("retain_floor_segment_id: {retain_floor_segment_id}"),
        format!("retain_last_sealed_segments: {retain_last_sealed_segments}"),
        format!("retain_min_age_ms: {retain_min_age_ms}"),
        String::new(),
    ];
    if pruned_segments.is_empty() {
        let prefix = if dry_run {
            "Automation journal prune dry-run would not remove any sealed segments.".to_owned()
        } else {
            "No sealed automation journal segments were pruned.".to_owned()
        };
        header_lines.push(render_journal_inspect(inspection)?);
        return Ok(format!("{prefix}\n\n{}", header_lines.join("\n")));
    }
    let pruned_list = pruned_segments
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let prefix = if dry_run {
        format!("Automation journal prune dry-run would remove: {pruned_list}")
    } else {
        format!("Pruned automation journal segments: {pruned_list}")
    };
    header_lines.push(render_journal_inspect(inspection)?);
    Ok(format!("{prefix}\n\n{}", header_lines.join("\n")))
}

fn render_journal_repair(payload: &Value) -> CliResult<String> {
    let layout = payload
        .get("layout")
        .ok_or_else(|| "automation journal repair payload missing layout".to_owned())?;
    Ok(format!(
        "Repaired automation journal state.\n\n{}",
        render_journal_inspect(&json!({
            "command": "journal_inspect",
            "serve_owner_active": payload["serve_owner_active"],
            "cursor_path": payload["cursor_path"],
            "cursor": payload["cursor"],
            "layout": layout,
            "state_path": payload["state_path"],
            "active_marker_path": payload["active_marker_path"],
        }))?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::test_support::ScopedEnv;

    struct TempHomeGuard {
        path: PathBuf,
    }

    impl TempHomeGuard {
        fn new(prefix: &str) -> Self {
            static NEXT_TEMP_HOME_SEED: AtomicUsize = AtomicUsize::new(1);
            let seed = NEXT_TEMP_HOME_SEED.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            let process_id = std::process::id();
            let path = std::env::temp_dir().join(format!("{prefix}-{process_id}-{seed}-{nanos}"));
            fs::create_dir_all(&path).expect("create temp home");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempHomeGuard {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).ok();
        }
    }

    fn isolated_automation_home(prefix: &str) -> (ScopedEnv, TempHomeGuard) {
        let mut env = ScopedEnv::new();
        let temp_home = TempHomeGuard::new(prefix);
        env.set("LOONG_HOME", temp_home.path().as_os_str());
        (env, temp_home)
    }

    fn sample_runner_state(
        owner_token: &str,
        last_heartbeat_at: u64,
    ) -> PersistedAutomationRunnerState {
        PersistedAutomationRunnerState {
            phase: "running".to_owned(),
            running: true,
            pid: Some(42),
            version: "test".to_owned(),
            config_path: Some("/tmp/loong.toml".to_owned()),
            bind_address: None,
            event_path: Some("/automation/events".to_owned()),
            poll_ms: 250,
            retain_last_sealed_segments: 0,
            retain_min_age_ms: None,
            started_at_ms: last_heartbeat_at.saturating_sub(500),
            last_heartbeat_at,
            stopped_at_ms: None,
            shutdown_reason: None,
            last_error: None,
            owner_token: owner_token.to_owned(),
        }
    }

    fn sample_schedule_trigger(interval_ms: Option<u64>) -> AutomationTriggerRecord {
        AutomationTriggerRecord {
            trigger_id: "atrg-test".to_owned(),
            name: "Sample".to_owned(),
            status: AutomationTriggerStatus::Active,
            source: AutomationTriggerSource::Schedule {
                schedule: AutomationScheduleSpec {
                    next_fire_at_ms: 1_000,
                    interval_ms,
                },
            },
            action: AutomationAction::BackgroundTask {
                background_task: BackgroundTaskAction {
                    session: "default".to_owned(),
                    task: "check status".to_owned(),
                    label: None,
                    timeout_seconds: None,
                },
            },
            created_at_ms: 100,
            updated_at_ms: 100,
            last_fired_at_ms: None,
            last_task_id: None,
            last_error: None,
            fire_count: 0,
            run_history: Vec::new(),
        }
    }

    #[test]
    fn normalize_event_name_accepts_stable_event_tokens() {
        let normalized = normalize_event_name("Hook.Task.Completed");
        assert!(normalized.is_ok());
        assert_eq!(normalized.unwrap_or_default(), "hook.task.completed");
    }

    #[test]
    fn normalize_event_name_rejects_invalid_characters() {
        let error = normalize_event_name("hook task").expect_err("spaces should fail");
        assert!(error.contains("may only contain"));
    }

    #[test]
    fn one_shot_schedule_completes_after_successful_fire() {
        let mut trigger = sample_schedule_trigger(None);
        apply_post_fire_schedule_state(&mut trigger, 5_000, true);
        assert_eq!(trigger.status, AutomationTriggerStatus::Completed);
    }

    #[test]
    fn recurring_schedule_reschedules_from_fire_time() {
        let mut trigger = sample_schedule_trigger(Some(30_000));
        apply_post_fire_schedule_state(&mut trigger, 5_000, true);
        let AutomationTriggerSource::Schedule { schedule } = &trigger.source else {
            panic!("expected schedule trigger");
        };
        assert_eq!(schedule.next_fire_at_ms, 35_000);
        assert_eq!(trigger.status, AutomationTriggerStatus::Active);
    }

    #[test]
    fn failed_one_shot_retries_instead_of_completing() {
        let mut trigger = sample_schedule_trigger(None);
        apply_post_fire_schedule_state(&mut trigger, 5_000, false);
        let AutomationTriggerSource::Schedule { schedule } = &trigger.source else {
            panic!("expected schedule trigger");
        };
        assert_eq!(
            schedule.next_fire_at_ms,
            5_000 + AUTOMATION_FAILURE_RETRY_MS
        );
        assert_eq!(trigger.status, AutomationTriggerStatus::Active);
    }

    #[test]
    fn parse_cron_expression_accepts_wildcards_ranges_lists_and_steps() {
        let expression = parse_cron_expression("*/15 9-17 * 1,6,12 1-5");
        assert!(expression.is_ok());
    }

    #[test]
    fn next_cron_fire_finds_future_match() {
        let next = next_cron_fire_at_ms("0 0 1 1 *", 1_700_000_000_000);
        assert!(next.is_ok());
        assert!(next.unwrap_or_default() > 1_700_000_000_000);
    }

    #[test]
    fn execute_cron_preview_command_returns_bounded_future_fire_times() {
        let payload = execute_cron_preview_command(AutomationCronPreviewCommandOptions {
            cron: "0 0 * * *".to_owned(),
            after: None,
            after_ms: Some(1_700_000_000_000),
            count: 3,
        })
        .expect("preview cron expression");

        assert_eq!(payload["command"], "cron_preview");
        assert_eq!(payload["expression"], "0 0 * * *");
        assert_eq!(payload["timezone"], "UTC");
        assert_eq!(payload["preview"].as_array().map(Vec::len), Some(3));
        let first_fire_at_ms = payload["preview"][0]["fire_at_ms"]
            .as_i64()
            .expect("first preview fire_at_ms");
        assert!(first_fire_at_ms > 1_700_000_000_000);
    }

    #[test]
    fn trigger_matches_event_uses_json_pointer_filter() {
        let trigger = AutomationTriggerRecord {
            trigger_id: "atrg-event".to_owned(),
            name: "Filtered".to_owned(),
            status: AutomationTriggerStatus::Active,
            source: AutomationTriggerSource::Event {
                event: AutomationEventSpec {
                    event_name: "work_unit.completed".to_owned(),
                    json_pointer: Some("/work_unit/status".to_owned()),
                    equals_json: Some(Value::String("completed".to_owned())),
                    contains_text: None,
                },
            },
            action: AutomationAction::BackgroundTask {
                background_task: BackgroundTaskAction {
                    session: "default".to_owned(),
                    task: "noop".to_owned(),
                    label: None,
                    timeout_seconds: None,
                },
            },
            created_at_ms: 0,
            updated_at_ms: 0,
            last_fired_at_ms: None,
            last_task_id: None,
            last_error: None,
            fire_count: 0,
            run_history: Vec::new(),
        };

        assert!(trigger_matches_event(
            &trigger,
            "work_unit.completed",
            Some(&json!({"work_unit": {"status": "completed"}})),
        ));
        assert!(!trigger_matches_event(
            &trigger,
            "work_unit.completed",
            Some(&json!({"work_unit": {"status": "failed_terminal"}})),
        ));
    }

    #[test]
    fn trigger_matches_event_supports_pointer_exists_without_equality() {
        let trigger = AutomationTriggerRecord {
            trigger_id: "atrg-exists".to_owned(),
            name: "Exists".to_owned(),
            status: AutomationTriggerStatus::Active,
            source: AutomationTriggerSource::Event {
                event: AutomationEventSpec {
                    event_name: "build.ready".to_owned(),
                    json_pointer: Some("/reason".to_owned()),
                    equals_json: None,
                    contains_text: None,
                },
            },
            action: AutomationAction::BackgroundTask {
                background_task: BackgroundTaskAction {
                    session: "default".to_owned(),
                    task: "noop".to_owned(),
                    label: None,
                    timeout_seconds: None,
                },
            },
            created_at_ms: 0,
            updated_at_ms: 0,
            last_fired_at_ms: None,
            last_task_id: None,
            last_error: None,
            fire_count: 0,
            run_history: Vec::new(),
        };

        assert!(trigger_matches_event(
            &trigger,
            "build.ready",
            Some(&json!({"reason": "ready to ship"})),
        ));
        assert!(!trigger_matches_event(
            &trigger,
            "build.ready",
            Some(&json!({"other": "value"})),
        ));
    }

    #[test]
    fn trigger_matches_event_supports_contains_text() {
        let trigger = AutomationTriggerRecord {
            trigger_id: "atrg-contains".to_owned(),
            name: "Contains".to_owned(),
            status: AutomationTriggerStatus::Active,
            source: AutomationTriggerSource::Event {
                event: AutomationEventSpec {
                    event_name: "build.ready".to_owned(),
                    json_pointer: Some("/reason".to_owned()),
                    equals_json: None,
                    contains_text: Some("ship".to_owned()),
                },
            },
            action: AutomationAction::BackgroundTask {
                background_task: BackgroundTaskAction {
                    session: "default".to_owned(),
                    task: "noop".to_owned(),
                    label: None,
                    timeout_seconds: None,
                },
            },
            created_at_ms: 0,
            updated_at_ms: 0,
            last_fired_at_ms: None,
            last_task_id: None,
            last_error: None,
            fire_count: 0,
            run_history: Vec::new(),
        };

        assert!(trigger_matches_event(
            &trigger,
            "build.ready",
            Some(&json!({"reason": "ready to ship"})),
        ));
        assert!(!trigger_matches_event(
            &trigger,
            "build.ready",
            Some(&json!({"reason": "ready to review"})),
        ));
    }

    #[test]
    fn normalize_event_path_rejects_root_path() {
        let error = normalize_event_path("/").expect_err("root should fail");
        assert!(error.contains("must not be `/`"));
    }

    #[test]
    fn resolve_automation_serve_settings_prefers_cli_over_config_defaults() {
        let mut config = mvp::config::LoongConfig::default();
        config.automation.event_path = "/automation/from-config".to_owned();
        config.automation.poll_ms = 900;
        config.automation.retain_last_sealed_segments = 3;
        config.automation.retain_min_age_seconds = Some(60);

        let options = AutomationServeCommandOptions {
            bind: None,
            auth_token: None,
            path: Some("/automation/from-cli".to_owned()),
            poll_ms: Some(250),
            retain_last_sealed: Some(1),
            retain_min_age_seconds: Some(5),
        };

        let resolved_settings = resolve_automation_serve_settings(&options, Some(&config));

        assert_eq!(resolved_settings.event_path, "/automation/from-cli");
        assert_eq!(resolved_settings.poll_ms, 250);
        assert_eq!(
            resolved_settings
                .retention_policy
                .retain_last_sealed_segments,
            1
        );
        assert_eq!(
            resolved_settings.retention_policy.retain_min_age_ms,
            Some(5_000)
        );
    }

    #[test]
    fn automation_runner_status_reports_stale_lease_expiry() {
        let (_env, _temp_home) = isolated_automation_home("loong-automation-runner-status");
        let last_heartbeat_at = 1;
        let stale_state = sample_runner_state("owner-a", last_heartbeat_at);
        write_json_path(
            automation_serve_lock_path().as_path(),
            &stale_state,
            "automation serve owner",
        )
        .expect("write active owner");

        let status = load_automation_runner_status().expect("load automation runner status");
        assert!(status.stale);
        assert_eq!(status.lease_timeout_ms, AUTOMATION_RUNTIME_STALE_MS);
        assert_eq!(
            status.lease_expires_at_ms,
            automation_runner_lease_expires_at_ms(last_heartbeat_at)
        );
    }

    #[test]
    fn automation_runner_reclaim_clears_stale_owner_and_stop_request() {
        let (_env, _temp_home) = isolated_automation_home("loong-automation-runner-reclaim");
        let stale_state = sample_runner_state("owner-stale", 1);
        write_json_path(
            automation_serve_lock_path().as_path(),
            &stale_state,
            "automation serve owner",
        )
        .expect("write stale owner");
        write_json_path(
            automation_runner_status_snapshot_path().as_path(),
            &stale_state,
            "automation serve status snapshot",
        )
        .expect("write stale snapshot");
        let stop_request = PersistedAutomationStopRequest {
            requested_at_ms: 2,
            requested_by_pid: 77,
            target_owner_token: "owner-stale".to_owned(),
        };
        write_json_path(
            automation_runner_stop_request_path().as_path(),
            &stop_request,
            "automation serve stop request",
        )
        .expect("write stop request");

        let outcome = reclaim_stale_automation_runner_owner().expect("reclaim stale runner");
        assert_eq!(outcome, AutomationRunnerReclaimOutcome::Reclaimed);
        assert!(!automation_serve_lock_path().exists());
        assert!(!automation_runner_stop_request_path().exists());

        let status = load_automation_runner_status().expect("load reclaimed status");
        assert!(!status.running);
        assert_eq!(status.shutdown_reason.as_deref(), Some("stale_reclaimed"));
        assert!(!status.stale);
    }

    #[test]
    fn automation_runner_heartbeat_rejects_lost_ownership() {
        let (_env, _temp_home) = isolated_automation_home("loong-automation-runner-heartbeat");
        let options = AutomationServeCommandOptions {
            bind: None,
            auth_token: None,
            path: Some(AUTOMATION_DEFAULT_EVENT_PATH.to_owned()),
            poll_ms: Some(250),
            retain_last_sealed: Some(0),
            retain_min_age_seconds: None,
        };
        let tracker = AutomationRunnerTracker::acquire(Some("/tmp/loong.toml"), &options)
            .expect("acquire tracker");

        let mut foreign_state = {
            let state_guard = tracker
                .state
                .lock()
                .expect("automation runner state lock should not be poisoned");
            state_guard.clone()
        };
        foreign_state.owner_token = "owner-foreign".to_owned();
        write_json_path(
            automation_serve_lock_path().as_path(),
            &foreign_state,
            "automation serve owner",
        )
        .expect("replace owner slot");

        let error = tracker
            .heartbeat("running")
            .expect_err("lost ownership should fail heartbeat");
        assert!(error.contains("ownership changed"));
    }
}
