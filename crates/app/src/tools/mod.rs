#[cfg(test)]
use std::cell::Cell;
use std::{
    collections::BTreeSet,
    ffi::OsString,
    future::Future,
    path::{Path, PathBuf},
};

use loong_contracts::{Capability, ToolCoreOutcome, ToolCoreRequest};
use serde_json::{Value, json};
pub(crate) use tool_lease::merge_trusted_internal_tool_context_into_arguments;
#[cfg(test)]
use tool_search::searchable_entry_from_provider_definition;
pub(crate) use tool_search::tool_id_visible_in_view;
use tool_search::{SearchableToolEntry, execute_tool_search_tool_with_config};
#[cfg(test)]
use tool_search::{
    runtime_discoverable_tool_entries, runtime_tool_search_entries,
    searchable_entry_from_descriptor,
};

use crate::KernelContext;
use provider_schema::provider_definition_for_view;
use routing::route_hidden_discoverable_tool_name;
#[cfg(test)]
use routing::{
    route_direct_browser_tool_name, route_direct_web_tool_name, route_direct_web_tool_name_for_view,
};

pub(crate) mod approval;
mod bash;
mod bash_ast;
mod bash_governance;
mod bash_rules;
#[cfg(feature = "tool-browser")]
mod browser;
#[cfg(feature = "tool-browser")]
mod browser_companion;
mod bundled_skills;
mod catalog;
mod config_import;
pub(crate) mod delegate;
mod direct_policy_preflight;
pub(crate) mod download_guard;
mod external_skills;
mod external_skills_scan;
mod external_skills_sources;
#[cfg(feature = "feishu-integration")]
mod feishu;
mod file;
pub mod file_policy_ext;
#[cfg(feature = "tool-http")]
mod http_request;
mod kernel_adapter;
#[cfg(feature = "tool-file")]
mod memory_tools;
pub(crate) mod messaging;
mod payload;
mod process_exec;
mod provider_schema;
mod provider_switch;
#[cfg(test)]
mod required_capabilities_tests;
mod routing;
pub mod runtime_config;
pub(crate) mod runtime_events;
pub(crate) mod session;
#[cfg(feature = "memory-sqlite")]
mod session_search;
mod shell;
pub mod shell_policy_ext;
mod shell_request_prep;
mod tool_app_runtime;
mod tool_dispatch;
mod tool_lease;
mod tool_lease_authority;
mod tool_search;
mod tool_snapshot;
mod tool_surface;
// Browser reuses the shared SSRF and HTML helpers from web_fetch even when the
// public web.fetch tool is compiled out.
#[cfg(any(
    feature = "tool-http",
    feature = "tool-webfetch",
    feature = "tool-browser"
))]
mod web_fetch;
pub(crate) mod web_http;
mod web_search;

#[cfg(test)]
mod workspace_root_tests;

