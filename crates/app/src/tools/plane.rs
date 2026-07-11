use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::OnceLock,
};

use async_trait::async_trait;
use loong_contracts::{
    Capability, PolicyDecision, PolicyGrant, ToolExecutionError, ToolInputError, ToolPlaneError,
};
use loong_core::{
    policy::grant::Granted,
    policy::{
        action::{ActionMeta, ActionMetadata},
        context::ContextFactory,
        policy::Policy,
    },
    tool::{RegisteredTool, ToolImpl, ToolProvenance},
};
use serde_json::{Value, json};
use slotmap::{SlotMap, new_key_type};

use crate::context::AppContextFactory;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ToolPath(String);

impl ToolPath {
    #[must_use]
    pub(crate) fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ToolPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<&str> for ToolPath {
    fn from(path: &str) -> Self {
        Self::new(path)
    }
}

impl From<String> for ToolPath {
    fn from(path: String) -> Self {
        Self::new(path)
    }
}

/// App-plane action for authorizing entry into one registered tool.
///
/// This gates dispatch only. Side effects inside the tool still need their own
/// access actions, such as fs read/write actions.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolInvocationAction {
    path: ToolPath,
    required_capabilities: Vec<Capability>,
    payload: Value,
}

impl ToolInvocationAction {
    #[must_use]
    pub(crate) fn new(
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
    pub(crate) fn path(&self) -> &ToolPath {
        &self.path
    }

    #[must_use]
    pub(crate) fn required_capabilities(&self) -> &[Capability] {
        self.required_capabilities.as_slice()
    }

    #[must_use]
    pub(crate) fn into_parts(self) -> (ToolPath, Vec<Capability>, Value) {
        (self.path, self.required_capabilities, self.payload)
    }
}

impl ActionMeta for ToolInvocationAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "tool.invoke",
            operation: Cow::Borrowed(self.path.as_str()),
            required_capabilities: Cow::Borrowed(self.required_capabilities.as_slice()),
        }
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "tool_path": self.path.as_str(),
            "payload": self.payload,
        }))
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ToolInvocationAllowPolicy;

#[async_trait]
impl<C> Policy<C, ToolInvocationAction> for ToolInvocationAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("tool-invocation-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &ToolInvocationAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("tool invocation passed capability gate".into()),
            reason: "tool invocation allowed by app policy".into(),
        }
    }
}

/// App-owned typed tool dispatch plane.
///
/// The plane only resolves and executes app-registered tools. Kernel remains
/// responsible for authorization and audit, so callers resolve by path, request
/// a kernel grant, then call `invoke`. Payload parsing belongs to the selected
/// tool; the plane does not claim ownership of a payload shape.
#[async_trait]
pub(crate) trait ToolPlane<C: ContextFactory>: Send + Sync {
    fn contains(&self, path: &ToolPath) -> bool;

    async fn invoke(
        &self,
        grant: Granted<ToolInvocationAction>,
        ctx: &C::Cx<'_>,
    ) -> Result<Value, ToolPlaneError>;
}

new_key_type! {
    // Internal storage handle only. Public identity and audit payloads keep
    // using ToolPath so slot allocation never becomes observable API.
    struct ToolSlot;
}

pub(crate) struct AppToolPlane<C: ContextFactory> {
    entries: SlotMap<ToolSlot, ToolEntry<C>>,
    paths: BTreeMap<ToolPath, ToolSlot>,
}

struct ToolEntry<C: ContextFactory> {
    tool: RegisteredTool<C>,
}

impl<C> ToolEntry<C>
where
    C: ContextFactory,
{
    fn new(tool: RegisteredTool<C>) -> Self {
        Self { tool }
    }
}

