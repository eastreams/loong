use crate::{
    contracts::{CapabilityToken, TaskIntent},
    kernel::{KernelDispatch, LoongClawKernel},
    policy::PolicyEngine,
};
use loongclaw_contracts::{Fault, TaskState};

/// Opt-in wrapper around `execute_task` that enforces FSM transitions.
pub struct TaskSupervisor {
    state: TaskState,
}

impl TaskSupervisor {
    pub fn new(intent: TaskIntent) -> Self {
        Self {
            state: TaskState::Runnable(intent),
        }
    }

    pub fn state(&self) -> &TaskState {
        &self.state
    }

    pub fn is_runnable(&self) -> bool {
        matches!(self.state, TaskState::Runnable(_))
    }

    /// Execute the task through the kernel, tracking state transitions.
    pub async fn execute<P: PolicyEngine>(
        &mut self,
        kernel: &LoongClawKernel<P>,
        pack_id: &str,
        token: &CapabilityToken,
    ) -> Result<KernelDispatch, Fault> {
        // Extract intent from Runnable state
        #[allow(clippy::wildcard_enum_match_arm)]
        let intent = match std::mem::replace(
            &mut self.state,
            TaskState::InSend {
                task_id: String::new(),
            },
        ) {
            TaskState::Runnable(intent) => intent,
            other => {
                self.state = other;
                return Err(Fault::ProtocolViolation {
                    detail: "task is not in Runnable state".to_owned(),
                });
            }
        };

        let task_id = intent.task_id.clone();
        self.state = TaskState::InSend {
            task_id: task_id.clone(),
        };

        // Transition to InReply
        self.state = TaskState::InReply {
            task_id: task_id.clone(),
        };

        // Execute through kernel
        match kernel.execute_task(pack_id, token, intent).await {
            Ok(dispatch) => {
                self.state = TaskState::Completed(dispatch.outcome.clone());
                Ok(dispatch)
            }
            Err(kernel_err) => {
                let fault = Fault::from_kernel_error(kernel_err);
                self.state = TaskState::Faulted(fault.clone());
                Err(fault)
            }
        }
    }

    /// Force state -- for testing only.
    #[cfg(test)]
    pub fn force_state(&mut self, state: TaskState) {
        self.state = state;
    }
}