pub use catalog::{
    CapabilityActionClass, ToolApprovalMode, ToolAvailability, ToolCatalog, ToolDescriptor,
    ToolExecutionKind, ToolGovernanceProfile, ToolGovernanceScope, ToolRiskClass,
    ToolSchedulingClass, ToolView, capability_action_class_for_descriptor,
    capability_action_class_for_tool_name, delegate_child_tool_view_for_config,
    delegate_child_tool_view_for_config_with_delegate, delegate_child_tool_view_for_contract,
    delegate_child_tool_view_with_constraints, governance_profile_for_descriptor,
    governance_profile_for_tool_name, planned_delegate_child_tool_view, planned_root_tool_view,
    runtime_tool_view, runtime_tool_view_for_config,
    runtime_tool_view_for_config_with_external_skills, runtime_tool_view_for_runtime_config,
    tool_catalog,
};
#[cfg(feature = "feishu-integration")]
pub(crate) use feishu::{DeferredFeishuCardUpdate, drain_deferred_feishu_card_updates};
pub use kernel_adapter::MvpToolAdapter;
pub use shell_request_prep::summarize_tool_request_for_display;
pub(crate) use shell_request_prep::{
    TOOL_LEASE_SESSION_ID_FIELD, TOOL_LEASE_TOKEN_ID_FIELD, TOOL_LEASE_TURN_ID_FIELD,
    TOOL_SEARCH_GRANTED_CAPABILITIES_FIELD, inject_tool_lease_binding,
    normalize_shell_payload_for_request, normalize_shell_request_for_execution,
    prepare_kernel_tool_request,
};
pub(crate) use tool_dispatch::{
    execute_discoverable_tool_core_with_config, execute_tool_core_with_config,
};
#[cfg(test)]
pub(crate) use tool_dispatch::{
    is_expected_tool_request_error, run_blocking_with_timeout, tool_uses_dedicated_timeout,
};
pub(crate) use tool_lease::{
    bridge_provider_tool_call_with_scope, execute_tool_invoke_tool_with_config, issue_tool_lease,
    resolve_tool_invoke_request,
};
#[cfg(test)]
pub(crate) use tool_lease::{
    synthesize_test_provider_tool_call, synthesize_test_provider_tool_call_with_scope,
};
pub(crate) use tool_snapshot::capability_snapshot_for_view_with_config;
pub use tool_snapshot::{DiscoverableToolSurfaceSummary, ToolRegistryEntry};
#[cfg(test)]
pub(crate) use tool_snapshot::{
    runtime_discoverable_tool_surface_summary_with_config, tool_registry_with_config,
};
#[cfg(any(
    feature = "tool-http",
    feature = "tool-webfetch",
    feature = "tool-websearch"
))]
pub use web_http::build_ssrf_safe_client;

pub(crate) const BROWSER_SESSION_SCOPE_FIELD: &str = "__loong_browser_scope";
pub(crate) const LEGACY_BROWSER_SESSION_SCOPE_FIELD: &str = "__loong_browser_scope";
pub const BROWSER_COMPANION_PREVIEW_SKILL_ID: &str =
    bundled_skills::BROWSER_COMPANION_PREVIEW_SKILL_ID;
pub const BROWSER_COMPANION_COMMAND: &str = bundled_skills::BROWSER_COMPANION_COMMAND;
pub use bundled_skills::{
    BundledPreinstallTarget, BundledPreinstallTargetKind, BundledSkillPack,
    bundled_preinstall_targets, bundled_skill_pack, bundled_skill_pack_memberships,
    bundled_skill_packs,
};
pub(crate) use provider_schema::provider_tool_definitions_with_config;
pub use provider_schema::{
    provider_tool_definitions, tool_parameter_schema_types, try_provider_tool_definitions_for_view,
};
pub(crate) use routing::hidden_operation_for_tool_name;
#[cfg(all(test, unix, feature = "tool-shell"))]
pub(crate) use routing::route_direct_tool_name;
pub use tool_snapshot::{
    capability_snapshot, capability_snapshot_for_view, capability_snapshot_with_config,
    tool_registry,
};
pub use tool_surface::ToolSurfaceState;

const BROWSER_COMPANION_TOOL_PREFIX: &str = "browser.companion.";
const DELEGATE_ASYNC_TOOL_NAME: &str = "delegate_async";
const DELEGATE_TOOL_NAME: &str = "delegate";
// Grouped hidden façade ids keep the model-facing vocabulary small.
// `channel` stays separate from `agent`/`skills` so addon boundaries remain explicit.
const HIDDEN_AGENT_TOOL_NAME: &str = "agent";
const HIDDEN_SKILLS_TOOL_NAME: &str = "skills";
const HIDDEN_CHANNEL_TOOL_NAME: &str = "channel";
pub(crate) const SHELL_EXEC_TOOL_NAME: &str = "shell.exec";
const BASH_EXEC_TOOL_NAME: &str = "bash.exec";
const HTTP_REQUEST_TOOL_NAME: &str = "http.request";
const WEB_FETCH_TOOL_NAME: &str = "web.fetch";
const WEB_SEARCH_TOOL_NAME: &str = "web.search";

