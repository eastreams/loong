use super::latest_selector_process_support::LatestSelectorCliFixture;
use loong_app::config::{ProviderKind, ProviderWireApi};
use loong_contracts::SecretRef;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

const MOCK_PROVIDER_REPLY: &str = "process latest selector ask reply";
const MOCK_PROVIDER_STREAM_READ_TIMEOUT: Duration = Duration::from_secs(5);

fn render_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn header_end_offset(bytes: &[u8]) -> Option<usize> {
    let marker = b"\r\n\r\n";
    let position = bytes
        .windows(marker.len())
        .position(|window| window == marker)?;
    Some(position + marker.len())
}

fn parse_content_length(bytes: &[u8]) -> Option<usize> {
    let header_text = String::from_utf8_lossy(bytes);
    for line in header_text.lines() {
        let lower_line = line.to_ascii_lowercase();
        if !lower_line.starts_with("content-length:") {
            continue;
        }

        let (_, value) = line.split_once(':')?;
        let trimmed_value = value.trim();
        let parsed_value = trimmed_value.parse::<usize>().ok()?;
        return Some(parsed_value);
    }

    None
}

fn read_provider_request(stream: &mut TcpStream) -> String {
    stream
        .set_nonblocking(false)
        .expect("set provider stream blocking");
    stream
        .set_read_timeout(Some(MOCK_PROVIDER_STREAM_READ_TIMEOUT))
        .expect("set provider stream read timeout");
    let mut request_bytes = Vec::new();
    let mut read_buffer = [0_u8; 4096];
    let mut expected_total_length = None::<usize>;

    loop {
        let read_len = stream
            .read(&mut read_buffer)
            .expect("read provider request");
        if read_len == 0 {
            break;
        }

        let chunk = read_buffer
            .get(..read_len)
            .expect("provider request length should fit within the read buffer");
        request_bytes.extend_from_slice(chunk);

        if expected_total_length.is_none()
            && let Some(header_end) = header_end_offset(request_bytes.as_slice())
        {
            let header_bytes = request_bytes
                .get(..header_end)
                .expect("header_end should be within request bytes");
            let content_length = parse_content_length(header_bytes).unwrap_or(0);
            expected_total_length = Some(header_end + content_length);
        }

        if let Some(expected_total_length) = expected_total_length
            && request_bytes.len() >= expected_total_length
        {
            break;
        }
    }

    String::from_utf8_lossy(request_bytes.as_slice()).into_owned()
}

fn provider_request_json_body(request: &str) -> Value {
    let request_bytes = request.as_bytes();
    let header_end = header_end_offset(request_bytes).expect("provider request header end");
    let body_bytes = request_bytes
        .get(header_end..)
        .expect("provider request body should start after headers");
    let body_text = String::from_utf8_lossy(body_bytes);
    serde_json::from_str(body_text.as_ref()).expect("provider request body should be json")
}

fn json_value_contains_text(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text.contains(needle),
        Value::Array(items) => items
            .iter()
            .any(|item| json_value_contains_text(item, needle)),
        Value::Object(entries) => entries
            .values()
            .any(|entry| json_value_contains_text(entry, needle)),
        _ => false,
    }
}

fn assert_provider_request_contains_text(request: &str, needle: &str, context: &str) {
    let body = provider_request_json_body(request);
    let contains_text = json_value_contains_text(&body, needle);
    assert!(
        contains_text,
        "{context} should contain {needle:?}; body={body:#?}"
    );
}

fn extract_json_string_field_from_text(text: &str, field_name: &str) -> Option<String> {
    let compact_marker = format!("\"{field_name}\":\"");
    if let Some(value) = extract_until_quote_after_marker(text, compact_marker.as_str()) {
        return Some(value);
    }

    let spaced_marker = format!("\"{field_name}\": \"");
    if let Some(value) = extract_until_quote_after_marker(text, spaced_marker.as_str()) {
        return Some(value);
    }

    let escaped_compact_marker = format!("\\\"{field_name}\\\":\\\"");
    if let Some(value) = extract_until_quote_after_marker(text, escaped_compact_marker.as_str()) {
        return Some(value);
    }

    let escaped_spaced_marker = format!("\\\"{field_name}\\\": \\\"");
    extract_until_quote_after_marker(text, escaped_spaced_marker.as_str())
}

fn extract_until_quote_after_marker(text: &str, marker: &str) -> Option<String> {
    let start = text.find(marker)?;
    let value_start = start.saturating_add(marker.len());
    let value_text = text.get(value_start..)?;
    let raw_value = value_text
        .chars()
        .take_while(|character| *character != '"')
        .collect::<String>();
    let value = raw_value.trim_end_matches('\\').to_owned();
    (!value.is_empty()).then_some(value)
}

fn find_json_string_field_in_json_value(value: &Value, field_name: &str) -> Option<String> {
    match value {
        Value::String(text) => extract_json_string_field_from_text(text, field_name),
        Value::Array(items) => items
            .iter()
            .find_map(|item| find_json_string_field_in_json_value(item, field_name)),
        Value::Object(entries) => {
            if let Some(field_value) = entries.get(field_name).and_then(Value::as_str) {
                return Some(field_value.to_owned());
            }
            entries
                .values()
                .find_map(|entry| find_json_string_field_in_json_value(entry, field_name))
        }
        _ => None,
    }
}

fn extract_browser_session_id_from_request(request: &str) -> String {
    let body = provider_request_json_body(request);
    find_json_string_field_in_json_value(&body, "session_id")
        .filter(|session_id| session_id.starts_with("browser-"))
        .expect("provider follow-up request should include a browser session id")
}

fn extract_tool_lease_from_request(request: &str) -> String {
    let body = provider_request_json_body(request);
    find_json_string_field_in_json_value(&body, "lease")
        .expect("provider follow-up request should include a tool lease")
}

struct MockProviderResponse {
    status_line: String,
    body: String,
}

impl MockProviderResponse {
    fn ok_json(body: Value) -> Self {
        let body = serde_json::to_string(&body).expect("mock provider body should encode");
        Self {
            status_line: "HTTP/1.1 200 OK".to_owned(),
            body,
        }
    }

    fn unexpected_extra_request() -> Self {
        Self {
            status_line: "HTTP/1.1 500 Internal Server Error".to_owned(),
            body: r#"{"error":{"message":"unexpected extra request"}}"#.to_owned(),
        }
    }
}

enum MockProviderServerControl {
    Start,
    Shutdown,
}

struct MockProviderServer {
    base_url: String,
    control_sender: mpsc::Sender<MockProviderServerControl>,
    join_handle: std::thread::JoinHandle<Vec<String>>,
}

impl MockProviderServer {
    fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider listener");
        let address = listener.local_addr().expect("local provider address");
        let (control_sender, control_receiver) = mpsc::channel();
        let join_handle = std::thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("set local provider listener nonblocking");
            let start_signal = control_receiver
                .recv()
                .expect("receive provider server start signal");
            match start_signal {
                MockProviderServerControl::Start => {}
                MockProviderServerControl::Shutdown => return Vec::new(),
            }

            let mut requests = Vec::new();

            loop {
                let control_message = control_receiver.try_recv();
                match control_message {
                    Ok(MockProviderServerControl::Shutdown) => return requests,
                    Ok(MockProviderServerControl::Start) => {}
                    Err(TryRecvError::Disconnected) => return requests,
                    Err(TryRecvError::Empty) => {}
                }

                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let request = read_provider_request(&mut stream);
                        requests.push(request.clone());

                        let (status_line, response_body) = if request
                            .starts_with("POST /v1/responses ")
                        {
                            (
                                "HTTP/1.1 200 OK",
                                format!(r#"{{"output_text":"{MOCK_PROVIDER_REPLY}"}}"#),
                            )
                        } else if request.starts_with("POST /v1/chat/completions ") {
                            (
                                "HTTP/1.1 200 OK",
                                format!(
                                    r#"{{"choices":[{{"message":{{"role":"assistant","content":"{MOCK_PROVIDER_REPLY}"}}}}]}}"#
                                ),
                            )
                        } else {
                            (
                                "HTTP/1.1 404 Not Found",
                                r#"{"error":{"message":"unexpected request"}}"#.to_owned(),
                            )
                        };
                        let response = format!(
                            "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            response_body.len(),
                            response_body
                        );
                        stream
                            .write_all(response.as_bytes())
                            .expect("write provider response");

                        return requests;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::yield_now();
                    }
                    Err(error) => panic!("accept provider request: {error}"),
                }
            }
        });
        let base_url = format!("http://{address}");

        Self {
            base_url,
            control_sender,
            join_handle,
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn arm(&self) {
        self.control_sender
            .send(MockProviderServerControl::Start)
            .expect("start local provider server");
    }

    fn finish(self, stdout: &str, stderr: &str) -> Vec<String> {
        let shutdown_result = self
            .control_sender
            .send(MockProviderServerControl::Shutdown);
        if let Err(_error) = shutdown_result {}

        match self.join_handle.join() {
            Ok(requests) => requests,
            Err(payload) => {
                panic!(
                    "join local provider server failed, stdout={stdout:?}, stderr={stderr:?}, panic={payload:?}"
                );
            }
        }
    }
}

struct ScriptedMockProviderServer {
    base_url: String,
    control_sender: mpsc::Sender<MockProviderServerControl>,
    join_handle: std::thread::JoinHandle<Vec<String>>,
}

