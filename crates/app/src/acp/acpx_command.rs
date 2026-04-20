use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::CliResult;
use crate::config::LoongConfig;

use super::{
    ACPX_DEFAULT_COMMAND, ACPX_DEFAULT_NON_INTERACTIVE_PERMISSIONS, ACPX_DEFAULT_PERMISSION_MODE,
    ACPX_DEFAULT_QUEUE_OWNER_TTL_SECONDS, AcpSessionBootstrap, AcpSessionMode, AcpxMcpServerEntry,
    AcpxMcpServerEnvEntry, ResolvedAcpxProfile, build_mcp_proxy_agent_command, format_number,
    normalized_non_empty, run_process,
};

pub(super) fn resolve_profile(config: &LoongConfig) -> CliResult<ResolvedAcpxProfile> {
    let profile = config.acp.acpx_profile().cloned().unwrap_or_default();
    let command = profile
        .command()
        .unwrap_or_else(|| ACPX_DEFAULT_COMMAND.to_owned());
    let cwd = profile.cwd();
    let permission_mode = profile
        .permission_mode()
        .unwrap_or_else(|| ACPX_DEFAULT_PERMISSION_MODE.to_owned());
    let non_interactive_permissions = profile
        .non_interactive_permissions()
        .unwrap_or_else(|| ACPX_DEFAULT_NON_INTERACTIVE_PERMISSIONS.to_owned());
    let timeout_seconds = profile.timeout_seconds;
    let queue_owner_ttl_seconds = profile
        .queue_owner_ttl_seconds
        .unwrap_or(ACPX_DEFAULT_QUEUE_OWNER_TTL_SECONDS);

    if !matches!(
        permission_mode.as_str(),
        "approve-all" | "approve-reads" | "deny-all"
    ) {
        return Err(format!(
            "ACPX permission_mode must be one of: approve-all, approve-reads, deny-all (got `{permission_mode}`)"
        ));
    }
    if !matches!(non_interactive_permissions.as_str(), "deny" | "fail") {
        return Err(format!(
            "ACPX non_interactive_permissions must be one of: deny, fail (got `{non_interactive_permissions}`)"
        ));
    }
    if timeout_seconds.is_some_and(|value| !value.is_finite() || value <= 0.0) {
        return Err("ACPX timeout_seconds must be a positive finite number".to_owned());
    }
    if !queue_owner_ttl_seconds.is_finite() || queue_owner_ttl_seconds < 0.0 {
        return Err("ACPX queue_owner_ttl_seconds must be a non-negative finite number".to_owned());
    }

    Ok(ResolvedAcpxProfile {
        command,
        cwd,
        permission_mode,
        non_interactive_permissions,
        timeout_seconds,
        queue_owner_ttl_seconds,
        mcp_servers: profile.mcp_servers,
    })
}

pub(super) fn validate_requested_mcp_servers(
    config: &LoongConfig,
    profile: &ResolvedAcpxProfile,
    request: &AcpSessionBootstrap,
) -> CliResult<Vec<String>> {
    if request.mcp_servers.is_empty() {
        return Ok(Vec::new());
    }
    if !config.acp.allow_mcp_server_injection {
        return Err(
            "ACPX bootstrap requested MCP server injection but acp.allow_mcp_server_injection=false"
                .to_owned(),
        );
    }

    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    let mut missing = Vec::new();
    for raw_name in &request.mcp_servers {
        let Some(name) = normalized_non_empty(raw_name.as_str()) else {
            return Err("ACPX bootstrap mcp_servers entries must not be empty".to_owned());
        };
        if !profile.mcp_servers.contains_key(&name) {
            missing.push(name);
            continue;
        }
        if seen.insert(name.clone()) {
            selected.push(name);
        }
    }

    if missing.is_empty() {
        Ok(selected)
    } else {
        Err(format!(
            "ACPX requested mcp_servers are not configured under [acp.backends.acpx.mcp_servers]: {}",
            missing.join(", ")
        ))
    }
}

