use std::borrow::Cow;

use async_trait::async_trait;
use loong_contracts::PolicyGrant;

use crate::policy::{action::ActionMeta, context::ContextFactory};

/// Typed policy that can evaluate one action kind.
///
/// Kernel/runtime crates own concrete policy registration and ordering. Core
/// keeps only the action-facing contract so access facades can authorize typed
/// effects without depending on a concrete kernel.
#[async_trait]
pub trait Policy<C: ContextFactory, A: ActionMeta>: Send + Sync {
    /// Stable policy name used in grant metadata.
    fn name(&self) -> Cow<'static, str>;

    /// Evaluate whether this policy decides authorization or controls pipeline
    /// flow for `action`.
    async fn grant(&self, ctx: &C::Cx<'_>, action: &A) -> PolicyGrant;
}

/// Untyped policy that can evaluate every action kind in a pipeline subchain.
#[async_trait]
pub trait PolicyAny<C: ContextFactory>: Send + Sync {
    fn name(&self) -> Cow<'static, str>;

    async fn grant(&self, ctx: &C::Cx<'_>, action: &dyn ActionMeta) -> PolicyGrant;
}
