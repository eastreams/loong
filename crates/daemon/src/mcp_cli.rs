use serde_json::{Value, json};

use crate::{CliResult, mvp, render_string_list};

pub fn run_list_mcp_servers_cli(config_path: Option<&str>, as_json: bool) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let snapshot = mvp::mcp::collect_mcp_runtime_snapshot(&config)?;

    if as_json {
        let payload =
            build_mcp_servers_cli_json_payload(&resolved_path.display().to_string(), &snapshot);
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize MCP server output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    println!(
        "{}",
        render_mcp_servers_snapshot_text(&resolved_path.display().to_string(), &snapshot)
    );
    Ok(())
}

pub fn run_show_mcp_server_cli(
    config_path: Option<&str>,
    server_name: &str,
    as_json: bool,
) -> CliResult<()> {
    let (resolved_path, config) = mvp::config::load(config_path)?;
    let snapshot = mvp::mcp::collect_mcp_runtime_snapshot(&config)?;
    let normalized_name = normalize_mcp_server_name(server_name)?;
    let maybe_server = snapshot
        .servers
        .iter()
        .find(|server| server.name == normalized_name);
    let Some(server) = maybe_server else {
        return Err(format!(
            "MCP server `{normalized_name}` was not found in the runtime snapshot"
        ));
    };

    if as_json {
        let payload =
            build_mcp_server_detail_cli_json_payload(&resolved_path.display().to_string(), server);
        let pretty = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize MCP server detail output failed: {error}"))?;
        println!("{pretty}");
        return Ok(());
    }

    let rendered = render_mcp_server_detail_text(&resolved_path.display().to_string(), server);
    println!("{rendered}");
    Ok(())
}

pub fn build_mcp_servers_cli_json_payload(
    config_path: &str,
    snapshot: &mvp::mcp::McpRuntimeSnapshot,
) -> Value {
    json!({
        "config": config_path,
        "server_count": snapshot.servers.len(),
        "servers": snapshot
            .servers
            .iter()
            .map(mcp_runtime_server_json)
            .collect::<Vec<_>>(),
        "missing_selected_servers": snapshot.missing_selected_servers,
    })
}

pub fn build_mcp_server_detail_cli_json_payload(
    config_path: &str,
    server: &mvp::mcp::McpRuntimeServerSnapshot,
) -> Value {
    let server_json = mcp_runtime_server_json(server);
    json!({
        "config": config_path,
        "server": server_json,
    })
}

pub(crate) fn mcp_runtime_snapshot_json(snapshot: &mvp::mcp::McpRuntimeSnapshot) -> Value {
    json!({
        "servers": snapshot
            .servers
            .iter()
            .map(mcp_runtime_server_json)
            .collect::<Vec<_>>(),
        "missing_selected_servers": snapshot.missing_selected_servers,
    })
}

fn mcp_runtime_server_json(server: &mvp::mcp::McpRuntimeServerSnapshot) -> Value {
    let origins = server
        .origins
        .iter()
        .map(|origin| {
            json!({
                "kind": mcp_origin_kind_str(origin.kind),
                "source_id": origin.source_id,
            })
        })
        .collect::<Vec<_>>();
    let status_kind = mcp_server_status_kind_str(server.status.kind);
    let auth_kind = mcp_auth_status_str(server.status.auth);

    json!({
        "name": server.name,
        "enabled": server.enabled,
        "required": server.required,
        "selected_for_acp_bootstrap": server.selected_for_acp_bootstrap,
        "origins": origins,
        "status": {
            "kind": status_kind,
            "auth": auth_kind,
            "last_error": server.status.last_error,
        },
        "transport": match &server.transport {
            mvp::mcp::McpTransportSnapshot::Stdio {
                command,
                args,
                cwd,
                env_var_names,
            } => json!({
                "transport": "stdio",
                "command": command,
                "args": args,
                "cwd": cwd,
                "env_var_names": env_var_names,
            }),
            mvp::mcp::McpTransportSnapshot::StreamableHttp {
                url,
                bearer_token_env_var,
                http_header_names,
                env_http_header_names,
            } => json!({
                "transport": "streamable_http",
                "url": url,
                "bearer_token_env_var": bearer_token_env_var,
                "http_header_names": http_header_names,
                "env_http_header_names": env_http_header_names,
            }),
        },
        "enabled_tools": server.enabled_tools,
        "disabled_tools": server.disabled_tools,
        "startup_timeout_ms": server.startup_timeout_ms,
        "tool_timeout_ms": server.tool_timeout_ms,
    })
}