pub(super) async fn build_verb_args<I>(
    profile: &ResolvedAcpxProfile,
    timeout_ms: u64,
    agent: &str,
    cwd: &str,
    selected_mcp_servers: &[String],
    mut prefix: Vec<String>,
    command: I,
) -> CliResult<Vec<String>>
where
    I: IntoIterator<Item = String>,
{
    let raw_agent_command =
        resolve_raw_agent_command(profile, timeout_ms, agent, cwd, selected_mcp_servers).await?;
    if let Some(agent_command) = raw_agent_command {
        prefix.extend(["--agent".to_owned(), agent_command]);
    } else {
        prefix.push(agent.to_owned());
    }
    prefix.extend(command);
    Ok(prefix)
}

pub(super) async fn build_prompt_args(
    profile: &ResolvedAcpxProfile,
    timeout_ms: u64,
    agent: &str,
    cwd: &str,
    selected_mcp_servers: &[String],
) -> CliResult<Vec<String>> {
    let mut prompt_prefix = build_control_args(cwd);
    prompt_prefix.extend(build_permission_args(profile.permission_mode.as_str()));
    prompt_prefix.extend([
        "--non-interactive-permissions".to_owned(),
        profile.non_interactive_permissions.clone(),
    ]);
    if let Some(timeout_seconds) = profile.timeout_seconds {
        prompt_prefix.extend(["--timeout".to_owned(), format_number(timeout_seconds)]);
    }
    prompt_prefix.extend([
        "--ttl".to_owned(),
        format_number(profile.queue_owner_ttl_seconds),
    ]);

    build_verb_args(
        profile,
        timeout_ms,
        agent,
        cwd,
        selected_mcp_servers,
        prompt_prefix,
        Vec::<String>::new(),
    )
    .await
}

pub(super) async fn resolve_raw_agent_command(
    profile: &ResolvedAcpxProfile,
    timeout_ms: u64,
    agent: &str,
    cwd: &str,
    selected_mcp_servers: &[String],
) -> CliResult<Option<String>> {
    if selected_mcp_servers.is_empty() {
        return Ok(None);
    }

    let target_command = resolve_acpx_agent_command(profile, timeout_ms, cwd, agent).await?;
    let mcp_servers = resolve_selected_mcp_server_entries(profile, selected_mcp_servers)?;
    let proxy_command = build_mcp_proxy_agent_command(target_command.as_str(), &mcp_servers)?;
    Ok(Some(proxy_command))
}

pub(super) async fn resolve_acpx_agent_command(
    profile: &ResolvedAcpxProfile,
    timeout_ms: u64,
    cwd: &str,
    agent: &str,
) -> CliResult<String> {
    let normalized_agent = agent.trim().to_ascii_lowercase();
    let overrides = load_agent_overrides(profile, timeout_ms, cwd).await;
    Ok(overrides
        .get(&normalized_agent)
        .cloned()
        .or_else(|| builtin_agent_command(normalized_agent.as_str()))
        .unwrap_or_else(|| agent.to_owned()))
}