impl ScriptedMockProviderServer {
    fn spawn(responses: Vec<MockProviderResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider listener");
        let address = listener.local_addr().expect("local provider address");
        let (control_sender, control_receiver) = mpsc::channel();
        let join_handle = std::thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("set local provider listener nonblocking");
            let start_signal = control_receiver
                .recv()
                .expect("receive provider server start signal");
            match start_signal {
                MockProviderServerControl::Start => {}
                MockProviderServerControl::Shutdown => return Vec::new(),
            }

            let mut responses = VecDeque::from(responses);
            let mut requests = Vec::new();

            loop {
                let control_message = control_receiver.try_recv();
                match control_message {
                    Ok(MockProviderServerControl::Shutdown) => return requests,
                    Ok(MockProviderServerControl::Start) => {}
                    Err(TryRecvError::Disconnected) => return requests,
                    Err(TryRecvError::Empty) => {}
                }

                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let request = read_provider_request(&mut stream);
                        requests.push(request);
                        let response = responses
                            .pop_front()
                            .unwrap_or_else(MockProviderResponse::unexpected_extra_request);
                        write_provider_response(&mut stream, &response);

                        if responses.is_empty() {
                            return requests;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::yield_now();
                    }
                    Err(error) => panic!("accept provider request: {error}"),
                }
            }
        });
        let base_url = format!("http://{address}");

        Self {
            base_url,
            control_sender,
            join_handle,
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn arm(&self) {
        self.control_sender
            .send(MockProviderServerControl::Start)
            .expect("start local provider server");
    }

    fn finish(self, stdout: &str, stderr: &str) -> Vec<String> {
        let shutdown_message = MockProviderServerControl::Shutdown;
        let _send_result = self.control_sender.send(shutdown_message);

        match self.join_handle.join() {
            Ok(requests) => requests,
            Err(payload) => {
                panic!(
                    "join scripted provider server failed, stdout={stdout:?}, stderr={stderr:?}, panic={payload:?}"
                );
            }
        }
    }
}

struct DynamicMockProviderServer {
    base_url: String,
    control_sender: mpsc::Sender<MockProviderServerControl>,
    join_handle: std::thread::JoinHandle<Vec<String>>,
}

impl DynamicMockProviderServer {
    fn spawn(
        expected_requests: usize,
        handler: impl FnMut(usize, &str) -> MockProviderResponse + Send + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider listener");
        let address = listener.local_addr().expect("local provider address");
        let (control_sender, control_receiver) = mpsc::channel();
        let join_handle = std::thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("set local provider listener nonblocking");
            let start_signal = control_receiver
                .recv()
                .expect("receive provider server start signal");
            match start_signal {
                MockProviderServerControl::Start => {}
                MockProviderServerControl::Shutdown => return Vec::new(),
            }

            let mut handler = handler;
            let mut requests = Vec::new();

            loop {
                let control_message = control_receiver.try_recv();
                match control_message {
                    Ok(MockProviderServerControl::Shutdown) => return requests,
                    Ok(MockProviderServerControl::Start) => {}
                    Err(TryRecvError::Disconnected) => return requests,
                    Err(TryRecvError::Empty) => {}
                }

                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let request = read_provider_request(&mut stream);
                        let request_index = requests.len();
                        let response = handler(request_index, request.as_str());
                        requests.push(request);
                        write_provider_response(&mut stream, &response);

                        if requests.len() >= expected_requests {
                            return requests;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::yield_now();
                    }
                    Err(error) => panic!("accept provider request: {error}"),
                }
            }
        });
        let base_url = format!("http://{address}");

        Self {
            base_url,
            control_sender,
            join_handle,
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn arm(&self) {
        self.control_sender
            .send(MockProviderServerControl::Start)
            .expect("start local provider server");
    }

    fn finish(self, stdout: &str, stderr: &str) -> Vec<String> {
        let shutdown_message = MockProviderServerControl::Shutdown;
        let _send_result = self.control_sender.send(shutdown_message);

        match self.join_handle.join() {
            Ok(requests) => requests,
            Err(payload) => {
                panic!(
                    "join dynamic provider server failed, stdout={stdout:?}, stderr={stderr:?}, panic={payload:?}"
                );
            }
        }
    }
}

fn write_provider_response(stream: &mut TcpStream, response: &MockProviderResponse) {
    let response_body = response.body.as_str();
    let status_line = response.status_line.as_str();
    let response_text = format!(
        "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    stream
        .write_all(response_text.as_bytes())
        .expect("write provider response");
}

struct BrowserFixtureServer {
    base_url: String,
    join_handle: std::thread::JoinHandle<Vec<String>>,
}

impl BrowserFixtureServer {
    fn spawn(expected_requests: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local browser listener");
        let address = listener.local_addr().expect("local browser address");
        let join_handle = std::thread::spawn(move || {
            let mut requests = Vec::new();

            while requests.len() < expected_requests {
                let (mut stream, _) = listener.accept().expect("accept browser request");
                let request = read_provider_request(&mut stream);
                let body = browser_fixture_body_for_request(request.as_str());
                let content_type = "text/html; charset=utf-8";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write browser fixture response");
                requests.push(request);
            }

            requests
        });
        let base_url = format!("http://{address}");

        Self {
            base_url,
            join_handle,
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn finish(self) -> Vec<String> {
        self.join_handle
            .join()
            .expect("join browser fixture server")
    }
}

struct WebSummaryFixtureServer {
    base_url: String,
    join_handle: std::thread::JoinHandle<Vec<String>>,
}

impl WebSummaryFixtureServer {
    fn spawn(expected_requests: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local web summary listener");
        let address = listener.local_addr().expect("local web summary address");
        let join_handle = std::thread::spawn(move || {
            let mut requests = Vec::new();

            while requests.len() < expected_requests {
                let (mut stream, _) = listener.accept().expect("accept web summary request");
                let request = read_provider_request(&mut stream);
                let body = r#"<!doctype html>
<html>
  <head>
    <title>Example Summary Fixture</title>
    <meta name="description" content="Short profile summary for the target page.">
  </head>
  <body>
    <header>Marketing Nav Pricing Docs</header>
    <main>
      <h1>Summary Fixture</h1>
      <p>Main profile content with links and details.</p>
    </main>
    <footer>Footer noise</footer>
  </body>
</html>"#;
                let content_type = "text/html; charset=utf-8";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write web summary fixture response");
                requests.push(request);
            }

            requests
        });
        let base_url = format!("http://{address}");

        Self {
            base_url,
            join_handle,
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn finish(self) -> Vec<String> {
        self.join_handle
            .join()
            .expect("join web summary fixture server")
    }
}

fn browser_fixture_body_for_request(request: &str) -> &'static str {
    if request.starts_with("GET /next ") {
        return r#"<!doctype html>
<html>
  <head><title>Browser Destination</title></head>
  <body>
    <main>
      <h1 id="destination">Clicked Destination</h1>
      <p>The browser click followed the discovered link.</p>
    </main>
  </body>
</html>"#;
    }

    r#"<!doctype html>
<html>
  <head><title>Browser Fixture Home</title></head>
  <body>
    <main>
      <h1 id="headline">Fixture Headline</h1>
      <a href="/next">Read More</a>
    </main>
  </body>
</html>"#
}

fn openai_chat_tool_call_body(content: &str, call_id: &str, tool_name: &str, args: Value) -> Value {
    let arguments = serde_json::to_string(&args).expect("tool arguments should encode");
    json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": content,
                "tool_calls": [{
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": arguments,
                    }
                }]
            }
        }]
    })
}

fn openai_chat_final_body(content: &str) -> Value {
    json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": content,
            }
        }]
    })
}

fn openai_responses_function_call_body(call_id: &str, tool_name: &str, args: Value) -> Value {
    let arguments = serde_json::to_string(&args).expect("tool arguments should encode");
    json!({
        "output": [{
            "type": "function_call",
            "name": tool_name,
            "arguments": arguments,
            "call_id": call_id,
        }]
    })
}

fn openai_responses_final_body(content: &str) -> Value {
    json!({
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": content,
            }],
        }],
    })
}

#[test]
fn ask_cli_latest_session_selector_process_uses_selected_root_session_history() {
    let fixture = LatestSelectorCliFixture::new("ask-latest-selector-process");
    let provider_server = MockProviderServer::spawn();
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
    });

    fixture.create_root_session("root-old");
    fixture.append_session_turn("root-old", "user", "old root turn");
    fixture.set_session_updated_at("root-old", 100);
    fixture.set_turn_timestamps("root-old", 100);

    fixture.create_root_session("root-new");
    fixture.append_session_turn("root-new", "user", "selected user turn");
    fixture.append_session_turn("root-new", "assistant", "selected assistant turn");
    fixture.set_session_updated_at("root-new", 200);
    fixture.set_turn_timestamps("root-new", 200);

    fixture.create_delegate_child_session("delegate-child", "root-new");
    fixture.append_session_turn("delegate-child", "assistant", "delegate child turn");
    fixture.set_session_updated_at("delegate-child", 400);
    fixture.set_turn_timestamps("delegate-child", 400);

    fixture.create_root_session("root-archived");
    fixture.append_session_turn("root-archived", "assistant", "archived root turn");
    fixture.set_session_updated_at("root-archived", 500);
    fixture.set_turn_timestamps("root-archived", 500);
    fixture.archive_session("root-archived", 600);

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--session",
            "latest",
            "--message",
            "Summarize the current session.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);

    assert!(
        output.status.success(),
        "ask latest selector should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(MOCK_PROVIDER_REPLY),
        "ask should print the mock provider reply: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        1,
        "ask should issue exactly one provider request: {provider_requests:#?}"
    );

    let request = &provider_requests[0];
    let request_path_is_supported = request.starts_with("POST /v1/chat/completions ")
        || request.starts_with("POST /v1/responses ");
    assert!(
        request_path_is_supported,
        "ask should target a supported provider endpoint: {request}"
    );
    assert!(
        request.contains("selected user turn"),
        "selected latest root user history should reach the provider request: {request}"
    );
    assert!(
        request.contains("selected assistant turn"),
        "selected latest root assistant history should reach the provider request: {request}"
    );
    assert!(
        !request.contains("old root turn"),
        "older root history should not leak into the selected latest request: {request}"
    );
    assert!(
        !request.contains("delegate child turn"),
        "delegate child history should not be selected as the latest resumable root: {request}"
    );
    assert!(
        !request.contains("archived root turn"),
        "archived root history should not be selected as the latest resumable root: {request}"
    );
}

