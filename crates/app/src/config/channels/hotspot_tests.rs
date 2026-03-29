use super::*;
use serde_json::json;

#[test]
fn tlon_partial_deserialization_keeps_default_env_pointers() {
    let config: TlonChannelConfig = serde_json::from_value(json!({
        "enabled": true
    }))
    .expect("deserialize tlon config");

    assert_eq!(config.ship_env.as_deref(), Some(TLON_SHIP_ENV));
    assert_eq!(config.url_env.as_deref(), Some(TLON_URL_ENV));
    assert_eq!(config.code_env.as_deref(), Some(TLON_CODE_ENV));
}

#[test]
fn tlon_resolves_credentials_from_env_pointers() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.set("TEST_TLON_SHIP", "~zod");
    env.set("TEST_TLON_URL", "ship.example.test");
    env.set("TEST_TLON_CODE", "lidlut-tabwed-pillex-ridrup");

    let config: TlonChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "ship_env": "TEST_TLON_SHIP",
        "url_env": "TEST_TLON_URL",
        "code_env": "TEST_TLON_CODE"
    }))
    .expect("deserialize tlon config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default tlon account");
    let ship = resolved.ship();
    let url = resolved.url();
    let code = resolved.code();

    assert_eq!(ship.as_deref(), Some("~zod"));
    assert_eq!(url.as_deref(), Some("ship.example.test"));
    assert_eq!(code.as_deref(), Some("lidlut-tabwed-pillex-ridrup"));
}

#[test]
fn tlon_multi_account_resolution_merges_base_and_account_overrides() {
    let config: TlonChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "ship": "~zod",
        "url": "ship.example.test",
        "code": "base-code",
        "default_account": "Primary",
        "accounts": {
            "Primary": {
                "account_id": "Tlon-Ops",
                "code": "primary-code"
            },
            "Backup": {
                "enabled": false,
                "ship": "~bus"
            }
        }
    }))
    .expect("deserialize tlon multi-account config");

    assert_eq!(config.configured_account_ids(), vec!["backup", "primary"]);
    assert_eq!(config.default_configured_account_id(), "primary");

    let primary = config
        .resolve_account(None)
        .expect("resolve default tlon account");
    let primary_ship = primary.ship();
    let primary_url = primary.url();
    let primary_code = primary.code();

    assert_eq!(primary.configured_account_id, "primary");
    assert_eq!(primary.account.id, "tlon-ops");
    assert_eq!(primary.account.label, "Tlon-Ops");
    assert_eq!(primary_ship.as_deref(), Some("~zod"));
    assert_eq!(primary_url.as_deref(), Some("ship.example.test"));
    assert_eq!(primary_code.as_deref(), Some("primary-code"));

    let backup = config
        .resolve_account(Some("Backup"))
        .expect("resolve explicit tlon account");
    let backup_ship = backup.ship();
    let backup_url = backup.url();
    let backup_code = backup.code();

    assert_eq!(backup.configured_account_id, "backup");
    assert!(!backup.enabled);
    assert_eq!(backup.account.id, "tlon_bus");
    assert_eq!(backup.account.label, "ship:~bus");
    assert_eq!(backup_ship.as_deref(), Some("~bus"));
    assert_eq!(backup_url.as_deref(), Some("ship.example.test"));
    assert_eq!(backup_code.as_deref(), Some("base-code"));
}

#[test]
fn tlon_account_without_explicit_env_override_inherits_top_level_env_pointers() {
    let mut env = crate::test_support::ScopedEnv::new();
    env.set("TEST_TLON_BASE_SHIP", "~zod");
    env.set("TEST_TLON_BASE_URL", "ship.example.test");
    env.set("TEST_TLON_BASE_CODE", "lidlut-tabwed-pillex-ridrup");

    let config: TlonChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "ship_env": "TEST_TLON_BASE_SHIP",
        "url_env": "TEST_TLON_BASE_URL",
        "code_env": "TEST_TLON_BASE_CODE",
        "default_account": "Primary",
        "accounts": {
            "Primary": {
                "account_id": "Tlon-Ops"
            }
        }
    }))
    .expect("deserialize tlon config");

    let resolved = config
        .resolve_account(None)
        .expect("resolve default tlon account");
    let ship = resolved.ship();
    let url = resolved.url();
    let code = resolved.code();

    assert_eq!(resolved.configured_account_id, "primary");
    assert_eq!(ship.as_deref(), Some("~zod"));
    assert_eq!(url.as_deref(), Some("ship.example.test"));
    assert_eq!(code.as_deref(), Some("lidlut-tabwed-pillex-ridrup"));
}

#[test]
fn telegram_streaming_mode_deserializes_from_json() {
    let off: TelegramStreamingMode = serde_json::from_str("\"off\"").expect("deserialize off");
    assert_eq!(off, TelegramStreamingMode::Off);

    let draft: TelegramStreamingMode =
        serde_json::from_str("\"draft\"").expect("deserialize draft");
    assert_eq!(draft, TelegramStreamingMode::Draft);
}

#[test]
fn telegram_streaming_mode_default_is_off() {
    let config: TelegramChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "bot_token_env": "TEST_TOKEN"
    }))
    .expect("deserialize telegram config");
    assert_eq!(config.streaming_mode, TelegramStreamingMode::Off);
}

#[test]
fn telegram_streaming_mode_inherited_from_base_in_multi_account() {
    let config: TelegramChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "bot_token_env": "BASE_TOKEN",
        "streaming_mode": "draft",
        "accounts": {
            "Account1": {
                "bot_token_env": "ACCOUNT1_TOKEN"
            }
        }
    }))
    .expect("deserialize telegram config");

    let resolved = config
        .resolve_account(Some("Account1"))
        .expect("resolve account1");
    assert_eq!(resolved.streaming_mode, TelegramStreamingMode::Draft);
}

#[test]
fn telegram_streaming_mode_overridden_per_account() {
    let config: TelegramChannelConfig = serde_json::from_value(json!({
        "enabled": true,
        "bot_token_env": "BASE_TOKEN",
        "streaming_mode": "draft",
        "accounts": {
            "Account1": {
                "streaming_mode": "off",
                "bot_token_env": "ACCOUNT1_TOKEN"
            }
        }
    }))
    .expect("deserialize telegram config");

    let resolved = config
        .resolve_account(Some("Account1"))
        .expect("resolve account1");
    assert_eq!(resolved.streaming_mode, TelegramStreamingMode::Off);
}
