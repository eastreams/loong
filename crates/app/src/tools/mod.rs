#[cfg(test)]
use std::cell::Cell;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    future::Future,
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

use loong_contracts::{Capability, ToolCoreOutcome, ToolCoreRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[cfg(test)]
use tool_search::searchable_entry_from_provider_definition;
use tool_search::{
    SearchableToolEntry, collapse_hidden_surface_search_entries,
    execute_tool_search_tool_with_config, searchable_entry_from_descriptor,
    tool_search_entry_is_runtime_usable,
};

use crate::KernelContext;
use crate::config::ToolConfig;
use crate::session::store::SessionStoreConfig;
use provider_schema::provider_definition_for_view;
use routing::{
    execute_direct_tool_core_with_config, resolved_inner_tool_name_for_logs,
    route_hidden_discoverable_tool_name,
};
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
mod tool_lease;
mod tool_lease_authority;
mod tool_search;
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
pub(crate) use tool_lease::{
    bridge_provider_tool_call_with_scope, execute_tool_invoke_tool_with_config, issue_tool_lease,
    resolve_tool_invoke_request,
};
#[cfg(test)]
pub(crate) use tool_lease::{
    synthesize_test_provider_tool_call, synthesize_test_provider_tool_call_with_scope,
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
#[cfg(test)]
pub(crate) use routing::route_direct_tool_name;
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

pub fn execute_app_tool_with_config(
    request: ToolCoreRequest,
    current_session_id: &str,
    memory_config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_app_tool_with_browser_companion_readiness(
        request,
        current_session_id,
        memory_config,
        tool_config,
        false,
    )
}

pub(crate) fn execute_app_tool_with_visibility_checked_config(
    request: ToolCoreRequest,
    current_session_id: &str,
    memory_config: &SessionStoreConfig,
    tool_config: &ToolConfig,
) -> Result<ToolCoreOutcome, String> {
    execute_app_tool_with_browser_companion_readiness(
        request,
        current_session_id,
        memory_config,
        tool_config,
        true,
    )
}

fn execute_app_tool_with_browser_companion_readiness(
    request: ToolCoreRequest,
    current_session_id: &str,
    memory_config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    assume_browser_companion_ready: bool,
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
        "sessions_list"
        | "tasks_list"
        | "sessions_history"
        | "task_history"
        | "task_events"
        | "session_tool_policy_status"
        | "session_tool_policy_set"
        | "session_tool_policy_clear"
        | "session_status"
        | "task_status"
        | "session_events"
        | "session_search"
        | "session_archive"
        | "session_cancel"
        | "session_continue"
        | "session_recover" => session::execute_session_tool_with_policies(
            request,
            current_session_id,
            memory_config,
            tool_config,
        ),
        #[cfg(feature = "tool-browser")]
        "browser.companion.click" | "browser.companion.type" => {
            if assume_browser_companion_ready {
                browser_companion::execute_browser_companion_visible_app_tool_with_config(
                    request,
                    current_session_id,
                    tool_config,
                )
            } else {
                browser_companion::execute_browser_companion_app_tool_with_config(
                    request,
                    current_session_id,
                    tool_config,
                )
            }
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
    memory_config: &SessionStoreConfig,
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

pub async fn wait_for_task_with_config(
    payload: Value,
    current_session_id: &str,
    memory_config: &SessionStoreConfig,
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
            return Err("app_tool_disabled: task tools are disabled by config".to_owned());
        }
        session::wait_for_task_tool_with_policies(
            payload,
            current_session_id,
            memory_config,
            tool_config,
        )
        .await
    }
}

#[cfg(feature = "memory-sqlite")]
pub(crate) async fn continue_session_with_runtime<
    R: crate::conversation::ConversationRuntime + ?Sized,
>(
    payload: Value,
    current_session_id: &str,
    memory_config: &SessionStoreConfig,
    tool_config: &ToolConfig,
    app_config: &crate::config::LoongConfig,
    runtime: &R,
    binding: crate::conversation::ConversationRuntimeBinding<'_>,
) -> Result<ToolCoreOutcome, String> {
    session::continue_session_with_runtime(
        payload,
        current_session_id,
        memory_config,
        tool_config,
        app_config,
        runtime,
        binding,
    )
    .await
}

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

pub(crate) fn tool_surface_visible_in_view(surface_id: &str, view: &ToolView) -> bool {
    tool_surface::tool_surface_visible_in_view(surface_id, view)
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

pub fn execute_tool_core_with_config(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let requested_tool_name = request.tool_name.clone();
    let canonical_name = canonical_tool_name(request.tool_name.as_str());
    let payload = request.payload;
    let workspace_root = trusted_workspace_root_from_payload(&payload)?;
    let runtime_narrowing = trusted_runtime_narrowing_from_payload(&payload)?;
    let mut effective_config = config
        .workspace_root
        .clone()
        .map(|workspace_root| config.with_file_root_override(workspace_root))
        .unwrap_or_else(|| config.clone());
    if let Some(workspace_root) = workspace_root {
        effective_config = effective_config.with_file_root_override(workspace_root);
    }
    if let Some(runtime_narrowing) = runtime_narrowing {
        effective_config = effective_config.narrowed(&runtime_narrowing);
    }
    let config = &effective_config;
    let debug_log_enabled = tracing::enabled!(target: "loong.tools", tracing::Level::DEBUG);
    let warn_log_enabled = tracing::enabled!(target: "loong.tools", tracing::Level::WARN);
    let should_log_payload_metadata = debug_log_enabled || warn_log_enabled;
    let mut payload_kind = "-";
    let mut payload_keys = Vec::new();
    if should_log_payload_metadata {
        payload_kind = crate::observability::json_value_kind(&payload);
        payload_keys = crate::observability::top_level_json_keys(&payload);
    }
    let inner_tool_name = resolved_inner_tool_name_for_logs(canonical_name, &payload);
    let started_at = std::time::Instant::now();
    let execute_request = || {
        ensure_untrusted_payload_does_not_use_reserved_internal_tool_context(
            requested_tool_name.as_str(),
            &payload,
            "payload",
        )?;
        let request = ToolCoreRequest {
            tool_name: canonical_name.to_owned(),
            payload,
        };
        let request = normalize_shell_request_for_execution(request);
        let effective_config = trusted_runtime_narrowing_from_payload(&request.payload)?;
        let effective_config = effective_config.map(|narrowing| config.narrowed(&narrowing));
        let config = effective_config.as_ref().unwrap_or(config);

        match canonical_name {
            "tool.search" => execute_tool_search_tool_with_config(request, config),
            "tool.invoke" => execute_tool_invoke_tool_with_config(request, config),
            "read" | "write" | "exec" | "web" | "browser" | "memory" => {
                execute_direct_tool_core_with_config(request, config)
            }
            _ => execute_discoverable_tool_core_with_config(request, config),
        }
    };
    let result = execute_request();
    let duration_ms = started_at.elapsed().as_millis();
    match &result {
        Ok(outcome) => {
            if debug_log_enabled {
                tracing::debug!(
                    target: "loong.tools",
                    requested_tool_name = %requested_tool_name,
                    canonical_tool_name = %canonical_name,
                    inner_tool_name = %inner_tool_name,
                    payload_kind,
                    payload_keys = ?payload_keys,
                    status = %outcome.status,
                    duration_ms,
                    "tool execution completed"
                );
            }
        }
        Err(error) => {
            if is_expected_tool_request_error(error) {
                if debug_log_enabled {
                    tracing::debug!(
                        target: "loong.tools",
                        requested_tool_name = %requested_tool_name,
                        canonical_tool_name = %canonical_name,
                        inner_tool_name = %inner_tool_name,
                        payload_kind,
                        payload_keys = ?payload_keys,
                        duration_ms,
                        error = %crate::observability::summarize_error(error),
                        "tool execution rejected"
                    );
                }
            } else if warn_log_enabled {
                tracing::warn!(
                    target: "loong.tools",
                    requested_tool_name = %requested_tool_name,
                    canonical_tool_name = %canonical_name,
                    inner_tool_name = %inner_tool_name,
                    payload_kind,
                    payload_keys = ?payload_keys,
                    duration_ms,
                    error = %crate::observability::summarize_error(error),
                    "tool execution failed"
                );
            }
        }
    }

    result
}

fn is_expected_tool_request_error(error: &str) -> bool {
    if error.starts_with("tool_not_found:") {
        return true;
    }
    if error.starts_with("app_tool_not_found:") {
        return true;
    }
    if error.starts_with("invalid_tool_lease:") {
        return true;
    }
    if error.starts_with("invalid_internal_runtime_narrowing:") {
        return true;
    }
    if error.starts_with("tool_surface_unavailable:") {
        return true;
    }
    if error.starts_with("direct_") {
        return true;
    }
    if error.contains("max_bytes limit") {
        return true;
    }
    if error.contains("browser tools are disabled by config.tools.browser.enabled=false") {
        return true;
    }
    if error.contains("web.fetch is disabled by config.tools.web.enabled=false") {
        return true;
    }
    error.contains("reserved for trusted internal tool context")
}

fn trusted_runtime_narrowing_from_payload(
    payload: &Value,
) -> Result<Option<runtime_config::ToolRuntimeNarrowing>, String> {
    if !trusted_internal_tool_payload_enabled() {
        return Ok(None);
    }

    let Some(value) = trusted_internal_tool_context_from_payload(payload)
        .and_then(|body| body.get(LOONG_INTERNAL_RUNTIME_NARROWING_KEY))
        .cloned()
    else {
        return Ok(None);
    };

    serde_json::from_value(value)
        .map(Some)
        .map_err(|error| format!("invalid_internal_runtime_narrowing: {error}"))
}

fn trusted_workspace_root_from_payload(payload: &Value) -> Result<Option<PathBuf>, String> {
    if !trusted_internal_tool_payload_enabled() {
        return Ok(None);
    }

    let Some(value) = trusted_internal_tool_context_from_payload(payload)
        .and_then(|body| body.get(LOONG_INTERNAL_WORKSPACE_ROOT_KEY))
        .cloned()
    else {
        return Ok(None);
    };

    let raw_workspace_root = serde_json::from_value::<String>(value)
        .map_err(|error| format!("invalid_internal_workspace_root: {error}"))?;
    let trimmed_workspace_root = raw_workspace_root.trim();
    if trimmed_workspace_root.is_empty() {
        return Err("invalid_internal_workspace_root: expected a non-empty path".to_owned());
    }
    let workspace_root = PathBuf::from(trimmed_workspace_root);
    if !workspace_root.is_absolute() {
        return Err("invalid_internal_workspace_root: path must be absolute".to_owned());
    }
    let canonical_workspace_root = std::fs::canonicalize(&workspace_root).map_err(|error| {
        format!("invalid_internal_workspace_root: canonicalize failed: {error}")
    })?;
    if !canonical_workspace_root.is_dir() {
        return Err("invalid_internal_workspace_root: path must be a directory".to_owned());
    }
    Ok(Some(canonical_workspace_root))
}

pub(crate) fn merge_trusted_internal_tool_context_into_arguments(
    arguments: &mut serde_json::Map<String, Value>,
    internal_context: &Value,
) -> Result<(), String> {
    let trusted_context = internal_context.as_object().cloned().ok_or_else(|| {
        format!("tool.invoke payload.{LOONG_INTERNAL_TOOL_CONTEXT_KEY} must be an object")
    })?;
    if let Some(offending_key) = reserved_internal_tool_context_key_in_map(arguments) {
        return Err(format!(
            "tool.invoke payload.arguments.{offending_key} is reserved for trusted internal tool context"
        ));
    }
    let merged_context = Value::Object(trusted_context);
    arguments.insert(LOONG_INTERNAL_TOOL_CONTEXT_KEY.to_owned(), merged_context);
    Ok(())
}

fn execute_discoverable_tool_core_with_config(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let request = normalize_shell_request_for_execution(request);
    let tool_name = request.tool_name.clone();
    direct_policy_preflight::run(&request, config)?;
    let timeout_seconds = config.tool_execution.timeout_for_tool(&tool_name);

    let inner = {
        let config = config.clone();
        move || dispatch_tool_request(request, &config)
    };

    match timeout_seconds {
        Some(seconds) if seconds > 0 && !tool_uses_dedicated_timeout(&tool_name) => {
            run_blocking_with_timeout(inner, seconds, &tool_name)
        }
        _ => inner(),
    }
}

fn tool_uses_dedicated_timeout(tool_name: &str) -> bool {
    if tool_name == SHELL_EXEC_TOOL_NAME {
        return true;
    }
    if tool_name == BASH_EXEC_TOOL_NAME {
        return true;
    }
    if tool_name == HTTP_REQUEST_TOOL_NAME {
        return true;
    }
    if tool_name == WEB_FETCH_TOOL_NAME {
        return true;
    }
    if tool_name == WEB_SEARCH_TOOL_NAME {
        return true;
    }
    if tool_name == DELEGATE_TOOL_NAME {
        return true;
    }
    if tool_name == DELEGATE_ASYNC_TOOL_NAME {
        return true;
    }
    tool_name.starts_with(BROWSER_COMPANION_TOOL_PREFIX)
}

fn dispatch_tool_request(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    match request.tool_name.as_str() {
        config_import::CONFIG_IMPORT_TOOL_NAME => {
            config_import::execute_config_import_tool_with_config(request, config)
        }
        "external_skills.resolve" => {
            external_skills::execute_external_skills_resolve_tool_with_config(request, config)
        }
        "external_skills.search" => {
            external_skills::execute_external_skills_search_tool_with_config(request, config)
        }
        "external_skills.recommend" => {
            external_skills::execute_external_skills_recommend_tool_with_config(request, config)
        }
        "external_skills.source_search" => {
            external_skills::execute_external_skills_source_search_tool_with_config(request, config)
        }
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
        #[cfg(feature = "tool-browser")]
        "browser.companion.session.start"
        | "browser.companion.navigate"
        | "browser.companion.snapshot"
        | "browser.companion.wait"
        | "browser.companion.session.stop" => {
            browser_companion::execute_browser_companion_core_tool_with_config(request, config)
        }
        #[cfg(feature = "tool-browser")]
        "browser.open" | "browser.extract" | "browser.click" => {
            browser::execute_browser_tool_with_config(request, config)
        }
        #[cfg(feature = "feishu-integration")]
        other if feishu::is_known_feishu_tool_name(other) => {
            feishu::execute_feishu_tool_with_config(request, config)
        }
        #[cfg(feature = "tool-http")]
        "http.request" => http_request::execute_http_request_tool_with_config(request, config),
        "shell.exec" => shell::execute_shell_tool_with_config(request, config),
        "bash.exec" => bash::execute_bash_tool_with_config(request, config),
        "file.read" => file::execute_file_read_tool_with_config(request, config),
        "glob.search" => file::execute_glob_search_tool_with_config(request, config),
        "content.search" => file::execute_content_search_tool_with_config(request, config),
        #[cfg(feature = "tool-file")]
        "memory_search" => memory_tools::execute_memory_search_tool_with_config(request, config),
        #[cfg(feature = "tool-file")]
        "memory_get" => memory_tools::execute_memory_get_tool_with_config(request, config),
        "file.write" => file::execute_file_write_tool_with_config(request, config),
        "file.edit" => file::execute_file_edit_tool_with_config(request, config),
        "provider.switch" => {
            provider_switch::execute_provider_switch_tool_with_config(request, config)
        }
        #[cfg(feature = "tool-webfetch")]
        "web.fetch" => web_fetch::execute_web_fetch_tool_with_config(request, config),
        "web.search" => web_search::execute_web_search_tool_with_config(request, config),
        _ => Err(format!(
            "tool_not_found: unknown tool `{}`",
            request.tool_name
        )),
    }
}

fn run_blocking_with_timeout<F, T>(f: F, timeout_seconds: u64, tool_name: &str) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    let timeout = Duration::from_secs(timeout_seconds);
    let tool_name = tool_name.to_owned();
    let worker_name = format!("tool-timeout:{tool_name}");
    let (sender, receiver) = mpsc::sync_channel(1);

    let worker = std::thread::Builder::new()
        .name(worker_name)
        .spawn(move || {
            let result = f();
            let _ = sender.send(result);
        })
        .map_err(|error| format!("failed to spawn tool timeout worker for {tool_name}: {error}"))?;

    match receiver.recv_timeout(timeout) {
        Ok(result) => {
            let join_result = worker.join();
            if join_result.is_err() {
                return Err(format!(
                    "tool_execution_join_error: {tool_name} worker thread panicked"
                ));
            }
            result
        }
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!(
            "tool_execution_timeout: {tool_name} exceeded {timeout_seconds}s"
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let join_result = worker.join();
            if join_result.is_err() {
                return Err(format!(
                    "tool_execution_join_error: {tool_name} worker thread panicked"
                ));
            }
            Err(format!(
                "tool_execution_join_error: {tool_name} worker thread exited without returning a result"
            ))
        }
    }
}

/// Tool registry entry for capability snapshot disclosure.
#[derive(Debug, Clone)]
pub struct ToolRegistryEntry {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoverableToolSurfaceSummary {
    #[serde(default)]
    pub visible_direct_tools: Vec<String>,
    pub hidden_tool_count: usize,
    #[serde(default)]
    pub hidden_tags: Vec<String>,
    #[serde(default)]
    pub hidden_surfaces: Vec<ToolSurfaceState>,
}

/// Returns a sorted list of all registered hidden specialized tools, gated by feature flags.
pub fn tool_registry() -> Vec<ToolRegistryEntry> {
    tool_registry_with_config(Some(runtime_config::get_tool_runtime_config()))
}

pub(crate) fn tool_registry_with_config(
    config: Option<&runtime_config::ToolRuntimeConfig>,
) -> Vec<ToolRegistryEntry> {
    let default_runtime_config;
    let config = match config {
        Some(config) => config,
        None => {
            default_runtime_config = runtime_config::ToolRuntimeConfig::default();
            &default_runtime_config
        }
    };

    let discoverable_entries = runtime_discoverable_tool_entries(config, None, false);
    let mut entries = Vec::new();

    for entry in discoverable_entries {
        let registry_entry = ToolRegistryEntry {
            name: entry.canonical_name,
            description: entry.summary,
        };
        entries.push(registry_entry);
    }

    entries.sort_by(|left, right| left.name.cmp(&right.name));
    entries
}

/// Produce a deterministic text block listing available tools,
/// suitable for appending to the system prompt.
pub fn capability_snapshot() -> String {
    capability_snapshot_with_config(runtime_config::get_tool_runtime_config())
}

pub fn capability_snapshot_with_config(config: &runtime_config::ToolRuntimeConfig) -> String {
    capability_snapshot_for_view_with_config(&runtime_tool_view_for_runtime_config(config), config)
}

pub fn capability_snapshot_for_view(view: &ToolView) -> String {
    capability_snapshot_for_view_with_config(view, runtime_config::get_tool_runtime_config())
}

pub(crate) fn capability_snapshot_for_view_with_config(
    view: &ToolView,
    config: &runtime_config::ToolRuntimeConfig,
) -> String {
    let mut lines = vec![
        "[tool_discovery_runtime]".to_owned(),
        "Available tools:".to_owned(),
    ];

    let visible_direct_states = tool_surface::visible_direct_tool_states_for_view(view);
    let visible_direct_lines = render_visible_direct_tool_lines(visible_direct_states.as_slice());
    lines.extend(visible_direct_lines);

    let gateway_entries = catalog::provider_exposed_tool_catalog();
    let gateway_entries = gateway_entries
        .into_iter()
        .filter(|entry| entry.is_gateway())
        .collect::<Vec<_>>();
    for entry in gateway_entries {
        let line = format!("- {}: {}", entry.canonical_name, entry.summary);
        lines.push(line);
    }

    let discoverable_summary =
        runtime_discoverable_tool_surface_summary_with_config(config, Some(view));
    let hidden_tool_count = discoverable_summary.hidden_tool_count;

    if hidden_tool_count == 0 {
        lines.push(
            "No additional specialized tools are currently available through tool.search."
                .to_owned(),
        );
    } else {
        let hidden_count_line = format!(
            "Additional specialized tools available through tool.search: {hidden_tool_count}."
        );
        lines.push(hidden_count_line);

        let hidden_surface_lines =
            render_hidden_tool_surface_lines(discoverable_summary.hidden_surfaces.as_slice());
        lines.extend(hidden_surface_lines);

        let hidden_tag_line = hidden_tool_tag_line(discoverable_summary.hidden_tags.as_slice());
        if let Some(hidden_tag_line) = hidden_tag_line {
            lines.push(hidden_tag_line);
        }
    }

    lines.push("Guidelines:".to_owned());
    lines.extend(render_active_tool_guideline_lines(
        visible_direct_states.as_slice(),
        discoverable_summary.hidden_surfaces.as_slice(),
    ));
    if let Some(skill_catalog_section) =
        external_skills::model_skill_catalog_section_with_config(config)
    {
        lines.push(skill_catalog_section);
    }
    lines.join("\n")
}

fn render_visible_direct_tool_lines(states: &[ToolSurfaceState]) -> Vec<String> {
    let mut lines = Vec::new();

    for state in states {
        let line = format!(
            "- {}: {} {}",
            state.surface_id, state.prompt_snippet, state.usage_guidance
        );
        lines.push(line);
    }

    lines
}

fn render_active_tool_guideline_lines(
    visible_direct_states: &[ToolSurfaceState],
    hidden_surfaces: &[ToolSurfaceState],
) -> Vec<String> {
    let mut lines = vec![
        "- Prefer a direct tool when one clearly fits.".to_owned(),
        "- Use tool.search only when you need a specialized capability that is not already direct.".to_owned(),
        "- Keep tool.search queries short and capability-focused.".to_owned(),
        "- Use tool.invoke only with a fresh lease returned by tool.search.".to_owned(),
        "- If the user wants different permissions or guardrails, edit the relevant config or prompt files instead of treating the runtime as fixed.".to_owned(),
    ];
    let mut seen = lines.iter().cloned().collect::<BTreeSet<_>>();

    for surface in visible_direct_states.iter().chain(hidden_surfaces.iter()) {
        let Some(guidelines) =
            tool_surface::tool_surface_prompt_guidelines_for_id(surface.surface_id.as_str())
        else {
            continue;
        };
        for guideline in guidelines {
            let line = format!("- {guideline}");
            let inserted = seen.insert(line.clone());
            if inserted {
                lines.push(line);
            }
        }
    }

    lines
}

fn render_hidden_tool_surface_lines(surfaces: &[ToolSurfaceState]) -> Vec<String> {
    surfaces
        .iter()
        .map(ToolSurfaceState::render_prompt_line)
        .collect()
}

pub fn runtime_discoverable_tool_surface_summary_with_config(
    config: &runtime_config::ToolRuntimeConfig,
    visible_tool_view: Option<&ToolView>,
) -> DiscoverableToolSurfaceSummary {
    let effective_view = effective_runtime_visible_tool_view(config, visible_tool_view);
    let discoverable_entries =
        runtime_discoverable_tool_entries(config, Some(&effective_view), true);
    let direct_states = tool_surface::visible_direct_tool_states_for_view(&effective_view);
    summarize_discoverable_tool_surface(discoverable_entries.as_slice(), direct_states)
}

fn summarize_discoverable_tool_surface(
    discoverable_entries: &[SearchableToolEntry],
    direct_states: Vec<ToolSurfaceState>,
) -> DiscoverableToolSurfaceSummary {
    let visible_direct_tools = direct_states
        .into_iter()
        .map(|state| state.surface_id)
        .collect::<Vec<_>>();
    let hidden_surfaces = tool_surface::active_discoverable_tool_surface_states(
        discoverable_entries
            .iter()
            .map(|entry| entry.tool_id.as_str()),
    );

    DiscoverableToolSurfaceSummary {
        visible_direct_tools,
        hidden_tool_count: discoverable_entries.len(),
        hidden_tags: summarize_hidden_tool_tags(discoverable_entries),
        hidden_surfaces,
    }
}

fn hidden_tool_tag_line(hidden_tags: &[String]) -> Option<String> {
    if hidden_tags.is_empty() {
        return None;
    }

    let joined_tags = hidden_tags.join(", ");
    let line = format!("Hidden specialized tool tags currently discoverable: {joined_tags}.");
    Some(line)
}

fn summarize_hidden_tool_tags(entries: &[SearchableToolEntry]) -> Vec<String> {
    const IGNORED_TAGS: &[&str] = &["core", "discover", "search", "dispatch", "invoke"];
    const MAX_DISCOVERABLE_CAPABILITY_TAGS: usize = 8;

    let mut tag_counts = BTreeMap::<String, usize>::new();

    for entry in entries {
        for tag in &entry.tags {
            let normalized_tag = tag.trim();
            if normalized_tag.is_empty() {
                continue;
            }

            let ignored_tag = IGNORED_TAGS.contains(&normalized_tag);
            if ignored_tag {
                continue;
            }

            let count_entry = tag_counts.entry(normalized_tag.to_owned()).or_insert(0);
            *count_entry += 1;
        }
    }

    let mut ranked_tags = tag_counts.into_iter().collect::<Vec<_>>();
    ranked_tags.sort_by(|left, right| {
        let left_count = left.1;
        let right_count = right.1;
        right_count
            .cmp(&left_count)
            .then_with(|| left.0.cmp(&right.0))
    });

    ranked_tags
        .into_iter()
        .take(MAX_DISCOVERABLE_CAPABILITY_TAGS)
        .map(|(tag, _count)| tag)
        .collect()
}

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

fn runtime_tool_search_entries(
    config: &runtime_config::ToolRuntimeConfig,
    visible_tool_view: Option<&ToolView>,
    collapse_hidden_surfaces: bool,
) -> Vec<SearchableToolEntry> {
    let visible_tool_view = effective_runtime_visible_tool_view(config, visible_tool_view);
    let mut entries = Vec::new();

    for descriptor in catalog::tool_catalog().descriptors().iter() {
        let runtime_available = descriptor.availability == ToolAvailability::Runtime;
        if !runtime_available {
            continue;
        }

        if descriptor.is_direct() {
            let direct_tool_visible =
                tool_surface::direct_tool_visible_in_view(descriptor.name, &visible_tool_view);
            if !direct_tool_visible {
                continue;
            }
            let entry = tool_search::searchable_entry_from_descriptor_for_runtime_view(
                descriptor,
                &visible_tool_view,
            );
            entries.push(entry);
        }
    }

    let hidden_entries = runtime_discoverable_tool_entries(config, Some(&visible_tool_view), true);
    let hidden_entries = if collapse_hidden_surfaces {
        let collapsible_surface_ids =
            provider_visible_collapsible_hidden_surface_ids(config, &visible_tool_view);
        collapse_hidden_surface_search_entries(hidden_entries, &collapsible_surface_ids)
    } else {
        hidden_entries
    };
    entries.extend(hidden_entries);
    entries
}

fn runtime_discoverable_tool_entries(
    config: &runtime_config::ToolRuntimeConfig,
    visible_tool_view: Option<&ToolView>,
    provider_invokable_only: bool,
) -> Vec<SearchableToolEntry> {
    let visible_tool_view = effective_runtime_visible_tool_view(config, visible_tool_view);
    catalog::tool_catalog()
        .descriptors()
        .iter()
        .filter(|descriptor| {
            let is_discoverable = descriptor.is_discoverable();
            if !is_discoverable {
                return false;
            }

            if !provider_invokable_only {
                return true;
            }

            descriptor.is_provider_invokable_discoverable()
        })
        .filter(|descriptor| visible_tool_view.contains(descriptor.name))
        .filter(|descriptor| {
            descriptor.name == SHELL_EXEC_TOOL_NAME
                || tool_search_entry_is_runtime_usable(descriptor.name, config)
        })
        .filter(|descriptor| {
            !tool_surface::hidden_tool_is_covered_by_visible_direct_tool(
                descriptor.name,
                &visible_tool_view,
            )
        })
        .map(searchable_entry_from_descriptor)
        .collect::<Vec<_>>()
}

fn hidden_surface_entry_counts(entries: &[SearchableToolEntry]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();

    for entry in entries {
        let Some(surface_id) = entry.surface_id.as_deref() else {
            continue;
        };
        let is_grouped_surface = matches!(surface_id, "agent" | "skills" | "channel");
        if !is_grouped_surface {
            continue;
        }

        let entry_count = counts.entry(surface_id.to_owned()).or_insert(0);
        *entry_count += 1;
    }

    counts
}

pub(super) fn provider_visible_collapsible_hidden_surface_ids(
    config: &runtime_config::ToolRuntimeConfig,
    visible_tool_view: &ToolView,
) -> BTreeSet<String> {
    let all_discoverable_entries =
        runtime_discoverable_tool_entries(config, Some(visible_tool_view), false);
    let provider_discoverable_entries =
        runtime_discoverable_tool_entries(config, Some(visible_tool_view), true);
    let all_entry_counts = hidden_surface_entry_counts(all_discoverable_entries.as_slice());
    let provider_entry_counts =
        hidden_surface_entry_counts(provider_discoverable_entries.as_slice());
    let mut surface_ids = BTreeSet::new();

    for (surface_id, all_entry_count) in all_entry_counts {
        let Some(provider_entry_count) = provider_entry_counts.get(surface_id.as_str()).copied()
        else {
            continue;
        };
        let surface_is_fully_provider_visible = provider_entry_count == all_entry_count;
        if !surface_is_fully_provider_visible {
            continue;
        }

        surface_ids.insert(surface_id);
    }

    surface_ids
}

#[cfg(test)]
#[path = "tools_mod_tests.rs"]
mod tests;
