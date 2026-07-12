use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use loong_contracts::{
    CapabilityToken, ExecutionPlane, InvocationOutcome, PlaneTier, ToolPlaneError,
};
use loong_core::policy::context::{CapabilityContext, ContextFactory};
use loong_kernel::access::fs::{FsPathPolicyContext, FsResolutionContext};
use loong_kernel::{
    AccessCx, AuditSink, Capability, Clock, ExecutionRoute, FanoutAuditSink, HarnessKind,
    InMemoryAuditSink, JsonlAuditSink, Kernel, KernelAccess, KernelInvocationContext,
    PolicyPipeline, SystemClock, VerticalPackManifest,
    policy::{
        FsAtomicWriteAllowPolicy, FsContentSearchAllowPolicy, FsCopyFileAllowPolicy,
        FsCreateDirAllAllowPolicy, FsGlobAllowPolicy, FsInspectPathAllowPolicy, FsReadAllowPolicy,
        FsReadDirAllowPolicy, FsReadFilenameDenyPolicy, FsRemoveFileAllowPolicy,
        FsRemoveFileAllowedRootsPolicy, FsResolvePathAllowedRootsPolicy, FsWriteAllowPolicy,
    },
};
use serde_json::Value;

use crate::config::{AuditMode, LoongConfig};

/// Default pack identifier used by embedded runtime entry points.
const EMBEDDED_RUNTIME_PACK_ID: &str = "dev-automation";

/// Default token TTL (24 hours) for long-running embedded runtime entry points.
pub const DEFAULT_TOKEN_TTL_S: u64 = 86400;

/// Kernel execution context for policy-gated embedded runtime operations.
///
/// When present, memory and tool operations route through the kernel's
/// capability/policy/audit system instead of direct adapter calls.
///
/// `pack_id` and `agent_id` are accessed via the embedded `CapabilityToken`
/// to avoid data divergence.
///
/// TODO(deprecate-kernel-context): after the unified session/agent context owns
/// this state, add `#[deprecated]` here and migrate call sites instead of
/// threading new `KernelContext` uses.
#[derive(Clone)]
pub struct KernelContext {
    pub kernel: Arc<Kernel<AppContextFactory>>,
    pub pack: Arc<VerticalPackManifest>,
    pub token: CapabilityToken,
    pub tool_runtime_config: crate::tools::runtime_config::ToolRuntimeConfig,
}

impl KernelContext {
    pub fn pack_id(&self) -> &str {
        &self.token.pack_id
    }

    pub fn agent_id(&self) -> &str {
        &self.token.agent_id
    }

    pub(crate) fn execution_context<'a>(
        &'a self,
        plane: ExecutionPlane,
        tier: PlaneTier,
        request_parameters: Option<&'a Value>,
        tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
    ) -> Result<AppExecutionContext<'a>, String> {
        AppExecutionContext::new(
            self.kernel.as_ref(),
            self.pack.as_ref(),
            &self.token,
            self.kernel.now_epoch_s(),
            plane,
            tier,
            request_parameters,
            tool_runtime_config,
        )
    }

    pub(crate) fn memory_core_execution_context(&self) -> Result<AppExecutionContext<'_>, String> {
        self.execution_context(
            ExecutionPlane::Memory,
            PlaneTier::Core,
            None,
            &self.tool_runtime_config,
        )
    }
}

