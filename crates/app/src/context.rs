use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use loong_contracts::{
    CapabilityToken, ExecutionPlane, GovernedSessionMode, InvocationOutcome, PlaneTier,
    ToolPlaneError,
};
use loong_core::policy::context::{CapabilityContext, ContextFactory};
use loong_kernel::access::fs::{FsPathPolicyContext, FsResolutionContext};
use loong_kernel::{
    AccessCx, AuditSink, Capability, Clock, ExecutionRoute, FanoutAuditSink, HarnessKind,
    InMemoryAuditSink, JsonlAuditSink, Kernel, KernelAccess, KernelInvocationContext,
    PolicyPipeline, SystemClock, VerticalPackManifest,
    policy::{
        FsAtomicWriteAllowPolicy, FsContentSearchAllowPolicy, FsCopyFileAllowPolicy,
        FsCreateDirAllAllowPolicy, FsGlobAllowPolicy, FsInspectPathAllowPolicy,
        FsPathAllowedRootsPolicy, FsReadAllowPolicy, FsReadDirAllowPolicy,
        FsReadFilenameDenyPolicy, FsRemoveDirAllAllowPolicy, FsRemoveFileAllowPolicy,
        FsRenameAllowPolicy, FsResolvePathAllowPolicy, FsWriteAllowPolicy,
    },
};
use loong_runtime::{
    runtime::Runtime,
    tool_plane::{ToolInvocationAction, ToolPath},
};
use serde_json::Value;

use crate::config::{AuditMode, LoongConfig};
use crate::conversation::{
    ConstrainedSubagentContractView, ConstrainedSubagentExecution, ConstrainedSubagentIdentity,
    ConstrainedSubagentProfile, DelegateBuiltinProfile,
};
use crate::runtime_self_continuity::RuntimeSelfContinuity;
use crate::tools::ToolView;
use crate::tools::runtime_config::ToolRuntimeNarrowing;

/// Default pack identifier used by embedded runtime entry points.
const EMBEDDED_RUNTIME_PACK_ID: &str = "dev-automation";

/// Default token TTL (24 hours) for long-running embedded runtime entry points.
pub const DEFAULT_TOKEN_TTL_S: u64 = 86400;

/// App-owned execution context shared by session, tool, action, and policy paths.
///
/// Long-lived authority is shared through `Arc`; per-invocation state is an
/// immutable overlay. Deriving a child context can replace invocation metadata
/// or narrow capabilities, but can never add authority beyond its parent.
#[derive(Clone)]
pub struct AppContext {
    inner: Arc<AppContextInner>,
}

impl fmt::Debug for AppContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Capability tokens are authority-bearing and must never enter debug logs.
        formatter
            .debug_struct("AppContext")
            .field("session_id", &self.session_id)
            .field("parent_session_id", &self.parent_session_id)
            .field("plane", &self.plane)
            .field("tier", &self.tier)
            .field("session_mode", &self.session_mode)
            .finish_non_exhaustive()
    }
}

/// Storage behind the cheap-clone [`AppContext`] handle.
///
/// This type has no independent lifecycle or behavior. It is public only so
/// Rust's `Deref` contract can preserve direct read access to context fields.
#[doc(hidden)]
#[derive(Clone)]
pub struct AppContextInner {
    pub(crate) runtime: Arc<Runtime<AppContextFactory>>,
    pub(crate) pack: Arc<VerticalPackManifest>,
    pub(crate) token: Arc<CapabilityToken>,
    pub(crate) tool_runtime_config: Arc<crate::tools::runtime_config::ToolRuntimeConfig>,
    pub(crate) effective_capabilities: BTreeSet<Capability>,
    pub(crate) plane: ExecutionPlane,
    pub(crate) tier: PlaneTier,
    pub(crate) request_parameters: Option<Arc<Value>>,
    pub(crate) fs_resolution_root: Arc<PathBuf>,
    pub(crate) fs_allowed_roots: Arc<[PathBuf]>,
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub profile: Option<DelegateBuiltinProfile>,
    pub tool_view: ToolView,
    pub session_mode: GovernedSessionMode,
    pub workspace_root: Option<PathBuf>,
    pub active_skill_roots: Vec<PathBuf>,
    pub visible_skill_roots: Vec<PathBuf>,
    pub runtime_narrowing: Option<ToolRuntimeNarrowing>,
    pub subagent_execution: Option<ConstrainedSubagentExecution>,
    pub subagent_contract: Option<ConstrainedSubagentContractView>,
    pub(crate) runtime_self_continuity: Option<RuntimeSelfContinuity>,
}

impl Deref for AppContext {
    type Target = AppContextInner;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

impl DerefMut for AppContext {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.inner)
    }
}

impl AppContext {
    pub fn new(
        runtime: Arc<Runtime<AppContextFactory>>,
        token: CapabilityToken,
        tool_runtime_config: crate::tools::runtime_config::ToolRuntimeConfig,
        session_id: impl Into<String>,
        tool_view: ToolView,
        session_mode: GovernedSessionMode,
    ) -> Result<Self, String> {
        let pack = runtime
            .kernel()
            .pack_manifest(&token.pack_id)
            .map_err(|error| format!("app context pack lookup failed: {error}"))?
            .clone();
        let effective_capabilities = token.allowed_capabilities.clone();
        let (fs_resolution_root, fs_allowed_roots) = fs_access_root_view(&tool_runtime_config)?;
        let session_id = normalize_session_id(session_id.into());
        let _ = crate::conversation::mailbox_for_session(&session_id);
        Ok(Self {
            inner: Arc::new(AppContextInner {
                runtime,
                pack: Arc::new(pack),
                token: Arc::new(token),
                tool_runtime_config: Arc::new(tool_runtime_config),
                effective_capabilities,
                plane: ExecutionPlane::Runtime,
                tier: PlaneTier::Core,
                request_parameters: None,
                fs_resolution_root: Arc::new(fs_resolution_root),
                fs_allowed_roots: fs_allowed_roots.into(),
                session_id,
                parent_session_id: None,
                profile: None,
                tool_view,
                session_mode,
                workspace_root: None,
                active_skill_roots: Vec::new(),
                visible_skill_roots: Vec::new(),
                runtime_narrowing: None,
                subagent_execution: None,
                subagent_contract: None,
                runtime_self_continuity: None,
            }),
        })
    }

