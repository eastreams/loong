use super::*;
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Request, State},
    routing::{get, post},
};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

fn temp_feishu_cli_dir(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "loongclaw-feishu-cli-{label}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

fn write_sample_feishu_config(dir: &std::path::Path) -> std::path::PathBuf {
    fs::create_dir_all(dir).expect("create temp feishu config dir");
    let config_path = dir.join("loongclaw.toml");
    let sqlite_path = dir.join("feishu.sqlite3");

    let mut config = mvp::config::LoongClawConfig::default();
    config.feishu.enabled = true;
    config.feishu.account_id = Some("feishu_main".to_owned());
    config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline(
        "cli_a1b2c3".to_owned(),
    ));
    config.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
        "app-secret".to_owned(),
    ));
    config.feishu_integration.sqlite_path = sqlite_path.display().to_string();

    mvp::config::write(config_path.to_str(), &config, true).expect("write sample feishu config");
    config_path
}

fn write_sample_feishu_config_with_account_alias(
    dir: &std::path::Path,
    configured_account_id: &str,
    storage_account_id: &str,
) -> std::path::PathBuf {
    fs::create_dir_all(dir).expect("create temp feishu config dir");
    let config_path = dir.join("loongclaw.toml");
    let sqlite_path = dir.join("feishu.sqlite3");

    let mut config = mvp::config::LoongClawConfig::default();
    config.feishu.enabled = true;
    config.feishu.accounts = BTreeMap::from([(
        configured_account_id.to_owned(),
        mvp::config::FeishuAccountConfig {
            account_id: Some(storage_account_id.to_owned()),
            app_id: Some(loongclaw_contracts::SecretRef::Inline(
                "cli_alias".to_owned(),
            )),
            app_secret: Some(loongclaw_contracts::SecretRef::Inline(
                "app-secret-alias".to_owned(),
            )),
            ..mvp::config::FeishuAccountConfig::default()
        },
    )]);
    config.feishu.default_account = Some(configured_account_id.to_owned());
    config.feishu_integration.sqlite_path = sqlite_path.display().to_string();

    mvp::config::write(config_path.to_str(), &config, true).expect("write sample feishu config");
    config_path
}

fn write_sample_feishu_config_with_account_alias_and_base_url(
    dir: &std::path::Path,
    configured_account_id: &str,
    storage_account_id: &str,
    base_url: &str,
) -> std::path::PathBuf {
    fs::create_dir_all(dir).expect("create temp feishu config dir");
    let config_path = dir.join("loongclaw.toml");
    let sqlite_path = dir.join("feishu.sqlite3");

    let mut config = mvp::config::LoongClawConfig::default();
    config.feishu.enabled = true;
    config.feishu.accounts = BTreeMap::from([(
        configured_account_id.to_owned(),
        mvp::config::FeishuAccountConfig {
            account_id: Some(storage_account_id.to_owned()),
            app_id: Some(loongclaw_contracts::SecretRef::Inline(
                "cli_alias".to_owned(),
            )),
            app_secret: Some(loongclaw_contracts::SecretRef::Inline(
                "app-secret-alias".to_owned(),
            )),
            base_url: Some(base_url.to_owned()),
            ..mvp::config::FeishuAccountConfig::default()
        },
    )]);
    config.feishu.default_account = Some(configured_account_id.to_owned());
    config.feishu_integration.sqlite_path = sqlite_path.display().to_string();

    mvp::config::write(config_path.to_str(), &config, true).expect("write sample feishu config");
    config_path
}

fn write_sample_feishu_config_with_base_url(
    dir: &std::path::Path,
    base_url: &str,
) -> std::path::PathBuf {
    fs::create_dir_all(dir).expect("create temp feishu config dir");
    let config_path = dir.join("loongclaw.toml");
    let sqlite_path = dir.join("feishu.sqlite3");

    let mut config = mvp::config::LoongClawConfig::default();
    config.feishu.enabled = true;
    config.feishu.account_id = Some("feishu_main".to_owned());
    config.feishu.app_id = Some(loongclaw_contracts::SecretRef::Inline(
        "cli_a1b2c3".to_owned(),
    ));
    config.feishu.app_secret = Some(loongclaw_contracts::SecretRef::Inline(
        "app-secret".to_owned(),
    ));
    config.feishu.base_url = Some(base_url.to_owned());
    config.feishu_integration.sqlite_path = sqlite_path.display().to_string();

    mvp::config::write(config_path.to_str(), &config, true).expect("write sample feishu config");
    config_path
}

fn sample_grant(
    account_id: &str,
    open_id: &str,
    access_token: &str,
    refresh_token: &str,
    now_s: i64,
) -> mvp::channel::feishu::api::FeishuGrant {
    mvp::channel::feishu::api::FeishuGrant {
        principal: mvp::channel::feishu::api::FeishuUserPrincipal {
            account_id: account_id.to_owned(),
            open_id: open_id.to_owned(),
            union_id: Some("on_456".to_owned()),
            user_id: Some("u_789".to_owned()),
            name: Some("Alice".to_owned()),
            tenant_key: Some("tenant_x".to_owned()),
            avatar_url: None,
            email: Some("alice@example.com".to_owned()),
            enterprise_email: None,
        },
        access_token: access_token.to_owned(),
        refresh_token: refresh_token.to_owned(),
        scopes: mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
            "offline_access",
            "docx:document:readonly",
            "im:message:readonly",
            "im:message.group_msg:readonly",
            "search:message",
            "calendar:calendar:readonly",
        ]),
        access_expires_at_s: now_s + 3600,
        refresh_expires_at_s: now_s + 86_400,
        refreshed_at_s: now_s,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MockRequest {
    method: String,
    path: String,
    query: Option<String>,
    authorization: Option<String>,
    body: String,
}

#[derive(Clone, Default)]
struct MockServerState {
    requests: Arc<Mutex<Vec<MockRequest>>>,
}

async fn spawn_mock_feishu_server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock feishu server");
    let address = listener.local_addr().expect("mock server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve mock feishu api");
    });
    (format!("http://{address}"), handle)
}

async fn record_request(State(state): State<MockServerState>, request: Request) {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, usize::MAX)
        .await
        .expect("read mock request body");
    state.requests.lock().await.push(MockRequest {
        method: parts.method.to_string(),
        path: parts.uri.path().to_owned(),
        query: parts.uri.query().map(ToOwned::to_owned),
        authorization: parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value: &axum::http::HeaderValue| value.to_str().ok())
            .map(ToOwned::to_owned),
        body: String::from_utf8(body.to_vec()).expect("mock request body utf8"),
    });
}

#[test]
fn feishu_command_registers_nested_integration_subcommands() {
    let help = render_cli_help(["feishu"]);

    assert!(help.contains("auth"));
    assert!(help.contains("whoami"));
    assert!(help.contains("doc"));
    assert!(help.contains("read"));
    assert!(help.contains("messages"));
    assert!(help.contains("search"));
    assert!(help.contains("calendar"));
    assert!(help.contains("send"));
    assert!(help.contains("reply"));
    assert!(help.contains("serve"));
}

#[test]
fn feishu_auth_subcommand_registers_start_exchange_status_and_revoke() {
    let help = render_cli_help(["feishu", "auth"]);

    assert!(help.contains("start"));
    assert!(help.contains("exchange"));
    assert!(help.contains("list"));
    assert!(help.contains("select"));
    assert!(help.contains("status"));
    assert!(help.contains("revoke"));
}

#[test]
fn feishu_resource_subcommands_parse() {
    try_parse_cli(["loongclaw", "feishu", "auth", "list"]).expect("auth list command should parse");

    try_parse_cli([
        "loongclaw",
        "feishu",
        "auth",
        "select",
        "--open-id",
        "ou_demo",
    ])
    .expect("auth select command should parse");

    try_parse_cli([
        "loongclaw",
        "feishu",
        "doc",
        "create",
        "--title",
        "Release Plan",
    ])
    .expect("doc create command should parse");

    try_parse_cli([
        "loongclaw",
        "feishu",
        "doc",
        "append",
        "--url",
        "https://open.feishu.cn/docx/doxcnDemo",
        "--content",
        "Follow-up note",
    ])
    .expect("doc append command should parse");

    try_parse_cli([
        "loongclaw",
        "feishu",
        "read",
        "doc",
        "--url",
        "https://open.feishu.cn/docx/doxcnDemo",
    ])
    .expect("read doc command should parse");

    try_parse_cli([
        "loongclaw",
        "feishu",
        "messages",
        "history",
        "--container-id-type",
        "chat",
        "--container-id",
        "oc_demo",
    ])
    .expect("messages history command should parse");

    try_parse_cli([
        "loongclaw",
        "feishu",
        "messages",
        "resource",
        "--message-id",
        "om_demo_resource",
        "--file-key",
        "file_demo_resource",
        "--type",
        "file",
        "--output",
        "downloads/spec-sheet.pdf",
    ])
    .expect("messages resource command should parse");

    let parsed = try_parse_cli([
        "loongclaw",
        "feishu",
        "messages",
        "resource",
        "--message-id",
        "om_demo_audio",
        "--file-key",
        "file_demo_audio",
        "--type",
        "audio",
        "--output",
        "downloads/voice.ogg",
    ])
    .expect("messages resource command should accept audio alias");

    let Some(Commands::Feishu { command }) = parsed.command else {
        panic!("expected feishu command");
    };
    let loongclaw_daemon::feishu_cli::FeishuCommand::Messages { command } = command else {
        panic!("expected feishu messages command");
    };
    let loongclaw_daemon::feishu_cli::FeishuMessagesCommand::Resource(args) = command else {
        panic!("expected feishu messages resource command");
    };
    assert!(matches!(
        args.resource_type,
        loongclaw_daemon::feishu_cli::FeishuMessageResourceCliType::File
    ));

    try_parse_cli([
        "loongclaw",
        "feishu",
        "search",
        "messages",
        "--query",
        "design review",
    ])
    .expect("search messages command should parse");

    try_parse_cli([
        "loongclaw",
        "feishu",
        "calendar",
        "freebusy",
        "--time-min",
        "2026-03-12T09:00:00+08:00",
        "--time-max",
        "2026-03-12T10:00:00+08:00",
        "--user-id",
        "ou_demo",
    ])
    .expect("calendar freebusy command should parse");

    try_parse_cli([
        "loongclaw",
        "feishu",
        "send",
        "--open-id",
        "ou_demo",
        "--receive-id-type",
        "chat_id",
        "--uuid",
        "send-uuid-1",
        "--receive-id",
        "oc_demo",
        "--text",
        "hello",
    ])
    .expect("nested feishu send command should parse");

    try_parse_cli([
        "loongclaw",
        "feishu",
        "reply",
        "--open-id",
        "ou_demo",
        "--message-id",
        "om_parent_1",
        "--text",
        "hello",
        "--uuid",
        "reply-uuid-1",
        "--reply-in-thread",
    ])
    .expect("nested feishu reply command should parse");

    try_parse_cli([
        "loongclaw",
        "feishu",
        "send",
        "--receive-id",
        "oc_demo",
        "--post-json",
        "{\"zh_cn\":{\"title\":\"Ship update\",\"content\":[[{\"tag\":\"text\",\"text\":\"rich ship\"}]]}}",
    ])
    .expect("nested feishu send post command should parse");

    try_parse_cli([
        "loongclaw",
        "feishu",
        "send",
        "--receive-id",
        "oc_demo",
        "--image-path",
        "/tmp/demo.png",
    ])
    .expect("nested feishu send image-path command should parse");

    try_parse_cli([
        "loongclaw",
        "feishu",
        "reply",
        "--message-id",
        "om_parent_1",
        "--file-path",
        "/tmp/demo.txt",
    ])
    .expect("nested feishu reply file-path command should parse");

    try_parse_cli(["loongclaw", "feishu", "serve", "--bind", "127.0.0.1:18080"])
        .expect("nested feishu serve command should parse");
}