#[test]
fn ask_cli_latest_session_selector_process_rejects_missing_resumable_root() {
    let fixture = LatestSelectorCliFixture::new("ask-latest-selector-empty");
    let provider_server = MockProviderServer::spawn();
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--session",
            "latest",
            "--message",
            "Summarize the current session.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);

    assert_eq!(
        output.status.code(),
        Some(2),
        "missing latest root session should fail before ask runs, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stderr.contains("latest"),
        "error output should mention the latest selector: {stderr:?}"
    );
    assert!(
        stderr.contains("resumable root session"),
        "error output should explain the missing latest root session: {stderr:?}"
    );
    assert!(
        provider_requests.is_empty(),
        "selector failure should abort before any provider request: {provider_requests:#?}"
    );
}

#[test]
fn ask_cli_latest_session_selector_process_wait_budget_starts_with_process_run() {
    let fixture = LatestSelectorCliFixture::new("ask-latest-selector-budget");
    let provider_server = MockProviderServer::spawn();
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
    });

    fixture.create_root_session("root-latest");
    fixture.append_session_turn("root-latest", "user", "latest root turn");
    fixture.set_session_updated_at("root-latest", 200);
    fixture.set_turn_timestamps("root-latest", 200);

    // The delay must exceed the old fixed server budget so this test proves the
    // wait window now starts with the spawned process run, not server creation.
    let setup_delay = Duration::from_secs(6);
    std::thread::sleep(setup_delay);

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--session",
            "latest",
            "--message",
            "Summarize the current session.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);

    assert!(
        output.status.success(),
        "ask should succeed even after slow fixture setup, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(MOCK_PROVIDER_REPLY),
        "ask should still print the mock provider reply after slow setup: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        1,
        "ask should still issue exactly one provider request after slow setup: {provider_requests:#?}"
    );
}

#[test]
fn turn_run_cli_latest_session_selector_process_uses_selected_root_session_history() {
    let fixture = LatestSelectorCliFixture::new("turn-run-latest-selector-process");
    let provider_server = MockProviderServer::spawn();
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
    });

    fixture.create_root_session("root-old");
    fixture.append_session_turn("root-old", "user", "old root turn");
    fixture.set_session_updated_at("root-old", 100);
    fixture.set_turn_timestamps("root-old", 100);

    fixture.create_root_session("root-new");
    fixture.append_session_turn("root-new", "user", "selected user turn");
    fixture.append_session_turn("root-new", "assistant", "selected assistant turn");
    fixture.set_session_updated_at("root-new", 200);
    fixture.set_turn_timestamps("root-new", 200);

    fixture.create_delegate_child_session("delegate-child", "root-new");
    fixture.append_session_turn("delegate-child", "assistant", "delegate child turn");
    fixture.set_session_updated_at("delegate-child", 400);
    fixture.set_turn_timestamps("delegate-child", 400);

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "turn",
            "run",
            "--session",
            "latest",
            "--message",
            "Summarize the current session.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);

    assert!(
        output.status.success(),
        "turn run latest selector should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(MOCK_PROVIDER_REPLY),
        "turn run should print the mock provider reply: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        1,
        "turn run should issue exactly one provider request: {provider_requests:#?}"
    );

    let request = &provider_requests[0];
    let request_path_is_supported = request.starts_with("POST /v1/chat/completions ")
        || request.starts_with("POST /v1/responses ");
    assert!(
        request_path_is_supported,
        "turn run should target a supported provider endpoint: {request}"
    );
    assert!(
        request.contains("selected user turn"),
        "selected latest root user history should reach the turn run provider request: {request}"
    );
    assert!(
        request.contains("selected assistant turn"),
        "selected latest root assistant history should reach the turn run provider request: {request}"
    );
    assert!(
        !request.contains("old root turn"),
        "older root history should not leak into the selected latest request: {request}"
    );
    assert!(
        !request.contains("delegate child turn"),
        "delegate child history should not be selected as the latest resumable root: {request}"
    );
}

#[test]
fn ask_cli_openai_preface_plus_tool_search_then_exec_runs_full_e2e() {
    let fixture = LatestSelectorCliFixture::new("ask-tool-search-exec-e2e");
    let final_reply = "E2E PASS direct bash execution.";
    let provider_responses = vec![
        MockProviderResponse::ok_json(openai_chat_tool_call_body(
            "I'll run the guarded workspace command.",
            "call-run-bash-tool",
            "bash",
            json!({
                "command": "printf LOONG_E2E_EXEC_OK",
            }),
        )),
        MockProviderResponse::ok_json(openai_chat_final_body(final_reply)),
    ];
    let provider_server = ScriptedMockProviderServer::spawn(provider_responses);
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.shell_default_mode = "allow".to_owned();
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--message",
            "Search for the command tool, run printf, and report when done.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);

    assert!(
        output.status.success(),
        "ask bash e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain only the terminal assistant answer: {stdout:?}"
    );
    assert!(
        !stdout.contains("I'll run the guarded workspace command."),
        "stdout should not finalize the first assistant preface: {stdout:?}"
    );
    assert!(
        !stdout.contains("[tool_loop_warning]"),
        "stdout should not surface a loop warning on the happy path: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        2,
        "ask should continue through bash execution and final reply: {provider_requests:#?}"
    );

    let initial_request = &provider_requests[0];
    assert_provider_request_contains_text(
        initial_request,
        "run printf",
        "initial provider request",
    );

    let exec_followup_request = &provider_requests[1];
    assert_provider_request_contains_text(
        exec_followup_request,
        "[tool_result]",
        "bash follow-up provider request",
    );
    assert_provider_request_contains_text(
        exec_followup_request,
        "LOONG_E2E_EXEC_OK",
        "bash follow-up provider request",
    );
}

