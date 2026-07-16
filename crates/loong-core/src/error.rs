use std::{borrow::Cow, error::Error, fmt, path::PathBuf};

use loong_contracts::{AuthorizationEvidence, Capability, PolicyEntry, PolicyReport};
use thiserror::Error;

use crate::TaskLifecycle;

/// Correlation identity whose allocation prevented authorization from continuing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationIdentityKind {
    Attempt,
    Grant,
}

impl fmt::Display for AuthorizationIdentityKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Attempt => "attempt",
            Self::Grant => "grant",
        })
    }
}

#[derive(Debug, Error)]
pub enum PolicyGrantError {
    /// The backend rejected authorization evidence.
    ///
    /// `evidence` is the exact envelope core intended to write; it is diagnostic
    /// context and does not claim that the sink durably accepted it.
    #[error("authorization audit failed: {source}")]
    Audit {
        evidence: Box<AuthorizationEvidence>,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    /// Identity allocation failed, but the backend accepted evidence of that failure.
    #[error("authorization {identity} identity allocation failed: {source}")]
    IdentityAllocation {
        identity: AuthorizationIdentityKind,
        evidence: Box<AuthorizationEvidence>,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    /// Identity allocation and the attempt to record that failure both failed.
    ///
    /// Both concrete sources remain available for direct pattern matching. The
    /// allocation error is the primary `Error::source`; `audit_source` records
    /// why the diagnostic evidence could not be accepted.
    #[error(
        "authorization {identity} identity allocation failed: {allocation_source}; recording that failure also failed: {audit_source}"
    )]
    IdentityAllocationAndAudit {
        identity: AuthorizationIdentityKind,
        evidence: Box<AuthorizationEvidence>,
        #[source]
        allocation_source: Box<dyn Error + Send + Sync>,
        audit_source: Box<dyn Error + Send + Sync>,
    },
    #[error("missing capability: {capability:?}")]
    MissingCapability { capability: Capability },
    #[error("authorization denied: {reason}")]
    Denied {
        /// Box the full report so authorization errors stay cheap to propagate.
        report: Box<PolicyReport>,
        reason: Cow<'static, str>,
    },
    #[error("permission denied: {reason}")]
    PermissionDenied {
        report: Box<PolicyReport>,
        reason: Cow<'static, str>,
    },
    #[error("permission request failed: {source}")]
    PermissionRequest {
        report: Box<PolicyReport>,
        #[source]
        source: PermissionRequestError,
    },
}

/// Failure to obtain a decision from a permission authority.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PermissionRequestError {
    #[error("permission surface unavailable: {reason}")]
    Unavailable { reason: Cow<'static, str> },
    #[error("permission transport failed: {reason}")]
    Failed { reason: Cow<'static, str> },
    #[error("user permission cannot escalate to a higher authority")]
    EscalationUnavailable,
}

#[derive(Debug, Error)]
pub enum AuthorizationError {
    #[error("{0}")]
    PolicyGrant(
        #[from]
        #[source]
        PolicyGrantError,
    ),
    #[error("missing capability: {0:?}")]
    MissingCapability(Capability),
    #[error("authorization denied: {grant_source:?} {reason:?}")]
    Denied {
        grant_source: Option<PolicyEntry>,
        reason: Cow<'static, str>,
    },
    #[error("IO error: {0:?}")]
    Io(std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CoreModelError {
    #[error("invalid task lifecycle transition from {from:?} to {to:?}")]
    InvalidLifecycleTransition {
        from: TaskLifecycle,
        to: TaskLifecycle,
    },
    #[error("session {session_id} already contains task {task_id}")]
    DuplicateTask { session_id: String, task_id: String },
    #[error("session {session_id} does not contain task {task_id}")]
    UnknownTask { session_id: String, task_id: String },
    #[error("session workspace repo root mismatch: expected {expected:?}, got {actual:?}")]
    RepositoryMismatch { expected: PathBuf, actual: PathBuf },
    #[error("task {task_id} already has an active turn")]
    ActiveTurnAlreadyOpen { task_id: String },
    #[error("session {session_id} exceeded max_parallel_tasks limit {limit}")]
    SessionParallelTaskBudgetExceeded { session_id: String, limit: usize },
    #[error("session {session_id} exceeded max_parallel_child_tasks limit {limit}")]
    SessionParallelChildTaskBudgetExceeded { session_id: String, limit: usize },
    #[error("task {task_id} exceeded max_child_tasks limit {limit}")]
    TaskChildBudgetExceeded { task_id: String, limit: usize },
}

#[derive(Debug)]
pub enum ExecutionError {
    Authorization(AuthorizationError),
    Capability(CapabilityError),
    Other(Cow<'static, str>),
}

#[derive(Debug)]
pub enum CapabilityError {
    GrantMismatch,
    Denied,
    Io(std::io::Error),
}

#[derive(Debug)]
pub enum ToolError {
    UnknownTool(String),
    DuplicateTool(String),
    InvalidSpec,
    InvalidInput(InputError),
    Authorization(AuthorizationError),
    Execution(ExecutionError),
}

#[derive(Debug)]
pub enum InputError {
    MissingField(&'static str),
    InvalidField(&'static str),
}