#[test]
fn legacy_feishu_send_subcommand_supports_rich_outbound_flags() {
    try_parse_cli([
        "loongclaw",
        "feishu-send",
        "--receive-id",
        "oc_demo",
        "--post-json",
        "{\"zh_cn\":{\"title\":\"Ship update\",\"content\":[[{\"tag\":\"text\",\"text\":\"rich ship\"}]]}}",
    ])
    .expect("legacy feishu-send should parse post content");

    try_parse_cli([
        "loongclaw",
        "feishu-send",
        "--receive-id-type",
        "open_id",
        "--receive-id",
        "ou_demo",
        "--image-path",
        "/tmp/demo.png",
        "--uuid",
        "legacy-send-image-1",
    ])
    .expect("legacy feishu-send should parse image-path content");

    try_parse_cli([
        "loongclaw",
        "feishu-send",
        "--receive-id",
        "oc_demo",
        "--file-key",
        "file_v2_demo",
    ])
    .expect("legacy feishu-send should parse file-key content");
}

#[tokio::test]
async fn feishu_send_command_requires_confirmed_write_scope() {
    let temp_dir = temp_feishu_cli_dir("send-command-scope");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "tenant_access_token": "t-token-send-cli"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_send_cli_ignored",
                                "root_id": "om_send_cli_ignored"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant("feishu_main", "ou_123", "u-token-1", "r-token-1", now_s);
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes(["offline_access"]);
    store.save_grant(&grant).expect("seed send grant");
    store
        .set_selected_grant("feishu_main", "ou_123", now_s + 1)
        .expect("select send grant");

    let error = loongclaw_daemon::feishu_cli::execute_feishu_send(
        &loongclaw_daemon::feishu_cli::FeishuSendArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: None,
            },
            receive_id_type: None,
            receive_id: "oc_demo".to_owned(),
            text: Some("hello".to_owned()),
            post_json: None,
            image_key: None,
            file_key: None,
            image_path: None,
            file_path: None,
            file_type: None,
            card: false,
            uuid: None,
        },
    )
    .await
    .expect_err("send should reject grants without a confirmed write scope");

    assert!(
        error.contains("loong feishu send requires at least one Feishu scope [im:message, im:message:send_as_bot, im:message:send]"),
        "error={error}"
    );
    assert!(
        error.contains("loong feishu auth start --account feishu_main --capability message-write"),
        "error={error}"
    );
    assert!(requests.lock().await.is_empty());

    server.abort();
}

#[tokio::test]
async fn feishu_send_command_uses_tenant_token_receive_id_override_and_uuid() {
    let temp_dir = temp_feishu_cli_dir("send-command");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "tenant_access_token": "t-token-send-cli"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_send_cli_1",
                                "root_id": "om_send_cli_1"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant("feishu_main", "ou_123", "u-token-1", "r-token-1", now_s);
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "im:message:send_as_bot",
    ]);
    store.save_grant(&grant).expect("seed send grant");
    store
        .set_selected_grant("feishu_main", "ou_123", now_s + 1)
        .expect("select send grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_send(
        &loongclaw_daemon::feishu_cli::FeishuSendArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            receive_id_type: Some("chat_id".to_owned()),
            receive_id: "oc_send_demo".to_owned(),
            text: Some("operator send with uuid".to_owned()),
            post_json: None,
            image_key: None,
            file_key: None,
            image_path: None,
            file_path: None,
            file_type: None,
            card: false,
            uuid: Some("send-uuid-1".to_owned()),
        },
    )
    .await
    .expect("execute feishu send");

    assert_eq!(payload["account_id"], "feishu_main");
    assert_eq!(payload["principal"]["open_id"], "ou_123");
    assert_eq!(payload["delivery"]["message_id"], "om_send_cli_1");
    assert_eq!(payload["delivery"]["receive_id_type"], "chat_id");
    assert_eq!(payload["delivery"]["receive_id"], "oc_send_demo");
    assert_eq!(payload["delivery"]["uuid"], "send-uuid-1");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-send-cli")
    );
    assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
    assert!(
        requests[1]
            .query
            .as_deref()
            .is_some_and(|query| query.contains("receive_id_type=chat_id"))
    );
    assert!(requests[1].body.contains("\"uuid\":\"send-uuid-1\""));
    assert!(requests[1].body.contains("\"receive_id\":\"oc_send_demo\""));
    assert!(
        requests[1]
            .body
            .contains("\\\"text\\\":\\\"operator send with uuid\\\"")
    );

    server.abort();
}

#[tokio::test]
async fn feishu_send_command_supports_post_content() {
    let temp_dir = temp_feishu_cli_dir("send-command-post");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "tenant_access_token": "t-token-send-cli-post"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_send_cli_post_1",
                                "root_id": "om_send_cli_post_1"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant(
        "feishu_main",
        "ou_123",
        "u-token-post",
        "r-token-post",
        now_s,
    );
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "im:message:send_as_bot",
    ]);
    store.save_grant(&grant).expect("seed send post grant");
    store
        .set_selected_grant("feishu_main", "ou_123", now_s + 1)
        .expect("select send post grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_send(&loongclaw_daemon::feishu_cli::FeishuSendArgs {
        grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
            open_id: Some("ou_123".to_owned()),
        },
        receive_id_type: Some("chat_id".to_owned()),
        receive_id: "oc_send_post".to_owned(),
        text: None,
        post_json: Some(
            "{\"zh_cn\":{\"title\":\"Ship update\",\"content\":[[{\"tag\":\"text\",\"text\":\"rich ship\"}]]}}"
                .to_owned(),
        ),
        image_key: None,
        file_key: None,
        image_path: None,
        file_path: None,
        file_type: None,
        card: false,
        uuid: Some("send-post-uuid-1".to_owned()),
    })
    .await
    .expect("execute feishu send post");

    assert_eq!(payload["delivery"]["message_id"], "om_send_cli_post_1");
    assert_eq!(payload["delivery"]["msg_type"], "post");
    assert_eq!(payload["delivery"]["uuid"], "send-post-uuid-1");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
    assert!(requests[1].body.contains("\"msg_type\":\"post\""));
    assert!(
        requests[1]
            .body
            .contains("\\\"title\\\":\\\"Ship update\\\"")
    );
    assert!(requests[1].body.contains("\\\"text\\\":\\\"rich ship\\\""));

    server.abort();
}

#[tokio::test]
async fn feishu_send_command_uploads_image_path_and_sends_image_message() {
    use std::fs;

    let temp_dir = temp_feishu_cli_dir("send-command-image");
    fs::create_dir_all(&temp_dir).expect("create temp feishu dir");
    let image_path = temp_dir.join("demo-image.png");
    fs::write(&image_path, "fake-png").expect("write sample image file");

    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "tenant_access_token": "t-token-send-cli-image"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/images",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "image_key": "img_v2_cli_demo"
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_send_cli_image_1",
                                "root_id": "om_send_cli_image_1"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant(
        "feishu_main",
        "ou_123",
        "u-token-image",
        "r-token-image",
        now_s,
    );
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "im:message:send_as_bot",
    ]);
    store.save_grant(&grant).expect("seed send image grant");
    store
        .set_selected_grant("feishu_main", "ou_123", now_s + 1)
        .expect("select send image grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_send(
        &loongclaw_daemon::feishu_cli::FeishuSendArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            receive_id_type: Some("chat_id".to_owned()),
            receive_id: "oc_send_image".to_owned(),
            text: None,
            post_json: None,
            card: false,
            uuid: Some("send-image-uuid-1".to_owned()),
            image_key: None,
            file_key: None,
            image_path: Some(image_path.display().to_string()),
            file_path: None,
            file_type: None,
        },
    )
    .await
    .expect("execute feishu send image");

    assert_eq!(payload["delivery"]["message_id"], "om_send_cli_image_1");
    assert_eq!(payload["delivery"]["msg_type"], "image");
    assert_eq!(payload["delivery"]["uuid"], "send-image-uuid-1");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1].path, "/open-apis/im/v1/images");
    assert!(requests[1].body.contains("name=\"image_type\""));
    assert!(requests[1].body.contains("message"));
    assert!(requests[1].body.contains("filename=\"demo-image.png\""));
    assert!(requests[1].body.contains("fake-png"));
    assert_eq!(requests[2].path, "/open-apis/im/v1/messages");
    assert!(requests[2].body.contains("\"msg_type\":\"image\""));
    assert!(
        requests[2]
            .body
            .contains("\\\"image_key\\\":\\\"img_v2_cli_demo\\\"")
    );

    server.abort();
}

#[tokio::test]
async fn feishu_reply_command_uses_tenant_token_and_thread_flag() {
    let temp_dir = temp_feishu_cli_dir("reply-command");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "tenant_access_token": "t-token-reply-cli"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_parent_1/reply",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_reply_cli_1",
                                "root_id": "om_parent_1",
                                "parent_id": "om_parent_1"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant("feishu_main", "ou_123", "u-token-1", "r-token-1", now_s);
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "im:message:send_as_bot",
    ]);
    store.save_grant(&grant).expect("seed reply grant");
    store
        .set_selected_grant("feishu_main", "ou_123", now_s + 1)
        .expect("select reply grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_reply(
        &loongclaw_daemon::feishu_cli::FeishuReplyArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: None,
            },
            message_id: "om_parent_1".to_owned(),
            text: Some("threaded operator reply".to_owned()),
            post_json: None,
            image_key: None,
            file_key: None,
            image_path: None,
            file_path: None,
            file_type: None,
            card: false,
            reply_in_thread: true,
            uuid: Some("reply-uuid-1".to_owned()),
        },
    )
    .await
    .expect("execute feishu reply");

    assert_eq!(payload["account_id"], "feishu_main");
    assert_eq!(payload["delivery"]["message_id"], "om_reply_cli_1");
    assert_eq!(payload["delivery"]["reply_to_message_id"], "om_parent_1");
    assert_eq!(payload["delivery"]["reply_in_thread"], true);
    assert_eq!(payload["delivery"]["uuid"], "reply-uuid-1");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-reply-cli")
    );
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_parent_1/reply"
    );
    assert!(requests[1].body.contains("\"reply_in_thread\":true"));
    assert!(requests[1].body.contains("\"uuid\":\"reply-uuid-1\""));
    assert!(
        requests[1]
            .body
            .contains("\\\"text\\\":\\\"threaded operator reply\\\"")
    );

    server.abort();
}

