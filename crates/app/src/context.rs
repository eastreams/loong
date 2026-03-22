use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use loongclaw_contracts::CapabilityToken;
use loongclaw_kernel::{
    AuditSink, Capability, Clock, ExecutionRoute, FanoutAuditSink, HarnessKind, InMemoryAuditSink,
    JsonlAuditSink, LoongClawKernel, StaticPolicyEngine, SystemClock, VerticalPackManifest,
};

use crate::config::{AuditMode, LoongClawConfig};

/// Default pack identifier used by MVP entry points.
const MVP_PACK_ID: &str = "dev-automation";

/// Default token TTL (24 hours) for long-running MVP entry points.
pub const DEFAULT_TOKEN_TTL_S: u64 = 86400;

/// Kernel execution context for policy-gated MVP operations.
///
/// When present, memory and tool operations route through the kernel's
/// capability/policy/audit system instead of direct adapter calls.
///
/// `pack_id` and `agent_id` are accessed via the embedded `CapabilityToken`
/// to avoid data divergence.
#[derive(Clone)]
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

/// Bootstrap a minimal in-memory kernel suitable for tests.
///
/// Registers a default pack manifest with `InvokeTool`, `MemoryRead`, and
/// `MemoryWrite` capabilities, then issues a long-lived token for the given
/// `agent_id`.
pub fn bootstrap_kernel_context(agent_id: &str, ttl_s: u64) -> Result<KernelContext, String> {
    bootstrap_kernel_context_with_audit_sink(
        agent_id,
        ttl_s,
        Arc::new(InMemoryAuditSink::default()) as Arc<dyn AuditSink>,
    )
}

/// Bootstrap a minimal in-memory kernel suitable for tests.
///
/// Production-facing runtime entrypoints should prefer
/// `bootstrap_kernel_context_with_config` so audit retention follows config.
#[cfg(test)]
pub(crate) fn bootstrap_test_kernel_context(
    agent_id: &str,
    ttl_s: u64,
) -> Result<KernelContext, String> {
    bootstrap_kernel_context(agent_id, ttl_s)
}

pub(crate) fn bootstrap_kernel_context_with_config(
    agent_id: &str,
    ttl_s: u64,
    config: &LoongClawConfig,
) -> Result<KernelContext, String> {
    bootstrap_kernel_context_with_audit_sink(agent_id, ttl_s, build_audit_sink(config)?)
}

fn build_audit_sink(config: &LoongClawConfig) -> Result<Arc<dyn AuditSink>, String> {
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

fn build_jsonl_audit_sink(config: &LoongClawConfig) -> Result<Arc<dyn AuditSink>, String> {
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
) -> Result<KernelContext, String> {
    let mut kernel = LoongClawKernel::with_runtime(
        StaticPolicyEngine::default(),
        Arc::new(SystemClock) as Arc<dyn Clock>,
        audit_sink,
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
    let tool_rt = crate::tools::runtime_config::get_tool_runtime_config();
    kernel.register_policy_extension(
        crate::tools::shell_policy_ext::ToolPolicyExtension::from_config(tool_rt),
    );
    let file_root = tool_rt.file_root.clone();
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

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn bootstrap_kernel_context_with_config_writes_jsonl_audit_events() {
        let tempdir = tempdir().expect("tempdir");
        let audit_path = tempdir.path().join("audit").join("events.jsonl");
        let mut config = LoongClawConfig::default();
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
        let mut config = LoongClawConfig::default();
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
}
