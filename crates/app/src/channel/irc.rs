use std::sync::Arc;
use std::time::Duration;

use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector as TokioTlsConnector;

use crate::{
    CliResult,
    config::{
        IrcServerEndpoint, IrcServerTransport, ResolvedIrcChannelConfig, parse_irc_server_endpoint,
    },
};

use super::ChannelOutboundTargetKind;

trait IrcIo: AsyncRead + AsyncWrite + Send + Unpin {}

impl<T> IrcIo for T where T: AsyncRead + AsyncWrite + Send + Unpin {}

const IRC_CHANNEL_PREFIXES: [char; 4] = ['#', '&', '+', '!'];
const IRC_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IRC_SESSION_READY_TIMEOUT: Duration = Duration::from_secs(30);
const IRC_MAX_FRAME_BYTES: usize = 512;
const IRC_MAX_COMMAND_BYTES: usize = IRC_MAX_FRAME_BYTES - 2;

pub(super) async fn run_irc_send(
    resolved: &ResolvedIrcChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
) -> CliResult<()> {
    run_irc_send_with_timeouts(
        resolved,
        target_kind,
        target_id,
        text,
        IRC_CONNECT_TIMEOUT,
        IRC_SESSION_READY_TIMEOUT,
    )
    .await
}

async fn run_irc_send_with_timeouts(
    resolved: &ResolvedIrcChannelConfig,
    target_kind: ChannelOutboundTargetKind,
    target_id: &str,
    text: &str,
    connect_timeout: Duration,
    session_ready_timeout: Duration,
) -> CliResult<()> {
    ensure_irc_target_kind(target_kind)?;

    let target = normalize_irc_target(target_id)?;
    let message_lines = normalize_irc_message_lines(text)?;

    let server = resolved
        .server()
        .ok_or_else(|| "irc server missing (set irc.server or env)".to_owned())?;
    let endpoint = parse_irc_server_endpoint(server.as_str())?;

    let nickname_value = resolved
        .nickname()
        .ok_or_else(|| "irc nickname missing (set irc.nickname or env)".to_owned())?;
    let nickname = normalize_irc_atom("nickname", nickname_value.as_str())?;

    let resolved_username = resolved.username().unwrap_or(nickname.as_str());
    let username = normalize_irc_atom("username", resolved_username)?;

    let resolved_realname = resolved.realname().unwrap_or(username.as_str());
    let realname = normalize_irc_realname(resolved_realname)?;

    let password = resolved.password();
    let password = normalize_optional_irc_password(password.as_deref())?;
    ensure_irc_password_transport(&endpoint, password.as_deref())?;

    let transport = connect_irc_stream(&endpoint, connect_timeout).await?;
    run_irc_send_session(
        transport,
        target.as_str(),
        nickname.as_str(),
        username.as_str(),
        realname.as_str(),
        password.as_deref(),
        &message_lines,
        session_ready_timeout,
    )
    .await
}

fn ensure_irc_target_kind(target_kind: ChannelOutboundTargetKind) -> CliResult<()> {
    if target_kind == ChannelOutboundTargetKind::Conversation {
        return Ok(());
    }

    Err(format!(
        "irc send requires conversation target kind, got {}",
        target_kind.as_str()
    ))
}

fn normalize_irc_target(raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();
    if trimmed.contains(',') {
        return Err("irc target must not contain multiple recipients".to_owned());
    }
    normalize_irc_atom("target", trimmed)
}

fn normalize_irc_atom(label: &str, raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("irc {label} is empty"));
    }
    let contains_whitespace = trimmed.chars().any(char::is_whitespace);
    if contains_whitespace {
        return Err(format!("irc {label} must not contain whitespace"));
    }
    if trimmed.contains('\r') || trimmed.contains('\n') || trimmed.contains('\0') {
        return Err(format!("irc {label} contains forbidden control characters"));
    }
    Ok(trimmed.to_owned())
}

fn normalize_irc_realname(raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("irc realname is empty".to_owned());
    }
    if trimmed.contains('\r') || trimmed.contains('\n') || trimmed.contains('\0') {
        return Err("irc realname contains forbidden control characters".to_owned());
    }
    Ok(trimmed.to_owned())
}

