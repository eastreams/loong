use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::Value;

use super::{
    BASH_EXEC_TOOL_NAME, ToolView, canonical_tool_name, execute_discoverable_tool_core_with_config,
    runtime_config, runtime_tool_view_for_runtime_config, tool_surface,
};
use super::{DELEGATE_ASYNC_TOOL_NAME, DELEGATE_TOOL_NAME, config_import};

pub(super) fn resolved_inner_tool_name_for_logs(canonical_name: &str, payload: &Value) -> String {
    if canonical_name == "tool.invoke" {
        let inner_tool_id = payload.get("tool_id").and_then(Value::as_str);
        let inner_tool_name = inner_tool_id
            .map(canonical_tool_name)
            .map(display_inner_tool_name_for_logs)
            .unwrap_or("-");
        return inner_tool_name.to_owned();
    }

    let is_direct_tool = matches!(
        canonical_name,
        "read" | "write" | "edit" | "bash" | "web" | "browse" | "memory" | "browser"
    );
    if !is_direct_tool {
        return "-".to_owned();
    }

    let direct_tool_name = canonical_name;
    let resolved_tool_name = route_direct_tool_name(direct_tool_name, payload).ok();
    let resolved_tool_name = resolved_tool_name
        .map(display_inner_tool_name_for_logs)
        .unwrap_or("-");
    resolved_tool_name.to_owned()
}

pub(super) fn execute_direct_tool_core_with_config(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let routed_request = route_direct_tool_request(request, config)?;
    execute_discoverable_tool_core_with_config(routed_request, config)
}

fn route_direct_tool_request(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreRequest, String> {
    let tool_name = request.tool_name;
    let mut payload = request.payload;
    let runtime_view = runtime_tool_view_for_runtime_config(config);
    let routed_tool_name =
        route_direct_tool_name_for_view(tool_name.as_str(), &payload, &runtime_view)?;
    let tool_visible = runtime_view.contains(routed_tool_name);
    if !tool_visible {
        let routed_tool_display = routed_tool_display_name(routed_tool_name);
        let unavailable_hint = unavailable_runtime_hint(routed_tool_name, &runtime_view);
        return Err(format!(
            "tool_surface_unavailable: `{}` cannot route to `{}` in this runtime{}",
            tool_name, routed_tool_display, unavailable_hint
        ));
    }

    normalize_direct_payload_for_routed_tool(tool_name.as_str(), routed_tool_name, &mut payload);

    Ok(ToolCoreRequest {
        tool_name: routed_tool_name.to_owned(),
        payload,
    })
}

fn route_direct_tool_name_for_view(
    tool_name: &str,
    payload: &Value,
    view: &ToolView,
) -> Result<&'static str, String> {
    match tool_name {
        "web" => route_direct_web_tool_name_for_view(payload, view),
        "browse" | "browser" => route_direct_browser_tool_name_for_view(payload, view),
        _ => route_direct_tool_name(tool_name, payload),
    }
}

fn route_direct_browser_tool_name_for_view(
    payload: &Value,
    view: &ToolView,
) -> Result<&'static str, String> {
    let routed_tool_name = route_direct_browser_tool_name(payload)?;
    let page_inspection_available = tool_surface::browser_page_inspection_available_in_view(view);
    if page_inspection_available {
        return Ok(routed_tool_name);
    }
    Err("browser page inspection is unavailable in this runtime".to_owned())
}

pub(crate) fn route_direct_tool_name(
    tool_name: &str,
    payload: &Value,
) -> Result<&'static str, String> {
    match tool_name {
        "read" => route_direct_read_tool_name(payload),
        "write" => route_direct_write_tool_name(payload),
        "edit" => route_direct_edit_tool_name(payload),
        "bash" => route_direct_bash_tool_name(payload),
        "web" => route_direct_web_tool_name(payload),
        "browse" | "browser" => route_direct_browser_tool_name(payload),
        "memory" => route_direct_memory_tool_name(payload),
        _ => Ok("-"),
    }
}

fn route_direct_bash_tool_name(payload: &Value) -> Result<&'static str, String> {
    let command = payload
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if command.is_none() {
        return Err("direct_bash_requires_command: expected `command`".to_owned());
    }
    Ok(BASH_EXEC_TOOL_NAME)
}

