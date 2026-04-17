use super::*;

#[cfg(all(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn tool_search_hides_filesystem_tools_without_filesystem_capabilities() {
    let root = std::env::temp_dir().join(format!(
        "loong-tool-search-cap-filter-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "read file import config",
                TOOL_SEARCH_GRANTED_CAPABILITIES_FIELD: serde_json::to_value(BTreeSet::from([Capability::InvokeTool]))
                    .expect("serialize capabilities")
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    assert!(results.iter().all(|entry| entry["tool_id"] != "file.read"));
    assert!(results.iter().all(|entry| entry["tool_id"] != "file.write"));
    assert!(
        results
            .iter()
            .all(|entry| entry["tool_id"] != "config.import")
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-shell")]
#[test]
fn tool_search_includes_shell_exec_when_runtime_allowlist_is_empty() {
    let root = std::env::temp_dir().join(format!(
        "loong-tool-search-shell-filter-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = runtime_config::ToolRuntimeConfig {
        shell_allow: BTreeSet::new(),
        shell_default_mode: shell_policy_ext::ShellPolicyDefault::Deny,
        file_root: Some(root.clone()),
        messages_enabled: true,
        ..Default::default()
    };

    let request = ToolCoreRequest {
        tool_name: "tool.search".to_owned(),
        payload: json!({"exact_tool_id": "shell.exec"}),
    };
    let outcome =
        execute_tool_core_with_config(request, &config).expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    let shell_entry = results
        .iter()
        .find(|entry| entry["tool_id"] == "exec")
        .expect("direct exec should remain discoverable");

    assert!(shell_entry.get("lease").is_none());

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-shell")]
#[test]
fn shell_exec_rejects_path_qualified_commands() {
    let config = test_tool_runtime_config(std::env::temp_dir());
    for cmd in ["/tmp/git", "./git", "../git", "..\\git", "/usr/bin/ls"] {
        let error = execute_tool_core_with_config(
            ToolCoreRequest {
                tool_name: "shell.exec".to_owned(),
                payload: json!({"command": cmd}),
            },
            &config,
        )
        .expect_err(&format!("path-qualified `{cmd}` should be denied"));
        assert!(
            error.contains("path separators"),
            "expected path separator rejection for `{cmd}`, got: {error}"
        );
    }
}

#[cfg(feature = "tool-shell")]
#[test]
fn shell_exec_rejects_cwd_outside_file_root() {
    let root = unique_tool_temp_dir("loong-shell-cwd-root");
    let outside_root = unique_tool_temp_dir("loong-shell-cwd-outside");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::create_dir_all(&outside_root).expect("create outside root");

    let config = test_tool_runtime_config(root.clone());
    let outside_cwd = outside_root.display().to_string();
    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": "ls",
                "cwd": outside_cwd,
            }),
        },
        &config,
    )
    .expect_err("cwd outside file_root should be denied");

    assert!(
        error.contains("escapes configured file root"),
        "expected file-root escape denial, got: {error}"
    );

    std::fs::remove_dir_all(&root).ok();
    std::fs::remove_dir_all(&outside_root).ok();
}

#[cfg(feature = "tool-shell")]
#[test]
fn shell_exec_rejects_cwd_that_is_not_directory() {
    let root = unique_tool_temp_dir("loong-shell-cwd-file");
    std::fs::create_dir_all(&root).expect("create root");
    let file_path = root.join("note.txt");
    std::fs::write(&file_path, "hello").expect("write file");

    let config = test_tool_runtime_config(root.clone());
    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": "ls",
                "cwd": "note.txt",
            }),
        },
        &config,
    )
    .expect_err("file cwd should be denied");

    assert!(
        error.contains("is not a directory"),
        "expected non-directory cwd denial, got: {error}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(all(unix, feature = "tool-shell"))]
#[test]
fn shell_exec_rejects_cwd_symlink_outside_file_root() {
    use std::os::unix::fs::symlink;

    let root = unique_tool_temp_dir("loong-shell-cwd-symlink-root");
    let outside_root = unique_tool_temp_dir("loong-shell-cwd-symlink-outside");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::create_dir_all(&outside_root).expect("create outside root");

    let link_path = root.join("outside-link");
    symlink(&outside_root, &link_path).expect("create cwd symlink");

    let config = test_tool_runtime_config(root.clone());
    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": "ls",
                "cwd": "outside-link",
            }),
        },
        &config,
    )
    .expect_err("symlink cwd should be denied");

    assert!(
        error.contains("escapes configured file root"),
        "expected file-root escape denial, got: {error}"
    );

    std::fs::remove_dir_all(&root).ok();
    std::fs::remove_dir_all(&outside_root).ok();
}

