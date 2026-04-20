use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::Path;

use crate::CliResult;
use crate::config::{AcpxBackendConfig, AcpxMcpServerConfig, LoongConfig};

use super::config::{McpServerConfig, McpServerTransportConfig};
use super::types::{
    McpAuthStatus, McpRuntimeServerSnapshot, McpRuntimeSnapshot, McpServerOrigin,
    McpServerOriginKind, McpServerStatus, McpServerStatusKind, McpStdioServerLaunchSpec,
    McpTransportSnapshot,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct McpRegistryEntry {
    snapshot: McpRuntimeServerSnapshot,
    stdio_launch_spec: Option<McpStdioServerLaunchSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct McpRegistry {
    servers: BTreeMap<String, McpRegistryEntry>,
    missing_selected_servers: Vec<String>,
}

impl McpRegistry {
    pub fn from_config(config: &LoongConfig) -> CliResult<Self> {
        let mut registry = Self::default();

        for (raw_name, server) in &config.mcp.servers {
            let name = canonical_server_name(raw_name)?;
            let origin = McpServerOrigin {
                kind: McpServerOriginKind::Config,
                source_id: None,
            };

            registry.upsert_server_config(name, server, origin);
        }

        if let Some(profile) = config.acp.acpx_profile() {
            registry.ingest_acpx_profile(profile)?;
        }

        let bootstrap_names = config.acp.dispatch.bootstrap_mcp_server_names()?;

        for selected_name in bootstrap_names {
            let maybe_server = registry.servers.get_mut(&selected_name);

            if let Some(server) = maybe_server {
                server.snapshot.selected_for_acp_bootstrap = true;

                let origin = McpServerOrigin {
                    kind: McpServerOriginKind::AcpBootstrapSelection,
                    source_id: None,
                };

                push_origin(&mut server.snapshot.origins, origin);
                continue;
            }

            registry.missing_selected_servers.push(selected_name);
        }

        Ok(registry)
    }

    pub fn snapshot(&self) -> McpRuntimeSnapshot {
        let servers = self
            .servers
            .values()
            .map(|entry| entry.snapshot.clone())
            .collect();

        McpRuntimeSnapshot {
            servers,
            missing_selected_servers: self.missing_selected_servers.clone(),
        }
    }

    pub fn injectable_stdio_server_count(&self) -> usize {
        let mut count = 0;

        for entry in self.servers.values() {
            let is_stdio = entry.stdio_launch_spec.is_some();
            let is_enabled = entry.snapshot.enabled;
            let status_kind = entry.snapshot.status.kind;
            let is_launchable = matches!(
                status_kind,
                McpServerStatusKind::Pending | McpServerStatusKind::Connected
            );

            if is_stdio && is_enabled && is_launchable {
                count += 1;
            }
        }

        count
    }

    pub fn resolve_selected_server_names(
        &self,
        requested_names: &[String],
    ) -> CliResult<Vec<String>> {
        let mut selected_names = Vec::new();
        let mut seen_names = BTreeSet::new();
        let mut missing_names = Vec::new();

        for raw_name in requested_names {
            let normalized_name = canonical_server_name(raw_name)?;
            let inserted = seen_names.insert(normalized_name.clone());

            if !inserted {
                continue;
            }

            let contains_server = self.servers.contains_key(&normalized_name);

            if contains_server {
                selected_names.push(normalized_name);
                continue;
            }

            missing_names.push(normalized_name);
        }

        if missing_names.is_empty() {
            return Ok(selected_names);
        }

        let rendered_names = missing_names.join(", ");
        let message = format!(
            "ACPX requested mcp_servers are not configured in the shared MCP registry ([mcp.servers] or [acp.backends.acpx.mcp_servers]): {rendered_names}"
        );

        Err(message)
    }

    pub fn resolve_injectable_stdio_launch_specs(
        &self,
        selected_names: &[String],
    ) -> CliResult<Vec<McpStdioServerLaunchSpec>> {
        let mut launch_specs = Vec::new();

        for name in selected_names {
            let maybe_entry = self.servers.get(name);

            let Some(entry) = maybe_entry else {
                let message = format!(
                    "ACPX requested mcp_servers are not configured in the shared MCP registry ([mcp.servers] or [acp.backends.acpx.mcp_servers]): {name}"
                );

                return Err(message);
            };

            let is_enabled = entry.snapshot.enabled;

            if !is_enabled {
                let message = format!(
                    "ACPX requested mcp_server `{name}` exists, but it is disabled in the shared MCP registry"
                );

                return Err(message);
            }

            let status_kind = entry.snapshot.status.kind;
            let launchable_status = matches!(
                status_kind,
                McpServerStatusKind::Pending | McpServerStatusKind::Connected
            );
            if !launchable_status {
                let last_error = entry
                    .snapshot
                    .status
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "mcp_server_not_launchable".to_owned());
                let message = format!(
                    "ACPX requested mcp_server `{name}` exists, but it is not launchable in the shared MCP registry: {last_error}"
                );

                return Err(message);
            }

            let maybe_launch_spec = entry.stdio_launch_spec.clone();

            let Some(launch_spec) = maybe_launch_spec else {
                let transport_kind = transport_kind_label(&entry.snapshot.transport);
                let message = format!(
                    "ACPX requested mcp_server `{name}` exists, but its transport `{transport_kind}` is not compatible with ACPX injection; only stdio MCP servers can be proxied"
                );

                return Err(message);
            };

            launch_specs.push(launch_spec);
        }

        Ok(launch_specs)
    }

    fn ingest_acpx_profile(&mut self, profile: &AcpxBackendConfig) -> CliResult<()> {
        for (raw_name, server) in &profile.mcp_servers {
            let name = canonical_server_name(raw_name)?;
            let entry = registry_entry_from_acpx_profile(name, server);
            self.merge_or_insert(entry);
        }

        Ok(())
    }

    fn upsert_server_config(
        &mut self,
        name: String,
        server: &McpServerConfig,
        origin: McpServerOrigin,
    ) {
        let entry = registry_entry_from_config(name, server, origin);
        self.merge_or_insert(entry);
    }

    fn merge_or_insert(&mut self, mut next: McpRegistryEntry) {
        let maybe_existing = self.servers.get_mut(&next.snapshot.name);

        if let Some(existing) = maybe_existing {
            for origin in next.snapshot.origins.drain(..) {
                push_origin(&mut existing.snapshot.origins, origin);
            }
            // Keep the first-seen transport authoritative for a given canonical
            // server name so the runtime snapshot and injectable launch spec
            // cannot drift to different transports.

            return;
        }

        let name = next.snapshot.name.clone();
        self.servers.insert(name, next);
    }
}

pub fn collect_mcp_runtime_snapshot(config: &LoongConfig) -> CliResult<McpRuntimeSnapshot> {
    let registry = McpRegistry::from_config(config)?;
    let snapshot = registry.snapshot();
    Ok(snapshot)
}

fn registry_entry_from_config(
    name: String,
    server: &McpServerConfig,
    origin: McpServerOrigin,
) -> McpRegistryEntry {
    let transport = transport_snapshot(&server.transport);
    let status = mcp_server_status_from_config(server);

    let snapshot = McpRuntimeServerSnapshot {
        name,
        enabled: server.enabled,
        required: server.required,
        selected_for_acp_bootstrap: false,
        origins: vec![origin],
        status,
        transport,
        enabled_tools: server.enabled_tools.clone(),
        disabled_tools: server.disabled_tools.clone(),
        startup_timeout_ms: server.startup_timeout_ms,
        tool_timeout_ms: server.tool_timeout_ms,
    };

    let stdio_launch_spec = stdio_launch_spec_from_config(&snapshot, &server.transport);

    McpRegistryEntry {
        snapshot,
        stdio_launch_spec,
    }
}

fn registry_entry_from_acpx_profile(
    name: String,
    server: &AcpxMcpServerConfig,
) -> McpRegistryEntry {
    let transport = McpTransportSnapshot::Stdio {
        command: redact_stdio_command(server.command.as_str()),
        args: redact_stdio_args(&server.args),
        cwd: None,
        env_var_names: server.env.keys().cloned().collect(),
    };

    let snapshot = McpRuntimeServerSnapshot {
        name: name.clone(),
        enabled: true,
        required: false,
        selected_for_acp_bootstrap: false,
        origins: vec![McpServerOrigin {
            kind: McpServerOriginKind::AcpBackendProfile,
            source_id: Some("acpx".to_owned()),
        }],
        status: mcp_server_status_from_acpx_profile(server),
        transport,
        enabled_tools: Vec::new(),
        disabled_tools: Vec::new(),
        startup_timeout_ms: None,
        tool_timeout_ms: None,
    };

    let stdio_launch_spec = McpStdioServerLaunchSpec {
        name,
        command: server.command.clone(),
        args: server.args.clone(),
        env: server.env.clone(),
        cwd: None,
        startup_timeout_ms: None,
        tool_timeout_ms: None,
    };

    McpRegistryEntry {
        snapshot,
        stdio_launch_spec: Some(stdio_launch_spec),
    }
}

fn mcp_server_status_from_config(server: &McpServerConfig) -> McpServerStatus {
    if !server.enabled {
        let status = disabled_mcp_server_status();
        return status;
    }

    match &server.transport {
        McpServerTransportConfig::Stdio {
            command, env, cwd, ..
        } => stdio_mcp_server_status(command.as_str(), env, cwd.as_deref()),
        McpServerTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
        } => streamable_http_mcp_server_status(
            url.as_str(),
            bearer_token_env_var.as_deref(),
            http_headers,
            env_http_headers,
        ),
    }
}

