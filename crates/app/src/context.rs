use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use loong_contracts::CapabilityToken;
use loong_kernel::{
    AuditSink, Capability, Clock, ExecutionRoute, FanoutAuditSink, HarnessKind, InMemoryAuditSink,
    JsonlAuditSink, LoongKernel, StaticPolicyEngine, SystemClock, VerticalPackManifest,
};

use crate::config::{AuditMode, LoongConfig};

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
    pub kernel: Arc<LoongKernel<StaticPolicyEngine>>,
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
/// Registers a default pack manifest with the MVP tool, memory, filesystem,
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
/// This installs the audit sink selected by `config.audit`, registers the MVP
/// pack plus the core tool/memory adapters and policy extensions, and issues a
/// long-lived capability token for `agent_id`.
///
/// The helper intentionally stays below higher-level runtime initialization: it
/// does not export `LOONG_*` environment variables, resolve chat session
/// ids, or prepare channel/conversation state. Callers that need those side
/// effects should compose it with `runtime_env::initialize_runtime_environment`
/// or a surface-specific bootstrap such as
/// `turn_runtime::initialize_cli_turn_runtime`.
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
    let mut kernel = LoongKernel::with_runtime(
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
        kernel
            .register_core_memory_adapter(crate::memory::MvpMemoryAdapter::with_config(mem_config));
        kernel
            .set_default_core_memory_adapter("mvp-memory")
            .map_err(|e| format!("set default memory adapter failed: {e}"))?;
    }

    let tool_rt = crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
    let file_root = tool_rt.file_root.clone();
    kernel.register_core_tool_adapter(crate::tools::MvpToolAdapter::with_config(tool_rt));
    kernel
        .set_default_core_tool_adapter("mvp-tools")
        .map_err(|e| format!("set default tool adapter failed: {e}"))?;

    // Register policy extensions for unified security enforcement.
    let tool_policy_rt =
        crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
    kernel.register_policy_extension(
        crate::tools::shell_policy_ext::ToolPolicyExtension::from_config(&tool_policy_rt),
    );
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
    use std::collections::BTreeSet;
    use std::fs;

    use loong_contracts::Capability;
    use tempfile::tempdir;

    use super::*;
    use crate::config::MemoryProfile;
    use crate::memory::runtime_config::MemoryRuntimeConfig;
    use crate::test_support::ScopedEnv;

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
        let outcome = context
            .kernel
            .execute_memory_core(context.pack_id(), &context.token, &caps, None, request)
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
