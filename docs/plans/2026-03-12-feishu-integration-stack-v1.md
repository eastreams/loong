# Feishu Integration Stack V1 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Upgrade LoongClaw from a Feishu encrypted-webhook adapter into a
principal-aware Feishu integration stack with user OAuth, grant persistence,
read-only Docs/Messages/Calendar access, and operator-facing CLI/doctor
surfaces.

**Architecture:** Keep `crates/app/src/channel/feishu` transport-only, and add
an app-layer `crates/app/src/feishu` module for auth, grant storage, typed
clients, and read-only resource APIs. Add a separate top-level
`FeishuIntegrationConfig` instead of expanding `FeishuChannelConfig` with
OAuth/database state. Daemon work should consume the new app-layer services
through a single `loongclaw feishu ...` namespace and an extended doctor pass.

**Tech Stack:** Rust 2024, reqwest, serde/serde_json, rusqlite, clap, tokio,
existing `loongclaw-app` / `loongclaw-daemon` crates, Feishu Open Platform
OAuth + Docs + Search + Calendar APIs.

---

### Task 1: Add Feishu Integration Config Surface

**Files:**
- Create: `crates/app/src/config/feishu_integration.rs`
- Modify: `crates/app/src/config/mod.rs`
- Modify: `crates/app/src/config/runtime.rs`
- Modify: `crates/app/src/lib.rs`
- Test: `crates/app/src/config/feishu_integration.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn feishu_integration_defaults_use_dedicated_runtime_db() {
    let config = FeishuIntegrationConfig::default();
    assert_eq!(config.resolved_sqlite_path(), crate::config::default_loongclaw_home().join("feishu.sqlite3"));
    assert_eq!(config.oauth_state_ttl_s, 600);
    assert!(config.default_scopes.iter().any(|scope| scope == "offline_access"));
}

#[test]
fn runtime_config_loads_feishu_integration_block() {
    let raw = r#"
        [feishu_integration]
        sqlite_path = "~/runtime/feishu.sqlite3"
        oauth_state_ttl_s = 900
        default_scopes = ["offline_access", "docs:document:readonly"]
    "#;
    let config = super::parse_toml_config_without_validation(raw).expect("parse config");
    assert_eq!(config.feishu_integration.oauth_state_ttl_s, 900);
    assert_eq!(config.feishu_integration.default_scopes.len(), 2);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app feishu_integration_defaults_use_dedicated_runtime_db -- --exact`

Expected: FAIL because `FeishuIntegrationConfig` does not exist yet.

**Step 3: Write minimal implementation**

Add a separate `FeishuIntegrationConfig` with:

- `sqlite_path`
- `oauth_state_ttl_s`
- `default_scopes`
- `request_timeout_s`
- helpers like `resolved_sqlite_path()` and `trimmed_default_scopes()`

Wire it into `LoongClawConfig` and re-export it from `crates/app/src/config/mod.rs`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app feishu_integration -- --nocapture`

Expected: PASS for the new config tests.

**Step 5: Commit**

```bash
git add crates/app/src/config/feishu_integration.rs crates/app/src/config/mod.rs crates/app/src/config/runtime.rs crates/app/src/lib.rs
git commit -m "feat: add feishu integration runtime config"
```

### Task 2: Scaffold The App-Layer Feishu Module

**Files:**
- Create: `crates/app/src/feishu/mod.rs`
- Create: `crates/app/src/feishu/error.rs`
- Create: `crates/app/src/feishu/principal.rs`
- Create: `crates/app/src/feishu/resources/mod.rs`
- Create: `crates/app/src/feishu/resources/types.rs`
- Modify: `crates/app/src/lib.rs`
- Test: `crates/app/src/feishu/principal.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn user_principal_key_is_stable_for_account_and_open_id() {
    let principal = FeishuUserPrincipal {
        account_id: "feishu_main".to_owned(),
        open_id: "ou_123".to_owned(),
        union_id: Some("on_456".to_owned()),
        user_id: Some("u_789".to_owned()),
        name: Some("Alice".to_owned()),
        tenant_key: Some("tenant_x".to_owned()),
    };

    assert_eq!(principal.storage_key(), "feishu_main:ou_123");
}

