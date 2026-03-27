use std::time::{SystemTime, UNIX_EPOCH};

use rand::random;
use serde_json::{Value, json};

use crate::{CliResult, config::ResolvedTlonChannelConfig};

use super::{
    ChannelOutboundTargetKind,
    http::{build_outbound_http_client, response_body_detail},
};

const TLON_LOGIN_PATH: &str = "/~/login";
const TLON_CHANNEL_PATH_PREFIX: &str = "/~/channel";
const TLON_DM_APP: &str = "chat";
const TLON_DM_MARK: &str = "chat-dm-action";
const TLON_GROUP_APP: &str = "channels";
const TLON_GROUP_MARK: &str = "channel-action-1";
const TLON_TARGET_HINT: &str =
    "dm/~sampel-palnet | ~sampel-palnet | chat/~host-ship/channel | group:~host-ship/channel";
const TLON_DA_UNIX_EPOCH: u128 = 170141184475152167957503069145530368000;
const TLON_DA_SECOND: u128 = 18446744073709551616;

#[derive(Debug, Clone, PartialEq, Eq)]
enum TlonSendTarget {
    DirectMessage {
        ship: String,
    },
    Group {
        host_ship: String,
        channel_name: String,
    },
}

pub(super) async fn run_tlon_send(
    resolved: &ResolvedTlonChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
) -> CliResult<()> {
    ensure_tlon_target_kind(target_kind)?;

    let ship_value = resolved
        .ship()
        .ok_or_else(|| "tlon ship missing (set tlon.ship or env)".to_owned())?;
    let from_ship = normalize_tlon_ship(ship_value.as_str())?;

    let url_value = resolved
        .url()
        .ok_or_else(|| "tlon url missing (set tlon.url or env)".to_owned())?;
    let base_url = normalize_tlon_base_url(url_value.as_str())?;

    let code = resolved
        .code()
        .ok_or_else(|| "tlon code missing (set tlon.code or env)".to_owned())?;
    let target = parse_tlon_send_target(target_id)?;
    let text = normalize_tlon_text(text)?;

    let client = build_outbound_http_client("tlon send")?;
    let cookie = login_tlon_ship(&client, base_url.as_str(), code.as_str()).await?;
    let channel_id = build_tlon_channel_id()?;
    let ship_name = from_ship.trim_start_matches('~');
    let sent_at_ms = current_unix_millis()?;
    let request_body = build_tlon_poke_request_body(
        ship_name,
        from_ship.as_str(),
        sent_at_ms,
        &target,
        text.as_str(),
    )?;

    send_tlon_poke(
        &client,
        base_url.as_str(),
        channel_id.as_str(),
        cookie.as_str(),
        &request_body,
    )
    .await
}

fn ensure_tlon_target_kind(target_kind: ChannelOutboundTargetKind) -> CliResult<()> {
    if target_kind == ChannelOutboundTargetKind::Conversation {
        return Ok(());
    }

    Err(format!(
        "tlon send requires conversation target kind, got {}",
        target_kind.as_str()
    ))
}

fn normalize_tlon_text(raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("tlon send text is empty".to_owned());
    }
    if raw.contains('\0') {
        return Err("tlon send text contains forbidden control characters".to_owned());
    }

    Ok(raw.to_owned())
}

fn parse_tlon_send_target(raw: &str) -> CliResult<TlonSendTarget> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        let error = format!("invalid tlon target; use {TLON_TARGET_HINT}");
        return Err(error);
    }

    let without_channel_prefix = strip_named_prefix(trimmed, "tlon").unwrap_or(trimmed);

    if let Some(dm_target) = strip_prefixed_target(without_channel_prefix, "dm") {
        let ship = normalize_tlon_ship(dm_target)?;
        return Ok(TlonSendTarget::DirectMessage { ship });
    }

    if let Some(group_target) = strip_group_target(without_channel_prefix) {
        return parse_tlon_group_target(group_target);
    }

    if without_channel_prefix.starts_with("chat/") {
        return parse_tlon_group_target(without_channel_prefix);
    }

    let ship = normalize_tlon_ship(without_channel_prefix)?;
    Ok(TlonSendTarget::DirectMessage { ship })
}

