//! Capability limits and requests to a parent.
//!
//! A nested context must never allow more capabilities than its parent. If an
//! action needs more, it may ask the parent to evaluate the same action. It
//! must not enlarge its own set or create another authority token.

use alloc::boxed::Box;
use async_trait::async_trait;
use loong_contracts::capability::Capabilities;

use crate::action::{ActionMeta, Denied, Granted};

/// Has a set of allowed capabilities that are nested monotonically.
pub trait CapabilityContext {
    fn allowed_capabilities(&self) -> Capabilities;
}

/// Can ask a parent policy boundary to authorize the same action.
///
/// `async_trait` intentionally boxes this edge because a parent implementation
/// may call another `PolicyEngine::grant`, which would otherwise create a
/// recursive future type.
#[async_trait]
pub trait ParentGrantRequester {
    /// Ask the parent for a final grant, propagating refusal as `Err(Denied)`.
    ///
    /// The `Err` branch is a policy conclusion rather than a transport or
    /// runtime failure.
    async fn grant<A: ActionMeta>(&self, action: A) -> Result<Granted<A>, Denied>;
}