#[cfg(test)]
pub(crate) fn pack_manifest_from_token(token: &CapabilityToken) -> VerticalPackManifest {
    VerticalPackManifest {
        pack_id: token.pack_id.clone(),
        domain: "app-context".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: token.allowed_capabilities.clone(),
        metadata: BTreeMap::new(),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AppContextFactory;

impl ContextFactory for AppContextFactory {
    type Cx<'a> = AppExecutionContext<'a>;
}

pub struct AppExecutionContext<'a> {
    kernel: &'a Kernel<AppContextFactory>,
    pack: &'a VerticalPackManifest,
    token: &'a CapabilityToken,
    effective_capabilities: BTreeSet<Capability>,
    now_epoch_s: u64,
    plane: ExecutionPlane,
    tier: PlaneTier,
    request_parameters: Option<&'a Value>,
    fs_resolution_root: PathBuf,
    fs_allowed_roots: Vec<PathBuf>,
}

impl<'a> AppExecutionContext<'a> {
    #[must_use]
    pub fn plane(&self) -> ExecutionPlane {
        self.plane
    }

    #[must_use]
    pub fn tier(&self) -> PlaneTier {
        self.tier
    }

    pub(crate) fn new(
        kernel: &'a Kernel<AppContextFactory>,
        pack: &'a VerticalPackManifest,
        token: &'a CapabilityToken,
        now_epoch_s: u64,
        plane: ExecutionPlane,
        tier: PlaneTier,
        request_parameters: Option<&'a Value>,
        tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
    ) -> Result<Self, String> {
        Self::new_with_effective_capabilities(
            kernel,
            pack,
            token,
            token.allowed_capabilities.clone(),
            now_epoch_s,
            plane,
            tier,
            request_parameters,
            tool_runtime_config,
        )
    }

    pub(crate) fn new_with_effective_capabilities(
        kernel: &'a Kernel<AppContextFactory>,
        pack: &'a VerticalPackManifest,
        token: &'a CapabilityToken,
        effective_capabilities: BTreeSet<Capability>,
        now_epoch_s: u64,
        plane: ExecutionPlane,
        tier: PlaneTier,
        request_parameters: Option<&'a Value>,
        tool_runtime_config: &crate::tools::runtime_config::ToolRuntimeConfig,
    ) -> Result<Self, String> {
        if !effective_capabilities.is_subset(&token.allowed_capabilities) {
            return Err("execution context cannot add capabilities beyond token".to_owned());
        }

        let (fs_resolution_root, fs_allowed_roots) = fs_access_root_view(tool_runtime_config)?;
        // Policy reads effective_capabilities so child invocations can narrow
        // authority while KernelInvocationContext still exposes original token
        // evidence for audit.
        Ok(Self {
            kernel,
            pack,
            token,
            effective_capabilities,
            now_epoch_s,
            plane,
            tier,
            request_parameters,
            fs_resolution_root,
            fs_allowed_roots,
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
        Ok(Self {
            kernel: self.kernel,
            pack: self.pack,
            token: self.token,
            effective_capabilities,
            now_epoch_s: self.now_epoch_s,
            plane: self.plane,
            tier: self.tier,
            request_parameters: self.request_parameters,
            fs_resolution_root: self.fs_resolution_root.clone(),
            fs_allowed_roots: self.fs_allowed_roots.clone(),
        })
    }

    #[must_use]
    pub(crate) fn access(&self) -> AccessCx<'_, 'a, AppContextFactory> {
        // AccessCx construction is localized at the concrete context boundary.
        // Tool/action code should call ctx.access() rather than rethreading the
        // kernel reference or recreating access facades by hand.
        AccessCx::new(self.kernel, self)
    }

    pub(crate) fn tool(
        &self,
        path: crate::tools::plane::ToolPath,
    ) -> Result<ToolInvocation<'_, 'a>, ToolPlaneError> {
        let spec = crate::tools::app_tool_plane().spec(&path)?;
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
pub(crate) struct ToolInvocation<'ctx, 'a> {
    ctx: &'ctx AppExecutionContext<'a>,
    path: crate::tools::plane::ToolPath,
    default_capabilities: BTreeSet<Capability>,
    capability_override: Option<BTreeSet<Capability>>,
}

impl ToolInvocation<'_, '_> {
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
        let action = crate::tools::plane::ToolInvocationAction::new(
            self.path,
            required_capabilities,
            payload,
        );
        let grant = self
            .ctx
            .kernel
            .grant_action(
                tool_ctx.pack.pack_id.as_str(),
                tool_ctx.token,
                action,
                &tool_ctx,
            )
            .await?;
        let audit_path = grant.granted.as_ref().path().to_string();
        let audit_caps = grant
            .granted
            .as_ref()
            .required_capabilities()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();

        match crate::tools::app_tool_plane()
            .invoke(grant.granted, &tool_ctx)
            .await
        {
            Ok(output) => {
                tool_ctx.kernel.record_tool_invocation(
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
                tool_ctx.kernel.record_tool_invocation(
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

impl KernelAccess<AppContextFactory> for AppExecutionContext<'_> {
    fn access(&self) -> AccessCx<'_, '_, AppContextFactory> {
        // Concrete tools depend on this narrow requirement instead of the app
        // context type. Delegate to the inherent accessor so this concrete
        // context has one AccessCx construction point.
        AppExecutionContext::access(self)
    }
}

impl CapabilityContext for AppExecutionContext<'_> {
    fn allowed_capabilities(&self) -> BTreeSet<Capability> {
        self.effective_capabilities.clone()
    }
}

impl KernelInvocationContext for AppExecutionContext<'_> {
    fn pack(&self) -> &VerticalPackManifest {
        self.pack
    }

    fn token(&self) -> &CapabilityToken {
        self.token
    }

    fn now_epoch_s(&self) -> u64 {
        self.now_epoch_s
    }

    fn request_parameters(&self) -> Option<&Value> {
        self.request_parameters
    }
}

impl FsResolutionContext for AppExecutionContext<'_> {
    fn fs_resolution_root(&self) -> &Path {
        self.fs_resolution_root.as_path()
    }
}

impl FsPathPolicyContext for AppExecutionContext<'_> {
    fn fs_allowed_roots(&self) -> &[PathBuf] {
        self.fs_allowed_roots.as_slice()
    }
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

/// Bootstrap a minimal in-memory kernel suitable for tests.
///
/// Registers a default pack manifest with the embedded runtime tool, memory, filesystem,
/// and public-web capabilities, then issues a long-lived token for the given
/// `agent_id`.
///
/// Production-facing runtime entrypoints should prefer
/// `bootstrap_kernel_context_with_config` so audit retention follows config.
#[cfg(test)]
pub(crate) fn bootstrap_test_kernel_context(
    agent_id: &str,
    ttl_s: u64,
) -> Result<KernelContext, String> {
    bootstrap_kernel_context_with_audit_sink(
        agent_id,
        ttl_s,
        Arc::new(InMemoryAuditSink::default()) as Arc<dyn AuditSink>,
        &LoongConfig::default(),
    )
}

/// Bootstrap a governed kernel context for production-facing runtime entrypoints.
///
/// This installs the audit sink selected by `config.audit`, registers the embedded runtime
/// pack plus the core tool/memory adapters and policy pipeline, and issues a
/// long-lived capability token for `agent_id`.
///
/// The helper intentionally stays below higher-level runtime initialization: it
/// does not export `LOONG_*` environment variables, resolve chat session
/// ids, or prepare channel/conversation state. Callers that need those side
/// effects should compose it with `runtime_env::initialize_runtime_environment`
/// or a surface-specific bootstrap such as `chat::initialize_cli_turn_runtime`.
pub fn bootstrap_kernel_context_with_config(
    agent_id: &str,
    ttl_s: u64,
    config: &LoongConfig,
) -> Result<KernelContext, String> {
    bootstrap_kernel_context_with_audit_sink(agent_id, ttl_s, build_audit_sink(config)?, config)
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

fn bootstrap_kernel_context_with_audit_sink(
    agent_id: &str,
    ttl_s: u64,
    audit_sink: Arc<dyn AuditSink>,
    config: &LoongConfig,
) -> Result<KernelContext, String> {
    let tool_rt = crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
    let mut policy = PolicyPipeline::<AppContextFactory>::new_legacy_allow_fallback()
        .with_policy(crate::tools::plane::ToolInvocationAllowPolicy)
        .with_policy(FsResolvePathAllowedRootsPolicy)
        .with_policy(FsRemoveFileAllowedRootsPolicy);
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
    let pack = Arc::new(pack);

    kernel
        .register_pack((*pack).clone())
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

    let token = kernel
        .issue_token(EMBEDDED_RUNTIME_PACK_ID, agent_id, ttl_s)
        .map_err(|e| format!("kernel token issue failed: {e}"))?;

    Ok(KernelContext {
        kernel: Arc::new(kernel),
        pack,
        token,
        tool_runtime_config: tool_rt,
    })
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
    fn bootstrap_kernel_context_with_config_writes_jsonl_audit_events() {
        let tempdir = tempdir().expect("tempdir");
        let audit_path = tempdir.path().join("audit").join("events.jsonl");
        let mut config = LoongConfig::default();
        config.audit.mode = AuditMode::Jsonl;
        config.audit.path = audit_path.display().to_string();
        config.audit.retain_in_memory = false;

        let context = bootstrap_kernel_context_with_config("test-agent", 60, &config)
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
    fn bootstrap_kernel_context_with_config_writes_fanout_audit_events() {
        let tempdir = tempdir().expect("tempdir");
        let audit_path = tempdir.path().join("audit").join("events.jsonl");
        let mut config = LoongConfig::default();
        config.audit.mode = AuditMode::Fanout;
        config.audit.path = audit_path.display().to_string();
        config.audit.retain_in_memory = true;

        let context = bootstrap_kernel_context_with_config("test-agent", 60, &config)
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
    fn bootstrap_kernel_context_with_config_grants_network_egress() {
        let mut config = LoongConfig::default();
        config.audit.mode = AuditMode::InMemory;

        let context = bootstrap_kernel_context_with_config("test-agent", 60, &config)
            .expect("bootstrap with default config should succeed");

        let allowed_capabilities = &context.token.allowed_capabilities;

        assert!(
            allowed_capabilities.contains(&Capability::InvokeTool),
            "bootstrap token should retain invoke tool capability"
        );
        assert!(
            allowed_capabilities.contains(&Capability::NetworkEgress),
            "bootstrap token should grant network egress for kernel-bound web tools"
        );
    }

    #[test]
    fn new_with_effective_capabilities_updates_policy_caps_without_changing_token() {
        let context = bootstrap_test_kernel_context("test-agent", 60).expect("bootstrap context");
        let narrowed = BTreeSet::from([Capability::MemoryRead]);

        let execution_context = AppExecutionContext::new_with_effective_capabilities(
            context.kernel.as_ref(),
            context.pack.as_ref(),
            &context.token,
            narrowed.clone(),
            context.kernel.now_epoch_s(),
            ExecutionPlane::Memory,
            PlaneTier::Core,
            None,
            &context.tool_runtime_config,
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
    fn new_with_effective_capabilities_rejects_added_capabilities() {
        let context = bootstrap_test_kernel_context("test-agent", 60).expect("bootstrap context");
        let widened = BTreeSet::from([Capability::MemoryRead, Capability::ControlRead]);

        let error = match AppExecutionContext::new_with_effective_capabilities(
            context.kernel.as_ref(),
            context.pack.as_ref(),
            &context.token,
            widened,
            context.kernel.now_epoch_s(),
            ExecutionPlane::Memory,
            PlaneTier::Core,
            None,
            &context.tool_runtime_config,
        ) {
            Ok(_) => panic!("execution context must not add capabilities"),
            Err(error) => error,
        };

        assert_eq!(
            error,
            "execution context cannot add capabilities beyond token"
        );
    }

    #[test]
    fn narrow_capabilities_rejects_capabilities_removed_by_parent_context() {
        let context = bootstrap_test_kernel_context("test-agent", 60).expect("bootstrap context");
        let parent = AppExecutionContext::new_with_effective_capabilities(
            context.kernel.as_ref(),
            context.pack.as_ref(),
            &context.token,
            BTreeSet::from([Capability::MemoryRead]),
            context.kernel.now_epoch_s(),
            ExecutionPlane::Memory,
            PlaneTier::Core,
            None,
            &context.tool_runtime_config,
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
        let context = bootstrap_test_kernel_context("test-agent", 60).expect("bootstrap context");
        let execution_context = context
            .execution_context(
                ExecutionPlane::Tool,
                PlaneTier::Core,
                None,
                &context.tool_runtime_config,
            )
            .expect("build execution context");
        let invocation = execution_context
            .tool(crate::tools::plane::ToolPath::from("read"))
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
        let context = bootstrap_kernel_context_with_config("test-agent", 60, &config)
            .expect("bootstrap context");
        let execution_context = context
            .execution_context(
                ExecutionPlane::Tool,
                PlaneTier::Core,
                None,
                &context.tool_runtime_config,
            )
            .expect("build execution context");
        let invocation = execution_context
            .tool(crate::tools::plane::ToolPath::from("read"))
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
    async fn bootstrap_kernel_context_with_config_ignores_memory_env_overrides() {
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

        let context = bootstrap_kernel_context_with_config("test-agent", 60, &config)
            .expect("bootstrap with config should succeed");
        let request = crate::memory::build_read_context_request("kernel-bootstrap-env-session");
        let caps = BTreeSet::from([Capability::MemoryRead]);
        let execution_context = context
            .memory_core_execution_context()
            .expect("build memory execution context");
        let outcome = context
            .kernel
            .execute_memory_core(
                context.pack_id(),
                &context.token,
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