fn route_direct_read_tool_name(payload: &Value) -> Result<&'static str, String> {
    let has_path = payload_has_non_empty_string_field(payload, "path");
    let has_query = payload_has_non_empty_string_field(payload, "query");
    let has_pattern = payload_has_non_empty_string_field(payload, "pattern")
        || payload_has_non_empty_string_field(payload, "glob");

    if !has_path && !has_query && !has_pattern {
        return Err(
            "direct_read_requires_one_of: expected exactly one of `path`, `query`, or `pattern`"
                .to_owned(),
        );
    }

    if has_path {
        return Ok("file.read");
    }

    if has_query {
        return Ok("content.search");
    }

    Ok("glob.search")
}

fn payload_has_non_empty_string_field(payload: &Value, field_name: &str) -> bool {
    payload
        .get(field_name)
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

fn route_direct_write_tool_name(payload: &Value) -> Result<&'static str, String> {
    let has_content = payload_has_non_null_field(payload, "content");
    if !payload_has_non_null_field(payload, "path") {
        return Err("direct_write_requires_path: expected `path` for direct write".to_owned());
    }
    if !has_content {
        return Err("direct_write_requires_content: expected `content`".to_owned());
    }
    Ok("file.write")
}

fn route_direct_edit_tool_name(payload: &Value) -> Result<&'static str, String> {
    let has_edits = payload_has_non_null_field(payload, "edits");
    let has_old_string = payload_has_non_null_field(payload, "old_string");
    let has_new_string = payload_has_non_null_field(payload, "new_string");
    let legacy_edit_mode = has_old_string || has_new_string;
    let mode_count = count_true([has_edits, legacy_edit_mode]);

    if mode_count == 0 {
        return Err(
            "direct_edit_requires_one_mode: expected `edits`, or legacy `old_string` and `new_string`"
                .to_owned(),
        );
    }
    if mode_count > 1 {
        return Err(
            "direct_edit_ambiguous: do not mix `edits` with legacy `old_string` / `new_string`"
                .to_owned(),
        );
    }
    if !payload_has_non_null_field(payload, "path") {
        return Err("direct_edit_requires_path: expected `path` for direct edit".to_owned());
    }
    if legacy_edit_mode && (!has_old_string || !has_new_string) {
        return Err(
            "direct_edit_requires_complete_legacy_fields: expected both `old_string` and `new_string`"
                .to_owned(),
        );
    }
    Ok("file.edit")
}

pub(super) fn route_direct_web_tool_name(payload: &Value) -> Result<&'static str, String> {
    let has_url = payload_has_non_null_field(payload, "url");
    let has_query = payload_has_non_null_field(payload, "query");
    let has_method = payload_has_non_null_field(payload, "method");
    let has_headers = payload_has_non_null_field(payload, "headers");
    let has_body = payload_has_non_null_field(payload, "body");
    let has_content_type = payload_has_non_null_field(payload, "content_type");
    let request_mode = has_method || has_headers || has_body || has_content_type;

    if has_query {
        if has_url || request_mode {
            return Err(
                "direct_web_ambiguous: use `query` for search, or `url` plus optional request fields for fetch/request mode"
                    .to_owned(),
            );
        }
        return Ok("web.search");
    }

    if !has_url {
        return Err(
            "direct_web_requires_query_or_url: expected `query`, or `url` for fetch/request mode"
                .to_owned(),
        );
    }

    if request_mode {
        return Ok("http.request");
    }

    Ok("web.fetch")
}