fn normalize_optional_irc_password(raw: Option<&str>) -> CliResult<Option<String>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.contains('\r') || trimmed.contains('\n') || trimmed.contains('\0') {
        return Err("irc password contains forbidden control characters".to_owned());
    }
    Ok(Some(trimmed.to_owned()))
}

fn normalize_irc_message_lines(text: &str) -> CliResult<Vec<String>> {
    let mut lines = Vec::new();

    for raw_line in text.lines() {
        let line = raw_line.trim_end();
        let visible = line.trim();
        if visible.is_empty() {
            continue;
        }
        let contains_carriage_return = line.contains('\r');
        let contains_null = line.contains('\0');
        if contains_carriage_return || contains_null {
            return Err(
                "irc send text contains forbidden control characters (`\\r` / `\\0`)".to_owned(),
            );
        }
        lines.push(line.to_owned());
    }

    if lines.is_empty() {
        return Err("irc send text is empty".to_owned());
    }

    Ok(lines)
}

fn ensure_irc_password_transport(
    endpoint: &IrcServerEndpoint,
    password: Option<&str>,
) -> CliResult<()> {
    let has_password = password.is_some();
    let uses_plain_transport = endpoint.transport == IrcServerTransport::Plain;
    if has_password && uses_plain_transport {
        return Err(
            "irc password requires an `ircs://` server endpoint; refusing to send PASS over plaintext transport"
                .to_owned(),
        );
    }

    Ok(())
}

async fn connect_irc_stream(
    endpoint: &IrcServerEndpoint,
    connect_timeout: Duration,
) -> CliResult<Box<dyn IrcIo>> {
    let address = format!("{}:{}", endpoint.host, endpoint.port);
    let connect_result = timeout(connect_timeout, TcpStream::connect(address.as_str())).await;
    let tcp_stream = connect_result
        .map_err(|_timeout_error| {
            let formatted_timeout = format_irc_timeout(connect_timeout);
            format!("connect irc server timed out after {formatted_timeout}")
        })?
        .map_err(|error| format!("connect irc server failed: {error}"))?;

    if endpoint.transport == IrcServerTransport::Plain {
        return Ok(Box::new(tcp_stream));
    }

    ensure_irc_tls_provider();
    let tls_config = build_irc_tls_config()?;
    let tls_config = Arc::new(tls_config);
    let tokio_tls_connector = TokioTlsConnector::from(tls_config);
    let server_name_value = endpoint.host.as_str();
    let server_name = ServerName::try_from(server_name_value).map_err(|error| {
        format!(
            "irc tls server name is invalid: {} ({error})",
            endpoint.host
        )
    })?;
    let server_name = server_name.to_owned();
    let tls_connect_result = timeout(
        connect_timeout,
        tokio_tls_connector.connect(server_name, tcp_stream),
    )
    .await;
    let tls_stream = tls_connect_result
        .map_err(|_timeout_error| {
            let formatted_timeout = format_irc_timeout(connect_timeout);
            format!("connect irc tls session timed out after {formatted_timeout}")
        })?
        .map_err(|error| format!("connect irc tls session failed: {error}"))?;
    Ok(Box::new(tls_stream))
}

fn ensure_irc_tls_provider() {
    #[allow(clippy::disallowed_methods)]
    {
        let crypto_provider = rustls::crypto::CryptoProvider::get_default();
        if crypto_provider.is_none() {
            let default_provider = rustls::crypto::ring::default_provider();
            let _ = default_provider.install_default();
        }
    }
}

fn build_irc_tls_config() -> CliResult<ClientConfig> {
    let root_store = load_irc_root_store()?;
    let config_builder = ClientConfig::builder();
    let config_builder = config_builder.with_root_certificates(root_store);
    let config = config_builder.with_no_client_auth();
    Ok(config)
}

