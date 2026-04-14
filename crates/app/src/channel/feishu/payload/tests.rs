use std::collections::BTreeSet;

use aes::Aes256;
use base64::Engine;
use cbc::cipher::block_padding::Pkcs7;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::*;
use crate::channel::{
    ChannelOutboundTarget, ChannelOutboundTargetKind, ChannelPlatform,
    access_policy::ChannelInboundAccessPolicy,
};

fn parse_feishu_inbound_summary(text: &str) -> Value {
    let payload = text
        .strip_prefix("[feishu_inbound_message]\n")
        .expect("structured marker");
    serde_json::from_str(payload).expect("structured summary json")
}

fn parse_feishu_card_callback_summary(text: &str) -> Value {
    let payload = text
        .strip_prefix("[feishu_card_callback]\n")
        .expect("card callback structured marker");
    serde_json::from_str(payload).expect("structured callback summary json")
}

fn parse_feishu_download_hint(summary: &Value, payload_type: &str) -> Value {
    summary
        .get("resource_download_hints")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("payload_type").and_then(Value::as_str) == Some(payload_type))
        })
        .cloned()
        .unwrap_or_else(|| panic!("missing resource download hint for payload_type={payload_type}"))
}

fn parse_feishu_resource_inventory(summary: &Value) -> Vec<Value> {
    summary
        .get("resource_inventory")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| panic!("missing resource inventory"))
}

fn expect_url_verification(action: FeishuWebhookAction) -> String {
    match action {
        FeishuWebhookAction::UrlVerification { challenge } => challenge,
        FeishuWebhookAction::Ignore => panic!("unexpected ignore action"),
        FeishuWebhookAction::Inbound(event) => panic!("unexpected inbound action: {event:?}"),
        FeishuWebhookAction::CardCallback(event) => {
            panic!("unexpected card callback action: {event:?}")
        }
    }
}

fn expect_inbound(action: FeishuWebhookAction) -> FeishuInboundEvent {
    match action {
        FeishuWebhookAction::Inbound(event) => event,
        FeishuWebhookAction::UrlVerification { challenge } => {
            panic!("unexpected url verification action: {challenge}")
        }
        FeishuWebhookAction::Ignore => panic!("unexpected ignore action"),
        FeishuWebhookAction::CardCallback(event) => {
            panic!("unexpected card callback action: {event:?}")
        }
    }
}

fn expect_card_callback(action: FeishuWebhookAction) -> FeishuCardCallbackEvent {
    match action {
        FeishuWebhookAction::CardCallback(event) => event,
        FeishuWebhookAction::UrlVerification { challenge } => {
            panic!("unexpected url verification action: {challenge}")
        }
        FeishuWebhookAction::Ignore => panic!("unexpected ignore action"),
        FeishuWebhookAction::Inbound(event) => panic!("unexpected inbound action: {event:?}"),
    }
}

#[test]
fn feishu_url_verification_payload_parses() {
    let payload = json!({
        "type": "url_verification",
        "token": "token-123",
        "challenge": "abc"
    });
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &BTreeSet::new(),
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("parse feishu url verification");

    let challenge = expect_url_verification(action);
    assert_eq!(challenge, "abc");
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
                "sender_type": "user",
                "sender_id": {
                    "open_id": "ou_sender_1"
                }
            },
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_123",
                "root_id": "om_root_1",
                "message_type": "text",
                "content": "{\"text\":\"hello loongclaw\"}"
            }
        }
    });

    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "work",
        "feishu_cli_a1b2c3",
    )
    .expect("parse feishu event");

    let event = expect_inbound(action);
    assert_eq!(event.event_id, "evt_1");
    assert_eq!(event.session.configured_account_id.as_deref(), Some("work"));
    assert_eq!(
        event.session.session_key(),
        "feishu:cfg=work:feishu_cli_a1b2c3:oc_123:ou_sender_1:om_root_1"
    );
    assert_eq!(
        event.reply_target,
        ChannelOutboundTarget::feishu_message_reply("om_123")
            .with_feishu_reply_chat_id("oc_123")
            .with_feishu_reply_in_thread(true)
    );
    assert_eq!(event.reply_target.platform, ChannelPlatform::Feishu);
    assert_eq!(
        event.reply_target.kind,
        ChannelOutboundTargetKind::MessageReply
    );
    assert_eq!(event.reply_target.feishu_reply_in_thread(), Some(true));
    assert_eq!(event.text, "hello loongclaw");
}

#[test]
fn feishu_message_event_is_ignored_when_sender_is_not_allowlisted() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_sender_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {
                "sender_type": "user",
                "sender_id": {
                    "open_id": "ou_blocked"
                }
            },
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_123",
                "message_type": "text",
                "content": "{\"text\":\"hello loongclaw\"}"
            }
        }
    });

    let access_policy = ChannelInboundAccessPolicy::from_string_lists(
        &["oc_123".to_owned()],
        &["ou_allowed".to_owned()],
        true,
    );
    let action = parse_feishu_webhook_payload_with_access_policy(
        &payload,
        Some("token-123"),
        None,
        &access_policy,
        true,
        "work",
        "feishu_cli_a1b2c3",
    )
    .expect("parse feishu event");

    assert!(matches!(action, FeishuWebhookAction::Ignore));
}