pub(super) fn route_direct_web_tool_name_for_view(
    payload: &Value,
    view: &ToolView,
) -> Result<&'static str, String> {
    let routed_tool_name = route_direct_web_tool_name(payload)?;
    let web_runtime_modes = tool_surface::direct_web_runtime_modes_for_view(view);
    let fetch_only_mode_requested = payload_has_non_null_field(payload, "mode");

    match routed_tool_name {
        "web.search" if !web_runtime_modes.query_search_available => {
            if web_runtime_modes.ordinary_network_access_available() {
                return Err(
                    "direct_web_search_unavailable: `web { query }` is unavailable in this runtime, but ordinary network access still works through `web { url }` or low-level request fields"
                        .to_owned(),
                );
            }
            Err(
                "direct_web_search_unavailable: `web { query }` is unavailable in this runtime"
                    .to_owned(),
            )
        }
        "web.fetch" if !web_runtime_modes.fetch_available => {
            if web_runtime_modes.request_available && !fetch_only_mode_requested {
                return Ok("http.request");
            }
            if web_runtime_modes.request_available {
                return Err(
                    "direct_web_fetch_unavailable: plain fetch mode is unavailable in this runtime; low-level request mode is still available through `web { url, method }` or other request fields"
                        .to_owned(),
                );
            }
            Err(
                "direct_web_fetch_unavailable: plain fetch mode is unavailable in this runtime"
                    .to_owned(),
            )
        }
        "http.request" if !web_runtime_modes.request_available => {
            if web_runtime_modes.fetch_available {
                return Err(
                    "direct_web_request_unavailable: low-level request mode is unavailable in this runtime, but ordinary `web { url }` fetch mode is still available"
                        .to_owned(),
                );
            }
            Err(
                "direct_web_request_unavailable: low-level request mode is unavailable in this runtime"
                    .to_owned(),
            )
        }
        _ => Ok(routed_tool_name),
    }
}

pub(super) fn route_browser_page_tool_name(payload: &Value) -> Result<&'static str, String> {
    let action = payload
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let has_url = payload_has_non_null_field(payload, "url");
    let has_session_id = payload_has_non_null_field(payload, "session_id");
    let has_link_id = payload_has_non_null_field(payload, "link_id");
    let has_selector = payload_has_non_null_field(payload, "selector");
    let mode_value = payload.get("mode").and_then(Value::as_str).map(str::trim);

    let route_for_click = || -> Result<&'static str, String> {
        if !has_session_id {
            return Err(
                "direct_browser_page_click_requires_session_id: expected `session_id` for page click actions"
                    .to_owned(),
            );
        }
        if has_link_id && has_selector {
            return Err(
                "direct_browser_page_click_ambiguous: provide either `link_id` or `selector`, not both"
                    .to_owned(),
            );
        }
        if has_link_id {
            return Ok("browser.click");
        }
        Err(
            "direct_browser_page_click_requires_link_id: expected `link_id` for page-link click actions"
                .to_owned(),
        )
    };

    if let Some(action) = action {
        return match action {
            "open" => {
                if has_url && !has_session_id {
                    Ok("browser.open")
                } else {
                    Err("direct_browser_page_open_requires_url: expected `url` without `session_id`".to_owned())
                }
            }
            "extract" => {
                if has_session_id {
                    Ok("browser.extract")
                } else {
                    Err(
                        "direct_browser_page_extract_requires_session_id: expected `session_id`"
                            .to_owned(),
                    )
                }
            }
            "click" => route_for_click(),
            _ => Err(format!(
                "direct_browser_page_unknown_action: unknown page action `{action}`"
            )),
        };
    }

    if has_url {
        return Ok("browser.open");
    }

    if has_link_id {
        return route_for_click();
    }

    if has_selector {
        if has_session_id {
            return Ok("browser.extract");
        }
        return Err(
            "direct_browser_page_extract_requires_session_id: expected `session_id` for selector-based page extraction"
                .to_owned(),
        );
    }

    if let Some(mode) = mode_value {
        let extract_modes = ["page_text", "title", "links", "selector_text"];
        if extract_modes.contains(&mode) {
            if has_session_id {
                return Ok("browser.extract");
            }
            return Err(
                "direct_browser_page_extract_requires_session_id: expected `session_id` for page extraction"
                    .to_owned(),
            );
        }
    }

    if has_session_id {
        return Ok("browser.extract");
    }

    Err(
        "direct_browser_page_requires_actionable_fields: expected `url`, or `session_id` plus the fields for extract or click"
            .to_owned(),
    )
}

pub(super) fn route_direct_browser_tool_name(payload: &Value) -> Result<&'static str, String> {
    let action = payload
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let has_url = payload_has_non_null_field(payload, "url");
    let has_session_id = payload_has_non_null_field(payload, "session_id");
    let has_text = payload_has_non_null_field(payload, "text");
    let has_condition = payload_has_non_null_field(payload, "condition");
    let has_timeout_ms = payload_has_non_null_field(payload, "timeout_ms");

    if matches!(
        action,
        Some("start" | "navigate" | "snapshot" | "wait" | "stop" | "type")
    ) || has_text
        || has_condition
        || has_timeout_ms
        || (has_session_id && has_url)
    {
        return Err(
            "direct_browser_interactive_automation_unavailable: built-in browser only supports `open`, `extract`, and page-link `click`; use the `agent-browser` skill for richer browser automation".to_owned(),
        );
    }

    route_browser_page_tool_name(payload)
}

