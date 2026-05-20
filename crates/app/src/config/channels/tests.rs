use super::*;
use serde_json::json;

#[test]
fn telegram_account_identity_prefers_explicit_account_id() {
    let config: TelegramChannelConfig = serde_json::from_value(json!({
        "account_id": "Ops-Bot",
        "bot_token": "123456:token-value"
    }))
    .expect("deserialize telegram config");

    let identity = config.resolved_account_identity();
    assert_eq!(identity.id, "ops-bot");
    assert_eq!(identity.label, "Ops-Bot");
}

#[test]
fn telegram_account_identity_derives_from_bot_token_prefix() {
    let config = TelegramChannelConfig {
        bot_token: Some(loong_contracts::SecretRef::Inline(
            "987654:token-value".to_owned(),
        )),
        bot_token_env: None,
        ..TelegramChannelConfig::default()
    };

    let identity = config.resolved_account_identity();
    assert_eq!(identity.id, "bot_987654");
    assert_eq!(identity.label, "bot:987654");
}

#[test]
fn feishu_account_identity_prefers_explicit_account_id() {
    let config: FeishuChannelConfig = serde_json::from_value(json!({
        "account_id": "Customer-Support",
        "app_id": "cli_a1b2c3",
        "domain": "lark"
    }))
    .expect("deserialize feishu config");

    let identity = config.resolved_account_identity();
    assert_eq!(identity.id, "customer-support");
    assert_eq!(identity.label, "Customer-Support");
}

#[test]
fn feishu_account_identity_derives_from_domain_and_app_id() {
    let config = FeishuChannelConfig {
        app_id: Some(loong_contracts::SecretRef::Inline("cli_a1b2c3".to_owned())),
        app_id_env: None,
        domain: FeishuDomain::Lark,
        ..FeishuChannelConfig::default()
    };

    let identity = config.resolved_account_identity();
    assert_eq!(identity.id, "lark_cli_a1b2c3");
    assert_eq!(identity.label, "lark:cli_a1b2c3");
}

#[test]
fn configured_account_identity_rejects_non_alphanumeric_labels() {
    assert_eq!(resolve_configured_account_identity(Some(" !!! ")), None);
}

#[test]
fn telegram_multi_account_resolution_merges_base_and_account_overrides() {
    let config: TelegramChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "bot_token_env": "BASE_TELEGRAM_TOKEN",
        "polling_timeout_s": 25,
        "allowed_chat_ids": [1001],
        "allowed_sender_ids": [7],
        "require_mention": true,
        "acp": {
            "bootstrap_mcp_servers": ["filesystem"],
            "working_directory": " /workspace/base "
        },
        "default_account": "Work Bot",
        "accounts": {
            "Work Bot": {
                "account_id": "Ops-Bot",
                "bot_token_env": "WORK_TELEGRAM_TOKEN",
                "allowed_chat_ids": [2002],
                "allowed_sender_ids": [8],
                "require_mention": false,
                "acp": {
                    "bootstrap_mcp_servers": ["search"],
                    "working_directory": "/workspace/work-bot"
                }
            },
            "Personal": {
                "enabled": false,
                "bot_token_env": "PERSONAL_TELEGRAM_TOKEN"
            }
        }
    }))
    .expect("deserialize telegram multi-account config");

    assert_eq!(
        config.configured_account_ids(),
        vec!["personal", "work-bot"]
    );
    assert_eq!(config.default_configured_account_id(), "work-bot");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default telegram account");
    assert_eq!(resolved.configured_account_id, "work-bot");
    assert_eq!(resolved.account.id, "ops-bot");
    assert_eq!(resolved.account.label, "Ops-Bot");
    assert_eq!(
        resolved.bot_token_env.as_deref(),
        Some("WORK_TELEGRAM_TOKEN")
    );
    assert_eq!(resolved.allowed_chat_ids, vec![2002]);
    assert_eq!(resolved.allowed_sender_ids, vec![8]);
    assert!(!resolved.require_mention);
    assert_eq!(
        resolved.acp.bootstrap_mcp_servers,
        vec!["search".to_owned()]
    );
    assert_eq!(
        resolved.acp.resolved_working_directory(),
        Some(std::path::PathBuf::from("/workspace/work-bot"))
    );
    assert_eq!(resolved.polling_timeout_s, 25);

    let disabled = config
        .resolve_account(Some("Personal"))
        .expect("resolve explicit telegram account");
    assert_eq!(disabled.configured_account_id, "personal");
    assert!(!disabled.enabled);
    assert_eq!(disabled.allowed_chat_ids, vec![1001]);
    assert_eq!(disabled.allowed_sender_ids, vec![7]);
    assert!(disabled.require_mention);
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
fn telegram_resolve_account_for_session_account_id_matches_runtime_identity() {
    let config: TelegramChannelConfig = serde_json::from_value(json!({
        "default_account": "Work Bot",
        "accounts": {
            "Work Bot": {
                "account_id": "Ops-Bot",
                "bot_token_env": "WORK_TELEGRAM_TOKEN",
                "acp": {
                    "bootstrap_mcp_servers": ["search"],
                    "working_directory": "/workspace/work-bot"
                }
            }
        }
    }))
    .expect("deserialize telegram config");

    let resolved = config
        .resolve_account_for_session_account_id(Some("ops-bot"))
        .expect("resolve telegram runtime account identity");
    assert_eq!(resolved.configured_account_id, "work-bot");
    assert_eq!(resolved.account.id, "ops-bot");
    assert_eq!(
        resolved.acp.bootstrap_mcp_servers,
        vec!["search".to_owned()]
    );
    assert_eq!(
        resolved.acp.resolved_working_directory(),
        Some(std::path::PathBuf::from("/workspace/work-bot"))
    );
}

#[test]
fn telegram_default_account_selection_source_tracks_explicit_default() {
    let config: TelegramChannelConfig = serde_json::from_value(json!({
        "default_account": "Work Bot",
        "accounts": {
            "Work Bot": {
                "bot_token_env": "WORK_TELEGRAM_TOKEN"
            },
            "Personal": {
                "bot_token_env": "PERSONAL_TELEGRAM_TOKEN"
            }
        }
    }))
    .expect("deserialize telegram config");

    let selection = config.default_configured_account_selection();
    assert_eq!(selection.id, "work-bot");
    assert_eq!(
        selection.source,
        ChannelDefaultAccountSelectionSource::ExplicitDefault
    );
}

#[test]
fn telegram_default_account_selection_source_tracks_mapped_default() {
    let config: TelegramChannelConfig = serde_json::from_value(json!({
        "accounts": {
            "default": {
                "bot_token_env": "DEFAULT_TELEGRAM_TOKEN"
            },
            "Work": {
                "bot_token_env": "WORK_TELEGRAM_TOKEN"
            }
        }
    }))
    .expect("deserialize telegram config");

    let selection = config.default_configured_account_selection();
    assert_eq!(selection.id, "default");
    assert_eq!(
        selection.source,
        ChannelDefaultAccountSelectionSource::MappedDefault
    );
}

#[test]
fn telegram_default_account_selection_source_tracks_sorted_fallback() {
    let config: TelegramChannelConfig = serde_json::from_value(json!({
        "accounts": {
            "Work": {
                "bot_token_env": "WORK_TELEGRAM_TOKEN"
            },
            "Alerts": {
                "bot_token_env": "ALERTS_TELEGRAM_TOKEN"
            }
        }
    }))
    .expect("deserialize telegram config");

    let selection = config.default_configured_account_selection();
    assert_eq!(selection.id, "alerts");
    assert_eq!(
        selection.source,
        ChannelDefaultAccountSelectionSource::Fallback
    );
}

#[test]
fn telegram_default_account_does_not_override_single_account_fallback_identity() {
    let config: TelegramChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "default_account": "Work Bot",
        "bot_token": "123456:token-value",
        "allowed_chat_ids": [1001]
    }))
    .expect("deserialize single-account telegram config");

    let selection = config.default_configured_account_selection();
    assert_eq!(selection.id, "bot_123456");
    assert_eq!(
        selection.source,
        ChannelDefaultAccountSelectionSource::RuntimeIdentity
    );

    let resolved = config
        .resolve_account(None)
        .expect("resolve single-account telegram config");
    assert_eq!(resolved.configured_account_id, "bot_123456");
    assert_eq!(resolved.account.id, "bot_123456");
}

