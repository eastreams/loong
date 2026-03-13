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

    /// Swap the current state out, leaving a placeholder Faulted value.
    ///
    /// The caller MUST assign a new valid state back to `self.state`
    /// before returning.
    fn take_state(&mut self) -> TaskState {
        std::mem::replace(
            &mut self.state,
            TaskState::Faulted(Fault::ProtocolViolation {
                detail: "state taken for transition".to_owned(),
            }),
        )
    }

    /// Execute the task through the kernel, tracking state transitions.
    pub async fn execute<P: PolicyEngine>(
        &mut self,
        kernel: &LoongClawKernel<P>,
        pack_id: &str,
        token: &CapabilityToken,
    ) -> Result<KernelDispatch, Fault> {
        // Clone the intent before transitioning, since we need it for the
        // kernel call and transition_to_in_send consumes it.
        let intent = match &self.state {
            TaskState::Runnable(intent) => intent.clone(),
            _ => {
                return Err(Fault::ProtocolViolation {
                    detail: "task is not in Runnable state".to_owned(),
                });
            }
        };

        // Runnable -> InSend (guarded transition)
        let taken = self.take_state();
        self.state = taken
            .transition_to_in_send()
            .map_err(|detail| Fault::ProtocolViolation { detail })?;

        // InSend -> InReply (guarded transition)
        let taken = self.take_state();
        self.state = taken
            .transition_to_in_reply()
            .map_err(|detail| Fault::ProtocolViolation { detail })?;

        // Execute through kernel
        match kernel.execute_task(pack_id, token, intent).await {
            Ok(dispatch) => {
                // InReply -> Completed (guarded transition)
                let taken = self.take_state();
                self.state = taken
                    .transition_to_completed(dispatch.outcome.clone())
                    .map_err(|detail| Fault::ProtocolViolation { detail })?;
                Ok(dispatch)
            }
            Err(kernel_err) => {
                let fault = Fault::from_kernel_error(kernel_err);
                // Any non-terminal -> Faulted
                let taken = self.take_state();
                self.state = taken.transition_to_faulted(fault.clone());
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
