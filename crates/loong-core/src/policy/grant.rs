use std::marker::PhantomData;

use crate::{action::Action, error::ExecutionError};

#[derive(Clone, Copy, Debug)]
pub struct GrantId(pub u64);

/// Authorization represented as an unforgeable value.
///
/// The constructor is crate-private. Callers can inspect and execute a grant,
/// but only policy helpers inside core can mint one.
#[derive(Debug)]
pub struct Granted<A: Action> {
    pub grant_id: GrantId,
    // info: ActionGrantInfo, // TODO
    pub action: A,
    /// avoid manual implementation
    _phantom_data: PhantomData<()>,
}

impl<A: Action> Granted<A> {
    pub(super) fn new(grant_id: GrantId, /*info: ActionGrantInfo,*/ action: A) -> Self {
        Self {
            grant_id,
            action,
            _phantom_data: PhantomData,
        }
    }

    pub async fn execute_with<E>(self, executor: &E) -> Result<E::Output, ExecutionError>
    where
        E: crate::action::ActionExecutor<A> + ?Sized,
    {
        executor.execute(self).await
    }
}
