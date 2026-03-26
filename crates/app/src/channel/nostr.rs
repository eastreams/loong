use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bech32::{FromBase32, Variant};
use futures_util::{SinkExt, StreamExt};
use secp256k1::{Keypair, Secp256k1, SecretKey, XOnlyPublicKey};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::time::timeout;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::{CliResult, config::ResolvedNostrChannelConfig};

use super::ChannelOutboundTargetKind;

const NOSTR_TEXT_NOTE_KIND: u64 = 1;
const NOSTR_PUBLISH_ACK_TIMEOUT: Duration = Duration::from_secs(15);

type NostrWebsocketStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Debug, Clone)]
struct ParsedNostrPrivateKey {
    secret_key: SecretKey,
    public_key_hex: String,
}

#[derive(Debug, Clone, Serialize)]
struct NostrEvent {
    id: String,
    pubkey: String,
    created_at: u64,
    kind: u64,
    tags: Vec<Vec<String>>,
    content: String,
    sig: String,
}

pub(super) fn validate_nostr_private_key_input(private_key: &str) -> CliResult<()> {
    let _ = parse_nostr_private_key(private_key)?;
    Ok(())
}

pub(super) async fn run_nostr_send(
    resolved: &ResolvedNostrChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target: Option<&str>,
    text: &str,
) -> CliResult<()> {
    ensure_nostr_target_kind(target_kind)?;

    let relay_urls = resolve_nostr_publish_relays(resolved, target)?;
    let private_key = resolved
        .private_key()
        .ok_or_else(|| "nostr private_key missing (set nostr.private_key or env)".to_owned())?;
    let parsed_private_key = parse_nostr_private_key(private_key.as_str())?;
    let created_at = current_unix_timestamp_seconds();
    let event = build_nostr_text_note_event(&parsed_private_key, created_at, text)?;

    let mut relay_errors = Vec::new();
    for relay_url in relay_urls {
        let publish_result = publish_nostr_event_to_relay(relay_url.as_str(), &event).await;
        if let Err(error) = publish_result {
            let relay_error = format!("{relay_url}: {error}");
            relay_errors.push(relay_error);
        }
    }

    if relay_errors.is_empty() {
        return Ok(());
    }

    let relay_error_count = relay_errors.len();
    let relay_error_summary = relay_errors.join("; ");
    Err(format!(
        "nostr publish failed for {relay_error_count} relay(s): {relay_error_summary}"
    ))
}

fn ensure_nostr_target_kind(target_kind: ChannelOutboundTargetKind) -> CliResult<()> {
    if target_kind == ChannelOutboundTargetKind::Address {
        return Ok(());
    }

    Err(format!(
        "nostr send requires address target kind, got {}",
        target_kind.as_str()
    ))
}

fn resolve_nostr_publish_relays(
    resolved: &ResolvedNostrChannelConfig,
    target: Option<&str>,
) -> CliResult<Vec<String>> {
    let target = target.map(str::trim).filter(|value| !value.is_empty());
    if let Some(target) = target {
        let relay_url = target.to_owned();
        return Ok(vec![relay_url]);
    }

    let relay_urls = resolved.relay_urls();
    if !relay_urls.is_empty() {
        return Ok(relay_urls);
    }

    Err("nostr relay_urls missing (set nostr.relay_urls or pass --target)".to_owned())
}

fn parse_nostr_private_key(private_key: &str) -> CliResult<ParsedNostrPrivateKey> {
    let trimmed_private_key = private_key.trim();
    if trimmed_private_key.is_empty() {
        return Err("nostr private_key is empty".to_owned());
    }

    let secret_key_bytes = if trimmed_private_key.starts_with("nsec1") {
        decode_nostr_nsec_private_key(trimmed_private_key)?
    } else {
        decode_nostr_hex_private_key(trimmed_private_key)?
    };
    let secret_key = SecretKey::from_byte_array(secret_key_bytes)
        .map_err(|error| format!("nostr private_key is invalid: {error}"))?;
    let secp = Secp256k1::new();
    let keypair = Keypair::from_secret_key(&secp, &secret_key);
    let (public_key, _) = XOnlyPublicKey::from_keypair(&keypair);
    let public_key_hex = public_key.to_string();

    Ok(ParsedNostrPrivateKey {
        secret_key,
        public_key_hex,
    })
}

