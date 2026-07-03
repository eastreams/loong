use std::{borrow::Cow, path::PathBuf};

use loong_contracts::{Capability, PolicyEntry};
use thiserror::Error;

use crate::TaskLifecycle;

#[derive(Debug, Error)]
pub enum AuthorizationError {
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