fn route_direct_memory_tool_name(payload: &Value) -> Result<&'static str, String> {
    let has_query = payload_has_non_null_field(payload, "query");
    let has_path = payload_has_non_null_field(payload, "path");
    let mode_count = count_true([has_query, has_path]);

    if mode_count == 0 {
        return Err(
            "direct_memory_requires_one_of: expected exactly one of `query` or `path`".to_owned(),
        );
    }

    if mode_count > 1 {
        return Err(
            "direct_memory_ambiguous: provide either `query` or `path`, not both".to_owned(),
        );
    }

    if has_query {
        return Ok("memory_search");
    }

    Ok("memory_get")
}

fn hidden_operation(payload: &Value) -> Option<&str> {
    payload
        .get("operation")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn route_hidden_agent_tool_name(payload: &Value) -> Result<&'static str, String> {
    if let Some(operation) = hidden_operation(payload) {
        return match operation {
            "approval-list" => Ok("approval_requests_list"),
            "approval-status" => Ok("approval_request_status"),
            "approval-resolve" => Ok("approval_request_resolve"),
            "sessions-list" => Ok("sessions_list"),
            "session-history" => Ok("sessions_history"),
            "session-heads" => Ok("session_heads"),
            "session-path" => Ok("session_path"),
            "session-children" => Ok("session_children"),
            "session-artifacts" => Ok("session_artifacts"),
            "session-events" => Ok("session_events"),
            "session-search" => Ok("session_search"),
            "session-status" => Ok("session_status"),
            "session-wait" => Ok("session_wait"),
            "task-history" => Ok("task_history"),
            "task-events" => Ok("task_events"),
            "tasks-list" => Ok("tasks_list"),
            "tasks-search" => Ok("tasks_search"),
            "task-status" => Ok("task_status"),
            "task-wait" => Ok("task_wait"),
            "session-policy-status" => Ok("session_tool_policy_status"),
            "session-policy-set" => Ok("session_tool_policy_set"),
            "session-policy-clear" => Ok("session_tool_policy_clear"),
            "session-create-branch-summary" => Ok("session_create_branch_summary"),
            "session-create-checkpoint" => Ok("session_create_checkpoint"),
            "session-fork-head" => Ok("session_fork_head"),
            "session-pin-head" => Ok("session_pin_head"),
            "session-set-active-head" => Ok("session_set_active_head"),
            "session-unpin-head" => Ok("session_unpin_head"),
            "session-archive" => Ok("session_archive"),
            "session-cancel" => Ok("session_cancel"),
            "session-continue" => Ok("session_continue"),
            "session-recover" => Ok("session_recover"),
            "sessions-send" => Ok("sessions_send"),
            "delegate" => Ok(DELEGATE_TOOL_NAME),
            "delegate-background" => Ok(DELEGATE_ASYNC_TOOL_NAME),
            "provider-switch" => Ok("provider.switch"),
            "config-import" => Ok(config_import::CONFIG_IMPORT_TOOL_NAME),
            _ => Err(format!(
                "hidden_agent_unknown_operation: unknown agent operation `{operation}`"
            )),
        };
    }

    let has_approval_request_id = payload_has_non_null_field(payload, "approval_request_id");
    let has_decision = payload_has_non_null_field(payload, "decision");
    let has_selector = payload_has_non_null_field(payload, "selector");
    let has_task = payload_has_non_null_field(payload, "task");
    let has_background = payload
        .get("background")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let has_input_path = payload_has_non_null_field(payload, "input_path");
    let has_output_path = payload_has_non_null_field(payload, "output_path");
    let has_source = payload_has_non_null_field(payload, "source")
        || payload_has_non_null_field(payload, "source_id")
        || payload_has_non_null_field(payload, "selection_id")
        || payload_has_non_null_field(payload, "primary_source_id")
        || payload_has_non_null_field(payload, "primary_selection_id")
        || payload_has_non_null_field(payload, "safe_profile_merge")
        || payload_has_non_null_field(payload, "apply_external_skills_plan")
        || payload_has_non_null_field(payload, "force");
    let has_approval_status = payload_has_non_null_field(payload, "status");
    let has_query = payload_has_non_null_field(payload, "query");
    let has_text = payload_has_non_null_field(payload, "text");
    let has_input = payload_has_non_null_field(payload, "input");
    let has_task_id = payload_has_non_null_field(payload, "task_id");
    let has_task_ids = payload_has_non_null_field(payload, "task_ids");
    let has_task_state = payload_has_non_null_field(payload, "task_state");
    let has_stable_only = payload_has_non_null_field(payload, "stable_only");
    let has_tool_ids = payload_has_non_null_field(payload, "tool_ids");
    let has_runtime_narrowing = payload_has_non_null_field(payload, "runtime_narrowing");
    let has_session_id = payload_has_non_null_field(payload, "session_id");
    let has_session_ids = payload_has_non_null_field(payload, "session_ids");
    let has_after_id = payload_has_non_null_field(payload, "after_id");
    let has_timeout_ms = payload_has_non_null_field(payload, "timeout_ms");
    let has_limit = payload_has_non_null_field(payload, "limit");
    let has_offset = payload_has_non_null_field(payload, "offset");
    let has_state = payload_has_non_null_field(payload, "state");
    let has_kind = payload_has_non_null_field(payload, "kind");
    let has_parent_session_id = payload_has_non_null_field(payload, "parent_session_id");
    let has_overdue_only = payload_has_non_null_field(payload, "overdue_only");
    let has_include_archived = payload_has_non_null_field(payload, "include_archived");
    let has_include_delegate_lifecycle =
        payload_has_non_null_field(payload, "include_delegate_lifecycle");
    let has_dry_run = payload_has_non_null_field(payload, "dry_run");

    if has_approval_request_id {
        if has_decision {
            return Ok("approval_request_resolve");
        }
        return Ok("approval_request_status");
    }

    if has_selector {
        return Ok("provider.switch");
    }

    if has_task {
        if has_background {
            return Ok(DELEGATE_ASYNC_TOOL_NAME);
        }
        return Ok(DELEGATE_TOOL_NAME);
    }

    if has_input_path
        || has_output_path
        || has_source
        || payload_has_non_null_field(payload, "mode")
    {
        return Ok(config_import::CONFIG_IMPORT_TOOL_NAME);
    }

    if has_approval_status {
        return Ok("approval_requests_list");
    }

    if has_text {
        return Ok("sessions_send");
    }

    if has_task_id || has_task_ids {
        if has_timeout_ms {
            return Ok("task_wait");
        }
        if has_after_id {
            return Ok("task_events");
        }
        if has_limit {
            return Ok("task_history");
        }
        return Ok("task_status");
    }

    if has_query
        && (payload_has_non_null_field(payload, "task_state")
            || payload_has_non_null_field(payload, "stable_only"))
    {
        return Ok("tasks_search");
    }

    if has_task_state || has_stable_only {
        return Ok("tasks_list");
    }

    if has_input {
        return Ok("session_continue");
    }

    if has_query {
        return Ok("session_search");
    }

    if has_tool_ids || has_runtime_narrowing {
        return Ok("session_tool_policy_set");
    }

    if has_session_ids || has_dry_run {
        return Err(
            "hidden_agent_requires_operation: provide `operation` for archive, cancel, recover, or other multi-session control work"
                .to_owned(),
        );
    }

    if has_session_id {
        if has_timeout_ms {
            return Ok("session_wait");
        }
        if has_after_id {
            return Ok("session_events");
        }
        if has_limit {
            return Ok("sessions_history");
        }
        return Ok("session_status");
    }

    if has_limit
        || has_offset
        || has_state
        || has_kind
        || has_parent_session_id
        || has_overdue_only
        || has_include_archived
        || has_include_delegate_lifecycle
    {
        return Ok("sessions_list");
    }

    Err(
        "hidden_agent_requires_actionable_fields: expected approval, session, delegate, provider, or config fields; add `operation` when the request is ambiguous"
            .to_owned(),
    )
}