#[tokio::test]
async fn feishu_reply_command_supports_post_content() {
    let temp_dir = temp_feishu_cli_dir("reply-command-post");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "tenant_access_token": "t-token-reply-cli-post"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_parent_post_cli/reply",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_reply_cli_post_1",
                                "root_id": "om_parent_post_cli",
                                "parent_id": "om_parent_post_cli"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant(
        "feishu_main",
        "ou_123",
        "u-token-reply-post",
        "r-token-reply-post",
        now_s,
    );
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "im:message:send_as_bot",
    ]);
    store.save_grant(&grant).expect("seed reply post grant");
    store
        .set_selected_grant("feishu_main", "ou_123", now_s + 1)
        .expect("select reply post grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_reply(&loongclaw_daemon::feishu_cli::FeishuReplyArgs {
        grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
            open_id: None,
        },
        message_id: "om_parent_post_cli".to_owned(),
        text: None,
        post_json: Some(
            "{\"zh_cn\":{\"title\":\"Thread update\",\"content\":[[{\"tag\":\"text\",\"text\":\"rich reply\"}]]}}"
                .to_owned(),
        ),
        image_key: None,
        file_key: None,
        image_path: None,
        file_path: None,
        file_type: None,
        card: false,
        reply_in_thread: false,
        uuid: Some("reply-post-uuid-1".to_owned()),
    })
    .await
    .expect("execute feishu reply post");

    assert_eq!(payload["delivery"]["message_id"], "om_reply_cli_post_1");
    assert_eq!(payload["delivery"]["msg_type"], "post");
    assert_eq!(payload["delivery"]["uuid"], "reply-post-uuid-1");

    let requests = requests.lock().await.clone();
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_parent_post_cli/reply"
    );
    assert!(requests[1].body.contains("\"msg_type\":\"post\""));
    assert!(
        requests[1]
            .body
            .contains("\\\"title\\\":\\\"Thread update\\\"")
    );
    assert!(requests[1].body.contains("\\\"text\\\":\\\"rich reply\\\""));

    server.abort();
}

#[tokio::test]
async fn feishu_reply_command_uploads_file_path_and_sends_file_message() {
    use std::fs;

    let temp_dir = temp_feishu_cli_dir("reply-command-file");
    fs::create_dir_all(&temp_dir).expect("create temp feishu dir");
    let file_path = temp_dir.join("demo-file.txt");
    fs::write(&file_path, "file-attachment").expect("write sample file");

    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "tenant_access_token": "t-token-reply-cli-file"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/files",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "file_key": "file_v2_cli_demo"
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_parent_file_cli/reply",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "message_id": "om_reply_cli_file_1",
                                "root_id": "om_parent_file_cli",
                                "parent_id": "om_parent_file_cli"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant(
        "feishu_main",
        "ou_123",
        "u-token-file",
        "r-token-file",
        now_s,
    );
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "im:message:send_as_bot",
    ]);
    store.save_grant(&grant).expect("seed reply file grant");
    store
        .set_selected_grant("feishu_main", "ou_123", now_s + 1)
        .expect("select reply file grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_reply(
        &loongclaw_daemon::feishu_cli::FeishuReplyArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: None,
            },
            message_id: "om_parent_file_cli".to_owned(),
            text: None,
            post_json: None,
            card: false,
            reply_in_thread: false,
            uuid: Some("reply-file-uuid-1".to_owned()),
            image_key: None,
            file_key: None,
            image_path: None,
            file_path: Some(file_path.display().to_string()),
            file_type: Some("stream".to_owned()),
        },
    )
    .await
    .expect("execute feishu reply file");

    assert_eq!(payload["delivery"]["message_id"], "om_reply_cli_file_1");
    assert_eq!(payload["delivery"]["msg_type"], "file");
    assert_eq!(payload["delivery"]["uuid"], "reply-file-uuid-1");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1].path, "/open-apis/im/v1/files");
    assert!(requests[1].body.contains("name=\"file_type\""));
    assert!(requests[1].body.contains("stream"));
    assert!(requests[1].body.contains("name=\"file_name\""));
    assert!(requests[1].body.contains("demo-file.txt"));
    assert!(requests[1].body.contains("filename=\"demo-file.txt\""));
    assert!(requests[1].body.contains("file-attachment"));
    assert_eq!(
        requests[2].path,
        "/open-apis/im/v1/messages/om_parent_file_cli/reply"
    );
    assert!(requests[2].body.contains("\"msg_type\":\"file\""));
    assert!(
        requests[2]
            .body
            .contains("\\\"file_key\\\":\\\"file_v2_cli_demo\\\"")
    );

    server.abort();
}

#[tokio::test]
async fn feishu_auth_start_persists_oauth_state_and_authorize_url() {
    let temp_dir = temp_feishu_cli_dir("auth-start");
    let config_path = write_sample_feishu_config(&temp_dir);
    let args = loongclaw_daemon::feishu_cli::FeishuAuthStartArgs {
        common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
            config: Some(config_path.display().to_string()),
            account: Some("feishu_main".to_owned()),
            json: true,
        },
        redirect_uri: "http://127.0.0.1:34819/callback".to_owned(),
        principal_hint: Some("operator".to_owned()),
        scopes: Vec::new(),
        capabilities: Vec::new(),
        include_message_write: false,
    };

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_start(&args)
        .await
        .expect("execute feishu auth start");

    let authorize_url = payload
        .get("authorize_url")
        .and_then(Value::as_str)
        .expect("authorize_url");
    let state = payload.get("state").and_then(Value::as_str).expect("state");

    assert!(authorize_url.contains("https://accounts.feishu.cn/open-apis/authen/v1/authorize"));
    assert!(authorize_url.contains("client_id=cli_a1b2c3"));
    assert!(authorize_url.contains("state="));

    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let stored = store
        .consume_oauth_state(state, payload["expires_at_s"].as_i64().expect("expiry") - 1)
        .expect("stored oauth state should exist");

    assert_eq!(stored.account_id, "feishu_main");
    assert_eq!(stored.principal_hint, "operator");
    assert_eq!(
        stored.redirect_uri.as_deref(),
        Some("http://127.0.0.1:34819/callback")
    );
    assert!(
        stored
            .scope_csv
            .split_whitespace()
            .any(|scope| scope == "offline_access")
    );
}

#[tokio::test]
async fn feishu_auth_start_can_include_recommended_message_write_scopes() {
    let temp_dir = temp_feishu_cli_dir("auth-start-write");
    let config_path = write_sample_feishu_config(&temp_dir);
    let args = loongclaw_daemon::feishu_cli::FeishuAuthStartArgs {
        common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
            config: Some(config_path.display().to_string()),
            account: Some("feishu_main".to_owned()),
            json: true,
        },
        redirect_uri: "http://127.0.0.1:34819/callback".to_owned(),
        principal_hint: Some("operator".to_owned()),
        scopes: vec!["offline_access".to_owned()],
        capabilities: Vec::new(),
        include_message_write: true,
    };

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_start(&args)
        .await
        .expect("execute feishu auth start");

    let authorize_url = payload
        .get("authorize_url")
        .and_then(Value::as_str)
        .expect("authorize_url");
    let state = payload.get("state").and_then(Value::as_str).expect("state");
    assert!(authorize_url.contains("im%3Amessage"));
    assert!(authorize_url.contains("im%3Amessage%3Asend_as_bot"));

    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let stored = store
        .consume_oauth_state(state, payload["expires_at_s"].as_i64().expect("expiry") - 1)
        .expect("stored oauth state should exist");
    let scopes = stored.scope_csv.split_whitespace().collect::<Vec<_>>();

    assert!(scopes.contains(&"offline_access"));
    assert!(scopes.contains(&"im:message"));
    assert!(scopes.contains(&"im:message:send_as_bot"));
    assert_eq!(
        scopes
            .iter()
            .filter(|scope| **scope == "offline_access")
            .count(),
        1
    );
}

#[tokio::test]
async fn feishu_auth_start_capability_can_expand_read_and_write_scope_bundles() {
    let temp_dir = temp_feishu_cli_dir("auth-start-capability-all");
    let config_path = write_sample_feishu_config(&temp_dir);
    let args = loongclaw_daemon::feishu_cli::FeishuAuthStartArgs {
        common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
            config: Some(config_path.display().to_string()),
            account: Some("feishu_main".to_owned()),
            json: true,
        },
        redirect_uri: "http://127.0.0.1:34819/callback".to_owned(),
        principal_hint: Some("operator".to_owned()),
        scopes: vec!["offline_access".to_owned()],
        capabilities: vec![loongclaw_daemon::feishu_support::FeishuAuthCapability::All],
        include_message_write: false,
    };

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_start(&args)
        .await
        .expect("execute feishu auth start");
    let state = payload.get("state").and_then(Value::as_str).expect("state");

    assert_eq!(payload["capabilities"][0], "all");

    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let stored = store
        .consume_oauth_state(state, payload["expires_at_s"].as_i64().expect("expiry") - 1)
        .expect("stored oauth state should exist");
    let scopes = stored.scope_csv.split_whitespace().collect::<Vec<_>>();

    assert!(scopes.contains(&"offline_access"));
    assert!(scopes.contains(&"docx:document:readonly"));
    assert!(scopes.contains(&"search:message"));
    assert!(scopes.contains(&"calendar:calendar:readonly"));
    assert!(scopes.contains(&"im:message"));
    assert!(scopes.contains(&"im:message:send_as_bot"));
}

#[tokio::test]
async fn feishu_auth_exchange_sets_selected_open_id_for_new_grant() {
    let temp_dir = temp_feishu_cli_dir("auth-exchange-select");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/authen/v2/oauth/token",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "access_token": "u-token-new",
                            "refresh_token": "r-token-new",
                            "expires_in": 7200,
                            "refresh_token_expires_in": 2592000,
                            "scope": "offline_access docx:document:readonly im:message:readonly"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/authen/v1/user_info",
            get({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "name": "Alice",
                                "open_id": "ou_123",
                                "union_id": "on_456",
                                "user_id": "u_789",
                                "tenant_key": "tenant_x"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_oauth_state_record(&mvp::channel::feishu::api::FeishuOauthStateRecord {
            state: "state-123".to_owned(),
            account_id: "feishu_main".to_owned(),
            principal_hint: "operator".to_owned(),
            scope_csv: "offline_access docx:document:readonly im:message:readonly".to_owned(),
            redirect_uri: Some("http://127.0.0.1:34819/callback".to_owned()),
            code_verifier: Some("verifier-123".to_owned()),
            expires_at_s: now_s + 600,
            created_at_s: now_s,
        })
        .expect("seed oauth state");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_exchange(
        &loongclaw_daemon::feishu_cli::FeishuAuthExchangeArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
            state: "state-123".to_owned(),
            code: "code-123".to_owned(),
        },
    )
    .await
    .expect("execute feishu auth exchange");

    assert_eq!(payload["selected_open_id"], "ou_123");
    assert_eq!(
        store
            .load_selected_grant("feishu_main")
            .expect("load selected grant")
            .as_deref(),
        Some("ou_123")
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/open-apis/authen/v2/oauth/token");
    assert!(requests[0].body.contains("\"code\":\"code-123\""));
    assert!(
        requests[0]
            .body
            .contains("\"code_verifier\":\"verifier-123\"")
    );
    assert_eq!(requests[1].path, "/open-apis/authen/v1/user_info");
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer u-token-new")
    );

    server.abort();
}

#[tokio::test]
async fn feishu_whoami_refreshes_expired_grant_and_updates_store() {
    let temp_dir = temp_feishu_cli_dir("whoami-refresh");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/authen/v2/oauth/token",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "access_token": "u-token-refreshed",
                            "refresh_token": "r-token-next",
                            "expires_in": 7200,
                            "refresh_token_expires_in": 2592000,
                            "scope": "offline_access docx:document:readonly im:message:readonly im:message.group_msg:readonly search:message calendar:calendar:readonly"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/authen/v1/user_info",
            get({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "name": "Alice",
                                "open_id": "ou_123",
                                "union_id": "on_456",
                                "user_id": "u_789",
                                "tenant_key": "tenant_x"
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut expired_grant =
        sample_grant("feishu_main", "ou_123", "u-token-old", "r-token-old", now_s);
    expired_grant.access_expires_at_s = now_s - 10;
    expired_grant.refresh_expires_at_s = now_s + 86_400;
    store
        .save_grant(&expired_grant)
        .expect("seed expired grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_whoami(
        &loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
            open_id: Some("ou_123".to_owned()),
        },
    )
    .await
    .expect("execute feishu whoami");

    assert_eq!(payload["principal"]["open_id"], "ou_123");
    assert_eq!(payload["principal"]["name"], "Alice");

    let stored = store
        .load_grant("feishu_main", "ou_123")
        .expect("load refreshed grant")
        .expect("grant should exist");
    assert_eq!(stored.access_token, "u-token-refreshed");
    assert_eq!(stored.refresh_token, "r-token-next");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/open-apis/authen/v2/oauth/token");
    assert!(
        requests[0]
            .body
            .contains("\"grant_type\":\"refresh_token\"")
    );
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer u-token-refreshed")
    );

    server.abort();
}

#[tokio::test]
async fn feishu_whoami_includes_configured_account_in_payload_for_account_alias() {
    let temp_dir = temp_feishu_cli_dir("whoami-configured-account");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new().route(
        "/open-apis/authen/v1/user_info",
        get({
            let state = state.clone();
            move |request| {
                let state = state.clone();
                async move {
                    record_request(State(state), request).await;
                    Json(json!({
                        "code": 0,
                        "data": {
                            "name": "Alice",
                            "open_id": "ou_123",
                            "union_id": "on_456",
                            "user_id": "u_789",
                            "tenant_key": "tenant_x"
                        }
                    }))
                }
            }
        }),
    );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_account_alias_and_base_url(
        &temp_dir,
        "work",
        "feishu_secondary",
        &base_url,
    );
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_grant(
            "feishu_secondary",
            "ou_123",
            "u-token",
            "r-token",
            now_s,
        ))
        .expect("seed alias grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_whoami(
        &loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("work".to_owned()),
                json: true,
            },
            open_id: Some("ou_123".to_owned()),
        },
    )
    .await
    .expect("execute feishu whoami");

    assert_eq!(payload["account_id"], "feishu_secondary");
    assert_eq!(payload["configured_account"], "work");
    assert_eq!(payload["principal"]["open_id"], "ou_123");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/open-apis/authen/v1/user_info");
    assert_eq!(requests[0].authorization.as_deref(), Some("Bearer u-token"));

    server.abort();
}

