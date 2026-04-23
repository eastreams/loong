use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::Value;

use super::{
    BASH_EXEC_TOOL_NAME, DELEGATE_ASYNC_TOOL_NAME, DELEGATE_TOOL_NAME, HIDDEN_AGENT_TOOL_NAME,
    HIDDEN_CHANNEL_TOOL_NAME, HIDDEN_SKILLS_TOOL_NAME, SHELL_EXEC_TOOL_NAME, ToolView,
    canonical_tool_name, config_import, execute_discoverable_tool_core_with_config,
    resolve_tool_execution, runtime_config, runtime_tool_view_for_runtime_config, tool_surface,
};

pub(super) fn resolved_inner_tool_name_for_logs(canonical_name: &str, payload: &Value) -> String {
    if canonical_name == "tool.invoke" {
        let inner_tool_id = payload.get("tool_id").and_then(Value::as_str);
        let inner_arguments = payload.get("arguments").unwrap_or(&Value::Null);
        let resolved_hidden_tool_name = inner_tool_id
            .and_then(|tool_id| route_hidden_discoverable_tool_name(tool_id, inner_arguments).ok());
        let inner_tool_name = resolved_hidden_tool_name
            .or_else(|| inner_tool_id.map(canonical_tool_name))
            .map(display_inner_tool_name_for_logs)
            .unwrap_or("-");
        return inner_tool_name.to_owned();
    }

    let is_direct_tool = matches!(
        canonical_name,
        "read" | "write" | "exec" | "web" | "browser" | "memory"
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
    let payload = request.payload;
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
        _ => route_direct_tool_name(tool_name, payload),
    }
}

pub(crate) fn route_direct_tool_name(
    tool_name: &str,
    payload: &Value,
) -> Result<&'static str, String> {
    match tool_name {
        "read" => route_direct_read_tool_name(payload),
        "write" => route_direct_write_tool_name(payload),
        "exec" => route_direct_exec_tool_name(payload),
        "web" => route_direct_web_tool_name(payload),
        "browser" => route_direct_browser_tool_name(payload),
        "memory" => route_direct_memory_tool_name(payload),
        _ => Ok("-"),
    }
}

fn route_direct_exec_tool_name(payload: &Value) -> Result<&'static str, String> {
    let command = payload
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let script = payload
        .get("script")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let has_args = payload_has_non_null_field(payload, "args");
    let mode_count = count_true([command.is_some(), script.is_some()]);

    if mode_count == 0 {
        return Err(
            "direct_exec_requires_command_or_script: expected `command` for argv mode, or `script` for raw shell mode"
                .to_owned(),
        );
    }

    if mode_count > 1 {
        if command == script
            && let Some(value) = command
        {
            return Ok(if !has_args && command_uses_shell_syntax(value) {
                BASH_EXEC_TOOL_NAME
            } else {
                SHELL_EXEC_TOOL_NAME
            });
        }

        return Err(
            "direct_exec_ambiguous: provide either `command` or `script`, not both".to_owned(),
        );
    }

    if script.is_some() {
        return Ok(BASH_EXEC_TOOL_NAME);
    }

    let command = command.ok_or_else(|| {
        "direct_exec_requires_command_or_script: expected `command` for argv mode, or `script` for raw shell mode"
            .to_owned()
    })?;
    let uses_shell_syntax = command_uses_shell_syntax(command);

    if !has_args && uses_shell_syntax {
        return Ok(BASH_EXEC_TOOL_NAME);
    }

    Ok(SHELL_EXEC_TOOL_NAME)
}

fn command_uses_shell_syntax(command: &str) -> bool {
    command.contains('\n')
        || command.contains("&&")
        || command.contains("||")
        || command.contains('|')
        || command.contains(';')
        || command.contains('>')
        || command.contains('<')
        || command.contains("$(")
        || command.contains('`')
}

