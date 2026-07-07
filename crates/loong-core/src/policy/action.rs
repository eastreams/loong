use std::{any::Any, borrow::Cow, collections::BTreeSet};

use loong_contracts::Capability;

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

    /// Capabilities this action requires before it may execute.
    fn required_capabilities(&self) -> BTreeSet<Capability>;
}