    /// Constructs authority for one concrete session after its identity is known.
    ///
    /// Hosts retain the runtime and call this once when they create or attach to
    /// a session. Invocation overlays derive from the returned context and must
    /// not mint replacement session tokens.
    pub fn new_session(
        runtime: Arc<Runtime<AppContextFactory>>,
        config: &LoongConfig,
        session_id: impl Into<String>,
        agent_id: &str,
        session_mode: GovernedSessionMode,
        ttl_s: u64,
    ) -> Result<Self, String> {
        let token = match session_mode {
            GovernedSessionMode::MutatingCapable => {
                runtime
                    .kernel()
                    .issue_token(EMBEDDED_RUNTIME_PACK_ID, agent_id, ttl_s)
            }
            GovernedSessionMode::AdvisoryOnly => {
                // Advisory sessions may assemble governed read/provider input,
                // but never receive generic tool invocation or write authority.
                let allowed_capabilities = BTreeSet::from([
                    Capability::MemoryRead,
                    Capability::FilesystemRead,
                    Capability::NetworkEgress,
                ]);
                runtime.kernel().issue_scoped_token(
                    EMBEDDED_RUNTIME_PACK_ID,
                    agent_id,
                    &allowed_capabilities,
                    ttl_s,
                )
            }
        }
        .map_err(|error| format!("kernel session token issue failed: {error}"))?;
        let tool_runtime_config =
            crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);

