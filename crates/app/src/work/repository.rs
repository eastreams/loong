use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use loongclaw_contracts::{
    WorkRuntimeHealthSnapshot, WorkUnitEventRecord, WorkUnitKind, WorkUnitLeaseRecord,
    WorkUnitPriority, WorkUnitRecord, WorkUnitRetryPolicy, WorkUnitSnapshot, WorkUnitSourceRef,
    WorkUnitStatus,
};
use rand::random;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};

use crate::memory;
use crate::memory::runtime_config::MemoryRuntimeConfig;

const WORK_UNIT_CREATED_EVENT_KIND: &str = "work_unit_created";
const WORK_UNIT_LEASED_EVENT_KIND: &str = "work_unit_leased";
const WORK_UNIT_STARTED_EVENT_KIND: &str = "work_unit_started";
const WORK_UNIT_HEARTBEAT_EVENT_KIND: &str = "work_unit_heartbeat";
const WORK_UNIT_RETRY_EVENT_KIND: &str = "work_unit_retry_scheduled";
const WORK_UNIT_COMPLETED_EVENT_KIND: &str = "work_unit_completed";
const WORK_UNIT_FAILED_EVENT_KIND: &str = "work_unit_failed_terminal";
const WORK_UNIT_CANCELLED_EVENT_KIND: &str = "work_unit_cancelled";
const WORK_UNIT_ARCHIVED_EVENT_KIND: &str = "work_unit_archived";
const WORK_UNIT_LEASE_EXPIRED_EVENT_KIND: &str = "work_unit_lease_expired_recovered";
const WORK_UNIT_ASSIGNED_EVENT_KIND: &str = "work_unit_assigned";
const WORK_UNIT_DEPENDENCY_ADDED_EVENT_KIND: &str = "work_unit_dependency_added";
const WORK_UNIT_DEPENDENCY_REMOVED_EVENT_KIND: &str = "work_unit_dependency_removed";
const WORK_UNIT_NOTE_ADDED_EVENT_KIND: &str = "work_unit_note_added";
const WORK_UNIT_UPDATED_EVENT_KIND: &str = "work_unit_updated";