#[test]
fn telegram_resolved_account_route_flags_implicit_multi_account_fallback() {
    let config: TelegramChannelConfig = serde_json::from_value(json!({
        "accounts": {
            "Work": {
                "bot_token_env": "WORK_TELEGRAM_TOKEN"
            },
            "Alerts": {
                "bot_token_env": "ALERTS_TELEGRAM_TOKEN"
            }
        }
    }))
    .expect("deserialize telegram config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default telegram account");
    let route = config.resolved_account_route(None, resolved.configured_account_id.as_str());

    assert!(route.selected_by_default());
    assert_eq!(route.selected_configured_account_id, "alerts");
    assert_eq!(route.configured_account_count, 2);
    assert_eq!(
        route.default_account_source,
        ChannelDefaultAccountSelectionSource::Fallback
    );
    assert!(route.uses_implicit_fallback_default());
}

#[test]
fn telegram_resolved_account_route_does_not_flag_explicit_account_request() {
    let config: TelegramChannelConfig = serde_json::from_value(json!({
        "accounts": {
            "Work": {
                "bot_token_env": "WORK_TELEGRAM_TOKEN"
            },
            "Alerts": {
                "bot_token_env": "ALERTS_TELEGRAM_TOKEN"
            }
        }
    }))
    .expect("deserialize telegram config");

    let resolved = config
        .resolve_account(Some("Work"))
        .expect("resolve explicit telegram account");
    let route =
        config.resolved_account_route(Some("Work"), resolved.configured_account_id.as_str());

    assert!(!route.selected_by_default());
    assert_eq!(route.requested_account_id.as_deref(), Some("work"));
    assert_eq!(route.selected_configured_account_id, "work");
    assert!(!route.uses_implicit_fallback_default());
}

#[test]
fn discord_multi_account_resolution_merges_reserved_runtime_fields() {
    let config: DiscordChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "account_id": "discord-shared",
        "application_id": "base-application-id",
        "allowed_guild_ids": ["guild-base"],
        "default_account": "Ops",
        "accounts": {
            "Ops": {
                "account_id": "discord-ops",
                "bot_token_env": "DISCORD_OPS_TOKEN",
                "application_id_env": "DISCORD_OPS_APPLICATION_ID",
                "allowed_guild_ids": ["guild-ops", "guild-backup"]
            },
            "Backup": {
                "enabled": false,
                "bot_token_env": "DISCORD_BACKUP_TOKEN"
            }
        }
    }))
    .expect("deserialize discord config");

    let ops = config
        .resolve_account(None)
        .expect("resolve default discord account");
    assert_eq!(ops.configured_account_id, "ops");
    assert_eq!(ops.account.id, "discord-ops");
    assert_eq!(ops.account.label, "discord-ops");
    assert_eq!(
        ops.application_id_env.as_deref(),
        Some("DISCORD_OPS_APPLICATION_ID")
    );
    assert_eq!(ops.allowed_guild_ids, vec!["guild-ops", "guild-backup"]);

    let backup = config
        .resolve_account(Some("Backup"))
        .expect("resolve backup discord account");
    assert_eq!(backup.configured_account_id, "backup");
    assert!(!backup.enabled);
    assert_eq!(backup.account.id, "discord-shared");
    assert_eq!(
        backup.application_id.as_deref(),
        Some("base-application-id")
    );
    assert_eq!(backup.allowed_guild_ids, vec!["guild-base"]);
}

#[test]
fn feishu_multi_account_resolution_allows_websocket_mode_override() {
    let config: FeishuChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "mode": "webhook",
        "app_id": "cli_base",
        "app_secret": "base-secret",
        "allowed_chat_ids": ["oc_base"],
        "accounts": {
            "Long Connection": {
                "mode": "websocket",
                "app_id": "cli_ws",
                "app_secret": "ws-secret"
            }
        }
    }))
    .expect("deserialize feishu config");

    let resolved = config
        .resolve_account(Some("Long Connection"))
        .expect("resolve websocket feishu account");

    assert_eq!(resolved.mode, FeishuChannelServeMode::Websocket);
    assert_eq!(resolved.allowed_chat_ids, vec!["oc_base".to_owned()]);
}

#[test]
fn feishu_multi_account_resolution_merges_require_mention_override() {
    let config: FeishuChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "mode": "websocket",
        "app_id": "cli_base",
        "app_secret": "base-secret",
        "allowed_chat_ids": ["oc_base"],
        "allowed_sender_ids": ["ou_owner"],
        "require_mention": true,
        "accounts": {
            "Ops": {
                "account_id": "Ops-Bot",
                "app_id": "cli_ops",
                "app_secret": "ops-secret",
                "allowed_chat_ids": ["oc_ops"],
                "allowed_sender_ids": ["ou_ops"],
                "require_mention": false
            },
            "Backup": {
                "enabled": false,
                "app_id": "cli_backup",
                "app_secret": "backup-secret"
            }
        },
        "default_account": "Ops"
    }))
    .expect("deserialize feishu multi-account config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default feishu account");
    assert_eq!(resolved.configured_account_id, "ops");
    assert_eq!(resolved.account.id, "ops-bot");
    assert_eq!(resolved.allowed_chat_ids, vec!["oc_ops".to_owned()]);
    assert_eq!(resolved.allowed_sender_ids, vec!["ou_ops".to_owned()]);
    assert!(!resolved.require_mention);

    let backup = config
        .resolve_account(Some("Backup"))
        .expect("resolve backup feishu account");
    assert_eq!(backup.configured_account_id, "backup");
    assert!(!backup.enabled);
    assert_eq!(backup.allowed_chat_ids, vec!["oc_base".to_owned()]);
    assert_eq!(backup.allowed_sender_ids, vec!["ou_owner".to_owned()]);
    assert!(backup.require_mention);
}

#[test]
fn feishu_resolve_account_for_session_account_id_matches_runtime_identity() {
    let config: FeishuChannelConfig = serde_json::from_value(json!({
        "default_account": "Lark Prod",
        "accounts": {
            "Lark Prod": {
                "domain": "lark",
                "app_id": "cli_lark_123",
                "app_secret": "secret",
                "acp": {
                    "bootstrap_mcp_servers": ["search"],
                    "working_directory": "/workspace/lark-prod"
                }
            }
        }
    }))
    .expect("deserialize feishu config");

    let resolved = config
        .resolve_account_for_session_account_id(Some("lark_cli_lark_123"))
        .expect("resolve feishu runtime account identity");
    assert_eq!(resolved.configured_account_id, "lark-prod");
    assert_eq!(resolved.account.id, "lark_cli_lark_123");
    assert_eq!(
        resolved.acp.bootstrap_mcp_servers,
        vec!["search".to_owned()]
    );
    assert_eq!(
        resolved.acp.resolved_working_directory(),
        Some(std::path::PathBuf::from("/workspace/lark-prod"))
    );
}

#[test]
fn feishu_resolved_account_route_tracks_explicit_default_without_fallback_warning() {
    let config: FeishuChannelConfig = serde_json::from_value(json!({
        "default_account": "Lark Prod",
        "accounts": {
            "Lark Prod": {
                "domain": "lark",
                "app_id": "cli_lark_123",
                "app_secret": "secret"
            },
            "Feishu Backup": {
                "app_id": "cli_backup_456",
                "app_secret": "secret"
            }
        }
    }))
    .expect("deserialize feishu config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default feishu account");
    let route = config.resolved_account_route(None, resolved.configured_account_id.as_str());

    assert!(route.selected_by_default());
    assert_eq!(route.selected_configured_account_id, "lark-prod");
    assert_eq!(
        route.default_account_source,
        ChannelDefaultAccountSelectionSource::ExplicitDefault
    );
    assert!(!route.uses_implicit_fallback_default());
}