fn route_direct_read_tool_name(payload: &Value) -> Result<&'static str, String> {
    let has_path = payload_has_non_null_field(payload, "path");
    let has_query = payload_has_non_null_field(payload, "query");
    let has_pattern = payload_has_non_null_field(payload, "pattern");
    let mode_count = count_true([has_path, has_query, has_pattern]);

    if mode_count == 0 {
        return Err(
            "direct_read_requires_one_of: expected exactly one of `path`, `query`, or `pattern`"
                .to_owned(),
        );
    }

    if mode_count > 1 {
        return Err(
            "direct_read_ambiguous: provide exactly one of `path`, `query`, or `pattern`"
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

fn route_direct_write_tool_name(payload: &Value) -> Result<&'static str, String> {
    let has_content = payload_has_non_null_field(payload, "content");
    let has_edits = payload_has_non_null_field(payload, "edits");
    let has_old_string = payload_has_non_null_field(payload, "old_string");
    let has_new_string = payload_has_non_null_field(payload, "new_string");
    let legacy_exact_edit_mode = has_old_string || has_new_string;
    let exact_edit_mode = has_edits || legacy_exact_edit_mode;
    let create_mode = has_content;
    let mode_count = count_true([create_mode, exact_edit_mode]);

    if mode_count == 0 {
        return Err(
            "direct_write_requires_one_mode: expected `path` plus `content`, `path` plus `edits`, or legacy `path` plus `old_string` and `new_string`"
                .to_owned(),
        );
    }

    if mode_count > 1 {
        return Err(
            "direct_write_ambiguous: do not mix whole-file write fields with exact-edit fields"
                .to_owned(),
        );
    }

    if !payload_has_non_null_field(payload, "path") {
        return Err("direct_write_requires_path: expected `path` for direct write".to_owned());
    }

    if create_mode {
        return Ok("file.write");
    }

    if has_edits {
        return Ok("file.edit");
    }

    if !has_old_string || !has_new_string {
        return Err(
            "direct_write_edit_requires_complete_legacy_fields: expected `edits`, or legacy `old_string` and `new_string` for exact-edit mode"
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

pub(super) fn route_direct_browser_tool_name(payload: &Value) -> Result<&'static str, String> {
    let action = payload
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let has_url = payload_has_non_null_field(payload, "url");
    let has_session_id = payload_has_non_null_field(payload, "session_id");
    let has_link_id = payload_has_non_null_field(payload, "link_id");
    let has_selector = payload_has_non_null_field(payload, "selector");
    let has_text = payload_has_non_null_field(payload, "text");
    let has_condition = payload_has_non_null_field(payload, "condition");
    let has_timeout_ms = payload_has_non_null_field(payload, "timeout_ms");
    let mode_value = payload.get("mode").and_then(Value::as_str).map(str::trim);

    let route_for_click = || -> Result<&'static str, String> {
        if !has_session_id {
            return Err(
                "direct_browser_click_requires_session_id: expected `session_id` for browser click actions"
                    .to_owned(),
            );
        }
        if has_link_id && has_selector {
            return Err(
                "direct_browser_click_ambiguous: provide either `link_id` or `selector`, not both"
                    .to_owned(),
            );
        }
        if has_link_id {
            return Ok("browser.click");
        }
        if has_selector {
            return Ok("browser.companion.click");
        }
        Err(
            "direct_browser_click_requires_target: expected `link_id` for page-link click or `selector` for managed browser click"
                .to_owned(),
        )
    };

    if let Some(action) = action {
        return match action {
            "open" => {
                if has_url && !has_session_id {
                    Ok("browser.open")
                } else {
                    Err(
                        "direct_browser_open_requires_url: expected `url` without `session_id`"
                            .to_owned(),
                    )
                }
            }
            "start" => {
                if has_url && !has_session_id {
                    Ok("browser.companion.session.start")
                } else {
                    Err(
                        "direct_browser_start_requires_url: expected `url` without `session_id`"
                            .to_owned(),
                    )
                }
            }
            "navigate" => {
                if has_url && has_session_id {
                    Ok("browser.companion.navigate")
                } else {
                    Err("direct_browser_navigate_requires_session_and_url: expected `session_id` and `url`".to_owned())
                }
            }
            "extract" => {
                if has_session_id {
                    Ok("browser.extract")
                } else {
                    Err(
                        "direct_browser_extract_requires_session_id: expected `session_id`"
                            .to_owned(),
                    )
                }
            }
            "snapshot" => {
                if has_session_id {
                    Ok("browser.companion.snapshot")
                } else {
                    Err(
                        "direct_browser_snapshot_requires_session_id: expected `session_id`"
                            .to_owned(),
                    )
                }
            }
            "wait" => {
                if has_session_id {
                    Ok("browser.companion.wait")
                } else {
                    Err("direct_browser_wait_requires_session_id: expected `session_id`".to_owned())
                }
            }
            "stop" => {
                if has_session_id {
                    Ok("browser.companion.session.stop")
                } else {
                    Err("direct_browser_stop_requires_session_id: expected `session_id`".to_owned())
                }
            }
            "click" => route_for_click(),
            "type" => {
                if has_session_id && has_selector && has_text {
                    Ok("browser.companion.type")
                } else {
                    Err("direct_browser_type_requires_session_selector_text: expected `session_id`, `selector`, and `text`".to_owned())
                }
            }
            _ => Err(format!(
                "direct_browser_unknown_action: unknown browser action `{action}`"
            )),
        };
    }

    if has_text {
        if has_session_id && has_selector {
            return Ok("browser.companion.type");
        }
        return Err(
            "direct_browser_type_requires_session_selector_text: expected `session_id`, `selector`, and `text`"
                .to_owned(),
        );
    }

    if has_url && has_session_id {
        return Ok("browser.companion.navigate");
    }

    if has_url {
        return Ok("browser.open");
    }

    if has_selector {
        return route_for_click();
    }

    if has_condition || has_timeout_ms {
        if has_session_id {
            return Ok("browser.companion.wait");
        }
        return Err(
            "direct_browser_wait_requires_session_id: expected `session_id` for browser wait"
                .to_owned(),
        );
    }

    if has_link_id {
        return route_for_click();
    }

    if let Some(mode_value) = mode_value
        && matches!(mode_value, "summary" | "html")
    {
        if has_session_id {
            return Ok("browser.companion.snapshot");
        }
        return Err(
            "direct_browser_snapshot_requires_session_id: expected `session_id` for browser snapshot"
                .to_owned(),
        );
    }

    if has_session_id {
        return Ok("browser.extract");
    }

    Err(
        "direct_browser_requires_actionable_fields: expected `url`, or `session_id` plus the fields for extract, click, type, wait, snapshot, navigate, or stop"
            .to_owned(),
    )
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

pub(super) fn route_hidden_discoverable_tool_name(
    tool_name: &str,
    payload: &Value,
) -> Result<&'static str, String> {
    let canonical_name = canonical_tool_name(tool_name);
    match canonical_name {
        HIDDEN_AGENT_TOOL_NAME => route_hidden_agent_tool_name(payload),
        HIDDEN_SKILLS_TOOL_NAME => route_hidden_skills_tool_name(payload),
        HIDDEN_CHANNEL_TOOL_NAME => route_hidden_channel_tool_name(payload),
        _ => resolve_tool_execution(canonical_name)
            .map(|resolved| resolved.canonical_name)
            .ok_or_else(|| format!("tool_not_found: unknown tool `{canonical_name}`")),
    }
}

fn hidden_operation(payload: &Value) -> Option<&str> {
    payload
        .get("operation")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn route_hidden_agent_tool_name(payload: &Value) -> Result<&'static str, String> {
    if let Some(operation) = hidden_operation(payload) {
        return match operation {
            "approval-list" => Ok("approval_requests_list"),
            "approval-status" => Ok("approval_request_status"),
            "approval-resolve" => Ok("approval_request_resolve"),
            "sessions-list" => Ok("sessions_list"),
            "session-history" => Ok("sessions_history"),
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

fn route_hidden_skills_tool_name(payload: &Value) -> Result<&'static str, String> {
    if let Some(operation) = hidden_operation(payload) {
        return match operation {
            "search" => Ok("external_skills.search"),
            "recommend" => Ok("external_skills.recommend"),
            "source-search" => Ok("external_skills.source_search"),
            "inspect" => Ok("external_skills.inspect"),
            "install" => Ok("external_skills.install"),
            "run" | "invoke" => Ok("external_skills.invoke"),
            "list" => Ok("external_skills.list"),
            "policy" => Ok("external_skills.policy"),
            "fetch" => Ok("external_skills.fetch"),
            "resolve" => Ok("external_skills.resolve"),
            "remove" => Ok("external_skills.remove"),
            _ => Err(format!(
                "hidden_skills_unknown_operation: unknown skills operation `{operation}`"
            )),
        };
    }

    let has_query = payload_has_non_null_field(payload, "query");
    let has_sources = payload_has_non_null_field(payload, "sources");
    let has_url = payload_has_non_null_field(payload, "url");
    let has_reference = payload_has_non_null_field(payload, "reference");
    let has_save_as = payload_has_non_null_field(payload, "save_as");
    let has_skill_id = payload_has_non_null_field(payload, "skill_id");
    let has_path = payload_has_non_null_field(payload, "path");
    let has_bundled_skill_id = payload_has_non_null_field(payload, "bundled_skill_id");
    let has_source_skill_id = payload_has_non_null_field(payload, "source_skill_id");
    let has_allowed_domains = payload_has_non_null_field(payload, "allowed_domains");
    let has_blocked_domains = payload_has_non_null_field(payload, "blocked_domains");
    let has_enabled = payload_has_non_null_field(payload, "enabled");

    if has_sources {
        return Ok("external_skills.source_search");
    }

    if has_allowed_domains || has_blocked_domains || has_enabled {
        return Ok("external_skills.policy");
    }

    if has_path || has_bundled_skill_id || has_source_skill_id {
        return Ok("external_skills.install");
    }

    if has_url || has_save_as {
        return Ok("external_skills.fetch");
    }

    if has_reference {
        let fetch_by_reference = payload_has_non_null_field(payload, "approval_granted")
            || payload_has_non_null_field(payload, "max_bytes");
        if fetch_by_reference {
            return Ok("external_skills.fetch");
        }
        return Ok("external_skills.resolve");
    }

    if has_query {
        return Ok("external_skills.search");
    }

    if has_skill_id {
        let invokes_skill = payload.as_object().is_some_and(|object| {
            object
                .keys()
                .any(|key| key != "skill_id" && key != "operation")
        });
        if invokes_skill {
            return Ok("external_skills.invoke");
        }
        return Err(
            "hidden_skills_requires_operation_for_skill_id: add `operation` (`inspect` or `run`) when payload.skill_id is present"
                .to_owned(),
        );
    }

    if payload.as_object().is_some_and(|object| object.is_empty()) {
        return Ok("external_skills.list");
    }

    Err(
        "hidden_skills_requires_actionable_fields: expected search, inspect, install, fetch, resolve, policy, or list fields; add `operation` when the request is ambiguous"
            .to_owned(),
    )
}

fn route_hidden_channel_tool_name(payload: &Value) -> Result<&'static str, String> {
    let Some(operation) = hidden_operation(payload) else {
        return Err(
            "hidden_channel_requires_operation: provide `operation`, such as `messages.send`, `messages.reply`, `card.update`, or `feishu.whoami`"
                .to_owned(),
        );
    };

    let normalized_operation = operation.replace(['_', '-'], ".");
    let mut candidates = vec![operation.to_owned()];
    if normalized_operation != operation {
        candidates.push(normalized_operation.clone());
    }
    if !normalized_operation.starts_with("feishu.") {
        candidates.push(format!("feishu.{normalized_operation}"));
    }

    for candidate in candidates {
        let Some(resolved) = resolve_tool_execution(candidate.as_str()) else {
            continue;
        };
        if tool_surface::tool_surface_id_for_name(resolved.canonical_name) == Some("channel") {
            return Ok(resolved.canonical_name);
        }
    }

    Err(format!(
        "hidden_channel_unknown_operation: unknown channel operation `{operation}`"
    ))
}

pub(crate) fn hidden_operation_for_tool_name(raw: &str) -> Option<String> {
    let canonical_name = canonical_tool_name(raw);
    let hidden_tool_name = super::hidden_facade_tool_name_for_hidden_tool(canonical_name)?;

    match hidden_tool_name {
        HIDDEN_AGENT_TOOL_NAME => match canonical_name {
            "approval_requests_list" => Some("approval-list".to_owned()),
            "approval_request_status" => Some("approval-status".to_owned()),
            "approval_request_resolve" => Some("approval-resolve".to_owned()),
            "sessions_list" => Some("sessions-list".to_owned()),
            "sessions_history" => Some("session-history".to_owned()),
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
            "session_archive" => Some("session-archive".to_owned()),
            "session_cancel" => Some("session-cancel".to_owned()),
            "session_continue" => Some("session-continue".to_owned()),
            "session_recover" => Some("session-recover".to_owned()),
            "sessions_send" => Some("sessions-send".to_owned()),
            DELEGATE_TOOL_NAME => Some("delegate".to_owned()),
            DELEGATE_ASYNC_TOOL_NAME => Some("delegate-background".to_owned()),
            "provider.switch" => Some("provider-switch".to_owned()),
            config_import::CONFIG_IMPORT_TOOL_NAME => Some("config-import".to_owned()),
            _ => None,
        },
        HIDDEN_SKILLS_TOOL_NAME => match canonical_name {
            "external_skills.search" => Some("search".to_owned()),
            "external_skills.recommend" => Some("recommend".to_owned()),
            "external_skills.source_search" => Some("source-search".to_owned()),
            "external_skills.inspect" => Some("inspect".to_owned()),
            "external_skills.install" => Some("install".to_owned()),
            "external_skills.invoke" => Some("run".to_owned()),
            "external_skills.list" => Some("list".to_owned()),
            "external_skills.policy" => Some("policy".to_owned()),
            "external_skills.fetch" => Some("fetch".to_owned()),
            "external_skills.resolve" => Some("resolve".to_owned()),
            "external_skills.remove" => Some("remove".to_owned()),
            _ => None,
        },
        HIDDEN_CHANNEL_TOOL_NAME => canonical_name.strip_prefix("feishu.").map(str::to_owned),
        _ => None,
    }
}

pub(super) fn payload_has_non_null_field(payload: &Value, field_name: &str) -> bool {
    payload
        .get(field_name)
        .filter(|value| !value.is_null())
        .is_some()
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
    if !routed_tool_name.starts_with("browser.companion.") {
        return "";
    }

    if runtime_view.contains("browser.open") || runtime_view.contains("browser.extract") {
        return "; read-only browser inspection is still available";
    }

    "; browser interaction is unavailable in this runtime"
}

fn routed_tool_display_name(routed_tool_name: &str) -> &str {
    if routed_tool_name.starts_with("browser.companion.") {
        return "managed browser actions";
    }

    routed_tool_name
}

fn display_inner_tool_name_for_logs(tool_name: &str) -> &str {
    if tool_name.starts_with("browser.companion.") {
        return "browser";
    }

    tool_name
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn direct_exec_normalizes_duplicate_command_and_script_sources() {
        let routed = route_direct_exec_tool_name(&json!({
            "command": "echo hello",
            "script": "echo hello"
        }))
        .expect("equivalent exec sources should normalize");

        assert_eq!(routed, SHELL_EXEC_TOOL_NAME);
    }

    #[test]
    fn direct_exec_ignores_blank_aliases() {
        let routed = route_direct_exec_tool_name(&json!({
            "command": "   ",
            "script": "echo hello"
        }))
        .expect("blank command alias should be ignored");

        assert_eq!(routed, BASH_EXEC_TOOL_NAME);
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
            tool_name: "browser".to_owned(),
            payload: payload.clone(),
        };
        let managed_browser_route =
            route_direct_browser_tool_name(&payload).expect("managed browser payload should route");

        let error =
            route_direct_tool_request(request, &runtime_config::ToolRuntimeConfig::default())
                .expect_err("managed browser type should be unavailable in default runtime");

        assert!(error.contains("managed browser actions"));
        assert!(error.contains("read-only browser inspection"));
        assert!(
            unavailable_runtime_hint(managed_browser_route, &runtime_view)
                .contains("read-only browser inspection")
        );
    }

    #[test]
    fn browser_companion_routes_collapse_to_browser_in_logs() {
        let logged_tool_name = resolved_inner_tool_name_for_logs(
            "browser",
            &json!({
                "session_id": "browser-companion-1",
                "selector": "#submit",
                "text": "hello"
            }),
        );

        assert_eq!(logged_tool_name, "browser");
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