#[test]
fn feishu_websocket_message_event_parses_without_verification_token() {
    let payload = json!({
        "header": {
            "event_id": "evt_ws_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {
                "sender_type": "user",
                "sender_id": {
                    "open_id": "ou_sender_ws_1"
                }
            },
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_ws_123",
                "message_type": "text",
                "content": "{\"text\":\"hello from websocket\"}"
            }
        }
    });

    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_inbound_payload(
        &payload,
        FeishuTransportAuth::websocket(),
        &allowlist,
        true,
        "work",
        "feishu_cli_a1b2c3",
    )
    .expect("parse websocket feishu event");

    let event = expect_inbound(action);
    assert_eq!(event.event_id, "evt_ws_1");
    assert_eq!(event.text, "hello from websocket");
    assert_eq!(event.session.configured_account_id.as_deref(), Some("work"));
    assert_eq!(
        event.reply_target,
        ChannelOutboundTarget::feishu_message_reply("om_ws_123")
            .with_feishu_reply_chat_id("oc_123")
    );
}

#[test]
fn feishu_message_event_uses_thread_id_and_sender_open_id_when_present() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_thread_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {
                "sender_type": "user",
                "sender_id": {
                    "open_id": "ou_123",
                    "union_id": "on_456",
                    "user_id": "u_789"
                }
            },
            "message": {
                "chat_id": "oc_123",
                "thread_id": "omt_456",
                "message_id": "om_123",
                "message_type": "text",
                "content": "{\"text\":\"hello loongclaw\"}"
            }
        }
    });

    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_main",
        "feishu_main",
    )
    .expect("parse feishu event");

    let event = expect_inbound(action);

    assert_eq!(
        event.session.session_key(),
        "feishu:feishu_main:oc_123:ou_123:omt_456"
    );
    assert_eq!(event.session.thread_id.as_deref(), Some("omt_456"));
    assert_eq!(event.reply_target.feishu_reply_chat_id(), Some("oc_123"));
    assert_eq!(event.reply_target.feishu_reply_in_thread(), Some(true));
    assert_eq!(event.session.participant_id.as_deref(), Some("ou_123"));
    assert_eq!(
        event.principal.as_ref().map(|value| value.open_id.as_str()),
        Some("ou_123")
    );
}

#[test]
fn feishu_message_without_sender_open_id_keeps_principal_empty() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_no_sender_open_id",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {
                "sender_type": "user",
                "sender_id": {
                    "union_id": "on_456",
                    "user_id": "u_789"
                }
            },
            "message": {
                "chat_id": "oc_123",
                "root_id": "om_root_1",
                "message_id": "om_123",
                "message_type": "text",
                "content": "{\"text\":\"hello loongclaw\"}"
            }
        }
    });

    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_main",
        "feishu_main",
    )
    .expect("parse feishu event");

    let event = expect_inbound(action);

    assert_eq!(event.session.thread_id.as_deref(), Some("om_root_1"));
    assert!(event.session.participant_id.is_none());
    assert!(event.principal.is_none());
}

#[test]
fn feishu_card_callback_v2_payload_is_not_ignored() {
    let payload = json!({
        "header": {
            "event_id": "evt_card_v2_1",
            "event_type": "card.action.trigger",
            "token": "token-123"
        },
        "event": {
            "app_id": "cli_a1b2c3",
            "token": "c-123",
            "operator": {
                "operator_id": {
                    "open_id": "ou_operator_1",
                    "union_id": "on_operator_1",
                    "user_id": "u_operator_1"
                }
            },
            "action": {
                "tag": "button",
                "name": "approve_request",
                "value": {
                    "ticket_id": "T-100"
                },
                "form_value": {
                    "comment": "ship it"
                }
            },
            "context": {
                "open_message_id": "om_callback_1",
                "open_chat_id": "oc_123"
            }
        }
    });

    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_main",
        "feishu_main",
    )
    .expect("parse v2 callback payload");

    let event = expect_card_callback(action);
    assert_eq!(event.version, FeishuCardCallbackVersion::V2);
    assert_eq!(event.event_id, "evt_card_v2_1");
    assert_eq!(event.callback_token.as_deref(), Some("c-123"));
    assert_eq!(event.action.tag, "button");
    assert_eq!(event.action.name.as_deref(), Some("approve_request"));
    assert_eq!(event.action.value, Some(json!({"ticket_id": "T-100"})));
    assert_eq!(event.action.form_value, Some(json!({"comment": "ship it"})));
    assert_eq!(
        event.context.open_message_id.as_deref(),
        Some("om_callback_1")
    );
    assert_eq!(event.context.open_chat_id.as_deref(), Some("oc_123"));
    assert_eq!(
        event.session.session_key(),
        "feishu:feishu_main:oc_123:ou_operator_1:om_callback_1"
    );
    assert_eq!(
        event.principal.as_ref().map(|value| value.open_id.as_str()),
        Some("ou_operator_1")
    );

    let summary = parse_feishu_card_callback_summary(&event.text);
    assert_eq!(summary["callback_version"], "v2");
    assert_eq!(summary["tag"], "button");
    assert_eq!(summary["name"], "approve_request");
    assert_eq!(summary["operator_open_id"], "ou_operator_1");
    assert_eq!(summary["open_message_id"], "om_callback_1");
    assert_eq!(summary["open_chat_id"], "oc_123");
    assert!(summary["card_update_hint"].as_str().is_some_and(|hint| {
        hint.contains("shared=true")
            && hint.contains("open_ids")
            && hint.contains("markdown")
            && hint.contains("30 minutes")
            && hint.contains("twice")
    }));
    assert!(
        summary["callback_response_hint"]
            .as_str()
            .is_some_and(|hint| {
                hint.contains("[feishu_callback_response]")
                    && hint.contains("\"mode\":\"toast\"")
                    && hint.contains("\"mode\":\"card\"")
                    && hint.contains("\"card\"")
                    && hint.contains("\"markdown\"")
                    && hint.contains("\"kind\":\"success|info|warning|error\"")
            })
    );
}

