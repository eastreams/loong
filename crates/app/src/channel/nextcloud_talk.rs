use hmac::{KeyInit, Mac};
use serde::Serialize;

use crate::{CliResult, config::ResolvedNextcloudTalkChannelConfig};

use super::{
    ChannelOutboundTargetKind,
    http::{ChannelOutboundHttpPolicy, build_outbound_http_client, validate_outbound_http_target},
};

type NextcloudTalkHmacSha256 = hmac::Hmac<sha2::Sha256>;

const NEXTCLOUD_TALK_BOT_RANDOM_HEADER: &str = "X-Nextcloud-Talk-Bot-Random";
const NEXTCLOUD_TALK_BOT_SIGNATURE_HEADER: &str = "X-Nextcloud-Talk-Bot-Signature";
const NEXTCLOUD_TALK_OCS_API_REQUEST_HEADER: &str = "OCS-APIRequest";

#[derive(Debug, Serialize)]
struct NextcloudTalkSendRequestBody {
    message: String,
    #[serde(rename = "referenceId")]
    reference_id: String,
}

pub(super) async fn run_nextcloud_talk_send(
    resolved: &ResolvedNextcloudTalkChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<()> {
    if target_kind != ChannelOutboundTargetKind::Conversation {
        return Err(format!(
            "nextcloud talk send requires conversation target kind, got {}",
            target_kind.as_str()
        ));
    }

    let server_url = resolved.server_url().ok_or_else(|| {
        "nextcloud_talk server_url missing (set nextcloud_talk.server_url or env)".to_owned()
    })?;
    let shared_secret = resolved.shared_secret().ok_or_else(|| {
        "nextcloud_talk shared_secret missing (set nextcloud_talk.shared_secret or env)".to_owned()
    })?;
    let conversation_token = target_id.trim();
    if conversation_token.is_empty() {
        return Err("nextcloud talk outbound target id is empty".to_owned());
    }

    let random_header = build_random_reference_id();
    let request_body = NextcloudTalkSendRequestBody {
        message: text.to_owned(),
        reference_id: random_header.clone(),
    };
    let request_body_json = serde_json::to_string(&request_body)
        .map_err(|error| format!("serialize nextcloud talk request failed: {error}"))?;
    let request_signature = build_nextcloud_talk_signature(
        shared_secret.as_str(),
        random_header.as_str(),
        request_body_json.as_str(),
    )?;
    let request_url =
        build_nextcloud_talk_request_url(server_url.as_str(), conversation_token, policy)?;

    let client = build_outbound_http_client("nextcloud talk send", policy)?;
    let request = client
        .post(request_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(NEXTCLOUD_TALK_OCS_API_REQUEST_HEADER, "true")
        .header(NEXTCLOUD_TALK_BOT_RANDOM_HEADER, random_header.as_str())
        .header(
            NEXTCLOUD_TALK_BOT_SIGNATURE_HEADER,
            request_signature.as_str(),
        )
        .body(request_body_json);
    let response = request
        .send()
        .await
        .map_err(|error| format!("nextcloud talk send failed: {error}"))?;

    ensure_nextcloud_talk_success(response).await
}

fn build_random_reference_id() -> String {
    let random_bytes = rand::random::<[u8; 32]>();
    hex::encode(random_bytes)
}

fn build_nextcloud_talk_request_url(
    server_url: &str,
    conversation_token: &str,
    policy: ChannelOutboundHttpPolicy,
) -> CliResult<String> {
    let mut url = validate_outbound_http_target("nextcloud talk server_url", server_url, policy)?;
    let mut path_segments = url.path_segments_mut().map_err(|_path_error| {
        "nextcloud talk server_url cannot be used as a hierarchical base url".to_owned()
    })?;
    path_segments.pop_if_empty();
    path_segments.push("ocs");
    path_segments.push("v2.php");
    path_segments.push("apps");
    path_segments.push("spreed");
    path_segments.push("api");
    path_segments.push("v1");
    path_segments.push("bot");
    path_segments.push(conversation_token);
    path_segments.push("message");
    drop(path_segments);
    Ok(url.to_string())
}

fn build_nextcloud_talk_signature(
    shared_secret: &str,
    random_header: &str,
    request_body_json: &str,
) -> CliResult<String> {
    let mut mac = NextcloudTalkHmacSha256::new_from_slice(shared_secret.as_bytes())
        .map_err(|error| format!("build nextcloud talk signature failed: {error}"))?;
    mac.update(random_header.as_bytes());
    mac.update(request_body_json.as_bytes());
    let signature = mac.finalize().into_bytes();
    Ok(hex::encode(signature))
}

async fn ensure_nextcloud_talk_success(response: reqwest::Response) -> CliResult<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let body = response
        .text()
        .await
        .map_err(|error| format!("read nextcloud talk error response failed: {error}"))?;
    let trimmed_body = body.trim();
    let detail = if trimmed_body.is_empty() {
        "empty response body".to_owned()
    } else {
        trimmed_body.to_owned()
    };

    Err(format!(
        "nextcloud talk send failed with status {}: {detail}",
        status.as_u16()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_nextcloud_talk_request_url_preserves_base_path() {
        let policy = ChannelOutboundHttpPolicy {
            allow_private_hosts: false,
        };
        let request_url = build_nextcloud_talk_request_url(
            "https://cloud.example.test/nextcloud",
            "room-token",
            policy,
        )
        .expect("build nextcloud talk request url");

        assert_eq!(
            request_url,
            "https://cloud.example.test/nextcloud/ocs/v2.php/apps/spreed/api/v1/bot/room-token/message"
        );
    }

    #[test]
    fn build_nextcloud_talk_signature_matches_reference_vector() {
        let signature = build_nextcloud_talk_signature(
            "shared-secret",
            "0123456789abcdef",
            "{\"message\":\"hello\",\"referenceId\":\"abc123\"}",
        )
        .expect("build nextcloud talk signature");

        assert_eq!(
            signature,
            "70194893e46ad651aac6e3d23beb9285c984894876e6420fac105c5be9edd0bf"
        );
    }
}
