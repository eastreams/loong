use std::collections::BTreeSet;

use async_trait::async_trait;
use loong_contracts::{Capability, PolicyOutcome};

use crate::{
    action::{Action, ActionExecutor},
    error::{AuthorizationError, ExecutionError},
    policy::{
        context::{PolicyContext, PolicyContextFactory},
        grant::Granted,
    },
};

/// Authorization engine for typed actions.
///
/// Implementors decide actions and provide grant context. Core turns an allow
/// report into a [`Granted`] token and turns deny reports into structured
/// authorization errors without inventing policy reasons.
pub trait PolicyEngine {
    /// contains PolicyContext
    type Factory: PolicyContextFactory;

    /// Evaluate a borrowed action without consuming it.
    fn decide<A: Action>(
        &self,
        ctx: &<Self::Factory as PolicyContextFactory>::Context<'_>,
        action: &A,
    ) -> PolicyOutcome;

    /// Authorize `action` and return a grant that can be consumed by an
    /// executor.
    fn grant<A: Action>(
        &self,
        ctx: &<Self::Factory as PolicyContextFactory>::Context<'_>,
        action: A,
    ) -> Result<Granted<A>, AuthorizationError> {
        let outcome = self.decide(ctx, &action);
        match outcome {
            PolicyOutcome::Allow {
                source: _,
                reason: _,
            } => {
                todo!()
            }
            PolicyOutcome::Deny {
                grant_source,
                reason,
            } => Err(AuthorizationError::Denied {
                grant_source,
                reason,
            }),
        }
    }
}

impl PolicyContext for () {
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::new()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MockPolicyContextFactory;

impl PolicyContextFactory for MockPolicyContextFactory {
    type Context<'a> = ();
}

/// The temporary replace for old PolicyEngine.
#[derive(Debug, Default, Clone, Copy)]
pub struct MockPolicyEngine;

impl PolicyEngine for MockPolicyEngine {
    type Factory = MockPolicyContextFactory;

    fn decide<A: Action>(&self, _ctx: &(), _action: &A) -> PolicyOutcome {
        PolicyOutcome::Deny {
            grant_source: None,
            reason: "".into(),
        }
    }
}

#[async_trait]
pub trait HasPolicyEngine: Sync {
    type PolicyEngine<'a>: PolicyEngine
    where
        Self: 'a;

    fn policy_engine(&self) -> &Self::PolicyEngine<'_>;

    async fn execute_granted<A, E>(
        &self,
        granted: Granted<A>,
        executor: &E,
    ) -> Result<E::Output, ExecutionError>
    where
        Self: Sized,
        A: Action,
        E: ActionExecutor<A> + ?Sized,
    {
        granted.execute_with(executor).await
    }
}
