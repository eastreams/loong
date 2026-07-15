use loong_contracts::{AuthorizationActionSnapshot, AuthorizationSubject, GrantId, PolicyReport};

use crate::policy::action::{Action, ActionMeta};

/// Immutable authorization facts minted with an action grant.
///
/// The report, subject, and action snapshot come from the same authorization
/// attempt. Keeping them bound to the execution proof prevents later callers
/// from re-running policy or reconstructing correlation from side channels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionGrantInfo {
    pub report: PolicyReport,
    pub subject: AuthorizationSubject,
    pub action: AuthorizationActionSnapshot,
}

/// Authorized action plus grant metadata returned by a policy engine.
///
/// The fields stay private so callers cannot attach one real [`Granted`] value
/// to a different id or authorization snapshot. `into_granted` is the only
/// transition into the execution proof.
#[derive(Debug)]
pub struct ActionGrant<A: ActionMeta> {
    granted: Granted<A>,
}

impl<A: ActionMeta> ActionGrant<A> {
    pub(super) fn new(id: GrantId, info: ActionGrantInfo, action: A) -> Self {
        Self {
            granted: Granted::new(id, info, action),
        }
    }

    #[must_use]
    pub fn id(&self) -> GrantId {
        self.granted.id()
    }

    #[must_use]
    pub fn info(&self) -> &ActionGrantInfo {
        self.granted.info()
    }

    #[must_use]
    pub fn into_granted(self) -> Granted<A> {
        self.granted
    }
}

/// Authorization represented as an unforgeable value.
///
/// The constructor is crate-private. Callers can inspect and execute a grant,
/// but only policy helpers inside core can mint one.
#[derive(Debug)]
pub struct Granted<A: ActionMeta> {
    id: GrantId,
    info: ActionGrantInfo,
    action: A,
}

impl<A: ActionMeta> Granted<A> {
    fn new(id: GrantId, info: ActionGrantInfo, action: A) -> Self {
        Self { id, info, action }
    }

    /// Correlation identity minted with this exact execution proof.
    #[must_use]
    pub fn id(&self) -> GrantId {
        self.id
    }

    /// Immutable authorization facts minted with this exact execution proof.
    #[must_use]
    pub fn info(&self) -> &ActionGrantInfo {
        &self.info
    }

    pub fn into_action(self) -> A {
        self.action
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
        &self.action
    }
}