fn strip_prefixed_target<'a>(raw: &'a str, prefix: &str) -> Option<&'a str> {
    let normalized_raw = raw.to_ascii_lowercase();
    let prefix_slash = format!("{prefix}/");
    let prefix_colon = format!("{prefix}:");
    if normalized_raw.starts_with(prefix_slash.as_str()) {
        let offset = prefix_slash.len();
        return Some(&raw[offset..]);
    }
    if normalized_raw.starts_with(prefix_colon.as_str()) {
        let offset = prefix_colon.len();
        return Some(&raw[offset..]);
    }

    None
}

fn strip_named_prefix<'a>(raw: &'a str, prefix: &str) -> Option<&'a str> {
    let normalized_raw = raw.to_ascii_lowercase();
    let prefix_value = format!("{prefix}:");
    if !normalized_raw.starts_with(prefix_value.as_str()) {
        return None;
    }

    let offset = prefix_value.len();
    Some(&raw[offset..])
}

fn strip_group_target(raw: &str) -> Option<&str> {
    let group_target = strip_prefixed_target(raw, "group");
    if group_target.is_some() {
        return group_target;
    }

    strip_prefixed_target(raw, "room")
}

fn parse_tlon_group_target(raw: &str) -> CliResult<TlonSendTarget> {
    let normalized = raw.trim();
    if normalized.starts_with("chat/") {
        return parse_tlon_group_nest(normalized);
    }

    let segments = normalized.split('/').collect::<Vec<_>>();
    if segments.len() != 2 {
        let error = format!("invalid tlon target; use {TLON_TARGET_HINT}");
        return Err(error);
    }

    let host_ship_segment = segments
        .first()
        .copied()
        .ok_or_else(|| format!("invalid tlon target; use {TLON_TARGET_HINT}"))?;
    let channel_name_segment = segments
        .get(1)
        .copied()
        .ok_or_else(|| format!("invalid tlon target; use {TLON_TARGET_HINT}"))?;
    let host_ship = normalize_tlon_ship(host_ship_segment)?;
    let channel_name = normalize_tlon_channel_name(channel_name_segment)?;
    Ok(TlonSendTarget::Group {
        host_ship,
        channel_name,
    })
}

fn parse_tlon_group_nest(raw: &str) -> CliResult<TlonSendTarget> {
    let segments = raw.split('/').collect::<Vec<_>>();
    if segments.len() != 3 {
        let error = format!("invalid tlon target; use {TLON_TARGET_HINT}");
        return Err(error);
    }

    let kind_segment = segments
        .first()
        .copied()
        .ok_or_else(|| format!("invalid tlon target; use {TLON_TARGET_HINT}"))?;
    if kind_segment != "chat" {
        let error = format!("invalid tlon target; use {TLON_TARGET_HINT}");
        return Err(error);
    }

    let host_ship_segment = segments
        .get(1)
        .copied()
        .ok_or_else(|| format!("invalid tlon target; use {TLON_TARGET_HINT}"))?;
    let channel_name_segment = segments
        .get(2)
        .copied()
        .ok_or_else(|| format!("invalid tlon target; use {TLON_TARGET_HINT}"))?;
    let host_ship = normalize_tlon_ship(host_ship_segment)?;
    let channel_name = normalize_tlon_channel_name(channel_name_segment)?;
    Ok(TlonSendTarget::Group {
        host_ship,
        channel_name,
    })
}