#[test]
fn matrix_multi_account_resolution_merges_sender_allowlist_overrides() {
    let config: MatrixChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "access_token_env": "BASE_MATRIX_ACCESS_TOKEN",
        "base_url": "https://matrix.example.org",
        "allowed_room_ids": ["!ops:example.org"],
        "allowed_sender_ids": ["@alice:example.org"],
        "require_mention": true,
        "accounts": {
            "Ops": {
                "account_id": "Ops-Bot",
                "access_token": "ops-token",
                "allowed_room_ids": ["!ops-room:example.org"],
                "allowed_sender_ids": ["@ops-user:example.org"],
                "require_mention": false
            },
            "Backup": {
                "enabled": false,
                "access_token": "backup-token"
            }
        },
        "default_account": "Ops"
    }))
    .expect("deserialize matrix multi-account config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default matrix account");
    assert_eq!(
        resolved.allowed_room_ids,
        vec!["!ops-room:example.org".to_owned()]
    );
    assert_eq!(
        resolved.allowed_sender_ids,
        vec!["@ops-user:example.org".to_owned()]
    );
    assert!(!resolved.require_mention);

    let backup = config
        .resolve_account(Some("Backup"))
        .expect("resolve backup matrix account");
    assert_eq!(backup.allowed_room_ids, vec!["!ops:example.org".to_owned()]);
    assert_eq!(
        backup.allowed_sender_ids,
        vec!["@alice:example.org".to_owned()]
    );
    assert!(backup.require_mention);
}

#[test]
fn wecom_account_identity_prefers_explicit_account_id() {
    let config: WecomChannelConfig = serde_json::from_value(json!({
        "account_id": "Ops-Bot",
        "bot_id": "bot_123"
    }))
    .expect("deserialize wecom config");

    let identity = config.resolved_account_identity();
    assert_eq!(identity.id, "ops-bot");
    assert_eq!(identity.label, "Ops-Bot");
}

#[test]
fn wecom_account_identity_derives_from_bot_id() {
    let config = WecomChannelConfig {
        bot_id: Some(loong_contracts::SecretRef::Inline("bot_123".to_owned())),
        bot_id_env: None,
        ..WecomChannelConfig::default()
    };

    let identity = config.resolved_account_identity();
    assert_eq!(identity.id, "wecom_bot_123");
    assert_eq!(identity.label, "wecom:bot_123");
}

#[test]
fn wecom_multi_account_resolution_merges_base_and_account_overrides() {
    let config: WecomChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "bot_id_env": "BASE_WECOM_BOT_ID",
        "secret_env": "BASE_WECOM_SECRET",
        "ping_interval_s": 45,
        "reconnect_interval_s": 12,
        "allowed_conversation_ids": ["group_base"],
        "allowed_sender_ids": ["user_base"],
        "acp": {
            "bootstrap_mcp_servers": ["filesystem"],
            "working_directory": " /workspace/base "
        },
        "default_account": "Work Bot",
        "accounts": {
            "Work Bot": {
                "account_id": "WeCom-Work",
                "bot_id": "bot_work",
                "secret": "secret-work",
                "allowed_conversation_ids": ["group_work"],
                "allowed_sender_ids": ["user_work"],
                "acp": {
                    "bootstrap_mcp_servers": ["search"],
                    "working_directory": "/workspace/work-bot"
                }
            },
            "Alerts": {
                "enabled": false,
                "bot_id": "bot_alerts",
                "secret": "secret-alerts"
            }
        }
    }))
    .expect("deserialize wecom multi-account config");

    assert_eq!(config.configured_account_ids(), vec!["alerts", "work-bot"]);
    assert_eq!(config.default_configured_account_id(), "work-bot");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default wecom account");
    assert_eq!(resolved.configured_account_id, "work-bot");
    assert_eq!(resolved.account.id, "wecom-work");
    assert_eq!(resolved.account.label, "WeCom-Work");
    assert_eq!(
        resolved.allowed_conversation_ids,
        vec!["group_work".to_owned()]
    );
    assert_eq!(resolved.allowed_sender_ids, vec!["user_work".to_owned()]);
    assert_eq!(resolved.ping_interval_s, 45);
    assert_eq!(resolved.reconnect_interval_s, 12);
    assert_eq!(
        resolved.acp.bootstrap_mcp_servers,
        vec!["search".to_owned()]
    );
    assert_eq!(
        resolved.acp.resolved_working_directory(),
        Some(std::path::PathBuf::from("/workspace/work-bot"))
    );
    assert_eq!(
        resolved.resolved_websocket_url(),
        "wss://openws.work.weixin.qq.com"
    );

    let disabled = config
        .resolve_account(Some("Alerts"))
        .expect("resolve explicit wecom account");
    assert_eq!(disabled.configured_account_id, "alerts");
    assert!(!disabled.enabled);
    assert_eq!(
        disabled.allowed_conversation_ids,
        vec!["group_base".to_owned()]
    );
    assert_eq!(disabled.allowed_sender_ids, vec!["user_base".to_owned()]);
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
fn wecom_resolve_account_for_session_account_id_matches_runtime_identity() {
    let config: WecomChannelConfig = serde_json::from_value(json!({
        "default_account": "Work Bot",
        "accounts": {
            "Work Bot": {
                "account_id": "wecom-shared",
                "bot_id": "bot_work",
                "secret": "secret-work",
                "acp": {
                    "bootstrap_mcp_servers": ["search"],
                    "working_directory": "/workspace/work-bot"
                }
            }
        }
    }))
    .expect("deserialize wecom config");

    let resolved = config
        .resolve_account_for_session_account_id(Some("wecom-shared"))
        .expect("resolve wecom runtime account identity");
    assert_eq!(resolved.configured_account_id, "work-bot");
    assert_eq!(resolved.account.id, "wecom-shared");
    assert_eq!(
        resolved.acp.bootstrap_mcp_servers,
        vec!["search".to_owned()]
    );
    assert_eq!(
        resolved.acp.resolved_working_directory(),
        Some(std::path::PathBuf::from("/workspace/work-bot"))
    );
}

#[test]
fn line_resolves_account_credentials_from_env_pointers() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.set("TEST_LINE_CHANNEL_ACCESS_TOKEN", "line-access-token");
    env.set("TEST_LINE_CHANNEL_SECRET", "line-channel-secret");

    let config_value = json!({
        "enabled": true,
        "account_id": "Line-Primary",
        "channel_access_token_env": "TEST_LINE_CHANNEL_ACCESS_TOKEN",
        "channel_secret_env": "TEST_LINE_CHANNEL_SECRET"
    });
    let config: LineChannelConfig =
        serde_json::from_value(config_value).expect("deserialize line config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default line account");
    let channel_access_token = resolved.channel_access_token();
    let channel_secret = resolved.channel_secret();

    assert_eq!(resolved.configured_account_id, "line-primary");
    assert_eq!(resolved.account.id, "line-primary");
    assert_eq!(resolved.account.label, "Line-Primary");
    assert_eq!(channel_access_token.as_deref(), Some("line-access-token"));
    assert_eq!(channel_secret.as_deref(), Some("line-channel-secret"));
    assert_eq!(
        resolved.resolved_api_base_url(),
        "https://api.line.me/v2/bot"
    );
}

#[test]
fn line_multi_account_resolution_merges_base_and_account_overrides() {
    let config_value = json!({
        "enabled": true,
        "account_id": "Line-Shared",
        "channel_access_token": "base-line-token",
        "default_account": "Marketing",
        "accounts": {
            "Marketing": {
                "account_id": "Line-Marketing",
                "channel_access_token": "marketing-line-token"
            },
            "Backup": {
                "enabled": false,
                "channel_secret": "backup-secret",
                "api_base_url": "https://line.example.test/v2/bot"
            }
        }
    });
    let config: LineChannelConfig =
        serde_json::from_value(config_value).expect("deserialize line multi-account config");

    assert_eq!(config.configured_account_ids(), vec!["backup", "marketing"]);
    assert_eq!(config.default_configured_account_id(), "marketing");

    let marketing = config
        .resolve_account(None)
        .expect("resolve default line account");
    let marketing_channel_access_token = marketing.channel_access_token();

    assert_eq!(marketing.configured_account_id, "marketing");
    assert_eq!(marketing.account.id, "line-marketing");
    assert_eq!(marketing.account.label, "Line-Marketing");
    assert_eq!(
        marketing_channel_access_token.as_deref(),
        Some("marketing-line-token")
    );
    assert_eq!(marketing.channel_secret(), None);
    assert_eq!(
        marketing.resolved_api_base_url(),
        "https://api.line.me/v2/bot"
    );

    let backup = config
        .resolve_account(Some("Backup"))
        .expect("resolve explicit line account");
    let backup_channel_access_token = backup.channel_access_token();
    let backup_channel_secret = backup.channel_secret();

    assert_eq!(backup.configured_account_id, "backup");
    assert!(!backup.enabled);
    assert_eq!(backup.account.id, "line-shared");
    assert_eq!(backup.account.label, "Line-Shared");
    assert_eq!(
        backup_channel_access_token.as_deref(),
        Some("base-line-token")
    );
    assert_eq!(backup_channel_secret.as_deref(), Some("backup-secret"));
    assert_eq!(
        backup.resolved_api_base_url(),
        "https://line.example.test/v2/bot"
    );
}

