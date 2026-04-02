use std::collections::BTreeMap;
use std::io::Read;
use std::io::Write;
use std::net::IpAddr;
use std::net::TcpListener;
use std::net::TcpStream;
use std::process::Command;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use reqwest::Url;
use serde::Deserialize;

use crate::CliResult;
use crate::oauth_support::{build_pkce_pair, generate_oauth_state};

const DEFAULT_OPENAI_CODEX_OAUTH_ISSUER: &str = "https://auth.openai.com";
const DEFAULT_OPENAI_CODEX_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const DEFAULT_OPENAI_CODEX_OAUTH_ORIGINATOR: &str = "codex_cli_rs";
const DEFAULT_CALLBACK_TIMEOUT_SECS: u64 = 180;
const DEFAULT_CALLBACK_BIND_HOST: &str = "127.0.0.1";
const DEFAULT_CALLBACK_REDIRECT_HOST: &str = "localhost";
const DEFAULT_CALLBACK_PORT: u16 = 1455;
const CALLBACK_PATH: &str = "/auth/callback";
const API_TOKEN_EXCHANGE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const API_TOKEN_EXCHANGE_REQUESTED_TOKEN: &str = "openai-api-key";
const API_TOKEN_EXCHANGE_SUBJECT_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:id_token";
const AUTH_SCOPE: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenaiCodexOauthGrant {
    pub(crate) access_token: String,
}

pub(crate) trait OpenaiCodexOauthFlow {
    fn authorization_url(&self) -> &str;
    fn callback_redirect_uri(&self) -> &str;
    fn open_browser(&mut self) -> CliResult<()>;
    fn wait_for_browser_callback(&mut self) -> CliResult<OpenaiCodexOauthGrant>;
    fn complete_from_manual_input(&mut self, input: &str) -> CliResult<OpenaiCodexOauthGrant>;
}

#[derive(Debug, Clone)]
struct OpenaiCodexOauthSettings {
    issuer: String,
    client_id: String,
    originator: String,
    callback_timeout: Duration,
}

#[derive(Debug, Clone)]
struct AuthorizationCodeExchange {
    id_token: String,
}

#[derive(Debug, Clone)]
struct CallbackPayload {
    code: String,
}

pub(crate) struct OpenaiCodexOauthSession {
    settings: OpenaiCodexOauthSettings,
    listener: TcpListener,
    redirect_uri: String,
    code_verifier: String,
    state: String,
    auth_url: String,
}

#[allow(dead_code)]
pub(crate) fn authorize_openai_codex_with_browser() -> CliResult<OpenaiCodexOauthGrant> {
    let mut flow = start_openai_codex_oauth_flow()?;
    flow.open_browser()?;
    flow.wait_for_browser_callback()
}

pub(crate) fn start_openai_codex_oauth_flow() -> CliResult<Box<dyn OpenaiCodexOauthFlow>> {
    let session = OpenaiCodexOauthSession::start()?;
    Ok(Box::new(session))
}

impl OpenaiCodexOauthSettings {
    fn from_env() -> Self {
        let issuer = std::env::var("LOONGCLAW_OPENAI_CODEX_OAUTH_ISSUER")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_OPENAI_CODEX_OAUTH_ISSUER.to_owned());
        let client_id = std::env::var("LOONGCLAW_OPENAI_CODEX_OAUTH_CLIENT_ID")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_OPENAI_CODEX_OAUTH_CLIENT_ID.to_owned());
        let originator = std::env::var("LOONGCLAW_OPENAI_CODEX_OAUTH_ORIGINATOR")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_OPENAI_CODEX_OAUTH_ORIGINATOR.to_owned());
        let callback_timeout = std::env::var("LOONGCLAW_OPENAI_CODEX_OAUTH_CALLBACK_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_CALLBACK_TIMEOUT_SECS));

        Self {
            issuer,
            client_id,
            originator,
            callback_timeout,
        }
    }
}

