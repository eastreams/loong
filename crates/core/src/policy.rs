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

use alloc::boxed::Box;
use async_trait::async_trait;
use loong_contracts::policy::PolicyResult;

use crate::{ContextFactory, action::ActionMeta};

#[async_trait]
pub trait Policy<A: ActionMeta, C: ContextFactory> {
    async fn evaluate(&self, ctx: &C::Cx<'_>, action: &A) -> PolicyResult;
}

#[async_trait]
pub trait PolicyAny<C: ContextFactory> {
    async fn evaluate(&self, ctx: &C::Cx<'_>, action: &dyn ActionMeta) -> PolicyResult;
}
