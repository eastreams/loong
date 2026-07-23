//! Interfaces for checking actions against policy.
//!
//! `Policy` handles a known action type. `PolicyAny` accepts any `ActionMeta`.
//! A policy returns a decision; it does not run the action or create a grant.

mod context;
pub use context::*;

mod engine;
pub use engine::*;

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