#[cfg(test)]
pub(crate) fn hidden_operation_for_tool_name(raw: &str) -> Option<String> {
    let canonical_name = canonical_tool_name(raw);
    match canonical_name {
        "approval_requests_list" => Some("approval-list".to_owned()),
        "approval_request_status" => Some("approval-status".to_owned()),
        "approval_request_resolve" => Some("approval-resolve".to_owned()),
        "sessions_list" => Some("sessions-list".to_owned()),
        "sessions_history" => Some("session-history".to_owned()),
        "session_heads" => Some("session-heads".to_owned()),
        "session_path" => Some("session-path".to_owned()),
        "session_children" => Some("session-children".to_owned()),
        "session_artifacts" => Some("session-artifacts".to_owned()),
        "session_events" => Some("session-events".to_owned()),
        "session_search" => Some("session-search".to_owned()),
        "session_status" => Some("session-status".to_owned()),
        "session_wait" => Some("session-wait".to_owned()),
        "task_history" => Some("task-history".to_owned()),
        "task_events" => Some("task-events".to_owned()),
        "tasks_list" => Some("tasks-list".to_owned()),
        "tasks_search" => Some("tasks-search".to_owned()),
        "task_status" => Some("task-status".to_owned()),
        "task_wait" => Some("task-wait".to_owned()),
        "session_tool_policy_status" => Some("session-policy-status".to_owned()),
        "session_tool_policy_set" => Some("session-policy-set".to_owned()),
        "session_tool_policy_clear" => Some("session-policy-clear".to_owned()),
        "session_create_branch_summary" => Some("session-create-branch-summary".to_owned()),
        "session_create_checkpoint" => Some("session-create-checkpoint".to_owned()),
        "session_fork_head" => Some("session-fork-head".to_owned()),
        "session_pin_head" => Some("session-pin-head".to_owned()),
        "session_set_active_head" => Some("session-set-active-head".to_owned()),
        "session_unpin_head" => Some("session-unpin-head".to_owned()),
        "session_archive" => Some("session-archive".to_owned()),
        "session_cancel" => Some("session-cancel".to_owned()),
        "session_continue" => Some("session-continue".to_owned()),
        "session_recover" => Some("session-recover".to_owned()),
        "sessions_send" => Some("sessions-send".to_owned()),
        DELEGATE_TOOL_NAME => Some("delegate".to_owned()),
        DELEGATE_ASYNC_TOOL_NAME => Some("delegate-background".to_owned()),
        "provider.switch" => Some("provider-switch".to_owned()),
        config_import::CONFIG_IMPORT_TOOL_NAME => Some("config-import".to_owned()),
        "skills.search" => Some("search".to_owned()),
        "skills.recommend" => Some("recommend".to_owned()),
        "skills.source_search" => Some("source-search".to_owned()),
        "skills.inspect" => Some("inspect".to_owned()),
        "skills.install" => Some("install".to_owned()),
        "skills.invoke" => Some("run".to_owned()),
        "skills.list" => Some("list".to_owned()),
        "skills.policy" => Some("policy".to_owned()),
        "skills.fetch" => Some("fetch".to_owned()),
        "skills.resolve" => Some("resolve".to_owned()),
        "skills.remove" => Some("remove".to_owned()),
        _ => canonical_name.strip_prefix("feishu.").map(str::to_owned),
    }
}