fn normalize_tlon_ship(raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("tlon ship is empty".to_owned());
    }

    let ship_body = trimmed.trim_start_matches('~');
    if ship_body.is_empty() {
        return Err("tlon ship is empty".to_owned());
    }

    let has_invalid_character = ship_body.chars().any(|value| {
        let is_letter = value.is_ascii_alphabetic();
        let is_separator = value == '-';
        !is_letter && !is_separator
    });
    if has_invalid_character {
        return Err("tlon ship must contain only letters and `-`".to_owned());
    }

    let normalized_body = ship_body.to_ascii_lowercase();
    let ship = format!("~{normalized_body}");
    Ok(ship)
}

fn normalize_tlon_channel_name(raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("tlon channel name is empty".to_owned());
    }
    if trimmed.contains('/') {
        return Err("tlon channel name must not contain `/`".to_owned());
    }

    Ok(trimmed.to_owned())
}

fn normalize_tlon_base_url(raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("tlon url is empty".to_owned());
    }

    let has_scheme = trimmed.contains("://");
    let candidate = if has_scheme {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    };

    let parsed_url = reqwest::Url::parse(candidate.as_str())
        .map_err(|error| format!("tlon url is invalid: {error}"))?;
    let scheme = parsed_url.scheme();
    let is_http = scheme == "http";
    let is_https = scheme == "https";
    if !is_http && !is_https {
        return Err(format!("tlon url must use http or https, got {scheme}"));
    }
    if !parsed_url.username().is_empty() || parsed_url.password().is_some() {
        return Err("tlon url must not include credentials".to_owned());
    }

    let path = parsed_url.path();
    let has_non_root_path = path != "/" && !path.is_empty();
    let has_query = parsed_url.query().is_some();
    let has_fragment = parsed_url.fragment().is_some();
    if has_non_root_path || has_query || has_fragment {
        return Err("tlon url must not include a path, query, or fragment".to_owned());
    }

    let hostname = parsed_url
        .host_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "tlon url hostname is invalid".to_owned())?;
    let normalized_hostname = hostname.to_ascii_lowercase();
    let normalized_hostname = normalized_hostname.trim_end_matches('.');
    if normalized_hostname.is_empty() {
        return Err("tlon url hostname is invalid".to_owned());
    }

    let is_ipv6 = normalized_hostname.contains(':');
    let host = match parsed_url.port() {
        Some(port) => {
            if is_ipv6 {
                format!("[{normalized_hostname}]:{port}")
            } else {
                format!("{normalized_hostname}:{port}")
            }
        }
        None => {
            if is_ipv6 {
                format!("[{normalized_hostname}]")
            } else {
                normalized_hostname.to_owned()
            }
        }
    };

    let normalized_url = format!("{scheme}://{host}");
    Ok(normalized_url)
}

async fn login_tlon_ship(
    client: &reqwest::Client,
    base_url: &str,
    code: &str,
) -> CliResult<String> {
    let request_url = build_tlon_path_url(base_url, TLON_LOGIN_PATH)?;
    let form_fields = [("password", code)];
    let request = client.post(request_url).form(&form_fields);
    let response = request
        .send()
        .await
        .map_err(|error| format!("tlon login failed: {error}"))?;

    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .text()
        .await
        .map_err(|error| format!("read tlon login response failed: {error}"))?;
    if !status.is_success() {
        let detail = response_body_detail(body.as_str());
        return Err(format!(
            "tlon login failed with status {}: {detail}",
            status.as_u16()
        ));
    }

    let mut cookie_pairs = Vec::new();
    let set_cookie_values = headers.get_all(reqwest::header::SET_COOKIE);
    for value in set_cookie_values {
        let Some(cookie_value) = value.to_str().ok() else {
            continue;
        };
        let trimmed_cookie_value = cookie_value.trim();
        if trimmed_cookie_value.is_empty() {
            continue;
        }
        let session_cookie = trimmed_cookie_value
            .split(';')
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(session_cookie) = session_cookie else {
            continue;
        };
        cookie_pairs.push(session_cookie.to_owned());
    }

    if cookie_pairs.is_empty() {
        return Err("tlon login did not return a set-cookie header".to_owned());
    }

    let cookie_header = cookie_pairs.join("; ");
    Ok(cookie_header)
}