#[test]
fn feishu_card_callback_is_ignored_when_sender_is_not_allowlisted() {
    let payload = json!({
        "header": {
            "event_id": "evt_card_sender_1",
            "event_type": "card.action.trigger",
            "token": "token-123"
        },
        "event": {
            "app_id": "cli_a1b2c3",
            "token": "c-123",
            "operator": {
                "operator_id": {
                    "open_id": "ou_blocked"
                }
            },
            "action": {
                "tag": "button",
                "name": "approve_request"
            },
            "context": {
                "open_message_id": "om_callback_1",
                "open_chat_id": "oc_123"
            }
        }
    });

    let access_policy = ChannelInboundAccessPolicy::from_string_lists(
        &["oc_123".to_owned()],
        &["ou_allowed".to_owned()],
        true,
    );
    let action = parse_feishu_webhook_payload_with_access_policy(
        &payload,
        Some("token-123"),
        None,
        &access_policy,
        true,
        "feishu_main",
        "feishu_main",
    )
    .expect("parse v2 callback payload");

    assert!(matches!(action, FeishuWebhookAction::Ignore));
}

#[test]
fn feishu_card_callback_v1_payload_is_not_ignored() {
    let payload = json!({
        "open_id": "ou_operator_legacy",
        "user_id": "u_operator_legacy",
        "open_message_id": "om_callback_legacy",
        "open_chat_id": "oc_123",
        "tenant_key": "tenant_1",
        "token": "token-123",
        "action": {
            "tag": "button",
            "name": "reject_request",
            "value": {
                "ticket_id": "T-101"
            }
        }
    });

    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_main",
        "feishu_main",
    )
    .expect("parse legacy callback payload");

    let event = expect_card_callback(action);
    assert_eq!(event.version, FeishuCardCallbackVersion::V1);
    assert!(
        event
            .event_id
            .starts_with("card_callback:v1:om_callback_legacy:")
    );
    assert!(event.callback_token.is_none());
    assert_eq!(event.action.tag, "button");
    assert_eq!(event.action.name.as_deref(), Some("reject_request"));
    assert_eq!(event.action.value, Some(json!({"ticket_id": "T-101"})));
    assert_eq!(
        event.context.open_message_id.as_deref(),
        Some("om_callback_legacy")
    );
    assert_eq!(event.context.open_chat_id.as_deref(), Some("oc_123"));
    assert_eq!(
        event.session.session_key(),
        "feishu:feishu_main:oc_123:ou_operator_legacy:om_callback_legacy"
    );
    assert_eq!(
        event
            .principal
            .as_ref()
            .and_then(|value| value.tenant_key.as_deref()),
        Some("tenant_1")
    );

    let summary = parse_feishu_card_callback_summary(&event.text);
    assert_eq!(summary["callback_version"], "v1");
    assert_eq!(summary["tag"], "button");
    assert_eq!(summary["name"], "reject_request");
    assert_eq!(summary["operator_open_id"], "ou_operator_legacy");
    assert_eq!(summary["open_message_id"], "om_callback_legacy");
    assert_eq!(summary["open_chat_id"], "oc_123");
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
    let error = parse_feishu_webhook_payload(
        &payload,
        Some("token-y"),
        None,
        &BTreeSet::new(),
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect_err("token mismatch should fail");
    assert!(error.contains("unauthorized"));
}

#[test]
fn feishu_image_message_event_is_preserved_as_structured_text() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_image_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_123",
                "message_type": "image",
                "content": "{\"image_key\":\"img_v2_123\"}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("image payload should parse");

    let event = expect_inbound(action);
    assert!(event.text.contains("[feishu_inbound_message]"));
    assert!(event.text.contains("\"message_type\":\"image\""));
    assert!(event.text.contains("\"image_key\":\"img_v2_123\""));
    assert_eq!(event.resources.len(), 1);
    assert_eq!(event.resources[0].resource_type, "image");
    assert_eq!(event.resources[0].file_key, "img_v2_123");
    assert!(event.resources[0].file_name.is_none());
    let summary = parse_feishu_inbound_summary(event.text.as_str());
    let hint = parse_feishu_download_hint(&summary, "image");
    assert_eq!(hint["tool"], "feishu.messages.resource.get");
    assert_eq!(hint["canonical_type"], "image");
    assert_eq!(hint["use_key_from"], "image_key");
    assert_eq!(hint["payload_message_id_can_default_from_ingress"], true);
    assert_eq!(hint["payload_file_key_can_default_from_ingress"], true);
    assert_eq!(hint["save_as_required"], true);
    assert!(
        hint["note"]
            .as_str()
            .is_some_and(|note| note.contains("payload.message_id"))
    );
    assert!(
        summary["resource_selection_hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("override payload.message_id to another message"))
    );
}