#[cfg(feature = "tool-shell")]
#[test]
fn shell_exec_rejects_missing_cwd_directory() {
    let root = unique_tool_temp_dir("loong-shell-cwd-missing");
    std::fs::create_dir_all(&root).expect("create root");

    let config = test_tool_runtime_config(root.clone());
    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": "ls",
                "cwd": "missing-dir",
            }),
        },
        &config,
    )
    .expect_err("missing cwd should be denied");

    assert!(
        error.contains("does not exist"),
        "expected missing cwd denial, got: {error}"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn tool_execution_config_timeout_for_tool_prefers_per_tool() {
    use std::collections::BTreeMap;

    let mut per_tool = BTreeMap::new();
    per_tool.insert("file.read".to_owned(), 30u64);

    let config = runtime_config::ToolExecutionConfig {
        default_timeout_seconds: Some(60u64),
        per_tool_timeout: per_tool,
    };

    assert_eq!(config.timeout_for_tool("file.read"), Some(30));
    assert_eq!(config.timeout_for_tool("file.write"), Some(60));
    assert_eq!(config.timeout_for_tool("unknown"), Some(60));
}

#[test]
fn tool_execution_config_timeout_for_tool_none_when_no_default() {
    let config = runtime_config::ToolExecutionConfig {
        default_timeout_seconds: None,
        per_tool_timeout: BTreeMap::new(),
    };

    assert_eq!(config.timeout_for_tool("file.read"), None);
}

#[test]
fn tool_execution_config_default_is_no_timeout() {
    let config = runtime_config::ToolExecutionConfig::default();
    assert_eq!(config.default_timeout_seconds, None);
    assert!(config.per_tool_timeout.is_empty());
}

#[test]
fn framework_timeout_excludes_tools_with_dedicated_timeout_controls() {
    assert!(tool_uses_dedicated_timeout(
        "browser.companion.session.start"
    ));
    assert!(tool_uses_dedicated_timeout("browser.companion.wait"));
    assert!(tool_uses_dedicated_timeout("delegate"));
    assert!(tool_uses_dedicated_timeout("delegate_async"));
    assert!(tool_uses_dedicated_timeout("shell.exec"));
    assert!(tool_uses_dedicated_timeout("web.fetch"));
    assert!(tool_uses_dedicated_timeout("web.search"));
    assert!(!tool_uses_dedicated_timeout("file.read"));
}

#[test]
fn tool_without_timeout_config_completes_normally() {
    use std::fs;

    let root = unique_temp_dir("no-timeout-test");
    fs::create_dir_all(&root).expect("create temp dir");
    let readme_path = root.join("README.md");
    fs::write(&readme_path, "readme content").expect("write test file");

    let config = test_tool_runtime_config(root);

    let request = ToolCoreRequest {
        tool_name: "file.read".to_owned(),
        payload: json!({
            "path": "README.md"
        }),
    };

    let result = execute_tool_core_with_test_context(request, &config);

    assert!(
        result.is_ok(),
        "tool should complete normally without timeout, got: {result:?}"
    );
}

#[test]
fn framework_timeout_returns_without_waiting_for_worker_completion() {
    let start = std::time::Instant::now();

    let error = run_blocking_with_timeout(
        || {
            let (_sender, receiver) = mpsc::channel::<()>();
            let _ = receiver.recv_timeout(Duration::from_secs(3));
            Ok::<(), String>(())
        },
        1,
        "file.read",
    )
    .expect_err("timeout should be reported");

    let elapsed = start.elapsed();

    assert_eq!(error, "tool_execution_timeout: file.read exceeded 1s");
    assert!(
        elapsed < Duration::from_secs(2),
        "timeout helper should return promptly, got {elapsed:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn framework_timeout_supports_async_core_tool_calls() {
    use std::fs;

    let root = unique_temp_dir("tool-timeout-async-core");
    fs::create_dir_all(&root).expect("create temp dir");
    let readme_path = root.join("README.md");
    fs::write(&readme_path, "readme content").expect("write test file");

    let mut config = test_tool_runtime_config(root);
    config.tool_execution.default_timeout_seconds = Some(1);

    let adapter = MvpToolAdapter::with_config(config.into_inner());
    let request = ToolCoreRequest {
        tool_name: "file.read".to_owned(),
        payload: json!({
            "path": "README.md"
        }),
    };

    let result = loong_kernel::CoreToolAdapter::execute_core_tool(&adapter, request).await;

    assert!(
        result.is_ok(),
        "async core tool execution should stay usable with framework timeouts, got: {result:?}"
    );
}

#[cfg(feature = "tool-shell")]
#[test]
fn shell_exec_normalizes_embedded_whitespace_into_args_when_args_missing() {
    #[cfg(unix)]
    let config = test_tool_runtime_config(std::env::temp_dir());
    #[cfg(windows)]
    let mut config = test_tool_runtime_config(std::env::temp_dir());
    #[cfg(windows)]
    config.shell_allow.insert("cmd".to_owned());
    #[cfg(unix)]
    let command = "echo hello world";
    #[cfg(windows)]
    let command = "cmd /C echo hello world";
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({"command": command}),
        },
        &config,
    )
    .expect("embedded whitespace should be normalized into args");
    assert_eq!(outcome.status, "ok");
    #[cfg(unix)]
    assert_eq!(outcome.payload["command"], "echo");
    #[cfg(unix)]
    assert_eq!(outcome.payload["args"], json!(["hello", "world"]));
    #[cfg(windows)]
    assert_eq!(outcome.payload["command"], "cmd");
    #[cfg(windows)]
    assert_eq!(
        outcome.payload["args"],
        json!(["/C", "echo", "hello", "world"])
    );
    assert_eq!(outcome.payload["stdout"], json!("hello world"));
}