pub(crate) const LOONG_INTERNAL_TOOL_CONTEXT_KEY: &str = "_loong";
pub(crate) const LOONG_INTERNAL_TOOL_SEARCH_KEY: &str = "tool_search";
pub(crate) const LOONG_INTERNAL_TOOL_SEARCH_VISIBLE_TOOL_IDS_KEY: &str = "visible_tool_ids";
pub(crate) const LOONG_INTERNAL_RUNTIME_NARROWING_KEY: &str = "runtime_narrowing";
pub(crate) const LOONG_INTERNAL_WORKSPACE_ROOT_KEY: &str = "workspace_root";

pub fn normalize_external_skills_domain_rule(raw: &str) -> Result<String, String> {
    external_skills::normalize_domain_rule(raw)
}

pub fn normalize_external_skill_domain_rule(raw: &str) -> Result<String, String> {
    normalize_external_skills_domain_rule(raw)
}

pub fn external_skills_operator_list_with_config(
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    external_skills::execute_external_skills_operator_list_tool_with_config(config)
}

pub fn external_skills_operator_inspect_with_config(
    skill_id: &str,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    external_skills::execute_external_skills_operator_inspect_tool_with_config(skill_id, config)
}

pub(crate) fn discover_installable_external_skill_roots(
    root: &Path,
) -> Result<Vec<PathBuf>, String> {
    external_skills::discover_installable_skill_roots(root)
}

pub(crate) fn resolve_installable_external_skill_id(root: &Path) -> Result<String, String> {
    external_skills::resolve_installable_skill_id(root)
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

#[cfg(test)]
pub(crate) fn reset_runtime_home_state_for_tests() {
    tool_lease_authority::clear_tool_lease_secret_cache_for_tests();
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

pub(crate) fn payload_uses_reserved_internal_tool_context(payload: &Value) -> bool {
    reserved_internal_tool_context_key_in_payload(payload).is_some()
}

fn reserved_internal_tool_context_key_in_payload(payload: &Value) -> Option<&'static str> {
    payload
        .as_object()
        .and_then(reserved_internal_tool_context_key_in_map)
}

fn reserved_internal_tool_context_key_in_map(
    body: &serde_json::Map<String, Value>,
) -> Option<&'static str> {
    if body.contains_key(LOONG_INTERNAL_TOOL_CONTEXT_KEY) {
        Some(LOONG_INTERNAL_TOOL_CONTEXT_KEY)
    } else {
        None
    }
}

pub(crate) fn trusted_internal_tool_context_from_payload(
    payload: &Value,
) -> Option<&serde_json::Map<String, Value>> {
    let body = payload.as_object()?;
    let key = reserved_internal_tool_context_key_in_map(body)?;
    body.get(key)?.as_object()
}

pub(crate) fn take_trusted_internal_tool_context(
    body: &mut serde_json::Map<String, Value>,
) -> serde_json::Map<String, Value> {
    for key in [
        LOONG_INTERNAL_TOOL_CONTEXT_KEY,
        LOONG_INTERNAL_TOOL_CONTEXT_KEY,
    ] {
        let Some(value) = body.remove(key) else {
            continue;
        };
        if let Some(object) = value.as_object() {
            return object.clone();
        }
    }
    serde_json::Map::new()
}

