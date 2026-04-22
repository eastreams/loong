use std::fs;
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

static AUTOMATION_TRIGGER_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum AutomationCommands {
    /// Create one schedule-based automation trigger
    CreateSchedule(AutomationCreateScheduleCommandOptions),
    /// Create one cron-style automation trigger
    CreateCron(AutomationCreateCronCommandOptions),
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
    #[arg(long, default_value = AUTOMATION_DEFAULT_EVENT_PATH)]
    pub path: String,
    #[arg(long, default_value_t = AUTOMATION_DEFAULT_POLL_MS)]
    pub poll_ms: u64,
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct AutomationJournalCommandOptions {
    #[command(subcommand)]
    pub command: AutomationJournalCommands,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum AutomationJournalCommands {
    /// Inspect the internal automation journal layout and cursor
    Inspect(AutomationJournalInspectCommandOptions),
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
pub struct AutomationJournalRotateCommandOptions {}

#[derive(Args, Debug, Clone, PartialEq, Eq, Default)]
pub struct AutomationJournalPruneCommandOptions {
    #[arg(long)]
    pub retain_segment_id: Option<String>,
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

struct AutomationServeLock {
    path: PathBuf,
}

impl Drop for AutomationServeLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
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
    automation_serve_lock_path().exists()
}

fn automation_event_cursor_path() -> PathBuf {
    crate::mvp::config::default_loong_home()
        .join("automation")
        .join("internal-events.cursor")
}

fn automation_journal_state_path() -> PathBuf {
    crate::mvp::internal_events::internal_event_journal_state_path()
}

fn automation_active_segment_marker_path() -> PathBuf {
    crate::mvp::internal_events::internal_event_active_segment_id_path()
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
    if raw == "" || raw.starts_with('/') {
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
    Ok(CronExpression {
        minute: parse_cron_field(fields[0], 0, 59, false)?,
        hour: parse_cron_field(fields[1], 0, 23, false)?,
        day_of_month: parse_cron_field(fields[2], 1, 31, false)?,
        month: parse_cron_field(fields[3], 1, 12, false)?,
        day_of_week: parse_cron_field(fields[4], 0, 6, true)?,
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
        let weekday = candidate.weekday().number_days_from_sunday() as u8;
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

fn acquire_automation_serve_lock() -> CliResult<AutomationServeLock> {
    let path = automation_serve_lock_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create automation runtime directory {} failed: {error}",
                parent.display()
            )
        })?;
    }

    let write_lock = |path: &Path| -> CliResult<()> {
        let payload = json!({
            "pid": std::process::id(),
            "started_at_ms": now_ms(),
        });
        let bytes = serde_json::to_vec_pretty(&payload)
            .map_err(|error| format!("serialize automation serve lock failed: {error}"))?;
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .and_then(|mut file| std::io::Write::write_all(&mut file, &bytes))
            .map_err(|error| {
                format!(
                    "write automation serve lock {} failed: {error}",
                    path.display()
                )
            })
    };

    match write_lock(path.as_path()) {
        Ok(()) => Ok(AutomationServeLock { path }),
        Err(error) if error.contains("File exists") || error.contains("file exists") => {
            Err(format!(
                "automation serve lock {} already exists; stop the active automation runner or remove the stale lock file manually",
                path.display()
            ))
        }
        Err(error) => Err(format!(
            "automation serve is already active or lock acquisition failed: {error}"
        )),
    }
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
        .map(|raw| serde_json::from_str::<Value>(raw))
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
        AutomationJournalCommands::Rotate(_command) => execute_journal_rotate_command(config).await,
        AutomationJournalCommands::Prune(command) => execute_journal_prune_command(command),
        AutomationJournalCommands::Repair(_command) => execute_journal_repair_command(),
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
    options: AutomationJournalPruneCommandOptions,
) -> CliResult<Value> {
    let cursor_path = automation_event_cursor_path();
    let cursor = load_internal_event_cursor(cursor_path.as_path())?;
    let retain_cursor = if let Some(retain_segment_id) = options.retain_segment_id {
        mvp::internal_events::InternalEventJournalCursor {
            segment_id: Some(retain_segment_id),
            ..mvp::internal_events::InternalEventJournalCursor::default()
        }
    } else {
        cursor.clone()
    };
    let pruned_segments =
        mvp::internal_events::prune_internal_event_journal_segments(&retain_cursor)?;
    let inspection = execute_journal_inspect_command()?;
    Ok(json!({
        "command": "journal_prune",
        "cursor_path": cursor_path.display().to_string(),
        "cursor": cursor,
        "retain_cursor": retain_cursor,
        "pruned_segments": pruned_segments,
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
    let sink_config = config.clone();
    let sink = std::sync::Arc::new(move |event_name: &str, payload: Value| {
        let config = sink_config.clone();
        let event_name = event_name.to_owned();
        let payload = payload.clone();
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
    config: Option<String>,
    options: AutomationServeCommandOptions,
) -> CliResult<Value> {
    let _serve_lock = acquire_automation_serve_lock()?;
    let poll_ms = options.poll_ms.max(250);
    let stop_requested = Arc::new(AtomicBool::new(false));
    let scheduler_stop = stop_requested.clone();
    let scheduler_store_path = store_path.to_path_buf();
    let scheduler_config = config.clone();
    let scheduler_task = tokio::spawn(async move {
        while !scheduler_stop.load(Ordering::SeqCst) {
            if let Err(error) = process_due_schedule_triggers(
                scheduler_store_path.as_path(),
                scheduler_config.as_ref(),
            )
            .await
            {
                eprintln!("automation scheduler error: {error}");
            }
            if let Err(error) = process_internal_journal_events(scheduler_config.as_ref()).await {
                eprintln!("automation internal event journal error: {error}");
            }
            tokio::time::sleep(Duration::from_millis(poll_ms)).await;
        }
    });

    let shutdown_requested = stop_requested.clone();
    let shutdown_signal = async move {
        let _ = tokio::signal::ctrl_c().await;
        shutdown_requested.store(true, Ordering::SeqCst);
    };

    if let Some(bind) = options.bind {
        let normalized_path = normalize_event_path(options.path.as_str())?;
        let route_path = format!("{normalized_path}/:event_name");
        let listener = tokio::net::TcpListener::bind(bind.as_str())
            .await
            .map_err(|error| format!("bind automation webhook listener failed: {error}"))?;
        let state = AutomationServeState {
            config,
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

    Ok(json!({
        "command": "serve",
        "store_path": store_path.display().to_string(),
        "poll_ms": poll_ms,
        "status": "stopped",
    }))
}

async fn process_internal_journal_events(config: Option<&String>) -> CliResult<()> {
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
        let _ = mvp::internal_events::prune_internal_event_journal_segments(&next_cursor)?;
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
        let is_due = trigger.status == AutomationTriggerStatus::Active
            && matches!(
                &trigger.source,
                AutomationTriggerSource::Schedule { schedule }
                    if schedule.next_fire_at_ms <= now
            )
            || trigger.status == AutomationTriggerStatus::Active
                && matches!(
                    &trigger.source,
                    AutomationTriggerSource::Cron { cron }
                        if cron.next_fire_at_ms <= now
                );
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
        "journal_rotate" => render_journal_rotate(payload),
        "journal_prune" => render_journal_prune(payload),
        "journal_repair" => render_journal_repair(payload),
        "serve" => Ok("Automation runner stopped.".to_owned()),
        other => Err(format!("unsupported automation render command `{other}`")),
    }
}

fn render_trigger_list(payload: &Value) -> CliResult<String> {
    let triggers = payload["triggers"]
        .as_array()
        .ok_or_else(|| "automation list payload missing triggers".to_owned())?;
    if triggers.is_empty() {
        return Ok("No automation triggers found.".to_owned());
    }
    let mut lines = vec!["Automation triggers".to_owned(), String::new()];
    for trigger in triggers {
        let id = trigger["trigger_id"].as_str().unwrap_or("unknown");
        let name = trigger["name"].as_str().unwrap_or("unnamed");
        let status = trigger["status"].as_str().unwrap_or("unknown");
        let kind = trigger["source"]["type"].as_str().unwrap_or("unknown");
        lines.push(format!("- {id} [{status}] {kind} {name}"));
    }
    Ok(lines.join("\n"))
}

fn render_trigger_detail(trigger: &Value) -> CliResult<String> {
    let id = trigger["trigger_id"]
        .as_str()
        .ok_or_else(|| "automation trigger missing trigger_id".to_owned())?;
    let name = trigger["name"].as_str().unwrap_or("unnamed");
    let status = trigger["status"].as_str().unwrap_or("unknown");
    let source_kind = trigger["source"]["type"].as_str().unwrap_or("unknown");
    let mut lines = vec![
        format!("{name} ({id})"),
        format!("status: {status}"),
        format!("source: {source_kind}"),
    ];

    match source_kind {
        "schedule" => {
            lines.push(format!(
                "next_fire_at_ms: {}",
                trigger["source"]["schedule"]["next_fire_at_ms"]
                    .as_i64()
                    .unwrap_or_default()
            ));
            if let Some(interval_ms) = trigger["source"]["schedule"]["interval_ms"].as_u64() {
                lines.push(format!("interval_ms: {interval_ms}"));
            } else {
                lines.push("interval_ms: none".to_owned());
            }
        }
        "event" => {
            lines.push(format!(
                "event: {}",
                trigger["source"]["event"]["event_name"]
                    .as_str()
                    .unwrap_or("unknown")
            ));
            if let Some(pointer) = trigger["source"]["event"]["json_pointer"].as_str() {
                lines.push(format!("json_pointer: {pointer}"));
            }
            if !trigger["source"]["event"]["equals_json"].is_null() {
                lines.push(format!(
                    "equals_json: {}",
                    trigger["source"]["event"]["equals_json"]
                ));
            }
            if let Some(contains_text) = trigger["source"]["event"]["contains_text"].as_str() {
                lines.push(format!("contains_text: {contains_text}"));
            }
        }
        "cron" => {
            lines.push(format!(
                "cron: {}",
                trigger["source"]["cron"]["expression"]
                    .as_str()
                    .unwrap_or("unknown")
            ));
            lines.push(format!(
                "next_fire_at_ms: {}",
                trigger["source"]["cron"]["next_fire_at_ms"]
                    .as_i64()
                    .unwrap_or_default()
            ));
        }
        _ => {}
    }

    lines.push(format!(
        "session: {}",
        trigger["action"]["background_task"]["session"]
            .as_str()
            .unwrap_or("unknown")
    ));
    lines.push(format!(
        "task: {}",
        trigger["action"]["background_task"]["task"]
            .as_str()
            .unwrap_or("")
    ));
    if let Some(label) = trigger["action"]["background_task"]["label"].as_str() {
        lines.push(format!("label: {label}"));
    }
    if let Some(timeout_seconds) = trigger["action"]["background_task"]["timeout_seconds"].as_u64()
    {
        lines.push(format!("timeout_seconds: {timeout_seconds}"));
    }
    if let Some(last_fired_at_ms) = trigger["last_fired_at_ms"].as_i64() {
        lines.push(format!("last_fired_at_ms: {last_fired_at_ms}"));
    }
    if let Some(last_task_id) = trigger["last_task_id"].as_str() {
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
    let pruned_segments = payload["pruned_segments"]
        .as_array()
        .ok_or_else(|| "automation journal prune payload missing pruned_segments".to_owned())?;
    let inspection = payload
        .get("inspection")
        .ok_or_else(|| "automation journal prune payload missing inspection".to_owned())?;
    if pruned_segments.is_empty() {
        return Ok(format!(
            "No sealed automation journal segments were pruned.\n\n{}",
            render_journal_inspect(inspection)?
        ));
    }
    let pruned_list = pruned_segments
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!(
        "Pruned automation journal segments: {pruned_list}\n\n{}",
        render_journal_inspect(inspection)?
    ))
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
}