#[test]
fn feishu_file_message_event_is_preserved_as_structured_text() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_file_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_456",
                "message_type": "file",
                "content": "{\"file_key\":\"file_v2_123\",\"file_name\":\"report.pdf\"}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("file payload should parse");

    let event = expect_inbound(action);
    assert!(event.text.contains("[feishu_inbound_message]"));
    assert!(event.text.contains("\"message_type\":\"file\""));
    assert!(event.text.contains("\"file_key\":\"file_v2_123\""));
    assert!(event.text.contains("\"file_name\":\"report.pdf\""));
    assert_eq!(event.resources.len(), 1);
    assert_eq!(event.resources[0].resource_type, "file");
    assert_eq!(event.resources[0].file_key, "file_v2_123");
    assert_eq!(event.resources[0].file_name.as_deref(), Some("report.pdf"));
    let summary = parse_feishu_inbound_summary(event.text.as_str());
    let hint = parse_feishu_download_hint(&summary, "file");
    assert_eq!(hint["tool"], "feishu.messages.resource.get");
    assert_eq!(hint["canonical_type"], "file");
    assert_eq!(hint["use_key_from"], "file_key");
    assert_eq!(hint["payload_message_id_can_default_from_ingress"], true);
    assert_eq!(hint["payload_file_key_can_default_from_ingress"], true);
    assert_eq!(hint["save_as_required"], true);
    assert!(
        hint["note"]
            .as_str()
            .is_some_and(|note| note.contains("payload.message_id"))
    );
}