fn mcp_server_status_from_acpx_profile(server: &AcpxMcpServerConfig) -> McpServerStatus {
    let command = server.command.as_str();
    let env = &server.env;
    stdio_mcp_server_status(command, env, None)
}

fn disabled_mcp_server_status() -> McpServerStatus {
    let kind = McpServerStatusKind::Disabled;
    let auth = McpAuthStatus::Unknown;
    let last_error = None;
    McpServerStatus {
        kind,
        auth,
        last_error,
    }
}

fn stdio_mcp_server_status(
    command: &str,
    env: &BTreeMap<String, String>,
    cwd: Option<&Path>,
) -> McpServerStatus {
    let auth = McpAuthStatus::Unsupported;

    let cwd_result = validate_stdio_cwd(cwd);
    if let Err(last_error) = cwd_result {
        let status = McpServerStatus {
            kind: McpServerStatusKind::Failed,
            auth,
            last_error: Some(last_error),
        };
        return status;
    }

    let command_result = validate_stdio_command(command, env, cwd);
    if let Err(last_error) = command_result {
        let status = McpServerStatus {
            kind: McpServerStatusKind::Failed,
            auth,
            last_error: Some(last_error),
        };
        return status;
    }

    let kind = McpServerStatusKind::Pending;
    let last_error = None;
    McpServerStatus {
        kind,
        auth,
        last_error,
    }
}

fn validate_stdio_command(
    command: &str,
    env: &BTreeMap<String, String>,
    cwd: Option<&Path>,
) -> Result<(), String> {
    let trimmed_command = command.trim();
    if trimmed_command.is_empty() {
        let error = "stdio_command_missing".to_owned();
        return Err(error);
    }

    let command_path = Path::new(trimmed_command);
    let is_path_like = command_path.components().count() > 1;

    if command_path.is_absolute() {
        let path_exists = command_path.is_file();
        if path_exists {
            return Ok(());
        }

        let rendered_command = command_path.display().to_string();
        let error = format!("stdio_command_not_found: {rendered_command}");
        return Err(error);
    }

    if is_path_like {
        let Some(cwd) = cwd else {
            return Ok(());
        };

        if !cwd.is_absolute() {
            return Ok(());
        }

        let resolved_command = cwd.join(command_path);
        let path_exists = resolved_command.is_file();
        if path_exists {
            return Ok(());
        }

        let rendered_command = resolved_command.display().to_string();
        let error = format!("stdio_command_not_found: {rendered_command}");
        return Err(error);
    }

    let search_path = resolve_stdio_command_search_path(env);
    let fallback_cwd = Path::new(".");
    let search_cwd = cwd.unwrap_or(fallback_cwd);
    let command_lookup = which::which_in(trimmed_command, search_path, search_cwd);
    let command_found = command_lookup.is_ok();
    if command_found {
        return Ok(());
    }

    let error = format!("stdio_command_not_found: {trimmed_command}");
    Err(error)
}

fn validate_stdio_cwd(cwd: Option<&Path>) -> Result<(), String> {
    let Some(cwd) = cwd else {
        return Ok(());
    };

    let cwd_exists = cwd.exists();
    if !cwd_exists {
        let is_absolute = cwd.is_absolute();
        if !is_absolute {
            return Ok(());
        }

        let rendered_cwd = cwd.display().to_string();
        let error = format!("stdio_cwd_not_found: {rendered_cwd}");
        return Err(error);
    }

    let is_directory = cwd.is_dir();
    if is_directory {
        return Ok(());
    }

    let rendered_cwd = cwd.display().to_string();
    let error = format!("stdio_cwd_not_directory: {rendered_cwd}");
    Err(error)
}

fn resolve_stdio_command_search_path(env: &BTreeMap<String, String>) -> Option<OsString> {
    let path_override = find_env_value_ignore_case(env, "PATH");
    if let Some(path_override) = path_override {
        let path_override = OsString::from(path_override);
        return Some(path_override);
    }

    std::env::var_os("PATH")
}

fn find_env_value_ignore_case<'a>(
    env: &'a BTreeMap<String, String>,
    expected_name: &str,
) -> Option<&'a str> {
    for (name, value) in env {
        let is_match = name.eq_ignore_ascii_case(expected_name);
        if !is_match {
            continue;
        }

        let value = value.as_str();
        return Some(value);
    }

    None
}

