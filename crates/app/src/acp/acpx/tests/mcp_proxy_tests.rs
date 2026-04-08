use std::time::Duration;

use serde_json::Value;

use super::*;

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
    let payload_marker = "--payload-file ";
    let payload_index = command.find(payload_marker).expect("payload marker");
    let payload_path = &command[payload_index + payload_marker.len()..];
    let payload_path = decode_quoted_command_part(payload_path);
    let payload_bytes = std::fs::read(&payload_path).expect("read payload file");
    let payload: Value = serde_json::from_slice(payload_bytes.as_slice()).expect("parse payload");

    assert_eq!(
        payload["mcpServers"][0]["cwd"],
        Value::String("/workspace/docs".to_owned())
    );

    std::fs::remove_file(payload_path).ok();
}

#[cfg(unix)]
#[test]
fn probe_mcp_proxy_support_invokes_script_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("create test runtime");
    let _guard = runtime.block_on(lock_acpx_runtime_tests());
    let temp_dir = unique_temp_dir("loongclaw-acpx-mcp-probe-runtime");
    let node_log_path = temp_dir.join("node-args.log");
    let node_script_path = temp_dir.join("fake-node.sh");
    let node_script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" > '{}'\nprintf 'fake-node 1\\n'\nexit 0\n",
        node_log_path.display()
    );
    write_executable_script_atomically(&node_script_path, node_script.as_str())
        .expect("write fake node script");

    let embedded_script_path = temp_dir.join("embedded-proxy.mjs");
    write_executable_script_atomically(&embedded_script_path, "#!/bin/sh\nexit 0\n")
        .expect("write embedded proxy script");

    let (reported_script_path, observed) = runtime
        .block_on(probe_mcp_proxy_support_with_runtime(
            node_script_path.to_string_lossy().as_ref(),
            embedded_script_path.to_string_lossy().as_ref(),
            Some(temp_dir.to_string_lossy().as_ref()),
            Duration::from_secs(30),
        ))
        .expect("probe should succeed with fake runtime");

    let logged_args = std::fs::read_to_string(&node_log_path).expect("read fake node log");

    assert_eq!(
        reported_script_path,
        embedded_script_path.display().to_string()
    );
    assert_eq!(observed, "fake-node 1");
    assert!(
        logged_args.contains(embedded_script_path.to_string_lossy().as_ref()),
        "probe should execute the embedded proxy script, got: {logged_args}"
    );
    assert!(
        logged_args.contains("--version"),
        "probe should pass --version to the embedded proxy script, got: {logged_args}"
    );
}