fn ensure_untrusted_payload_does_not_use_reserved_internal_tool_context(
    tool_name: &str,
    payload: &Value,
    payload_path: &str,
) -> Result<(), String> {
    if trusted_internal_tool_payload_enabled() {
        return Ok(());
    }
    let Some(offending_key) = reserved_internal_tool_context_key_in_payload(payload) else {
        return Ok(());
    };

    Err(format!(
        "tool `{tool_name}` {payload_path}.{offending_key} is reserved for trusted internal tool context; retry without that field"
    ))
}
/// Execute a tool request, routing through the kernel for
/// policy enforcement and audit recording.
///
/// All requests are dispatched via `kernel.execute_tool_core` which
/// enforces the derived capability set for the effective tool request, runs
/// policy extensions, and records audit events.
/// `kernel.execute_tool_core` which enforces the derived capability set
/// for the effective tool request and records audit events.
pub async fn execute_tool(
    request: ToolCoreRequest,
    kernel_ctx: &KernelContext,
) -> Result<ToolCoreOutcome, String> {
    let request = prepare_kernel_tool_request(
        request,
        &kernel_ctx.token.allowed_capabilities,
        Some(kernel_ctx.token.token_id.as_str()),
        None,
        None,
    );
    execute_kernel_tool_request(kernel_ctx, request, false)
        .await
        .map_err(|e| format!("{e}"))
}

pub(crate) async fn execute_kernel_tool_request(
    ctx: &KernelContext,
    request: ToolCoreRequest,
    trusted_internal_payload: bool,
) -> Result<ToolCoreOutcome, loong_kernel::KernelError> {
    let caps = required_capabilities_for_request(&request);
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

pub(crate) use tool_app_runtime::{
    continue_session_with_runtime, execute_app_tool_with_visibility_checked_config,
};
pub use tool_app_runtime::{
    execute_app_tool_with_config, wait_for_session_with_config, wait_for_task_with_config,
};

/// Normalize a path by resolving `.` and `..` components without filesystem access.
///
/// - `Prefix` and `RootDir` are tracked separately so `..` can never "eat" them.
/// - `..` past the filesystem root (or volume root on Windows) is silently dropped.
/// - Relative paths preserve leading `..` components (e.g. `../../foo` stays as-is).
///
/// All three path-handling modules (`file`, `config_import`, `file_policy_ext`) use
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

pub(crate) fn required_capabilities_for_request(request: &ToolCoreRequest) -> BTreeSet<Capability> {
    required_capabilities_for_tool_name_and_payload(
        canonical_tool_name(request.tool_name.as_str()),
        &request.payload,
    )
}

fn required_capabilities_for_tool_name_and_payload(
    tool_name: &str,
    payload: &Value,
) -> BTreeSet<Capability> {
    let mut caps = BTreeSet::from([Capability::InvokeTool]);
    if tool_requires_network_egress(tool_name) {
        caps.insert(Capability::NetworkEgress);
    }
    match tool_name {
        "tool.invoke" => {
            let Some((invoked_tool_name, invoked_payload)) =
                invoked_discoverable_tool_request(payload)
            else {
                return caps;
            };
            return required_capabilities_for_tool_name_and_payload(
                invoked_tool_name,
                invoked_payload,
            );
        }
        "read" | "file.read" | "glob.search" | "content.search" => {
            caps.insert(Capability::FilesystemRead);
        }
        "memory" | "memory_search" | "memory_get" => {
            caps.insert(Capability::FilesystemRead);
        }
        "sessions_list"
        | "tasks_list"
        | "sessions_history"
        | "session_status"
        | "session_heads"
        | "session_path"
        | "session_children"
        | "session_artifacts"
        | "session_events"
        | "session_wait"
        | "task_status"
        | "task_wait"
        | "task_history"
        | "task_events"
        | "session_search"
        | "session_tool_policy_status" => {
            caps.insert(Capability::MemoryRead);
        }
        "write" | "file.write" | "file.edit" => {
            caps.insert(Capability::FilesystemWrite);
        }
        "exec" | BASH_EXEC_TOOL_NAME => {
            caps.insert(Capability::FilesystemRead);
            caps.insert(Capability::FilesystemWrite);
            caps.insert(Capability::NetworkEgress);
        }
        config_import::CONFIG_IMPORT_TOOL_NAME => {
            caps.insert(Capability::FilesystemRead);
            let mode_requires_write =
                config_import::config_import_mode_requires_write_value(payload);
            if mode_requires_write {
                caps.insert(Capability::FilesystemWrite);
            }
        }
        _ => {}
    }
    caps
}

pub(crate) fn invoked_discoverable_tool_request(payload: &Value) -> Option<(&str, &Value)> {
    let tool_id = payload
        .get("tool_id")
        .and_then(Value::as_str)
        .map(canonical_tool_name)?;
    if matches!(tool_id, "tool.search" | "tool.invoke") {
        return None;
    }
    let arguments = payload.get("arguments").unwrap_or(payload);
    let routed_hidden_tool_name = route_hidden_discoverable_tool_name(tool_id, arguments).ok();
    let resolved_tool_name = routed_hidden_tool_name.unwrap_or(tool_id);
    let resolved = resolve_tool_execution(resolved_tool_name)?;
    if is_provider_exposed_tool_name(resolved.canonical_name) {
        return None;
    }
    Some((resolved.canonical_name, arguments))
}

fn tool_requires_network_egress(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "web"
            | "browser"
            | HTTP_REQUEST_TOOL_NAME
            | "web.fetch"
            | "web.search"
            | "browser.open"
            | "browser.click"
            | "exec"
            | "external_skills.fetch"
            | "external_skills.source_search"
    )
}

