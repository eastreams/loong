use std::{any::Any, borrow::Cow};

use async_trait::async_trait;
use loong_contracts::Capability;
use serde_json::Value;

use crate::policy::grant::Granted;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionMetadata<'a> {
    pub kind: &'static str,
    pub operation: Cow<'a, str>,
    pub required_capabilities: Cow<'a, [Capability]>,
}

/// Type-erased metadata for a policy-facing unit of side-effect intent.
///
/// Access facades construct actions, policy engines authorize them, and domain
/// executors only run after receiving a [`Granted`] value. This trait is the
/// metadata view used by policy, audit, and type-erased pipeline stages; it is
/// not an execution hook. The execution hook is [`Action`].
pub trait ActionMeta: Any + Send + Sync + 'static {
    /// Cheap metadata used by capability gates, policy diagnostics, and audit.
    fn metadata(&self) -> ActionMetadata<'_>;

    /// Optional resource label for audit records, such as a path, URL, or
    /// process command.
    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        None
    }

    /// Structured action payload for type-erased policy stages.
    ///
    /// The action type already carries the policy meaning; this method only
    /// materializes the dynamic payload when a policy needs JSON-shaped input.
    /// Return a borrowed value when the action stores JSON already, or an owned
    /// view when the JSON is derived. It has no default so actions cannot
    /// silently omit their policy-facing payload.
    fn payload(&self) -> Cow<'_, Value>;
}

/// Executable action implementation for one concrete invocation context.
///
/// Implement this only at the domain side-effect boundary. `run` consumes a
/// [`Granted<Self>`], so raw action values cannot execute side effects.
#[async_trait]
pub trait Action<Cx: ?Sized>: ActionMeta + Sized
where
    Cx: Sync,
{
    type Output;
    type Error;

    async fn run(granted: Granted<Self>, ctx: &Cx) -> Result<Self::Output, Self::Error>;
}