impl OpenaiCodexOauthSession {
    fn start() -> CliResult<Self> {
        let settings = OpenaiCodexOauthSettings::from_env();
        let listener = bind_local_callback_listener()?;
        let redirect_uri = callback_redirect_uri(DEFAULT_CALLBACK_PORT);
        let (code_verifier, code_challenge) = build_pkce_pair();
        let state = generate_oauth_state();
        let auth_url = build_authorize_url(&settings, &redirect_uri, &code_challenge, &state)?;

        Ok(Self {
            settings,
            listener,
            redirect_uri,
            code_verifier,
            state,
            auth_url,
        })
    }

    fn exchange_callback_payload(
        &self,
        payload: &CallbackPayload,
    ) -> CliResult<OpenaiCodexOauthGrant> {
        let exchanged = exchange_authorization_code(
            &self.settings,
            &self.redirect_uri,
            &self.code_verifier,
            &payload.code,
        )?;
        let access_token = exchange_id_token_for_api_key(&self.settings, &exchanged.id_token)?;

        Ok(OpenaiCodexOauthGrant { access_token })
    }
}

impl OpenaiCodexOauthFlow for OpenaiCodexOauthSession {
    fn authorization_url(&self) -> &str {
        &self.auth_url
    }

    fn callback_redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    fn open_browser(&mut self) -> CliResult<()> {
        let browser_opened = open_auth_url_in_browser(self.auth_url.as_str());
        if browser_opened {
            return Ok(());
        }

        let message = "could not open a browser automatically. Open the authorization link shown on screen manually.".to_owned();
        Err(message)
    }

    fn wait_for_browser_callback(&mut self) -> CliResult<OpenaiCodexOauthGrant> {
        let payload =
            wait_for_oauth_callback(&self.listener, self.settings.callback_timeout, &self.state)?;
        self.exchange_callback_payload(&payload)
    }

    fn complete_from_manual_input(&mut self, input: &str) -> CliResult<OpenaiCodexOauthGrant> {
        let payload = parse_manual_callback_input(input, &self.redirect_uri, &self.state)?;
        self.exchange_callback_payload(&payload)
    }
}

fn bind_local_callback_listener() -> CliResult<TcpListener> {
    let bind_target = (DEFAULT_CALLBACK_BIND_HOST, DEFAULT_CALLBACK_PORT);
    let bind_result = TcpListener::bind(bind_target);
    let listener = match bind_result {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            let message = format!(
                "bind local oauth callback listener failed: {DEFAULT_CALLBACK_BIND_HOST}:{DEFAULT_CALLBACK_PORT} is already in use. Close the other OAuth listener and try again."
            );
            return Err(message);
        }
        Err(error) => {
            let message = format!("bind local oauth callback listener failed: {error}");
            return Err(message);
        }
    };
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("set oauth callback listener nonblocking failed: {error}"))?;
    Ok(listener)
}

fn callback_redirect_uri(port: u16) -> String {
    format!("http://{DEFAULT_CALLBACK_REDIRECT_HOST}:{port}{CALLBACK_PATH}")
}

fn build_authorize_url(
    settings: &OpenaiCodexOauthSettings,
    redirect_uri: &str,
    code_challenge: &str,
    state: &str,
) -> CliResult<String> {
    let authorize_base = format!("{}/oauth/authorize", settings.issuer);
    let mut authorize_url = Url::parse(authorize_base.as_str())
        .map_err(|error| format!("build oauth authorize url failed: {error}"))?;
    {
        let mut query_pairs = authorize_url.query_pairs_mut();
        query_pairs.append_pair("response_type", "code");
        query_pairs.append_pair("client_id", settings.client_id.as_str());
        query_pairs.append_pair("redirect_uri", redirect_uri);
        query_pairs.append_pair("scope", AUTH_SCOPE);
        query_pairs.append_pair("code_challenge", code_challenge);
        query_pairs.append_pair("code_challenge_method", "S256");
        query_pairs.append_pair("id_token_add_organizations", "true");
        query_pairs.append_pair("codex_cli_simplified_flow", "true");
        query_pairs.append_pair("state", state);
        query_pairs.append_pair("originator", settings.originator.as_str());
    }
    Ok(authorize_url.into())
}

