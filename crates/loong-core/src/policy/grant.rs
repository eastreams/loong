use loong_contracts::GrantId;

use crate::policy::action::Action;

/// TODO: Placeholder for structured action grant metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionGrantInfo;

/// Authorized action plus grant metadata returned by a policy engine.
#[derive(Debug)]
pub struct ActionGrant<A: Action> {
    pub id: GrantId,
    /// TODO: Replace this placeholder with structured grant/audit metadata.
    pub info: ActionGrantInfo,
    pub granted: Granted<A>,
}

impl<A: Action> ActionGrant<A> {
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
pub struct Granted<A: Action>(A);

impl<A: Action> Granted<A> {
    pub(super) fn new(action: A) -> Self {
        Self(action)
    }

    pub fn into_action(self) -> A {
        self.0
    }
}
