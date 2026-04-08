use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::memory::runtime_config::MemoryRuntimeConfig;
use crate::memory::{
    CanonicalMemoryRecord, PersistedConversationTurnRecord,
    canonical_memory_record_from_persisted_turn, session_turn_records_direct,
};

use super::repository::{
    ApprovalRequestRecord, SessionEventRecord, SessionRepository, SessionSummaryRecord,
    SessionTerminalOutcomeRecord,
};

pub const RUNTIME_TRAJECTORY_ARTIFACT_JSON_SCHEMA_VERSION: u32 = 1;
pub const RUNTIME_TRAJECTORY_ARTIFACT_SURFACE: &str = "runtime_trajectory";
pub const RUNTIME_TRAJECTORY_ARTIFACT_PURPOSE: &str = "session_lineage_export";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeTrajectoryExportMode {
    SessionOnly,
    Lineage,
}

impl RuntimeTrajectoryExportMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionOnly => "session_only",
            Self::Lineage => "lineage",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeTrajectoryArtifactSchema {
    pub version: u32,
    pub surface: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeTrajectoryStatistics {
    pub session_count: usize,
    pub turn_count: usize,
    pub terminal_outcome_count: usize,
    pub session_event_count: usize,
    pub approval_request_count: usize,
    pub canonical_kind_counts: BTreeMap<String, usize>,
    pub conversation_event_name_counts: BTreeMap<String, usize>,
    pub session_event_kind_counts: BTreeMap<String, usize>,
    pub approval_status_counts: BTreeMap<String, usize>,
    pub tool_intent_status_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeTrajectoryCanonicalRecord {
    pub scope: String,
    pub kind: String,
    pub role: Option<String>,
    pub content: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeTrajectoryTurnRecord {
    pub row_id: i64,
    pub session_turn_index: i64,
    pub role: String,
    pub content: String,
    pub ts: i64,
    pub canonical_record: RuntimeTrajectoryCanonicalRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeTrajectorySessionSummary {
    pub session_id: String,
    pub kind: String,
    pub parent_session_id: Option<String>,
    pub label: Option<String>,
    pub state: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
    pub turn_count: usize,
    pub last_turn_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeTrajectorySessionEvent {
    pub id: i64,
    pub event_kind: String,
    pub actor_session_id: Option<String>,
    pub payload_json: Value,
    pub ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeTrajectoryTerminalOutcome {
    pub status: String,
    pub payload_json: Value,
    pub recorded_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeTrajectoryApprovalRequest {
    pub approval_request_id: String,
    pub turn_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub approval_key: String,
    pub status: String,
    pub decision: Option<String>,
    pub request_payload_json: Value,
    pub governance_snapshot_json: Value,
    pub requested_at: i64,
    pub resolved_at: Option<i64>,
    pub resolved_by_session_id: Option<String>,
    pub executed_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeTrajectorySession {
    pub summary: RuntimeTrajectorySessionSummary,
    pub lineage_depth: usize,
    pub turns: Vec<RuntimeTrajectoryTurnRecord>,
    pub session_events: Vec<RuntimeTrajectorySessionEvent>,
    pub terminal_outcome: Option<RuntimeTrajectoryTerminalOutcome>,
    pub approval_requests: Vec<RuntimeTrajectoryApprovalRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeTrajectoryArtifactDocument {
    pub schema: RuntimeTrajectoryArtifactSchema,
    pub exported_at: String,
    pub requested_session_id: String,
    pub root_session_id: String,
    pub export_mode: RuntimeTrajectoryExportMode,
    pub sessions: Vec<RuntimeTrajectorySession>,
    pub statistics: RuntimeTrajectoryStatistics,
}

pub fn export_runtime_trajectory(
    requested_session_id: &str,
    export_mode: RuntimeTrajectoryExportMode,
    memory_config: &MemoryRuntimeConfig,
    exported_at: &str,
) -> Result<RuntimeTrajectoryArtifactDocument, String> {
    let requested_session_id = normalize_required_text(
        requested_session_id,
        "runtime trajectory export requires session_id",
    )?;
    let exported_at = normalize_required_text(
        exported_at,
        "runtime trajectory export requires exported_at",
    )?;
    let repo = SessionRepository::new(memory_config)?;
    let requested_summary = repo
        .load_session_summary_with_legacy_fallback(requested_session_id.as_str())?
        .ok_or_else(|| {
            format!("runtime trajectory export session `{requested_session_id}` was not found")
        })?;
    let root_session_id = resolve_root_session_id(&repo, requested_session_id.as_str())?;
    let session_summaries =
        collect_export_session_summaries(&repo, &requested_summary, &root_session_id, export_mode)?;
    let sessions = collect_export_sessions(&repo, memory_config, session_summaries.as_slice())?;
    let statistics = build_runtime_trajectory_statistics(sessions.as_slice());

    Ok(RuntimeTrajectoryArtifactDocument {
        schema: RuntimeTrajectoryArtifactSchema {
            version: RUNTIME_TRAJECTORY_ARTIFACT_JSON_SCHEMA_VERSION,
            surface: RUNTIME_TRAJECTORY_ARTIFACT_SURFACE.to_owned(),
            purpose: RUNTIME_TRAJECTORY_ARTIFACT_PURPOSE.to_owned(),
        },
        exported_at,
        requested_session_id,
        root_session_id,
        export_mode,
        sessions,
        statistics,
    })
}

fn normalize_required_text(raw: &str, error_message: &str) -> Result<String, String> {
    let normalized = raw.trim();
    if normalized.is_empty() {
        return Err(error_message.to_owned());
    }
    Ok(normalized.to_owned())
}

fn resolve_root_session_id(
    repo: &SessionRepository,
    requested_session_id: &str,
) -> Result<String, String> {
    let lineage_root = repo.lineage_root_session_id(requested_session_id)?;
    let root_session_id = lineage_root.unwrap_or_else(|| requested_session_id.to_owned());
    Ok(root_session_id)
}

fn collect_export_session_summaries(
    repo: &SessionRepository,
    requested_summary: &SessionSummaryRecord,
    root_session_id: &str,
    export_mode: RuntimeTrajectoryExportMode,
) -> Result<Vec<SessionSummaryRecord>, String> {
    let mut session_summaries = if export_mode == RuntimeTrajectoryExportMode::Lineage {
        repo.list_visible_sessions(root_session_id)?
    } else {
        vec![requested_summary.clone()]
    };

    sort_runtime_trajectory_sessions(repo, &mut session_summaries)?;
    Ok(session_summaries)
}

fn sort_runtime_trajectory_sessions(
    repo: &SessionRepository,
    sessions: &mut [SessionSummaryRecord],
) -> Result<(), String> {
    let mut depth_by_session_id = BTreeMap::new();
    for session in sessions.iter() {
        let depth = repo.session_lineage_depth(session.session_id.as_str())?;
        let session_id = session.session_id.clone();
        depth_by_session_id.insert(session_id, depth);
    }

    sessions.sort_by(|left, right| {
        let left_depth = depth_by_session_id
            .get(left.session_id.as_str())
            .copied()
            .unwrap_or_default();
        let right_depth = depth_by_session_id
            .get(right.session_id.as_str())
            .copied()
            .unwrap_or_default();
        left_depth
            .cmp(&right_depth)
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    Ok(())
}

fn collect_export_sessions(
    repo: &SessionRepository,
    memory_config: &MemoryRuntimeConfig,
    session_summaries: &[SessionSummaryRecord],
) -> Result<Vec<RuntimeTrajectorySession>, String> {
    let mut sessions = Vec::with_capacity(session_summaries.len());
    for session_summary in session_summaries {
        let session = build_runtime_trajectory_session(repo, memory_config, session_summary)?;
        sessions.push(session);
    }
    Ok(sessions)
}

fn build_runtime_trajectory_session(
    repo: &SessionRepository,
    memory_config: &MemoryRuntimeConfig,
    session_summary: &SessionSummaryRecord,
) -> Result<RuntimeTrajectorySession, String> {
    let session_id = session_summary.session_id.as_str();
    let lineage_depth = repo.session_lineage_depth(session_id)?;
    let turns = session_turn_records_direct(session_id, memory_config)?
        .into_iter()
        .map(|turn| runtime_trajectory_turn_record(session_id, turn))
        .collect::<Vec<_>>();
    let session_events = repo
        .list_all_events(session_id)?
        .iter()
        .map(runtime_trajectory_session_event)
        .collect::<Vec<_>>();
    let terminal_outcome = repo
        .load_terminal_outcome(session_id)?
        .as_ref()
        .map(runtime_trajectory_terminal_outcome);
    let approval_requests = repo
        .list_approval_requests_for_session(session_id, None)?
        .iter()
        .map(runtime_trajectory_approval_request)
        .collect::<Vec<_>>();

    Ok(RuntimeTrajectorySession {
        summary: runtime_trajectory_session_summary(session_summary),
        lineage_depth,
        turns,
        session_events,
        terminal_outcome,
        approval_requests,
    })
}

fn runtime_trajectory_turn_record(
    session_id: &str,
    turn: PersistedConversationTurnRecord,
) -> RuntimeTrajectoryTurnRecord {
    let canonical_record = canonical_memory_record_from_persisted_turn(
        session_id,
        turn.role.as_str(),
        turn.content.as_str(),
    );
    let canonical_record = runtime_trajectory_canonical_record(&canonical_record);

    RuntimeTrajectoryTurnRecord {
        row_id: turn.row_id,
        session_turn_index: turn.session_turn_index,
        role: turn.role,
        content: turn.content,
        ts: turn.ts,
        canonical_record,
    }
}

fn runtime_trajectory_canonical_record(
    record: &CanonicalMemoryRecord,
) -> RuntimeTrajectoryCanonicalRecord {
    RuntimeTrajectoryCanonicalRecord {
        scope: record.scope.as_str().to_owned(),
        kind: record.kind.as_str().to_owned(),
        role: record.role.clone(),
        content: record.content.clone(),
        metadata: record.metadata.clone(),
    }
}

fn runtime_trajectory_session_summary(
    summary: &SessionSummaryRecord,
) -> RuntimeTrajectorySessionSummary {
    RuntimeTrajectorySessionSummary {
        session_id: summary.session_id.clone(),
        kind: summary.kind.as_str().to_owned(),
        parent_session_id: summary.parent_session_id.clone(),
        label: summary.label.clone(),
        state: summary.state.as_str().to_owned(),
        created_at: summary.created_at,
        updated_at: summary.updated_at,
        archived_at: summary.archived_at,
        turn_count: summary.turn_count,
        last_turn_at: summary.last_turn_at,
        last_error: summary.last_error.clone(),
    }
}

fn runtime_trajectory_session_event(event: &SessionEventRecord) -> RuntimeTrajectorySessionEvent {
    RuntimeTrajectorySessionEvent {
        id: event.id,
        event_kind: event.event_kind.clone(),
        actor_session_id: event.actor_session_id.clone(),
        payload_json: event.payload_json.clone(),
        ts: event.ts,
    }
}

fn runtime_trajectory_terminal_outcome(
    outcome: &SessionTerminalOutcomeRecord,
) -> RuntimeTrajectoryTerminalOutcome {
    RuntimeTrajectoryTerminalOutcome {
        status: outcome.status.clone(),
        payload_json: outcome.payload_json.clone(),
        recorded_at: outcome.recorded_at,
    }
}

fn runtime_trajectory_approval_request(
    request: &ApprovalRequestRecord,
) -> RuntimeTrajectoryApprovalRequest {
    let decision = request
        .decision
        .map(|decision| decision.as_str().to_owned());
    let status = request.status.as_str().to_owned();
    RuntimeTrajectoryApprovalRequest {
        approval_request_id: request.approval_request_id.clone(),
        turn_id: request.turn_id.clone(),
        tool_call_id: request.tool_call_id.clone(),
        tool_name: request.tool_name.clone(),
        approval_key: request.approval_key.clone(),
        status,
        decision,
        request_payload_json: request.request_payload_json.clone(),
        governance_snapshot_json: request.governance_snapshot_json.clone(),
        requested_at: request.requested_at,
        resolved_at: request.resolved_at,
        resolved_by_session_id: request.resolved_by_session_id.clone(),
        executed_at: request.executed_at,
        last_error: request.last_error.clone(),
    }
}

fn build_runtime_trajectory_statistics(
    sessions: &[RuntimeTrajectorySession],
) -> RuntimeTrajectoryStatistics {
    let session_count = sessions.len();
    let mut turn_count = 0usize;
    let mut terminal_outcome_count = 0usize;
    let mut session_event_count = 0usize;
    let mut approval_request_count = 0usize;
    let mut canonical_kind_counts = BTreeMap::new();
    let mut conversation_event_name_counts = BTreeMap::new();
    let mut session_event_kind_counts = BTreeMap::new();
    let mut approval_status_counts = BTreeMap::new();
    let mut tool_intent_status_counts = BTreeMap::new();

    for session in sessions {
        turn_count += session.turns.len();
        session_event_count += session.session_events.len();
        approval_request_count += session.approval_requests.len();
        if session.terminal_outcome.is_some() {
            terminal_outcome_count += 1;
        }

        for turn in &session.turns {
            let kind = turn.canonical_record.kind.clone();
            let current_count = canonical_kind_counts
                .get(kind.as_str())
                .copied()
                .unwrap_or_default();
            let next_count = current_count + 1;
            canonical_kind_counts.insert(kind, next_count);
            record_conversation_event_counts(
                turn,
                &mut conversation_event_name_counts,
                &mut tool_intent_status_counts,
            );
        }

        for event in &session.session_events {
            let event_kind = event.event_kind.clone();
            let current_count = session_event_kind_counts
                .get(event_kind.as_str())
                .copied()
                .unwrap_or_default();
            let next_count = current_count + 1;
            session_event_kind_counts.insert(event_kind, next_count);
        }

        for approval_request in &session.approval_requests {
            let status = approval_request.status.clone();
            let current_count = approval_status_counts
                .get(status.as_str())
                .copied()
                .unwrap_or_default();
            let next_count = current_count + 1;
            approval_status_counts.insert(status, next_count);
        }
    }

    RuntimeTrajectoryStatistics {
        session_count,
        turn_count,
        terminal_outcome_count,
        session_event_count,
        approval_request_count,
        canonical_kind_counts,
        conversation_event_name_counts,
        session_event_kind_counts,
        approval_status_counts,
        tool_intent_status_counts,
    }
}

fn record_conversation_event_counts(
    turn: &RuntimeTrajectoryTurnRecord,
    conversation_event_name_counts: &mut BTreeMap<String, usize>,
    tool_intent_status_counts: &mut BTreeMap<String, usize>,
) {
    if turn.canonical_record.kind != "conversation_event" {
        return;
    }

    let metadata = &turn.canonical_record.metadata;
    let Some(event_name) = metadata.get("event").and_then(Value::as_str) else {
        return;
    };

    let event_name = event_name.to_owned();
    let current_event_count = conversation_event_name_counts
        .get(event_name.as_str())
        .copied()
        .unwrap_or_default();
    let next_event_count = current_event_count + 1;
    conversation_event_name_counts.insert(event_name, next_event_count);

    let Some(payload) = metadata.get("payload").and_then(Value::as_object) else {
        return;
    };
    let Some(intent_outcomes) = payload.get("intent_outcomes").and_then(Value::as_array) else {
        return;
    };

    for intent_outcome in intent_outcomes {
        let Some(status) = intent_outcome.get("status").and_then(Value::as_str) else {
            continue;
        };
        let status = status.to_owned();
        let current_status_count = tool_intent_status_counts
            .get(status.as_str())
            .copied()
            .unwrap_or_default();
        let next_status_count = current_status_count + 1;
        tool_intent_status_counts.insert(status, next_status_count);
    }
}