#[tokio::test]
async fn feishu_whoami_accepts_unique_runtime_account_id_for_configured_alias() {
    let temp_dir = temp_feishu_cli_dir("whoami-runtime-account-alias");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new().route(
        "/open-apis/authen/v1/user_info",
        get({
            let state = state.clone();
            move |request| {
                let state = state.clone();
                async move {
                    record_request(State(state), request).await;
                    Json(json!({
                        "code": 0,
                        "data": {
                            "name": "Alice",
                            "open_id": "ou_123",
                            "union_id": "on_456",
                            "user_id": "u_789",
                            "tenant_key": "tenant_x"
                        }
                    }))
                }
            }
        }),
    );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_account_alias_and_base_url(
        &temp_dir,
        "work",
        "feishu_secondary",
        &base_url,
    );
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_grant(
            "feishu_secondary",
            "ou_123",
            "u-token",
            "r-token",
            now_s,
        ))
        .expect("seed alias grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_whoami(
        &loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_secondary".to_owned()),
                json: true,
            },
            open_id: Some("ou_123".to_owned()),
        },
    )
    .await
    .expect("execute feishu whoami with runtime account alias");

    assert_eq!(payload["account_id"], "feishu_secondary");
    assert_eq!(payload["configured_account"], "work");
    assert_eq!(payload["principal"]["open_id"], "ou_123");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/open-apis/authen/v1/user_info");
    assert_eq!(requests[0].authorization.as_deref(), Some("Bearer u-token"));

    server.abort();
}

#[tokio::test]
async fn feishu_whoami_reports_ambiguous_runtime_account_id_for_multiple_configured_aliases() {
    let temp_dir = temp_feishu_cli_dir("whoami-runtime-account-ambiguous");
    fs::create_dir_all(&temp_dir).expect("create temp feishu config dir");
    let config_path = temp_dir.join("loongclaw.toml");
    let sqlite_path = temp_dir.join("feishu.sqlite3");

    let mut config = mvp::config::LoongClawConfig::default();
    config.feishu.enabled = true;
    config.feishu.accounts = BTreeMap::from([
        (
            "work".to_owned(),
            mvp::config::FeishuAccountConfig {
                account_id: Some("feishu_shared".to_owned()),
                app_id: Some(loongclaw_contracts::SecretRef::Inline(
                    "cli_work".to_owned(),
                )),
                app_secret: Some(loongclaw_contracts::SecretRef::Inline(
                    "app-secret-work".to_owned(),
                )),
                ..mvp::config::FeishuAccountConfig::default()
            },
        ),
        (
            "alerts".to_owned(),
            mvp::config::FeishuAccountConfig {
                account_id: Some("feishu_shared".to_owned()),
                app_id: Some(loongclaw_contracts::SecretRef::Inline(
                    "cli_alerts".to_owned(),
                )),
                app_secret: Some(loongclaw_contracts::SecretRef::Inline(
                    "app-secret-alerts".to_owned(),
                )),
                ..mvp::config::FeishuAccountConfig::default()
            },
        ),
    ]);
    config.feishu.default_account = Some("work".to_owned());
    config.feishu_integration.sqlite_path = sqlite_path.display().to_string();
    mvp::config::write(config_path.to_str(), &config, true).expect("write sample feishu config");

    let error = loongclaw_daemon::feishu_cli::execute_feishu_whoami(
        &loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_shared".to_owned()),
                json: true,
            },
            open_id: Some("ou_123".to_owned()),
        },
    )
    .await
    .expect_err("ambiguous runtime account should fail");

    assert!(error.contains("requested Feishu runtime account `feishu_shared` is ambiguous"));
    assert!(error.contains("Use configured_account_id `alerts` or `work` to disambiguate"));
    assert!(error.contains("work"));
    assert!(error.contains("alerts"));
    assert!(error.contains("--account"));
}

#[tokio::test]
async fn feishu_auth_list_reports_multiple_grants_for_account() {
    let temp_dir = temp_feishu_cli_dir("auth-list");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    let mut first = sample_grant(
        "feishu_main",
        "ou_123",
        "u-token-1",
        "r-token-1",
        now_s - 60,
    );
    first.principal.name = Some("Alice".to_owned());
    first.access_expires_at_s = now_s - 30;
    first.refresh_expires_at_s = now_s + 86_400;
    first.refreshed_at_s = now_s - 60;
    store.save_grant(&first).expect("seed first grant");

    let mut second = sample_grant("feishu_main", "ou_456", "u-token-2", "r-token-2", now_s);
    second.principal.name = Some("Bob".to_owned());
    second.refreshed_at_s = now_s;
    store.save_grant(&second).expect("seed second grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_list(
        &loongclaw_daemon::feishu_cli::FeishuAuthListArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
        },
    )
    .await
    .expect("execute feishu auth list");

    assert_eq!(payload["account_id"], "feishu_main");
    assert_eq!(payload["grant_count"], 2);
    assert_eq!(payload["selected_open_id"], Value::Null);
    assert_eq!(payload["recommendations"]["selection_required"], true);
    assert_eq!(
        payload["recommendations"]["select_command"],
        "loong feishu auth select --account feishu_main --open-id <open_id>"
    );
    assert_eq!(payload["grants"][0]["selected"], false);
    assert_eq!(payload["grants"][0]["principal"]["open_id"], "ou_456");
    assert_eq!(payload["grants"][0]["principal"]["name"], "Bob");
    assert_eq!(
        payload["grants"][0]["status"]["access_token_expired"],
        false
    );
    assert_eq!(payload["grants"][1]["principal"]["open_id"], "ou_123");
    assert_eq!(payload["grants"][1]["status"]["access_token_expired"], true);
    assert!(
        payload["grants"][1]["status"]["missing_scopes"]
            .as_array()
            .is_some_and(|values| values.is_empty())
    );
}

#[tokio::test]
async fn feishu_auth_list_marks_single_grant_as_effectively_selected() {
    let temp_dir = temp_feishu_cli_dir("auth-list-single-effective");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token-1",
            "r-token-1",
            now_s,
        ))
        .expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_list(
        &loongclaw_daemon::feishu_cli::FeishuAuthListArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
        },
    )
    .await
    .expect("execute feishu auth list");

    assert_eq!(payload["selected_open_id"], Value::Null);
    assert_eq!(payload["effective_open_id"], "ou_123");
    assert_eq!(payload["grants"][0]["selected"], false);
    assert_eq!(payload["grants"][0]["effective_selected"], true);
    assert_eq!(payload["grants"][0]["principal"]["open_id"], "ou_123");
}

#[tokio::test]
async fn feishu_auth_list_clears_stale_selected_open_id_without_false_selected_flags() {
    let temp_dir = temp_feishu_cli_dir("auth-list-stale-selection");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token-1",
            "r-token-1",
            now_s,
        ))
        .expect("seed first grant");
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_456",
            "u-token-2",
            "r-token-2",
            now_s + 1,
        ))
        .expect("seed second grant");
    store
        .set_selected_grant("feishu_main", "ou_missing", now_s + 2)
        .expect("persist stale selected grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_list(
        &loongclaw_daemon::feishu_cli::FeishuAuthListArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
        },
    )
    .await
    .expect("execute feishu auth list");

    assert_eq!(payload["selected_open_id"], Value::Null);
    assert_eq!(
        payload["recommendations"]["stale_selected_open_id"],
        "ou_missing"
    );
    assert_eq!(payload["recommendations"]["selection_required"], true);
    assert!(
        payload["grants"]
            .as_array()
            .expect("grants array")
            .iter()
            .all(|grant| {
                grant["selected"] == Value::Bool(false)
                    && grant["effective_selected"] == Value::Bool(false)
            })
    );
}

#[tokio::test]
async fn feishu_auth_select_persists_selected_grant_for_account() {
    let temp_dir = temp_feishu_cli_dir("auth-select");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token-1",
            "r-token-1",
            now_s,
        ))
        .expect("seed first grant");
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_456",
            "u-token-2",
            "r-token-2",
            now_s + 1,
        ))
        .expect("seed second grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_select(
        &loongclaw_daemon::feishu_cli::FeishuAuthSelectArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
            open_id: "ou_456".to_owned(),
        },
    )
    .await
    .expect("execute feishu auth select");

    assert_eq!(payload["account_id"], "feishu_main");
    assert_eq!(payload["selected_open_id"], "ou_456");
    assert_eq!(payload["grant"]["principal"]["open_id"], "ou_456");
    assert_eq!(
        store
            .load_selected_grant("feishu_main")
            .expect("load selected grant")
            .as_deref(),
        Some("ou_456")
    );
}

#[tokio::test]
async fn feishu_auth_select_uses_configured_account_in_missing_grant_error() {
    let temp_dir = temp_feishu_cli_dir("auth-select-configured-account-error");
    let config_path =
        write_sample_feishu_config_with_account_alias(&temp_dir, "work", "feishu_secondary");

    let error = loongclaw_daemon::feishu_cli::execute_feishu_auth_select(
        &loongclaw_daemon::feishu_cli::FeishuAuthSelectArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("work".to_owned()),
                json: true,
            },
            open_id: "ou_missing".to_owned(),
        },
    )
    .await
    .expect_err("select should fail for an unknown explicit open_id");

    assert!(error.contains("account `work`"));
    assert!(error.contains("loong feishu auth list --account work"));
    assert!(!error.contains("feishu_secondary"));
}