#[derive(Debug, Clone, PartialEq)]
pub struct NewWorkUnitRecord {
    pub work_unit_id: Option<String>,
    pub kind: WorkUnitKind,
    pub title: String,
    pub description: String,
    pub source_ref: WorkUnitSourceRef,
    pub status: WorkUnitStatus,
    pub priority: WorkUnitPriority,
    pub retry_policy: WorkUnitRetryPolicy,
    pub parent_work_unit_id: Option<String>,
    pub next_run_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitListQuery {
    pub status: Option<WorkUnitStatus>,
    pub include_archived: bool,
    pub limit: usize,
}

impl Default for WorkUnitListQuery {
    fn default() -> Self {
        Self {
            status: None,
            include_archived: false,
            limit: 100,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquireWorkUnitLeaseRequest {
    pub owner: String,
    pub ttl_ms: u64,
    pub actor: Option<String>,
    pub now_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartWorkUnitLeaseRequest {
    pub work_unit_id: String,
    pub owner: String,
    pub actor: Option<String>,
    pub now_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitHeartbeatRequest {
    pub work_unit_id: String,
    pub owner: String,
    pub ttl_ms: u64,
    pub actor: Option<String>,
    pub now_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkUnitCompletionDisposition {
    Completed,
    RetryPending,
    FailedTerminal,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompleteWorkUnitRequest {
    pub work_unit_id: String,
    pub owner: String,
    pub disposition: WorkUnitCompletionDisposition,
    pub actor: Option<String>,
    pub now_ms: Option<i64>,
    pub next_run_at_ms: Option<i64>,
    pub result_payload_json: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveWorkUnitRequest {
    pub work_unit_id: String,
    pub actor: Option<String>,
    pub now_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignWorkUnitRequest {
    pub work_unit_id: String,
    pub assigned_to: Option<String>,
    pub actor: Option<String>,
    pub now_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddWorkUnitDependencyRequest {
    pub blocking_work_unit_id: String,
    pub blocked_work_unit_id: String,
    pub actor: Option<String>,
    pub now_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveWorkUnitDependencyRequest {
    pub blocking_work_unit_id: String,
    pub blocked_work_unit_id: String,
    pub actor: Option<String>,
    pub now_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendWorkUnitNoteRequest {
    pub work_unit_id: String,
    pub actor: Option<String>,
    pub note: String,
    pub now_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateWorkUnitRequest {
    pub work_unit_id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<WorkUnitStatus>,
    pub priority: Option<WorkUnitPriority>,
    pub next_run_at_ms: Option<i64>,
    pub blocking_reason: Option<String>,
    pub clear_blocking_reason: bool,
    pub actor: Option<String>,
    pub now_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct WorkUnitRepository {
    db_path: PathBuf,
}

#[derive(Debug, Clone)]
struct RawWorkUnitRecord {
    work_unit_id: String,
    kind: String,
    title: String,
    description: String,
    source_ref_json: String,
    status: String,
    priority: String,
    retry_policy_json: String,
    attempt_count: i64,
    next_run_at_ms: i64,
    last_error: Option<String>,
    blocking_reason: Option<String>,
    parent_work_unit_id: Option<String>,
    assigned_to: Option<String>,
    blocks_work_unit_ids: Vec<String>,
    blocked_by_work_unit_ids: Vec<String>,
    result_payload_json: Option<String>,
    lease_owner: Option<String>,
    lease_version: i64,
    lease_acquired_at_ms: Option<i64>,
    lease_heartbeat_at_ms: Option<i64>,
    lease_expires_at_ms: Option<i64>,
    created_at_ms: i64,
    updated_at_ms: i64,
    archived_at_ms: Option<i64>,
}

impl WorkUnitRepository {
    pub fn new(config: &MemoryRuntimeConfig) -> Result<Self, String> {
        let db_path = memory::ensure_memory_db_ready(config.sqlite_path.clone(), config)?;
        let repository = Self { db_path };
        repository.ensure_schema()?;
        Ok(repository)
    }

    pub fn create_work_unit(
        &self,
        record: NewWorkUnitRecord,
        actor: Option<&str>,
    ) -> Result<WorkUnitSnapshot, String> {
        validate_initial_status(record.status)?;
        validate_retry_policy(&record.retry_policy)?;

        let generated_id = record
            .work_unit_id
            .as_deref()
            .map(|value| normalize_required_text(value, "work_unit_id"))
            .transpose()?;
        let work_unit_id = generated_id.unwrap_or_else(generate_work_unit_id);

        let title = normalize_required_text(&record.title, "title")?;
        let description = normalize_required_text(&record.description, "description")?;
        let parent_work_unit_id = normalize_optional_text(record.parent_work_unit_id);
        let source_ref = normalize_source_ref(record.source_ref);
        let source_ref_json = encode_json(&source_ref, "source_ref")?;
        let retry_policy_json = encode_json(&record.retry_policy, "retry_policy")?;
        let now_ms = current_unix_ms();
        let next_run_at_ms = record.next_run_at_ms.unwrap_or(now_ms);
        let priority_rank = priority_rank(record.priority);
        let normalized_actor = normalize_optional_text(actor.map(str::to_owned));

        let mut connection = self.open_connection()?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("open work unit create transaction failed: {error}"))?;

        transaction
            .execute(
                "INSERT INTO work_units(
                    work_unit_id,
                    kind,
                    title,
                    description,
                    source_ref_json,
                    status,
                    priority,
                    priority_rank,
                    retry_policy_json,
                    attempt_count,
                    next_run_at_ms,
                    last_error,
                    blocking_reason,
                    parent_work_unit_id,
                    assigned_to,
                    result_payload_json,
                    lease_owner,
                    lease_version,
                    lease_acquired_at_ms,
                    lease_heartbeat_at_ms,
                    lease_expires_at_ms,
                    created_at_ms,
                    updated_at_ms,
                    archived_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, NULL, NULL, ?11, NULL, NULL, NULL, 0, NULL, NULL, NULL, ?12, ?12, NULL)",
                params![
                    work_unit_id,
                    record.kind.as_str(),
                    title,
                    description,
                    source_ref_json,
                    record.status.as_str(),
                    record.priority.as_str(),
                    priority_rank,
                    retry_policy_json,
                    next_run_at_ms,
                    parent_work_unit_id,
                    now_ms,
                ],
            )
            .map_err(|error| format!("insert work unit row failed: {error}"))?;

        let event_payload = json!({
            "kind": record.kind.as_str(),
            "status": record.status.as_str(),
            "priority": record.priority.as_str(),
            "next_run_at_ms": next_run_at_ms,
            "source_ref": source_ref,
        });
        insert_event_in_tx(
            &transaction,
            &work_unit_id,
            WORK_UNIT_CREATED_EVENT_KIND,
            normalized_actor.as_deref(),
            &event_payload,
            now_ms,
        )?;

        transaction
            .commit()
            .map_err(|error| format!("commit work unit create transaction failed: {error}"))?;

        self.load_work_unit_snapshot(&work_unit_id)?
            .ok_or_else(|| format!("work unit `{work_unit_id}` disappeared after insert"))
    }

    pub fn load_work_unit_snapshot(
        &self,
        work_unit_id: &str,
    ) -> Result<Option<WorkUnitSnapshot>, String> {
        let work_unit_id = normalize_required_text(work_unit_id, "work_unit_id")?;
        let connection = self.open_connection()?;
        let raw = load_raw_work_unit_with_conn(&connection, &work_unit_id)?;
        raw.map(try_work_unit_snapshot_from_raw).transpose()
    }

    pub fn list_work_units(
        &self,
        query: WorkUnitListQuery,
    ) -> Result<Vec<WorkUnitSnapshot>, String> {
        let limit = normalize_limit(query.limit)?;
        let connection = self.open_connection()?;
        let raw_records = load_raw_work_units_with_query(
            &connection,
            query.status,
            query.include_archived,
            limit,
        )?;
        let mut snapshots = Vec::with_capacity(raw_records.len());

        for raw_record in raw_records {
            let snapshot = try_work_unit_snapshot_from_raw(raw_record)?;
            snapshots.push(snapshot);
        }

        Ok(snapshots)
    }

    pub fn list_work_unit_events(
        &self,
        work_unit_id: &str,
        limit: usize,
    ) -> Result<Vec<WorkUnitEventRecord>, String> {
        let work_unit_id = normalize_required_text(work_unit_id, "work_unit_id")?;
        let limit = normalize_limit(limit)?;
        let limit =
            i64::try_from(limit).map_err(|error| format!("event limit overflowed i64: {error}"))?;
        let connection = self.open_connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, work_unit_id, event_kind, actor, payload_json, recorded_at_ms
                 FROM work_unit_events
                 WHERE work_unit_id = ?1
                 ORDER BY id DESC
                 LIMIT ?2",
            )
            .map_err(|error| format!("prepare work unit event query failed: {error}"))?;
        let rows = statement
            .query_map(params![work_unit_id, limit], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(|error| format!("query work unit events failed: {error}"))?;
        let mut events = Vec::new();

        for row in rows {
            let (sequence_id, row_work_unit_id, event_kind, actor, payload_json, recorded_at_ms) =
                row.map_err(|error| format!("decode work unit event row failed: {error}"))?;
            let payload_value = decode_json::<Value>(&payload_json, "work unit event payload")?;
            let event = WorkUnitEventRecord {
                sequence_id,
                work_unit_id: row_work_unit_id,
                event_kind,
                actor,
                payload_json: payload_value,
                recorded_at_ms,
            };
            events.push(event);
        }

        Ok(events)
    }

    pub fn acquire_next_ready_lease(
        &self,
        request: AcquireWorkUnitLeaseRequest,
    ) -> Result<Option<WorkUnitSnapshot>, String> {
        let owner = normalize_required_text(&request.owner, "owner")?;
        validate_ttl_ms(request.ttl_ms)?;
        let actor = normalize_optional_text(request.actor);
        let now_ms = request.now_ms.unwrap_or_else(current_unix_ms);
        let expires_at_ms = add_delay_ms(now_ms, request.ttl_ms)?;
        let mut connection = self.open_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("open work unit lease transaction failed: {error}"))?;
        let Some(raw_record) = select_next_ready_raw_work_unit(&transaction, now_ms)? else {
            return Ok(None);
        };
        let next_lease_version = raw_record.lease_version + 1;

        transaction
            .execute(
                "UPDATE work_units
                 SET status = ?1,
                     attempt_count = attempt_count + 1,
                     lease_owner = ?2,
                     lease_version = ?3,
                     lease_acquired_at_ms = ?4,
                     lease_heartbeat_at_ms = ?4,
                     lease_expires_at_ms = ?5,
                     updated_at_ms = ?4
                 WHERE work_unit_id = ?6
                   AND status IN ('ready', 'retry_pending')
                   AND archived_at_ms IS NULL
                   AND next_run_at_ms <= ?4
                   AND (lease_expires_at_ms IS NULL OR lease_expires_at_ms <= ?4)",
                params![
                    WorkUnitStatus::Leased.as_str(),
                    owner,
                    next_lease_version,
                    now_ms,
                    expires_at_ms,
                    raw_record.work_unit_id,
                ],
            )
            .map_err(|error| format!("update work unit lease state failed: {error}"))?;

        let event_payload = json!({
            "owner": owner,
            "lease_version": next_lease_version,
            "previous_status": raw_record.status,
            "ttl_ms": request.ttl_ms,
            "expires_at_ms": expires_at_ms,
        });
        insert_event_in_tx(
            &transaction,
            &raw_record.work_unit_id,
            WORK_UNIT_LEASED_EVENT_KIND,
            actor.as_deref(),
            &event_payload,
            now_ms,
        )?;

        transaction
            .commit()
            .map_err(|error| format!("commit work unit lease transaction failed: {error}"))?;

        self.load_work_unit_snapshot(&raw_record.work_unit_id)
    }

    pub fn mark_leased_running(
        &self,
        request: StartWorkUnitLeaseRequest,
    ) -> Result<Option<WorkUnitSnapshot>, String> {
        let work_unit_id = normalize_required_text(&request.work_unit_id, "work_unit_id")?;
        let owner = normalize_required_text(&request.owner, "owner")?;
        let actor = normalize_optional_text(request.actor);
        let now_ms = request.now_ms.unwrap_or_else(current_unix_ms);
        let mut connection = self.open_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("open work unit start transaction failed: {error}"))?;
        let Some(raw_record) = load_raw_work_unit_with_conn(&transaction, &work_unit_id)? else {
            return Ok(None);
        };
        if raw_record.status != WorkUnitStatus::Leased.as_str() {
            return Ok(None);
        }
        if raw_record.lease_owner.as_deref() != Some(owner.as_str()) {
            return Ok(None);
        }

        transaction
            .execute(
                "UPDATE work_units
                 SET status = ?1,
                     updated_at_ms = ?2
                 WHERE work_unit_id = ?3
                   AND status = ?4
                   AND lease_owner = ?5",
                params![
                    WorkUnitStatus::Running.as_str(),
                    now_ms,
                    work_unit_id,
                    WorkUnitStatus::Leased.as_str(),
                    owner,
                ],
            )
            .map_err(|error| format!("mark work unit running failed: {error}"))?;

        let event_payload = json!({
            "owner": owner,
            "previous_status": raw_record.status,
        });
        insert_event_in_tx(
            &transaction,
            &work_unit_id,
            WORK_UNIT_STARTED_EVENT_KIND,
            actor.as_deref(),
            &event_payload,
            now_ms,
        )?;

        transaction
            .commit()
            .map_err(|error| format!("commit work unit start transaction failed: {error}"))?;

        self.load_work_unit_snapshot(&work_unit_id)
    }

    pub fn heartbeat_lease(
        &self,
        request: WorkUnitHeartbeatRequest,
    ) -> Result<Option<WorkUnitSnapshot>, String> {
        let work_unit_id = normalize_required_text(&request.work_unit_id, "work_unit_id")?;
        let owner = normalize_required_text(&request.owner, "owner")?;
        validate_ttl_ms(request.ttl_ms)?;
        let actor = normalize_optional_text(request.actor);
        let now_ms = request.now_ms.unwrap_or_else(current_unix_ms);
        let expires_at_ms = add_delay_ms(now_ms, request.ttl_ms)?;
        let mut connection = self.open_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("open work unit heartbeat transaction failed: {error}"))?;
        let Some(raw_record) = load_raw_work_unit_with_conn(&transaction, &work_unit_id)? else {
            return Ok(None);
        };
        let status = raw_record.status.as_str();
        let is_active = status == WorkUnitStatus::Leased.as_str();
        let is_running = status == WorkUnitStatus::Running.as_str();
        if !is_active && !is_running {
            return Ok(None);
        }
        if raw_record.lease_owner.as_deref() != Some(owner.as_str()) {
            return Ok(None);
        }

        transaction
            .execute(
                "UPDATE work_units
                 SET lease_heartbeat_at_ms = ?1,
                     lease_expires_at_ms = ?2,
                     updated_at_ms = ?1
                 WHERE work_unit_id = ?3
                   AND lease_owner = ?4
                   AND status IN ('leased', 'running')",
                params![now_ms, expires_at_ms, work_unit_id, owner],
            )
            .map_err(|error| format!("update work unit heartbeat failed: {error}"))?;

        let event_payload = json!({
            "owner": owner,
            "ttl_ms": request.ttl_ms,
            "expires_at_ms": expires_at_ms,
        });
        insert_event_in_tx(
            &transaction,
            &work_unit_id,
            WORK_UNIT_HEARTBEAT_EVENT_KIND,
            actor.as_deref(),
            &event_payload,
            now_ms,
        )?;

        transaction
            .commit()
            .map_err(|error| format!("commit work unit heartbeat transaction failed: {error}"))?;

        self.load_work_unit_snapshot(&work_unit_id)
    }

    pub fn complete_work_unit(
        &self,
        request: CompleteWorkUnitRequest,
    ) -> Result<Option<WorkUnitSnapshot>, String> {
        let work_unit_id = normalize_required_text(&request.work_unit_id, "work_unit_id")?;
        let owner = normalize_required_text(&request.owner, "owner")?;
        let actor = normalize_optional_text(request.actor);
        let now_ms = request.now_ms.unwrap_or_else(current_unix_ms);
        let mut connection = self.open_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("open work unit complete transaction failed: {error}"))?;
        let Some(raw_record) = load_raw_work_unit_with_conn(&transaction, &work_unit_id)? else {
            return Ok(None);
        };
        let status = raw_record.status.as_str();
        let is_leased = status == WorkUnitStatus::Leased.as_str();
        let is_running = status == WorkUnitStatus::Running.as_str();
        if !is_leased && !is_running {
            return Ok(None);
        }
        if raw_record.lease_owner.as_deref() != Some(owner.as_str()) {
            return Ok(None);
        }

        let retry_policy =
            decode_json::<WorkUnitRetryPolicy>(&raw_record.retry_policy_json, "retry policy")?;
        let attempt_count = u32::try_from(raw_record.attempt_count)
            .map_err(|error| format!("attempt_count overflowed u32: {error}"))?;
        let error = normalize_optional_text(request.error);
        let result_payload_json = request
            .result_payload_json
            .as_ref()
            .map(|value| encode_json(value, "result_payload"))
            .transpose()?;
        let completion = resolve_completion(
            request.disposition,
            &retry_policy,
            attempt_count,
            now_ms,
            request.next_run_at_ms,
            error.as_deref(),
        )?;

        transaction
            .execute(
                "UPDATE work_units
                 SET status = ?1,
                     next_run_at_ms = ?2,
                     last_error = ?3,
                     blocking_reason = NULL,
                     result_payload_json = ?4,
                     lease_owner = NULL,
                     lease_acquired_at_ms = NULL,
                     lease_heartbeat_at_ms = NULL,
                     lease_expires_at_ms = NULL,
                     updated_at_ms = ?5
                 WHERE work_unit_id = ?6
                   AND lease_owner = ?7
                   AND status IN ('leased', 'running')",
                params![
                    completion.status.as_str(),
                    completion.next_run_at_ms,
                    completion.last_error,
                    result_payload_json,
                    now_ms,
                    work_unit_id,
                    owner,
                ],
            )
            .map_err(|error| format!("update completed work unit failed: {error}"))?;

        let event_payload = json!({
            "owner": owner,
            "previous_status": raw_record.status,
            "next_status": completion.status.as_str(),
            "next_run_at_ms": completion.next_run_at_ms,
            "last_error": completion.last_error,
            "attempt_count": attempt_count,
        });
        insert_event_in_tx(
            &transaction,
            &work_unit_id,
            completion.event_kind,
            actor.as_deref(),
            &event_payload,
            now_ms,
        )?;

        transaction
            .commit()
            .map_err(|error| format!("commit work unit complete transaction failed: {error}"))?;

        self.load_work_unit_snapshot(&work_unit_id)
    }

    pub fn archive_work_unit(
        &self,
        request: ArchiveWorkUnitRequest,
    ) -> Result<Option<WorkUnitSnapshot>, String> {
        let work_unit_id = normalize_required_text(&request.work_unit_id, "work_unit_id")?;
        let actor = normalize_optional_text(request.actor);
        let now_ms = request.now_ms.unwrap_or_else(current_unix_ms);
        let mut connection = self.open_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("open work unit archive transaction failed: {error}"))?;
        let Some(raw_record) = load_raw_work_unit_with_conn(&transaction, &work_unit_id)? else {
            return Ok(None);
        };
        let current_status = WorkUnitStatus::parse(&raw_record.status)
            .ok_or_else(|| format!("unknown work unit status `{}`", raw_record.status))?;
        if !current_status.is_terminal() {
            return Ok(None);
        }
        if raw_record.archived_at_ms.is_some() {
            return Ok(None);
        }

        transaction
            .execute(
                "UPDATE work_units
                 SET status = ?1,
                     archived_at_ms = ?2,
                     updated_at_ms = ?2
                 WHERE work_unit_id = ?3
                   AND archived_at_ms IS NULL",
                params![WorkUnitStatus::Archived.as_str(), now_ms, work_unit_id],
            )
            .map_err(|error| format!("archive work unit failed: {error}"))?;

        let event_payload = json!({
            "previous_status": raw_record.status,
            "archived_at_ms": now_ms,
        });
        insert_event_in_tx(
            &transaction,
            &work_unit_id,
            WORK_UNIT_ARCHIVED_EVENT_KIND,
            actor.as_deref(),
            &event_payload,
            now_ms,
        )?;

        transaction
            .commit()
            .map_err(|error| format!("commit work unit archive transaction failed: {error}"))?;

        self.load_work_unit_snapshot(&work_unit_id)
    }

    pub fn update_work_unit(
        &self,
        request: UpdateWorkUnitRequest,
    ) -> Result<Option<WorkUnitSnapshot>, String> {
        let work_unit_id = normalize_required_text(&request.work_unit_id, "work_unit_id")?;
        let actor = normalize_optional_text(request.actor);
        let now_ms = request.now_ms.unwrap_or_else(current_unix_ms);
        let mut connection = self.open_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("open work unit update transaction failed: {error}"))?;
        let Some(raw_record) = load_raw_work_unit_with_conn(&transaction, &work_unit_id)? else {
            return Ok(None);
        };
        if raw_record.archived_at_ms.is_some() {
            return Ok(None);
        }

        let current_status = WorkUnitStatus::parse(&raw_record.status)
            .ok_or_else(|| format!("unknown work unit status `{}`", raw_record.status))?;
        let mut changed_fields = Vec::new();

        let next_title = match request.title {
            Some(title) => {
                let title = normalize_required_text(title.as_str(), "title")?;
                if title != raw_record.title {
                    changed_fields.push("title".to_owned());
                }
                title
            }
            None => raw_record.title.clone(),
        };

        let next_description = match request.description {
            Some(description) => {
                let description = normalize_required_text(description.as_str(), "description")?;
                if description != raw_record.description {
                    changed_fields.push("description".to_owned());
                }
                description
            }
            None => raw_record.description.clone(),
        };

        let next_status = match request.status {
            Some(status) => {
                validate_manual_update_status(current_status, status)?;
                let status_label = status.as_str().to_owned();
                if status_label != raw_record.status {
                    changed_fields.push("status".to_owned());
                }
                status_label
            }
            None => raw_record.status.clone(),
        };

        let next_priority = match request.priority {
            Some(priority) => {
                let priority_label = priority.as_str().to_owned();
                if priority_label != raw_record.priority {
                    changed_fields.push("priority".to_owned());
                }
                priority_label
            }
            None => raw_record.priority.clone(),
        };
        let next_priority_rank = priority_rank(
            WorkUnitPriority::parse(next_priority.as_str())
                .ok_or_else(|| format!("unknown work unit priority `{}`", next_priority))?,
        );

        let next_next_run_at_ms = match request.next_run_at_ms {
            Some(next_run_at_ms) => {
                if next_run_at_ms != raw_record.next_run_at_ms {
                    changed_fields.push("next_run_at_ms".to_owned());
                }
                next_run_at_ms
            }
            None => raw_record.next_run_at_ms,
        };

        let explicit_blocking_reason = request
            .blocking_reason
            .map(Some)
            .unwrap_or_else(|| raw_record.blocking_reason.clone());
        let normalized_blocking_reason = normalize_optional_text(explicit_blocking_reason);
        let next_blocking_reason = if request.clear_blocking_reason {
            None
        } else {
            normalized_blocking_reason
        };
        if next_blocking_reason != raw_record.blocking_reason {
            changed_fields.push("blocking_reason".to_owned());
        }

        if changed_fields.is_empty() {
            transaction.commit().map_err(|error| {
                format!("commit unchanged work unit update transaction failed: {error}")
            })?;
            return self.load_work_unit_snapshot(&work_unit_id);
        }

        transaction
            .execute(
                "UPDATE work_units
                 SET title = ?1,
                     description = ?2,
                     status = ?3,
                     priority = ?4,
                     priority_rank = ?5,
                     next_run_at_ms = ?6,
                     blocking_reason = ?7,
                     updated_at_ms = ?8
                 WHERE work_unit_id = ?9
                   AND archived_at_ms IS NULL",
                params![
                    next_title,
                    next_description,
                    next_status,
                    next_priority,
                    next_priority_rank,
                    next_next_run_at_ms,
                    next_blocking_reason,
                    now_ms,
                    work_unit_id,
                ],
            )
            .map_err(|error| format!("update work unit fields failed: {error}"))?;

        let event_payload = json!({
            "changed_fields": changed_fields,
            "previous": {
                "title": raw_record.title,
                "description": raw_record.description,
                "status": raw_record.status,
                "priority": raw_record.priority,
                "next_run_at_ms": raw_record.next_run_at_ms,
                "blocking_reason": raw_record.blocking_reason,
            },
            "current": {
                "title": next_title,
                "description": next_description,
                "status": next_status,
                "priority": next_priority,
                "next_run_at_ms": next_next_run_at_ms,
                "blocking_reason": next_blocking_reason,
            }
        });
        insert_event_in_tx(
            &transaction,
            &work_unit_id,
            WORK_UNIT_UPDATED_EVENT_KIND,
            actor.as_deref(),
            &event_payload,
            now_ms,
        )?;

        transaction
            .commit()
            .map_err(|error| format!("commit work unit update transaction failed: {error}"))?;

        self.load_work_unit_snapshot(&work_unit_id)
    }

    pub fn assign_work_unit(
        &self,
        request: AssignWorkUnitRequest,
    ) -> Result<Option<WorkUnitSnapshot>, String> {
        let work_unit_id = normalize_required_text(&request.work_unit_id, "work_unit_id")?;
        let assigned_to = normalize_optional_text(request.assigned_to);
        let actor = normalize_optional_text(request.actor);
        let now_ms = request.now_ms.unwrap_or_else(current_unix_ms);
        let mut connection = self.open_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("open work unit assignment transaction failed: {error}"))?;
        let Some(raw_record) = load_raw_work_unit_with_conn(&transaction, &work_unit_id)? else {
            return Ok(None);
        };
        if raw_record.archived_at_ms.is_some() {
            return Ok(None);
        }

        let previous_assigned_to = raw_record.assigned_to;
        let changed = previous_assigned_to != assigned_to;
        if !changed {
            transaction.commit().map_err(|error| {
                format!("commit unchanged work unit assignment transaction failed: {error}")
            })?;
            return self.load_work_unit_snapshot(&work_unit_id);
        }

        transaction
            .execute(
                "UPDATE work_units
                 SET assigned_to = ?1,
                     updated_at_ms = ?2
                 WHERE work_unit_id = ?3
                   AND archived_at_ms IS NULL",
                params![assigned_to, now_ms, work_unit_id],
            )
            .map_err(|error| format!("assign work unit failed: {error}"))?;

        let event_payload = json!({
            "previous_assigned_to": previous_assigned_to,
            "assigned_to": assigned_to,
        });
        insert_event_in_tx(
            &transaction,
            &work_unit_id,
            WORK_UNIT_ASSIGNED_EVENT_KIND,
            actor.as_deref(),
            &event_payload,
            now_ms,
        )?;

        transaction
            .commit()
            .map_err(|error| format!("commit work unit assignment transaction failed: {error}"))?;

        self.load_work_unit_snapshot(&work_unit_id)
    }

    pub fn add_dependency(
        &self,
        request: AddWorkUnitDependencyRequest,
    ) -> Result<Option<WorkUnitSnapshot>, String> {
        let blocking_work_unit_id =
            normalize_required_text(&request.blocking_work_unit_id, "blocking_work_unit_id")?;
        let blocked_work_unit_id =
            normalize_required_text(&request.blocked_work_unit_id, "blocked_work_unit_id")?;
        let actor = normalize_optional_text(request.actor);
        let now_ms = request.now_ms.unwrap_or_else(current_unix_ms);
        validate_dependency_endpoints(
            blocking_work_unit_id.as_str(),
            blocked_work_unit_id.as_str(),
        )?;
        let mut connection = self.open_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("open work unit dependency transaction failed: {error}"))?;
        ensure_work_unit_exists(&transaction, blocking_work_unit_id.as_str())?;
        ensure_work_unit_exists(&transaction, blocked_work_unit_id.as_str())?;
        let creates_cycle = would_create_dependency_cycle(
            &transaction,
            blocking_work_unit_id.as_str(),
            blocked_work_unit_id.as_str(),
        )?;
        if creates_cycle {
            return Err(format!(
                "work unit dependency would create a cycle: `{}` -> `{}`",
                blocking_work_unit_id, blocked_work_unit_id
            ));
        }

        let inserted_rows = transaction
            .execute(
                "INSERT OR IGNORE INTO work_unit_dependencies(
                    blocking_work_unit_id,
                    blocked_work_unit_id,
                    created_at_ms,
                    created_by
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    blocking_work_unit_id,
                    blocked_work_unit_id,
                    now_ms,
                    actor.as_deref(),
                ],
            )
            .map_err(|error| format!("insert work unit dependency failed: {error}"))?;

        if inserted_rows > 0 {
            touch_work_unit(&transaction, blocked_work_unit_id.as_str(), now_ms)?;
            let event_payload = json!({
                "blocking_work_unit_id": blocking_work_unit_id,
                "blocked_work_unit_id": blocked_work_unit_id,
            });
            insert_event_in_tx(
                &transaction,
                blocked_work_unit_id.as_str(),
                WORK_UNIT_DEPENDENCY_ADDED_EVENT_KIND,
                actor.as_deref(),
                &event_payload,
                now_ms,
            )?;
        }

        transaction
            .commit()
            .map_err(|error| format!("commit work unit dependency transaction failed: {error}"))?;

        self.load_work_unit_snapshot(&blocked_work_unit_id)
    }

    pub fn remove_dependency(
        &self,
        request: RemoveWorkUnitDependencyRequest,
    ) -> Result<Option<WorkUnitSnapshot>, String> {
        let blocking_work_unit_id =
            normalize_required_text(&request.blocking_work_unit_id, "blocking_work_unit_id")?;
        let blocked_work_unit_id =
            normalize_required_text(&request.blocked_work_unit_id, "blocked_work_unit_id")?;
        let actor = normalize_optional_text(request.actor);
        let now_ms = request.now_ms.unwrap_or_else(current_unix_ms);
        validate_dependency_endpoints(
            blocking_work_unit_id.as_str(),
            blocked_work_unit_id.as_str(),
        )?;
        let mut connection = self.open_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| {
                format!("open work unit dependency removal transaction failed: {error}")
            })?;
        let Some(_raw_record) = load_raw_work_unit_with_conn(&transaction, &blocked_work_unit_id)?
        else {
            return Ok(None);
        };

        let removed_rows = transaction
            .execute(
                "DELETE FROM work_unit_dependencies
                 WHERE blocking_work_unit_id = ?1
                   AND blocked_work_unit_id = ?2",
                params![blocking_work_unit_id, blocked_work_unit_id],
            )
            .map_err(|error| format!("remove work unit dependency failed: {error}"))?;

        if removed_rows > 0 {
            touch_work_unit(&transaction, blocked_work_unit_id.as_str(), now_ms)?;
            let event_payload = json!({
                "blocking_work_unit_id": blocking_work_unit_id,
                "blocked_work_unit_id": blocked_work_unit_id,
            });
            insert_event_in_tx(
                &transaction,
                blocked_work_unit_id.as_str(),
                WORK_UNIT_DEPENDENCY_REMOVED_EVENT_KIND,
                actor.as_deref(),
                &event_payload,
                now_ms,
            )?;
        }

        transaction.commit().map_err(|error| {
            format!("commit work unit dependency removal transaction failed: {error}")
        })?;

        self.load_work_unit_snapshot(&blocked_work_unit_id)
    }

    pub fn append_note(
        &self,
        request: AppendWorkUnitNoteRequest,
    ) -> Result<Option<WorkUnitEventRecord>, String> {
        let work_unit_id = normalize_required_text(&request.work_unit_id, "work_unit_id")?;
        let note = normalize_required_text(&request.note, "note")?;
        let actor = normalize_optional_text(request.actor);
        let now_ms = request.now_ms.unwrap_or_else(current_unix_ms);
        let mut connection = self.open_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("open work unit note transaction failed: {error}"))?;
        let Some(raw_record) = load_raw_work_unit_with_conn(&transaction, &work_unit_id)? else {
            return Ok(None);
        };
        if raw_record.archived_at_ms.is_some() {
            return Ok(None);
        }

        touch_work_unit(&transaction, work_unit_id.as_str(), now_ms)?;
        let event_payload = json!({
            "note": note,
        });
        let event = insert_event_in_tx(
            &transaction,
            work_unit_id.as_str(),
            WORK_UNIT_NOTE_ADDED_EVENT_KIND,
            actor.as_deref(),
            &event_payload,
            now_ms,
        )?;
        transaction
            .commit()
            .map_err(|error| format!("commit work unit note transaction failed: {error}"))?;

        Ok(Some(event))
    }

    pub fn recover_expired_leases(
        &self,
        actor: Option<&str>,
        now_ms: Option<i64>,
    ) -> Result<Vec<WorkUnitSnapshot>, String> {
        let normalized_actor = normalize_optional_text(actor.map(str::to_owned));
        let now_ms = now_ms.unwrap_or_else(current_unix_ms);
        let mut connection = self.open_connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("open expired lease recovery transaction failed: {error}"))?;
        let raw_records = load_expired_raw_work_units_with_conn(&transaction, now_ms)?;
        let mut recovered_ids = Vec::new();

        for raw_record in raw_records {
            let retry_policy =
                decode_json::<WorkUnitRetryPolicy>(&raw_record.retry_policy_json, "retry policy")?;
            let attempt_count = u32::try_from(raw_record.attempt_count)
                .map_err(|error| format!("attempt_count overflowed u32: {error}"))?;
            let previous_owner = raw_record.lease_owner.clone();
            let previous_status = raw_record.status.clone();
            let expiration_error = build_expired_lease_error(&raw_record)?;
            let recovery =
                resolve_recovery(&retry_policy, attempt_count, now_ms, &expiration_error)?;

            transaction
                .execute(
                    "UPDATE work_units
                     SET status = ?1,
                         next_run_at_ms = ?2,
                         last_error = ?3,
                         lease_owner = NULL,
                         lease_acquired_at_ms = NULL,
                         lease_heartbeat_at_ms = NULL,
                         lease_expires_at_ms = NULL,
                         updated_at_ms = ?4
                     WHERE work_unit_id = ?5
                       AND status IN ('leased', 'running')
                       AND lease_expires_at_ms IS NOT NULL
                       AND lease_expires_at_ms < ?4",
                    params![
                        recovery.status.as_str(),
                        recovery.next_run_at_ms,
                        recovery.last_error,
                        now_ms,
                        raw_record.work_unit_id,
                    ],
                )
                .map_err(|error| format!("recover expired work unit lease failed: {error}"))?;

            let event_payload = json!({
                "previous_owner": previous_owner,
                "previous_status": previous_status,
                "next_status": recovery.status.as_str(),
                "next_run_at_ms": recovery.next_run_at_ms,
                "last_error": recovery.last_error,
                "attempt_count": attempt_count,
            });
            insert_event_in_tx(
                &transaction,
                &raw_record.work_unit_id,
                WORK_UNIT_LEASE_EXPIRED_EVENT_KIND,
                normalized_actor.as_deref(),
                &event_payload,
                now_ms,
            )?;
            recovered_ids.push(raw_record.work_unit_id);
        }

        transaction.commit().map_err(|error| {
            format!("commit expired lease recovery transaction failed: {error}")
        })?;

        let mut recovered_snapshots = Vec::new();
        for recovered_id in recovered_ids {
            let snapshot = self
                .load_work_unit_snapshot(&recovered_id)?
                .ok_or_else(|| format!("recovered work unit `{recovered_id}` disappeared"))?;
            recovered_snapshots.push(snapshot);
        }

        Ok(recovered_snapshots)
    }

