use super::*;

#[cfg(feature = "tool-shell")]
#[test]
fn runtime_tool_view_hides_bash_exec_when_runtime_is_unavailable() {
    let root = unique_tool_temp_dir("loong-bash-tool-view-hidden");
    std::fs::create_dir_all(&root).expect("create root dir");

    let config = test_tool_runtime_config(root);
    let tool_view = runtime_tool_view_for_runtime_config(&config);

    assert!(!tool_view.contains("bash.exec"));
}

#[cfg(feature = "tool-shell")]
#[test]
fn runtime_tool_view_includes_bash_exec_when_runtime_is_available() {
    let root = unique_tool_temp_dir("loong-bash-tool-view-visible");
    std::fs::create_dir_all(&root).expect("create root dir");

    let mut config = test_tool_runtime_config(root);
    config.bash_exec = ready_bash_exec_runtime_policy();
    let tool_view = runtime_tool_view_for_runtime_config(&config);

    assert!(tool_view.contains("bash.exec"));
}

#[cfg(feature = "tool-shell")]
#[test]
fn tool_search_hides_bash_exec_when_runtime_is_unavailable() {
    let root = unique_tool_temp_dir("loong-bash-tool-search-hidden");
    std::fs::create_dir_all(&root).expect("create root dir");

    let config = test_tool_runtime_config(root);
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "bash command cwd timeout",
                "limit": 10
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    assert!(results.iter().all(|entry| entry["tool_id"] != "bash.exec"));
}

#[cfg(feature = "tool-shell")]
#[test]
fn tool_search_hides_bash_exec_when_governance_rules_failed_to_load() {
    let root = unique_tool_temp_dir("loong-bash-tool-search-broken-rules");
    std::fs::create_dir_all(&root).expect("create root dir");

    let mut config = test_tool_runtime_config(root);
    config.bash_exec = ready_bash_exec_runtime_policy();
    config.bash_exec.governance.load_error = Some("broken rules".to_owned());
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "bash command cwd timeout",
                "limit": 10
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    assert!(
        results.iter().all(|entry| entry["tool_id"] != "bash.exec"),
        "bash.exec should stay hidden when governance rules fail to load"
    );
}

#[cfg(feature = "tool-shell")]
#[test]
fn tool_search_includes_bash_exec_when_runtime_is_available() {
    let root = unique_tool_temp_dir("loong-bash-tool-search-visible");
    std::fs::create_dir_all(&root).expect("create root dir");

    let mut config = test_tool_runtime_config(root);
    config.bash_exec = ready_bash_exec_runtime_policy();
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "bash command cwd timeout",
                "limit": 10
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    assert!(
        results.iter().any(|entry| entry["tool_id"] == "bash-exec"),
        "runtime-ready bash-exec should appear in tool search results"
    );
}

#[cfg(feature = "tool-shell")]
#[test]
fn tool_search_exact_query_surfaces_bash_exec() {
    let root = unique_tool_temp_dir("loong-bash-tool-search-exact-query");
    std::fs::create_dir_all(&root).expect("create root dir");

    let mut config = test_tool_runtime_config(root);
    config.bash_exec = ready_bash_exec_runtime_policy();
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({
                "query": "bash.exec",
                "limit": 10
            }),
        },
        &config,
    )
    .expect("tool search should succeed");

    let results = outcome.payload["results"].as_array().expect("results");
    assert!(
        results.iter().any(|entry| entry["tool_id"] == "bash-exec"),
        "exact tool-id query should surface bash-exec, got: {results:?}"
    );
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_catalog_exposes_command_cwd_and_timeout_ms() {
    let catalog = tool_catalog();
    let descriptor = catalog
        .descriptor("bash.exec")
        .expect("bash.exec should be in the catalog");
    let definition = descriptor.provider_definition();
    let properties = definition["function"]["parameters"]["properties"]
        .as_object()
        .expect("bash.exec parameters");

    assert!(properties.contains_key("command"));
    assert!(properties.contains_key("cwd"));
    assert!(properties.contains_key("timeout_ms"));
    assert!(!properties.contains_key("args"));
    assert_eq!(properties["timeout_ms"]["minimum"], json!(1000));
    assert_eq!(properties["timeout_ms"]["maximum"], json!(600000));
    assert_eq!(
        definition["function"]["parameters"]["required"],
        json!(["command"])
    );

    let entry = catalog::find_tool_catalog_entry("bash.exec")
        .expect("bash.exec should be in catalog entries");
    assert_eq!(
        entry.argument_hint,
        "command:string,cwd?:string,timeout_ms?:integer"
    );
    assert_eq!(
        entry.parameter_types,
        vec![
            ("command", "string"),
            ("cwd", "string"),
            ("timeout_ms", "integer"),
        ]
    );
    assert_eq!(entry.required_fields, vec!["command"]);
    assert_eq!(entry.tags, vec!["bash", "command", "process", "exec"]);
}