        Self::new(
            runtime,
            token,
            tool_runtime_config,
            session_id,
            crate::tools::runtime_tool_view_from_loong_config(config),
            session_mode,
        )
    }

    pub fn child(
        &self,
        session_id: impl Into<String>,
        parent_session_id: impl Into<String>,
        tool_view: ToolView,
    ) -> Self {
        let session_id = normalize_session_id(session_id.into());
        let parent_session_id = normalize_session_id(parent_session_id.into());
        let _ = crate::conversation::mailbox_for_session(&session_id);
        let _ = crate::conversation::mailbox_for_session(&parent_session_id);
        let mut child = self.clone();
        let state = Arc::make_mut(&mut child.inner);
        state.session_id = session_id;
        state.parent_session_id = Some(parent_session_id);
        state.profile = None;
        state.tool_view = tool_view;
        state.workspace_root = None;
        state.active_skill_roots.clear();
        state.visible_skill_roots.clear();
        state.runtime_narrowing = None;
        state.subagent_execution = None;
        state.subagent_contract = None;
        state.runtime_self_continuity = None;
        child
    }

    #[must_use]
    pub fn for_session(&self, session_id: impl Into<String>, tool_view: ToolView) -> Self {
        let session_id = normalize_session_id(session_id.into());
        let _ = crate::conversation::mailbox_for_session(&session_id);
        let mut session = self.clone();
        let state = Arc::make_mut(&mut session.inner);
        state.session_id = session_id;
        state.parent_session_id = None;
        state.profile = None;
        state.tool_view = tool_view;
        state.workspace_root = None;
        state.active_skill_roots.clear();
        state.visible_skill_roots.clear();
        state.runtime_narrowing = None;
        state.subagent_execution = None;
        state.subagent_contract = None;
        state.runtime_self_continuity = None;
        session
    }

    #[must_use]
    pub fn with_workspace_root(mut self, workspace_root: PathBuf) -> Self {
        self.workspace_root = Some(workspace_root);
        self
    }

    #[must_use]
    pub fn with_active_skill_roots(mut self, active_skill_roots: Vec<PathBuf>) -> Self {
        self.active_skill_roots = active_skill_roots
            .into_iter()
            .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
            .collect();
        self
    }

    #[must_use]
    pub fn with_visible_skill_roots(mut self, visible_skill_roots: Vec<PathBuf>) -> Self {
        self.visible_skill_roots = visible_skill_roots
            .into_iter()
            .map(|path| std::fs::canonicalize(&path).unwrap_or(path))
            .collect();
        self
    }

    #[must_use]
    pub fn with_profile(mut self, profile: DelegateBuiltinProfile) -> Self {
        self.profile = Some(profile);
        self
    }

    #[must_use]
    pub fn with_runtime_narrowing(mut self, runtime_narrowing: ToolRuntimeNarrowing) -> Self {
        if !runtime_narrowing.is_empty() {
            self.runtime_narrowing = Some(runtime_narrowing.clone());
            let contract = self.subagent_contract.take().unwrap_or_default();
            self.subagent_contract = Some(contract.with_runtime_narrowing(runtime_narrowing));
            self.synchronize_runtime_narrowing_views();
        }
        self
    }

    #[must_use]
    pub fn with_subagent_execution(
        mut self,
        subagent_execution: ConstrainedSubagentExecution,
    ) -> Self {
        let existing_contract = self.subagent_contract.take();
        let existing_workspace_root = self.workspace_root.clone();
        let existing_identity = existing_contract
            .as_ref()
            .and_then(ConstrainedSubagentContractView::resolved_identity)
            .cloned();
        let existing_profile = existing_contract
            .as_ref()
            .and_then(|contract| contract.profile);
        let existing_runtime_narrowing = existing_contract
            .as_ref()
            .map(|contract| contract.runtime_narrowing.clone())
            .filter(|runtime_narrowing| !runtime_narrowing.is_empty());
        let mut subagent_execution = subagent_execution.with_resolved_profile();
        if subagent_execution.identity.is_none()
            && let Some(identity) = existing_identity
        {
            subagent_execution.identity = Some(identity);
        }
        let mut merged_contract = subagent_execution.contract_view();
        if merged_contract.profile.is_none()
            && let Some(profile) = existing_profile
        {
            merged_contract = merged_contract.with_profile(profile);
        }
        if merged_contract.runtime_narrowing.is_empty()
            && let Some(runtime_narrowing) = existing_runtime_narrowing
        {
            merged_contract = merged_contract.with_runtime_narrowing(runtime_narrowing);
        }
        if self.workspace_root.is_none() {
            self.workspace_root = subagent_execution
                .workspace_root
                .clone()
                .or(existing_workspace_root);
        }
        self.subagent_contract = Some(merged_contract);
        self.subagent_execution = Some(subagent_execution);
        self.synchronize_runtime_narrowing_views();
        self
    }

    #[must_use]
    pub fn with_subagent_profile(mut self, subagent_profile: ConstrainedSubagentProfile) -> Self {
        if let Some(subagent_execution) = self.subagent_execution.as_mut() {
            subagent_execution.profile = Some(subagent_profile);
        }
        let contract = self.subagent_contract.take().unwrap_or_default();
        self.subagent_contract = Some(contract.with_profile(subagent_profile));
        self.synchronize_runtime_narrowing_views();
        self
    }

    #[must_use]
    pub fn with_subagent_identity(
        mut self,
        subagent_identity: ConstrainedSubagentIdentity,
    ) -> Self {
        if subagent_identity.is_empty() {
            return self;
        }
        if let Some(subagent_execution) = self.subagent_execution.as_mut() {
            subagent_execution.identity = Some(subagent_identity.clone());
        }
        let contract = self.subagent_contract.take().unwrap_or_default();
        self.subagent_contract = Some(contract.with_identity(subagent_identity));
        self.synchronize_runtime_narrowing_views();
        self
    }

    pub fn resolved_runtime_narrowing(&self) -> Option<&ToolRuntimeNarrowing> {
        self.resolve_runtime_narrowing_ref()
    }

    pub fn resolved_subagent_profile(&self) -> Option<ConstrainedSubagentProfile> {
        self.subagent_execution
            .as_ref()
            .map(ConstrainedSubagentExecution::resolved_profile)
            .or_else(|| {
                self.subagent_contract
                    .as_ref()
                    .and_then(ConstrainedSubagentContractView::resolved_profile)
            })
    }

    pub fn resolved_subagent_identity(&self) -> Option<&ConstrainedSubagentIdentity> {
        self.subagent_execution
            .as_ref()
            .and_then(|execution| execution.identity.as_ref())
            .or_else(|| {
                self.subagent_contract
                    .as_ref()
                    .and_then(ConstrainedSubagentContractView::resolved_identity)
            })
    }

    pub fn resolved_subagent_contract(&self) -> Option<ConstrainedSubagentContractView> {
        let mut contract = self
            .subagent_execution
            .as_ref()
            .map(ConstrainedSubagentExecution::contract_view)
            .or(self.subagent_contract.clone())?;
        if let Some(stored_contract) = self.subagent_contract.as_ref()
            && contract.profile.is_none()
            && let Some(profile) = stored_contract.profile
        {
            contract = contract.with_profile(profile);
        }
        if let Some(runtime_narrowing) = self.resolved_runtime_narrowing().cloned() {
            contract = contract.with_runtime_narrowing(runtime_narrowing);
        }
        (!contract.is_empty()).then_some(contract)
    }

    pub fn subagent_runtime_narrowing(&self) -> Option<&ToolRuntimeNarrowing> {
        self.resolved_runtime_narrowing()
    }

    #[must_use]
    pub(crate) fn with_runtime_self_continuity(
        mut self,
        runtime_self_continuity: RuntimeSelfContinuity,
    ) -> Self {
        if !runtime_self_continuity.is_empty() {
            self.runtime_self_continuity = Some(runtime_self_continuity);
        }
        self
    }

    fn synchronize_runtime_narrowing_views(&mut self) {
        let resolved = self.resolve_runtime_narrowing_ref().cloned();
        let execution_narrowing = resolved.clone().unwrap_or_default();
        self.runtime_narrowing = resolved;
        if let Some(execution) = self.subagent_execution.as_mut() {
            execution.runtime_narrowing = execution_narrowing.clone();
        }
        if let Some(contract) = self.subagent_contract.as_mut() {
            contract.runtime_narrowing = execution_narrowing;
        }
    }

    fn resolve_runtime_narrowing_ref(&self) -> Option<&ToolRuntimeNarrowing> {
        self.runtime_narrowing
            .as_ref()
            .filter(|narrowing| !narrowing.is_empty())
            .or_else(|| {
                self.subagent_execution
                    .as_ref()
                    .map(|execution| &execution.runtime_narrowing)
                    .filter(|narrowing| !narrowing.is_empty())
            })
            .or_else(|| {
                self.subagent_contract
                    .as_ref()
                    .map(|contract| &contract.runtime_narrowing)
                    .filter(|narrowing| !narrowing.is_empty())
            })
    }

    pub fn pack_id(&self) -> &str {
        &self.token.pack_id
    }

    pub fn agent_id(&self) -> &str {
        &self.token.agent_id
    }

    #[must_use]
    pub(crate) fn runtime(&self) -> &Runtime<AppContextFactory> {
        self.runtime.as_ref()
    }

    #[must_use]
    pub(crate) fn pack(&self) -> &VerticalPackManifest {
        self.pack.as_ref()
    }

    #[must_use]
    pub fn token(&self) -> &CapabilityToken {
        self.token.as_ref()
    }

    #[must_use]
    pub(crate) fn tool_runtime_config(&self) -> &crate::tools::runtime_config::ToolRuntimeConfig {
        self.tool_runtime_config.as_ref()
    }

    #[must_use]
    pub fn plane(&self) -> ExecutionPlane {
        self.plane
    }

    #[must_use]
    pub fn tier(&self) -> PlaneTier {
        self.tier
    }

    pub(crate) fn for_invocation(
        &self,
        plane: ExecutionPlane,
        tier: PlaneTier,
        request_parameters: Option<&Value>,
        tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
    ) -> Result<Self, String> {
        self.for_invocation_with_capabilities(
            self.effective_capabilities.clone(),
            plane,
            tier,
            request_parameters,
            tool_runtime_config,
        )
    }

    pub(crate) fn for_invocation_with_capabilities(
        &self,
        effective_capabilities: BTreeSet<Capability>,
        plane: ExecutionPlane,
        tier: PlaneTier,
        request_parameters: Option<&Value>,
        tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
    ) -> Result<Self, String> {
        if !effective_capabilities.is_subset(&self.effective_capabilities) {
            let missing_capabilities = effective_capabilities
                .difference(&self.effective_capabilities)
                .map(|capability| capability.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "child execution context cannot add capabilities: missing {missing_capabilities}"
            ));
        }

        let (fs_resolution_root, fs_allowed_roots) = fs_access_root_view(tool_runtime_config)?;
        Ok(Self {
            inner: Arc::new(AppContextInner {
                runtime: self.runtime.clone(),
                pack: self.pack.clone(),
                token: self.token.clone(),
                tool_runtime_config: Arc::new(tool_runtime_config.clone()),
                effective_capabilities,
                plane,
                tier,
                request_parameters: request_parameters.cloned().map(Arc::new),
                fs_resolution_root: Arc::new(fs_resolution_root),
                fs_allowed_roots: fs_allowed_roots.into(),
                session_id: self.session_id.clone(),
                parent_session_id: self.parent_session_id.clone(),
                profile: self.profile,
                tool_view: self.tool_view.clone(),
                session_mode: self.session_mode,
                workspace_root: self.workspace_root.clone(),
                active_skill_roots: self.active_skill_roots.clone(),
                visible_skill_roots: self.visible_skill_roots.clone(),
                runtime_narrowing: self.runtime_narrowing.clone(),
                subagent_execution: self.subagent_execution.clone(),
                subagent_contract: self.subagent_contract.clone(),
                runtime_self_continuity: self.runtime_self_continuity.clone(),
            }),
        })
    }

    pub(crate) fn narrow_capabilities(
        &self,
        effective_capabilities: BTreeSet<Capability>,
    ) -> Result<Self, String> {
        if !effective_capabilities.is_subset(&self.effective_capabilities) {
            let missing_capabilities = effective_capabilities
                .difference(&self.effective_capabilities)
                .map(|capability| capability.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "child execution context cannot add capabilities: missing {missing_capabilities}"
            ));
        }

        // Tool-to-tool and tool-to-access paths inherit runtime references but
        // must not regain capabilities removed by the parent context.
        let mut child = self.clone();
        child.effective_capabilities = effective_capabilities;
        Ok(child)
    }

    #[must_use]
    pub(crate) fn access(&self) -> AccessCx<'_, '_, AppContextFactory> {
        // AccessCx construction is localized at the concrete context boundary.
        // Tool/action code should call ctx.access() rather than rethreading the
        // kernel reference or recreating access facades by hand.
        AccessCx::new(self.runtime.kernel(), self)
    }

    pub(crate) fn tool(&self, path: ToolPath) -> Result<ToolInvocation<'_>, ToolPlaneError> {
        let spec = self.runtime.tools().spec(&path)?;
        let mut required_capabilities = BTreeSet::from([Capability::InvokeTool]);
        required_capabilities.extend(spec.required_capabilities.iter().copied());

        Ok(ToolInvocation {
            ctx: self,
            path,
            default_capabilities: required_capabilities,
            capability_override: None,
        })
    }
}