#[test]
fn ask_cli_browser_open_extract_click_runs_full_e2e() {
    let fixture = LatestSelectorCliFixture::new("ask-browser-chain-e2e");
    let browser_server = BrowserFixtureServer::spawn(2);
    let browser_base_url = browser_server.base_url().to_owned();
    let final_reply = "E2E PASS browser open extract click.";
    let provider_server =
        DynamicMockProviderServer::spawn(4, move |request_index, request| match request_index {
            0 => {
                assert_provider_request_contains_text(
                    request,
                    "Open the browser fixture",
                    "initial browser provider request",
                );
                MockProviderResponse::ok_json(openai_chat_tool_call_body(
                    "I will open the fixture page first.",
                    "call-browser-open",
                    "browser",
                    json!({
                        "url": browser_base_url,
                    }),
                ))
            }
            1 => {
                assert_provider_request_contains_text(
                    request,
                    "Browser Fixture Home",
                    "browser open follow-up request",
                );
                assert_provider_request_contains_text(
                    request,
                    "Read More",
                    "browser open follow-up request",
                );
                let session_id = extract_browser_session_id_from_request(request);
                MockProviderResponse::ok_json(openai_chat_tool_call_body(
                    "I will extract the headline next.",
                    "call-browser-extract",
                    "browser",
                    json!({
                        "session_id": session_id,
                        "mode": "selector_text",
                        "selector": "#headline",
                    }),
                ))
            }
            2 => {
                assert_provider_request_contains_text(
                    request,
                    "Fixture Headline",
                    "browser extract follow-up request",
                );
                let session_id = extract_browser_session_id_from_request(request);
                MockProviderResponse::ok_json(openai_chat_tool_call_body(
                    "I will follow the discovered link.",
                    "call-browser-click",
                    "browser",
                    json!({
                        "session_id": session_id,
                        "link_id": 1,
                    }),
                ))
            }
            3 => {
                assert_provider_request_contains_text(
                    request,
                    "Clicked Destination",
                    "browser click follow-up request",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(final_reply))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.browser.enabled = true;
        config.tools.browser.max_sessions = 4;
        config.tools.web.allow_private_hosts = true;
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--message",
            "Open the browser fixture, extract the headline, follow the link, then report done.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);
    let browser_requests = browser_server.finish();

    assert!(
        output.status.success(),
        "ask browser e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the terminal browser answer: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        4,
        "ask should continue through browser open, extract, and click: {provider_requests:#?}"
    );
    assert_eq!(
        browser_requests.len(),
        2,
        "browser fixture should receive open and click requests: {browser_requests:#?}"
    );
    assert!(
        browser_requests[0].starts_with("GET / "),
        "first browser request should open the fixture home: {browser_requests:#?}"
    );
    assert!(
        browser_requests[1].starts_with("GET /next "),
        "second browser request should follow the discovered link: {browser_requests:#?}"
    );
}

#[test]
fn ask_cli_browser_continues_after_shell_heavy_page_without_confirmation() {
    let fixture = LatestSelectorCliFixture::new("ask-browser-shell-heavy-followup-e2e");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind shell-heavy browser listener");
    let address = listener
        .local_addr()
        .expect("shell-heavy browser local address");
    let browser_join = std::thread::spawn(move || {
        let mut requests = Vec::new();
        while requests.len() < 2 {
            let (mut stream, _) = listener
                .accept()
                .expect("accept shell-heavy browser request");
            let request = read_provider_request(&mut stream);
            let body = if requests.is_empty() {
                r#"<!doctype html>
<html>
  <head>
    <title>Browser Shell Heavy Fixture</title>
    <meta name="description" content="Short profile summary for the target page.">
  </head>
  <body>
    <header>Marketing Nav Pricing Docs</header>
    <nav>Overview Repositories Projects Stars</nav>
    <footer>Footer policies and links</footer>
  </body>
</html>"#
            } else {
                r#"<!doctype html>
<html>
  <head>
    <title>Browser Rich Fixture</title>
    <meta name="description" content="Short profile summary for the target page.">
  </head>
  <body>
    <header>Marketing Nav Pricing Docs</header>
    <main>
      <h1 id="headline">Rich Browser Fixture</h1>
      <p>Main profile content with links and details.</p>
    </main>
    <footer>Footer noise</footer>
  </body>
</html>"#
            };
            let content_type = "text/html; charset=utf-8";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write shell-heavy browser fixture response");
            requests.push(request);
        }
        requests
    });
    let browser_base_url = format!("http://{address}");
    let permission_reply = "I still need a narrower browser extract because the page looks like shell-heavy navigation.";
    let final_reply = "E2E PASS shell-heavy browser continuation.";
    let provider_server = DynamicMockProviderServer::spawn(4, move |request_index, request| {
        match request_index {
            0 => {
                assert_provider_request_contains_text(
                    request,
                    "Open the browser fixture",
                    "initial shell-heavy browser provider request",
                );
                MockProviderResponse::ok_json(openai_chat_tool_call_body(
                    "I will open the fixture page first.",
                    "call-browser-open-shell",
                    "browser",
                    json!({
                        "url": browser_base_url,
                    }),
                ))
            }
            1 => {
                assert_provider_request_contains_text(
                    request,
                    "insufficient_page_evidence",
                    "shell-heavy browser follow-up request should include continuation state",
                );
                assert_provider_request_contains_text(
                    request,
                    "continue with `web`",
                    "shell-heavy browser follow-up request should recommend a fuller web fetch",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(permission_reply))
            }
            2 => {
                assert_provider_request_contains_text(
                    request,
                    permission_reply,
                    "runtime should preserve the shell-heavy browser permission reply inside the forced follow-up context",
                );
                MockProviderResponse::ok_json(openai_chat_tool_call_body(
                    "",
                    "call-web-refetch-from-browser-shell",
                    "web",
                    json!({
                        "url": browser_base_url,
                    }),
                ))
            }
            3 => {
                assert_provider_request_contains_text(
                    request,
                    "Main profile content with links and details.",
                    "follow-up after shell-heavy browser permission reply should include actual page content",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(final_reply))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        }
    });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.browser.enabled = true;
        config.tools.browser.max_sessions = 4;
        config.tools.web.allow_private_hosts = true;
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--message",
            "Open the browser fixture and summarize what the page says without asking for confirmation.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);
    let browser_requests = browser_join
        .join()
        .expect("join shell-heavy browser fixture");

    assert!(
        output.status.success(),
        "ask shell-heavy browser continuation e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the terminal browser answer after continuing: {stdout:?}"
    );
    assert!(
        !stdout.contains(permission_reply),
        "stdout should not stop at the shell-heavy browser permission reply: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        4,
        "ask should continue after the shell-heavy browser permission reply: {provider_requests:#?}"
    );
    assert_eq!(
        browser_requests.len(),
        2,
        "shell-heavy browser fixture should receive the initial open and follow-up extract requests: {browser_requests:#?}"
    );
}

#[test]
fn ask_cli_web_summary_uses_page_metadata_and_hides_tool_markup() {
    let fixture = LatestSelectorCliFixture::new("ask-web-summary-e2e");
    let web_server = WebSummaryFixtureServer::spawn(1);
    let web_base_url = web_server.base_url().to_owned();
    let final_reply = "E2E PASS web summary.";
    let provider_server =
        DynamicMockProviderServer::spawn(2, move |request_index, request| match request_index {
            0 => {
                assert_provider_request_contains_text(
                    request,
                    "Summarize the fixture page",
                    "initial web summary provider request",
                );
                MockProviderResponse::ok_json(openai_chat_tool_call_body(
                    "I will fetch the page first.",
                    "call-web-fetch",
                    "web",
                    json!({
                        "url": web_base_url,
                    }),
                ))
            }
            1 => {
                assert_provider_request_contains_text(
                    request,
                    "Summary: Short profile summary for the target page.",
                    "web follow-up provider request",
                );
                assert_provider_request_contains_text(
                    request,
                    "Main profile content with links and details.",
                    "web follow-up provider request",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(final_reply))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.web.allow_private_hosts = true;
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--message",
            "Summarize the fixture page without asking for confirmation.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);
    let web_requests = web_server.finish();

    assert!(
        output.status.success(),
        "ask web summary e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the terminal web summary answer: {stdout:?}"
    );
    assert!(
        !stdout.contains("[tool_request]"),
        "stdout should not leak tool request markup: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        2,
        "ask should continue through one web fetch and final reply: {provider_requests:#?}"
    );
    assert_eq!(
        web_requests.len(),
        1,
        "web summary fixture should receive a single fetch request: {web_requests:#?}"
    );
    assert!(
        web_requests[0].starts_with("GET / "),
        "web summary fixture should receive a fetch for the root page: {web_requests:#?}"
    );
}

#[test]
fn ask_cli_repairs_pseudo_done_browser_reply_before_finishing() {
    let fixture = LatestSelectorCliFixture::new("ask-browser-pseudo-done-followup-e2e");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind pseudo-done browser listener");
    let address = listener
        .local_addr()
        .expect("pseudo-done browser local address");
    let browser_join = std::thread::spawn(move || {
        let mut requests = Vec::new();
        while requests.len() < 2 {
            let (mut stream, _) = listener
                .accept()
                .expect("accept pseudo-done browser request");
            let request = read_provider_request(&mut stream);
            let body = if requests.is_empty() {
                r#"<!doctype html>
<html>
  <head>
    <title>Browser Shell Heavy Fixture</title>
    <meta name="description" content="Short profile summary for the target page.">
  </head>
  <body>
    <header>Marketing Nav Pricing Docs</header>
    <nav>Overview Repositories Projects Stars</nav>
    <footer>Footer policies and links</footer>
  </body>
</html>"#
            } else {
                r#"<!doctype html>
<html>
  <head>
    <title>Browser Rich Fixture</title>
    <meta name="description" content="Short profile summary for the target page.">
  </head>
  <body>
    <header>Marketing Nav Pricing Docs</header>
    <main>
      <h1 id="headline">Browser Rich Fixture</h1>
      <p>Main profile content with links and details.</p>
    </main>
    <footer>Footer noise</footer>
  </body>
</html>"#
            };
            let content_type = "text/html; charset=utf-8";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write pseudo-done browser fixture response");
            requests.push(request);
        }
        requests
    });
    let browser_base_url = format!("http://{address}");
    let pseudo_done_reply = "[followup_state:done]\nI still need a narrower browser extract because the page looks like shell-heavy navigation.";
    let final_reply = "E2E PASS pseudo-done browser continuation.";
    let provider_server = DynamicMockProviderServer::spawn(4, move |request_index, request| {
        match request_index {
            0 => {
                assert_provider_request_contains_text(
                    request,
                    "Open the browser fixture",
                    "initial pseudo-done browser provider request",
                );
                MockProviderResponse::ok_json(openai_chat_tool_call_body(
                    "I will open the fixture page first.",
                    "call-browser-open-pseudo-done",
                    "browser",
                    json!({
                        "url": browser_base_url,
                    }),
                ))
            }
            1 => {
                assert_provider_request_contains_text(
                    request,
                    "insufficient_page_evidence",
                    "pseudo-done browser follow-up request should include continuation state",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(pseudo_done_reply))
            }
            2 => {
                assert_provider_request_contains_text(
                    request,
                    "I still need a narrower browser extract because the page looks like shell-heavy navigation.",
                    "runtime should preserve the pseudo-done browser reply text inside the repair follow-up context",
                );
                assert_provider_request_contains_text(
                    request,
                    "continue with `web`",
                    "repair follow-up should retain the structured browser continuation guidance",
                );
                MockProviderResponse::ok_json(openai_chat_tool_call_body(
                    "",
                    "call-web-after-browser-pseudo-done",
                    "web",
                    json!({
                        "url": browser_base_url,
                    }),
                ))
            }
            3 => {
                assert_provider_request_contains_text(
                    request,
                    "Main profile content with links and details.",
                    "follow-up after browser pseudo-done repair should include actual page content",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(&format!(
                    "[followup_state:done]\n{final_reply}"
                )))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        }
    });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.browser.enabled = true;
        config.tools.browser.max_sessions = 4;
        config.tools.web.allow_private_hosts = true;
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--message",
            "Open the browser fixture and summarize what the page says without asking for confirmation.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);
    let browser_requests = browser_join
        .join()
        .expect("join pseudo-done browser fixture");

    assert!(
        output.status.success(),
        "ask pseudo-done browser continuation e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the final answer after repairing the pseudo-done browser reply: {stdout:?}"
    );
    assert!(
        !stdout.contains(pseudo_done_reply),
        "stdout should not stop at the pseudo-done browser reply: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        4,
        "ask should request one more provider round after the pseudo-done browser reply: {provider_requests:#?}"
    );
    assert_eq!(
        browser_requests.len(),
        2,
        "pseudo-done browser fixture should receive the initial open and the web follow-up fetch: {browser_requests:#?}"
    );
}

#[test]
fn ask_cli_web_summary_continues_after_shell_heavy_page_without_confirmation() {
    let fixture = LatestSelectorCliFixture::new("ask-web-summary-shell-heavy-followup-e2e");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind shell-heavy web summary listener");
    let address = listener
        .local_addr()
        .expect("shell-heavy web summary local address");
    let web_join = std::thread::spawn(move || {
        let mut requests = Vec::new();
        while requests.len() < 2 {
            let (mut stream, _) = listener.accept().expect("accept shell-heavy web request");
            let request = read_provider_request(&mut stream);
            let body = if requests.is_empty() {
                r#"<!doctype html>
<html>
  <head>
    <title>Shell Heavy Fixture</title>
    <meta name="description" content="Short profile summary for the target page.">
  </head>
  <body>
    <header>Marketing Nav Pricing Docs</header>
    <nav>Overview Repositories Projects Stars</nav>
    <footer>Footer policies and links</footer>
  </body>
</html>"#
            } else {
                r#"<!doctype html>
<html>
  <head>
    <title>Rich Fixture</title>
    <meta name="description" content="Short profile summary for the target page.">
  </head>
  <body>
    <header>Marketing Nav Pricing Docs</header>
    <main>
      <h1>Rich Fixture</h1>
      <p>Main profile content with links and details.</p>
    </main>
    <footer>Footer noise</footer>
  </body>
</html>"#
            };
            let content_type = "text/html; charset=utf-8";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write shell-heavy web summary fixture response");
            requests.push(request);
        }
        requests
    });
    let web_base_url = format!("http://{address}");
    let permission_reply =
        "I still need a narrower fetch because the page looks like shell-heavy navigation.";
    let final_reply = "E2E PASS shell-heavy web continuation.";
    let provider_server = DynamicMockProviderServer::spawn(4, move |request_index, request| {
        match request_index {
            0 => {
                assert_provider_request_contains_text(
                    request,
                    "Summarize the fixture page",
                    "initial shell-heavy web summary provider request",
                );
                MockProviderResponse::ok_json(openai_chat_tool_call_body(
                    "I will fetch the page first.",
                    "call-web-fetch-shell",
                    "web",
                    json!({
                        "url": web_base_url,
                    }),
                ))
            }
            1 => {
                assert_provider_request_contains_text(
                    request,
                    "insufficient_page_evidence",
                    "shell-heavy web follow-up provider request should include continuation state",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(permission_reply))
            }
            2 => {
                assert_provider_request_contains_text(
                    request,
                    permission_reply,
                    "runtime should preserve the shell-heavy permission reply inside the forced follow-up context",
                );
                MockProviderResponse::ok_json(openai_chat_tool_call_body(
                    "",
                    "call-web-refetch-shell",
                    "web",
                    json!({
                        "url": web_base_url,
                    }),
                ))
            }
            3 => {
                assert_provider_request_contains_text(
                    request,
                    "Main profile content with links and details.",
                    "follow-up after shell-heavy permission reply should include actual page content",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(final_reply))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        }
    });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.web.allow_private_hosts = true;
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--message",
            "Summarize the fixture page without asking for confirmation.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);
    let web_requests = web_join
        .join()
        .expect("join shell-heavy web summary fixture");

    assert!(
        output.status.success(),
        "ask shell-heavy web continuation e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the terminal web summary answer after continuing: {stdout:?}"
    );
    assert!(
        !stdout.contains(permission_reply),
        "stdout should not stop at the shell-heavy permission reply: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        4,
        "ask should continue after the shell-heavy web permission reply: {provider_requests:#?}"
    );
    assert_eq!(
        web_requests.len(),
        2,
        "shell-heavy web fixture should receive the initial and follow-up fetches: {web_requests:#?}"
    );
}

#[test]
fn ask_cli_repairs_pseudo_done_web_reply_before_finishing() {
    let fixture = LatestSelectorCliFixture::new("ask-web-summary-pseudo-done-followup-e2e");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind pseudo-done web summary listener");
    let address = listener
        .local_addr()
        .expect("pseudo-done web summary local address");
    let web_join = std::thread::spawn(move || {
        let mut requests = Vec::new();
        while requests.len() < 2 {
            let (mut stream, _) = listener.accept().expect("accept pseudo-done web request");
            let request = read_provider_request(&mut stream);
            let body = if requests.is_empty() {
                r#"<!doctype html>
<html>
  <head>
    <title>Shell Heavy Fixture</title>
    <meta name="description" content="Short profile summary for the target page.">
  </head>
  <body>
    <header>Marketing Nav Pricing Docs</header>
    <nav>Overview Repositories Projects Stars</nav>
    <footer>Footer policies and links</footer>
  </body>
</html>"#
            } else {
                r#"<!doctype html>
<html>
  <head>
    <title>Rich Fixture</title>
    <meta name="description" content="Short profile summary for the target page.">
  </head>
  <body>
    <header>Marketing Nav Pricing Docs</header>
    <main>
      <h1>Rich Fixture</h1>
      <p>Main profile content with links and details.</p>
    </main>
    <footer>Footer noise</footer>
  </body>
</html>"#
            };
            let content_type = "text/html; charset=utf-8";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write pseudo-done web summary fixture response");
            requests.push(request);
        }
        requests
    });
    let web_base_url = format!("http://{address}");
    let pseudo_done_reply = "[followup_state:done]\nI still need a narrower fetch because the page looks like shell-heavy navigation. Please allow another fetch before I summarize it.";
    let final_reply = "E2E PASS pseudo-done web continuation.";
    let provider_server = DynamicMockProviderServer::spawn(4, move |request_index, request| {
        match request_index {
            0 => {
                assert_provider_request_contains_text(
                    request,
                    "Summarize the fixture page",
                    "initial pseudo-done web summary provider request",
                );
                MockProviderResponse::ok_json(openai_chat_tool_call_body(
                    "I will fetch the page first.",
                    "call-web-fetch-pseudo-done",
                    "web",
                    json!({
                        "url": web_base_url,
                    }),
                ))
            }
            1 => {
                assert_provider_request_contains_text(
                    request,
                    "insufficient_page_evidence",
                    "pseudo-done web follow-up provider request should include continuation state",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(pseudo_done_reply))
            }
            2 => {
                assert_provider_request_contains_text(
                    request,
                    "I still need a narrower fetch because the page looks like shell-heavy navigation.",
                    "runtime should preserve the pseudo-done web reply text inside the repair follow-up context",
                );
                assert_provider_request_contains_text(
                    request,
                    "continue with `web`",
                    "repair follow-up should retain the structured web continuation guidance",
                );
                MockProviderResponse::ok_json(openai_chat_tool_call_body(
                    "",
                    "call-web-refetch-after-pseudo-done",
                    "web",
                    json!({
                        "url": web_base_url,
                    }),
                ))
            }
            3 => {
                assert_provider_request_contains_text(
                    request,
                    "Main profile content with links and details.",
                    "follow-up after web pseudo-done repair should include actual page content",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(&format!(
                    "[followup_state:done]\n{final_reply}"
                )))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        }
    });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.web.allow_private_hosts = true;
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--message",
            "Summarize the fixture page without asking for confirmation.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);
    let web_requests = web_join
        .join()
        .expect("join pseudo-done web summary fixture");

    assert!(
        output.status.success(),
        "ask pseudo-done web continuation e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the final answer after repairing the pseudo-done web reply: {stdout:?}"
    );
    assert!(
        !stdout.contains(pseudo_done_reply),
        "stdout should not stop at the pseudo-done permission reply: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        4,
        "ask should request one more provider round after the pseudo-done web reply: {provider_requests:#?}"
    );
    assert_eq!(
        web_requests.len(),
        2,
        "pseudo-done web fixture should receive the initial and follow-up fetches: {web_requests:#?}"
    );
}

#[test]
fn ask_cli_recovers_textual_tool_request_wrappers_and_completes() {
    let fixture = LatestSelectorCliFixture::new("ask-text-tool-request-e2e");
    std::fs::create_dir_all(fixture.root_path().join("docs")).expect("create docs dir");
    std::fs::write(
        fixture.root_path().join("AGENTS.md"),
        "Repository guidance for E2E textual tool request recovery.",
    )
    .expect("write AGENTS fixture");
    std::fs::write(
        fixture.root_path().join("docs/README.md"),
        "Documentation overview for E2E textual tool request recovery.",
    )
    .expect("write docs README fixture");

    let final_reply = "E2E PASS textual tool request recovery.";
    let provider_server = DynamicMockProviderServer::spawn(2, move |request_index, request| {
        match request_index {
            0 => MockProviderResponse::ok_json(openai_chat_final_body(
                "[tool_request]\n{\"arguments\":{\"path\":\"AGENTS.md\"},\"name\":\"read\"}[tool_request]\n{\"arguments\":{\"path\":\"docs/README.md\"},\"name\":\"read\"}I need the contents of `AGENTS.md` and `docs/README.md` to ground the summary, but I do not have their tool outputs yet.",
            )),
            1 => {
                assert_provider_request_contains_text(
                    request,
                    "Repository guidance for E2E textual tool request recovery.",
                    "textual tool request follow-up provider request",
                );
                assert_provider_request_contains_text(
                    request,
                    "Documentation overview for E2E textual tool request recovery.",
                    "textual tool request follow-up provider request",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(&format!(
                    "[followup_state:done]\n{final_reply}"
                )))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        }
    });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.file_root = Some(fixture.root_path().display().to_string());
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--message",
            "Summarize this repository and suggest the best next step.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);

    assert!(
        output.status.success(),
        "ask textual tool request e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the terminal answer after recovering textual tool requests: {stdout:?}"
    );
    assert!(
        !stdout.contains("[tool_request]"),
        "stdout should not leak textual tool request markup: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        2,
        "ask should continue after recovering textual tool request wrappers: {provider_requests:#?}"
    );
}

#[test]
fn ask_cli_recovers_textual_tool_request_wrappers_without_trailing_text_and_completes() {
    let fixture = LatestSelectorCliFixture::new("ask-text-tool-request-no-tail-e2e");
    std::fs::create_dir_all(fixture.root_path().join("docs")).expect("create docs dir");
    std::fs::write(
        fixture.root_path().join("README.md"),
        "Repository overview for no-tail textual tool request recovery.",
    )
    .expect("write README fixture");
    std::fs::write(
        fixture.root_path().join("ARCHITECTURE.md"),
        "Architecture notes for no-tail textual tool request recovery.",
    )
    .expect("write ARCHITECTURE fixture");
    std::fs::write(
        fixture.root_path().join("docs/ROADMAP.md"),
        "Roadmap notes for no-tail textual tool request recovery.",
    )
    .expect("write ROADMAP fixture");

    let final_reply = "E2E PASS no-tail textual tool request recovery.";
    let provider_server = DynamicMockProviderServer::spawn(2, move |request_index, request| {
        match request_index {
            0 => MockProviderResponse::ok_json(openai_chat_final_body(
                "[tool_request]\n{\"arguments\":{\"path\":\"README.md\"},\"name\":\"read\"}[tool_request]\n{\"arguments\":{\"path\":\"ARCHITECTURE.md\"},\"name\":\"read\"}[tool_request]\n{\"arguments\":{\"path\":\"docs/ROADMAP.md\"},\"name\":\"read\"}",
            )),
            1 => {
                assert_provider_request_contains_text(
                    request,
                    "Repository overview for no-tail textual tool request recovery.",
                    "no-tail textual tool request follow-up provider request",
                );
                assert_provider_request_contains_text(
                    request,
                    "Architecture notes for no-tail textual tool request recovery.",
                    "no-tail textual tool request follow-up provider request",
                );
                assert_provider_request_contains_text(
                    request,
                    "Roadmap notes for no-tail textual tool request recovery.",
                    "no-tail textual tool request follow-up provider request",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(final_reply))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        }
    });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.file_root = Some(fixture.root_path().display().to_string());
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--message",
            "Summarize this repository and suggest the best next step.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);

    assert!(
        output.status.success(),
        "ask no-tail textual tool request e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the terminal answer after recovering no-tail textual tool requests: {stdout:?}"
    );
    assert!(
        !stdout.contains("[tool_request]"),
        "stdout should not leak no-tail textual tool request markup: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        2,
        "ask should continue after recovering no-tail textual tool request wrappers: {provider_requests:#?}"
    );
}

#[test]
fn ask_cli_continues_after_glob_path_listing_instead_of_returning_permission_request() {
    let fixture = LatestSelectorCliFixture::new("ask-glob-listing-followup-e2e");
    std::fs::create_dir_all(fixture.root_path().join("docs")).expect("create docs dir");
    std::fs::write(
        fixture.root_path().join("README.md"),
        "Repository overview for glob listing continuation.",
    )
    .expect("write README fixture");
    std::fs::write(
        fixture.root_path().join("ARCHITECTURE.md"),
        "Architecture notes for glob listing continuation.",
    )
    .expect("write ARCHITECTURE fixture");
    std::fs::write(
        fixture.root_path().join("docs/ROADMAP.md"),
        "Roadmap notes for glob listing continuation.",
    )
    .expect("write ROADMAP fixture");

    let permission_reply = "I need one more inspection step to ground the summary in the actual docs. Please allow me to read the top-level docs.";
    let final_reply = "E2E PASS glob listing continuation.";
    let provider_server = DynamicMockProviderServer::spawn(4, move |request_index, request| {
        match request_index {
            0 => MockProviderResponse::ok_json(openai_chat_tool_call_body(
                "",
                "call-glob",
                "read",
                json!({
                    "glob": "README.md|ARCHITECTURE.md|docs/ROADMAP.md",
                    "root": ".",
                    "max_results": 20
                }),
            )),
            1 => {
                assert_provider_request_contains_text(
                    request,
                    "path_listing",
                    "glob listing follow-up provider request should include continuation state",
                );
                assert_provider_request_contains_text(
                    request,
                    "ARCHITECTURE.md",
                    "glob listing follow-up provider request should include the recommended follow-up read path",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(permission_reply))
            }
            2 => {
                assert_provider_request_contains_text(
                    request,
                    permission_reply,
                    "runtime should preserve the provider permission reply inside the forced follow-up context",
                );
                MockProviderResponse::ok_json(openai_chat_tool_call_body(
                    "",
                    "call-readme",
                    "read",
                    json!({
                        "path": "README.md"
                    }),
                ))
            }
            3 => {
                assert_provider_request_contains_text(
                    request,
                    "Repository overview for glob listing continuation.",
                    "follow-up after permission reply should include actual file content",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(final_reply))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        }
    });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.file_root = Some(fixture.root_path().display().to_string());
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--session",
            "fresh-glob-followup-session",
            "--message",
            "Summarize this repository and suggest the best next step.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);

    assert!(
        output.status.success(),
        "ask glob listing continuation e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the terminal answer after continuing past the permission reply: {stdout:?}"
    );
    assert!(
        !stdout.contains(permission_reply),
        "stdout should not stop at the permission reply: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        4,
        "ask should force a follow-up after the path-listing permission reply: {provider_requests:#?}"
    );
}

#[test]
fn ask_cli_repairs_pseudo_done_glob_path_listing_reply_before_finishing() {
    let fixture = LatestSelectorCliFixture::new("ask-glob-listing-pseudo-done-followup-e2e");
    std::fs::create_dir_all(fixture.root_path().join("docs")).expect("create docs dir");
    std::fs::write(
        fixture.root_path().join("README.md"),
        "Repository overview for pseudo-done glob continuation.",
    )
    .expect("write README fixture");
    std::fs::write(
        fixture.root_path().join("ARCHITECTURE.md"),
        "Architecture notes for pseudo-done glob continuation.",
    )
    .expect("write ARCHITECTURE fixture");
    std::fs::write(
        fixture.root_path().join("docs/ROADMAP.md"),
        "Roadmap notes for pseudo-done glob continuation.",
    )
    .expect("write ROADMAP fixture");

    let pseudo_done_reply = "[followup_state:done]\nI need one more inspection step to ground the summary in the actual docs. Please allow me to read the top-level docs.";
    let final_reply = "E2E PASS pseudo-done glob listing continuation.";
    let provider_server = DynamicMockProviderServer::spawn(4, move |request_index, request| {
        match request_index {
            0 => MockProviderResponse::ok_json(openai_chat_tool_call_body(
                "",
                "call-glob-pseudo-done",
                "read",
                json!({
                    "glob": "README.md|ARCHITECTURE.md|docs/ROADMAP.md",
                    "root": ".",
                    "max_results": 20
                }),
            )),
            1 => {
                assert_provider_request_contains_text(
                    request,
                    "path_listing",
                    "pseudo-done glob listing follow-up should include continuation state",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(pseudo_done_reply))
            }
            2 => {
                assert_provider_request_contains_text(
                    request,
                    "I need one more inspection step to ground the summary in the actual docs.",
                    "runtime should preserve the pseudo-done reply text inside the repair follow-up context",
                );
                assert_provider_request_contains_text(
                    request,
                    "Please allow me to read the top-level docs.",
                    "repair follow-up should preserve the permission-style evidence gap text",
                );
                assert_provider_request_contains_text(
                    request,
                    "Continuation guidance:",
                    "repair follow-up should retain path-listing continuation guidance",
                );
                MockProviderResponse::ok_json(openai_chat_tool_call_body(
                    "",
                    "call-readme-after-pseudo-done",
                    "read",
                    json!({
                        "path": "README.md"
                    }),
                ))
            }
            3 => {
                assert_provider_request_contains_text(
                    request,
                    "Repository overview for pseudo-done glob continuation.",
                    "follow-up after pseudo-done repair should include actual file content",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(&format!(
                    "[followup_state:done]\n{final_reply}"
                )))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        }
    });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.file_root = Some(fixture.root_path().display().to_string());
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--session",
            "fresh-glob-pseudo-done-followup-session",
            "--message",
            "Summarize this repository and suggest the best next step.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);

    assert!(
        output.status.success(),
        "ask pseudo-done glob listing continuation e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the final answer after repairing the pseudo-done reply: {stdout:?}"
    );
    assert!(
        !stdout.contains(pseudo_done_reply),
        "stdout should not stop at the pseudo-done reply: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        4,
        "ask should request one more provider round after the pseudo-done glob reply: {provider_requests:#?}"
    );
}

#[test]
fn ask_cli_recovers_same_line_tool_request_wrapper_after_leading_preface() {
    let fixture = LatestSelectorCliFixture::new("ask-same-line-tool-request-preface-e2e");
    std::fs::create_dir_all(fixture.root_path().join("docs")).expect("create docs dir");
    std::fs::write(
        fixture.root_path().join("docs/README.md"),
        "Documentation overview for same-line tool request preface recovery.",
    )
    .expect("write docs README fixture");

    let final_reply = "E2E PASS same-line textual tool request recovery.";
    let provider_server = DynamicMockProviderServer::spawn(2, move |request_index, request| {
        match request_index {
            0 => MockProviderResponse::ok_json(openai_chat_final_body(
                "to summarize repo need inspect key docs.[tool_request]\n{\"arguments\":{\"path\":\"docs/README.md\"},\"name\":\"read\"}",
            )),
            1 => {
                assert_provider_request_contains_text(
                    request,
                    "Documentation overview for same-line tool request preface recovery.",
                    "same-line textual tool request follow-up provider request",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(&format!(
                    "[followup_state:done]\n{final_reply}"
                )))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        }
    });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.file_root = Some(fixture.root_path().display().to_string());
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--session",
            "same-line-tool-request-preface-session",
            "--message",
            "Summarize this repository and suggest the best next step.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);

    assert!(
        output.status.success(),
        "ask same-line textual tool request e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the final answer after recovering the same-line textual tool request: {stdout:?}"
    );
    assert!(
        !stdout.contains("[tool_request]"),
        "stdout should not leak same-line textual tool request markup: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        2,
        "ask should continue after recovering same-line textual tool request wrapper: {provider_requests:#?}"
    );
}

#[test]
fn ask_cli_continues_after_same_line_textual_tool_preface_permission_reply() {
    let fixture = LatestSelectorCliFixture::new("ask-same-line-preface-followup-e2e");
    std::fs::create_dir_all(fixture.root_path().join("docs")).expect("create docs dir");
    std::fs::write(
        fixture.root_path().join("docs/README.md"),
        "Documentation overview for same-line preface follow-up recovery.",
    )
    .expect("write docs README fixture");
    std::fs::write(
        fixture.root_path().join("ARCHITECTURE.md"),
        "Architecture overview for same-line preface follow-up recovery.",
    )
    .expect("write ARCHITECTURE fixture");

    let permission_reply = "I do not yet have usable repository evidence. Please allow another read of `ARCHITECTURE.md` so I can finish the summary.";
    let final_reply = "E2E PASS same-line textual tool preface continuation.";
    let provider_server = DynamicMockProviderServer::spawn(4, move |request_index, request| {
        match request_index {
            0 => MockProviderResponse::ok_json(openai_chat_final_body(
                "to summarize repo need inspect key docs.[tool_request]\n{\"arguments\":{\"path\":\"docs/README.md\"},\"name\":\"read\"}",
            )),
            1 => {
                assert_provider_request_contains_text(
                    request,
                    "Documentation overview for same-line preface follow-up recovery.",
                    "same-line preface follow-up provider request should include first file content",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(permission_reply))
            }
            2 => {
                assert_provider_request_contains_text(
                    request,
                    permission_reply,
                    "runtime should continue after the permission reply instead of returning it",
                );
                MockProviderResponse::ok_json(openai_chat_tool_call_body(
                    "",
                    "call-architecture",
                    "read",
                    json!({
                        "path": "ARCHITECTURE.md"
                    }),
                ))
            }
            3 => {
                assert_provider_request_contains_text(
                    request,
                    "Architecture overview for same-line preface follow-up recovery.",
                    "same-line preface continuation should include second file content",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(final_reply))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        }
    });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.file_root = Some(fixture.root_path().display().to_string());
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--session",
            "same-line-preface-followup-session",
            "--message",
            "Summarize this repository and suggest the best next step.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);

    assert!(
        output.status.success(),
        "ask same-line textual preface follow-up e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the final answer after continuing past the permission reply: {stdout:?}"
    );
    assert!(
        !stdout.contains(permission_reply),
        "stdout should not stop at the permission reply: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        4,
        "ask should continue after the same-line textual preface permission reply: {provider_requests:#?}"
    );
}

#[test]
fn ask_cli_continues_after_second_same_line_textual_tool_request_reply() {
    let fixture = LatestSelectorCliFixture::new("ask-second-same-line-preface-followup-e2e");
    std::fs::create_dir_all(fixture.root_path().join("docs")).expect("create docs dir");
    std::fs::write(
        fixture.root_path().join("docs/README.md"),
        "Documentation overview for second same-line follow-up recovery.",
    )
    .expect("write docs README fixture");
    std::fs::write(
        fixture.root_path().join("ARCHITECTURE.md"),
        "Architecture overview for second same-line follow-up recovery.",
    )
    .expect("write ARCHITECTURE fixture");

    let final_reply = "E2E PASS second same-line textual tool request continuation.";
    let provider_server = DynamicMockProviderServer::spawn(3, move |request_index, request| {
        match request_index {
            0 => MockProviderResponse::ok_json(openai_chat_final_body(
                "to summarize repo, inspect docs first.[tool_request]\n{\"arguments\":{\"path\":\"docs/README.md\"},\"name\":\"read\"}",
            )),
            1 => {
                assert_provider_request_contains_text(
                    request,
                    "Documentation overview for second same-line follow-up recovery.",
                    "second same-line follow-up should include first file content",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(
                    "to summarize repo, inspect architecture next.[tool_request]\n{\"arguments\":{\"path\":\"ARCHITECTURE.md\"},\"name\":\"read\"}",
                ))
            }
            2 => {
                assert_provider_request_contains_text(
                    request,
                    "Architecture overview for second same-line follow-up recovery.",
                    "second same-line continuation should include second file content",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(&format!(
                    "[followup_state:done]\n{final_reply}"
                )))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        }
    });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.file_root = Some(fixture.root_path().display().to_string());
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--session",
            "second-same-line-preface-followup-session",
            "--message",
            "Summarize this repository and suggest the best next step.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);

    assert!(
        output.status.success(),
        "ask second same-line textual preface follow-up e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the final answer after continuing past the second same-line textual tool request: {stdout:?}"
    );
    assert!(
        !stdout.contains("[tool_request]"),
        "stdout should not leak the second same-line textual tool request markup: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        3,
        "ask should continue after the second same-line textual tool request reply: {provider_requests:#?}"
    );
}

#[test]
fn ask_cli_recovers_tool_request_array_wrapper_and_hides_markup() {
    let fixture = LatestSelectorCliFixture::new("ask-tool-request-array-wrapper-e2e");
    std::fs::create_dir_all(fixture.root_path().join("docs")).expect("create docs dir");
    std::fs::write(
        fixture.root_path().join("AGENTS.md"),
        "Repository guidance for tool-request array wrapper recovery.",
    )
    .expect("write AGENTS fixture");
    std::fs::write(
        fixture.root_path().join("docs/README.md"),
        "Documentation overview for tool-request array wrapper recovery.",
    )
    .expect("write docs README fixture");

    let final_reply = "E2E PASS tool-request array wrapper recovery.";
    let provider_server = DynamicMockProviderServer::spawn(2, move |request_index, request| {
        match request_index {
            0 => MockProviderResponse::ok_json(openai_chat_final_body(
                "[tool_request]\n[{\"arguments\":{\"path\":\"AGENTS.md\"},\"name\":\"read\"},{\"arguments\":{\"path\":\"docs/README.md\"},\"name\":\"read\"}]This repository is a Rust workspace.",
            )),
            1 => {
                assert_provider_request_contains_text(
                    request,
                    "Repository guidance for tool-request array wrapper recovery.",
                    "array wrapper follow-up provider request should include AGENTS content",
                );
                assert_provider_request_contains_text(
                    request,
                    "Documentation overview for tool-request array wrapper recovery.",
                    "array wrapper follow-up provider request should include docs README content",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(&format!(
                    "[followup_state:done]\n{final_reply}"
                )))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        }
    });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.file_root = Some(fixture.root_path().display().to_string());
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--session",
            "tool-request-array-wrapper-session",
            "--message",
            "Summarize this repository and suggest the best next step.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);

    assert!(
        output.status.success(),
        "ask tool-request array wrapper e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the final answer after recovering the tool-request array wrapper: {stdout:?}"
    );
    assert!(
        !stdout.contains("[tool_request]"),
        "stdout should not leak tool-request array markup: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        2,
        "ask should continue after recovering the tool-request array wrapper: {provider_requests:#?}"
    );
}

#[test]
fn ask_cli_executes_large_recovered_tool_request_batch_without_legacy_step_limit() {
    let fixture = LatestSelectorCliFixture::new("ask-large-tool-request-array-batch-e2e");
    std::fs::create_dir_all(fixture.root_path().join("docs")).expect("create docs dir");
    std::fs::create_dir_all(fixture.root_path().join("src")).expect("create src dir");
    std::fs::write(
        fixture.root_path().join("AGENTS.md"),
        "Repository guidance for large recovered tool batch.",
    )
    .expect("write AGENTS fixture");
    std::fs::write(
        fixture.root_path().join("README.md"),
        "Repository overview for large recovered tool batch.",
    )
    .expect("write README fixture");
    std::fs::write(
        fixture.root_path().join("ARCHITECTURE.md"),
        "Architecture notes for large recovered tool batch.",
    )
    .expect("write ARCHITECTURE fixture");
    std::fs::write(
        fixture.root_path().join("docs/README.md"),
        "Documentation overview for large recovered tool batch.",
    )
    .expect("write docs README fixture");
    std::fs::write(
        fixture.root_path().join("docs/ROADMAP.md"),
        "Roadmap notes for large recovered tool batch.",
    )
    .expect("write ROADMAP fixture");
    std::fs::write(
        fixture.root_path().join("docs/RELIABILITY.md"),
        "Reliability notes for large recovered tool batch.",
    )
    .expect("write RELIABILITY fixture");
    std::fs::write(
        fixture.root_path().join("Cargo.toml"),
        "[workspace]\nmembers = []\n",
    )
    .expect("write Cargo fixture");

    let final_reply = "E2E PASS large recovered tool batch.";
    let provider_server = DynamicMockProviderServer::spawn(2, move |request_index, request| {
        match request_index {
            0 => MockProviderResponse::ok_json(openai_chat_final_body(
                "[tool_request]\n[{\"arguments\":{\"path\":\"AGENTS.md\"},\"name\":\"read\"},{\"arguments\":{\"path\":\"README.md\"},\"name\":\"read\"},{\"arguments\":{\"path\":\"ARCHITECTURE.md\"},\"name\":\"read\"},{\"arguments\":{\"path\":\"docs/README.md\"},\"name\":\"read\"},{\"arguments\":{\"path\":\"docs/ROADMAP.md\"},\"name\":\"read\"},{\"arguments\":{\"path\":\"docs/RELIABILITY.md\"},\"name\":\"read\"}]This repository is a Rust workspace with layered architecture.",
            )),
            1 => {
                assert_provider_request_contains_text(
                    request,
                    "Repository guidance for large recovered tool batch.",
                    "large recovered batch follow-up should include AGENTS content",
                );
                assert_provider_request_contains_text(
                    request,
                    "Reliability notes for large recovered tool batch.",
                    "large recovered batch follow-up should include RELIABILITY content",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(&format!(
                    "[followup_state:done]\n{final_reply}"
                )))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        }
    });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.file_root = Some(fixture.root_path().display().to_string());
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--session",
            "large-tool-request-array-batch-session",
            "--message",
            "Summarize this repository and suggest the best next step.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);

    assert!(
        output.status.success(),
        "ask large recovered tool batch e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the final answer after executing the large recovered tool batch: {stdout:?}"
    );
    assert!(
        !stdout.contains("max_tool_steps_exceeded"),
        "stdout should not trip the legacy max-tool-steps gate: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        2,
        "ask should finish the large recovered tool batch in one follow-up turn: {provider_requests:#?}"
    );
}

#[test]
fn ask_cli_strips_textual_tool_wrapper_when_structured_tool_call_is_also_present() {
    let fixture = LatestSelectorCliFixture::new("ask-structured-plus-textual-wrapper-e2e");
    std::fs::create_dir_all(fixture.root_path().join("docs")).expect("create docs dir");
    std::fs::write(
        fixture.root_path().join("docs/README.md"),
        "Documentation overview for structured plus textual wrapper recovery.",
    )
    .expect("write docs README fixture");

    let final_reply = "E2E PASS structured tool call plus textual wrapper recovery.";
    let provider_server = DynamicMockProviderServer::spawn(2, move |request_index, request| {
        match request_index {
            0 => MockProviderResponse::ok_json(openai_chat_tool_call_body(
                "To summarize the repository, I will inspect the main docs first.[tool_request]\n{\"arguments\":{\"path\":\"docs/README.md\"},\"name\":\"read\"}",
                "call-docs-readme",
                "read",
                json!({
                    "path": "docs/README.md"
                }),
            )),
            1 => {
                assert_provider_request_contains_text(
                    request,
                    "Documentation overview for structured plus textual wrapper recovery.",
                    "structured plus textual wrapper follow-up should include docs README content",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(&format!(
                    "[followup_state:done]\n{final_reply}"
                )))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        }
    });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.tools.file_root = Some(fixture.root_path().display().to_string());
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--session",
            "structured-plus-textual-wrapper-session",
            "--message",
            "Summarize this repository and suggest the best next step.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);

    assert!(
        output.status.success(),
        "ask structured plus textual wrapper e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the final answer after stripping the leaked textual wrapper: {stdout:?}"
    );
    assert!(
        !stdout.contains("[tool_request]"),
        "stdout should not leak textual tool wrapper when structured tool calls are present: {stdout:?}"
    );
    assert_eq!(
        provider_requests.len(),
        2,
        "ask should continue normally when structured tool calls are paired with leaked textual wrappers: {provider_requests:#?}"
    );
}

#[test]
fn ask_cli_installed_skill_can_be_discovered_and_loaded_e2e() {
    let fixture = LatestSelectorCliFixture::new("ask-installed-skill-e2e");
    let skill_source_dir = fixture.root_path().join("source/release-guard");
    let skill_source_path = skill_source_dir.join("SKILL.md");
    let skill_source = [
        "---",
        "name: release-guard",
        "description: Guard release discipline for CLI E2E validation.",
        "invocation_policy: both",
        "---",
        "",
        "# Release Guard",
        "",
        "SKILL_E2E_INVOCATION_MARKER: apply release verification before completion.",
        "",
    ]
    .join("\n");
    std::fs::create_dir_all(skill_source_dir.as_path()).expect("create skill source");
    std::fs::write(skill_source_path.as_path(), skill_source).expect("write skill source");

    fixture.write_config_with(|config| {
        config.skills.enabled = true;
        config.skills.auto_expose_installed = true;
        config.skills.install_root = Some(
            fixture
                .root_path()
                .join("managed-skills")
                .display()
                .to_string(),
        );
    });

    let install_output = fixture.run_process(
        &[
            "skills",
            "install",
            "source/release-guard",
            "--replace",
            "--json",
        ],
        None,
    );
    let install_stdout = render_output(&install_output.stdout);
    let install_stderr = render_output(&install_output.stderr);
    assert!(
        install_output.status.success(),
        "skills install should succeed, stdout={install_stdout:?}, stderr={install_stderr:?}"
    );
    assert!(
        install_stdout.contains("release-guard"),
        "skills install output should mention the installed skill: {install_stdout:?}"
    );

    let list_output = fixture.run_process(&["skills", "list", "--json"], None);
    let list_stdout = render_output(&list_output.stdout);
    let list_stderr = render_output(&list_output.stderr);
    assert!(
        list_output.status.success(),
        "skills list should succeed, stdout={list_stdout:?}, stderr={list_stderr:?}"
    );
    assert!(
        list_stdout.contains("release-guard"),
        "skills list output should include the installed skill: {list_stdout:?}"
    );

    let info_output = fixture.run_process(&["skills", "info", "release-guard", "--json"], None);
    let info_stdout = render_output(&info_output.stdout);
    let info_stderr = render_output(&info_output.stderr);
    assert!(
        info_output.status.success(),
        "skills info should succeed, stdout={info_stdout:?}, stderr={info_stderr:?}"
    );
    assert!(
        info_stdout.contains("SKILL_E2E_INVOCATION_MARKER"),
        "skills info should preview the installed skill instructions: {info_stdout:?}"
    );

    let final_reply = "E2E PASS installed skill loaded.";
    let provider_server =
        DynamicMockProviderServer::spawn(1, move |request_index, request| match request_index {
            0 => {
                assert_provider_request_contains_text(
                    request,
                    "[available_skills]",
                    "initial skill provider request",
                );
                assert_provider_request_contains_text(
                    request,
                    "release-guard",
                    "initial skill provider request",
                );
                assert_provider_request_contains_text(
                    request,
                    "SKILL_E2E_INVOCATION_MARKER",
                    "initial skill provider request",
                );
                MockProviderResponse::ok_json(openai_chat_final_body(final_reply))
            }
            _ => MockProviderResponse::unexpected_extra_request(),
        });
    let provider_base_url = provider_server.base_url().to_owned();
    fixture.write_config_with(|config| {
        config.provider.kind = ProviderKind::Openai;
        config.provider.base_url = provider_base_url;
        config.provider.model = "test-model".to_owned();
        config.provider.wire_api = ProviderWireApi::ChatCompletions;
        config.provider.api_key = Some(SecretRef::Inline("test-provider-key".to_owned()));
        config.skills.enabled = true;
        config.skills.auto_expose_installed = true;
        config.skills.install_root = Some(
            fixture
                .root_path()
                .join("managed-skills")
                .display()
                .to_string(),
        );
    });

    provider_server.arm();
    let output = fixture.run_process(
        &[
            "ask",
            "--message",
            "Use the release guard skill and confirm the skill instructions were loaded.",
        ],
        None,
    );
    let stdout = render_output(&output.stdout);
    let stderr = render_output(&output.stderr);
    let provider_requests = provider_server.finish(&stdout, &stderr);

    assert!(
        output.status.success(),
        "ask installed-skill e2e should succeed, stdout={stdout:?}, stderr={stderr:?}"
    );
    assert_eq!(
        provider_requests.len(),
        1,
        "ask should reach the provider with injected skill context in one request: {provider_requests:#?}"
    );
    assert!(
        stdout.contains(final_reply),
        "stdout should contain the terminal skill answer: {stdout:?}"
    );
}
