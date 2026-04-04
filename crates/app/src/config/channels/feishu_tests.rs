use super::*;
use serde_json::json;

#[test]
fn feishu_multi_account_resolution_merges_base_and_account_overrides() {
    let config: FeishuChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "mode": "webhook",
        "app_id_env": "BASE_FEISHU_APP_ID",
        "app_secret_env": "BASE_FEISHU_APP_SECRET",
        "verification_token_env": "BASE_FEISHU_VERIFY",
        "encrypt_key_env": "BASE_FEISHU_ENCRYPT",
        "receive_id_type": "chat_id",
        "webhook_bind": "127.0.0.1:8080",
        "webhook_path": "/feishu/events",
        "allowed_chat_ids": ["oc_base"],
        "acp": {
            "bootstrap_mcp_servers": ["filesystem"],
            "working_directory": " /workspace/base "
        },
        "default_account": "Lark Prod",
        "accounts": {
            "Lark Prod": {
                "domain": "lark",
                "app_id": "cli_lark_123",
                "app_secret": "secret",
                "verification_token": "verify",
                "encrypt_key": "encrypt",
                "allowed_chat_ids": ["oc_lark"],
                "acp": {
                    "bootstrap_mcp_servers": ["search"],
                    "working_directory": "/workspace/lark-prod"
                }
            },
            "Feishu Backup": {
                "enabled": false,
                "app_id": "cli_backup_456",
                "app_secret": "secret"
            }
        }
    }))
    .expect("deserialize feishu multi-account config");

    assert_eq!(
        config.configured_account_ids(),
        vec!["feishu-backup", "lark-prod"]
    );
    assert_eq!(config.default_configured_account_id(), "lark-prod");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default feishu account");
    assert_eq!(resolved.configured_account_id, "lark-prod");
    assert_eq!(resolved.domain, FeishuDomain::Lark);
    assert_eq!(resolved.account.id, "lark_cli_lark_123");
    assert_eq!(resolved.account.label, "lark:cli_lark_123");
    assert_eq!(resolved.allowed_chat_ids, vec!["oc_lark".to_owned()]);
    assert_eq!(
        resolved.acp.bootstrap_mcp_servers,
        vec!["search".to_owned()]
    );
    assert_eq!(
        resolved.acp.resolved_working_directory(),
        Some(std::path::PathBuf::from("/workspace/lark-prod"))
    );
    assert_eq!(resolved.receive_id_type, "chat_id");
    assert_eq!(resolved.mode, FeishuChannelServeMode::Webhook);
    assert_eq!(resolved.resolved_base_url(), "https://open.larksuite.com");
    assert!(resolved.ack_reactions);

    let disabled = config
        .resolve_account(Some("Feishu Backup"))
        .expect("resolve explicit feishu account");
    assert_eq!(disabled.configured_account_id, "feishu-backup");
    assert!(!disabled.enabled);
    assert_eq!(disabled.allowed_chat_ids, vec!["oc_base".to_owned()]);
    assert_eq!(
        disabled.acp.bootstrap_mcp_servers,
        vec!["filesystem".to_owned()]
    );
    assert_eq!(
        disabled.acp.resolved_working_directory(),
        Some(std::path::PathBuf::from("/workspace/base"))
    );
}

#[test]
fn feishu_ack_reactions_default_to_true_and_allow_account_override() {
    let base_config: FeishuChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "app_id": "cli_base",
        "app_secret": "base-secret"
    }))
    .expect("deserialize base feishu config");

    let default_resolved = base_config
        .resolve_account(None)
        .expect("resolve default feishu account");
    assert!(default_resolved.ack_reactions);

    let override_config: FeishuChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "app_id": "cli_base",
        "app_secret": "base-secret",
        "accounts": {
            "Quiet Bot": {
                "ack_reactions": false,
                "app_id": "cli_quiet",
                "app_secret": "quiet-secret"
            }
        }
    }))
    .expect("deserialize override feishu config");

    let quiet_resolved = override_config
        .resolve_account(Some("Quiet Bot"))
        .expect("resolve quiet feishu account");
    assert!(!quiet_resolved.ack_reactions);
}

#[test]
fn feishu_mode_defaults_to_websocket_when_not_configured() {
    let config: FeishuChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "app_id": "cli_a1b2c3",
        "app_secret": "secret"
    }))
    .expect("deserialize feishu config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default feishu account");

    assert_eq!(resolved.mode, FeishuChannelServeMode::Websocket);
}