#[tokio::test]
async fn feishu_auth_select_includes_configured_account_in_payload() {
    let temp_dir = temp_feishu_cli_dir("auth-select-configured-account-payload");
    let config_path =
        write_sample_feishu_config_with_account_alias(&temp_dir, "work", "feishu_secondary");
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    store
        .save_grant(&sample_grant(
            "feishu_secondary",
            "ou_123",
            "u-token-1",
            "r-token-1",
            now_s,
        ))
        .expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_select(
        &loongclaw_daemon::feishu_cli::FeishuAuthSelectArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("work".to_owned()),
                json: true,
            },
            open_id: "ou_123".to_owned(),
        },
    )
    .await
    .expect("execute feishu auth select");

    assert_eq!(payload["account_id"], "feishu_secondary");
    assert_eq!(payload["configured_account"], "work");
    assert_eq!(payload["selected_open_id"], "ou_123");
}

#[tokio::test]
async fn feishu_auth_status_without_open_id_summarizes_multiple_grants() {
    let temp_dir = temp_feishu_cli_dir("auth-status-multi");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    let mut first = sample_grant(
        "feishu_main",
        "ou_123",
        "u-token-1",
        "r-token-1",
        now_s - 60,
    );
    first.principal.name = Some("Alice".to_owned());
    first.access_expires_at_s = now_s - 30;
    first.refreshed_at_s = now_s - 60;
    store.save_grant(&first).expect("seed first grant");

    let mut second = sample_grant("feishu_main", "ou_456", "u-token-2", "r-token-2", now_s);
    second.principal.name = Some("Bob".to_owned());
    second.refreshed_at_s = now_s;
    store.save_grant(&second).expect("seed second grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_status(
        &loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
            open_id: None,
        },
    )
    .await
    .expect("execute feishu auth status");

    assert_eq!(payload["status_scope"], "account");
    assert_eq!(payload["grant_count"], 2);
    assert_eq!(payload["recommendations"]["selection_required"], true);
    assert_eq!(
        payload["recommendations"]["select_command"],
        "loong feishu auth select --account feishu_main --open-id <open_id>"
    );
    assert_eq!(payload["grants"][0]["principal"]["open_id"], "ou_456");
    assert_eq!(payload["grants"][1]["principal"]["open_id"], "ou_123");
    assert_eq!(payload["grants"][1]["status"]["access_token_expired"], true);
    assert_eq!(payload["grants"][0]["doc_write_status"]["ready"], false);
    assert_eq!(
        payload["grants"][0]["recommendations"]["missing_doc_write_scope"],
        true
    );
    assert_eq!(payload["grants"][0]["message_write_status"]["ready"], false);
    assert_eq!(
        payload["grants"][0]["message_write_status"]["accepted_scopes"][1],
        "im:message:send_as_bot"
    );
    assert_eq!(
        payload["grants"][0]["recommendations"]["missing_message_write_scope"],
        true
    );
    assert_eq!(
        payload["grants"][0]["recommendations"]["auth_start_command"],
        "loong feishu auth start --account feishu_main --capability doc-write --capability message-write"
    );
}

#[tokio::test]
async fn feishu_auth_status_without_open_id_uses_effective_selected_grant_when_present() {
    let temp_dir = temp_feishu_cli_dir("auth-status-selected-default");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token-1",
            "r-token-1",
            now_s,
        ))
        .expect("seed first grant");
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_456",
            "u-token-2",
            "r-token-2",
            now_s + 1,
        ))
        .expect("seed second grant");
    store
        .set_selected_grant("feishu_main", "ou_456", now_s + 2)
        .expect("persist selected grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_status(
        &loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
            open_id: None,
        },
    )
    .await
    .expect("execute feishu auth status");

    assert_eq!(payload["status_scope"], "grant");
    assert_eq!(payload["selected_open_id"], "ou_456");
    assert_eq!(payload["effective_open_id"], "ou_456");
    assert_eq!(payload["grant"]["principal"]["open_id"], "ou_456");
    assert_eq!(payload["grant"]["selected"], true);
    assert_eq!(payload["grant"]["effective_selected"], true);
}

#[tokio::test]
async fn feishu_auth_status_account_scope_clears_stale_selected_open_id() {
    let temp_dir = temp_feishu_cli_dir("auth-status-stale-selection");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token-1",
            "r-token-1",
            now_s,
        ))
        .expect("seed first grant");
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_456",
            "u-token-2",
            "r-token-2",
            now_s + 1,
        ))
        .expect("seed second grant");
    store
        .set_selected_grant("feishu_main", "ou_missing", now_s + 2)
        .expect("persist stale selected grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_status(
        &loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
            open_id: None,
        },
    )
    .await
    .expect("execute feishu auth status");

    assert_eq!(payload["status_scope"], "account");
    assert_eq!(payload["selected_open_id"], Value::Null);
    assert_eq!(
        payload["recommendations"]["stale_selected_open_id"],
        "ou_missing"
    );
    assert_eq!(payload["recommendations"]["selection_required"], true);
}

#[tokio::test]
async fn feishu_auth_status_recommends_account_scoped_reauthorize_for_missing_write_scope() {
    let temp_dir = temp_feishu_cli_dir("auth-status-write-remediation");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    let mut grant = sample_grant("feishu_main", "ou_123", "u-token-1", "r-token-1", now_s);
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "docx:document:readonly",
        "im:message:readonly",
    ]);
    store.save_grant(&grant).expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_status(
        &loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
            open_id: Some("ou_123".to_owned()),
        },
    )
    .await
    .expect("execute feishu auth status");

    assert_eq!(payload["status_scope"], "grant");
    assert_eq!(payload["doc_write_status"]["ready"], false);
    assert_eq!(payload["recommendations"]["missing_doc_write_scope"], true);
    assert_eq!(payload["message_write_status"]["ready"], false);
    assert_eq!(
        payload["recommendations"]["missing_message_write_scope"],
        true
    );
    assert_eq!(
        payload["recommendations"]["auth_start_command"],
        "loong feishu auth start --account feishu_main --capability doc-write --capability message-write"
    );
}

#[tokio::test]
async fn feishu_auth_status_without_grant_recommends_readonly_auth_start() {
    let temp_dir = temp_feishu_cli_dir("auth-status-no-grant");
    let config_path = write_sample_feishu_config(&temp_dir);

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_status(
        &loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
            open_id: Some("ou_missing".to_owned()),
        },
    )
    .await
    .expect("execute feishu auth status");

    assert_eq!(payload["status_scope"], "grant");
    assert_eq!(payload["status"]["has_grant"], false);
    assert_eq!(
        payload["recommendations"]["auth_start_command"],
        "loong feishu auth start --account feishu_main"
    );
    assert_eq!(
        payload["recommendations"]["missing_message_write_scope"],
        false
    );
    assert_eq!(payload["recommendations"]["missing_doc_write_scope"], false);
}

#[tokio::test]
async fn feishu_auth_status_with_missing_open_id_reports_available_grants() {
    let temp_dir = temp_feishu_cli_dir("auth-status-missing-open-id");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token-1",
            "r-token-1",
            now_s,
        ))
        .expect("seed first grant");
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_456",
            "u-token-2",
            "r-token-2",
            now_s + 1,
        ))
        .expect("seed second grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_status(
        &loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
            open_id: Some("ou_missing".to_owned()),
        },
    )
    .await
    .expect("execute feishu auth status");

    assert_eq!(payload["status_scope"], "grant");
    assert_eq!(payload["requested_open_id"], "ou_missing");
    assert_eq!(payload["status"]["has_grant"], false);
    assert_eq!(
        payload["recommendations"]["select_command"],
        "loong feishu auth select --account feishu_main --open-id <open_id>"
    );
    assert_eq!(
        payload["recommendations"]["requested_open_id_missing"],
        true
    );
    assert_eq!(
        payload["recommendations"]["auth_start_command"],
        Value::Null
    );
    assert_eq!(payload["available_open_ids"][0], "ou_456");
    assert_eq!(payload["available_open_ids"][1], "ou_123");
}

#[tokio::test]
async fn feishu_auth_revoke_reports_missing_explicit_open_id() {
    let temp_dir = temp_feishu_cli_dir("auth-revoke-missing-open-id");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token-1",
            "r-token-1",
            now_s,
        ))
        .expect("seed grant");

    let error = loongclaw_daemon::feishu_cli::execute_feishu_auth_revoke(
        &loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
            open_id: Some("ou_missing".to_owned()),
        },
    )
    .await
    .expect_err("revoke should fail for an unknown explicit open_id");

    assert!(error.contains("open_id `ou_missing`"));
    assert!(error.contains("ou_123"));
    assert!(error.contains("loong feishu auth select --account feishu_main --open-id <open_id>"));
}

#[tokio::test]
async fn feishu_auth_revoke_reports_remaining_effective_grant_after_deleting_selected_grant() {
    let temp_dir = temp_feishu_cli_dir("auth-revoke-selected-single-remaining");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token-1",
            "r-token-1",
            now_s,
        ))
        .expect("seed first grant");
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_456",
            "u-token-2",
            "r-token-2",
            now_s + 1,
        ))
        .expect("seed second grant");
    store
        .set_selected_grant("feishu_main", "ou_456", now_s + 2)
        .expect("persist selected grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_revoke(
        &loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
            open_id: Some("ou_456".to_owned()),
        },
    )
    .await
    .expect("revoke selected grant");

    assert_eq!(payload["deleted"], true);
    assert_eq!(payload["open_id"], "ou_456");
    assert_eq!(payload["grant_count"], 1);
    assert_eq!(payload["selected_open_id"], Value::Null);
    assert_eq!(payload["effective_open_id"], "ou_123");
    assert_eq!(payload["recommendations"]["selection_required"], false);
    assert_eq!(payload["recommendations"]["select_command"], Value::Null);
}

#[tokio::test]
async fn feishu_auth_revoke_reports_reselection_needed_when_multiple_grants_remain() {
    let temp_dir = temp_feishu_cli_dir("auth-revoke-selected-multi-remaining");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token-1",
            "r-token-1",
            now_s,
        ))
        .expect("seed first grant");
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_456",
            "u-token-2",
            "r-token-2",
            now_s + 1,
        ))
        .expect("seed second grant");
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_789",
            "u-token-3",
            "r-token-3",
            now_s + 2,
        ))
        .expect("seed third grant");
    store
        .set_selected_grant("feishu_main", "ou_789", now_s + 3)
        .expect("persist selected grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_auth_revoke(
        &loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
            open_id: Some("ou_789".to_owned()),
        },
    )
    .await
    .expect("revoke selected grant");

    assert_eq!(payload["deleted"], true);
    assert_eq!(payload["open_id"], "ou_789");
    assert_eq!(payload["grant_count"], 2);
    assert_eq!(payload["selected_open_id"], Value::Null);
    assert_eq!(payload["effective_open_id"], Value::Null);
    assert_eq!(payload["recommendations"]["selection_required"], true);
    assert_eq!(
        payload["recommendations"]["select_command"],
        "loong feishu auth select --account feishu_main --open-id <open_id>"
    );
}

#[tokio::test]
async fn feishu_whoami_requires_open_id_when_multiple_grants_exist_without_selection() {
    let temp_dir = temp_feishu_cli_dir("whoami-multi-grant");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token-1",
            "r-token-1",
            now_s,
        ))
        .expect("seed first grant");
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_456",
            "u-token-2",
            "r-token-2",
            now_s + 1,
        ))
        .expect("seed second grant");

    let error = loongclaw_daemon::feishu_cli::execute_feishu_whoami(
        &loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
            open_id: None,
        },
    )
    .await
    .expect_err("whoami should require explicit selection when multiple grants exist");

    assert!(error.contains("multiple stored Feishu grants exist"));
    assert!(error.contains("loong feishu auth list"));
    assert!(error.contains("--open-id"));
}

