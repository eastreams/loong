use std::{borrow::Cow, collections::BTreeSet};

use async_trait::async_trait;
use loong_contracts::{Capability, PolicyGrant, PolicyOutcome};

use crate::{
    action::Action,
    error::AuthorizationError,
    policy::{PolicyContext, PolicyContextFactory},
};

use super::Granted;

/// Typed policy that can evaluate one action kind.
///
/// Kernel/runtime crates own concrete policy registration and ordering. Core
/// keeps only the action-facing contract so access facades can authorize typed
/// effects without depending on a concrete kernel.
#[async_trait]
pub trait Policy<F: PolicyContextFactory, A: Action>: Send + Sync {
    /// Stable policy name used in grant metadata.
    fn name(&self) -> Cow<'static, str>;

    /// Evaluate whether this policy allows, denies, or abstains from `action`.
    async fn grant(&self, ctx: &F::Context<'_>, action: &A) -> PolicyGrant;
}

/// Untyped policy that evaluate all action kinds
#[async_trait]
pub trait PolicyAny<F: PolicyContextFactory>: Send + Sync {
    fn name(&self) -> &'static str;

    async fn grant(&self, ctx: &F::Context<'_>, action: &dyn Action) -> PolicyGrant;
}

/// Authorization engine for typed actions.
///
/// Implementors decide actions and provide grant context. Core turns an allow
/// report into a [`Granted`] token and turns deny reports into structured
/// authorization errors without inventing policy reasons.
pub trait PolicyEngine<F: PolicyContextFactory> {
    /// Evaluate a borrowed action without consuming it.
    fn decide<A: Action>(&self, ctx: &F::Context<'_>, action: &A) -> PolicyOutcome;

    /// Authorize `action` and return a grant that can be consumed by an
    /// executor.
    fn grant<A: Action>(
        &self,
        ctx: &F::Context<'_>,
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

/// The temporary replace for old PolicyEngine.
pub struct MockPolicyEngine;

impl<F: PolicyContextFactory> PolicyEngine<F> for MockPolicyEngine {
    fn decide<A: Action>(&self, _ctx: &F::Context<'_>, _action: &A) -> PolicyOutcome {
        PolicyOutcome::Deny {
            grant_source: None,
            reason: "".into(),
        }
    }
}