impl<C> AppToolPlane<C>
where
    C: ContextFactory,
{
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            entries: SlotMap::with_key(),
            paths: BTreeMap::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn register<T>(&mut self, path: ToolPath, tool: T) -> Result<(), ToolPlaneError>
    where
        T: ToolImpl<C>,
    {
        self.register_with_provenance(path, ToolProvenance::Builtin, tool)
    }

    pub(crate) fn register_with_provenance<T>(
        &mut self,
        path: ToolPath,
        provenance: ToolProvenance,
        tool: T,
    ) -> Result<(), ToolPlaneError>
    where
        T: ToolImpl<C>,
    {
        if self.paths.contains_key(&path) {
            return Err(ToolPlaneError::DuplicateTool(path.to_string()));
        }

        let entry = ToolEntry::new(RegisteredTool::from_tool(provenance, tool));
        let slot = self.entries.insert(entry);
        self.paths.insert(path, slot);
        Ok(())
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn contains(&self, path: &ToolPath) -> bool {
        self.paths.contains_key(path)
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.entry_count()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn entry_count(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn path_count(&self) -> usize {
        self.paths.len()
    }
}

#[async_trait]
impl<C> ToolPlane<C> for AppToolPlane<C>
where
    C: ContextFactory,
{
    fn contains(&self, path: &ToolPath) -> bool {
        self.paths.contains_key(path)
    }

    /// This is not expected to be used directly.
    /// Use `ctx.invoke_tool()` in the future
    async fn invoke(
        &self,
        grant: Granted<ToolInvocationAction>,
        ctx: &C::Cx<'_>,
    ) -> Result<Value, ToolPlaneError> {
        // Consuming the grant here makes audit/grant enforcement automatic for
        // concrete tool authors: ToolImpl implementers never receive a raw
        // dispatch path that can bypass app orchestration.
        let (path, _required_capabilities, payload) = grant.into_action().into_parts();
        let slot = self
            .paths
            .get(&path)
            .ok_or_else(|| ToolPlaneError::ToolNotFound(path.to_string()))?;
        let entry = self
            .entries
            .get(*slot)
            .ok_or_else(|| ToolPlaneError::ToolNotFound(path.to_string()))?;

        entry
            .tool
            .invoke(ctx, payload)
            .await
            .map_err(|error| ToolPlaneError::Execution(tool_execution_error_reason(error)))
    }
}

// TODO: make this in a unified struct
pub(crate) fn app_tool_plane() -> &'static dyn ToolPlane<AppContextFactory> {
    static TOOL_PLANE: OnceLock<AppToolPlane<AppContextFactory>> = OnceLock::new();
    TOOL_PLANE.get_or_init(|| {
        #[allow(unused_mut)]
        let mut plane = AppToolPlane::new();
        #[cfg(feature = "tool-file")]
        // Only the provider-facing file-read tool is migrated here. The direct
        // `read` facade stays on the legacy fallback path until it becomes an
        // aggregate typed tool for path/query/glob actions.
        {
            let duplicate = plane
                .register_with_provenance(
                    ToolPath::from("file.read"),
                    ToolProvenance::Builtin,
                    loong_tools::file::ReadFileTool,
                )
                .err();
            debug_assert!(
                duplicate.is_none(),
                "duplicate builtin tool path: {duplicate:?}"
            );
        }
        plane
    })
}

// Boundary conversion: legacy kernel/app error surfaces still carry plain
// strings, but policy-denial detection depends on preserving prefixes such as
// `policy_denied:` instead of formatting the whole error display.
pub(crate) fn tool_execution_error_reason(error: ToolExecutionError) -> String {
    match error {
        ToolExecutionError::Input(input_error) => match input_error {
            ToolInputError::MissingField { field } => {
                format!("missing tool input field `{field}`")
            }
            ToolInputError::InvalidField { field, reason } => {
                format!("invalid tool input field `{field}`: {reason}")
            }
            ToolInputError::InvalidPayload { reason } => reason,
            unknown => unknown.to_string(),
        },
        ToolExecutionError::Execution { reason } => reason,
        unknown => unknown.to_string(),
    }
}

#[cfg(test)]
mod tests;
