use std::path::PathBuf;

use thiserror::Error;

use crate::TaskLifecycle;

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