#[test]
fn dingtalk_resolves_webhook_url_and_secret_from_env_pointers() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.set(
        "TEST_DINGTALK_WEBHOOK_URL",
        "https://oapi.dingtalk.com/robot/send?access_token=test-token",
    );
    env.set("TEST_DINGTALK_SECRET", "ding-secret");

    let config_value = json!({
        "enabled": true,
        "account_id": "DingTalk-Primary",
        "webhook_url_env": "TEST_DINGTALK_WEBHOOK_URL",
        "secret_env": "TEST_DINGTALK_SECRET"
    });
    let config: DingtalkChannelConfig =
        serde_json::from_value(config_value).expect("deserialize dingtalk config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default dingtalk account");
    let webhook_url = resolved.webhook_url();
    let secret = resolved.secret();

    assert_eq!(resolved.configured_account_id, "dingtalk-primary");
    assert_eq!(resolved.account.id, "dingtalk-primary");
    assert_eq!(resolved.account.label, "DingTalk-Primary");
    assert_eq!(
        webhook_url.as_deref(),
        Some("https://oapi.dingtalk.com/robot/send?access_token=test-token")
    );
    assert_eq!(secret.as_deref(), Some("ding-secret"));
}

#[test]
fn dingtalk_multi_account_resolution_merges_base_and_account_overrides() {
    let config_value = json!({
        "enabled": true,
        "account_id": "DingTalk-Shared",
        "webhook_url": "https://oapi.dingtalk.com/robot/send?access_token=base-token",
        "secret": "base-secret",
        "default_account": "Ops",
        "accounts": {
            "Ops": {
                "account_id": "DingTalk-Ops",
                "webhook_url": "https://oapi.dingtalk.com/robot/send?access_token=ops-token"
            },
            "Backup": {
                "enabled": false,
                "secret": "backup-secret"
            }
        }
    });
    let config: DingtalkChannelConfig =
        serde_json::from_value(config_value).expect("deserialize dingtalk multi-account config");

    assert_eq!(config.configured_account_ids(), vec!["backup", "ops"]);
    assert_eq!(config.default_configured_account_id(), "ops");

    let ops = config
        .resolve_account(None)
        .expect("resolve default dingtalk account");
    let ops_webhook_url = ops.webhook_url();
    let ops_secret = ops.secret();

    assert_eq!(ops.configured_account_id, "ops");
    assert_eq!(ops.account.id, "dingtalk-ops");
    assert_eq!(ops.account.label, "DingTalk-Ops");
    assert_eq!(
        ops_webhook_url.as_deref(),
        Some("https://oapi.dingtalk.com/robot/send?access_token=ops-token")
    );
    assert_eq!(ops_secret.as_deref(), Some("base-secret"));

    let backup = config
        .resolve_account(Some("Backup"))
        .expect("resolve explicit dingtalk account");
    let backup_webhook_url = backup.webhook_url();
    let backup_secret = backup.secret();

    assert_eq!(backup.configured_account_id, "backup");
    assert!(!backup.enabled);
    assert_eq!(backup.account.id, "dingtalk-shared");
    assert_eq!(backup.account.label, "DingTalk-Shared");
    assert_eq!(
        backup_webhook_url.as_deref(),
        Some("https://oapi.dingtalk.com/robot/send?access_token=base-token")
    );
    assert_eq!(backup_secret.as_deref(), Some("backup-secret"));
}

#[test]
fn webhook_resolves_endpoint_and_secrets_from_env_pointers() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.set(
        "TEST_WEBHOOK_ENDPOINT_URL",
        "https://hooks.example.test/ingest?token=secret",
    );
    env.set("TEST_WEBHOOK_AUTH_TOKEN", "token-123");
    env.set("TEST_WEBHOOK_SIGNING_SECRET", "signing-secret-123");

    let config_value = json!({
        "enabled": true,
        "account_id": "Webhook-Ops",
        "endpoint_url_env": "TEST_WEBHOOK_ENDPOINT_URL",
        "auth_token_env": "TEST_WEBHOOK_AUTH_TOKEN",
        "signing_secret_env": "TEST_WEBHOOK_SIGNING_SECRET"
    });
    let config: WebhookChannelConfig =
        serde_json::from_value(config_value).expect("deserialize webhook config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default webhook account");
    let endpoint_url = resolved.endpoint_url();
    let auth_token = resolved.auth_token();
    let signing_secret = resolved.signing_secret();

    assert_eq!(resolved.configured_account_id, "webhook-ops");
    assert_eq!(resolved.account.id, "webhook-ops");
    assert_eq!(resolved.account.label, "Webhook-Ops");
    assert_eq!(
        endpoint_url.as_deref(),
        Some("https://hooks.example.test/ingest?token=secret")
    );
    assert_eq!(auth_token.as_deref(), Some("token-123"));
    assert_eq!(signing_secret.as_deref(), Some("signing-secret-123"));
    assert_eq!(resolved.auth_header_name, "Authorization");
    assert_eq!(resolved.auth_token_prefix, "Bearer ");
    assert_eq!(resolved.payload_format, WebhookPayloadFormat::JsonText);
    assert_eq!(resolved.payload_text_field, "text");
}

#[test]
fn webhook_multi_account_resolution_merges_base_and_account_overrides() {
    let config_value = json!({
        "enabled": true,
        "account_id": "Webhook-Shared",
        "endpoint_url": "https://hooks.example.test/base",
        "auth_token": "base-token",
        "auth_header_name": "X-Loong-Token",
        "auth_token_prefix": "Token ",
        "payload_format": "json_text",
        "payload_text_field": "message",
        "public_base_url": "https://public.example.test/webhook",
        "signing_secret": "base-signing-secret",
        "default_account": "Ops",
        "accounts": {
            "Ops": {
                "account_id": "Webhook-Ops",
                "endpoint_url": "https://hooks.example.test/ops",
                "payload_format": "plain_text"
            },
            "Backup": {
                "enabled": false,
                "auth_token": "backup-token",
                "auth_header_name": "X-Backup-Token",
                "payload_text_field": "backup_message"
            }
        }
    });
    let config: WebhookChannelConfig =
        serde_json::from_value(config_value).expect("deserialize webhook multi-account config");

    assert_eq!(config.configured_account_ids(), vec!["backup", "ops"]);
    assert_eq!(config.default_configured_account_id(), "ops");

    let ops = config
        .resolve_account(None)
        .expect("resolve default webhook account");
    let ops_endpoint_url = ops.endpoint_url();
    let ops_auth_token = ops.auth_token();
    let ops_signing_secret = ops.signing_secret();

    assert_eq!(ops.configured_account_id, "ops");
    assert_eq!(ops.account.id, "webhook-ops");
    assert_eq!(ops.account.label, "Webhook-Ops");
    assert_eq!(
        ops_endpoint_url.as_deref(),
        Some("https://hooks.example.test/ops")
    );
    assert_eq!(ops_auth_token.as_deref(), Some("base-token"));
    assert_eq!(ops_signing_secret.as_deref(), Some("base-signing-secret"));
    assert_eq!(ops.auth_header_name, "X-Loong-Token");
    assert_eq!(ops.auth_token_prefix, "Token ");
    assert_eq!(ops.payload_format, WebhookPayloadFormat::PlainText);
    assert_eq!(ops.payload_text_field, "message");
    assert_eq!(
        ops.public_base_url.as_deref(),
        Some("https://public.example.test/webhook")
    );

    let backup = config
        .resolve_account(Some("Backup"))
        .expect("resolve explicit webhook account");
    let backup_endpoint_url = backup.endpoint_url();
    let backup_auth_token = backup.auth_token();
    let backup_signing_secret = backup.signing_secret();

    assert_eq!(backup.configured_account_id, "backup");
    assert!(!backup.enabled);
    assert_eq!(backup.account.id, "webhook-shared");
    assert_eq!(backup.account.label, "Webhook-Shared");
    assert_eq!(
        backup_endpoint_url.as_deref(),
        Some("https://hooks.example.test/base")
    );
    assert_eq!(backup_auth_token.as_deref(), Some("backup-token"));
    assert_eq!(
        backup_signing_secret.as_deref(),
        Some("base-signing-secret")
    );
    assert_eq!(backup.auth_header_name, "X-Backup-Token");
    assert_eq!(backup.auth_token_prefix, "Token ");
    assert_eq!(backup.payload_format, WebhookPayloadFormat::JsonText);
    assert_eq!(backup.payload_text_field, "backup_message");
}

