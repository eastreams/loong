use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::{Value, json};
pub(crate) use tool_internal_context::{
    ensure_untrusted_payload_does_not_use_reserved_internal_tool_context,
    payload_uses_reserved_internal_tool_context, reserved_internal_tool_context_key_in_map,
    take_trusted_internal_tool_context, trusted_internal_tool_context_from_payload,
    trusted_internal_tool_payload_enabled, with_trusted_internal_tool_payload_async,
};
#[cfg(test)]
pub(crate) use tool_internal_context::{
    reset_runtime_home_state_for_tests, with_trusted_internal_tool_payload,
};
pub(crate) use tool_lease::merge_trusted_internal_tool_context_into_arguments;
use tool_search::SearchableToolEntry;
#[cfg(test)]
use tool_search::searchable_entry_from_provider_definition;
#[cfg(test)]
use tool_search::{runtime_discoverable_tool_entries, runtime_tool_search_entries};

use crate::KernelContext;
use provider_schema::provider_definition_for_view;
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
mod bundled_skills;
mod catalog;
mod config_import;
pub(crate) mod delegate;
mod direct_policy_preflight;
pub(crate) mod download_guard;
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
mod skills;
mod skills_scan;
mod skills_sources;
mod tool_app_runtime;
mod tool_dispatch;
mod tool_identity;
mod tool_internal_context;
mod tool_lease;
mod tool_lease_authority;
mod tool_path;
mod tool_runtime_view;
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
    runtime_tool_view, runtime_tool_view_for_config, runtime_tool_view_for_config_with_skills,
    runtime_tool_view_for_runtime_config, tool_catalog,
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
pub(crate) use tool_dispatch::execute_discoverable_tool_core_with_config;
pub use tool_dispatch::execute_tool_core_with_config;
#[cfg(test)]
pub(crate) use tool_dispatch::{
    is_expected_tool_request_error, run_blocking_with_timeout, tool_uses_dedicated_timeout,
};
pub(crate) use tool_identity::{
    ResolvedToolExecution, direct_tool_name_for_hidden_tool, is_provider_exposed_tool_name,
    model_visible_tool_name, required_capabilities_for_request,
    required_capabilities_for_tool_name_and_payload, resolve_tool_execution,
};
pub use tool_identity::{
    canonical_tool_name, is_known_tool_name, is_known_tool_name_in_view, user_visible_tool_name,
};
pub(crate) use tool_lease::{bridge_provider_tool_call_with_scope, issue_tool_lease};
pub(crate) use tool_lease::{peek_tool_invoke_request, resolve_tool_invoke_request};
#[cfg(test)]
pub(crate) use tool_lease::{
    synthesize_test_provider_tool_call, synthesize_test_provider_tool_call_with_scope,
};
pub(crate) use tool_path::normalize_without_fs;
pub use tool_runtime_view::runtime_tool_view_from_loong_config;
pub(crate) use tool_runtime_view::{
    effective_runtime_visible_tool_view, full_runtime_tool_view_for_runtime_config,
    model_visible_skill_context_payload_for_path, model_visible_skill_context_payload_for_skill_id,
    model_visible_skill_roots_for_runtime_config, runtime_tool_view_with_runtime_config,
};
pub(crate) use tool_snapshot::capability_snapshot_for_direct_states_with_config;
pub(crate) use tool_snapshot::capability_snapshot_for_view_with_config;
pub use tool_snapshot::{
    DiscoverableToolSurfaceSummary, ToolRegistryEntry,
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
pub use bundled_skills::{
    BundledPreinstallTarget, BundledPreinstallTargetKind, BundledSkillPack,
    bundled_preinstall_targets, bundled_skill_pack, bundled_skill_pack_memberships,
    bundled_skill_packs,
};
pub(crate) use provider_schema::provider_tool_definitions_with_config;
pub use provider_schema::{
    provider_tool_definitions, tool_parameter_schema_types, try_provider_tool_definitions_for_view,
};
pub(crate) use routing::route_direct_tool_name;
pub use tool_snapshot::{
    capability_snapshot, capability_snapshot_for_view, capability_snapshot_with_config,
    tool_registry,
};
pub use tool_surface::ToolSurfaceState;
pub(crate) use tool_surface::visible_direct_tool_states_for_view;

#[cfg(test)]
pub(crate) fn tool_id_visible_in_view(tool_id: &str, view: &ToolView) -> bool {
    tool_search::tool_id_visible_in_view(tool_id, view)
}

const DELEGATE_ASYNC_TOOL_NAME: &str = "delegate_async";
const DELEGATE_TOOL_NAME: &str = "delegate";
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

pub fn normalize_skills_domain_rule(raw: &str) -> Result<String, String> {
    skills::normalize_domain_rule(raw)
}

pub fn normalize_skill_domain_rule(raw: &str) -> Result<String, String> {
    normalize_skills_domain_rule(raw)
}

pub fn skills_list_with_config(
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    skills::execute_skills_list_with_config(config)
}

pub fn skills_inspect_with_config(
    skill_id: &str,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    skills::execute_skills_inspect_with_config(skill_id, config)
}

pub(crate) fn model_visible_skill_ids_with_config(
    config: &runtime_config::ToolRuntimeConfig,
) -> Vec<String> {
    skills::model_visible_skill_catalog_entries_with_config(config)
        .into_iter()
        .map(|entry| entry.skill_id)
        .collect()
}

pub(crate) fn install_bundled_preinstall_targets_for_bootstrap(
    config: &runtime_config::ToolRuntimeConfig,
    selected_target_ids: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    skills::install_bundled_preinstall_targets_for_bootstrap(config, selected_target_ids)
}

pub(crate) fn remove_bundled_preinstall_targets_for_bootstrap(
    config: &runtime_config::ToolRuntimeConfig,
    selected_target_ids: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    skills::remove_bundled_preinstall_targets_for_bootstrap(config, selected_target_ids)
}

pub(crate) fn installed_managed_skill_ids_for_bootstrap(
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<BTreeSet<String>, String> {
    skills::installed_managed_skill_ids_for_bootstrap(config)
}

pub fn skills_search_with_config(
    query: &str,
    limit: usize,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    skills::execute_skills_search_with_config(query, limit, config)
}

pub fn skills_recommend_with_config(
    query: &str,
    limit: usize,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    skills::execute_skills_recommend_with_config(query, limit, config)
}

pub fn skills_fetch_with_config(
    reference: &str,
    save_as: Option<&str>,
    max_bytes: Option<usize>,
    approval_granted: bool,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    skills::execute_skills_fetch_with_config(
        reference,
        save_as,
        max_bytes,
        approval_granted,
        config,
    )
}

pub fn skills_install_with_config(
    path: Option<&str>,
    bundled_skill_id: Option<&str>,
    skill_id: Option<&str>,
    source_skill_id: Option<&str>,
    approve_security_once: bool,
    replace: bool,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    skills::execute_skills_install_with_config(
        path,
        bundled_skill_id,
        skill_id,
        source_skill_id,
        approve_security_once,
        replace,
        config,
    )
}

pub fn skills_remove_with_config(
    skill_id: &str,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    skills::execute_skills_remove_with_config(skill_id, config)
}

pub fn skills_policy_get_with_config(
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    skills::execute_skills_policy_get_with_config(config)
}

pub fn skills_policy_set_with_config(
    enabled: Option<bool>,
    require_download_approval: Option<bool>,
    allowed_domains: Option<BTreeSet<String>>,
    blocked_domains: Option<BTreeSet<String>>,
    policy_update_approved: bool,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    skills::execute_skills_policy_set_with_config(
        enabled,
        require_download_approval,
        allowed_domains,
        blocked_domains,
        policy_update_approved,
        config,
    )
}

pub fn skills_policy_reset_with_config(
    policy_update_approved: bool,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    skills::execute_skills_policy_reset_with_config(policy_update_approved, config)
}

pub(crate) fn discover_installable_skill_roots(root: &Path) -> Result<Vec<PathBuf>, String> {
    skills::discover_installable_skill_roots(root)
}

pub(crate) fn resolve_installable_skill_id(root: &Path) -> Result<String, String> {
    skills::resolve_installable_skill_id(root)
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

#[cfg(test)]
#[path = "tools_mod_tests.rs"]
mod tests;