    pub fn load_runtime_health(
        &self,
        now_ms: Option<i64>,
    ) -> Result<WorkRuntimeHealthSnapshot, String> {
        let now_ms = now_ms.unwrap_or_else(current_unix_ms);
        let connection = self.open_connection()?;
        let row = connection
            .query_row(
                "SELECT
                    COUNT(*),
                    SUM(CASE WHEN status = 'ready' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN status = 'leased' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END),
                    SUM(CASE
                            WHEN status IN ('waiting_external', 'waiting_review') THEN 1
                            WHEN status IN ('ready', 'retry_pending')
                                 AND EXISTS (
                                     SELECT 1
                                     FROM work_unit_dependencies dependencies
                                     JOIN work_units blockers
                                       ON blockers.work_unit_id = dependencies.blocking_work_unit_id
                                     WHERE dependencies.blocked_work_unit_id = work_units.work_unit_id
                                       AND blockers.status NOT IN ('completed', 'failed_terminal', 'cancelled', 'archived')
                                 ) THEN 1
                            ELSE 0
                        END),
                    SUM(CASE WHEN status = 'retry_pending' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN status IN ('completed', 'failed_terminal', 'cancelled') THEN 1 ELSE 0 END),
                    SUM(CASE WHEN status = 'archived' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN status IN ('leased', 'running')
                                 AND lease_expires_at_ms IS NOT NULL
                                 AND lease_expires_at_ms < ?1
                             THEN 1 ELSE 0 END)
                 FROM work_units",
                params![now_ms],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                },
            )
            .map_err(|error| format!("load work runtime health failed: {error}"))?;
        let total_count = usize_from_i64(row.0, "total_count")?;
        let ready_count = usize_from_i64(row.1, "ready_count")?;
        let leased_count = usize_from_i64(row.2, "leased_count")?;
        let running_count = usize_from_i64(row.3, "running_count")?;
        let blocked_count = usize_from_i64(row.4, "blocked_count")?;
        let retry_pending_count = usize_from_i64(row.5, "retry_pending_count")?;
        let terminal_count = usize_from_i64(row.6, "terminal_count")?;
        let archived_count = usize_from_i64(row.7, "archived_count")?;
        let expired_lease_count = usize_from_i64(row.8, "expired_lease_count")?;