#[test]
fn webhook_account_without_env_overrides_inherits_top_level_env_names() {
    let config_value = json!({
        "enabled": true,
        "endpoint_url_env": "ACME_WEBHOOK_ENDPOINT",
        "auth_token_env": "ACME_WEBHOOK_AUTH_TOKEN",
        "signing_secret_env": "ACME_WEBHOOK_SIGNING_SECRET",
        "default_account": "Ops",
        "accounts": {
            "Ops": {}
        }
    });
    let config: WebhookChannelConfig =
        serde_json::from_value(config_value).expect("deserialize webhook multi-account config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default webhook account");

    assert_eq!(
        resolved.endpoint_url_env.as_deref(),
        Some("ACME_WEBHOOK_ENDPOINT")
    );
    assert_eq!(
        resolved.auth_token_env.as_deref(),
        Some("ACME_WEBHOOK_AUTH_TOKEN")
    );
    assert_eq!(
        resolved.signing_secret_env.as_deref(),
        Some("ACME_WEBHOOK_SIGNING_SECRET")
    );
}

#[test]
fn google_chat_resolves_webhook_url_from_env_pointers() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.set(
        "TEST_GOOGLE_CHAT_WEBHOOK_URL",
        "https://chat.googleapis.com/v1/spaces/AAAA/messages?key=test-key&token=test-token",
    );

    let config_value = json!({
        "enabled": true,
        "account_id": "Google-Chat-Primary",
        "webhook_url_env": "TEST_GOOGLE_CHAT_WEBHOOK_URL"
    });
    let config: GoogleChatChannelConfig =
        serde_json::from_value(config_value).expect("deserialize google chat config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default google chat account");
    let webhook_url = resolved.webhook_url();

    assert_eq!(resolved.configured_account_id, "google-chat-primary");
    assert_eq!(resolved.account.id, "google-chat-primary");
    assert_eq!(resolved.account.label, "Google-Chat-Primary");
    assert_eq!(
        webhook_url.as_deref(),
        Some("https://chat.googleapis.com/v1/spaces/AAAA/messages?key=test-key&token=test-token")
    );
}

#[test]
fn google_chat_multi_account_resolution_merges_base_and_account_overrides() {
    let config_value = json!({
        "enabled": true,
        "account_id": "Google-Chat-Shared",
        "webhook_url": "https://chat.googleapis.com/v1/spaces/AAAA/messages?key=base-key&token=base-token",
        "default_account": "Announcements",
        "accounts": {
            "Announcements": {
                "account_id": "Google-Chat-Announcements",
                "webhook_url": "https://chat.googleapis.com/v1/spaces/BBBB/messages?key=ann-key&token=ann-token"
            },
            "Backup": {
                "enabled": false
            }
        }
    });
    let config: GoogleChatChannelConfig =
        serde_json::from_value(config_value).expect("deserialize google chat multi-account config");

    assert_eq!(
        config.configured_account_ids(),
        vec!["announcements", "backup"]
    );
    assert_eq!(config.default_configured_account_id(), "announcements");

    let announcements = config
        .resolve_account(None)
        .expect("resolve default google chat account");
    let announcements_webhook_url = announcements.webhook_url();

    assert_eq!(announcements.configured_account_id, "announcements");
    assert_eq!(announcements.account.id, "google-chat-announcements");
    assert_eq!(announcements.account.label, "Google-Chat-Announcements");
    assert_eq!(
        announcements_webhook_url.as_deref(),
        Some("https://chat.googleapis.com/v1/spaces/BBBB/messages?key=ann-key&token=ann-token")
    );

    let backup = config
        .resolve_account(Some("Backup"))
        .expect("resolve explicit google chat account");
    let backup_webhook_url = backup.webhook_url();

    assert_eq!(backup.configured_account_id, "backup");
    assert!(!backup.enabled);
    assert_eq!(backup.account.id, "google-chat-shared");
    assert_eq!(backup.account.label, "Google-Chat-Shared");
    assert_eq!(
        backup_webhook_url.as_deref(),
        Some("https://chat.googleapis.com/v1/spaces/AAAA/messages?key=base-key&token=base-token")
    );
}

#[test]
fn nextcloud_talk_resolves_server_url_and_shared_secret_from_env_pointers() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.set(
        "TEST_NEXTCLOUD_TALK_SERVER_URL",
        "https://cloud.example.test",
    );
    env.set(
        "TEST_NEXTCLOUD_TALK_SHARED_SECRET",
        "nextcloud-shared-secret",
    );

    let config_value = json!({
        "enabled": true,
        "account_id": "Nextcloud-Primary",
        "server_url_env": "TEST_NEXTCLOUD_TALK_SERVER_URL",
        "shared_secret_env": "TEST_NEXTCLOUD_TALK_SHARED_SECRET"
    });
    let config: NextcloudTalkChannelConfig =
        serde_json::from_value(config_value).expect("deserialize nextcloud talk config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default nextcloud talk account");
    let server_url = resolved.server_url();
    let shared_secret = resolved.shared_secret();

    assert_eq!(resolved.configured_account_id, "nextcloud-primary");
    assert_eq!(resolved.account.id, "nextcloud-primary");
    assert_eq!(resolved.account.label, "Nextcloud-Primary");
    assert_eq!(server_url.as_deref(), Some("https://cloud.example.test"));
    assert_eq!(shared_secret.as_deref(), Some("nextcloud-shared-secret"));
}

#[test]
fn nextcloud_talk_multi_account_resolution_merges_base_and_account_overrides() {
    let config_value = json!({
        "enabled": true,
        "account_id": "Nextcloud-Shared",
        "server_url": "https://cloud.example.test",
        "shared_secret": "base-shared-secret",
        "default_account": "Ops",
        "accounts": {
            "Ops": {
                "account_id": "Nextcloud-Ops",
                "server_url": "https://ops.example.test"
            },
            "Backup": {
                "enabled": false,
                "shared_secret": "backup-shared-secret"
            }
        }
    });
    let config: NextcloudTalkChannelConfig = serde_json::from_value(config_value)
        .expect("deserialize nextcloud talk multi-account config");

    assert_eq!(config.configured_account_ids(), vec!["backup", "ops"]);
    assert_eq!(config.default_configured_account_id(), "ops");

    let ops = config
        .resolve_account(None)
        .expect("resolve default nextcloud talk account");
    let ops_server_url = ops.server_url();
    let ops_shared_secret = ops.shared_secret();

    assert_eq!(ops.configured_account_id, "ops");
    assert_eq!(ops.account.id, "nextcloud-ops");
    assert_eq!(ops.account.label, "Nextcloud-Ops");
    assert_eq!(ops_server_url.as_deref(), Some("https://ops.example.test"));
    assert_eq!(ops_shared_secret.as_deref(), Some("base-shared-secret"));

    let backup = config
        .resolve_account(Some("Backup"))
        .expect("resolve explicit nextcloud talk account");
    let backup_server_url = backup.server_url();
    let backup_shared_secret = backup.shared_secret();

    assert_eq!(backup.configured_account_id, "backup");
    assert!(!backup.enabled);
    assert_eq!(backup.account.id, "nextcloud-shared");
    assert_eq!(backup.account.label, "Nextcloud-Shared");
    assert_eq!(
        backup_server_url.as_deref(),
        Some("https://cloud.example.test")
    );
    assert_eq!(
        backup_shared_secret.as_deref(),
        Some("backup-shared-secret")
    );
}

