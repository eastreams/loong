//! Code that connects domain access to the application.

pub mod access;
pub mod actors;
pub mod policy;

use loac::{ActorRef, CallError, prelude::*};

use thiserror::Error;

use crate::access::fs::FsAccess;
use crate::policy::action::{ActionMeta, Denied, Granted};
use crate::policy::engine::PolicyEngine;
use loong_contracts::capability::Capabilities;

pub struct Kernel {
    policy_engine: PolicyEngine,
}

#[actor(mailbox)]
impl Actor for Kernel {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self {
            policy_engine: Default::default(),
        }
    }
}

#[derive(Message)]
#[message(reply = Result<Granted<A>, Denied>)]
pub struct PolicyEvent<A: ActionMeta> {
    action: A,
    ctx: Facade,
}

impl<A: ActionMeta> SyncHandler<PolicyEvent<A>> for Kernel {
    fn handle(
        &mut self,
        msg: PolicyEvent<A>,
        _scope: &mut ActorScope<Self>,
    ) -> Result<Granted<A>, Denied> {
        self.policy_engine.grant(&msg.ctx, msg.action)
    }
}

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
    pub async fn grant<A: ActionMeta>(&self, action: A) -> Result<Granted<A>, GrantSendError> {
        Ok(self
            .handle
            .call(PolicyEvent {
                action,
                ctx: self.clone(),
            })
            .await??)
    }
}

impl Facade {
    #[must_use]
    pub fn fs(&self) -> FsAccess<'_> {
        FsAccess::new(self)
    }
}

#[cfg(test)]
mod tests {
    use loong_contracts::capability::Capability;

    use super::*;

    #[tokio::test]
    async fn test() {
        let kernel = loac::spawn::<Kernel>(());
        let ctx = Facade {
            handle: kernel.actor_ref(),
            capabilities: Capabilities::singleton(Capability::FsRead),
        };
        println!("{}", ctx.fs().read("test.txt").await.unwrap_err());
    }
}