#[tokio::test]
async fn feishu_whoami_reports_missing_explicit_open_id() {
    let temp_dir = temp_feishu_cli_dir("whoami-missing-open-id");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token-1",
            "r-token-1",
            now_s,
        ))
        .expect("seed grant");

    let error = loongclaw_daemon::feishu_cli::execute_feishu_whoami(
        &loongclaw_daemon::feishu_cli::FeishuGrantArgs {
            common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                config: Some(config_path.display().to_string()),
                account: Some("feishu_main".to_owned()),
                json: true,
            },
            open_id: Some("ou_missing".to_owned()),
        },
    )
    .await
    .expect_err("whoami should fail for an unknown explicit open_id");

    assert!(error.contains("open_id `ou_missing`"));
    assert!(error.contains("ou_123"));
    assert!(error.contains("loong feishu auth select --account feishu_main --open-id <open_id>"));
}

#[tokio::test]
async fn feishu_read_doc_requires_open_id_when_multiple_grants_exist_without_selection() {
    let temp_dir = temp_feishu_cli_dir("read-doc-multi-grant");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));

    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token-1",
            "r-token-1",
            now_s,
        ))
        .expect("seed first grant");
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_456",
            "u-token-2",
            "r-token-2",
            now_s + 1,
        ))
        .expect("seed second grant");

    let error = loongclaw_daemon::feishu_cli::execute_feishu_read_doc(
        &loongclaw_daemon::feishu_cli::FeishuReadDocArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: None,
            },
            url: "https://open.feishu.cn/docx/doxcnDemo".to_owned(),
            lang: Some(1),
        },
    )
    .await
    .expect_err("read doc should require explicit selection when multiple grants exist");

    assert!(error.contains("multiple stored Feishu grants exist"));
    assert!(error.contains("loong feishu auth list"));
    assert!(error.contains("--open-id"));
}

#[tokio::test]
async fn feishu_calendar_freebusy_uses_open_id_type_for_implicit_selected_user() {
    let temp_dir = temp_feishu_cli_dir("calendar-freebusy-default-user");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new().route(
        "/open-apis/calendar/v4/freebusy/list",
        post({
            let state = state.clone();
            move |request| {
                let state = state.clone();
                async move {
                    record_request(State(state), request).await;
                    Json(json!({
                        "code": 0,
                        "data": {
                            "freebusy_list": [{
                                "start_time": "2026-03-12T09:00:00+08:00",
                                "end_time": "2026-03-12T10:00:00+08:00",
                                "rsvp_status": "busy"
                            }]
                        }
                    }))
                }
            }
        }),
    );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token",
            "r-token",
            now_s,
        ))
        .expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_calendar_freebusy(
        &loongclaw_daemon::feishu_cli::FeishuCalendarFreebusyArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            user_id_type: None,
            time_min: "2026-03-12T09:00:00+08:00".to_owned(),
            time_max: "2026-03-12T10:00:00+08:00".to_owned(),
            user_id: None,
            room_id: None,
            include_external_calendar: Some(true),
            only_busy: Some(true),
            need_rsvp_status: Some(true),
        },
    )
    .await
    .expect("execute freebusy");

    assert_eq!(payload["result"]["freebusy_list"][0]["rsvp_status"], "busy");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].authorization.as_deref(), Some("Bearer u-token"));
    assert!(
        requests[0]
            .query
            .as_deref()
            .is_some_and(|query| query.contains("user_id_type=open_id")),
        "freebusy query should default user_id_type=open_id when using selected grant open_id"
    );
    assert!(requests[0].body.contains("\"user_id\":\"ou_123\""));

    server.abort();
}

#[tokio::test]
async fn feishu_messages_history_fetches_tenant_token_before_im_request() {
    let temp_dir = temp_feishu_cli_dir("messages-history-tenant-token");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "tenant_access_token": "t-token"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            get({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "items": [{
                                    "message_id": "om_1",
                                    "chat_id": "oc_1",
                                    "root_id": "om_root_1",
                                    "parent_id": "om_parent_1",
                                    "msg_type": "text",
                                    "create_time": "1700000000",
                                    "update_time": "1700000001"
                                }],
                                "page_token": "next-page",
                                "has_more": true
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token",
            "r-token",
            now_s,
        ))
        .expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_messages_history(
        &loongclaw_daemon::feishu_cli::FeishuMessagesHistoryArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            container_id_type: "chat".to_owned(),
            container_id: "oc_1".to_owned(),
            start_time: None,
            end_time: None,
            sort_type: None,
            page_size: Some(20),
            page_token: None,
        },
    )
    .await
    .expect("execute message history");

    assert_eq!(payload["page"]["items"][0]["message_id"], "om_1");
    assert_eq!(payload["page"]["has_more"], true);

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].path,
        "/open-apis/auth/v3/tenant_access_token/internal"
    );
    assert!(requests[0].body.contains("\"app_id\":\"cli_a1b2c3\""));
    assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
    assert_eq!(requests[1].authorization.as_deref(), Some("Bearer t-token"));
    assert!(requests[1].query.as_deref().is_some_and(|query| {
        query.contains("container_id_type=chat") && query.contains("container_id=oc_1")
    }));

    server.abort();
}

#[tokio::test]
async fn feishu_messages_history_includes_configured_account_in_payload_for_account_alias() {
    let temp_dir = temp_feishu_cli_dir("messages-history-configured-account");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "tenant_access_token": "t-token"
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages",
            get({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "items": [{
                                    "message_id": "om_1",
                                    "chat_id": "oc_1",
                                    "root_id": "om_root_1",
                                    "parent_id": "om_parent_1",
                                    "msg_type": "text",
                                    "create_time": "1700000000",
                                    "update_time": "1700000001"
                                }],
                                "page_token": "next-page",
                                "has_more": true
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_account_alias_and_base_url(
        &temp_dir,
        "work",
        "feishu_secondary",
        &base_url,
    );
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_grant(
            "feishu_secondary",
            "ou_123",
            "u-token",
            "r-token",
            now_s,
        ))
        .expect("seed alias grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_messages_history(
        &loongclaw_daemon::feishu_cli::FeishuMessagesHistoryArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("work".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            container_id_type: "chat".to_owned(),
            container_id: "oc_1".to_owned(),
            start_time: None,
            end_time: None,
            sort_type: None,
            page_size: Some(20),
            page_token: None,
        },
    )
    .await
    .expect("execute message history");

    assert_eq!(payload["account_id"], "feishu_secondary");
    assert_eq!(payload["configured_account"], "work");
    assert_eq!(payload["page"]["items"][0]["message_id"], "om_1");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].path,
        "/open-apis/auth/v3/tenant_access_token/internal"
    );
    assert_eq!(requests[1].path, "/open-apis/im/v1/messages");
    assert_eq!(requests[1].authorization.as_deref(), Some("Bearer t-token"));

    server.abort();
}

#[tokio::test]
async fn feishu_search_messages_uses_user_grant_token_directly() {
    let temp_dir = temp_feishu_cli_dir("search-messages-user-token");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new().route(
        "/open-apis/search/v2/message",
        post({
            let state = state.clone();
            move |request| {
                let state = state.clone();
                async move {
                    record_request(State(state), request).await;
                    Json(json!({
                        "code": 0,
                        "data": {
                            "items": ["om_1", "om_2"],
                            "page_token": "next-search",
                            "has_more": true
                        }
                    }))
                }
            }
        }),
    );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token",
            "r-token",
            now_s,
        ))
        .expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_search_messages(
        &loongclaw_daemon::feishu_cli::FeishuSearchMessagesArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            query: "incident".to_owned(),
            user_id_type: Some("open_id".to_owned()),
            from_ids: vec!["ou_123".to_owned()],
            chat_ids: vec!["oc_1".to_owned()],
            at_chatter_ids: Vec::new(),
            message_type: Some("text".to_owned()),
            from_type: None,
            chat_type: None,
            start_time: None,
            end_time: None,
            page_size: Some(10),
            page_token: None,
        },
    )
    .await
    .expect("execute message search");

    assert_eq!(payload["page"]["items"][0], "om_1");
    assert_eq!(payload["page"]["has_more"], true);

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/open-apis/search/v2/message");
    assert_eq!(requests[0].authorization.as_deref(), Some("Bearer u-token"));
    assert!(requests[0].query.as_deref().is_some_and(
        |query| query.contains("user_id_type=open_id") && query.contains("page_size=10")
    ));
    assert!(requests[0].body.contains("\"query\":\"incident\""));
    assert!(requests[0].body.contains("\"chat_ids\":[\"oc_1\"]"));

    server.abort();
}

#[tokio::test]
async fn feishu_read_doc_uses_user_grant_token_directly() {
    let temp_dir = temp_feishu_cli_dir("read-doc-user-token");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new().route(
        "/open-apis/docx/v1/documents/doxcnDemo/raw_content",
        get({
            let state = state.clone();
            move |request| {
                let state = state.clone();
                async move {
                    record_request(State(state), request).await;
                    Json(json!({
                        "code": 0,
                        "data": {
                            "content": "hello from docs"
                        }
                    }))
                }
            }
        }),
    );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token",
            "r-token",
            now_s,
        ))
        .expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_read_doc(
        &loongclaw_daemon::feishu_cli::FeishuReadDocArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            url: "https://open.feishu.cn/docx/doxcnDemo".to_owned(),
            lang: Some(1),
        },
    )
    .await
    .expect("execute read doc");

    assert_eq!(payload["document"]["document_id"], "doxcnDemo");
    assert_eq!(payload["document"]["content"], "hello from docs");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].path,
        "/open-apis/docx/v1/documents/doxcnDemo/raw_content"
    );
    assert_eq!(requests[0].authorization.as_deref(), Some("Bearer u-token"));
    assert!(
        requests[0]
            .query
            .as_deref()
            .is_some_and(|query| query.contains("lang=1"))
    );

    server.abort();
}

#[tokio::test]
async fn feishu_doc_create_uses_user_grant_token_and_inserts_initial_content() {
    let temp_dir = temp_feishu_cli_dir("doc-create-user-token");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/docx/v1/documents",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "document": {
                                    "document_id": "doxcnCreated",
                                    "revision_id": 1,
                                    "title": "Release Plan"
                                }
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/blocks/convert",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "first_level_block_ids": ["tmp-heading"],
                                "blocks": [{
                                    "block_id": "tmp-heading",
                                    "block_type": 3,
                                    "heading1": {
                                        "elements": [{"text_run": {"content": "Release Plan"}}]
                                    },
                                    "children": []
                                }]
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnCreated/blocks/doxcnCreated/descendant",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": [{
                                    "block_id": "blk_real_heading",
                                    "temporary_block_id": "tmp-heading"
                                }]
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant(
        "feishu_main",
        "ou_123",
        "u-token-doc-create",
        "r-token",
        now_s,
    );
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "docx:document",
    ]);
    store.save_grant(&grant).expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_doc_create(
        &loongclaw_daemon::feishu_cli::FeishuDocCreateArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            title: Some("Release Plan".to_owned()),
            folder_token: None,
            content: Some("# Release Plan".to_owned()),
            content_path: None,
            content_type: Some("markdown".to_owned()),
        },
    )
    .await
    .expect("execute doc create");

    assert_eq!(payload["document"]["document_id"], "doxcnCreated");
    assert_eq!(payload["content_inserted"], true);
    assert_eq!(payload["inserted_block_count"], 1);
    assert_eq!(payload["insert_batch_count"], 1);

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer u-token-doc-create")
    );
    assert_eq!(requests[0].path, "/open-apis/docx/v1/documents");
    assert_eq!(
        serde_json::from_str::<Value>(&requests[0].body).expect("create request json"),
        json!({
            "title": "Release Plan"
        })
    );
    assert_eq!(
        requests[1].path,
        "/open-apis/docx/v1/documents/blocks/convert"
    );
    assert_eq!(
        requests[2].path,
        "/open-apis/docx/v1/documents/doxcnCreated/blocks/doxcnCreated/descendant"
    );
    assert_eq!(
        requests[2].query.as_deref(),
        Some("document_revision_id=-1")
    );

    server.abort();
}

