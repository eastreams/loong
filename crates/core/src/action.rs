//! Actions and their grants.
//!
//! `ActionMeta` gives policy an action's name, payload, and capabilities.
//! `Action<Cx>` lets an action execute itself when that model fits.
//!
//! Code that performs a Loong side effect must require `Granted<A>`. Its
//! constructor is private to `loong-core`. `GrantId` only identifies the grant
//! record; it does not grant permission. An action does not choose where it
//! runs.

use core::any::Any;

use alloc::{borrow::Cow, string::String};
use loong_contracts::capability::Capabilities;
use serde_json::Value;
use thiserror::Error;
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
pub trait Action<Cx: ?Sized>: ActionMeta + Sized
where
    Cx: Sync,
{
    type Output;
    type Error: core::error::Error + Send + Sync + 'static;

    fn run(
        granted: Granted<Self>,
        ctx: &Cx,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send;
}

/// Identifier assigned to an action grant by the concrete policy engine.
///
/// This is correlation metadata; [`Granted<A>`] remains the execution proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GrantId(Uuid);

/// A recorded authorization represented as an unforgeable value.
///
/// Its fields and constructor are private. Callers can inspect or consume a
/// grant, but only the trusted policy path inside core can bind an action to a
/// recorded grant identifier.
#[derive(Debug)]
pub struct Granted<A: ActionMeta> {
    grant_id: GrantId,
    action: A,
}

impl<A: ActionMeta> Granted<A> {
    pub(crate) fn new(grant_id: Uuid, action: A) -> Self {
        Self {
            grant_id: GrantId(grant_id),
            action,
        }
    }

    pub fn grant_id(&self) -> GrantId {
        self.grant_id
    }

    pub fn action(&self) -> &A {
        &self.action
    }

    pub fn into_parts(self) -> (GrantId, A) {
        (self.grant_id, self.action)
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

/// A final policy denial.
///
/// This is an expected authorization conclusion rather than a policy-engine
/// failure. It still implements [`core::error::Error`] so application errors
/// can preserve and propagate the denial as their source.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error(
    "action denied: {}",
    .reason.as_deref().unwrap_or("no reason provided")
)]
pub struct Denied {
    pub reason: Option<String>,
}

// TODO: Here may be a Approval-relevant type for "requesting parent for approval"

#[cfg(test)]
mod tests {
    use alloc::borrow::Cow;

    use loong_contracts::capability::Capabilities;
    use serde_json::Value;
    use uuid::Uuid;

    use super::{ActionMeta, Granted};

    struct TestAction;

    impl ActionMeta for TestAction {
        fn name(&self) -> Cow<'_, str> {
            Cow::Borrowed("test")
        }

        fn payload(&self) -> Cow<'_, Value> {
            Cow::Borrowed(&Value::Null)
        }

        fn required_capabilities(&self) -> Capabilities {
            Capabilities::empty()
        }
    }

    #[test]
    fn granted_binds_the_recorded_id_to_the_action() {
        for recorded_id in [Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(42)] {
            let granted = Granted::new(recorded_id, TestAction);

            assert_eq!(granted.grant_id.0, recorded_id);
            assert!(matches!(granted.action, TestAction));
        }
    }
}