#[test]
fn canonical_tool_name_maps_bash_exec_provider_name() {
    assert_eq!(canonical_tool_name("bash_exec"), "bash.exec");
}

#[test]
fn framework_timeout_treats_bash_exec_as_dedicated_timeout_tool() {
    assert!(tool_uses_dedicated_timeout("bash.exec"));
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_rejects_blank_command() {
    let config = test_tool_runtime_config(std::env::temp_dir());
    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "  "}),
        },
        &config,
    )
    .expect_err("blank command should be rejected");

    assert!(
        error.contains("bash.exec requires payload.command"),
        "expected blank-command validation error, got: {error}"
    );
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_returns_runtime_unavailable_error_when_no_bash_is_configured() {
    let config = test_tool_runtime_config(std::env::temp_dir());
    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "printf 'hi\\n'"}),
        },
        &config,
    )
    .expect_err("missing runtime should fail");

    assert!(
        error.contains("bash unavailable"),
        "expected runtime-unavailable error, got: {error}"
    );
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_fails_closed_when_rule_loading_failed() {
    let mut config = test_tool_runtime_config(std::env::temp_dir());
    config.bash_exec = ready_bash_exec_runtime_policy();
    config.bash_exec.governance.load_error = Some("broken rules".to_owned());

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "cargo test"}),
        },
        &config,
    )
    .expect_err("broken rules should fail closed");

    assert!(error.contains("broken rules"));
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_reports_failed_status_for_non_zero_exit() {
    let mut config = test_tool_runtime_config(std::env::temp_dir());
    config.shell_default_mode = shell_policy_ext::ShellPolicyDefault::Allow;
    config.bash_exec = ready_bash_exec_runtime_policy();

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "printf 'hello'; exit 7"}),
        },
        &config,
    )
    .expect("non-zero exit should still produce an outcome");

    assert_eq!(outcome.status, "failed");
    assert_eq!(outcome.payload["stdout"].as_str(), Some("hello"));
    assert_eq!(outcome.payload["exit_code"].as_i64(), Some(7));
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_runtime_policy_defaults_to_non_login_shell() {
    let config = test_tool_runtime_config(std::env::temp_dir());

    assert!(!config.bash_exec.login_shell);
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_runs_command_string_via_bash_runtime() {
    use std::fs;

    let root = unique_tool_temp_dir("loong-bash-exec-command");
    fs::create_dir_all(&root).expect("create fixture root");
    let log_path = root.join("bash-args.log");
    let runtime_path = write_fake_bash_runtime(&root, "fake-bash", &log_path);

    let mut config = test_tool_runtime_config(root.clone());
    config.shell_default_mode = shell_policy_ext::ShellPolicyDefault::Allow;
    config.bash_exec = runtime_config::BashExecRuntimePolicy {
        available: true,
        command: Some(runtime_path),
        ..runtime_config::BashExecRuntimePolicy::default()
    };

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "printf 'hello-from-bash'"}),
        },
        &config,
    )
    .expect("bash command should succeed");

    let logged_args = fs::read_to_string(&log_path).expect("read fake bash args");
    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["stdout"].as_str(), Some("hello-from-bash"));
    assert!(
        logged_args
            .lines()
            .eq(["-c", "printf 'hello-from-bash'"].into_iter()),
        "expected non-login bash invocation, got: {logged_args:?}"
    );

    fs::remove_dir_all(&root).ok();
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_falls_back_to_file_root_when_current_dir_is_unavailable() {
    use std::fs;

    let root = unique_tool_temp_dir("loong-bash-exec-missing-cwd");
    let deleted_cwd = root.join("deleted-cwd");
    let fallback_root = root.join("fallback-root");
    fs::create_dir_all(&deleted_cwd).expect("create deleted cwd");
    fs::create_dir_all(&fallback_root).expect("create fallback root");

    let log_path = fallback_root.join("bash-args.log");
    let runtime_path = write_fake_bash_runtime(&fallback_root, "fake-bash", &log_path);

    let mut config = test_tool_runtime_config(fallback_root);
    config.shell_default_mode = shell_policy_ext::ShellPolicyDefault::Allow;
    config.bash_exec = runtime_config::BashExecRuntimePolicy {
        available: true,
        command: Some(runtime_path),
        ..runtime_config::BashExecRuntimePolicy::default()
    };

    let cwd_guard = ScopedCurrentDir::new(&deleted_cwd);
    fs::remove_dir_all(&deleted_cwd).expect("remove deleted cwd");

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "printf fallback-from-file-root"}),
        },
        &config,
    )
    .expect("bash command should succeed when current dir is unavailable");

    drop(cwd_guard);

    let logged_args = fs::read_to_string(&log_path).expect("read fake bash args");
    assert_eq!(outcome.status, "ok");
    assert_eq!(
        outcome.payload["stdout"].as_str(),
        Some("fallback-from-file-root")
    );
    assert!(
        logged_args
            .lines()
            .eq(["-c", "printf fallback-from-file-root"].into_iter()),
        "expected bash invocation to keep command args when current dir is missing, got: {logged_args:?}"
    );

    fs::remove_dir_all(&root).ok();
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_defaults_cwd_to_configured_file_root() {
    use std::fs;
    use std::path::Path;

    let root = unique_tool_temp_dir("loong-bash-default-cwd");
    fs::create_dir_all(&root).expect("create root");

    let mut config = test_tool_runtime_config(root.clone());
    config.shell_default_mode = shell_policy_ext::ShellPolicyDefault::Allow;
    config.bash_exec = ready_bash_exec_runtime_policy();

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({
                "command": "pwd",
            }),
        },
        &config,
    )
    .expect("bash command should default to configured file root");

    let stdout = outcome.payload["stdout"]
        .as_str()
        .expect("bash.exec should return stdout");
    let actual_cwd = fs::canonicalize(Path::new(stdout)).expect("canonicalize actual cwd");
    let expected_cwd = fs::canonicalize(&root).expect("canonicalize expected cwd");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["cwd"], root.display().to_string());
    assert_eq!(actual_cwd, expected_cwd);

    fs::remove_dir_all(&root).ok();
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_allows_plain_command_when_prefix_rule_allows() {
    use std::fs;

    let root = unique_tool_temp_dir("loong-bash-governance-allow");
    let rules_dir = root.join(crate::config::HOME_DIR_NAME).join("rules");
    fs::create_dir_all(&rules_dir).expect("rules dir");
    fs::write(
        rules_dir.join("allow.rules"),
        "prefix_rule(pattern=[\"printf\",\"ok\"], decision=\"allow\")\n",
    )
    .expect("rule file");

    let mut config = test_tool_runtime_config(root.clone());
    let (bash_exec, _log_path) = configured_test_bash_runtime_with_rules(&root);
    config.bash_exec = bash_exec;

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "printf ok"}),
        },
        &config,
    )
    .expect("allow rule should permit execution");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["stdout"].as_str(), Some("ok"));

    fs::remove_dir_all(&root).ok();
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_uses_loong_home_rules_dir_even_when_runtime_is_built_without_config_path() {
    use std::fs;

    let home = crate::test_support::ScopedLoongHome::new("loong-bash-home-rules");
    let workspace = unique_tool_temp_dir("loong-bash-home-rules-workspace");
    let rules_dir = home.path().join("rules");
    fs::create_dir_all(&rules_dir).expect("rules dir");
    fs::write(
        rules_dir.join("allow.rules"),
        "prefix_rule(pattern=[\"printf\",\"ok\"], decision=\"allow\")\n",
    )
    .expect("rule file");
    fs::create_dir_all(&workspace).expect("workspace");
    let _cwd = ScopedCurrentDir::new(&workspace);

    let mut runtime = runtime_config::ToolRuntimeConfig::from_loong_config(
        &crate::config::LoongConfig::default(),
        None,
    );
    assert_eq!(runtime.bash_exec.governance.rules_dir, rules_dir);
    assert!(
        runtime.bash_exec.governance.load_error.is_none(),
        "default runtime-home rules should load cleanly"
    );

    let log_path = home.join("bash-args.log");
    let runtime_path = write_fake_bash_runtime(home.path(), "fake-bash", &log_path);
    runtime.bash_exec.available = true;
    runtime.bash_exec.command = Some(runtime_path);

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "printf ok"}),
        },
        &runtime,
    )
    .expect("home-default rules should permit execution");

    let logged_args = fs::read_to_string(&log_path).expect("read fake bash args");
    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["stdout"].as_str(), Some("ok"));
    assert!(
        logged_args.lines().eq(["-c", "printf ok"].into_iter()),
        "expected non-login bash invocation, got: {logged_args:?}"
    );

    drop(_cwd);
    drop(home);
    fs::remove_dir_all(&workspace).ok();
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_denies_plain_command_when_prefix_rule_denies() {
    use std::fs;

    let root = unique_tool_temp_dir("loong-bash-governance-deny");
    let rules_dir = root.join(crate::config::HOME_DIR_NAME).join("rules");
    fs::create_dir_all(&rules_dir).expect("rules dir");
    fs::write(
        rules_dir.join("deny.rules"),
        "prefix_rule(pattern=[\"cargo\",\"publish\"], decision=\"deny\")\n",
    )
    .expect("rule file");

    let mut config = test_tool_runtime_config(root.clone());
    let (bash_exec, log_path) = configured_test_bash_runtime_with_rules(&root);
    config.bash_exec = bash_exec;

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "cargo publish"}),
        },
        &config,
    )
    .expect_err("deny rule should block execution");

    assert!(error.contains("policy_denied"));
    assert!(error.contains("matched deny rule"));
    assert!(
        !log_path.exists(),
        "bash runtime should not have been launched"
    );

    fs::remove_dir_all(&root).ok();
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_denies_escaped_static_command_name_when_deny_rule_matches_under_default_allow() {
    use std::fs;

    let root = unique_tool_temp_dir("loong-bash-governance-escaped-deny");
    let rules_dir = root.join(crate::config::HOME_DIR_NAME).join("rules");
    fs::create_dir_all(&rules_dir).expect("rules dir");
    fs::write(
        rules_dir.join("deny.rules"),
        "prefix_rule(pattern=[\"rm\"], decision=\"deny\")\n",
    )
    .expect("rule file");

    let mut config = test_tool_runtime_config(root.clone());
    config.shell_default_mode = shell_policy_ext::ShellPolicyDefault::Allow;
    let (bash_exec, log_path) = configured_test_bash_runtime_with_rules(&root);
    config.bash_exec = bash_exec;

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": r"r\m --version"}),
        },
        &config,
    )
    .expect_err("escaped static command name should still match deny rule");

    assert!(error.contains("policy_denied"));
    assert!(error.contains("matched deny rule"));
    assert!(
        !log_path.exists(),
        "bash runtime should not have been launched"
    );

    fs::remove_dir_all(&root).ok();
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_denies_or_list_when_rhs_branch_matches_deny_rule() {
    use std::fs;

    let root = unique_tool_temp_dir("loong-bash-governance-or-deny");
    let rules_dir = root.join(crate::config::HOME_DIR_NAME).join("rules");
    fs::create_dir_all(&rules_dir).expect("rules dir");
    fs::write(
        rules_dir.join("rules.rules"),
        concat!(
            "prefix_rule(pattern=[\"printf\",\"ok\"], decision=\"allow\")\n",
            "prefix_rule(pattern=[\"printf\",\"blocked\"], decision=\"deny\")\n",
        ),
    )
    .expect("rule file");

    let mut config = test_tool_runtime_config(root.clone());
    let (bash_exec, log_path) = configured_test_bash_runtime_with_rules(&root);
    config.bash_exec = bash_exec;

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "printf ok || printf blocked"}),
        },
        &config,
    )
    .expect_err("conservative || governance should deny the rhs branch");

    assert!(error.contains("policy_denied"));
    assert!(error.contains("matched deny rule"));
    assert!(
        !log_path.exists(),
        "bash runtime should not have been launched"
    );

    fs::remove_dir_all(&root).ok();
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_allows_parse_unreliable_command_when_shell_default_mode_is_allow() {
    use std::fs;

    let root = unique_tool_temp_dir("loong-bash-governance-default-allow");
    fs::create_dir_all(&root).expect("fixture root");

    let mut config = test_tool_runtime_config(root.clone());
    config.shell_default_mode = shell_policy_ext::ShellPolicyDefault::Allow;
    let (bash_exec, _log_path) = configured_test_bash_runtime_with_rules(&root);
    config.bash_exec = bash_exec;

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "if then"}),
        },
        &config,
    )
    .expect("default-allow should permit parse-unreliable input to reach bash");

    assert_eq!(outcome.status, "failed");

    fs::remove_dir_all(&root).ok();
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_keeps_shell_exec_unchanged() {
    let config = test_tool_runtime_config(std::env::temp_dir());
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({"command": "echo", "args": ["hi"]}),
        },
        &config,
    )
    .expect("shell.exec should remain runnable");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["stdout"].as_str(), Some("hi"));
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_honors_cwd() {
    use std::fs;
    use std::path::Path;

    let root = unique_tool_temp_dir("loong-bash-exec-cwd");
    let nested = root.join("nested");
    let requested_cwd = "nested";
    fs::create_dir_all(&nested).expect("create nested dir");

    let mut config = test_tool_runtime_config(root.clone());
    config.shell_default_mode = shell_policy_ext::ShellPolicyDefault::Allow;
    config.bash_exec = ready_bash_exec_runtime_policy();

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({
                "command": "pwd",
                "cwd": requested_cwd,
            }),
        },
        &config,
    )
    .expect("bash command should honor cwd");

    let expected_cwd = fs::canonicalize(&nested).expect("canonicalize expected cwd");
    let stdout = outcome.payload["stdout"]
        .as_str()
        .expect("bash.exec should return stdout");
    let actual_cwd = fs::canonicalize(Path::new(stdout)).expect("canonicalize actual cwd");

    assert_eq!(outcome.status, "ok");
    assert_eq!(actual_cwd, expected_cwd);

    fs::remove_dir_all(&root).ok();
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_rejects_cwd_that_escapes_configured_file_root() {
    use std::fs;

    let root = unique_tool_temp_dir("loong-bash-cwd-root");
    let outside = unique_tool_temp_dir("loong-bash-cwd-outside");
    fs::create_dir_all(&root).expect("create root");
    fs::create_dir_all(&outside).expect("create outside");

    let mut config = test_tool_runtime_config(root.clone());
    config.shell_default_mode = shell_policy_ext::ShellPolicyDefault::Allow;
    config.bash_exec = ready_bash_exec_runtime_policy();

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({
                "command": "pwd",
                "cwd": outside.display().to_string(),
            }),
        },
        &config,
    )
    .expect_err("bash cwd outside file root should fail");

    assert!(
        error.contains("escapes configured file root"),
        "expected file_root confinement failure, got: {error}"
    );

    fs::remove_dir_all(&root).ok();
    fs::remove_dir_all(&outside).ok();
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn bash_exec_times_out_when_timeout_ms_is_small() {
    let mut config = test_tool_runtime_config(std::env::temp_dir());
    config.shell_default_mode = shell_policy_ext::ShellPolicyDefault::Allow;
    config.bash_exec = ready_bash_exec_runtime_policy();

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({
                "command": "sleep 10",
                "timeout_ms": 1,
            }),
        },
        &config,
    )
    .expect_err("slow bash command should time out");

    assert!(
        error.contains("timed out after"),
        "expected timeout failure, got: {error}"
    );
}