pub(super) fn payload_has_non_null_field(payload: &Value, field_name: &str) -> bool {
    payload
        .get(field_name)
        .filter(|value| !value.is_null())
        .is_some()
}

fn normalize_direct_payload_for_routed_tool(
    original_tool_name: &str,
    routed_tool_name: &str,
    payload: &mut Value,
) {
    if original_tool_name != "read" {
        return;
    }
    let Some(payload_object) = payload.as_object_mut() else {
        return;
    };

    match routed_tool_name {
        "file.read" => {
            payload_object.remove("query");
            payload_object.remove("pattern");
            payload_object.remove("glob");
            payload_object.remove("root");
            payload_object.remove("max_results");
            payload_object.remove("max_bytes_per_file");
            payload_object.remove("case_sensitive");
            payload_object.remove("include_directories");
        }
        "content.search" => {
            payload_object.remove("path");
            payload_object.remove("pattern");
            payload_object.remove("include_directories");
            payload_object.remove("offset");
            payload_object.remove("limit");
        }
        "glob.search" => {
            let pattern_missing = payload_object
                .get("pattern")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_none_or(str::is_empty);
            if pattern_missing
                && let Some(glob_value) = payload_object
                    .get("glob")
                    .cloned()
                    .filter(|value| value.as_str().map(str::trim).is_some_and(|v| !v.is_empty()))
            {
                let normalized_glob = glob_value
                    .as_str()
                    .map(normalize_direct_read_glob_alias_pattern)
                    .map(Value::String)
                    .unwrap_or(glob_value);
                payload_object.insert("pattern".to_owned(), normalized_glob);
            }
        }
        _ => {}
    }
}

