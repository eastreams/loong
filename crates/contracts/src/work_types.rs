use serde::{Deserialize, Serialize};
use serde_json::Value;

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkUnitKind {
    Feature,
    Issue,
    Review,
    Ops,
}

impl WorkUnitKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Feature => "feature",
            Self::Issue => "issue",
            Self::Review => "review",
            Self::Ops => "ops",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "feature" => Some(Self::Feature),
            "issue" => Some(Self::Issue),
            "review" => Some(Self::Review),
            "ops" => Some(Self::Ops),
            _ => None,
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkSourceKind {
    Manual,
    Discord,
    Github,
}

impl WorkSourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Discord => "discord",
            Self::Github => "github",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "manual" => Some(Self::Manual),
            "discord" => Some(Self::Discord),
            "github" => Some(Self::Github),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkUnitSourceRef {
    pub source_kind: WorkSourceKind,
    pub project_id: Option<String>,
    pub channel_id: Option<String>,
    pub thread_id: Option<String>,
    pub message_id: Option<String>,
    pub external_ref: Option<String>,
    pub source_url: Option<String>,
}

impl Default for WorkUnitSourceRef {
    fn default() -> Self {
        Self {
            source_kind: WorkSourceKind::Manual,
            project_id: None,
            channel_id: None,
            thread_id: None,
            message_id: None,
            external_ref: None,
            source_url: None,
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkUnitPriority {
    Low,
    Normal,
    High,
    Critical,
}

impl WorkUnitPriority {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "low" => Some(Self::Low),
            "normal" => Some(Self::Normal),
            "high" => Some(Self::High),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkUnitStatus {
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

impl WorkUnitStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Captured => "captured",
            Self::Triaged => "triaged",
            Self::Ready => "ready",
            Self::Leased => "leased",
            Self::Running => "running",
            Self::WaitingExternal => "waiting_external",
            Self::WaitingReview => "waiting_review",
            Self::RetryPending => "retry_pending",
            Self::Completed => "completed",
            Self::FailedTerminal => "failed_terminal",
            Self::Cancelled => "cancelled",
            Self::Archived => "archived",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "captured" => Some(Self::Captured),
            "triaged" => Some(Self::Triaged),
            "ready" => Some(Self::Ready),
            "leased" => Some(Self::Leased),
            "running" => Some(Self::Running),
            "waiting_external" => Some(Self::WaitingExternal),
            "waiting_review" => Some(Self::WaitingReview),
            "retry_pending" => Some(Self::RetryPending),
            "completed" => Some(Self::Completed),
            "failed_terminal" => Some(Self::FailedTerminal),
            "cancelled" => Some(Self::Cancelled),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::FailedTerminal | Self::Cancelled | Self::Archived
        )
    }

    pub const fn is_ready_for_lease(self) -> bool {
        matches!(self, Self::Ready | Self::RetryPending)
    }

    pub const fn is_blocked(self) -> bool {
        matches!(self, Self::WaitingExternal | Self::WaitingReview)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkUnitRetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
}

impl Default for WorkUnitRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff_ms: 1_000,
            max_backoff_ms: 60_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkUnitRecord {
    pub work_unit_id: String,
    pub kind: WorkUnitKind,
    pub title: String,
    pub description: String,
    pub source_ref: WorkUnitSourceRef,
    pub status: WorkUnitStatus,
    pub priority: WorkUnitPriority,
    pub retry_policy: WorkUnitRetryPolicy,
    pub attempt_count: u32,
    pub next_run_at_ms: i64,
    pub last_error: Option<String>,
    pub blocking_reason: Option<String>,
    pub parent_work_unit_id: Option<String>,
    pub result_payload_json: Option<Value>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub archived_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkUnitLeaseRecord {
    pub work_unit_id: String,
    pub owner: String,
    pub lease_version: u64,
    pub acquired_at_ms: i64,
    pub heartbeat_at_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkUnitSnapshot {
    pub work_unit: WorkUnitRecord,
    pub lease: Option<WorkUnitLeaseRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkUnitEventRecord {
    pub sequence_id: i64,
    pub work_unit_id: String,
    pub event_kind: String,
    pub actor: Option<String>,
    pub payload_json: Value,
    pub recorded_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRuntimeHealthSnapshot {
    pub total_count: usize,
    pub ready_count: usize,
    pub leased_count: usize,
    pub running_count: usize,
    pub blocked_count: usize,
    pub retry_pending_count: usize,
    pub terminal_count: usize,
    pub archived_count: usize,
    pub expired_lease_count: usize,
}
