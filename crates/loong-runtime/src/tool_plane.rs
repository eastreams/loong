//! Runtime-owned typed tool lookup and granted dispatch.
//!
//! Concrete tools and policies remain outside this module. The registry only
//! binds a plane-local path to an erased `RegisteredTool` and consumes a grant
//! before dispatch, so storage choices cannot become tool identity.

pub mod error;

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use async_trait::async_trait;
use loong_contracts::{Capability, ToolSpec};
use loong_core::{
    policy::{
        action::{ActionMeta, ActionMetadata},
        context::ContextFactory,
        grant::Granted,
    },
    tool::{RegisteredTool, ToolImpl, ToolProvenance},
};
use serde_json::{Value, json};
use slotmap::{SlotMap, new_key_type};

use self::error::{DispatchError, LookupError, RegistrationError};

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
    pub fn new(
        path: ToolPath,
        required_capabilities: BTreeSet<Capability>,
        payload: Value,
    ) -> Self {
        Self {
            path,
            required_capabilities: required_capabilities.into_iter().collect(),
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
    pub fn into_parts(self) -> (ToolPath, Vec<Capability>, Value) {
        (self.path, self.required_capabilities, self.payload)
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

/// Runtime tool-plane capability independent of its registry representation.
#[async_trait]
pub trait ToolPlane<C: ContextFactory>: Send + Sync {
    type Path: Clone + fmt::Debug + Ord + fmt::Display + Send + Sync + 'static;
    type InvocationAction: ActionMeta + Send + Sync + 'static;

    fn registered_paths(&self) -> Vec<Self::Path>;

    fn spec(&self, path: &Self::Path) -> Result<&ToolSpec, LookupError<Self::Path>>;

    /// Consumes an already governed invocation grant.
    ///
    /// App orchestration should use `ctx.tool(path)?.invoke(payload).await` so
    /// capability restriction, policy grant, audit, and dispatch remain paired.
    async fn invoke(
        &self,
        grant: Granted<Self::InvocationAction>,
        ctx: &C::Cx<'_>,
    ) -> Result<Value, DispatchError<Self::Path>>;
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
    entries: SlotMap<ToolSlot, ToolEntry<C>>,
    paths: BTreeMap<ToolPath, ToolSlot>,
}

struct ToolEntry<C: ContextFactory> {
    tool: RegisteredTool<C>,
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
        self.register_with_provenance(path, ToolProvenance::Builtin, tool)
    }

    pub fn register_with_provenance<T>(
        &mut self,
        path: ToolPath,
        provenance: ToolProvenance,
        tool: T,
    ) -> Result<(), RegistrationError>
    where
        T: ToolImpl<C>,
    {
        if self.paths.contains_key(&path) {
            return Err(RegistrationError::AlreadyRegistered { path });
        }

        let slot = self.entries.insert(ToolEntry {
            tool: RegisteredTool::from_tool(provenance, tool),
        });
        self.paths.insert(path, slot);
        Ok(())
    }

    pub fn register_with_provenance_and_success_observer<T, F>(
        &mut self,
        path: ToolPath,
        provenance: ToolProvenance,
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

        let slot = self.entries.insert(ToolEntry {
            tool: RegisteredTool::from_tool_with_success_observer(provenance, tool, observer),
        });
        self.paths.insert(path, slot);
        Ok(())
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

#[async_trait]
impl<C> ToolPlane<C> for ToolPlaneRegistry<C>
where
    C: ContextFactory,
{
    type Path = ToolPath;
    type InvocationAction = ToolInvocationAction;

    fn registered_paths(&self) -> Vec<ToolPath> {
        self.paths.keys().cloned().collect()
    }

    fn spec(&self, path: &ToolPath) -> Result<&ToolSpec, LookupError<ToolPath>> {
        let slot = self
            .paths
            .get(path)
            .ok_or_else(|| LookupError::NotRegistered { path: path.clone() })?;
        let entry = self
            .entries
            .get(*slot)
            .ok_or_else(|| LookupError::RegistryInvariant { path: path.clone() })?;

        Ok(entry.tool.spec())
    }

    async fn invoke(
        &self,
        grant: Granted<ToolInvocationAction>,
        ctx: &C::Cx<'_>,
    ) -> Result<Value, DispatchError<ToolPath>> {
        let (path, _required_capabilities, payload) = grant.into_action().into_parts();
        let slot = self
            .paths
            .get(&path)
            .ok_or_else(|| DispatchError::RegistryInvariant { path: path.clone() })?;
        let entry = self
            .entries
            .get(*slot)
            .ok_or_else(|| DispatchError::RegistryInvariant { path })?;

        entry
            .tool
            .invoke(ctx, payload)
            .await
            .map_err(|source| DispatchError::Tool { source })
    }
}

#[cfg(test)]
mod tests;
