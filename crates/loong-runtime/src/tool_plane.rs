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

use std::{borrow::Cow, collections::BTreeMap, fmt};

use loong_contracts::{Capabilities, Capability};
use loong_core::{
    policy::{
        action::{ActionMeta, ActionMetadata},
        context::ContextFactory,
    },
    tool::ToolImpl,
};
use serde_json::{Value, json};
use slotmap::{SlotMap, new_key_type};

use self::error::{LookupError, RegistrationError};
use self::registered::RegisteredTool;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Path type chosen by the default runtime registry.
///
/// Paths stay out of core/contracts because another `ToolPlane` implementation
/// may choose a different lookup key.
pub struct ToolPath {
    segments: Vec<String>,
}

impl ToolPath {
    #[must_use]
    pub fn from_segments(segments: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            segments: segments.into_iter().map(Into::into).collect(),
        }
    }

    #[must_use]
    pub fn segments(&self) -> &[String] {
        self.segments.as_slice()
    }

    // Provider/catalog names still arrive as dotted strings. This conversion
    // belongs to the concrete plane path, not the core tool abstraction.
    fn from_dotted(path: &str) -> Self {
        Self::from_segments(path.split('.'))
    }
}

impl fmt::Display for ToolPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut segments = self.segments.iter();
        let Some(first) = segments.next() else {
            return Ok(());
        };
        formatter.write_str(first)?;
        for segment in segments {
            formatter.write_str(".")?;
            formatter.write_str(segment)?;
        }
        Ok(())
    }
}

impl From<&str> for ToolPath {
    fn from(path: &str) -> Self {
        Self::from_dotted(path)
    }
}

impl From<String> for ToolPath {
    fn from(path: String) -> Self {
        Self::from_dotted(path.as_str())
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
    type Path;

    fn registered_paths(&self) -> Vec<Self::Path>;

    fn resolve(&self, path: &Self::Path) -> Result<&RegisteredTool<C>, LookupError<Self::Path>>;
}

new_key_type! {
    // Storage identity is intentionally private. ToolPath remains the stable
    // lookup and audit identity if the registry representation changes.
    struct ToolSlot;
}

/// Slot-backed default registry for the runtime tool plane.
///
/// The ordered path index provides stable lookup/enumeration while slots keep
/// tool storage independent from externally visible identity.
pub struct ToolPlaneRegistry<C: ContextFactory> {
    entries: SlotMap<ToolSlot, RegisteredTool<C>>,
    paths: BTreeMap<ToolPath, ToolSlot>,
}

impl<C> ToolPlaneRegistry<C>
where
    C: ContextFactory,
{
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: SlotMap::with_key(),
            paths: BTreeMap::new(),
        }
    }

    pub fn register<T>(&mut self, path: ToolPath, tool: T) -> Result<(), RegistrationError>
    where
        T: ToolImpl<C>,
    {
        if self.paths.contains_key(&path) {
            return Err(RegistrationError::AlreadyRegistered { path });
        }

        let slot = self.entries.insert(RegisteredTool::from_tool(tool));
        self.paths.insert(path, slot);
        Ok(())
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
        if self.paths.contains_key(&path) {
            return Err(RegistrationError::AlreadyRegistered { path });
        }

        let slot = self
            .entries
            .insert(RegisteredTool::from_tool_with_success_observer(
                tool, observer,
            ));
        self.paths.insert(path, slot);
        Ok(())
    }

    #[must_use]
    pub fn registered_paths(&self) -> Vec<ToolPath> {
        self.paths.keys().cloned().collect()
    }

    pub(crate) fn resolve(
        &self,
        path: &ToolPath,
    ) -> Result<&RegisteredTool<C>, LookupError<ToolPath>> {
        let slot = self
            .paths
            .get(path)
            .ok_or_else(|| LookupError::NotRegistered { path: path.clone() })?;
        self.entries
            .get(*slot)
            .ok_or_else(|| LookupError::RegistryInvariant { path: path.clone() })
    }

    #[cfg(test)]
    #[must_use]
    fn entry_count(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    #[must_use]
    fn path_count(&self) -> usize {
        self.paths.len()
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
    type Path = ToolPath;

    fn registered_paths(&self) -> Vec<ToolPath> {
        ToolPlaneRegistry::registered_paths(self)
    }

    fn resolve(&self, path: &ToolPath) -> Result<&RegisteredTool<C>, LookupError<ToolPath>> {
        ToolPlaneRegistry::resolve(self, path)
    }
}

#[cfg(test)]
mod tests;