fn open_auth_url_in_browser(auth_url: &str) -> bool {
    let command_candidates = browser_open_command_candidates(auth_url);
    for candidate in command_candidates {
        let Some((program, arguments)) = candidate.split_first() else {
            continue;
        };
        let status_result = Command::new(program).args(arguments).status();
        let Ok(status) = status_result else {
            continue;
        };
        if status.success() {
            return true;
        }
    }
    false
}

fn browser_open_command_candidates(auth_url: &str) -> Vec<Vec<String>> {
    if cfg!(target_os = "macos") {
        return vec![vec!["open".to_owned(), auth_url.to_owned()]];
    }
    if cfg!(target_os = "windows") {
        return vec![vec![
            "cmd".to_owned(),
            "/C".to_owned(),
            "start".to_owned(),
            "".to_owned(),
            auth_url.to_owned(),
        ]];
    }
    vec![
        vec!["xdg-open".to_owned(), auth_url.to_owned()],
        vec!["gio".to_owned(), "open".to_owned(), auth_url.to_owned()],
    ]
}

fn wait_for_oauth_callback(
    listener: &TcpListener,
    timeout: Duration,
    expected_state: &str,
) -> CliResult<CallbackPayload> {
    let start = Instant::now();
    loop {
        let accept_result = listener.accept();
        match accept_result {
            Ok((mut stream, _address)) => {
                let payload = process_callback_request(&mut stream, expected_state)?;
                return Ok(payload);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let elapsed = start.elapsed();
                if elapsed >= timeout {
                    return Err("timed out waiting for OAuth browser callback".to_owned());
                }
                let retry_delay = Duration::from_millis(100);
                thread::park_timeout(retry_delay);
            }
            Err(error) => {
                let message = format!("accept oauth callback connection failed: {error}");
                return Err(message);
            }
        }
    }
}

fn process_callback_request(
    stream: &mut TcpStream,
    expected_state: &str,
) -> CliResult<CallbackPayload> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("set oauth callback read timeout failed: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("set oauth callback write timeout failed: {error}"))?;

    let request_target = read_request_target(stream)?;
    let callback_url = format!("http://localhost{request_target}");
    let parsed_url = Url::parse(callback_url.as_str())
        .map_err(|error| format!("parse oauth callback failed: {error}"))?;
    let callback_path = parsed_url.path().to_owned();
    if callback_path != CALLBACK_PATH {
        write_html_response(
            stream,
            "404 Not Found",
            error_html("OAuth callback path not found"),
        )?;
        return Err(format!("unexpected oauth callback path: {callback_path}"));
    }

    let query_map = callback_query_map(&parsed_url);
    let payload =
        callback_payload_from_query_map(&query_map, expected_state).inspect_err(|error| {
            let _ = write_html_response(stream, "400 Bad Request", error_html(error.as_str()));
        })?;
    write_html_response(stream, "200 OK", success_html())?;

    Ok(payload)
}

fn parse_manual_callback_input(
    raw_input: &str,
    expected_redirect_uri: &str,
    expected_state: &str,
) -> CliResult<CallbackPayload> {
    let trimmed = raw_input.trim();
    if trimmed.is_empty() {
        return Err("oauth callback input was empty".to_owned());
    }

    if !looks_like_callback_redirect(trimmed) {
        return Ok(CallbackPayload {
            code: trimmed.to_owned(),
        });
    }

    let parsed_url = normalize_manual_callback_input(trimmed, expected_redirect_uri)?;
    validate_manual_redirect_target(&parsed_url, expected_redirect_uri)?;
    let query_map = callback_query_map(&parsed_url);
    callback_payload_from_query_map(&query_map, expected_state)
}

fn looks_like_callback_redirect(raw_input: &str) -> bool {
    raw_input.starts_with("http://")
        || raw_input.starts_with("https://")
        || raw_input.starts_with('/')
        || raw_input.starts_with('?')
}