fn streamable_http_mcp_server_status(
    raw_url: &str,
    bearer_token_env_var: Option<&str>,
    http_headers: &BTreeMap<String, String>,
    env_http_headers: &BTreeMap<String, String>,
) -> McpServerStatus {
    let auth_state =
        resolve_streamable_http_auth_state(bearer_token_env_var, http_headers, env_http_headers);
    let url_result = validate_streamable_http_url(raw_url);

    if let Err(last_error) = url_result {
        let status = McpServerStatus {
            kind: McpServerStatusKind::Failed,
            auth: auth_state.auth,
            last_error: Some(last_error),
        };
        return status;
    }

    if let Some(last_error) = auth_state.last_error {
        let status = McpServerStatus {
            kind: McpServerStatusKind::NeedsAuth,
            auth: auth_state.auth,
            last_error: Some(last_error),
        };
        return status;
    }

    let kind = McpServerStatusKind::Pending;
    let auth = auth_state.auth;
    let last_error = None;
    McpServerStatus {
        kind,
        auth,
        last_error,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamableHttpAuthState {
    auth: McpAuthStatus,
    last_error: Option<String>,
}

fn resolve_streamable_http_auth_state(
    bearer_token_env_var: Option<&str>,
    http_headers: &BTreeMap<String, String>,
    env_http_headers: &BTreeMap<String, String>,
) -> StreamableHttpAuthState {
    if let Some(env_var_name) = bearer_token_env_var {
        let trimmed_name = env_var_name.trim();
        let env_is_present = environment_variable_has_value(trimmed_name);
        if env_is_present {
            let auth = McpAuthStatus::BearerToken;
            let state = StreamableHttpAuthState {
                auth,
                last_error: None,
            };
            return state;
        }

        let error = format!("streamable_http_bearer_token_env_missing: {trimmed_name}");
        let state = StreamableHttpAuthState {
            auth: McpAuthStatus::NotLoggedIn,
            last_error: Some(error),
        };
        return state;
    }

    let env_auth_header = find_header_ignore_case(env_http_headers, "authorization");
    if let Some(env_var_name) = env_auth_header {
        let trimmed_name = env_var_name.trim();
        let env_is_present = environment_variable_has_value(trimmed_name);
        if env_is_present {
            let auth = McpAuthStatus::Unknown;
            let state = StreamableHttpAuthState {
                auth,
                last_error: None,
            };
            return state;
        }

        let error = format!("streamable_http_auth_header_env_missing: {trimmed_name}");
        let state = StreamableHttpAuthState {
            auth: McpAuthStatus::NotLoggedIn,
            last_error: Some(error),
        };
        return state;
    }

    let static_auth_header = find_header_ignore_case(http_headers, "authorization");
    if let Some(static_auth_header) = static_auth_header {
        let trimmed_header = static_auth_header.trim();
        let normalized_header = trimmed_header.to_ascii_lowercase();
        let is_bearer_header = normalized_header.starts_with("bearer ");
        let auth = if is_bearer_header {
            McpAuthStatus::BearerToken
        } else {
            McpAuthStatus::Unknown
        };
        let state = StreamableHttpAuthState {
            auth,
            last_error: None,
        };
        return state;
    }

    StreamableHttpAuthState {
        auth: McpAuthStatus::Unknown,
        last_error: None,
    }
}

fn validate_streamable_http_url(raw_url: &str) -> Result<(), String> {
    let parsed_url = reqwest::Url::parse(raw_url);
    let parsed_url = match parsed_url {
        Ok(parsed_url) => parsed_url,
        Err(_) => {
            let error = "streamable_http_url_invalid: expected http:// or https:// URL".to_owned();
            return Err(error);
        }
    };

    let scheme = parsed_url.scheme();
    let is_supported_scheme = matches!(scheme, "http" | "https");
    if is_supported_scheme {
        return Ok(());
    }

    let error = "streamable_http_url_invalid: expected http:// or https:// URL".to_owned();
    Err(error)
}

fn environment_variable_has_value(name: &str) -> bool {
    let value = std::env::var_os(name);
    let Some(value) = value else {
        return false;
    };

    let is_empty = value.to_string_lossy().trim().is_empty();
    !is_empty
}

fn find_header_ignore_case<'a>(
    headers: &'a BTreeMap<String, String>,
    expected_name: &str,
) -> Option<&'a str> {
    for (name, value) in headers {
        let is_match = name.eq_ignore_ascii_case(expected_name);
        if !is_match {
            continue;
        }

        let value = value.as_str();
        return Some(value);
    }

    None
}

fn canonical_server_name(raw: &str) -> CliResult<String> {
    let normalized = raw.trim().to_ascii_lowercase();

    if normalized.is_empty() {
        return Err("MCP server names must not be empty".to_owned());
    }

    Ok(normalized)
}

fn push_origin(origins: &mut Vec<McpServerOrigin>, candidate: McpServerOrigin) {
    let already_present = origins.iter().any(|origin| origin == &candidate);

    if already_present {
        return;
    }

    origins.push(candidate);
}

fn transport_snapshot(transport: &McpServerTransportConfig) -> McpTransportSnapshot {
    match transport {
        McpServerTransportConfig::Stdio {
            command,
            args,
            env,
            cwd,
        } => McpTransportSnapshot::Stdio {
            command: redact_stdio_command(command),
            args: redact_stdio_args(args),
            cwd: cwd.as_ref().map(|value| value.display().to_string()),
            env_var_names: env.keys().cloned().collect(),
        },
        McpServerTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
        } => McpTransportSnapshot::StreamableHttp {
            url: redact_transport_url(url),
            bearer_token_env_var: bearer_token_env_var.clone(),
            http_header_names: http_headers.keys().cloned().collect(),
            env_http_header_names: env_http_headers.keys().cloned().collect(),
        },
    }
}

fn redact_stdio_command(command: &str) -> String {
    redact_transport_value(command)
}

fn redact_stdio_args(args: &[String]) -> Vec<String> {
    let mut redacted_args = Vec::new();
    let mut redact_next_value = false;

    for argument in args {
        if redact_next_value {
            let redacted_value = "<redacted>".to_owned();
            redacted_args.push(redacted_value);
            redact_next_value = false;
            continue;
        }

        let maybe_assignment = split_flag_assignment(argument);
        if let Some((flag, value)) = maybe_assignment {
            let is_sensitive = argument_key_looks_sensitive(flag);
            let rendered_value = if is_sensitive {
                "<redacted>".to_owned()
            } else {
                redact_transport_value(value)
            };
            let rendered_argument = format!("{flag}={rendered_value}");
            redacted_args.push(rendered_argument);
            continue;
        }

        let is_sensitive_flag = argument_key_looks_sensitive(argument);
        if is_sensitive_flag {
            redacted_args.push(argument.clone());
            redact_next_value = true;
            continue;
        }

        let redacted_argument = redact_transport_value(argument);
        redacted_args.push(redacted_argument);
    }

    redacted_args
}

fn split_flag_assignment(argument: &str) -> Option<(&str, &str)> {
    if !argument.starts_with('-') {
        return None;
    }

    argument.split_once('=')
}

fn normalized_argument_key(argument: &str) -> String {
    let trimmed = argument.trim_start_matches('-');
    let mut normalized = String::new();

    for (index, character) in trimmed.chars().enumerate() {
        let is_separator = matches!(character, '_' | '.');
        if is_separator {
            normalized.push('-');
            continue;
        }

        let is_uppercase = character.is_ascii_uppercase();
        let should_insert_separator = is_uppercase && index > 0;
        if should_insert_separator {
            normalized.push('-');
        }

        let lowercased = character.to_ascii_lowercase();
        normalized.push(lowercased);
    }

    normalized
}

