//! Interfaces for checking actions against policy.
//!
//! `Policy` handles a known action type. `PolicyAny` accepts any `ActionMeta`.
//! Evaluation has no operational error channel: failures return deny, while
//! `Abstain` and `SkipChain` remain deliberate chain-control decisions. A
//! policy does not run the action or create a grant.

pub mod action;
pub mod engine;

use loong_contracts::capability::Capabilities;
use loong_contracts::policy::PolicyResult;

use crate::{Facade, policy::action::ActionMeta};

pub trait Policy<A: ActionMeta>: Sync {
    fn evaluate(&self, ctx: &Facade, action: &A) -> PolicyResult;
}

pub trait PolicyAny: Sync {
    fn evaluate(&self, ctx: &Facade, action: &dyn ActionMeta) -> PolicyResult;
}

/// Has a set of allowed capabilities that are nested monotonically.
pub trait CapabilityContext {
    fn allowed_capabilities(&self) -> Capabilities;
}