fn normalize_manual_callback_input(raw_input: &str, expected_redirect_uri: &str) -> CliResult<Url> {
    if raw_input.starts_with("http://") || raw_input.starts_with("https://") {
        return Url::parse(raw_input)
            .map_err(|error| format!("parse oauth redirect url failed: {error}"));
    }

    if raw_input.starts_with('?') {
        let url = format!("{expected_redirect_uri}{raw_input}");
        return Url::parse(url.as_str())
            .map_err(|error| format!("parse oauth redirect url failed: {error}"));
    }

    let expected_url = Url::parse(expected_redirect_uri)
        .map_err(|error| format!("parse expected oauth redirect url failed: {error}"))?;
    let origin = expected_url.origin().ascii_serialization();
    let url = format!("{origin}{raw_input}");
    Url::parse(url.as_str()).map_err(|error| format!("parse oauth redirect url failed: {error}"))
}

fn validate_manual_redirect_target(parsed_url: &Url, expected_redirect_uri: &str) -> CliResult<()> {
    let expected_url = Url::parse(expected_redirect_uri)
        .map_err(|error| format!("parse expected oauth redirect url failed: {error}"))?;
    let expected_host = expected_url.host_str().unwrap_or_default();
    let parsed_host = parsed_url.host_str().unwrap_or_default();
    let expected_port = expected_url.port_or_known_default();
    let parsed_port = parsed_url.port_or_known_default();
    let matches = parsed_url.scheme() == expected_url.scheme()
        && loopback_hosts_match(parsed_host, expected_host)
        && parsed_port == expected_port
        && parsed_url.path() == expected_url.path();
    if matches {
        return Ok(());
    }

    let message = "pasted redirect URL does not match this login attempt".to_owned();
    Err(message)
}

fn loopback_hosts_match(parsed_host: &str, expected_host: &str) -> bool {
    if parsed_host == expected_host {
        return true;
    }

    loopback_host(parsed_host) && loopback_host(expected_host)
}

fn loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    let parsed_address = host.parse::<IpAddr>();
    let Ok(address) = parsed_address else {
        return false;
    };

    address.is_loopback()
}

fn callback_query_map(parsed_url: &Url) -> BTreeMap<String, String> {
    let query_pairs = parsed_url.query_pairs();
    let mut query_map = BTreeMap::new();
    for (key, value) in query_pairs {
        let owned_key = key.into_owned();
        let owned_value = value.into_owned();
        query_map.insert(owned_key, owned_value);
    }
    query_map
}

fn callback_payload_from_query_map(
    query_map: &BTreeMap<String, String>,
    expected_state: &str,
) -> CliResult<CallbackPayload> {
    let error_code = query_map
        .get("error")
        .map(String::as_str)
        .unwrap_or_default();
    if !error_code.is_empty() {
        let error_description = query_map
            .get("error_description")
            .cloned()
            .unwrap_or_else(|| error_code.to_owned());
        return Err(format!("oauth authorization failed: {error_description}"));
    }

    let returned_state = query_map
        .get("state")
        .map(String::as_str)
        .unwrap_or_default();
    if returned_state != expected_state {
        return Err("oauth state verification failed".to_owned());
    }

    let code = query_map
        .get("code")
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "oauth callback missing authorization code".to_owned())?;

    Ok(CallbackPayload { code })
}

fn read_request_target(stream: &mut TcpStream) -> CliResult<String> {
    let mut buffer = [0_u8; 8192];
    let bytes_read = stream
        .read(&mut buffer)
        .map_err(|error| format!("read oauth callback request failed: {error}"))?;
    if bytes_read == 0 {
        return Err("oauth callback request was empty".to_owned());
    }

    let request_bytes = buffer
        .get(..bytes_read)
        .ok_or_else(|| "oauth callback request exceeded the read buffer".to_owned())?;
    let request_text = String::from_utf8_lossy(request_bytes);
    let request_line = request_text
        .lines()
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "oauth callback request line was empty".to_owned())?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default();
    if method != "GET" {
        return Err(format!("unsupported oauth callback method: {method}"));
    }
    let request_target = request_parts
        .next()
        .map(str::to_owned)
        .ok_or_else(|| "oauth callback request target was missing".to_owned())?;
    Ok(request_target)
}

