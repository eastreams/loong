//! Interfaces for checking actions against policy.
//!
//! `Policy` handles a known action type. `PolicyAny` accepts any `ActionMeta`.
//! Evaluation has no operational error channel: failures return deny, while
//! `Abstain` and `SkipChain` remain deliberate chain-control decisions. A
//! policy does not run the action or create a grant.

pub mod action;
pub mod engine;

use std::sync::Arc;

use contracts::capability::Capabilities;
use contracts::policy::PolicyResult;

use crate::policy::action::ActionMeta;
use crate::resource::Resources;

/// Immutable facts supplied for one policy evaluation.
///
/// A trusted gateway creates this snapshot. Its capabilities set the caller's
/// allowed ceiling. Authorization is decided per action by the policy engine.
/// Policy configuration remains in the policy instance or engine; handles and
/// services stay outside this value.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PolicyContext {
    pub capabilities: Capabilities,
    /// Operational resources fixed at agent assembly time. Policies may read
    /// these to evaluate actions (for example a workspace boundary), but
    /// resources are not capabilities: they never expand the capability
    /// ceiling.
    pub resources: Arc<Resources>,
}

impl PolicyContext {
    #[must_use]
    pub(crate) fn new(capabilities: Capabilities, resources: Arc<Resources>) -> Self {
        Self {
            capabilities,
            resources,
        }
    }
}

pub trait Policy<A: ActionMeta>: Send + Sync {
    fn name(&self) -> &'static str;

    fn evaluate(&self, context: &PolicyContext, action: &A) -> PolicyResult;
}

pub trait PolicyAny: Send + Sync {
    fn name(&self) -> &'static str;

    fn evaluate(&self, context: &PolicyContext, action: &dyn ActionMeta) -> PolicyResult;
}
