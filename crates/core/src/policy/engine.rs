//! Policy Engine for Loong.
//!
//! Callers use `PolicyEngine`. `PolicyEngineImpl` is trusted code. An engine may
//! return a grant only after policy allows the action and the grant is recorded.
//! Denial and approval are decisions; evaluation and recording failures are
//! errors.

use loong_contracts::policy::PolicyResultFinal;
use uuid::Uuid;

use crate::{
    action::{ActionGrant, ActionMeta},
    policy::GrantRequester,
};

/// This trait should be restrained in implementation.
/// The implementor of this trait should stay in trusted domain.
///
/// Mostly, this should only be implemented in Kernel.
pub trait PolicyEngineImpl<Cx>
where
    Cx: GrantRequester + Sync,
    Self: Sync,
{
    type Error: std::error::Error + Send + Sync + 'static;

    fn evaluate<A: ActionMeta>(
        &self,
        ctx: &Cx,
        action: &A,
    ) -> impl Future<Output = Result<PolicyResultFinal, Self::Error>> + Send;

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
    type Error: std::error::Error + Send + Sync + 'static;

    // TODO: proper error type
    fn grant<A: ActionMeta>(
        &self,
        ctx: &Cx,
        action: A,
    ) -> impl Future<Output = Result<ActionGrant<A>, Self::Error>> + Send;
}

impl<Cx, P: PolicyEngineImpl<Cx>> sealed::Sealed<Cx> for P
where
    Cx: GrantRequester + Sync,
    P: Sync,
{
}

impl<Cx, P: PolicyEngineImpl<Cx>> PolicyEngine<Cx> for P
where
    Cx: GrantRequester + Sync,
    P: Sync,
{
    type Error = P::Error;

    async fn grant<A: ActionMeta>(
        &self,
        ctx: &Cx,
        action: A,
    ) -> Result<ActionGrant<A>, Self::Error> {
        let result = self.evaluate(ctx, &action).await?;
        todo!()
        // match result.decision {
        //     PolicyDecisionFinal::Allow => Ok(ActionGrant::Allow),
        //     PolicyDecisionFinal::Deny => Ok(ActionGrant::Deny),
        //     PolicyDecisionFinal::RequiresApproval => ctx.
        // }
    }
}
