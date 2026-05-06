use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use loong_contracts::{ToolCoreOutcome, ToolCoreRequest};
use serde_json::Value;

use super::*;

pub fn execute_tool_core_with_config(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let requested_tool_name = request.tool_name.clone();
    let canonical_name = canonical_tool_name(request.tool_name.as_str()).to_owned();
    let payload = request.payload;
    let workspace_root = trusted_workspace_root_from_payload(&payload)?;
    let runtime_narrowing = trusted_runtime_narrowing_from_payload(&payload)?;
    let mut effective_config = config
        .workspace_root
        .clone()
        .map(|workspace_root| config.with_workspace_root_override(workspace_root))
        .unwrap_or_else(|| config.clone());
    if let Some(workspace_root) = workspace_root {
        effective_config = effective_config
            .with_workspace_root_override(workspace_root.clone())
            .with_file_root_override(workspace_root);
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
    let inner_tool_name =
        super::routing::resolved_inner_tool_name_for_logs(canonical_name.as_str(), &payload);
    let started_at = std::time::Instant::now();
    let execute_request = || {
        ensure_untrusted_payload_does_not_use_reserved_internal_tool_context(
            requested_tool_name.as_str(),
            &payload,
            "payload",
        )?;
        let request = ToolCoreRequest {
            tool_name: canonical_name.clone(),
            payload,
        };
        let request = normalize_shell_request_for_execution(request);
        let effective_config = trusted_runtime_narrowing_from_payload(&request.payload)?;
        let effective_config = effective_config.map(|narrowing| config.narrowed(&narrowing));
        let config = effective_config.as_ref().unwrap_or(config);

        match canonical_name.as_str() {
            "tool.search" => tool_search::execute_tool_search_tool_with_config(request, config),
            "tool.invoke" => tool_lease::execute_tool_invoke_tool_with_config(request, config),
            "read" | "write" | "edit" | "bash" | "web" | "browse" | "memory" => {
                super::routing::execute_direct_tool_core_with_config(request, config)
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

pub(crate) fn is_expected_tool_request_error(error: &str) -> bool {
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

pub(crate) fn execute_discoverable_tool_core_with_config(
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

pub(crate) fn tool_uses_dedicated_timeout(tool_name: &str) -> bool {
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
    matches!(
        tool_name,
        "browser.open" | "browser.extract" | "browser.click"
    )
}

fn dispatch_tool_request(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    match request.tool_name.as_str() {
        config_import::CONFIG_IMPORT_TOOL_NAME => {
            config_import::execute_config_import_tool_with_config(request, config)
        }
        #[cfg(test)]
        "skills.resolve" => skills::execute_skills_resolve_tool_with_config(request, config),
        #[cfg(test)]
        "skills.search" => skills::execute_skills_search_tool_with_config(request, config),
        #[cfg(test)]
        "skills.recommend" => skills::execute_skills_recommend_tool_with_config(request, config),
        #[cfg(test)]
        "skills.source_search" => {
            skills::execute_skills_source_search_tool_with_config(request, config)
        }
        #[cfg(test)]
        "skills.inspect" => skills::execute_skills_inspect_tool_with_config(request, config),
        #[cfg(test)]
        "skills.install" => skills::execute_skills_install_tool_with_config(request, config),
        #[cfg(test)]
        "skills.list" => skills::execute_skills_list_tool_with_config(request, config),
        #[cfg(test)]
        "skills.policy" => skills::execute_skills_policy_tool_with_config(request, config),
        #[cfg(test)]
        "skills.fetch" => skills::execute_skills_fetch_tool_with_config(request, config),
        #[cfg(test)]
        "skills.remove" => skills::execute_skills_remove_tool_with_config(request, config),
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
        "read" => file::execute_file_read_tool_with_config(request, config),
        "write" => file::execute_file_write_tool_with_config(request, config),
        "edit" => file::execute_file_edit_tool_with_config(request, config),
        "glob.search" => file::execute_glob_search_tool_with_config(request, config),
        "content.search" => file::execute_content_search_tool_with_config(request, config),
        #[cfg(feature = "tool-file")]
        "memory_search" => memory_tools::execute_memory_search_tool_with_config(request, config),
        #[cfg(feature = "tool-file")]
        "memory_get" => memory_tools::execute_memory_get_tool_with_config(request, config),
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

pub(crate) fn run_blocking_with_timeout<F, T>(
    f: F,
    timeout_seconds: u64,
    tool_name: &str,
) -> Result<T, String>
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
