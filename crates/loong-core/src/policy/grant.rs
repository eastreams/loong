use crate::{Action, error::ExecutionError};

#[derive(Clone, Copy, Debug)]
pub struct GrantId(pub u64);

/// Authorization represented as an unforgeable value.
///
/// The constructor is crate-private. Callers can inspect and execute a grant,
/// but only policy helpers inside core can mint one.
#[derive(Debug)]
pub struct Granted<A: Action> {
    grant_id: GrantId,
    // info: ActionGrantInfo, // TODO
    action: A,
}

impl<A: Action> Granted<A> {
    pub(super) fn new(grant_id: GrantId, /*info: ActionGrantInfo,*/ action: A) -> Self {
        Self { grant_id, action }
    }

    #[must_use]
    pub const fn grant_id(&self) -> GrantId {
        self.grant_id
    }

    // #[must_use]
    // pub const fn info(&self) -> &GrantInfo {
    //     &self.info
    // }

    #[must_use]
    pub const fn action(&self) -> &A {
        &self.action
    }

    #[must_use]
    pub fn into_action(self) -> A {
        self.action
    }

    pub async fn execute_with<E>(self, executor: &E) -> Result<E::Output, ExecutionError>
    where
        E: crate::action::ActionExecutor<A> + ?Sized,
    {
        executor.execute(self).await
    }
}