pub(crate) fn render_mcp_servers_snapshot_text(
    config_path: &str,
    snapshot: &mvp::mcp::McpRuntimeSnapshot,
) -> String {
    let mut lines = vec![format!("config={config_path}")];
    if snapshot.servers.is_empty() {
        lines.push("mcp_servers=none".to_owned());
    } else {
        lines.push(format!("mcp_servers={}", snapshot.servers.len()));
        for server in &snapshot.servers {
            let origins = server
                .origins
                .iter()
                .map(render_mcp_origin_label)
                .collect::<Vec<_>>()
                .join(",");
            let transport = render_mcp_transport_summary(&server.transport);
            let mut line = format!(
                "- {} status={} auth={} selected_for_acp_bootstrap={} origins={} transport={}",
                server.name,
                mcp_server_status_kind_str(server.status.kind),
                mcp_auth_status_str(server.status.auth),
                server.selected_for_acp_bootstrap,
                origins,
                transport
            );
            if let Some(last_error) = &server.status.last_error {
                line.push_str(" last_error=");
                line.push_str(last_error);
            }
            lines.push(line);
        }
    }
    if !snapshot.missing_selected_servers.is_empty() {
        lines.push(format!(
            "missing_selected_servers={}",
            snapshot.missing_selected_servers.join(",")
        ));
    }
    lines.join("\n")
}

pub(crate) fn render_mcp_server_detail_text(
    config_path: &str,
    server: &mvp::mcp::McpRuntimeServerSnapshot,
) -> String {
    let mut lines = Vec::new();
    lines.push(format!("config={config_path}"));
    lines.push(format!("name={}", server.name));
    lines.push(format!("enabled={}", server.enabled));
    lines.push(format!("required={}", server.required));
    lines.push(format!(
        "selected_for_acp_bootstrap={}",
        server.selected_for_acp_bootstrap
    ));
    lines.push(format!(
        "status={}",
        mcp_server_status_kind_str(server.status.kind)
    ));
    lines.push(format!("auth={}", mcp_auth_status_str(server.status.auth)));
    append_mcp_origin_detail_lines(&mut lines, &server.origins);
    append_mcp_transport_detail_lines(&mut lines, &server.transport);
    if !server.enabled_tools.is_empty() {
        let enabled_tools = server.enabled_tools.join(",");
        lines.push(format!("enabled_tools={enabled_tools}"));
    }
    if !server.disabled_tools.is_empty() {
        let disabled_tools = server.disabled_tools.join(",");
        lines.push(format!("disabled_tools={disabled_tools}"));
    }
    if let Some(last_error) = &server.status.last_error {
        lines.push(format!("last_error={last_error}"));
    }
    if let Some(startup_timeout_ms) = server.startup_timeout_ms {
        lines.push(format!("startup_timeout_ms={startup_timeout_ms}"));
    }
    if let Some(tool_timeout_ms) = server.tool_timeout_ms {
        lines.push(format!("tool_timeout_ms={tool_timeout_ms}"));
    }
    lines.join("\n")
}

