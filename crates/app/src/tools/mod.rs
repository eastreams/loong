#[cfg(test)]
use std::cell::Cell;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    future::Future,
    path::{Path, PathBuf},
};

use loongclaw_contracts::{Capability, ToolCoreOutcome, ToolCoreRequest};
use serde_json::{Value, json};

use crate::KernelContext;
use crate::config::ToolConfig;
use crate::memory::runtime_config::MemoryRuntimeConfig;

pub(crate) mod approval;
mod catalog;
mod claw_import;
pub(crate) mod delegate;
mod external_skills;
#[cfg(feature = "feishu-integration")]
mod feishu;
mod file;
pub mod file_policy_ext;
mod kernel_adapter;
pub(crate) mod messaging;
mod provider_switch;
pub mod runtime_config;
mod session;
mod shell;
pub mod shell_policy_ext;

pub use catalog::{
    ToolApprovalMode, ToolAvailability, ToolCatalog, ToolDescriptor, ToolExecutionKind,
    ToolGovernanceProfile, ToolGovernanceScope, ToolRiskClass, ToolView,
    delegate_child_tool_view_for_config, delegate_child_tool_view_for_config_with_delegate,
    governance_profile_for_descriptor, governance_profile_for_tool_name,
    planned_delegate_child_tool_view, planned_root_tool_view, runtime_tool_view,
    runtime_tool_view_for_config, tool_catalog,
};
#[cfg(feature = "feishu-integration")]
pub(crate) use feishu::{DeferredFeishuCardUpdate, drain_deferred_feishu_card_updates};
pub use kernel_adapter::MvpToolAdapter;

pub(crate) const LOONGCLAW_INTERNAL_TOOL_CONTEXT_KEY: &str = "_loongclaw";

pub fn normalize_external_skills_domain_rule(raw: &str) -> Result<String, String> {
    external_skills::normalize_domain_rule(raw)
}

tokio::task_local! {
    static TRUSTED_INTERNAL_TOOL_PAYLOAD_TASK: bool;
}

#[cfg(test)]
thread_local! {
    static TRUSTED_INTERNAL_TOOL_PAYLOAD_DEPTH: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn with_trusted_internal_tool_payload<T>(f: impl FnOnce() -> T) -> T {
    struct TrustedInternalToolPayloadGuard;

    impl Drop for TrustedInternalToolPayloadGuard {
        fn drop(&mut self) {
            TRUSTED_INTERNAL_TOOL_PAYLOAD_DEPTH.with(|depth| {
                depth.set(depth.get().saturating_sub(1));
            });
        }
    }

    TRUSTED_INTERNAL_TOOL_PAYLOAD_DEPTH.with(|depth| {
        depth.set(depth.get().saturating_add(1));
    });
    let _guard = TrustedInternalToolPayloadGuard;
    f()
}

pub(crate) async fn with_trusted_internal_tool_payload_async<T>(
    future: impl Future<Output = T>,
) -> T {
    if trusted_internal_tool_payload_enabled() {
        return future.await;
    }

    TRUSTED_INTERNAL_TOOL_PAYLOAD_TASK.scope(true, future).await
}

fn trusted_internal_tool_payload_enabled() -> bool {
    #[cfg(test)]
    let test_enabled = TRUSTED_INTERNAL_TOOL_PAYLOAD_DEPTH.with(|depth| depth.get() > 0);
    #[cfg(not(test))]
    let test_enabled = false;

    test_enabled
        || TRUSTED_INTERNAL_TOOL_PAYLOAD_TASK
            .try_with(|enabled| *enabled)
            .unwrap_or(false)
}

fn payload_uses_reserved_internal_tool_context(payload: &Value) -> bool {
    payload
        .as_object()
        .is_some_and(|body| body.contains_key(LOONGCLAW_INTERNAL_TOOL_CONTEXT_KEY))
}

/// Execute a tool request, routing through the kernel for
/// policy enforcement and audit recording.
///
/// All requests are dispatched via `kernel.execute_tool_core` which
/// enforces `InvokeTool` capability, runs policy extensions, and records
/// audit events.  Callers must always supply a `KernelContext`.
pub async fn execute_tool(
    request: ToolCoreRequest,
    kernel_ctx: &KernelContext,
) -> Result<ToolCoreOutcome, String> {
    execute_kernel_tool_request(kernel_ctx, request, false)
        .await
        .map_err(|e| format!("{e}"))
}

pub(crate) async fn execute_kernel_tool_request(
    ctx: &KernelContext,
    request: ToolCoreRequest,
    trusted_internal_payload: bool,
) -> Result<ToolCoreOutcome, loongclaw_kernel::KernelError> {
    let caps = BTreeSet::from([Capability::InvokeTool]);
    if trusted_internal_payload {
        return with_trusted_internal_tool_payload_async(async move {
            ctx.kernel
                .execute_tool_core(ctx.pack_id(), &ctx.token, &caps, None, request)
                .await
        })
        .await;
    }

    ctx.kernel
        .execute_tool_core(ctx.pack_id(), &ctx.token, &caps, None, request)
        .await
}

pub fn execute_tool_core(request: ToolCoreRequest) -> Result<ToolCoreOutcome, String> {
    execute_tool_core_with_config(request, runtime_config::get_tool_runtime_config())
}

pub fn execute_app_tool_with_config(
    request: ToolCoreRequest,
    current_session_id: &str,
    memory_config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    let canonical_name = canonical_tool_name(request.tool_name.as_str());
    let request = ToolCoreRequest {
        tool_name: canonical_name.to_owned(),
        payload: request.payload,
    };

    match canonical_name {
        "approval_requests_list" | "approval_request_status" | "approval_request_resolve" => {
            approval::execute_approval_tool_with_policies(
                request,
                current_session_id,
                memory_config,
                tool_config,
            )
        }
        "sessions_list" | "sessions_history" | "session_status" | "session_events"
        | "session_archive" | "session_cancel" | "session_recover" => {
            session::execute_session_tool_with_policies(
                request,
                current_session_id,
                memory_config,
                tool_config,
            )
        }
        _ => Err(format!(
            "app_tool_not_found: unknown app tool `{}`",
            request.tool_name
        )),
    }
}

pub async fn wait_for_session_with_config(
    payload: Value,
    current_session_id: &str,
    memory_config: &MemoryRuntimeConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    #[cfg(not(feature = "memory-sqlite"))]
    {
        let _ = (payload, current_session_id, memory_config, tool_config);
        return Err(
            "session tools require sqlite memory support (enable feature `memory-sqlite`)"
                .to_owned(),
        );
    }

    #[cfg(feature = "memory-sqlite")]
    {
        if !tool_config.sessions.enabled {
            return Err("app_tool_disabled: session tools are disabled by config".to_owned());
        }
        session::wait_for_session_tool_with_policies(
            payload,
            current_session_id,
            memory_config,
            tool_config,
        )
        .await
    }
}

/// Normalize a path by resolving `.` and `..` components without filesystem access.
///
/// - `Prefix` and `RootDir` are tracked separately so `..` can never "eat" them.
/// - `..` past the filesystem root (or volume root on Windows) is silently dropped.
/// - Relative paths preserve leading `..` components (e.g. `../../foo` stays as-is).
///
/// All three path-handling modules (`file`, `claw_import`, `file_policy_ext`) use
/// this single implementation to avoid divergence.
pub(super) fn normalize_without_fs(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut parts: Vec<OsString> = Vec::new();
    let mut prefix: Option<OsString> = None;
    let mut has_root = false;

    for component in path.components() {
        match component {
            Component::Prefix(value) => prefix = Some(value.as_os_str().to_owned()),
            Component::RootDir => has_root = true,
            Component::CurDir => {}
            Component::ParentDir => {
                if let Some(last) = parts.last() {
                    if last != ".." {
                        let _ = parts.pop();
                    } else if !has_root {
                        parts.push(OsString::from(".."));
                    }
                } else if !has_root {
                    parts.push(OsString::from(".."));
                }
            }
            Component::Normal(value) => parts.push(value.to_owned()),
        }
    }

    let mut normalized = PathBuf::new();
    if let Some(prefix) = prefix {
        normalized.push(prefix);
    }
    if has_root {
        normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR));
    }
    for part in parts {
        normalized.push(part);
    }
    if normalized.as_os_str().is_empty() {
        if has_root {
            PathBuf::from(std::path::MAIN_SEPARATOR_STR)
        } else {
            PathBuf::from(".")
        }
    } else {
        normalized
    }
}

pub fn canonical_tool_name(raw: &str) -> &str {
    let catalog = tool_catalog();
    if let Some(descriptor) = catalog.resolve(raw) {
        return descriptor.name;
    }
    #[cfg(feature = "feishu-integration")]
    if let Some(canonical) = feishu::canonical_feishu_tool_name(raw) {
        return canonical;
    }
    raw
}

pub fn is_known_tool_name(raw: &str) -> bool {
    if is_known_tool_name_in_view(raw, &runtime_tool_view()) {
        return true;
    }
    #[cfg(feature = "feishu-integration")]
    {
        feishu::is_known_feishu_tool_name(raw)
    }
    #[cfg(not(feature = "feishu-integration"))]
    {
        false
    }
}

pub fn is_known_tool_name_in_view(raw: &str, view: &ToolView) -> bool {
    view.contains(canonical_tool_name(raw))
}

pub(crate) fn runtime_tool_view_from_loongclaw_config(
    config: &crate::config::LoongClawConfig,
) -> ToolView {
    let runtime_config = runtime_config::ToolRuntimeConfig::from_loongclaw_config(config, None);
    runtime_tool_view_with_runtime_config(&config.tools, &runtime_config)
}

pub(crate) fn runtime_tool_view_with_runtime_config(
    tool_config: &crate::config::ToolConfig,
    runtime_config: &runtime_config::ToolRuntimeConfig,
) -> ToolView {
    let catalog = tool_catalog();
    let mut names = runtime_tool_view_for_config(tool_config)
        .iter(&catalog)
        .map(|descriptor| descriptor.name)
        .collect::<Vec<_>>();
    #[cfg(feature = "feishu-integration")]
    if runtime_config.feishu.is_some() {
        names.extend(
            feishu::feishu_tool_registry_entries()
                .into_iter()
                .map(|entry| entry.name),
        );
    }
    ToolView::from_tool_names(names)
}

pub(crate) struct ResolvedToolExecution {
    pub canonical_name: &'static str,
    pub execution_kind: ToolExecutionKind,
}

pub(crate) fn resolve_tool_execution(raw: &str) -> Option<ResolvedToolExecution> {
    let catalog = tool_catalog();
    if let Some(descriptor) = catalog.resolve(raw) {
        return Some(ResolvedToolExecution {
            canonical_name: descriptor.name,
            execution_kind: descriptor.execution_kind,
        });
    }
    #[cfg(feature = "feishu-integration")]
    if let Some(canonical_name) = feishu::canonical_feishu_tool_name(raw) {
        return Some(ResolvedToolExecution {
            canonical_name,
            execution_kind: ToolExecutionKind::Core,
        });
    }
    None
}

pub fn execute_tool_core_with_config(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    if !trusted_internal_tool_payload_enabled()
        && payload_uses_reserved_internal_tool_context(&request.payload)
    {
        return Err(format!(
            "tool `{}` payload.{LOONGCLAW_INTERNAL_TOOL_CONTEXT_KEY} is reserved for trusted internal tool context; retry without that field",
            request.tool_name
        ));
    }
    let canonical_name = canonical_tool_name(request.tool_name.as_str());
    let request = ToolCoreRequest {
        tool_name: canonical_name.to_owned(),
        payload: request.payload,
    };
    match canonical_name {
        "claw.import" => claw_import::execute_claw_import_tool_with_config(request, config),
        "external_skills.inspect" => {
            external_skills::execute_external_skills_inspect_tool_with_config(request, config)
        }
        "external_skills.install" => {
            external_skills::execute_external_skills_install_tool_with_config(request, config)
        }
        "external_skills.invoke" => {
            external_skills::execute_external_skills_invoke_tool_with_config(request, config)
        }
        "external_skills.list" => {
            external_skills::execute_external_skills_list_tool_with_config(request, config)
        }
        "external_skills.policy" => {
            external_skills::execute_external_skills_policy_tool_with_config(request, config)
        }
        "external_skills.fetch" => {
            external_skills::execute_external_skills_fetch_tool_with_config(request, config)
        }
        "external_skills.remove" => {
            external_skills::execute_external_skills_remove_tool_with_config(request, config)
        }
        #[cfg(feature = "feishu-integration")]
        other if feishu::is_known_feishu_tool_name(other) => {
            feishu::execute_feishu_tool_with_config(request, config)
        }
        "shell.exec" => shell::execute_shell_tool_with_config(request, config),
        "file.read" => file::execute_file_read_tool_with_config(request, config),
        "file.write" => file::execute_file_write_tool_with_config(request, config),
        "provider.switch" => {
            provider_switch::execute_provider_switch_tool_with_config(request, config)
        }
        _ => Err(format!(
            "tool_not_found: unknown tool `{}`",
            request.tool_name
        )),
    }
}

/// Tool registry entry for capability snapshot disclosure.
#[derive(Debug, Clone)]
pub struct ToolRegistryEntry {
    pub name: &'static str,
    pub description: &'static str,
}

/// Returns a sorted list of all registered tools, gated by feature flags.
pub fn tool_registry() -> Vec<ToolRegistryEntry> {
    tool_registry_with_config(Some(runtime_config::get_tool_runtime_config()))
}

pub(crate) fn tool_registry_with_config(
    config: Option<&runtime_config::ToolRuntimeConfig>,
) -> Vec<ToolRegistryEntry> {
    let catalog = tool_catalog();
    let mut entries = runtime_tool_view()
        .iter(&catalog)
        .map(|descriptor| ToolRegistryEntry {
            name: descriptor.name,
            description: descriptor.description,
        })
        .collect::<Vec<_>>();
    #[cfg(feature = "feishu-integration")]
    if feishu_runtime_enabled(config) {
        entries.extend(feishu::feishu_tool_registry_entries());
        entries.sort_by_key(|entry| entry.name);
    }
    entries
}

/// Produce a deterministic text block listing available tools,
/// suitable for appending to the system prompt.
pub fn capability_snapshot() -> String {
    capability_snapshot_with_config(runtime_config::get_tool_runtime_config())
}

pub fn capability_snapshot_with_config(config: &runtime_config::ToolRuntimeConfig) -> String {
    capability_snapshot_for_view_with_config(&runtime_tool_view(), config)
}

pub fn capability_snapshot_for_view(view: &ToolView) -> String {
    capability_snapshot_for_view_with_config(view, runtime_config::get_tool_runtime_config())
}

pub(crate) fn capability_snapshot_for_view_with_config(
    view: &ToolView,
    config: &runtime_config::ToolRuntimeConfig,
) -> String {
    let root_view = runtime_tool_view();
    let mut lines = vec!["[available_tools]".to_owned()];
    if view == &root_view {
        for entry in tool_registry_with_config(Some(config)) {
            lines.push(format!("- {}: {}", entry.name, entry.description));
        }
    } else {
        let catalog = tool_catalog();
        for descriptor in view.iter(&catalog) {
            lines.push(format!("- {}: {}", descriptor.name, descriptor.description));
        }
    }
    let catalog = tool_catalog();
    let includes_external_skills = view
        .iter(&catalog)
        .any(|descriptor| descriptor.name.starts_with("external_skills."));
    if includes_external_skills
        && let Ok(skill_lines) = external_skills::installed_skill_snapshot_lines_with_config(config)
        && !skill_lines.is_empty()
    {
        lines.push(String::new());
        lines.push("[available_external_skills]".to_owned());
        lines.extend(skill_lines);
    }
    lines.join("\n")
}

/// Provider request tool schema for function-calling capable models.
///
/// The output shape matches OpenAI-compatible `tools=[{type:function,...}]`.
/// Order is deterministic for stable prompting/tests.
pub fn provider_tool_definitions() -> Vec<Value> {
    provider_tool_definitions_with_config(Some(runtime_config::get_tool_runtime_config()))
}

pub(crate) fn provider_tool_definitions_with_config(
    config: Option<&runtime_config::ToolRuntimeConfig>,
) -> Vec<Value> {
    let catalog = tool_catalog();
    let mut tools = runtime_tool_view()
        .iter(&catalog)
        .map(|descriptor| {
            debug_assert_eq!(descriptor.availability, ToolAvailability::Runtime);
            descriptor.provider_definition()
        })
        .collect::<Vec<_>>();
    #[cfg(feature = "feishu-integration")]
    if feishu_runtime_enabled(config) {
        tools.extend(feishu::feishu_provider_tool_definitions());
        tools.sort_by(|left, right| tool_function_name(left).cmp(tool_function_name(right)));
    }
    tools
}

pub fn try_provider_tool_definitions_for_view(view: &ToolView) -> Result<Vec<Value>, String> {
    let catalog = tool_catalog();
    let mut tools = Vec::new();
    for descriptor in view.iter(&catalog) {
        if descriptor.availability != ToolAvailability::Runtime {
            return Err(format!(
                "tool_not_advertisable: `{}` is still planned and cannot be exposed yet",
                descriptor.name
            ));
        }
        tools.push(descriptor.provider_definition());
    }
    Ok(tools)
}

#[cfg(feature = "feishu-integration")]
fn feishu_runtime_enabled(config: Option<&runtime_config::ToolRuntimeConfig>) -> bool {
    match config {
        Some(config) => config.feishu.is_some(),
        None => runtime_config::get_tool_runtime_config().feishu.is_some(),
    }
}

fn tool_function_name(tool: &Value) -> &str {
    tool.get("function")
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
}