fn decode_nostr_hex_private_key(private_key: &str) -> CliResult<[u8; 32]> {
    let decoded_bytes = hex::decode(private_key)
        .map_err(|error| format!("nostr hex private_key decode failed: {error}"))?;
    let decoded_length = decoded_bytes.len();
    if decoded_length != 32 {
        return Err(format!(
            "nostr hex private_key must decode to 32 bytes, got {decoded_length}"
        ));
    }

    let mut secret_key_bytes = [0_u8; 32];
    secret_key_bytes.copy_from_slice(decoded_bytes.as_slice());
    Ok(secret_key_bytes)
}

fn decode_nostr_nsec_private_key(private_key: &str) -> CliResult<[u8; 32]> {
    let decoded = bech32::decode(private_key)
        .map_err(|error| format!("nostr nsec private_key decode failed: {error}"))?;
    let hrp = decoded.0;
    let data = decoded.1;
    let variant = decoded.2;

    if hrp != "nsec" {
        return Err(format!(
            "nostr private_key must use the nsec hrp, got `{hrp}`"
        ));
    }
    if variant != Variant::Bech32 {
        return Err("nostr nsec private_key must use bech32 encoding".to_owned());
    }

    let decoded_bytes = Vec::<u8>::from_base32(&data)
        .map_err(|error| format!("nostr nsec private_key base32 decode failed: {error}"))?;
    let decoded_length = decoded_bytes.len();
    if decoded_length != 32 {
        return Err(format!(
            "nostr nsec private_key must decode to 32 bytes, got {decoded_length}"
        ));
    }

    let mut secret_key_bytes = [0_u8; 32];
    secret_key_bytes.copy_from_slice(decoded_bytes.as_slice());
    Ok(secret_key_bytes)
}

fn build_nostr_text_note_event(
    private_key: &ParsedNostrPrivateKey,
    created_at: u64,
    content: &str,
) -> CliResult<NostrEvent> {
    let preimage =
        serialize_nostr_event_preimage(private_key.public_key_hex.as_str(), created_at, content)?;
    let event_id_bytes = Sha256::digest(preimage);
    let event_id_bytes: [u8; 32] = event_id_bytes.into();
    let event_id = hex::encode(event_id_bytes);
    let signature = sign_nostr_event_id(event_id_bytes, &private_key.secret_key)?;

    Ok(NostrEvent {
        id: event_id,
        pubkey: private_key.public_key_hex.clone(),
        created_at,
        kind: NOSTR_TEXT_NOTE_KIND,
        tags: Vec::new(),
        content: content.to_owned(),
        sig: signature,
    })
}

fn serialize_nostr_event_preimage(
    public_key_hex: &str,
    created_at: u64,
    content: &str,
) -> CliResult<Vec<u8>> {
    let kind = NOSTR_TEXT_NOTE_KIND;
    let tags = Vec::<Vec<String>>::new();
    let preimage = serde_json::json!([0, public_key_hex, created_at, kind, tags, content]);
    let serialized = serde_json::to_vec(&preimage)
        .map_err(|error| format!("serialize nostr event preimage failed: {error}"))?;
    Ok(serialized)
}

fn sign_nostr_event_id(event_id_bytes: [u8; 32], secret_key: &SecretKey) -> CliResult<String> {
    let secp = Secp256k1::new();
    let keypair = Keypair::from_secret_key(&secp, secret_key);
    let signature = secp.sign_schnorr_no_aux_rand(&event_id_bytes, &keypair);
    let signature_hex = signature.to_string();
    Ok(signature_hex)
}

fn parse_nostr_relay_url(relay_url: &str) -> CliResult<reqwest::Url> {
    let trimmed_relay_url = relay_url.trim();
    if trimmed_relay_url.is_empty() {
        return Err("nostr relay_url is empty".to_owned());
    }

    let parsed_relay_url = reqwest::Url::parse(trimmed_relay_url)
        .map_err(|error| format!("nostr relay_url is invalid: {error}"))?;
    let scheme = parsed_relay_url.scheme();
    if scheme != "ws" && scheme != "wss" {
        return Err(format!(
            "nostr relay_url must use ws:// or wss://, got {scheme}://"
        ));
    }

    let host = parsed_relay_url.host_str();
    if host.is_none() {
        return Err("nostr relay_url is missing a host".to_owned());
    }

    Ok(parsed_relay_url)
}