fn build_tlon_channel_id() -> CliResult<String> {
    let unix_seconds = current_unix_seconds()?;
    let random_suffix = random::<u64>();
    let channel_id = format!("{unix_seconds}-{random_suffix:016x}");
    Ok(channel_id)
}

fn current_unix_millis() -> CliResult<u64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("tlon time is before unix epoch: {error}"))?;
    let millis = duration.as_millis();
    let millis =
        u64::try_from(millis).map_err(|error| format!("tlon unix millis overflow: {error}"))?;
    Ok(millis)
}

fn current_unix_seconds() -> CliResult<u64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("tlon time is before unix epoch: {error}"))?;
    Ok(duration.as_secs())
}

fn build_tlon_poke_request_body(
    ship_name: &str,
    from_ship: &str,
    sent_at_ms: u64,
    target: &TlonSendTarget,
    text: &str,
) -> CliResult<Value> {
    let poke_id = sent_at_ms;
    let story = build_tlon_story(text);

    let (app, mark, action_json) = match target {
        TlonSendTarget::DirectMessage { ship } => {
            let message_id = build_tlon_dm_message_id(from_ship, sent_at_ms)?;
            let action_json = build_tlon_dm_action_json(
                ship.as_str(),
                from_ship,
                sent_at_ms,
                message_id.as_str(),
                story,
            );
            (TLON_DM_APP, TLON_DM_MARK, action_json)
        }
        TlonSendTarget::Group {
            host_ship,
            channel_name,
        } => {
            let action_json = build_tlon_group_action_json(
                host_ship.as_str(),
                channel_name.as_str(),
                from_ship,
                sent_at_ms,
                story,
            );
            (TLON_GROUP_APP, TLON_GROUP_MARK, action_json)
        }
    };

    let request_body = json!([
        {
            "id": poke_id,
            "action": "poke",
            "ship": ship_name,
            "app": app,
            "mark": mark,
            "json": action_json,
        }
    ]);
    Ok(request_body)
}

fn build_tlon_story(text: &str) -> Value {
    let lines = text.split('\n').collect::<Vec<_>>();
    let mut inlines = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        if !line.is_empty() {
            inlines.push(Value::String((*line).to_owned()));
        }
        let is_last_line = index + 1 == lines.len();
        if !is_last_line {
            inlines.push(json!({ "break": Value::Null }));
        }
    }

    json!([{ "inline": inlines }])
}

fn build_tlon_dm_action_json(
    to_ship: &str,
    from_ship: &str,
    sent_at_ms: u64,
    message_id: &str,
    story: Value,
) -> Value {
    json!({
        "ship": to_ship,
        "diff": {
            "id": message_id,
            "delta": {
                "add": {
                    "memo": {
                        "content": story,
                        "author": from_ship,
                        "sent": sent_at_ms,
                    },
                    "kind": Value::Null,
                    "time": Value::Null,
                }
            }
        }
    })
}

fn build_tlon_group_action_json(
    host_ship: &str,
    channel_name: &str,
    from_ship: &str,
    sent_at_ms: u64,
    story: Value,
) -> Value {
    let nest = format!("chat/{host_ship}/{channel_name}");
    json!({
        "channel": {
            "nest": nest,
            "action": {
                "post": {
                    "add": {
                        "content": story,
                        "author": from_ship,
                        "sent": sent_at_ms,
                        "kind": "/chat",
                        "blob": Value::Null,
                        "meta": Value::Null,
                    }
                }
            }
        }
    })
}

fn build_tlon_dm_message_id(from_ship: &str, sent_at_ms: u64) -> CliResult<String> {
    let ud_value = tlon_ud_from_unix_millis(sent_at_ms)?;
    let message_id = format!("{from_ship}/{ud_value}");
    Ok(message_id)
}