fn argument_key_looks_sensitive(argument: &str) -> bool {
    if !argument.starts_with('-') {
        return false;
    }

    let normalized = normalized_argument_key(argument);
    let is_header_flag = matches!(normalized.as_str(), "h" | "header" | "headers");
    if is_header_flag {
        return true;
    }

    let compact = normalized.replace('-', "");
    let sensitive_keywords = [
        "token",
        "secret",
        "password",
        "passwd",
        "authorization",
        "cookie",
        "bearer",
    ];
    let contains_sensitive_keyword = sensitive_keywords
        .iter()
        .any(|keyword| compact.contains(keyword));
    if contains_sensitive_keyword {
        return true;
    }

    let parts = normalized
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.contains(&"auth") {
        return true;
    }

    let key_scopes = ["api", "access", "client", "private", "session", "auth"];
    let contains_key = compact.contains("key");
    let contains_key_scope = key_scopes.iter().any(|scope| compact.contains(scope));

    contains_key && contains_key_scope
}

fn redact_transport_value(value: &str) -> String {
    let looks_like_url = value.contains("://");
    if looks_like_url {
        return redact_transport_url(value);
    }

    value.to_owned()
}

fn redact_transport_url(raw: &str) -> String {
    let parsed = reqwest::Url::parse(raw);
    let Ok(mut parsed) = parsed else {
        return "<redacted-invalid-url>".to_owned();
    };

    let has_username = !parsed.username().is_empty();
    if has_username {
        let _ = parsed.set_username("<redacted>");
    }

    let has_password = parsed.password().is_some();
    if has_password {
        let _ = parsed.set_password(Some("<redacted>"));
    }

    let query_keys = parsed
        .query_pairs()
        .map(|(name, _value)| name.into_owned())
        .collect::<Vec<_>>();
    if !query_keys.is_empty() {
        parsed.set_query(None);
        let mut query_pairs = parsed.query_pairs_mut();
        for query_key in query_keys {
            query_pairs.append_pair(query_key.as_str(), "<redacted>");
        }
        drop(query_pairs);
    }

    let has_fragment = parsed.fragment().is_some();
    if has_fragment {
        parsed.set_fragment(Some("<redacted>"));
    }

    parsed.to_string()
}

fn transport_kind_label(transport: &McpTransportSnapshot) -> &'static str {
    match transport {
        McpTransportSnapshot::Stdio { .. } => "stdio",
        McpTransportSnapshot::StreamableHttp { .. } => "streamable_http",
    }
}

