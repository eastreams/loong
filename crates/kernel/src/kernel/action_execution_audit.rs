//! Grant-bound writer for typed action execution evidence.
//!
//! Domain executors own concrete dispatch; Kernel owns clock, event identity,
//! and the audit sink. This method is their narrow crossing: callers provide a
//! real `Granted<A>`, never a caller-selected grant id or persisted envelope.

use loong_contracts::{ActionExecutionEvent, AuditError, AuditEventKind};
use loong_core::policy::{action::ActionMeta, context::ContextFactory, grant::Granted};

use super::Kernel;

impl<C> Kernel<C>
where
    C: ContextFactory,
{
    /// Record one execution event correlated to the supplied action grant.
    pub fn record_granted_action_execution<A>(
        &self,
        grant: &Granted<A>,
        event: ActionExecutionEvent,
    ) -> Result<(), AuditError>
    where
        A: ActionMeta,
    {
        let info = grant.info();
        self.audit_state.record(
            Some(info.subject.actor_id.clone()),
            AuditEventKind::ActionExecution {
                grant_id: grant.id(),
                event,
            },
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