#[test]
fn synology_chat_resolves_token_and_incoming_url_from_env_pointers() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.set("TEST_SYNOLOGY_CHAT_TOKEN", "synology-outgoing-token");
    env.set(
            "TEST_SYNOLOGY_CHAT_INCOMING_URL",
            "https://chat.example.test/webapi/entry.cgi?api=SYNO.Chat.External&method=incoming&version=2&token=secret-token",
        );

    let config_value = json!({
        "enabled": true,
        "account_id": "Synology-Ops",
        "token_env": "TEST_SYNOLOGY_CHAT_TOKEN",
        "incoming_url_env": "TEST_SYNOLOGY_CHAT_INCOMING_URL",
        "allowed_user_ids": [42]
    });
    let config: SynologyChatChannelConfig =
        serde_json::from_value(config_value).expect("deserialize synology chat config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default synology chat account");
    let token = resolved.token();
    let incoming_url = resolved.incoming_url();

    assert_eq!(resolved.configured_account_id, "synology-ops");
    assert_eq!(resolved.account.id, "synology-ops");
    assert_eq!(resolved.account.label, "Synology-Ops");
    assert_eq!(token.as_deref(), Some("synology-outgoing-token"));
    assert_eq!(
        incoming_url.as_deref(),
        Some(
            "https://chat.example.test/webapi/entry.cgi?api=SYNO.Chat.External&method=incoming&version=2&token=secret-token"
        )
    );
    assert_eq!(resolved.allowed_user_ids, vec![42]);
}

#[test]
fn synology_chat_multi_account_resolution_merges_base_and_account_overrides() {
    let config_value = json!({
        "enabled": true,
        "account_id": "Synology-Shared",
        "token": "base-synology-token",
        "incoming_url": "https://chat.example.test/webapi/entry.cgi?token=base-token",
        "allowed_user_ids": [1, 2],
        "default_account": "Ops",
        "accounts": {
            "Ops": {
                "account_id": "Synology-Ops",
                "incoming_url": "https://ops.example.test/webapi/entry.cgi?token=ops-token"
            },
            "Backup": {
                "enabled": false,
                "token": "backup-synology-token",
                "allowed_user_ids": [9]
            }
        }
    });
    let config: SynologyChatChannelConfig = serde_json::from_value(config_value)
        .expect("deserialize synology chat multi-account config");

    assert_eq!(config.configured_account_ids(), vec!["backup", "ops"]);
    assert_eq!(config.default_configured_account_id(), "ops");

    let ops = config
        .resolve_account(None)
        .expect("resolve default synology chat account");
    let ops_token = ops.token();
    let ops_incoming_url = ops.incoming_url();

    assert_eq!(ops.configured_account_id, "ops");
    assert_eq!(ops.account.id, "synology-ops");
    assert_eq!(ops.account.label, "Synology-Ops");
    assert_eq!(ops_token.as_deref(), Some("base-synology-token"));
    assert_eq!(
        ops_incoming_url.as_deref(),
        Some("https://ops.example.test/webapi/entry.cgi?token=ops-token")
    );
    assert_eq!(ops.allowed_user_ids, vec![1, 2]);

    let backup = config
        .resolve_account(Some("Backup"))
        .expect("resolve explicit synology chat account");
    let backup_token = backup.token();
    let backup_incoming_url = backup.incoming_url();

    assert_eq!(backup.configured_account_id, "backup");
    assert!(!backup.enabled);
    assert_eq!(backup.account.id, "synology-shared");
    assert_eq!(backup.account.label, "Synology-Shared");
    assert_eq!(backup_token.as_deref(), Some("backup-synology-token"));
    assert_eq!(
        backup_incoming_url.as_deref(),
        Some("https://chat.example.test/webapi/entry.cgi?token=base-token")
    );
    assert_eq!(backup.allowed_user_ids, vec![9]);
}

#[test]
fn teams_resolves_webhook_and_future_serve_credentials_from_env_pointers() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.set(
        "TEST_TEAMS_WEBHOOK_URL",
        "https://teams.example.test/webhook/connector",
    );
    env.set("TEST_TEAMS_APP_ID", "teams-app-id");
    env.set("TEST_TEAMS_APP_PASSWORD", "teams-app-password");
    env.set("TEST_TEAMS_TENANT_ID", "teams-tenant-id");

    let config_value = json!({
        "enabled": true,
        "account_id": "Teams-Ops",
        "webhook_url_env": "TEST_TEAMS_WEBHOOK_URL",
        "app_id_env": "TEST_TEAMS_APP_ID",
        "app_password_env": "TEST_TEAMS_APP_PASSWORD",
        "tenant_id_env": "TEST_TEAMS_TENANT_ID",
        "allowed_conversation_ids": ["19:ops-thread"]
    });
    let config: TeamsChannelConfig =
        serde_json::from_value(config_value).expect("deserialize teams config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default teams account");
    let webhook_url = resolved.webhook_url();
    let app_id = resolved.app_id();
    let app_password = resolved.app_password();
    let tenant_id = resolved.tenant_id();

    assert_eq!(resolved.configured_account_id, "teams-ops");
    assert_eq!(resolved.account.id, "teams-ops");
    assert_eq!(resolved.account.label, "Teams-Ops");
    assert_eq!(
        webhook_url.as_deref(),
        Some("https://teams.example.test/webhook/connector")
    );
    assert_eq!(app_id.as_deref(), Some("teams-app-id"));
    assert_eq!(app_password.as_deref(), Some("teams-app-password"));
    assert_eq!(tenant_id.as_deref(), Some("teams-tenant-id"));
    assert_eq!(
        resolved.allowed_conversation_ids,
        vec!["19:ops-thread".to_owned()]
    );
}

#[test]
fn teams_multi_account_resolution_merges_send_and_future_serve_overrides() {
    let config_value = json!({
        "enabled": true,
        "account_id": "Teams-Shared",
        "webhook_url": "https://teams.example.test/webhook/base",
        "app_id": "base-app-id",
        "app_password": "base-app-password",
        "tenant_id": "base-tenant-id",
        "allowed_conversation_ids": ["19:base-thread"],
        "default_account": "Ops",
        "accounts": {
            "Ops": {
                "account_id": "Teams-Ops",
                "webhook_url": "https://teams.example.test/webhook/ops"
            },
            "Backup": {
                "enabled": false,
                "app_password": "backup-app-password",
                "allowed_conversation_ids": ["19:backup-thread"]
            }
        }
    });
    let config: TeamsChannelConfig =
        serde_json::from_value(config_value).expect("deserialize teams multi-account config");

    assert_eq!(config.configured_account_ids(), vec!["backup", "ops"]);
    assert_eq!(config.default_configured_account_id(), "ops");

    let ops = config
        .resolve_account(None)
        .expect("resolve default teams account");
    let ops_webhook_url = ops.webhook_url();
    let ops_app_id = ops.app_id();
    let ops_app_password = ops.app_password();
    let ops_tenant_id = ops.tenant_id();

    assert_eq!(ops.configured_account_id, "ops");
    assert_eq!(ops.account.id, "teams-ops");
    assert_eq!(ops.account.label, "Teams-Ops");
    assert_eq!(
        ops_webhook_url.as_deref(),
        Some("https://teams.example.test/webhook/ops")
    );
    assert_eq!(ops_app_id.as_deref(), Some("base-app-id"));
    assert_eq!(ops_app_password.as_deref(), Some("base-app-password"));
    assert_eq!(ops_tenant_id.as_deref(), Some("base-tenant-id"));
    assert_eq!(
        ops.allowed_conversation_ids,
        vec!["19:base-thread".to_owned()]
    );

    let backup = config
        .resolve_account(Some("Backup"))
        .expect("resolve explicit teams account");
    let backup_webhook_url = backup.webhook_url();
    let backup_app_id = backup.app_id();
    let backup_app_password = backup.app_password();
    let backup_tenant_id = backup.tenant_id();

    assert_eq!(backup.configured_account_id, "backup");
    assert!(!backup.enabled);
    assert_eq!(backup.account.id, "teams-shared");
    assert_eq!(backup.account.label, "Teams-Shared");
    assert_eq!(
        backup_webhook_url.as_deref(),
        Some("https://teams.example.test/webhook/base")
    );
    assert_eq!(backup_app_id.as_deref(), Some("base-app-id"));
    assert_eq!(backup_app_password.as_deref(), Some("backup-app-password"));
    assert_eq!(backup_tenant_id.as_deref(), Some("base-tenant-id"));
    assert_eq!(
        backup.allowed_conversation_ids,
        vec!["19:backup-thread".to_owned()]
    );
}

