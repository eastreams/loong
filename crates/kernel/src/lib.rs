//! Code that connects domain access to the application.

pub mod access;
pub mod actors;
pub mod policy;

use loac::{ActorOwner, ActorRef, CallError, prelude::*};

use loong_contracts::capability::Capabilities;
use thiserror::Error;

use crate::access::fs::FsAccess;
use crate::policy::PolicyContext;
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
/// [`grant`](Self::grant) method is an assembly boundary, not a tool registry.
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
    pub fn for_owner(owner: &ActorOwner<Kernel>, capabilities: Capabilities) -> Self {
        Self {
            handle: owner.actor_ref(),
            capabilities,
        }
    }

    pub async fn grant<A: ActionMeta>(&self, action: A) -> Result<Granted<A>, GrantSendError> {
        Ok(self
            .handle
            .call(PolicyEvent {
                action,
                context: PolicyContext::new(self.capabilities),
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
    use std::borrow::Cow;

    use loac::Shutdown;
    use loong_contracts::capability::{Capabilities, Capability};
    use serde_json::Value;

    use super::*;

    #[derive(Debug)]
    struct TestAction;

    impl ActionMeta for TestAction {
        fn name(&self) -> Cow<'_, str> {
            Cow::Borrowed("test")
        }

        fn payload(&self) -> Cow<'_, Value> {
            Cow::Owned(Value::Null)
        }

        fn required_capabilities(&self) -> loong_contracts::capability::Capabilities {
            Capability::FsRead.into()
        }
    }

    #[tokio::test]
    async fn explicit_capability_policy_grants_unique_ids() {
        let owner = loac::spawn::<Kernel>(PolicyEngine::allow_capabilities());
        let facade = Facade::for_owner(&owner, Capability::FsRead.into());

        let first = facade.grant(TestAction).await.unwrap();
        let second = facade.grant(TestAction).await.unwrap();
        assert_ne!(first.grant_id(), second.grant_id());
        assert_eq!(first.action().name(), "test");

        owner.shutdown(Shutdown::Stop).await;
    }

    #[tokio::test]
    async fn default_policy_denies_without_a_matching_policy() {
        let owner = loac::spawn::<Kernel>(PolicyEngine::default());
        let facade = Facade::for_owner(&owner, Capabilities::empty());

        let error = facade.grant(TestAction).await.unwrap_err();
        assert!(matches!(error, GrantSendError::Denied(_)));

        owner.shutdown(Shutdown::Stop).await;
    }
}
