use serde::{Deserialize, Serialize};

use crate::contracts::{HarnessOutcome, TaskIntent};
use crate::fault::Fault;

/// State machine for task lifecycle.
///
/// Valid transitions:
/// - Runnable -> InSend
/// - InSend -> InReply
/// - InReply -> Completed | Faulted
/// - Any non-terminal -> Faulted
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TaskState {
    Runnable(TaskIntent),
    InSend { task_id: String },
    InReply { task_id: String },
    Completed(HarnessOutcome),
    Faulted(Fault),
}

impl TaskState {
    pub fn task_id(&self) -> Option<&str> {
        match self {
            Self::Runnable(intent) => Some(&intent.task_id),
            Self::InSend { task_id } | Self::InReply { task_id } => Some(task_id),
            Self::Completed(_) | Self::Faulted(_) => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed(_) | Self::Faulted(_))
    }

    pub fn transition_to_in_send(self) -> Result<Self, String> {
        #[allow(clippy::wildcard_enum_match_arm)]
        match self {
            Self::Runnable(intent) => Ok(Self::InSend {
                task_id: intent.task_id,
            }),
            other => Err(format!(
                "invalid transition: cannot move to InSend from {other:?}"
            )),
        }
    }

    pub fn transition_to_in_reply(self) -> Result<Self, String> {
        #[allow(clippy::wildcard_enum_match_arm)]
        match self {
            Self::InSend { task_id } => Ok(Self::InReply { task_id }),
            other => Err(format!(
                "invalid transition: cannot move to InReply from {other:?}"
            )),
        }
    }

    pub fn transition_to_completed(self, outcome: HarnessOutcome) -> Result<Self, String> {
        #[allow(clippy::wildcard_enum_match_arm)]
        match self {
            Self::InReply { .. } => Ok(Self::Completed(outcome)),
            other => Err(format!(
                "invalid transition: cannot move to Completed from {other:?}"
            )),
        }
    }

    pub fn transition_to_faulted(self, fault: Fault) -> Self {
        if self.is_terminal() {
            self // Already terminal -- ignore
        } else {
            Self::Faulted(fault)
        }
    }
}
