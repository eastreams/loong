//! Interfaces for checking actions against policy.
//!
//! `Policy` handles a known action type. `PolicyAny` accepts any `ActionMeta`.
//! Evaluation has no operational error channel: failures return deny, while
//! `Abstain` and `SkipChain` remain deliberate chain-control decisions. A
//! policy does not run the action or create a grant.

mod context;
pub use context::*;

mod engine;
pub use engine::*;

use async_trait::async_trait;
use loong_contracts::policy::PolicyResult;

use crate::{
    ContextFactory,
    action::{ActionMeta, Granted},
};

/// The final result of trying to authorize one action.
///
/// Approval requests are resolved through the parent before this result is
/// returned, so callers only handle an executable grant or a denial.
#[derive(Debug)]
pub enum GrantOutcome<A: ActionMeta> {
    Granted(Granted<A>),
    Denied { reason: Option<String> },
}

#[async_trait]
pub trait Policy<A: ActionMeta, C: ContextFactory> {
    async fn evaluate(&self, ctx: &C::Cx<'_>, action: &A) -> PolicyResult;
}

#[async_trait]
pub trait PolicyAny<C: ContextFactory> {
    async fn evaluate(&self, ctx: &C::Cx<'_>, action: &dyn ActionMeta) -> PolicyResult;
}
