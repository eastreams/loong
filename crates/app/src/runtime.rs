//! App-owned Runtime composition.
//!
//! This module is the single place that binds concrete policy, audit, legacy
//! ingress, and typed tool registrations. `Context` only borrows the resulting
//! Runtime and must not know how that long-lived authority root was assembled.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use loong_kernel::access::fs::{
    FsAtomicWriteAllowPolicy, FsContentSearchAllowPolicy, FsCopyFileAllowPolicy,
    FsCreateDirAllAllowPolicy, FsGlobAllowPolicy, FsInspectPathAllowPolicy,
    FsPathAllowedRootsPolicy, FsReadAllowPolicy, FsReadDirAllowPolicy, FsReadFilenameDenyPolicy,
    FsRemoveDirAllAllowPolicy, FsRemoveFileAllowPolicy, FsRenameAllowPolicy,
    FsResolvePathAllowPolicy, FsWriteAllowPolicy,
};
use loong_kernel::access::memory::{
    MemoryAppendTurnAllowPolicy, MemoryCompactAllowPolicy, MemoryReadStageEnvelopeAllowPolicy,
    MemoryReplaceTurnsAllowPolicy, MemoryTranscriptAllowPolicy, MemoryWindowAllowPolicy,
};
use loong_kernel::{
    AuditSink, Capability, Clock, ExecutionRoute, FanoutAuditSink, HarnessKind, InMemoryAuditSink,
    JsonlAuditSink, Kernel, SystemClock, VerticalPackManifest, policy::PolicyPipelineBuilder,
};
use loong_runtime::runtime::Runtime;

use crate::RuntimeContextFactory;
use crate::config::{AuditMode, LoongConfig};

/// Bootstrap the long-lived runtime authority shared by app sessions.
///
/// This constructs the configured kernel and tool plane without issuing a
/// session token or inventing a host-level context. Hosts retain the Runtime
/// beside owned Sessions and borrow both into recursive execution Contexts.
pub fn bootstrap_runtime_with_config(
    config: &LoongConfig,
) -> Result<Arc<Runtime<RuntimeContextFactory>>, String> {
    let tool_rt = crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
    let audit_sink = match config.audit.mode {
        AuditMode::InMemory => Ok(Arc::new(InMemoryAuditSink::default()) as Arc<dyn AuditSink>),
        AuditMode::Jsonl => build_jsonl_audit_sink(config),
        AuditMode::Fanout => {
            let durable = build_jsonl_audit_sink(config)?;
            if !config.audit.retain_in_memory {
                Ok(durable)
            } else {
                Ok(Arc::new(FanoutAuditSink::new(vec![
                    durable,
                    Arc::new(InMemoryAuditSink::default()) as Arc<dyn AuditSink>,
                ])) as Arc<dyn AuditSink>)
            }
        }
    }?;
    bootstrap_runtime_with_audit_sink(audit_sink, config, &tool_rt)
}

/// Construct the durable sink shared by Jsonl and Fanout audit modes.
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

/// Shared construction path for production-selected and test-injected audit sinks.
pub(crate) fn bootstrap_runtime_with_audit_sink(
    audit_sink: Arc<dyn AuditSink>,
    config: &LoongConfig,
    tool_rt: &crate::tools::runtime_config::ToolRuntimeConfig,
) -> Result<Arc<Runtime<RuntimeContextFactory>>, String> {
    let mut policy = PolicyPipelineBuilder::<RuntimeContextFactory>::new_legacy_allow_fallback()
        // Visibility is a non-overridable deny gate. It must run before a
        // consent policy can stop evaluation to request permission.
        .with_pre_policy(crate::tools::plane::ToolVisibilityPolicy)
        .with_pre_policy(
            crate::tools::plane::consent::ToolMutationConsentPolicy::from(&config.tools),
        )
        .with_policy(crate::tools::plane::ToolInvocationAllowPolicy)
        .with_policy(FsResolvePathAllowPolicy::target())
        .with_policy(FsResolvePathAllowPolicy::entry())
        .with_policy(FsPathAllowedRootsPolicy::target())
        .with_policy(FsPathAllowedRootsPolicy::entry())
        .with_policy(MemoryAppendTurnAllowPolicy)
        .with_policy(MemoryWindowAllowPolicy)
        .with_policy(MemoryTranscriptAllowPolicy)
        .with_policy(MemoryReplaceTurnsAllowPolicy)
        .with_policy(MemoryReadStageEnvelopeAllowPolicy)
        .with_policy(MemoryCompactAllowPolicy);
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

    // Pack registration exists only for bearer-backed legacy ingress. Typed
    // Context, ToolInvocation, and Access authorization do not read this pack.
    let pack = VerticalPackManifest {
        pack_id: crate::legacy_kernel::EMBEDDED_RUNTIME_PACK_ID.to_owned(),
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
        .map_err(|error| format!("kernel pack registration failed: {error}"))?;

    let tools = crate::tools::plane::builtin_tool_plane()
        .map_err(|error| format!("builtin tool registration failed: {error}"))?;
    Ok(Arc::new(Runtime::new(kernel, tools)))
}

#[cfg(test)]
mod tests;
