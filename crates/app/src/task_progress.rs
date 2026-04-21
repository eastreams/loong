use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct TaskResumeRecipeRecord {
    pub recommended_tool: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct TaskProgressRecord {
    pub task_id: String,
    pub owner_kind: String,
    pub status: TaskProgressStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_state: Option<TaskVerificationState>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub active_handles: Vec<TaskActiveHandleRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
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
        let decoded = task_progress_from_event_payload(&payload).expect("decode task progress");

        assert_eq!(decoded, record);
    }
}