fn write_html_response(stream: &mut TcpStream, status_line: &str, body: String) -> CliResult<()> {
    let body_length = body.len();
    let response = format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {body_length}\r\nConnection: close\r\n\r\n{body}"
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| format!("write oauth callback response failed: {error}"))?;
    stream
        .flush()
        .map_err(|error| format!("flush oauth callback response failed: {error}"))?;
    Ok(())
}

fn success_html() -> String {
    let html = [
        "<!doctype html>",
        "<html><head><meta charset=\"utf-8\"><title>LoongClaw OAuth</title></head>",
        "<body><h1>LoongClaw is authorized.</h1>",
        "<p>You can return to the terminal now.</p></body></html>",
    ];
    html.join("")
}

fn error_html(message: &str) -> String {
    let escaped_message = html_escape(message);
    let html = [
        "<!doctype html>",
        "<html><head><meta charset=\"utf-8\"><title>LoongClaw OAuth</title></head>",
        "<body><h1>Authorization failed.</h1><p>",
        escaped_message.as_str(),
        "</p></body></html>",
    ];
    html.join("")
}

fn html_escape(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn exchange_authorization_code(
    settings: &OpenaiCodexOauthSettings,
    redirect_uri: &str,
    code_verifier: &str,
    code: &str,
) -> CliResult<AuthorizationCodeExchange> {
    #[derive(Debug, Deserialize)]
    struct AuthorizationCodeExchangeResponse {
        id_token: String,
    }

    let http_client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|error| format!("build oauth token exchange client failed: {error}"))?;
    let token_url = format!("{}/oauth/token", settings.issuer);
    let body = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", settings.client_id.as_str()),
        ("code_verifier", code_verifier),
    ];
    let response = http_client
        .post(token_url)
        .form(&body)
        .send()
        .map_err(|error| format!("exchange oauth authorization code failed: {error}"))?;
    let response_status = response.status();
    if !response_status.is_success() {
        let response_body = response.text().unwrap_or_default();
        let message = format!(
            "oauth authorization code exchange failed with status {response_status}: {response_body}"
        );
        return Err(message);
    }
    let exchange_response = response
        .json::<AuthorizationCodeExchangeResponse>()
        .map_err(|error| format!("parse oauth authorization code exchange failed: {error}"))?;

    Ok(AuthorizationCodeExchange {
        id_token: exchange_response.id_token,
    })
}