/// App orchestration handle for one typed tool invocation.
///
/// Concrete tool implementations never receive this handle; they only receive
/// parsed input after `invoke` has paired kernel grant, plane dispatch, and audit.
pub(crate) struct ToolInvocation<'ctx> {
    ctx: &'ctx AppContext,
    path: ToolPath,
    default_capabilities: BTreeSet<Capability>,
    capability_override: Option<BTreeSet<Capability>>,
}

impl ToolInvocation<'_> {
    /// Bind a narrowed capability set before payload dispatch.
    ///
    /// This proves the override cannot add authority before `invoke` builds the
    /// child context and asks kernel for a `ToolInvocationAction` grant.
    pub(crate) fn with_capabilities_override(
        mut self,
        capabilities: BTreeSet<Capability>,
    ) -> Result<Self, loong_kernel::KernelError> {
        let mut default_tool_capabilities = self.default_capabilities.clone();
        default_tool_capabilities.remove(&Capability::InvokeTool);

        if !capabilities.is_subset(&default_tool_capabilities) {
            return Err(loong_kernel::KernelError::ToolPlane(
                ToolPlaneError::Execution(
                    "policy_denied: tool capability override cannot add capabilities".to_owned(),
                ),
            ));
        }

        self.capability_override = Some(capabilities);
        Ok(self)
    }

    pub(crate) async fn invoke(self, payload: Value) -> Result<Value, loong_kernel::KernelError> {
        let mut default_tool_capabilities = self.default_capabilities.clone();
        default_tool_capabilities.remove(&Capability::InvokeTool);
        let tool_capabilities = self
            .capability_override
            .unwrap_or(default_tool_capabilities);

        let mut required_capabilities = BTreeSet::from([Capability::InvokeTool]);
        required_capabilities.extend(tool_capabilities);
        let tool_ctx = self
            .ctx
            .narrow_capabilities(required_capabilities.clone())
            .map_err(|error| {
                loong_kernel::KernelError::ToolPlane(ToolPlaneError::Execution(format!(
                    "policy_denied: {error}"
                )))
            })?;
        let action = ToolInvocationAction::new(self.path, required_capabilities, payload);
        let grant = self
            .ctx
            .runtime
            .kernel()
            .grant_action(tool_ctx.pack_id(), tool_ctx.token(), action, &tool_ctx)
            .await?;
        let audit_path = grant.granted.as_ref().path().to_string();
        let audit_caps = grant
            .granted
            .as_ref()
            .required_capabilities()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();

        match self
            .ctx
            .runtime
            .tools()
            .invoke(grant.granted, &tool_ctx)
            .await
        {
            Ok(output) => {
                tool_ctx.runtime.kernel().record_tool_invocation(
                    &tool_ctx,
                    audit_path,
                    &audit_caps,
                    InvocationOutcome::Completed,
                )?;
                Ok(output)
            }
            Err(error) => {
                let (error_kind, reason) = match &error {
                    ToolPlaneError::ToolNotFound(reason) => ("not_found", reason.clone()),
                    ToolPlaneError::DuplicateTool(reason) => ("duplicate_tool", reason.clone()),
                    ToolPlaneError::CoreAdapterNotFound(reason) => {
                        ("core_adapter_not_found", reason.clone())
                    }
                    ToolPlaneError::ExtensionNotFound(reason) => {
                        ("extension_not_found", reason.clone())
                    }
                    ToolPlaneError::NoDefaultCoreAdapter => {
                        ("no_default_core_adapter", error.to_string())
                    }
                    ToolPlaneError::Input(input_error) => ("input_error", input_error.to_string()),
                    ToolPlaneError::Execution(reason) => ("execution", reason.clone()),
                    _ => ("tool_plane", error.to_string()),
                };
                tool_ctx.runtime.kernel().record_tool_invocation(
                    &tool_ctx,
                    audit_path,
                    &audit_caps,
                    InvocationOutcome::Failed {
                        error_kind: error_kind.to_owned(),
                        reason,
                    },
                )?;
                Err(loong_kernel::KernelError::ToolPlane(error))
            }
        }
    }
}