fn stdio_launch_spec_from_config(
    snapshot: &McpRuntimeServerSnapshot,
    transport: &McpServerTransportConfig,
) -> Option<McpStdioServerLaunchSpec> {
    let McpServerTransportConfig::Stdio {
        command,
        args,
        env,
        cwd,
    } = transport
    else {
        return None;
    };

    let cwd_string = cwd.as_ref().map(|value| value.display().to_string());

    let launch_spec = McpStdioServerLaunchSpec {
        name: snapshot.name.clone(),
        command: command.clone(),
        args: args.clone(),
        env: env.clone(),
        cwd: cwd_string,
        startup_timeout_ms: snapshot.startup_timeout_ms,
        tool_timeout_ms: snapshot.tool_timeout_ms,
    };

    Some(launch_spec)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use crate::config::{AcpConfig, AcpxMcpServerConfig};
    use crate::test_support::ScopedEnv;
    use crate::test_support::unique_temp_dir;

    use super::*;
    use crate::mcp::config::{McpConfig, McpServerConfig, McpServerTransportConfig};

    fn existing_test_command() -> String {
        let current_executable = std::env::current_exe().expect("current executable path");
        current_executable.display().to_string()
    }

    fn missing_test_command_path(label: &str) -> String {
        let root = unique_temp_dir(label);
        let missing_path = root.join("missing-command");
        missing_path.display().to_string()
    }

    fn configured_stdio_server() -> McpServerConfig {
        let command = existing_test_command();
        configured_stdio_server_with_command(command)
    }

    fn configured_stdio_server_with_command(command: String) -> McpServerConfig {
        let cwd = std::env::temp_dir();
        McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command,
                args: vec!["context7-mcp".to_owned()],
                env: BTreeMap::from([("API_TOKEN".to_owned(), "secret".to_owned())]),
                cwd: Some(cwd),
            },
            enabled: true,
            required: false,
            startup_timeout_ms: Some(15_000),
            tool_timeout_ms: Some(120_000),
            enabled_tools: vec!["search".to_owned()],
            disabled_tools: vec!["write".to_owned()],
        }
    }

    #[test]
    fn collect_mcp_runtime_snapshot_includes_config_servers_and_bootstrap_selection() {
        let expected_command = existing_test_command();
        let expected_cwd = std::env::temp_dir().display().to_string();
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([("Docs".to_owned(), configured_stdio_server())]),
            },
            acp: AcpConfig {
                dispatch: crate::config::AcpDispatchConfig {
                    bootstrap_mcp_servers: vec![" docs ".to_owned()],
                    ..crate::config::AcpDispatchConfig::default()
                },
                ..AcpConfig::default()
            },
            ..LoongConfig::default()
        };

        let snapshot = collect_mcp_runtime_snapshot(&config).expect("collect MCP snapshot");

        assert!(snapshot.missing_selected_servers.is_empty());
        assert_eq!(snapshot.servers.len(), 1);

        let server = &snapshot.servers[0];

        assert_eq!(server.name, "docs");
        assert!(server.selected_for_acp_bootstrap);
        assert_eq!(server.status.kind, McpServerStatusKind::Pending);
        assert_eq!(server.status.auth, McpAuthStatus::Unsupported);
        assert_eq!(server.enabled_tools, vec!["search".to_owned()]);
        assert_eq!(server.disabled_tools, vec!["write".to_owned()]);

        let has_config_origin = server
            .origins
            .iter()
            .any(|origin| origin.kind == McpServerOriginKind::Config && origin.source_id.is_none());
        let has_bootstrap_origin = server.origins.iter().any(|origin| {
            origin.kind == McpServerOriginKind::AcpBootstrapSelection && origin.source_id.is_none()
        });

        assert!(has_config_origin);
        assert!(has_bootstrap_origin);
        assert_eq!(
            server.transport,
            McpTransportSnapshot::Stdio {
                command: expected_command,
                args: vec!["context7-mcp".to_owned()],
                cwd: Some(expected_cwd),
                env_var_names: vec!["API_TOKEN".to_owned()],
            }
        );
    }

    #[test]
    fn collect_mcp_runtime_snapshot_includes_acpx_profile_servers() {
        let expected_command = existing_test_command();
        let config = LoongConfig {
            acp: AcpConfig {
                backends: crate::config::AcpBackendProfilesConfig {
                    acpx: Some(crate::config::AcpxBackendConfig {
                        mcp_servers: BTreeMap::from([(
                            "filesystem".to_owned(),
                            AcpxMcpServerConfig {
                                command: expected_command,
                                args: vec![
                                    "-y".to_owned(),
                                    "@modelcontextprotocol/server-filesystem".to_owned(),
                                ],
                                env: BTreeMap::from([(
                                    "NODE_ENV".to_owned(),
                                    "production".to_owned(),
                                )]),
                            },
                        )]),
                        ..crate::config::AcpxBackendConfig::default()
                    }),
                },
                ..AcpConfig::default()
            },
            ..LoongConfig::default()
        };

        let snapshot = collect_mcp_runtime_snapshot(&config).expect("collect MCP snapshot");

        assert_eq!(snapshot.servers.len(), 1);

        let server = &snapshot.servers[0];

        assert_eq!(server.name, "filesystem");

        let has_acpx_origin = server.origins.iter().any(|origin| {
            origin.kind == McpServerOriginKind::AcpBackendProfile
                && origin.source_id.as_deref() == Some("acpx")
        });

        assert!(has_acpx_origin);
        assert_eq!(server.status.kind, McpServerStatusKind::Pending);
        assert_eq!(server.status.auth, McpAuthStatus::Unsupported);
    }

    #[test]
    fn collect_mcp_runtime_snapshot_redacts_acpx_profile_servers() {
        let expected_command = existing_test_command();
        let config = LoongConfig {
            acp: AcpConfig {
                backends: crate::config::AcpBackendProfilesConfig {
                    acpx: Some(crate::config::AcpxBackendConfig {
                        mcp_servers: BTreeMap::from([(
                            "filesystem".to_owned(),
                            AcpxMcpServerConfig {
                                command: expected_command.clone(),
                                args: vec![
                                    "--apiKey=secret".to_owned(),
                                    "-H".to_owned(),
                                    "Authorization: Bearer secret".to_owned(),
                                    "https://mcp.example.com?token=secret".to_owned(),
                                ],
                                env: BTreeMap::from([(
                                    "NODE_ENV".to_owned(),
                                    "production".to_owned(),
                                )]),
                            },
                        )]),
                        ..crate::config::AcpxBackendConfig::default()
                    }),
                },
                ..AcpConfig::default()
            },
            ..LoongConfig::default()
        };

        let snapshot = collect_mcp_runtime_snapshot(&config).expect("collect MCP snapshot");
        let server = snapshot
            .servers
            .iter()
            .find(|server| server.name == "filesystem")
            .expect("filesystem server");

        assert_eq!(
            server.transport,
            McpTransportSnapshot::Stdio {
                command: expected_command,
                args: vec![
                    "--apiKey=<redacted>".to_owned(),
                    "-H".to_owned(),
                    "<redacted>".to_owned(),
                    "https://mcp.example.com/?token=%3Credacted%3E".to_owned(),
                ],
                cwd: None,
                env_var_names: vec!["NODE_ENV".to_owned()],
            }
        );
    }

    #[test]
    fn collect_mcp_runtime_snapshot_reports_missing_bootstrap_names() {
        let config = LoongConfig {
            acp: AcpConfig {
                dispatch: crate::config::AcpDispatchConfig {
                    bootstrap_mcp_servers: vec!["missing".to_owned()],
                    ..crate::config::AcpDispatchConfig::default()
                },
                ..AcpConfig::default()
            },
            ..LoongConfig::default()
        };

        let snapshot = collect_mcp_runtime_snapshot(&config).expect("collect MCP snapshot");

        assert!(snapshot.servers.is_empty());
        assert_eq!(
            snapshot.missing_selected_servers,
            vec!["missing".to_owned()]
        );
    }

    #[test]
    fn registry_merges_same_server_across_config_and_acpx_profile() {
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([("filesystem".to_owned(), configured_stdio_server())]),
            },
            acp: AcpConfig {
                backends: crate::config::AcpBackendProfilesConfig {
                    acpx: Some(crate::config::AcpxBackendConfig {
                        mcp_servers: BTreeMap::from([(
                            "filesystem".to_owned(),
                            AcpxMcpServerConfig {
                                command: "npx".to_owned(),
                                args: Vec::new(),
                                env: BTreeMap::new(),
                            },
                        )]),
                        ..crate::config::AcpxBackendConfig::default()
                    }),
                },
                dispatch: crate::config::AcpDispatchConfig {
                    bootstrap_mcp_servers: vec!["filesystem".to_owned()],
                    ..crate::config::AcpDispatchConfig::default()
                },
                ..AcpConfig::default()
            },
            ..LoongConfig::default()
        };

        let snapshot = collect_mcp_runtime_snapshot(&config).expect("collect MCP snapshot");

        assert_eq!(snapshot.servers.len(), 1);

        let server = &snapshot.servers[0];

        assert_eq!(server.name, "filesystem");
        assert_eq!(server.origins.len(), 3);
        assert!(server.selected_for_acp_bootstrap);
    }

    #[test]
    fn registry_keeps_config_transport_authoritative_for_same_name_conflicts() {
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([(
                    "shared".to_owned(),
                    McpServerConfig {
                        transport: McpServerTransportConfig::StreamableHttp {
                            url: "https://mcp.example.com".to_owned(),
                            bearer_token_env_var: Some("MCP_TOKEN".to_owned()),
                            http_headers: BTreeMap::new(),
                            env_http_headers: BTreeMap::new(),
                        },
                        enabled: true,
                        required: false,
                        startup_timeout_ms: None,
                        tool_timeout_ms: None,
                        enabled_tools: Vec::new(),
                        disabled_tools: Vec::new(),
                    },
                )]),
            },
            acp: AcpConfig {
                backends: crate::config::AcpBackendProfilesConfig {
                    acpx: Some(crate::config::AcpxBackendConfig {
                        mcp_servers: BTreeMap::from([(
                            "shared".to_owned(),
                            AcpxMcpServerConfig {
                                command: "npx".to_owned(),
                                args: vec!["@modelcontextprotocol/server-filesystem".to_owned()],
                                env: BTreeMap::new(),
                            },
                        )]),
                        ..crate::config::AcpxBackendConfig::default()
                    }),
                },
                ..AcpConfig::default()
            },
            ..LoongConfig::default()
        };

        let registry = McpRegistry::from_config(&config).expect("registry");
        let snapshot = registry.snapshot();
        let server = snapshot
            .servers
            .iter()
            .find(|server| server.name == "shared")
            .expect("shared server");
        let selected_names = vec!["shared".to_owned()];
        let error = registry
            .resolve_injectable_stdio_launch_specs(&selected_names)
            .expect_err("config transport should remain authoritative");

        assert!(matches!(
            server.transport,
            McpTransportSnapshot::StreamableHttp { .. }
        ));
        assert!(error.contains("streamable_http"), "error={error}");
    }

    #[test]
    fn registry_resolves_injectable_stdio_launch_specs_from_shared_mcp_config() {
        let expected_command = existing_test_command();
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([("Docs".to_owned(), configured_stdio_server())]),
            },
            ..LoongConfig::default()
        };

        let registry = McpRegistry::from_config(&config).expect("registry");
        let requested_names = vec![" docs ".to_owned(), "docs".to_owned()];
        let selected_names = registry
            .resolve_selected_server_names(&requested_names)
            .expect("selected names");
        let launch_specs = registry
            .resolve_injectable_stdio_launch_specs(&selected_names)
            .expect("launch specs");

        assert_eq!(selected_names, vec!["docs".to_owned()]);
        assert_eq!(launch_specs.len(), 1);
        assert_eq!(launch_specs[0].name, "docs");
        assert_eq!(launch_specs[0].command, expected_command);
        assert_eq!(launch_specs[0].args, vec!["context7-mcp".to_owned()]);
        assert_eq!(
            launch_specs[0].env,
            BTreeMap::from([("API_TOKEN".to_owned(), "secret".to_owned())])
        );
    }

    #[test]
    fn registry_rejects_disabled_servers_for_acpx_injection() {
        let disabled_server = McpServerConfig {
            enabled: false,
            ..configured_stdio_server()
        };
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([("docs".to_owned(), disabled_server)]),
            },
            ..LoongConfig::default()
        };

        let registry = McpRegistry::from_config(&config).expect("registry");
        let requested_names = vec!["docs".to_owned()];
        let selected_names = registry
            .resolve_selected_server_names(&requested_names)
            .expect("selected names");
        let error = registry
            .resolve_injectable_stdio_launch_specs(&selected_names)
            .expect_err("disabled server must be rejected for ACPX injection");

        assert!(error.contains("disabled"), "error={error}");
    }

    #[test]
    fn registry_rejects_non_stdio_servers_for_acpx_injection() {
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([(
                    "remote".to_owned(),
                    McpServerConfig {
                        transport: McpServerTransportConfig::StreamableHttp {
                            url: "https://mcp.example.com".to_owned(),
                            bearer_token_env_var: Some("MCP_TOKEN".to_owned()),
                            http_headers: BTreeMap::new(),
                            env_http_headers: BTreeMap::new(),
                        },
                        enabled: true,
                        required: false,
                        startup_timeout_ms: None,
                        tool_timeout_ms: None,
                        enabled_tools: Vec::new(),
                        disabled_tools: Vec::new(),
                    },
                )]),
            },
            ..LoongConfig::default()
        };

        let registry = McpRegistry::from_config(&config).expect("registry");
        let requested_names = vec!["remote".to_owned()];
        let selected_names = registry
            .resolve_selected_server_names(&requested_names)
            .expect("selected names");
        let error = registry
            .resolve_injectable_stdio_launch_specs(&selected_names)
            .expect_err("http server must be rejected for ACPX injection");

        assert!(error.contains("streamable_http"), "error={error}");
    }

    #[test]
    fn collect_mcp_runtime_snapshot_marks_stdio_server_failed_when_command_missing() {
        let missing_command = missing_test_command_path("loong-mcp-missing-command");
        let server = configured_stdio_server_with_command(missing_command.clone());
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([("docs".to_owned(), server)]),
            },
            ..LoongConfig::default()
        };

        let snapshot = collect_mcp_runtime_snapshot(&config).expect("collect MCP snapshot");
        let server = snapshot.servers.first().expect("docs server");
        let last_error = server
            .status
            .last_error
            .as_deref()
            .expect("failed server should expose last_error");

        assert_eq!(server.status.kind, McpServerStatusKind::Failed);
        assert_eq!(server.status.auth, McpAuthStatus::Unsupported);
        assert!(
            last_error.contains("stdio_command_not_found"),
            "last_error={last_error}"
        );
        assert!(
            last_error.contains(missing_command.as_str()),
            "last_error={last_error}"
        );
    }

    #[test]
    fn collect_mcp_runtime_snapshot_uses_configured_stdio_path_for_command_lookup() {
        let mut scoped_env = ScopedEnv::new();
        scoped_env.set("PATH", OsString::from(""));

        let command_dir = unique_temp_dir("loong-mcp-command-path-override");
        std::fs::create_dir_all(&command_dir).expect("create command directory");

        let source_executable = std::env::current_exe().expect("current executable path");
        let command_file_name = source_executable
            .file_name()
            .expect("current executable file name");
        let command_file_name = command_file_name.to_string_lossy();
        let command_file_name = command_file_name.to_string();
        let copied_command_path = command_dir.join(&command_file_name);
        let copied_bytes = std::fs::copy(&source_executable, &copied_command_path)
            .expect("copy executable into configured PATH");
        assert!(copied_bytes > 0, "copied executable should not be empty");

        let configured_path = command_dir.display().to_string();

        let server = McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command: command_file_name,
                args: Vec::new(),
                env: BTreeMap::from([("PATH".to_owned(), configured_path)]),
                cwd: None,
            },
            enabled: true,
            required: false,
            startup_timeout_ms: None,
            tool_timeout_ms: None,
            enabled_tools: Vec::new(),
            disabled_tools: Vec::new(),
        };
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([("docs".to_owned(), server)]),
            },
            ..LoongConfig::default()
        };

        let snapshot = collect_mcp_runtime_snapshot(&config).expect("collect MCP snapshot");
        let server = snapshot.servers.first().expect("docs server");

        assert_eq!(server.status.kind, McpServerStatusKind::Pending);
        assert_eq!(server.status.auth, McpAuthStatus::Unsupported);
        assert_eq!(server.status.last_error, None);
    }

    #[test]
    fn collect_mcp_runtime_snapshot_marks_relative_stdio_command_pending_when_absolute_cwd_contains_command()
     {
        let cwd = unique_temp_dir("loong-mcp-relative-command-cwd");
        std::fs::create_dir_all(&cwd).expect("create command cwd");
        let command_path = cwd.join("fake-mcp");
        std::fs::write(&command_path, "#!/bin/sh\n").expect("write fake command");

        let server = McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command: "./fake-mcp".to_owned(),
                args: Vec::new(),
                env: BTreeMap::new(),
                cwd: Some(cwd),
            },
            enabled: true,
            required: false,
            startup_timeout_ms: None,
            tool_timeout_ms: None,
            enabled_tools: Vec::new(),
            disabled_tools: Vec::new(),
        };
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([("docs".to_owned(), server)]),
            },
            ..LoongConfig::default()
        };

        let snapshot = collect_mcp_runtime_snapshot(&config).expect("collect MCP snapshot");
        let server = snapshot.servers.first().expect("docs server");

        assert_eq!(server.status.kind, McpServerStatusKind::Pending);
        assert_eq!(server.status.auth, McpAuthStatus::Unsupported);
        assert_eq!(server.status.last_error, None);
    }

    #[test]
    fn collect_mcp_runtime_snapshot_marks_stdio_server_failed_when_absolute_cwd_missing() {
        let command = existing_test_command();
        let missing_cwd = unique_temp_dir("loong-mcp-missing-cwd").join("missing-cwd");
        let server = McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command,
                args: Vec::new(),
                env: BTreeMap::new(),
                cwd: Some(missing_cwd.clone()),
            },
            enabled: true,
            required: false,
            startup_timeout_ms: None,
            tool_timeout_ms: None,
            enabled_tools: Vec::new(),
            disabled_tools: Vec::new(),
        };
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([("docs".to_owned(), server)]),
            },
            ..LoongConfig::default()
        };

        let snapshot = collect_mcp_runtime_snapshot(&config).expect("collect MCP snapshot");
        let server = snapshot.servers.first().expect("docs server");
        let last_error = server
            .status
            .last_error
            .as_deref()
            .expect("failed server should expose last_error");
        let expected_cwd = missing_cwd.display().to_string();

        assert_eq!(server.status.kind, McpServerStatusKind::Failed);
        assert_eq!(server.status.auth, McpAuthStatus::Unsupported);
        assert!(
            last_error.contains("stdio_cwd_not_found"),
            "last_error={last_error}"
        );
        assert!(
            last_error.contains(expected_cwd.as_str()),
            "last_error={last_error}"
        );
    }

    #[test]
    fn collect_mcp_runtime_snapshot_prefers_stdio_cwd_error_for_relative_command_when_absolute_cwd_missing()
     {
        let missing_cwd =
            unique_temp_dir("loong-mcp-relative-command-missing-cwd").join("missing-cwd");
        let server = McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command: "./fake-mcp".to_owned(),
                args: Vec::new(),
                env: BTreeMap::new(),
                cwd: Some(missing_cwd),
            },
            enabled: true,
            required: false,
            startup_timeout_ms: None,
            tool_timeout_ms: None,
            enabled_tools: Vec::new(),
            disabled_tools: Vec::new(),
        };
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([("docs".to_owned(), server)]),
            },
            ..LoongConfig::default()
        };

        let snapshot = collect_mcp_runtime_snapshot(&config).expect("collect MCP snapshot");
        let server = snapshot.servers.first().expect("docs server");
        let last_error = server
            .status
            .last_error
            .as_deref()
            .expect("failed server should expose last_error");

        assert_eq!(server.status.kind, McpServerStatusKind::Failed);
        assert_eq!(server.status.auth, McpAuthStatus::Unsupported);
        assert!(
            last_error.contains("stdio_cwd_not_found"),
            "last_error={last_error}"
        );
        assert!(
            !last_error.contains("stdio_command_not_found"),
            "last_error={last_error}"
        );
    }

    #[test]
    fn collect_mcp_runtime_snapshot_marks_stdio_server_failed_when_absolute_cwd_is_not_directory() {
        let cwd_root = unique_temp_dir("loong-mcp-cwd-not-directory");
        std::fs::create_dir_all(&cwd_root).expect("create cwd root");
        let cwd_file = cwd_root.join("not-a-directory");
        std::fs::write(&cwd_file, "not a directory").expect("write cwd file");
        let command = existing_test_command();
        let server = McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command,
                args: Vec::new(),
                env: BTreeMap::new(),
                cwd: Some(cwd_file.clone()),
            },
            enabled: true,
            required: false,
            startup_timeout_ms: None,
            tool_timeout_ms: None,
            enabled_tools: Vec::new(),
            disabled_tools: Vec::new(),
        };
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([("docs".to_owned(), server)]),
            },
            ..LoongConfig::default()
        };

        let snapshot = collect_mcp_runtime_snapshot(&config).expect("collect MCP snapshot");
        let server = snapshot.servers.first().expect("docs server");
        let last_error = server
            .status
            .last_error
            .as_deref()
            .expect("failed server should expose last_error");
        let expected_cwd = cwd_file.display().to_string();

        assert_eq!(server.status.kind, McpServerStatusKind::Failed);
        assert_eq!(server.status.auth, McpAuthStatus::Unsupported);
        assert!(
            last_error.contains("stdio_cwd_not_directory"),
            "last_error={last_error}"
        );
        assert!(
            last_error.contains(expected_cwd.as_str()),
            "last_error={last_error}"
        );
    }

    #[test]
    fn collect_mcp_runtime_snapshot_marks_streamable_http_server_failed_for_invalid_url() {
        let server = McpServerConfig {
            transport: McpServerTransportConfig::StreamableHttp {
                url: "ftp://mcp.example.com".to_owned(),
                bearer_token_env_var: None,
                http_headers: BTreeMap::new(),
                env_http_headers: BTreeMap::new(),
            },
            enabled: true,
            required: false,
            startup_timeout_ms: None,
            tool_timeout_ms: None,
            enabled_tools: Vec::new(),
            disabled_tools: Vec::new(),
        };
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([("remote".to_owned(), server)]),
            },
            ..LoongConfig::default()
        };

        let snapshot = collect_mcp_runtime_snapshot(&config).expect("collect MCP snapshot");
        let server = snapshot.servers.first().expect("remote server");
        let last_error = server
            .status
            .last_error
            .as_deref()
            .expect("failed server should expose last_error");

        assert_eq!(server.status.kind, McpServerStatusKind::Failed);
        assert_eq!(server.status.auth, McpAuthStatus::Unknown);
        assert_eq!(
            last_error,
            "streamable_http_url_invalid: expected http:// or https:// URL"
        );
    }

    #[test]
    fn collect_mcp_runtime_snapshot_marks_streamable_http_server_needs_auth_when_bearer_env_missing()
     {
        let mut scoped_env = ScopedEnv::new();
        scoped_env.remove("LOONG_TEST_MCP_TOKEN_MISSING");

        let server = McpServerConfig {
            transport: McpServerTransportConfig::StreamableHttp {
                url: "https://mcp.example.com".to_owned(),
                bearer_token_env_var: Some("LOONG_TEST_MCP_TOKEN_MISSING".to_owned()),
                http_headers: BTreeMap::new(),
                env_http_headers: BTreeMap::new(),
            },
            enabled: true,
            required: false,
            startup_timeout_ms: None,
            tool_timeout_ms: None,
            enabled_tools: Vec::new(),
            disabled_tools: Vec::new(),
        };
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([("remote".to_owned(), server)]),
            },
            ..LoongConfig::default()
        };

        let snapshot = collect_mcp_runtime_snapshot(&config).expect("collect MCP snapshot");
        let server = snapshot.servers.first().expect("remote server");
        let last_error = server
            .status
            .last_error
            .as_deref()
            .expect("needs_auth server should expose last_error");

        assert_eq!(server.status.kind, McpServerStatusKind::NeedsAuth);
        assert_eq!(server.status.auth, McpAuthStatus::NotLoggedIn);
        assert_eq!(
            last_error,
            "streamable_http_bearer_token_env_missing: LOONG_TEST_MCP_TOKEN_MISSING"
        );
    }

    #[test]
    fn collect_mcp_runtime_snapshot_marks_streamable_http_server_needs_auth_when_bearer_env_blank()
    {
        let mut scoped_env = ScopedEnv::new();
        scoped_env.set("LOONG_TEST_MCP_TOKEN_BLANK", OsString::from("   "));

        let server = McpServerConfig {
            transport: McpServerTransportConfig::StreamableHttp {
                url: "https://mcp.example.com".to_owned(),
                bearer_token_env_var: Some("LOONG_TEST_MCP_TOKEN_BLANK".to_owned()),
                http_headers: BTreeMap::new(),
                env_http_headers: BTreeMap::new(),
            },
            enabled: true,
            required: false,
            startup_timeout_ms: None,
            tool_timeout_ms: None,
            enabled_tools: Vec::new(),
            disabled_tools: Vec::new(),
        };
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([("remote".to_owned(), server)]),
            },
            ..LoongConfig::default()
        };

        let snapshot = collect_mcp_runtime_snapshot(&config).expect("collect MCP snapshot");
        let server = snapshot.servers.first().expect("remote server");
        let last_error = server
            .status
            .last_error
            .as_deref()
            .expect("needs_auth server should expose last_error");

        assert_eq!(server.status.kind, McpServerStatusKind::NeedsAuth);
        assert_eq!(server.status.auth, McpAuthStatus::NotLoggedIn);
        assert_eq!(
            last_error,
            "streamable_http_bearer_token_env_missing: LOONG_TEST_MCP_TOKEN_BLANK"
        );
    }

    #[test]
    fn collect_mcp_runtime_snapshot_marks_streamable_http_server_ready_when_bearer_env_present() {
        let mut scoped_env = ScopedEnv::new();
        let token_value = OsString::from("test-token");
        scoped_env.set("LOONG_TEST_MCP_TOKEN_PRESENT", token_value);

        let server = McpServerConfig {
            transport: McpServerTransportConfig::StreamableHttp {
                url: "https://mcp.example.com".to_owned(),
                bearer_token_env_var: Some("LOONG_TEST_MCP_TOKEN_PRESENT".to_owned()),
                http_headers: BTreeMap::new(),
                env_http_headers: BTreeMap::new(),
            },
            enabled: true,
            required: false,
            startup_timeout_ms: None,
            tool_timeout_ms: None,
            enabled_tools: Vec::new(),
            disabled_tools: Vec::new(),
        };
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([("remote".to_owned(), server)]),
            },
            ..LoongConfig::default()
        };

        let snapshot = collect_mcp_runtime_snapshot(&config).expect("collect MCP snapshot");
        let server = snapshot.servers.first().expect("remote server");

        assert_eq!(server.status.kind, McpServerStatusKind::Pending);
        assert_eq!(server.status.auth, McpAuthStatus::BearerToken);
        assert_eq!(server.status.last_error, None);
    }

    #[test]
    fn collect_mcp_runtime_snapshot_marks_acpx_profile_server_failed_when_command_missing() {
        let missing_command = missing_test_command_path("loong-acpx-mcp-missing-command");
        let config = LoongConfig {
            acp: AcpConfig {
                backends: crate::config::AcpBackendProfilesConfig {
                    acpx: Some(crate::config::AcpxBackendConfig {
                        mcp_servers: BTreeMap::from([(
                            "filesystem".to_owned(),
                            AcpxMcpServerConfig {
                                command: missing_command,
                                args: Vec::new(),
                                env: BTreeMap::new(),
                            },
                        )]),
                        ..crate::config::AcpxBackendConfig::default()
                    }),
                },
                ..AcpConfig::default()
            },
            ..LoongConfig::default()
        };

        let snapshot = collect_mcp_runtime_snapshot(&config).expect("collect MCP snapshot");
        let server = snapshot.servers.first().expect("filesystem server");
        let last_error = server
            .status
            .last_error
            .as_deref()
            .expect("failed server should expose last_error");

        assert_eq!(server.status.kind, McpServerStatusKind::Failed);
        assert_eq!(server.status.auth, McpAuthStatus::Unsupported);
        assert!(
            last_error.contains("stdio_command_not_found"),
            "last_error={last_error}"
        );
    }

    #[test]
    fn registry_rejects_failed_stdio_servers_for_acpx_injection() {
        let missing_command = missing_test_command_path("loong-mcp-injection-missing-command");
        let server = configured_stdio_server_with_command(missing_command);
        let config = LoongConfig {
            mcp: McpConfig {
                servers: BTreeMap::from([("docs".to_owned(), server)]),
            },
            ..LoongConfig::default()
        };

        let registry = McpRegistry::from_config(&config).expect("registry");
        let requested_names = vec!["docs".to_owned()];
        let selected_names = registry
            .resolve_selected_server_names(&requested_names)
            .expect("selected names");
        let error = registry
            .resolve_injectable_stdio_launch_specs(&selected_names)
            .expect_err("failed stdio servers must be rejected for ACPX injection");

        assert!(error.contains("not launchable"), "error={error}");
        assert!(error.contains("stdio_command_not_found"), "error={error}");
    }

    #[test]
    fn transport_snapshot_redacts_sensitive_stdio_arguments() {
        let transport = McpServerTransportConfig::Stdio {
            command: "uvx".to_owned(),
            args: vec![
                "--api-key=secret".to_owned(),
                "--token".to_owned(),
                "token-value".to_owned(),
                "https://mcp.example.com?access_token=secret".to_owned(),
            ],
            env: BTreeMap::new(),
            cwd: Some(PathBuf::from("/workspace/repo")),
        };

        let snapshot = transport_snapshot(&transport);

        assert_eq!(
            snapshot,
            McpTransportSnapshot::Stdio {
                command: "uvx".to_owned(),
                args: vec![
                    "--api-key=<redacted>".to_owned(),
                    "--token".to_owned(),
                    "<redacted>".to_owned(),
                    "https://mcp.example.com/?access_token=%3Credacted%3E".to_owned(),
                ],
                cwd: Some("/workspace/repo".to_owned()),
                env_var_names: Vec::new(),
            }
        );
    }

    #[test]
    fn transport_snapshot_redacts_camel_case_and_header_stdio_arguments() {
        let transport = McpServerTransportConfig::Stdio {
            command: "uvx".to_owned(),
            args: vec![
                "--apiKey=secret".to_owned(),
                "--accessToken".to_owned(),
                "token-value".to_owned(),
                "-H".to_owned(),
                "Authorization: Bearer secret".to_owned(),
                "--header=Cookie: session=secret".to_owned(),
            ],
            env: BTreeMap::new(),
            cwd: Some(PathBuf::from("/workspace/repo")),
        };

        let snapshot = transport_snapshot(&transport);

        assert_eq!(
            snapshot,
            McpTransportSnapshot::Stdio {
                command: "uvx".to_owned(),
                args: vec![
                    "--apiKey=<redacted>".to_owned(),
                    "--accessToken".to_owned(),
                    "<redacted>".to_owned(),
                    "-H".to_owned(),
                    "<redacted>".to_owned(),
                    "--header=<redacted>".to_owned(),
                ],
                cwd: Some("/workspace/repo".to_owned()),
                env_var_names: Vec::new(),
            }
        );
    }

    #[test]
    fn transport_snapshot_preserves_non_sensitive_author_argument() {
        let transport = McpServerTransportConfig::Stdio {
            command: "uvx".to_owned(),
            args: vec!["--author=alice".to_owned()],
            env: BTreeMap::new(),
            cwd: None,
        };

        let snapshot = transport_snapshot(&transport);

        assert_eq!(
            snapshot,
            McpTransportSnapshot::Stdio {
                command: "uvx".to_owned(),
                args: vec!["--author=alice".to_owned()],
                cwd: None,
                env_var_names: Vec::new(),
            }
        );
    }

    #[test]
    fn transport_snapshot_redacts_sensitive_http_url_components() {
        let transport = McpServerTransportConfig::StreamableHttp {
            url: "https://alice:secret@mcp.example.com/path?token=secret&mode=read#frag".to_owned(),
            bearer_token_env_var: Some("MCP_TOKEN".to_owned()),
            http_headers: BTreeMap::new(),
            env_http_headers: BTreeMap::new(),
        };

        let snapshot = transport_snapshot(&transport);

        assert_eq!(
            snapshot,
            McpTransportSnapshot::StreamableHttp {
                url: "https://%3Credacted%3E:%3Credacted%3E@mcp.example.com/path?token=%3Credacted%3E&mode=%3Credacted%3E#%3Credacted%3E".to_owned(),
                bearer_token_env_var: Some("MCP_TOKEN".to_owned()),
                http_header_names: Vec::new(),
                env_http_header_names: Vec::new(),
            }
        );
    }

    #[test]
    fn transport_snapshot_redacts_invalid_http_urls() {
        let transport = McpServerTransportConfig::StreamableHttp {
            url: "https:// bad.example/path?token=secret".to_owned(),
            bearer_token_env_var: Some("MCP_TOKEN".to_owned()),
            http_headers: BTreeMap::new(),
            env_http_headers: BTreeMap::new(),
        };

        let snapshot = transport_snapshot(&transport);

        assert_eq!(
            snapshot,
            McpTransportSnapshot::StreamableHttp {
                url: "<redacted-invalid-url>".to_owned(),
                bearer_token_env_var: Some("MCP_TOKEN".to_owned()),
                http_header_names: Vec::new(),
                env_http_header_names: Vec::new(),
            }
        );
    }
}