#[test]
fn feishu_post_message_event_is_preserved_as_structured_text() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_post_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_789",
                "message_type": "post",
                "content": "{\"zh_cn\":{\"title\":\"Status update\",\"content\":[[{\"tag\":\"text\",\"text\":\"hello\"},{\"tag\":\"a\",\"text\":\"link\",\"href\":\"https://example.com\"},{\"tag\":\"img\",\"image_key\":\"img_v2_post_123\"},{\"tag\":\"media\",\"file_key\":\"file_v2_post_456\",\"image_key\":\"img_v2_post_preview_456\",\"file_name\":\"demo.mp4\"},{\"tag\":\"img\",\"image_key\":\"img_v2_post_123\"}]]}}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("post payload should parse");

    let event = expect_inbound(action);
    assert!(event.text.contains("[feishu_inbound_message]"));
    assert!(event.text.contains("\"message_type\":\"post\""));
    assert!(event.text.contains("\"title\":\"Status update\""));
    assert!(event.text.contains("\"text\":\"hello link\""));
    assert_eq!(event.resources.len(), 3);
    assert_eq!(event.resources[0].resource_type, "image");
    assert_eq!(event.resources[0].file_key, "img_v2_post_123");
    assert!(event.resources[0].file_name.is_none());
    assert_eq!(event.resources[1].resource_type, "file");
    assert_eq!(event.resources[1].file_key, "file_v2_post_456");
    assert_eq!(event.resources[1].file_name.as_deref(), Some("demo.mp4"));
    assert_eq!(event.resources[2].resource_type, "image");
    assert_eq!(event.resources[2].file_key, "img_v2_post_preview_456");
    assert!(event.resources[2].file_name.is_none());
    let summary = parse_feishu_inbound_summary(event.text.as_str());
    let file_hint = parse_feishu_download_hint(&summary, "file");
    assert_eq!(file_hint["canonical_type"], "file");
    assert_eq!(file_hint["use_key_from"], "file_key");
    assert_eq!(
        file_hint["payload_file_key_can_default_from_ingress"],
        false
    );
    assert!(
        file_hint["note"]
            .as_str()
            .is_some_and(|note| note.contains("multiple resources"))
    );
    assert!(
        file_hint["note"]
            .as_str()
            .is_some_and(|note| note.contains("payload.message_id"))
    );
    let image_hint = parse_feishu_download_hint(&summary, "image");
    assert_eq!(image_hint["canonical_type"], "image");
    assert_eq!(image_hint["use_key_from"], "image_key");
    assert_eq!(
        image_hint["payload_file_key_can_default_from_ingress"],
        false
    );
    assert!(
        image_hint["note"]
            .as_str()
            .is_some_and(|note| note.contains("multiple resources"))
    );
    assert!(
        image_hint["note"]
            .as_str()
            .is_some_and(|note| note.contains("payload.message_id"))
    );
    let inventory = parse_feishu_resource_inventory(&summary);
    assert_eq!(inventory.len(), 3);
    assert_eq!(inventory[0]["role"], "post_image");
    assert_eq!(inventory[0]["payload_type"], "image");
    assert_eq!(inventory[0]["canonical_type"], "image");
    assert_eq!(inventory[0]["source_key_field"], "image_key");
    assert_eq!(inventory[0]["file_key"], "img_v2_post_123");
    assert_eq!(inventory[0]["selection_required"], true);
    assert_eq!(inventory[1]["role"], "post_file");
    assert_eq!(inventory[1]["payload_type"], "file");
    assert_eq!(inventory[1]["canonical_type"], "file");
    assert_eq!(inventory[1]["source_key_field"], "file_key");
    assert_eq!(inventory[1]["file_key"], "file_v2_post_456");
    assert_eq!(inventory[1]["file_name"], "demo.mp4");
    assert_eq!(inventory[1]["selection_required"], true);
    assert_eq!(inventory[2]["role"], "post_image");
    assert_eq!(inventory[2]["payload_type"], "image");
    assert_eq!(inventory[2]["canonical_type"], "image");
    assert_eq!(inventory[2]["source_key_field"], "image_key");
    assert_eq!(inventory[2]["file_key"], "img_v2_post_preview_456");
    assert_eq!(inventory[2]["selection_required"], true);
    assert!(
        summary["resource_selection_hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("choose one entry from resource_inventory"))
    );
    assert!(
        summary["resource_selection_hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("payload.file_key"))
    );
    assert!(
        summary["resource_selection_hint"]
            .as_str()
            .is_some_and(
                |hint| hint.contains("payload.type") && hint.contains("uniquely identifies")
            )
    );
    assert!(
        summary["resource_selection_hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("override payload.message_id to another message"))
    );
}

#[test]
fn feishu_audio_message_event_is_preserved_as_structured_text() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_audio_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_audio_123",
                "message_type": "audio",
                "content": "{\"file_key\":\"file_audio_v2_123\",\"duration\":2000}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("audio payload should parse");

    let event = expect_inbound(action);
    assert!(event.text.contains("[feishu_inbound_message]"));
    assert!(event.text.contains("\"message_type\":\"audio\""));
    assert!(event.text.contains("\"file_key\":\"file_audio_v2_123\""));
    assert!(event.text.contains("\"duration\":2000"));
    assert_eq!(event.resources.len(), 1);
    assert_eq!(event.resources[0].resource_type, "file");
    assert_eq!(event.resources[0].file_key, "file_audio_v2_123");
    assert!(event.resources[0].file_name.is_none());
    let summary = parse_feishu_inbound_summary(event.text.as_str());
    let hint = parse_feishu_download_hint(&summary, "audio");
    assert_eq!(hint["tool"], "feishu.messages.resource.get");
    assert_eq!(hint["canonical_type"], "file");
    assert_eq!(hint["use_key_from"], "file_key");
    assert_eq!(hint["payload_message_id_can_default_from_ingress"], true);
    assert_eq!(hint["payload_file_key_can_default_from_ingress"], true);
    assert!(
        hint["note"]
            .as_str()
            .is_some_and(|note| note.contains("payload.type=\"file\""))
    );
    assert!(
        hint["note"]
            .as_str()
            .is_some_and(|note| note.contains("payload.message_id"))
    );
}

#[test]
fn feishu_media_message_event_is_preserved_as_structured_text() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_media_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_media_123",
                "message_type": "media",
                "content": "{\"file_key\":\"file_media_v2_123\",\"image_key\":\"img_media_v2_123\",\"file_name\":\"clip.mp4\",\"duration\":2000}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("media payload should parse");

    let event = expect_inbound(action);
    assert!(event.text.contains("[feishu_inbound_message]"));
    assert!(event.text.contains("\"message_type\":\"media\""));
    assert!(event.text.contains("\"file_key\":\"file_media_v2_123\""));
    assert!(event.text.contains("\"image_key\":\"img_media_v2_123\""));
    assert!(event.text.contains("\"file_name\":\"clip.mp4\""));
    assert_eq!(event.resources.len(), 2);
    assert_eq!(event.resources[0].resource_type, "file");
    assert_eq!(event.resources[0].file_key, "file_media_v2_123");
    assert_eq!(event.resources[0].file_name.as_deref(), Some("clip.mp4"));
    assert_eq!(event.resources[1].resource_type, "image");
    assert_eq!(event.resources[1].file_key, "img_media_v2_123");
    assert!(event.resources[1].file_name.is_none());
    let summary = parse_feishu_inbound_summary(event.text.as_str());
    let media_hint = parse_feishu_download_hint(&summary, "media");
    assert_eq!(media_hint["canonical_type"], "file");
    assert_eq!(media_hint["use_key_from"], "file_key");
    assert_eq!(
        media_hint["payload_file_key_can_default_from_ingress"],
        false
    );
    assert!(
        media_hint["note"]
            .as_str()
            .is_some_and(|note| note.contains("preview image"))
    );
    assert!(
        media_hint["note"]
            .as_str()
            .is_some_and(|note| note.contains("payload.message_id"))
    );
    let image_hint = parse_feishu_download_hint(&summary, "image");
    assert_eq!(image_hint["canonical_type"], "image");
    assert_eq!(image_hint["use_key_from"], "image_key");
    assert_eq!(
        image_hint["payload_file_key_can_default_from_ingress"],
        false
    );
    assert!(
        image_hint["note"]
            .as_str()
            .is_some_and(|note| note.contains("preview image"))
    );
    assert!(
        image_hint["note"]
            .as_str()
            .is_some_and(|note| note.contains("payload.message_id"))
    );
    let inventory = parse_feishu_resource_inventory(&summary);
    assert_eq!(inventory.len(), 2);
    assert_eq!(inventory[0]["role"], "media_file");
    assert_eq!(inventory[0]["payload_type"], "media");
    assert_eq!(inventory[0]["canonical_type"], "file");
    assert_eq!(inventory[0]["source_key_field"], "file_key");
    assert_eq!(inventory[0]["file_key"], "file_media_v2_123");
    assert_eq!(inventory[0]["file_name"], "clip.mp4");
    assert_eq!(inventory[0]["selection_required"], true);
    assert_eq!(inventory[1]["role"], "media_preview_image");
    assert_eq!(inventory[1]["payload_type"], "image");
    assert_eq!(inventory[1]["canonical_type"], "image");
    assert_eq!(inventory[1]["source_key_field"], "image_key");
    assert_eq!(inventory[1]["file_key"], "img_media_v2_123");
    assert_eq!(inventory[1]["selection_required"], true);
    assert!(
        summary["resource_selection_hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("choose one entry from resource_inventory"))
    );
    assert!(
        summary["resource_selection_hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("payload.file_key"))
    );
    assert!(
        summary["resource_selection_hint"]
            .as_str()
            .is_some_and(
                |hint| hint.contains("payload.type") && hint.contains("uniquely identifies")
            )
    );
    assert!(
        summary["resource_selection_hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("override payload.message_id to another message"))
    );
}

#[test]
fn feishu_folder_message_event_is_preserved_as_structured_text_without_resources() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_folder_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_folder_123",
                "message_type": "folder",
                "content": "{\"file_key\":\"fld_v2_123\",\"file_name\":\"Project Folder\"}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("folder payload should parse");

    let event = expect_inbound(action);
    assert!(event.text.contains("[feishu_inbound_message]"));
    assert!(event.text.contains("\"message_type\":\"folder\""));
    assert!(event.text.contains("\"file_key\":\"fld_v2_123\""));
    assert!(event.text.contains("\"file_name\":\"Project Folder\""));
    assert!(event.text.contains("cannot be downloaded"));
    assert!(event.resources.is_empty());
}

#[test]
fn feishu_location_message_event_is_preserved_as_structured_text_without_resources() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_location_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_location_123",
                "message_type": "location",
                "content": "{\"name\":\"Shanghai Tower\",\"address\":\"Pudong, Shanghai\",\"longitude\":\"121.5018\",\"latitude\":\"31.2397\"}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("location payload should parse");

    let event = expect_inbound(action);
    assert!(event.text.contains("[feishu_inbound_message]"));
    assert!(event.text.contains("\"message_type\":\"location\""));
    assert!(event.text.contains("\"name\":\"Shanghai Tower\""));
    assert!(event.text.contains("\"address\":\"Pudong, Shanghai\""));
    assert!(
        event
            .text
            .contains("Structured Feishu location content preserved verbatim.")
    );
    assert!(event.resources.is_empty());
}

#[test]
fn feishu_interactive_message_event_extracts_high_signal_summary_fields() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_interactive_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_interactive_123",
                "message_type": "interactive",
                "content": "{\"type\":\"template\",\"elements\":[{\"tag\":\"button\",\"text\":{\"tag\":\"plain_text\",\"content\":\"Approve\"},\"action\":{\"type\":\"submit\",\"value\":{\"action\":\"approve\",\"block_id\":\"btn_approve\"}}}]}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("interactive payload should parse");

    let event = expect_inbound(action);
    let summary = parse_feishu_inbound_summary(event.text.as_str());
    assert_eq!(summary["message_type"], "interactive");
    assert_eq!(summary["type"], "template");
    assert_eq!(summary["action"], "approve");
    assert_eq!(summary["block_id"], "btn_approve");
    assert_eq!(summary["text"], "Approve");
    assert_eq!(
        summary["note"],
        "Interactive Feishu card content preserved with action metadata."
    );
    assert!(event.resources.is_empty());
}

