use std::{any::Any, borrow::Cow, collections::BTreeSet};

use async_trait::async_trait;
use loong_contracts::{Capability, ExecutionPlane, PlaneTier};

use crate::{error::ExecutionError, policy::grant::Granted};

/// A typed, policy-facing unit of side-effect intent.
///
/// Access facades construct actions, policy engines authorize them, and domain
/// executors only run after receiving a [`Granted`] value.
pub trait Action: Any + Send + Sync + 'static {
    /// Stable action kind used by policy diagnostics/audit and grant metadata.
    fn kind(&self) -> &'static str;

    /// Human-readable operation within the coarse execution plane.
    fn operation(&self) -> Cow<'static, str>;

    /// Optional resource label for audit records, such as a path, URL, or
    /// process command.
    fn audit_resource(&self) -> Option<Cow<'static, str>> {
        None
    }

    /// Coarse runtime lane for existing audit and dispatch taxonomy.
    fn execution_plane(&self) -> ExecutionPlane;

    /// Core-vs-extension tier within the coarse execution plane.
    fn plane_tier(&self) -> PlaneTier {
        PlaneTier::Core
    }

    /// Capabilities required before this action may be granted.
    fn required_capabilities(&self) -> BTreeSet<Capability>;
}

impl dyn Action + '_ {
    pub fn downcast_ref<A: Action>(&self) -> Option<&A> {
        (self as &dyn Any).downcast_ref::<A>()
    }
}

/// Executor for a granted domain action.
#[async_trait]
pub trait ActionExecutor<A: Action>: Send + Sync {
    type Output;

    async fn execute(&self, granted: Granted<A>) -> Result<Self::Output, ExecutionError>;
}
