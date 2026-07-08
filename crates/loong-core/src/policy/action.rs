use std::{any::Any, borrow::Cow, collections::BTreeSet};

use async_trait::async_trait;
use loong_contracts::Capability;

use crate::policy::grant::Granted;

/// Type-erased metadata for a policy-facing unit of side-effect intent.
///
/// Access facades construct actions, policy engines authorize them, and domain
/// executors only run after receiving a [`Granted`] value. This trait is the
/// metadata view used by policy, audit, and type-erased pipeline stages; it is
/// not an execution hook. The execution hook is [`Action`].
pub trait ActionMeta: Any + Send + Sync + 'static {
    /// Stable action kind used by policy diagnostics/audit and grant metadata.
    fn kind(&self) -> &'static str;

    /// Human-readable operation within the coarse execution plane.
    fn operation(&self) -> Cow<'static, str>;

    /// Optional resource label for audit records, such as a path, URL, or
    /// process command.
    fn audit_resource(&self) -> Option<Cow<'static, str>> {
        None
    }

    /// Capabilities this action requires before it may execute.
    fn required_capabilities(&self) -> BTreeSet<Capability>;
}

/// Executable action implementation for one concrete invocation context.
///
/// Implement this only at the domain side-effect boundary. `run` consumes a
/// [`Granted<Self>`], so raw action values cannot execute side effects.
#[async_trait]
pub trait Action<Cx>: ActionMeta + Sized
where
    Cx: Sync,
{
    type Output;
    type Error;

    async fn run(granted: Granted<Self>, ctx: &Cx) -> Result<Self::Output, Self::Error>;
}