        Ok(WorkRuntimeHealthSnapshot {
            total_count,
            ready_count,
            leased_count,
            running_count,
            blocked_count,
            retry_pending_count,
            terminal_count,
            archived_count,
            expired_lease_count,
        })
    }

    fn ensure_schema(&self) -> Result<(), String> {
        let connection = self.open_connection()?;
        connection
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS work_units(
                    work_unit_id TEXT PRIMARY KEY,
                    kind TEXT NOT NULL,
                    title TEXT NOT NULL,
                    description TEXT NOT NULL,
                    source_ref_json TEXT NOT NULL,
                    status TEXT NOT NULL,
                    priority TEXT NOT NULL,
                    priority_rank INTEGER NOT NULL,
                    retry_policy_json TEXT NOT NULL,
                    attempt_count INTEGER NOT NULL,
                    next_run_at_ms INTEGER NOT NULL,
                    last_error TEXT NULL,
                    blocking_reason TEXT NULL,
                    parent_work_unit_id TEXT NULL,
                    assigned_to TEXT NULL,
                    result_payload_json TEXT NULL,
                    lease_owner TEXT NULL,
                    lease_version INTEGER NOT NULL DEFAULT 0,
                    lease_acquired_at_ms INTEGER NULL,
                    lease_heartbeat_at_ms INTEGER NULL,
                    lease_expires_at_ms INTEGER NULL,
                    created_at_ms INTEGER NOT NULL,
                    updated_at_ms INTEGER NOT NULL,
                    archived_at_ms INTEGER NULL
                );
                CREATE INDEX IF NOT EXISTS idx_work_units_status_next_run
                  ON work_units(status, next_run_at_ms, priority_rank, updated_at_ms, work_unit_id);
                CREATE INDEX IF NOT EXISTS idx_work_units_lease_expiry
                  ON work_units(lease_expires_at_ms, status, updated_at_ms, work_unit_id);
                CREATE INDEX IF NOT EXISTS idx_work_units_archived_status
                  ON work_units(archived_at_ms, status, updated_at_ms, work_unit_id);
                CREATE TABLE IF NOT EXISTS work_unit_dependencies(
                    blocking_work_unit_id TEXT NOT NULL,
                    blocked_work_unit_id TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    created_by TEXT NULL,
                    PRIMARY KEY(blocking_work_unit_id, blocked_work_unit_id)
                );
                CREATE INDEX IF NOT EXISTS idx_work_unit_dependencies_blocked
                  ON work_unit_dependencies(blocked_work_unit_id, blocking_work_unit_id);
                CREATE TABLE IF NOT EXISTS work_unit_events(
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    work_unit_id TEXT NOT NULL,
                    event_kind TEXT NOT NULL,
                    actor TEXT NULL,
                    payload_json TEXT NOT NULL,
                    recorded_at_ms INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_work_unit_events_work_unit_id
                  ON work_unit_events(work_unit_id, id);
                ",
            )
            .map_err(|error| format!("ensure work unit schema failed: {error}"))?;
        Ok(())
    }

    fn open_connection(&self) -> Result<Connection, String> {
        Connection::open(&self.db_path)
            .map_err(|error| format!("open work unit repository sqlite db failed: {error}"))
    }
}

fn insert_event_in_tx(
    transaction: &Transaction<'_>,
    work_unit_id: &str,
    event_kind: &str,
    actor: Option<&str>,
    payload_json: &Value,
    recorded_at_ms: i64,
) -> Result<WorkUnitEventRecord, String> {
    let encoded_payload = encode_json(payload_json, "work unit event payload")?;
    transaction
        .execute(
            "INSERT INTO work_unit_events(
                work_unit_id,
                event_kind,
                actor,
                payload_json,
                recorded_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                work_unit_id,
                event_kind,
                actor,
                encoded_payload,
                recorded_at_ms
            ],
        )
        .map_err(|error| format!("insert work unit event failed: {error}"))?;

    Ok(WorkUnitEventRecord {
        sequence_id: transaction.last_insert_rowid(),
        work_unit_id: work_unit_id.to_owned(),
        event_kind: event_kind.to_owned(),
        actor: actor.map(str::to_owned),
        payload_json: payload_json.clone(),
        recorded_at_ms,
    })
}

fn load_raw_work_unit_with_conn(
    connection: &Connection,
    work_unit_id: &str,
) -> Result<Option<RawWorkUnitRecord>, String> {
    let raw_record = connection
        .query_row(
            "SELECT
                work_unit_id,
                kind,
                title,
                description,
                source_ref_json,
                status,
                priority,
                retry_policy_json,
                attempt_count,
                next_run_at_ms,
                last_error,
                blocking_reason,
                parent_work_unit_id,
                assigned_to,
                result_payload_json,
                lease_owner,
                lease_version,
                lease_acquired_at_ms,
                lease_heartbeat_at_ms,
                lease_expires_at_ms,
                created_at_ms,
                updated_at_ms,
                archived_at_ms
             FROM work_units
             WHERE work_unit_id = ?1",
            params![work_unit_id],
            |row| {
                Ok(RawWorkUnitRecord {
                    work_unit_id: row.get(0)?,
                    kind: row.get(1)?,
                    title: row.get(2)?,
                    description: row.get(3)?,
                    source_ref_json: row.get(4)?,
                    status: row.get(5)?,
                    priority: row.get(6)?,
                    retry_policy_json: row.get(7)?,
                    attempt_count: row.get(8)?,
                    next_run_at_ms: row.get(9)?,
                    last_error: row.get(10)?,
                    blocking_reason: row.get(11)?,
                    parent_work_unit_id: row.get(12)?,
                    assigned_to: row.get(13)?,
                    blocks_work_unit_ids: Vec::new(),
                    blocked_by_work_unit_ids: Vec::new(),
                    result_payload_json: row.get(14)?,
                    lease_owner: row.get(15)?,
                    lease_version: row.get(16)?,
                    lease_acquired_at_ms: row.get(17)?,
                    lease_heartbeat_at_ms: row.get(18)?,
                    lease_expires_at_ms: row.get(19)?,
                    created_at_ms: row.get(20)?,
                    updated_at_ms: row.get(21)?,
                    archived_at_ms: row.get(22)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("load work unit row failed: {error}"))?;
    let Some(raw_record) = raw_record else {
        return Ok(None);
    };
    let raw_record = hydrate_raw_work_unit_relationships(connection, raw_record)?;
    Ok(Some(raw_record))
}

fn load_raw_work_units_with_query(
    connection: &Connection,
    status: Option<WorkUnitStatus>,
    include_archived: bool,
    limit: usize,
) -> Result<Vec<RawWorkUnitRecord>, String> {
    let limit =
        i64::try_from(limit).map_err(|error| format!("list limit overflowed i64: {error}"))?;
    let sql = match (status, include_archived) {
        (Some(_), false) => {
            "SELECT
                work_unit_id,
                kind,
                title,
                description,
                source_ref_json,
                status,
                priority,
                retry_policy_json,
                attempt_count,
                next_run_at_ms,
                last_error,
                blocking_reason,
                parent_work_unit_id,
                assigned_to,
                result_payload_json,
                lease_owner,
                lease_version,
                lease_acquired_at_ms,
                lease_heartbeat_at_ms,
                lease_expires_at_ms,
                created_at_ms,
                updated_at_ms,
                archived_at_ms
             FROM work_units
             WHERE status = ?1
               AND archived_at_ms IS NULL
             ORDER BY priority_rank DESC, next_run_at_ms ASC, updated_at_ms DESC, work_unit_id ASC
             LIMIT ?2"
        }
        (Some(_), true) => {
            "SELECT
                work_unit_id,
                kind,
                title,
                description,
                source_ref_json,
                status,
                priority,
                retry_policy_json,
                attempt_count,
                next_run_at_ms,
                last_error,
                blocking_reason,
                parent_work_unit_id,
                assigned_to,
                result_payload_json,
                lease_owner,
                lease_version,
                lease_acquired_at_ms,
                lease_heartbeat_at_ms,
                lease_expires_at_ms,
                created_at_ms,
                updated_at_ms,
                archived_at_ms
             FROM work_units
             WHERE status = ?1
             ORDER BY priority_rank DESC, next_run_at_ms ASC, updated_at_ms DESC, work_unit_id ASC
             LIMIT ?2"
        }
        (None, false) => {
            "SELECT
                work_unit_id,
                kind,
                title,
                description,
                source_ref_json,
                status,
                priority,
                retry_policy_json,
                attempt_count,
                next_run_at_ms,
                last_error,
                blocking_reason,
                parent_work_unit_id,
                assigned_to,
                result_payload_json,
                lease_owner,
                lease_version,
                lease_acquired_at_ms,
                lease_heartbeat_at_ms,
                lease_expires_at_ms,
                created_at_ms,
                updated_at_ms,
                archived_at_ms
             FROM work_units
             WHERE archived_at_ms IS NULL
             ORDER BY priority_rank DESC, next_run_at_ms ASC, updated_at_ms DESC, work_unit_id ASC
             LIMIT ?1"
        }
        (None, true) => {
            "SELECT
                work_unit_id,
                kind,
                title,
                description,
                source_ref_json,
                status,
                priority,
                retry_policy_json,
                attempt_count,
                next_run_at_ms,
                last_error,
                blocking_reason,
                parent_work_unit_id,
                assigned_to,
                result_payload_json,
                lease_owner,
                lease_version,
                lease_acquired_at_ms,
                lease_heartbeat_at_ms,
                lease_expires_at_ms,
                created_at_ms,
                updated_at_ms,
                archived_at_ms
             FROM work_units
             ORDER BY priority_rank DESC, next_run_at_ms ASC, updated_at_ms DESC, work_unit_id ASC
             LIMIT ?1"
        }
    };
    let mut statement = connection
        .prepare(sql)
        .map_err(|error| format!("prepare work unit list query failed: {error}"))?;
    let row_mapper = |row: &rusqlite::Row<'_>| {
        Ok(RawWorkUnitRecord {
            work_unit_id: row.get(0)?,
            kind: row.get(1)?,
            title: row.get(2)?,
            description: row.get(3)?,
            source_ref_json: row.get(4)?,
            status: row.get(5)?,
            priority: row.get(6)?,
            retry_policy_json: row.get(7)?,
            attempt_count: row.get(8)?,
            next_run_at_ms: row.get(9)?,
            last_error: row.get(10)?,
            blocking_reason: row.get(11)?,
            parent_work_unit_id: row.get(12)?,
            assigned_to: row.get(13)?,
            blocks_work_unit_ids: Vec::new(),
            blocked_by_work_unit_ids: Vec::new(),
            result_payload_json: row.get(14)?,
            lease_owner: row.get(15)?,
            lease_version: row.get(16)?,
            lease_acquired_at_ms: row.get(17)?,
            lease_heartbeat_at_ms: row.get(18)?,
            lease_expires_at_ms: row.get(19)?,
            created_at_ms: row.get(20)?,
            updated_at_ms: row.get(21)?,
            archived_at_ms: row.get(22)?,
        })
    };
    let rows = match status {
        Some(status) => statement
            .query_map(params![status.as_str(), limit], row_mapper)
            .map_err(|error| format!("query work unit list failed: {error}"))?,
        None => statement
            .query_map(params![limit], row_mapper)
            .map_err(|error| format!("query work unit list failed: {error}"))?,
    };
    let mut raw_records = Vec::new();

    for row in rows {
        let raw_record =
            row.map_err(|error| format!("decode work unit list row failed: {error}"))?;
        let raw_record = hydrate_raw_work_unit_relationships(connection, raw_record)?;
        raw_records.push(raw_record);
    }

    Ok(raw_records)
}

fn hydrate_raw_work_unit_relationships(
    connection: &Connection,
    mut raw_record: RawWorkUnitRecord,
) -> Result<RawWorkUnitRecord, String> {
    let blocks_work_unit_ids =
        load_blocked_work_unit_ids(connection, raw_record.work_unit_id.as_str())?;
    let blocked_by_work_unit_ids =
        load_blocking_work_unit_ids(connection, raw_record.work_unit_id.as_str())?;
    raw_record.blocks_work_unit_ids = blocks_work_unit_ids;
    raw_record.blocked_by_work_unit_ids = blocked_by_work_unit_ids;
    Ok(raw_record)
}

fn load_blocked_work_unit_ids(
    connection: &Connection,
    work_unit_id: &str,
) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT blocked_work_unit_id
             FROM work_unit_dependencies
             WHERE blocking_work_unit_id = ?1
             ORDER BY blocked_work_unit_id ASC",
        )
        .map_err(|error| format!("prepare blocked work unit query failed: {error}"))?;
    let rows = statement
        .query_map(params![work_unit_id], |row| row.get::<_, String>(0))
        .map_err(|error| format!("query blocked work units failed: {error}"))?;
    let mut work_unit_ids = Vec::new();

    for row in rows {
        let blocked_work_unit_id =
            row.map_err(|error| format!("decode blocked work unit row failed: {error}"))?;
        work_unit_ids.push(blocked_work_unit_id);
    }

    Ok(work_unit_ids)
}

