use super::latest_selector_process_support::LatestSelectorCliFixture;
use loong_app::config::ProviderKind;
use loong_contracts::SecretRef;
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

fn read_provider_request(stream: &mut TcpStream) -> String {
    stream
        .set_nonblocking(false)
        .expect("set provider stream blocking");
    stream
        .set_read_timeout(Some(MOCK_PROVIDER_STREAM_READ_TIMEOUT))
        .expect("set provider stream read timeout");
    let mut request_buffer = [0_u8; 8192];
    let request_len = stream
        .read(&mut request_buffer)
        .expect("read provider request");
    let request_bytes = request_buffer
        .get(..request_len)
        .expect("provider request length should fit within the read buffer");
    String::from_utf8_lossy(request_bytes).into_owned()
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
