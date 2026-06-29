#![forbid(unsafe_code)]

mod action;
mod artifact;
mod budget;
mod error;
mod event;
mod execution;
mod lifecycle;
mod policy;
mod session;
mod task;
mod workspace;

pub use action::{Action, ActionExecutor};
pub use artifact::{
    ApprovalState, ArtifactDurabilityClass, DiagnosticSeverity, ExecutionArtifact,
    ExecutionArtifactKind, ExecutionArtifacts,
};
pub use budget::{ChildBudgetPolicy, RetentionBudget, SessionBudgetOverlay, TaskBudget};
pub use error::AuthorizationError;
pub use error::CoreModelError;
pub use event::{SessionEvent, TaskEvent};
pub use execution::{CancellationPolicy, Subtask, TaskExecutionMode, Turn, TurnStatus};
pub use lifecycle::TaskLifecycle;
pub use policy::MockPolicyEngine;
pub use policy::{Granted, Policy, PolicyAny, PolicyContext, PolicyContextFactory, PolicyEngine};
pub use session::Session;
pub use task::Task;
pub use workspace::WorkspaceContext;
