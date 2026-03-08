use std::collections::BTreeSet;

use aes::Aes256;
use base64::Engine;
use cbc::cipher::block_padding::Pkcs7;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::*;

#[test]
fn feishu_url_verification_payload_parses() {
    let payload = json!({
        "type": "url_verification",
        "token": "token-123",
        "challenge": "abc"
    });
    let action =
        parse_feishu_webhook_payload(&payload, Some("token-123"), None, &BTreeSet::new(), true)
            .expect("parse feishu url verification");

    match action {
        FeishuWebhookAction::UrlVerification { challenge } => assert_eq!(challenge, "abc"),
        _ => panic!("unexpected action"),
    }
}

#[test]
fn feishu_message_event_parses_text_payload() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {
                "sender_type": "user"
            },
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_123",
                "message_type": "text",
                "content": "{\"text\":\"hello loongclaw\"}"
            }
        }
    });

    let action =
        parse_feishu_webhook_payload(&payload, Some("token-123"), None, &BTreeSet::new(), true)
            .expect("parse feishu event");

    match action {
        FeishuWebhookAction::Inbound(event) => {
            assert_eq!(event.event_id, "evt_1");
            assert_eq!(event.session_id, "feishu:oc_123");
            assert_eq!(event.message_id, "om_123");
            assert_eq!(event.text, "hello loongclaw");
        }
        _ => panic!("unexpected action"),
    }
}

#[test]
fn feishu_send_payload_serializes_content() {
    let payload = build_feishu_send_payload("oc_1", "text", json!({"text": "hi"}))
        .expect("build feishu send payload");
    assert_eq!(payload["receive_id"], "oc_1");
    assert_eq!(payload["msg_type"], "text");
    assert_eq!(payload["content"], "{\"text\":\"hi\"}");
}

#[test]
fn feishu_token_mismatch_is_rejected() {
    let payload = json!({
        "type": "url_verification",
        "token": "token-x",
        "challenge": "abc"
    });
    let error =
        parse_feishu_webhook_payload(&payload, Some("token-y"), None, &BTreeSet::new(), true)
            .expect_err("token mismatch should fail");
    assert!(error.contains("unauthorized"));
}

#[test]
fn feishu_non_text_message_is_ignored() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_123",
                "message_type": "image",
                "content": "{}"
            }
        }
    });
    let action =
        parse_feishu_webhook_payload(&payload, Some("token-123"), None, &BTreeSet::new(), true)
            .expect("non-text payload should parse");

    assert!(matches!(action, FeishuWebhookAction::Ignore));
}

fn encrypt_event_payload_for_test(plain_payload: &str, encrypt_key: &str) -> String {
    use cbc::cipher::{BlockEncryptMut, KeyIvInit};

    let key = Sha256::digest(encrypt_key.as_bytes());
    let iv = [7_u8; 16];

    let mut buffer = plain_payload.as_bytes().to_vec();
    let message_len = buffer.len();
    let pad_len = 16 - (message_len % 16);
    buffer.resize(message_len + pad_len, 0);

    let encrypted = cbc::Encryptor::<Aes256>::new_from_slices(&key, &iv)
        .expect("create encryptor")
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, message_len)
        .expect("encrypt payload");

    let mut merged = iv.to_vec();
    merged.extend_from_slice(encrypted);
    base64::engine::general_purpose::STANDARD.encode(merged)
}

#[test]
fn feishu_encrypted_payload_parses_with_encrypt_key() {
    let plain_payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_encrypted_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_encrypt",
                "message_id": "om_encrypt",
                "message_type": "text",
                "content": "{\"text\":\"encrypted hello\"}"
            }
        }
    });
    let plain_payload_str = serde_json::to_string(&plain_payload).expect("encode plain payload");
    let encrypted = encrypt_event_payload_for_test(&plain_payload_str, "encrypt-key");
    let wrapper = json!({ "encrypt": encrypted });

    let parsed = parse_feishu_webhook_payload(
        &wrapper,
        Some("token-123"),
        Some("encrypt-key"),
        &BTreeSet::new(),
        true,
    )
    .expect("parse encrypted payload");

    match parsed {
        FeishuWebhookAction::Inbound(event) => {
            assert_eq!(event.event_id, "evt_encrypted_1");
            assert_eq!(event.session_id, "feishu:oc_encrypt");
            assert_eq!(event.message_id, "om_encrypt");
            assert_eq!(event.text, "encrypted hello");
        }
        _ => panic!("unexpected action"),
    }
}

#[test]
fn feishu_encrypted_payload_requires_encrypt_key() {
    let wrapper = json!({ "encrypt": "opaque" });
    let error =
        parse_feishu_webhook_payload(&wrapper, Some("token-123"), None, &BTreeSet::new(), true)
            .expect_err("encrypted payload without key should fail");

    assert!(error.contains("encrypt key is not configured"));
}