fn load_blocking_work_unit_ids(
    connection: &Connection,
    work_unit_id: &str,
) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT blocking_work_unit_id
             FROM work_unit_dependencies
             WHERE blocked_work_unit_id = ?1
             ORDER BY blocking_work_unit_id ASC",
        )
        .map_err(|error| format!("prepare blocking work unit query failed: {error}"))?;
    let rows = statement
        .query_map(params![work_unit_id], |row| row.get::<_, String>(0))
        .map_err(|error| format!("query blocking work units failed: {error}"))?;
    let mut work_unit_ids = Vec::new();

    for row in rows {
        let blocking_work_unit_id =
            row.map_err(|error| format!("decode blocking work unit row failed: {error}"))?;
        work_unit_ids.push(blocking_work_unit_id);
    }

    Ok(work_unit_ids)
}

fn select_next_ready_raw_work_unit(
    connection: &Connection,
    now_ms: i64,
) -> Result<Option<RawWorkUnitRecord>, String> {
    let mut statement = connection
        .prepare(
            "SELECT
                work_unit_id,
                kind,
                title,
                description,
                source_ref_json,
                status,
                priority,
                retry_policy_json,
                attempt_count,
                next_run_at_ms,
                last_error,
                blocking_reason,
                parent_work_unit_id,
                assigned_to,
                result_payload_json,
                lease_owner,
                lease_version,
                lease_acquired_at_ms,
                lease_heartbeat_at_ms,
                lease_expires_at_ms,
                created_at_ms,
                updated_at_ms,
                archived_at_ms
             FROM work_units
             WHERE status IN ('ready', 'retry_pending')
               AND archived_at_ms IS NULL
               AND next_run_at_ms <= ?1
               AND (lease_expires_at_ms IS NULL OR lease_expires_at_ms <= ?1)
               AND NOT EXISTS (
                    SELECT 1
                    FROM work_unit_dependencies dependencies
                    JOIN work_units blockers
                      ON blockers.work_unit_id = dependencies.blocking_work_unit_id
                    WHERE dependencies.blocked_work_unit_id = work_units.work_unit_id
                      AND blockers.status NOT IN ('completed', 'failed_terminal', 'cancelled', 'archived')
               )
             ORDER BY priority_rank DESC, next_run_at_ms ASC, updated_at_ms ASC, work_unit_id ASC
             LIMIT 1",
        )
        .map_err(|error| format!("prepare next ready work unit query failed: {error}"))?;
    let raw_record = statement
        .query_row(params![now_ms], |row| {
            Ok(RawWorkUnitRecord {
                work_unit_id: row.get(0)?,
                kind: row.get(1)?,
                title: row.get(2)?,
                description: row.get(3)?,
                source_ref_json: row.get(4)?,
                status: row.get(5)?,
                priority: row.get(6)?,
                retry_policy_json: row.get(7)?,
                attempt_count: row.get(8)?,
                next_run_at_ms: row.get(9)?,
                last_error: row.get(10)?,
                blocking_reason: row.get(11)?,
                parent_work_unit_id: row.get(12)?,
                assigned_to: row.get(13)?,
                blocks_work_unit_ids: Vec::new(),
                blocked_by_work_unit_ids: Vec::new(),
                result_payload_json: row.get(14)?,
                lease_owner: row.get(15)?,
                lease_version: row.get(16)?,
                lease_acquired_at_ms: row.get(17)?,
                lease_heartbeat_at_ms: row.get(18)?,
                lease_expires_at_ms: row.get(19)?,
                created_at_ms: row.get(20)?,
                updated_at_ms: row.get(21)?,
                archived_at_ms: row.get(22)?,
            })
        })
        .optional()
        .map_err(|error| format!("query next ready work unit failed: {error}"))?;
    let hydrated = raw_record
        .map(|raw_record| hydrate_raw_work_unit_relationships(connection, raw_record))
        .transpose()?;
    Ok(hydrated)
}

fn load_expired_raw_work_units_with_conn(
    connection: &Connection,
    now_ms: i64,
) -> Result<Vec<RawWorkUnitRecord>, String> {
    let mut statement = connection
        .prepare(
            "SELECT
                work_unit_id,
                kind,
                title,
                description,
                source_ref_json,
                status,
                priority,
                retry_policy_json,
                attempt_count,
                next_run_at_ms,
                last_error,
                blocking_reason,
                parent_work_unit_id,
                assigned_to,
                result_payload_json,
                lease_owner,
                lease_version,
                lease_acquired_at_ms,
                lease_heartbeat_at_ms,
                lease_expires_at_ms,
                created_at_ms,
                updated_at_ms,
                archived_at_ms
             FROM work_units
             WHERE status IN ('leased', 'running')
               AND lease_expires_at_ms IS NOT NULL
               AND lease_expires_at_ms < ?1
             ORDER BY lease_expires_at_ms ASC, work_unit_id ASC",
        )
        .map_err(|error| format!("prepare expired work unit query failed: {error}"))?;
    let rows = statement
        .query_map(params![now_ms], |row| {
            Ok(RawWorkUnitRecord {
                work_unit_id: row.get(0)?,
                kind: row.get(1)?,
                title: row.get(2)?,
                description: row.get(3)?,
                source_ref_json: row.get(4)?,
                status: row.get(5)?,
                priority: row.get(6)?,
                retry_policy_json: row.get(7)?,
                attempt_count: row.get(8)?,
                next_run_at_ms: row.get(9)?,
                last_error: row.get(10)?,
                blocking_reason: row.get(11)?,
                parent_work_unit_id: row.get(12)?,
                assigned_to: row.get(13)?,
                blocks_work_unit_ids: Vec::new(),
                blocked_by_work_unit_ids: Vec::new(),
                result_payload_json: row.get(14)?,
                lease_owner: row.get(15)?,
                lease_version: row.get(16)?,
                lease_acquired_at_ms: row.get(17)?,
                lease_heartbeat_at_ms: row.get(18)?,
                lease_expires_at_ms: row.get(19)?,
                created_at_ms: row.get(20)?,
                updated_at_ms: row.get(21)?,
                archived_at_ms: row.get(22)?,
            })
        })
        .map_err(|error| format!("query expired work units failed: {error}"))?;
    let mut raw_records = Vec::new();

    for row in rows {
        let raw_record =
            row.map_err(|error| format!("decode expired work unit row failed: {error}"))?;
        let raw_record = hydrate_raw_work_unit_relationships(connection, raw_record)?;
        raw_records.push(raw_record);
    }

    Ok(raw_records)
}

fn try_work_unit_snapshot_from_raw(
    raw_record: RawWorkUnitRecord,
) -> Result<WorkUnitSnapshot, String> {
    let kind = WorkUnitKind::parse(&raw_record.kind)
        .ok_or_else(|| format!("unknown work unit kind `{}`", raw_record.kind))?;
    let status = WorkUnitStatus::parse(&raw_record.status)
        .ok_or_else(|| format!("unknown work unit status `{}`", raw_record.status))?;
    let priority = WorkUnitPriority::parse(&raw_record.priority)
        .ok_or_else(|| format!("unknown work unit priority `{}`", raw_record.priority))?;
    let source_ref = decode_json::<WorkUnitSourceRef>(&raw_record.source_ref_json, "source_ref")?;
    let retry_policy =
        decode_json::<WorkUnitRetryPolicy>(&raw_record.retry_policy_json, "retry_policy")?;
    let attempt_count = u32::try_from(raw_record.attempt_count)
        .map_err(|error| format!("attempt_count overflowed u32: {error}"))?;
    let result_payload_json = raw_record
        .result_payload_json
        .as_deref()
        .map(|value| decode_json::<Value>(value, "result_payload"))
        .transpose()?;
    let lease = build_lease_record(&raw_record)?;
    let work_unit = WorkUnitRecord {
        work_unit_id: raw_record.work_unit_id.clone(),
        kind,
        title: raw_record.title,
        description: raw_record.description,
        source_ref,
        status,
        priority,
        assigned_to: raw_record.assigned_to.clone(),
        retry_policy,
        attempt_count,
        next_run_at_ms: raw_record.next_run_at_ms,
        last_error: raw_record.last_error.clone(),
        blocking_reason: raw_record.blocking_reason.clone(),
        parent_work_unit_id: raw_record.parent_work_unit_id.clone(),
        blocks_work_unit_ids: raw_record.blocks_work_unit_ids.clone(),
        blocked_by_work_unit_ids: raw_record.blocked_by_work_unit_ids.clone(),
        result_payload_json,
        created_at_ms: raw_record.created_at_ms,
        updated_at_ms: raw_record.updated_at_ms,
        archived_at_ms: raw_record.archived_at_ms,
    };
    Ok(WorkUnitSnapshot { work_unit, lease })
}

fn build_lease_record(
    raw_record: &RawWorkUnitRecord,
) -> Result<Option<WorkUnitLeaseRecord>, String> {
    let Some(owner) = raw_record.lease_owner.clone() else {
        return Ok(None);
    };
    let lease_version = u64::try_from(raw_record.lease_version)
        .map_err(|error| format!("lease_version overflowed u64: {error}"))?;
    let acquired_at_ms = raw_record
        .lease_acquired_at_ms
        .ok_or_else(|| "lease_owner set without lease_acquired_at_ms".to_owned())?;
    let heartbeat_at_ms = raw_record
        .lease_heartbeat_at_ms
        .ok_or_else(|| "lease_owner set without lease_heartbeat_at_ms".to_owned())?;
    let expires_at_ms = raw_record
        .lease_expires_at_ms
        .ok_or_else(|| "lease_owner set without lease_expires_at_ms".to_owned())?;
    let lease = WorkUnitLeaseRecord {
        work_unit_id: raw_record.work_unit_id.clone(),
        owner,
        lease_version,
        acquired_at_ms,
        heartbeat_at_ms,
        expires_at_ms,
    };
    Ok(Some(lease))
}

struct CompletionResolution {
    status: WorkUnitStatus,
    next_run_at_ms: i64,
    last_error: Option<String>,
    event_kind: &'static str,
}