#[cfg(feature = "tool-shell")]
#[test]
fn tool_invoke_shell_exec_normalizes_embedded_whitespace_into_args_when_args_missing() {
    #[cfg(unix)]
    let config = test_tool_runtime_config(std::env::temp_dir());
    #[cfg(windows)]
    let mut config = test_tool_runtime_config(std::env::temp_dir());
    #[cfg(windows)]
    config.shell_allow.insert("cmd".to_owned());
    #[cfg(unix)]
    let command = "echo hello from invoke";
    #[cfg(windows)]
    let command = "cmd /C echo hello from invoke";
    let lease =
        crate::tools::tool_lease_authority::issue_tool_lease("shell.exec", &serde_json::Map::new())
            .expect("tool lease");
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.invoke".to_owned(),
            payload: json!({
                "tool_id": "shell.exec",
                "lease": lease,
                "arguments": {"command": command}
            }),
        },
        &config,
    )
    .expect("tool.invoke shell payload should be normalized into args");
    assert_eq!(outcome.status, "ok");
    #[cfg(unix)]
    assert_eq!(outcome.payload["command"], "echo");
    #[cfg(unix)]
    assert_eq!(outcome.payload["args"], json!(["hello", "from", "invoke"]));
    #[cfg(windows)]
    assert_eq!(outcome.payload["command"], "cmd");
    #[cfg(windows)]
    assert_eq!(
        outcome.payload["args"],
        json!(["/C", "echo", "hello", "from", "invoke"])
    );
    assert_eq!(outcome.payload["stdout"], json!("hello from invoke"));
}

