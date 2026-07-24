//! Code that connects domain access to the application.
use actix::prelude::*;

use async_trait::async_trait;
use uuid::Uuid;

use loong_access::fs::FsAccess;
use loong_contracts::{
    capability::Capabilities,
    policy::{PolicyDecisionFinal, PolicyResultFinal},
};
use loong_core::{
    action::{ActionMeta, Denied, Granted},
    policy::{ParentGrantRequester, PolicyEngineImpl},
};

pub struct Kernel {}

impl Actor for Kernel {
    type Context = actix::Context<Self>;
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Shutdown {}

impl Handler<Shutdown> for Kernel {
    type Result = ResponseActFuture<Self, ()>;
    fn handle(&mut self, _msg: Shutdown, _ctx: &mut Self::Context) -> Self::Result {
        tokio::time::sleep(std::time::Duration::from_secs(1))
            .into_actor(self)
            .map(|(), _actor, ctx| ctx.stop())
            .boxed_local()
    }
}

pub struct Handle {
    kernel: Addr<Kernel>,
}

impl<Cx: Sync> PolicyEngineImpl<Cx> for Handle {
    async fn evaluate<A: loong_core::action::ActionMeta>(
        &self,
        _ctx: &Cx,
        _action: &A,
    ) -> PolicyResultFinal {
        PolicyResultFinal {
            decision: PolicyDecisionFinal::Deny,
            reason: None,
        }
    }

    async fn record_action_granted<A: loong_core::action::ActionMeta>(
        &self,
        _ctx: &Cx,
        _action: &A,
    ) -> Uuid {
        Uuid::nil() // TODO: real id generation
    }
}

impl Handle {}

#[derive(Clone)]
pub struct Facade<'a> {
    handle: &'a Handle,
    capabilities: Capabilities,
}

#[async_trait]
impl ParentGrantRequester for Facade<'_> {
    async fn grant<A: ActionMeta>(&self, _action: A) -> Result<Granted<A>, Denied> {
        Err(Denied { reason: None })
    }
}

impl<'a> Facade<'a> {
    #[must_use]
    pub fn fs<'b>(&'b self) -> FsAccess<'b, Self, Handle> {
        FsAccess::new(self.handle, self)
    }
}
