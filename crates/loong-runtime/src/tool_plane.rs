//! Runtime-owned typed tool lookup and granted dispatch.
//!
//! Concrete tools and policies remain outside this module. The registry only
//! binds a contracts-owned path to an erased `RegisteredTool`. Runtime-owned
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

use self::error::{LookupError, RegistrationError};
use self::registered::RegisteredTool;
use loong_contracts::{Capabilities, Capability, ToolPath};
use loong_core::{
    policy::{
        action::{ActionMeta, ActionMetadata},
        context::ContextFactory,
    },
    tool::ToolImpl,
};
use serde_json::Value;

/// Plane-owned presentation metadata for one registered tool.
///
/// Path is deliberately absent. The registry index owns identity; this value
/// only describes how that identity is presented outside the plane. A provider
/// wire name exists only for direct tools; discoverable tools are invoked through
/// the leased discovery envelope and retain their exact plane path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRegistration {
    Direct { provider_name: String },
    Discoverable { discovery_name: String },
}

impl ToolRegistration {
    #[must_use]
    pub fn direct(provider_name: impl Into<String>) -> Self {
        Self::Direct {
            provider_name: provider_name.into(),
        }
    }

    #[must_use]
    pub fn discoverable(discovery_name: impl Into<String>) -> Self {
        Self::Discoverable {
            discovery_name: discovery_name.into(),
        }
    }
}

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
        // The action type and metadata already identify tool invocation and its
        // contracts-owned path. Broad policy should inspect the original agent
        // payload rather than an allocation-heavy runtime envelope.
        Cow::Borrowed(&self.payload)
    }
}

/// Runtime-owned registry for the tool plane.
///
/// Runtime owns this concrete type directly. A single ordered map keeps the
/// contracts-owned path as both lookup identity and storage key; introducing a
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

    pub fn register<T>(
        &mut self,
        path: ToolPath,
        registration: ToolRegistration,
        tool: T,
    ) -> Result<(), RegistrationError>
    where
        T: ToolImpl<C>,
    {
        self.insert_registered(path, || RegisteredTool::from_tool(registration, tool))
    }

    pub fn register_with_success_observer<T, F>(
        &mut self,
        path: ToolPath,
        registration: ToolRegistration,
        tool: T,
        observer: F,
    ) -> Result<(), RegistrationError>
    where
        T: ToolImpl<C>,
        F: for<'a> Fn(&C::Cx<'a>, &T::Output) + Send + Sync + 'static,
    {
        self.insert_registered(path, || {
            RegisteredTool::from_tool_with_success_observer(registration, tool, observer)
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

    pub(crate) fn resolve(
        &self,
        path: &ToolPath,
    ) -> Result<(&ToolPath, &RegisteredTool<C>), LookupError> {
        self.entries
            .get_key_value(path)
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

#[cfg(test)]
mod tests;