fn load_irc_root_store() -> CliResult<RootCertStore> {
    let certificate_result = rustls_native_certs::load_native_certs();
    let certs = certificate_result.certs;
    let errors = certificate_result.errors;
    let total_certificate_count = certs.len();
    let mut root_store = RootCertStore::empty();
    let (added_certificate_count, ignored_certificate_count) =
        root_store.add_parsable_certificates(certs);

    if added_certificate_count > 0 {
        return Ok(root_store);
    }

    let error = format!(
        "load irc tls root certificates failed: no usable root certificates were found (loaded {total_certificate_count}, ignored {ignored_certificate_count}, errors: {errors:?})"
    );
    Err(error)
}

async fn run_irc_send_session(
    stream: Box<dyn IrcIo>,
    target: &str,
    nickname: &str,
    username: &str,
    realname: &str,
    password: Option<&str>,
    message_lines: &[String],
    session_ready_timeout: Duration,
) -> CliResult<()> {
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);

    if let Some(password) = password {
        let pass_command = format!("PASS {password}");
        send_irc_command(&mut write_half, pass_command.as_str(), "irc pass").await?;
    }

    let nick_command = format!("NICK {nickname}");
    send_irc_command(&mut write_half, nick_command.as_str(), "irc nick").await?;

    let user_command = format!("USER {username} 0 * :{realname}");
    send_irc_command(&mut write_half, user_command.as_str(), "irc user").await?;

    let ready_future = wait_for_irc_session_ready(&mut reader, &mut write_half, target);
    let ready_result = timeout(session_ready_timeout, ready_future).await;
    let ready_result = ready_result.map_err(|_timeout_error| {
        let formatted_timeout = format_irc_timeout(session_ready_timeout);
        format!("irc send timed out after {formatted_timeout} while waiting for server readiness")
    })?;
    ready_result?;

    for message_line in message_lines {
        let privmsg_command = format!("PRIVMSG {target} :{message_line}");
        send_irc_command(&mut write_half, privmsg_command.as_str(), "irc privmsg").await?;
    }

    send_irc_command(&mut write_half, "QUIT :loongclaw send complete", "irc quit").await?;
    Ok(())
}

async fn wait_for_irc_session_ready<R, W>(
    reader: &mut BufReader<R>,
    writer: &mut W,
    target: &str,
) -> CliResult<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    wait_for_irc_welcome(reader, writer).await?;

    if !irc_target_requires_join(target) {
        return Ok(());
    }

    let join_command = format!("JOIN {target}");
    send_irc_command(writer, join_command.as_str(), "irc join").await?;
    wait_for_irc_join(reader, writer).await
}

async fn send_irc_command<W>(writer: &mut W, command: &str, context: &str) -> CliResult<()>
where
    W: AsyncWrite + Unpin,
{
    let command_bytes = command.as_bytes();
    let command_length = command_bytes.len();
    if command_length > IRC_MAX_COMMAND_BYTES {
        return Err(format!(
            "{context} failed: irc command exceeds the {IRC_MAX_COMMAND_BYTES}-byte limit before CRLF"
        ));
    }

    writer
        .write_all(command_bytes)
        .await
        .map_err(|error| format!("{context} failed: {error}"))?;
    writer
        .write_all(b"\r\n")
        .await
        .map_err(|error| format!("{context} failed: {error}"))?;
    writer
        .flush()
        .await
        .map_err(|error| format!("{context} failed: {error}"))?;
    Ok(())
}

async fn wait_for_irc_welcome<R, W>(reader: &mut BufReader<R>, writer: &mut W) -> CliResult<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    loop {
        let line = read_irc_line(reader, writer, "welcome").await?;
        let command = parse_irc_command(line.as_str());
        if command == Some("001") {
            return Ok(());
        }
        if is_irc_registration_error(command) {
            return Err(format!("irc registration failed: {line}"));
        }
    }
}

async fn wait_for_irc_join<R, W>(reader: &mut BufReader<R>, writer: &mut W) -> CliResult<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    loop {
        let line = read_irc_line(reader, writer, "join").await?;
        let command = parse_irc_command(line.as_str());
        if command == Some("366") {
            return Ok(());
        }
        if is_irc_join_error(command) {
            return Err(format!("irc join failed: {line}"));
        }
    }
}

