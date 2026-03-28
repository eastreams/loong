use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use secp256k1::{Keypair, Secp256k1, SecretKey, XOnlyPublicKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WebSocketMessage;

use crate::{
    CliResult,
    config::{ResolvedNostrChannelConfig, parse_nostr_public_key_hex},
};

use super::ChannelOutboundTargetKind;

const NOSTR_TEXT_NOTE_KIND: u64 = 1;
const NOSTR_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const NOSTR_ACK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
struct NostrUnsignedEvent {
    pubkey: String,
    created_at: u64,
    kind: u64,
    tags: Vec<Vec<String>>,
    content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct NostrEvent {
    id: String,
    pubkey: String,
    created_at: u64,
    kind: u64,
    tags: Vec<Vec<String>>,
    content: String,
    sig: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NostrRelayAck {
    relay_url: String,
    accepted: bool,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NostrRelayServerFrame {
    Ok(NostrRelayAck),
    Notice(String),
    Other,
}

pub(super) async fn run_nostr_send(
    resolved: &ResolvedNostrChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: Option<&str>,
    text: &str,
) -> CliResult<()> {
    if target_kind != ChannelOutboundTargetKind::Address {
        return Err(format!(
            "nostr send requires address target kind, got {}",
            target_kind.as_str()
        ));
    }

    let relay_urls = resolved.relay_urls();
    if relay_urls.is_empty() {
        return Err("nostr relay_urls missing (set nostr.relay_urls or env)".to_owned());
    }

    let private_key_hex = resolved.normalized_private_key_hex()?;
    let Some(private_key_hex) = private_key_hex else {
        return Err("nostr private_key missing (set nostr.private_key or env)".to_owned());
    };

    let target_pubkey_hex = parse_optional_target_pubkey_hex(target_id)?;
    let event =
        build_text_note_event(text, private_key_hex.as_str(), target_pubkey_hex.as_deref())?;

    publish_event_to_relays(relay_urls.as_slice(), &event).await
}

fn parse_optional_target_pubkey_hex(target_id: Option<&str>) -> CliResult<Option<String>> {
    let target_id = target_id.map(str::trim).filter(|value| !value.is_empty());
    let Some(target_id) = target_id else {
        return Ok(None);
    };

    let target_pubkey_hex = parse_nostr_public_key_hex(target_id)?;
    Ok(Some(target_pubkey_hex))
}

fn build_text_note_event(
    text: &str,
    private_key_hex: &str,
    target_pubkey_hex: Option<&str>,
) -> CliResult<NostrEvent> {
    let created_at = current_unix_timestamp_seconds()?;
    build_text_note_event_with_created_at(text, private_key_hex, target_pubkey_hex, created_at)
}

fn build_text_note_event_with_created_at(
    text: &str,
    private_key_hex: &str,
    target_pubkey_hex: Option<&str>,
    created_at: u64,
) -> CliResult<NostrEvent> {
    let secret_key = parse_secret_key(private_key_hex)?;
    let secp = Secp256k1::new();
    let keypair = Keypair::from_secret_key(&secp, &secret_key);
    let public_key = XOnlyPublicKey::from_keypair(&keypair).0;
    let public_key_hex = public_key.to_string();
    let tags = build_text_note_tags(target_pubkey_hex);
    let unsigned_event = NostrUnsignedEvent {
        pubkey: public_key_hex,
        created_at,
        kind: NOSTR_TEXT_NOTE_KIND,
        tags,
        content: text.to_owned(),
    };
    let serialized = serialize_unsigned_event(&unsigned_event)?;
    let id = compute_event_id_hex(serialized.as_str());
    let signature = sign_event_id(&keypair, id.as_str())?;

    Ok(NostrEvent {
        id,
        pubkey: unsigned_event.pubkey,
        created_at: unsigned_event.created_at,
        kind: unsigned_event.kind,
        tags: unsigned_event.tags,
        content: unsigned_event.content,
        sig: signature,
    })
}

fn build_text_note_tags(target_pubkey_hex: Option<&str>) -> Vec<Vec<String>> {
    let target_pubkey_hex = target_pubkey_hex
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(target_pubkey_hex) = target_pubkey_hex else {
        return Vec::new();
    };

    let tag_name = "p".to_owned();
    let target_pubkey = target_pubkey_hex.to_owned();
    vec![vec![tag_name, target_pubkey]]
}

fn current_unix_timestamp_seconds() -> CliResult<u64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before unix epoch: {error}"))?;
    Ok(duration.as_secs())
}

fn parse_secret_key(private_key_hex: &str) -> CliResult<SecretKey> {
    let trimmed_private_key = private_key_hex.trim();
    let decoded_bytes = hex::decode(trimmed_private_key)
        .map_err(|error| format!("invalid nostr private key hex: {error}"))?;
    if decoded_bytes.len() != 32 {
        return Err(format!(
            "invalid nostr private key length {}; expected 32 bytes",
            decoded_bytes.len()
        ));
    }

    let byte_array = <[u8; 32]>::try_from(decoded_bytes.as_slice())
        .map_err(|_conversion_error| "invalid nostr private key length".to_owned())?;
    let secret_key = SecretKey::from_byte_array(byte_array)
        .map_err(|error| format!("invalid nostr private key: {error}"))?;
    Ok(secret_key)
}

fn serialize_unsigned_event(event: &NostrUnsignedEvent) -> CliResult<String> {
    let canonical_payload = serde_json::json!([
        0,
        event.pubkey,
        event.created_at,
        event.kind,
        event.tags,
        event.content
    ]);
    let serialized = serde_json::to_string(&canonical_payload)
        .map_err(|error| format!("serialize nostr event failed: {error}"))?;
    Ok(serialized)
}

fn compute_event_id_hex(serialized: &str) -> String {
    let digest = Sha256::digest(serialized.as_bytes());
    hex::encode(digest)
}

fn sign_event_id(keypair: &Keypair, event_id_hex: &str) -> CliResult<String> {
    let event_id_bytes = hex::decode(event_id_hex)
        .map_err(|error| format!("invalid nostr event id hex: {error}"))?;
    let event_id_array = <[u8; 32]>::try_from(event_id_bytes.as_slice())
        .map_err(|_conversion_error| "invalid nostr event id length".to_owned())?;
    let secp = Secp256k1::new();
    let signature = secp.sign_schnorr_no_aux_rand(event_id_array.as_slice(), keypair);
    Ok(signature.to_string())
}

async fn publish_event_to_relays(relay_urls: &[String], event: &NostrEvent) -> CliResult<()> {
    // this send surface fails closed: every configured relay must accept the
    // publish so operators do not get a false-positive success.
    let mut relay_errors = Vec::new();

    for relay_url in relay_urls {
        let publish_result = publish_event_to_relay(relay_url.as_str(), event).await;
        if let Err(error) = publish_result {
            let relay_error = format!("{relay_url}: {error}");
            relay_errors.push(relay_error);
        }
    }

    if relay_errors.is_empty() {
        return Ok(());
    }

    let joined_errors = relay_errors.join("; ");
    Err(format!(
        "nostr publish failed for {} relay(s): {joined_errors}",
        relay_errors.len()
    ))
}

async fn publish_event_to_relay(relay_url: &str, event: &NostrEvent) -> CliResult<()> {
    let parsed_relay_url = reqwest::Url::parse(relay_url)
        .map_err(|error| format!("invalid relay url `{relay_url}`: {error}"))?;
    let relay_scheme = parsed_relay_url.scheme();
    let is_websocket_scheme = relay_scheme == "ws" || relay_scheme == "wss";
    if !is_websocket_scheme {
        return Err(format!(
            "relay url `{relay_url}` must use ws:// or wss://, got {relay_scheme}://"
        ));
    }

    let connect_future = connect_async(parsed_relay_url.as_str());
    let connect_result = timeout(NOSTR_CONNECT_TIMEOUT, connect_future)
        .await
        .map_err(|_timeout_error| format!("relay connect timed out for `{relay_url}`"))?;
    let (mut stream, _response) = connect_result
        .map_err(|error| format!("relay connect failed for `{relay_url}`: {error}"))?;
    let event_frame = serde_json::json!(["EVENT", event]);
    let event_frame_text = serde_json::to_string(&event_frame)
        .map_err(|error| format!("serialize relay event frame failed: {error}"))?;

    stream
        .send(WebSocketMessage::Text(event_frame_text.into()))
        .await
        .map_err(|error| format!("relay send failed for `{relay_url}`: {error}"))?;

    let ack = wait_for_relay_ok_ack(&mut stream, relay_url, event.id.as_str()).await?;
    if !ack.accepted {
        return Err(format!(
            "relay rejected event `{}`: {}",
            event.id, ack.message
        ));
    }

    let close_result = stream.close(None).await;
    let _ = close_result;
    Ok(())
}

async fn wait_for_relay_ok_ack<S>(
    stream: &mut tokio_tungstenite::WebSocketStream<S>,
    relay_url: &str,
    expected_event_id: &str,
) -> CliResult<NostrRelayAck>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let read_ack_future = async {
        let mut last_notice = None;

        loop {
            let next_frame = stream.next().await;
            let Some(next_frame) = next_frame else {
                let relay_closed_error = match last_notice {
                    Some(notice) => {
                        format!("relay `{relay_url}` closed before OK ack: {notice}")
                    }
                    None => format!("relay `{relay_url}` closed before OK ack"),
                };
                return Err(relay_closed_error);
            };

            let frame =
                next_frame.map_err(|error| format!("relay `{relay_url}` read failed: {error}"))?;
            match frame {
                WebSocketMessage::Text(text_frame) => {
                    let server_frame = parse_relay_server_frame(
                        text_frame.as_ref(),
                        relay_url,
                        expected_event_id,
                    )?;
                    match server_frame {
                        NostrRelayServerFrame::Ok(ack) => return Ok(ack),
                        NostrRelayServerFrame::Notice(notice) => last_notice = Some(notice),
                        NostrRelayServerFrame::Other => {}
                    }
                }
                WebSocketMessage::Close(close_frame) => {
                    let close_message = close_frame
                        .map(|frame| frame.reason.to_string())
                        .filter(|value| !value.trim().is_empty());
                    let error = close_message
                        .map(|value| format!("relay `{relay_url}` closed before OK ack: {value}"))
                        .unwrap_or_else(|| format!("relay `{relay_url}` closed before OK ack"));
                    return Err(error);
                }
                WebSocketMessage::Binary(_)
                | WebSocketMessage::Ping(_)
                | WebSocketMessage::Pong(_)
                | WebSocketMessage::Frame(_) => {}
            }
        }
    };

    let ack_result = timeout(NOSTR_ACK_TIMEOUT, read_ack_future)
        .await
        .map_err(|_timeout_error| format!("relay `{relay_url}` did not return OK ack in time"))?;
    let ack = ack_result?;

    Ok(ack)
}

fn parse_relay_server_frame(
    raw_text: &str,
    relay_url: &str,
    expected_event_id: &str,
) -> CliResult<NostrRelayServerFrame> {
    let payload: Value = serde_json::from_str(raw_text)
        .map_err(|error| format!("relay `{relay_url}` returned invalid json: {error}"))?;
    let payload_array = payload
        .as_array()
        .ok_or_else(|| format!("relay `{relay_url}` returned non-array frame"))?;
    let frame_name = payload_array.first().and_then(Value::as_str).unwrap_or("");

    if frame_name == "NOTICE" {
        let notice = payload_array
            .get(1)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| "relay notice without message".to_owned());
        return Ok(NostrRelayServerFrame::Notice(notice));
    }

    if frame_name != "OK" {
        return Ok(NostrRelayServerFrame::Other);
    }

    let event_id = payload_array
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("relay `{relay_url}` returned malformed OK event id"))?;
    if event_id != expected_event_id {
        return Ok(NostrRelayServerFrame::Other);
    }

    let accepted = payload_array
        .get(2)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("relay `{relay_url}` returned malformed OK acceptance flag"))?;
    let message = payload_array
        .get(3)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_default();

    Ok(NostrRelayServerFrame::Ok(NostrRelayAck {
        relay_url: relay_url.to_owned(),
        accepted,
        message,
    }))
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

    use super::*;

    const TEST_PRIVATE_KEY_HEX: &str =
        "67dea2ed01af4efe6b84652f82d193946d9d6d74a8d8ddf1ee8f5a67f9f4b1f0";

    fn test_target_pubkey_hex() -> String {
        let secret_key = parse_secret_key(TEST_PRIVATE_KEY_HEX).expect("parse test private key");
        let secp = Secp256k1::new();
        let keypair = Keypair::from_secret_key(&secp, &secret_key);
        let public_key = XOnlyPublicKey::from_keypair(&keypair).0;
        public_key.to_string()
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum MockRelayResponse {
        Accept,
        Reject,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ObservedRelayEvent {
        event: NostrEvent,
    }

    #[test]
    fn build_text_note_event_adds_optional_p_tag() {
        let target_pubkey_hex = test_target_pubkey_hex();
        let event = build_text_note_event_with_created_at(
            "hello nostr",
            TEST_PRIVATE_KEY_HEX,
            Some(target_pubkey_hex.as_str()),
            1_700_000_000,
        )
        .expect("build event");

        assert_eq!(event.kind, NOSTR_TEXT_NOTE_KIND);
        assert_eq!(event.content, "hello nostr");
        assert_eq!(event.tags, vec![vec!["p".to_owned(), target_pubkey_hex]]);
        assert_eq!(event.id.len(), 64);
        assert_eq!(event.pubkey.len(), 64);
        assert_eq!(event.sig.len(), 128);
    }

    #[tokio::test]
    async fn run_nostr_send_publishes_event_and_waits_for_ok() {
        let relay = spawn_mock_nostr_relay(MockRelayResponse::Accept)
            .await
            .expect("spawn relay");
        let relay_url = relay.0;
        let relay_task = relay.1;
        let resolved = resolved_nostr_config(vec![relay_url.clone()]);
        let target_pubkey_hex = test_target_pubkey_hex();

        run_nostr_send(
            &resolved,
            ChannelOutboundTargetKind::Address,
            Some(target_pubkey_hex.as_str()),
            "relay publish",
        )
        .await
        .expect("nostr publish should succeed");

        let observed = relay_task
            .await
            .expect("relay task join")
            .expect("relay event");
        assert_eq!(observed.event.kind, NOSTR_TEXT_NOTE_KIND);
        assert_eq!(observed.event.content, "relay publish");
        assert_eq!(
            observed.event.tags,
            vec![vec!["p".to_owned(), target_pubkey_hex]]
        );
    }

    #[tokio::test]
    async fn run_nostr_send_fails_when_relay_rejects_event() {
        let relay = spawn_mock_nostr_relay(MockRelayResponse::Reject)
            .await
            .expect("spawn relay");
        let relay_url = relay.0;
        let resolved = resolved_nostr_config(vec![relay_url.clone()]);

        let error = run_nostr_send(
            &resolved,
            ChannelOutboundTargetKind::Address,
            None,
            "reject me",
        )
        .await
        .expect_err("rejected relay should fail");

        assert!(error.contains("nostr publish failed for 1 relay(s):"));
        assert!(error.contains("relay rejected event"));
    }

    #[tokio::test]
    async fn run_nostr_send_rejects_non_address_target_kind() {
        let resolved = resolved_nostr_config(Vec::new());

        let error = run_nostr_send(
            &resolved,
            ChannelOutboundTargetKind::Conversation,
            None,
            "hello",
        )
        .await
        .expect_err("wrong target kind should fail");

        assert_eq!(
            error,
            "nostr send requires address target kind, got conversation"
        );
    }

    fn resolved_nostr_config(relay_urls: Vec<String>) -> ResolvedNostrChannelConfig {
        let config = crate::config::NostrChannelConfig {
            enabled: true,
            relay_urls,
            private_key: Some(loongclaw_contracts::SecretRef::Inline(
                TEST_PRIVATE_KEY_HEX.to_owned(),
            )),
            ..crate::config::NostrChannelConfig::default()
        };

        config.resolve_account(None).expect("resolve nostr config")
    }

    async fn spawn_mock_nostr_relay(
        response: MockRelayResponse,
    ) -> CliResult<(
        String,
        tokio::task::JoinHandle<CliResult<ObservedRelayEvent>>,
    )> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| format!("bind relay listener failed: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("read relay listener address failed: {error}"))?;
        let relay_url = format!("ws://{address}");
        let relay_task = tokio::spawn(async move {
            let accepted = listener.accept().await;
            let (stream, _) = accepted.map_err(|error| format!("accept relay failed: {error}"))?;
            let mut websocket = accept_async(stream)
                .await
                .map_err(|error| format!("upgrade relay websocket failed: {error}"))?;
            let inbound = websocket
                .next()
                .await
                .ok_or_else(|| "relay did not receive event frame".to_owned())?
                .map_err(|error| format!("relay read event frame failed: {error}"))?;
            let text_frame = match inbound {
                WebSocketMessage::Text(text_frame) => text_frame,
                other @ WebSocketMessage::Binary(_)
                | other @ WebSocketMessage::Ping(_)
                | other @ WebSocketMessage::Pong(_)
                | other @ WebSocketMessage::Close(_)
                | other @ WebSocketMessage::Frame(_) => {
                    return Err(format!("unexpected relay inbound frame: {other:?}"));
                }
            };
            let payload: Value = serde_json::from_str(text_frame.as_ref())
                .map_err(|error| format!("parse relay inbound json failed: {error}"))?;
            let payload_array = payload
                .as_array()
                .ok_or_else(|| "relay inbound frame was not an array".to_owned())?;
            let frame_name = payload_array.first().and_then(Value::as_str).unwrap_or("");
            if frame_name != "EVENT" {
                return Err(format!("unexpected relay frame `{frame_name}`"));
            }
            let event_value = payload_array
                .get(1)
                .cloned()
                .ok_or_else(|| "relay EVENT frame missing payload".to_owned())?;
            let event: NostrEvent = serde_json::from_value(event_value)
                .map_err(|error| format!("parse relay event failed: {error}"))?;
            let accepted = response == MockRelayResponse::Accept;
            let message = if accepted { "accepted" } else { "blocked" };
            let ack_frame = serde_json::json!(["OK", event.id, accepted, message]);
            let ack_text = serde_json::to_string(&ack_frame)
                .map_err(|error| format!("serialize relay ack failed: {error}"))?;

            websocket
                .send(WebSocketMessage::Text(ack_text.into()))
                .await
                .map_err(|error| format!("send relay ack failed: {error}"))?;

            Ok(ObservedRelayEvent { event })
        });

        Ok((relay_url, relay_task))
    }
}
