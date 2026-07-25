//! Code that connects domain access to the application.

pub mod access;
pub mod actors;
pub mod policy;

use actix::{WeakAddr, prelude::*};

use thiserror::Error;

use crate::access::fs::FsAccess;
use crate::policy::action::{ActionMeta, Denied, Granted};
use crate::policy::engine::PolicyEngine;
use loong_contracts::capability::Capabilities;

pub struct Kernel {
    policy_engine: PolicyEngine,
}

impl Actor for Kernel {
    type Context = actix::Context<Self>;
}

#[derive(Message)]
#[rtype(result = "Result<Granted<A>, Denied>")]
pub struct PolicyEvent<A: ActionMeta> {
    action: A,
    ctx: Facade,
}

impl<A: ActionMeta> Handler<PolicyEvent<A>> for Kernel {
    type Result = Result<Granted<A>, Denied>;
    fn handle(&mut self, msg: PolicyEvent<A>, _ctx: &mut Self::Context) -> Self::Result {
        self.policy_engine.grant(&msg.ctx, msg.action)
    }
}

#[derive(Clone)]
pub struct Facade {
    handle: WeakAddr<Kernel>,
    capabilities: Capabilities,
}

#[derive(Debug, Error)]
pub enum GrantSendError {
    #[error("denied {0}")]
    Denied(#[from] Denied),
    #[error("kernel unavailable")]
    KernelUnavailable,
    #[error("mailbox error {0}")]
    Mailbox(#[from] MailboxError),
}

impl Facade {
    pub async fn grant<A: ActionMeta>(&self, action: A) -> Result<Granted<A>, GrantSendError> {
        Ok(self
            .handle
            .upgrade()
            .ok_or(GrantSendError::KernelUnavailable)?
            .send(PolicyEvent {
                action,
                ctx: self.clone(),
            })
            .await??)
    }
}

impl Facade {
    #[must_use]
    pub fn fs<'b>(&'b self) -> FsAccess<'b> {
        FsAccess::new(self)
    }
}