async fn read_irc_line<R, W>(
    reader: &mut BufReader<R>,
    writer: &mut W,
    context: &str,
) -> CliResult<String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    loop {
        let line_bytes = read_bounded_irc_line_bytes(reader, context).await?;
        let line = String::from_utf8(line_bytes)
            .map_err(|error| format!("read irc {context} line failed: {error}"))?;
        let trimmed_line = line.trim_end_matches(['\r', '\n']).to_owned();
        if let Some(payload) = parse_irc_ping_payload(trimmed_line.as_str()) {
            let pong_command = format!("PONG :{payload}");
            send_irc_command(writer, pong_command.as_str(), "irc pong").await?;
            continue;
        }

        return Ok(trimmed_line);
    }
}

async fn read_bounded_irc_line_bytes<R>(
    reader: &mut BufReader<R>,
    context: &str,
) -> CliResult<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut line_bytes = Vec::new();

    loop {
        let available = reader
            .fill_buf()
            .await
            .map_err(|error| format!("read irc {context} line failed: {error}"))?;
        if available.is_empty() {
            if line_bytes.is_empty() {
                return Err(format!(
                    "irc server closed connection while waiting for {context}"
                ));
            }
            return Ok(line_bytes);
        }

        let newline_position = available.iter().position(|byte| *byte == b'\n');
        let consume_count = match newline_position {
            Some(position) => position + 1,
            None => available.len(),
        };
        let next_line_length = line_bytes.len() + consume_count;
        if next_line_length > IRC_MAX_FRAME_BYTES {
            return Err(format!(
                "irc server sent a line longer than {IRC_MAX_FRAME_BYTES} bytes while waiting for {context}"
            ));
        }

        let chunk = available
            .get(..consume_count)
            .ok_or_else(|| format!("read irc {context} line failed: internal framing error"))?;
        line_bytes.extend_from_slice(chunk);
        reader.consume(consume_count);

        if newline_position.is_some() {
            return Ok(line_bytes);
        }
    }
}

fn parse_irc_command(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let payload = if trimmed.starts_with(':') {
        let mut parts = trimmed.splitn(2, ' ');
        let _prefix = parts.next();
        parts.next().unwrap_or_default()
    } else {
        trimmed
    };

    payload.split_whitespace().next()
}

fn parse_irc_ping_payload(line: &str) -> Option<&str> {
    if let Some(payload) = line.strip_prefix("PING :") {
        return Some(payload.trim());
    }
    if let Some(payload) = line.strip_prefix("PING ") {
        return Some(payload.trim());
    }
    None
}

fn format_irc_timeout(timeout: Duration) -> String {
    let timeout_ms = timeout.as_millis();
    let is_second_aligned = timeout_ms.is_multiple_of(1_000);
    if is_second_aligned {
        let timeout_s = timeout_ms / 1_000;
        return format!("{timeout_s}s");
    }

    format!("{timeout_ms}ms")
}

fn is_irc_registration_error(command: Option<&str>) -> bool {
    matches!(
        command,
        Some("431" | "432" | "433" | "436" | "464" | "465" | "466")
    )
}

fn is_irc_join_error(command: Option<&str>) -> bool {
    matches!(
        command,
        Some("403" | "404" | "405" | "471" | "472" | "473" | "474" | "475" | "476" | "477")
    )
}

