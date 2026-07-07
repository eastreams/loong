use std::collections::BTreeSet;

use async_trait::async_trait;
use loong_contracts::{Capability, GrantId, PolicyOutcome};

use crate::{
    error::AuthorizationError,
    policy::{
        action::Action,
        context::PolicyContext,
        grant::{ActionGrant, ActionGrantInfo},
    },
};

/// Authorization engine for typed actions.
///
/// Implementors decide actions and provide grant context. Core turns an allow
/// report into a [`Granted`] token and turns deny reports into structured
/// authorization errors without inventing policy reasons.
#[async_trait]
pub trait PolicyEngine: Sync {
    /// contains PolicyContext
    type Cx<'a>: PolicyContext;

    /// Evaluate a borrowed action without consuming it.
    async fn decide<A: Action>(&self, ctx: &Self::Cx<'_>, action: &A) -> PolicyOutcome;

    /// Allocate the next grant id for an allowed action.
    async fn next_grant_id(&self) -> GrantId;

    /// Authorize `action` and return grant metadata plus a token that can be
    /// consumed by an executor.
    async fn grant<A: Action>(
        &self,
        ctx: &Self::Cx<'_>,
        action: A,
    ) -> Result<ActionGrant<A>, AuthorizationError> {
        let outcome = self.decide(ctx, &action).await;
        match outcome {
            PolicyOutcome::Allow {
                source: _,
                reason: _,
            } => Ok(ActionGrant::new(
                self.next_grant_id().await,
                ActionGrantInfo,
                action,
            )),
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
