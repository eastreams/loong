//! Connects domain access to the application.
//!
//! [`Kernel`] is the policy actor: it evaluates [`PolicyEvent`] messages
//! against a [`PolicyEngine`](policy::engine::PolicyEngine) and returns either
//! a [`Granted`] proof or a [`Denied`] refusal. [`Facade`] is the trusted
//! handle that application and access code use to reach that actor, and the
//! [`access`] module exposes narrow domain-operation APIs on top of the
//! facade.

pub mod access;
pub mod actors;
pub mod policy;

use loac::{ActorRef, CallError, prelude::*};

use contracts::capability::{Capabilities, Capability};
use thiserror::Error;

use crate::policy::action::{ActionMeta, Denied, Granted};
use crate::policy::engine::PolicyEngine;

pub struct Kernel {
    policy_engine: PolicyEngine,
}

#[actor(mailbox)]
impl Actor for Kernel {
    type SpawnArgs = PolicyEngine;

    async fn init(policy_engine: Self::SpawnArgs, _: &mut ActorScope<'_, Self>) -> Self {
        Self { policy_engine }
    }
}

#[derive(Message)]
#[message(reply = Result<Granted<A>, Denied>)]
pub struct PolicyEvent<A: ActionMeta> {
    action: A,
    capabilities: Capabilities,
}

impl<A: ActionMeta> SyncHandler<PolicyEvent<A>> for Kernel {
    fn handle(
        &mut self,
        msg: PolicyEvent<A>,
        _scope: &mut ActorScope<Self>,
    ) -> Result<Granted<A>, Denied> {
        self.policy_engine.grant(msg.capabilities, msg.action)
    }
}

/// Trusted gateway from application code into the kernel policy actor.
///
/// Keep this value out of untrusted model output and tool inputs. The generic
/// [`grant`](Self::grant) method is an assembly boundary: only assembly code
/// should call it with concrete actions, while caller-facing APIs such as
/// [`FsAccess`](crate::access::fs::FsAccess) narrow it into fixed operations.
#[derive(Clone)]
pub struct Facade {
    handle: ActorRef<Kernel>,
    capabilities: Capabilities,
}

#[derive(Debug, Error)]
pub enum GrantSendError {
    #[error(transparent)]
    Denied(#[from] Denied),
    #[error("call error: {0}")]
    CallError(#[from] CallError),
}

impl Facade {
    #[must_use]
    pub fn new(
        handle: ActorRef<Kernel>,
        capabilities: impl IntoIterator<Item = Capability>,
    ) -> Self {
        Self {
            handle,
            capabilities: capabilities.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Returns a facade whose capability ceiling is the intersection of the
    /// current ceiling and `capabilities`.
    ///
    /// This can only reduce the ceiling, so a caller cannot use it to grant
    /// itself new capabilities. Workflow assembly uses it to derive empty
    /// sandbox facades for planner and worker agents from a capability-owning
    /// root facade.
    #[must_use]
    pub fn narrow(self, capabilities: Capabilities) -> Self {
        Self {
            capabilities: self.capabilities.intersection(capabilities),
            ..self
        }
    }

    pub async fn grant<A: ActionMeta>(&self, action: A) -> Result<Granted<A>, GrantSendError> {
        Ok(self
            .handle
            .call(PolicyEvent {
                action,
                capabilities: self.capabilities,
            })
            .await??)
    }
}

#[cfg(test)]
mod tests;
