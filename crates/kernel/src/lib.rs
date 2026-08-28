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
pub mod resource;

use std::sync::Arc;

use loac::{ActorRef, CallError, prelude::*};

use contracts::capability::{Capabilities, Capability};
use thiserror::Error;

use crate::policy::PolicyContext;
use crate::policy::action::{ActionMeta, Denied, Granted};
use crate::policy::engine::PolicyEngine;
use crate::resource::Resources;

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
    context: PolicyContext,
}

impl<A: ActionMeta> SyncHandler<PolicyEvent<A>> for Kernel {
    fn handle(
        &mut self,
        msg: PolicyEvent<A>,
        _scope: &mut ActorScope<Self>,
    ) -> Result<Granted<A>, Denied> {
        self.policy_engine.grant(msg.context, msg.action)
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
    resources: Arc<Resources>,
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
            resources: Arc::new(Resources::new()),
        }
    }

    /// Returns a facade with the supplied resources.
    ///
    /// Resources are fixed when an agent is assembled, so this consumes the
    /// current facade and returns a new one with the same kernel handle and
    /// capability ceiling.
    #[must_use]
    pub fn with_resources(self, resources: Resources) -> Self {
        Self {
            resources: Arc::new(resources),
            ..self
        }
    }

    #[must_use]
    pub fn resources(&self) -> &Resources {
        self.resources.as_ref()
    }

    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    pub async fn grant<A: ActionMeta>(&self, action: A) -> Result<Granted<A>, GrantSendError> {
        let context = PolicyContext::new(self.capabilities, Arc::clone(&self.resources));
        Ok(self.handle.call(PolicyEvent { action, context }).await??)
    }
}

#[cfg(test)]
mod tests;