#[test]
fn feishu_share_chat_message_event_extracts_high_signal_summary_fields() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_share_chat_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_share_chat_123",
                "message_type": "share_chat",
                "content": "{\"chat_id\":\"oc_shared_123\",\"user_id\":\"ou_owner_123\",\"title\":\"Ops Bridge\"}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("share_chat payload should parse");

    let event = expect_inbound(action);
    let summary = parse_feishu_inbound_summary(event.text.as_str());
    assert_eq!(summary["message_type"], "share_chat");
    assert_eq!(summary["chat_id"], "oc_shared_123");
    assert_eq!(summary["user_id"], "ou_owner_123");
    assert_eq!(summary["title"], "Ops Bridge");
    assert_eq!(
        summary["note"],
        "Shared Feishu chat content preserved with chat metadata."
    );
    assert!(event.resources.is_empty());
}

#[test]
fn feishu_share_user_message_event_extracts_high_signal_summary_fields() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_share_user_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_share_user_123",
                "message_type": "share_user",
                "content": "{\"user_id\":\"ou_user_123\",\"name\":\"Alice Chen\"}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("share_user payload should parse");

    let event = expect_inbound(action);
    let summary = parse_feishu_inbound_summary(event.text.as_str());
    assert_eq!(summary["message_type"], "share_user");
    assert_eq!(summary["user_id"], "ou_user_123");
    assert_eq!(summary["name"], "Alice Chen");
    assert_eq!(
        summary["note"],
        "Shared Feishu user content preserved with user metadata."
    );
    assert!(event.resources.is_empty());
}