#[test]
fn imessage_resolves_bridge_url_and_token_from_env_pointers() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.set(
        "TEST_IMESSAGE_BRIDGE_URL",
        "https://bluebubbles.example.test/base",
    );
    env.set("TEST_IMESSAGE_BRIDGE_TOKEN", "bluebubbles-password");

    let config_value = json!({
        "enabled": true,
        "account_id": "BlueBubbles-Ops",
        "bridge_url_env": "TEST_IMESSAGE_BRIDGE_URL",
        "bridge_token_env": "TEST_IMESSAGE_BRIDGE_TOKEN",
        "allowed_chat_ids": ["iMessage;-;+15550001111"]
    });
    let config: ImessageChannelConfig =
        serde_json::from_value(config_value).expect("deserialize imessage config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default imessage account");
    let bridge_url = resolved.bridge_url();
    let bridge_token = resolved.bridge_token();

    assert_eq!(resolved.configured_account_id, "bluebubbles-ops");
    assert_eq!(resolved.account.id, "bluebubbles-ops");
    assert_eq!(resolved.account.label, "BlueBubbles-Ops");
    assert_eq!(
        bridge_url.as_deref(),
        Some("https://bluebubbles.example.test/base")
    );
    assert_eq!(bridge_token.as_deref(), Some("bluebubbles-password"));
    assert_eq!(
        resolved.allowed_chat_ids,
        vec!["iMessage;-;+15550001111".to_owned()]
    );
}

#[test]
fn imessage_multi_account_resolution_merges_base_and_account_overrides() {
    let config_value = json!({
        "enabled": true,
        "account_id": "BlueBubbles-Shared",
        "bridge_url": "https://bluebubbles.example.test/base",
        "bridge_token": "base-bridge-token",
        "allowed_chat_ids": ["iMessage;-;+15550001111"],
        "default_account": "Ops",
        "accounts": {
            "Ops": {
                "account_id": "BlueBubbles-Ops",
                "bridge_url": "https://bluebubbles.example.test/ops"
            },
            "Backup": {
                "enabled": false,
                "bridge_token": "backup-bridge-token",
                "allowed_chat_ids": ["iMessage;-;+15550002222"]
            }
        }
    });
    let config: ImessageChannelConfig =
        serde_json::from_value(config_value).expect("deserialize imessage multi-account config");

    assert_eq!(config.configured_account_ids(), vec!["backup", "ops"]);
    assert_eq!(config.default_configured_account_id(), "ops");

    let ops = config
        .resolve_account(None)
        .expect("resolve default imessage account");
    let ops_bridge_url = ops.bridge_url();
    let ops_bridge_token = ops.bridge_token();

    assert_eq!(ops.configured_account_id, "ops");
    assert_eq!(ops.account.id, "bluebubbles-ops");
    assert_eq!(ops.account.label, "BlueBubbles-Ops");
    assert_eq!(
        ops_bridge_url.as_deref(),
        Some("https://bluebubbles.example.test/ops")
    );
    assert_eq!(ops_bridge_token.as_deref(), Some("base-bridge-token"));
    assert_eq!(
        ops.allowed_chat_ids,
        vec!["iMessage;-;+15550001111".to_owned()]
    );

    let backup = config
        .resolve_account(Some("Backup"))
        .expect("resolve explicit imessage account");
    let backup_bridge_url = backup.bridge_url();
    let backup_bridge_token = backup.bridge_token();

    assert_eq!(backup.configured_account_id, "backup");
    assert!(!backup.enabled);
    assert_eq!(backup.account.id, "bluebubbles-shared");
    assert_eq!(backup.account.label, "BlueBubbles-Shared");
    assert_eq!(
        backup_bridge_url.as_deref(),
        Some("https://bluebubbles.example.test/base")
    );
    assert_eq!(backup_bridge_token.as_deref(), Some("backup-bridge-token"));
    assert_eq!(
        backup.allowed_chat_ids,
        vec!["iMessage;-;+15550002222".to_owned()]
    );
}

#[test]
fn mattermost_resolves_server_url_and_bot_token_from_env_pointers() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.set(
        "TEST_MATTERMOST_SERVER_URL",
        "https://mattermost.example.test",
    );
    env.set("TEST_MATTERMOST_BOT_TOKEN", "mattermost-token");

    let config_value = json!({
        "enabled": true,
        "account_id": "Mattermost-Ops",
        "server_url_env": "TEST_MATTERMOST_SERVER_URL",
        "bot_token_env": "TEST_MATTERMOST_BOT_TOKEN"
    });
    let config: MattermostChannelConfig =
        serde_json::from_value(config_value).expect("deserialize mattermost config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default mattermost account");
    let server_url = resolved.server_url();
    let bot_token = resolved.bot_token();

    assert_eq!(resolved.configured_account_id, "mattermost-ops");
    assert_eq!(resolved.account.id, "mattermost-ops");
    assert_eq!(resolved.account.label, "Mattermost-Ops");
    assert_eq!(
        server_url.as_deref(),
        Some("https://mattermost.example.test")
    );
    assert_eq!(bot_token.as_deref(), Some("mattermost-token"));
}

#[test]
fn mattermost_multi_account_resolution_merges_base_and_account_overrides() {
    let config_value = json!({
        "enabled": true,
        "account_id": "Mattermost-Shared",
        "server_url": "https://mattermost.example.test",
        "bot_token": "base-mattermost-token",
        "default_account": "Ops",
        "accounts": {
            "Ops": {
                "account_id": "Mattermost-Ops",
                "bot_token": "ops-mattermost-token"
            },
            "Backup": {
                "enabled": false,
                "server_url": "https://backup-mattermost.example.test"
            }
        }
    });
    let config: MattermostChannelConfig =
        serde_json::from_value(config_value).expect("deserialize mattermost multi-account config");

    assert_eq!(config.configured_account_ids(), vec!["backup", "ops"]);
    assert_eq!(config.default_configured_account_id(), "ops");

    let ops = config
        .resolve_account(None)
        .expect("resolve default mattermost account");
    let ops_server_url = ops.server_url();
    let ops_bot_token = ops.bot_token();

    assert_eq!(ops.configured_account_id, "ops");
    assert_eq!(ops.account.id, "mattermost-ops");
    assert_eq!(ops.account.label, "Mattermost-Ops");
    assert_eq!(
        ops_server_url.as_deref(),
        Some("https://mattermost.example.test")
    );
    assert_eq!(ops_bot_token.as_deref(), Some("ops-mattermost-token"));

    let backup = config
        .resolve_account(Some("Backup"))
        .expect("resolve explicit mattermost account");
    let backup_server_url = backup.server_url();
    let backup_bot_token = backup.bot_token();

    assert_eq!(backup.configured_account_id, "backup");
    assert!(!backup.enabled);
    assert_eq!(backup.account.id, "mattermost-shared");
    assert_eq!(backup.account.label, "Mattermost-Shared");
    assert_eq!(
        backup_server_url.as_deref(),
        Some("https://backup-mattermost.example.test")
    );
    assert_eq!(backup_bot_token.as_deref(), Some("base-mattermost-token"));
}

#[test]
fn signal_resolves_account_and_service_url_from_env_pointers() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.set("TEST_SIGNAL_ACCOUNT", "+15550001111");
    env.set("TEST_SIGNAL_SERVICE_URL", "http://signal.example.test:8080");

    let config_value = json!({
        "enabled": true,
        "account_env": "TEST_SIGNAL_ACCOUNT",
        "service_url_env": "TEST_SIGNAL_SERVICE_URL"
    });
    let config: SignalChannelConfig =
        serde_json::from_value(config_value).expect("deserialize signal config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default signal account");
    let signal_account = resolved.signal_account();
    let service_url = resolved.service_url();

    assert_eq!(resolved.configured_account_id, "signal_15550001111");
    assert_eq!(resolved.account.id, "signal_15550001111");
    assert_eq!(resolved.account.label, "signal:+15550001111");
    assert_eq!(signal_account.as_deref(), Some("+15550001111"));
    assert_eq!(
        service_url.as_deref(),
        Some("http://signal.example.test:8080")
    );
}

#[test]
fn imessage_partial_deserialization_keeps_default_env_pointers() {
    let config: ImessageChannelConfig = serde_json::from_value(json!({
        "enabled": true
    }))
    .expect("deserialize imessage config");

    assert_eq!(
        config.bridge_url_env.as_deref(),
        Some(IMESSAGE_BRIDGE_URL_ENV)
    );
    assert_eq!(
        config.bridge_token_env.as_deref(),
        Some(IMESSAGE_BRIDGE_TOKEN_ENV)
    );
}