fn tlon_ud_from_unix_millis(unix_millis: u64) -> CliResult<String> {
    let unix_millis = u128::from(unix_millis);
    let scaled = unix_millis
        .checked_mul(TLON_DA_SECOND)
        .ok_or_else(|| "tlon @da conversion overflowed".to_owned())?;
    let da_offset = scaled / 1000;
    let da_value = TLON_DA_UNIX_EPOCH
        .checked_add(da_offset)
        .ok_or_else(|| "tlon @da conversion overflowed".to_owned())?;
    let ud_value = format_tlon_ud(da_value);
    Ok(ud_value)
}

fn format_tlon_ud(value: u128) -> String {
    let digits = value.to_string();
    let length = digits.len();
    let mut rendered = String::with_capacity(length + (length / 3));

    for (index, character) in digits.chars().enumerate() {
        let remaining = length - index;
        let should_insert_separator = index > 0 && remaining.is_multiple_of(3);
        if should_insert_separator {
            rendered.push('.');
        }
        rendered.push(character);
    }

    rendered
}

async fn send_tlon_poke(
    client: &reqwest::Client,
    base_url: &str,
    channel_id: &str,
    cookie: &str,
    request_body: &Value,
) -> CliResult<()> {
    let channel_path = format!("{TLON_CHANNEL_PATH_PREFIX}/{channel_id}");
    let request_url = build_tlon_path_url(base_url, channel_path.as_str())?;
    let request = client
        .put(request_url)
        .header("Content-Type", "application/json")
        .header(reqwest::header::COOKIE, cookie)
        .json(request_body);
    let response = request
        .send()
        .await
        .map_err(|error| format!("tlon poke failed: {error}"))?;

    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let body = response
        .text()
        .await
        .map_err(|error| format!("read tlon poke error response failed: {error}"))?;
    let detail = response_body_detail(body.as_str());
    Err(format!(
        "tlon poke failed with status {}: {detail}",
        status.as_u16()
    ))
}

