use std::{any::Any, borrow::Cow};

use async_trait::async_trait;
use loong_contracts::capability::Capabilities;
use serde_json::Value;
use uuid::Uuid;

pub trait ActionMeta: Any + Send + Sync {
    fn name(&self) -> Cow<'_, str>;
    fn payload(&self) -> Cow<'_, Value>;
    fn required_capabilities(&self) -> Capabilities;
}

/// Executable action implementation for one concrete invocation context.
///
/// Implement this only at the domain side-effect boundary. `run` consumes a
/// [`Granted<Self>`], so raw action values cannot execute side effects.
#[async_trait]
pub trait Action<Cx: ?Sized>: ActionMeta + Sized
where
    Cx: Sync,
{
    type Output;
    type Error;

    async fn run(granted: Granted<Self>, ctx: &Cx) -> Result<Self::Output, Self::Error>;
}

/// Authorization represented as an unforgeable value.
///
/// The constructor is crate-private. Callers can inspect and execute a grant,
/// but only policy helpers inside core can mint one.
#[derive(Debug)]
pub struct Granted<A: ActionMeta>(A);

impl<A: ActionMeta> Granted<A> {
    pub(crate) fn new(action: A) -> Self {
        Self(action)
    }

    pub fn into_action(self) -> A {
        self.0
    }

    /// Consume this authorization proof through the action's execution hook.
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
        Cx: ?Sized + Sync,
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

/// Identifier assigned to an action grant by the concrete policy engine.
///
/// This is correlation metadata; [`Granted<A>`] remains the execution proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GrantId(Uuid);

#[derive(Debug)]
pub struct ActionGrant<A: ActionMeta> {
    grant_id: GrantId,
    granted: Granted<A>,
}

impl<A: ActionMeta> ActionGrant<A> {
    pub(crate) fn new(grant_id: GrantId, granted: Granted<A>) -> Self {
        Self { grant_id, granted }
    }

    pub fn into_parts(self) -> (GrantId, Granted<A>) {
        (self.grant_id, self.granted)
    }

    pub fn grant_id(&self) -> GrantId {
        self.grant_id
    }

    pub fn granted(&self) -> &Granted<A> {
        &self.granted
    }
}

pub struct ToBeApproved<A: ActionMeta>(A);

/// The action to approve one action.
/// This action has a special privilege to grant other actions.
///
/// When action A is given `RequireApproval` in the policy engine
/// of one node, this action should be evaluated by its parent.
pub struct ApprovalAction /*<A: ActionMeta>*/ {
    // TODO
}
