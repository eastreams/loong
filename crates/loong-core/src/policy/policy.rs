use std::borrow::Cow;

use async_trait::async_trait;
use loong_contracts::PolicyGrant;

use crate::{policy::action::Action, policy::context::PolicyContextFactory};

/// Typed policy that can evaluate one action kind.
///
/// Kernel/runtime crates own concrete policy registration and ordering. Core
/// keeps only the action-facing contract so access facades can authorize typed
/// effects without depending on a concrete kernel.
#[async_trait]
pub trait Policy<F: PolicyContextFactory, A: Action>: Send + Sync {
    /// Stable policy name used in grant metadata.
    fn name(&self) -> Cow<'static, str>;

    /// Evaluate whether this policy allows, denies, or abstains from `action`.
    async fn grant(&self, ctx: &F::Context<'_>, action: &A) -> PolicyGrant;
}

/// Untyped policy that evaluate all action kinds
#[async_trait]
pub trait PolicyAny<F: PolicyContextFactory>: Send + Sync {
    fn name(&self) -> &'static str;

    async fn grant(&self, ctx: &F::Context<'_>, action: &dyn Action) -> PolicyGrant;
}
