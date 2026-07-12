use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::OnceLock,
};

use async_trait::async_trait;
use loong_contracts::{
    Capability, PolicyDecision, PolicyGrant, ToolExecutionError, ToolPlaneError, ToolSpec,
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
pub(crate) struct ToolPath {
    segments: Vec<String>,
}

impl ToolPath {
    #[must_use]
    pub(crate) fn from_segments(segments: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            segments: segments.into_iter().map(Into::into).collect(),
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn segments(&self) -> &[String] {
        self.segments.as_slice()
    }

    // Provider/catalog names still arrive as dotted strings. Keep that bridge
    // at the app plane boundary so core/contracts never learn this path shape.
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
    type Path: Clone + Ord + fmt::Display + Send + Sync + 'static;
    type InvocationAction: ActionMeta + Send + Sync + 'static;

    /// Enumerates app-registered paths from the plane index.
    ///
    /// Catalog/prompt projection should depend on this boundary instead of
    /// rebuilding typed tool paths from static descriptors.
    fn registered_paths(&self) -> Vec<Self::Path>;

    fn spec(&self, path: &Self::Path) -> Result<&ToolSpec, ToolPlaneError>;

    async fn invoke(
        &self,
        grant: Granted<Self::InvocationAction>,
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

    pub(crate) fn register_with_provenance_and_success_observer<T, F>(
        &mut self,
        path: ToolPath,
        provenance: ToolProvenance,
        tool: T,
        observer: F,
    ) -> Result<(), ToolPlaneError>
    where
        T: ToolImpl<C>,
        F: for<'a> Fn(&C::Cx<'a>, &T::Output) -> Result<(), ToolExecutionError>
            + Send
            + Sync
            + 'static,
    {
        if self.paths.contains_key(&path) {
            return Err(ToolPlaneError::DuplicateTool(path.to_string()));
        }

        let entry = ToolEntry::new(RegisteredTool::from_tool_with_success_observer(
            provenance, tool, observer,
        ));
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
    type Path = ToolPath;
    type InvocationAction = ToolInvocationAction;

    fn registered_paths(&self) -> Vec<ToolPath> {
        self.paths.keys().cloned().collect()
    }

    fn spec(&self, path: &ToolPath) -> Result<&ToolSpec, ToolPlaneError> {
        let slot = self
            .paths
            .get(path)
            .ok_or_else(|| ToolPlaneError::ToolNotFound(path.to_string()))?;
        let entry = self
            .entries
            .get(*slot)
            .ok_or_else(|| ToolPlaneError::ToolNotFound(path.to_string()))?;

        Ok(entry.tool.spec())
    }

    /// This is not expected to be called from tool orchestration directly.
    /// Use `ctx.tool(path)?.invoke(payload).await` so grant and audit stay paired.
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
            .map_err(ToolPlaneError::from)
    }
}

// TODO(runtime-owner): remove this lint exception with the global OnceLock.
// `Runtime<C>` must construct the builtin plane through a fallible bootstrap,
// so duplicate registrations are returned to the host instead of panicking.
#[allow(clippy::expect_used)]
pub(crate) fn app_tool_plane()
-> &'static dyn ToolPlane<AppContextFactory, Path = ToolPath, InvocationAction = ToolInvocationAction>
{
    static TOOL_PLANE: OnceLock<AppToolPlane<AppContextFactory>> = OnceLock::new();
    TOOL_PLANE.get_or_init(|| {
        #[allow(unused_mut)]
        let mut plane = AppToolPlane::new();
        #[cfg(feature = "tool-file")]
        // `read` is the aggregate typed facade; provider aliases such as
        // `file.read` canonicalize to this path before plane lookup.
        {
            plane
                .register_with_provenance(
                    ToolPath::from("read"),
                    ToolProvenance::Builtin,
                    loong_tools::file::ReadTool::new("read"),
                )
                .expect("builtin typed tool path `read` must be unique");

            plane
                .register_with_provenance(
                    ToolPath::from("write"),
                    ToolProvenance::Builtin,
                    loong_tools::file::WriteTool::new("write"),
                )
                .expect("builtin typed tool path `write` must be unique");

            plane
                .register_with_provenance_and_success_observer(
                    ToolPath::from("edit"),
                    ToolProvenance::Builtin,
                    loong_tools::file::EditTool::new("edit"),
                    |_ctx, output: &loong_tools::file::EditOutput| {
                        // Preview events are an app-runtime side channel; the
                        // concrete tool only returns typed before/after data.
                        crate::tools::file::emit_file_change_preview(
                            output.path.as_path(),
                            crate::tools::runtime_events::ToolFileChangeKind::Edit,
                            Some(output.before.as_str()),
                            output.after.as_str(),
                        );
                        Ok::<(), ToolExecutionError>(())
                    },
                )
                .expect("builtin typed tool path `edit` must be unique");

            // Legacy read-family discoverable paths keep their own typed
            // entries so audit path and response metadata do not collapse into
            // the aggregate `read` surface while fs side effects still move to
            // access actions.
            plane
                .register_with_provenance(
                    ToolPath::from("glob.search"),
                    ToolProvenance::Builtin,
                    loong_tools::file::GlobSearchTool::new("glob.search"),
                )
                .expect("builtin typed tool path `glob.search` must be unique");

            plane
                .register_with_provenance(
                    ToolPath::from("content.search"),
                    ToolProvenance::Builtin,
                    loong_tools::file::ContentSearchTool::new("content.search"),
                )
                .expect("builtin typed tool path `content.search` must be unique");
        }
        plane
    })
}

#[cfg(test)]
mod tests;