fn resolve_completion(
    disposition: WorkUnitCompletionDisposition,
    retry_policy: &WorkUnitRetryPolicy,
    attempt_count: u32,
    now_ms: i64,
    next_run_at_ms: Option<i64>,
    error: Option<&str>,
) -> Result<CompletionResolution, String> {
    match disposition {
        WorkUnitCompletionDisposition::Completed => Ok(CompletionResolution {
            status: WorkUnitStatus::Completed,
            next_run_at_ms: now_ms,
            last_error: None,
            event_kind: WORK_UNIT_COMPLETED_EVENT_KIND,
        }),
        WorkUnitCompletionDisposition::Cancelled => Ok(CompletionResolution {
            status: WorkUnitStatus::Cancelled,
            next_run_at_ms: now_ms,
            last_error: error.map(str::to_owned),
            event_kind: WORK_UNIT_CANCELLED_EVENT_KIND,
        }),
        WorkUnitCompletionDisposition::FailedTerminal => Ok(CompletionResolution {
            status: WorkUnitStatus::FailedTerminal,
            next_run_at_ms: now_ms,
            last_error: error.map(str::to_owned),
            event_kind: WORK_UNIT_FAILED_EVENT_KIND,
        }),
        WorkUnitCompletionDisposition::RetryPending => {
            if attempt_count >= retry_policy.max_attempts {
                let retry_exhausted_error = error.map(str::to_owned).unwrap_or_else(|| {
                    format!(
                        "retry budget exhausted after {} attempt(s)",
                        retry_policy.max_attempts
                    )
                });
                return Ok(CompletionResolution {
                    status: WorkUnitStatus::FailedTerminal,
                    next_run_at_ms: now_ms,
                    last_error: Some(retry_exhausted_error),
                    event_kind: WORK_UNIT_FAILED_EVENT_KIND,
                });
            }

            let computed_next_run_at_ms = match next_run_at_ms {
                Some(next_run_at_ms) => next_run_at_ms,
                None => {
                    let delay_ms = compute_retry_delay_ms(retry_policy, attempt_count)?;
                    add_delay_ms(now_ms, delay_ms)?
                }
            };
            Ok(CompletionResolution {
                status: WorkUnitStatus::RetryPending,
                next_run_at_ms: computed_next_run_at_ms,
                last_error: error.map(str::to_owned),
                event_kind: WORK_UNIT_RETRY_EVENT_KIND,
            })
        }
    }
}

struct RecoveryResolution {
    status: WorkUnitStatus,
    next_run_at_ms: i64,
    last_error: Option<String>,
}

fn resolve_recovery(
    retry_policy: &WorkUnitRetryPolicy,
    attempt_count: u32,
    now_ms: i64,
    last_error: &str,
) -> Result<RecoveryResolution, String> {
    if attempt_count >= retry_policy.max_attempts {
        return Ok(RecoveryResolution {
            status: WorkUnitStatus::FailedTerminal,
            next_run_at_ms: now_ms,
            last_error: Some(last_error.to_owned()),
        });
    }

    let delay_ms = compute_retry_delay_ms(retry_policy, attempt_count)?;
    let next_run_at_ms = add_delay_ms(now_ms, delay_ms)?;
    Ok(RecoveryResolution {
        status: WorkUnitStatus::RetryPending,
        next_run_at_ms,
        last_error: Some(last_error.to_owned()),
    })
}

fn build_expired_lease_error(raw_record: &RawWorkUnitRecord) -> Result<String, String> {
    let owner = raw_record
        .lease_owner
        .as_deref()
        .ok_or_else(|| "expired lease recovery requires lease owner".to_owned())?;
    let expires_at_ms = raw_record
        .lease_expires_at_ms
        .ok_or_else(|| "expired lease recovery requires lease_expires_at_ms".to_owned())?;
    Ok(format!(
        "lease expired for owner `{owner}` at {expires_at_ms}"
    ))
}

fn ensure_work_unit_exists(connection: &Connection, work_unit_id: &str) -> Result<(), String> {
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM work_units WHERE work_unit_id = ?1)",
            params![work_unit_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("check work unit existence failed: {error}"))?;
    if exists == 0 {
        return Err(format!("work unit `{work_unit_id}` not found"));
    }
    Ok(())
}

fn touch_work_unit(connection: &Connection, work_unit_id: &str, now_ms: i64) -> Result<(), String> {
    connection
        .execute(
            "UPDATE work_units
             SET updated_at_ms = ?1
             WHERE work_unit_id = ?2",
            params![now_ms, work_unit_id],
        )
        .map_err(|error| format!("touch work unit failed: {error}"))?;
    Ok(())
}

fn validate_dependency_endpoints(
    blocking_work_unit_id: &str,
    blocked_work_unit_id: &str,
) -> Result<(), String> {
    if blocking_work_unit_id == blocked_work_unit_id {
        return Err("work unit dependency cannot target the same work unit".to_owned());
    }
    Ok(())
}

fn would_create_dependency_cycle(
    connection: &Connection,
    blocking_work_unit_id: &str,
    blocked_work_unit_id: &str,
) -> Result<bool, String> {
    let mut statement = connection
        .prepare(
            "WITH RECURSIVE dependency_chain(work_unit_id) AS (
                 SELECT blocked_work_unit_id
                 FROM work_unit_dependencies
                 WHERE blocking_work_unit_id = ?1
                 UNION
                 SELECT dependencies.blocked_work_unit_id
                 FROM work_unit_dependencies dependencies
                 JOIN dependency_chain chain
                   ON chain.work_unit_id = dependencies.blocking_work_unit_id
             )
             SELECT EXISTS(
                 SELECT 1
                 FROM dependency_chain
                 WHERE work_unit_id = ?2
             )",
        )
        .map_err(|error| format!("prepare dependency cycle query failed: {error}"))?;
    let exists = statement
        .query_row(
            params![blocked_work_unit_id, blocking_work_unit_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("query dependency cycle failed: {error}"))?;
    Ok(exists != 0)
}

fn validate_initial_status(status: WorkUnitStatus) -> Result<(), String> {
    let is_allowed = matches!(
        status,
        WorkUnitStatus::Captured
            | WorkUnitStatus::Triaged
            | WorkUnitStatus::Ready
            | WorkUnitStatus::WaitingExternal
            | WorkUnitStatus::WaitingReview
            | WorkUnitStatus::RetryPending
    );
    if !is_allowed {
        return Err(format!(
            "work unit repository does not allow initial status `{}`",
            status.as_str()
        ));
    }
    Ok(())
}

fn validate_manual_update_status(
    current_status: WorkUnitStatus,
    next_status: WorkUnitStatus,
) -> Result<(), String> {
    let next_is_allowed = matches!(
        next_status,
        WorkUnitStatus::Captured
            | WorkUnitStatus::Triaged
            | WorkUnitStatus::Ready
            | WorkUnitStatus::WaitingExternal
            | WorkUnitStatus::WaitingReview
            | WorkUnitStatus::RetryPending
            | WorkUnitStatus::Cancelled
    );
    if !next_is_allowed {
        return Err(format!(
            "manual work unit update does not allow target status `{}`",
            next_status.as_str()
        ));
    }

    let current_is_mutable = matches!(
        current_status,
        WorkUnitStatus::Captured
            | WorkUnitStatus::Triaged
            | WorkUnitStatus::Ready
            | WorkUnitStatus::WaitingExternal
            | WorkUnitStatus::WaitingReview
            | WorkUnitStatus::RetryPending
            | WorkUnitStatus::Cancelled
    );
    if !current_is_mutable {
        return Err(format!(
            "manual work unit update cannot change status from `{}`",
            current_status.as_str()
        ));
    }

    Ok(())
}

fn validate_retry_policy(retry_policy: &WorkUnitRetryPolicy) -> Result<(), String> {
    if retry_policy.max_attempts == 0 {
        return Err("work unit retry policy requires max_attempts >= 1".to_owned());
    }
    if retry_policy.initial_backoff_ms == 0 {
        return Err("work unit retry policy requires initial_backoff_ms >= 1".to_owned());
    }
    if retry_policy.max_backoff_ms < retry_policy.initial_backoff_ms {
        return Err(
            "work unit retry policy requires max_backoff_ms >= initial_backoff_ms".to_owned(),
        );
    }
    Ok(())
}

fn validate_ttl_ms(ttl_ms: u64) -> Result<(), String> {
    if ttl_ms == 0 {
        return Err("work unit lease ttl_ms must be greater than zero".to_owned());
    }
    Ok(())
}

fn normalize_required_text(value: &str, field_name: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("work unit repository requires {field_name}"));
    }
    Ok(trimmed.to_owned())
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|raw_value| {
        let trimmed_value = raw_value.trim();
        if trimmed_value.is_empty() {
            return None;
        }
        Some(trimmed_value.to_owned())
    })
}

fn normalize_source_ref(source_ref: WorkUnitSourceRef) -> WorkUnitSourceRef {
    WorkUnitSourceRef {
        source_kind: source_ref.source_kind,
        project_id: normalize_optional_text(source_ref.project_id),
        channel_id: normalize_optional_text(source_ref.channel_id),
        thread_id: normalize_optional_text(source_ref.thread_id),
        message_id: normalize_optional_text(source_ref.message_id),
        external_ref: normalize_optional_text(source_ref.external_ref),
        source_url: normalize_optional_text(source_ref.source_url),
    }
}

fn normalize_limit(limit: usize) -> Result<usize, String> {
    if limit == 0 {
        return Err("work unit repository requires limit >= 1".to_owned());
    }
    Ok(limit)
}

fn encode_json<T>(value: &T, label: &str) -> Result<String, String>
where
    T: serde::Serialize,
{
    serde_json::to_string(value).map_err(|error| format!("encode {label} failed: {error}"))
}

fn decode_json<T>(raw: &str, label: &str) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(raw).map_err(|error| format!("decode {label} failed: {error}"))
}