#[tokio::test]
async fn feishu_doc_create_reports_doc_write_hint_when_only_message_write_scope_exists() {
    let temp_dir = temp_feishu_cli_dir("doc-create-missing-doc-write");
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant(
        "feishu_main",
        "ou_123",
        "u-token-doc-create",
        "r-token",
        now_s,
    );

    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "im:message",
    ]);

    store.save_grant(&grant).expect("seed grant");

    let error = loongclaw_daemon::feishu_cli::execute_feishu_doc_create(
        &loongclaw_daemon::feishu_cli::FeishuDocCreateArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            title: Some("Release Plan".to_owned()),
            folder_token: None,
            content: Some("# Release Plan".to_owned()),
            content_path: None,
            content_type: Some("markdown".to_owned()),
        },
    )
    .await
    .expect_err("doc create should reject grants without a confirmed doc write scope");

    assert!(
        error
            .contains("loong feishu doc create requires at least one Feishu scope [docx:document]"),
        "error={error}"
    );
    assert!(
        error.contains("loong feishu auth start --account feishu_main --capability doc-write"),
        "error={error}"
    );
    assert!(
        !error.contains("--capability message-write"),
        "error={error}"
    );
}

#[tokio::test]
async fn feishu_doc_create_reads_content_path_and_infers_markdown_type() {
    let temp_dir = temp_feishu_cli_dir("doc-create-content-path");
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let content_path = temp_dir.join("fixtures/release-plan.md");
    std::fs::create_dir_all(content_path.parent().expect("content path parent"))
        .expect("create content parent");
    std::fs::write(&content_path, "# Release Plan").expect("write content fixture");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/docx/v1/documents",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "document": {
                                    "document_id": "doxcnCreated",
                                    "revision_id": 1,
                                    "title": "Release Plan",
                                    "url": "https://open.feishu.cn/docx/doxcnCreated"
                                }
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/blocks/convert",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "first_level_block_ids": ["tmp-heading"],
                                "blocks": [{
                                    "block_id": "tmp-heading",
                                    "block_type": 3,
                                    "heading1": {
                                        "elements": [{"text_run": {"content": "Release Plan"}}]
                                    },
                                    "children": []
                                }]
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnCreated/blocks/doxcnCreated/descendant",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": [{
                                    "block_id": "blk_real_heading",
                                    "temporary_block_id": "tmp-heading"
                                }]
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant(
        "feishu_main",
        "ou_123",
        "u-token-doc-create-path",
        "r-token",
        now_s,
    );
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "docx:document",
    ]);
    store.save_grant(&grant).expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_doc_create(
        &loongclaw_daemon::feishu_cli::FeishuDocCreateArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            title: Some("Release Plan".to_owned()),
            folder_token: None,
            content: None,
            content_path: Some(content_path.display().to_string()),
            content_type: None,
        },
    )
    .await
    .expect("execute doc create from content path");

    assert_eq!(payload["content_inserted"], true);
    assert_eq!(payload["content_type"], "markdown");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        serde_json::from_str::<Value>(&requests[1].body).expect("convert request json"),
        json!({
            "content_type": "markdown",
            "content": "# Release Plan"
        })
    );

    server.abort();
}

#[tokio::test]
async fn feishu_doc_append_uses_user_grant_token_and_appends_content() {
    let temp_dir = temp_feishu_cli_dir("doc-append-user-token");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/docx/v1/documents/blocks/convert",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "first_level_block_ids": ["tmp-paragraph"],
                                "blocks": [{
                                    "block_id": "tmp-paragraph",
                                    "block_type": 2,
                                    "text": {
                                        "elements": [{"text_run": {"content": "Follow-up note"}}]
                                    },
                                    "children": []
                                }]
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": [{
                                    "block_id": "blk_real_paragraph",
                                    "temporary_block_id": "tmp-paragraph"
                                }]
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant(
        "feishu_main",
        "ou_123",
        "u-token-doc-append",
        "r-token",
        now_s,
    );
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "docx:document",
    ]);
    store.save_grant(&grant).expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_doc_append(
        &loongclaw_daemon::feishu_cli::FeishuDocAppendArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            url: "https://open.feishu.cn/docx/doxcnExisting".to_owned(),
            content: Some("Follow-up note".to_owned()),
            content_path: None,
            content_type: None,
        },
    )
    .await
    .expect("execute doc append");

    assert_eq!(payload["document"]["document_id"], "doxcnExisting");
    assert_eq!(payload["inserted_block_count"], 1);
    assert_eq!(payload["insert_batch_count"], 1);
    assert_eq!(payload["content_type"], "markdown");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer u-token-doc-append")
    );
    assert_eq!(
        requests[0].path,
        "/open-apis/docx/v1/documents/blocks/convert"
    );
    assert_eq!(
        requests[1].path,
        "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant"
    );
    assert_eq!(
        requests[1].query.as_deref(),
        Some("document_revision_id=-1")
    );

    server.abort();
}

#[tokio::test]
async fn feishu_doc_append_supports_oversized_table_subtree() {
    let temp_dir = temp_feishu_cli_dir("doc-append-oversized-table");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let leaf_ids = (0..1000)
        .map(|index| format!("tmp-cell-1-leaf-{index:04}"))
        .collect::<Vec<_>>();
    let mut descendants = Vec::with_capacity(leaf_ids.len() + 3);
    descendants.push(json!({
        "block_id": "tmp-table",
        "block_type": 31,
        "table": {
            "property": {
                "row_size": 1,
                "column_size": 2
            }
        },
        "children": ["tmp-cell-1", "tmp-cell-2"]
    }));
    descendants.push(json!({
        "block_id": "tmp-cell-1",
        "block_type": 32,
        "table_cell": {},
        "children": leaf_ids
    }));
    descendants.push(json!({
        "block_id": "tmp-cell-2",
        "block_type": 32,
        "table_cell": {},
        "children": []
    }));
    descendants.extend(leaf_ids.iter().map(|block_id| {
        json!({
            "block_id": block_id,
            "block_type": 2,
            "text": {
                "elements": [{"text_run": {"content": block_id}}]
            },
            "children": []
        })
    }));
    let convert_response = json!({
        "code": 0,
        "data": {
            "first_level_block_ids": ["tmp-table"],
            "blocks": descendants
        }
    });
    let router = Router::new()
        .route(
            "/open-apis/docx/v1/documents/blocks/convert",
            post({
                let state = state.clone();
                let convert_response = convert_response.clone();
                move |request| {
                    let state = state.clone();
                    let convert_response = convert_response.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(convert_response)
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "children": [{
                                    "block_id": "blk_real_table",
                                    "block_type": 31,
                                    "children": ["blk_real_cell_1", "blk_real_cell_2"],
                                    "table": {
                                        "cells": ["blk_real_cell_1", "blk_real_cell_2"],
                                        "property": {
                                            "row_size": 1,
                                            "column_size": 2
                                        }
                                    }
                                }]
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_cell_1/descendant",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": []
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant(
        "feishu_main",
        "ou_123",
        "u-token-doc-append-oversized-table",
        "r-token",
        now_s,
    );
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "docx:document",
    ]);
    store.save_grant(&grant).expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_doc_append(
        &loongclaw_daemon::feishu_cli::FeishuDocAppendArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            url: "https://open.feishu.cn/docx/doxcnExisting".to_owned(),
            content: Some("oversized table subtree".to_owned()),
            content_path: None,
            content_type: None,
        },
    )
    .await
    .expect("execute doc append oversized table subtree");

    assert_eq!(payload["document"]["document_id"], "doxcnExisting");
    assert_eq!(payload["inserted_block_count"], 1003);
    assert_eq!(payload["insert_batch_count"], 2);

    let requests = requests.lock().await.clone();
    let paths = requests
        .iter()
        .map(|request| request.path.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![
            "/open-apis/docx/v1/documents/blocks/convert".to_owned(),
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children".to_owned(),
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_cell_1/descendant"
                .to_owned(),
        ]
    );

    server.abort();
}

#[tokio::test]
async fn feishu_doc_append_supports_oversized_callout_subtree() {
    let temp_dir = temp_feishu_cli_dir("doc-append-oversized-callout");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let leaf_ids = (0..1001)
        .map(|index| format!("tmp-callout-leaf-{index:04}"))
        .collect::<Vec<_>>();
    let mut descendants = Vec::with_capacity(leaf_ids.len() + 1);
    descendants.push(json!({
        "block_id": "tmp-callout",
        "block_type": 19,
        "callout": {
            "emoji_id": "smile"
        },
        "children": leaf_ids
    }));
    descendants.extend(leaf_ids.iter().map(|block_id| {
        json!({
            "block_id": block_id,
            "block_type": 2,
            "text": {
                "elements": [{"text_run": {"content": block_id}}]
            },
            "children": []
        })
    }));
    let convert_response = json!({
        "code": 0,
        "data": {
            "first_level_block_ids": ["tmp-callout"],
            "blocks": descendants
        }
    });
    let router = Router::new()
        .route(
            "/open-apis/docx/v1/documents/blocks/convert",
            post({
                let state = state.clone();
                let convert_response = convert_response.clone();
                move |request| {
                    let state = state.clone();
                    let convert_response = convert_response.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(convert_response)
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "children": [{
                                    "block_id": "blk_real_callout",
                                    "block_type": 19,
                                    "children": [],
                                    "callout": {
                                        "emoji_id": "smile"
                                    }
                                }]
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_callout/descendant",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": []
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant(
        "feishu_main",
        "ou_123",
        "u-token-doc-append-oversized-callout",
        "r-token",
        now_s,
    );
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "docx:document",
    ]);
    store.save_grant(&grant).expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_doc_append(
        &loongclaw_daemon::feishu_cli::FeishuDocAppendArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            url: "https://open.feishu.cn/docx/doxcnExisting".to_owned(),
            content: Some("oversized callout subtree".to_owned()),
            content_path: None,
            content_type: None,
        },
    )
    .await
    .expect("execute doc append oversized callout subtree");

    assert_eq!(payload["document"]["document_id"], "doxcnExisting");
    assert_eq!(payload["inserted_block_count"], 1002);
    assert_eq!(payload["insert_batch_count"], 3);

    let requests = requests.lock().await.clone();
    let paths = requests
        .iter()
        .map(|request| request.path.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![
            "/open-apis/docx/v1/documents/blocks/convert".to_owned(),
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children".to_owned(),
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_callout/descendant"
                .to_owned(),
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_callout/descendant"
                .to_owned(),
        ]
    );

    server.abort();
}

#[tokio::test]
async fn feishu_doc_append_supports_oversized_grid_subtree() {
    let temp_dir = temp_feishu_cli_dir("doc-append-oversized-grid");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let leaf_ids = (0..1000)
        .map(|index| format!("tmp-grid-leaf-{index:04}"))
        .collect::<Vec<_>>();
    let mut descendants = Vec::with_capacity(leaf_ids.len() + 3);
    descendants.push(json!({
        "block_id": "tmp-grid",
        "block_type": 24,
        "grid": {},
        "children": ["tmp-grid-col-1", "tmp-grid-col-2"]
    }));
    descendants.push(json!({
        "block_id": "tmp-grid-col-1",
        "block_type": 25,
        "grid_column": {},
        "children": leaf_ids
    }));
    descendants.push(json!({
        "block_id": "tmp-grid-col-2",
        "block_type": 25,
        "grid_column": {},
        "children": []
    }));
    descendants.extend(leaf_ids.iter().map(|block_id| {
        json!({
            "block_id": block_id,
            "block_type": 2,
            "text": {
                "elements": [{"text_run": {"content": block_id}}]
            },
            "children": []
        })
    }));
    let convert_response = json!({
        "code": 0,
        "data": {
            "first_level_block_ids": ["tmp-grid"],
            "blocks": descendants
        }
    });
    let router = Router::new()
        .route(
            "/open-apis/docx/v1/documents/blocks/convert",
            post({
                let state = state.clone();
                let convert_response = convert_response.clone();
                move |request| {
                    let state = state.clone();
                    let convert_response = convert_response.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(convert_response)
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "children": [{
                                    "block_id": "blk_real_grid",
                                    "block_type": 24,
                                    "children": ["blk_real_grid_col_1", "blk_real_grid_col_2"],
                                    "grid": {}
                                }]
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_grid_col_1/descendant",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": []
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant(
        "feishu_main",
        "ou_123",
        "u-token-doc-append-oversized-grid",
        "r-token",
        now_s,
    );
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "docx:document",
    ]);
    store.save_grant(&grant).expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_doc_append(
        &loongclaw_daemon::feishu_cli::FeishuDocAppendArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            url: "https://open.feishu.cn/docx/doxcnExisting".to_owned(),
            content: Some("oversized grid subtree".to_owned()),
            content_path: None,
            content_type: None,
        },
    )
    .await
    .expect("execute doc append oversized grid subtree");

    assert_eq!(payload["document"]["document_id"], "doxcnExisting");
    assert_eq!(payload["inserted_block_count"], 1003);
    assert_eq!(payload["insert_batch_count"], 2);

    let requests = requests.lock().await.clone();
    let paths = requests
        .iter()
        .map(|request| request.path.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![
            "/open-apis/docx/v1/documents/blocks/convert".to_owned(),
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/children".to_owned(),
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/blk_real_grid_col_1/descendant"
                .to_owned(),
        ]
    );

    server.abort();
}

#[tokio::test]
async fn feishu_doc_append_reads_html_content_path_and_infers_html_type() {
    let temp_dir = temp_feishu_cli_dir("doc-append-content-path");
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let content_path = temp_dir.join("fixtures/follow-up.html");
    std::fs::create_dir_all(content_path.parent().expect("content path parent"))
        .expect("create content parent");
    std::fs::write(&content_path, "<p>Follow-up note</p>").expect("write content fixture");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/docx/v1/documents/blocks/convert",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "first_level_block_ids": ["tmp-paragraph"],
                                "blocks": [{
                                    "block_id": "tmp-paragraph",
                                    "block_type": 2,
                                    "text": {
                                        "elements": [{"text_run": {"content": "Follow-up note"}}]
                                    },
                                    "children": []
                                }]
                            }
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/docx/v1/documents/doxcnExisting/blocks/doxcnExisting/descendant",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "block_id_relations": [{
                                    "block_id": "blk_real_paragraph",
                                    "temporary_block_id": "tmp-paragraph"
                                }]
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant(
        "feishu_main",
        "ou_123",
        "u-token-doc-append-path",
        "r-token",
        now_s,
    );
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "docx:document",
    ]);
    store.save_grant(&grant).expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_doc_append(
        &loongclaw_daemon::feishu_cli::FeishuDocAppendArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            url: "https://open.feishu.cn/docx/doxcnExisting".to_owned(),
            content: None,
            content_path: Some(content_path.display().to_string()),
            content_type: None,
        },
    )
    .await
    .expect("execute doc append from content path");

    assert_eq!(payload["content_type"], "html");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        serde_json::from_str::<Value>(&requests[0].body).expect("convert request json"),
        json!({
            "content_type": "html",
            "content": "<p>Follow-up note</p>"
        })
    );

    server.abort();
}