#[cfg(feature = "tool-shell")]
#[test]
fn shell_exec_does_not_normalize_multiline_command_into_args() {
    let config = test_tool_runtime_config(std::env::temp_dir());
    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({"command": "echo hello\nworld"}),
        },
        &config,
    )
    .expect_err("multiline commands should stay repairable instead of executing");

    assert!(
        error.contains("payload.command"),
        "expected payload.command validation failure, got: {error}"
    );
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn shell_exec_rejects_non_lowercase_command_names_before_execution() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let root = unique_tool_temp_dir("loong-shell-mixed-case");
    fs::create_dir_all(&root).expect("create fixture root");

    let script = root.join("MiXeDCmd");
    fs::write(&script, "#!/bin/sh\nprintf '%s' \"$0\"\n").expect("write mixed-case script");
    let mut perms = fs::metadata(&script)
        .expect("script metadata")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script, perms).expect("mark script executable");

    let mut env = ScopedEnv::new();
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let mut path_value = root.clone().into_os_string();
    if !original_path.is_empty() {
        path_value.push(std::ffi::OsStr::new(":"));
        path_value.push(original_path);
    }
    env.set("PATH", path_value);

    let mut config = test_tool_runtime_config(&root);
    config.shell_allow = BTreeSet::from(["mixedcmd".to_owned()]);

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({"command": "MiXeDCmd"}),
        },
        &config,
    )
    .expect_err("mixed-case commands should be rejected before execution");

    assert!(
        error.contains("lowercase"),
        "expected lowercase command rejection, got: {error}"
    );

    fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-shell")]
#[test]
fn shell_exec_times_out_when_timeout_ms_is_small() {
    let mut config = test_tool_runtime_config(std::env::temp_dir());
    #[cfg(unix)]
    {
        config.shell_allow.insert("sleep".to_owned());
    }
    #[cfg(windows)]
    {
        config.shell_allow.insert("ping".to_owned());
    }

    #[cfg(unix)]
    let (command, args) = ("sleep", vec!["10"]);
    #[cfg(windows)]
    let (command, args) = ("ping", vec!["127.0.0.1", "-n", "10"]);

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": command,
                "args": args,
                "timeout_ms": 1,
            }),
        },
        &config,
    )
    .expect_err("slow command should time out");

    assert!(
        error.contains("timed out after"),
        "expected timeout failure for long command, got: {error}"
    );
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn shell_exec_timeout_returns_without_waiting_for_descendant_pipe_holders() {
    let mut config = test_tool_runtime_config(std::env::temp_dir());
    let args = vec!["-c", "sleep 5 & wait"];
    let started_at = std::time::Instant::now();

    config.shell_allow.insert("sh".to_owned());

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": "sh",
                "args": args,
                "timeout_ms": 1_000,
            }),
        },
        &config,
    )
    .expect_err("timed-out shell should return an error");

    let elapsed = started_at.elapsed();

    assert!(
        error.contains("timed out after 1000ms"),
        "expected timeout message, got: {error}"
    );
    assert!(
        elapsed < std::time::Duration::from_millis(2_500),
        "timeout path should not wait for descendant pipe holders; elapsed={elapsed:?}"
    );
}