fn exchange_id_token_for_api_key(
    settings: &OpenaiCodexOauthSettings,
    id_token: &str,
) -> CliResult<String> {
    #[derive(Debug, Deserialize)]
    struct ApiKeyExchangeResponse {
        access_token: String,
    }

    let http_client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|error| format!("build oauth api-key exchange client failed: {error}"))?;
    let token_url = format!("{}/oauth/token", settings.issuer);
    let body = [
        ("grant_type", API_TOKEN_EXCHANGE_GRANT_TYPE),
        ("client_id", settings.client_id.as_str()),
        ("requested_token", API_TOKEN_EXCHANGE_REQUESTED_TOKEN),
        ("subject_token", id_token),
        ("subject_token_type", API_TOKEN_EXCHANGE_SUBJECT_TOKEN_TYPE),
    ];
    let response = http_client
        .post(token_url)
        .form(&body)
        .send()
        .map_err(|error| format!("exchange oauth id token for OpenAI API token failed: {error}"))?;
    let response_status = response.status();
    if !response_status.is_success() {
        let response_body = response.text().unwrap_or_default();
        let message = format!(
            "oauth OpenAI API token exchange failed with status {response_status}: {response_body}"
        );
        return Err(message);
    }
    let exchange_response = response
        .json::<ApiKeyExchangeResponse>()
        .map_err(|error| format!("parse oauth OpenAI API token exchange failed: {error}"))?;
    Ok(exchange_response.access_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_settings() -> OpenaiCodexOauthSettings {
        OpenaiCodexOauthSettings {
            issuer: "https://auth.openai.test".to_owned(),
            client_id: "client-test-123".to_owned(),
            originator: "loongclaw_test".to_owned(),
            callback_timeout: Duration::from_secs(5),
        }
    }

    #[test]
    fn authorize_url_includes_expected_openai_codex_parameters() {
        let settings = sample_settings();
        let authorize_url = build_authorize_url(
            &settings,
            "http://localhost:1455/auth/callback",
            "challenge-123",
            "state-456",
        )
        .expect("authorize url should build");
        let parsed_url = Url::parse(authorize_url.as_str()).expect("authorize url should parse");
        let query_pairs = parsed_url.query_pairs().collect::<BTreeMap<_, _>>();

        assert_eq!(parsed_url.path(), "/oauth/authorize");
        assert_eq!(
            query_pairs.get("client_id").map(|value| value.as_ref()),
            Some("client-test-123")
        );
        assert_eq!(
            query_pairs.get("redirect_uri").map(|value| value.as_ref()),
            Some("http://localhost:1455/auth/callback")
        );
        assert_eq!(
            query_pairs.get("scope").map(|value| value.as_ref()),
            Some(AUTH_SCOPE)
        );
        assert_eq!(
            query_pairs
                .get("code_challenge")
                .map(|value| value.as_ref()),
            Some("challenge-123")
        );
        assert_eq!(
            query_pairs
                .get("code_challenge_method")
                .map(|value| value.as_ref()),
            Some("S256")
        );
        assert_eq!(
            query_pairs.get("state").map(|value| value.as_ref()),
            Some("state-456")
        );
        assert_eq!(
            query_pairs.get("originator").map(|value| value.as_ref()),
            Some("loongclaw_test")
        );
    }

    #[test]
    fn callback_request_returns_authorization_code_for_matching_state() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = listener.local_addr().expect("listener address");
        let sender = thread::spawn(move || {
            let mut client = TcpStream::connect(address).expect("client should connect");
            let request = "GET /auth/callback?code=code-123&state=state-123 HTTP/1.1\r\nHost: localhost\r\n\r\n";
            client
                .write_all(request.as_bytes())
                .expect("client should write callback");
            let mut response = String::new();
            let _ = client.read_to_string(&mut response);
            response
        });

        let (mut server_stream, _server_address) = listener.accept().expect("server accept");
        let payload =
            process_callback_request(&mut server_stream, "state-123").expect("callback payload");
        drop(server_stream);

        let response = sender.join().expect("sender thread");

        assert_eq!(payload.code, "code-123");
        assert!(response.contains("200 OK"));
        assert!(response.contains("LoongClaw is authorized."));
    }

    #[test]
    fn callback_redirect_uri_uses_registered_localhost_origin() {
        let redirect_uri = callback_redirect_uri(DEFAULT_CALLBACK_PORT);

        assert_eq!(redirect_uri, "http://localhost:1455/auth/callback");
    }

    #[test]
    fn manual_callback_input_accepts_full_redirect_url() {
        let payload = parse_manual_callback_input(
            "http://localhost:1455/auth/callback?code=manual-code-123&state=state-123",
            "http://localhost:1455/auth/callback",
            "state-123",
        )
        .expect("manual redirect url should parse");

        assert_eq!(payload.code, "manual-code-123");
    }

    #[test]
    fn manual_callback_input_accepts_loopback_alias_for_localhost_redirect() {
        let payload = parse_manual_callback_input(
            "http://127.0.0.1:1455/auth/callback?code=manual-code-123&state=state-123",
            "http://localhost:1455/auth/callback",
            "state-123",
        )
        .expect("loopback alias should be accepted for localhost redirect");

        assert_eq!(payload.code, "manual-code-123");
    }

    #[test]
    fn manual_callback_input_rejects_stale_state() {
        let error = parse_manual_callback_input(
            "http://localhost:1455/auth/callback?code=manual-code-123&state=stale-state",
            "http://localhost:1455/auth/callback",
            "state-123",
        )
        .expect_err("stale state should be rejected");

        assert!(
            error.contains("state"),
            "manual redirect error should mention state mismatch: {error}"
        );
    }

    #[test]
    fn manual_callback_input_accepts_raw_authorization_code_fallback() {
        let payload = parse_manual_callback_input(
            "manual-code-123",
            "http://localhost:1455/auth/callback",
            "state-123",
        )
        .expect("raw authorization code should be accepted as a fallback");

        assert_eq!(payload.code, "manual-code-123");
    }
}