pub(super) async fn load_agent_overrides(
    profile: &ResolvedAcpxProfile,
    timeout_ms: u64,
    cwd: &str,
) -> BTreeMap<String, String> {
    let args = vec![
        "--cwd".to_owned(),
        cwd.to_owned(),
        "config".to_owned(),
        "show".to_owned(),
    ];
    let Ok(output) = run_process(profile.command.as_str(), &args, cwd, timeout_ms, None).await
    else {
        return BTreeMap::new();
    };
    if output.exit_code.is_some_and(|code| code != 0) {
        return BTreeMap::new();
    }

    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(output.stdout.as_str()) else {
        return BTreeMap::new();
    };
    parsed
        .get("agents")
        .and_then(serde_json::Value::as_object)
        .map(|agents| {
            agents
                .iter()
                .filter_map(|(name, entry)| {
                    entry
                        .get("command")
                        .and_then(serde_json::Value::as_str)
                        .and_then(normalized_non_empty)
                        .map(|command| (name.trim().to_ascii_lowercase(), command))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn builtin_agent_command(agent: &str) -> Option<String> {
    let command = match agent {
        "codex" => "npx @zed-industries/codex-acp",
        "claude" => "npx -y @zed-industries/claude-agent-acp",
        "gemini" => "gemini",
        "opencode" => "npx -y opencode-ai acp",
        "pi" => "npx pi-acp",
        _ => return None,
    };
    Some(command.to_owned())
}

pub(super) fn resolve_selected_mcp_server_entries(
    profile: &ResolvedAcpxProfile,
    selected_mcp_servers: &[String],
) -> CliResult<Vec<AcpxMcpServerEntry>> {
    selected_mcp_servers
        .iter()
        .map(|name| {
            let server = profile.mcp_servers.get(name).ok_or_else(|| {
                format!(
                    "ACPX requested mcp_servers are not configured under [acp.backends.acpx.mcp_servers]: {name}"
                )
            })?;
            Ok(AcpxMcpServerEntry {
                name: name.clone(),
                command: server.command.clone(),
                args: server.args.clone(),
                cwd: None,
                env: server
                    .env
                    .iter()
                    .map(|(key, value)| AcpxMcpServerEnvEntry {
                        name: key.clone(),
                        value: value.clone(),
                    })
                    .collect(),
            })
        })
        .collect()
}

pub(super) fn resolve_effective_cwd(
    request_cwd: Option<&PathBuf>,
    profile_cwd: Option<&str>,
) -> CliResult<String> {
    if let Some(path) = request_cwd {
        return Ok(path.display().to_string());
    }
    if let Some(cwd) = profile_cwd {
        return Ok(cwd.to_owned());
    }
    std::env::current_dir()
        .map(|path| path.display().to_string())
        .map_err(|error| format!("resolve current working directory for ACPX failed: {error}"))
}

pub(super) fn derive_agent_id(
    config: &LoongConfig,
    session_key: &str,
    metadata: &BTreeMap<String, String>,
) -> CliResult<String> {
    let metadata_agent = metadata
        .get("acp_agent")
        .or_else(|| metadata.get("agent"))
        .and_then(|value| normalized_non_empty(value));
    let session_agent = parse_session_key_agent_id(session_key);

    if let Some(session_agent) = session_agent {
        let resolved = config.acp.resolve_allowed_agent(session_agent.as_str())?;
        if let Some(metadata_agent) = metadata_agent {
            let metadata_resolved = config.acp.resolve_allowed_agent(metadata_agent.as_str())?;
            if metadata_resolved != resolved {
                return Err(format!(
                    "ACPX agent metadata `{metadata_resolved}` does not match session-key agent `{resolved}`"
                ));
            }
        }
        return Ok(resolved);
    }

    if let Some(metadata_agent) = metadata_agent {
        return config.acp.resolve_allowed_agent(metadata_agent.as_str());
    }

    config.acp.resolved_default_agent()
}

pub(super) fn parse_session_key_agent_id(session_key: &str) -> Option<String> {
    session_key
        .strip_prefix("agent:")
        .and_then(|remainder| remainder.split_once(':').map(|(agent, _rest)| agent.trim()))
        .filter(|agent| !agent.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn build_control_args(cwd: &str) -> Vec<String> {
    vec![
        "--format".to_owned(),
        "json".to_owned(),
        "--json-strict".to_owned(),
        "--cwd".to_owned(),
        cwd.to_owned(),
    ]
}

pub(super) fn build_permission_args(mode: &str) -> Vec<String> {
    match mode {
        "approve-all" => vec!["--approve-all".to_owned()],
        "deny-all" => vec!["--deny-all".to_owned()],
        _ => vec!["--approve-reads".to_owned()],
    }
}

pub(super) fn mode_label(mode: AcpSessionMode) -> &'static str {
    match mode {
        AcpSessionMode::Interactive => "interactive",
        AcpSessionMode::Background => "background",
        AcpSessionMode::Review => "review",
    }
}