#[test]
fn feishu_calendar_message_event_extracts_high_signal_summary_fields() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_calendar_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_calendar_123",
                "message_type": "calendar",
                "content": "{\"event_id\":\"evt_cal_123\",\"start_time\":\"1710300000\",\"end_time\":\"1710303600\",\"summary\":\"Daily sync\"}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("calendar payload should parse");

    let event = expect_inbound(action);
    let summary = parse_feishu_inbound_summary(event.text.as_str());
    assert_eq!(summary["message_type"], "calendar");
    assert_eq!(summary["event_id"], "evt_cal_123");
    assert_eq!(summary["start_time"], "1710300000");
    assert_eq!(summary["end_time"], "1710303600");
    assert_eq!(summary["summary"], "Daily sync");
    assert_eq!(
        summary["note"],
        "Feishu calendar event content preserved with event metadata."
    );
    assert!(event.resources.is_empty());
}

#[test]
fn feishu_share_calendar_event_message_event_extracts_calendar_variant_summary_fields() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_share_calendar_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_share_calendar_123",
                "message_type": "share_calendar_event",
                "content": "{\"summary\":\"Event Sharing Card Test\",\"start_time\":\"1608265395000\",\"end_time\":\"1608267015000\"}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("share calendar payload should parse");

    let event = expect_inbound(action);
    let summary = parse_feishu_inbound_summary(event.text.as_str());
    assert_eq!(summary["message_type"], "share_calendar_event");
    assert_eq!(summary["start_time"], "1608265395000");
    assert_eq!(summary["end_time"], "1608267015000");
    assert_eq!(summary["summary"], "Event Sharing Card Test");
    assert_eq!(
        summary["note"],
        "Feishu calendar event content preserved with event metadata."
    );
    assert!(event.resources.is_empty());
}

#[test]
fn feishu_general_calendar_message_event_extracts_calendar_variant_summary_fields() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_general_calendar_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_general_calendar_123",
                "message_type": "general_calendar",
                "content": "{\"summary\":\"Event Transfer Card Test\",\"start_time\":\"1608265395000\",\"end_time\":\"1608267015000\"}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("general calendar payload should parse");

    let event = expect_inbound(action);
    let summary = parse_feishu_inbound_summary(event.text.as_str());
    assert_eq!(summary["message_type"], "general_calendar");
    assert_eq!(summary["start_time"], "1608265395000");
    assert_eq!(summary["end_time"], "1608267015000");
    assert_eq!(summary["summary"], "Event Transfer Card Test");
    assert_eq!(
        summary["note"],
        "Feishu calendar event content preserved with event metadata."
    );
    assert!(event.resources.is_empty());
}

#[test]
fn feishu_system_message_event_extracts_high_signal_summary_fields() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_system_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_system_123",
                "message_type": "system",
                "content": "{\"template\":\"{from_user} invited {to_chatters} to this chat.\",\"from_user\":[\"botName\"],\"to_chatters\":[\"Xiaoming\",\"Xiaowang\"]}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("system payload should parse");

    let event = expect_inbound(action);
    let summary = parse_feishu_inbound_summary(event.text.as_str());
    assert_eq!(summary["message_type"], "system");
    assert_eq!(
        summary["template"],
        "{from_user} invited {to_chatters} to this chat."
    );
    assert_eq!(summary["from_user"], json!(["botName"]));
    assert_eq!(summary["to_chatters"], json!(["Xiaoming", "Xiaowang"]));
    assert_eq!(
        summary["note"],
        "Feishu system message content preserved with template variables."
    );
    assert!(event.resources.is_empty());
}

#[test]
fn feishu_video_chat_message_event_extracts_high_signal_summary_fields() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_video_chat_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_video_chat_123",
                "message_type": "video_chat",
                "content": "{\"topic\":\"Video call message\",\"start_time\":\"6745784522794598413\"}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("video chat payload should parse");

    let event = expect_inbound(action);
    let summary = parse_feishu_inbound_summary(event.text.as_str());
    assert_eq!(summary["message_type"], "video_chat");
    assert_eq!(summary["topic"], "Video call message");
    assert_eq!(summary["start_time"], "6745784522794598413");
    assert_eq!(
        summary["note"],
        "Feishu video chat content preserved with call metadata."
    );
    assert!(event.resources.is_empty());
}

#[test]
fn feishu_todo_message_event_extracts_high_signal_summary_fields() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_todo_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_todo_123",
                "message_type": "todo",
                "content": "{\"task_id\":\"acd096a5-a157-4b9d-80e2-5b317456f005\",\"summary\":{\"title\":\"Health Tips\",\"content\":[[{\"tag\":\"text\",\"text\":\"Eat more fruits\"}]]},\"due_time\":\"1623124318000\"}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("todo payload should parse");

    let event = expect_inbound(action);
    let summary = parse_feishu_inbound_summary(event.text.as_str());
    assert_eq!(summary["message_type"], "todo");
    assert_eq!(summary["task_id"], "acd096a5-a157-4b9d-80e2-5b317456f005");
    assert_eq!(summary["title"], "Health Tips");
    assert_eq!(summary["due_time"], "1623124318000");
    assert_eq!(summary["text"], "Eat more fruits");
    assert_eq!(
        summary["note"],
        "Feishu todo content preserved with task metadata."
    );
    assert!(event.resources.is_empty());
}

