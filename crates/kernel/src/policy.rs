//! Interfaces for checking actions against policy.
//!
//! `Policy` handles a known action type. `PolicyAny` accepts any `ActionMeta`.
//! Evaluation has no operational error channel: failures return deny, while
//! `Abstain` and `SkipChain` remain deliberate chain-control decisions. A
//! policy does not run the action or create a grant.

pub mod action;
pub mod engine;

use contracts::capability::Capabilities;
use contracts::policy::PolicyResult;

use crate::policy::action::ActionMeta;

/// Immutable facts supplied for one policy evaluation.
///
/// A trusted gateway creates this snapshot. Its capabilities set the caller's
/// allowed ceiling. Authorization is decided per action by the policy engine.
/// Policy configuration remains in the policy instance or engine; handles and
/// services stay outside this value.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyContext {
    pub capabilities: Capabilities,
}

impl PolicyContext {
    #[must_use]
    pub(crate) const fn new(capabilities: Capabilities) -> Self {
        Self { capabilities }
    }
}

pub trait Policy<A: ActionMeta>: Send + Sync {
    fn evaluate(&self, context: &PolicyContext, action: &A) -> PolicyResult;
}

pub trait PolicyAny: Send + Sync {
    fn evaluate(&self, context: &PolicyContext, action: &dyn ActionMeta) -> PolicyResult;
}
