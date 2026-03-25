use serde_json::json;

use crate::{CliResult, config::ResolvedSignalChannelConfig};

use super::ChannelOutboundTargetKind;

pub(super) async fn run_signal_send(
    resolved: &ResolvedSignalChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
) -> CliResult<()> {
    if target_kind != ChannelOutboundTargetKind::Address {
        return Err(format!(
            "signal send requires address target kind, got {}",
            target_kind.as_str()
        ));
    }

    let service_url = resolved
        .service_url()
        .ok_or_else(|| "signal service_url missing (set signal.service_url or env)".to_owned())?;
    let account = resolved
        .signal_account()
        .ok_or_else(|| "signal account missing (set signal.account or env)".to_owned())?;
    let recipient = target_id.trim();
    if recipient.is_empty() {
        return Err("signal outbound target id is empty".to_owned());
    }

    let request_url = format!("{}/v2/send", service_url.trim_end_matches('/'));
    let request_body = json!({
        "message": text,
        "number": account,
        "recipients": [recipient],
    });

    let client = reqwest::Client::new();
    let request = client.post(request_url.as_str()).json(&request_body);
    let response = request
        .send()
        .await
        .map_err(|error| format!("signal send failed: {error}"))?;
    ensure_signal_success(response).await
}

async fn ensure_signal_success(response: reqwest::Response) -> CliResult<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let body = response
        .text()
        .await
        .map_err(|error| format!("read signal error response failed: {error}"))?;
    let trimmed_body = body.trim();
    let detail = if trimmed_body.is_empty() {
        "empty response body".to_owned()
    } else {
        trimmed_body.to_owned()
    };
    Err(format!(
        "signal send failed with status {}: {detail}",
        status.as_u16()
    ))
}
