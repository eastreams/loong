use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::memory;
use crate::memory::ConversationTurn;
use crate::memory::canonical_memory_record_from_persisted_turn;
use crate::memory::runtime_config::MemoryRuntimeConfig;

use super::repository::ApprovalDecision;
use super::repository::ApprovalRequestRecord;
use super::repository::SessionEventRecord;
use super::repository::SessionRepository;
use super::repository::SessionSummaryRecord;
use super::repository::SessionTerminalOutcomeRecord;

pub const SESSION_TRAJECTORY_ARTIFACT_JSON_SCHEMA_VERSION: u32 = 1;
pub const SESSION_TRAJECTORY_ARTIFACT_SURFACE: &str = "runtime_trajectory";
pub const SESSION_TRAJECTORY_ARTIFACT_PURPOSE: &str = "runtime_trajectory_export";
const DEFAULT_EVENT_PAGE_LIMIT: usize = 200;
const DEFAULT_TRANSCRIPT_PAGE_SIZE: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTrajectoryExportOptions {
    pub turn_limit: Option<usize>,
    pub event_page_limit: usize,
}

impl Default for SessionTrajectoryExportOptions {
    fn default() -> Self {
        Self {
            turn_limit: None,
            event_page_limit: DEFAULT_EVENT_PAGE_LIMIT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTrajectoryArtifactSchema {
    pub version: u32,
    pub surface: String,
    pub purpose: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTrajectorySession {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTrajectoryLineage {
    pub root_session_id: Option<String>,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTrajectoryTurn {
    pub sequence: usize,
    pub role: String,
    pub content: String,
    pub ts: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTrajectoryEvent {
    pub id: i64,
    pub session_id: String,
    pub event_kind: String,
    pub actor_session_id: Option<String>,
    pub payload_json: Value,
    pub ts: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTrajectoryTerminalOutcome {
    pub session_id: String,
    pub status: String,
    pub payload_json: Value,
    pub recorded_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTrajectoryCanonicalRecord {
    pub scope: String,
    pub kind: String,
    pub role: Option<String>,
    pub content: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTrajectoryApprovalRequest {
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTrajectoryArtifact {
    pub schema: SessionTrajectoryArtifactSchema,
    pub exported_at: String,
    pub session: SessionTrajectorySession,
    pub lineage: SessionTrajectoryLineage,
    pub exported_turn_count: usize,
    pub turns_truncated: bool,
    pub turns: Vec<SessionTrajectoryTurn>,
    pub canonical_record_count: usize,
    pub canonical_records: Vec<SessionTrajectoryCanonicalRecord>,
    pub event_count: usize,
    pub event_page_limit: usize,
    pub events: Vec<SessionTrajectoryEvent>,
    pub approval_request_count: usize,
    pub approval_requests: Vec<SessionTrajectoryApprovalRequest>,
    pub terminal_outcome: Option<SessionTrajectoryTerminalOutcome>,
}

pub fn export_session_trajectory(
    session_id: &str,
    config: &MemoryRuntimeConfig,
    options: &SessionTrajectoryExportOptions,
) -> Result<SessionTrajectoryArtifact, String> {
    validate_export_options(options)?;

    let repository = SessionRepository::new(config)?;
    let summary_option = repository.load_session_summary_with_legacy_fallback(session_id)?;
    let summary = summary_option.ok_or_else(|| format!("session `{session_id}` not found"))?;
    let turn_limit = resolve_turn_limit(summary.turn_count, options.turn_limit)?;
    let root_session_id = repository.lineage_root_session_id(session_id)?;
    let lineage_depth = repository.session_lineage_depth(session_id)?;

    let turns = load_export_turns(session_id, turn_limit, config)?;

    let events = repository.list_all_events(session_id, options.event_page_limit)?;
    let approval_requests = repository.list_approval_requests_for_session(session_id, None)?;
    let terminal_outcome = repository.load_terminal_outcome(session_id)?;
    let turns_truncated = summary.turn_count > turn_limit;
    let exported_at = now_rfc3339()?;
    let session = SessionTrajectorySession::from_summary(&summary);
    let lineage = SessionTrajectoryLineage {
        root_session_id,
        depth: lineage_depth,
    };
    let first_sequence = resolve_first_sequence(summary.turn_count, turn_limit);
    let trajectory_turns = build_trajectory_turns(&turns, first_sequence);
    let canonical_records = build_canonical_records(session_id, &turns);
    let trajectory_events = build_trajectory_events(&events);
    let trajectory_approval_requests = build_approval_requests(&approval_requests);
    let trajectory_outcome = terminal_outcome
        .as_ref()
        .map(SessionTrajectoryTerminalOutcome::from_terminal_outcome);
    let exported_turn_count = trajectory_turns.len();
    let canonical_record_count = canonical_records.len();
    let event_count = trajectory_events.len();
    let approval_request_count = trajectory_approval_requests.len();
    let schema = SessionTrajectoryArtifactSchema::default();

    Ok(SessionTrajectoryArtifact {
        schema,
        exported_at,
        session,
        lineage,
        exported_turn_count,
        turns_truncated,
        turns: trajectory_turns,
        canonical_record_count,
        canonical_records,
        event_count,
        event_page_limit: options.event_page_limit,
        events: trajectory_events,
        approval_request_count,
        approval_requests: trajectory_approval_requests,
        terminal_outcome: trajectory_outcome,
    })
}

fn load_export_turns(
    session_id: &str,
    turn_limit: usize,
    config: &MemoryRuntimeConfig,
) -> Result<Vec<ConversationTurn>, String> {
    if turn_limit == 0 {
        return Ok(Vec::new());
    }

    let recent_turns = memory::window_direct(session_id, turn_limit, config)?;
    if recent_turns.len() == turn_limit {
        return Ok(recent_turns);
    }

    let transcript =
        memory::transcript_direct_paged(session_id, DEFAULT_TRANSCRIPT_PAGE_SIZE, config)?;

    Ok(transcript)
}

fn validate_export_options(options: &SessionTrajectoryExportOptions) -> Result<(), String> {
    if options.event_page_limit == 0 {
        return Err("session trajectory event_page_limit must be >= 1".to_owned());
    }

    if matches!(options.turn_limit, Some(0)) {
        return Err("session trajectory turn_limit must be >= 1 when provided".to_owned());
    }

    Ok(())
}

fn resolve_turn_limit(
    total_turn_count: usize,
    requested_turn_limit: Option<usize>,
) -> Result<usize, String> {
    if total_turn_count == 0 {
        return Ok(0);
    }

    let Some(requested_turn_limit) = requested_turn_limit else {
        return Ok(total_turn_count);
    };

    if requested_turn_limit == 0 {
        return Err("session trajectory turn_limit must be >= 1 when provided".to_owned());
    }

    let bounded_turn_limit = requested_turn_limit.min(total_turn_count);
    Ok(bounded_turn_limit)
}

fn resolve_first_sequence(total_turn_count: usize, exported_turn_count: usize) -> usize {
    if exported_turn_count == 0 {
        return 0;
    }

    let hidden_turn_count = total_turn_count.saturating_sub(exported_turn_count);
    hidden_turn_count.saturating_add(1)
}

fn build_trajectory_turns(
    turns: &[ConversationTurn],
    first_sequence: usize,
) -> Vec<SessionTrajectoryTurn> {
    let mut trajectory_turns = Vec::with_capacity(turns.len());

    for (index, turn) in turns.iter().enumerate() {
        let offset = index;
        let sequence = first_sequence.saturating_add(offset);
        let trajectory_turn = SessionTrajectoryTurn::from_turn(sequence, turn);
        trajectory_turns.push(trajectory_turn);
    }

    trajectory_turns
}

fn build_trajectory_events(events: &[SessionEventRecord]) -> Vec<SessionTrajectoryEvent> {
    let mut trajectory_events = Vec::with_capacity(events.len());

    for event in events {
        let trajectory_event = SessionTrajectoryEvent::from_event(event);
        trajectory_events.push(trajectory_event);
    }

    trajectory_events
}

fn build_canonical_records(
    session_id: &str,
    turns: &[ConversationTurn],
) -> Vec<SessionTrajectoryCanonicalRecord> {
    let mut canonical_records = Vec::with_capacity(turns.len());

    for turn in turns {
        let role = turn.role.as_str();
        let content = turn.content.as_str();
        let canonical_record =
            canonical_memory_record_from_persisted_turn(session_id, role, content);
        let trajectory_record = SessionTrajectoryCanonicalRecord::from_record(&canonical_record);
        canonical_records.push(trajectory_record);
    }

    canonical_records
}

fn build_approval_requests(
    approval_requests: &[ApprovalRequestRecord],
) -> Vec<SessionTrajectoryApprovalRequest> {
    let mut trajectory_approval_requests = Vec::with_capacity(approval_requests.len());

    for approval_request in approval_requests {
        let trajectory_approval_request =
            SessionTrajectoryApprovalRequest::from_record(approval_request);
        trajectory_approval_requests.push(trajectory_approval_request);
    }

    trajectory_approval_requests
}

fn now_rfc3339() -> Result<String, String> {
    let now = OffsetDateTime::now_utc();
    let formatted = now
        .format(&Rfc3339)
        .map_err(|error| format!("format session trajectory export timestamp failed: {error}"))?;
    Ok(formatted)
}

impl Default for SessionTrajectoryArtifactSchema {
    fn default() -> Self {
        Self {
            version: SESSION_TRAJECTORY_ARTIFACT_JSON_SCHEMA_VERSION,
            surface: SESSION_TRAJECTORY_ARTIFACT_SURFACE.to_owned(),
            purpose: SESSION_TRAJECTORY_ARTIFACT_PURPOSE.to_owned(),
        }
    }
}

impl SessionTrajectorySession {
    fn from_summary(summary: &SessionSummaryRecord) -> Self {
        let kind = summary.kind.as_str().to_owned();
        let state = summary.state.as_str().to_owned();

        Self {
            session_id: summary.session_id.clone(),
            kind,
            parent_session_id: summary.parent_session_id.clone(),
            label: summary.label.clone(),
            state,
            created_at: summary.created_at,
            updated_at: summary.updated_at,
            archived_at: summary.archived_at,
            turn_count: summary.turn_count,
            last_turn_at: summary.last_turn_at,
            last_error: summary.last_error.clone(),
        }
    }
}

impl SessionTrajectoryTurn {
    fn from_turn(sequence: usize, turn: &ConversationTurn) -> Self {
        Self {
            sequence,
            role: turn.role.clone(),
            content: turn.content.clone(),
            ts: turn.ts,
        }
    }
}

impl SessionTrajectoryEvent {
    fn from_event(event: &SessionEventRecord) -> Self {
        Self {
            id: event.id,
            session_id: event.session_id.clone(),
            event_kind: event.event_kind.clone(),
            actor_session_id: event.actor_session_id.clone(),
            payload_json: event.payload_json.clone(),
            ts: event.ts,
        }
    }
}

impl SessionTrajectoryTerminalOutcome {
    fn from_terminal_outcome(outcome: &SessionTerminalOutcomeRecord) -> Self {
        Self {
            session_id: outcome.session_id.clone(),
            status: outcome.status.clone(),
            payload_json: outcome.payload_json.clone(),
            recorded_at: outcome.recorded_at,
        }
    }
}

impl SessionTrajectoryCanonicalRecord {
    fn from_record(record: &crate::memory::CanonicalMemoryRecord) -> Self {
        let scope = record.scope.as_str().to_owned();
        let kind = record.kind.as_str().to_owned();

        Self {
            scope,
            kind,
            role: record.role.clone(),
            content: record.content.clone(),
            metadata: record.metadata.clone(),
        }
    }
}

impl SessionTrajectoryApprovalRequest {
    fn from_record(record: &ApprovalRequestRecord) -> Self {
        let status = record.status.as_str().to_owned();
        let decision = record
            .decision
            .map(ApprovalDecision::as_str)
            .map(str::to_owned);

        Self {
            approval_request_id: record.approval_request_id.clone(),
            turn_id: record.turn_id.clone(),
            tool_call_id: record.tool_call_id.clone(),
            tool_name: record.tool_name.clone(),
            approval_key: record.approval_key.clone(),
            status,
            decision,
            request_payload_json: record.request_payload_json.clone(),
            governance_snapshot_json: record.governance_snapshot_json.clone(),
            requested_at: record.requested_at,
            resolved_at: record.resolved_at,
            resolved_by_session_id: record.resolved_by_session_id.clone(),
            executed_at: record.executed_at,
            last_error: record.last_error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::SessionTrajectoryExportOptions;
    use super::export_session_trajectory;
    use crate::memory;
    use crate::memory::runtime_config::MemoryRuntimeConfig;
    use crate::session::repository::FinalizeSessionTerminalRequest;
    use crate::session::repository::NewApprovalRequestRecord;
    use crate::session::repository::NewSessionEvent;
    use crate::session::repository::NewSessionRecord;
    use crate::session::repository::SessionKind;
    use crate::session::repository::SessionRecord;
    use crate::session::repository::SessionRepository;
    use crate::session::repository::SessionState;
    use crate::test_support::unique_temp_dir;

    fn isolated_memory_config(test_name: &str) -> MemoryRuntimeConfig {
        let root = unique_temp_dir(test_name);
        let sqlite_path = root.join("memory.sqlite3");
        MemoryRuntimeConfig {
            sqlite_path: Some(sqlite_path),
            ..MemoryRuntimeConfig::default()
        }
    }

    fn append_turns(
        session_id: &str,
        config: &MemoryRuntimeConfig,
        contents: &[&str],
    ) -> Result<(), String> {
        for content in contents {
            memory::append_turn_direct(session_id, "assistant", content, config)?;
        }

        Ok(())
    }

    fn create_session(
        repository: &SessionRepository,
        session_id: &str,
    ) -> Result<SessionRecord, String> {
        let record = NewSessionRecord {
            session_id: session_id.to_owned(),
            kind: SessionKind::Root,
            parent_session_id: None,
            label: Some("Root".to_owned()),
            state: SessionState::Running,
        };
        repository.create_session(record)
    }

    #[test]
    fn export_session_trajectory_collects_turns_events_and_terminal_outcome() {
        let config = isolated_memory_config("session-trajectory-full");
        let repository = SessionRepository::new(&config).expect("repository");
        create_session(&repository, "root-session").expect("create session");
        append_turns("root-session", &config, &["step one", "step two"]).expect("append turns");
        let start_payload = json!({
            "task": "summarize"
        });
        let start_event = NewSessionEvent {
            session_id: "root-session".to_owned(),
            event_kind: "delegate_started".to_owned(),
            actor_session_id: Some("operator".to_owned()),
            payload_json: start_payload,
        };
        repository.append_event(start_event).expect("append event");

        let approval_payload = json!({
            "tool_name": "delegate"
        });
        let governance_payload = json!({
            "rule_id": "delegate_review"
        });
        let approval_request = NewApprovalRequestRecord {
            approval_request_id: "approval-1".to_owned(),
            session_id: "root-session".to_owned(),
            turn_id: "turn-1".to_owned(),
            tool_call_id: "tool-call-1".to_owned(),
            tool_name: "delegate".to_owned(),
            approval_key: "tool:delegate".to_owned(),
            request_payload_json: approval_payload,
            governance_snapshot_json: governance_payload,
        };
        repository
            .ensure_approval_request(approval_request)
            .expect("ensure approval request");

        let terminal_event_payload = json!({
            "task": "summarize"
        });
        let terminal_outcome_payload = json!({
            "summary": "done"
        });
        let finalize_request = FinalizeSessionTerminalRequest {
            state: SessionState::Completed,
            last_error: None,
            event_kind: "delegate_completed".to_owned(),
            actor_session_id: Some("operator".to_owned()),
            event_payload_json: terminal_event_payload,
            outcome_status: "ok".to_owned(),
            outcome_payload_json: terminal_outcome_payload,
        };
        repository
            .finalize_session_terminal("root-session", finalize_request)
            .expect("finalize session");

        let options = SessionTrajectoryExportOptions::default();
        let artifact =
            export_session_trajectory("root-session", &config, &options).expect("export");

        assert_eq!(artifact.session.session_id, "root-session");
        assert_eq!(artifact.session.turn_count, 2);
        assert_eq!(
            artifact.lineage.root_session_id.as_deref(),
            Some("root-session")
        );
        assert_eq!(artifact.lineage.depth, 0);
        assert_eq!(artifact.exported_turn_count, 2);
        assert!(!artifact.turns_truncated);
        assert_eq!(artifact.canonical_record_count, 2);
        assert_eq!(artifact.canonical_records[0].kind, "assistant_turn");
        assert_eq!(artifact.event_count, 2);
        assert_eq!(artifact.events[0].event_kind, "delegate_started");
        assert_eq!(artifact.events[1].event_kind, "delegate_completed");
        assert_eq!(artifact.approval_request_count, 1);
        assert_eq!(artifact.approval_requests[0].tool_name, "delegate");
        assert_eq!(
            artifact
                .terminal_outcome
                .as_ref()
                .expect("terminal outcome")
                .status,
            "ok"
        );
    }

    #[test]
    fn export_session_trajectory_applies_turn_limit_without_guessing_counts() {
        let config = isolated_memory_config("session-trajectory-turn-limit");
        let repository = SessionRepository::new(&config).expect("repository");
        create_session(&repository, "root-session").expect("create session");
        append_turns("root-session", &config, &["one", "two", "three"]).expect("append turns");

        let options = SessionTrajectoryExportOptions {
            turn_limit: Some(2),
            ..SessionTrajectoryExportOptions::default()
        };
        let artifact =
            export_session_trajectory("root-session", &config, &options).expect("export");

        assert_eq!(artifact.session.turn_count, 3);
        assert_eq!(artifact.exported_turn_count, 2);
        assert!(artifact.turns_truncated);
        assert_eq!(artifact.turns[0].sequence, 2);
        assert_eq!(artifact.turns[1].sequence, 3);
        assert_eq!(artifact.turns[0].content, "two");
        assert_eq!(artifact.turns[1].content, "three");
    }

    #[test]
    fn export_session_trajectory_rejects_invalid_options() {
        let config = isolated_memory_config("session-trajectory-invalid-options");
        let repository = SessionRepository::new(&config).expect("repository");
        create_session(&repository, "root-session").expect("create session");

        let invalid_turn_limit_options = SessionTrajectoryExportOptions {
            turn_limit: Some(0),
            event_page_limit: 10,
        };
        let turn_limit_error =
            export_session_trajectory("root-session", &config, &invalid_turn_limit_options)
                .expect_err("zero turn limit must fail");
        assert!(turn_limit_error.contains("turn_limit"));

        let invalid_event_page_options = SessionTrajectoryExportOptions {
            turn_limit: None,
            event_page_limit: 0,
        };
        let event_page_error =
            export_session_trajectory("root-session", &config, &invalid_event_page_options)
                .expect_err("zero event page limit must fail");
        assert!(event_page_error.contains("event_page_limit"));
    }
}
