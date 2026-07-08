use loong_contracts::GrantId;

use crate::policy::action::{Action, ActionMeta};

/// TODO: Placeholder for structured action grant metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionGrantInfo;

/// Authorized action plus grant metadata returned by a policy engine.
#[derive(Debug)]
pub struct ActionGrant<A: ActionMeta> {
    pub id: GrantId,
    /// TODO: Replace this placeholder with structured grant/audit metadata.
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