pub(crate) fn append_mcp_runtime_snapshot_lines(
    lines: &mut Vec<String>,
    snapshot: &mvp::mcp::McpRuntimeSnapshot,
) {
    if snapshot.servers.is_empty() {
        lines.push("acp mcp_servers=none".to_owned());
    } else {
        lines.push(format!("acp mcp_servers={}", snapshot.servers.len()));
        for server in &snapshot.servers {
            let origins = server
                .origins
                .iter()
                .map(render_mcp_origin_label)
                .collect::<Vec<_>>()
                .join(",");
            let transport = render_mcp_transport_summary(&server.transport);
            let mut line = format!(
                "  acp_mcp {} status={} auth={} selected_for_acp_bootstrap={} origins={} transport={}",
                server.name,
                mcp_server_status_kind_str(server.status.kind),
                mcp_auth_status_str(server.status.auth),
                server.selected_for_acp_bootstrap,
                origins,
                transport
            );
            if let Some(last_error) = &server.status.last_error {
                line.push_str(" last_error=");
                line.push_str(last_error);
            }
            lines.push(line);
        }
    }

    if !snapshot.missing_selected_servers.is_empty() {
        let missing = snapshot.missing_selected_servers.join(",");
        lines.push(format!("acp missing_selected_mcp_servers={missing}"));
    }
}

fn append_mcp_origin_detail_lines(lines: &mut Vec<String>, origins: &[mvp::mcp::McpServerOrigin]) {
    let labels = origins
        .iter()
        .map(render_mcp_origin_label)
        .collect::<Vec<_>>()
        .join(",");
    lines.push(format!("origins={labels}"));
}

fn append_mcp_transport_detail_lines(
    lines: &mut Vec<String>,
    transport: &mvp::mcp::McpTransportSnapshot,
) {
    match transport {
        mvp::mcp::McpTransportSnapshot::Stdio {
            command,
            args,
            cwd,
            env_var_names,
        } => {
            lines.push(format!("transport=stdio:{command}"));
            if !args.is_empty() {
                let rendered_args = render_string_list(args.iter().map(String::as_str));
                lines.push(format!("transport_args={rendered_args}"));
            }
            if let Some(cwd) = cwd {
                lines.push(format!("transport_cwd={cwd}"));
            }
            if !env_var_names.is_empty() {
                let rendered_names = render_string_list(env_var_names.iter().map(String::as_str));
                lines.push(format!("transport_env_var_names={rendered_names}"));
            }
        }
        mvp::mcp::McpTransportSnapshot::StreamableHttp {
            url,
            bearer_token_env_var,
            http_header_names,
            env_http_header_names,
        } => {
            lines.push(format!("transport=streamable_http:{url}"));
            if let Some(bearer_token_env_var) = bearer_token_env_var {
                lines.push(format!(
                    "transport_bearer_token_env_var={bearer_token_env_var}"
                ));
            }
            if !http_header_names.is_empty() {
                let rendered_names =
                    render_string_list(http_header_names.iter().map(String::as_str));
                lines.push(format!("transport_http_header_names={rendered_names}"));
            }
            if !env_http_header_names.is_empty() {
                let rendered_names =
                    render_string_list(env_http_header_names.iter().map(String::as_str));
                lines.push(format!("transport_env_http_header_names={rendered_names}"));
            }
        }
    }
}

fn render_mcp_transport_summary(transport: &mvp::mcp::McpTransportSnapshot) -> String {
    match transport {
        mvp::mcp::McpTransportSnapshot::Stdio { command, .. } => {
            format!("stdio:{command}")
        }
        mvp::mcp::McpTransportSnapshot::StreamableHttp { url, .. } => {
            format!("streamable_http:{url}")
        }
    }
}

fn render_mcp_origin_label(origin: &mvp::mcp::McpServerOrigin) -> String {
    let kind = mcp_origin_kind_str(origin.kind);
    let maybe_source_id = origin.source_id.as_deref();
    let Some(source_id) = maybe_source_id else {
        return kind.to_owned();
    };
    format!("{kind}:{source_id}")
}

