use crate::{
    contracts::{CapabilityToken, TaskIntent},
    kernel::{KernelDispatch, LoongKernel},
    policy::PolicyEngine,
};
use loong_contracts::{Fault, TaskState};

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

    /// Clone the current state before attempting a guarded transition so
    /// rejected transitions leave the supervisor unchanged.
    fn take_state(&self) -> TaskState {
        self.state.clone()
    }

    /// Execute the task through the kernel, tracking state transitions.
    pub async fn execute<P: PolicyEngine>(
        &mut self,
        kernel: &LoongKernel<P>,
        pack_id: &str,
        token: &CapabilityToken,
    ) -> Result<KernelDispatch, Fault> {
        // Clone the intent before transitioning, since we need it for the
        // kernel call and transition_to_in_send consumes it.
        let intent = match &self.state {
            TaskState::Runnable(intent) => intent.clone(),
            TaskState::InSend { .. }
            | TaskState::InReply { .. }
            | TaskState::Completed(_)
            | TaskState::Faulted(_) => {
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::TaskSupervisor;
    use crate::contracts::{Capability, TaskIntent};
    use loong_contracts::{Fault, TaskState};

    fn sample_intent() -> TaskIntent {
        TaskIntent {
            task_id: "supervised-guard".to_owned(),
            objective: "exercise guarded transition".to_owned(),
            required_capabilities: BTreeSet::from([Capability::InvokeTool]),
            payload: json!({}),
        }
    }

    #[test]
    fn take_state_does_not_poison_supervisor_when_transition_is_rejected() {
        let supervisor = TaskSupervisor::new(sample_intent());

        let error = supervisor
            .take_state()
            .transition_to_in_reply()
            .expect_err("Runnable cannot transition directly to InReply");

        assert!(error.contains("cannot move to InReply"));
        assert!(matches!(
            supervisor.state(),
            TaskState::Runnable(intent) if intent.task_id == "supervised-guard"
        ));
        assert!(!matches!(
            supervisor.state(),
            TaskState::Faulted(Fault::ProtocolViolation { .. })
        ));
    }
}