async fn publish_nostr_event_to_relay(relay_url: &str, event: &NostrEvent) -> CliResult<()> {
    let parsed_relay_url = parse_nostr_relay_url(relay_url)?;
    let websocket_url = parsed_relay_url.to_string();

    ensure_nostr_websocket_rustls_provider();
    let connect_result = connect_async(websocket_url.as_str()).await;
    let (mut websocket, _) =
        connect_result.map_err(|error| format!("connect nostr relay failed: {error}"))?;

    let event_envelope = serde_json::json!(["EVENT", event]);
    let payload = serde_json::to_string(&event_envelope)
        .map_err(|error| format!("serialize nostr publish payload failed: {error}"))?;
    let message = Message::Text(payload.into());
    websocket
        .send(message)
        .await
        .map_err(|error| format!("send nostr publish payload failed: {error}"))?;

    wait_for_nostr_publish_ack(&mut websocket, relay_url, event.id.as_str()).await
}

async fn wait_for_nostr_publish_ack(
    websocket: &mut NostrWebsocketStream,
    relay_url: &str,
    expected_event_id: &str,
) -> CliResult<()> {
    let mut last_notice = None;
    let wait_result = timeout(NOSTR_PUBLISH_ACK_TIMEOUT, async {
        loop {
            let frame = websocket.next().await;
            let frame = match frame {
                Some(Ok(frame)) => frame,
                Some(Err(error)) => {
                    return Err(format!("read nostr relay frame failed: {error}"));
                }
                None => {
                    return Err("nostr relay closed before publish acknowledgement".to_owned());
                }
            };

            match frame {
                Message::Text(text) => {
                    let payload = parse_nostr_relay_json_frame(text.as_ref())?;
                    let publish_status = classify_nostr_publish_frame(&payload, expected_event_id);
                    match publish_status {
                        NostrPublishFrame::Accepted => return Ok(()),
                        NostrPublishFrame::Rejected(detail) => return Err(detail),
                        NostrPublishFrame::Notice(detail) => {
                            last_notice = Some(detail);
                        }
                        NostrPublishFrame::Continue => {}
                    }
                }
                Message::Binary(bytes) => {
                    let text = std::str::from_utf8(bytes.as_ref()).map_err(|error| {
                        format!("decode nostr relay binary frame as utf8 failed: {error}")
                    })?;
                    let payload = parse_nostr_relay_json_frame(text)?;
                    let publish_status = classify_nostr_publish_frame(&payload, expected_event_id);
                    match publish_status {
                        NostrPublishFrame::Accepted => return Ok(()),
                        NostrPublishFrame::Rejected(detail) => return Err(detail),
                        NostrPublishFrame::Notice(detail) => {
                            last_notice = Some(detail);
                        }
                        NostrPublishFrame::Continue => {}
                    }
                }
                Message::Ping(payload) => {
                    let pong = Message::Pong(payload);
                    websocket
                        .send(pong)
                        .await
                        .map_err(|error| format!("send nostr relay pong failed: {error}"))?;
                }
                Message::Pong(_) => {}
                Message::Frame(_) => {}
                Message::Close(frame) => {
                    let reason = frame
                        .as_ref()
                        .map(|frame| frame.reason.to_string())
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_else(|| "remote peer closed the socket".to_owned());
                    let detail =
                        format!("nostr relay closed before publish acknowledgement: {reason}");
                    return Err(detail);
                }
            }
        }
    })
    .await;

    match wait_result {
        Ok(result) => result,
        Err(_) => {
            let detail = last_notice.unwrap_or_else(|| "no relay notice received".to_owned());
            Err(format!(
                "nostr relay `{relay_url}` timed out waiting for publish acknowledgement for event `{expected_event_id}` ({detail})"
            ))
        }
    }
}

