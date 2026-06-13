use std::{fs, fs::OpenOptions, io::Write, path::Path};

use axum::{Json, http::StatusCode};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::json;

use super::state::GatewayStopRequestOutcome;
use crate::CliResult;

#[cfg(unix)]
pub(super) const GATEWAY_CONTROL_TOKEN_FILE_MODE: u32 = 0o600;
#[cfg(unix)]
pub(super) const GATEWAY_CONTROL_RUNTIME_DIR_MODE: u32 = 0o700;

type GatewayControlJsonResponse = (StatusCode, Json<serde_json::Value>);

pub(super) fn gateway_current_time_ms() -> u64 {
    crate::control_plane_device_auth::current_time_ms()
}

pub(super) fn new_gateway_control_bearer_token() -> String {
    let random_bytes = rand::random::<[u8; 32]>();
    URL_SAFE_NO_PAD.encode(random_bytes)
}

pub(super) fn write_gateway_control_token_file(path: &Path, token: &str) -> CliResult<()> {
    ensure_gateway_control_parent_dir(path)?;
    harden_gateway_control_parent_dir(path)?;

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    // TODO: add non-unix ACL support
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(GATEWAY_CONTROL_TOKEN_FILE_MODE);
    }
    let open_result = options.open(path);
    let mut file = open_result.map_err(|error| {
        format!(
            "open gateway control token file failed for {}: {error}",
            path.display()
        )
    })?;
    file.write_all(token.as_bytes()).map_err(|error| {
        format!(
            "write gateway control token file failed for {}: {error}",
            path.display()
        )
    })?;
    file.sync_all().map_err(|error| {
        format!(
            "sync gateway control token file failed for {}: {error}",
            path.display()
        )
    })?;
    harden_gateway_control_token_file(path)
}

fn ensure_gateway_control_parent_dir(path: &Path) -> CliResult<()> {
    let parent = path.parent();
    let Some(parent) = parent else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }

    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "create gateway control token parent directory failed for {}: {error}",
            parent.display()
        )
    })
}

#[cfg(unix)]
fn harden_gateway_control_parent_dir(path: &Path) -> CliResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let parent = path.parent();
    let Some(parent) = parent else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() || !parent.exists() {
        return Ok(());
    }

    let metadata = fs::metadata(parent).map_err(|error| {
        format!(
            "read gateway control runtime directory metadata failed for {}: {error}",
            parent.display()
        )
    })?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(GATEWAY_CONTROL_RUNTIME_DIR_MODE);
    fs::set_permissions(parent, permissions).map_err(|error| {
        format!(
            "set gateway control runtime directory permissions failed for {}: {error}",
            parent.display()
        )
    })
}

#[cfg(not(unix))]
fn harden_gateway_control_parent_dir(_path: &Path) -> CliResult<()> {
    // TODO: add windows ACL support
    Ok(())
}

#[cfg(unix)]
fn harden_gateway_control_token_file(path: &Path) -> CliResult<()> {
    use std::os::unix::fs::PermissionsExt;

    if !path.exists() {
        return Ok(());
    }

    let metadata = fs::metadata(path).map_err(|error| {
        format!(
            "read gateway control token metadata failed for {}: {error}",
            path.display()
        )
    })?;
    let mut permissions = metadata.permissions();
    permissions.set_mode(GATEWAY_CONTROL_TOKEN_FILE_MODE);
    fs::set_permissions(path, permissions).map_err(|error| {
        format!(
            "set gateway control token permissions failed for {}: {error}",
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn harden_gateway_control_token_file(_path: &Path) -> CliResult<()> {
    Ok(())
}

pub(super) fn remove_gateway_control_token_file(path: &Path) -> CliResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "remove gateway control token file failed for {}: {error}",
            path.display()
        )),
    }
}

pub(super) fn combine_gateway_control_task_results(
    server_result: CliResult<()>,
    cleanup_result: CliResult<()>,
) -> CliResult<()> {
    match (server_result, cleanup_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(server_error), Ok(())) => Err(server_error),
        (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
        (Err(server_error), Err(cleanup_error)) => {
            let final_error = format!("{server_error}; {cleanup_error}");
            Err(final_error)
        }
    }
}

pub(super) fn merge_gateway_control_errors(
    primary_error: String,
    secondary_error: Option<String>,
) -> String {
    let Some(secondary_error) = secondary_error else {
        return primary_error;
    };

    format!("{primary_error}; {secondary_error}")
}

pub(super) fn gateway_stop_outcome_status(outcome: GatewayStopRequestOutcome) -> StatusCode {
    match outcome {
        GatewayStopRequestOutcome::Requested => StatusCode::ACCEPTED,
        GatewayStopRequestOutcome::AlreadyRequested => StatusCode::ACCEPTED,
        GatewayStopRequestOutcome::AlreadyStopped => StatusCode::OK,
    }
}

pub(super) fn gateway_stop_outcome_message(outcome: GatewayStopRequestOutcome) -> &'static str {
    match outcome {
        GatewayStopRequestOutcome::Requested => "gateway stop requested",
        GatewayStopRequestOutcome::AlreadyRequested => "gateway stop already requested",
        GatewayStopRequestOutcome::AlreadyStopped => "gateway is not running",
    }
}

pub(super) fn gateway_stop_outcome_code(outcome: GatewayStopRequestOutcome) -> &'static str {
    match outcome {
        GatewayStopRequestOutcome::Requested => "requested",
        GatewayStopRequestOutcome::AlreadyRequested => "already_requested",
        GatewayStopRequestOutcome::AlreadyStopped => "already_stopped",
    }
}

pub(super) fn json_response(
    status_code: StatusCode,
    payload: serde_json::Value,
) -> GatewayControlJsonResponse {
    (status_code, Json(payload))
}

pub(super) fn json_error(
    status_code: StatusCode,
    code: &str,
    message: &str,
) -> GatewayControlJsonResponse {
    let payload = json!({
        "error": {
            "code": code,
            "message": message,
        }
    });
    json_response(status_code, payload)
}
