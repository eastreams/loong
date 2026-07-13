use loong_contracts::{GrantId, PolicyReport};

use crate::policy::action::{Action, ActionMeta};

/// Policy evidence minted with an action grant.
///
/// The report is the same evaluation that authorized the action. Keeping it on
/// the grant prevents audit callers from re-running policy or reconstructing
/// authorization evidence from side channels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionGrantInfo {
    pub report: PolicyReport,
}

/// Authorized action plus grant metadata returned by a policy engine.
#[derive(Debug)]
pub struct ActionGrant<A: ActionMeta> {
    pub id: GrantId,
    pub info: ActionGrantInfo,
    pub granted: Granted<A>,
}

impl<A: ActionMeta> ActionGrant<A> {
    pub(super) fn new(id: GrantId, info: ActionGrantInfo, action: A) -> Self {
        Self {
            id,
            info,
            granted: Granted::new(action),
        }
    }
}

/// Authorization represented as an unforgeable value.
///
/// The constructor is crate-private. Callers can inspect and execute a grant,
/// but only policy helpers inside core can mint one.
#[derive(Debug)]
pub struct Granted<A: ActionMeta>(A);

impl<A: ActionMeta> Granted<A> {
    pub(super) fn new(action: A) -> Self {
        Self(action)
    }

    pub fn into_action(self) -> A {
        self.0
    }

    /// Consume this authorization token through the action's execution hook.
    ///
    /// This is the preferred port from authorization into side-effect
    /// execution. Callers should not invoke `Action::run` directly unless they
    /// are implementing the port itself.
    pub async fn run<Cx>(
        self,
        ctx: &Cx,
    ) -> Result<<A as Action<Cx>>::Output, <A as Action<Cx>>::Error>
    where
        A: Action<Cx>,
        Cx: Sync,
    {
        A::run(self, ctx).await
    }
}

impl<A> AsRef<A> for Granted<A>
where
    A: ActionMeta,
{
    /// Inspect the granted action before the grant is consumed.
    ///
    /// This supports audit and metadata capture at the execution boundary. It
    /// must not grow into a way to clone, mint, or bypass grants.
    fn as_ref(&self) -> &A {
        &self.0
    }
}