fn current_unix_ms() -> i64 {
    let now = SystemTime::now();
    let since_epoch = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let millis = since_epoch.as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

fn add_delay_ms(base_ms: i64, delay_ms: u64) -> Result<i64, String> {
    let delay_ms =
        i64::try_from(delay_ms).map_err(|error| format!("delay_ms overflowed i64: {error}"))?;
    Ok(base_ms.saturating_add(delay_ms))
}

fn compute_retry_delay_ms(
    retry_policy: &WorkUnitRetryPolicy,
    attempt_count: u32,
) -> Result<u64, String> {
    validate_retry_policy(retry_policy)?;
    let mut delay_ms = retry_policy.initial_backoff_ms;
    let mut remaining_steps = attempt_count.saturating_sub(1);

    while remaining_steps > 0 {
        let doubled_delay_ms = delay_ms.saturating_mul(2);
        let clamped_delay_ms = doubled_delay_ms.min(retry_policy.max_backoff_ms);
        delay_ms = clamped_delay_ms;
        remaining_steps = remaining_steps.saturating_sub(1);
    }

    Ok(delay_ms)
}

fn generate_work_unit_id() -> String {
    let entropy = random::<u64>();
    format!("wu-{entropy:016x}")
}

fn priority_rank(priority: WorkUnitPriority) -> i64 {
    match priority {
        WorkUnitPriority::Low => 1,
        WorkUnitPriority::Normal => 2,
        WorkUnitPriority::High => 3,
        WorkUnitPriority::Critical => 4,
        _ => 2,
    }
}

fn usize_from_i64(value: i64, label: &str) -> Result<usize, String> {
    usize::try_from(value).map_err(|error| format!("{label} overflowed usize: {error}"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;

    use crate::memory::runtime_config::MemoryRuntimeConfig;

    use super::{
        AcquireWorkUnitLeaseRequest, AddWorkUnitDependencyRequest, AppendWorkUnitNoteRequest,
        ArchiveWorkUnitRequest, AssignWorkUnitRequest, CompleteWorkUnitRequest, NewWorkUnitRecord,
        RemoveWorkUnitDependencyRequest, StartWorkUnitLeaseRequest, UpdateWorkUnitRequest,
        WORK_UNIT_ASSIGNED_EVENT_KIND, WORK_UNIT_DEPENDENCY_ADDED_EVENT_KIND,
        WORK_UNIT_DEPENDENCY_REMOVED_EVENT_KIND, WORK_UNIT_NOTE_ADDED_EVENT_KIND,
        WORK_UNIT_UPDATED_EVENT_KIND, WorkUnitCompletionDisposition, WorkUnitHeartbeatRequest,
        WorkUnitListQuery, WorkUnitRepository,
    };
    use loongclaw_contracts::{
        WorkSourceKind, WorkUnitKind, WorkUnitPriority, WorkUnitRetryPolicy, WorkUnitSourceRef,
        WorkUnitStatus,
    };

    fn isolated_memory_config(test_name: &str) -> MemoryRuntimeConfig {
        let base = std::env::temp_dir().join(format!(
            "loongclaw-work-unit-repository-{test_name}-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&base);
        let db_path = base.join("memory.sqlite3");
        let _ = fs::remove_file(&db_path);
        MemoryRuntimeConfig {
            sqlite_path: Some(db_path),
            ..MemoryRuntimeConfig::default()
        }
    }

    fn sample_source_ref() -> WorkUnitSourceRef {
        WorkUnitSourceRef {
            source_kind: WorkSourceKind::Discord,
            project_id: Some("loongclaw-ai/server".to_owned()),
            channel_id: Some("feature".to_owned()),
            thread_id: Some("thread-42".to_owned()),
            message_id: Some("msg-7".to_owned()),
            external_ref: Some("feature-thread".to_owned()),
            source_url: Some("https://discord.example/feature/thread-42".to_owned()),
        }
    }

    fn sample_work_unit(status: WorkUnitStatus) -> NewWorkUnitRecord {
        NewWorkUnitRecord {
            work_unit_id: Some("wu-test".to_owned()),
            kind: WorkUnitKind::Feature,
            title: "Durable runtime foundation".to_owned(),
            description: "Implement the first durable work-unit runtime slice".to_owned(),
            source_ref: sample_source_ref(),
            status,
            priority: WorkUnitPriority::High,
            retry_policy: WorkUnitRetryPolicy {
                max_attempts: 3,
                initial_backoff_ms: 1_000,
                max_backoff_ms: 8_000,
            },
            parent_work_unit_id: None,
            next_run_at_ms: Some(1_000),
        }
    }

    #[test]
    fn create_work_unit_round_trips_snapshot_fields() {
        let config = isolated_memory_config("create-roundtrip");
        let repository = WorkUnitRepository::new(&config).expect("repository");
        let created = repository
            .create_work_unit(sample_work_unit(WorkUnitStatus::Ready), Some("operator"))
            .expect("create work unit");

        assert_eq!(created.work_unit.work_unit_id, "wu-test");
        assert_eq!(created.work_unit.status, WorkUnitStatus::Ready);
        assert_eq!(created.work_unit.priority, WorkUnitPriority::High);
        assert_eq!(
            created.work_unit.source_ref.source_kind,
            WorkSourceKind::Discord
        );
        assert_eq!(created.work_unit.attempt_count, 0);
        assert_eq!(created.work_unit.assigned_to, None);
        assert!(created.work_unit.blocks_work_unit_ids.is_empty());
        assert!(created.work_unit.blocked_by_work_unit_ids.is_empty());
        assert!(created.lease.is_none());

        let events = repository
            .list_work_unit_events("wu-test", 10)
            .expect("list work unit events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_kind, "work_unit_created");
    }

    #[test]
    fn update_work_unit_mutates_editable_fields_and_records_event() {
        let config = isolated_memory_config("update-fields");
        let repository = WorkUnitRepository::new(&config).expect("repository");
        repository
            .create_work_unit(sample_work_unit(WorkUnitStatus::Triaged), Some("operator"))
            .expect("create work unit");

        let updated = repository
            .update_work_unit(UpdateWorkUnitRequest {
                work_unit_id: "wu-test".to_owned(),
                title: Some("Durable runtime foundation v2".to_owned()),
                description: Some("Refine orchestration surface".to_owned()),
                status: Some(WorkUnitStatus::WaitingReview),
                priority: Some(WorkUnitPriority::Critical),
                next_run_at_ms: Some(2_500),
                blocking_reason: Some("waiting for design review".to_owned()),
                clear_blocking_reason: false,
                actor: Some("planner".to_owned()),
                now_ms: Some(2_000),
            })
            .expect("update work unit")
            .expect("updated snapshot");
        assert_eq!(updated.work_unit.title, "Durable runtime foundation v2");
        assert_eq!(
            updated.work_unit.description,
            "Refine orchestration surface"
        );
        assert_eq!(updated.work_unit.status, WorkUnitStatus::WaitingReview);
        assert_eq!(updated.work_unit.priority, WorkUnitPriority::Critical);
        assert_eq!(updated.work_unit.next_run_at_ms, 2_500);
        assert_eq!(
            updated.work_unit.blocking_reason.as_deref(),
            Some("waiting for design review")
        );

        let ready = repository
            .update_work_unit(UpdateWorkUnitRequest {
                work_unit_id: "wu-test".to_owned(),
                title: None,
                description: None,
                status: Some(WorkUnitStatus::Ready),
                priority: None,
                next_run_at_ms: None,
                blocking_reason: None,
                clear_blocking_reason: true,
                actor: Some("planner".to_owned()),
                now_ms: Some(2_100),
            })
            .expect("clear blocking reason")
            .expect("ready snapshot");
        assert_eq!(ready.work_unit.status, WorkUnitStatus::Ready);
        assert_eq!(ready.work_unit.blocking_reason, None);

        let events = repository
            .list_work_unit_events("wu-test", 10)
            .expect("list work unit events");
        assert!(
            events
                .iter()
                .any(|event| event.event_kind == WORK_UNIT_UPDATED_EVENT_KIND),
            "expected work-unit update event"
        );
    }

    #[test]
    fn update_work_unit_rejects_runtime_owned_status_transition() {
        let config = isolated_memory_config("update-runtime-owned-status");
        let repository = WorkUnitRepository::new(&config).expect("repository");
        repository
            .create_work_unit(sample_work_unit(WorkUnitStatus::Ready), Some("operator"))
            .expect("create work unit");
        repository
            .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
                owner: "worker-a".to_owned(),
                ttl_ms: 5_000,
                actor: Some("scheduler".to_owned()),
                now_ms: Some(1_000),
            })
            .expect("acquire lease")
            .expect("leased snapshot");

        let error = repository
            .update_work_unit(UpdateWorkUnitRequest {
                work_unit_id: "wu-test".to_owned(),
                title: None,
                description: None,
                status: Some(WorkUnitStatus::WaitingReview),
                priority: None,
                next_run_at_ms: None,
                blocking_reason: None,
                clear_blocking_reason: false,
                actor: Some("planner".to_owned()),
                now_ms: Some(1_100),
            })
            .expect_err("runtime-owned status transition should be rejected");

        assert!(error.contains("cannot change status from `leased`"));
    }

    #[test]
    fn acquire_start_heartbeat_complete_flow_updates_snapshot_and_events() {
        let config = isolated_memory_config("lease-flow");
        let repository = WorkUnitRepository::new(&config).expect("repository");
        repository
            .create_work_unit(sample_work_unit(WorkUnitStatus::Ready), Some("operator"))
            .expect("create work unit");

        let leased = repository
            .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
                owner: "worker-a".to_owned(),
                ttl_ms: 5_000,
                actor: Some("scheduler".to_owned()),
                now_ms: Some(2_000),
            })
            .expect("acquire lease")
            .expect("leased work unit");
        assert_eq!(leased.work_unit.status, WorkUnitStatus::Leased);
        assert_eq!(leased.work_unit.attempt_count, 1);
        assert_eq!(leased.lease.as_ref().expect("lease").owner, "worker-a");

        let running = repository
            .mark_leased_running(StartWorkUnitLeaseRequest {
                work_unit_id: "wu-test".to_owned(),
                owner: "worker-a".to_owned(),
                actor: Some("worker-a".to_owned()),
                now_ms: Some(2_500),
            })
            .expect("mark running")
            .expect("running snapshot");
        assert_eq!(running.work_unit.status, WorkUnitStatus::Running);

        let heartbeat = repository
            .heartbeat_lease(WorkUnitHeartbeatRequest {
                work_unit_id: "wu-test".to_owned(),
                owner: "worker-a".to_owned(),
                ttl_ms: 7_000,
                actor: Some("worker-a".to_owned()),
                now_ms: Some(3_000),
            })
            .expect("heartbeat")
            .expect("heartbeat snapshot");
        let heartbeat_lease = heartbeat.lease.expect("lease after heartbeat");
        assert_eq!(heartbeat_lease.expires_at_ms, 10_000);

        let completed = repository
            .complete_work_unit(CompleteWorkUnitRequest {
                work_unit_id: "wu-test".to_owned(),
                owner: "worker-a".to_owned(),
                disposition: WorkUnitCompletionDisposition::Completed,
                actor: Some("worker-a".to_owned()),
                now_ms: Some(4_000),
                next_run_at_ms: None,
                result_payload_json: Some(json!({"summary": "done"})),
                error: None,
            })
            .expect("complete work unit")
            .expect("completed snapshot");
        assert_eq!(completed.work_unit.status, WorkUnitStatus::Completed);
        assert!(completed.lease.is_none());
        assert_eq!(
            completed.work_unit.result_payload_json,
            Some(json!({"summary": "done"}))
        );

        let events = repository
            .list_work_unit_events("wu-test", 10)
            .expect("list events");
        let event_kinds = events
            .iter()
            .map(|event| event.event_kind.as_str())
            .collect::<Vec<_>>();
        assert!(event_kinds.contains(&"work_unit_created"));
        assert!(event_kinds.contains(&"work_unit_leased"));
        assert!(event_kinds.contains(&"work_unit_started"));
        assert!(event_kinds.contains(&"work_unit_heartbeat"));
        assert!(event_kinds.contains(&"work_unit_completed"));
    }

    #[test]
    fn retry_completion_schedules_backoff_and_exhaustion_becomes_failed_terminal() {
        let config = isolated_memory_config("retry-completion");
        let repository = WorkUnitRepository::new(&config).expect("repository");
        repository
            .create_work_unit(sample_work_unit(WorkUnitStatus::Ready), Some("operator"))
            .expect("create work unit");

        let first_lease = repository
            .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
                owner: "worker-a".to_owned(),
                ttl_ms: 5_000,
                actor: None,
                now_ms: Some(2_000),
            })
            .expect("first lease")
            .expect("leased snapshot");
        assert_eq!(first_lease.work_unit.attempt_count, 1);

        let first_retry = repository
            .complete_work_unit(CompleteWorkUnitRequest {
                work_unit_id: "wu-test".to_owned(),
                owner: "worker-a".to_owned(),
                disposition: WorkUnitCompletionDisposition::RetryPending,
                actor: Some("worker-a".to_owned()),
                now_ms: Some(4_000),
                next_run_at_ms: None,
                result_payload_json: None,
                error: Some("transient".to_owned()),
            })
            .expect("first retry")
            .expect("retry snapshot");
        assert_eq!(first_retry.work_unit.status, WorkUnitStatus::RetryPending);
        assert_eq!(first_retry.work_unit.next_run_at_ms, 5_000);
        assert_eq!(
            first_retry.work_unit.last_error.as_deref(),
            Some("transient")
        );

        let second_lease = repository
            .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
                owner: "worker-b".to_owned(),
                ttl_ms: 5_000,
                actor: None,
                now_ms: Some(5_000),
            })
            .expect("second lease")
            .expect("second leased snapshot");
        assert_eq!(second_lease.work_unit.attempt_count, 2);

        let second_retry = repository
            .complete_work_unit(CompleteWorkUnitRequest {
                work_unit_id: "wu-test".to_owned(),
                owner: "worker-b".to_owned(),
                disposition: WorkUnitCompletionDisposition::RetryPending,
                actor: None,
                now_ms: Some(6_000),
                next_run_at_ms: None,
                result_payload_json: None,
                error: Some("still transient".to_owned()),
            })
            .expect("second retry")
            .expect("second retry snapshot");
        assert_eq!(second_retry.work_unit.next_run_at_ms, 8_000);

        let third_lease = repository
            .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
                owner: "worker-c".to_owned(),
                ttl_ms: 5_000,
                actor: None,
                now_ms: Some(8_000),
            })
            .expect("third lease")
            .expect("third leased snapshot");
        assert_eq!(third_lease.work_unit.attempt_count, 3);

        let exhausted = repository
            .complete_work_unit(CompleteWorkUnitRequest {
                work_unit_id: "wu-test".to_owned(),
                owner: "worker-c".to_owned(),
                disposition: WorkUnitCompletionDisposition::RetryPending,
                actor: None,
                now_ms: Some(9_000),
                next_run_at_ms: None,
                result_payload_json: None,
                error: Some("retry budget exhausted".to_owned()),
            })
            .expect("retry exhaustion")
            .expect("failed terminal snapshot");
        assert_eq!(exhausted.work_unit.status, WorkUnitStatus::FailedTerminal);
    }

    #[test]
    fn recover_expired_leases_moves_units_to_retry_pending_or_failed_terminal() {
        let config = isolated_memory_config("recover-expired");
        let repository = WorkUnitRepository::new(&config).expect("repository");

        let first = NewWorkUnitRecord {
            work_unit_id: Some("wu-first".to_owned()),
            retry_policy: WorkUnitRetryPolicy {
                max_attempts: 3,
                initial_backoff_ms: 1_000,
                max_backoff_ms: 4_000,
            },
            ..sample_work_unit(WorkUnitStatus::Ready)
        };
        let second = NewWorkUnitRecord {
            work_unit_id: Some("wu-second".to_owned()),
            retry_policy: WorkUnitRetryPolicy {
                max_attempts: 1,
                initial_backoff_ms: 1_000,
                max_backoff_ms: 4_000,
            },
            ..sample_work_unit(WorkUnitStatus::Ready)
        };

        repository
            .create_work_unit(first, None)
            .expect("create first work unit");
        repository
            .create_work_unit(second, None)
            .expect("create second work unit");

        repository
            .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
                owner: "worker-a".to_owned(),
                ttl_ms: 2_000,
                actor: None,
                now_ms: Some(1_000),
            })
            .expect("lease first work unit")
            .expect("first leased");
        repository
            .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
                owner: "worker-b".to_owned(),
                ttl_ms: 2_000,
                actor: None,
                now_ms: Some(1_100),
            })
            .expect("lease second work unit")
            .expect("second leased");

        let recovered = repository
            .recover_expired_leases(Some("recovery-scan"), Some(5_000))
            .expect("recover expired leases");
        assert_eq!(recovered.len(), 2);

        let first_snapshot = repository
            .load_work_unit_snapshot("wu-first")
            .expect("load first snapshot")
            .expect("first snapshot");
        assert_eq!(
            first_snapshot.work_unit.status,
            WorkUnitStatus::RetryPending
        );
        assert_eq!(first_snapshot.work_unit.next_run_at_ms, 6_000);

        let second_snapshot = repository
            .load_work_unit_snapshot("wu-second")
            .expect("load second snapshot")
            .expect("second snapshot");
        assert_eq!(
            second_snapshot.work_unit.status,
            WorkUnitStatus::FailedTerminal
        );
    }

    #[test]
    fn archive_work_unit_requires_terminal_status() {
        let config = isolated_memory_config("archive-work-unit");
        let repository = WorkUnitRepository::new(&config).expect("repository");
        repository
            .create_work_unit(sample_work_unit(WorkUnitStatus::Ready), None)
            .expect("create work unit");

        let archived_before_terminal = repository
            .archive_work_unit(ArchiveWorkUnitRequest {
                work_unit_id: "wu-test".to_owned(),
                actor: Some("operator".to_owned()),
                now_ms: Some(2_000),
            })
            .expect("archive before terminal should not error");
        assert!(archived_before_terminal.is_none());

        repository
            .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
                owner: "worker-a".to_owned(),
                ttl_ms: 5_000,
                actor: None,
                now_ms: Some(2_100),
            })
            .expect("lease for archive flow")
            .expect("leased snapshot");
        repository
            .complete_work_unit(CompleteWorkUnitRequest {
                work_unit_id: "wu-test".to_owned(),
                owner: "worker-a".to_owned(),
                disposition: WorkUnitCompletionDisposition::Cancelled,
                actor: None,
                now_ms: Some(2_200),
                next_run_at_ms: None,
                result_payload_json: None,
                error: Some("operator cancelled".to_owned()),
            })
            .expect("cancel work unit")
            .expect("cancelled snapshot");

        let archived = repository
            .archive_work_unit(ArchiveWorkUnitRequest {
                work_unit_id: "wu-test".to_owned(),
                actor: Some("operator".to_owned()),
                now_ms: Some(2_300),
            })
            .expect("archive terminal work unit")
            .expect("archived snapshot");
        assert_eq!(archived.work_unit.status, WorkUnitStatus::Archived);
        assert_eq!(archived.work_unit.archived_at_ms, Some(2_300));
    }

    #[test]
    fn runtime_health_reports_counts_and_expired_leases() {
        let config = isolated_memory_config("runtime-health");
        let repository = WorkUnitRepository::new(&config).expect("repository");

        let ready = NewWorkUnitRecord {
            work_unit_id: Some("wu-ready".to_owned()),
            ..sample_work_unit(WorkUnitStatus::Ready)
        };
        let blocked = NewWorkUnitRecord {
            work_unit_id: Some("wu-blocked".to_owned()),
            ..sample_work_unit(WorkUnitStatus::WaitingReview)
        };

        repository
            .create_work_unit(ready, None)
            .expect("create ready");
        repository
            .create_work_unit(blocked, None)
            .expect("create blocked");

        repository
            .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
                owner: "worker-a".to_owned(),
                ttl_ms: 1_000,
                actor: None,
                now_ms: Some(1_000),
            })
            .expect("lease ready")
            .expect("leased snapshot");

        let health = repository
            .load_runtime_health(Some(5_000))
            .expect("load runtime health");
        assert_eq!(health.total_count, 2);
        assert_eq!(health.ready_count, 0);
        assert_eq!(health.leased_count, 1);
        assert_eq!(health.blocked_count, 1);
        assert_eq!(health.expired_lease_count, 1);
    }

    #[test]
    fn list_work_units_filters_archived_entries_by_default() {
        let config = isolated_memory_config("list-filter");
        let repository = WorkUnitRepository::new(&config).expect("repository");

        let active = NewWorkUnitRecord {
            work_unit_id: Some("wu-active".to_owned()),
            ..sample_work_unit(WorkUnitStatus::Ready)
        };
        let archived = NewWorkUnitRecord {
            work_unit_id: Some("wu-archived".to_owned()),
            ..sample_work_unit(WorkUnitStatus::Ready)
        };

        repository
            .create_work_unit(active, None)
            .expect("create active");
        repository
            .create_work_unit(archived, None)
            .expect("create archived candidate");
        repository
            .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
                owner: "worker-a".to_owned(),
                ttl_ms: 5_000,
                actor: None,
                now_ms: Some(1_000),
            })
            .expect("lease active")
            .expect("leased active");
        repository
            .complete_work_unit(CompleteWorkUnitRequest {
                work_unit_id: "wu-active".to_owned(),
                owner: "worker-a".to_owned(),
                disposition: WorkUnitCompletionDisposition::Completed,
                actor: None,
                now_ms: Some(2_000),
                next_run_at_ms: None,
                result_payload_json: None,
                error: None,
            })
            .expect("complete active")
            .expect("completed active");
        repository
            .archive_work_unit(ArchiveWorkUnitRequest {
                work_unit_id: "wu-active".to_owned(),
                actor: None,
                now_ms: Some(3_000),
            })
            .expect("archive active")
            .expect("archived active");

        let visible = repository
            .list_work_units(WorkUnitListQuery::default())
            .expect("list visible work units");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].work_unit.work_unit_id, "wu-archived");

        let with_archived = repository
            .list_work_units(WorkUnitListQuery {
                include_archived: true,
                ..WorkUnitListQuery::default()
            })
            .expect("list work units including archived");
        assert_eq!(with_archived.len(), 2);
    }

    #[test]
    fn assignment_dependency_and_note_actions_round_trip_through_snapshot() {
        let config = isolated_memory_config("assignment-dependency-note");
        let repository = WorkUnitRepository::new(&config).expect("repository");
        let blocker = NewWorkUnitRecord {
            work_unit_id: Some("wu-blocker".to_owned()),
            title: "Blocker".to_owned(),
            description: "Complete prerequisite".to_owned(),
            priority: WorkUnitPriority::Low,
            ..sample_work_unit(WorkUnitStatus::Ready)
        };
        let blocked = sample_work_unit(WorkUnitStatus::Ready);

        repository
            .create_work_unit(blocker, Some("operator"))
            .expect("create blocker work unit");
        repository
            .create_work_unit(blocked, Some("operator"))
            .expect("create blocked work unit");

        let assigned = repository
            .assign_work_unit(AssignWorkUnitRequest {
                work_unit_id: "wu-test".to_owned(),
                assigned_to: Some("designer".to_owned()),
                actor: Some("operator".to_owned()),
                now_ms: Some(1_200),
            })
            .expect("assign work unit")
            .expect("assigned snapshot");
        assert_eq!(assigned.work_unit.assigned_to.as_deref(), Some("designer"));

        let dependency_added = repository
            .add_dependency(AddWorkUnitDependencyRequest {
                blocking_work_unit_id: "wu-blocker".to_owned(),
                blocked_work_unit_id: "wu-test".to_owned(),
                actor: Some("operator".to_owned()),
                now_ms: Some(1_300),
            })
            .expect("add dependency")
            .expect("dependency snapshot");
        assert_eq!(
            dependency_added.work_unit.blocked_by_work_unit_ids,
            vec!["wu-blocker".to_owned()]
        );

        let note = repository
            .append_note(AppendWorkUnitNoteRequest {
                work_unit_id: "wu-test".to_owned(),
                actor: Some("operator".to_owned()),
                note: "needs design review".to_owned(),
                now_ms: Some(1_350),
            })
            .expect("append note")
            .expect("note event");
        assert_eq!(note.event_kind, WORK_UNIT_NOTE_ADDED_EVENT_KIND);

        let leased = repository
            .acquire_next_ready_lease(AcquireWorkUnitLeaseRequest {
                owner: "worker-a".to_owned(),
                ttl_ms: 5_000,
                actor: Some("scheduler".to_owned()),
                now_ms: Some(1_400),
            })
            .expect("acquire lease")
            .expect("leased snapshot");
        assert_eq!(leased.work_unit.work_unit_id, "wu-blocker");

        let dependency_removed = repository
            .remove_dependency(RemoveWorkUnitDependencyRequest {
                blocking_work_unit_id: "wu-blocker".to_owned(),
                blocked_work_unit_id: "wu-test".to_owned(),
                actor: Some("operator".to_owned()),
                now_ms: Some(1_500),
            })
            .expect("remove dependency")
            .expect("dependency removed snapshot");
        assert!(
            dependency_removed
                .work_unit
                .blocked_by_work_unit_ids
                .is_empty()
        );

        let snapshot = repository
            .load_work_unit_snapshot("wu-test")
            .expect("load work unit snapshot")
            .expect("work unit snapshot");
        assert_eq!(snapshot.work_unit.assigned_to.as_deref(), Some("designer"));
        assert!(snapshot.work_unit.blocked_by_work_unit_ids.is_empty());

        let blocker_snapshot = repository
            .load_work_unit_snapshot("wu-blocker")
            .expect("load blocker snapshot")
            .expect("blocker snapshot");
        assert!(
            blocker_snapshot.work_unit.blocks_work_unit_ids.is_empty(),
            "dependency removal should clear blocker-side relation view"
        );

        let events = repository
            .list_work_unit_events("wu-test", 20)
            .expect("list work unit events");
        let event_kinds = events
            .iter()
            .map(|event| event.event_kind.as_str())
            .collect::<Vec<_>>();
        assert!(event_kinds.contains(&WORK_UNIT_ASSIGNED_EVENT_KIND));
        assert!(event_kinds.contains(&WORK_UNIT_DEPENDENCY_ADDED_EVENT_KIND));
        assert!(event_kinds.contains(&WORK_UNIT_DEPENDENCY_REMOVED_EVENT_KIND));
        assert!(event_kinds.contains(&WORK_UNIT_NOTE_ADDED_EVENT_KIND));
    }

    #[test]
    fn dependency_cycle_is_rejected_before_persisting_edge() {
        let config = isolated_memory_config("dependency-cycle");
        let repository = WorkUnitRepository::new(&config).expect("repository");
        let first = NewWorkUnitRecord {
            work_unit_id: Some("wu-first".to_owned()),
            title: "First".to_owned(),
            description: "First work unit".to_owned(),
            ..sample_work_unit(WorkUnitStatus::Ready)
        };
        let second = NewWorkUnitRecord {
            work_unit_id: Some("wu-second".to_owned()),
            title: "Second".to_owned(),
            description: "Second work unit".to_owned(),
            ..sample_work_unit(WorkUnitStatus::Ready)
        };

        repository
            .create_work_unit(first, Some("operator"))
            .expect("create first work unit");
        repository
            .create_work_unit(second, Some("operator"))
            .expect("create second work unit");
        repository
            .add_dependency(AddWorkUnitDependencyRequest {
                blocking_work_unit_id: "wu-first".to_owned(),
                blocked_work_unit_id: "wu-second".to_owned(),
                actor: Some("operator".to_owned()),
                now_ms: Some(1_000),
            })
            .expect("add first dependency");

        let error = repository
            .add_dependency(AddWorkUnitDependencyRequest {
                blocking_work_unit_id: "wu-second".to_owned(),
                blocked_work_unit_id: "wu-first".to_owned(),
                actor: Some("operator".to_owned()),
                now_ms: Some(1_100),
            })
            .expect_err("dependency cycle should be rejected");

        assert!(error.contains("would create a cycle"));
    }
}