#[test]
fn feishu_vote_message_event_extracts_high_signal_summary_fields() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_vote_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_vote_123",
                "message_type": "vote",
                "content": "{\"topic\":\"vote test\",\"options\":[\"option 1\",\"option 2\",\"option 3\"]}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("vote payload should parse");

    let event = expect_inbound(action);
    let summary = parse_feishu_inbound_summary(event.text.as_str());
    assert_eq!(summary["message_type"], "vote");
    assert_eq!(summary["topic"], "vote test");
    assert_eq!(
        summary["options"],
        json!(["option 1", "option 2", "option 3"])
    );
    assert_eq!(
        summary["note"],
        "Feishu vote content preserved with topic and options."
    );
    assert!(event.resources.is_empty());
}

#[test]
fn feishu_merge_forward_message_event_extracts_high_signal_summary_fields() {
    let payload = json!({
        "token": "token-123",
        "header": {
            "event_id": "evt_merge_forward_1",
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {"sender_type": "user"},
            "message": {
                "chat_id": "oc_123",
                "message_id": "om_merge_forward_123",
                "message_type": "merge_forward",
                "content": "{\"content\":\"Merged and Forwarded Message\"}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("merge forward payload should parse");

    let event = expect_inbound(action);
    let summary = parse_feishu_inbound_summary(event.text.as_str());
    assert_eq!(summary["message_type"], "merge_forward");
    assert_eq!(summary["merged_content"], "Merged and Forwarded Message");
    assert_eq!(
        summary["note"],
        "Feishu merged-forward content preserved; fetch the message body explicitly to inspect sub-messages."
    );
    assert!(event.resources.is_empty());
}

#[test]
fn feishu_unsupported_message_type_is_ignored() {
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
                "message_type": "unknown_type",
                "content": "{}"
            }
        }
    });
    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("unsupported payload should parse");

    assert!(matches!(action, FeishuWebhookAction::Ignore));
}

fn encrypt_event_payload_for_test(plain_payload: &str, encrypt_key: &str) -> String {
    use cbc::cipher::{BlockModeEncrypt, KeyIvInit};

    let key = Sha256::digest(encrypt_key.as_bytes());
    let iv = [7_u8; 16];

    let mut buffer = plain_payload.as_bytes().to_vec();
    let message_len = buffer.len();
    let pad_len = 16 - (message_len % 16);
    buffer.resize(message_len + pad_len, 0);

    let encrypted = cbc::Encryptor::<Aes256>::new_from_slices(&key, &iv)
        .expect("create encryptor")
        .encrypt_padded::<Pkcs7>(&mut buffer, message_len)
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
            "sender": {
                "sender_type": "user",
                "sender_id": {
                    "open_id": "ou_sender_encrypt"
                }
            },
            "message": {
                "chat_id": "oc_encrypt",
                "message_id": "om_encrypt",
                "thread_id": "om_thread_encrypt",
                "message_type": "text",
                "content": "{\"text\":\"encrypted hello\"}"
            }
        }
    });
    let plain_payload_str = serde_json::to_string(&plain_payload).expect("encode plain payload");
    let encrypted = encrypt_event_payload_for_test(&plain_payload_str, "encrypt-key");
    let wrapper = json!({ "encrypt": encrypted });

    let allowlist = BTreeSet::from([String::from("oc_encrypt")]);
    let parsed = parse_feishu_webhook_payload(
        &wrapper,
        Some("token-123"),
        Some("encrypt-key"),
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("parse encrypted payload");

    let event = expect_inbound(parsed);
    assert_eq!(event.event_id, "evt_encrypted_1");
    assert_eq!(
        event.session.session_key(),
        "feishu:feishu_cli_a1b2c3:oc_encrypt:ou_sender_encrypt:om_thread_encrypt"
    );
    assert_eq!(
        event.reply_target,
        ChannelOutboundTarget::feishu_message_reply("om_encrypt")
            .with_feishu_reply_chat_id("oc_encrypt")
            .with_feishu_reply_in_thread(true)
    );
    assert_eq!(event.reply_target.feishu_reply_in_thread(), Some(true));
    assert_eq!(event.text, "encrypted hello");
}

#[test]
fn feishu_encrypted_payload_requires_encrypt_key() {
    let wrapper = json!({ "encrypt": "opaque" });
    let error = parse_feishu_webhook_payload(
        &wrapper,
        Some("token-123"),
        None,
        &BTreeSet::new(),
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect_err("encrypted payload without key should fail");

    assert!(error.contains("encrypt key is not configured"));
}

#[test]
fn feishu_message_event_is_ignored_when_allowlist_is_empty() {
    let payload = json!({
        "token": "token-123",
        "header": {
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

    let action = parse_feishu_webhook_payload(
        &payload,
        Some("token-123"),
        None,
        &BTreeSet::new(),
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect("parse feishu event");
    assert!(matches!(action, FeishuWebhookAction::Ignore));
}

#[test]
fn feishu_message_event_requires_verification_token_configuration() {
    let payload = json!({
        "token": "token-123",
        "header": {
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

    let allowlist = BTreeSet::from([String::from("oc_123")]);
    let error = parse_feishu_webhook_payload(
        &payload,
        None,
        None,
        &allowlist,
        true,
        "feishu_cli_a1b2c3",
        "feishu_cli_a1b2c3",
    )
    .expect_err("missing verification token should fail");

    assert!(error.contains("verification token is not configured"));
}
