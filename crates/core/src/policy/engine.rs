//! Policy Engine for Loong.
//!
//! Callers use `PolicyEngine`. `PolicyEngineImpl` is trusted final-application
//! code. Evaluation, grant recording, and parent requests are total and
//! fail-closed: the only negative outcome exposed to callers is `Denied`.

use loong_contracts::policy::{PolicyDecisionFinal, PolicyResultFinal};
use uuid::Uuid;

use crate::{
    action::{ActionMeta, Denied, Granted},
    policy::ParentGrantRequester,
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
    ) -> impl Future<Output = Uuid> + Send;
}

mod sealed {
    pub trait Sealed<Cx> {}
}

pub trait PolicyEngine<Cx>: sealed::Sealed<Cx> {
    /// `Err(Denied)` is a final policy refusal, not an operational failure.
    fn grant<A: ActionMeta>(
        &self,
        ctx: &Cx,
        action: A,
    ) -> impl Future<Output = Result<Granted<A>, Denied>> + Send;
}

impl<Cx, P: PolicyEngineImpl<Cx>> sealed::Sealed<Cx> for P where Cx: ParentGrantRequester + Sync {}

impl<Cx, P: PolicyEngineImpl<Cx>> PolicyEngine<Cx> for P
where
    Cx: ParentGrantRequester + Sync,
{
    async fn grant<A: ActionMeta>(&self, ctx: &Cx, action: A) -> Result<Granted<A>, Denied> {
        let result = self.evaluate(ctx, &action).await;

        match result.decision {
            PolicyDecisionFinal::Allow => {
                let grant_id = self.record_action_granted(ctx, &action).await;
                Ok(Granted::new(grant_id, action))
            }
            PolicyDecisionFinal::Deny => Err(Denied {
                reason: result.reason,
            }),
            PolicyDecisionFinal::RequiresApproval => ctx.grant(action).await,
        }
    }
}

#[cfg(test)]
mod tests;