fn irc_target_requires_join(target: &str) -> bool {
    let Some(first_char) = target.chars().next() else {
        return false;
    };
    IRC_CHANNEL_PREFIXES.contains(&first_char)
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;

    use crate::config::IrcChannelConfig;

    use super::*;

    #[test]
    fn parse_irc_server_endpoint_accepts_bare_host() {
        let endpoint = parse_irc_server_endpoint("irc.example.test").expect("parse bare irc host");

        assert_eq!(endpoint.transport, IrcServerTransport::Plain);
        assert_eq!(endpoint.host, "irc.example.test");
        assert_eq!(endpoint.port, 6667);
    }

    #[test]
    fn parse_irc_server_endpoint_accepts_ircs_url() {
        let endpoint =
            parse_irc_server_endpoint("ircs://irc.example.test:6697").expect("parse ircs url");

        assert_eq!(endpoint.transport, IrcServerTransport::Tls);
        assert_eq!(endpoint.host, "irc.example.test");
        assert_eq!(endpoint.port, 6697);
    }

    #[test]
    fn parse_irc_server_endpoint_rejects_bare_host_port() {
        let error = parse_irc_server_endpoint("irc.example.test:6667")
            .expect_err("bare host:port should be rejected");

        assert!(
            error.contains("bare `host:port` is not supported"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn parse_irc_server_endpoint_rejects_bare_host_with_whitespace() {
        let error = parse_irc_server_endpoint("irc.example test")
            .expect_err("bare host with whitespace should be rejected");

        assert!(
            error.contains("must not contain whitespace"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn ensure_irc_target_kind_rejects_non_conversation_targets() {
        let error = ensure_irc_target_kind(ChannelOutboundTargetKind::Address)
            .expect_err("address target kind should be rejected");

        assert!(
            error.contains("irc send requires conversation target kind"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn normalize_irc_target_rejects_multiple_recipients() {
        let error = normalize_irc_target("#ops,#alerts")
            .expect_err("multi-recipient target should be rejected");

        assert!(
            error.contains("must not contain multiple recipients"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn normalize_irc_atom_rejects_tab_whitespace() {
        let error = normalize_irc_atom("nickname", "loong\tclaw")
            .expect_err("nickname containing a tab should be rejected");

        assert!(
            error.contains("must not contain whitespace"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn normalize_irc_message_lines_rejects_blank_message() {
        let error =
            normalize_irc_message_lines(" \n\t").expect_err("blank irc message should be rejected");

        assert_eq!(error, "irc send text is empty");
    }

    #[test]
    fn normalize_irc_message_lines_rejects_carriage_returns() {
        let error = normalize_irc_message_lines("hello\rworld")
            .expect_err("carriage return should be rejected");

        assert!(
            error.contains("forbidden control characters"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn parse_irc_server_endpoint_rejects_zero_port() {
        let error = parse_irc_server_endpoint("irc://irc.example.test:0")
            .expect_err("port zero should be rejected");

        assert!(
            error.contains("between 1 and 65535"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn send_irc_command_rejects_command_longer_than_limit() {
        let mut sink = tokio::io::sink();
        let long_command = "x".repeat(IRC_MAX_COMMAND_BYTES + 1);

        let error = send_irc_command(&mut sink, long_command.as_str(), "irc test")
            .await
            .expect_err("oversized command should be rejected");

        assert!(
            error.contains("510-byte limit"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn read_irc_line_rejects_line_longer_than_limit() {
        let (mut client, server) = tokio::io::duplex(2048);
        let long_line = format!("{}\r\n", "x".repeat(IRC_MAX_FRAME_BYTES));
        let write_task = tokio::spawn(async move {
            client
                .write_all(long_line.as_bytes())
                .await
                .expect("write oversized irc line");
        });
        let mut reader = BufReader::new(server);
        let mut writer = tokio::io::sink();

        let error = read_irc_line(&mut reader, &mut writer, "welcome")
            .await
            .expect_err("oversized inbound line should be rejected");

        assert!(
            error.contains("longer than 512 bytes"),
            "unexpected error: {error}"
        );

        write_task.await.expect("join oversized line writer");
    }

    #[tokio::test]
    async fn run_irc_send_rejects_password_over_plaintext_transport() {
        let config = IrcChannelConfig {
            enabled: true,
            server: Some("irc://irc.example.test".to_owned()),
            nickname: Some("loongclaw_bot".to_owned()),
            password: Some(loongclaw_contracts::SecretRef::Inline(
                "server-password".to_owned(),
            )),
            ..IrcChannelConfig::default()
        };
        let resolved = config.resolve_account(None).expect("resolve irc config");

        let error = run_irc_send(
            &resolved,
            ChannelOutboundTargetKind::Conversation,
            "#ops",
            "hello from irc",
        )
        .await
        .expect_err("plaintext password transport should be rejected");

        assert!(
            error.contains("refusing to send PASS over plaintext"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn run_irc_send_joins_channel_and_sends_privmsg() {
        let (server, task) = spawn_mock_irc_server(true).await;
        let config = IrcChannelConfig {
            enabled: true,
            server: Some(server),
            nickname: Some("loongclaw_bot".to_owned()),
            username: Some("loongclaw".to_owned()),
            realname: Some("LoongClaw Bot".to_owned()),
            ..IrcChannelConfig::default()
        };
        let resolved = config.resolve_account(None).expect("resolve irc config");

        run_irc_send(
            &resolved,
            ChannelOutboundTargetKind::Conversation,
            "#ops",
            "hello from irc",
        )
        .await
        .expect("run irc send");

        let frames = task
            .await
            .expect("join irc server")
            .expect("irc server result");
        assert_eq!(frames[0], "NICK loongclaw_bot");
        assert_eq!(frames[1], "USER loongclaw 0 * :LoongClaw Bot");
        assert_eq!(frames[2], "JOIN #ops");
        assert_eq!(frames[3], "PRIVMSG #ops :hello from irc");
        assert_eq!(frames[4], "QUIT :loongclaw send complete");
    }

    #[tokio::test]
    async fn run_irc_send_times_out_while_waiting_for_welcome() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock irc server");
        let address = listener.local_addr().expect("mock irc server address");
        let server_task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.map_err(|error| error.to_string())?;
            let _socket = socket;
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok::<(), String>(())
        });

        let config = IrcChannelConfig {
            enabled: true,
            server: Some(format!("irc://{address}")),
            nickname: Some("loongclaw_bot".to_owned()),
            username: Some("loongclaw".to_owned()),
            realname: Some("LoongClaw Bot".to_owned()),
            ..IrcChannelConfig::default()
        };
        let resolved = config.resolve_account(None).expect("resolve irc config");

        let error = run_irc_send_with_timeouts(
            &resolved,
            ChannelOutboundTargetKind::Conversation,
            "#ops",
            "hello from irc",
            Duration::from_secs(1),
            Duration::from_millis(50),
        )
        .await
        .expect_err("welcome timeout should be reported");

        assert!(
            error.contains("waiting for server readiness"),
            "unexpected error: {error}"
        );

        server_task
            .await
            .expect("join timeout server task")
            .expect("timeout server task should complete");
    }

    async fn spawn_mock_irc_server(
        expect_join: bool,
    ) -> (String, tokio::task::JoinHandle<CliResult<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock irc server");
        let address = listener.local_addr().expect("mock irc server address");
        let task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.map_err(|error| error.to_string())?;
            let (read_half, mut write_half) = socket.into_split();
            let mut reader = BufReader::new(read_half);
            let mut frames = Vec::new();

            let nick = read_mock_irc_line(&mut reader).await?;
            frames.push(nick);

            let user = read_mock_irc_line(&mut reader).await?;
            frames.push(user);

            write_half
                .write_all(b":server 001 loongclaw_bot :welcome\r\n")
                .await
                .map_err(|error| format!("write irc welcome failed: {error}"))?;

            if expect_join {
                let join = read_mock_irc_line(&mut reader).await?;
                frames.push(join);

                write_half
                    .write_all(b":loongclaw_bot!user@example JOIN #ops\r\n")
                    .await
                    .map_err(|error| format!("write irc join event failed: {error}"))?;
                write_half
                    .write_all(b":server 366 loongclaw_bot #ops :End of /NAMES list.\r\n")
                    .await
                    .map_err(|error| format!("write irc names end failed: {error}"))?;
            }

            let privmsg = read_mock_irc_line(&mut reader).await?;
            frames.push(privmsg);

            let quit = read_mock_irc_line(&mut reader).await?;
            frames.push(quit);

            Ok(frames)
        });

        (format!("irc://{}", address), task)
    }

    async fn read_mock_irc_line<R>(reader: &mut BufReader<R>) -> CliResult<String>
    where
        R: AsyncRead + Unpin,
    {
        let mut line = String::new();
        let byte_count = reader
            .read_line(&mut line)
            .await
            .map_err(|error| format!("read mock irc line failed: {error}"))?;
        if byte_count == 0 {
            return Err("mock irc peer disconnected".to_owned());
        }
        Ok(line.trim_end_matches(['\r', '\n']).to_owned())
    }
}