pub fn is_known_tool_name(raw: &str) -> bool {
    if tool_catalog().resolve(raw).is_some() {
        return true;
    }
    let canonical_name = canonical_tool_name(raw);
    if matches!(
        canonical_name,
        HIDDEN_AGENT_TOOL_NAME | HIDDEN_SKILLS_TOOL_NAME | HIDDEN_CHANNEL_TOOL_NAME
    ) {
        return true;
    }
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
    let canonical_name = canonical_tool_name(raw);
    is_provider_exposed_tool_name(canonical_name) || view.contains(canonical_name)
}

pub fn is_provider_exposed_tool_name(raw: &str) -> bool {
    catalog::find_tool_catalog_entry(canonical_tool_name(raw))
        .is_some_and(|entry| entry.is_provider_exposed())
}

pub(crate) fn hidden_facade_tool_name_for_hidden_tool(raw: &str) -> Option<&'static str> {
    let canonical_name = canonical_tool_name(raw);
    tool_surface::hidden_facade_tool_name_for_hidden_tool(canonical_name)
}

pub(crate) fn direct_tool_name_for_hidden_tool(raw: &str) -> Option<&'static str> {
    let canonical_name = canonical_tool_name(raw);
    tool_surface::direct_tool_name_for_hidden_tool(canonical_name)
}

pub(crate) fn model_visible_external_skill_roots_for_runtime_config(
    config: &runtime_config::ToolRuntimeConfig,
) -> Vec<PathBuf> {
    external_skills::model_visible_skill_roots_with_config(config)
}

pub(crate) fn model_visible_external_skill_context_payload_for_path(
    config: &runtime_config::ToolRuntimeConfig,
    raw_path: &Path,
) -> Result<Option<Value>, String> {
    external_skills::model_visible_skill_context_payload_for_path(config, raw_path)
}

pub fn user_visible_tool_name(raw: &str) -> String {
    let canonical_name = canonical_tool_name(raw);

    if is_tool_surface_id(canonical_name) {
        return canonical_name.to_owned();
    }

    if let Some(direct_tool_name) = direct_tool_name_for_hidden_tool(canonical_name) {
        return direct_tool_name.to_owned();
    }

    canonical_name.to_owned()
}

pub(crate) fn model_visible_tool_name(raw: &str) -> String {
    let canonical_name = canonical_tool_name(raw);

    if let Some(hidden_facade_tool_name) = hidden_facade_tool_name_for_hidden_tool(canonical_name) {
        return hidden_facade_tool_name.to_owned();
    }

    user_visible_tool_name(canonical_name)
}

