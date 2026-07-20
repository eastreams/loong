//! Runtime-owned typed tool lookup and granted dispatch.
//!
//! Concrete tools and policies remain outside this module. The registry only
//! binds a plane-local path to an erased `RegisteredTool`. Runtime-owned
//! `ToolInvocation` consumes the grant before calling the resolved entry, so
//! storage choices cannot become tool identity.

pub mod error;
mod invocation;
mod registered;

pub use invocation::{ToolInvocation, ToolInvocationContext};
pub use registered::RegisteredToolError;

use std::{
    borrow::Cow,
    collections::{BTreeMap, btree_map::Entry},
};

use loong_contracts::{Capabilities, Capability, ToolPath};
use loong_core::{
    policy::{
        action::{ActionMeta, ActionMetadata},
        context::ContextFactory,
    },
    tool::ToolImpl,
};
use serde_json::{Value, json};

use self::error::{LookupError, RegistrationError};
use self::registered::RegisteredTool;

/// Authorizes dispatch into one registered tool.
///
/// This action gates tool dispatch only. Side effects inside the selected tool
/// remain governed by their own access actions.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInvocationAction {
    path: ToolPath,
    required_capabilities: Vec<Capability>,
    payload: Value,
}

impl ToolInvocationAction {
    #[must_use]
    pub(crate) fn new(path: ToolPath, required_capabilities: Capabilities, payload: Value) -> Self {
        Self {
            path,
            required_capabilities: required_capabilities.iter().collect(),
            payload,
        }
    }

    #[must_use]
    pub fn path(&self) -> &ToolPath {
        &self.path
    }

    #[must_use]
    pub fn required_capabilities(&self) -> &[Capability] {
        self.required_capabilities.as_slice()
    }

    #[must_use]
    pub fn payload(&self) -> &Value {
        &self.payload
    }
}

impl ActionMeta for ToolInvocationAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "tool.invoke",
            operation: Cow::Owned(self.path.to_string()),
            required_capabilities: Cow::Borrowed(self.required_capabilities.as_slice()),
        }
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "tool_path": self.path.to_string(),
            "payload": self.payload,
        }))
    }
}

/// Runtime-internal storage capability independent of registry representation.
///
/// Resolution returns the concrete registered entry so one successful lookup
/// remains valid through authorization and execution. Grant consumption belongs
/// to `ToolInvocation`, not to the storage abstraction. This private substitution
/// point intentionally permits a future trie or another path index without
/// exposing registered dispatch outside Runtime.
pub(crate) trait ToolPlane<C: ContextFactory>: Send + Sync {
    fn registered_paths(&self) -> Vec<ToolPath>;

    fn resolve(&self, path: &ToolPath) -> Result<&RegisteredTool<C>, LookupError>;
}

/// Ordered default registry for the runtime tool plane.
///
/// The contracts-owned path remains both lookup identity and storage key. A
/// second storage identity is deferred until removal or replacement requires it.
pub struct ToolPlaneRegistry<C: ContextFactory> {
    entries: BTreeMap<ToolPath, RegisteredTool<C>>,
}

impl<C> ToolPlaneRegistry<C>
where
    C: ContextFactory,
{
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub fn register<T>(&mut self, path: ToolPath, tool: T) -> Result<(), RegistrationError>
    where
        T: ToolImpl<C>,
    {
        self.insert_registered(path, || RegisteredTool::from_tool(tool))
    }

    pub fn register_with_success_observer<T, F>(
        &mut self,
        path: ToolPath,
        tool: T,
        observer: F,
    ) -> Result<(), RegistrationError>
    where
        T: ToolImpl<C>,
        F: for<'a> Fn(&C::Cx<'a>, &T::Output) + Send + Sync + 'static,
    {
        self.insert_registered(path, || {
            RegisteredTool::from_tool_with_success_observer(tool, observer)
        })
    }

    /// Share duplicate handling without observing a rejected tool's descriptor.
    fn insert_registered<F>(&mut self, path: ToolPath, build: F) -> Result<(), RegistrationError>
    where
        F: FnOnce() -> RegisteredTool<C>,
    {
        match self.entries.entry(path) {
            Entry::Vacant(entry) => {
                entry.insert(build());
                Ok(())
            }
            Entry::Occupied(entry) => Err(RegistrationError::AlreadyRegistered {
                path: entry.key().clone(),
            }),
        }
    }

    #[must_use]
    pub fn registered_paths(&self) -> Vec<ToolPath> {
        self.entries.keys().cloned().collect()
    }

    pub(crate) fn resolve(&self, path: &ToolPath) -> Result<&RegisteredTool<C>, LookupError> {
        self.entries
            .get(path)
            .ok_or_else(|| LookupError::NotRegistered { path: path.clone() })
    }
}

impl<C> Default for ToolPlaneRegistry<C>
where
    C: ContextFactory,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<C> ToolPlane<C> for ToolPlaneRegistry<C>
where
    C: ContextFactory,
{
    fn registered_paths(&self) -> Vec<ToolPath> {
        ToolPlaneRegistry::registered_paths(self)
    }

    fn resolve(&self, path: &ToolPath) -> Result<&RegisteredTool<C>, LookupError> {
        ToolPlaneRegistry::resolve(self, path)
    }
}

#[cfg(test)]
mod tests;