#[tokio::test]
async fn feishu_doc_append_rejects_content_and_content_path_together() {
    let temp_dir = temp_feishu_cli_dir("doc-append-content-conflict");
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let content_path = temp_dir.join("fixtures/follow-up.md");
    std::fs::create_dir_all(content_path.parent().expect("content path parent"))
        .expect("create content parent");
    std::fs::write(&content_path, "Follow-up note").expect("write content fixture");
    let config_path = write_sample_feishu_config(&temp_dir);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    let mut grant = sample_grant(
        "feishu_main",
        "ou_123",
        "u-token-doc-append-conflict",
        "r-token",
        now_s,
    );
    grant.scopes = mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes([
        "offline_access",
        "docx:document",
    ]);
    store.save_grant(&grant).expect("seed grant");

    let error = loongclaw_daemon::feishu_cli::execute_feishu_doc_append(
        &loongclaw_daemon::feishu_cli::FeishuDocAppendArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            url: "https://open.feishu.cn/docx/doxcnExisting".to_owned(),
            content: Some("inline".to_owned()),
            content_path: Some(content_path.display().to_string()),
            content_type: None,
        },
    )
    .await
    .expect_err("doc append should reject mixed content sources");

    assert_eq!(
        error,
        "loong feishu doc append accepts either --content or --content-path, not both"
    );
}

#[tokio::test]
async fn feishu_calendar_primary_uses_user_grant_token_and_defaults_open_id() {
    let temp_dir = temp_feishu_cli_dir("calendar-primary-user-token");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new().route(
        "/open-apis/calendar/v4/calendars/primary",
        post({
            let state = state.clone();
            move |request| {
                let state = state.clone();
                async move {
                    record_request(State(state), request).await;
                    Json(json!({
                        "code": 0,
                        "data": {
                            "calendars": [{
                                "calendar": {
                                    "calendar_id": "cal_1",
                                    "summary": "Primary",
                                    "permissions": "owner"
                                },
                                "user_id": "ou_123"
                            }]
                        }
                    }))
                }
            }
        }),
    );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token",
            "r-token",
            now_s,
        ))
        .expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_calendar_list(
        &loongclaw_daemon::feishu_cli::FeishuCalendarListArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            primary: true,
            user_id_type: None,
            page_size: None,
            page_token: None,
            sync_token: None,
        },
    )
    .await
    .expect("execute calendar primary");

    assert_eq!(payload["primary"], true);
    assert_eq!(
        payload["calendars"]["calendars"][0]["calendar"]["calendar_id"],
        "cal_1"
    );
    assert_eq!(payload["calendars"]["calendars"][0]["user_id"], "ou_123");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].path, "/open-apis/calendar/v4/calendars/primary");
    assert_eq!(requests[0].authorization.as_deref(), Some("Bearer u-token"));
    assert!(
        requests[0]
            .query
            .as_deref()
            .is_some_and(|query| query.contains("user_id_type=open_id"))
    );
    assert_eq!(requests[0].body, "{}");

    server.abort();
}

#[tokio::test]
async fn feishu_messages_get_fetches_tenant_token_before_im_detail_request() {
    let temp_dir = temp_feishu_cli_dir("messages-get-tenant-token");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "tenant_access_token": "t-token",
                            "expire": 7200
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_1",
            get({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "data": {
                                "items": [{
                                    "message_id": "om_1",
                                    "chat_id": "oc_1",
                                    "root_id": "om_root_1",
                                    "parent_id": "om_parent_1",
                                    "msg_type": "text",
                                    "create_time": "1700000000",
                                    "update_time": "1700000001",
                                    "sender": {
                                        "id": "ou_123",
                                        "sender_type": "user"
                                    }
                                }]
                            }
                        }))
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token",
            "r-token",
            now_s,
        ))
        .expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_messages_get(
        &loongclaw_daemon::feishu_cli::FeishuMessagesGetArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            message_id: "om_1".to_owned(),
        },
    )
    .await
    .expect("execute message detail");

    assert_eq!(payload["message"]["message_id"], "om_1");
    assert_eq!(payload["message"]["sender_id"], "ou_123");

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].path,
        "/open-apis/auth/v3/tenant_access_token/internal"
    );
    assert_eq!(requests[1].path, "/open-apis/im/v1/messages/om_1");
    assert_eq!(requests[1].authorization.as_deref(), Some("Bearer t-token"));

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feishu_messages_resource_downloads_binary_to_output_path() {
    let temp_dir = temp_feishu_cli_dir("messages-resource");
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let output_path = temp_dir.join("downloads/spec-sheet.pdf");
    let requests = Arc::new(Mutex::new(Vec::<MockRequest>::new()));
    let state = MockServerState {
        requests: requests.clone(),
    };
    let router = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        Json(json!({
                            "code": 0,
                            "tenant_access_token": "t-token-resource",
                            "expire": 7200
                        }))
                    }
                }
            }),
        )
        .route(
            "/open-apis/im/v1/messages/om_resource_1/resources/file_resource_1",
            get({
                let state = state.clone();
                move |request| {
                    let state = state.clone();
                    async move {
                        record_request(State(state), request).await;
                        let mut response = axum::response::Response::new(axum::body::Body::from(
                            "pdf-resource-bytes",
                        ));
                        response.headers_mut().insert(
                            axum::http::header::CONTENT_TYPE,
                            axum::http::HeaderValue::from_static("application/pdf"),
                        );
                        response.headers_mut().insert(
                            axum::http::header::CONTENT_DISPOSITION,
                            axum::http::HeaderValue::from_static(
                                "attachment; filename=\"spec-sheet.pdf\"",
                            ),
                        );
                        response
                    }
                }
            }),
        );
    let (base_url, server) = spawn_mock_feishu_server(router).await;
    let config_path = write_sample_feishu_config_with_base_url(&temp_dir, &base_url);
    let now_s = loongclaw_daemon::feishu_support::unix_ts_now();
    let store = mvp::channel::feishu::api::FeishuTokenStore::new(temp_dir.join("feishu.sqlite3"));
    store
        .save_grant(&sample_grant(
            "feishu_main",
            "ou_123",
            "u-token-resource",
            "r-token-resource",
            now_s,
        ))
        .expect("seed grant");

    let payload = loongclaw_daemon::feishu_cli::execute_feishu_messages_resource(
        &loongclaw_daemon::feishu_cli::FeishuMessagesResourceArgs {
            grant: loongclaw_daemon::feishu_cli::FeishuGrantArgs {
                common: loongclaw_daemon::feishu_cli::FeishuCommonArgs {
                    config: Some(config_path.display().to_string()),
                    account: Some("feishu_main".to_owned()),
                    json: true,
                },
                open_id: Some("ou_123".to_owned()),
            },
            message_id: "om_resource_1".to_owned(),
            file_key: "file_resource_1".to_owned(),
            resource_type: loongclaw_daemon::feishu_cli::FeishuMessageResourceCliType::File,
            output: output_path.display().to_string(),
        },
    )
    .await
    .expect("execute message resource download");

    assert_eq!(payload["message_id"], "om_resource_1");
    assert_eq!(payload["file_key"], "file_resource_1");
    assert_eq!(payload["resource_type"], "file");
    assert_eq!(payload["content_type"], "application/pdf");
    assert_eq!(payload["file_name"], "spec-sheet.pdf");
    assert_eq!(payload["bytes_written"], 18);
    assert_eq!(
        fs::read(&output_path).expect("read downloaded resource"),
        b"pdf-resource-bytes"
    );

    let requests = requests.lock().await.clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].path,
        "/open-apis/im/v1/messages/om_resource_1/resources/file_resource_1"
    );
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer t-token-resource")
    );
    assert_eq!(requests[1].query.as_deref(), Some("type=file"));

    server.abort();
}
