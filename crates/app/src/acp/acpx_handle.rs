use base64::Engine;

use super::*;
use crate::CliResult;

pub(super) fn encode_runtime_handle_state(state: &AcpxRuntimeHandleState) -> CliResult<String> {
    let payload = serde_json::to_vec(state)
        .map_err(|error| format!("serialize ACPX runtime handle state failed: {error}"))?;
    Ok(format!(
        "{ACPX_HANDLE_PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload)
    ))
}

fn decode_runtime_handle_state(
    runtime_session_name: &str,
) -> CliResult<Option<AcpxRuntimeHandleState>> {
    let trimmed = runtime_session_name.trim();
    let Some(encoded) = trimmed.strip_prefix(ACPX_HANDLE_PREFIX) else {
        return Ok(None);
    };
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|error| format!("decode ACPX runtime handle state failed: {error}"))?;
    serde_json::from_slice::<AcpxRuntimeHandleState>(&decoded)
        .map(Some)
        .map_err(|error| format!("parse ACPX runtime handle state failed: {error}"))
}

pub(super) fn resolve_handle_state(
    profile: &ResolvedAcpxProfile,
    session: &AcpSessionHandle,
) -> CliResult<AcpxRuntimeHandleState> {
    if let Some(state) = decode_runtime_handle_state(session.runtime_session_name.as_str())? {
        return Ok(state);
    }

    let cwd = session
        .working_directory
        .as_ref()
        .map(|path| path.display().to_string())
        .or_else(|| profile.cwd.clone())
        .map(Ok)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .map(|path| path.display().to_string())
                .map_err(|error| {
                    format!("resolve current working directory for ACPX handle failed: {error}")
                })
        })?;
    let name = normalized_non_empty(session.runtime_session_name.as_str())
        .unwrap_or_else(|| session.session_key.clone());

    Ok(AcpxRuntimeHandleState {
        name,
        agent: parse_session_key_agent_id(session.session_key.as_str())
            .unwrap_or_else(|| ACPX_DEFAULT_AGENT.to_owned()),
        cwd,
        mode: "persistent".to_owned(),
        mcp_servers: Vec::new(),
        acpx_record_id: None,
        backend_session_id: session.backend_session_id.clone(),
        agent_session_id: session.agent_session_id.clone(),
    })
}
