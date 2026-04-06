use base64::Engine;
use serde_json::Value;

use super::*;

#[cfg(unix)]
#[test]
fn fake_acpx_script_helpers_work_with_empty_path() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let temp_dir = unique_temp_dir("loongclaw-acpx-script-builtins");
    let log_path = temp_dir.join("calls.log");
    let script_path = write_fake_acpx_script(
        &temp_dir,
        "fake-acpx",
        &log_path,
        r#"
if args_contain "$*" 'prompt --session'; then
  drain_stdin
  echo '{"type":"text","content":"builtins ok"}'
  echo '{"type":"done"}'
  exit 0
fi

exit 0
"#,
    );

    let mut command = Command::new(&script_path);
    command
        .args(["prompt", "--session", "sess-builtins", "--file", "-"])
        .current_dir(&temp_dir)
        .env("PATH", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child =
        retry_executable_file_busy_blocking(|| command.spawn()).expect("spawn fake acpx script");
    let mut stdin = child.stdin.take().expect("fake acpx stdin");
    stdin
        .write_all(b"payload without trailing newline")
        .expect("write fake acpx stdin");
    drop(stdin);

    let output = child.wait_with_output().expect("wait for fake acpx script");
    assert!(output.status.success(), "fake acpx script should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("{\"type\":\"text\",\"content\":\"builtins ok\"}"),
        "expected built-in helper response in stdout: {stdout}"
    );
    assert!(
        stdout.contains("{\"type\":\"done\"}"),
        "expected done event in stdout: {stdout}"
    );
}

#[test]
fn build_mcp_proxy_agent_command_preserves_server_cwd() {
    fn decode_quoted_command_part(value: &str) -> String {
        let trimmed = value.trim();
        let quoted = trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2;
        if !quoted {
            return trimmed.to_owned();
        }

        let inner = &trimmed[1..trimmed.len() - 1];
        let unescaped_backslashes = inner.replace("\\\\", "\\");
        unescaped_backslashes.replace("\\\"", "\"")
    }

    let server = AcpxMcpServerEntry {
        name: "docs".to_owned(),
        command: "uvx".to_owned(),
        args: vec!["context7-mcp".to_owned()],
        env: vec![AcpxMcpServerEnvEntry {
            name: "API_TOKEN".to_owned(),
            value: "secret".to_owned(),
        }],
        cwd: Some("/workspace/docs".to_owned()),
    };

    let command = build_mcp_proxy_agent_command("npx @zed-industries/codex-acp", &[server])
        .expect("proxy command");
    let payload_marker = "--payload ";
    let payload_index = command.find(payload_marker).expect("payload marker");
    let encoded_payload = &command[payload_index + payload_marker.len()..];
    let encoded_payload = decode_quoted_command_part(encoded_payload);
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded_payload)
        .expect("decode payload");
    let payload: Value = serde_json::from_slice(&payload_bytes).expect("parse payload");

    assert_eq!(
        payload["mcpServers"][0]["cwd"],
        Value::String("/workspace/docs".to_owned())
    );
}
