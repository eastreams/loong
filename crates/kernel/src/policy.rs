//! Interfaces for checking actions against policy.
//!
//! `Policy` handles a known action type. `PolicyAny` accepts any `ActionMeta`.
//! Evaluation has no operational error channel: failures return deny, while
//! `Abstain` and `SkipChain` remain deliberate chain-control decisions. A
//! policy does not run the action or create a grant.

pub mod action;
pub mod engine;

use contracts::policy::PolicyResult;

use crate::policy::action::ActionMeta;

pub trait Policy<A: ActionMeta>: Send + Sync {
    fn name(&self) -> &'static str;

    fn evaluate(&self, action: &A) -> PolicyResult;
}

pub trait PolicyAny: Send + Sync {
    fn name(&self) -> &'static str;

    fn evaluate(&self, action: &dyn ActionMeta) -> PolicyResult;
}