impl KernelAccess<AppContextFactory> for AppContext {
    fn access(&self) -> AccessCx<'_, '_, AppContextFactory> {
        // Concrete tools depend on this narrow requirement instead of the app
        // context type. Delegate to the inherent accessor so this concrete
        // context has one AccessCx construction point.
        AppContext::access(self)
    }
}

impl CapabilityContext for AppContext {
    fn allowed_capabilities(&self) -> BTreeSet<Capability> {
        self.effective_capabilities.clone()
    }
}

impl KernelInvocationContext for AppContext {
    fn pack(&self) -> &VerticalPackManifest {
        self.pack()
    }

    fn token(&self) -> &CapabilityToken {
        self.token()
    }

    fn now_epoch_s(&self) -> u64 {
        self.runtime.kernel().now_epoch_s()
    }

    fn request_parameters(&self) -> Option<&Value> {
        self.request_parameters.as_deref()
    }
}

impl FsResolutionContext for AppContext {
    fn fs_resolution_root(&self) -> &Path {
        self.fs_resolution_root.as_path()
    }
}

impl FsPathPolicyContext for AppContext {
    fn fs_allowed_roots(&self) -> &[PathBuf] {
        self.fs_allowed_roots.as_ref()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AppContextFactory;

impl ContextFactory for AppContextFactory {
    type Cx<'a> = AppContext;
}

fn fs_access_root_view(
    config: &crate::tools::runtime_config::ToolRuntimeConfig,
) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let allowed_roots = collect_allowed_roots(config)?;
    let Some(default_resolution_root) = allowed_roots.first().cloned() else {
        return Err("filesystem access requires at least one allowed root".to_owned());
    };
    let resolution_root = config
        .path_resolution_root()
        .map(Path::to_path_buf)
        .unwrap_or(default_resolution_root);
    Ok((resolution_root, allowed_roots))
}

fn collect_allowed_roots(
    config: &crate::tools::runtime_config::ToolRuntimeConfig,
) -> Result<Vec<PathBuf>, String> {
    let mut raw_roots = Vec::new();

    if let Some(file_root) = config.file_root.as_ref() {
        raw_roots.push(file_root.clone());
    }

    if let Some(workspace_root) = config.workspace_root.as_ref() {
        let workspace_root_is_new = raw_roots.iter().all(|root| root != workspace_root);
        if workspace_root_is_new {
            raw_roots.push(workspace_root.clone());
        }
    }

    if raw_roots.is_empty() {
        raw_roots.push(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    }

    raw_roots
        .into_iter()
        .map(canonicalize_or_fallback)
        .collect::<Result<Vec<_>, _>>()
}

fn canonicalize_or_fallback(path: PathBuf) -> Result<PathBuf, String> {
    if path.exists() {
        let canonical = dunce::canonicalize(&path)
            .map_err(|error| format!("failed to canonicalize {}: {error}", path.display()))?;
        return Ok(dunce::simplified(&canonical).to_path_buf());
    }
    Ok(crate::tools::normalize_without_fs(&path))
}

fn normalize_session_id(session_id: String) -> String {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        "default".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Bootstrap a minimal in-memory kernel suitable for tests.
///
/// Registers a default pack manifest with the embedded runtime tool, memory, filesystem,
/// and public-web capabilities, then issues a long-lived token for the given
/// `agent_id`.
///
/// Production hosts should retain `bootstrap_runtime_with_config` and call
/// `AppContext::new_session` after resolving concrete session identity.
#[cfg(test)]
pub(crate) fn bootstrap_test_app_context(agent_id: &str, ttl_s: u64) -> Result<AppContext, String> {
    bootstrap_app_context_with_audit_sink(
        agent_id,
        ttl_s,
        Arc::new(InMemoryAuditSink::default()) as Arc<dyn AuditSink>,
        &LoongConfig::default(),
    )
}

/// Bootstrap a governed host context for transitional runtime entrypoints.
///
/// This installs the audit sink selected by `config.audit`, registers the
/// embedded runtime pack plus the core tool/memory adapters and policy
/// pipeline, and issues a long-lived capability token before a concrete
/// session is known.
///
/// The helper intentionally stays below higher-level runtime initialization: it
/// does not export `LOONG_*` environment variables, resolve chat session
/// ids, or prepare channel/conversation state. Callers that need those side
/// effects should compose it with `runtime_env::initialize_runtime_environment`
/// or a surface-specific bootstrap such as `chat::initialize_cli_turn_runtime`.
// TODO(session-owned-context): delete this host/root-context bootstrap after
// runtime owners construct one AppContext per concrete session.
pub fn bootstrap_app_context_with_config(
    agent_id: &str,
    ttl_s: u64,
    config: &LoongConfig,
) -> Result<AppContext, String> {
    bootstrap_app_context_with_audit_sink(agent_id, ttl_s, build_audit_sink(config)?, config)
}

/// Bootstrap the long-lived runtime authority shared by app sessions.
///
/// This constructs the configured kernel and tool plane without issuing a
/// session token or inventing a host-level context. Hosts should retain the
/// returned runtime and construct `AppContext` values for concrete sessions.
pub fn bootstrap_runtime_with_config(
    config: &LoongConfig,
) -> Result<Arc<Runtime<AppContextFactory>>, String> {
    let tool_rt = crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
    bootstrap_runtime_with_audit_sink(build_audit_sink(config)?, config, &tool_rt)
}

fn build_audit_sink(config: &LoongConfig) -> Result<Arc<dyn AuditSink>, String> {
    match config.audit.mode {
        AuditMode::InMemory => Ok(Arc::new(InMemoryAuditSink::default()) as Arc<dyn AuditSink>),
        AuditMode::Jsonl => build_jsonl_audit_sink(config),
        AuditMode::Fanout => {
            let durable = build_jsonl_audit_sink(config)?;
            if !config.audit.retain_in_memory {
                return Ok(durable);
            }

            Ok(Arc::new(FanoutAuditSink::new(vec![
                durable,
                Arc::new(InMemoryAuditSink::default()) as Arc<dyn AuditSink>,
            ])) as Arc<dyn AuditSink>)
        }
    }
}

fn build_jsonl_audit_sink(config: &LoongConfig) -> Result<Arc<dyn AuditSink>, String> {
    let path = config.audit.resolved_path();
    JsonlAuditSink::new(path.clone())
        .map(|sink| Arc::new(sink) as Arc<dyn AuditSink>)
        .map_err(|error| {
            format!(
                "failed to initialize durable audit journal {}: {error}",
                path.display()
            )
        })
}

fn bootstrap_app_context_with_audit_sink(
    agent_id: &str,
    ttl_s: u64,
    audit_sink: Arc<dyn AuditSink>,
    config: &LoongConfig,
) -> Result<AppContext, String> {
    let tool_rt = crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
    let runtime = bootstrap_runtime_with_audit_sink(audit_sink, config, &tool_rt)?;
    let token = runtime
        .kernel()
        .issue_token(EMBEDDED_RUNTIME_PACK_ID, agent_id, ttl_s)
        .map_err(|e| format!("kernel token issue failed: {e}"))?;

    AppContext::new(
        runtime,
        token,
        tool_rt,
        agent_id,
        crate::tools::runtime_tool_view_from_loong_config(config),
        GovernedSessionMode::MutatingCapable,
    )
}

// Keep production-selected and test-injected audit sinks on one runtime construction path.
fn bootstrap_runtime_with_audit_sink(
    audit_sink: Arc<dyn AuditSink>,
    config: &LoongConfig,
    tool_rt: &crate::tools::runtime_config::ToolRuntimeConfig,
) -> Result<Arc<Runtime<AppContextFactory>>, String> {
    let mut policy = PolicyPipeline::<AppContextFactory>::new_legacy_allow_fallback()
        .with_policy(crate::tools::plane::ToolInvocationAllowPolicy)
        .with_policy(FsResolvePathAllowPolicy::target())
        .with_policy(FsResolvePathAllowPolicy::entry())
        .with_policy(FsPathAllowedRootsPolicy::target())
        .with_policy(FsPathAllowedRootsPolicy::entry());
    if !tool_rt.fs.deny_read_filenames.is_empty() {
        policy.push_policy(FsReadFilenameDenyPolicy::new(
            tool_rt.fs.deny_read_filenames.clone(),
        ));
    }
    policy.push_policy(FsReadAllowPolicy);
    policy.push_policy(FsWriteAllowPolicy);
    policy.push_policy(FsAtomicWriteAllowPolicy);
    policy.push_policy(FsCopyFileAllowPolicy);
    policy.push_policy(FsCreateDirAllAllowPolicy);
    policy.push_policy(FsRemoveFileAllowPolicy);
    policy.push_policy(FsRemoveDirAllAllowPolicy);
    policy.push_policy(FsRenameAllowPolicy);
    policy.push_policy(FsInspectPathAllowPolicy);
    policy.push_policy(FsGlobAllowPolicy);
    policy.push_policy(FsReadDirAllowPolicy);
    policy.push_policy(FsContentSearchAllowPolicy);
    let mut kernel =
        Kernel::with_policy_runtime(policy, Arc::new(SystemClock) as Arc<dyn Clock>, audit_sink);

    let pack = VerticalPackManifest {
        pack_id: EMBEDDED_RUNTIME_PACK_ID.to_owned(),
        domain: "mvp".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([
            Capability::InvokeTool,
            Capability::NetworkEgress,
            Capability::MemoryRead,
            Capability::MemoryWrite,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
        ]),
        metadata: BTreeMap::new(),
    };
    kernel
        .register_pack(pack)
        .map_err(|e| format!("kernel pack registration failed: {e}"))?;

    #[cfg(feature = "memory-sqlite")]
    {
        let mem_config =
            crate::memory::runtime_config::MemoryRuntimeConfig::from_memory_config_without_env_overrides(
                &config.memory,
            );
        kernel.register_core_memory_adapter(crate::memory::KernelMemoryAdapter::with_config(
            mem_config,
        ));
        kernel
            .set_default_core_memory_adapter("mvp-memory")
            .map_err(|e| format!("set default memory adapter failed: {e}"))?;
    }

    crate::tools::register_kernel_tools(&mut kernel, tool_rt.clone(), config.observability.clone())
        .map_err(|e| format!("kernel tool registration failed: {e}"))?;

    let tools = crate::tools::plane::builtin_tool_plane()
        .map_err(|error| format!("builtin tool registration failed: {error}"))?;
    Ok(Arc::new(Runtime::new(kernel, tools)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;

    use loong_contracts::Capability;
    use tempfile::tempdir;

    use super::*;
    use crate::config::MemoryProfile;
    use crate::memory::runtime_config::MemoryRuntimeConfig;
    use crate::test_utils::ScopedEnv;

    #[test]
    fn runtime_bootstrap_does_not_issue_a_host_token() {
        let audit = Arc::new(InMemoryAuditSink::default());
        let config = LoongConfig::default();
        let tool_rt =
            crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(&config, None);

        let runtime = bootstrap_runtime_with_audit_sink(audit.clone(), &config, &tool_rt)
            .expect("runtime bootstrap");

        assert!(
            runtime
                .kernel()
                .pack_manifest(EMBEDDED_RUNTIME_PACK_ID)
                .is_ok()
        );
        assert!(
            audit.snapshot().is_empty(),
            "runtime ownership must not mint a host-level token"
        );
    }

    #[test]
    fn advisory_session_context_uses_non_mutating_capabilities() {
        let audit = Arc::new(InMemoryAuditSink::default());
        let config = LoongConfig::default();
        let tool_runtime_config =
            crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(&config, None);
        let runtime = bootstrap_runtime_with_audit_sink(audit, &config, &tool_runtime_config)
            .expect("runtime bootstrap");

        let context = AppContext::new_session(
            runtime,
            &config,
            "advisory-session",
            "advisory-agent",
            GovernedSessionMode::AdvisoryOnly,
            60,
        )
        .expect("advisory session context");

        assert_eq!(
            context.token().allowed_capabilities,
            BTreeSet::from([
                Capability::MemoryRead,
                Capability::FilesystemRead,
                Capability::NetworkEgress,
            ])
        );
    }

    #[test]
    fn app_context_rejects_token_for_unregistered_pack() {
        let runtime = Arc::new(Runtime::new(
            Kernel::<AppContextFactory>::new_without_audit(),
            crate::tools::plane::test_builtin_tool_plane(),
        ));
        let token = CapabilityToken {
            token_id: "unregistered-pack-token".to_owned(),
            pack_id: "missing-pack".to_owned(),
            agent_id: "test-agent".to_owned(),
            allowed_capabilities: BTreeSet::new(),
            issued_at_epoch_s: 0,
            expires_at_epoch_s: 60,
            generation: 1,
        };

        let error = match AppContext::new(
            runtime,
            token,
            crate::tools::runtime_config::ToolRuntimeConfig::default(),
            "test-session",
            crate::tools::runtime_tool_view(),
            loong_contracts::GovernedSessionMode::MutatingCapable,
        ) {
            Ok(_) => panic!("unregistered token pack must not construct an app context"),
            Err(error) => error,
        };

        assert!(error.contains("pack not found: missing-pack"));
    }

    #[test]
    fn bootstrap_app_context_with_config_writes_jsonl_audit_events() {
        let tempdir = tempdir().expect("tempdir");
        let audit_path = tempdir.path().join("audit").join("events.jsonl");
        let mut config = LoongConfig::default();
        config.audit.mode = AuditMode::Jsonl;
        config.audit.path = audit_path.display().to_string();
        config.audit.retain_in_memory = false;

        let context = bootstrap_app_context_with_config("test-agent", 60, &config)
            .expect("bootstrap with jsonl audit should succeed");

        assert_eq!(context.agent_id(), "test-agent");

        let journal = fs::read_to_string(&audit_path).expect("audit journal should exist");
        assert_eq!(
            journal.lines().count(),
            1,
            "token bootstrap should emit one audit event"
        );
        assert!(
            journal.contains("\"TokenIssued\"") || journal.contains("\"token_id\""),
            "bootstrap journal should capture token issuance"
        );
    }

    #[test]
    fn bootstrap_app_context_with_config_writes_fanout_audit_events() {
        let tempdir = tempdir().expect("tempdir");
        let audit_path = tempdir.path().join("audit").join("events.jsonl");
        let mut config = LoongConfig::default();
        config.audit.mode = AuditMode::Fanout;
        config.audit.path = audit_path.display().to_string();
        config.audit.retain_in_memory = true;

        let context = bootstrap_app_context_with_config("test-agent", 60, &config)
            .expect("bootstrap with fanout audit should succeed");

        assert_eq!(context.agent_id(), "test-agent");

        let journal = fs::read_to_string(&audit_path).expect("audit journal should exist");
        assert_eq!(
            journal.lines().count(),
            1,
            "token bootstrap should emit one audit event"
        );
        assert!(
            journal.contains("\"TokenIssued\"") || journal.contains("\"token_id\""),
            "fanout journal should capture token issuance"
        );
    }

    #[test]
    fn bootstrap_app_context_with_config_grants_network_egress() {
        let mut config = LoongConfig::default();
        config.audit.mode = AuditMode::InMemory;

        let context = bootstrap_app_context_with_config("test-agent", 60, &config)
            .expect("bootstrap with default config should succeed");

        let allowed_capabilities = &context.token().allowed_capabilities;

        assert!(
            allowed_capabilities.contains(&Capability::InvokeTool),
            "bootstrap token should retain invoke tool capability"
        );
        assert!(
            allowed_capabilities.contains(&Capability::NetworkEgress),
            "bootstrap token should grant network egress for context-bound web tools"
        );
    }

    #[test]
    fn invocation_context_updates_policy_caps_without_changing_token() {
        let context = bootstrap_test_app_context("test-agent", 60).expect("bootstrap context");
        let narrowed = BTreeSet::from([Capability::MemoryRead]);

        let execution_context = context
            .for_invocation_with_capabilities(
                narrowed.clone(),
                ExecutionPlane::Memory,
                PlaneTier::Core,
                None,
                context.tool_runtime_config(),
            )
            .expect("narrowed execution context should build");

        assert_eq!(execution_context.allowed_capabilities(), narrowed);
        assert!(
            execution_context
                .token()
                .allowed_capabilities
                .contains(&Capability::InvokeTool),
            "token evidence should keep the originally issued capabilities"
        );
    }

    #[test]
    fn invocation_context_rejects_added_capabilities() {
        let context = bootstrap_test_app_context("test-agent", 60).expect("bootstrap context");
        let widened = BTreeSet::from([Capability::MemoryRead, Capability::ControlRead]);

        let error = match context.for_invocation_with_capabilities(
            widened,
            ExecutionPlane::Memory,
            PlaneTier::Core,
            None,
            context.tool_runtime_config(),
        ) {
            Ok(_) => panic!("execution context must not add capabilities"),
            Err(error) => error,
        };

        assert_eq!(
            error,
            "child execution context cannot add capabilities: missing control_read"
        );
    }

    #[test]
    fn narrow_capabilities_rejects_capabilities_removed_by_parent_context() {
        let context = bootstrap_test_app_context("test-agent", 60).expect("bootstrap context");
        let parent = context
            .for_invocation_with_capabilities(
                BTreeSet::from([Capability::MemoryRead]),
                ExecutionPlane::Memory,
                PlaneTier::Core,
                None,
                context.tool_runtime_config(),
            )
            .expect("parent execution context should build");
        let child_caps = BTreeSet::from([Capability::MemoryRead, Capability::FilesystemRead]);

        let error = match parent.narrow_capabilities(child_caps) {
            Ok(_) => panic!("child context must not regain parent-removed capabilities"),
            Err(error) => error,
        };

        assert_eq!(
            error,
            "child execution context cannot add capabilities: missing filesystem_read"
        );
    }

    #[cfg(feature = "tool-file")]
    #[tokio::test]
    async fn typed_tool_capability_override_rejects_added_capabilities() {
        let context = bootstrap_test_app_context("test-agent", 60).expect("bootstrap context");
        let execution_context = context
            .for_invocation(
                ExecutionPlane::Tool,
                PlaneTier::Core,
                None,
                context.tool_runtime_config(),
            )
            .expect("build execution context");
        let invocation = execution_context
            .tool(ToolPath::from("read"))
            .expect("read should be registered");

        let error = match invocation
            .with_capabilities_override(BTreeSet::from([Capability::FilesystemWrite]))
        {
            Ok(_) => panic!("override must not add capabilities"),
            Err(error) => error,
        };

        assert!(
            error
                .to_string()
                .contains("tool capability override cannot add capabilities"),
            "expected capability override rejection, got: {error}"
        );
    }

    #[cfg(feature = "tool-file")]
    #[tokio::test]
    async fn typed_tool_capability_override_narrows_domain_action_caps() {
        let tempdir = tempdir().expect("tempdir");
        fs::write(tempdir.path().join("notes.txt"), "alpha").expect("write fixture");
        let mut config = LoongConfig::default();
        config.tools.file_root = Some(tempdir.path().display().to_string());
        let context = bootstrap_app_context_with_config("test-agent", 60, &config)
            .expect("bootstrap context");
        let execution_context = context
            .for_invocation(
                ExecutionPlane::Tool,
                PlaneTier::Core,
                None,
                context.tool_runtime_config(),
            )
            .expect("build execution context");
        let invocation = execution_context
            .tool(ToolPath::from("read"))
            .expect("read should be registered");

        let error = invocation
            .with_capabilities_override(BTreeSet::new())
            .expect("empty override is a valid narrowing")
            .invoke(serde_json::json!({ "path": "notes.txt" }))
            .await
            .expect_err("filesystem read should lose FilesystemRead capability");

        assert!(
            error.to_string().contains("FilesystemRead")
                || error.to_string().contains("filesystem_read"),
            "expected filesystem read capability denial, got: {error}"
        );
    }

    #[cfg(feature = "memory-sqlite")]
    #[tokio::test]
    async fn bootstrap_app_context_with_config_ignores_memory_env_overrides() {
        let tempdir = tempdir().expect("tempdir");
        let sqlite_path = tempdir.path().join("memory.sqlite3");

        let mut seeded_runtime = MemoryRuntimeConfig::for_sqlite_path(sqlite_path.clone());
        seeded_runtime.profile = MemoryProfile::WindowPlusSummary;
        seeded_runtime.sliding_window = 2;

        crate::memory::append_turn_direct(
            "kernel-bootstrap-env-session",
            "user",
            "turn 1",
            &seeded_runtime,
        )
        .expect("append turn 1");
        crate::memory::append_turn_direct(
            "kernel-bootstrap-env-session",
            "assistant",
            "turn 2",
            &seeded_runtime,
        )
        .expect("append turn 2");
        crate::memory::append_turn_direct(
            "kernel-bootstrap-env-session",
            "user",
            "turn 3",
            &seeded_runtime,
        )
        .expect("append turn 3");

        let mut env = ScopedEnv::new();
        env.set("LOONG_MEMORY_PROFILE", "window_plus_summary");
        env.set("LOONG_SQLITE_PATH", "/tmp/env-bootstrap-memory.sqlite3");

        let mut config = LoongConfig::default();
        config.audit.mode = AuditMode::InMemory;
        config.memory.profile = MemoryProfile::WindowOnly;
        config.memory.sqlite_path = sqlite_path.display().to_string();
        config.memory.sliding_window = 2;

        let context = bootstrap_app_context_with_config("test-agent", 60, &config)
            .expect("bootstrap with config should succeed");
        let request = crate::memory::build_read_context_request("kernel-bootstrap-env-session");
        let caps = BTreeSet::from([Capability::MemoryRead]);
        let execution_context = context
            .for_invocation(
                ExecutionPlane::Memory,
                PlaneTier::Core,
                None,
                context.tool_runtime_config(),
            )
            .expect("build memory execution context");
        let outcome = context
            .runtime()
            .kernel()
            .execute_memory_core(
                context.pack_id(),
                context.token(),
                &caps,
                None,
                request,
                &execution_context,
            )
            .await
            .expect("read context via kernel");
        let entries = outcome
            .payload
            .get("entries")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();

        assert!(
            entries
                .iter()
                .all(|entry| entry.get("kind") != Some(&serde_json::json!("summary"))),
            "window-only bootstrap should ignore env-driven summary profile"
        );
    }
}
