use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use loong_contracts::{CapabilityToken, ExecutionPlane, PlaneTier};
use loong_core::policy::context::{CapabilityContext, ContextFactory};
use loong_kernel::access::fs::{FsPathPolicyContext, FsResolutionContext};
use loong_kernel::{
    AccessCx, AuditSink, Capability, Clock, ExecutionRoute, FanoutAuditSink, HarnessKind,
    InMemoryAuditSink, JsonlAuditSink, Kernel, KernelAccess, KernelInvocationContext,
    NoopAuditSink, PolicyPipeline, SystemClock, VerticalPackManifest,
    policy::{FsReadAllowPolicy, FsReadFilenameDenyPolicy, FsResolvePathAllowedRootsPolicy},
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
        let (fs_resolution_root, fs_allowed_roots) = fs_access_root_view(tool_runtime_config)?;
        Ok(Self {
            kernel,
            pack,
            token,
            now_epoch_s,
            plane,
            tier,
            request_parameters,
            fs_resolution_root,
            fs_allowed_roots,
        })
    }

    #[must_use]
    pub(crate) fn access(&self) -> AccessCx<'_, 'a, AppContextFactory> {
        // AccessCx construction is localized at the concrete context boundary.
        // Tool/action code should call ctx.access() rather than rethreading the
        // kernel reference or recreating access facades by hand.
        AccessCx::new(self.kernel, self)
    }
}

impl KernelAccess<AppContextFactory> for AppExecutionContext<'_> {
    fn access(&self) -> AccessCx<'_, '_, AppContextFactory> {
        // Concrete tools depend on this narrow requirement instead of the app
        // context type, keeping loong-tools reusable across app/test contexts.
        AccessCx::new(self.kernel, self)
    }
}

impl CapabilityContext for AppExecutionContext<'_> {
    fn allowed_capabilities(&self) -> BTreeSet<Capability> {
        self.token.allowed_capabilities.clone()
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
    let Some(primary_root) = allowed_roots.first().cloned() else {
        return Err("filesystem access requires at least one allowed root".to_owned());
    };
    let resolution_root = config
        .path_resolution_root()
        .map(Path::to_path_buf)
        .unwrap_or(primary_root);
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

pub(crate) fn read_file_with_access_for_runtime_config(
    path: impl AsRef<Path>,
    config: &crate::tools::runtime_config::ToolRuntimeConfig,
) -> Result<(PathBuf, Vec<u8>), String> {
    let path = path.as_ref().to_path_buf();
    let config = config.clone();

    block_on_context_future(
        async move {
            let mut policy = PolicyPipeline::<AppContextFactory>::new_legacy_allow_fallback()
                .with_policy(crate::tools::plane::ToolInvocationAllowPolicy)
                .with_policy(FsResolvePathAllowedRootsPolicy);
            if !config.fs.deny_read_filenames.is_empty() {
                policy.push_policy(FsReadFilenameDenyPolicy::new(
                    config.fs.deny_read_filenames.clone(),
                ));
            }
            policy.push_policy(FsReadAllowPolicy);
            let kernel = Kernel::with_policy_runtime(
                policy,
                Arc::new(SystemClock) as Arc<dyn Clock>,
                Arc::new(NoopAuditSink),
            );
            let now_epoch_s = kernel.now_epoch_s();
            let pack = runtime_file_read_pack_manifest();
            let token = runtime_file_read_token(now_epoch_s);
            let execution_context = AppExecutionContext::new(
                &kernel,
                &pack,
                &token,
                now_epoch_s,
                ExecutionPlane::Tool,
                PlaneTier::Core,
                None,
                &config,
            )?;
            let output = execution_context
                .access()
                .fs()
                .read_file(path)
                .await
                .map_err(|error| {
                    let rendered = error.to_string();
                    if loong_kernel::access::fs_read_error_is_policy_denial(&error) {
                        format!("policy_denied: {rendered}")
                    } else {
                        rendered
                    }
                })?;
            Ok((output.path, output.bytes))
        },
        "access-backed file read",
    )
}

fn runtime_file_read_pack_manifest() -> VerticalPackManifest {
    VerticalPackManifest {
        pack_id: EMBEDDED_RUNTIME_PACK_ID.to_owned(),
        domain: "app-runtime-context".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([Capability::FilesystemRead]),
        metadata: BTreeMap::new(),
    }
}

fn runtime_file_read_token(now_epoch_s: u64) -> CapabilityToken {
    CapabilityToken {
        token_id: "runtime-file-read".to_owned(),
        pack_id: EMBEDDED_RUNTIME_PACK_ID.to_owned(),
        agent_id: "runtime-context".to_owned(),
        allowed_capabilities: BTreeSet::from([Capability::FilesystemRead]),
        issued_at_epoch_s: now_epoch_s,
        expires_at_epoch_s: now_epoch_s,
        generation: 0,
    }
}

fn block_on_context_future<F, T>(future: F, label: &str) -> Result<T, String>
where
    F: Future<Output = Result<T, String>> + Send,
    T: Send,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| handle.block_on(future))
        }
        Ok(_) => thread::scope(|scope| {
            scope
                .spawn(|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| {
                            format!("failed to create tokio runtime for {label}: {error}")
                        })?;
                    runtime.block_on(future)
                })
                .join()
                .map_err(|_panic| format!("{label} worker thread panicked"))?
        }),
        Err(_) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("failed to create tokio runtime for {label}: {error}"))?;
            runtime.block_on(future)
        }
    }
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
        .with_policy(FsResolvePathAllowedRootsPolicy);
    if !tool_rt.fs.deny_read_filenames.is_empty() {
        policy.push_policy(FsReadFilenameDenyPolicy::new(
            tool_rt.fs.deny_read_filenames.clone(),
        ));
    }
    policy.push_policy(FsReadAllowPolicy);
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
