use std::collections::BTreeSet;

use async_trait::async_trait;
use loong_contracts::{Capability, GrantId, PolicyOutcome, PolicyReport};

use crate::{
    error::PolicyGrantError,
    policy::{
        action::Action,
        context::{ContextFactory, PolicyContext},
        grant::{ActionGrant, ActionGrantInfo},
    },
};

/// Authorization engine for typed actions.
///
/// Implementors decide actions and provide grant context. Core turns an allow
/// report into a [`Granted`] token and turns deny reports into structured
/// authorization errors without inventing policy reasons.
#[async_trait]
pub trait PolicyEngine<C: ContextFactory>: Sync {
    /// Evaluate a borrowed action without consuming it.
    async fn decide<A: Action + 'static>(&self, ctx: &C::Cx<'_>, action: &A) -> PolicyReport;

    /// Allocate the next grant id for an allowed action.
    async fn next_grant_id(&self) -> GrantId;

    /// Authorize `action` and return grant metadata plus a token that can be
    /// consumed by access code.
    ///
    /// The capability gate is deliberately built in here so policies only run
    /// after the caller already has every capability declared by the action.
    async fn grant<A: Action>(
        &self,
        ctx: &C::Cx<'_>,
        action: A,
    ) -> Result<ActionGrant<A>, PolicyGrantError>
    where
        A: 'static,
    {
        let granted_capabilities = ctx.capabilities();
        for capability in action.required_capabilities() {
            if !granted_capabilities.contains(&capability) {
                return Err(PolicyGrantError::MissingCapability { capability });
            }
        }

        let report = self.decide(ctx, &action).await;
        match report.outcome.clone() {
            PolicyOutcome::Allow {
                source: _,
                reason: _,
            } => Ok(ActionGrant::new(
                self.next_grant_id().await,
                ActionGrantInfo,
                action,
            )),
            PolicyOutcome::Deny {
                grant_source: _,
                reason,
            } => Err(PolicyGrantError::Denied { report, reason }),
        }
    }
}

impl PolicyContext for () {
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::new()
    }
}