fn mcp_origin_kind_str(kind: mvp::mcp::McpServerOriginKind) -> &'static str {
    match kind {
        mvp::mcp::McpServerOriginKind::Config => "config",
        mvp::mcp::McpServerOriginKind::Plugin => "plugin",
        mvp::mcp::McpServerOriginKind::Managed => "managed",
        mvp::mcp::McpServerOriginKind::AcpBackendProfile => "acp_backend_profile",
        mvp::mcp::McpServerOriginKind::AcpBootstrapSelection => "acp_bootstrap_selection",
    }
}

fn mcp_server_status_kind_str(kind: mvp::mcp::McpServerStatusKind) -> &'static str {
    match kind {
        mvp::mcp::McpServerStatusKind::Pending => "pending",
        mvp::mcp::McpServerStatusKind::Connected => "connected",
        mvp::mcp::McpServerStatusKind::NeedsAuth => "needs_auth",
        mvp::mcp::McpServerStatusKind::Failed => "failed",
        mvp::mcp::McpServerStatusKind::Disabled => "disabled",
    }
}

fn mcp_auth_status_str(auth: mvp::mcp::McpAuthStatus) -> &'static str {
    match auth {
        mvp::mcp::McpAuthStatus::Unknown => "unknown",
        mvp::mcp::McpAuthStatus::Unsupported => "unsupported",
        mvp::mcp::McpAuthStatus::NotLoggedIn => "not_logged_in",
        mvp::mcp::McpAuthStatus::BearerToken => "bearer_token",
        mvp::mcp::McpAuthStatus::OAuth => "oauth",
    }
}

pub(crate) fn normalize_mcp_server_name(raw: &str) -> CliResult<String> {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Err("MCP server name must not be empty".to_owned());
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed_stdio_server() -> mvp::mcp::McpRuntimeServerSnapshot {
        mvp::mcp::McpRuntimeServerSnapshot {
            name: "docs".to_owned(),
            enabled: true,
            required: false,
            selected_for_acp_bootstrap: true,
            origins: vec![mvp::mcp::McpServerOrigin {
                kind: mvp::mcp::McpServerOriginKind::Config,
                source_id: None,
            }],
            status: mvp::mcp::McpServerStatus {
                kind: mvp::mcp::McpServerStatusKind::Failed,
                auth: mvp::mcp::McpAuthStatus::Unsupported,
                last_error: Some("stdio_command_not_found: /tmp/missing".to_owned()),
            },
            transport: mvp::mcp::McpTransportSnapshot::Stdio {
                command: "/tmp/missing".to_owned(),
                args: Vec::new(),
                cwd: None,
                env_var_names: Vec::new(),
            },
            enabled_tools: Vec::new(),
            disabled_tools: Vec::new(),
            startup_timeout_ms: None,
            tool_timeout_ms: None,
        }
    }

    #[test]
    fn render_mcp_servers_snapshot_text_includes_last_error_for_failed_servers() {
        let snapshot = mvp::mcp::McpRuntimeSnapshot {
            servers: vec![failed_stdio_server()],
            missing_selected_servers: Vec::new(),
        };

        let rendered = render_mcp_servers_snapshot_text("/tmp/loong.toml", &snapshot);

        assert!(rendered.contains("status=failed"), "rendered={rendered}");
        assert!(
            rendered.contains("last_error=stdio_command_not_found: /tmp/missing"),
            "rendered={rendered}"
        );
    }

    #[test]
    fn append_mcp_runtime_snapshot_lines_includes_last_error_for_failed_servers() {
        let snapshot = mvp::mcp::McpRuntimeSnapshot {
            servers: vec![failed_stdio_server()],
            missing_selected_servers: Vec::new(),
        };
        let mut lines = Vec::new();

        append_mcp_runtime_snapshot_lines(&mut lines, &snapshot);

        let rendered = lines.join("\n");
        assert!(
            rendered.contains("acp_mcp docs status=failed"),
            "rendered={rendered}"
        );
        assert!(
            rendered.contains("last_error=stdio_command_not_found: /tmp/missing"),
            "rendered={rendered}"
        );
    }
}
