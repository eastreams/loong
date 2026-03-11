use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use loongclaw_contracts::CapabilityToken;
use loongclaw_kernel::{
    AuditSink, Capability, Clock, ExecutionRoute, HarnessKind, InMemoryAuditSink, LoongClawKernel,
    StaticPolicyEngine, SystemClock, VerticalPackManifest,
};

/// Default pack identifier used by MVP entry points.
const MVP_PACK_ID: &str = "dev-automation";

/// Default token TTL (24 hours) for long-running MVP entry points.
pub(crate) const DEFAULT_TOKEN_TTL_S: u64 = 86400;

/// Kernel execution context for policy-gated MVP operations.
///
/// When present, memory and tool operations route through the kernel's
/// capability/policy/audit system instead of direct adapter calls.
///
/// `pack_id` and `agent_id` are accessed via the embedded `CapabilityToken`
/// to avoid data divergence.
pub struct KernelContext {
    pub kernel: Arc<LoongClawKernel<StaticPolicyEngine>>,
    pub token: CapabilityToken,
}

impl KernelContext {
    pub fn pack_id(&self) -> &str {
        &self.token.pack_id
    }

    pub fn agent_id(&self) -> &str {
        &self.token.agent_id
    }
}

/// Bootstrap a minimal kernel suitable for MVP entry points.
///
/// Registers a default pack manifest with `InvokeTool`, `MemoryRead`, and
/// `MemoryWrite` capabilities, then issues a long-lived token for the given
/// `agent_id`.
pub(crate) fn bootstrap_kernel_context(
    agent_id: &str,
    ttl_s: u64,
) -> Result<KernelContext, String> {
    let mut kernel = LoongClawKernel::with_runtime(
        StaticPolicyEngine::default(),
        Arc::new(SystemClock) as Arc<dyn Clock>,
        Arc::new(InMemoryAuditSink::default()) as Arc<dyn AuditSink>,
    );

    let pack = VerticalPackManifest {
        pack_id: MVP_PACK_ID.to_owned(),
        domain: "mvp".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([
            Capability::InvokeTool,
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
        let mem_config = crate::memory::runtime_config::get_memory_runtime_config().clone();
        kernel
            .register_core_memory_adapter(crate::memory::MvpMemoryAdapter::with_config(mem_config));
        kernel
            .set_default_core_memory_adapter("mvp-memory")
            .map_err(|e| format!("set default memory adapter failed: {e}"))?;
    }

    kernel.register_core_tool_adapter(crate::tools::MvpToolAdapter::new());
    kernel
        .set_default_core_tool_adapter("mvp-tools")
        .map_err(|e| format!("set default tool adapter failed: {e}"))?;

    // Register policy extensions for unified security enforcement.
    kernel
        .register_policy_extension(crate::tools::policy_ext::ToolPolicyExtension::default_rules());
    let file_root = crate::tools::runtime_config::get_tool_runtime_config()
        .file_root
        .clone();
    kernel.register_policy_extension(crate::tools::file_policy_ext::FilePolicyExtension::new(
        file_root,
    ));

    let token = kernel
        .issue_token(MVP_PACK_ID, agent_id, ttl_s)
        .map_err(|e| format!("kernel token issue failed: {e}"))?;

    Ok(KernelContext {
        kernel: Arc::new(kernel),
        token,
    })
}