#[test]
fn email_partial_deserialization_keeps_default_env_pointers() {
    let config: EmailChannelConfig = serde_json::from_value(json!({
        "enabled": true
    }))
    .expect("deserialize email config");

    assert_eq!(
        config.smtp_username_env.as_deref(),
        Some(EMAIL_SMTP_USERNAME_ENV)
    );
    assert_eq!(
        config.smtp_password_env.as_deref(),
        Some(EMAIL_SMTP_PASSWORD_ENV)
    );
    assert_eq!(
        config.imap_username_env.as_deref(),
        Some(EMAIL_IMAP_USERNAME_ENV)
    );
    assert_eq!(
        config.imap_password_env.as_deref(),
        Some(EMAIL_IMAP_PASSWORD_ENV)
    );
}

#[test]
fn parse_email_smtp_endpoint_accepts_relay_host() {
    let endpoint = parse_email_smtp_endpoint("smtp.example.test").expect("relay host should parse");

    assert_eq!(
        endpoint,
        EmailSmtpEndpoint::RelayHost("smtp.example.test".to_owned())
    );
}

#[test]
fn parse_email_smtp_endpoint_accepts_connection_url() {
    let endpoint =
        parse_email_smtp_endpoint("smtps://smtp.example.test:465").expect("smtp url should parse");

    assert_eq!(
        endpoint,
        EmailSmtpEndpoint::ConnectionUrl("smtps://smtp.example.test:465".to_owned())
    );
}

#[test]
fn parse_email_smtp_endpoint_rejects_host_port_without_scheme() {
    let error = parse_email_smtp_endpoint("smtp.example.test:587")
        .expect_err("bare host:port should be rejected");

    assert_eq!(
        error,
        "email smtp_host with an explicit port must use a full smtp:// or smtps:// URL"
    );
}

#[test]
fn signal_partial_deserialization_keeps_default_env_pointers() {
    let config: SignalChannelConfig = serde_json::from_value(json!({
        "enabled": true
    }))
    .expect("deserialize signal config");

    assert_eq!(
        config.signal_account_env.as_deref(),
        Some(SIGNAL_ACCOUNT_ENV)
    );
    assert_eq!(
        config.service_url_env.as_deref(),
        Some(SIGNAL_SERVICE_URL_ENV)
    );
}

#[test]
fn whatsapp_partial_deserialization_keeps_default_env_pointers() {
    let config: WhatsappChannelConfig = serde_json::from_value(json!({
        "enabled": true
    }))
    .expect("deserialize whatsapp config");

    assert_eq!(
        config.access_token_env.as_deref(),
        Some(WHATSAPP_ACCESS_TOKEN_ENV)
    );
    assert_eq!(
        config.phone_number_id_env.as_deref(),
        Some(WHATSAPP_PHONE_NUMBER_ID_ENV)
    );
    assert_eq!(
        config.verify_token_env.as_deref(),
        Some(WHATSAPP_VERIFY_TOKEN_ENV)
    );
    assert_eq!(
        config.app_secret_env.as_deref(),
        Some(WHATSAPP_APP_SECRET_ENV)
    );
}

#[test]
fn signal_default_service_url_env_override_wins_over_fallback() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.set("SIGNAL_SERVICE_URL", "http://signal.override.test:8080");

    let config = SignalChannelConfig::default();
    let service_url = config.service_url();

    assert_eq!(
        service_url.as_deref(),
        Some("http://signal.override.test:8080")
    );
}

#[test]
fn signal_multi_account_resolution_merges_base_and_account_overrides() {
    let config_value = json!({
        "enabled": true,
        "account": "+15550001111",
        "service_url": "http://127.0.0.1:8080",
        "default_account": "Alerts",
        "accounts": {
            "Alerts": {
                "account_id": "Signal-Alerts",
                "account": "+15550002222"
            },
            "Backup": {
                "enabled": false,
                "service_url": "http://backup.example.test:8080"
            }
        }
    });
    let config: SignalChannelConfig =
        serde_json::from_value(config_value).expect("deserialize signal multi-account config");

    assert_eq!(config.configured_account_ids(), vec!["alerts", "backup"]);
    assert_eq!(config.default_configured_account_id(), "alerts");

    let alerts = config
        .resolve_account(None)
        .expect("resolve default signal account");
    let alerts_signal_account = alerts.signal_account();
    let alerts_service_url = alerts.service_url();

    assert_eq!(alerts.configured_account_id, "alerts");
    assert_eq!(alerts.account.id, "signal-alerts");
    assert_eq!(alerts.account.label, "Signal-Alerts");
    assert_eq!(alerts_signal_account.as_deref(), Some("+15550002222"));
    assert_eq!(alerts_service_url.as_deref(), Some("http://127.0.0.1:8080"));

    let backup = config
        .resolve_account(Some("Backup"))
        .expect("resolve explicit signal account");
    let backup_signal_account = backup.signal_account();
    let backup_service_url = backup.service_url();

    assert_eq!(backup.configured_account_id, "backup");
    assert!(!backup.enabled);
    assert_eq!(backup_signal_account.as_deref(), Some("+15550001111"));
    assert_eq!(
        backup_service_url.as_deref(),
        Some("http://backup.example.test:8080")
    );
}

#[test]
fn whatsapp_resolves_phone_number_id_from_env_pointer() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.set("TEST_WHATSAPP_PHONE_NUMBER_ID", "1234567890");

    let config_value = json!({
        "enabled": true,
        "access_token": "whatsapp-token",
        "phone_number_id_env": "TEST_WHATSAPP_PHONE_NUMBER_ID"
    });
    let config: WhatsappChannelConfig =
        serde_json::from_value(config_value).expect("deserialize whatsapp config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default whatsapp account");
    let phone_number_id = resolved.phone_number_id();

    assert_eq!(resolved.configured_account_id, "whatsapp_1234567890");
    assert_eq!(resolved.account.id, "whatsapp_1234567890");
    assert_eq!(resolved.account.label, "whatsapp:1234567890");
    assert_eq!(phone_number_id.as_deref(), Some("1234567890"));
}

#[test]
fn whatsapp_multi_account_resolution_merges_base_and_account_overrides() {
    let config_value = json!({
        "enabled": true,
        "access_token": "base-access-token",
        "api_base_url": "https://graph.facebook.com/v25.0",
        "default_account": "Business",
        "accounts": {
            "Business": {
                "account_id": "WhatsApp-Biz",
                "phone_number_id": "1111111111"
            },
            "Backup": {
                "enabled": false,
                "phone_number_id": "2222222222",
                "api_base_url": "https://graph.facebook.com/v26.0"
            }
        }
    });
    let config: WhatsappChannelConfig =
        serde_json::from_value(config_value).expect("deserialize whatsapp multi-account config");

    assert_eq!(config.configured_account_ids(), vec!["backup", "business"]);
    assert_eq!(config.default_configured_account_id(), "business");

    let business = config
        .resolve_account(None)
        .expect("resolve default whatsapp account");
    let business_access_token = business.access_token();
    let business_phone_number_id = business.phone_number_id();

    assert_eq!(business.configured_account_id, "business");
    assert_eq!(business.account.id, "whatsapp-biz");
    assert_eq!(business.account.label, "WhatsApp-Biz");
    assert_eq!(business_access_token.as_deref(), Some("base-access-token"));
    assert_eq!(business_phone_number_id.as_deref(), Some("1111111111"));
    assert_eq!(
        business.resolved_api_base_url(),
        "https://graph.facebook.com/v25.0"
    );

    let backup = config
        .resolve_account(Some("Backup"))
        .expect("resolve explicit whatsapp account");
    let backup_access_token = backup.access_token();
    let backup_phone_number_id = backup.phone_number_id();

    assert_eq!(backup.configured_account_id, "backup");
    assert!(!backup.enabled);
    assert_eq!(backup_access_token.as_deref(), Some("base-access-token"));
    assert_eq!(backup_phone_number_id.as_deref(), Some("2222222222"));
    assert_eq!(
        backup.resolved_api_base_url(),
        "https://graph.facebook.com/v26.0"
    );
}