fn normalize_direct_read_glob_alias_pattern(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.contains('|') && !trimmed.contains('{') && !trimmed.contains('}') {
        let parts = trimmed
            .split('|')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if parts.len() > 1 {
            return format!("{{{}}}", parts.join(","));
        }
    }
    trimmed.to_owned()
}

pub(super) fn count_true<const N: usize>(values: [bool; N]) -> usize {
    let mut count = 0usize;

    for value in values {
        if value {
            count = count.saturating_add(1);
        }
    }

    count
}

fn unavailable_runtime_hint(routed_tool_name: &str, runtime_view: &ToolView) -> &'static str {
    let _ = (routed_tool_name, runtime_view);
    ""
}

fn routed_tool_display_name(routed_tool_name: &str) -> &str {
    routed_tool_name
}

fn display_inner_tool_name_for_logs(tool_name: &str) -> &str {
    if matches!(
        tool_name,
        "browser.open" | "browser.extract" | "browser.click"
    ) {
        return "browse";
    }
    tool_name
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn direct_bash_routes_command_to_bash_exec() {
        let routed = route_direct_bash_tool_name(&json!({
            "command": "echo hello"
        }))
        .expect("bash command should route");

        assert_eq!(routed, BASH_EXEC_TOOL_NAME);
    }

    #[test]
    fn direct_bash_requires_command() {
        let error = route_direct_bash_tool_name(&json!({
            "command": "   "
        }))
        .expect_err("blank command should fail");

        assert!(error.contains("direct_bash_requires_command"));
    }

    #[test]
    fn browser_surface_unavailable_hint_mentions_read_only_fallbacks() {
        let runtime_view = ToolView::from_tool_names(["browser.open", "browser.extract"]);
        let payload = json!({
            "session_id": "browser-companion-1",
            "selector": "#submit",
            "text": "hello"
        });
        let request = loong_contracts::ToolCoreRequest {
            tool_name: "browse".to_owned(),
            payload,
        };
        let error =
            route_direct_tool_request(request, &runtime_config::ToolRuntimeConfig::default())
                .expect_err("interactive browser automation should be unavailable");

        assert!(error.contains("agent-browser"));
        assert_eq!(
            unavailable_runtime_hint("browser.extract", &runtime_view),
            ""
        );
    }

    #[test]
    fn direct_browse_routes_page_actions() {
        let open = route_direct_browser_tool_name(&json!({
            "action": "open",
            "url": "https://example.com"
        }))
        .expect("browse open should route");
        assert_eq!(open, "browser.open");

        let extract = route_direct_browser_tool_name(&json!({
            "session_id": "browser-1",
            "mode": "links"
        }))
        .expect("browse extract should route");
        assert_eq!(extract, "browser.extract");

        let click = route_direct_browser_tool_name(&json!({
            "session_id": "browser-1",
            "link_id": 1
        }))
        .expect("browse click should route");
        assert_eq!(click, "browser.click");
    }

    #[test]
    fn interactive_browser_payloads_surface_agent_browser_guidance() {
        let error = route_direct_browser_tool_name(&json!({
            "session_id": "browser-1",
            "selector": "#submit",
            "text": "hello"
        }))
        .expect_err("DOM interaction should not route through browse");
        assert!(error.contains("agent-browser"));
    }

    #[test]
    fn browse_routes_collapse_to_browse_in_logs() {
        let logged_tool_name = resolved_inner_tool_name_for_logs(
            "browse",
            &json!({
                "session_id": "browser-1",
                "link_id": 1
            }),
        );

        assert_eq!(logged_tool_name, "browse");
    }

    #[test]
    fn hidden_agent_routes_task_events_when_task_cursor_is_present() {
        let routed = route_hidden_agent_tool_name(&json!({
            "task_id": "task-root",
            "after_id": 10
        }))
        .expect("task events payload should route");

        assert_eq!(routed, "task_events");
    }

    #[test]
    fn hidden_operation_for_task_events_uses_task_events_alias() {
        let operation = hidden_operation_for_tool_name("task_events").expect("task events alias");

        assert_eq!(operation, "task-events");
    }
}