#[cfg(feature = "tool-shell")]
#[test]
fn shell_exec_succeeds_when_fast_command_receives_timeout_ms() {
    #[cfg(unix)]
    let config = test_tool_runtime_config(std::env::temp_dir());
    #[cfg(windows)]
    let mut config = test_tool_runtime_config(std::env::temp_dir());

    #[cfg(unix)]
    let (command, args, expected_stdout) = ("echo", vec!["hello"], "hello");
    #[cfg(windows)]
    {
        config.shell_allow.insert("cmd".to_owned());
    }
    #[cfg(windows)]
    let (command, args, expected_stdout) = ("cmd", vec!["/C", "echo", "hello"], "hello");

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": command,
                "args": args,
                "timeout_ms": 5_000,
            }),
        },
        &config,
    )
    .expect("fast command should succeed");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["stdout"].as_str(), Some(expected_stdout));
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn shell_exec_truncates_large_stdout_without_failing_command() {
    use std::process::Command;

    const SHELL_STDOUT_TRUNCATION_LIMIT: usize = 1_048_576;

    let perl_available = Command::new("perl")
        .arg("-v")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !perl_available {
        eprintln!("skipping large stdout shell test because perl is unavailable");
        return;
    }

    let mut config = test_tool_runtime_config(std::env::temp_dir());
    config.shell_allow.insert("perl".to_owned());

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({
                "command": "perl",
                "args": ["-e", "print chr(97) x 2000000"],
                "timeout_ms": 5_000,
            }),
        },
        &config,
    )
    .expect("large-output command should still complete");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["exit_code"].as_i64(), Some(0));

    let stdout = outcome.payload["stdout"]
        .as_str()
        .expect("stdout should be present");
    assert_eq!(stdout.len(), SHELL_STDOUT_TRUNCATION_LIMIT);
    assert!(stdout.bytes().all(|byte| byte == b'a'));
}

#[cfg(all(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn tool_search_result_includes_compact_argument_hints() {
    let root = std::env::temp_dir().join(format!("loong-tool-search-hints-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({"query": "shell command"}),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    assert!(results.iter().any(|entry| {
        entry["tool_id"] == "exec"
            && entry["argument_hint"].as_str()
                == Some("command:string,args?:string[],timeout_ms?:integer,cwd?:string")
    }));

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-file")]
#[test]
fn tool_search_exact_tool_id_refresh_returns_one_current_card_with_lease() {
    let root = unique_tool_temp_dir("loong-tool-search-exact-refresh");
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "exact_tool_id": "file.read"
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    let first = results.first().expect("one result should be returned");

    assert_eq!(outcome.payload["returned"], 1);
    assert_eq!(first["tool_id"], "read");
    assert_eq!(first["surface_id"], "read");
    assert!(
        first["usage_guidance"]
            .as_str()
            .is_some_and(|value| value.contains("direct read first"))
    );
    assert!(first.get("lease").is_none());

    std::fs::remove_dir_all(&root).ok();
}

#[cfg(all(feature = "tool-file", feature = "tool-shell"))]
#[test]
fn tool_search_exact_tool_id_not_visible_preserves_raw_request_and_diagnostics_with_fallback_results()
 {
    let root = unique_tool_temp_dir("loong-tool-search-exact-refresh-fallback");
    std::fs::create_dir_all(&root).expect("create fixture root");

    let config = test_tool_runtime_config(root.clone());
    let outcome = execute_tool_core_with_test_context(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "exact_tool_id": "file_read",
                "query": "run shell command",
                "_loong": {
                    "tool_search": {
                        "visible_tool_ids": ["tool.search", "tool.invoke", "shell.exec"],
                    }
                }
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    let diagnostics = &outcome.payload["diagnostics"];

    assert!(!results.is_empty());
    assert_eq!(results[0]["tool_id"], "exec");
    assert_eq!(outcome.payload["exact_tool_id"], "file_read");
    assert_eq!(diagnostics["reason"], "exact_tool_id_not_visible");
    assert_eq!(diagnostics["requested_tool_id"], "file_read");

    std::fs::remove_dir_all(&root).ok();
}