#[cfg(all(feature = "tool-shell", unix))]
#[test]
fn tool_invoke_dispatches_bash_exec_with_trusted_internal_context() {
    use std::fs;

    let root = unique_tool_temp_dir("loong-tool-invoke-bash-exec");
    fs::create_dir_all(&root).expect("create fixture root");

    let mut config = test_tool_runtime_config(root.clone());
    config.shell_default_mode = shell_policy_ext::ShellPolicyDefault::Allow;
    config.bash_exec = ready_bash_exec_runtime_policy();

    let search = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "tool.search".to_owned(),
            payload: json!({"query": "bash command cwd timeout"}),
        },
        &config,
    )
    .expect("tool search should succeed");

    let result = search.payload["results"]
        .as_array()
        .expect("results")
        .iter()
        .find(|entry| entry["tool_id"] == "bash-exec")
        .expect("bash-exec search result");

    let outcome = execute_tool_core_with_test_context(
        ToolCoreRequest {
            tool_name: "tool.invoke".to_owned(),
            payload: json!({
                "tool_id": "bash-exec",
                "lease": result["lease"].clone(),
                "arguments": {
                    "command": "printf 'invoke-bash'"
                },
                "_loong": {
                    LOONG_INTERNAL_RUNTIME_NARROWING_KEY: {}
                }
            }),
        },
        &config,
    )
    .expect("tool.invoke should execute bash.exec with trusted internal context");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["stdout"].as_str(), Some("invoke-bash"));

    fs::remove_dir_all(&root).ok();
}