pub(crate) fn is_tool_surface_id(surface_id: &str) -> bool {
    tool_surface::is_tool_surface_id(surface_id)
}

pub fn runtime_tool_view_from_loong_config(config: &crate::config::LoongConfig) -> ToolView {
    let runtime_config = runtime_config::ToolRuntimeConfig::from_loong_config(config, None);
    runtime_tool_view_with_runtime_config(&config.tools, &runtime_config)
}

pub(crate) fn runtime_tool_view_with_runtime_config(
    _tool_config: &crate::config::ToolConfig,
    runtime_config: &runtime_config::ToolRuntimeConfig,
) -> ToolView {
    runtime_tool_view_for_runtime_config(runtime_config)
}

/// Build a tool view from runtime config (respecting runtime toggles) plus
/// feishu entries when the feishu integration is configured. This avoids
/// using `ToolConfig::default()` which ignores runtime-disabled tools.
fn full_runtime_tool_view_for_runtime_config(
    config: &runtime_config::ToolRuntimeConfig,
) -> ToolView {
    runtime_tool_view_for_runtime_config(config)
}

#[derive(Debug, Clone, Copy)]
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

    let canonical_name = canonical_tool_name(raw);
    if canonical_name == HIDDEN_AGENT_TOOL_NAME {
        return Some(ResolvedToolExecution {
            canonical_name: HIDDEN_AGENT_TOOL_NAME,
            execution_kind: ToolExecutionKind::Core,
        });
    }
    if canonical_name == HIDDEN_SKILLS_TOOL_NAME {
        return Some(ResolvedToolExecution {
            canonical_name: HIDDEN_SKILLS_TOOL_NAME,
            execution_kind: ToolExecutionKind::Core,
        });
    }
    if canonical_name == HIDDEN_CHANNEL_TOOL_NAME {
        return Some(ResolvedToolExecution {
            canonical_name: HIDDEN_CHANNEL_TOOL_NAME,
            execution_kind: ToolExecutionKind::Core,
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

/// Tool registry entry for capability snapshot disclosure.
#[cfg(all(test, feature = "feishu-integration"))]
fn feishu_searchable_entries() -> Vec<SearchableToolEntry> {
    feishu::feishu_provider_tool_definitions()
        .into_iter()
        .filter_map(|tool| {
            let function = tool.get("function")?;
            let provider_name = function.get("name")?.as_str()?;
            let parameters = function
                .get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let summary = function
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let tags = vec!["feishu".to_owned()];
            let canonical_name = canonical_tool_name(provider_name).to_owned();
            let tool_id = tool_surface::discovery_tool_name_for_tool_name(canonical_name.as_str());
            let search_hint = canonical_name.clone();
            let preferred_parameter_order: &[(&str, &str)] = &[];
            Some(searchable_entry_from_provider_definition(
                canonical_name.as_str(),
                provider_name,
                &[],
                tool_id,
                summary,
                search_hint,
                &parameters,
                preferred_parameter_order,
                tags,
                None,
                None,
                true,
            ))
        })
        .collect()
}

fn effective_runtime_visible_tool_view(
    config: &runtime_config::ToolRuntimeConfig,
    visible_tool_view: Option<&ToolView>,
) -> ToolView {
    let runtime_view = full_runtime_tool_view_for_runtime_config(config);

    match visible_tool_view {
        Some(injected) => {
            // Intersect the injected view with the runtime-visible surface so that
            // trusted _loong.tool_search.visible_tool_ids cannot re-expose
            // tools disabled by runtime config (browser.*, session_*, etc.).
            injected.intersect(&runtime_view)
        }
        None => runtime_view,
    }
}

#[cfg(test)]
#[path = "tools_mod_tests.rs"]
mod tests;