fn parse_nostr_relay_json_frame(payload: &str) -> CliResult<Value> {
    let value = serde_json::from_str::<Value>(payload)
        .map_err(|error| format!("decode nostr relay json failed: {error}"))?;
    Ok(value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NostrPublishFrame {
    Accepted,
    Rejected(String),
    Notice(String),
    Continue,
}

fn classify_nostr_publish_frame(payload: &Value, expected_event_id: &str) -> NostrPublishFrame {
    let items = payload.as_array();
    let Some(items) = items else {
        return NostrPublishFrame::Continue;
    };
    let command = items.first().and_then(Value::as_str);
    let Some(command) = command else {
        return NostrPublishFrame::Continue;
    };

    if command == "NOTICE" {
        let notice = items
            .get(1)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| "relay returned an empty notice".to_owned());
        return NostrPublishFrame::Notice(notice);
    }

    if command != "OK" {
        return NostrPublishFrame::Continue;
    }

    let response_event_id = items.get(1).and_then(Value::as_str);
    if response_event_id != Some(expected_event_id) {
        return NostrPublishFrame::Continue;
    }

    let accepted = items.get(2).and_then(Value::as_bool).unwrap_or(false);
    let message = items
        .get(3)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| "relay did not return an acknowledgement message".to_owned());
    if accepted {
        return NostrPublishFrame::Accepted;
    }

    let detail = format!("relay rejected event `{expected_event_id}`: {message}");
    NostrPublishFrame::Rejected(detail)
}

fn ensure_nostr_websocket_rustls_provider() {
    static RUSTLS_PROVIDER_INIT: OnceLock<()> = OnceLock::new();

    RUSTLS_PROVIDER_INIT.get_or_init(|| {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let provider = rustls::crypto::ring::default_provider();
            let _ = provider.install_default();
        }
    });
}