fn build_tlon_path_url(base_url: &str, path: &str) -> CliResult<reqwest::Url> {
    let normalized_base_url = normalize_tlon_base_url(base_url)?;
    let base_url = reqwest::Url::parse(normalized_base_url.as_str())
        .map_err(|error| format!("tlon url is invalid: {error}"))?;
    let request_url = base_url
        .join(path)
        .map_err(|error| format!("build tlon request url failed: {error}"))?;
    Ok(request_url)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{
        Json, Router,
        extract::{Path, State},
        http::{HeaderMap, HeaderValue, StatusCode},
        response::IntoResponse,
        routing::{post, put},
    };
    use loongclaw_contracts::SecretRef;
    use tokio::net::TcpListener;

    use super::*;

    #[derive(Debug, Clone)]
    struct MockTlonState {
        login_cookies: Vec<String>,
        poke_requests: Arc<Mutex<Vec<MockTlonPokeRequest>>>,
    }

    #[derive(Debug, Clone)]
    struct MockTlonPokeRequest {
        channel_id: String,
        cookie: String,
        body: Value,
    }

    impl Default for MockTlonState {
        fn default() -> Self {
            let login_cookies = vec!["urbauth=ship-session".to_owned()];
            let poke_requests = Arc::new(Mutex::new(Vec::new()));
            Self {
                login_cookies,
                poke_requests,
            }
        }
    }

    #[tokio::test]
    async fn run_tlon_send_posts_expected_dm_poke_payload() {
        let state = MockTlonState::default();
        let router = build_mock_tlon_router(state.clone());
        let (base_url, server) = spawn_mock_tlon_server(router).await;
        let resolved = build_resolved_tlon_config(base_url.as_str());

        let send_result = run_tlon_send(
            &resolved,
            ChannelOutboundTargetKind::Conversation,
            "~nec",
            "hello\nworld",
        )
        .await;

        send_result.expect("tlon dm send should succeed");

        let requests = state.poke_requests.lock().expect("poke requests");
        assert_eq!(requests.len(), 1);

        let request = &requests[0];
        assert_eq!(request.cookie, "urbauth=ship-session");
        assert!(!request.channel_id.is_empty());

        let payload = request.body.as_array().expect("poke request array");
        assert_eq!(payload.len(), 1);

        let entry = &payload[0];
        assert_eq!(entry["action"], "poke");
        assert_eq!(entry["ship"], "zod");
        assert_eq!(entry["app"], TLON_DM_APP);
        assert_eq!(entry["mark"], TLON_DM_MARK);
        assert_eq!(entry["json"]["ship"], "~nec");
        assert_eq!(
            entry["json"]["diff"]["delta"]["add"]["memo"]["author"],
            "~zod"
        );
        assert_eq!(
            entry["json"]["diff"]["delta"]["add"]["memo"]["content"],
            json!([
                {
                    "inline": [
                        "hello",
                        { "break": Value::Null },
                        "world",
                    ],
                }
            ])
        );

        let message_id = entry["json"]["diff"]["id"].as_str().expect("dm message id");
        assert!(message_id.starts_with("~zod/"));

        server.abort();
    }

    #[tokio::test]
    async fn run_tlon_send_posts_expected_group_poke_payload() {
        let state = MockTlonState::default();
        let router = build_mock_tlon_router(state.clone());
        let (base_url, server) = spawn_mock_tlon_server(router).await;
        let resolved = build_resolved_tlon_config(base_url.as_str());

        let send_result = run_tlon_send(
            &resolved,
            ChannelOutboundTargetKind::Conversation,
            "group:~bus/chat-room",
            "hello group",
        )
        .await;

        send_result.expect("tlon group send should succeed");

        let requests = state.poke_requests.lock().expect("poke requests");
        assert_eq!(requests.len(), 1);

        let payload = requests[0].body.as_array().expect("poke request array");
        let entry = &payload[0];
        assert_eq!(entry["app"], TLON_GROUP_APP);
        assert_eq!(entry["mark"], TLON_GROUP_MARK);
        assert_eq!(entry["json"]["channel"]["nest"], "chat/~bus/chat-room");
        assert_eq!(
            entry["json"]["channel"]["action"]["post"]["add"]["author"],
            "~zod"
        );
        assert_eq!(
            entry["json"]["channel"]["action"]["post"]["add"]["kind"],
            "/chat"
        );

        server.abort();
    }

    #[tokio::test]
    async fn run_tlon_send_combines_multiple_login_cookies() {
        let state = MockTlonState {
            login_cookies: vec![
                "urbauth=ship-session; Path=/; HttpOnly".to_owned(),
                "tlonpref=compact; Path=/".to_owned(),
            ],
            ..MockTlonState::default()
        };
        let router = build_mock_tlon_router(state.clone());
        let (base_url, server) = spawn_mock_tlon_server(router).await;
        let resolved = build_resolved_tlon_config(base_url.as_str());

        let send_result = run_tlon_send(
            &resolved,
            ChannelOutboundTargetKind::Conversation,
            "~nec",
            "hello",
        )
        .await;

        send_result.expect("tlon multi-cookie login should succeed");

        let requests = state.poke_requests.lock().expect("poke requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].cookie, "urbauth=ship-session; tlonpref=compact");

        server.abort();
    }

    #[tokio::test]
    async fn run_tlon_send_requires_login_cookie() {
        let state = MockTlonState {
            login_cookies: Vec::new(),
            ..MockTlonState::default()
        };
        let router = build_mock_tlon_router(state);
        let (base_url, server) = spawn_mock_tlon_server(router).await;
        let resolved = build_resolved_tlon_config(base_url.as_str());

        let error = run_tlon_send(
            &resolved,
            ChannelOutboundTargetKind::Conversation,
            "~nec",
            "hello",
        )
        .await
        .expect_err("missing login cookie should fail");

        assert_eq!(error, "tlon login did not return a set-cookie header");

        server.abort();
    }

    #[test]
    fn normalize_tlon_base_url_accepts_host_without_scheme() {
        let normalized = normalize_tlon_base_url("ship.example.test")
            .expect("normalize base url without scheme");

        assert_eq!(normalized, "https://ship.example.test");
    }

    #[test]
    fn normalize_tlon_base_url_rejects_path_query_and_fragment() {
        let path_error = normalize_tlon_base_url("https://ship.example.test/~/channel")
            .expect_err("path should be rejected");
        assert_eq!(
            path_error,
            "tlon url must not include a path, query, or fragment"
        );

        let query_error = normalize_tlon_base_url("https://ship.example.test?foo=bar")
            .expect_err("query should be rejected");
        assert_eq!(
            query_error,
            "tlon url must not include a path, query, or fragment"
        );

        let fragment_error = normalize_tlon_base_url("https://ship.example.test#frag")
            .expect_err("fragment should be rejected");
        assert_eq!(
            fragment_error,
            "tlon url must not include a path, query, or fragment"
        );
    }

    #[test]
    fn parse_tlon_send_target_accepts_dm_and_group_forms() {
        let dm_target = parse_tlon_send_target("dm:~nec").expect("parse dm target");
        assert_eq!(
            dm_target,
            TlonSendTarget::DirectMessage {
                ship: "~nec".to_owned(),
            }
        );

        let group_target =
            parse_tlon_send_target("chat/~bus/chat-room").expect("parse group target");
        assert_eq!(
            group_target,
            TlonSendTarget::Group {
                host_ship: "~bus".to_owned(),
                channel_name: "chat-room".to_owned(),
            }
        );
    }

    fn build_mock_tlon_router(state: MockTlonState) -> Router {
        Router::new()
            .route(TLON_LOGIN_PATH, post(handle_mock_tlon_login))
            .route("/~/channel/{channel_id}", put(handle_mock_tlon_poke))
            .with_state(state)
    }

    async fn spawn_mock_tlon_server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind tlon mock listener");
        let address = listener.local_addr().expect("tlon mock listener address");
        let base_url = format!("http://{}", address);
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve tlon mock router");
        });
        (base_url, server)
    }

    async fn handle_mock_tlon_login(State(state): State<MockTlonState>) -> impl IntoResponse {
        let mut headers = HeaderMap::new();
        for cookie in &state.login_cookies {
            let header_value =
                HeaderValue::from_str(cookie.as_str()).expect("build set-cookie header value");
            headers.append(reqwest::header::SET_COOKIE, header_value);
        }
        (StatusCode::OK, headers, "logged in")
    }

    async fn handle_mock_tlon_poke(
        State(state): State<MockTlonState>,
        Path(channel_id): Path<String>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> impl IntoResponse {
        let cookie = headers
            .get(reqwest::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .unwrap_or_default();
        let request = MockTlonPokeRequest {
            channel_id,
            cookie,
            body,
        };
        let mut requests = state.poke_requests.lock().expect("poke request log");
        requests.push(request);
        StatusCode::NO_CONTENT
    }

    fn build_resolved_tlon_config(base_url: &str) -> ResolvedTlonChannelConfig {
        ResolvedTlonChannelConfig {
            configured_account_id: "default".to_owned(),
            configured_account_label: "default".to_owned(),
            account: crate::config::ChannelAccountIdentity {
                id: "tlon_zod".to_owned(),
                label: "ship:~zod".to_owned(),
                source: crate::config::ChannelAccountIdentitySource::DerivedCredential,
            },
            enabled: true,
            ship: Some("~zod".to_owned()),
            ship_env: None,
            url: Some(base_url.to_owned()),
            url_env: None,
            code: Some(SecretRef::Inline("lidlut-tabwed-pillex-ridrup".to_owned())),
            code_env: None,
        }
    }
}