#[allow(dead_code)]
fn _shape_examples() -> BTreeMap<&'static str, Value> {
    let mut shapes = BTreeMap::from([
        (
            "claw.import",
            json!({
                "input_path": "/tmp/nanobot-workspace",
                "mode": "plan",
                "source": "auto"
            }),
        ),
        (
            "shell.exec",
            json!({
                "command": "echo",
                "args": ["hello"]
            }),
        ),
        (
            "external_skills.policy",
            json!({
                "action": "set",
                "policy_update_approved": true,
                "enabled": true,
                "require_download_approval": true,
                "allowed_domains": ["skills.sh"],
                "blocked_domains": ["*.evil.example"]
            }),
        ),
        (
            "external_skills.fetch",
            json!({
                "url": "https://skills.sh/packages/demo-skill.tar.gz",
                "approval_granted": true
            }),
        ),
        (
            "file.read",
            json!({
                "path": "README.md",
                "max_bytes": 4096
            }),
        ),
        (
            "file.write",
            json!({
                "path": "notes.txt",
                "content": "hello",
                "create_dirs": true
            }),
        ),
    ]);
    #[cfg(feature = "feishu-integration")]
    {
        shapes.extend(feishu::feishu_shape_examples());
    }
    shapes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn execute_tool_core_with_test_context(
        request: ToolCoreRequest,
        config: &runtime_config::ToolRuntimeConfig,
    ) -> Result<ToolCoreOutcome, String> {
        if payload_uses_reserved_internal_tool_context(&request.payload) {
            with_trusted_internal_tool_payload(|| {
                super::execute_tool_core_with_config(request, config)
            })
        } else {
            super::execute_tool_core_with_config(request, config)
        }
    }

    #[test]
    fn capability_snapshot_is_deterministic() {
        let snapshot =
            capability_snapshot_with_config(&runtime_config::ToolRuntimeConfig::default());
        assert!(snapshot.starts_with("[available_tools]"));

        let snapshot2 =
            capability_snapshot_with_config(&runtime_config::ToolRuntimeConfig::default());
        assert_eq!(snapshot, snapshot2);
    }

    #[test]
    fn capability_snapshot_can_include_installed_external_skills() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-capability-snapshot-skills");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "skills/demo-skill/SKILL.md",
            "# Demo Skill\n\nUse this skill for explicit verification.\n",
        );

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            config_path: None,
            external_skills: runtime_config::ExternalSkillsRuntimePolicy {
                enabled: true,
                require_download_approval: true,
                allowed_domains: BTreeSet::new(),
                blocked_domains: BTreeSet::new(),
                install_root: None,
                auto_expose_installed: true,
            },
            ..runtime_config::ToolRuntimeConfig::default()
        };
        execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "external_skills.install".to_owned(),
                payload: json!({
                    "path": "skills/demo-skill"
                }),
            },
            &config,
        )
        .expect("install should succeed");

        let snapshot = capability_snapshot_with_config(&config);
        assert!(snapshot.contains("[available_external_skills]"));
        assert!(snapshot.contains(
            "- demo-skill: installed managed external skill; use external_skills.inspect or external_skills.invoke for details"
        ));

        fs::remove_dir_all(&root).ok();
    }

    #[cfg(all(
        feature = "tool-file",
        feature = "tool-shell",
        feature = "memory-sqlite"
    ))]
    #[test]
    fn capability_snapshot_lists_all_tools_when_all_features_enabled() {
        let snapshot =
            capability_snapshot_with_config(&runtime_config::ToolRuntimeConfig::default());
        assert!(snapshot.contains(
            "- approval_request_resolve: Resolve one visible governed tool approval request"
        ));
        assert!(snapshot.contains(
            "- approval_request_status: Inspect full detail for a visible governed tool approval request"
        ));
        assert!(snapshot.contains(
            "- approval_requests_list: List visible governed tool approval requests across the current session scope"
        ));
        assert!(
            snapshot.contains(
                "- claw.import: Import legacy Claw configs into native LoongClaw settings"
            )
        );
        assert!(snapshot.contains("- delegate: Delegate a focused subtask into a child session"));
        assert!(snapshot.contains(
            "- delegate_async: Delegate a focused subtask into a background child session"
        ));
        assert!(snapshot.contains("- external_skills.fetch: Download external skills artifacts with domain policy and approval guards"));
        assert!(snapshot.contains("- external_skills.install: Install a managed external skill from a local directory or archive"));
        assert!(
            snapshot.contains(
                "- external_skills.inspect: Read metadata for an installed external skill"
            )
        );
        assert!(snapshot.contains(
            "- external_skills.invoke: Load an installed external skill into the conversation loop"
        ));
        assert!(snapshot.contains(
            "- external_skills.list: List managed external skills available for invocation"
        ));
        assert!(snapshot.contains("- external_skills.policy: Read/update external skills domain allow/block policy at runtime"));
        assert!(snapshot.contains(
            "- external_skills.remove: Remove an installed external skill from the managed runtime"
        ));
        assert!(snapshot.contains("- file.read: Read file contents"));
        assert!(snapshot.contains("- file.write: Write file contents"));
        assert!(snapshot.contains(
            "- provider.switch: Inspect current provider state or switch the default provider profile for subsequent turns"
        ));
        assert!(snapshot.contains(
            "- session_archive: Archive a visible terminal session from default session listings"
        ));
        assert!(
            snapshot.contains("- session_cancel: Cancel a visible async delegate child session")
        );
        assert!(snapshot.contains("- session_events: Fetch session events for a visible session"));
        assert!(snapshot.contains(
            "- session_recover: Recover an overdue queued async delegate child session by marking it failed"
        ));
        assert!(
            snapshot.contains("- session_status: Inspect the current status of a visible session")
        );
        assert!(
            snapshot
                .contains("- session_wait: Wait for a visible session to reach a terminal state")
        );
        assert!(
            snapshot.contains("- sessions_history: Fetch transcript history for a visible session")
        );
        assert!(
            snapshot.contains("- sessions_list: List visible sessions and their high-level state")
        );
        assert!(snapshot.contains("- shell.exec: Execute shell commands"));

        // Verify sorted order matches the catalog snapshot emitted to providers.
        let lines: Vec<&str> = snapshot.lines().skip(1).collect();
        let ordered_prefixes = vec![
            "- approval_request_resolve",
            "- approval_request_status",
            "- approval_requests_list",
            "- claw.import",
            "- delegate",
            "- delegate_async",
            "- external_skills.fetch",
            "- external_skills.inspect",
            "- external_skills.install",
            "- external_skills.invoke",
            "- external_skills.list",
            "- external_skills.policy",
            "- external_skills.remove",
            "- file.read",
            "- file.write",
            "- provider.switch",
            "- session_archive",
            "- session_cancel",
            "- session_events",
            "- session_recover",
            "- session_status",
            "- session_wait",
            "- sessions_history",
            "- sessions_list",
            "- shell.exec",
        ];
        assert_eq!(lines.len(), ordered_prefixes.len());
        for (line, prefix) in lines.iter().zip(ordered_prefixes) {
            assert!(
                line.starts_with(prefix),
                "expected `{prefix}`, got `{line}`"
            );
        }
    }

    #[cfg(all(
        feature = "tool-file",
        feature = "tool-shell",
        feature = "memory-sqlite"
    ))]
    #[test]
    fn tool_registry_returns_all_known_tools() {
        let entries = tool_registry();
        assert_eq!(entries.len(), 25);
        let names: Vec<&str> = entries.iter().map(|e| e.name).collect();
        assert!(names.contains(&"approval_request_resolve"));
        assert!(names.contains(&"approval_request_status"));
        assert!(names.contains(&"approval_requests_list"));
        assert!(names.contains(&"claw.import"));
        assert!(names.contains(&"delegate"));
        assert!(names.contains(&"delegate_async"));
        assert!(names.contains(&"external_skills.fetch"));
        assert!(names.contains(&"external_skills.install"));
        assert!(names.contains(&"external_skills.inspect"));
        assert!(names.contains(&"external_skills.invoke"));
        assert!(names.contains(&"external_skills.list"));
        assert!(names.contains(&"external_skills.policy"));
        assert!(names.contains(&"external_skills.remove"));
        assert!(names.contains(&"shell.exec"));
        assert!(names.contains(&"file.read"));
        assert!(names.contains(&"file.write"));
        assert!(names.contains(&"provider.switch"));
        assert!(names.contains(&"session_archive"));
        assert!(names.contains(&"session_cancel"));
        assert!(names.contains(&"session_events"));
        assert!(names.contains(&"session_recover"));
        assert!(names.contains(&"session_status"));
        assert!(names.contains(&"session_wait"));
        assert!(names.contains(&"sessions_history"));
        assert!(names.contains(&"sessions_list"));
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn capability_snapshot_for_view_only_lists_selected_tools() {
        let view = ToolView::from_tool_names(["claw.import", "shell.exec"]);
        let snapshot = capability_snapshot_for_view(&view);

        assert!(snapshot.contains("- claw.import:"));
        assert!(snapshot.contains("- shell.exec:"));
        assert!(!snapshot.contains("- file.read:"));
        assert!(!snapshot.contains("- external_skills.list:"));
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn try_provider_tool_definitions_for_view_returns_sorted_subset() {
        let view = ToolView::from_tool_names(["shell.exec", "claw.import"]);
        let defs = try_provider_tool_definitions_for_view(&view)
            .expect("restricted runtime view should be advertisable");
        let names: Vec<&str> = defs
            .iter()
            .filter_map(|item| item.get("function"))
            .filter_map(|function| function.get("name"))
            .filter_map(Value::as_str)
            .collect();

        assert_eq!(names, vec!["claw_import", "shell_exec"]);
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn planned_root_tool_view_is_advertisable_when_all_tools_are_runtime_ready() {
        let defs = try_provider_tool_definitions_for_view(&planned_root_tool_view())
            .expect("all tools should now be advertisable");

        assert_eq!(defs.len(), 26);
    }

    #[cfg(feature = "memory-sqlite")]
    #[test]
    fn runtime_tool_view_includes_runtime_session_tools_but_hides_planned_ones() {
        let view = runtime_tool_view_for_config(&crate::config::ToolConfig::default());

        for tool_name in [
            "approval_request_resolve",
            "approval_request_status",
            "approval_requests_list",
            "delegate",
            "delegate_async",
            "session_archive",
            "session_cancel",
            "session_events",
            "session_recover",
            "session_status",
            "session_wait",
            "sessions_history",
            "sessions_list",
        ] {
            assert!(
                view.contains(tool_name),
                "expected runtime view to include `{tool_name}`"
            );
        }

        let tool_name = "sessions_send";
        assert!(
            !view.contains(tool_name),
            "expected runtime view to keep `{tool_name}` hidden"
        );
    }

    #[test]
    fn runtime_tool_view_exposes_delegate_tools_with_depth_budget_only() {
        let config = crate::config::ToolConfig::default();

        let root_view = runtime_tool_view_for_config(&config);
        assert!(root_view.contains("delegate"));
        assert!(root_view.contains("delegate_async"));

        let child_view = delegate_child_tool_view_for_config(&config);
        assert!(!child_view.contains("delegate"));
        assert!(!child_view.contains("delegate_async"));

        let depth_budgeted_child = delegate_child_tool_view_for_config_with_delegate(&config, true);
        assert!(depth_budgeted_child.contains("delegate"));
        assert!(depth_budgeted_child.contains("delegate_async"));
    }

    #[test]
    fn runtime_tool_view_exposes_sessions_send_only_when_messages_enabled() {
        let default_root_view = runtime_tool_view_for_config(&crate::config::ToolConfig::default());
        assert!(!default_root_view.contains("sessions_send"));

        let mut config = crate::config::ToolConfig::default();
        config.messages.enabled = true;

        let root_view = runtime_tool_view_for_config(&config);
        assert!(root_view.contains("sessions_send"));

        let child_view = delegate_child_tool_view_for_config(&config);
        assert!(!child_view.contains("sessions_send"));
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn delegate_child_tool_view_hides_shell_by_default() {
        let view = delegate_child_tool_view_for_config(&crate::config::ToolConfig::default());

        assert!(view.contains("file.read"));
        assert!(view.contains("file.write"));
        assert!(!view.contains("shell.exec"));
    }

    #[cfg(all(feature = "tool-file", feature = "tool-shell"))]
    #[test]
    fn delegate_child_tool_view_can_allow_shell_when_enabled() {
        let mut config = crate::config::ToolConfig::default();
        config.delegate.allow_shell_in_child = true;

        let view = delegate_child_tool_view_for_config(&config);

        assert!(view.contains("file.read"));
        assert!(view.contains("file.write"));
        assert!(view.contains("shell.exec"));
    }

    #[cfg(all(
        feature = "tool-file",
        feature = "tool-shell",
        feature = "memory-sqlite"
    ))]
    #[test]
    fn provider_tool_definitions_are_stable_and_complete() {
        let defs = provider_tool_definitions();
        assert_eq!(defs.len(), 25);

        let names: Vec<&str> = defs
            .iter()
            .filter_map(|item| item.get("function"))
            .filter_map(|function| function.get("name"))
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(
            names,
            vec![
                "approval_request_resolve",
                "approval_request_status",
                "approval_requests_list",
                "claw_import",
                "delegate",
                "delegate_async",
                "external_skills_fetch",
                "external_skills_inspect",
                "external_skills_install",
                "external_skills_invoke",
                "external_skills_list",
                "external_skills_policy",
                "external_skills_remove",
                "file_read",
                "file_write",
                "provider_switch",
                "session_archive",
                "session_cancel",
                "session_events",
                "session_recover",
                "session_status",
                "session_wait",
                "sessions_history",
                "sessions_list",
                "shell_exec"
            ]
        );

        for item in &defs {
            assert_eq!(item["type"], "function");
            assert_eq!(item["function"]["parameters"]["type"], "object");
        }

        let claw_import = defs
            .iter()
            .find(|item| {
                item.get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
                    == Some("claw_import")
            })
            .expect("claw_import definition should exist");
        let mode_enum: Vec<&str> =
            claw_import["function"]["parameters"]["properties"]["mode"]["enum"]
                .as_array()
                .expect("mode enum should be an array")
                .iter()
                .filter_map(Value::as_str)
                .collect();
        assert_eq!(
            mode_enum,
            vec![
                "plan",
                "apply",
                "discover",
                "plan_many",
                "recommend_primary",
                "merge_profiles",
                "map_external_skills",
                "apply_selected",
                "rollback_last_apply"
            ]
        );
        assert!(
            claw_import["function"]["parameters"]["required"]
                .as_array()
                .expect("required should be an array")
                .is_empty()
        );
    }

    #[test]
    fn provider_tool_definitions_include_delegate_when_enabled() {
        let defs = try_provider_tool_definitions_for_view(&runtime_tool_view_for_config(
            &crate::config::ToolConfig::default(),
        ))
        .expect("runtime-visible tool schemas");
        let delegate = defs
            .iter()
            .find(|item| item["function"]["name"] == "delegate")
            .expect("delegate definition");
        let properties = delegate["function"]["parameters"]["properties"]
            .as_object()
            .expect("delegate properties");
        assert!(properties.contains_key("task"));
        assert!(properties.contains_key("label"));
        assert!(properties.contains_key("timeout_seconds"));
    }

    #[test]
    fn provider_tool_definitions_include_delegate_async_when_enabled() {
        let defs = try_provider_tool_definitions_for_view(&runtime_tool_view_for_config(
            &crate::config::ToolConfig::default(),
        ))
        .expect("runtime-visible tool schemas");
        let delegate_async = defs
            .iter()
            .find(|item| item["function"]["name"] == "delegate_async")
            .expect("delegate_async definition");
        let properties = delegate_async["function"]["parameters"]["properties"]
            .as_object()
            .expect("delegate_async properties");
        assert!(properties.contains_key("task"));
        assert!(properties.contains_key("label"));
        assert!(properties.contains_key("timeout_seconds"));
    }

    #[test]
    fn provider_tool_definitions_include_approval_request_resolve_when_enabled() {
        let defs = try_provider_tool_definitions_for_view(&runtime_tool_view_for_config(
            &crate::config::ToolConfig::default(),
        ))
        .expect("runtime-visible tool schemas");
        let approval_request_resolve = defs
            .iter()
            .find(|item| item["function"]["name"] == "approval_request_resolve")
            .expect("approval_request_resolve definition");
        let properties = approval_request_resolve["function"]["parameters"]["properties"]
            .as_object()
            .expect("approval_request_resolve properties");
        assert!(properties.contains_key("approval_request_id"));
        assert!(properties.contains_key("decision"));
    }

    #[test]
    fn provider_tool_definitions_include_sessions_send_when_enabled() {
        let mut config = crate::config::ToolConfig::default();
        config.messages.enabled = true;

        let defs = try_provider_tool_definitions_for_view(&runtime_tool_view_for_config(&config))
            .expect("runtime-visible tool schemas");
        let sessions_send = defs
            .iter()
            .find(|item| item["function"]["name"] == "sessions_send")
            .expect("sessions_send definition");
        let properties = sessions_send["function"]["parameters"]["properties"]
            .as_object()
            .expect("sessions_send properties");
        assert!(properties.contains_key("session_id"));
        assert!(properties.contains_key("text"));
    }

    #[test]
    fn canonical_tool_name_maps_known_aliases() {
        assert_eq!(canonical_tool_name("claw_import"), "claw.import");
        assert_eq!(
            canonical_tool_name("external_skills_policy"),
            "external_skills.policy"
        );
        assert_eq!(
            canonical_tool_name("external_skills_fetch"),
            "external_skills.fetch"
        );
        assert_eq!(canonical_tool_name("file_read"), "file.read");
        assert_eq!(canonical_tool_name("file_write"), "file.write");
        assert_eq!(canonical_tool_name("provider_switch"), "provider.switch");
        assert_eq!(canonical_tool_name("shell_exec"), "shell.exec");
        assert_eq!(canonical_tool_name("shell"), "shell.exec");
        assert_eq!(canonical_tool_name("feishu_whoami"), "feishu.whoami");
        assert_eq!(
            canonical_tool_name("feishu_doc_create"),
            "feishu.doc.create"
        );
        assert_eq!(
            canonical_tool_name("feishu_doc_append"),
            "feishu.doc.append"
        );
        assert_eq!(canonical_tool_name("feishu_doc_read"), "feishu.doc.read");
        assert_eq!(
            canonical_tool_name("feishu_messages_history"),
            "feishu.messages.history"
        );
        assert_eq!(
            canonical_tool_name("feishu_messages_get"),
            "feishu.messages.get"
        );
        assert_eq!(
            canonical_tool_name("feishu_messages_resource_get"),
            "feishu.messages.resource.get"
        );
        assert_eq!(
            canonical_tool_name("feishu_messages_search"),
            "feishu.messages.search"
        );
        assert_eq!(
            canonical_tool_name("feishu_messages_send"),
            "feishu.messages.send"
        );
        assert_eq!(
            canonical_tool_name("feishu_messages_reply"),
            "feishu.messages.reply"
        );
        assert_eq!(
            canonical_tool_name("feishu_calendar_list"),
            "feishu.calendar.list"
        );
        assert_eq!(
            canonical_tool_name("feishu_calendar_freebusy"),
            "feishu.calendar.freebusy"
        );
        assert_eq!(canonical_tool_name("file.read"), "file.read");
    }

    #[test]
    fn is_known_tool_name_accepts_canonical_and_alias_forms() {
        assert!(is_known_tool_name("claw.import"));
        assert!(is_known_tool_name("claw_import"));
        assert!(is_known_tool_name("external_skills.policy"));
        assert!(is_known_tool_name("external_skills_policy"));
        assert!(is_known_tool_name("external_skills.fetch"));
        assert!(is_known_tool_name("external_skills_fetch"));
        assert!(is_known_tool_name("file.read"));
        assert!(is_known_tool_name("file_read"));
        assert!(is_known_tool_name("file.write"));
        assert!(is_known_tool_name("file_write"));
        assert!(is_known_tool_name("provider.switch"));
        assert!(is_known_tool_name("provider_switch"));
        assert!(is_known_tool_name("shell.exec"));
        assert!(is_known_tool_name("shell_exec"));
        assert!(is_known_tool_name("shell"));
        assert!(is_known_tool_name("feishu.whoami"));
        assert!(is_known_tool_name("feishu_whoami"));
        assert!(is_known_tool_name("feishu.doc.create"));
        assert!(is_known_tool_name("feishu_doc_create"));
        assert!(is_known_tool_name("feishu.doc.append"));
        assert!(is_known_tool_name("feishu_doc_append"));
        assert!(is_known_tool_name("feishu.doc.read"));
        assert!(is_known_tool_name("feishu_doc_read"));
        assert!(is_known_tool_name("feishu.messages.history"));
        assert!(is_known_tool_name("feishu_messages_history"));
        assert!(is_known_tool_name("feishu.messages.get"));
        assert!(is_known_tool_name("feishu_messages_get"));
        assert!(is_known_tool_name("feishu.messages.resource.get"));
        assert!(is_known_tool_name("feishu_messages_resource_get"));
        assert!(is_known_tool_name("feishu.messages.search"));
        assert!(is_known_tool_name("feishu_messages_search"));
        assert!(is_known_tool_name("feishu.messages.send"));
        assert!(is_known_tool_name("feishu_messages_send"));
        assert!(is_known_tool_name("feishu.messages.reply"));
        assert!(is_known_tool_name("feishu_messages_reply"));
        assert!(is_known_tool_name("feishu.card.update"));
        assert!(is_known_tool_name("feishu_card_update"));
        assert!(is_known_tool_name("feishu.calendar.list"));
        assert!(is_known_tool_name("feishu_calendar_list"));
        assert!(is_known_tool_name("feishu.calendar.freebusy"));
        assert!(is_known_tool_name("feishu_calendar_freebusy"));
        assert!(!is_known_tool_name("nonexistent.tool"));
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn tool_registry_with_config_includes_feishu_tools_when_runtime_configured() {
        let mut config = runtime_config::ToolRuntimeConfig::default();
        config.feishu = Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                app_id: Some("cli_a1b2c3".to_owned()),
                app_secret: Some("app-secret".to_owned()),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig::default(),
        });

        let entries = tool_registry_with_config(Some(&config));
        let names = entries.iter().map(|entry| entry.name).collect::<Vec<_>>();

        assert!(names.contains(&"feishu.whoami"));
        assert!(names.contains(&"feishu.doc.create"));
        assert!(names.contains(&"feishu.doc.append"));
        assert!(names.contains(&"feishu.doc.read"));
        assert!(names.contains(&"feishu.messages.history"));
        assert!(names.contains(&"feishu.messages.get"));
        assert!(names.contains(&"feishu.messages.resource.get"));
        assert!(names.contains(&"feishu.messages.search"));
        assert!(names.contains(&"feishu.messages.send"));
        assert!(names.contains(&"feishu.messages.reply"));
        assert!(names.contains(&"feishu.card.update"));
        assert!(names.contains(&"feishu.calendar.list"));
        assert!(names.contains(&"feishu.calendar.freebusy"));
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn provider_tool_definitions_with_config_includes_feishu_functions_when_runtime_configured() {
        let mut config = runtime_config::ToolRuntimeConfig::default();
        config.feishu = Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                app_id: Some("cli_a1b2c3".to_owned()),
                app_secret: Some("app-secret".to_owned()),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig::default(),
        });

        let defs = provider_tool_definitions_with_config(Some(&config));
        let names = defs
            .iter()
            .filter_map(|item| item.get("function"))
            .filter_map(|function| function.get("name"))
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();

        assert!(names.contains(&"feishu_whoami"));
        assert!(names.contains(&"feishu_doc_create"));
        assert!(names.contains(&"feishu_doc_append"));
        assert!(names.contains(&"feishu_doc_read"));
        assert!(names.contains(&"feishu_messages_history"));
        assert!(names.contains(&"feishu_messages_get"));
        assert!(names.contains(&"feishu_messages_resource_get"));
        assert!(names.contains(&"feishu_messages_search"));
        assert!(names.contains(&"feishu_messages_send"));
        assert!(names.contains(&"feishu_messages_reply"));
        assert!(names.contains(&"feishu_card_update"));
        assert!(names.contains(&"feishu_calendar_list"));
        assert!(names.contains(&"feishu_calendar_freebusy"));
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn feishu_tool_metadata_catalog_is_self_consistent() {
        let aliases = feishu::feishu_tool_alias_pairs();
        let registry = feishu::feishu_tool_registry_entries();
        let defs = feishu::feishu_provider_tool_definitions();

        let registry_names = registry
            .iter()
            .map(|entry| entry.name)
            .collect::<BTreeSet<_>>();
        let definition_names = defs
            .iter()
            .filter_map(|item| item.get("function"))
            .filter_map(|function| function.get("name"))
            .filter_map(Value::as_str)
            .map(canonical_tool_name)
            .collect::<BTreeSet<_>>();

        assert_eq!(registry_names, definition_names);
        for (alias, canonical) in aliases {
            assert_eq!(canonical_tool_name(alias), *canonical);
            assert!(feishu::is_known_feishu_tool_name(alias));
            assert!(feishu::is_known_feishu_tool_name(canonical));
        }
        for name in registry_names {
            assert!(feishu::is_known_feishu_tool_name(name));
        }
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn feishu_shape_examples_reference_only_known_feishu_tools() {
        let shapes = feishu::feishu_shape_examples();
        let names = shapes.keys().copied().collect::<BTreeSet<_>>();

        assert!(names.contains("feishu.whoami"));
        assert!(names.contains("feishu.doc.create"));
        assert!(names.contains("feishu.doc.append"));
        assert!(names.contains("feishu.doc.read"));
        assert!(names.contains("feishu.messages.get"));
        assert!(names.contains("feishu.messages.history"));
        assert!(names.contains("feishu.messages.search"));
        assert!(names.contains("feishu.card.update"));
        assert!(names.contains("feishu.calendar.list"));
        assert!(names.contains("feishu.calendar.freebusy"));
        #[cfg(feature = "tool-file")]
        assert!(names.contains("feishu.messages.resource.get"));

        for name in names {
            assert!(feishu::is_known_feishu_tool_name(name));
            assert!(shapes.get(name).and_then(Value::as_object).is_some());
        }
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn provider_tool_definitions_with_config_advertises_feishu_message_write_uuid() {
        let mut config = runtime_config::ToolRuntimeConfig::default();
        config.feishu = Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                app_id: Some("cli_a1b2c3".to_owned()),
                app_secret: Some("app-secret".to_owned()),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig::default(),
        });

        let defs = provider_tool_definitions_with_config(Some(&config));
        let send = defs
            .iter()
            .find(|item| item["function"]["name"] == "feishu_messages_send")
            .expect("send definition should exist");
        let reply = defs
            .iter()
            .find(|item| item["function"]["name"] == "feishu_messages_reply")
            .expect("reply definition should exist");

        assert_eq!(
            send["function"]["parameters"]["properties"]["uuid"]["type"],
            "string"
        );
        assert_eq!(
            reply["function"]["parameters"]["properties"]["uuid"]["type"],
            "string"
        );
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn provider_tool_definitions_with_config_advertises_feishu_message_post_and_media_payloads() {
        let mut config = runtime_config::ToolRuntimeConfig::default();
        config.feishu = Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                app_id: Some("cli_a1b2c3".to_owned()),
                app_secret: Some("app-secret".to_owned()),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig::default(),
        });

        let defs = provider_tool_definitions_with_config(Some(&config));
        let send = defs
            .iter()
            .find(|item| item["function"]["name"] == "feishu_messages_send")
            .expect("send definition should exist");
        let reply = defs
            .iter()
            .find(|item| item["function"]["name"] == "feishu_messages_reply")
            .expect("reply definition should exist");
        let card_update = defs
            .iter()
            .find(|item| item["function"]["name"] == "feishu_card_update")
            .expect("card update definition should exist");

        assert!(
            send["function"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("post")
                    && description.contains("image")
                    && description.contains("file"))
        );
        assert!(
            reply["function"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("post")
                    && description.contains("image")
                    && description.contains("file"))
        );
        assert_eq!(
            send["function"]["parameters"]["properties"]["post"]["type"],
            "object"
        );
        assert_eq!(
            reply["function"]["parameters"]["properties"]["post"]["type"],
            "object"
        );
        assert_eq!(
            send["function"]["parameters"]["properties"]["image_key"]["type"],
            "string"
        );
        assert_eq!(
            send["function"]["parameters"]["properties"]["file_key"]["type"],
            "string"
        );
        assert_eq!(
            reply["function"]["parameters"]["properties"]["image_key"]["type"],
            "string"
        );
        assert_eq!(
            reply["function"]["parameters"]["properties"]["file_key"]["type"],
            "string"
        );
        #[cfg(feature = "tool-file")]
        {
            assert_eq!(
                send["function"]["parameters"]["properties"]["image_path"]["type"],
                "string"
            );
            assert_eq!(
                send["function"]["parameters"]["properties"]["file_path"]["type"],
                "string"
            );
            assert_eq!(
                send["function"]["parameters"]["properties"]["file_type"]["type"],
                "string"
            );
            assert_eq!(
                reply["function"]["parameters"]["properties"]["image_path"]["type"],
                "string"
            );
            assert_eq!(
                reply["function"]["parameters"]["properties"]["file_path"]["type"],
                "string"
            );
            assert_eq!(
                reply["function"]["parameters"]["properties"]["file_type"]["type"],
                "string"
            );
        }
        assert_eq!(send["function"]["parameters"]["required"], json!([]));
        assert_eq!(reply["function"]["parameters"]["required"], json!([]));
        assert_eq!(
            card_update["function"]["parameters"]["properties"]["card"]["type"],
            "object"
        );
        assert_eq!(
            card_update["function"]["parameters"]["properties"]["markdown"]["type"],
            "string"
        );
        assert_eq!(card_update["function"]["parameters"]["required"], json!([]));
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn provider_tool_definitions_with_config_advertises_feishu_ingress_defaults() {
        let mut config = runtime_config::ToolRuntimeConfig::default();
        config.feishu = Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                app_id: Some("cli_a1b2c3".to_owned()),
                app_secret: Some("app-secret".to_owned()),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig::default(),
        });

        let defs = provider_tool_definitions_with_config(Some(&config));
        let send = defs
            .iter()
            .find(|item| item["function"]["name"] == "feishu_messages_send")
            .expect("send definition should exist");
        let doc_create = defs
            .iter()
            .find(|item| item["function"]["name"] == "feishu_doc_create")
            .expect("doc create definition should exist");
        let doc_append = defs
            .iter()
            .find(|item| item["function"]["name"] == "feishu_doc_append")
            .expect("doc append definition should exist");
        let reply = defs
            .iter()
            .find(|item| item["function"]["name"] == "feishu_messages_reply")
            .expect("reply definition should exist");
        let get = defs
            .iter()
            .find(|item| item["function"]["name"] == "feishu_messages_get")
            .expect("get definition should exist");
        let resource_get = defs
            .iter()
            .find(|item| item["function"]["name"] == "feishu_messages_resource_get")
            .expect("resource get definition should exist");
        let history = defs
            .iter()
            .find(|item| item["function"]["name"] == "feishu_messages_history")
            .expect("history definition should exist");

        assert!(
            send["function"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("current Feishu conversation"))
        );
        assert!(
            doc_create["function"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("markdown or html"))
        );
        assert!(
            doc_append["function"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("existing Feishu document"))
        );
        assert_eq!(
            doc_create["function"]["parameters"]["properties"]["content_type"]["type"],
            "string"
        );
        assert_eq!(
            doc_create["function"]["parameters"]["properties"]["content_type"]["enum"],
            json!(["markdown", "html"])
        );
        assert_eq!(
            doc_create["function"]["parameters"]["properties"]["content"]["type"],
            "string"
        );
        #[cfg(feature = "tool-file")]
        assert_eq!(
            doc_create["function"]["parameters"]["properties"]["content_path"]["type"],
            "string"
        );
        assert_eq!(
            doc_append["function"]["parameters"]["properties"]["url"]["type"],
            "string"
        );
        assert_eq!(
            doc_append["function"]["parameters"]["properties"]["content"]["type"],
            "string"
        );
        assert_eq!(
            doc_append["function"]["parameters"]["properties"]["content_type"]["enum"],
            json!(["markdown", "html"])
        );
        #[cfg(feature = "tool-file")]
        assert_eq!(
            doc_append["function"]["parameters"]["properties"]["content_path"]["type"],
            "string"
        );
        #[cfg(feature = "tool-file")]
        assert_eq!(
            doc_append["function"]["parameters"]["required"],
            json!(["url"])
        );
        #[cfg(feature = "tool-file")]
        assert_eq!(
            doc_append["function"]["parameters"]["anyOf"],
            json!([
                { "required": ["content"] },
                { "required": ["content_path"] }
            ])
        );
        #[cfg(not(feature = "tool-file"))]
        assert_eq!(
            doc_append["function"]["parameters"]["required"],
            json!(["url", "content"])
        );
        assert!(
            reply["function"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("source Feishu message"))
        );
        assert!(
            get["function"]["parameters"]["properties"]["message_id"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("current Feishu ingress"))
        );
        assert!(
            resource_get["function"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("download")
                    && description.contains("configured file root"))
        );
        assert!(
            resource_get["function"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("resource_inventory")
                    && description.contains("uniquely identifies")
                    && description.contains(
                        "payload.message_id is omitted or matches the current ingress message"
                    )
                    && description.contains("Outside the current ingress turn")
                    && description.contains("source message_id"))
        );
        assert!(
            resource_get["function"]["parameters"]["properties"]["message_id"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("current Feishu ingress")
                    && description.contains("Outside the current ingress turn"))
        );
        assert!(
            resource_get["function"]["parameters"]["properties"]["file_key"]["description"]
                .as_str()
                .is_some_and(|description| {
                    description.contains("exactly one Feishu message resource")
                        && description.contains("payload.type uniquely selects")
                        && description.contains("same message")
                        && description.contains("resource_inventory")
                        && description.contains("multiple resources")
                })
        );
        assert_eq!(
            resource_get["function"]["parameters"]["properties"]["file_key"]["type"],
            "string"
        );
        assert_eq!(
            resource_get["function"]["parameters"]["properties"]["save_as"]["type"],
            "string"
        );
        assert_eq!(
            resource_get["function"]["parameters"]["properties"]["type"]["enum"],
            json!(["image", "file", "audio", "media"])
        );
        assert!(
            resource_get["function"]["parameters"]["properties"]["type"]["description"]
                .as_str()
                .is_some_and(|description| {
                    description.contains("audio")
                        && description.contains("media")
                        && description.contains("normalize")
                        && description.contains("preview")
                        && description.contains("resource_inventory")
                        && description.contains("exactly one Feishu message resource")
                        && description.contains("same message")
                        && description.contains("payload.file_key uniquely selects")
                })
        );
        assert_eq!(
            resource_get["function"]["parameters"]["required"],
            json!(["save_as"])
        );
        assert!(
            history["function"]["parameters"]["properties"]["container_id_type"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("current Feishu conversation"))
        );
        assert_eq!(get["function"]["parameters"]["required"], json!([]));
        assert_eq!(history["function"]["parameters"]["required"], json!([]));
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn feishu_shape_examples_advertise_explicit_resource_inventory_selection() {
        let shapes = feishu::feishu_shape_examples();
        assert_eq!(
            shapes.get("feishu.messages.resource.get"),
            Some(&json!({
                "message_id": "om_123",
                "file_key": "img_from_resource_inventory",
                "type": "image",
                "save_as": "downloads/preview.png"
            }))
        );
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn provider_tool_definitions_with_config_advertises_feishu_card_update_shared_semantics() {
        let mut config = runtime_config::ToolRuntimeConfig::default();
        config.feishu = Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                app_id: Some("cli_a1b2c3".to_owned()),
                app_secret: Some("app-secret".to_owned()),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig::default(),
        });

        let defs = provider_tool_definitions_with_config(Some(&config));
        let card_update = defs
            .iter()
            .find(|item| item["function"]["name"] == "feishu_card_update")
            .expect("card update definition should exist");

        assert!(
            card_update["function"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("shared=true"))
        );
        assert_eq!(
            card_update["function"]["parameters"]["properties"]["shared"]["type"],
            "boolean"
        );
        assert!(
            card_update["function"]["parameters"]["properties"]["open_ids"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("shared=true"))
        );

        let shapes = feishu::feishu_shape_examples();
        assert_eq!(
            shapes.get("feishu.card.update"),
            Some(&json!({
                "shared": true,
                "markdown": "Approved for everyone"
            }))
        );
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn provider_tool_definitions_with_config_advertises_feishu_card_update_markdown_shortcut() {
        let mut config = runtime_config::ToolRuntimeConfig::default();
        config.feishu = Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                app_id: Some("cli_a1b2c3".to_owned()),
                app_secret: Some("app-secret".to_owned()),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig::default(),
        });

        let defs = provider_tool_definitions_with_config(Some(&config));
        let card_update = defs
            .iter()
            .find(|item| item["function"]["name"] == "feishu_card_update")
            .expect("card update definition should exist");

        assert!(
            card_update["function"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("markdown"))
        );
        assert!(
            card_update["function"]["parameters"]["properties"]["markdown"]["description"]
                .as_str()
                .is_some_and(|description| {
                    description.contains("standard markdown card")
                        && description.contains("Mutually exclusive with `card`")
                })
        );
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn provider_tool_definitions_with_config_advertises_feishu_card_update_token_budget() {
        let mut config = runtime_config::ToolRuntimeConfig::default();
        config.feishu = Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                app_id: Some("cli_a1b2c3".to_owned()),
                app_secret: Some("app-secret".to_owned()),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig::default(),
        });

        let defs = provider_tool_definitions_with_config(Some(&config));
        let card_update = defs
            .iter()
            .find(|item| item["function"]["name"] == "feishu_card_update")
            .expect("card update definition should exist");

        assert!(
            card_update["function"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("30 minutes")
                    && description.contains("twice"))
        );
    }

    #[cfg(feature = "feishu-integration")]
    #[test]
    fn provider_tool_definitions_with_config_advertises_feishu_search_ingress_scope() {
        let mut config = runtime_config::ToolRuntimeConfig::default();
        config.feishu = Some(runtime_config::FeishuToolRuntimeConfig {
            channel: crate::config::FeishuChannelConfig {
                enabled: true,
                app_id: Some("cli_a1b2c3".to_owned()),
                app_secret: Some("app-secret".to_owned()),
                ..crate::config::FeishuChannelConfig::default()
            },
            integration: crate::config::FeishuIntegrationConfig::default(),
        });

        let defs = provider_tool_definitions_with_config(Some(&config));
        let search = defs
            .iter()
            .find(|item| item["function"]["name"] == "feishu_messages_search")
            .expect("search definition should exist");

        assert!(
            search["function"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("current Feishu conversation"))
        );
        assert!(
            search["function"]["parameters"]["properties"]["chat_ids"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("current Feishu conversation"))
        );
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FeishuToolMockRequest {
        method: String,
        path: String,
        query: Option<String>,
        authorization: Option<String>,
        body: String,
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[derive(Clone, Default)]
    struct FeishuToolMockServerState {
        requests: std::sync::Arc<tokio::sync::Mutex<Vec<FeishuToolMockRequest>>>,
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    async fn record_feishu_tool_request(
        axum::extract::State(state): axum::extract::State<FeishuToolMockServerState>,
        request: axum::extract::Request,
    ) {
        let (parts, body) = request.into_parts();
        let body = axum::body::to_bytes(body, usize::MAX)
            .await
            .expect("read mock request body");
        state.requests.lock().await.push(FeishuToolMockRequest {
            method: parts.method.to_string(),
            path: parts.uri.path().to_owned(),
            query: parts.uri.query().map(ToOwned::to_owned),
            authorization: parts
                .headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned),
            body: String::from_utf8(body.to_vec()).expect("mock request body utf8"),
        });
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    async fn spawn_feishu_tool_mock_server(
        router: axum::Router,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock feishu server");
        let address = listener.local_addr().expect("mock server addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve mock feishu api");
        });
        (format!("http://{address}"), handle)
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    fn unique_feishu_tool_temp_dir(label: &str) -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};

        std::env::temp_dir().join(format!(
            "loongclaw-tool-feishu-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    fn seed_feishu_tool_grant(
        sqlite_path: &std::path::Path,
        access_token: &str,
        scopes: &[&str],
    ) -> crate::feishu::FeishuTokenStore {
        seed_feishu_tool_grant_for_account(
            sqlite_path,
            "feishu_main",
            "ou_123",
            access_token,
            scopes,
        )
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    fn seed_feishu_tool_grant_for_account(
        sqlite_path: &std::path::Path,
        account_id: &str,
        open_id: &str,
        access_token: &str,
        scopes: &[&str],
    ) -> crate::feishu::FeishuTokenStore {
        let now_s = crate::feishu::unix_ts_now();
        let store = crate::feishu::FeishuTokenStore::new(sqlite_path.to_path_buf());
        store
            .save_grant(&crate::feishu::FeishuGrant {
                principal: crate::feishu::FeishuUserPrincipal {
                    account_id: account_id.to_owned(),
                    open_id: open_id.to_owned(),
                    union_id: Some("on_456".to_owned()),
                    user_id: Some("u_789".to_owned()),
                    name: Some("Alice".to_owned()),
                    tenant_key: Some("tenant_x".to_owned()),
                    avatar_url: None,
                    email: Some("alice@example.com".to_owned()),
                    enterprise_email: None,
                },
                access_token: access_token.to_owned(),
                refresh_token: format!("r-{access_token}"),
                scopes: crate::feishu::FeishuGrantScopeSet::from_scopes(scopes.iter().copied()),
                access_expires_at_s: now_s + 3600,
                refresh_expires_at_s: now_s + 86_400,
                refreshed_at_s: now_s,
            })
            .expect("save grant");
        store
            .set_selected_grant(account_id, open_id, now_s + 1)
            .expect("select grant");
        store
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    fn build_feishu_tool_runtime_config(
        base_url: String,
        sqlite_path: &std::path::Path,
    ) -> runtime_config::ToolRuntimeConfig {
        runtime_config::ToolRuntimeConfig {
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    account_id: Some("feishu_main".to_owned()),
                    app_id: Some("cli_a1b2c3".to_owned()),
                    app_secret: Some("app-secret".to_owned()),
                    base_url: Some(base_url),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        }
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_doc_read_tool_uses_selected_grant_and_user_token() {
        use std::fs;
        use std::sync::Arc;
        use std::time::{SystemTime, UNIX_EPOCH};

        use axum::{
            Json, Router,
            body::to_bytes,
            extract::{Request, State},
            routing::get,
        };
        use tokio::sync::Mutex;

        #[derive(Debug, Clone, PartialEq, Eq)]
        struct MockRequest {
            path: String,
            authorization: Option<String>,
        }

        #[derive(Clone, Default)]
        struct MockServerState {
            requests: Arc<Mutex<Vec<MockRequest>>>,
        }

        async fn record_request(State(state): State<MockServerState>, request: Request) {
            let (parts, body) = request.into_parts();
            let _ = to_bytes(body, usize::MAX)
                .await
                .expect("read mock request body");
            state.requests.lock().await.push(MockRequest {
                path: parts.uri.path().to_owned(),
                authorization: parts
                    .headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok())
                    .map(ToOwned::to_owned),
            });
        }

        async fn spawn_mock_feishu_server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind mock feishu server");
            let address = listener.local_addr().expect("mock server addr");
            let handle = tokio::spawn(async move {
                axum::serve(listener, router)
                    .await
                    .expect("serve mock feishu api");
            });
            (format!("http://{address}"), handle)
        }

        fn unique_temp_dir(label: &str) -> std::path::PathBuf {
            std::env::temp_dir().join(format!(
                "loongclaw-tool-feishu-{label}-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos()
            ))
        }

        let temp_dir = unique_temp_dir("doc-read");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
        let state = MockServerState {
            requests: requests.clone(),
        };
        let router = Router::new().route(
            "/open-apis/docx/v1/documents/doxcnDemo/raw_content",
            get({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "content": "hello from docs"
                            }
                        }))
                    }
                }
            }),
        );
        let (base_url, server) = spawn_mock_feishu_server(router).await;
        let now_s = crate::feishu::unix_ts_now();
        let store = crate::feishu::FeishuTokenStore::new(sqlite_path.clone());
        store
            .save_grant(&crate::feishu::FeishuGrant {
                principal: crate::feishu::FeishuUserPrincipal {
                    account_id: "feishu_main".to_owned(),
                    open_id: "ou_123".to_owned(),
                    union_id: Some("on_456".to_owned()),
                    user_id: Some("u_789".to_owned()),
                    name: Some("Alice".to_owned()),
                    tenant_key: Some("tenant_x".to_owned()),
                    avatar_url: None,
                    email: Some("alice@example.com".to_owned()),
                    enterprise_email: None,
                },
                access_token: "u-token-doc".to_owned(),
                refresh_token: "r-token-doc".to_owned(),
                scopes: crate::feishu::FeishuGrantScopeSet::from_scopes([
                    "offline_access",
                    "docx:document:readonly",
                ]),
                access_expires_at_s: now_s + 3600,
                refresh_expires_at_s: now_s + 86_400,
                refreshed_at_s: now_s,
            })
            .expect("save grant");
        store
            .set_selected_grant("feishu_main", "ou_123", now_s + 1)
            .expect("select grant");

        let config = runtime_config::ToolRuntimeConfig {
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    account_id: Some("feishu_main".to_owned()),
                    app_id: Some("cli_a1b2c3".to_owned()),
                    app_secret: Some("app-secret".to_owned()),
                    base_url: Some(base_url),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.doc.read".to_owned(),
                payload: serde_json::json!({
                    "url": "https://open.feishu.cn/docx/doxcnDemo"
                }),
            },
            &config,
        )
        .expect("feishu doc read tool should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["document"]["document_id"], "doxcnDemo");
        assert_eq!(outcome.payload["document"]["content"], "hello from docs");
        assert_eq!(outcome.payload["principal"]["open_id"], "ou_123");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer u-token-doc")
        );
        assert_eq!(
            requests[0].path,
            "/open-apis/docx/v1/documents/doxcnDemo/raw_content"
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_doc_create_tool_uses_selected_grant_and_user_token() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("doc-create");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/docx/v1/documents",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "document": {
                                        "document_id": "doxcnCreated",
                                        "revision_id": 1,
                                        "title": "Release Plan"
                                    }
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/blocks/convert",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "first_level_block_ids": ["tmp-heading"],
                                    "blocks": [
                                        {
                                            "block_id": "tmp-heading",
                                            "block_type": 3,
                                            "heading1": {
                                                "elements": [{"text_run": {"content": "Release Plan"}}]
                                            },
                                            "children": []
                                        }
                                    ]
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnCreated/blocks/doxcnCreated/descendant",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "block_id_relations": [
                                        {
                                            "block_id": "doxcnRealHeading",
                                            "temporary_block_id": "tmp-heading"
                                        }
                                    ]
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-doc-create",
            &["offline_access", "docx:document"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.doc.create".to_owned(),
                payload: serde_json::json!({
                    "title": "Release Plan",
                    "content": "# Release Plan",
                    "content_type": "markdown"
                }),
            },
            &config,
        )
        .expect("feishu doc create tool should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["document"]["document_id"], "doxcnCreated");
        assert_eq!(outcome.payload["document"]["title"], "Release Plan");
        assert_eq!(outcome.payload["document"]["revision_id"], 1);
        assert_eq!(
            outcome.payload["document"]["url"],
            "https://open.feishu.cn/docx/doxcnCreated"
        );
        assert_eq!(outcome.payload["content_inserted"], true);
        assert_eq!(outcome.payload["inserted_block_count"], 1);
        assert_eq!(outcome.payload["insert_batch_count"], 1);
        assert_eq!(outcome.payload["principal"]["open_id"], "ou_123");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests
                .iter()
                .map(|request| request.authorization.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("Bearer u-token-doc-create"),
                Some("Bearer u-token-doc-create"),
                Some("Bearer u-token-doc-create"),
            ]
        );
        assert_eq!(requests[0].path, "/open-apis/docx/v1/documents");
        assert_eq!(
            serde_json::from_str::<Value>(&requests[0].body).expect("create request json"),
            serde_json::json!({
                "title": "Release Plan"
            })
        );
        assert_eq!(
            requests[1].path,
            "/open-apis/docx/v1/documents/blocks/convert"
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[1].body).expect("convert request json"),
            serde_json::json!({
                "content_type": "markdown",
                "content": "# Release Plan"
            })
        );
        assert_eq!(
            requests[2].path,
            "/open-apis/docx/v1/documents/doxcnCreated/blocks/doxcnCreated/descendant"
        );
        assert_eq!(
            requests[2].query.as_deref(),
            Some("document_revision_id=-1")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[2].body).expect("descendant request json"),
            serde_json::json!({
                "children_id": ["tmp-heading"],
                "descendants": [
                    {
                        "block_id": "tmp-heading",
                        "block_type": 3,
                        "heading1": {
                            "elements": [{"text_run": {"content": "Release Plan"}}]
                        },
                        "children": []
                    }
                ],
                "index": -1
            })
        );

        server.abort();
    }

    #[cfg(all(
        feature = "feishu-integration",
        feature = "channel-feishu",
        feature = "tool-file"
    ))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_doc_create_tool_reads_content_path_under_safe_file_root() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("doc-create-content-path");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("content-root");
        let content_path = file_root.join("docs/release-plan.md");
        fs::create_dir_all(content_path.parent().expect("content path parent"))
            .expect("create content parent");
        fs::write(&content_path, "# Release Plan").expect("write content fixture");

        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/docx/v1/documents",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "document": {
                                        "document_id": "doxcnCreated",
                                        "revision_id": 1,
                                        "title": "Release Plan",
                                        "url": "https://open.feishu.cn/docx/doxcnCreated"
                                    }
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/blocks/convert",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "first_level_block_ids": ["tmp-heading"],
                                    "blocks": [
                                        {
                                            "block_id": "tmp-heading",
                                            "block_type": 3,
                                            "heading1": {
                                                "elements": [{"text_run": {"content": "Release Plan"}}]
                                            },
                                            "children": []
                                        }
                                    ]
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnCreated/blocks/doxcnCreated/descendant",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "block_id_relations": [
                                        {
                                            "block_id": "doxcnRealHeading",
                                            "temporary_block_id": "tmp-heading"
                                        }
                                    ]
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-doc-create-path",
            &["offline_access", "docx:document"],
        );
        let mut config = build_feishu_tool_runtime_config(base_url, &sqlite_path);
        config.file_root = Some(file_root.clone());

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.doc.create".to_owned(),
                payload: serde_json::json!({
                    "title": "Release Plan",
                    "content_path": "docs/release-plan.md"
                }),
            },
            &config,
        )
        .expect("feishu doc create tool should read content_path");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["content_inserted"], true);
        assert_eq!(outcome.payload["content_type"], "markdown");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 3);
        assert_eq!(
            serde_json::from_str::<Value>(&requests[1].body).expect("convert request json"),
            serde_json::json!({
                "content_type": "markdown",
                "content": "# Release Plan"
            })
        );

        server.abort();
    }

    #[cfg(all(
        feature = "feishu-integration",
        feature = "channel-feishu",
        feature = "tool-file"
    ))]
    #[test]
    fn feishu_doc_create_tool_rejects_content_and_content_path_together() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("doc-create-content-conflict");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("content-root");
        let content_path = file_root.join("docs/release-plan.md");
        fs::create_dir_all(content_path.parent().expect("content path parent"))
            .expect("create content parent");
        fs::write(&content_path, "# Release Plan").expect("write content fixture");
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-doc-create-conflict",
            &["offline_access", "docx:document"],
        );
        let mut config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);
        config.file_root = Some(file_root);

        let error = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.doc.create".to_owned(),
                payload: serde_json::json!({
                    "title": "Release Plan",
                    "content": "# Inline",
                    "content_path": "docs/release-plan.md"
                }),
            },
            &config,
        )
        .expect_err("doc create should reject inline content mixed with content_path");

        assert_eq!(
            error,
            "feishu.doc.create accepts either payload.content or payload.content_path, not both"
        );
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_doc_create_tool_requires_docx_write_scope() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("doc-create-scope");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-doc-create-scope",
            &["offline_access", "docx:document:readonly"],
        );
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let error = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.doc.create".to_owned(),
                payload: serde_json::json!({
                    "title": "Release Plan"
                }),
            },
            &config,
        )
        .expect_err("feishu doc create should reject readonly grant");

        assert!(error.contains("feishu.doc.create requires Feishu scopes [docx:document]"));
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_doc_append_tool_uses_selected_grant_and_user_token() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("doc-append");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/docx/v1/documents/blocks/convert",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "first_level_block_ids": ["tmp-paragraph"],
                                    "blocks": [
                                        {
                                            "block_id": "tmp-paragraph",
                                            "block_type": 2,
                                            "text": {
                                                "elements": [{"text_run": {"content": "Follow-up note"}}]
                                            },
                                            "children": []
                                        }
                                    ]
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "block_id_relations": [
                                        {
                                            "block_id": "blk_real_paragraph",
                                            "temporary_block_id": "tmp-paragraph"
                                        }
                                    ]
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-doc-append",
            &["offline_access", "docx:document"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.doc.append".to_owned(),
                payload: serde_json::json!({
                    "url": "https://open.feishu.cn/docx/doxcnExisting",
                    "content": "Follow-up note"
                }),
            },
            &config,
        )
        .expect("feishu doc append tool should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["document"]["document_id"], "doxcnExisting");
        assert_eq!(
            outcome.payload["document"]["url"],
            "https://open.feishu.cn/docx/doxcnExisting"
        );
        assert_eq!(outcome.payload["inserted_block_count"], 1);
        assert_eq!(outcome.payload["insert_batch_count"], 1);
        assert_eq!(outcome.payload["content_type"], "markdown");
        assert_eq!(outcome.payload["principal"]["open_id"], "ou_123");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests
                .iter()
                .map(|request| request.authorization.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("Bearer u-token-doc-append"),
                Some("Bearer u-token-doc-append"),
            ]
        );
        assert_eq!(
            requests[0].path,
            "/open-apis/docx/v1/documents/blocks/convert"
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[0].body).expect("convert request json"),
            serde_json::json!({
                "content_type": "markdown",
                "content": "Follow-up note"
            })
        );
        assert_eq!(
            requests[1].path,
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant"
        );
        assert_eq!(
            requests[1].query.as_deref(),
            Some("document_revision_id=-1")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[1].body).expect("descendant request json"),
            serde_json::json!({
                "children_id": ["tmp-paragraph"],
                "descendants": [
                    {
                        "block_id": "tmp-paragraph",
                        "block_type": 2,
                        "text": {
                            "elements": [{"text_run": {"content": "Follow-up note"}}]
                        },
                        "children": []
                    }
                ],
                "index": -1
            })
        );

        server.abort();
    }

    #[cfg(all(
        feature = "feishu-integration",
        feature = "channel-feishu",
        feature = "tool-file"
    ))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_doc_append_tool_reads_html_content_path_under_safe_file_root() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("doc-append-content-path");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("content-root");
        let content_path = file_root.join("docs/follow-up.html");
        fs::create_dir_all(content_path.parent().expect("content path parent"))
            .expect("create content parent");
        fs::write(&content_path, "<p>Follow-up note</p>").expect("write content fixture");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/docx/v1/documents/blocks/convert",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "first_level_block_ids": ["tmp-paragraph"],
                                    "blocks": [
                                        {
                                            "block_id": "tmp-paragraph",
                                            "block_type": 2,
                                            "text": {
                                                "elements": [{"text_run": {"content": "Follow-up note"}}]
                                            },
                                            "children": []
                                        }
                                    ]
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "block_id_relations": [
                                        {
                                            "block_id": "blk_real_paragraph",
                                            "temporary_block_id": "tmp-paragraph"
                                        }
                                    ]
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-doc-append-path",
            &["offline_access", "docx:document"],
        );
        let mut config = build_feishu_tool_runtime_config(base_url, &sqlite_path);
        config.file_root = Some(file_root.clone());

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.doc.append".to_owned(),
                payload: serde_json::json!({
                    "url": "https://open.feishu.cn/docx/doxcnExisting",
                    "content_path": "docs/follow-up.html"
                }),
            },
            &config,
        )
        .expect("feishu doc append tool should read content_path");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["content_type"], "html");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            serde_json::from_str::<Value>(&requests[0].body).expect("convert request json"),
            serde_json::json!({
                "content_type": "html",
                "content": "<p>Follow-up note</p>"
            })
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_doc_append_tool_batches_nested_insert_requests_over_limit() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("doc-append-batch");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let first_level_block_ids = (0..1001)
            .map(|index| format!("tmp-block-{index:04}"))
            .collect::<Vec<_>>();
        let descendants = first_level_block_ids
            .iter()
            .map(|block_id| {
                serde_json::json!({
                    "block_id": block_id,
                    "block_type": 2,
                    "text": {
                        "elements": [{"text_run": {"content": block_id}}]
                    },
                    "children": []
                })
            })
            .collect::<Vec<_>>();
        let convert_response = serde_json::json!({
            "code": 0,
            "data": {
                "first_level_block_ids": first_level_block_ids,
                "blocks": descendants,
            }
        });
        let router = Router::new()
            .route(
                "/open-apis/docx/v1/documents/blocks/convert",
                post({
                    let state = state.clone();
                    let convert_response = convert_response.clone();
                    move |request: Request| {
                        let state = state.clone();
                        let convert_response = convert_response.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(convert_response)
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "block_id_relations": []
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-doc-append-batch",
            &["offline_access", "docx:document"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.doc.append".to_owned(),
                payload: serde_json::json!({
                    "url": "https://open.feishu.cn/docx/doxcnExisting",
                    "content": "large converted payload"
                }),
            },
            &config,
        )
        .expect("feishu doc append batching should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["inserted_block_count"], 1001);
        assert_eq!(outcome.payload["insert_batch_count"], 2);

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 3);
        let first_descendant_body =
            serde_json::from_str::<Value>(&requests[1].body).expect("first descendant body json");
        let second_descendant_body =
            serde_json::from_str::<Value>(&requests[2].body).expect("second descendant body json");
        assert_eq!(
            first_descendant_body["children_id"]
                .as_array()
                .map_or(0, Vec::len),
            1000
        );
        assert_eq!(
            first_descendant_body["descendants"]
                .as_array()
                .map_or(0, Vec::len),
            1000
        );
        assert_eq!(
            second_descendant_body["children_id"]
                .as_array()
                .map_or(0, Vec::len),
            1
        );
        assert_eq!(
            second_descendant_body["descendants"]
                .as_array()
                .map_or(0, Vec::len),
            1
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_doc_append_tool_recursively_inserts_single_oversized_subtree() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("doc-append-recursive-subtree");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let child_ids = (0..1001)
            .map(|index| format!("tmp-leaf-{index:04}"))
            .collect::<Vec<_>>();
        let mut descendants = Vec::with_capacity(child_ids.len() + 1);
        descendants.push(serde_json::json!({
            "block_id": "tmp-root",
            "block_type": 3,
            "heading1": {
                "elements": [{"text_run": {"content": "Large subtree"}}]
            },
            "children": child_ids,
        }));
        descendants.extend(child_ids.iter().map(|block_id| {
            serde_json::json!({
                "block_id": block_id,
                "block_type": 2,
                "text": {
                    "elements": [{"text_run": {"content": block_id}}]
                },
                "children": []
            })
        }));
        let convert_response = serde_json::json!({
            "code": 0,
            "data": {
                "first_level_block_ids": ["tmp-root"],
                "blocks": descendants,
            }
        });
        let router = Router::new()
            .route(
                "/open-apis/docx/v1/documents/blocks/convert",
                post({
                    let state = state.clone();
                    let convert_response = convert_response.clone();
                    move |request: Request| {
                        let state = state.clone();
                        let convert_response = convert_response.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(convert_response)
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "block_id_relations": [{
                                        "block_id": "blk_real_root",
                                        "temporary_block_id": "tmp-root"
                                    }]
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_root/descendant",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "block_id_relations": []
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-doc-append-recursive",
            &["offline_access", "docx:document"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.doc.append".to_owned(),
                payload: serde_json::json!({
                    "url": "https://open.feishu.cn/docx/doxcnExisting",
                    "content": "large single subtree"
                }),
            },
            &config,
        )
        .expect("feishu doc append recursive subtree insertion should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["inserted_block_count"], 1002);
        assert_eq!(outcome.payload["insert_batch_count"], 3);

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 4);
        let first_descendant_body =
            serde_json::from_str::<Value>(&requests[1].body).expect("first descendant body json");
        let second_descendant_body =
            serde_json::from_str::<Value>(&requests[2].body).expect("second descendant body json");
        let third_descendant_body =
            serde_json::from_str::<Value>(&requests[3].body).expect("third descendant body json");

        assert_eq!(
            requests[1].path,
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant"
        );
        assert_eq!(
            first_descendant_body["children_id"],
            serde_json::json!(["tmp-root"])
        );
        assert_eq!(
            first_descendant_body["descendants"]
                .as_array()
                .map_or(0, Vec::len),
            1
        );
        assert_eq!(
            first_descendant_body["descendants"][0]["children"],
            serde_json::json!([])
        );

        assert_eq!(
            requests[2].path,
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_root/descendant"
        );
        assert_eq!(
            second_descendant_body["children_id"]
                .as_array()
                .map_or(0, Vec::len),
            1000
        );
        assert_eq!(
            second_descendant_body["descendants"]
                .as_array()
                .map_or(0, Vec::len),
            1000
        );

        assert_eq!(
            requests[3].path,
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_root/descendant"
        );
        assert_eq!(
            third_descendant_body["children_id"]
                .as_array()
                .map_or(0, Vec::len),
            1
        );
        assert_eq!(
            third_descendant_body["descendants"]
                .as_array()
                .map_or(0, Vec::len),
            1
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_doc_append_tool_recursively_inserts_deep_oversized_subtree() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("doc-append-recursive-deep-subtree");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let leaf_ids = (0..1001)
            .map(|index| format!("tmp-leaf-{index:04}"))
            .collect::<Vec<_>>();
        let mut descendants = Vec::with_capacity(leaf_ids.len() + 2);
        descendants.push(serde_json::json!({
            "block_id": "tmp-root",
            "block_type": 3,
            "heading1": {
                "elements": [{"text_run": {"content": "Deep oversized subtree"}}]
            },
            "children": ["tmp-mid"],
        }));
        descendants.push(serde_json::json!({
            "block_id": "tmp-mid",
            "block_type": 2,
            "text": {
                "elements": [{"text_run": {"content": "Large child subtree"}}]
            },
            "children": leaf_ids,
        }));
        descendants.extend(leaf_ids.iter().map(|block_id| {
            serde_json::json!({
                "block_id": block_id,
                "block_type": 2,
                "text": {
                    "elements": [{"text_run": {"content": block_id}}]
                },
                "children": []
            })
        }));
        let convert_response = serde_json::json!({
            "code": 0,
            "data": {
                "first_level_block_ids": ["tmp-root"],
                "blocks": descendants,
            }
        });
        let router = Router::new()
            .route(
                "/open-apis/docx/v1/documents/blocks/convert",
                post({
                    let state = state.clone();
                    let convert_response = convert_response.clone();
                    move |request: Request| {
                        let state = state.clone();
                        let convert_response = convert_response.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(convert_response)
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "block_id_relations": [{
                                        "block_id": "blk_real_root",
                                        "temporary_block_id": "tmp-root"
                                    }]
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_root/descendant",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "block_id_relations": [{
                                        "block_id": "blk_real_mid",
                                        "temporary_block_id": "tmp-mid"
                                    }]
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_mid/descendant",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "block_id_relations": []
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-doc-append-recursive-deep",
            &["offline_access", "docx:document"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.doc.append".to_owned(),
                payload: serde_json::json!({
                    "url": "https://open.feishu.cn/docx/doxcnExisting",
                    "content": "deep large single subtree"
                }),
            },
            &config,
        )
        .expect("feishu doc append deep recursive subtree insertion should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["inserted_block_count"], 1003);
        assert_eq!(outcome.payload["insert_batch_count"], 4);

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 5);
        let first_descendant_body =
            serde_json::from_str::<Value>(&requests[1].body).expect("first descendant body json");
        let second_descendant_body =
            serde_json::from_str::<Value>(&requests[2].body).expect("second descendant body json");
        let third_descendant_body =
            serde_json::from_str::<Value>(&requests[3].body).expect("third descendant body json");
        let fourth_descendant_body =
            serde_json::from_str::<Value>(&requests[4].body).expect("fourth descendant body json");

        assert_eq!(
            requests[1].path,
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant"
        );
        assert_eq!(
            first_descendant_body["children_id"],
            serde_json::json!(["tmp-root"])
        );
        assert_eq!(
            first_descendant_body["descendants"],
            serde_json::json!([{
                "block_id": "tmp-root",
                "block_type": 3,
                "heading1": {
                    "elements": [{"text_run": {"content": "Deep oversized subtree"}}]
                },
                "children": []
            }])
        );

        assert_eq!(
            requests[2].path,
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_root/descendant"
        );
        assert_eq!(
            second_descendant_body["children_id"],
            serde_json::json!(["tmp-mid"])
        );
        assert_eq!(
            second_descendant_body["descendants"],
            serde_json::json!([{
                "block_id": "tmp-mid",
                "block_type": 2,
                "text": {
                    "elements": [{"text_run": {"content": "Large child subtree"}}]
                },
                "children": []
            }])
        );

        assert_eq!(
            requests[3].path,
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_mid/descendant"
        );
        assert_eq!(
            third_descendant_body["children_id"]
                .as_array()
                .map_or(0, Vec::len),
            1000
        );
        assert_eq!(
            third_descendant_body["descendants"]
                .as_array()
                .map_or(0, Vec::len),
            1000
        );

        assert_eq!(
            requests[4].path,
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_mid/descendant"
        );
        assert_eq!(
            fourth_descendant_body["children_id"]
                .as_array()
                .map_or(0, Vec::len),
            1
        );
        assert_eq!(
            fourth_descendant_body["descendants"]
                .as_array()
                .map_or(0, Vec::len),
            1
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_doc_append_tool_supports_oversized_table_subtree() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("doc-append-oversized-table-subtree");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let leaf_ids = (0..1000)
            .map(|index| format!("tmp-cell-1-leaf-{index:04}"))
            .collect::<Vec<_>>();
        let mut descendants = Vec::with_capacity(leaf_ids.len() + 3);
        descendants.push(serde_json::json!({
            "block_id": "tmp-table",
            "block_type": 31,
            "table": {
                "property": {
                    "row_size": 1,
                    "column_size": 2
                }
            },
            "children": ["tmp-cell-1", "tmp-cell-2"],
        }));
        descendants.push(serde_json::json!({
            "block_id": "tmp-cell-1",
            "block_type": 32,
            "table_cell": {},
            "children": leaf_ids,
        }));
        descendants.push(serde_json::json!({
            "block_id": "tmp-cell-2",
            "block_type": 32,
            "table_cell": {},
            "children": [],
        }));
        descendants.extend(leaf_ids.iter().map(|block_id| {
            serde_json::json!({
                "block_id": block_id,
                "block_type": 2,
                "text": {
                    "elements": [{"text_run": {"content": block_id}}]
                },
                "children": []
            })
        }));
        let convert_response = serde_json::json!({
            "code": 0,
            "data": {
                "first_level_block_ids": ["tmp-table"],
                "blocks": descendants,
            }
        });
        let router = Router::new()
            .route(
                "/open-apis/docx/v1/documents/blocks/convert",
                post({
                    let state = state.clone();
                    let convert_response = convert_response.clone();
                    move |request: Request| {
                        let state = state.clone();
                        let convert_response = convert_response.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(convert_response)
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "children": [{
                                        "block_id": "blk_real_table",
                                        "block_type": 31,
                                        "children": ["blk_real_cell_1", "blk_real_cell_2"],
                                        "table": {
                                            "cells": ["blk_real_cell_1", "blk_real_cell_2"],
                                            "property": {
                                                "row_size": 1,
                                                "column_size": 2
                                            }
                                        }
                                    }]
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_cell_1/descendant",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "block_id_relations": []
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-doc-append-oversized-table",
            &["offline_access", "docx:document"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.doc.append".to_owned(),
                payload: serde_json::json!({
                    "url": "https://open.feishu.cn/docx/doxcnExisting",
                    "content": "oversized table subtree"
                }),
            },
            &config,
        )
        .expect("feishu doc append oversized table subtree insertion should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["inserted_block_count"], 1003);
        assert_eq!(outcome.payload["insert_batch_count"], 2);

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 3);

        let create_table_body =
            serde_json::from_str::<Value>(&requests[1].body).expect("create table body json");
        assert_eq!(
            requests[1].path,
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children"
        );
        assert_eq!(
            create_table_body["children"].as_array().map_or(0, Vec::len),
            1
        );
        assert_eq!(create_table_body["children"][0]["block_type"], 31);
        assert!(create_table_body["children"][0].get("block_id").is_none());
        assert!(create_table_body["children"][0].get("children").is_none());

        let first_cell_body =
            serde_json::from_str::<Value>(&requests[2].body).expect("first cell body json");
        assert_eq!(
            requests[2].path,
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_cell_1/descendant"
        );
        assert_eq!(
            first_cell_body["children_id"]
                .as_array()
                .map_or(0, Vec::len),
            1000
        );
        assert_eq!(
            first_cell_body["descendants"]
                .as_array()
                .map_or(0, Vec::len),
            1000
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_doc_append_tool_supports_oversized_callout_subtree() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("doc-append-oversized-callout-subtree");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let leaf_ids = (0..1001)
            .map(|index| format!("tmp-callout-leaf-{index:04}"))
            .collect::<Vec<_>>();
        let mut descendants = Vec::with_capacity(leaf_ids.len() + 1);
        descendants.push(serde_json::json!({
            "block_id": "tmp-callout",
            "block_type": 19,
            "callout": {
                "emoji_id": "smile"
            },
            "children": leaf_ids,
        }));
        descendants.extend(leaf_ids.iter().map(|block_id| {
            serde_json::json!({
                "block_id": block_id,
                "block_type": 2,
                "text": {
                    "elements": [{"text_run": {"content": block_id}}]
                },
                "children": []
            })
        }));
        let convert_response = serde_json::json!({
            "code": 0,
            "data": {
                "first_level_block_ids": ["tmp-callout"],
                "blocks": descendants,
            }
        });
        let router = Router::new()
            .route(
                "/open-apis/docx/v1/documents/blocks/convert",
                post({
                    let state = state.clone();
                    let convert_response = convert_response.clone();
                    move |request: Request| {
                        let state = state.clone();
                        let convert_response = convert_response.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(convert_response)
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "children": [{
                                        "block_id": "blk_real_callout",
                                        "block_type": 19,
                                        "children": [],
                                        "callout": {
                                            "emoji_id": "smile"
                                        }
                                    }]
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_callout/descendant",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "block_id_relations": []
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-doc-append-oversized-callout",
            &["offline_access", "docx:document"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.doc.append".to_owned(),
                payload: serde_json::json!({
                    "url": "https://open.feishu.cn/docx/doxcnExisting",
                    "content": "oversized callout subtree"
                }),
            },
            &config,
        )
        .expect("feishu doc append oversized callout subtree insertion should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["inserted_block_count"], 1002);
        assert_eq!(outcome.payload["insert_batch_count"], 3);

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 4);
        let create_callout_body =
            serde_json::from_str::<Value>(&requests[1].body).expect("create callout body json");
        assert_eq!(
            requests[1].path,
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children"
        );
        assert_eq!(
            create_callout_body["children"],
            serde_json::json!([{
                "block_type": 19,
                "callout": {
                    "emoji_id": "smile"
                }
            }])
        );
        assert_eq!(
            requests[2].path,
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_callout/descendant"
        );
        assert_eq!(
            requests[3].path,
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_callout/descendant"
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_doc_append_tool_supports_oversized_grid_subtree() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("doc-append-oversized-grid-subtree");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let leaf_ids = (0..1000)
            .map(|index| format!("tmp-grid-leaf-{index:04}"))
            .collect::<Vec<_>>();
        let mut descendants = Vec::with_capacity(leaf_ids.len() + 3);
        descendants.push(serde_json::json!({
            "block_id": "tmp-grid",
            "block_type": 24,
            "grid": {},
            "children": ["tmp-grid-col-1", "tmp-grid-col-2"],
        }));
        descendants.push(serde_json::json!({
            "block_id": "tmp-grid-col-1",
            "block_type": 25,
            "grid_column": {},
            "children": leaf_ids,
        }));
        descendants.push(serde_json::json!({
            "block_id": "tmp-grid-col-2",
            "block_type": 25,
            "grid_column": {},
            "children": [],
        }));
        descendants.extend(leaf_ids.iter().map(|block_id| {
            serde_json::json!({
                "block_id": block_id,
                "block_type": 2,
                "text": {
                    "elements": [{"text_run": {"content": block_id}}]
                },
                "children": []
            })
        }));
        let convert_response = serde_json::json!({
            "code": 0,
            "data": {
                "first_level_block_ids": ["tmp-grid"],
                "blocks": descendants,
            }
        });
        let router = Router::new()
            .route(
                "/open-apis/docx/v1/documents/blocks/convert",
                post({
                    let state = state.clone();
                    let convert_response = convert_response.clone();
                    move |request: Request| {
                        let state = state.clone();
                        let convert_response = convert_response.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(convert_response)
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "children": [{
                                        "block_id": "blk_real_grid",
                                        "block_type": 24,
                                        "children": ["blk_real_grid_col_1", "blk_real_grid_col_2"],
                                        "grid": {}
                                    }]
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_grid_col_1/descendant",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "block_id_relations": []
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-doc-append-oversized-grid",
            &["offline_access", "docx:document"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.doc.append".to_owned(),
                payload: serde_json::json!({
                    "url": "https://open.feishu.cn/docx/doxcnExisting",
                    "content": "oversized grid subtree"
                }),
            },
            &config,
        )
        .expect("feishu doc append oversized grid subtree insertion should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["inserted_block_count"], 1003);
        assert_eq!(outcome.payload["insert_batch_count"], 2);

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 3);
        let create_grid_body =
            serde_json::from_str::<Value>(&requests[1].body).expect("create grid body json");
        assert_eq!(
            requests[1].path,
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children"
        );
        assert_eq!(
            create_grid_body["children"],
            serde_json::json!([{
                "block_type": 24,
                "grid": {}
            }])
        );
        assert_eq!(
            requests[2].path,
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_grid_col_1/descendant"
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_doc_append_tool_requires_docx_write_scope() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("doc-append-scope");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-doc-append-scope",
            &["offline_access", "docx:document:readonly"],
        );
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let error = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.doc.append".to_owned(),
                payload: serde_json::json!({
                    "url": "https://open.feishu.cn/docx/doxcnExisting",
                    "content": "Follow-up note"
                }),
            },
            &config,
        )
        .expect_err("feishu doc append should reject readonly grant");

        assert!(error.contains("feishu.doc.append requires Feishu scopes [docx:document]"));
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_history_tool_uses_tenant_token_before_im_request() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::{get, post},
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-history");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-history"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages",
                get({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "has_more": false,
                                    "page_token": "",
                                    "items": [{
                                        "message_id": "om_123",
                                        "chat_id": "oc_demo",
                                        "msg_type": "text",
                                        "create_time": "1700000000"
                                    }]
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-history",
            &["offline_access", "im:message:readonly"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.history".to_owned(),
                payload: serde_json::json!({
                    "container_id_type": "chat",
                    "container_id": "oc_demo",
                    "page_size": 20
                }),
            },
            &config,
        )
        .expect("feishu messages history tool should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["principal"]["open_id"], "ou_123");
        assert_eq!(outcome.payload["page"]["items"][0]["message_id"], "om_123");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].path,
            "/open-apis/auth/v3/tenant_access_token/internal"
        );
        assert_eq!(requests[0].authorization, None);
        assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-history")
        );
        let query = requests[1].query.as_deref().unwrap_or_default();
        assert!(query.contains("container_id_type=chat"));
        assert!(query.contains("container_id=oc_demo"));
        assert!(query.contains("page_size=20"));

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_history_tool_defaults_thread_container_and_account_from_internal_ingress()
     {
        use std::collections::BTreeMap;
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::{get, post},
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-history-ingress");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-history-ingress"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages",
                get({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "has_more": false,
                                    "page_token": "",
                                    "items": [{
                                        "message_id": "om_thread_hist_1",
                                        "chat_id": "oc_ingress_history",
                                        "root_id": "omt_ingress_history",
                                        "msg_type": "text",
                                        "create_time": "1700000100"
                                    }]
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant_for_account(
            &sqlite_path,
            "feishu_shared",
            "ou_shared",
            "u-token-history-ingress",
            &["offline_access", "im:message:readonly"],
        );
        let config = runtime_config::ToolRuntimeConfig {
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([
                        (
                            "work".to_owned(),
                            crate::config::FeishuAccountConfig {
                                account_id: Some("feishu_shared".to_owned()),
                                app_id: Some("cli_work".to_owned()),
                                app_secret: Some("app-secret-work".to_owned()),
                                base_url: Some(base_url),
                                ..crate::config::FeishuAccountConfig::default()
                            },
                        ),
                        (
                            "alerts".to_owned(),
                            crate::config::FeishuAccountConfig {
                                account_id: Some("feishu_shared".to_owned()),
                                app_id: Some("cli_alerts".to_owned()),
                                app_secret: Some("app-secret-alerts".to_owned()),
                                base_url: Some("http://127.0.0.1:9".to_owned()),
                                ..crate::config::FeishuAccountConfig::default()
                            },
                        ),
                    ]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.history".to_owned(),
                payload: serde_json::json!({
                    "page_size": 20,
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "configured_account_id": "work",
                                "account_id": "feishu_shared",
                                "conversation_id": "oc_ingress_history",
                                "thread_id": "omt_ingress_history"
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect("feishu messages history tool should infer thread container from ingress");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["configured_account"], "work");
        assert_eq!(
            outcome.payload["page"]["items"][0]["message_id"],
            "om_thread_hist_1"
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
        let query = requests[1].query.as_deref().unwrap_or_default();
        assert!(query.contains("container_id_type=thread"), "query={query}");
        assert!(
            query.contains("container_id=omt_ingress_history"),
            "query={query}"
        );
        assert!(query.contains("page_size=20"), "query={query}");

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_get_tool_uses_tenant_token_for_detail_request() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::{get, post},
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-get");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-detail"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_789",
                get({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "items": [{
                                        "message_id": "om_789",
                                        "chat_id": "oc_demo",
                                        "msg_type": "text",
                                        "sender": {
                                            "id": "ou_123",
                                            "sender_type": "user"
                                        }
                                    }]
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-detail",
            &["offline_access", "im:message.group_msg:readonly"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.get".to_owned(),
                payload: serde_json::json!({
                    "message_id": "om_789"
                }),
            },
            &config,
        )
        .expect("feishu messages get tool should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["message"]["message_id"], "om_789");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-detail")
        );
        assert_eq!(requests[1].path, "/open-apis/im/v1/messages/om_789");

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_get_tool_defaults_message_id_and_account_from_internal_ingress() {
        use std::collections::BTreeMap;
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::{get, post},
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-get-ingress");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-detail-ingress"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_ingress_detail",
                get({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "items": [{
                                        "message_id": "om_ingress_detail",
                                        "chat_id": "oc_demo",
                                        "msg_type": "text",
                                        "sender": {
                                            "id": "ou_shared",
                                            "sender_type": "user"
                                        }
                                    }]
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant_for_account(
            &sqlite_path,
            "feishu_shared",
            "ou_shared",
            "u-token-detail-ingress",
            &["offline_access", "im:message.group_msg:readonly"],
        );
        let config = runtime_config::ToolRuntimeConfig {
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([
                        (
                            "work".to_owned(),
                            crate::config::FeishuAccountConfig {
                                account_id: Some("feishu_shared".to_owned()),
                                app_id: Some("cli_work".to_owned()),
                                app_secret: Some("app-secret-work".to_owned()),
                                base_url: Some(base_url),
                                ..crate::config::FeishuAccountConfig::default()
                            },
                        ),
                        (
                            "alerts".to_owned(),
                            crate::config::FeishuAccountConfig {
                                account_id: Some("feishu_shared".to_owned()),
                                app_id: Some("cli_alerts".to_owned()),
                                app_secret: Some("app-secret-alerts".to_owned()),
                                base_url: Some("http://127.0.0.1:9".to_owned()),
                                ..crate::config::FeishuAccountConfig::default()
                            },
                        ),
                    ]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.get".to_owned(),
                payload: serde_json::json!({
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "configured_account_id": "work",
                                "account_id": "feishu_shared",
                                "conversation_id": "oc_demo"
                            },
                            "delivery": {
                                "source_message_id": "om_ingress_detail"
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect("feishu messages get tool should infer message id and account from ingress");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["configured_account"], "work");
        assert_eq!(
            outcome.payload["message"]["message_id"],
            "om_ingress_detail"
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-detail-ingress")
        );
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_ingress_detail"
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_resource_get_tool_downloads_message_resource_to_safe_file_root() {
        use std::fs;

        use axum::{
            Router,
            body::Body,
            extract::{Request, State},
            http::{HeaderMap, HeaderValue, StatusCode},
            response::Response,
            routing::{get, post},
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("downloads-root");
        fs::create_dir_all(&file_root).expect("create file root");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            axum::Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-resource"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_resource_123/resources/file_demo_456",
                get({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            let mut headers = HeaderMap::new();
                            headers.insert(
                                axum::http::header::CONTENT_TYPE,
                                HeaderValue::from_static("application/pdf"),
                            );
                            headers.insert(
                                axum::http::header::CONTENT_DISPOSITION,
                                HeaderValue::from_static("attachment; filename=\"spec-sheet.pdf\""),
                            );
                            Response::builder()
                                .status(StatusCode::OK)
                                .body(Body::from("pdf-demo-bytes"))
                                .map(|mut response| {
                                    *response.headers_mut() = headers;
                                    response
                                })
                                .expect("build binary response")
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-resource",
            &["offline_access", "im:message:readonly"],
        );
        let mut config = build_feishu_tool_runtime_config(base_url, &sqlite_path);
        config.file_root = Some(file_root.clone());

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.resource.get".to_owned(),
                payload: serde_json::json!({
                    "message_id": "om_resource_123",
                    "file_key": "file_demo_456",
                    "type": "file",
                    "save_as": "artifacts/specs/spec-sheet.pdf"
                }),
            },
            &config,
        )
        .expect("feishu message resource tool should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["message_id"], "om_resource_123");
        assert_eq!(outcome.payload["file_key"], "file_demo_456");
        assert_eq!(outcome.payload["resource_type"], "file");
        assert_eq!(outcome.payload["content_type"], "application/pdf");
        assert_eq!(outcome.payload["file_name"], "spec-sheet.pdf");
        assert_eq!(outcome.payload["bytes_written"], 14);

        let expected_path = file_root.join("artifacts/specs/spec-sheet.pdf");
        let canonical_expected_path =
            std::fs::canonicalize(&expected_path).expect("canonicalize downloaded file");
        assert_eq!(
            outcome.payload["path"].as_str(),
            Some(canonical_expected_path.display().to_string().as_str())
        );
        assert_eq!(
            fs::read(&expected_path).expect("read downloaded file"),
            b"pdf-demo-bytes"
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-resource")
        );
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_resource_123/resources/file_demo_456"
        );
        assert_eq!(requests[1].query.as_deref(), Some("type=file"));

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_resource_get_tool_defaults_message_id_account_and_single_resource_from_internal_ingress()
     {
        use std::collections::BTreeMap;
        use std::fs;

        use axum::{
            Router,
            body::Body,
            extract::{Request, State},
            http::{HeaderMap, HeaderValue, StatusCode},
            response::Response,
            routing::{get, post},
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-ingress");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("downloads-root");
        fs::create_dir_all(&file_root).expect("create file root");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            axum::Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-resource-ingress"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_ingress_resource/resources/img_ingress_456",
                get({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            let mut headers = HeaderMap::new();
                            headers.insert(
                                axum::http::header::CONTENT_TYPE,
                                HeaderValue::from_static("image/png"),
                            );
                            Response::builder()
                                .status(StatusCode::OK)
                                .body(Body::from("png-demo-bytes"))
                                .map(|mut response| {
                                    *response.headers_mut() = headers;
                                    response
                                })
                                .expect("build binary response")
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant_for_account(
            &sqlite_path,
            "feishu_shared",
            "ou_shared",
            "u-token-resource-ingress",
            &["offline_access", "im:message:readonly"],
        );
        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(file_root.clone()),
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([
                        (
                            "work".to_owned(),
                            crate::config::FeishuAccountConfig {
                                account_id: Some("feishu_shared".to_owned()),
                                app_id: Some("cli_work".to_owned()),
                                app_secret: Some("app-secret-work".to_owned()),
                                base_url: Some(base_url),
                                ..crate::config::FeishuAccountConfig::default()
                            },
                        ),
                        (
                            "alerts".to_owned(),
                            crate::config::FeishuAccountConfig {
                                account_id: Some("feishu_shared".to_owned()),
                                app_id: Some("cli_alerts".to_owned()),
                                app_secret: Some("app-secret-alerts".to_owned()),
                                base_url: Some("http://127.0.0.1:9".to_owned()),
                                ..crate::config::FeishuAccountConfig::default()
                            },
                        ),
                    ]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.resource.get".to_owned(),
                payload: serde_json::json!({
                    "save_as": "artifacts/images/incoming.png",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "configured_account_id": "work",
                                "account_id": "feishu_shared",
                                "conversation_id": "oc_demo"
                            },
                            "delivery": {
                                "source_message_id": "om_ingress_resource",
                                "resources": [
                                    {
                                        "type": "image",
                                        "file_key": "img_ingress_456"
                                    }
                                ]
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect("feishu message resource tool should infer message id and account from ingress");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["configured_account"], "work");
        assert_eq!(outcome.payload["message_id"], "om_ingress_resource");
        assert_eq!(outcome.payload["resource_type"], "image");

        let saved_path = outcome.payload["path"]
            .as_str()
            .expect("resource output path");
        assert!(saved_path.ends_with("/artifacts/images/incoming.png"));
        assert_eq!(
            fs::read(saved_path).expect("read downloaded image"),
            b"png-demo-bytes"
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-resource-ingress")
        );
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_ingress_resource/resources/img_ingress_456"
        );
        assert_eq!(requests[1].query.as_deref(), Some("type=image"));

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_resource_get_tool_accepts_audio_alias_for_single_file_ingress_resource()
     {
        use std::collections::BTreeMap;
        use std::fs;

        use axum::{
            Router,
            body::Body,
            extract::{Request, State},
            http::{HeaderMap, HeaderValue, StatusCode},
            response::Response,
            routing::{get, post},
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-audio-alias");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("downloads-root");
        fs::create_dir_all(&file_root).expect("create file root");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            axum::Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-resource-audio-alias"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_ingress_audio/resources/file_audio_456",
                get({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            let mut headers = HeaderMap::new();
                            headers.insert(
                                axum::http::header::CONTENT_TYPE,
                                HeaderValue::from_static("audio/ogg"),
                            );
                            headers.insert(
                                axum::http::header::CONTENT_DISPOSITION,
                                HeaderValue::from_static("attachment; filename=\"voice.ogg\""),
                            );
                            Response::builder()
                                .status(StatusCode::OK)
                                .body(Body::from("voice-demo-bytes"))
                                .map(|mut response| {
                                    *response.headers_mut() = headers;
                                    response
                                })
                                .expect("build binary response")
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant_for_account(
            &sqlite_path,
            "feishu_shared",
            "ou_shared",
            "u-token-resource-audio-alias",
            &["offline_access", "im:message:readonly"],
        );
        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(file_root.clone()),
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([(
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some("cli_work".to_owned()),
                            app_secret: Some("app-secret-work".to_owned()),
                            base_url: Some(base_url),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    )]),
                    default_account: Some("work".to_owned()),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.resource.get".to_owned(),
                payload: serde_json::json!({
                    "type": "audio",
                    "save_as": "artifacts/audio/voice.ogg",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "configured_account_id": "work",
                                "account_id": "feishu_shared",
                                "conversation_id": "oc_demo"
                            },
                            "delivery": {
                                "source_message_id": "om_ingress_audio",
                                "resources": [
                                    {
                                        "type": "file",
                                        "file_key": "file_audio_456"
                                    }
                                ]
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect("feishu message resource tool should accept audio alias");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["configured_account"], "work");
        assert_eq!(outcome.payload["message_id"], "om_ingress_audio");
        assert_eq!(outcome.payload["file_key"], "file_audio_456");
        assert_eq!(outcome.payload["resource_type"], "file");
        assert_eq!(outcome.payload["content_type"], "audio/ogg");
        assert_eq!(outcome.payload["file_name"], "voice.ogg");

        let saved_path = outcome.payload["path"]
            .as_str()
            .expect("resource output path");
        assert!(saved_path.ends_with("/artifacts/audio/voice.ogg"));
        assert_eq!(
            fs::read(saved_path).expect("read downloaded audio"),
            b"voice-demo-bytes"
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-resource-audio-alias")
        );
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_ingress_audio/resources/file_audio_456"
        );
        assert_eq!(requests[1].query.as_deref(), Some("type=file"));

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_resource_get_tool_infers_file_key_from_unique_type_in_multi_resource_ingress()
     {
        use std::collections::BTreeMap;
        use std::fs;

        use axum::{
            Router,
            body::Body,
            extract::{Request, State},
            http::{HeaderMap, HeaderValue, StatusCode},
            response::Response,
            routing::{get, post},
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-multi-type-infer");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("downloads-root");
        fs::create_dir_all(&file_root).expect("create file root");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            axum::Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-resource-multi-type"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_ingress_media/resources/img_media_456",
                get({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            let mut headers = HeaderMap::new();
                            headers.insert(
                                axum::http::header::CONTENT_TYPE,
                                HeaderValue::from_static("image/png"),
                            );
                            Response::builder()
                                .status(StatusCode::OK)
                                .body(Body::from("png-media-preview"))
                                .map(|mut response| {
                                    *response.headers_mut() = headers;
                                    response
                                })
                                .expect("build binary response")
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant_for_account(
            &sqlite_path,
            "feishu_shared",
            "ou_shared",
            "u-token-resource-multi-type",
            &["offline_access", "im:message:readonly"],
        );
        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(file_root.clone()),
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([(
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some("cli_work".to_owned()),
                            app_secret: Some("app-secret-work".to_owned()),
                            base_url: Some(base_url),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    )]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.resource.get".to_owned(),
                payload: serde_json::json!({
                    "type": "image",
                    "save_as": "artifacts/media/preview.png",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "configured_account_id": "work",
                                "account_id": "feishu_shared",
                                "conversation_id": "oc_demo"
                            },
                            "delivery": {
                                "source_message_id": "om_ingress_media",
                                "resources": [
                                    {
                                        "type": "file",
                                        "file_key": "file_media_123",
                                        "file_name": "clip.mp4"
                                    },
                                    {
                                        "type": "image",
                                        "file_key": "img_media_456"
                                    }
                                ]
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect("unique ingress type should infer file key");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["message_id"], "om_ingress_media");
        assert_eq!(outcome.payload["file_key"], "img_media_456");
        assert_eq!(outcome.payload["resource_type"], "image");

        let saved_path = outcome.payload["path"]
            .as_str()
            .expect("resource output path");
        assert!(saved_path.ends_with("/artifacts/media/preview.png"));
        assert_eq!(
            fs::read(saved_path).expect("read downloaded image"),
            b"png-media-preview"
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_ingress_media/resources/img_media_456"
        );
        assert_eq!(requests[1].query.as_deref(), Some("type=image"));

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_resource_get_tool_infers_type_from_matching_file_key_in_multi_resource_ingress()
     {
        use std::collections::BTreeMap;
        use std::fs;

        use axum::{
            Router,
            body::Body,
            extract::{Request, State},
            http::{HeaderMap, HeaderValue, StatusCode},
            response::Response,
            routing::{get, post},
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-multi-key-infer");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("downloads-root");
        fs::create_dir_all(&file_root).expect("create file root");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            axum::Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-resource-multi-key"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_ingress_post/resources/img_post_456",
                get({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            let mut headers = HeaderMap::new();
                            headers.insert(
                                axum::http::header::CONTENT_TYPE,
                                HeaderValue::from_static("image/jpeg"),
                            );
                            Response::builder()
                                .status(StatusCode::OK)
                                .body(Body::from("jpeg-post-image"))
                                .map(|mut response| {
                                    *response.headers_mut() = headers;
                                    response
                                })
                                .expect("build binary response")
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant_for_account(
            &sqlite_path,
            "feishu_shared",
            "ou_shared",
            "u-token-resource-multi-key",
            &["offline_access", "im:message:readonly"],
        );
        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(file_root.clone()),
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([(
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some("cli_work".to_owned()),
                            app_secret: Some("app-secret-work".to_owned()),
                            base_url: Some(base_url),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    )]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.resource.get".to_owned(),
                payload: serde_json::json!({
                    "file_key": "img_post_456",
                    "save_as": "artifacts/post/image.jpg",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "configured_account_id": "work",
                                "account_id": "feishu_shared",
                                "conversation_id": "oc_demo"
                            },
                            "delivery": {
                                "source_message_id": "om_ingress_post",
                                "resources": [
                                    {
                                        "type": "file",
                                        "file_key": "file_post_123",
                                        "file_name": "report.pdf"
                                    },
                                    {
                                        "type": "image",
                                        "file_key": "img_post_456"
                                    }
                                ]
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect("matching ingress file key should infer type");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["message_id"], "om_ingress_post");
        assert_eq!(outcome.payload["file_key"], "img_post_456");
        assert_eq!(outcome.payload["resource_type"], "image");

        let saved_path = outcome.payload["path"]
            .as_str()
            .expect("resource output path");
        assert!(saved_path.ends_with("/artifacts/post/image.jpg"));
        assert_eq!(
            fs::read(saved_path).expect("read downloaded image"),
            b"jpeg-post-image"
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_ingress_post/resources/img_post_456"
        );
        assert_eq!(requests[1].query.as_deref(), Some("type=image"));

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_resource_get_tool_rejects_type_only_when_multi_resource_ingress_has_multiple_matches()
     {
        use std::collections::BTreeMap;
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-multi-type-ambiguous");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("downloads-root");
        fs::create_dir_all(&file_root).expect("create file root");
        let _store = seed_feishu_tool_grant_for_account(
            &sqlite_path,
            "feishu_shared",
            "ou_shared",
            "u-token-resource-multi-type-ambiguous",
            &["offline_access", "im:message:readonly"],
        );
        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(file_root),
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([(
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some("cli_work".to_owned()),
                            app_secret: Some("app-secret-work".to_owned()),
                            base_url: Some("http://127.0.0.1:9".to_owned()),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    )]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let error = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.resource.get".to_owned(),
                payload: serde_json::json!({
                    "type": "image",
                    "save_as": "artifacts/post/ambiguous.jpg",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "configured_account_id": "work",
                                "account_id": "feishu_shared",
                                "conversation_id": "oc_demo"
                            },
                            "delivery": {
                                "source_message_id": "om_ingress_post",
                                "resources": [
                                    {
                                        "type": "image",
                                        "file_key": "img_post_111"
                                    },
                                    {
                                        "type": "image",
                                        "file_key": "img_post_222"
                                    },
                                    {
                                        "type": "file",
                                        "file_key": "file_post_333",
                                        "file_name": "report.pdf"
                                    }
                                ]
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect_err("ambiguous type-only selection should be rejected");

        assert!(
            error.contains("payload.type matches multiple current Feishu ingress resources")
                && error.contains("img_post_111")
                && error.contains("img_post_222")
                && error.contains("payload.file_key")
                && error.contains("resource_inventory"),
            "unexpected error: {error}"
        );
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_resource_get_tool_does_not_infer_ingress_resource_when_message_id_overrides_current_ingress_message()
     {
        use std::collections::BTreeMap;
        use std::fs;

        let temp_dir =
            unique_feishu_tool_temp_dir("messages-resource-get-override-message-no-infer");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("downloads-root");
        fs::create_dir_all(&file_root).expect("create file root");
        let _store = seed_feishu_tool_grant_for_account(
            &sqlite_path,
            "feishu_shared",
            "ou_shared",
            "u-token-resource-override-no-infer",
            &["offline_access", "im:message:readonly"],
        );
        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(file_root),
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([(
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some("cli_work".to_owned()),
                            app_secret: Some("app-secret-work".to_owned()),
                            base_url: Some("http://127.0.0.1:9".to_owned()),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    )]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let error = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.resource.get".to_owned(),
                payload: serde_json::json!({
                    "message_id": "om_other_message",
                    "type": "image",
                    "save_as": "artifacts/post/override.jpg",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "configured_account_id": "work",
                                "account_id": "feishu_shared",
                                "conversation_id": "oc_demo"
                            },
                            "delivery": {
                                "source_message_id": "om_ingress_post",
                                "resources": [
                                    {
                                        "type": "image",
                                        "file_key": "img_post_111"
                                    },
                                    {
                                        "type": "file",
                                        "file_key": "file_post_333",
                                        "file_name": "report.pdf"
                                    }
                                ]
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect_err("explicit message override should disable ingress resource inference");

        assert!(
            error.contains("requires payload.file_key")
                && error.contains("payload.message_id `om_other_message` differs")
                && error.contains("current Feishu ingress message `om_ingress_post`")
                && error.contains(
                    "defaults only apply when payload.message_id is omitted or matches the current message"
                ),
            "unexpected error: {error}"
        );
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_resource_get_tool_rejects_explicit_resource_pair_that_conflicts_with_current_ingress_resource()
     {
        use std::collections::BTreeMap;
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-explicit-pair-conflict");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("downloads-root");
        fs::create_dir_all(&file_root).expect("create file root");
        let _store = seed_feishu_tool_grant_for_account(
            &sqlite_path,
            "feishu_shared",
            "ou_shared",
            "u-token-resource-pair-conflict",
            &["offline_access", "im:message:readonly"],
        );
        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(file_root),
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([(
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some("cli_work".to_owned()),
                            app_secret: Some("app-secret-work".to_owned()),
                            base_url: Some("http://127.0.0.1:9".to_owned()),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    )]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let error = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.resource.get".to_owned(),
                payload: serde_json::json!({
                    "file_key": "img_post_111",
                    "type": "file",
                    "save_as": "artifacts/post/conflict.bin",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "configured_account_id": "work",
                                "account_id": "feishu_shared",
                                "conversation_id": "oc_demo"
                            },
                            "delivery": {
                                "source_message_id": "om_ingress_post",
                                "resources": [
                                    {
                                        "type": "image",
                                        "file_key": "img_post_111"
                                    },
                                    {
                                        "type": "file",
                                        "file_key": "file_post_333",
                                        "file_name": "report.pdf"
                                    }
                                ]
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect_err("mismatched explicit file_key and type should be rejected");

        assert!(
            error.contains("payload.type conflicts")
                && error.contains("type=image")
                && error.contains("file_key=img_post_111"),
            "unexpected error: {error}"
        );
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_resource_get_tool_rejects_ambiguous_ingress_resource_defaults() {
        use std::collections::BTreeMap;
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-ingress-ambiguous");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("downloads-root");
        fs::create_dir_all(&file_root).expect("create file root");
        let _store = seed_feishu_tool_grant_for_account(
            &sqlite_path,
            "feishu_shared",
            "ou_shared",
            "u-token-resource-ingress-ambiguous",
            &["offline_access", "im:message:readonly"],
        );
        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(file_root),
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([(
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some("cli_work".to_owned()),
                            app_secret: Some("app-secret-work".to_owned()),
                            base_url: Some("http://127.0.0.1:9".to_owned()),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    )]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let error = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.resource.get".to_owned(),
                payload: serde_json::json!({
                    "save_as": "artifacts/images/ambiguous.png",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "configured_account_id": "work",
                                "account_id": "feishu_shared",
                                "conversation_id": "oc_demo"
                            },
                            "delivery": {
                                "source_message_id": "om_ingress_resource",
                                "resources": [
                                    {
                                        "type": "image",
                                        "file_key": "img_ingress_456"
                                    },
                                    {
                                        "type": "file",
                                        "file_key": "file_ingress_789",
                                        "file_name": "report.pdf"
                                    }
                                ]
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect_err("ambiguous ingress resources should require explicit selection");

        assert!(
            error.contains("multiple Feishu message resources"),
            "unexpected error: {error}"
        );
        assert!(
            error.contains("img_ingress_456")
                && error.contains("file_ingress_789")
                && error.contains("report.pdf")
                && error.contains("resource_inventory"),
            "unexpected error: {error}"
        );
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_resource_get_tool_reports_current_ingress_resource_on_file_key_conflict()
     {
        use std::collections::BTreeMap;
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-file-key-conflict");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("downloads-root");
        fs::create_dir_all(&file_root).expect("create file root");
        let _store = seed_feishu_tool_grant_for_account(
            &sqlite_path,
            "feishu_shared",
            "ou_shared",
            "u-token-resource-conflict",
            &["offline_access", "im:message:readonly"],
        );
        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(file_root),
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([(
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_shared".to_owned()),
                            app_id: Some("cli_work".to_owned()),
                            app_secret: Some("app-secret-work".to_owned()),
                            base_url: Some("http://127.0.0.1:9".to_owned()),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    )]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let error = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.resource.get".to_owned(),
                payload: serde_json::json!({
                    "message_id": "om_ingress_resource",
                    "file_key": "img_other_999",
                    "save_as": "artifacts/images/conflict.png",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "configured_account_id": "work",
                                "account_id": "feishu_shared",
                                "conversation_id": "oc_demo"
                            },
                            "delivery": {
                                "source_message_id": "om_ingress_resource",
                                "resources": [
                                    {
                                        "type": "image",
                                        "file_key": "img_ingress_456"
                                    }
                                ]
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect_err("conflicting file key should be rejected");

        assert!(
            error.contains("payload.file_key conflicts")
                && error.contains("type=image")
                && error.contains("file_key=img_ingress_456"),
            "unexpected error: {error}"
        );
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_resource_get_tool_rejects_paths_that_escape_file_root() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("messages-resource-get-escape");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("downloads-root");
        fs::create_dir_all(&file_root).expect("create file root");
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-resource-escape",
            &["offline_access", "im:message:readonly"],
        );
        let mut config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);
        config.file_root = Some(file_root);

        let error = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.resource.get".to_owned(),
                payload: serde_json::json!({
                    "message_id": "om_resource_escape",
                    "file_key": "file_escape_456",
                    "type": "file",
                    "save_as": "../escape.pdf"
                }),
            },
            &config,
        )
        .expect_err("path escape should be rejected");

        assert!(
            error.contains("escapes configured file root"),
            "unexpected error: {error}"
        );
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_whoami_tool_refreshes_expired_grant_before_fetching_user_profile() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::{get, post},
        };

        let temp_dir = unique_feishu_tool_temp_dir("whoami-refresh");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/authen/v2/oauth/token",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "access_token": "u-token-refreshed",
                                "refresh_token": "r-token-next",
                                "expires_in": 7200,
                                "refresh_token_expires_in": 2592000,
                                "scope": "offline_access search:message calendar:calendar:readonly"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/authen/v1/user_info",
                get({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "name": "Alice Refreshed",
                                    "open_id": "ou_123",
                                    "union_id": "on_456",
                                    "user_id": "u_789",
                                    "tenant_key": "tenant_x"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let now_s = crate::feishu::unix_ts_now();
        let store = crate::feishu::FeishuTokenStore::new(sqlite_path.clone());
        store
            .save_grant(&crate::feishu::FeishuGrant {
                principal: crate::feishu::FeishuUserPrincipal {
                    account_id: "feishu_main".to_owned(),
                    open_id: "ou_123".to_owned(),
                    union_id: Some("on_456".to_owned()),
                    user_id: Some("u_789".to_owned()),
                    name: Some("Alice Old".to_owned()),
                    tenant_key: Some("tenant_x".to_owned()),
                    avatar_url: None,
                    email: Some("alice@example.com".to_owned()),
                    enterprise_email: None,
                },
                access_token: "u-token-expired".to_owned(),
                refresh_token: "r-token-old".to_owned(),
                scopes: crate::feishu::FeishuGrantScopeSet::from_scopes(["offline_access"]),
                access_expires_at_s: now_s - 10,
                refresh_expires_at_s: now_s + 86_400,
                refreshed_at_s: now_s - 3600,
            })
            .expect("save expired grant");
        store
            .set_selected_grant("feishu_main", "ou_123", now_s + 1)
            .expect("select grant");
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.whoami".to_owned(),
                payload: serde_json::json!({}),
            },
            &config,
        )
        .expect("feishu whoami tool should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["principal"]["open_id"], "ou_123");
        assert_eq!(outcome.payload["principal"]["name"], "Alice Refreshed");
        assert_eq!(outcome.payload["user_info"]["name"], "Alice Refreshed");

        let stored = store
            .load_grant("feishu_main", "ou_123")
            .expect("load refreshed grant")
            .expect("refreshed grant should still exist");
        assert_eq!(stored.access_token, "u-token-refreshed");
        assert_eq!(stored.refresh_token, "r-token-next");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].path, "/open-apis/authen/v2/oauth/token");
        assert!(
            requests[0]
                .body
                .contains("\"grant_type\":\"refresh_token\"")
        );
        assert_eq!(requests[1].path, "/open-apis/authen/v1/user_info");
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer u-token-refreshed")
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_whoami_tool_includes_configured_account_in_outcome_for_account_alias() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::get,
        };

        let temp_dir = unique_feishu_tool_temp_dir("whoami-configured-account-outcome");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new().route(
            "/open-apis/authen/v1/user_info",
            get({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "name": "Alice Alias",
                                "open_id": "ou_alias",
                                "union_id": "on_alias",
                                "user_id": "u_alias",
                                "tenant_key": "tenant_alias"
                            }
                        }))
                    }
                }
            }),
        );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant_for_account(
            &sqlite_path,
            "feishu_secondary",
            "ou_alias",
            "u-token-alias",
            &["offline_access"],
        );
        let config = runtime_config::ToolRuntimeConfig {
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([(
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_secondary".to_owned()),
                            app_id: Some("cli_secondary".to_owned()),
                            app_secret: Some("app-secret-secondary".to_owned()),
                            base_url: Some(base_url),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    )]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "feishu.whoami".to_owned(),
                payload: json!({
                    "account_id": "work",
                    "open_id": "ou_alias"
                }),
            },
            &config,
        )
        .expect("feishu whoami tool should succeed for configured account alias");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["account_id"], "feishu_secondary");
        assert_eq!(outcome.payload["configured_account"], "work");
        assert_eq!(outcome.payload["principal"]["open_id"], "ou_alias");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/open-apis/authen/v1/user_info");
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer u-token-alias")
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_whoami_tool_suggests_account_scoped_auth_start_when_no_grant_exists() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("whoami-no-grant");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let error = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "feishu.whoami".to_owned(),
                payload: json!({}),
            },
            &config,
        )
        .expect_err("missing Feishu grant should fail");

        assert!(error.contains("loongclaw feishu auth start --account feishu_main"));
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_whoami_tool_uses_configured_account_id_in_auth_hint() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("whoami-configured-account-hint");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let config = runtime_config::ToolRuntimeConfig {
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([(
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_secondary".to_owned()),
                            app_id: Some("cli_secondary".to_owned()),
                            app_secret: Some("app-secret-secondary".to_owned()),
                            base_url: Some("http://127.0.0.1:9".to_owned()),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    )]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let error = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "feishu.whoami".to_owned(),
                payload: json!({
                    "account_id": "work"
                }),
            },
            &config,
        )
        .expect_err("missing Feishu grant should fail");

        assert!(error.contains("loongclaw feishu auth start --account work"));
        assert!(!error.contains("--account feishu_secondary"));
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_whoami_tool_suggests_auth_select_when_multiple_grants_are_available() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("whoami-multi-grant");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let store = crate::feishu::FeishuTokenStore::new(sqlite_path.clone());
        let now_s = crate::feishu::unix_ts_now();
        store
            .save_grant(&crate::feishu::FeishuGrant {
                principal: crate::feishu::FeishuUserPrincipal {
                    account_id: "feishu_main".to_owned(),
                    open_id: "ou_123".to_owned(),
                    union_id: Some("on_123".to_owned()),
                    user_id: Some("u_123".to_owned()),
                    name: Some("Alice".to_owned()),
                    tenant_key: Some("tenant_x".to_owned()),
                    avatar_url: None,
                    email: Some("alice@example.com".to_owned()),
                    enterprise_email: None,
                },
                access_token: "u-token-123".to_owned(),
                refresh_token: "r-token-123".to_owned(),
                scopes: crate::feishu::FeishuGrantScopeSet::from_scopes(["offline_access"]),
                access_expires_at_s: now_s + 3600,
                refresh_expires_at_s: now_s + 86_400,
                refreshed_at_s: now_s,
            })
            .expect("save first grant");
        store
            .save_grant(&crate::feishu::FeishuGrant {
                principal: crate::feishu::FeishuUserPrincipal {
                    account_id: "feishu_main".to_owned(),
                    open_id: "ou_456".to_owned(),
                    union_id: Some("on_456".to_owned()),
                    user_id: Some("u_456".to_owned()),
                    name: Some("Bob".to_owned()),
                    tenant_key: Some("tenant_x".to_owned()),
                    avatar_url: None,
                    email: Some("bob@example.com".to_owned()),
                    enterprise_email: None,
                },
                access_token: "u-token-456".to_owned(),
                refresh_token: "r-token-456".to_owned(),
                scopes: crate::feishu::FeishuGrantScopeSet::from_scopes(["offline_access"]),
                access_expires_at_s: now_s + 3600,
                refresh_expires_at_s: now_s + 86_400,
                refreshed_at_s: now_s + 1,
            })
            .expect("save second grant");
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let error = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "feishu.whoami".to_owned(),
                payload: json!({}),
            },
            &config,
        )
        .expect_err("ambiguous Feishu grant selection should fail");

        assert!(
            error
                .contains("loongclaw feishu auth select --account feishu_main --open-id <open_id>")
        );
        assert!(error.contains("ou_123"));
        assert!(error.contains("ou_456"));
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_whoami_tool_reports_available_open_ids_for_missing_explicit_open_id() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("whoami-missing-explicit-open-id");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let store = crate::feishu::FeishuTokenStore::new(sqlite_path.clone());
        let now_s = crate::feishu::unix_ts_now();
        store
            .save_grant(&crate::feishu::FeishuGrant {
                principal: crate::feishu::FeishuUserPrincipal {
                    account_id: "feishu_main".to_owned(),
                    open_id: "ou_123".to_owned(),
                    union_id: Some("on_123".to_owned()),
                    user_id: Some("u_123".to_owned()),
                    name: Some("Alice".to_owned()),
                    tenant_key: Some("tenant_x".to_owned()),
                    avatar_url: None,
                    email: Some("alice@example.com".to_owned()),
                    enterprise_email: None,
                },
                access_token: "u-token-123".to_owned(),
                refresh_token: "r-token-123".to_owned(),
                scopes: crate::feishu::FeishuGrantScopeSet::from_scopes(["offline_access"]),
                access_expires_at_s: now_s + 3600,
                refresh_expires_at_s: now_s + 86_400,
                refreshed_at_s: now_s,
            })
            .expect("save first grant");
        store
            .save_grant(&crate::feishu::FeishuGrant {
                principal: crate::feishu::FeishuUserPrincipal {
                    account_id: "feishu_main".to_owned(),
                    open_id: "ou_456".to_owned(),
                    union_id: Some("on_456".to_owned()),
                    user_id: Some("u_456".to_owned()),
                    name: Some("Bob".to_owned()),
                    tenant_key: Some("tenant_x".to_owned()),
                    avatar_url: None,
                    email: Some("bob@example.com".to_owned()),
                    enterprise_email: None,
                },
                access_token: "u-token-456".to_owned(),
                refresh_token: "r-token-456".to_owned(),
                scopes: crate::feishu::FeishuGrantScopeSet::from_scopes(["offline_access"]),
                access_expires_at_s: now_s + 3600,
                refresh_expires_at_s: now_s + 86_400,
                refreshed_at_s: now_s + 1,
            })
            .expect("save second grant");
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let error = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "feishu.whoami".to_owned(),
                payload: json!({
                    "open_id": "ou_missing"
                }),
            },
            &config,
        )
        .expect_err("missing explicit open_id should fail");

        assert!(error.contains("open_id `ou_missing`"));
        assert!(error.contains("available open_ids: ou_456, ou_123"));
        assert!(
            error
                .contains("loongclaw feishu auth select --account feishu_main --open-id <open_id>")
        );
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_search_tool_uses_user_token_directly() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-search");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new().route(
            "/open-apis/search/v2/message",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "items": ["om_1", "om_2"],
                                "page_token": "next-search",
                                "has_more": true
                            }
                        }))
                    }
                }
            }),
        );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-search",
            &["offline_access", "search:message"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.search".to_owned(),
                payload: serde_json::json!({
                    "query": "incident",
                    "user_id_type": "open_id",
                    "page_size": 10,
                    "chat_ids": ["oc_demo"],
                    "from_ids": ["ou_123"],
                    "message_type": "text"
                }),
            },
            &config,
        )
        .expect("feishu messages search tool should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["page"]["items"][0], "om_1");
        assert_eq!(outcome.payload["page"]["has_more"], true);

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/open-apis/search/v2/message");
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer u-token-search")
        );
        assert!(requests[0].query.as_deref().is_some_and(|query| {
            query.contains("user_id_type=open_id") && query.contains("page_size=10")
        }));
        assert!(requests[0].body.contains("\"query\":\"incident\""));
        assert!(requests[0].body.contains("\"chat_ids\":[\"oc_demo\"]"));

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_search_tool_defaults_chat_scope_and_account_from_internal_ingress() {
        use std::collections::BTreeMap;
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-search-ingress");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new().route(
            "/open-apis/search/v2/message",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "items": ["om_ingress_search_1"],
                                "page_token": "",
                                "has_more": false
                            }
                        }))
                    }
                }
            }),
        );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant_for_account(
            &sqlite_path,
            "feishu_shared",
            "ou_shared",
            "u-token-search-ingress",
            &["offline_access", "search:message"],
        );
        let config = runtime_config::ToolRuntimeConfig {
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([
                        (
                            "work".to_owned(),
                            crate::config::FeishuAccountConfig {
                                account_id: Some("feishu_shared".to_owned()),
                                app_id: Some("cli_work".to_owned()),
                                app_secret: Some("app-secret-work".to_owned()),
                                base_url: Some(base_url),
                                ..crate::config::FeishuAccountConfig::default()
                            },
                        ),
                        (
                            "alerts".to_owned(),
                            crate::config::FeishuAccountConfig {
                                account_id: Some("feishu_shared".to_owned()),
                                app_id: Some("cli_alerts".to_owned()),
                                app_secret: Some("app-secret-alerts".to_owned()),
                                base_url: Some("http://127.0.0.1:9".to_owned()),
                                ..crate::config::FeishuAccountConfig::default()
                            },
                        ),
                    ]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.search".to_owned(),
                payload: serde_json::json!({
                    "query": "incident",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "configured_account_id": "work",
                                "account_id": "feishu_shared",
                                "conversation_id": "oc_ingress_search"
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect("feishu messages search tool should infer chat scope from ingress");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["configured_account"], "work");
        assert_eq!(outcome.payload["page"]["items"][0], "om_ingress_search_1");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/open-apis/search/v2/message");
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer u-token-search-ingress")
        );
        assert!(
            requests[0]
                .body
                .contains("\"chat_ids\":[\"oc_ingress_search\"]")
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_send_tool_uses_tenant_token_and_default_receive_id_type() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-send");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-send"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_sent_1",
                                    "root_id": "om_sent_1"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-send",
            &["offline_access", "im:message:send_as_bot"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.send".to_owned(),
                payload: serde_json::json!({
                    "receive_id": "oc_demo",
                    "text": "ship it"
                }),
            },
            &config,
        )
        .expect("feishu messages send tool should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["delivery"]["message_id"], "om_sent_1");
        assert_eq!(outcome.payload["delivery"]["mode"], "send");
        assert_eq!(outcome.payload["delivery"]["msg_type"], "text");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].path,
            "/open-apis/auth/v3/tenant_access_token/internal"
        );
        assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-send")
        );
        assert!(
            requests[1]
                .query
                .as_deref()
                .is_some_and(|query| { query.contains("receive_id_type=chat_id") })
        );
        assert!(requests[1].body.contains("\"receive_id\":\"oc_demo\""));
        assert!(requests[1].body.contains("\"msg_type\":\"text\""));
        assert!(requests[1].body.contains("\\\"text\\\":\\\"ship it\\\""));

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_send_tool_defaults_receive_id_and_account_from_internal_ingress() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-send-ingress");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-send-ingress"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_sent_ingress_1",
                                    "root_id": "om_sent_ingress_1"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant_for_account(
            &sqlite_path,
            "feishu_secondary",
            "ou_secondary",
            "u-token-secondary",
            &["offline_access", "im:message:send_as_bot"],
        );
        let config = runtime_config::ToolRuntimeConfig {
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    account_id: Some("feishu_primary".to_owned()),
                    app_id: Some("cli_primary".to_owned()),
                    app_secret: Some("app-secret-primary".to_owned()),
                    receive_id_type: "open_id".to_owned(),
                    accounts: BTreeMap::from([(
                        "work".to_owned(),
                        crate::config::FeishuAccountConfig {
                            account_id: Some("feishu_secondary".to_owned()),
                            app_id: Some("cli_secondary".to_owned()),
                            app_secret: Some("app-secret-secondary".to_owned()),
                            base_url: Some(base_url),
                            receive_id_type: Some("chat_id".to_owned()),
                            ..crate::config::FeishuAccountConfig::default()
                        },
                    )]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.send".to_owned(),
                payload: serde_json::json!({
                    "text": "ship by ingress",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "account_id": "feishu_secondary",
                                "conversation_id": "oc_ingress_send"
                            },
                            "delivery": {
                                "source_message_id": "om_source_send"
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect("feishu messages send tool should default from internal ingress");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["account_id"], "feishu_secondary");
        assert_eq!(outcome.payload["configured_account"], "work");
        assert_eq!(outcome.payload["principal"]["open_id"], "ou_secondary");
        assert_eq!(outcome.payload["delivery"]["receive_id"], "oc_ingress_send");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
        assert!(
            requests[1]
                .query
                .as_deref()
                .is_some_and(|query| { query.contains("receive_id_type=chat_id") })
        );
        assert!(
            requests[1]
                .body
                .contains("\"receive_id\":\"oc_ingress_send\"")
        );
        assert!(
            requests[1]
                .body
                .contains("\\\"text\\\":\\\"ship by ingress\\\"")
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_send_tool_prefers_configured_account_from_internal_ingress() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-send-configured-ingress");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-send-configured"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_send_configured_1",
                                    "root_id": "om_send_configured_1"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant_for_account(
            &sqlite_path,
            "feishu_shared",
            "ou_shared",
            "u-token-send-configured",
            &["offline_access", "im:message:send_as_bot"],
        );
        let config = runtime_config::ToolRuntimeConfig {
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([
                        (
                            "work".to_owned(),
                            crate::config::FeishuAccountConfig {
                                account_id: Some("feishu_shared".to_owned()),
                                app_id: Some("cli_work".to_owned()),
                                app_secret: Some("app-secret-work".to_owned()),
                                base_url: Some(base_url),
                                receive_id_type: Some("chat_id".to_owned()),
                                ..crate::config::FeishuAccountConfig::default()
                            },
                        ),
                        (
                            "alerts".to_owned(),
                            crate::config::FeishuAccountConfig {
                                account_id: Some("feishu_shared".to_owned()),
                                app_id: Some("cli_alerts".to_owned()),
                                app_secret: Some("app-secret-alerts".to_owned()),
                                base_url: Some("http://127.0.0.1:9".to_owned()),
                                receive_id_type: Some("open_id".to_owned()),
                                ..crate::config::FeishuAccountConfig::default()
                            },
                        ),
                    ]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.send".to_owned(),
                payload: serde_json::json!({
                    "text": "send from configured ingress",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "configured_account_id": "work",
                                "account_id": "feishu_shared",
                                "conversation_id": "oc_configured_send"
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect("feishu messages send tool should use configured account from ingress");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["configured_account"], "work");
        assert_eq!(outcome.payload["account_id"], "feishu_shared");
        assert_eq!(
            outcome.payload["delivery"]["receive_id"],
            "oc_configured_send"
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-send-configured")
        );
        assert!(
            requests[1]
                .query
                .as_deref()
                .is_some_and(|query| query.contains("receive_id_type=chat_id"))
        );
        assert!(
            requests[1]
                .body
                .contains("\"receive_id\":\"oc_configured_send\"")
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_send_tool_passes_uuid_to_api() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-send-uuid");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-send-uuid"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_sent_uuid_1",
                                    "root_id": "om_sent_uuid_1"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-send-uuid",
            &["offline_access", "im:message:send_as_bot"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.send".to_owned(),
                payload: serde_json::json!({
                    "receive_id": "oc_demo",
                    "text": "ship with uuid",
                    "uuid": "send-uuid-1"
                }),
            },
            &config,
        )
        .expect("feishu messages send tool should pass uuid");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["delivery"]["message_id"], "om_sent_uuid_1");
        assert_eq!(outcome.payload["delivery"]["uuid"], "send-uuid-1");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
        assert!(requests[1].body.contains("\"uuid\":\"send-uuid-1\""));
        assert!(
            requests[1]
                .body
                .contains("\\\"text\\\":\\\"ship with uuid\\\"")
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_send_tool_supports_post_content() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-send-post");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-send-post"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_sent_post_1",
                                    "root_id": "om_sent_post_1"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-send-post",
            &["offline_access", "im:message:send_as_bot"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.send".to_owned(),
                payload: serde_json::json!({
                    "receive_id": "oc_demo",
                    "post": {
                        "zh_cn": {
                            "title": "Ship update",
                            "content": [[
                                {
                                    "tag": "text",
                                    "text": "rich ship"
                                },
                                {
                                    "tag": "a",
                                    "text": "Open Platform",
                                    "href": "https://open.feishu.cn"
                                }
                            ]]
                        }
                    }
                }),
            },
            &config,
        )
        .expect("feishu messages send tool should support post content");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["delivery"]["message_id"], "om_sent_post_1");
        assert_eq!(outcome.payload["delivery"]["msg_type"], "post");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
        assert!(requests[1].body.contains("\"msg_type\":\"post\""));
        assert!(requests[1].body.contains("\\\"zh_cn\\\""));
        assert!(
            requests[1]
                .body
                .contains("\\\"title\\\":\\\"Ship update\\\"")
        );
        assert!(requests[1].body.contains("\\\"text\\\":\\\"rich ship\\\""));

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_send_tool_supports_image_key() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-send-image");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-send-image"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_sent_image_1",
                                    "root_id": "om_sent_image_1"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-send-image",
            &["offline_access", "im:message:send_as_bot"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.send".to_owned(),
                payload: serde_json::json!({
                    "receive_id": "oc_demo",
                    "image_key": "img_v2_demo"
                }),
            },
            &config,
        )
        .expect("feishu messages send tool should support image_key");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["delivery"]["message_id"], "om_sent_image_1");
        assert_eq!(outcome.payload["delivery"]["msg_type"], "image");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
        assert!(requests[1].body.contains("\"msg_type\":\"image\""));
        assert!(
            requests[1]
                .body
                .contains("\\\"image_key\\\":\\\"img_v2_demo\\\"")
        );

        server.abort();
    }

    #[cfg(all(
        feature = "feishu-integration",
        feature = "channel-feishu",
        feature = "tool-file"
    ))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_send_tool_uploads_image_path_under_safe_file_root_and_sends_image_message()
     {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-send-image-path");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("uploads-root");
        fs::create_dir_all(&file_root).expect("create file root");
        let image_path = file_root.join("assets/demo.png");
        fs::create_dir_all(image_path.parent().expect("image path parent"))
            .expect("create image parent");
        fs::write(&image_path, b"png-demo-bytes").expect("write image fixture");

        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-send-image-path"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/images",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "image_key": "img_uploaded_from_path"
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_sent_image_path_1",
                                    "root_id": "om_sent_image_path_1"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-send-image-path",
            &["offline_access", "im:message:send_as_bot"],
        );
        let mut config = build_feishu_tool_runtime_config(base_url, &sqlite_path);
        config.file_root = Some(file_root);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.send".to_owned(),
                payload: serde_json::json!({
                    "receive_id": "oc_demo",
                    "image_path": "assets/demo.png"
                }),
            },
            &config,
        )
        .expect("feishu messages send tool should upload image path");

        assert_eq!(outcome.status, "ok");
        assert_eq!(
            outcome.payload["delivery"]["message_id"],
            "om_sent_image_path_1"
        );
        assert_eq!(outcome.payload["delivery"]["msg_type"], "image");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests[0].path,
            "/open-apis/auth/v3/tenant_access_token/internal"
        );
        assert_eq!(requests[1].path, "/open-apis/im/v1/images");
        assert_eq!(requests[2].path, "/open-apis/im/v1/messages");
        assert!(
            requests[1].body.contains("name=\"image_type\"")
                && requests[1].body.contains("message")
        );
        assert!(requests[1].body.contains("filename=\"demo.png\""));
        assert!(requests[2].body.contains("\"msg_type\":\"image\""));
        assert!(
            requests[2]
                .body
                .contains("\\\"image_key\\\":\\\"img_uploaded_from_path\\\"")
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_messages_send_tool_rejects_mixed_text_and_post_content() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("messages-send-post-mixed");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let _store =
            seed_feishu_tool_grant(&sqlite_path, "u-token-send-post-mixed", &["offline_access"]);
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let error = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.send".to_owned(),
                payload: serde_json::json!({
                    "receive_id": "oc_demo",
                    "text": "plain text",
                    "post": {
                        "zh_cn": {
                            "title": "Ship update",
                            "content": [[{
                                "tag": "text",
                                "text": "rich ship"
                            }]]
                        }
                    }
                }),
            },
            &config,
        )
        .expect_err("mixed text and post content should fail");

        assert!(error.contains("payload.text"));
        assert!(error.contains("payload.post"));
        assert!(error.contains("not both"));
    }

    #[cfg(all(
        feature = "feishu-integration",
        feature = "channel-feishu",
        feature = "tool-file"
    ))]
    #[test]
    fn feishu_messages_send_tool_rejects_mixed_image_key_and_image_path() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("messages-send-image-mixed-source");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-send-image-mixed-source",
            &["offline_access"],
        );
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let error = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.send".to_owned(),
                payload: serde_json::json!({
                    "receive_id": "oc_demo",
                    "image_key": "img_v2_demo",
                    "image_path": "assets/demo.png"
                }),
            },
            &config,
        )
        .expect_err("mixed image key and image path should fail");

        assert_eq!(
            error,
            "feishu.messages.send accepts either payload.image_key or payload.image_path, not both"
        );
    }

    #[cfg(all(
        feature = "feishu-integration",
        feature = "channel-feishu",
        feature = "tool-file"
    ))]
    #[test]
    fn feishu_messages_send_tool_rejects_file_type_without_file_path() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("messages-send-file-type-without-path");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-send-file-type-without-path",
            &["offline_access"],
        );
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let error = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.send".to_owned(),
                payload: serde_json::json!({
                    "receive_id": "oc_demo",
                    "file_key": "file_v2_demo",
                    "file_type": "stream"
                }),
            },
            &config,
        )
        .expect_err("file type without file path should fail");

        assert_eq!(
            error,
            "feishu.messages.send only allows payload.file_type with payload.file_path"
        );
    }

    #[cfg(all(
        feature = "feishu-integration",
        feature = "channel-feishu",
        feature = "tool-file"
    ))]
    #[test]
    fn feishu_messages_send_tool_rejects_file_path_that_escapes_safe_file_root() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("messages-send-file-path-escape");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("uploads-root");
        fs::create_dir_all(&file_root).expect("create file root");
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-send-file-path-escape",
            &["offline_access"],
        );
        let mut config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);
        config.file_root = Some(file_root.clone());

        let escape_target = file_root
            .parent()
            .expect("temp dir parent")
            .join("outside.txt");
        fs::write(&escape_target, b"not allowed").expect("write outside file");

        let error = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.send".to_owned(),
                payload: serde_json::json!({
                    "receive_id": "oc_demo",
                    "file_path": "../outside.txt"
                }),
            },
            &config,
        )
        .expect_err("escaped file path should fail");

        assert!(error.contains("escapes configured file root"));
        assert!(error.contains("outside.txt"));
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_messages_send_tool_requires_confirmed_write_scope() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("messages-send-scope");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let _store =
            seed_feishu_tool_grant(&sqlite_path, "u-token-send-scope", &["offline_access"]);
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let error = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.send".to_owned(),
                payload: serde_json::json!({
                    "receive_id": "oc_demo",
                    "text": "ship it"
                }),
            },
            &config,
        )
        .expect_err("write scope enforcement should reject grants without message send scopes");

        assert!(
            error.contains("feishu.messages.send requires at least one Feishu scope [im:message, im:message:send_as_bot, im:message:send]"),
            "error={error}"
        );
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_reply_tool_uses_tenant_token_and_card_mode() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-reply");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-reply"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_parent_1/reply",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_reply_1",
                                    "root_id": "om_parent_1",
                                    "parent_id": "om_parent_1"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-reply",
            &["offline_access", "im:message:send_as_bot"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.reply".to_owned(),
                payload: serde_json::json!({
                    "message_id": "om_parent_1",
                    "text": "on it",
                    "as_card": true
                }),
            },
            &config,
        )
        .expect("feishu messages reply tool should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["delivery"]["message_id"], "om_reply_1");
        assert_eq!(outcome.payload["delivery"]["mode"], "reply");
        assert_eq!(outcome.payload["delivery"]["msg_type"], "interactive");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_parent_1/reply"
        );
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-reply")
        );
        assert!(requests[1].body.contains("\"msg_type\":\"interactive\""));
        assert!(requests[1].body.contains("\\\"tag\\\":\\\"markdown\\\""));
        assert!(requests[1].body.contains("\\\"content\\\":\\\"on it\\\""));

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_reply_tool_passes_reply_in_thread_to_api() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-reply-thread");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-reply-thread"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_parent_thread/reply",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_reply_thread_1",
                                    "root_id": "om_parent_thread",
                                    "parent_id": "om_parent_thread"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-reply-thread",
            &["offline_access", "im:message:send_as_bot"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.reply".to_owned(),
                payload: serde_json::json!({
                    "message_id": "om_parent_thread",
                    "text": "threaded reply",
                    "reply_in_thread": true
                }),
            },
            &config,
        )
        .expect("feishu messages reply tool should support reply_in_thread");

        assert_eq!(outcome.status, "ok");
        assert_eq!(
            outcome.payload["delivery"]["message_id"],
            "om_reply_thread_1"
        );
        assert_eq!(
            outcome.payload["delivery"]["reply_to_message_id"],
            "om_parent_thread"
        );
        assert_eq!(outcome.payload["delivery"]["reply_in_thread"], true);

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_parent_thread/reply"
        );
        assert!(requests[1].body.contains("\"reply_in_thread\":true"));
        assert!(
            requests[1]
                .body
                .contains("\\\"text\\\":\\\"threaded reply\\\"")
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_reply_tool_passes_uuid_to_api() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-reply-uuid");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-reply-uuid"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_parent_uuid/reply",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_reply_uuid_1",
                                    "root_id": "om_parent_uuid",
                                    "parent_id": "om_parent_uuid"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-reply-uuid",
            &["offline_access", "im:message:send_as_bot"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.reply".to_owned(),
                payload: serde_json::json!({
                    "message_id": "om_parent_uuid",
                    "text": "reply with uuid",
                    "uuid": "reply-uuid-1"
                }),
            },
            &config,
        )
        .expect("feishu messages reply tool should pass uuid");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["delivery"]["message_id"], "om_reply_uuid_1");
        assert_eq!(outcome.payload["delivery"]["uuid"], "reply-uuid-1");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_parent_uuid/reply"
        );
        assert!(requests[1].body.contains("\"uuid\":\"reply-uuid-1\""));
        assert!(
            requests[1]
                .body
                .contains("\\\"text\\\":\\\"reply with uuid\\\"")
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_reply_tool_supports_post_content() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-reply-post");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-reply-post"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_parent_post/reply",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_reply_post_1",
                                    "root_id": "om_parent_post",
                                    "parent_id": "om_parent_post"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-reply-post",
            &["offline_access", "im:message:send_as_bot"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.reply".to_owned(),
                payload: serde_json::json!({
                    "message_id": "om_parent_post",
                    "post": {
                        "zh_cn": {
                            "title": "Thread update",
                            "content": [[{
                                "tag": "text",
                                "text": "rich reply"
                            }]]
                        }
                    }
                }),
            },
            &config,
        )
        .expect("feishu messages reply tool should support post content");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["delivery"]["message_id"], "om_reply_post_1");
        assert_eq!(outcome.payload["delivery"]["msg_type"], "post");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_parent_post/reply"
        );
        assert!(requests[1].body.contains("\"msg_type\":\"post\""));
        assert!(
            requests[1]
                .body
                .contains("\\\"title\\\":\\\"Thread update\\\"")
        );
        assert!(requests[1].body.contains("\\\"text\\\":\\\"rich reply\\\""));

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_reply_tool_defaults_reply_in_thread_from_internal_ingress() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-reply-thread-ingress-default");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-reply-thread-ingress-default"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_source_thread_ingress/reply",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_reply_thread_ingress_1",
                                    "root_id": "om_thread_ingress",
                                    "parent_id": "om_source_thread_ingress"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-reply-thread-ingress-default",
            &["offline_access", "im:message:send_as_bot"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.reply".to_owned(),
                payload: serde_json::json!({
                    "text": "reply from threaded ingress",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "conversation_id": "oc_demo",
                                "thread_id": "om_thread_ingress"
                            },
                            "delivery": {
                                "source_message_id": "om_source_thread_ingress",
                                "thread_root_id": "om_thread_ingress"
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect("feishu messages reply tool should default reply_in_thread from ingress");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["delivery"]["reply_in_thread"], true);
        assert_eq!(
            outcome.payload["delivery"]["reply_to_message_id"],
            "om_source_thread_ingress"
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_source_thread_ingress/reply"
        );
        assert!(requests[1].body.contains("\"reply_in_thread\":true"));

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_reply_tool_explicit_false_overrides_ingress_thread_hint() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-reply-thread-ingress-explicit-false");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-reply-thread-ingress-false"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_source_thread_ingress_false/reply",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_reply_thread_ingress_false_1",
                                    "root_id": "om_source_thread_ingress_false",
                                    "parent_id": "om_source_thread_ingress_false"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-reply-thread-ingress-false",
            &["offline_access", "im:message:send_as_bot"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.reply".to_owned(),
                payload: serde_json::json!({
                    "text": "reply from threaded ingress but not in thread",
                    "reply_in_thread": false,
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "conversation_id": "oc_demo",
                                "thread_id": "om_thread_ingress_false"
                            },
                            "delivery": {
                                "source_message_id": "om_source_thread_ingress_false",
                                "thread_root_id": "om_thread_ingress_false"
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect("feishu messages reply tool should honor explicit false");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["delivery"]["reply_in_thread"], false);

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_source_thread_ingress_false/reply"
        );
        assert!(!requests[1].body.contains("\"reply_in_thread\":true"));

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_reply_tool_supports_file_key() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-reply-file");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-reply-file"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_parent_file/reply",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_reply_file_1",
                                    "root_id": "om_parent_file",
                                    "parent_id": "om_parent_file"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-reply-file",
            &["offline_access", "im:message:send_as_bot"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.reply".to_owned(),
                payload: serde_json::json!({
                    "message_id": "om_parent_file",
                    "file_key": "file_v2_demo"
                }),
            },
            &config,
        )
        .expect("feishu messages reply tool should support file_key");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["delivery"]["message_id"], "om_reply_file_1");
        assert_eq!(outcome.payload["delivery"]["msg_type"], "file");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_parent_file/reply"
        );
        assert!(requests[1].body.contains("\"msg_type\":\"file\""));
        assert!(
            requests[1]
                .body
                .contains("\\\"file_key\\\":\\\"file_v2_demo\\\"")
        );

        server.abort();
    }

    #[cfg(all(
        feature = "feishu-integration",
        feature = "channel-feishu",
        feature = "tool-file"
    ))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_reply_tool_uploads_file_path_under_safe_file_root_and_replies_with_file_message()
     {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-reply-file-path");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let file_root = temp_dir.join("uploads-root");
        fs::create_dir_all(&file_root).expect("create file root");
        let file_path = file_root.join("docs/spec-sheet.pdf");
        fs::create_dir_all(file_path.parent().expect("file path parent"))
            .expect("create file parent");
        fs::write(&file_path, b"pdf-demo-bytes").expect("write file fixture");

        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-reply-file-path"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/files",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "file_key": "file_uploaded_from_path"
                                }
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_parent_file_path/reply",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_reply_file_path_1",
                                    "root_id": "om_parent_file_path",
                                    "parent_id": "om_parent_file_path"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-reply-file-path",
            &["offline_access", "im:message:send_as_bot"],
        );
        let mut config = build_feishu_tool_runtime_config(base_url, &sqlite_path);
        config.file_root = Some(file_root);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.reply".to_owned(),
                payload: serde_json::json!({
                    "message_id": "om_parent_file_path",
                    "file_path": "docs/spec-sheet.pdf",
                    "file_type": "stream"
                }),
            },
            &config,
        )
        .expect("feishu messages reply tool should upload file path");

        assert_eq!(outcome.status, "ok");
        assert_eq!(
            outcome.payload["delivery"]["message_id"],
            "om_reply_file_path_1"
        );
        assert_eq!(outcome.payload["delivery"]["msg_type"], "file");

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests[0].path,
            "/open-apis/auth/v3/tenant_access_token/internal"
        );
        assert_eq!(requests[1].path, "/open-apis/im/v1/files");
        assert_eq!(
            requests[2].path,
            "/open-apis/im/v1/messages/om_parent_file_path/reply"
        );
        assert!(requests[1].body.contains("name=\"file_type\""));
        assert!(requests[1].body.contains("stream"));
        assert!(requests[1].body.contains("filename=\"spec-sheet.pdf\""));
        assert!(requests[2].body.contains("\"msg_type\":\"file\""));
        assert!(
            requests[2]
                .body
                .contains("\\\"file_key\\\":\\\"file_uploaded_from_path\\\"")
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_reply_tool_defaults_message_id_from_internal_ingress() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-reply-ingress");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-reply-ingress"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_source_ingress/reply",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_reply_ingress_1",
                                    "root_id": "om_source_ingress",
                                    "parent_id": "om_source_ingress"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-reply-ingress",
            &["offline_access", "im:message:send_as_bot"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.reply".to_owned(),
                payload: serde_json::json!({
                    "text": "reply from ingress",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "conversation_id": "oc_demo"
                            },
                            "delivery": {
                                "source_message_id": "om_source_ingress",
                                "parent_message_id": "om_parent_fallback"
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect("feishu messages reply tool should default message id from internal ingress");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["delivery"]["mode"], "reply");
        assert_eq!(
            outcome.payload["delivery"]["reply_to_message_id"],
            "om_source_ingress"
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_source_ingress/reply"
        );
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-reply-ingress")
        );
        assert!(requests[1].body.contains("\"msg_type\":\"text\""));
        assert!(
            requests[1]
                .body
                .contains("\\\"text\\\":\\\"reply from ingress\\\"")
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_reply_tool_prefers_configured_account_from_internal_ingress() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-reply-configured-ingress");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-reply-configured"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_source_configured/reply",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_reply_configured_1",
                                    "root_id": "om_source_configured",
                                    "parent_id": "om_source_configured"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant_for_account(
            &sqlite_path,
            "feishu_shared",
            "ou_shared",
            "u-token-reply-configured",
            &["offline_access", "im:message:send_as_bot"],
        );
        let config = runtime_config::ToolRuntimeConfig {
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([
                        (
                            "work".to_owned(),
                            crate::config::FeishuAccountConfig {
                                account_id: Some("feishu_shared".to_owned()),
                                app_id: Some("cli_work".to_owned()),
                                app_secret: Some("app-secret-work".to_owned()),
                                base_url: Some(base_url),
                                ..crate::config::FeishuAccountConfig::default()
                            },
                        ),
                        (
                            "alerts".to_owned(),
                            crate::config::FeishuAccountConfig {
                                account_id: Some("feishu_shared".to_owned()),
                                app_id: Some("cli_alerts".to_owned()),
                                app_secret: Some("app-secret-alerts".to_owned()),
                                base_url: Some("http://127.0.0.1:9".to_owned()),
                                ..crate::config::FeishuAccountConfig::default()
                            },
                        ),
                    ]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.reply".to_owned(),
                payload: serde_json::json!({
                    "text": "reply from configured ingress",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "configured_account_id": "work",
                                "account_id": "feishu_shared",
                                "conversation_id": "oc_configured_reply"
                            },
                            "delivery": {
                                "source_message_id": "om_source_configured"
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect("feishu messages reply tool should use configured account from ingress");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["configured_account"], "work");
        assert_eq!(
            outcome.payload["delivery"]["reply_to_message_id"],
            "om_source_configured"
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_source_configured/reply"
        );
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-reply-configured")
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_messages_reply_tool_falls_back_to_parent_message_id_from_internal_ingress() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("messages-reply-parent-ingress");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-reply-parent"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/im/v1/messages/om_parent_ingress/reply",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "data": {
                                    "message_id": "om_reply_parent_1",
                                    "root_id": "om_parent_ingress",
                                    "parent_id": "om_parent_ingress"
                                }
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-reply-parent",
            &["offline_access", "im:message:send_as_bot"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.reply".to_owned(),
                payload: serde_json::json!({
                    "text": "reply from parent fallback",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "conversation_id": "oc_demo"
                            },
                            "delivery": {
                                "parent_message_id": "om_parent_ingress"
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect("feishu messages reply tool should fall back to parent message id");

        assert_eq!(outcome.status, "ok");
        assert_eq!(
            outcome.payload["delivery"]["reply_to_message_id"],
            "om_parent_ingress"
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].path,
            "/open-apis/im/v1/messages/om_parent_ingress/reply"
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_card_update_tool_requires_callback_token_without_internal_context() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("card-update-missing-token");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let error = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.card.update".to_owned(),
                payload: serde_json::json!({
                    "card": {
                        "elements": [{
                            "tag": "markdown",
                            "content": "done"
                        }]
                    }
                }),
            },
            &config,
        )
        .expect_err("missing callback token should fail");

        assert!(error.contains("feishu.card.update requires payload.callback_token"));
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_card_update_tool_requires_card_or_markdown_payload() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("card-update-missing-card-and-markdown");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let error = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.card.update".to_owned(),
                payload: serde_json::json!({
                    "callback_token": "callback-token-1"
                }),
            },
            &config,
        )
        .expect_err("missing card and markdown should fail");

        assert!(error.contains("feishu.card.update requires payload.card or payload.markdown"));
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_card_update_tool_rejects_card_and_markdown_together() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("card-update-card-and-markdown-conflict");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let error = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.card.update".to_owned(),
                payload: serde_json::json!({
                    "callback_token": "callback-token-1",
                    "markdown": "approved",
                    "card": {
                        "elements": [{
                            "tag": "markdown",
                            "content": "approved"
                        }]
                    }
                }),
            },
            &config,
        )
        .expect_err("card and markdown together should fail");

        assert!(error.contains(
            "feishu.card.update accepts exactly one of payload.card or payload.markdown"
        ));
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_card_update_tool_defers_when_callback_context_requests_post_callback_dispatch() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("card-update-deferred");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.card.update".to_owned(),
                payload: serde_json::json!({
                    "card": {
                        "elements": [{
                            "tag": "markdown",
                            "content": "approved"
                        }]
                    },
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "account_id": "feishu_main",
                                "conversation_id": "oc_card_callback"
                            }
                        },
                        "feishu_callback": {
                            "callback_token": "callback-token-deferred",
                            "operator_open_id": "ou_card_operator",
                            "deferred_context_id": "evt_card_deferred_1"
                        }
                    }
                }),
            },
            &config,
        )
        .expect("callback-scoped card update should queue deferred work");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["update"]["mode"], "deferred");
        assert_eq!(
            outcome.payload["update"]["open_ids"],
            serde_json::json!(["ou_card_operator"])
        );

        let drained = drain_deferred_feishu_card_updates("evt_card_deferred_1");
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].token, "callback-token-deferred");
        assert_eq!(drained[0].open_ids, vec!["ou_card_operator"]);
        assert_eq!(
            drained[0].card,
            serde_json::json!({
                "elements": [{
                    "tag": "markdown",
                    "content": "approved"
                }]
            })
        );
        assert!(drain_deferred_feishu_card_updates("evt_card_deferred_1").is_empty());
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_card_update_tool_rejects_more_than_two_deferred_updates_per_callback_context() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("card-update-deferred-limit");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let build_request = || loongclaw_contracts::ToolCoreRequest {
            tool_name: "feishu.card.update".to_owned(),
            payload: serde_json::json!({
                "card": {
                    "elements": [{
                        "tag": "markdown",
                        "content": "approved"
                    }]
                },
                "_loongclaw": {
                    "ingress": {
                        "source": "channel",
                        "channel": {
                            "platform": "feishu",
                            "account_id": "feishu_main",
                            "conversation_id": "oc_card_callback"
                        }
                    },
                    "feishu_callback": {
                        "callback_token": "callback-token-deferred-limit",
                        "operator_open_id": "ou_card_operator",
                        "deferred_context_id": "evt_card_deferred_limit_1"
                    }
                }
            }),
        };

        let first = execute_tool_core_with_test_context(build_request(), &config)
            .expect("first callback-scoped card update should queue deferred work");
        assert_eq!(first.payload["update"]["callback_token_use_count"], 1);
        assert_eq!(first.payload["update"]["callback_token_use_limit"], 2);

        let second = execute_tool_core_with_test_context(build_request(), &config)
            .expect("second callback-scoped card update should stay within token budget");
        assert_eq!(second.payload["update"]["callback_token_use_count"], 2);
        assert_eq!(second.payload["update"]["callback_token_use_limit"], 2);

        let error = execute_tool_core_with_test_context(build_request(), &config)
            .expect_err("third callback-scoped card update should exceed token budget");
        assert!(error.contains("callback token can only be used twice"));

        let drained = drain_deferred_feishu_card_updates("evt_card_deferred_limit_1");
        assert_eq!(drained.len(), 2);
        assert!(drain_deferred_feishu_card_updates("evt_card_deferred_limit_1").is_empty());
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_card_update_tool_accepts_markdown_shortcut() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("card-update-markdown-shortcut");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-card-update-markdown"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/interactive/v1/card/update",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "msg": "ok"
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.card.update".to_owned(),
                payload: serde_json::json!({
                    "callback_token": "callback-token-markdown",
                    "shared": true,
                    "markdown": "Approved for everyone"
                }),
            },
            &config,
        )
        .expect("markdown shortcut should build a standard card");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["update"]["shared"], serde_json::json!(true));

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/open-apis/interactive/v1/card/update");
        assert!(
            requests[1]
                .body
                .contains("\"token\":\"callback-token-markdown\"")
        );
        assert!(requests[1].body.contains("\"wide_screen_mode\":true"));
        assert!(
            requests[1]
                .body
                .contains("\"content\":\"Approved for everyone\"")
        );
        assert!(
            !requests[1].body.contains("\"open_ids\""),
            "shared markdown card update should not send open_ids"
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_card_update_tool_defaults_account_callback_token_and_open_ids_from_internal_context()
     {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("card-update-ingress-defaults");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-card-update"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/interactive/v1/card/update",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "msg": "ok"
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.card.update".to_owned(),
                payload: serde_json::json!({
                    "card": {
                        "elements": [{
                            "tag": "markdown",
                            "content": "approved"
                        }]
                    },
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "account_id": "feishu_main",
                                "conversation_id": "oc_card_callback"
                            }
                        },
                        "feishu_callback": {
                            "callback_token": "callback-token-from-ingress",
                            "open_message_id": "om_card_callback_1",
                            "open_chat_id": "oc_card_callback",
                            "operator_open_id": "ou_card_operator"
                        }
                    }
                }),
            },
            &config,
        )
        .expect("feishu card update tool should default from internal callback context");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["account_id"], "feishu_main");
        assert_eq!(outcome.payload["update"]["message"], "ok");
        assert_eq!(
            outcome.payload["update"]["open_ids"],
            serde_json::json!(["ou_card_operator"])
        );
        assert_eq!(
            outcome.payload["update"]["callback_open_message_id"],
            "om_card_callback_1"
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].path,
            "/open-apis/auth/v3/tenant_access_token/internal"
        );
        assert_eq!(requests[1].path, "/open-apis/interactive/v1/card/update");
        assert_eq!(
            requests[1].authorization.as_deref(),
            Some("Bearer t-token-card-update")
        );
        assert!(
            requests[1]
                .body
                .contains("\"token\":\"callback-token-from-ingress\"")
        );
        assert!(
            requests[1]
                .body
                .contains("\"open_ids\":[\"ou_card_operator\"]")
        );
        assert!(requests[1].body.contains("\"content\":\"approved\""));

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_card_update_tool_explicit_callback_token_and_open_ids_override_internal_defaults()
     {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("card-update-explicit-overrides");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-card-update-explicit"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/interactive/v1/card/update",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "msg": "ok"
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.card.update".to_owned(),
                payload: serde_json::json!({
                    "account_id": "feishu_main",
                    "callback_token": "callback-token-explicit",
                    "open_ids": [],
                    "card": {
                        "elements": [{
                            "tag": "markdown",
                            "content": "shared update"
                        }]
                    },
                    "_loongclaw": {
                        "feishu_callback": {
                            "callback_token": "callback-token-from-ingress",
                            "operator_open_id": "ou_card_operator"
                        }
                    }
                }),
            },
            &config,
        )
        .expect("explicit payload should override internal callback defaults");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["update"]["open_ids"], serde_json::json!([]));

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/open-apis/interactive/v1/card/update");
        assert!(
            requests[1]
                .body
                .contains("\"token\":\"callback-token-explicit\"")
        );
        assert!(
            !requests[1].body.contains("callback-token-from-ingress"),
            "explicit callback token should override internal token"
        );
        assert!(
            !requests[1].body.contains("\"open_ids\""),
            "explicit empty open_ids should suppress exclusive-card defaults"
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_card_update_tool_shared_flag_suppresses_callback_open_id_default() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("card-update-shared-flag");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "tenant_access_token": "t-token-card-update-shared"
                            }))
                        }
                    }
                }),
            )
            .route(
                "/open-apis/interactive/v1/card/update",
                post({
                    let state = state.clone();
                    move |request: Request| {
                        let state = state.clone();
                        async move {
                            record_feishu_tool_request(State(state), request).await;
                            Json(serde_json::json!({
                                "code": 0,
                                "msg": "ok"
                            }))
                        }
                    }
                }),
            );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.card.update".to_owned(),
                payload: serde_json::json!({
                    "shared": true,
                    "card": {
                        "elements": [{
                            "tag": "markdown",
                            "content": "shared update explicit"
                        }]
                    },
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "account_id": "feishu_main",
                                "conversation_id": "oc_card_callback"
                            }
                        },
                        "feishu_callback": {
                            "callback_token": "callback-token-shared",
                            "operator_open_id": "ou_card_operator"
                        }
                    }
                }),
            },
            &config,
        )
        .expect("shared card update should suppress callback operator default");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["update"]["open_ids"], serde_json::json!([]));
        assert_eq!(outcome.payload["update"]["shared"], serde_json::json!(true));

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/open-apis/interactive/v1/card/update");
        assert!(
            !requests[1].body.contains("\"open_ids\""),
            "shared card update should not send callback operator defaults"
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_card_update_tool_rejects_nonempty_open_ids_when_shared() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("card-update-shared-open-ids-conflict");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let error = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.card.update".to_owned(),
                payload: serde_json::json!({
                    "callback_token": "callback-token-shared-conflict",
                    "shared": true,
                    "open_ids": ["ou_card_operator"],
                    "card": {
                        "elements": [{
                            "tag": "markdown",
                            "content": "shared conflict"
                        }]
                    }
                }),
            },
            &config,
        )
        .expect_err("shared card update should reject non-empty open_ids");

        assert!(
            error
                .contains("payload.shared=true cannot be combined with non-empty payload.open_ids")
        );
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_direct_tool_execution_rejects_reserved_internal_payload() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("reserved-internal-payload");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let error = super::execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.send".to_owned(),
                payload: serde_json::json!({
                    "text": "ship by ingress",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "account_id": "feishu_main",
                                "conversation_id": "oc_ingress_send"
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect_err("direct execution should reject reserved internal payloads");

        assert!(
            error.contains("payload._loongclaw is reserved for trusted internal tool context"),
            "error={error}"
        );
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_messages_send_tool_ignores_non_feishu_internal_ingress_context() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("messages-send-non-feishu-ingress");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let _store =
            seed_feishu_tool_grant(&sqlite_path, "u-token-send-plain", &["offline_access"]);
        let config =
            build_feishu_tool_runtime_config("http://127.0.0.1:9".to_owned(), &sqlite_path);

        let error = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.send".to_owned(),
                payload: serde_json::json!({
                    "text": "ship by ingress",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "telegram",
                                "conversation_id": "chat_telegram_1"
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect_err("non-feishu ingress should not satisfy receive_id defaults");

        assert!(
            error.contains("feishu.messages.send requires payload.receive_id"),
            "error={error}"
        );
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[test]
    fn feishu_messages_send_tool_reports_ambiguous_runtime_account_from_internal_ingress() {
        use std::fs;

        let temp_dir = unique_feishu_tool_temp_dir("messages-send-ambiguous-runtime-account");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let config = runtime_config::ToolRuntimeConfig {
            feishu: Some(runtime_config::FeishuToolRuntimeConfig {
                channel: crate::config::FeishuChannelConfig {
                    enabled: true,
                    accounts: BTreeMap::from([
                        (
                            "work".to_owned(),
                            crate::config::FeishuAccountConfig {
                                account_id: Some("feishu_shared".to_owned()),
                                app_id: Some("cli_work".to_owned()),
                                app_secret: Some("app-secret-work".to_owned()),
                                base_url: Some("http://127.0.0.1:9".to_owned()),
                                ..crate::config::FeishuAccountConfig::default()
                            },
                        ),
                        (
                            "alerts".to_owned(),
                            crate::config::FeishuAccountConfig {
                                account_id: Some("feishu_shared".to_owned()),
                                app_id: Some("cli_alerts".to_owned()),
                                app_secret: Some("app-secret-alerts".to_owned()),
                                base_url: Some("http://127.0.0.1:9".to_owned()),
                                ..crate::config::FeishuAccountConfig::default()
                            },
                        ),
                    ]),
                    ..crate::config::FeishuChannelConfig::default()
                },
                integration: crate::config::FeishuIntegrationConfig {
                    sqlite_path: sqlite_path.display().to_string(),
                    ..crate::config::FeishuIntegrationConfig::default()
                },
            }),
            ..runtime_config::ToolRuntimeConfig::default()
        };

        let error = execute_tool_core_with_test_context(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.messages.send".to_owned(),
                payload: serde_json::json!({
                    "text": "ship by ingress",
                    "_loongclaw": {
                        "ingress": {
                            "source": "channel",
                            "channel": {
                                "platform": "feishu",
                                "account_id": "feishu_shared",
                                "conversation_id": "oc_ingress_send"
                            }
                        }
                    }
                }),
            },
            &config,
        )
        .expect_err("ambiguous runtime account should fail");

        assert!(error.contains("requested Feishu runtime account `feishu_shared` is ambiguous"));
        assert!(error.contains("Use configured_account_id `alerts` or `work` to disambiguate"));
        assert!(error.contains("work"));
        assert!(error.contains("alerts"));
        assert!(error.contains("payload.account_id"));
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_calendar_freebusy_tool_defaults_selected_open_id_and_user_token() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("calendar-freebusy");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new().route(
            "/open-apis/calendar/v4/freebusy/list",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "freebusy_list": [{
                                    "start_time": "2026-03-12T09:00:00+08:00",
                                    "end_time": "2026-03-12T10:00:00+08:00",
                                    "rsvp_status": "busy"
                                }]
                            }
                        }))
                    }
                }
            }),
        );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-freebusy",
            &["offline_access", "calendar:calendar:readonly"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.calendar.freebusy".to_owned(),
                payload: serde_json::json!({
                    "time_min": "2026-03-12T09:00:00+08:00",
                    "time_max": "2026-03-12T10:00:00+08:00",
                    "include_external_calendar": true,
                    "only_busy": true,
                    "need_rsvp_status": true
                }),
            },
            &config,
        )
        .expect("feishu calendar freebusy tool should succeed");

        assert_eq!(
            outcome.payload["result"]["freebusy_list"][0]["rsvp_status"],
            "busy"
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer u-token-freebusy")
        );
        assert!(
            requests[0]
                .query
                .as_deref()
                .is_some_and(|query| { query.contains("user_id_type=open_id") })
        );
        assert!(requests[0].body.contains("\"user_id\":\"ou_123\""));
        assert!(
            requests[0]
                .body
                .contains("\"include_external_calendar\":true")
        );

        server.abort();
    }

    #[cfg(all(feature = "feishu-integration", feature = "channel-feishu"))]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn feishu_calendar_list_tool_primary_defaults_open_id_and_user_token() {
        use std::fs;

        use axum::{
            Json, Router,
            extract::{Request, State},
            routing::post,
        };

        let temp_dir = unique_feishu_tool_temp_dir("calendar-list");
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let sqlite_path = temp_dir.join("feishu.sqlite3");
        let requests =
            std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<FeishuToolMockRequest>::new()));
        let state = FeishuToolMockServerState {
            requests: requests.clone(),
        };
        let router = Router::new().route(
            "/open-apis/calendar/v4/calendars/primary",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        record_feishu_tool_request(State(state), request).await;
                        Json(serde_json::json!({
                            "code": 0,
                            "data": {
                                "calendars": [{
                                    "calendar": {
                                        "calendar_id": "cal_primary",
                                        "summary": "Alice Primary",
                                        "permissions": "private"
                                    },
                                    "user_id": "ou_123"
                                }]
                            }
                        }))
                    }
                }
            }),
        );
        let (base_url, server) = spawn_feishu_tool_mock_server(router).await;
        let _store = seed_feishu_tool_grant(
            &sqlite_path,
            "u-token-calendar",
            &["offline_access", "calendar:calendar:readonly"],
        );
        let config = build_feishu_tool_runtime_config(base_url, &sqlite_path);

        let outcome = execute_tool_core_with_config(
            loongclaw_contracts::ToolCoreRequest {
                tool_name: "feishu.calendar.list".to_owned(),
                payload: serde_json::json!({
                    "primary": true
                }),
            },
            &config,
        )
        .expect("feishu calendar list tool should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["primary"], true);
        assert_eq!(
            outcome.payload["calendars"]["calendars"][0]["calendar"]["calendar_id"],
            "cal_primary"
        );

        let requests = requests.lock().await.clone();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer u-token-calendar")
        );
        assert_eq!(requests[0].path, "/open-apis/calendar/v4/calendars/primary");
        let query = requests[0].query.as_deref().unwrap_or_default();
        assert!(query.contains("user_id_type=open_id"));

        server.abort();
    }

    #[test]
    fn provider_switch_tool_updates_target_config_and_reports_active_profile() {
        use std::{
            fs,
            path::PathBuf,
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        let root = unique_temp_dir("loongclaw-tool-provider-switch");
        fs::create_dir_all(&root).expect("create fixture root");
        let config_path = root.join("loongclaw.toml");

        let mut config = crate::config::LoongClawConfig::default();
        let mut openai =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
        openai.model = "gpt-5".to_owned();
        config.set_active_provider_profile(
            "openai-gpt-5",
            crate::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: openai.clone(),
            },
        );
        let mut deepseek =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Deepseek);
        deepseek.model = "deepseek-chat".to_owned();
        config.providers.insert(
            "deepseek-chat".to_owned(),
            crate::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: deepseek,
            },
        );
        config.provider = openai;
        config.active_provider = Some("openai-gpt-5".to_owned());
        fs::write(
            &config_path,
            crate::config::render(&config).expect("render provider config"),
        )
        .expect("write provider config");

        let runtime_config = runtime_config::ToolRuntimeConfig {
            shell_allow: BTreeSet::new(),
            file_root: Some(root.clone()),
            config_path: Some(config_path.clone()),
            external_skills: Default::default(),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "provider.switch".to_owned(),
                payload: json!({
                    "selector": "deepseek",
                    "config_path": "loongclaw.toml"
                }),
            },
            &runtime_config,
        )
        .expect("provider switch should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["tool_name"], "provider.switch");
        assert_eq!(outcome.payload["changed"], true);
        assert_eq!(outcome.payload["previous_active_provider"], "openai-gpt-5");
        assert_eq!(outcome.payload["active_provider"], "deepseek-chat");

        let (_, reloaded) =
            crate::config::load(Some(config_path.to_str().expect("utf8 config path")))
                .expect("load");
        assert_eq!(reloaded.active_provider_id(), Some("deepseek-chat"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn provider_switch_tool_accepts_unique_model_selector() {
        use std::{
            fs,
            path::PathBuf,
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        let root = unique_temp_dir("loongclaw-tool-provider-switch-model");
        fs::create_dir_all(&root).expect("create fixture root");
        let config_path = root.join("loongclaw.toml");

        let mut config = crate::config::LoongClawConfig::default();
        let mut openai =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
        openai.model = "gpt-5".to_owned();
        config.set_active_provider_profile(
            "openai-main",
            crate::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: openai.clone(),
            },
        );
        let mut deepseek =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Deepseek);
        deepseek.model = "deepseek-chat".to_owned();
        config.providers.insert(
            "deepseek-cn".to_owned(),
            crate::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: deepseek,
            },
        );
        config.provider = openai;
        config.active_provider = Some("openai-main".to_owned());
        fs::write(
            &config_path,
            crate::config::render(&config).expect("render provider config"),
        )
        .expect("write provider config");

        let runtime_config = runtime_config::ToolRuntimeConfig {
            shell_allow: BTreeSet::new(),
            file_root: Some(root.clone()),
            config_path: Some(config_path.clone()),
            external_skills: Default::default(),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "provider.switch".to_owned(),
                payload: json!({
                    "selector": "deepseek-chat"
                }),
            },
            &runtime_config,
        )
        .expect("provider switch by model should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["changed"], true);
        assert_eq!(outcome.payload["previous_active_provider"], "openai-main");
        assert_eq!(outcome.payload["active_provider"], "deepseek-cn");

        let (_, reloaded) =
            crate::config::load(Some(config_path.to_str().expect("utf8 config path")))
                .expect("load");
        assert_eq!(reloaded.active_provider_id(), Some("deepseek-cn"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn provider_switch_without_selector_reports_current_provider_state() {
        use std::{
            fs,
            path::PathBuf,
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        let root = unique_temp_dir("loongclaw-tool-provider-switch-inspect");
        fs::create_dir_all(&root).expect("create fixture root");
        let config_path = root.join("loongclaw.toml");

        let mut config = crate::config::LoongClawConfig::default();
        let mut openai =
            crate::config::ProviderConfig::fresh_for_kind(crate::config::ProviderKind::Openai);
        openai.model = "gpt-5".to_owned();
        config.set_active_provider_profile(
            "openai-gpt-5",
            crate::config::ProviderProfileConfig {
                default_for_kind: true,
                provider: openai,
            },
        );
        fs::write(
            &config_path,
            crate::config::render(&config).expect("render provider config"),
        )
        .expect("write provider config");

        let runtime_config = runtime_config::ToolRuntimeConfig {
            shell_allow: BTreeSet::new(),
            file_root: Some(root.clone()),
            config_path: Some(config_path.clone()),
            external_skills: Default::default(),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "provider.switch".to_owned(),
                payload: json!({}),
            },
            &runtime_config,
        )
        .expect("provider switch inspect should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["changed"], false);
        assert_eq!(outcome.payload["active_provider"], "openai-gpt-5");
        assert_eq!(outcome.payload["selector"], Value::Null);
        assert_eq!(outcome.payload["profiles"][0]["profile_id"], "openai-gpt-5");
        assert_eq!(
            outcome.payload["profiles"][0]["accepted_selectors"],
            json!(["openai-gpt-5", "gpt-5", "openai"])
        );

        let (_, reloaded) =
            crate::config::load(Some(config_path.to_str().expect("utf8 config path")))
                .expect("load");
        assert_eq!(reloaded.active_provider_id(), Some("openai-gpt-5"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unknown_tool_returns_hard_error_code() {
        let err = execute_tool_core(ToolCoreRequest {
            tool_name: "unknown".to_owned(),
            payload: json!({"hello":"world"}),
        })
        .expect_err("unknown tool should return an error");
        assert!(
            err.contains("tool_not_found"),
            "error should contain tool_not_found, got: {err}"
        );
    }

    #[test]
    fn claw_import_plan_mode_returns_nativeized_preview() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-plan");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "SOUL.md",
            "# Soul\n\nAlways prefer concise shell output. updated by nanobot.\n",
        );
        write_file(
            &root,
            "IDENTITY.md",
            "# Identity\n\n- Motto: your nanobot agent for deploys\n",
        );

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "plan",
                    "source": "nanobot",
                    "input_path": "."
                }),
            },
            &config,
        )
        .expect("claw import plan should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["tool_name"], "claw.import");
        assert_eq!(outcome.payload["mode"], "plan");
        assert_eq!(outcome.payload["source"], "nanobot");
        assert_eq!(
            outcome.payload["config_preview"]["prompt_pack_id"],
            "loongclaw-core-v1"
        );
        assert_eq!(
            outcome.payload["config_preview"]["memory_profile"],
            "profile_plus_window"
        );
        assert!(
            outcome.payload["config_preview"]["system_prompt_addendum"]
                .as_str()
                .expect("prompt addendum should exist")
                .contains("LoongClaw")
        );
        assert!(
            outcome.payload["config_preview"]["profile_note"]
                .as_str()
                .expect("profile note should exist")
                .contains("LoongClaw")
        );
        assert_eq!(outcome.payload["config_written"], false);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_apply_mode_writes_target_config() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-apply");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(
            &root,
            "SOUL.md",
            "# Soul\n\nAlways prefer concise shell output. updated by nanobot.\n",
        );
        write_file(
            &root,
            "IDENTITY.md",
            "# Identity\n\n- Motto: your nanobot agent for deploys\n",
        );

        let output_path = root.join("generated").join("loongclaw.toml");
        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw_import".to_owned(),
                payload: json!({
                    "mode": "apply",
                    "source": "nanobot",
                    "input_path": ".",
                    "output_path": "generated/loongclaw.toml",
                    "force": true
                }),
            },
            &config,
        )
        .expect("claw import apply should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "apply");
        assert_eq!(outcome.payload["config_written"], true);
        assert_eq!(
            outcome.payload["next_step"]
                .as_str()
                .expect("next_step should be present")
                .split_whitespace()
                .next(),
            Some("loongclaw")
        );
        assert_eq!(
            outcome.payload["output_path"]
                .as_str()
                .expect("output path should exist"),
            fs::canonicalize(&output_path)
                .expect("output path should canonicalize")
                .display()
                .to_string()
        );

        let raw = fs::read_to_string(&output_path).expect("output config should exist");
        assert!(raw.contains("prompt_pack_id = \"loongclaw-core-v1\""));
        assert!(raw.contains("profile = \"profile_plus_window\""));
        assert!(raw.contains("LoongClaw"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_discover_mode_returns_detected_sources() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-discover");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- Role: Release copilot\n- Priority: stability first\n",
        );

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "discover",
                    "input_path": "."
                }),
            },
            &config,
        )
        .expect("claw import discover should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "discover");
        assert_eq!(outcome.payload["sources"][0]["source_id"], "openclaw");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_plan_many_mode_returns_source_summaries_and_recommendation() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-plan-many");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- Role: Release copilot\n- Priority: stability first\n",
        );

        let nanobot_root = root.join("nanobot");
        fs::create_dir_all(&nanobot_root).expect("create nanobot root");
        write_file(
            &nanobot_root,
            "IDENTITY.md",
            "# Identity\n\n- Motto: your nanobot agent for deploys\n",
        );

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "plan_many",
                    "input_path": "."
                }),
            },
            &config,
        )
        .expect("claw import plan_many should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "plan_many");
        assert_eq!(outcome.payload["plans"][0]["source_id"], "openclaw");
        assert_eq!(outcome.payload["recommendation"]["source_id"], "openclaw");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_merge_profiles_mode_preserves_prompt_owner() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-merge-profiles");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n- tone: steady\n",
        );

        let nanobot_root = root.join("nanobot");
        fs::create_dir_all(&nanobot_root).expect("create nanobot root");
        write_file(
            &nanobot_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n- region: apac\n",
        );

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "merge_profiles",
                    "input_path": "."
                }),
            },
            &config,
        )
        .expect("claw import merge_profiles should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "merge_profiles");
        assert_eq!(
            outcome.payload["result"]["prompt_owner_source_id"],
            "openclaw"
        );
        assert!(
            outcome.payload["result"]["merged_profile_note"]
                .as_str()
                .expect("merged profile note should be present")
                .contains("region: apac")
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_map_external_skills_mode_returns_mapping_plan() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-map-external-skills");
        fs::create_dir_all(&root).expect("create fixture root");
        write_file(&root, "SKILLS.md", "# Skills\n\n- custom/skill-a\n");
        fs::create_dir_all(root.join(".codex/skills")).expect("create codex skills dir");

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "map_external_skills",
                    "input_path": "."
                }),
            },
            &config,
        )
        .expect("claw import map_external_skills should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "map_external_skills");
        assert_eq!(outcome.payload["result"]["artifact_count"], 2);
        assert_eq!(
            outcome.payload["result"]["declared_skills"][0],
            "custom/skill-a"
        );
        assert_eq!(
            outcome.payload["result"]["resolved_skills"][0],
            "custom/skill-a"
        );
        assert!(
            outcome.payload["result"]["profile_note_addendum"]
                .as_str()
                .expect("profile note addendum should exist")
                .contains("Imported External Skills Artifacts")
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_apply_selected_mode_writes_manifest_and_backup() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-apply-selected");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n- tone: steady\n",
        );

        let output_path = root.join("loongclaw.toml");
        let original_body = crate::config::render(&crate::config::LoongClawConfig::default())
            .expect("render default config");
        fs::write(&output_path, &original_body).expect("write original config");

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "apply_selected",
                    "input_path": ".",
                    "output_path": "loongclaw.toml",
                    "source_id": "openclaw"
                }),
            },
            &config,
        )
        .expect("claw import apply_selected should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(outcome.payload["mode"], "apply_selected");
        assert!(
            Path::new(
                outcome.payload["result"]["backup_path"]
                    .as_str()
                    .expect("backup path should be present")
            )
            .exists()
        );
        assert!(
            Path::new(
                outcome.payload["result"]["manifest_path"]
                    .as_str()
                    .expect("manifest path should be present")
            )
            .exists()
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_apply_selected_mode_can_apply_external_skills_plan() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-apply-selected-external");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n- tone: steady\n",
        );
        write_file(&root, "SKILLS.md", "# Skills\n\n- custom/skill-a\n");

        let output_path = root.join("loongclaw.toml");

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        let outcome = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "apply_selected",
                    "input_path": ".",
                    "output_path": "loongclaw.toml",
                    "source_id": "openclaw",
                    "apply_external_skills_plan": true
                }),
            },
            &config,
        )
        .expect("claw import apply_selected with external skills should succeed");

        assert_eq!(outcome.status, "ok");
        assert_eq!(
            outcome.payload["result"]["external_skill_artifact_count"],
            1
        );
        assert_eq!(
            outcome.payload["result"]["external_skill_entries_applied"],
            3
        );
        assert!(
            outcome.payload["result"]["external_skills_manifest_path"]
                .as_str()
                .is_some(),
            "external skills manifest path should exist"
        );
        let raw = fs::read_to_string(&output_path).expect("read output config");
        assert!(raw.contains("Imported External Skills Artifacts"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claw_import_rollback_last_apply_restores_original_config() {
        use std::{
            fs,
            path::{Path, PathBuf},
            time::{SystemTime, UNIX_EPOCH},
        };

        fn unique_temp_dir(prefix: &str) -> PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("{prefix}-{nanos}"))
        }

        fn write_file(root: &Path, relative: &str, content: &str) {
            let path = root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent directory");
            }
            fs::write(path, content).expect("write fixture");
        }

        let root = unique_temp_dir("loongclaw-tool-import-rollback-selected");
        fs::create_dir_all(&root).expect("create fixture root");

        let openclaw_root = root.join("openclaw-workspace");
        fs::create_dir_all(&openclaw_root).expect("create openclaw root");
        write_file(
            &openclaw_root,
            "SOUL.md",
            "# Soul\n\nPrefer direct answers and keep OpenClaw style concise.\n",
        );
        write_file(
            &openclaw_root,
            "IDENTITY.md",
            "# Identity\n\n- role: release copilot\n- tone: steady\n",
        );

        let output_path = root.join("loongclaw.toml");
        let original_body = crate::config::render(&crate::config::LoongClawConfig::default())
            .expect("render default config");
        fs::write(&output_path, &original_body).expect("write original config");

        let config = runtime_config::ToolRuntimeConfig {
            file_root: Some(root.clone()),
            ..runtime_config::ToolRuntimeConfig::default()
        };
        execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "apply_selected",
                    "input_path": ".",
                    "output_path": "loongclaw.toml",
                    "source_id": "openclaw"
                }),
            },
            &config,
        )
        .expect("claw import apply_selected should succeed");

        let rollback = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "claw.import".to_owned(),
                payload: json!({
                    "mode": "rollback_last_apply",
                    "output_path": "loongclaw.toml"
                }),
            },
            &config,
        )
        .expect("claw import rollback_last_apply should succeed");

        assert_eq!(rollback.status, "ok");
        assert!(
            rollback.payload["rolled_back"]
                .as_bool()
                .expect("rolled_back flag should exist")
        );
        assert_eq!(
            fs::read_to_string(&output_path).expect("read restored config"),
            original_body
        );

        fs::remove_dir_all(&root).ok();
    }

    // --- Kernel-routed tool tests ---

    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use loongclaw_contracts::{ExecutionRoute, HarnessKind, ToolPlaneError};
    use loongclaw_kernel::{
        CoreToolAdapter, FixedClock, InMemoryAuditSink, LoongClawKernel, StaticPolicyEngine,
        VerticalPackManifest,
    };

    struct SharedTestToolAdapter {
        invocations: Arc<Mutex<Vec<ToolCoreRequest>>>,
    }

    #[async_trait]
    impl CoreToolAdapter for SharedTestToolAdapter {
        fn name(&self) -> &str {
            "test-tool-shared"
        }

        async fn execute_core_tool(
            &self,
            request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, ToolPlaneError> {
            self.invocations
                .lock()
                .expect("invocations lock")
                .push(request);
            Ok(ToolCoreOutcome {
                status: "ok".to_owned(),
                payload: json!({}),
            })
        }
    }

    fn build_tool_kernel_context(
        audit: Arc<InMemoryAuditSink>,
        capabilities: BTreeSet<Capability>,
    ) -> (KernelContext, Arc<Mutex<Vec<ToolCoreRequest>>>) {
        let clock = Arc::new(FixedClock::new(1_700_000_000));
        let mut kernel = LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit);

        let pack = VerticalPackManifest {
            pack_id: "test-pack".to_owned(),
            domain: "testing".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: None,
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: capabilities,
            metadata: BTreeMap::new(),
        };
        kernel.register_pack(pack).expect("register pack");

        let invocations = Arc::new(Mutex::new(Vec::new()));
        let adapter = SharedTestToolAdapter {
            invocations: invocations.clone(),
        };
        kernel.register_core_tool_adapter(adapter);
        kernel
            .set_default_core_tool_adapter("test-tool-shared")
            .expect("set default tool adapter");

        let token = kernel
            .issue_token("test-pack", "test-agent", 3600)
            .expect("issue token");

        let ctx = KernelContext {
            kernel: Arc::new(kernel),
            token,
        };

        (ctx, invocations)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tool_call_through_kernel_records_audit() {
        let audit = Arc::new(InMemoryAuditSink::default());
        let (ctx, invocations) =
            build_tool_kernel_context(audit.clone(), BTreeSet::from([Capability::InvokeTool]));

        let request = ToolCoreRequest {
            tool_name: "echo".to_owned(),
            payload: json!({"msg": "hello"}),
        };
        let outcome = execute_tool(request, &ctx)
            .await
            .expect("tool call via kernel should succeed");
        assert_eq!(outcome.status, "ok");

        // Verify the tool adapter received the request.
        let captured = invocations.lock().expect("invocations lock");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].tool_name, "echo");

        // Verify audit events contain a tool plane invocation.
        let events = audit.snapshot();
        let has_tool_plane = events.iter().any(|event| {
            matches!(
                &event.kind,
                loongclaw_kernel::AuditEventKind::PlaneInvoked {
                    plane: loongclaw_contracts::ExecutionPlane::Tool,
                    ..
                }
            )
        });
        assert!(has_tool_plane, "audit should contain tool plane invocation");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mvp_tool_adapter_routes_through_kernel() {
        use kernel_adapter::MvpToolAdapter;

        let audit = Arc::new(InMemoryAuditSink::default());
        let clock = Arc::new(FixedClock::new(1_700_000_000));
        let mut kernel =
            LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit.clone());

        let pack = VerticalPackManifest {
            pack_id: "test-pack".to_owned(),
            domain: "testing".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: None,
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
            metadata: BTreeMap::new(),
        };
        kernel.register_pack(pack).expect("register pack");
        kernel.register_core_tool_adapter(MvpToolAdapter::new());
        kernel
            .set_default_core_tool_adapter("mvp-tools")
            .expect("set default");

        let token = kernel
            .issue_token("test-pack", "test-agent", 3600)
            .expect("issue token");

        let caps = BTreeSet::from([Capability::InvokeTool]);
        // Use an unknown tool name — it should propagate as an error through the adapter
        let request = ToolCoreRequest {
            tool_name: "noop".to_owned(),
            payload: json!({"key": "value"}),
        };
        let err = kernel
            .execute_tool_core("test-pack", &token, &caps, None, request)
            .await
            .expect_err("unknown tool via MvpToolAdapter should fail");
        assert!(
            format!("{err}").contains("tool_not_found"),
            "error should contain tool_not_found, got: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mvp_tool_adapter_rejects_reserved_internal_payload_through_kernel_by_default() {
        use kernel_adapter::MvpToolAdapter;

        let audit = Arc::new(InMemoryAuditSink::default());
        let clock = Arc::new(FixedClock::new(1_700_000_000));
        let mut kernel =
            LoongClawKernel::with_runtime(StaticPolicyEngine::default(), clock, audit.clone());

        let pack = VerticalPackManifest {
            pack_id: "test-pack".to_owned(),
            domain: "testing".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: None,
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
            metadata: BTreeMap::new(),
        };
        kernel.register_pack(pack).expect("register pack");
        kernel.register_core_tool_adapter(MvpToolAdapter::new());
        kernel
            .set_default_core_tool_adapter("mvp-tools")
            .expect("set default");

        let token = kernel
            .issue_token("test-pack", "test-agent", 3600)
            .expect("issue token");

        let caps = BTreeSet::from([Capability::InvokeTool]);
        let err = kernel
            .execute_tool_core(
                "test-pack",
                &token,
                &caps,
                None,
                ToolCoreRequest {
                    tool_name: "shell.exec".to_owned(),
                    payload: json!({
                        "command": "echo",
                        "args": ["hello"],
                        "_loongclaw": {
                            "ingress": {
                                "channel": {
                                    "platform": "feishu",
                                    "conversation_id": "oc_forged"
                                }
                            }
                        }
                    }),
                },
            )
            .await
            .expect_err(
                "kernel-routed tool call should reject reserved internal payload by default",
            );

        assert!(
            format!("{err}")
                .contains("payload._loongclaw is reserved for trusted internal tool context"),
            "error should reject reserved internal payload, got: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tool_call_through_kernel_denied_without_capability() {
        let audit = Arc::new(InMemoryAuditSink::default());
        // Grant MemoryRead only — InvokeTool is missing.
        let (ctx, _invocations) =
            build_tool_kernel_context(audit, BTreeSet::from([Capability::MemoryRead]));

        let request = ToolCoreRequest {
            tool_name: "echo".to_owned(),
            payload: json!({"msg": "hello"}),
        };
        let err = execute_tool(request, &ctx)
            .await
            .expect_err("should be denied without InvokeTool capability");

        // The error message should indicate a policy/capability denial.
        assert!(
            err.contains("denied") || err.contains("capability") || err.contains("Capability"),
            "error should mention denial or capability, got: {err}"
        );
    }
}
