//! Capability limits and requests to a parent.
//!
//! A nested context must never allow more capabilities than its parent. If an
//! action needs more, it may ask the parent to evaluate the same action. It
//! must not enlarge its own set or create another authority token.

use loong_contracts::capability::Capabilities;

use crate::action::{ActionGrant, ActionMeta};

/// Has a set of allowed capabilities that are nested monotonically.
pub trait CapabilityContext {
    fn allowed_capabilities(&self) -> Capabilities;
}

/// Can request parent for a grant
pub trait GrantRequester {
    type Error;

    fn grant<A: ActionMeta>(
        &self,
        action: A,
    ) -> impl Future<Output = Result<ActionGrant<A>, Self::Error>> + Send;
}
