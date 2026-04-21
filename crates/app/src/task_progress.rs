use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Value, json};

#[cfg(feature = "memory-sqlite")]
use crate::session::repository::SessionRepository;

pub(crate) const TASK_PROGRESS_EVENT_KIND: &str = "task_progress";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaskProgressStatus {
    #[default]
    Active,
    Waiting,
    Blocked,
    Verifying,
    Completed,
    Failed,
}

impl TaskProgressStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Waiting => "waiting",
            Self::Blocked => "blocked",
            Self::Verifying => "verifying",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub const fn is_stable(self) -> bool {
        matches!(
            self,
            Self::Waiting | Self::Blocked | Self::Completed | Self::Failed
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaskVerificationState {
    #[default]
    NotStarted,
    Pending,
    Passed,
    Failed,
}

impl TaskVerificationState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::Pending => "pending",
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct TaskActiveHandleRecord {
    pub handle_kind: String,
    pub handle_id: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<i64>,
    pub stop_condition: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TaskResumeRecipeRecord {
    pub recommended_tool: String,
    pub session_id: String,
    pub note: Option<String>,
}

#[cfg(test)]
impl TaskResumeRecipeRecord {
    #[must_use]
    pub fn task_session_id(&self) -> &str {
        &self.session_id
    }
}

impl Serialize for TaskResumeRecipeRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(if self.note.is_some() { 4 } else { 3 }))?;
        map.serialize_entry("recommended_tool", &self.recommended_tool)?;
        map.serialize_entry("task_session_id", &self.session_id)?;
        map.serialize_entry("session_id", &self.session_id)?;
        if let Some(note) = &self.note {
            map.serialize_entry("note", note)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for TaskResumeRecipeRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct RawTaskResumeRecipeRecord {
            recommended_tool: String,
            task_session_id: Option<String>,
            session_id: Option<String>,
            note: Option<String>,
        }

        let raw = RawTaskResumeRecipeRecord::deserialize(deserializer)?;
        let session_id = raw.task_session_id.or(raw.session_id).unwrap_or_default();

        Ok(TaskResumeRecipeRecord {
            recommended_tool: raw.recommended_tool,
            session_id,
            note: raw.note,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskProgressRecord {
    pub task_id: String,
    pub owner_kind: String,
    pub status: TaskProgressStatus,
    pub intent_summary: Option<String>,
    pub verification_state: Option<TaskVerificationState>,
    pub active_handles: Vec<TaskActiveHandleRecord>,
    pub resume_recipe: Option<TaskResumeRecipeRecord>,
    pub updated_at: i64,
}

impl Default for TaskProgressRecord {
    fn default() -> Self {
        Self {
            task_id: String::new(),
            owner_kind: String::new(),
            status: TaskProgressStatus::Active,
            intent_summary: None,
            verification_state: None,
            active_handles: Vec::new(),
            resume_recipe: None,
            updated_at: 0,
        }
    }
}

impl Serialize for TaskProgressRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut entry_count = 5;
        if self.intent_summary.is_some() {
            entry_count += 1;
        }
        if self.verification_state.is_some() {
            entry_count += 1;
        }
        if !self.active_handles.is_empty() {
            entry_count += 1;
        }
        if self.resume_recipe.is_some() {
            entry_count += 1;
        }

        let mut map = serializer.serialize_map(Some(entry_count))?;
        map.serialize_entry("task_id", &self.task_id)?;
        map.serialize_entry("session_id", &self.task_id)?;
        map.serialize_entry("owner_kind", &self.owner_kind)?;
        map.serialize_entry("status", &self.status)?;
        if let Some(intent_summary) = &self.intent_summary {
            map.serialize_entry("intent_summary", intent_summary)?;
        }
        if let Some(verification_state) = &self.verification_state {
            map.serialize_entry("verification_state", verification_state)?;
        }
        if !self.active_handles.is_empty() {
            map.serialize_entry("active_handles", &self.active_handles)?;
        }
        if let Some(resume_recipe) = &self.resume_recipe {
            map.serialize_entry("resume_recipe", resume_recipe)?;
        }
        map.serialize_entry("updated_at", &self.updated_at)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for TaskProgressRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct RawTaskProgressRecord {
            task_id: Option<String>,
            session_id: Option<String>,
            owner_kind: String,
            status: TaskProgressStatus,
            intent_summary: Option<String>,
            verification_state: Option<TaskVerificationState>,
            active_handles: Vec<TaskActiveHandleRecord>,
            resume_recipe: Option<TaskResumeRecipeRecord>,
            updated_at: i64,
        }

        let raw = RawTaskProgressRecord::deserialize(deserializer)?;
        let task_id = raw.task_id.or(raw.session_id).unwrap_or_default();

        Ok(TaskProgressRecord {
            task_id,
            owner_kind: raw.owner_kind,
            status: raw.status,
            intent_summary: raw.intent_summary,
            verification_state: raw.verification_state,
            active_handles: raw.active_handles,
            resume_recipe: raw.resume_recipe,
            updated_at: raw.updated_at,
        })
    }
}

pub(crate) fn task_progress_from_event_payload(payload: &Value) -> Option<TaskProgressRecord> {
    let task_progress = payload
        .get("task_progress")
        .cloned()
        .unwrap_or_else(|| payload.clone());
    serde_json::from_value(task_progress).ok()
}

pub(crate) fn task_progress_event_payload(
    source: &str,
    task_progress: &TaskProgressRecord,
) -> Value {
    json!({
        "source": source,
        "task_progress": task_progress,
    })
}

#[cfg(feature = "memory-sqlite")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedTaskIdentity {
    pub task_id: String,
    pub task_session_id: String,
}

#[cfg(feature = "memory-sqlite")]
impl ResolvedTaskIdentity {
    fn fallback(session_id: &str) -> Self {
        Self {
            task_id: session_id.to_owned(),
            task_session_id: session_id.to_owned(),
        }
    }
}

#[cfg(feature = "memory-sqlite")]
fn non_empty_string_field(payload: &Value, field: &str) -> Option<String> {
    let value = payload.get(field)?;
    let value = value.as_str()?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    Some(value.to_owned())
}

#[cfg(feature = "memory-sqlite")]
fn task_identity_from_delegate_payload(
    payload: &Value,
    session_id: &str,
) -> Option<ResolvedTaskIdentity> {
    let task_scope = payload.get("task_scope")?;
    let task_id = non_empty_string_field(task_scope, "task_id")?;
    let task_session_id =
        non_empty_string_field(payload, "task_session_id").unwrap_or_else(|| session_id.to_owned());

    Some(ResolvedTaskIdentity {
        task_id,
        task_session_id,
    })
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn resolve_canonical_task_id_for_session(
    repo: &SessionRepository,
    session_id: &str,
) -> Option<String> {
    let task_identity = resolve_task_identity_for_session(repo, session_id);
    Some(task_identity.task_id)
}

#[cfg(feature = "memory-sqlite")]
pub(crate) fn resolve_task_identity_for_session(
    repo: &SessionRepository,
    session_id: &str,
) -> ResolvedTaskIdentity {
    let fallback = ResolvedTaskIdentity::fallback(session_id);
    let latest_task_progress = repo.load_latest_event_by_kind(session_id, TASK_PROGRESS_EVENT_KIND);
    if let Ok(Some(latest_task_progress)) = latest_task_progress {
        let payload = &latest_task_progress.payload_json;
        let task_progress = task_progress_from_event_payload(payload);
        if let Some(task_progress) = task_progress {
            let task_id = task_progress.task_id.trim();
            if !task_id.is_empty() {
                return ResolvedTaskIdentity {
                    task_id: task_id.to_owned(),
                    task_session_id: session_id.to_owned(),
                };
            }
        }
    }

    let delegate_events = repo.list_delegate_lifecycle_events(session_id);
    if let Ok(delegate_events) = delegate_events {
        for delegate_event in delegate_events.into_iter().rev() {
            let event_kind = delegate_event.event_kind.as_str();
            let is_delegate_spawn_event =
                matches!(event_kind, "delegate_queued" | "delegate_started");
            if !is_delegate_spawn_event {
                continue;
            }

            let payload = &delegate_event.payload_json;
            let task_identity = task_identity_from_delegate_payload(payload, session_id);
            if let Some(task_identity) = task_identity {
                return task_identity;
            }
        }
    }

    fallback
}

pub(crate) fn unix_ts_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_progress_round_trips_through_event_payload() {
        let record = TaskProgressRecord {
            task_id: "session-1".to_owned(),
            owner_kind: "conversation_turn".to_owned(),
            status: TaskProgressStatus::Active,
            intent_summary: Some("Summarize the status surface".to_owned()),
            verification_state: Some(TaskVerificationState::NotStarted),
            active_handles: vec![TaskActiveHandleRecord {
                handle_kind: "conversation_turn".to_owned(),
                handle_id: "session-1".to_owned(),
                state: "running".to_owned(),
                last_event_at: Some(123),
                stop_condition: "terminal_reply".to_owned(),
            }],
            resume_recipe: Some(TaskResumeRecipeRecord {
                recommended_tool: "session_status".to_owned(),
                session_id: "session-1".to_owned(),
                note: Some("Inspect durable task progress.".to_owned()),
            }),
            updated_at: 123,
        };

        let payload = task_progress_event_payload("unit_test", &record);
        assert_eq!(
            payload["task_progress"]["resume_recipe"]["task_session_id"],
            "session-1"
        );
        assert_eq!(
            payload["task_progress"]["resume_recipe"]["session_id"],
            "session-1"
        );
        let decoded = task_progress_from_event_payload(&payload).expect("decode task progress");

        assert_eq!(decoded, record);
    }

    #[test]
    fn task_progress_parses_legacy_session_aliases() {
        let payload = json!({
            "source": "unit_test",
            "task_progress": {
                "session_id": "legacy-session-1",
                "owner_kind": "conversation_turn",
                "status": "waiting",
                "intent_summary": "Legacy task alias",
                "verification_state": "pending",
                "active_handles": [],
                "resume_recipe": {
                    "recommended_tool": "session_wait",
                    "session_id": "legacy-session-1",
                    "note": "Wait on the legacy session alias."
                },
                "updated_at": 456
            }
        });

        let decoded = task_progress_from_event_payload(&payload).expect("decode legacy payload");

        assert_eq!(decoded.task_id, "legacy-session-1");
        let resume_recipe = decoded.resume_recipe.expect("legacy resume recipe");
        assert_eq!(resume_recipe.task_session_id(), "legacy-session-1");
        assert_eq!(resume_recipe.session_id, "legacy-session-1");
    }

    #[test]
    fn task_progress_parses_canonical_task_session_id_in_resume_recipe() {
        let payload = json!({
            "source": "unit_test",
            "task_progress": {
                "task_id": "task-123",
                "owner_kind": "conversation_turn",
                "status": "active",
                "intent_summary": "Canonical task identity",
                "verification_state": "not_started",
                "active_handles": [],
                "resume_recipe": {
                    "recommended_tool": "task_wait",
                    "task_session_id": "session-123",
                    "note": "Wait on the canonical task session."
                },
                "updated_at": 789
            }
        });

        let decoded =
            task_progress_from_event_payload(&payload).expect("decode canonical task payload");

        assert_eq!(decoded.task_id, "task-123");
        let resume_recipe = decoded.resume_recipe.expect("canonical resume recipe");
        assert_eq!(resume_recipe.task_session_id(), "session-123");
        assert_eq!(resume_recipe.session_id, "session-123");
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn canonical_task_id_resolver_reads_latest_task_progress_event() {
        let temp_dir = std::env::temp_dir().join(format!(
            "loong-task-progress-resolver-{}-{}",
            std::process::id(),
            unix_ts_now()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let sqlite_path = temp_dir.join("memory.sqlite3");
        let config = crate::session::store::SessionStoreConfig {
            sqlite_path: Some(sqlite_path),
            ..crate::session::store::SessionStoreConfig::default()
        };
        let repo = crate::session::repository::SessionRepository::new(&config).expect("repository");
        repo.ensure_session(crate::session::repository::NewSessionRecord {
            session_id: "owner-session".to_owned(),
            kind: crate::session::repository::SessionKind::Root,
            parent_session_id: None,
            label: None,
            state: crate::session::repository::SessionState::Running,
        })
        .expect("create session");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "owner-session".to_owned(),
            event_kind: TASK_PROGRESS_EVENT_KIND.to_owned(),
            actor_session_id: Some("owner-session".to_owned()),
            payload_json: task_progress_event_payload(
                "unit_test",
                &TaskProgressRecord {
                    task_id: "task-123".to_owned(),
                    owner_kind: "conversation_turn".to_owned(),
                    status: TaskProgressStatus::Active,
                    intent_summary: None,
                    verification_state: None,
                    active_handles: Vec::new(),
                    resume_recipe: None,
                    updated_at: 123,
                },
            ),
        })
        .expect("append task progress");

        let task_id = resolve_canonical_task_id_for_session(&repo, "owner-session")
            .expect("canonical task id");
        assert_eq!(task_id, "task-123");
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn task_identity_resolver_falls_back_to_delegate_spawn_payload() {
        let temp_dir = std::env::temp_dir().join(format!(
            "loong-task-identity-resolver-{}-{}",
            std::process::id(),
            unix_ts_now()
        ));
        let _ = std::fs::create_dir_all(&temp_dir);
        let sqlite_path = temp_dir.join("memory.sqlite3");
        let config = crate::session::store::SessionStoreConfig {
            sqlite_path: Some(sqlite_path),
            ..crate::session::store::SessionStoreConfig::default()
        };
        let repo = crate::session::repository::SessionRepository::new(&config).expect("repository");
        repo.ensure_session(crate::session::repository::NewSessionRecord {
            session_id: "child-session".to_owned(),
            kind: crate::session::repository::SessionKind::DelegateChild,
            parent_session_id: Some("root-session".to_owned()),
            label: None,
            state: crate::session::repository::SessionState::Ready,
        })
        .expect("create child session");
        repo.append_event(crate::session::repository::NewSessionEvent {
            session_id: "child-session".to_owned(),
            event_kind: "delegate_queued".to_owned(),
            actor_session_id: Some("root-session".to_owned()),
            payload_json: json!({
                "task": "delegate work",
                "task_scope": {
                    "task_id": "task-root",
                },
                "task_session_id": "child-session",
            }),
        })
        .expect("append delegate payload");

        let task_identity = resolve_task_identity_for_session(&repo, "child-session");

        assert_eq!(task_identity.task_id, "task-root");
        assert_eq!(task_identity.task_session_id, "child-session");
        assert_eq!(
            resolve_canonical_task_id_for_session(&repo, "child-session")
                .expect("canonical task id"),
            "task-root"
        );
    }
}
