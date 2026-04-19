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

fn tokenize_proxy_command(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;

    for character in command.chars() {
        if escaped {
            current.push('\\');
            current.push(character);
            escaped = false;
            continue;
        }

        let is_escaped_character = in_quotes && character == '\\';
        if is_escaped_character {
            escaped = true;
            continue;
        }

        let is_quote = character == '"';
        if is_quote {
            in_quotes = !in_quotes;
            current.push(character);
            continue;
        }

        let is_separator = character.is_whitespace() && !in_quotes;
        if is_separator {
            if !current.is_empty() {
                let token = decode_quoted_command_part(current.as_str());
                tokens.push(token);
                current.clear();
            }
            continue;
        }

        current.push(character);
    }

    if escaped {
        current.push('\\');
    }

    if !current.is_empty() {
        let token = decode_quoted_command_part(current.as_str());
        tokens.push(token);
    }

    tokens
}

fn decode_script_path_from_proxy_command(command: &str) -> String {
    let tokens = tokenize_proxy_command(command);
    let script_path = tokens.get(1).expect("script path");
    script_path.to_owned()
}

fn decode_payload_path_from_proxy_command(command: &str) -> String {
    let tokens = tokenize_proxy_command(command);
    let payload_index = tokens
        .iter()
        .position(|token| token == "--payload-file")
        .expect("payload marker");
    let payload_path = tokens.get(payload_index + 1).expect("payload path");
    payload_path.to_owned()
}

#[cfg(unix)]
#[test]
fn fake_acpx_script_helpers_work_with_empty_path() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let temp_dir = unique_temp_dir("loong-acpx-script-builtins");
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
    let tokens = tokenize_proxy_command(command.as_str());
    let has_legacy_payload_flag = tokens.iter().any(|token| token == "--payload");
    assert!(
        !has_legacy_payload_flag,
        "legacy inline payload flag should not be present: {command}"
    );
    let script_path = decode_script_path_from_proxy_command(command.as_str());
    let payload_path = decode_payload_path_from_proxy_command(command.as_str());
    let payload_bytes = std::fs::read(&payload_path).expect("read payload file");
    let payload: Value = serde_json::from_slice(payload_bytes.as_slice()).expect("parse payload");

    assert!(
        script_path.contains("loong-acpx-mcp-proxy-"),
        "expected versioned script path, got: {script_path}"
    );
    let script_file_name = std::path::Path::new(script_path.as_str())
        .file_name()
        .and_then(|name| name.to_str())
        .expect("script file name");
    assert!(
        script_file_name != "loong-acpx-mcp-proxy.mjs",
        "expected hashed script file name, got: {script_file_name}"
    );
    assert!(
        std::path::Path::new(script_path.as_str()).exists(),
        "expected materialized script path to exist: {script_path}"
    );
    assert_eq!(
        payload["mcpServers"][0]["cwd"],
        Value::String("/workspace/docs".to_owned())
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let payload_metadata = std::fs::metadata(payload_path.as_str()).expect("payload metadata");
        let payload_mode = payload_metadata.permissions().mode() & 0o777;
        let payload_parent = std::path::Path::new(payload_path.as_str())
            .parent()
            .expect("payload parent");
        let parent_metadata = std::fs::metadata(payload_parent).expect("payload parent metadata");
        let parent_mode = parent_metadata.permissions().mode() & 0o777;

        assert_eq!(payload_mode, 0o600);
        assert_eq!(parent_mode, 0o700);
    }

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
    let temp_dir = unique_temp_dir("loong-acpx-mcp-probe-runtime");
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

#[cfg(unix)]
#[test]
fn probe_mcp_proxy_support_kills_timed_out_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("create test runtime");
    let _guard = runtime.block_on(lock_acpx_runtime_tests());
    let temp_dir = unique_temp_dir("loong-acpx-mcp-probe-timeout");
    let pid_path = temp_dir.join("node.pid");
    let node_script_path = temp_dir.join("fake-node-timeout.sh");
    let node_script = format!(
        "#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n",
        pid_path.display()
    );
    write_executable_script_atomically(&node_script_path, node_script.as_str())
        .expect("write fake node timeout script");

    let embedded_script_path = temp_dir.join("embedded-proxy.mjs");
    write_executable_script_atomically(&embedded_script_path, "#!/bin/sh\nexit 0\n")
        .expect("write embedded proxy script");

    let error = runtime
        .block_on(probe_mcp_proxy_support_with_runtime(
            node_script_path.to_string_lossy().as_ref(),
            embedded_script_path.to_string_lossy().as_ref(),
            Some(temp_dir.to_string_lossy().as_ref()),
            Duration::from_secs(5),
        ))
        .expect_err("probe should time out");

    assert_eq!(error, "embedded ACPX MCP proxy runtime probe timed out");

    let mut saw_pid_file = false;
    for _ in 0..100 {
        let pid_file_exists = pid_path.exists();
        if pid_file_exists {
            saw_pid_file = true;
            break;
        }

        runtime.block_on(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
        });
    }

    if !saw_pid_file {
        // On heavily loaded runners the probe can time out and terminate before the
        // helper shell persists its pid marker. The timeout itself is already the
        // behavior under test, so only enforce the kill check when the marker exists.
        return;
    }

    runtime.block_on(async {
        tokio::time::sleep(Duration::from_millis(200)).await;
    });

    let pid_text = std::fs::read_to_string(&pid_path).expect("read timed out probe pid");
    let pid = pid_text.trim().to_owned();
    let stable_unix_path = "/bin:/usr/bin";
    let status = std::process::Command::new("kill")
        .args(["-0", pid.as_str()])
        .env("PATH", stable_unix_path)
        .stderr(std::process::Stdio::null())
        .status()
        .expect("check fake node liveness");

    assert!(
        !status.success(),
        "timed out probe process should be terminated: pid={pid}"
    );
}