#[test]
fn account_binding_prefers_configured_account_id() {
    let binding = FeishuAccountBinding::new("feishu_main", "Feishu Main");
    assert_eq!(binding.account_id, "feishu_main");
    assert_eq!(binding.label, "Feishu Main");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app user_principal_key_is_stable_for_account_and_open_id -- --exact`

Expected: FAIL because the new Feishu module tree is not exported yet.

**Step 3: Write minimal implementation**

Create:

- `FeishuAccountBinding`
- `FeishuUserPrincipal`
- `FeishuGrantScopeSet`
- `FeishuApiError`
- shared resource DTOs under `resources/types.rs`

Export the new module from `crates/app/src/lib.rs`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app feishu:: -- --nocapture`

Expected: PASS for principal/error/type tests.

**Step 5: Commit**

```bash
git add crates/app/src/feishu/mod.rs crates/app/src/feishu/error.rs crates/app/src/feishu/principal.rs crates/app/src/feishu/resources/mod.rs crates/app/src/feishu/resources/types.rs crates/app/src/lib.rs
git commit -m "feat: scaffold feishu integration module"
```

### Task 3: Add SQLite Grant And OAuth State Store

**Files:**
- Create: `crates/app/src/feishu/token_store.rs`
- Modify: `crates/app/Cargo.toml`
- Test: `crates/app/src/feishu/token_store.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn token_store_round_trips_grant_for_principal() {
    let path = unique_temp_db("grant-round-trip");
    let store = FeishuTokenStore::new(path.clone());
    let principal = sample_principal();
    let grant = sample_grant(&principal);

    store.save_grant(&grant).expect("save grant");
    let loaded = store.load_grant("feishu_main", "ou_123").expect("load grant");

    assert_eq!(loaded.as_ref().map(|value| value.access_token.as_str()), Some("u-token"));
    assert_eq!(loaded.as_ref().map(|value| value.refresh_token.as_str()), Some("r-token"));
}

#[test]
fn token_store_rejects_expired_oauth_state() {
    let path = unique_temp_db("oauth-state-expiry");
    let store = FeishuTokenStore::new(path);
    store.save_oauth_state("state-1", "feishu_main", "ou_123", 10).expect("save state");

    let result = store.consume_oauth_state("state-1", 11);

    assert!(matches!(result, Err(error) if error.contains("expired")));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app token_store_round_trips_grant_for_principal -- --exact`

Expected: FAIL because `FeishuTokenStore` and schema helpers do not exist.

**Step 3: Write minimal implementation**

Model two tables:

- `feishu_oauth_states(state, account_id, principal_hint, expires_at_s, created_at_s)`
- `feishu_grants(account_id, open_id, union_id, user_id, access_token, refresh_token, scope_csv, access_expires_at_s, refresh_expires_at_s, refreshed_at_s, profile_json)`

Follow the existing style from `crates/app/src/memory/sqlite.rs`:

- create parent dirs
- `Connection::open`
- `execute_batch` schema bootstrap
- thin helpers per operation

Keep this store isolated from conversation memory.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app token_store -- --nocapture`

Expected: PASS for grant and oauth-state persistence tests.

**Step 5: Commit**

```bash
git add crates/app/src/feishu/token_store.rs crates/app/Cargo.toml
git commit -m "feat: add feishu oauth token store"
```

### Task 4: Add OAuth URL, Exchange, Refresh, And Status Services

**Files:**
- Create: `crates/app/src/feishu/auth.rs`
- Create: `crates/app/src/feishu/client.rs`
- Modify: `crates/app/src/feishu/mod.rs`
- Test: `crates/app/src/feishu/auth.rs`
- Test: `crates/app/src/feishu/client.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn auth_start_builds_feishu_authorize_url_with_state_and_scopes() {
    let spec = FeishuAuthStartSpec {
        app_id: "cli_xxx".to_owned(),
        redirect_uri: "http://127.0.0.1:34819/callback".to_owned(),
        scopes: vec!["offline_access".to_owned(), "docs:document:readonly".to_owned()],
        state: "state-123".to_owned(),
    };

    let url = build_authorize_url(&spec).expect("build authorize url");

    assert!(url.contains("https://accounts.feishu.cn/open-apis/authen/v1/authorize"));
    assert!(url.contains("client_id=cli_xxx"));
    assert!(url.contains("response_type=code"));
    assert!(url.contains("state=state-123"));
}

#[test]
fn exchange_response_converts_token_payload_into_grant() {
    let payload = serde_json::json!({
        "code": 0,
        "access_token": "u-token",
        "refresh_token": "r-token",
        "expires_in": 7200,
        "refresh_token_expires_in": 2592000,
        "scope": "offline_access docs:document:readonly",
        "token_type": "Bearer"
    });

    let grant = parse_token_exchange_response(&payload, 1_700_000_000, sample_principal()).expect("parse grant");

    assert_eq!(grant.access_token, "u-token");
    assert!(grant.scopes.iter().any(|scope| scope == "offline_access"));
    assert_eq!(grant.access_expires_at_s, 1_700_007_200);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app auth_start_builds_feishu_authorize_url_with_state_and_scopes -- --exact`

Expected: FAIL because the auth helpers and typed client do not exist.

**Step 3: Write minimal implementation**

Implement:

- authorization URL builder for `https://accounts.feishu.cn/open-apis/authen/v1/authorize`
- code exchange against `POST https://open.feishu.cn/open-apis/authen/v2/oauth/token`
- refresh flow reusing the same client
- grant/status domain types
- typed `get_user_info` against `GET https://open.feishu.cn/open-apis/authen/v1/user_info`

Use `reqwest::Url` for URL construction and keep refresh logic inside auth/client,
not daemon code.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app auth:: -- --nocapture`

Run: `cargo test -p loongclaw-app client:: -- --nocapture`

Expected: PASS for OAuth URL, token parsing, and whoami tests.

**Step 5: Commit**

```bash
git add crates/app/src/feishu/auth.rs crates/app/src/feishu/client.rs crates/app/src/feishu/mod.rs
git commit -m "feat: add feishu oauth services"
```

### Task 5: Add Docs Read-Only Resource Support

**Files:**
- Create: `crates/app/src/feishu/resources/docs.rs`
- Modify: `crates/app/src/feishu/resources/mod.rs`
- Modify: `crates/app/src/feishu/resources/types.rs`
- Test: `crates/app/src/feishu/resources/docs.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn extract_document_id_supports_full_docx_url() {
    let id = extract_document_id("https://example.feishu.cn/docx/doxbcmEtbFrbbq10nPNu8gabcef");
    assert_eq!(id.as_deref(), Some("doxbcmEtbFrbbq10nPNu8gabcef"));
}

#[test]
fn parse_raw_content_response_returns_plain_text() {
    let payload = serde_json::json!({
        "code": 0,
        "msg": "success",
        "data": { "content": "hello from docs" }
    });

    let doc = parse_raw_content_response("doxbcmEtbFrbbq10nPNu8gabcef", &payload).expect("parse doc");

    assert_eq!(doc.document_id, "doxbcmEtbFrbbq10nPNu8gabcef");
    assert_eq!(doc.content, "hello from docs");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app extract_document_id_supports_full_docx_url -- --exact`

Expected: FAIL because the docs resource helpers do not exist.

**Step 3: Write minimal implementation**

Implement read-only docs support using:

- `GET https://open.feishu.cn/open-apis/docx/v1/documents/:document_id/raw_content`
- optional `lang` query parameter
- typed output with `document_id`, `content`, and canonical URL

Accept either raw `document_id` or a doc URL on CLI-facing entry points.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app docs:: -- --nocapture`

Expected: PASS for document id parsing and raw-content decoding.

**Step 5: Commit**

```bash
git add crates/app/src/feishu/resources/docs.rs crates/app/src/feishu/resources/mod.rs crates/app/src/feishu/resources/types.rs
git commit -m "feat: add feishu docs read support"
```

### Task 6: Add Message History And Search Resource Support

**Files:**
- Create: `crates/app/src/feishu/resources/messages.rs`
- Modify: `crates/app/src/feishu/resources/mod.rs`
- Modify: `crates/app/src/feishu/resources/types.rs`
- Test: `crates/app/src/feishu/resources/messages.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn message_history_query_requires_container_id() {
    let query = FeishuMessageHistoryQuery {
        container_id_type: "chat".to_owned(),
        container_id: String::new(),
        start_time: None,
        end_time: None,
        page_size: Some(20),
        page_token: None,
    };

    let error = query.validate().expect_err("missing container should fail");
    assert!(error.contains("container_id"));
}

#[test]
fn search_message_response_parses_message_ids_and_has_more_flag() {
    let payload = serde_json::json!({
        "code": 0,
        "msg": "success",
        "data": {
            "items": ["om_1", "om_2"],
            "page_token": "next-page",
            "has_more": true
        }
    });

    let page = parse_search_messages_response(&payload).expect("parse search response");
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.page_token.as_deref(), Some("next-page"));
    assert!(page.has_more);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app message_history_query_requires_container_id -- --exact`

Expected: FAIL because the message resource query types do not exist.

**Step 3: Write minimal implementation**

Implement read-only messaging support for:

- `GET https://open.feishu.cn/open-apis/im/v1/messages`
- `POST https://open.feishu.cn/open-apis/search/v2/message`
- `GET https://open.feishu.cn/open-apis/im/v1/messages/:message_id`

Keep transport behavior unchanged. These APIs belong in `crates/app/src/feishu/resources/messages.rs`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app messages:: -- --nocapture`

Expected: PASS for message history/search DTO and validation tests.

**Step 5: Commit**

```bash
git add crates/app/src/feishu/resources/messages.rs crates/app/src/feishu/resources/mod.rs crates/app/src/feishu/resources/types.rs
git commit -m "feat: add feishu message read and search support"
```

### Task 7: Add Calendar List And Freebusy Resource Support

**Files:**
- Create: `crates/app/src/feishu/resources/calendar.rs`
- Modify: `crates/app/src/feishu/resources/mod.rs`
- Modify: `crates/app/src/feishu/resources/types.rs`
- Test: `crates/app/src/feishu/resources/calendar.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn freebusy_query_requires_time_window_and_subject() {
    let query = FeishuCalendarFreebusyQuery {
        user_id_type: Some("open_id".to_owned()),
        time_min: String::new(),
        time_max: String::new(),
        user_id: None,
        room_id: None,
        include_external_calendar: Some(true),
        only_busy: Some(true),
        need_rsvp_status: Some(false),
    };

    let error = query.validate().expect_err("invalid freebusy query");
    assert!(error.contains("time_min"));
}

#[test]
fn list_calendars_response_preserves_sync_token() {
    let payload = serde_json::json!({
        "code": 0,
        "msg": "success",
        "data": {
            "has_more": false,
            "page_token": "",
            "sync_token": "ListCalendarsSyncToken_xxx",
            "calendar_list": [{
                "calendar_id": "feishu.cn_xxx@group.calendar.feishu.cn",
                "summary": "Team Calendar",
                "description": "demo",
                "permissions": "private",
                "color": -1,
                "type": "shared",
                "summary_alias": "Alias",
                "is_deleted": false,
                "is_third_party": false,
                "role": "owner"
            }]
        }
    });

    let page = parse_calendar_list_response(&payload).expect("parse calendar list");
    assert_eq!(page.sync_token.as_deref(), Some("ListCalendarsSyncToken_xxx"));
    assert_eq!(page.calendar_list.len(), 1);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app freebusy_query_requires_time_window_and_subject -- --exact`

Expected: FAIL because calendar resource query types do not exist.

**Step 3: Write minimal implementation**

Implement read-only calendar support for:

- `GET https://open.feishu.cn/open-apis/calendar/v4/calendars`
- `POST https://open.feishu.cn/open-apis/calendar/v4/calendars/primary`
- `POST https://open.feishu.cn/open-apis/calendar/v4/freebusy/list`

Normalize the outputs into stable DTOs for CLI and future tool integration.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app calendar:: -- --nocapture`

Expected: PASS for calendar list/freebusy tests.

**Step 5: Commit**

```bash
git add crates/app/src/feishu/resources/calendar.rs crates/app/src/feishu/resources/mod.rs crates/app/src/feishu/resources/types.rs
git commit -m "feat: add feishu calendar read support"
```

### Task 8: Extend Channel Feishu Payload With Principal And Thread Context

**Files:**
- Modify: `crates/app/src/channel/mod.rs`
- Modify: `crates/app/src/channel/feishu/payload/types.rs`
- Modify: `crates/app/src/channel/feishu/payload/inbound.rs`
- Modify: `crates/app/src/channel/feishu/payload/tests.rs`
- Modify: `crates/app/src/channel/feishu/webhook.rs`
- Test: `crates/app/src/channel/feishu/payload/tests.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn feishu_message_event_uses_thread_id_when_present() {
    let payload = serde_json::json!({
        "token": "token-123",
        "header": { "event_id": "evt_1", "event_type": "im.message.receive_v1" },
        "event": {
            "sender": {
                "sender_type": "user",
                "sender_id": { "open_id": "ou_123", "union_id": "on_456", "user_id": "u_789" }
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

    let allowlist = std::collections::BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(&payload, Some("token-123"), None, &allowlist, true, "feishu_main").expect("parse");

    let event = match action {
        FeishuWebhookAction::Inbound(event) => event,
        _ => panic!("expected inbound"),
    };
    assert_eq!(event.session.thread_id.as_deref(), Some("omt_456"));
    assert_eq!(event.principal.as_ref().map(|value| value.open_id.as_str()), Some("ou_123"));
}

#[test]
fn feishu_message_without_sender_open_id_keeps_principal_empty() {
    let payload = sample_text_payload_without_sender_ids();
    let allowlist = std::collections::BTreeSet::from([String::from("oc_123")]);
    let action = parse_feishu_webhook_payload(&payload, Some("token-123"), None, &allowlist, true, "feishu_main").expect("parse");

    let event = match action {
        FeishuWebhookAction::Inbound(event) => event,
        _ => panic!("expected inbound"),
    };
    assert!(event.principal.is_none());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app feishu_message_event_uses_thread_id_when_present -- --exact`

Expected: FAIL because inbound Feishu events do not capture thread or principal yet.

**Step 3: Write minimal implementation**

Add:

- optional `principal` metadata to `ChannelInboundMessage` or the Feishu inbound event
- sender ID extraction from `event.sender.sender_id`
- `thread_id` extraction from `message.thread_id`
- `ChannelSession::with_account_and_thread(...)` for Feishu thread routing

Do not move OAuth or resource logic into `channel/feishu`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app feishu -- --nocapture`

Expected: PASS for Feishu payload/webhook tests, including the new thread/principal cases.

**Step 5: Commit**

```bash
git add crates/app/src/channel/mod.rs crates/app/src/channel/feishu/payload/types.rs crates/app/src/channel/feishu/payload/inbound.rs crates/app/src/channel/feishu/payload/tests.rs crates/app/src/channel/feishu/webhook.rs
git commit -m "feat: capture feishu principal and thread context"
```

### Task 9: Add `loongclaw feishu` CLI Namespace

**Files:**
- Modify: `crates/daemon/src/main.rs`
- Modify: `crates/daemon/src/tests/mod.rs`
- Create: `crates/daemon/src/tests/feishu_cli.rs`
- Test: `crates/daemon/src/tests/feishu_cli.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn feishu_auth_status_subcommand_is_registered() {
    let command = Cli::command();
    let mut rendered = Vec::new();
    command.write_long_help(&mut rendered).expect("render help");
    let help = String::from_utf8(rendered).expect("help utf8");
    assert!(help.contains("feishu auth start"));
    assert!(help.contains("feishu auth status"));
    assert!(help.contains("feishu read doc"));
    assert!(help.contains("feishu search messages"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon feishu_auth_status_subcommand_is_registered -- --exact`

Expected: FAIL because no `feishu` command tree exists yet.

**Step 3: Write minimal implementation**

Add a nested CLI namespace:

- `loongclaw feishu auth start`
- `loongclaw feishu auth exchange`
- `loongclaw feishu auth status`
- `loongclaw feishu auth revoke`
- `loongclaw feishu whoami`
- `loongclaw feishu read doc`
- `loongclaw feishu messages history`
- `loongclaw feishu messages get`
- `loongclaw feishu search messages`
- `loongclaw feishu calendar list`
- `loongclaw feishu calendar freebusy`

Keep existing `feishu-send` and `feishu-serve` intact for transport compatibility.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon feishu_cli -- --nocapture`

Expected: PASS for CLI help/dispatch tests.

**Step 5: Commit**

```bash
git add crates/daemon/src/main.rs crates/daemon/src/tests/mod.rs crates/daemon/src/tests/feishu_cli.rs
git commit -m "feat: add feishu integration cli namespace"
```

### Task 10: Wire CLI Commands To App-Layer Services

**Files:**
- Modify: `crates/daemon/src/main.rs`
- Modify: `crates/app/src/feishu/auth.rs`
- Modify: `crates/app/src/feishu/client.rs`
- Modify: `crates/app/src/feishu/resources/docs.rs`
- Modify: `crates/app/src/feishu/resources/messages.rs`
- Modify: `crates/app/src/feishu/resources/calendar.rs`
- Test: `crates/daemon/src/tests/feishu_cli.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn feishu_auth_start_prints_authorize_url_and_state() {
    let output = run_cli_for_test(&[
        "loongclaw",
        "feishu",
        "auth",
        "start",
        "--config",
        sample_feishu_config_path(),
        "--account",
        "feishu_main",
    ]);

    assert!(output.contains("authorize_url"));
    assert!(output.contains("state"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon feishu_auth_start_prints_authorize_url_and_state -- --exact`

Expected: FAIL because the command tree is not connected to the app-layer services yet.

**Step 3: Write minimal implementation**

Wire CLI handlers to:

- create/load `FeishuTokenStore`
- build auth URL and persist state
- exchange auth code for grant and persist it
- refresh or inspect grant status
- call typed docs/messages/calendar clients with `user_access_token`

Return stable JSON in `--json` mode and concise plain text otherwise.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon feishu_auth_start -- --nocapture`

Run: `cargo test -p loongclaw-daemon feishu_cli -- --nocapture`

Expected: PASS for CLI execution-path tests.

**Step 5: Commit**

```bash
git add crates/daemon/src/main.rs crates/app/src/feishu/auth.rs crates/app/src/feishu/client.rs crates/app/src/feishu/resources/docs.rs crates/app/src/feishu/resources/messages.rs crates/app/src/feishu/resources/calendar.rs crates/daemon/src/tests/feishu_cli.rs
git commit -m "feat: wire feishu integration cli handlers"
```

### Task 11: Extend Doctor With Feishu Integration Readiness Checks

**Files:**
- Modify: `crates/daemon/src/doctor_cli.rs`
- Modify: `crates/daemon/src/tests/mod.rs`
- Create: `crates/daemon/src/tests/doctor_feishu.rs`
- Test: `crates/daemon/src/tests/doctor_feishu.rs`

**Step 1: Write the failing test**

Add tests like:

```rust
#[test]
fn doctor_reports_missing_feishu_grant_when_channel_is_enabled() {
    let result = run_doctor_for_test(sample_config_with_feishu_channel(), false);
    assert!(result.contains("feishu user grant"));
    assert!(result.contains("missing"));
}

#[test]
fn doctor_reports_feishu_grant_freshness_when_valid_grant_exists() {
    let result = run_doctor_for_test(sample_config_with_seeded_feishu_grant(), false);
    assert!(result.contains("feishu token freshness"));
    assert!(result.contains("ok"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon doctor_reports_missing_feishu_grant_when_channel_is_enabled -- --exact`

Expected: FAIL because doctor has no Feishu integration checks yet.

**Step 3: Write minimal implementation**

Add Feishu doctor checks for:

- integration DB parent directory
- app credentials present for selected account
- webhook verification readiness
- stored grant presence for a requested principal hint or seeded sample
- token freshness / expired refresh-token warnings
- scope coverage for docs/messages/calendar read

Keep fix mode limited to directory creation and safe config defaults.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon doctor_feishu -- --nocapture`

Expected: PASS for Feishu doctor coverage.

**Step 5: Commit**

```bash
git add crates/daemon/src/doctor_cli.rs crates/daemon/src/tests/mod.rs crates/daemon/src/tests/doctor_feishu.rs
git commit -m "feat: add feishu integration doctor checks"
```

### Task 12: Full Verification And Cleanup

**Files:**
- Modify: `README.md`
- Modify: `docs/roadmap.md`
- Modify: `docs/plans/2026-03-12-feishu-integration-stack-v1-design.md`

**Step 1: Write the failing doc/test delta**

Before editing docs, confirm the delivered CLI and capability names differ from
the current docs. Update only the sections that describe Feishu support; do not
reshape unrelated roadmap items.

**Step 2: Run targeted verification**

Run:

```bash
cargo test -p loongclaw-app feishu -- --nocapture
cargo test -p loongclaw-daemon feishu -- --nocapture
cargo test -p loongclaw-daemon doctor_feishu -- --nocapture
```

Expected: PASS for all Feishu-specific tests.

**Step 3: Run broader safety verification**

Run:

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Expected: PASS without regressions.

**Step 4: Update docs to match shipped behavior**

Document:

- webhook channel behavior that still exists
- new `loongclaw feishu ...` namespace
- read-only v1 scope and known deferrals
- required Feishu scopes and auth flow expectations

**Step 5: Commit**

```bash
git add README.md docs/roadmap.md docs/plans/2026-03-12-feishu-integration-stack-v1-design.md
git commit -m "docs: document feishu integration stack v1"
```
