//! Policy Engine for Loong.
//!
//! Callers use `PolicyEngine`. `PolicyEngineImpl` is trusted final-application
//! code. Evaluation is total and fail-closed. Recording and parent-request
//! failures remain errors because callers may need to diagnose or retry them.

use loong_contracts::policy::{PolicyDecisionFinal, PolicyResultFinal};
use uuid::Uuid;

use crate::{
    action::{ActionMeta, Granted},
    policy::{GrantOutcome, ParentGrantRequester},
};

/// Trusted policy evaluation and grant-recording hooks.
///
/// Stable Rust cannot restrict this public trait to one downstream crate.
/// Implement it only in the final application composition boundary.
pub trait PolicyEngineImpl<Cx>
where
    Cx: Sync,
    Self: Sync,
{
    /// Failure to finish issuing a grant after evaluation has completed.
    ///
    /// Implementations return this when recording fails. The composed engine
    /// uses the same error type for parent-request failures. Policy denial is a
    /// successful [`GrantOutcome::Denied`], not an error.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Evaluate the action inside the final application's trusted boundary.
    ///
    /// Capability ceilings are policy input and are not rechecked by the
    /// blanket `grant` implementation. Never return `Allow` when the current
    /// context lacks authority; use `RequiresApproval` when a parent should
    /// evaluate the same action.
    fn evaluate<A: ActionMeta>(
        &self,
        ctx: &Cx,
        action: &A,
    ) -> impl Future<Output = PolicyResultFinal> + Send;

    fn record_action_granted<A: ActionMeta>(
        &self,
        ctx: &Cx,
        action: &A,
    ) -> impl Future<Output = Result<Uuid, Self::Error>> + Send;
}

mod sealed {
    pub trait Sealed<Cx> {}
}

pub trait PolicyEngine<Cx>: sealed::Sealed<Cx> {
    /// Failure to record a local grant or complete a parent grant request.
    ///
    /// Policy denial is returned as [`GrantOutcome::Denied`].
    type Error: std::error::Error + Send + Sync + 'static;

    fn grant<A: ActionMeta>(
        &self,
        ctx: &Cx,
        action: A,
    ) -> impl Future<Output = Result<GrantOutcome<A>, Self::Error>> + Send;
}

impl<Cx, P: PolicyEngineImpl<Cx>> sealed::Sealed<Cx> for P where
    Cx: ParentGrantRequester<Error = P::Error> + Sync
{
}

impl<Cx, P: PolicyEngineImpl<Cx>> PolicyEngine<Cx> for P
where
    Cx: ParentGrantRequester<Error = P::Error> + Sync,
{
    type Error = P::Error;

    async fn grant<A: ActionMeta>(
        &self,
        ctx: &Cx,
        action: A,
    ) -> Result<GrantOutcome<A>, Self::Error> {
        let result = self.evaluate(ctx, &action).await;

        match result.decision {
            PolicyDecisionFinal::Allow => {
                let grant_id = self.record_action_granted(ctx, &action).await?;
                Ok(GrantOutcome::Granted(Granted::new(grant_id, action)))
            }
            PolicyDecisionFinal::Deny => Ok(GrantOutcome::Denied {
                reason: result.reason,
            }),
            PolicyDecisionFinal::RequiresApproval => ctx.grant(action).await,
        }
    }
}

#[cfg(test)]
mod tests;