fn current_unix_timestamp_seconds() -> u64 {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH);
    let Ok(duration) = duration else {
        return 0;
    };

    duration.as_secs()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bech32::ToBase32;
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;
    use tokio_tungstenite::accept_async;

    use super::*;

    const VALID_HEX_PRIVATE_KEY: &str =
        "1111111111111111111111111111111111111111111111111111111111111111";

    #[derive(Debug, Clone)]
    enum MockNostrRelayBehavior {
        Accept,
        Reject(String),
    }

    #[derive(Debug, Clone, Default)]
    struct MockNostrRelayState {
        frames: Arc<Mutex<Vec<Value>>>,
    }

    #[tokio::test]
    async fn run_nostr_send_uses_configured_relays_when_target_is_omitted() {
        let state = MockNostrRelayState::default();
        let (relay_url, server) =
            spawn_mock_nostr_relay(state.clone(), MockNostrRelayBehavior::Accept).await;
        let resolved = build_resolved_nostr_config(vec![relay_url], VALID_HEX_PRIVATE_KEY);

        let send_result = run_nostr_send(
            &resolved,
            ChannelOutboundTargetKind::Address,
            None,
            "hello from loongclaw",
        )
        .await;

        send_result.expect("nostr send should succeed");

        let frames = state.frames.lock().await;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0][0], json!("EVENT"));
        assert_eq!(frames[0][1]["kind"], json!(1));
        assert_eq!(frames[0][1]["content"], json!("hello from loongclaw"));
        assert_eq!(frames[0][1]["tags"], json!([]));

        server.abort();
    }

    #[tokio::test]
    async fn run_nostr_send_allows_target_override_without_configured_relays() {
        let state = MockNostrRelayState::default();
        let (relay_url, server) =
            spawn_mock_nostr_relay(state.clone(), MockNostrRelayBehavior::Accept).await;
        let resolved = build_resolved_nostr_config(Vec::new(), VALID_HEX_PRIVATE_KEY);

        let send_result = run_nostr_send(
            &resolved,
            ChannelOutboundTargetKind::Address,
            Some(relay_url.as_str()),
            "override relay publish",
        )
        .await;

        send_result.expect("nostr send should succeed with relay target override");

        let frames = state.frames.lock().await;
        assert_eq!(frames.len(), 1);

        server.abort();
    }

    #[tokio::test]
    async fn run_nostr_send_requires_address_target_kind() {
        let resolved = build_resolved_nostr_config(Vec::new(), VALID_HEX_PRIVATE_KEY);

        let error = run_nostr_send(
            &resolved,
            ChannelOutboundTargetKind::Conversation,
            Some("wss://relay.example.test"),
            "invalid target kind",
        )
        .await
        .expect_err("conversation target kind should be rejected");

        assert_eq!(
            error,
            "nostr send requires address target kind, got conversation"
        );
    }

    #[tokio::test]
    async fn run_nostr_send_reports_relay_rejections() {
        let state = MockNostrRelayState::default();
        let behavior = MockNostrRelayBehavior::Reject("blocked by relay policy".to_owned());
        let (relay_url, server) = spawn_mock_nostr_relay(state, behavior).await;
        let resolved = build_resolved_nostr_config(vec![relay_url], VALID_HEX_PRIVATE_KEY);

        let error = run_nostr_send(
            &resolved,
            ChannelOutboundTargetKind::Address,
            None,
            "relay rejection",
        )
        .await
        .expect_err("relay rejection should fail");

        assert!(error.contains("blocked by relay policy"));

        server.abort();
    }

    #[test]
    fn parse_nostr_private_key_accepts_hex_and_nsec() {
        let hex_private_key =
            parse_nostr_private_key(VALID_HEX_PRIVATE_KEY).expect("parse hex private key");
        let private_key_bytes = [0x11_u8; 32];
        let nsec_private_key =
            bech32::encode("nsec", private_key_bytes.to_base32(), Variant::Bech32)
                .expect("encode nsec private key");
        let nsec_private_key =
            parse_nostr_private_key(nsec_private_key.as_str()).expect("parse nsec private key");

        assert_eq!(
            hex_private_key.public_key_hex,
            nsec_private_key.public_key_hex
        );
    }

    fn build_resolved_nostr_config(
        relay_urls: Vec<String>,
        private_key: &str,
    ) -> ResolvedNostrChannelConfig {
        ResolvedNostrChannelConfig {
            configured_account_id: "default".to_owned(),
            configured_account_label: "default".to_owned(),
            account: crate::config::ChannelAccountIdentity {
                id: "default".to_owned(),
                label: "default".to_owned(),
                source: crate::config::ChannelAccountIdentitySource::Default,
            },
            enabled: true,
            relay_urls,
            relay_urls_env: None,
            private_key: Some(loongclaw_contracts::SecretRef::Inline(
                private_key.to_owned(),
            )),
            private_key_env: None,
            allowed_pubkeys: Vec::new(),
        }
    }

    async fn spawn_mock_nostr_relay(
        state: MockNostrRelayState,
        behavior: MockNostrRelayBehavior,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock nostr relay");
        let address = listener.local_addr().expect("mock nostr relay addr");
        let handle = tokio::spawn(async move {
            let accept_result = listener.accept().await;
            let (stream, _) = accept_result.expect("accept mock nostr relay connection");
            let websocket = accept_async(stream).await.expect("accept websocket");
            handle_mock_nostr_relay_session(websocket, state, behavior).await;
        });
        let relay_url = format!("ws://{}", address);
        (relay_url, handle)
    }

    async fn handle_mock_nostr_relay_session(
        mut websocket: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        state: MockNostrRelayState,
        behavior: MockNostrRelayBehavior,
    ) {
        let next = websocket.next().await;
        let message = match next {
            Some(Ok(message)) => message,
            _ => return,
        };

        let Message::Text(text) = message else {
            return;
        };
        let payload = serde_json::from_str::<Value>(text.as_ref()).expect("decode relay payload");
        state.frames.lock().await.push(payload.clone());

        let event_id = payload[1]["id"]
            .as_str()
            .expect("event id in publish payload");
        let response = match behavior {
            MockNostrRelayBehavior::Accept => json!(["OK", event_id, true, "accepted"]),
            MockNostrRelayBehavior::Reject(message) => {
                json!(["OK", event_id, false, message])
            }
        };
        let response_text = serde_json::to_string(&response).expect("encode relay response");
        let response_message = Message::Text(response_text.into());
        websocket
            .send(response_message)
            .await
            .expect("send relay response");
    }
}
